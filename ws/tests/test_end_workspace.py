"""Tests for safe workspace termination workflow ('ws end' / 'ws close')."""

import json
from pathlib import Path
import tempfile
import pytest

from ws.cli import build_parser, normalize_cli_args
from ws.config import AppConfig
from ws.exceptions import (
    WorkspaceNotFoundException,
    WorkspaceSessionStopException,
    WorkspaceUncommittedChangesException,
    WorkspaceUnmergedBranchException,
)
from ws.git import GitService
from ws.models import RepoConfig, RepoSpec, WorkspaceMetadata
from ws.workspace import WorkspaceManager


@pytest.fixture
def temp_env():
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        bare_dir = root / "bares" / "repo1.git"
        bare_dir.mkdir(parents=True)

        ws_root = root / "workspaces"
        ws_root.mkdir(parents=True)

        app_cfg = AppConfig(
            repositories={
                "repo1": RepoConfig(name="repo1", bare=bare_dir, checkout="repo1")
            },
            workspaces_dir=ws_root,
        )
        git = GitService()
        manager = WorkspaceManager(config=app_cfg, git_service=git)
        yield root, manager, git, app_cfg


def _setup_mock_workspace(manager: WorkspaceManager, ws_name: str, branch: str = "feature/test") -> Path:
    ws_dir = manager.config.workspaces_dir / ws_name
    ws_dir.mkdir(parents=True, exist_ok=True)
    wt_path = ws_dir / "repo1"
    wt_path.mkdir(parents=True, exist_ok=True)

    meta = WorkspaceMetadata(
        name=ws_name,
        created="2026-09-20T12:00:00Z",
        status="active",
        repositories={
            "repo1": RepoSpec(
                name="repo1",
                branch=branch,
                create=True,
                path="repo1",
            )
        },
    )
    meta_file = ws_dir / "workspace.yml"
    import yaml
    with open(meta_file, "w", encoding="utf-8") as f:
        yaml.safe_dump(meta.to_dict(), f)

    return ws_dir


def test_end_workspace_clean_and_merged(temp_env, monkeypatch):
    """Clean and merged workspace closes cleanly without flags."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "prune_worktrees", lambda p: None)
    monkeypatch.setattr(git, "remove_worktree", lambda b, w, force=True: None)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {"has_uncommitted": False, "modified": [], "untracked": []})
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (True, "main", 0))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    manager.end_workspace("feat-auth")
    assert not ws_dir.exists()


def test_end_workspace_uncommitted_prevented(temp_env, monkeypatch):
    """Uncommitted changes prevent workspace closing."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {
        "has_uncommitted": True,
        "modified": ["src/index.ts"],
        "untracked": ["newfile.txt"],
    })
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (True, "main", 0))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    with pytest.raises(WorkspaceUncommittedChangesException) as excinfo:
        manager.end_workspace("feat-auth")

    assert "uncommitted changes detected" in str(excinfo.value)
    assert "%repo1" in str(excinfo.value)
    assert ws_dir.exists()


def test_end_workspace_uncommitted_prevented_even_with_no_merge(temp_env, monkeypatch):
    """--no-merge flag does NOT bypass uncommitted changes."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {
        "has_uncommitted": True,
        "modified": ["src/index.ts"],
        "untracked": [],
    })
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (False, "main", 3))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    with pytest.raises(WorkspaceUncommittedChangesException):
        manager.end_workspace("feat-auth", no_merge=True)

    assert ws_dir.exists()


def test_end_workspace_uncommitted_force_allowed(temp_env, monkeypatch):
    """--force flag allows closing despite uncommitted changes."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "prune_worktrees", lambda p: None)
    monkeypatch.setattr(git, "remove_worktree", lambda b, w, force=True: None)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {
        "has_uncommitted": True,
        "modified": ["src/index.ts"],
        "untracked": ["newfile.txt"],
    })
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (False, "main", 2))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    manager.end_workspace("feat-auth", force=True)
    assert not ws_dir.exists()


def test_end_workspace_unmerged_prevented(temp_env, monkeypatch):
    """Unmerged commits prevent closing when changes are committed."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {"has_uncommitted": False, "modified": [], "untracked": []})
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (False, "main", 4))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    with pytest.raises(WorkspaceUnmergedBranchException) as excinfo:
        manager.end_workspace("feat-auth")

    assert "unmerged work detected" in str(excinfo.value)
    assert "4 commit(s) not merged into 'main'" in str(excinfo.value)
    assert ws_dir.exists()


def test_end_workspace_unmerged_no_merge_allowed(temp_env, monkeypatch):
    """--no-merge allows closing unmerged branch if all work is committed."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "prune_worktrees", lambda p: None)
    monkeypatch.setattr(git, "remove_worktree", lambda b, w, force=True: None)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {"has_uncommitted": False, "modified": [], "untracked": []})
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (False, "main", 2))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    manager.end_workspace("feat-auth", no_merge=True)
    assert not ws_dir.exists()


