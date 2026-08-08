"""Unit tests for tmux and terminal window multiplexer launchers."""

from pathlib import Path
import pytest

from ws.multiplexer import TerminalLauncher, TmuxLauncher


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
