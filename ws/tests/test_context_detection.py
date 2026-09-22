"""Tests for workspace and repository context detection and non-root execution."""

import os
from pathlib import Path
from unittest.mock import MagicMock, patch
import pytest

from ws.cli import build_parser, main, resolve_ws_and_repo_args
from ws.config import AppConfig
from ws.exceptions import WSException
from ws.git import GitService
from ws.models import RepoConfig, RepoSpec, WorkspaceMetadata
from ws.workspace import WorkspaceManager


@pytest.fixture
def temp_project(tmp_path: Path):
    """Set up a mock project directory structure with bare repos and workspaces."""
    project_root = tmp_path / "project"
    project_root.mkdir()
    bares_dir = project_root / "bares"
    bares_dir.mkdir()
    workspaces_dir = project_root / "workspaces"
    workspaces_dir.mkdir()

    # Create mock bare repos
    server_bare = bares_dir / "server.git"
    server_bare.mkdir()
    (server_bare / "HEAD").write_text("ref: refs/heads/main\n", encoding="utf-8")

    web_bare = bares_dir / "web.git"
    web_bare.mkdir()
    (web_bare / "HEAD").write_text("ref: refs/heads/main\n", encoding="utf-8")

    # Write repositories.yml
    config_file = project_root / "repositories.yml"
    config_file.write_text(
        "repositories:\n"
        "  server:\n"
        "    bare: bares/server.git\n"
        "    checkout: server-app\n"
        "  web:\n"
        "    bare: bares/web.git\n"
        "    checkout: web-app\n",
        encoding="utf-8",
    )

    app_config = AppConfig(
        repositories={
            "server": RepoConfig(name="server", bare=Path("bares/server.git"), checkout="server-app"),
            "web": RepoConfig(name="web", bare=Path("bares/web.git"), checkout="web-app"),
        },
        workspaces_dir=workspaces_dir,
        config_file_path=config_file,
    )

    # Create a sample workspace "feature-auth"
    ws_dir = workspaces_dir / "feature-auth"
    ws_dir.mkdir()
    (ws_dir / "workspace.yml").write_text(
        "name: feature-auth\n"
        "repositories:\n"
        "  server:\n"
        "    path: server-app\n"
        "    branch: feature/auth\n"
        "  web:\n"
        "    path: web-app\n"
        "    branch: feature/auth\n",
        encoding="utf-8",
    )
    server_wt = ws_dir / "server-app"
    server_wt.mkdir()
    (server_wt / "src").mkdir(parents=True)
    (server_wt / "src" / "index.ts").write_text("// server", encoding="utf-8")

    web_wt = ws_dir / "web-app"
    web_wt.mkdir()

    return {
        "project_root": project_root,
        "workspaces_dir": workspaces_dir,
        "app_config": app_config,
        "ws_dir": ws_dir,
        "server_wt": server_wt,
        "server_sub": server_wt / "src",
        "web_wt": web_wt,
    }


def test_detect_context_at_project_root(temp_project):
    """Test detect_context when at project root returns (None, None)."""
    manager = WorkspaceManager(config=temp_project["app_config"])
    ws, repo = manager.detect_context(cwd=temp_project["project_root"])
    assert ws is None
    assert repo is None


def test_detect_context_at_workspace_root(temp_project):
    """Test detect_context when at workspace root returns (ws_name, None)."""
    manager = WorkspaceManager(config=temp_project["app_config"])
    ws, repo = manager.detect_context(cwd=temp_project["ws_dir"])
    assert ws == "feature-auth"
    assert repo is None


def test_detect_context_in_repo_checkout(temp_project):
    """Test detect_context in repository checkout directory returns (ws_name, repo_name)."""
    manager = WorkspaceManager(config=temp_project["app_config"])
    ws, repo = manager.detect_context(cwd=temp_project["server_wt"])
    assert ws == "feature-auth"
    assert repo == "server"


