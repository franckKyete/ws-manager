"""Process supervisor for concurrent workspace service execution and lifecycle management."""

from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
import logging
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading
import time
from typing import Any, Callable, Sequence

logger = logging.getLogger("ws.process")

# Regex to detect port numbers and URLs in service logs
PORT_REGEX = re.compile(
    r"(?:https?://(?:localhost|127\.0\.0\.1|0\.0\.0\.0):(\d{2,5})|localhost:(\d{2,5})|(?:port|PORT)\s*(?:=|:|\s)\s*(\d{2,5})|listening on\s*:?(\d{2,5}))",
    re.IGNORECASE,
)


class VirtualLineBuffer:
    """Stream-aware terminal line buffer supporting in-place \\r and cursor-up updates."""

    def __init__(self, max_lines: int = 5000) -> None:
        self.max_lines = max_lines
        self.lines: list[str] = []
        self.current_line: str = ""

    def feed(self, text: str) -> None:
        """Feed a streaming chunk, handling \\r, \\n, \\x1b[1A (cursor up), and \\x1b[2K (clear line)."""
        i = 0
        n = len(text)
        while i < n:
            # Handle CSI ANSI cursor control sequences
            if text[i] == "\x1b" and i + 1 < n and text[i + 1] == "[":
                match_end = i + 2
                while match_end < n and not (
                    ("A" <= text[match_end] <= "Z")
                    or ("a" <= text[match_end] <= "z")
                    or text[match_end] == "~"
                ):
                    match_end += 1
                if match_end < n:
                    cmd = text[match_end]
                    seq = text[i : match_end + 1]

                    if cmd == "A":  # Cursor Up (\x1b[1A)
                        count_str = seq[2:-1]
                        count = int(count_str) if count_str.isdigit() else 1
                        for _ in range(count):
                            if self.lines:
                                self.current_line = self.lines.pop()
                        i = match_end + 1
                        continue
                    elif cmd in ("K", "J"):  # Clear line / clear screen
                        self.current_line = ""
                        i = match_end + 1
                        continue

            ch = text[i]
            if ch == "\r":
                if i + 1 < n and text[i + 1] == "\n":
                    self.lines.append(self.current_line)
                    self.current_line = ""
                    i += 2
                    continue
                else:
                    self.current_line = ""
                    i += 1
                    continue
            elif ch == "\n":
                self.lines.append(self.current_line)
                self.current_line = ""
                i += 1
                continue
            else:
                self.current_line += ch
                i += 1

        if len(self.lines) > self.max_lines:
            self.lines = self.lines[-self.max_lines :]

    def get_lines(self) -> list[str]:
        """Return all committed lines plus the active in-progress line."""
        if self.current_line:
            return self.lines + [self.current_line]
        return list(self.lines)

    def clear(self) -> None:
        """Clear all lines and in-progress state."""
        self.lines.clear()
        self.current_line = ""


@dataclass
class ManagedService:
    """Represents an active, supervised workspace service process."""

    name: str
    command: str
    cwd: Path
    env: dict[str, str]
    process: subprocess.Popen[str] | None = None
    pid: int | None = None
    pgid: int | None = None
    master_fd: int | None = None
    status: str = "starting"  # starting, running, stopped, failed
    exit_code: int | None = None
    start_time: float = field(default_factory=time.time)
    line_buffer: VirtualLineBuffer = field(default_factory=VirtualLineBuffer)
    detected_port: int | None = None
    log_file: Path | None = None
    lock: threading.Lock = field(default_factory=threading.Lock)

    @property
    def log_buffer(self) -> list[str]:
        """Backward-compatible log buffer returning current lines."""
        return self.line_buffer.get_lines()

    @property
    def uptime(self) -> float:
        """Elapsed runtime in seconds."""
        return time.time() - self.start_time

    @property
    def is_alive(self) -> bool:
        """Check if process is currently running."""
        if not self.process:
            return False
        return self.process.poll() is None

    def send_input(self, data: str | bytes) -> bool:
        """Send interactive keyboard input into the service stdin or PTY."""
        raw = data if isinstance(data, bytes) else data.encode("utf-8")
        if self.master_fd is not None:
            try:
                os.write(self.master_fd, raw)
                return True
            except Exception as e:
                logger.debug("Failed writing to PTY master_fd: %s", e)
                return False
        elif self.process and self.process.stdin:
            try:
                if isinstance(data, bytes):
                    self.process.stdin.buffer.write(data)
                else:
                    self.process.stdin.write(data)
                self.process.stdin.flush()
                return True
            except Exception as e:
                logger.debug("Failed writing to process stdin: %s", e)
                return False
        return False


