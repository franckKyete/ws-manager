"""Unit tests for tmux and terminal window multiplexer launchers."""

from pathlib import Path
import pytest

from ws.multiplexer import TerminalLauncher, TmuxLauncher, ZellijLauncher


def test_tmux_command_building():
    """Test inline environment variable export builder for tmux."""
    cmd = TmuxLauncher._build_env_command(
        cmd="npm run dev",
        env={"PORT": "4010", "DB_NAME": "renttik_test"},
        ws_name="auth-flow",
        repo_name="server",
    )
    assert "PORT=4010" in cmd
    assert "WORKSPACE_NAME=auth-flow" in cmd
    assert "npm run dev" in cmd


def test_terminal_detection():
    """Test terminal detection helper."""
    detected = TerminalLauncher.detect_terminal()
    # On linux system, might be None or a detected terminal name
    assert detected is None or isinstance(detected, str)


def test_tmux_session_and_window_naming():
    """Test project-wide session and workspace window naming."""
    assert TmuxLauncher.session_name("renttik") == "ws-renttik"
    assert ZellijLauncher.session_name("renttik") == "ws-renttik"


def test_zellij_layout_generation_two_services():
    """Test Zellij KDL layout generation for two services."""
    services = [
        ("mobile", "/path/to/mobile", "npm run start", {"EXPO_PORT": "8081"}),
        ("server", "/path/to/server", "composer dev", {"PORT": "8000"}),
    ]
    kdl = ZellijLauncher.generate_layout_kdl("develop", services)
    assert 'tab name="develop"' in kdl
    assert 'pane name="mobile"' in kdl
    assert 'pane name="server"' in kdl
    assert 'cwd="/path/to/mobile"' in kdl
    assert 'cwd="/path/to/server"' in kdl
    assert 'split_direction="vertical"' in kdl
    assert 'args "bridge" "develop" "mobile"' in kdl
    assert 'args "bridge" "develop" "server"' in kdl



def test_zellij_layout_generation_grid_services():
    """Test Zellij KDL layout generation for 4+ services."""
    services = [
        ("frontend", "/path/to/fe", "npm run dev", {}),
        ("backend", "/path/to/be", "python main.py", {}),
        ("worker", "/path/to/worker", "celery worker", {}),
        ("redis", "/path/to/redis", "redis-server", {}),
    ]
    kdl = ZellijLauncher.generate_layout_kdl("prod-test", services)
    assert 'tab name="prod-test"' in kdl
    assert 'pane name="frontend"' in kdl
    assert 'pane name="backend"' in kdl
    assert 'pane name="worker"' in kdl
    assert 'pane name="redis"' in kdl
    assert 'split_direction="horizontal"' in kdl


def test_zellij_availability_and_session_check():
    """Test Zellij is_available and is_session_running handles missing binary gracefully."""
    avail = ZellijLauncher.is_available()
    assert isinstance(avail, bool)
    running = ZellijLauncher.is_session_running("nonexistent-project-12345")
    assert running is False


def test_tmux_list_panes_nonexistent_window():
    """Test Tmux list_panes returns empty list for non-existent window."""
    panes = TmuxLauncher.list_panes("nonexistent-proj", "nonexistent-ws")
    assert panes == []


def test_cli_switch_flag_parsed():
    """Test CLI parser accepts --switch / -s for launch and attach."""
    from ws.cli import build_parser
    parser = build_parser()

    args = parser.parse_args(["launch", "develop", "--tmux", "--switch"])
    assert args.tmux is True
    assert args.switch is True

    args_attach = parser.parse_args(["attach", "develop", "--zellij", "-s"])
    assert args_attach.zellij is True
    assert args_attach.switch is True




