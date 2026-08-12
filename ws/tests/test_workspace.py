"""Unit and integration tests for WorkspaceManager and rollback mechanisms."""

import os
from pathlib import Path
import shutil
import tempfile
import pytest

from ws.config import AppConfig
from ws.exceptions import (
    BranchAlreadyExistsException,
    BranchNotFoundException,
    RepositoryNotFoundException,
    RollbackException,
    WorkspaceExistsException,
    WorkspaceNotFoundException,
)
from ws.git import GitService
from ws.models import RepoConfig, RepoSpec
from ws.workspace import RollbackStack, WorkspaceManager


@pytest.fixture
def temp_dir():
    with tempfile.TemporaryDirectory() as tmp:
        yield Path(tmp)


@pytest.fixture
def mock_git(monkeypatch):
    git = GitService()
    # Mock installed
    monkeypatch.setattr(git, "is_git_installed", lambda: True)
    return git


def test_rollback_stack_execution():
    """Test RollbackStack records and executes functions in reverse order."""
    stack = RollbackStack()
    executed = []

    stack.add("step 1", lambda: executed.append(1))
    stack.add("step 2", lambda: executed.append(2))
    stack.add("step 3", lambda: executed.append(3))

    res = stack.execute()
    assert res == ["step 3", "step 2", "step 1"]
    assert executed == [3, 2, 1]


def test_validate_creation_existing_workspace(temp_dir, mock_git, monkeypatch):
    """Test validation fails if workspace directory already exists."""
    ws_dir = temp_dir / "workspaces" / "test-ws"
    ws_dir.mkdir(parents=True)

    app_cfg = AppConfig(repositories={}, workspaces_dir=temp_dir / "workspaces")
    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)

    with pytest.raises(WorkspaceExistsException, match="already exists"):
        manager.validate_creation("test-ws", [])


def test_validate_creation_repo_not_found(temp_dir, mock_git):
    """Test validation fails if configured repo path does not exist."""
    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(name="server", bare=temp_dir / "nonexistent.git", checkout="server")
        },
        workspaces_dir=temp_dir / "workspaces",
    )
    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    spec = RepoSpec(name="server", branch="feature/test", create=True, path="server")

    with pytest.raises(RepositoryNotFoundException, match="not found or invalid"):
        manager.validate_creation("test-ws", [spec])


def test_create_workspace_rollback_on_failure(temp_dir, mock_git, monkeypatch):
    """Test automatic rollback restores filesystem when creation fails mid-way."""
    bare1 = temp_dir / "repo1.git"
    bare2 = temp_dir / "repo2.git"
    bare1.mkdir()
    bare2.mkdir()

    app_cfg = AppConfig(
        repositories={
            "repo1": RepoConfig(name="repo1", bare=bare1, checkout="repo1"),
            "repo2": RepoConfig(name="repo2", bare=bare2, checkout="repo2"),
        },
        workspaces_dir=temp_dir / "workspaces",
    )

    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(mock_git, "branch_exists", lambda bare, br: False)

    def mock_create_worktree(bare_path, worktree_path, branch, create_branch):
        if "repo2" in str(bare_path):
            raise Exception("Git worktree error for repo2")
        # For repo1, create actual directory to simulate worktree
        worktree_path.mkdir(parents=True, exist_ok=True)

    monkeypatch.setattr(mock_git, "create_worktree", mock_create_worktree)
    monkeypatch.setattr(mock_git, "remove_worktree", lambda bare, wt, force=True: shutil.rmtree(wt, ignore_errors=True))
    monkeypatch.setattr(mock_git, "delete_branch", lambda bare, br, force=True: None)

    specs = [
        RepoSpec(name="repo1", branch="feature/test", create=True, path="repo1"),
        RepoSpec(name="repo2", branch="feature/test", create=True, path="repo2"),
    ]

    with pytest.raises(RollbackException):
        manager.create_workspace("test-ws", specs)

    # Verify workspace directory was cleaned up during rollback
    ws_dir = temp_dir / "workspaces" / "test-ws"
    assert not ws_dir.exists()


def test_parse_repo_url():
    """Test URL parsing for repository URLs."""
    name, url, bare_path, checkout = WorkspaceManager.parse_repo_url("git@github.com:Renttik/Renttik-server.git")
    assert name == "renttik-server"
    assert url == "git@github.com:Renttik/Renttik-server.git"
    assert str(bare_path) == "bares/Renttik-server.git"
    assert checkout == "Renttik-server"

    # Explicit name=url format
    name2, url2, bare_path2, checkout2 = WorkspaceManager.parse_repo_url("backend=https://github.com/myorg/api-service.git")
    assert name2 == "backend"
    assert url2 == "https://github.com/myorg/api-service.git"
    assert str(bare_path2) == "bares/api-service.git"
    assert checkout2 == "api-service"


