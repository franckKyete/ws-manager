"""Configuration file watcher for ws.

Monitors repositories.yml and workspace.yml across all registered projects and
automatically:
1. Pushes updated blueprint revisions to wshub when repositories.yml is edited.
2. Saves workspace state to wshub when workspace.yml is edited.
"""

from __future__ import annotations

import hashlib
import logging
from pathlib import Path
from typing import Any
import yaml

from ws.config import ConfigLoader
from ws.registry import list_registered_projects
from ws.utils import get_iso_timestamp
from ws.workspace import WorkspaceManager

logger = logging.getLogger("ws.watcher")


class ConfigFileWatcher:
    """Watches repositories.yml and workspace.yml files across registered projects."""

    def __init__(self, registry_path: Path | None = None) -> None:
        self.registry_path = registry_path
        self._file_hashes: dict[str, str] = {}
        self._initialized = False

    def _hash_file(self, file_path: Path) -> str | None:
        """Compute SHA-256 hash of file contents, returning None if file is missing."""
        if not file_path.exists() or not file_path.is_file():
            return None
        try:
            content = file_path.read_bytes()
            return hashlib.sha256(content).hexdigest()
        except OSError as e:
            logger.debug("Failed reading file %s: %s", file_path, e)
            return None

    def _is_valid_yaml(self, file_path: Path) -> bool:
        """Check if file currently parses as valid YAML."""
        try:
            with open(file_path, "r", encoding="utf-8") as f:
                data = yaml.safe_load(f)
                return isinstance(data, (dict, list)) or data is None
        except Exception:
            return False

    def initialize(self) -> None:
        """Scan and register initial baseline hashes for all existing config files."""
        project_paths = list_registered_projects(registry_path=self.registry_path, prune_missing=True)
        for p in project_paths:
            cfg_file = p / "repositories.yml"
            if not cfg_file.exists():
                cfg_file = p / "repository.yml"
            if cfg_file.exists():
                h = self._hash_file(cfg_file)
                if h:
                    self._file_hashes[str(cfg_file.resolve())] = h

            ws_root = p / "workspaces"
            if ws_root.exists() and ws_root.is_dir():
                for ws_dir in ws_root.iterdir():
                    if not ws_dir.is_dir():
                        continue
                    ws_file = ws_dir / "workspace.yml"
                    if not ws_file.exists():
                        ws_file = ws_dir / "workspace.yaml"
                    if ws_file.exists():
                        h = self._hash_file(ws_file)
                        if h:
                            self._file_hashes[str(ws_file.resolve())] = h

        self._initialized = True
        logger.debug("ConfigFileWatcher initialized with %d tracked configuration files.", len(self._file_hashes))

    def check_changes(self) -> dict[str, list[str]]:
        """
        Check for any modifications to watched files across all registered projects.
        Triggers hub revision pushes on repositories.yml changes and state saves
        on workspace.yml changes.
        """
        if not self._initialized:
            self.initialize()
            return {"pushed": [], "saved": []}

        project_paths = list_registered_projects(registry_path=self.registry_path, prune_missing=True)
        pushed_projects: list[str] = []
        saved_workspaces: list[str] = []

        seen_paths: set[str] = set()

        for p in project_paths:
            cfg_file = p / "repositories.yml"
            if not cfg_file.exists():
                cfg_file = p / "repository.yml"

            if cfg_file.exists():
                cfg_str = str(cfg_file.resolve())
                seen_paths.add(cfg_str)
                current_hash = self._hash_file(cfg_file)

                if current_hash is not None:
                    last_hash = self._file_hashes.get(cfg_str)
                    if last_hash is None:
                        # New config file discovered
                        self._file_hashes[cfg_str] = current_hash
                    elif current_hash != last_hash:
                        # File modified! Verify valid YAML before pushing
                        if self._is_valid_yaml(cfg_file):
                            self._file_hashes[cfg_str] = current_hash
                            logger.info("[%s] Detected modification to %s. Pushing revision to wshub...", p.name, cfg_file.name)
                            try:
                                config = ConfigLoader.load_config(config_path=cfg_file, allow_empty=True)
                                manager = WorkspaceManager(config=config)
                                rev_result = manager.hub_push(
                                    message=f"Auto-pushed blueprint from {cfg_file.name} edit",
                                    silent=True,
                                )
                                version = rev_result.get("revision", {}).get("version", "?")
                                pushed_projects.append(p.name)
                                logger.info("[%s] Auto-pushed blueprint revision v%s", p.name, version)

                                # Refresh hash in case hub_push updated hub metadata on disk
                                refreshed_hash = self._hash_file(cfg_file)
                                if refreshed_hash:
                                    self._file_hashes[cfg_str] = refreshed_hash

                                auto_cfg = getattr(config, "hub_auto_save", None)
                                should_notify = auto_cfg.notify if auto_cfg and hasattr(auto_cfg, "notify") else True
                                if should_notify:
                                    from ws.notify import notify_blueprint_push_success
                                    namespace, p_name = manager._get_project_namespace_and_name()
                                    notify_blueprint_push_success(f"{namespace}/{p_name}", str(version))
                            except Exception as e:
                                logger.error("[%s] Error pushing revision after %s edit: %s", p.name, cfg_file.name, e)
                                auto_cfg = getattr(config, "hub_auto_save", None) if "config" in locals() else None
                                should_notify = auto_cfg.notify if auto_cfg and hasattr(auto_cfg, "notify") else True
                                if should_notify:
                                    from ws.notify import notify_blueprint_push_failure
                                    notify_blueprint_push_failure(p.name, str(e))
                        else:
                            logger.debug("[%s] %s has invalid/incomplete YAML syntax during edit; skipping push.", p.name, cfg_file.name)

            # Check workspace.yml files
            ws_root = p / "workspaces"
            if ws_root.exists() and ws_root.is_dir():
                for ws_dir in ws_root.iterdir():
                    if not ws_dir.is_dir():
                        continue
                    ws_file = ws_dir / "workspace.yml"
                    if not ws_file.exists():
                        ws_file = ws_dir / "workspace.yaml"
                    if not ws_file.exists():
                        continue

                    ws_str = str(ws_file.resolve())
                    seen_paths.add(ws_str)
                    current_ws_hash = self._hash_file(ws_file)

                    if current_ws_hash is not None:
                        last_ws_hash = self._file_hashes.get(ws_str)
                        if last_ws_hash is None:
                            # New workspace file discovered
                            self._file_hashes[ws_str] = current_ws_hash
                        elif current_ws_hash != last_ws_hash:
                            # File modified! Verify valid YAML before saving
                            if self._is_valid_yaml(ws_file):
                                self._file_hashes[ws_str] = current_ws_hash
                                ws_name = ws_dir.name.lstrip("@")
                                logger.info("[%s] Detected modification to %s in @%s. Saving workspace state to wshub...", p.name, ws_file.name, ws_name)
                                try:
                                    config = ConfigLoader.load_config(config_path=cfg_file, allow_empty=True)
                                    manager = WorkspaceManager(config=config)
                                    if manager.has_workspace(ws_name):
                                        manager.hub_state_save(
                                            workspace_name=ws_name,
                                            silent=True,
                                            is_auto=True,
                                        )
                                        # Update auto-save cache with new fingerprint
                                        fp = manager.get_workspace_fingerprint(ws_name, include_wip=True)
                                        cache = manager._load_auto_save_cache()
                                        cache[ws_name] = {
                                            "fingerprint": fp,
                                            "last_saved_at": get_iso_timestamp(),
                                        }
                                        manager._save_auto_save_cache(cache)
                                        saved_workspaces.append(f"{p.name}@{ws_name}")
                                        logger.info("[%s] Successfully saved state for workspace @%s", p.name, ws_name)

                                        auto_cfg = getattr(config, "hub_auto_save", None)
                                        should_notify = auto_cfg.notify if auto_cfg and hasattr(auto_cfg, "notify") else True
                                        if should_notify:
                                            from ws.notify import notify_auto_save_success
                                            namespace, p_name = manager._get_project_namespace_and_name()
                                            notify_auto_save_success(workspace_name=ws_name, project=f"{namespace}/{p_name}")
                                except Exception as e:
                                    logger.error("[%s] Error saving workspace @%s after %s edit: %s", p.name, ws_name, ws_file.name, e)
                                    auto_cfg = getattr(config, "hub_auto_save", None) if "config" in locals() else None
                                    should_notify = auto_cfg.notify if auto_cfg and hasattr(auto_cfg, "notify") else True
                                    if should_notify:
                                        from ws.notify import notify_auto_save_failure
                                        notify_auto_save_failure(workspace_name=ws_name, error=str(e), project=p.name)
                            else:
                                logger.debug("[%s] %s has invalid/incomplete YAML syntax during edit; skipping save.", p.name, ws_file.name)

        # Prune deleted files
        for p_str in list(self._file_hashes.keys()):
            if p_str not in seen_paths and not Path(p_str).exists():
                del self._file_hashes[p_str]

        return {"pushed": pushed_projects, "saved": saved_workspaces}
