"""Unit tests for WorkspaceTUI layout, rendering, and scrollback."""

from pathlib import Path
from rich.console import Console
import pytest

from ws.process import ProcessSupervisor
from ws.tui import WorkspaceTUI


def test_tui_layout_generation_and_fullscreen_toggle(tmp_path):
    """Test generating dynamic multi-pane layouts and fullscreen toggling."""
    supervisor = ProcessSupervisor(workspace_name="test-ws")
    for name in ["server", "web", "mobile"]:
        supervisor.register_service(
            name=name,
            command="echo ok",
            cwd=tmp_path,
            env={},
        )

    tui = WorkspaceTUI(workspace_name="test-ws", supervisor=supervisor, console=Console(color_system=None))
    assert tui.focused_service_name == "server"

    # Render multi-pane layout
    layout = tui._render_screen()
    assert layout is not None

    # Toggle fullscreen
    tui.fullscreen_mode = True
    fs_layout = tui._render_screen()
    assert fs_layout is not None

    # Toggle interactive mode
    tui.interactive_mode = True
    it_layout = tui._render_screen()
    assert it_layout is not None


def test_tui_scrollback_offsets_and_navigation(tmp_path):
    """Test scrollback navigation, offset bounds, and history indicators."""
    supervisor = ProcessSupervisor(workspace_name="test-ws")
    service = supervisor.register_service(
        name="server",
        command="echo ok",
        cwd=tmp_path,
        env={},
    )
    # Populate 50 log lines
    with service.lock:
        for i in range(1, 51):
            service.line_buffer.feed(f"Server log line {i:02d}\n")


    tui = WorkspaceTUI(workspace_name="test-ws", supervisor=supervisor, console=Console(color_system=None))

    # 1. Default: follow mode (offset = 0)
    panel = tui._render_service_pane("server", is_focused=True, height=10)
    assert tui.scroll_offsets.get("server") == 0

    # 2. Scroll up 5 lines
    tui._scroll_focused(5)
    assert tui.scroll_offsets.get("server") == 5
    panel_scrolled = tui._render_service_pane("server", is_focused=True, height=10)
    # Scrollback badge appears in title
    assert "SCROLLBACK" in str(panel_scrolled.title)

    # 3. Scroll down 2 lines
    tui._scroll_focused(-2)
    assert tui.scroll_offsets.get("server") == 3

    # 4. Jump to top
    tui._scroll_top()
    tui._render_service_pane("server", is_focused=True, height=10)
    assert tui.scroll_offsets.get("server") > 0

    # 5. Jump to bottom / follow mode
    tui._scroll_bottom()
    assert tui.scroll_offsets.get("server") == 0
    panel_bottom = tui._render_service_pane("server", is_focused=True, height=10)
    assert "SCROLLBACK" not in str(panel_bottom.title)