def test_init_project(temp_dir, mock_git, monkeypatch):
    """Test init_project clones bare repos and creates configuration."""
    cloned_urls = []

    def mock_clone_bare(url, target_bare_path):
        cloned_urls.append(url)
        target_bare_path.mkdir(parents=True, exist_ok=True)

    monkeypatch.setattr(mock_git, "clone_bare", mock_clone_bare)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: path.exists())

    # Switch working directory to temp_dir
    monkeypatch.chdir(temp_dir)

    app_cfg = AppConfig(repositories={}, workspaces_dir=temp_dir / "workspaces")
    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)

    repo_inputs = [
        "server=git@github.com:Renttik/Renttik-server.git",
        "mobile=git@github.com:Renttik/Renttik-mobile.git",
    ]

    new_cfg = manager.init_project(repo_inputs)

    assert "server" in new_cfg.repositories
    assert "mobile" in new_cfg.repositories
    assert len(cloned_urls) == 2
    assert (temp_dir / "repositories.yml").exists()


def test_workspace_add_and_remove_repo(temp_dir, mock_git, monkeypatch):
    """Test adding and removing a repository from a workspace."""
    bare1 = temp_dir / "repo1.git"
    bare2 = temp_dir / "repo2.git"
    bare1.mkdir()
    bare2.mkdir()

    app_cfg = AppConfig(
        repositories={
            "repo1": RepoConfig(name="repo1", bare=bare1, checkout="repo1"),
            "repo2": RepoConfig(name="repo2", bare=bare2, checkout="repo2"),
        },
        workspaces_dir=temp_dir / "workspaces",
    )

    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(mock_git, "branch_exists", lambda bare, br: False)
    monkeypatch.setattr(mock_git, "create_worktree", lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True))
    monkeypatch.setattr(mock_git, "remove_worktree", lambda bare, wt, force=True: shutil.rmtree(wt, ignore_errors=True))
    monkeypatch.setattr(mock_git, "delete_branch", lambda bare, br, force=True: None)

    # 1. Create workspace with repo1 only
    spec1 = RepoSpec(name="repo1", branch="feature/test", create=True, path="repo1")
    manager.create_workspace("my-ws", [spec1])

    meta, ws_path = manager.get_workspace_info("my-ws")
    assert "repo1" in meta.repositories
    assert "repo2" not in meta.repositories

    # 2. Add repo2 to workspace
    manager.workspace_add_repo("my-ws", "repo2", branch="feature/test", create=True)
    meta, _ = manager.get_workspace_info("my-ws")
    assert "repo2" in meta.repositories
    assert (ws_path / "repo2").exists()

    # 3. Remove repo2 from workspace
    manager.workspace_remove_repo("my-ws", "repo2", delete_branch=False)
    meta, _ = manager.get_workspace_info("my-ws")
    assert "repo2" not in meta.repositories


def test_freeze_and_unfreeze_repo(temp_dir, mock_git, monkeypatch):
    """Test freezing and unfreezing a repository in a workspace."""
    bare1 = temp_dir / "repo1.git"
    bare1.mkdir()

    app_cfg = AppConfig(
        repositories={"repo1": RepoConfig(name="repo1", bare=bare1, checkout="repo1")},
        workspaces_dir=temp_dir / "workspaces",
    )

    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(mock_git, "branch_exists", lambda bare, br: False)
    monkeypatch.setattr(mock_git, "create_worktree", lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True))
    monkeypatch.setattr(mock_git, "set_tracked_files_readonly", lambda wt_path, readonly: None)

    spec1 = RepoSpec(name="repo1", branch="feature/test", create=True, path="repo1")
    manager.create_workspace("freeze-ws", [spec1])

    # Freeze / Lock
    manager.lock_repo("freeze-ws", "repo1")
    meta, _ = manager.get_workspace_info("freeze-ws")
    assert meta.repositories["repo1"].frozen is True
    assert meta.repositories["repo1"].locked is True

    # Unfreeze / Unlock
    manager.unlock_repo("freeze-ws", "repo1")
    meta, _ = manager.get_workspace_info("freeze-ws")
    assert meta.repositories["repo1"].frozen is False
    assert meta.repositories["repo1"].locked is False