def test_end_workspace_custom_target_branch(temp_env, monkeypatch):
    """Custom target branch is checked for merge status."""
    root, manager, git, _ = temp_env
    _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    target_seen = []
    def mock_is_branch_merged(bare_path, branch, target_branch=None, worktree_path=None):
        target_seen.append(target_branch)
        return True, target_branch or "main", 0

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "prune_worktrees", lambda p: None)
    monkeypatch.setattr(git, "remove_worktree", lambda b, w, force=True: None)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {"has_uncommitted": False, "modified": [], "untracked": []})
    monkeypatch.setattr(git, "is_branch_merged", mock_is_branch_merged)
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    manager.end_workspace("feat-auth", target_branch="develop")
    assert target_seen == ["develop"]


def test_end_workspace_delete_branch_flag(temp_env, monkeypatch):
    """--delete-branch flag deletes branch from bare repository."""
    root, manager, git, _ = temp_env
    _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    deleted_branches = []
    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "prune_worktrees", lambda p: None)
    monkeypatch.setattr(git, "remove_worktree", lambda b, w, force=True: None)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {"has_uncommitted": False, "modified": [], "untracked": []})
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (True, "main", 0))
    monkeypatch.setattr(git, "get_default_branch", lambda b: "main")
    monkeypatch.setattr(git, "delete_branch", lambda b, br, force=True: deleted_branches.append(br))
    monkeypatch.setattr(manager, "is_session_running", lambda name: False)

    # Without delete_branch: branch is kept
    _setup_mock_workspace(manager, "feat-auth", "feature/auth")
    manager.end_workspace("feat-auth", delete_branch=False)
    assert deleted_branches == []

    # With delete_branch: branch is deleted
    _setup_mock_workspace(manager, "feat-auth", "feature/auth")
    manager.end_workspace("feat-auth", delete_branch=True)
    assert deleted_branches == ["feature/auth"]


def test_end_workspace_session_stop_requirement(temp_env, monkeypatch):
    """Closing only happens if session is successfully terminated."""
    root, manager, git, _ = temp_env
    ws_dir = _setup_mock_workspace(manager, "feat-auth", "feature/auth")

    monkeypatch.setattr(git, "is_bare_repo", lambda p: True)
    monkeypatch.setattr(git, "check_worktree_uncommitted", lambda w: {"has_uncommitted": False, "modified": [], "untracked": []})
    monkeypatch.setattr(git, "is_branch_merged", lambda bare_path, branch, target_branch=None, worktree_path=None: (True, "main", 0))

    # Session running and fails to terminate
    monkeypatch.setattr(manager, "is_session_running", lambda name: True)
    monkeypatch.setattr(manager, "stop_workspace", lambda name: False)

    with pytest.raises(WorkspaceSessionStopException):
        manager.end_workspace("feat-auth")

    assert ws_dir.exists()

    # With force=True, closure proceeds anyway
    monkeypatch.setattr(git, "remove_worktree", lambda b, w, force=True: None)
    monkeypatch.setattr(git, "prune_worktrees", lambda p: None)
    manager.end_workspace("feat-auth", force=True)
    assert not ws_dir.exists()


def test_cli_end_and_close_parsing():
    """CLI parser recognizes end and close with all flags."""
    parser = build_parser()

    # ws end @test --no-merge --delete-branch
    args = parser.parse_args(["end", "@test", "--no-merge", "--delete-branch"])
    assert args.subcommand == "end"
    assert args.name == "@test"
    assert args.no_merge is True
    assert args.force is False
    assert args.delete_branch is True

    # ws close @test -f -t develop
    args = parser.parse_args(["close", "@test", "-f", "-t", "develop"])
    assert args.subcommand == "close"
    assert args.name == "@test"
    assert args.force is True
    assert args.target_branch == "develop"

    # ws delete @test --force
    args = parser.parse_args(["delete", "@test", "--force"])
    assert args.subcommand == "delete"
    assert args.force is True

    # Inverted syntax
    normalized = normalize_cli_args(["@feat-auth", "end", "--no-merge"])
    assert normalized == ["end", "@feat-auth", "--no-merge"]

    normalized_close = normalize_cli_args(["@feat-auth", "close"])
    assert normalized_close == ["close", "@feat-auth"]


