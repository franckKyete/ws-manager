import subprocess
from unittest.mock import MagicMock, patch
import pytest

from ws.notify import (
    notify_auto_save_failure,
    notify_auto_save_success,
    send_dbus_notification,
)


def test_send_dbus_notification_gdbus_success():
    with patch("shutil.which", side_effect=lambda cmd: "/usr/bin/gdbus" if cmd == "gdbus" else None):
        with patch("subprocess.run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=["gdbus"], returncode=0, stdout="(uint32 1,)", stderr=""
            )
            result = send_dbus_notification(
                summary="Test Summary",
                body="Test Body",
                icon="document-save",
                timeout_ms=5000,
            )
            assert result is True
            mock_run.assert_called_once()
            called_args = mock_run.call_args[0][0]
            assert called_args[0] == "gdbus"
            assert "org.freedesktop.Notifications" in called_args
            assert "Test Summary" in called_args
            assert "Test Body" in called_args
            assert "document-save" in called_args


def test_send_dbus_notification_fallback_to_notify_send():
    def fake_which(cmd):
        if cmd == "gdbus":
            return "/usr/bin/gdbus"
        if cmd == "notify-send":
            return "/usr/bin/notify-send"
        return None

    def fake_run(cmd, **kwargs):
        if cmd[0] == "gdbus":
            return subprocess.CompletedProcess(args=cmd, returncode=1, stdout="", stderr="error")
        if cmd[0] == "notify-send":
            return subprocess.CompletedProcess(args=cmd, returncode=0, stdout="", stderr="")
        raise FileNotFoundError()

    with patch("shutil.which", side_effect=fake_which):
        with patch("subprocess.run", side_effect=fake_run) as mock_run:
            result = send_dbus_notification(
                summary="Fallback Summary",
                body="Fallback Body",
                icon="dialog-error",
                timeout_ms=8000,
            )
            assert result is True
            assert mock_run.call_count == 2


def test_send_dbus_notification_handles_exceptions_gracefully():
    with patch("shutil.which", return_value="/usr/bin/gdbus"):
        with patch("subprocess.run", side_effect=OSError("Command failed")):
            result = send_dbus_notification(
                summary="Err Summary",
                body="Err Body",
            )
            assert result is False


def test_send_dbus_notification_no_tools_found():
    with patch("shutil.which", return_value=None):
        result = send_dbus_notification(
            summary="No tools",
            body="No tools found",
        )
        assert result is False


def test_notify_auto_save_success():
    with patch("ws.notify.send_dbus_notification", return_value=True) as mock_send:
        res = notify_auto_save_success(workspace_name="dev", project="kyete/ws")
        assert res is True
        mock_send.assert_called_once_with(
            summary="wshub Auto-Save",
            body="Workspace @dev successfully saved to kyete/ws",
            icon="document-save",
            timeout_ms=5000,
        )


def test_notify_auto_save_failure():
    with patch("ws.notify.send_dbus_notification", return_value=True) as mock_send:
        res = notify_auto_save_failure(workspace_name="dev", error="Connection refused", project="kyete/ws")
        assert res is True
        mock_send.assert_called_once_with(
            summary="wshub Auto-Save Failed",
            body="Failed to save @dev: Connection refused",
            icon="dialog-error",
            timeout_ms=8000,
        )


def test_notify_auto_save_failure_truncates_long_error():
    long_error = "x" * 250
    with patch("ws.notify.send_dbus_notification", return_value=True) as mock_send:
        notify_auto_save_failure(workspace_name="dev", error=long_error)
        mock_send.assert_called_once()
        called_body = mock_send.call_args[1]["body"]
        assert called_body.endswith("...")
        assert len(called_body) <= len("Failed to save @dev: ") + 200
