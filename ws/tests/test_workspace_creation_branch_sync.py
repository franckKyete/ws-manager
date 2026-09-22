"""Unit and integration tests for workspace creation target branch validation & synchronization."""

from pathlib import Path
import subprocess
import tempfile
import pytest

from ws.cli import parse_create_workspace_args
from ws.config import AppConfig
from ws.exceptions import ValidationException
from ws.git import GitService
from ws.models import RepoConfig, RepoSpec
from ws.workspace import WorkspaceManager


def _run(cmd: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True)


def _init_git_repo(path: Path, branch: str = "main") -> None:
    _run(["git", "init", "-b", branch], cwd=path)
    _run(["git", "config", "user.name", "Test User"], cwd=path)
    _run(["git", "config", "user.email", "test@example.com"], cwd=path)
    (path / "README.md").write_text("# Test Repo\n")
    _run(["git", "add", "README.md"], cwd=path)
    _run(["git", "commit", "-m", "Initial commit"], cwd=path)


def _make_bare_from_repo(source_repo: Path, bare_path: Path) -> None:
    _run(["git", "clone", "--bare", str(source_repo), str(bare_path)])
    _run(["git", "--git-dir", str(bare_path), "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"])


@pytest.fixture
def sync_test_env():
    """Create a temporary environment with an origin remote, a bare repo, and workspace manager."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)

        origin_dir = tmp_path / "remote_origin" / "repo1.git"
        origin_dir.mkdir(parents=True)
        # Create non-bare repo first to commit, then clone to bare origin
        seed_dir = tmp_path / "seed"
        seed_dir.mkdir(parents=True)
        _init_git_repo(seed_dir, branch="main")
        _run(["git", "clone", "--bare", str(seed_dir), str(origin_dir)])
        _run(["git", "remote", "add", "origin", str(origin_dir)], cwd=seed_dir)

        # Now create local bare repository for ws
        bare_dir = tmp_path / "bares" / "repo1.git"
        bare_dir.mkdir(parents=True)
        _run(["git", "clone", "--bare", str(origin_dir), str(bare_dir)])
        _run(["git", "--git-dir", str(bare_dir), "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"])
        # Fetch origin into bare_dir
        _run(["git", "--git-dir", str(bare_dir), "fetch", "origin"])

        workspaces_dir = tmp_path / "workspaces"
        workspaces_dir.mkdir(parents=True)

        app_cfg = AppConfig(
            repositories={
                "repo1": RepoConfig(name="repo1", bare=bare_dir, checkout="repo1")
            },
            workspaces_dir=workspaces_dir,
        )
        git = GitService()
        manager = WorkspaceManager(config=app_cfg, git_service=git)

        yield {
            "tmp": tmp_path,
            "origin": origin_dir,
            "bare": bare_dir,
            "seed": seed_dir,
            "workspaces": workspaces_dir,
            "manager": manager,
            "git": git,
            "app_cfg": app_cfg,
        }


def test_main_branch_resolution(sync_test_env):
    """Test resolution order: main > develop > master > default."""
    git = sync_test_env["git"]
    bare = sync_test_env["bare"]

    # In our fixture, main exists
    assert git.resolve_main_branch(bare) == "main"

    # In a repo with only develop and master
    with tempfile.TemporaryDirectory() as tmp2:
        p = Path(tmp2)
        _init_git_repo(p, branch="master")
        _run(["git", "checkout", "-b", "develop"], cwd=p)
        (p / "dev.txt").write_text("develop")
        _run(["git", "add", "dev.txt"], cwd=p)
        _run(["git", "commit", "-m", "develop commit"], cwd=p)

        bare2 = Path(tmp2) / "bare2.git"
        _run(["git", "clone", "--bare", str(p), str(bare2)])

        assert git.resolve_main_branch(bare2) == "develop"


def test_target_up_to_date_creates_directly(sync_test_env):
    """When local target branch is up-to-date with remote origin, workspace is created directly."""
    manager = sync_test_env["manager"]

    spec = RepoSpec(name="repo1", branch="feature/feat1", create=True, path="repo1")
    meta = manager.create_workspace("ws-up-to-date", [spec], no_tmux=True)

    assert meta.name == "ws-up-to-date"
    assert "repo1" in meta.repositories
    assert meta.repositories["repo1"].base_branch == "main"

    # Verify worktree exists
    wt = sync_test_env["workspaces"] / "ws-up-to-date" / "repo1"
    assert wt.exists()
    assert (wt / "README.md").exists()


def test_target_advanced_ahead_creates_directly(sync_test_env):
    """When local target branch is ahead of remote origin, workspace is created directly without pulling."""
    bare = sync_test_env["bare"]
    manager = sync_test_env["manager"]

    # Add a local commit to bare's main branch using a temporary worktree
    tmp_wt = sync_test_env["tmp"] / "tmp_wt"
    _run(["git", "--git-dir", str(bare), "worktree", "add", str(tmp_wt), "main"])
    (tmp_wt / "local_change.txt").write_text("ahead of origin")
    _run(["git", "add", "local_change.txt"], cwd=tmp_wt)
    _run(["git", "commit", "-m", "Commit ahead of origin"], cwd=tmp_wt)
    _run(["git", "--git-dir", str(bare), "worktree", "remove", str(tmp_wt)])

    ahead, behind = manager.git.get_branch_divergence(bare, "main")
    assert ahead == 1
    assert behind == 0

    spec = RepoSpec(name="repo1", branch="feature/feat-advanced", create=True, path="repo1")
    meta = manager.create_workspace("ws-advanced", [spec], no_tmux=True)

    wt = sync_test_env["workspaces"] / "ws-advanced" / "repo1"
    assert (wt / "local_change.txt").exists()
    assert meta.repositories["repo1"].base_branch == "main"


def test_target_diverged_raises_validation_exception(sync_test_env):
    """When local target branch and remote origin have diverged, creation fails with ValidationException."""
    origin = sync_test_env["origin"]
    bare = sync_test_env["bare"]
    seed = sync_test_env["seed"]
    manager = sync_test_env["manager"]

    # 1. Add commit on origin main
    (seed / "remote_change.txt").write_text("remote change")
    _run(["git", "add", "remote_change.txt"], cwd=seed)
    _run(["git", "commit", "-m", "Remote commit"], cwd=seed)
    _run(["git", "push", "origin", "main"], cwd=seed)

    # 2. Add conflicting local commit on bare main
    tmp_wt = sync_test_env["tmp"] / "tmp_wt"
    _run(["git", "--git-dir", str(bare), "worktree", "add", str(tmp_wt), "main"])
    (tmp_wt / "local_diverged.txt").write_text("local diverged")
    _run(["git", "add", "local_diverged.txt"], cwd=tmp_wt)
    _run(["git", "commit", "-m", "Local diverged commit"], cwd=tmp_wt)
    _run(["git", "--git-dir", str(bare), "worktree", "remove", str(tmp_wt)])

    spec = RepoSpec(name="repo1", branch="feature/feat-diverged", create=True, path="repo1")

    with pytest.raises(ValidationException, match="has diverged from 'origin/main'"):
        manager.create_workspace("ws-diverged", [spec], no_tmux=True)

    # Workspace should not exist
    assert not (sync_test_env["workspaces"] / "ws-diverged").exists()


def test_target_behind_bare_fast_forwards(sync_test_env):
    """When bare repo target branch is behind origin and not in a worktree, it fast-forwards."""
    origin = sync_test_env["origin"]
    bare = sync_test_env["bare"]
    seed = sync_test_env["seed"]
    manager = sync_test_env["manager"]

    # Add commit on origin
    (seed / "new_remote_feature.txt").write_text("from origin")
    _run(["git", "add", "new_remote_feature.txt"], cwd=seed)
    _run(["git", "commit", "-m", "New remote commit"], cwd=seed)
    _run(["git", "push", "origin", "main"], cwd=seed)

    spec = RepoSpec(name="repo1", branch="feature/feat-behind-bare", create=True, path="repo1")
    meta = manager.create_workspace("ws-behind-bare", [spec], no_tmux=True)

    wt = sync_test_env["workspaces"] / "ws-behind-bare" / "repo1"
    assert (wt / "new_remote_feature.txt").exists()
    assert (wt / "new_remote_feature.txt").read_text() == "from origin"


def test_target_behind_worktree_clean_pulls(sync_test_env):
    """When target branch is checked out in a clean worktree, ws pulls latest changes before creating."""
    origin = sync_test_env["origin"]
    bare = sync_test_env["bare"]
    seed = sync_test_env["seed"]
    manager = sync_test_env["manager"]

    # Check out main in an existing worktree
    main_wt = sync_test_env["tmp"] / "main_worktree"
    _run(["git", "--git-dir", str(bare), "worktree", "add", str(main_wt), "main"])

    # Push commit to origin
    (seed / "origin_file.txt").write_text("origin file content")
    _run(["git", "add", "origin_file.txt"], cwd=seed)
    _run(["git", "commit", "-m", "Pushed to origin"], cwd=seed)
    _run(["git", "push", "origin", "main"], cwd=seed)

    spec = RepoSpec(name="repo1", branch="feature/feat-pulled", create=True, path="repo1")
    manager.create_workspace("ws-pulled", [spec], no_tmux=True)

    # Main worktree was pulled
    assert (main_wt / "origin_file.txt").exists()

    # New workspace worktree has origin file
    new_wt = sync_test_env["workspaces"] / "ws-pulled" / "repo1"
    assert (new_wt / "origin_file.txt").exists()


def test_target_behind_worktree_dirty_tracked_files_blocks(sync_test_env):
    """When target branch is in a worktree with modified tracked files, creation fails before pull."""
    origin = sync_test_env["origin"]
    bare = sync_test_env["bare"]
    seed = sync_test_env["seed"]
    manager = sync_test_env["manager"]

    # Check out main in a worktree
    main_wt = sync_test_env["tmp"] / "main_dirty_wt"
    _run(["git", "--git-dir", str(bare), "worktree", "add", str(main_wt), "main"])

    # Push commit to origin
    (seed / "origin_update.txt").write_text("origin update")
    _run(["git", "add", "origin_update.txt"], cwd=seed)
    _run(["git", "commit", "-m", "Pushed to origin"], cwd=seed)
    _run(["git", "push", "origin", "main"], cwd=seed)

    # Modify a tracked file in main_wt
    (main_wt / "README.md").write_text("# Dirty Modified README\n")

    spec = RepoSpec(name="repo1", branch="feature/feat-blocked", create=True, path="repo1")

    with pytest.raises(ValidationException, match="uncommitted changes in tracked files"):
        manager.create_workspace("ws-blocked", [spec], no_tmux=True)

    # Verify pull was aborted and ws was not created
    assert not (sync_test_env["workspaces"] / "ws-blocked").exists()


def test_target_behind_worktree_untracked_files_allowed(sync_test_env):
    """Untracked files (e.g. .env, build output) in target worktree do NOT block pulling."""
    origin = sync_test_env["origin"]
    bare = sync_test_env["bare"]
    seed = sync_test_env["seed"]
    manager = sync_test_env["manager"]

    # Check out main in a worktree
    main_wt = sync_test_env["tmp"] / "main_untracked_wt"
    _run(["git", "--git-dir", str(bare), "worktree", "add", str(main_wt), "main"])

    # Push commit to origin
    (seed / "remote_doc.txt").write_text("remote doc")
    _run(["git", "add", "remote_doc.txt"], cwd=seed)
    _run(["git", "commit", "-m", "Remote doc commit"], cwd=seed)
    _run(["git", "push", "origin", "main"], cwd=seed)

    # Add untracked files (.env and some random file)
    (main_wt / ".env").write_text("SECRET=123\n")
    (main_wt / "untracked_scratch.txt").write_text("scratch\n")

    spec = RepoSpec(name="repo1", branch="feature/feat-untracked-ok", create=True, path="repo1")
    manager.create_workspace("ws-untracked-ok", [spec], no_tmux=True)

    # Succeeded! Both worktree and new workspace have the remote commit
    assert (main_wt / "remote_doc.txt").exists()
    new_wt = sync_test_env["workspaces"] / "ws-untracked-ok" / "repo1"
    assert (new_wt / "remote_doc.txt").exists()


def test_custom_target_branch_option(sync_test_env):
    """Custom target branch (develop) is used as base and follows identical synchronization rules."""
    origin = sync_test_env["origin"]
    bare = sync_test_env["bare"]
    seed = sync_test_env["seed"]
    manager = sync_test_env["manager"]

    # Create develop on origin
    _run(["git", "checkout", "-b", "develop"], cwd=seed)
    (seed / "develop.txt").write_text("develop branch content")
    _run(["git", "add", "develop.txt"], cwd=seed)
    _run(["git", "commit", "-m", "develop init"], cwd=seed)
    _run(["git", "push", "origin", "develop"], cwd=seed)

    spec = RepoSpec(
        name="repo1",
        branch="feature/feat-from-develop",
        create=True,
        path="repo1",
        base_branch="develop",
    )
    meta = manager.create_workspace("ws-from-dev", [spec], no_tmux=True)

    assert meta.repositories["repo1"].base_branch == "develop"
    wt = sync_test_env["workspaces"] / "ws-from-dev" / "repo1"
    assert (wt / "develop.txt").exists()


def test_cli_parsing_target_branch_options():
    """Verify CLI argument parser correctly parses --target, -t, --base, --from and per-repo colon syntax."""
    repos = {
        "server": RepoConfig(name="server", bare=Path("/bares/server.git"), checkout="server"),
        "client": RepoConfig(name="client", bare=Path("/bares/client.git"), checkout="client"),
    }

    # Global --target
    specs = parse_create_workspace_args(
        "my-ws",
        ["--all", "--target", "develop"],
        repositories=repos,
    )
    assert len(specs) == 2
    for s in specs:
        assert s.base_branch == "develop"
        assert s.branch == "feature/my-ws"
        assert s.create is True

    # Global -t
    specs = parse_create_workspace_args(
        "my-ws",
        ["%server", "-t", "master"],
        repositories=repos,
    )
    assert len(specs) == 1
    assert specs[0].name == "server"
    assert specs[0].base_branch == "master"

    # Per-repo colon syntax with base branch: %server:feat-auth:new:develop
    specs = parse_create_workspace_args(
        "my-ws",
        ["%server:feat-auth:new:develop"],
        repositories=repos,
    )
    assert len(specs) == 1
    assert specs[0].name == "server"
    assert specs[0].branch == "feat-auth"
    assert specs[0].create is True
    assert specs[0].base_branch == "develop"

    # Per-repo colon syntax: %server:feat-auth:develop
    specs = parse_create_workspace_args(
        "my-ws",
        ["%server:feat-auth:develop"],
        repositories=repos,
    )
    assert len(specs) == 1
    assert specs[0].name == "server"
    assert specs[0].branch == "feat-auth"
    assert specs[0].create is True
    assert specs[0].base_branch == "develop"
