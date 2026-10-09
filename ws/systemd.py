"""Systemd user service integration for ws background daemon.

Manages ~/.config/systemd/user/ws.service allowing automatic startup on boot,
clean background restarts, and native logging via journald.
"""

from __future__ import annotations

import logging
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any

from ws.utils import ensure_directory

logger = logging.getLogger("ws.systemd")

SERVICE_NAME = "ws.service"
USER_SYSTEMD_DIR = Path.home() / ".config" / "systemd" / "user"
SERVICE_PATH = USER_SYSTEMD_DIR / SERVICE_NAME


def is_systemctl_available() -> bool:
    """Check if systemctl CLI is installed on this system."""
    return shutil.which("systemctl") is not None


def find_ws_binary() -> str:
    """Find the path to the installed ws executable."""
    found = shutil.which("ws")
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / "ws"
    if fallback.exists():
        return str(fallback)
    return sys.executable + " -m ws.cli"


def generate_service_unit(ws_exec: str | None = None) -> str:
    """Generate systemd unit file content."""
    exec_path = ws_exec or find_ws_binary()
    return f"""[Unit]
Description=ws Global Background Daemon
Documentation=https://github.com/franckKyete/ws-manager
After=network.target

[Service]
Type=simple
ExecStart={exec_path} daemon
Restart=always
RestartSec=10
Environment=PYTHONUNBUFFERED=1

[Install]
WantedBy=default.target
"""


def install_service(ws_exec: str | None = None) -> tuple[bool, str]:
    """
    Install, enable, and start ws.service as a user systemd service.
    Returns (success, message).
    """
    if not is_systemctl_available():
        return False, "systemctl is not available on this system."

    ensure_directory(USER_SYSTEMD_DIR)
    unit_content = generate_service_unit(ws_exec)
    SERVICE_PATH.write_text(unit_content, encoding="utf-8")

    # Reload systemd user daemon
    subprocess.run(["systemctl", "--user", "daemon-reload"], check=False, capture_output=True)

    # Enable and start service
    res_enable = subprocess.run(
        ["systemctl", "--user", "enable", "--now", SERVICE_NAME],
        check=False,
        capture_output=True,
        text=True,
    )
    if res_enable.returncode != 0:
        err = res_enable.stderr.strip() or res_enable.stdout.strip()
        return False, f"Failed enabling {SERVICE_NAME}: {err}"

    return True, f"Installed and started {SERVICE_NAME} successfully."


def uninstall_service() -> tuple[bool, str]:
    """
    Stop, disable, and remove ws.service.
    Returns (success, message).
    """
    if not is_systemctl_available():
        if SERVICE_PATH.exists():
            SERVICE_PATH.unlink()
            return True, f"Removed {SERVICE_PATH}."
        return False, "systemctl is not available on this system."

    # Stop and disable
    subprocess.run(["systemctl", "--user", "stop", SERVICE_NAME], check=False, capture_output=True)
    subprocess.run(["systemctl", "--user", "disable", SERVICE_NAME], check=False, capture_output=True)

    if SERVICE_PATH.exists():
        SERVICE_PATH.unlink()

    subprocess.run(["systemctl", "--user", "daemon-reload"], check=False, capture_output=True)
    return True, f"Uninstalled {SERVICE_NAME} successfully."


def control_service(action: str) -> tuple[bool, str]:
    """
    Control ws.service (start, stop, restart, enable, disable).
    Returns (success, message).
    """
    if not is_systemctl_available():
        return False, "systemctl is not available on this system."

    if not SERVICE_PATH.exists() and action in ["start", "restart", "enable"]:
        return False, f"{SERVICE_NAME} is not installed. Run 'ws service install' first."

    res = subprocess.run(
        ["systemctl", "--user", action, SERVICE_NAME],
        check=False,
        capture_output=True,
        text=True,
    )
    if res.returncode != 0:
        err = res.stderr.strip() or res.stdout.strip()
        return False, f"systemctl {action} failed: {err}"

    return True, f"Successfully executed '{action}' on {SERVICE_NAME}."


def get_service_status() -> dict[str, Any]:
    """
    Query systemd status for ws.service.
    Returns structured status dictionary.
    """
    if not is_systemctl_available():
        return {
            "available": False,
            "installed": SERVICE_PATH.exists(),
            "active": False,
            "enabled": False,
            "details": "systemctl is not available",
            "unit_path": str(SERVICE_PATH),
        }

    installed = SERVICE_PATH.exists()
    if not installed:
        return {
            "available": True,
            "installed": False,
            "active": False,
            "enabled": False,
            "details": f"{SERVICE_NAME} is not installed (run 'ws service install')",
            "unit_path": str(SERVICE_PATH),
        }

    is_active_res = subprocess.run(
        ["systemctl", "--user", "is-active", SERVICE_NAME],
        check=False,
        capture_output=True,
        text=True,
    )
    is_active = is_active_res.stdout.strip() == "active"

    is_enabled_res = subprocess.run(
        ["systemctl", "--user", "is-enabled", SERVICE_NAME],
        check=False,
        capture_output=True,
        text=True,
    )
    is_enabled = is_enabled_res.stdout.strip() == "enabled"

    status_res = subprocess.run(
        ["systemctl", "--user", "status", SERVICE_NAME],
        check=False,
        capture_output=True,
        text=True,
    )

    return {
        "available": True,
        "installed": True,
        "active": is_active,
        "enabled": is_enabled,
        "details": status_res.stdout.strip(),
        "unit_path": str(SERVICE_PATH),
    }


def stream_service_logs(follow: bool = True, lines: int = 50) -> int:
    """Stream journalctl logs for ws.service."""
    if not shutil.which("journalctl"):
        print("journalctl is not available on this system.")
        return 1

    cmd = ["journalctl", "--user", "-u", SERVICE_NAME, "-n", str(lines), "--no-pager"]
    if follow:
        cmd.append("-f")

    try:
        return subprocess.call(cmd)
    except KeyboardInterrupt:
        return 0
