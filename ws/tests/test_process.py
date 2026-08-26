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
    for _ in range(20):
        if supervisor.services["server"].detected_port and supervisor.services["web"].detected_port:
            break
        time.sleep(0.05)

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

    # 2. Expo QR code and bundler progress preservation
    buf2 = VirtualLineBuffer()
    buf2.feed("Starting project at /workspace/develop/Renttik-mobile\n")
    buf2.feed("Starting Metro Bundler\n")
    sample_qr = [
        "\x1b[47m  \x1b[40m                        \x1b[47m  \x1b[0m",
        "\x1b[47m  \x1b[40m  \x1b[47m███████\x1b[40m  \x1b[47m█\x1b[40m \x1b[47m█\x1b[40m  \x1b[47m███████\x1b[40m  \x1b[47m  \x1b[0m",
        "\x1b[47m  \x1b[40m  \x1b[47m█\x1b[40m     \x1b[47m█\x1b[40m  \x1b[47m██\x1b[40m    \x1b[47m█\x1b[40m     \x1b[47m█\x1b[40m  \x1b[47m  \x1b[0m",
        "\x1b[47m  \x1b[40m  \x1b[47m███████\x1b[40m  \x1b[47m█\x1b[40m \x1b[47m█\x1b[40m \x1b[47m█\x1b[40m \x1b[47m███████\x1b[40m  \x1b[47m  \x1b[0m",
    ]
    for line in sample_qr:
        buf2.feed(f"{line}\n")
    buf2.feed("› Press a │ open Android\n")

    for pct in [81.4, 81.8, 90.0, 99.9]:
        buf2.feed(f"\rAndroid entry.js {pct}%")

    buf2.feed("\r\x1b[2KAndroid Bundled 471ms (4969 modules)\n")
    buf2.feed("LOG Sentry disabled in development\n")

    lines2 = buf2.get_lines()
    assert len(lines2) == 2 + len(sample_qr) + 1 + 2
    assert "Starting project" in lines2[0]
    assert "Starting Metro" in lines2[1]
    assert "███████" in lines2[3]
    assert lines2[-2] == "Android Bundled 471ms (4969 modules)"
    assert lines2[-1] == "LOG Sentry disabled in development"





