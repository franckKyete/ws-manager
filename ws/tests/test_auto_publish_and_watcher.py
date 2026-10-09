"""Unit tests for automatic hub publishing on 404 and ConfigFileWatcher."""

from pathlib import Path
from unittest.mock import MagicMock, patch
import pytest

from ws.config import ConfigLoader
from ws.hub import HubException
from ws.models import WorkspaceMetadata
from ws.registry import register_project
from ws.watcher import ConfigFileWatcher
from ws.workspace import WorkspaceManager


def test_hub_state_save_auto_publishes_on_404(tmp_path):
    """Test that hub_state_save automatically publishes the project if 404 is returned."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text(
        """
hub:
  project: "testorg/testproj"
repositories:
  server:
    bare: bares/server.git
    checkout: server
""",
        encoding="utf-8",
    )
    (tmp_path / "bares" / "server.git").mkdir(parents=True)
    ws_dir = tmp_path / "workspaces" / "dev"
    ws_dir.mkdir(parents=True)

    (ws_dir / "workspace.yml").write_text(
        """
name: dev
status: active
repositories: {}
""",
        encoding="utf-8",
    )

    config = ConfigLoader.load_config(config_path=cfg_file)
    manager = WorkspaceManager(config=config)

    mock_client = MagicMock()
    mock_client.parse_project_identifier.return_value = ("testorg", "testproj")
    # First save fails with 404
    save_call_count = 0

    def mock_save(*args, **kwargs):
        nonlocal save_call_count
        save_call_count += 1
        if save_call_count == 1:
            raise HubException("Project 'testorg/testproj' not found.", status_code=404)
        return {"status": "ok"}

    mock_client.save_workspace_state.side_effect = mock_save
    mock_client.create_project.return_value = {"project": {"id": "p123"}}
    mock_client.whoami.return_value = {"username": "testorg"}

    with patch("ws.hub.HubClient", return_value=mock_client):
        res = manager.hub_state_save("dev", silent=True)

    assert mock_client.create_project.called
    assert mock_client.save_workspace_state.call_count == 2
    assert res == {"status": "ok"}


def test_hub_push_auto_publishes_on_404(tmp_path):
    """Test that hub_push automatically publishes the project if 404 is returned."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text(
        """
hub:
  project: "testorg/testproj"
repositories:
  server:
    bare: bares/server.git
    checkout: server
""",
        encoding="utf-8",
    )
    (tmp_path / "bares" / "server.git").mkdir(parents=True)

    config = ConfigLoader.load_config(config_path=cfg_file)
    manager = WorkspaceManager(config=config)

    mock_client = MagicMock()
    mock_client.parse_project_identifier.return_value = ("testorg", "testproj")
    mock_client.push_revision.side_effect = HubException("Project not found", status_code=404)
    mock_client.create_project.return_value = {"project": {"id": "p123"}, "revision": {"version": "1"}}
    mock_client.whoami.return_value = {"username": "testorg"}

    with patch("ws.hub.HubClient", return_value=mock_client):
        res = manager.hub_push(silent=True)

    assert mock_client.create_project.called
    assert res.get("project", {}).get("id") == "p123"


def test_watcher_detects_repositories_yml_edit(tmp_path):
    """Test that ConfigFileWatcher detects edits to repositories.yml and pushes revision."""
    reg_file = tmp_path / "projects.yml"
    proj_dir = tmp_path / "proj1"
    proj_dir.mkdir()
    (proj_dir / "workspaces").mkdir()
    (proj_dir / "bares").mkdir()

    cfg_file = proj_dir / "repositories.yml"
    cfg_file.write_text(
        """repositories:
  server:
    bare: bares/server.git
    checkout: server
""",
        encoding="utf-8",
    )

    register_project(proj_dir, registry_path=reg_file)

    watcher = ConfigFileWatcher(registry_path=reg_file)
    # Baseline initialization
    res_init = watcher.check_changes()
    assert res_init == {"pushed": [], "saved": []}

    # Edit repositories.yml
    cfg_file.write_text(
        """repositories:
  server:
    bare: bares/server.git
    checkout: server
    command: npm run dev
""",
        encoding="utf-8",
    )

    with patch("ws.workspace.WorkspaceManager.hub_push", return_value={"revision": {"version": 2}}) as mock_push:
        with patch("ws.notify.send_dbus_notification", return_value=True):
            res = watcher.check_changes()

    assert mock_push.called
    assert "proj1" in res["pushed"]


def test_watcher_detects_workspace_yml_edit(tmp_path):
    """Test that ConfigFileWatcher detects edits to workspace.yml and saves workspace state."""
    reg_file = tmp_path / "projects.yml"
    proj_dir = tmp_path / "proj1"
    proj_dir.mkdir()
    ws_dir = proj_dir / "workspaces" / "feat-auth"
    ws_dir.mkdir(parents=True)
    (proj_dir / "bares").mkdir()

    cfg_file = proj_dir / "repositories.yml"
    cfg_file.write_text(
        """repositories:
  server:
    bare: bares/server.git
    checkout: server
""",
        encoding="utf-8",
    )

    ws_file = ws_dir / "workspace.yml"
    ws_file.write_text(
        """name: feat-auth
status: active
repositories:
  server:
    branch: feature/auth
    path: server
""",
        encoding="utf-8",
    )

    register_project(proj_dir, registry_path=reg_file)

    watcher = ConfigFileWatcher(registry_path=reg_file)
    watcher.initialize()

    # Edit workspace.yml
    ws_file.write_text(
        """name: feat-auth
status: active
repositories:
  server:
    branch: feature/auth-v2
    path: server
""",
        encoding="utf-8",
    )

    with patch("ws.workspace.WorkspaceManager.hub_state_save", return_value={"ok": True}) as mock_save:
        with patch("ws.workspace.WorkspaceManager.has_workspace", return_value=True):
            with patch("ws.notify.send_dbus_notification", return_value=True):
                res = watcher.check_changes()

    assert mock_save.called
    assert mock_save.call_args.kwargs["workspace_name"] == "feat-auth"
    assert "proj1@feat-auth" in res["saved"]


def test_watcher_ignores_invalid_yaml(tmp_path):
    """Test that ConfigFileWatcher does not push or crash when YAML is invalid."""
    reg_file = tmp_path / "projects.yml"
    proj_dir = tmp_path / "proj1"
    proj_dir.mkdir()
    (proj_dir / "workspaces").mkdir()

    cfg_file = proj_dir / "repositories.yml"
    cfg_file.write_text("repositories: {}\n", encoding="utf-8")

    register_project(proj_dir, registry_path=reg_file)

    watcher = ConfigFileWatcher(registry_path=reg_file)
    watcher.initialize()

    # Corrupt repositories.yml with invalid syntax
    cfg_file.write_text("repositories: [invalid syntax { unbalanced", encoding="utf-8")

    with patch("ws.workspace.WorkspaceManager.hub_push") as mock_push:
        res = watcher.check_changes()

    assert not mock_push.called
    assert res == {"pushed": [], "saved": []}
