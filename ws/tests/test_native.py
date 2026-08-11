"""Unit tests for native compiled Rust extension (ws._native)."""

import pytest


def test_native_service_spec():
    """Test instantiating native Rust ServiceSpec from Python."""
    try:
        from ws._native import ServiceSpec
    except ImportError:
        pytest.skip("Native Rust extension not compiled")

    spec = ServiceSpec(
        name="server",
        command="python3 -m http.server 4010",
        cwd="/tmp",
        env={"PORT": "4010", "NODE_ENV": "development"},
    )
    assert spec.name == "server"
    assert spec.command == "python3 -m http.server 4010"
    assert spec.cwd == "/tmp"
    assert spec.env.get("PORT") == "4010"
    assert spec.env.get("NODE_ENV") == "development"


def test_native_run_tui_signature():
    """Test native run_workspace_tui function existence."""
    try:
        from ws._native import run_workspace_tui, start_workspace_daemon, attach_workspace_session, is_session_active
    except ImportError:
        pytest.skip("Native Rust extension not compiled")

    assert callable(run_workspace_tui)
    assert callable(start_workspace_daemon)
    assert callable(attach_workspace_session)
    assert callable(is_session_active)
    assert is_session_active("/non/existent/path.sock") is False

