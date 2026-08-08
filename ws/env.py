"""Environment variable resolution, dynamic templating, and .env file synchronization engine."""

import logging
import os
from pathlib import Path
import re
import shutil
from typing import Any

from ws.models import AppConfig, RepoConfig

logger = logging.getLogger("ws.env")


class EnvEngine:
    """Engine for resolving scoped & dynamic environment variables and managing worktree .env files."""

    @staticmethod
    def get_workspace_slot(workspaces_dir: Path, workspace_name: str) -> int:
        """Compute deterministic integer slot index for a workspace."""
        if not workspaces_dir.exists():
            return 0

        # Discover all active workspace subdirectories containing workspace.yml
        ws_dirs: list[str] = []
        try:
            for item in sorted(workspaces_dir.iterdir()):
                if item.is_dir() and (item / "workspace.yml").exists():
                    ws_dirs.append(item.name)
        except Exception as e:
            logger.debug("Failed to list workspaces for slot index: %s", e)

        if workspace_name in ws_dirs:
            return ws_dirs.index(workspace_name)
        return len(ws_dirs)

    @classmethod
    def resolve_template_string(
        cls,
        template_str: str,
        workspace_name: str,
        repo_name: str,
        slot: int,
        project_root: Path | None = None,
        workspaces_dir: Path | None = None,
    ) -> str:
        """Resolve dynamic template placeholders in a configuration value."""
        if not isinstance(template_str, str):
            return str(template_str)

        result = template_str

        # Replace standard workspace identifiers
        result = result.replace("${WORKSPACE_NAME}", workspace_name)
        result = result.replace("${WS_NAME}", workspace_name)
        result = result.replace("${REPO_NAME}", repo_name)
        result = result.replace("${REPO}", repo_name)
        result = result.replace("${WORKSPACE_SLOT}", str(slot))
        result = result.replace("${WS_SLOT}", str(slot))

        # Project and Directory paths
        if project_root:
            p_root_str = str(project_root.resolve())
            result = result.replace("${PROJECT_ROOT}", p_root_str)
            result = result.replace("${ROOT_DIR}", p_root_str)
            result = result.replace("${SCRIPTS_DIR}", str(project_root.resolve() / "scripts"))

        if workspaces_dir:
            ws_path = workspaces_dir.resolve() / workspace_name
            result = result.replace("${WORKSPACE_DIR}", str(ws_path))
            result = result.replace("${WS_DIR}", str(ws_path))
            if repo_name:
                wt_path = ws_path / repo_name
                result = result.replace("${WORKTREE_DIR}", str(wt_path))
                result = result.replace("${WT_DIR}", str(wt_path))

        # Dynamic port offset: ${PORT:3000} -> 3000 + slot * 10
        port_pattern = re.compile(r"\$\{PORT:(\d+)\}")

        def port_repl(match: re.Match) -> str:
            base_port = int(match.group(1))
            return str(base_port + slot * 10)

        result = port_pattern.sub(port_repl, result)

        # Custom multiplier: ${PORT_OFFSET:8000:5} -> 8000 + slot * 5
        offset_pattern = re.compile(r"\$\{PORT_OFFSET:(\d+):(\d+)\}")

        def offset_repl(match: re.Match) -> str:
            base_port = int(match.group(1))
            multiplier = int(match.group(2))
            return str(base_port + slot * multiplier)

        result = offset_pattern.sub(offset_repl, result)

        return result

    @classmethod
    def expand_command(
        cls,
        command: str,
        env_vars: dict[str, str],
        workspace_name: str,
        repo_name: str = "",
        slot: int = 0,
        project_root: Path | None = None,
        workspaces_dir: Path | None = None,
    ) -> str:
        """Expand dynamic template placeholders, paths, and environment variables in a command."""
        if not isinstance(command, str):
            return str(command)

        # 1. Resolve standard dynamic template placeholders and path variables
        cmd = cls.resolve_template_string(
            command,
            workspace_name,
            repo_name,
            slot,
            project_root=project_root,
            workspaces_dir=workspaces_dir,
        )

        # 2. Resolve variable references (${KEY}) from env_vars
        for k, v in env_vars.items():
            cmd = cmd.replace(f"${{{k}}}", v)

        # 3. Auto-resolve ./scripts/ or scripts/ to project_root / scripts if not in worktree
        if project_root:
            scripts_dir = project_root / "scripts"
            if scripts_dir.exists() and (cmd.startswith("scripts/") or cmd.startswith("./scripts/")):
                script_rel = cmd.split()[0].lstrip("./")
                candidate = project_root / script_rel
                if candidate.exists():
                    cmd = str(candidate.resolve()) + cmd[len(cmd.split()[0]):]

        return cmd

    @classmethod
    def is_secret_key(cls, key: str, explicit_secrets: Sequence[str] | None = None) -> bool:
        """Check if an environment variable key should be masked as sensitive."""
        if explicit_secrets and key in explicit_secrets:
            return True

        k_upper = key.upper()
        patterns = [
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "TOKEN",
            "AUTH_KEY",
            "PRIVATE_KEY",
            "API_KEY",
            "CREDENTIAL",
            "SIGNING_KEY",
            "CERT_KEY",
        ]
        return any(p in k_upper for p in patterns)

    @classmethod
    def mask_secret_value(cls, value: str) -> str:
        """Return masked representation of a secret value."""
        return "********"

    @classmethod
    def sync_copied_files(
        cls,
        project_root: Path,
        worktree_path: Path,
        copy_files: Sequence[Any],
    ) -> tuple[bool, str]:
        """Copy configured project-level files directly into the worktree.

        Searches inside the 'files/' directory at the project root first,
        falling back to the project root directory.
        """
        if not copy_files:
            return True, ""

        copied_count = 0
        errors: list[str] = []
        files_dir = project_root / "files"

        for item in copy_files:
            if isinstance(item, str):
                src_rel = item
                # If path starts with files/, strip it for destination worktree
                dst_rel = item[6:] if item.startswith("files/") else item
            elif isinstance(item, dict):
                src_rel = item.get("source", item.get("src", ""))
                dst_rel = item.get("dest", item.get("dst", src_rel))
                if not src_rel:
                    # Alternative {"shared/file.json": "config/file.json"}
                    for k, v in item.items():
                        src_rel, dst_rel = k, v
                        break
            else:
                continue

            if not src_rel:
                continue

            # 1. Search inside project_root / 'files' / src_rel first
            src_path: Path | None = None
            if files_dir.exists() and (files_dir / src_rel).exists():
                src_path = files_dir / src_rel
            elif (project_root / src_rel).exists():
                src_path = project_root / src_rel
            elif src_rel.startswith("files/") and (project_root / src_rel).exists():
                src_path = project_root / src_rel

            if not src_path or not src_path.exists():
                errors.append(f"source file not found: '{src_rel}' (checked 'files/{src_rel}' and '{src_rel}')")
                continue

            dst_path = worktree_path / dst_rel

            try:
                dst_path.parent.mkdir(parents=True, exist_ok=True)
                if src_path.is_dir():
                    shutil.copytree(src_path, dst_path, dirs_exist_ok=True)
                else:
                    shutil.copy2(src_path, dst_path)
                copied_count += 1
            except Exception as e:
                errors.append(f"failed copying {src_rel} -> {dst_rel}: {e}")

        if errors:
            return False, "; ".join(errors)
        return True, f"copied {copied_count} file(s)"


    @classmethod
    def resolve_repo_env(
        cls,
        app_config: AppConfig,
        workspace_name: str,
        repo_name: str,
        slot: int | None = None,
    ) -> dict[str, str]:
        """Resolve merged environment dictionary for a specific repository worktree."""
        if slot is None:
            slot = cls.get_workspace_slot(app_config.workspaces_dir, workspace_name)

        merged: dict[str, str] = {}
        p_root = app_config.project_root
        ws_dir = app_config.workspaces_dir

        # 1. Top-level global environment store
        for k, v in app_config.global_env.items():
            merged[k] = cls.resolve_template_string(
                v, workspace_name, repo_name, slot, project_root=p_root, workspaces_dir=ws_dir
            )

        # 2. Top-level dynamic environment store
        for k, v in app_config.dynamic_env.items():
            merged[k] = cls.resolve_template_string(
                v, workspace_name, repo_name, slot, project_root=p_root, workspaces_dir=ws_dir
            )

        # 3. Repository-scoped environment overrides
        if repo_name in app_config.repositories:
            repo_cfg = app_config.repositories[repo_name]
            for k, v in repo_cfg.env.items():
                merged[k] = cls.resolve_template_string(
                    v, workspace_name, repo_name, slot, project_root=p_root, workspaces_dir=ws_dir
                )

        return merged


    @classmethod
    def prepare_and_sync_env_file(
        cls,
        worktree_path: Path,
        env_vars: dict[str, str],
        env_filename: str = ".env",
        example_filename: str = ".env.example",
    ) -> tuple[bool, str]:
        """Execute Step 1 (copy .env.example if missing) and Step 2 (merge/update variables)."""
        target_env = worktree_path / env_filename
        source_example = worktree_path / example_filename

        actions_taken: list[str] = []

        # Step 1: Copy example env if present and target does not exist
        if source_example.exists() and source_example.is_file() and not target_env.exists():
            try:
                shutil.copy2(source_example, target_env)
                actions_taken.append(f"copied {example_filename} -> {env_filename}")
            except Exception as e:
                logger.warning("Failed to copy %s to %s: %s", example_filename, env_filename, e)

        # Step 2: Merge/update variables into target .env file
        if not env_vars:
            if actions_taken:
                return True, ", ".join(actions_taken)
            return True, "no env variables configured"

        existing_lines: list[str] = []
        if target_env.exists():
            try:
                with open(target_env, "r", encoding="utf-8") as f:
                    existing_lines = f.readlines()
            except Exception as e:
                logger.warning("Failed to read %s: %s", target_env, e)

        updated_keys: set[str] = set()
        new_lines: list[str] = []

        for line in existing_lines:
            stripped = line.strip()
            if stripped and not stripped.startswith("#") and "=" in stripped:
                key = stripped.split("=", 1)[0].strip()
                if key in env_vars:
                    # Update value
                    val = env_vars[key]
                    new_lines.append(f"{key}={val}\n")
                    updated_keys.add(key)
                    continue
            new_lines.append(line)

        # Append variables that were not present in existing file
        appended_count = 0
        if new_lines and not new_lines[-1].endswith("\n"):
            new_lines[-1] += "\n"

        for k, v in env_vars.items():
            if k not in updated_keys:
                new_lines.append(f"{k}={v}\n")
                appended_count += 1

        try:
            with open(target_env, "w", encoding="utf-8") as f:
                f.writelines(new_lines)
            actions_taken.append(f"synced {len(env_vars)} variables in {env_filename}")
        except Exception as e:
            return False, f"failed to write {env_filename}: {e}"

        return True, ", ".join(actions_taken)
