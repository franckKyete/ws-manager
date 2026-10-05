import json
import os
import signal
import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch
import pytest

from ws.config import ConfigLoader
from ws.models import AppConfig, HubAutoSaveConfig, RepoConfig, RepoSpec, WorkspaceMetadata
from ws.utils import parse_duration
from ws.workspace import WorkspaceManager


def test_parse_duration():
    assert parse_duration("15m") == 900
    assert parse_duration("5m") == 300
    assert parse_duration("1h") == 3600
    assert parse_duration("30s") == 30
    assert parse_duration("2d") == 172800
    assert parse_duration(600) == 600
    assert parse_duration("600") == 600
    assert parse_duration("0") == 0
    assert parse_duration("never") == 0
    assert parse_duration("none") == 0
    assert parse_duration("false") == 0
    assert parse_duration(None) == 0


def test_config_loader_hub_auto_save(tmp_path):
    config_file = tmp_path / "repositories.yml"
    config_file.write_text(
        """
hub:
  project: "my-org/my-proj"
  auto_save:
    enabled: true
    interval: "10m"
    include_wip: true
    workspaces: "all"
repositories:
  server:
    bare: bares/server.git
    checkout: server
""",
        encoding="utf-8",
    )
    (tmp_path / "bares" / "server.git").mkdir(parents=True)

    config = ConfigLoader.load_config(config_path=config_file)
    assert config.hub_project == "my-org/my-proj"
    assert config.hub_auto_save is not None
    assert config.hub_auto_save.enabled is True
    assert config.hub_auto_save.interval == 600
    assert config.hub_auto_save.include_wip is True
    assert config.hub_auto_save.workspaces == "all"


def test_config_loader_hub_auto_save_disabled(tmp_path):
    config_file = tmp_path / "repositories.yml"
    config_file.write_text(
        """
hub:
  auto_save:
    enabled: false
    interval: "never"
repositories:
  server:
    bare: bares/server.git
    checkout: server
""",
        encoding="utf-8",
    )
    (tmp_path / "bares" / "server.git").mkdir(parents=True)

    config = ConfigLoader.load_config(config_path=config_file)
    assert config.hub_auto_save is not None
    assert config.hub_auto_save.enabled is False


@pytest.fixture
def auto_save_env(tmp_path):
    workspaces = tmp_path / "workspaces"
    workspaces.mkdir()
    bares = tmp_path / "bares"
    bares.mkdir()
    server_bare = bares / "server.git"
    server_bare.mkdir()

    config = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        workspaces_dir=workspaces,
        repositories={
            "server": RepoConfig(name="server", bare=server_bare, checkout="server"),
        },
        hub_auto_save=HubAutoSaveConfig(enabled=True, interval=300, include_wip=True, workspaces="all"),
        hub_project="test-org/test-proj",
    )

    manager = WorkspaceManager(config=config)

    # Create workspace @dev
    ws_dir = workspaces / "dev"
    ws_dir.mkdir()
    server_wt = ws_dir / "server"
    server_wt.mkdir()
    subprocess.run(["git", "init", "-b", "feat/auth"], cwd=server_wt, check=True, capture_output=True)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=server_wt, check=True, capture_output=True)
    subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=server_wt, check=True, capture_output=True)
    (server_wt / "init.txt").write_text("initial", encoding="utf-8")
    subprocess.run(["git", "add", "."], cwd=server_wt, check=True, capture_output=True)
    subprocess.run(["git", "commit", "-m", "Initial commit"], cwd=server_wt, check=True, capture_output=True)

    meta = WorkspaceMetadata(
        name="dev",
        created="2026-10-05T00:00:00Z",
        status="active",
        repositories={
            "server": RepoSpec(name="server", branch="feat/auth", create=False, path="server"),
        },
    )
    manager._save_metadata(ws_dir, meta)

    return {
        "manager": manager,
        "ws_dir": ws_dir,
        "server_wt": server_wt,
        "meta": meta,
        "config": config,
    }


def test_workspace_fingerprint_changes_on_modification(auto_save_env):
    manager = auto_save_env["manager"]
    server_wt = auto_save_env["server_wt"]

    fp1 = manager.get_workspace_fingerprint("dev")

    # Add an untracked file
    test_file = server_wt / "hello.txt"
    test_file.write_text("hello world", encoding="utf-8")

    fp2 = manager.get_workspace_fingerprint("dev")
    assert fp1 != fp2

    # Fingerprint stays the same if nothing changes
    fp3 = manager.get_workspace_fingerprint("dev")
    assert fp2 == fp3


def test_hub_auto_save_workspace_skips_when_unchanged(auto_save_env):
    manager = auto_save_env["manager"]

    with patch.object(manager, "hub_state_save", return_value={"status": "success"}) as mock_save:
        # First save: should execute because no prior fingerprint
        saved1 = manager.hub_auto_save_workspace("dev")
        assert saved1 is True
        assert mock_save.call_count == 1

        # Second save: should skip because unchanged
        saved2 = manager.hub_auto_save_workspace("dev")
        assert saved2 is False
        assert mock_save.call_count == 1

        # Force save: should execute even if unchanged
        saved3 = manager.hub_auto_save_workspace("dev", force=True)
        assert saved3 is True
        assert mock_save.call_count == 2


def test_hub_auto_save_all_workspaces(auto_save_env):
    manager = auto_save_env["manager"]

    with patch.object(manager, "hub_auto_save_workspace", return_value=True) as mock_auto_save:
        results = manager.hub_auto_save_all_workspaces()
        assert results == {"dev": True}
        assert mock_auto_save.call_count == 1


def test_auto_save_daemon_lifecycle(auto_save_env, tmp_path):
    manager = auto_save_env["manager"]

    active, pid = manager.is_auto_save_daemon_active()
    assert active is False
    assert pid is None

    # Simulate running daemon by creating PID file
    pid_file = manager.get_auto_save_pid_file()
    current_pid = os.getpid()
    pid_file.write_text(str(current_pid), encoding="utf-8")

    active, pid = manager.is_auto_save_daemon_active()
    assert active is True
    assert pid == current_pid

    # Clean up PID file
    pid_file.unlink()
    active, pid = manager.is_auto_save_daemon_active()
    assert active is False


def test_get_auto_save_status(auto_save_env):
    manager = auto_save_env["manager"]
    status = manager.get_auto_save_status()

    assert status["enabled"] is True
    assert status["interval"] == 300
    assert status["include_wip"] is True
    assert "dev" in status["workspaces"]
    assert status["workspaces"]["dev"]["last_saved_at"] is None