def test_is_branch_merged_direct_ancestry():
    """Direct ancestry (merge commit or fast-forward) is detected as merged."""
    import subprocess
    git = GitService()
    with tempfile.TemporaryDirectory() as tmp:
        repo_dir = Path(tmp)
        subprocess.run(["git", "init", "-b", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=repo_dir, check=True)
        (repo_dir / "init.txt").write_text("initial")
        subprocess.run(["git", "add", "init.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=repo_dir, check=True)

        subprocess.run(["git", "checkout", "-b", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        (repo_dir / "auth.txt").write_text("auth code")
        subprocess.run(["git", "add", "auth.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "add auth"], cwd=repo_dir, check=True)

        # Merge into main
        subprocess.run(["git", "checkout", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "merge", "--no-ff", "feature/auth", "-m", "merge feature/auth"], cwd=repo_dir, check=True, capture_output=True)

        is_merged, target, unmerged = git.is_branch_merged(
            bare_path=repo_dir / ".git",
            branch="feature/auth",
            worktree_path=repo_dir,
        )
        assert is_merged is True
        assert target == "main"
        assert unmerged == 0


def test_is_branch_merged_cherry_pick():
    """Branch whose commits were cherry-picked or rebase-merged is detected as merged."""
    import subprocess
    git = GitService()
    with tempfile.TemporaryDirectory() as tmp:
        repo_dir = Path(tmp)
        subprocess.run(["git", "init", "-b", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=repo_dir, check=True)
        (repo_dir / "init.txt").write_text("initial")
        subprocess.run(["git", "add", "init.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=repo_dir, check=True)

        subprocess.run(["git", "checkout", "-b", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        (repo_dir / "auth.txt").write_text("auth code")
        subprocess.run(["git", "add", "auth.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "add auth"], cwd=repo_dir, check=True)

        # Cherry-pick into main (creates different commit hash, so not direct ancestor)
        subprocess.run(["git", "checkout", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "cherry-pick", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)

        is_merged, target, unmerged = git.is_branch_merged(
            bare_path=repo_dir / ".git",
            branch="feature/auth",
            worktree_path=repo_dir,
        )
        assert is_merged is True
        assert target == "main"
        assert unmerged == 0


def test_is_branch_merged_squash_merge_multi_commit():
    """Branch squashed into target (even with subsequent target commits) is detected as merged."""
    import subprocess
    git = GitService()
    with tempfile.TemporaryDirectory() as tmp:
        repo_dir = Path(tmp)
        subprocess.run(["git", "init", "-b", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=repo_dir, check=True)
        (repo_dir / "init.txt").write_text("initial")
        subprocess.run(["git", "add", "init.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=repo_dir, check=True)

        subprocess.run(["git", "checkout", "-b", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        (repo_dir / "auth1.txt").write_text("auth step 1")
        subprocess.run(["git", "add", "auth1.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "auth step 1"], cwd=repo_dir, check=True)

        (repo_dir / "auth2.txt").write_text("auth step 2")
        subprocess.run(["git", "add", "auth2.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "auth step 2"], cwd=repo_dir, check=True)

        # Squash merge into main
        subprocess.run(["git", "checkout", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "merge", "--squash", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "commit", "-m", "squashed auth (#1)"], cwd=repo_dir, check=True, capture_output=True)

        # Subsequent commit on main
        (repo_dir / "later.txt").write_text("later commit")
        subprocess.run(["git", "add", "later.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "later commit on main"], cwd=repo_dir, check=True)

        is_merged, target, unmerged = git.is_branch_merged(
            bare_path=repo_dir / ".git",
            branch="feature/auth",
            worktree_path=repo_dir,
        )
        assert is_merged is True
        assert target == "main"
        assert unmerged == 0


def test_is_branch_merged_unmerged_commits():
    """Branch with unmerged commits is detected as unmerged with accurate count."""
    import subprocess
    git = GitService()
    with tempfile.TemporaryDirectory() as tmp:
        repo_dir = Path(tmp)
        subprocess.run(["git", "init", "-b", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=repo_dir, check=True)
        (repo_dir / "init.txt").write_text("initial")
        subprocess.run(["git", "add", "init.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=repo_dir, check=True)

        subprocess.run(["git", "checkout", "-b", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        (repo_dir / "auth1.txt").write_text("auth step 1")
        subprocess.run(["git", "add", "auth1.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "auth step 1"], cwd=repo_dir, check=True)

        (repo_dir / "auth2.txt").write_text("auth step 2")
        subprocess.run(["git", "add", "auth2.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "auth step 2"], cwd=repo_dir, check=True)

        is_merged, target, unmerged = git.is_branch_merged(
            bare_path=repo_dir / ".git",
            branch="feature/auth",
            target_branch="main",
            worktree_path=repo_dir,
        )
        assert is_merged is False
        assert target == "main"
        assert unmerged == 2


def test_is_branch_merged_partially_merged():
    """Branch with one cherry-picked commit and one unmerged commit reports 1 unmerged commit."""
    import subprocess
    git = GitService()
    with tempfile.TemporaryDirectory() as tmp:
        repo_dir = Path(tmp)
        subprocess.run(["git", "init", "-b", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=repo_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=repo_dir, check=True)
        (repo_dir / "init.txt").write_text("initial")
        subprocess.run(["git", "add", "init.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=repo_dir, check=True)

        subprocess.run(["git", "checkout", "-b", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        (repo_dir / "auth1.txt").write_text("auth step 1")
        subprocess.run(["git", "add", "auth1.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "auth step 1"], cwd=repo_dir, check=True)

        # Cherry-pick auth1 into main
        subprocess.run(["git", "checkout", "main"], cwd=repo_dir, check=True, capture_output=True)
        subprocess.run(["git", "cherry-pick", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)

        # Add auth2 commit to feature/auth (remains unmerged)
        subprocess.run(["git", "checkout", "feature/auth"], cwd=repo_dir, check=True, capture_output=True)
        (repo_dir / "auth2.txt").write_text("auth step 2")
        subprocess.run(["git", "add", "auth2.txt"], cwd=repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "auth step 2"], cwd=repo_dir, check=True)

        is_merged, target, unmerged = git.is_branch_merged(
            bare_path=repo_dir / ".git",
            branch="feature/auth",
            target_branch="main",
            worktree_path=repo_dir,
        )
        assert is_merged is False
        assert target == "main"
        assert unmerged == 1


def test_is_branch_merged_prefers_remote_target():
    """Branch merged on remote is detected as merged even if local base branch is stale."""
    import subprocess
    git = GitService()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        origin_bare = root / "origin.git"
        subprocess.run(["git", "init", "--bare", "-b", "main", str(origin_bare)], check=True, capture_output=True)

        # Initial seed commit pushed to origin
        seed_dir = root / "seed"
        subprocess.run(["git", "clone", str(origin_bare), str(seed_dir)], check=True, capture_output=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=seed_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=seed_dir, check=True)
        (seed_dir / "init.txt").write_text("initial")
        subprocess.run(["git", "add", "init.txt"], cwd=seed_dir, check=True)
        subprocess.run(["git", "commit", "-m", "initial commit"], cwd=seed_dir, check=True)
        subprocess.run(["git", "push", "origin", "main"], cwd=seed_dir, check=True)

        # Clone local bare repo (like ws-manager does)
        local_bare = root / "local.git"
        git.clone_bare(url=str(origin_bare), target_bare_path=local_bare)

        # Create worktree for feature/auth
        wt_dir = root / "wt-auth"
        git.create_worktree(bare_path=local_bare, worktree_path=wt_dir, branch="feature/auth", create_branch=True)
        subprocess.run(["git", "config", "user.name", "Test"], cwd=wt_dir, check=True)
        subprocess.run(["git", "config", "user.email", "test@test.com"], cwd=wt_dir, check=True)
        (wt_dir / "auth.txt").write_text("feature work")
        subprocess.run(["git", "add", "auth.txt"], cwd=wt_dir, check=True)
        subprocess.run(["git", "commit", "-m", "add auth feature"], cwd=wt_dir, check=True)
        # Push feature branch to origin
        subprocess.run(["git", "push", "origin", "feature/auth"], cwd=wt_dir, check=True)

        # Simulate PR merge on origin (remote main now has the feature commit)
        subprocess.run(["git", "checkout", "main"], cwd=seed_dir, check=True, capture_output=True)
        subprocess.run(["git", "fetch", "origin"], cwd=seed_dir, check=True, capture_output=True)
        subprocess.run(["git", "merge", "--no-ff", "origin/feature/auth", "-m", "Merge PR #1"], cwd=seed_dir, check=True, capture_output=True)
        subprocess.run(["git", "push", "origin", "main"], cwd=seed_dir, check=True, capture_output=True)

        # In local_bare, local 'main' is still at the initial commit!
        # But is_branch_merged should fetch and compare with origin/main:
        is_merged, target, unmerged = git.is_branch_merged(
            bare_path=local_bare,
            branch="feature/auth",
            worktree_path=wt_dir,
        )
        assert is_merged is True
        assert target == "origin/main"
        assert unmerged == 0

        # Now add an unmerged commit to feature/auth
        (wt_dir / "extra.txt").write_text("unmerged extra work")
        subprocess.run(["git", "add", "extra.txt"], cwd=wt_dir, check=True)
        subprocess.run(["git", "commit", "-m", "unmerged commit"], cwd=wt_dir, check=True)

        is_merged_2, target_2, unmerged_2 = git.is_branch_merged(
            bare_path=local_bare,
            branch="feature/auth",
            worktree_path=wt_dir,
        )
        assert is_merged_2 is False
        assert target_2 == "origin/main"
        assert unmerged_2 == 1


