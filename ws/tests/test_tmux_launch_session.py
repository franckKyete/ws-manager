"""Unit tests for tmux work session and launch session separation and isolation."""

from pathlib import Path
from unittest.mock import MagicMock, patch
import pytest
import yaml

from ws.config import ConfigLoader
from ws.exceptions import ConfigException, WorkspaceNotFoundException
from ws.models import AppConfig, TmuxConfig, WorkspaceMetadata, RepoSpec
from ws.multiplexer import TmuxLauncher
from ws.workspace import WorkspaceManager


def test_tmux_config_launch_session_parsing():
    """Test TmuxConfig parsing launch_session from dict and serialization."""
    # From dict with launch_session
    cfg = TmuxConfig.from_dict({
        "session": "Workspace",
        "launch_session": "running-myproj-1234",
        "command": "nvim",
        "switch": True,
    })
    assert cfg.session == "Workspace"
    assert cfg.launch_session == "running-myproj-1234"
    assert cfg.command == "nvim"
    assert cfg.switch is True
    d = cfg.to_dict()
    assert d["session"] == "Workspace"
    assert d["launch_session"] == "running-myproj-1234"

    # Alias key: launch
    cfg2 = TmuxConfig.from_dict({"session": "Dev", "launch": "running-dev-abcd"})
    assert cfg2.launch_session == "running-dev-abcd"

    # Alias key: launch_name
    cfg3 = TmuxConfig.from_dict({"session": "Dev", "launch_name": "running-dev-9999"})
    assert cfg3.launch_session == "running-dev-9999"

    # String shorthand
    cfg4 = TmuxConfig.from_dict("Workspace")
    assert cfg4.session == "Workspace"
    assert cfg4.launch_session is None


def test_enforce_different_session_names_in_config_loader(tmp_path: Path):
    """Test ConfigLoader raises ConfigException if session == launch_session."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""
tmux:
  session: "Workspace"
  launch_session: "Workspace"
repositories:
  web:
    bare: bares/web.git
    checkout: web
""")
    with pytest.raises(ConfigException) as excinfo:
        ConfigLoader.load_config(cfg_file)
    assert "must have different names to prevent collisions" in str(excinfo.value)


def test_valid_distinct_session_names_in_config_loader(tmp_path: Path):
    """Test ConfigLoader loads successfully when session != launch_session."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""
tmux:
  session: "Workspace"
  launch_session: "running-myproj-a8f2"
repositories:
  web:
    bare: bares/web.git
    checkout: web
""")
    config = ConfigLoader.load_config(cfg_file)
    assert config.tmux is not None
    assert config.tmux.session == "Workspace"
    assert config.tmux.launch_session == "running-myproj-a8f2"


def test_get_launch_session_name(tmp_path: Path):
    """Test get_launch_session_name returns configured name or default fallback."""
    # When configured
    cfg_with_launch = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        repositories={},
        workspaces_dir=tmp_path / "workspaces",
        tmux=TmuxConfig(session="Workspace", launch_session="running-custom-99"),
    )
    mgr = WorkspaceManager(cfg_with_launch)
    assert mgr.get_launch_session_name() == "running-custom-99"

    # When not configured
    cfg_without_launch = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        repositories={},
        workspaces_dir=tmp_path / "workspaces",
        tmux=TmuxConfig(session="Workspace"),
    )
    mgr2 = WorkspaceManager(cfg_without_launch)
    assert mgr2.get_launch_session_name() == f"running-{tmp_path.name}"


def test_get_or_create_launch_session_auto_generates_and_persists(tmp_path: Path):
    """Test get_or_create_launch_session auto-generates format and updates repositories.yml."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""# Project Configuration
tmux:
  session: "Workspace"
  command: "nvim" # Keep editor comment
repositories:
  service:
    bare: bares/service.git
    checkout: service
""")
    config = ConfigLoader.load_config(cfg_file)
    mgr = WorkspaceManager(config)

    assert config.tmux.launch_session is None
    launch_name = mgr.get_or_create_launch_session()

    # Verify format running-{project}-{4 hex}
    assert launch_name.startswith(f"running-{tmp_path.name}-")
    assert launch_name != "Workspace"
    assert len(launch_name) == len(f"running-{tmp_path.name}-") + 4

    # Verify in-memory update
    assert config.tmux.launch_session == launch_name

    # Verify file persisted and comment preserved
    persisted_content = cfg_file.read_text()
    assert f"launch_session: {launch_name}" in persisted_content
    assert "# Keep editor comment" in persisted_content

    # Calling it again returns existing without changing
    second_call = mgr.get_or_create_launch_session()
    assert second_call == launch_name


