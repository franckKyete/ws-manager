"""Unit tests for tmux workspace session and window integration."""

from pathlib import Path
from unittest.mock import MagicMock, call, patch
import pytest

from ws.cli import build_parser, normalize_cli_args
from ws.config import ConfigLoader
from ws.exceptions import ConfigException, WorkspaceNotFoundException
from ws.models import AppConfig, RepoConfig, RepoSpec, TmuxConfig
from ws.multiplexer import TmuxLauncher
from ws.workspace import WorkspaceManager


def test_tmux_config_model():
    """Test TmuxConfig dataclass serialization and deserialization."""
    # From string shorthand
    cfg1 = TmuxConfig.from_dict("Workspace")
    assert cfg1.session == "Workspace"
    assert cfg1.command is None
    assert cfg1.switch is False
    assert cfg1.to_dict() == {"session": "Workspace"}

    # From full dictionary
    cfg2 = TmuxConfig.from_dict({
        "session": "MyProject",
        "command": "nvim",
        "switch": True,
    })
    assert cfg2.session == "MyProject"
    assert cfg2.command == "nvim"
    assert cfg2.switch is True
    assert cfg2.to_dict() == {
        "session": "MyProject",
        "command": "nvim",
        "switch": True,
    }

    # Aliases session_name / cmd
    cfg3 = TmuxConfig.from_dict({"session_name": "DevSess", "cmd": "helix"})
    assert cfg3.session == "DevSess"
    assert cfg3.command == "helix"

    # Missing session raises error
    with pytest.raises(ValueError):
        TmuxConfig.from_dict({"command": "nvim"})


def test_config_loader_tmux(tmp_path: Path):
    """Test ConfigLoader loading tmux configuration from repositories.yml."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""
tmux:
  session: "WorkspaceSess"
  command: "nvim"
  switch: false

repositories:
  web:
    bare: bares/web.git
    checkout: web
""", encoding="utf-8")

    app_cfg = ConfigLoader.load_config(config_path=cfg_file)
    assert app_cfg.tmux is not None
    assert app_cfg.tmux.session == "WorkspaceSess"
    assert app_cfg.tmux.command == "nvim"
    assert app_cfg.tmux.switch is False


def test_config_loader_tmux_shorthand(tmp_path: Path):
    """Test ConfigLoader loading tmux shorthand string from repositories.yml."""
    cfg_file = tmp_path / "repositories.yml"
    cfg_file.write_text("""
tmux: "Workspace"

repositories:
  web:
    bare: bares/web.git
    checkout: web
""", encoding="utf-8")

    app_cfg = ConfigLoader.load_config(config_path=cfg_file)
    assert app_cfg.tmux is not None
    assert app_cfg.tmux.session == "Workspace"
    assert app_cfg.tmux.command is None


def test_tmux_launcher_create_workspace_window_new_session():
    """Test creating workspace window when session does not exist."""
    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "is_session_active", return_value=False), \
         patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0)

        res = TmuxLauncher.create_workspace_window(
            session_name="Workspace",
            window_name="feature-auth",
            cwd=Path("/workspaces/feature-auth"),
            command="nvim",
            switch=False,
        )

        assert res is True
        mock_run.assert_called_once_with(
            [
                "tmux", "new-session", "-d",
                "-s", "Workspace",
                "-n", "feature-auth",
                "-c", "/workspaces/feature-auth",
                "nvim",
            ],
            capture_output=True,
            check=False,
        )


def test_tmux_launcher_create_workspace_window_existing_session():
    """Test creating workspace window in an existing session."""
    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "is_session_active", return_value=True), \
         patch.object(TmuxLauncher, "is_window_active", return_value=False), \
         patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0)

        res = TmuxLauncher.create_workspace_window(
            session_name="Workspace",
            window_name="feature-auth",
            cwd=Path("/workspaces/feature-auth"),
            command=None,
            switch=False,
        )

        assert res is True
        mock_run.assert_called_once_with(
            [
                "tmux", "new-window", "-d",
                "-t", "Workspace",
                "-n", "feature-auth",
                "-c", "/workspaces/feature-auth",
            ],
            capture_output=True,
            check=False,
        )


def test_tmux_launcher_kill_workspace_window():
    """Test killing workspace window."""
    with patch.object(TmuxLauncher, "is_session_active", return_value=True), \
         patch.object(TmuxLauncher, "is_window_active", return_value=True), \
         patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=0)

        res = TmuxLauncher.kill_workspace_window("Workspace", "feature-auth")
        assert res is True
        mock_run.assert_called_once_with(
            ["tmux", "kill-window", "-t", "Workspace:feature-auth"],
            capture_output=True,
            check=False,
        )


def test_tmux_launcher_focus_workspace_window():
    """Test focusing workspace window inside and outside tmux."""
    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "is_session_active", return_value=True):

        # Inside tmux
        with patch.dict("os.environ", {"TMUX": "/tmp/tmux-1000/default,1234,0"}), \
             patch("subprocess.run") as mock_run:
            TmuxLauncher.focus_workspace_window("Workspace", "feature-auth")
            mock_run.assert_has_calls([
                call(["tmux", "select-window", "-t", "Workspace:feature-auth"], capture_output=True, check=False),
                call(["tmux", "switch-client", "-t", "Workspace:feature-auth"], check=False),
            ])

        # Outside tmux
        with patch.dict("os.environ", {}, clear=True), \
             patch("subprocess.run") as mock_run, \
             patch("os.system") as mock_sys:
            TmuxLauncher.focus_workspace_window("Workspace", "feature-auth")
            mock_run.assert_called_once_with(
                ["tmux", "select-window", "-t", "Workspace:feature-auth"],
                capture_output=True,
                check=False,
            )
            mock_sys.assert_called_once_with("tmux attach-session -t Workspace:feature-auth")


