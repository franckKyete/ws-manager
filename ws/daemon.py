"""Global ws background daemon.

Runs continuous background workers across all registered projects, including
periodic Hub auto-saving. Designed to be supervised by systemd (ws.service)
or executed standalone.
"""

from __future__ import annotations

import json
import logging
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from typing import Any

from ws.config import ConfigLoader
from ws.exceptions import WSException
from ws.registry import list_registered_projects, register_project
from ws.utils import ensure_directory, get_iso_timestamp
from ws.workspace import WorkspaceManager

logger = logging.getLogger("ws.daemon")

GLOBAL_CONFIG_DIR = Path.home() / ".config" / "ws"
DAEMON_PID_FILE = GLOBAL_CONFIG_DIR / "daemon.pid"
DAEMON_LOG_FILE = GLOBAL_CONFIG_DIR / "daemon.log"
GLOBAL_CACHE_FILE = GLOBAL_CONFIG_DIR / "global_auto_save_cache.json"


def get_global_cache() -> dict[str, Any]:
    """Load persistent global state and timestamp cache."""
    if GLOBAL_CACHE_FILE.exists():
        try:
            with open(GLOBAL_CACHE_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception as e:
            logger.warning("Failed loading global cache: %s", e)
    return {}


def save_global_cache(cache: dict[str, Any]) -> None:
    """Save persistent global state cache."""
    try:
        ensure_directory(GLOBAL_CACHE_FILE.parent)
        with open(GLOBAL_CACHE_FILE, "w", encoding="utf-8") as f:
            json.dump(cache, f, indent=2)
    except Exception as e:
        logger.warning("Failed saving global cache: %s", e)


def is_standalone_daemon_running() -> tuple[bool, int | None]:
    """Check if standalone daemon process is alive. Returns (active, pid)."""
    if not DAEMON_PID_FILE.exists():
        return False, None
    try:
        pid = int(DAEMON_PID_FILE.read_text(encoding="utf-8").strip())
        os.kill(pid, 0)
        return True, pid
    except (ValueError, ProcessLookupError):
        try:
            DAEMON_PID_FILE.unlink(missing_ok=True)
        except OSError:
            pass
        return False, None
    except PermissionError:
        return True, pid
    except Exception:
        return False, None


class GlobalAutoSaveWorker:
    """Worker responsible for periodic state auto-saving across all registered projects."""

    def __init__(self) -> None:
        self.cache = get_global_cache()

    def process_all_projects(self) -> dict[str, dict[str, bool]]:
        """
        Scan all registered projects and auto-save those with auto-save enabled
        whose interval has elapsed.
        """
        project_paths = list_registered_projects(prune_missing=True)
        results: dict[str, dict[str, bool]] = {}
        now = time.time()
        cache_updated = False

        for p in project_paths:
            cfg_file = p / "repositories.yml"
            if not cfg_file.exists():
                continue

            try:
                config = ConfigLoader.load_config(config_path=cfg_file, allow_empty=True)
            except Exception as e:
                logger.debug("Failed loading config for %s: %s", p, e)
                continue

            auto_cfg = getattr(config, "hub_auto_save", None)
            if not auto_cfg or not auto_cfg.enabled:
                continue

            interval = auto_cfg.interval if auto_cfg.interval > 0 else 300
            p_str = str(p)
            p_entry = self.cache.setdefault(p_str, {})
            last_checked = p_entry.get("last_checked_at", 0)

            # Check if this project is due for auto-save
            if now - last_checked < interval:
                continue

            p_entry["last_checked_at"] = now
            cache_updated = True

            try:
                manager = WorkspaceManager(config=config)
                ws_results = manager.hub_auto_save_all_workspaces(silent=True)
                results[p.name] = ws_results

                saved = [w for w, s in ws_results.items() if s]
                if saved:
                    logger.info("[%s] Auto-saved workspaces: %s", p.name, ", ".join(f"@{w}" for w in saved))
                    p_entry["last_saved_at"] = get_iso_timestamp()
                else:
                    logger.debug("[%s] Check complete: all workspaces up to date.", p.name)
            except Exception as e:
                logger.error("[%s] Auto-save error: %s", p.name, e)

        if cache_updated:
            save_global_cache(self.cache)

        return results


def run_daemon_loop(tick_seconds: int = 15) -> None:
    """
    Main loop for ws background daemon.
    Executes worker passes every tick_seconds.
    """
    # Configure unbuffered timestamped logging
    root_logger = logging.getLogger()
    if not root_logger.handlers:
        logging.basicConfig(
            level=logging.INFO,
            format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
            datefmt="%Y-%m-%d %H:%M:%S",
        )
    else:
        root_logger.setLevel(logging.INFO)

    logger.info("Starting ws global daemon (tick: %ds)...", tick_seconds)
    auto_save_worker = GlobalAutoSaveWorker()

    from ws.watcher import ConfigFileWatcher
    file_watcher = ConfigFileWatcher()
    file_watcher.initialize()

    running = True

    def _shutdown(signum, frame):
        nonlocal running
        sig_name = "SIGTERM" if signum == signal.SIGTERM else "SIGINT"
        logger.info("Daemon received %s; shutting down cleanly...", sig_name)
        running = False

    signal.signal(signal.SIGTERM, _shutdown)
    signal.signal(signal.SIGINT, _shutdown)

    # Initial check on startup
    try:
        auto_save_worker.process_all_projects()
    except Exception as e:
        logger.error("Error during initial daemon pass: %s", e)

    last_auto_save_run = time.time()

    while running:
        try:
            # 1. Check for immediate config / workspace file modifications (every 1s)
            file_watcher.check_changes()

            # 2. Check for periodic auto-save passes
            now = time.time()
            if now - last_auto_save_run >= tick_seconds:
                auto_save_worker.process_all_projects()
                last_auto_save_run = now

            time.sleep(1)
        except Exception as e:
            logger.error("Unhandled error in daemon loop: %s", e)

    logger.info("ws global daemon stopped.")


def start_standalone_daemon(detached: bool = True) -> int:
    """Start standalone daemon process (used when systemd is not active)."""
    active, existing_pid = is_standalone_daemon_running()
    if active and existing_pid != os.getpid():
        raise WSException(f"ws daemon is already running (PID {existing_pid}).")

    if detached:
        ensure_directory(DAEMON_PID_FILE.parent)
        cmd = [
            sys.executable,
            "-u",
            "-m",
            "ws.cli",
            "daemon",
            "run",
        ]
        with open(DAEMON_LOG_FILE, "a", encoding="utf-8") as out:
            proc = subprocess.Popen(
                cmd,
                stdout=out,
                stderr=out,
                start_new_session=True,
            )
        DAEMON_PID_FILE.write_text(str(proc.pid), encoding="utf-8")
        return proc.pid
    else:
        ensure_directory(DAEMON_PID_FILE.parent)
        DAEMON_PID_FILE.write_text(str(os.getpid()), encoding="utf-8")
        try:
            run_daemon_loop()
        finally:
            if DAEMON_PID_FILE.exists():
                try:
                    if DAEMON_PID_FILE.read_text(encoding="utf-8").strip() == str(os.getpid()):
                        DAEMON_PID_FILE.unlink(missing_ok=True)
                except Exception:
                    pass
        return os.getpid()


def stop_standalone_daemon() -> bool:
    """Stop running standalone daemon."""
    active, pid = is_standalone_daemon_running()
    if not active or pid is None:
        return False
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass
    try:
        DAEMON_PID_FILE.unlink(missing_ok=True)
    except OSError:
        pass
    return True