def test_get_or_create_launch_session_raises_on_collision(tmp_path: Path):
    """Test get_or_create_launch_session detects and raises on configured collision."""
    config = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        repositories={},
        workspaces_dir=tmp_path / "workspaces",
        tmux=TmuxConfig(session="Collide", launch_session="Collide"),
    )
    mgr = WorkspaceManager(config)
    with pytest.raises(ConfigException) as excinfo:
        mgr.get_or_create_launch_session()
    assert "must have different names to prevent collisions" in str(excinfo.value)


def test_setup_workspace_configures_launch_session(tmp_path: Path):
    """Test setup_workspace ensures launch_session is configured in repositories.yml."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""
tmux:
  session: "Workspace"
repositories:
  svc:
    bare: bares/svc.git
    checkout: svc
""")
    ws_dir = tmp_path / "workspaces" / "test-ws"
    ws_dir.mkdir(parents=True)
    (ws_dir / "workspace.yml").write_text(yaml.dump({
        "name": "test-ws",
        "repositories": {"svc": {"path": "svc", "branch": "main"}},
    }))
    (ws_dir / "svc").mkdir(parents=True)

    config = ConfigLoader.load_config(cfg_file)
    mgr = WorkspaceManager(config)

    # Dry run should not persist
    with patch.object(mgr.git, "is_git_installed", return_value=True):
        mgr.setup_workspace("test-ws", dry_run=True)
    assert mgr.config.tmux.launch_session is None

    # Normal setup should configure and persist
    with patch.object(mgr.git, "is_git_installed", return_value=True):
        mgr.setup_workspace("test-ws", dry_run=False)

    assert mgr.config.tmux.launch_session is not None
    assert mgr.config.tmux.launch_session.startswith(f"running-{tmp_path.name}-")
    assert "launch_session:" in cfg_file.read_text()


def test_launch_workspace_mode_tmux_passes_launch_session(tmp_path: Path):
    """Test launch_workspace in tmux mode invokes TmuxLauncher.launch with launch session."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""
tmux:
  session: "Workspace"
  launch_session: "running-myproj-fixed"
repositories:
  svc:
    bare: bares/svc.git
    checkout: svc
    launch: python app.py
""")
    ws_dir = tmp_path / "workspaces" / "feature-x"
    ws_dir.mkdir(parents=True)
    (ws_dir / "workspace.yml").write_text(yaml.dump({
        "name": "feature-x",
        "repositories": {"svc": {"path": "svc", "branch": "main"}},
    }))
    (ws_dir / "svc").mkdir(parents=True)

    config = ConfigLoader.load_config(cfg_file)
    mgr = WorkspaceManager(config)

    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "launch", return_value=True) as mock_launch, \
         patch.object(mgr, "get_active_engine", return_value=None), \
         patch("subprocess.Popen"):
        mgr.launch_workspace("feature-x", mode="tmux")

    mock_launch.assert_called_once()
    call_args, call_kwargs = mock_launch.call_args
    assert call_args[0] == "feature-x"
    assert call_kwargs.get("session_name") == "running-myproj-fixed"


def test_work_session_and_launch_session_isolation(tmp_path: Path):
    """Test that stop_workspace stops launch session window without affecting work session."""
    config = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        repositories={},
        workspaces_dir=tmp_path / "workspaces",
        tmux=TmuxConfig(session="WorkSession", launch_session="LaunchSession"),
    )
    mgr = WorkspaceManager(config)

    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "is_session_active", return_value=True), \
         patch.object(TmuxLauncher, "is_window_active") as mock_is_win_active, \
         patch.object(TmuxLauncher, "kill_workspace_window") as mock_kill_win:

        # Window exists in launch session
        mock_is_win_active.side_effect = lambda sess, win: sess == "LaunchSession" and win == "feat-a"

        stopped = mgr.stop_workspace("feat-a")
        assert stopped is True

        # Ensure kill_workspace_window was called for LaunchSession, NOT WorkSession
        mock_kill_win.assert_called_once_with("LaunchSession", "feat-a")
