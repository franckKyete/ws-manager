"""Tests for automatic uncommitted WIP synchronization via ws hub state save/restore."""

import base64
import os
from pathlib import Path
import subprocess
from unittest.mock import MagicMock, patch
import pytest

from ws.git import GitService
from ws.models import AppConfig, RepoConfig, RepoSpec, WorkspaceMetadata
from ws.workspace import WorkspaceManager


@pytest.fixture
def test_env(tmp_path: Path):
    """Fixture providing a mock workspace environment with a git repo."""
    project_root = tmp_path / "project"
    project_root.mkdir()
    bares_dir = project_root / "bares"
    bares_dir.mkdir()
    workspaces_dir = project_root / "workspaces"
    workspaces_dir.mkdir()

    # Create a real bare repo and initialize it
    server_bare = bares_dir / "server.git"
    subprocess.run(["git", "init", "--bare", str(server_bare)], check=True, capture_output=True)

    # Initial commit in bare repo via temp clone
    init_dir = tmp_path / "init_server"
    subprocess.run(["git", "clone", str(server_bare), str(init_dir)], check=True, capture_output=True)
    subprocess.run(["git", "config", "user.name", "Test User"], cwd=init_dir, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=init_dir, check=True)
    (init_dir / "README.md").write_text("# Server\nInitial server content\n")
    (init_dir / "app.py").write_text("print('hello')\n")
    subprocess.run(["git", "add", "."], cwd=init_dir, check=True)
    subprocess.run(["git", "commit", "-m", "Initial commit"], cwd=init_dir, check=True)
    subprocess.run(["git", "branch", "-M", "main"], cwd=init_dir, check=True)
    subprocess.run(["git", "push", "origin", "main"], cwd=init_dir, check=True)
    subprocess.run(["git", "--git-dir", str(server_bare), "symbolic-ref", "HEAD", "refs/heads/main"], check=True)

    # App configuration
    repo_cfg = RepoConfig(
        name="server",
        bare=server_bare,
        checkout="server",
        url=str(server_bare),
    )
    app_cfg = AppConfig(
        repositories={"server": repo_cfg},
        workspaces_dir=workspaces_dir,
        config_file_path=project_root / "repositories.yml",
    )

    manager = WorkspaceManager(config=app_cfg)
    return {
        "root": project_root,
        "bares_dir": bares_dir,
        "workspaces_dir": workspaces_dir,
        "manager": manager,
        "server_bare": server_bare,
    }


def test_git_service_wip_diff_and_untracked(test_env):
    """Test GitService.get_uncommitted_diff, get_untracked_files, and apply_patch."""
    manager: WorkspaceManager = test_env["manager"]
    git = manager.git

    # Create a workspace with a worktree
    manager.create_workspace(
        name="develop",
        repo_specs=[RepoSpec(name="server", branch="main", create=False, path="server")],
    )
    _, ws_dir = manager.get_workspace_info("develop")
    wt_path = ws_dir / "server"

    # Initially clean
    assert git.get_uncommitted_diff(wt_path) == ""
    assert git.get_untracked_files(wt_path) == []

    # 1. Modify an existing tracked file
    (wt_path / "app.py").write_text("print('hello modified')\n")
    # 2. Add a new untracked file
    (wt_path / "new_feature.py").write_text("def new_feature(): pass\n")

    diff = git.get_uncommitted_diff(wt_path)
    untracked = git.get_untracked_files(wt_path)

    assert "hello modified" in diff
    assert "new_feature.py" in untracked

    # 3. Test applying patch to a clean clone
    clean_clone = test_env["root"] / "clean_server"
    subprocess.run(["git", "clone", str(test_env["server_bare"]), str(clean_clone)], check=True, capture_output=True)
    assert (clean_clone / "app.py").read_text() == "print('hello')\n"

    applied = git.apply_patch(clean_clone, diff)
    assert applied is True
    assert (clean_clone / "app.py").read_text() == "print('hello modified')\n"


