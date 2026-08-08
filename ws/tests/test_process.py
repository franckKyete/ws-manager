"""Unit tests for concurrent ProcessSupervisor and service process group lifecycle."""

from pathlib import Path
import time
import pytest

from ws.process import ProcessSupervisor


def test_process_supervisor_concurrent_execution(tmp_path):
    """Test starting multiple services concurrently and capturing ring buffer output."""
    log_dir = tmp_path / "logs"
    supervisor = ProcessSupervisor(workspace_name="test-ws", log_dir=log_dir)

    # Register two mock services
    supervisor.register_service(
        name="server",
        command="python3 -u -c \"import time; print('Server listening on http://localhost:4010'); time.sleep(0.5); print('Server tick')\"",
        cwd=tmp_path,
        env={"PORT": "4010"},
    )
    supervisor.register_service(
        name="web",
        command="python3 -u -c \"import time; print('Vite dev server running on http://127.0.0.1:3010'); time.sleep(0.5); print('Web tick')\"",
        cwd=tmp_path,
        env={"PORT": "3010"},
    )

    # Start all concurrently
    supervisor.start_all()

    # Wait briefly for process execution and log capture
    time.sleep(0.5)

    server_svc = supervisor.services["server"]
    web_svc = supervisor.services["web"]

    # Verify both were alive and started
    assert server_svc.status in ("running", "stopped")
    assert web_svc.status in ("running", "stopped")

    # Verify port sniffing
    assert server_svc.detected_port == 4010
    assert web_svc.detected_port == 3010


    # Verify log capture in memory ring buffer
    server_logs = "".join(server_svc.log_buffer)
    assert "Server listening on http://localhost:4010" in server_logs

    # Stop all cleanly
    supervisor.stop_all(timeout=1.0)
    assert server_svc.is_alive is False
    assert web_svc.is_alive is False


def test_process_supervisor_restart_service(tmp_path):
    """Test restarting an individual service without affecting others."""
    supervisor = ProcessSupervisor(workspace_name="test-ws")
    supervisor.register_service(
        name="worker",
        command="python3 -c \"import time; print('Worker v1'); time.sleep(0.5)\"",
        cwd=tmp_path,
        env={},
    )

    supervisor.start_service("worker")
    time.sleep(0.2)
    first_pid = supervisor.services["worker"].pid
    assert first_pid is not None

    # Restart
    supervisor.restart_service("worker")
    time.sleep(0.2)
    second_pid = supervisor.services["worker"].pid

    assert second_pid is not None
    assert second_pid != first_pid

    supervisor.stop_all(timeout=1.0)


def test_process_supervisor_send_input(tmp_path):
    """Test sending interactive stdin input to a running service."""
    supervisor = ProcessSupervisor(workspace_name="test-ws")
    supervisor.register_service(
        name="repl",
        command="python3 -u -c \"name = input('Name: '); print('Hello ' + name)\"",
        cwd=tmp_path,
        env={},
    )

    supervisor.start_service("repl", use_pty=False)
    time.sleep(0.2)

    # Send interactive input
    supervisor.send_input("repl", "Alice\n")
    time.sleep(0.3)

    supervisor.stop_all(timeout=1.0)


def test_virtual_line_buffer_progress_updates():
    """Test VirtualLineBuffer properly handles in-place carriage returns and cursor updates."""
    from ws.process import VirtualLineBuffer

    buf = VirtualLineBuffer()

    # 1. Metro / Expo progress bar updates via \r
    buf.feed("Starting bundling...\n")
    for pct in range(10, 100, 10):
        buf.feed(f"Android entry.js {pct}.0% ({pct*40}/4000)\r")
    buf.feed("Android Bundled 120ms (4000 modules)\n")
    buf.feed("LOG Sentry disabled in dev\n")

    lines = buf.get_lines()
    assert len(lines) == 3
    assert lines[0] == "Starting bundling..."
    assert "Android Bundled" in lines[1]
    assert "10.0%" not in lines[1]
    assert lines[2] == "LOG Sentry disabled in dev"

    # 2. Vite / Webpack multi-line cursor up updates via \x1b[1A\x1b[2K\r
    buf2 = VirtualLineBuffer()
    buf2.feed("Compiling...\n")
    buf2.feed("Building: 10%\n")
    buf2.feed("\x1b[1A\x1b[2K\rBuilding: 50%\n")
    buf2.feed("\x1b[1A\x1b[2K\rBuilding: 100%\n")
    buf2.feed("Build complete\n")

    lines2 = buf2.get_lines()
    assert len(lines2) == 3
    assert lines2[0] == "Compiling..."
    assert lines2[1] == "Building: 100%"
    assert lines2[2] == "Build complete"


