"""Global project registry management for ws.

Maintains a machine-wide registry of known project root directories in
~/.config/ws/projects.yml so background daemons and service managers can discover
and manage all projects on the system.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any
import yaml

from ws.utils import ensure_directory

logger = logging.getLogger("ws.registry")

DEFAULT_REGISTRY_PATH = Path.home() / ".config" / "ws" / "projects.yml"


def get_registry_path() -> Path:
    """Return path to global project registry file."""
    return DEFAULT_REGISTRY_PATH


def load_registry(registry_path: Path | None = None) -> list[dict[str, Any]]:
    """Load list of registered project records from YAML registry."""
    path = registry_path or get_registry_path()
    if not path.exists():
        return []

    try:
        with open(path, "r", encoding="utf-8") as f:
            data = yaml.safe_load(f)
            if isinstance(data, dict) and "projects" in data and isinstance(data["projects"], list):
                return data["projects"]
            if isinstance(data, list):
                return data
    except Exception as e:
        logger.warning("Failed reading projects registry at %s: %s", path, e)

    return []


def save_registry(projects: list[dict[str, Any]], registry_path: Path | None = None) -> None:
    """Persist list of project records to YAML registry."""
    path = registry_path or get_registry_path()
    ensure_directory(path.parent)

    data = {"projects": projects}
    with open(path, "w", encoding="utf-8") as f:
        yaml.safe_dump(data, f, sort_keys=False)


def register_project(project_root: Path | str, registry_path: Path | None = None) -> bool:
    """
    Register a project root in the global registry.
    Returns True if newly added, False if already present.
    """
    path = Path(project_root).resolve()
    if not path.exists() or not path.is_dir():
        logger.debug("Cannot register non-existent directory: %s", path)
        return False

    path_str = str(path)
    # Ignore temporary test directories when writing to the default user registry
    target_reg = Path(registry_path or get_registry_path()).resolve()
    user_reg = (Path.home() / ".config" / "ws" / "projects.yml").resolve()
    if target_reg == user_reg and (path_str.startswith("/tmp") or "/.tmp" in path_str or "/pytest-" in path_str):
        return False

    records = load_registry(registry_path)
    for item in records:
        if isinstance(item, dict) and item.get("path") == path_str:
            return False  # Already registered

    records.append({
        "path": path_str,
        "name": path.name,
    })

    save_registry(records, registry_path)
    logger.debug("Registered project '%s' at %s", path.name, path_str)
    return True


def unregister_project(project_root: Path | str, registry_path: Path | None = None) -> bool:
    """
    Remove a project root from the global registry.
    Returns True if removed, False if not found.
    """
    path = Path(project_root).resolve()
    records = load_registry(registry_path)
    path_str = str(path)

    original_len = len(records)
    records = [item for item in records if isinstance(item, dict) and item.get("path") != path_str]

    if len(records) < original_len:
        save_registry(records, registry_path)
        logger.debug("Unregistered project at %s", path_str)
        return True

    return False


def list_registered_projects(
    registry_path: Path | None = None,
    prune_missing: bool = True,
) -> list[Path]:
    """
    Return a list of resolved Paths to registered projects.
    If prune_missing is True, directories that no longer exist are pruned from registry.
    """
    records = load_registry(registry_path)
    valid_paths: list[Path] = []
    pruned_records: list[dict[str, Any]] = []
    changed = False

    for item in records:
        if not isinstance(item, dict) or "path" not in item:
            changed = True
            continue

        p = Path(item["path"]).resolve()
        if p.exists() and p.is_dir():
            valid_paths.append(p)
            pruned_records.append(item)
        else:
            changed = True
            logger.info("Pruned non-existent project directory: %s", p)

    if prune_missing and changed:
        save_registry(pruned_records, registry_path)

    return valid_paths
