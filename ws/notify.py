"""Desktop notification module using D-Bus."""

import logging
import shutil
import subprocess

logger = logging.getLogger("ws.notify")


def send_dbus_notification(
    summary: str,
    body: str,
    icon: str = "document-save",
    app_name: str = "ws",
    timeout_ms: int = 5000,
) -> bool:
    """Send a desktop notification using D-Bus (org.freedesktop.Notifications).

    Attempts to invoke the Notify method on org.freedesktop.Notifications
    via gdbus first, falling back to notify-send.
    Returns True if sent successfully, False otherwise.
    """
    # 1. Try gdbus (direct D-Bus method call)
    if shutil.which("gdbus"):
        try:
            cmd = [
                "gdbus",
                "call",
                "--session",
                "--dest",
                "org.freedesktop.Notifications",
                "--object-path",
                "/org/freedesktop/Notifications",
                "--method",
                "org.freedesktop.Notifications.Notify",
                app_name,
                "0",
                icon,
                summary,
                body,
                "[]",
                "{}",
                str(timeout_ms),
            ]
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=3)
            if res.returncode == 0:
                logger.debug("D-Bus notification delivered via gdbus: %s", summary)
                return True
            else:
                logger.debug("gdbus call failed (code %d): %s", res.returncode, res.stderr)
        except Exception as e:
            logger.debug("Error calling gdbus: %s", e)

    # 2. Fallback to notify-send
    if shutil.which("notify-send"):
        try:
            cmd = [
                "notify-send",
                "-a",
                app_name,
                "-i",
                icon,
                "-t",
                str(timeout_ms),
                summary,
                body,
            ]
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=3)
            if res.returncode == 0:
                logger.debug("Notification delivered via notify-send: %s", summary)
                return True
            else:
                logger.debug("notify-send failed (code %d): %s", res.returncode, res.stderr)
        except Exception as e:
            logger.debug("Error calling notify-send: %s", e)

    logger.debug("Could not deliver desktop notification (no suitable D-Bus notification tool found).")
    return False


def notify_auto_save_success(workspace_name: str, project: str | None = None) -> bool:
    """Send success notification for workspace auto-save."""
    summary = "wshub Auto-Save"
    if project:
        body = f"Workspace @{workspace_name} successfully saved to {project}"
    else:
        body = f"Workspace @{workspace_name} successfully saved"
    return send_dbus_notification(
        summary=summary,
        body=body,
        icon="document-save",
        timeout_ms=5000,
    )


def notify_auto_save_failure(workspace_name: str, error: str, project: str | None = None) -> bool:
    """Send failure notification for workspace auto-save."""
    summary = "wshub Auto-Save Failed"
    clean_err = str(error).strip()
    if len(clean_err) > 200:
        clean_err = clean_err[:197] + "..."
    body = f"Failed to save @{workspace_name}: {clean_err}"
    return send_dbus_notification(
        summary=summary,
        body=body,
        icon="dialog-error",
        timeout_ms=8000,
    )


def notify_blueprint_push_success(project: str, revision: str | None = None) -> bool:
    """Send success notification when repositories.yml edit pushes a blueprint revision."""
    summary = "wshub Blueprint Updated"
    rev_text = f" (v{revision})" if revision else ""
    body = f"Project blueprint for {project} updated{rev_text}"
    return send_dbus_notification(
        summary=summary,
        body=body,
        icon="document-save",
        timeout_ms=5000,
    )


def notify_blueprint_push_failure(project: str, error: str) -> bool:
    """Send failure notification when repositories.yml blueprint push fails."""
    summary = "wshub Blueprint Push Failed"
    clean_err = str(error).strip()
    if len(clean_err) > 200:
        clean_err = clean_err[:197] + "..."
    body = f"Failed to push blueprint for {project}: {clean_err}"
    return send_dbus_notification(
        summary=summary,
        body=body,
        icon="dialog-error",
        timeout_ms=8000,
    )