class ProcessSupervisor:
    """Supervises multiple concurrent child service processes with non-blocking I/O and PTY support."""

    def __init__(self, workspace_name: str, log_dir: Path | None = None) -> None:
        self.workspace_name = workspace_name
        self.log_dir = log_dir
        self.services: dict[str, ManagedService] = {}
        self._threads: list[threading.Thread] = []
        self._on_output_callbacks: list[Callable[[str, str], None]] = []
        self._running = True

    def register_service(
        self,
        name: str,
        command: str,
        cwd: Path,
        env: dict[str, str],
    ) -> ManagedService:
        """Register a service to be supervised."""
        log_file: Path | None = None
        if self.log_dir:
            self.log_dir.mkdir(parents=True, exist_ok=True)
            log_file = self.log_dir / f"{name}.log"

        service = ManagedService(
            name=name,
            command=command,
            cwd=cwd,
            env=env,
            log_file=log_file,
        )
        self.services[name] = service
        return service

    def start_service(self, name: str, use_pty: bool = True) -> bool:
        """Start or restart a registered service in a background process group with optional PTY."""
        service = self.services.get(name)
        if not service:
            return False

        # Stop existing process if running
        if service.is_alive:
            self.stop_service(name)

        with service.lock:
            service.status = "starting"
            service.exit_code = None
            service.start_time = time.time()
            service.master_fd = None

            try:
                proc_env = os.environ.copy()
                proc_env.update(service.env)
                proc_env["WORKSPACE_NAME"] = self.workspace_name
                proc_env["REPO_NAME"] = name
                proc_env["PYTHONUNBUFFERED"] = "1"
                proc_env["FORCE_COLOR"] = "1"

                # Check if pty is supported
                master_fd: int | None = None
                slave_fd: int | None = None
                if use_pty and hasattr(os, "openpty"):
                    try:
                        import pty
                        import termios
                        master_fd, slave_fd = pty.openpty()
                        service.master_fd = master_fd
                        try:
                            slave_attr = termios.tcgetattr(slave_fd)
                            slave_attr[3] &= ~termios.ECHO  # Disable local echo in PTY slave
                            slave_attr[1] |= termios.ONLCR  # Map NL to CR-NL
                            termios.tcsetattr(slave_fd, termios.TCSANOW, slave_attr)
                        except Exception:
                            pass
                    except Exception as e:
                        logger.debug("PTY allocation fallback: %s", e)
                        master_fd, slave_fd = None, None

                # Start process in its own process group (POSIX setsid)
                preexec = os.setsid if hasattr(os, "setsid") else None

                if slave_fd is not None:
                    proc = subprocess.Popen(
                        service.command,
                        shell=True,
                        cwd=service.cwd,
                        env=proc_env,
                        stdin=slave_fd,
                        stdout=slave_fd,
                        stderr=slave_fd,
                        close_fds=True,
                        preexec_fn=preexec,
                    )
                    os.close(slave_fd)
                else:
                    proc = subprocess.Popen(
                        service.command,
                        shell=True,
                        cwd=service.cwd,
                        env=proc_env,
                        stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        text=True,
                        bufsize=1,
                        preexec_fn=preexec,
                    )

                service.process = proc
                service.pid = proc.pid
                if hasattr(os, "getpgid"):
                    try:
                        service.pgid = os.getpgid(proc.pid)
                    except Exception:
                        service.pgid = proc.pid
                else:
                    service.pgid = proc.pid

                service.status = "running"

            except Exception as e:
                logger.error("Failed to spawn process for '%s': %s", name, e)
                service.status = "failed"
                service.exit_code = -1
                service.line_buffer.feed(f"[Error: Failed to spawn process: {e}]\n")
                return False

        # Launch background reader thread
        reader_thread = threading.Thread(
            target=self._reader_loop,
            args=(service,),
            daemon=True,
            name=f"ws-reader-{name}",
        )
        reader_thread.start()
        self._threads.append(reader_thread)
        return True

    def start_all(self, use_pty: bool = True) -> None:
        """Start all registered services concurrently."""
        for name in list(self.services.keys()):
            self.start_service(name, use_pty=use_pty)

    def send_input(self, name: str, data: str | bytes) -> bool:
        """Send interactive input to a specific service."""
        service = self.services.get(name)
        if service:
            return service.send_input(data)
        return False

    def _reader_loop(self, service: ManagedService) -> None:
        """Asynchronously stream output from process stdout/PTY into log buffer and file."""
        file_handle = None
        if service.log_file:
            try:
                file_handle = open(service.log_file, "a", encoding="utf-8", buffering=1)
            except Exception as e:
                logger.warning("Could not open log file %s: %s", service.log_file, e)

        try:
            if service.master_fd is not None:
                # Read from PTY master fd
                while self._running:
                    try:
                        raw = os.read(service.master_fd, 1024)
                        if not raw:
                            break
                        line = raw.decode("utf-8", errors="replace")

                        # Port sniffing
                        if not service.detected_port:
                            match = PORT_REGEX.search(line)
                            if match:
                                for g in match.groups():
                                    if g and g.isdigit() and int(g) > 80:
                                        service.detected_port = int(g)
                                        break

                        with service.lock:
                            service.line_buffer.feed(line)

                        if file_handle:
                            file_handle.write(line)

                        for cb in self._on_output_callbacks:
                            try:
                                cb(service.name, line)
                            except Exception:
                                pass
                    except OSError:
                        break
            else:
                proc = service.process
                if proc and proc.stdout:
                    for line in iter(proc.stdout.readline, ""):
                        if not line:
                            break

                        # Port sniffing
                        if not service.detected_port:
                            match = PORT_REGEX.search(line)
                            if match:
                                for g in match.groups():
                                    if g and g.isdigit() and int(g) > 80:
                                        service.detected_port = int(g)
                                        break

                        with service.lock:
                            service.line_buffer.feed(line)

                        if file_handle:
                            file_handle.write(line)

                        for cb in self._on_output_callbacks:
                            try:
                                cb(service.name, line)
                            except Exception:
                                pass
        except Exception as e:
            logger.debug("Reader loop ended for %s: %s", service.name, e)
        finally:
            if file_handle:
                try:
                    file_handle.close()
                except Exception:
                    pass
            if service.master_fd is not None:
                try:
                    os.close(service.master_fd)
                except Exception:
                    pass
                service.master_fd = None

            if service.process:
                service.exit_code = service.process.poll()
                service.status = "stopped" if service.exit_code == 0 else "failed"

    def stop_service(self, name: str, timeout: float = 3.0) -> bool:
        """Gracefully stop a service process and its entire process group with SIGTERM -> SIGKILL."""
        service = self.services.get(name)
        if not service or not service.process or not service.is_alive:
            return True

        pgid = service.pgid
        pid = service.pid

        # Attempt graceful SIGTERM on the entire process group
        try:
            if pgid and hasattr(os, "killpg"):
                os.killpg(pgid, signal.SIGTERM)
            elif pid:
                os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        except Exception as e:
            logger.debug("SIGTERM failed for %s: %s", name, e)

        start_wait = time.time()
        while time.time() - start_wait < timeout:
            if not service.is_alive:
                break
            time.sleep(0.1)

        # Forceful SIGKILL if still alive after timeout
        if service.is_alive:
            try:
                if pgid and hasattr(os, "killpg"):
                    os.killpg(pgid, signal.SIGKILL)
                elif pid:
                    os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except Exception as e:
                logger.debug("SIGKILL failed for %s: %s", name, e)

        service.status = "stopped"
        if service.process:
            service.exit_code = service.process.poll()
        return True

    def restart_service(self, name: str) -> bool:
        """Restart a specific service."""
        self.stop_service(name)
        return self.start_service(name)

    def stop_all(self, timeout: float = 3.0) -> None:
        """Stop all supervised services cleanly."""
        self._running = False
        threads = []
        for name in list(self.services.keys()):
            t = threading.Thread(target=self.stop_service, args=(name, timeout))
            t.start()
            threads.append(t)

        for t in threads:
            t.join(timeout=timeout + 1.0)