def test_hub_state_save_and_restore_wip(test_env):
    """Test end-to-end hub_state_save and hub_state_restore with uncommitted work."""
    manager: WorkspaceManager = test_env["manager"]

    # 1. Create workspace @feature-wip
    manager.create_workspace(
        name="feature-wip",
        repo_specs=[RepoSpec(name="server", branch="main", create=False, path="server")],
    )
    _, ws_dir = manager.get_workspace_info("feature-wip")
    wt_path = ws_dir / "server"

    # 2. Make uncommitted changes
    (wt_path / "app.py").write_text("print('in-progress uncommitted work')\n")
    (wt_path / "scratch.txt").write_text("untracked scratch file content\n")
    subdir = wt_path / "pkg" / "nested"
    subdir.mkdir(parents=True)
    (subdir / "helper.py").write_text("VALUE = 42\n")

    saved_state_capture = {}

    def mock_save_state(namespace, name, workspace_name, state_dict):
        saved_state_capture["data"] = state_dict
        return {"status": "success"}

    def mock_get_state(namespace, name, workspace_name):
        return saved_state_capture["data"]

    with patch("ws.hub.HubClient") as MockHubClient:
        mock_client = MockHubClient.return_value
        mock_client.parse_project_identifier.side_effect = lambda s: tuple(s.split("/", 1)) if "/" in s else ("personal", s)
        mock_client.save_workspace_state.side_effect = mock_save_state
        mock_client.get_workspace_state.side_effect = mock_get_state

        # 3. Save workspace state with WIP
        manager.hub_state_save("feature-wip", project_identifier="test/my-project", include_wip=True)

        assert "wip" in saved_state_capture["data"]
        server_wip = saved_state_capture["data"]["wip"]["server"]
        assert "print('in-progress uncommitted work')" in server_wip["diff"]
        assert "scratch.txt" in server_wip["untracked"]
        assert "pkg/nested/helper.py" in server_wip["untracked"]

        # 4. Delete the local workspace directory (simulating fresh machine)
        import shutil
        shutil.rmtree(ws_dir)
        assert not ws_dir.exists()
        manager.git.prune_worktrees(test_env["server_bare"])

        # 5. Restore workspace state (Machine B simulation)
        manager.hub_state_restore("feature-wip", project_identifier="test/my-project", apply_wip=True)

        _, restored_ws_dir = manager.get_workspace_info("feature-wip")
        assert restored_ws_dir.exists()
        restored_wt = restored_ws_dir / "server"
        assert restored_wt.exists()
        assert (restored_wt / "app.py").read_text() == "print('in-progress uncommitted work')\n"
        assert (restored_wt / "scratch.txt").read_text() == "untracked scratch file content\n"
        assert (restored_wt / "pkg" / "nested" / "helper.py").read_text() == "VALUE = 42\n"


def test_hub_state_save_no_wip_flag(test_env):
    """Test that --no-wip excludes WIP modifications."""
    manager: WorkspaceManager = test_env["manager"]

    manager.create_workspace(
        name="test-nowip",
        repo_specs=[RepoSpec(name="server", branch="main", create=False, path="server")],
    )
    _, ws_dir = manager.get_workspace_info("test-nowip")
    wt_path = ws_dir / "server"

    (wt_path / "app.py").write_text("print('should not be saved')\n")

    saved_state_capture = {}

    def mock_save_state(namespace, name, workspace_name, state_dict):
        saved_state_capture["data"] = state_dict
        return {"status": "success"}

    with patch("ws.hub.HubClient") as MockHubClient:
        mock_client = MockHubClient.return_value
        mock_client.parse_project_identifier.side_effect = lambda s: tuple(s.split("/", 1)) if "/" in s else ("personal", s)
        mock_client.save_workspace_state.side_effect = mock_save_state

        manager.hub_state_save("test-nowip", project_identifier="test/my-project", include_wip=False)

        assert "wip" not in saved_state_capture["data"]