def test_detect_context_in_nested_subdir(temp_project):
    """Test detect_context in nested subdirectory inside repo returns (ws_name, repo_name)."""
    manager = WorkspaceManager(config=temp_project["app_config"])
    ws, repo = manager.detect_context(cwd=temp_project["server_sub"])
    assert ws == "feature-auth"
    assert repo == "server"


def test_detect_context_fallback_upward_search(tmp_path, temp_project):
    """Test detect_context fallback walking upward for workspace.yml in non-standard location."""
    custom_ws = tmp_path / "custom-ws"
    custom_ws.mkdir()
    (custom_ws / "workspace.yml").write_text("name: custom-ws\n", encoding="utf-8")
    server_dir = custom_ws / "server-app"
    server_dir.mkdir()
    deep_dir = server_dir / "deep" / "path"
    deep_dir.mkdir(parents=True)

    manager = WorkspaceManager(config=temp_project["app_config"])
    ws, repo = manager.detect_context(cwd=deep_dir)
    assert ws == "custom-ws"
    assert repo == "server"


def test_resolve_bare_path_relative_vs_absolute(temp_project):
    """Test _resolve_bare_path resolves relative bare paths against project_root."""
    manager = WorkspaceManager(config=temp_project["app_config"])
    rel_path = Path("bares/server.git")
    resolved = manager._resolve_bare_path(rel_path)
    assert resolved == (temp_project["project_root"] / "bares" / "server.git").resolve()

    abs_path = temp_project["project_root"] / "bares" / "server.git"
    assert manager._resolve_bare_path(abs_path) == abs_path.resolve()


def test_resolve_ws_and_repo_args_explicit(temp_project):
    """Test explicit arguments take precedence."""
    manager = WorkspaceManager(config=temp_project["app_config"])
    # Explicit workspace and repo
    ws, repo, repos = resolve_ws_and_repo_args(
        manager=manager,
        name_arg="@my-ws",
        repo_arg="%server",
    )
    assert ws == "my-ws"
    assert repo == "server"
    assert repos == []


def test_resolve_ws_and_repo_args_default_from_cwd(temp_project, monkeypatch):
    """Test defaulting workspace and repo from current directory."""
    monkeypatch.chdir(temp_project["server_wt"])
    manager = WorkspaceManager(config=temp_project["app_config"])

    ws, repo, _ = resolve_ws_and_repo_args(manager=manager, name_arg=None, repo_arg=None)
    assert ws == "feature-auth"
    assert repo == "server"


def test_resolve_ws_and_repo_args_smart_repo_arg(temp_project, monkeypatch):
    """Test passing 'ws logs %server' inside workspace correctly infers workspace."""
    monkeypatch.chdir(temp_project["ws_dir"])
    manager = WorkspaceManager(config=temp_project["app_config"])

    # First arg is %server (starts with %)
    ws, repo, repos = resolve_ws_and_repo_args(
        manager=manager,
        name_arg="%server",
        repo_arg=None,
    )
    assert ws == "feature-auth"
    assert repo == "server"
    assert repos == ["server"]


def test_resolve_ws_and_repo_args_smart_start_multiple_repos(temp_project, monkeypatch):
    """Test 'ws start %server %web' inside workspace."""
    monkeypatch.chdir(temp_project["ws_dir"])
    manager = WorkspaceManager(config=temp_project["app_config"])

    ws, repo, repos = resolve_ws_and_repo_args(
        manager=manager,
        name_arg="%server",
        repos_arg=["%web"],
    )
    assert ws == "feature-auth"
    assert repo == "server"
    assert repos == ["server", "web"]


def test_resolve_ws_and_repo_args_outside_workspace_raises(temp_project, monkeypatch):
    """Test omitting workspace outside workspace raises helpful exception."""
    monkeypatch.chdir(temp_project["project_root"])
    manager = WorkspaceManager(config=temp_project["app_config"])

    with pytest.raises(WSException, match="Workspace name.*required"):
        resolve_ws_and_repo_args(manager=manager, name_arg=None, require_ws=True)