def test_workspace_manager_create_and_end_hooks(tmp_path: Path):
    """Test WorkspaceManager create_workspace and end_workspace tmux window hooks."""
    ws_dir = tmp_path / "workspaces"
    bares_dir = tmp_path / "bares"
    bares_dir.mkdir(parents=True)
    bare_repo = bares_dir / "web.git"
    bare_repo.mkdir()

    repo_cfg = RepoConfig(name="web", bare=bare_repo, checkout="web")
    app_cfg = AppConfig(
        repositories={"web": repo_cfg},
        workspaces_dir=ws_dir,
        tmux=TmuxConfig(session="Workspace", command="nvim"),
    )

    manager = WorkspaceManager(config=app_cfg)
    manager.git = MagicMock()
    manager.git.get_default_branch.return_value = "main"
    manager.git.branch_exists.return_value = False

    spec = RepoSpec(name="web", branch="feature/test-tmux", create=True, path="web")

    with patch.object(TmuxLauncher, "create_workspace_window") as mock_create_win, \
         patch.object(TmuxLauncher, "is_window_active", return_value=True), \
         patch.object(TmuxLauncher, "kill_workspace_window") as mock_kill_win:

        # 1. Create workspace with tmux
        manager.create_workspace("test-tmux", [spec], tmux_cmd="helix")
        mock_create_win.assert_called_once_with(
            session_name="Workspace",
            window_name="test-tmux",
            cwd=ws_dir / "test-tmux",
            command="helix",
            switch=False,
        )

        # 2. End workspace with tmux
        manager.inspect_workspace_safety = MagicMock(return_value={
            "workspace": "test-tmux",
            "has_uncommitted": False,
            "has_unmerged": False,
            "repos": {},
        })
        manager.is_session_running = MagicMock(return_value=False)

        manager.end_workspace("test-tmux")
        mock_kill_win.assert_called_once_with("Workspace", "test-tmux")


def test_workspace_manager_create_with_no_tmux(tmp_path: Path):
    """Test create_workspace with --no-tmux flag skips window creation."""
    ws_dir = tmp_path / "workspaces"
    bares_dir = tmp_path / "bares"
    bares_dir.mkdir(parents=True)
    bare_repo = bares_dir / "web.git"
    bare_repo.mkdir()

    repo_cfg = RepoConfig(name="web", bare=bare_repo, checkout="web")
    app_cfg = AppConfig(
        repositories={"web": repo_cfg},
        workspaces_dir=ws_dir,
        tmux=TmuxConfig(session="Workspace"),
    )

    manager = WorkspaceManager(config=app_cfg)
    manager.git = MagicMock()
    manager.git.get_default_branch.return_value = "main"
    manager.git.branch_exists.return_value = False

    spec = RepoSpec(name="web", branch="feature/test-no-tmux", create=True, path="web")

    with patch.object(TmuxLauncher, "create_workspace_window") as mock_create_win:
        manager.create_workspace("test-no-tmux", [spec], no_tmux=True)
        mock_create_win.assert_not_called()


def test_workspace_manager_focus_workspace():
    """Test focus_workspace raises when tmux not configured or window not found."""
    # When tmux not configured
    app_cfg_no_tmux = AppConfig(repositories={})
    mgr_no_tmux = WorkspaceManager(config=app_cfg_no_tmux)
    with pytest.raises(ConfigException):
        mgr_no_tmux.focus_workspace("foo")

    # When tmux configured
    app_cfg_tmux = AppConfig(repositories={}, tmux=TmuxConfig(session="Workspace"))
    mgr_tmux = WorkspaceManager(config=app_cfg_tmux)

    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "is_window_active", return_value=False):
        with pytest.raises(WorkspaceNotFoundException):
            mgr_tmux.focus_workspace("nonexistent")

    with patch.object(TmuxLauncher, "is_available", return_value=True), \
         patch.object(TmuxLauncher, "is_window_active", return_value=True), \
         patch.object(TmuxLauncher, "focus_workspace_window", return_value=True) as mock_focus:
        assert mgr_tmux.focus_workspace("myws") is True
        mock_focus.assert_called_once_with("Workspace", "myws")


def test_cli_parsing_tmux_options():
    """Test CLI argument parser for tmux flags and focus command."""
    parser = build_parser()

    # ws create --cmd and --no-tmux
    args_create = parser.parse_args(["create", "@feat", "--cmd", "nvim", "--no-tmux"])
    assert args_create.tmux_cmd == "nvim"
    assert args_create.no_tmux is True

    # ws end --no-tmux
    args_end = parser.parse_args(["end", "@feat", "--no-tmux"])
    assert args_end.no_tmux is True

    # ws focus / ws switch
    args_focus = parser.parse_args(["focus", "@feat"])
    assert args_focus.name == "@feat"

    args_switch = parser.parse_args(["switch", "@feat"])
    assert args_switch.name == "@feat"


def test_cli_inverted_syntax_focus():
    """Test universal inverted syntax for focus and switch."""
    normalized1 = normalize_cli_args(["@feat", "focus"])
    assert normalized1 == ["focus", "@feat"]

    normalized2 = normalize_cli_args(["@feat", "switch"])
    assert normalized2 == ["switch", "@feat"]
