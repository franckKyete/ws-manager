"""Tests for global multi-project registry, systemd service, and daemon."""

from pathlib import Path
import subprocess
from unittest.mock import MagicMock, patch
import pytest

from ws.config import ConfigLoader
from ws.daemon import GlobalAutoSaveWorker, is_standalone_daemon_running, start_standalone_daemon
from ws.models import AppConfig, HubAutoSaveConfig, RepoConfig
from ws.registry import (
    get_registry_path,
    list_registered_projects,
    load_registry,
    register_project,
    save_registry,
    unregister_project,
)
from ws.systemd import (
    SERVICE_NAME,
    control_service,
    generate_service_unit,
    get_service_status,
    install_service,
    uninstall_service,
)


# ==================== Registry Tests ====================

def test_registry_lifecycle(tmp_path):
    reg_file = tmp_path / "projects.yml"

    proj1 = tmp_path / "proj1"
    proj1.mkdir()
    proj2 = tmp_path / "proj2"
    proj2.mkdir()

    # Register proj1
    assert register_project(proj1, registry_path=reg_file) is True
    # Duplicate registration returns False
    assert register_project(proj1, registry_path=reg_file) is False

    # Register proj2
    assert register_project(proj2, registry_path=reg_file) is True

    # List registered projects
    paths = list_registered_projects(registry_path=reg_file)
    assert len(paths) == 2
    assert proj1.resolve() in paths
    assert proj2.resolve() in paths

    # Unregister proj1
    assert unregister_project(proj1, registry_path=reg_file) is True
    # Unregistering again returns False
    assert unregister_project(proj1, registry_path=reg_file) is False

    paths_after = list_registered_projects(registry_path=reg_file)
    assert len(paths_after) == 1
    assert proj2.resolve() in paths_after


def test_registry_prunes_missing_directories(tmp_path):
    reg_file = tmp_path / "projects.yml"
    proj1 = tmp_path / "proj1"
    proj1.mkdir()
    proj_deleted = tmp_path / "deleted"
    proj_deleted.mkdir()

    register_project(proj1, registry_path=reg_file)
    register_project(proj_deleted, registry_path=reg_file)

    assert len(list_registered_projects(registry_path=reg_file, prune_missing=False)) == 2

    # Remove deleted project dir from disk
    proj_deleted.rmdir()

    # Prune missing
    valid_paths = list_registered_projects(registry_path=reg_file, prune_missing=True)
    assert len(valid_paths) == 1
    assert valid_paths[0] == proj1.resolve()

    # Verify persisted registry was updated
    records = load_registry(registry_path=reg_file)
    assert len(records) == 1
    assert records[0]["path"] == str(proj1.resolve())


def test_config_loader_auto_registers_project(tmp_path, monkeypatch):
    reg_file = tmp_path / "projects.yml"
    monkeypatch.setattr("ws.registry.DEFAULT_REGISTRY_PATH", reg_file)

    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("repositories: {}\n", encoding="utf-8")

    ConfigLoader.load_config(config_path=cfg_file, allow_empty=True)

    paths = list_registered_projects(registry_path=reg_file)
    assert tmp_path.resolve() in paths


# ==================== Systemd Service Tests ====================

def test_generate_service_unit():
    unit = generate_service_unit(ws_exec="/usr/local/bin/ws")
    assert "Description=ws Global Background Daemon" in unit
    assert "ExecStart=/usr/local/bin/ws daemon" in unit
    assert "WantedBy=default.target" in unit


def test_systemd_install_and_uninstall(tmp_path, monkeypatch):
    service_dir = tmp_path / "systemd" / "user"
    service_file = service_dir / SERVICE_NAME
    monkeypatch.setattr("ws.systemd.USER_SYSTEMD_DIR", service_dir)
    monkeypatch.setattr("ws.systemd.SERVICE_PATH", service_file)
    monkeypatch.setattr("ws.systemd.is_systemctl_available", lambda: True)

    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0, stdout="", stderr="")

        # Install
        success, msg = install_service(ws_exec="/custom/bin/ws")
        assert success is True
        assert service_file.exists()
        assert "/custom/bin/ws daemon" in service_file.read_text(encoding="utf-8")

        # Status
        status = get_service_status()
        assert status["installed"] is True

        # Uninstall
        success_un, msg_un = uninstall_service()
        assert success_un is True
        assert not service_file.exists()


def test_systemd_control_service(tmp_path, monkeypatch):
    service_dir = tmp_path / "systemd" / "user"
    service_file = service_dir / SERVICE_NAME
    service_dir.mkdir(parents=True)
    service_file.write_text("dummy", encoding="utf-8")

    monkeypatch.setattr("ws.systemd.SERVICE_PATH", service_file)
    monkeypatch.setattr("ws.systemd.is_systemctl_available", lambda: True)

    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0, stdout="OK", stderr="")
        success, msg = control_service("restart")
        assert success is True
        mock_run.assert_called_with(["systemctl", "--user", "restart", SERVICE_NAME], check=False, capture_output=True, text=True)


# ==================== Global Daemon Worker Tests ====================

def test_global_auto_save_worker(tmp_path, monkeypatch):
    reg_file = tmp_path / "projects.yml"
    cache_file = tmp_path / "global_cache.json"

    monkeypatch.setattr("ws.registry.DEFAULT_REGISTRY_PATH", reg_file)
    monkeypatch.setattr("ws.daemon.GLOBAL_CACHE_FILE", cache_file)

    # Setup project 1 (auto-save enabled, interval 300)
    p1 = tmp_path / "proj1"
    p1.mkdir()
    (p1 / "workspaces").mkdir()
    (p1 / "bares").mkdir()
    (p1 / "repositories.yml").write_text("""
hub:
  auto_save:
    enabled: true
    interval: 5m
repositories: {}
""", encoding="utf-8")

    # Setup project 2 (auto-save disabled)
    p2 = tmp_path / "proj2"
    p2.mkdir()
    (p2 / "repositories.yml").write_text("""
hub:
  auto_save:
    enabled: false
repositories: {}
""", encoding="utf-8")

    register_project(p1, registry_path=reg_file)
    register_project(p2, registry_path=reg_file)

    worker = GlobalAutoSaveWorker()

    with patch("ws.workspace.WorkspaceManager.hub_auto_save_all_workspaces", return_value={"dev": True}) as mock_save:
        results = worker.process_all_projects()

        # Proj1 should be saved, Proj2 should be skipped
        assert "proj1" in results
        assert results["proj1"] == {"dev": True}
        assert "proj2" not in results
        assert mock_save.call_count == 1

        # Second immediate pass: interval (300s) has not elapsed, should skip!
        results2 = worker.process_all_projects()
        assert "proj1" not in results2
        assert mock_save.call_count == 1