def test_non_root_execution_validate_repo_config(temp_project, monkeypatch):
    """Test validate_repository_config succeeds when cwd is inside a workspace worktree."""
    monkeypatch.chdir(temp_project["server_sub"])
    mock_git = MagicMock(spec=GitService)
    mock_git.is_git_installed.return_value = True
    mock_git.is_bare_repo.return_value = True

    manager = WorkspaceManager(config=temp_project["app_config"], git_service=mock_git)
    cfg = manager.validate_repository_config("server")
    expected_bare = (temp_project["project_root"] / "bares" / "server.git").resolve()
    assert cfg.bare == expected_bare


def test_cli_info_defaults_to_detected_ws(temp_project, monkeypatch):
    """Test 'ws info' with no args inside workspace defaults to that workspace."""
    monkeypatch.chdir(temp_project["server_wt"])
    mock_info = MagicMock()

    with patch("ws.cli.cmd_info", mock_info), \
         patch("ws.cli.ConfigLoader.load_config", return_value=temp_project["app_config"]):
        exit_code = main(["info"])
        assert exit_code == 0
        mock_info.assert_called_once()
        assert mock_info.call_args[1]["name"] == "feature-auth"


def test_cli_status_defaults_to_detected_ws(temp_project, monkeypatch):
    """Test 'ws status' with no args inside workspace defaults to that workspace."""
    monkeypatch.chdir(temp_project["server_wt"])
    mock_status = MagicMock()

    with patch("ws.cli.cmd_status", mock_status), \
         patch("ws.cli.ConfigLoader.load_config", return_value=temp_project["app_config"]):
        exit_code = main(["status"])
        assert exit_code == 0
        mock_status.assert_called_once()
        assert mock_status.call_args[1]["name"] == "feature-auth"


def test_cli_logs_with_sigil_inside_workspace(temp_project, monkeypatch):
    """Test 'ws logs %server' inside workspace defaults workspace and sets repo."""
    monkeypatch.chdir(temp_project["ws_dir"])
    mock_logs = MagicMock()

    with patch("ws.cli.cmd_logs", mock_logs), \
         patch("ws.cli.ConfigLoader.load_config", return_value=temp_project["app_config"]):
        exit_code = main(["logs", "%server"])
        assert exit_code == 0
        mock_logs.assert_called_once()
        assert mock_logs.call_args[1]["workspace_name"] == "feature-auth"
        assert mock_logs.call_args[1]["repo_name"] == "server"


def test_cli_exec_smart_command_inside_workspace(temp_project, monkeypatch):
    """Test 'ws exec git status' inside workspace correctly prepends command and uses detected ws."""
    monkeypatch.chdir(temp_project["server_wt"])
    mock_exec = MagicMock()

    with patch("ws.cli.cmd_exec", mock_exec), \
         patch("ws.cli.ConfigLoader.load_config", return_value=temp_project["app_config"]):
        exit_code = main(["exec", "git", "status"])
        assert exit_code == 0
        mock_exec.assert_called_once()
        assert mock_exec.call_args[1]["name"] == "feature-auth"
        assert mock_exec.call_args[1]["command"] == ["git", "status"]


def test_cli_end_defaults_to_detected_ws(temp_project, monkeypatch):
    """Test 'ws end' with no args inside workspace defaults to that workspace."""
    monkeypatch.chdir(temp_project["server_wt"])
    mock_end = MagicMock()

    with patch("ws.cli.cmd_end", mock_end), \
         patch("ws.cli.ConfigLoader.load_config", return_value=temp_project["app_config"]):
        exit_code = main(["end"])
        assert exit_code == 0
        mock_end.assert_called_once()
        assert mock_end.call_args[1]["name"] == "feature-auth"