def test_push_workspace_skips_frozen(temp_dir, mock_git, monkeypatch):
    """Test push_workspace pushes active repos and skips frozen repos."""
    bare1 = temp_dir / "repo1.git"
    bare2 = temp_dir / "repo2.git"
    bare1.mkdir()
    bare2.mkdir()

    app_cfg = AppConfig(
        repositories={
            "repo1": RepoConfig(name="repo1", bare=bare1, checkout="repo1"),
            "repo2": RepoConfig(name="repo2", bare=bare2, checkout="repo2"),
        },
        workspaces_dir=temp_dir / "workspaces",
    )

    pushed_repos = []

    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(mock_git, "branch_exists", lambda bare, br: False)
    monkeypatch.setattr(mock_git, "create_worktree", lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True))
    monkeypatch.setattr(mock_git, "set_tracked_files_readonly", lambda wt_path, readonly: None)
    def mock_push_branch(worktree_path, remote="origin", branch=None):
        pushed_repos.append(worktree_path.name)
        return True, "successfully pushed committed changes"

    monkeypatch.setattr(mock_git, "push_branch", mock_push_branch)

    specs = [
        RepoSpec(name="repo1", branch="feature/test", create=True, path="repo1"),
        RepoSpec(name="repo2", branch="feature/test", create=True, path="repo2"),
    ]
    manager.create_workspace("push-ws", specs)
    manager.freeze_repo("push-ws", "repo2")

    results = manager.push_workspace("push-ws")

    assert results["repo1"]["status"] == "pushed"
    assert results["repo2"]["status"] == "skipped"
    assert results["repo2"]["reason"] == "frozen repository (read-only)"
    assert pushed_repos == ["repo1"]


def test_pull_workspace(temp_dir, mock_git, monkeypatch):
    """Test pull_workspace pulls updates and handles errors/frozen repos."""
    bare1 = temp_dir / "repo1.git"
    bare2 = temp_dir / "repo2.git"
    bare1.mkdir()
    bare2.mkdir()

    app_cfg = AppConfig(
        repositories={
            "repo1": RepoConfig(name="repo1", bare=bare1, checkout="repo1"),
            "repo2": RepoConfig(name="repo2", bare=bare2, checkout="repo2"),
        },
        workspaces_dir=temp_dir / "workspaces",
    )

    pulled_repos = []

    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(mock_git, "branch_exists", lambda bare, br: False)
    monkeypatch.setattr(mock_git, "create_worktree", lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True))
    monkeypatch.setattr(mock_git, "set_tracked_files_readonly", lambda wt_path, readonly: None)

    def mock_pull_branch(worktree_path, remote="origin", branch=None):
        pulled_repos.append(worktree_path.name)
        return True, "successfully pulled updates"

    monkeypatch.setattr(mock_git, "pull_branch", mock_pull_branch)

    specs = [
        RepoSpec(name="repo1", branch="feature/test", create=True, path="repo1"),
        RepoSpec(name="repo2", branch="feature/test", create=True, path="repo2"),
    ]
    manager.create_workspace("pull-ws", specs)
    manager.freeze_repo("pull-ws", "repo2")

    results = manager.pull_workspace("pull-ws")

    assert results["repo1"]["status"] == "pulled"
    assert results["repo2"]["status"] == "skipped"
    assert results["repo2"]["reason"] == "frozen repository (read-only)"
    assert pulled_repos == ["repo1"]


def test_find_config_file_upward_traversal(temp_dir, monkeypatch):
    """Test ConfigLoader upward traversal to locate repositories.yml from subdirectory."""
    from ws.config import ConfigLoader

    project_root = temp_dir / "my_project"
    sub_dir = project_root / "workspaces" / "feature" / "deep_dir"
    sub_dir.mkdir(parents=True)

    config_file = project_root / "repositories.yml"
    config_file.write_text("repositories:\n  server:\n    bare: bares/server.git\n    checkout: server\n")

    # Change working directory to deep subdirectory
    monkeypatch.chdir(sub_dir)

    found = ConfigLoader.find_config_file()
    assert found is not None
    assert found.resolve() == config_file.resolve()

    app_cfg = ConfigLoader.load_config()
    assert app_cfg.project_root == project_root.resolve()
    assert app_cfg.workspaces_dir == project_root.resolve() / "workspaces"


def test_session_socket_and_stop_workspace(temp_dir, mock_git, monkeypatch):
    """Test session socket path resolution and stop_workspace."""
    bare1 = temp_dir / "repo1.git"
    bare1.mkdir()

    app_cfg = AppConfig(
        repositories={"repo1": RepoConfig(name="repo1", bare=bare1, checkout="repo1")},
        workspaces_dir=temp_dir / "workspaces",
    )

    manager = WorkspaceManager(config=app_cfg, git_service=mock_git)
    monkeypatch.setattr(mock_git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(mock_git, "branch_exists", lambda bare, br: False)
    monkeypatch.setattr(mock_git, "create_worktree", lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True))

    spec1 = RepoSpec(name="repo1", branch="feature/test", create=True, path="repo1")
    manager.create_workspace("session-ws", [spec1])

    sock_path = manager.get_session_socket_path("session-ws")
    assert sock_path == temp_dir / "workspaces" / "session-ws" / ".ws" / "session.sock"

    assert manager.is_session_running("session-ws") is False
    assert manager.stop_workspace("session-ws") is False






