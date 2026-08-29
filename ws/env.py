from datetime import datetime, timezone
import json
import logging
import os
from pathlib import Path
import re
import shutil
import stat
from typing import Any, Sequence

from ws.models import AppConfig, RepoConfig
from ws.network import (
    allocate_workspace_ports,
    compute_preferred_service_port,
    get_lan_ip,
)

logger = logging.getLogger("ws.env")


class EnvEngine:
    """Engine for resolving scoped & dynamic environment variables, multi-network discovery, and .env files."""

    @staticmethod
    def get_workspace_slot(workspaces_dir: Path, workspace_name: str) -> int:
        """Compute deterministic integer slot index for a workspace."""
        if not workspaces_dir.exists():
            return 0

        # Discover all active workspace subdirectories containing workspace.yml or .ws
        ws_dirs: list[str] = []
        try:
            for item in sorted(workspaces_dir.iterdir()):
                if item.is_dir() and ((item / "workspace.yml").exists() or (item / ".ws").exists()):
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
        service_ports: dict[str, int] | None = None,
        lan_ip: str | None = None,
        public_host: str | None = None,
        interface: str | None = None,
    ) -> str:
        """Resolve dynamic template placeholders in a configuration value."""
        if not isinstance(template_str, str):
            return str(template_str)

        result = template_str
        resolved_lan_ip = lan_ip or get_lan_ip(preferred_interface=interface)
        resolved_public_host = public_host or os.environ.get("WS_PUBLIC_HOST") or os.environ.get("PUBLIC_HOST") or resolved_lan_ip

        # 1. Standard workspace identifiers

        result = result.replace("${WORKSPACE_NAME}", workspace_name)
        result = result.replace("${WS_NAME}", workspace_name)
        result = result.replace("${REPO_NAME}", repo_name)
        result = result.replace("${REPO}", repo_name)
        result = result.replace("${WORKSPACE_SLOT}", str(slot))
        result = result.replace("${WS_SLOT}", str(slot))
        result = result.replace("${LAN_IP}", resolved_lan_ip)
        result = result.replace("${WS_LAN_IP}", resolved_lan_ip)
        result = result.replace("${HOST_LAN_IP}", resolved_lan_ip)
        result = result.replace("${PUBLIC_HOST}", resolved_public_host)
        result = result.replace("${WS_PUBLIC_HOST}", resolved_public_host)

        # 2. Project and Directory paths
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

        # 3. Dynamic service discovery placeholders (${SERVICE_PORT:<name>}, ${SERVICE_URL:<name>}, etc.)
        ports_map = service_ports or {}

        # Pattern: ${SERVICE_PORT:repo_name}
        svc_port_pattern = re.compile(r"\$\{SERVICE_PORT:([a-zA-Z0-9_-]+)\}")
        def svc_port_repl(m: re.Match) -> str:
            target_svc = m.group(1)
            if target_svc in ports_map:
                return str(ports_map[target_svc])
            return str(compute_preferred_service_port(0, slot))
        result = svc_port_pattern.sub(svc_port_repl, result)

        # Pattern: ${SERVICE_URL_LAN:repo_name}
        svc_url_lan_pattern = re.compile(r"\$\{SERVICE_URL_LAN:([a-zA-Z0-9_-]+)\}")
        def svc_url_lan_repl(m: re.Match) -> str:
            target_svc = m.group(1)
            port_val = ports_map.get(target_svc, compute_preferred_service_port(0, slot))
            return f"http://{resolved_lan_ip}:{port_val}"
        result = svc_url_lan_pattern.sub(svc_url_lan_repl, result)

        # Pattern: ${SERVICE_URL_PUBLIC:repo_name}
        svc_url_public_pattern = re.compile(r"\$\{SERVICE_URL_PUBLIC:([a-zA-Z0-9_-]+)\}")
        def svc_url_public_repl(m: re.Match) -> str:
            target_svc = m.group(1)
            port_val = ports_map.get(target_svc, compute_preferred_service_port(0, slot))
            scheme = "https" if "https://" in resolved_public_host else "http"
            clean_host = resolved_public_host.replace("https://", "").replace("http://", "")
            return f"{scheme}://{clean_host}:{port_val}"
        result = svc_url_public_pattern.sub(svc_url_public_repl, result)

        # Pattern: ${SERVICE_URL:repo_name} / ${SERVICE_URL_LOCAL:repo_name}
        svc_url_pattern = re.compile(r"\$\{SERVICE_URL(_LOCAL)?:([a-zA-Z0-9_-]+)\}")
        def svc_url_repl(m: re.Match) -> str:
            target_svc = m.group(2)
            port_val = ports_map.get(target_svc, compute_preferred_service_port(0, slot))
            return f"http://127.0.0.1:{port_val}"
        result = svc_url_pattern.sub(svc_url_repl, result)

        # Pattern: ${SERVICE_HOST_LAN:repo_name}
        result = re.sub(r"\$\{SERVICE_HOST_LAN:([a-zA-Z0-9_-]+)\}", resolved_lan_ip, result)
        # Pattern: ${SERVICE_HOST_PUBLIC:repo_name}
        result = re.sub(r"\$\{SERVICE_HOST_PUBLIC:([a-zA-Z0-9_-]+)\}", resolved_public_host, result)
        # Pattern: ${SERVICE_HOST:repo_name} / ${SERVICE_HOST_LOCAL:repo_name}
        result = re.sub(r"\$\{SERVICE_HOST(_LOCAL)?:([a-zA-Z0-9_-]+)\}", "127.0.0.1", result)

        # 4. Dynamic port offset: ${PORT:3000} -> 3000 + slot * 10
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

        # 5. Environment variable fallback: ${ENV:VAR_NAME:-default_value}
        env_fallback_pattern = re.compile(r"\$\{ENV:([a-zA-Z0-9_]+)(?::-([^}]*))?\}")
        def env_fallback_repl(match: re.Match) -> str:
            var_name = match.group(1)
            default_val = match.group(2) if match.group(2) is not None else ""
            return os.environ.get(var_name, default_val)
        result = env_fallback_pattern.sub(env_fallback_repl, result)

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
        service_ports: dict[str, int] | None = None,
        lan_ip: str | None = None,
        public_host: str | None = None,
        interface: str | None = None,
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
            service_ports=service_ports,
            lan_ip=lan_ip,
            public_host=public_host,
            interface=interface,
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
    def write_service_discovery_files(
        cls,
        workspace_dir: Path,
        workspace_name: str,
        slot: int,
        service_ports: dict[str, int],
        running_pids: dict[str, int] | None = None,
        public_host: str | None = None,
        lan_ip: str | None = None,
        interface: str | None = None,
    ) -> Path:
        """Write .ws/services.json and .ws/services.env for runtime zero-config service discovery."""
        ws_meta_dir = workspace_dir / ".ws"
        ws_meta_dir.mkdir(parents=True, exist_ok=True)
        resolved_lan_ip = lan_ip or get_lan_ip(preferred_interface=interface)
        pub_host = public_host or os.environ.get("WS_PUBLIC_HOST") or os.environ.get("PUBLIC_HOST") or resolved_lan_ip
        running_pids = running_pids or {}


        services_data = {}
        env_lines = [
            f"# Auto-generated service discovery for workspace @{workspace_name}\n",
            f"WS_WORKSPACE={workspace_name}\n",
            f"WS_SLOT={slot}\n",
            f"WS_LAN_IP={resolved_lan_ip}\n",
            f"WS_PUBLIC_HOST={pub_host}\n\n",
        ]

        for s_name, s_port in sorted(service_ports.items()):
            s_upper = s_name.upper().replace("-", "_")
            url_local = f"http://127.0.0.1:{s_port}"
            url_lan = f"http://{resolved_lan_ip}:{s_port}"
            url_pub = f"http://{pub_host}:{s_port}"

            services_data[s_name] = {
                "port": s_port,
                "url": url_local,
                "url_local": url_local,
                "url_lan": url_lan,
                "url_public": url_pub,
                "host_local": "127.0.0.1",
                "host_lan": resolved_lan_ip,
                "host_public": pub_host,
                "pid": running_pids.get(s_name),
            }

            env_lines.append(f"WS_SERVICE_{s_upper}_PORT={s_port}\n")
            env_lines.append(f"WS_SERVICE_{s_upper}_URL={url_local}\n")
            env_lines.append(f"WS_SERVICE_{s_upper}_URL_LOCAL={url_local}\n")
            env_lines.append(f"WS_SERVICE_{s_upper}_URL_LAN={url_lan}\n")
            env_lines.append(f"WS_SERVICE_{s_upper}_URL_PUBLIC={url_pub}\n")
            env_lines.append(f"WS_SERVICE_{s_upper}_HOST=127.0.0.1\n")
            env_lines.append(f"WS_SERVICE_{s_upper}_HOST_LAN={resolved_lan_ip}\n\n")

        json_path = ws_meta_dir / "services.json"
        descriptor = {
            "workspace": workspace_name,
            "slot": slot,
            "lan_ip": resolved_lan_ip,
            "public_host": pub_host,
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "services": services_data,
        }


        try:
            with open(json_path, "w", encoding="utf-8") as f:
                json.dump(descriptor, f, indent=2)
        except Exception as e:
            logger.warning("Failed writing %s: %s", json_path, e)

        env_path = ws_meta_dir / "services.env"
        try:
            with open(env_path, "w", encoding="utf-8") as f:
                f.writelines(env_lines)
        except Exception as e:
            logger.warning("Failed writing %s: %s", env_path, e)

        return json_path

    @classmethod
    def read_service_discovery_descriptor(cls, workspace_dir: Path) -> dict[str, Any] | None:
        """Read .ws/services.json from workspace directory if it exists."""
        json_path = workspace_dir / ".ws" / "services.json"
        if json_path.exists():
            try:
                with open(json_path, "r", encoding="utf-8") as f:
                    return json.load(f)
            except Exception as e:
                logger.debug("Failed reading %s: %s", json_path, e)
        return None

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
        """Copy configured project-level files directly into the worktree."""
        if not copy_files:
            return True, ""

        copied_count = 0
        errors: list[str] = []
        files_dir = project_root / "files"

        for item in copy_files:
            if isinstance(item, str):
                src_rel = item
                dst_rel = item[6:] if item.startswith("files/") else item
            elif isinstance(item, dict):
                src_rel = item.get("source", item.get("src", ""))
                dst_rel = item.get("dest", item.get("dst", src_rel))
                if not src_rel:
                    for k, v in item.items():
                        src_rel, dst_rel = k, v
                        break
            else:
                continue

            if not src_rel:
                continue

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
                    try:
                        dst_mode = dst_path.stat().st_mode
                        os.chmod(dst_path, dst_mode | stat.S_IWUSR | stat.S_IRUSR)
                    except Exception as e:
                        logger.debug("Failed ensuring write permission on %s: %s", dst_path, e)
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
        service_ports: dict[str, int] | None = None,
        lan_ip: str | None = None,
        public_host: str | None = None,
        interface: str | None = None,
    ) -> dict[str, str]:
        """Resolve merged environment dictionary for a specific repository worktree with full service discovery."""
        if slot is None:
            slot = cls.get_workspace_slot(app_config.workspaces_dir, workspace_name)

        merged: dict[str, str] = {}
        p_root = app_config.project_root
        ws_dir = app_config.workspaces_dir
        resolved_lan_ip = lan_ip or get_lan_ip(preferred_interface=interface)
        resolved_public_host = (
            public_host
            or app_config.global_env.get("PUBLIC_HOST")
            or os.environ.get("WS_PUBLIC_HOST")
            or os.environ.get("PUBLIC_HOST")
            or os.environ.get("TUNNEL_HOST")
            or resolved_lan_ip
        )

        # Discover or compute service ports
        ports_map = service_ports
        if ports_map is None:
            descriptor = None
            if ws_dir:
                descriptor = cls.read_service_discovery_descriptor(ws_dir / workspace_name)
            if descriptor and "services" in descriptor:
                ports_map = {s_k: s_v["port"] for s_k, s_v in descriptor["services"].items() if "port" in s_v}
            else:
                ports_map, _ = allocate_workspace_ports(app_config.repositories, slot)

        # 0. Automatically inject core workspace & discovery variables
        merged["WS_WORKSPACE"] = workspace_name
        merged["WS_SLOT"] = str(slot)
        merged["WS_LAN_IP"] = resolved_lan_ip
        merged["WS_PUBLIC_HOST"] = resolved_public_host

        # Auto-inject all sibling services' discovery variables
        for s_k, s_port in sorted(ports_map.items()):
            s_upper = s_k.upper().replace("-", "_")
            merged[f"WS_SERVICE_{s_upper}_PORT"] = str(s_port)
            merged[f"WS_SERVICE_{s_upper}_URL"] = f"http://127.0.0.1:{s_port}"
            merged[f"WS_SERVICE_{s_upper}_URL_LOCAL"] = f"http://127.0.0.1:{s_port}"
            merged[f"WS_SERVICE_{s_upper}_URL_LAN"] = f"http://{resolved_lan_ip}:{s_port}"
            merged[f"WS_SERVICE_{s_upper}_URL_PUBLIC"] = f"http://{resolved_public_host}:{s_port}"
            merged[f"WS_SERVICE_{s_upper}_HOST"] = "127.0.0.1"
            merged[f"WS_SERVICE_{s_upper}_HOST_LAN"] = resolved_lan_ip
            merged[f"WS_SERVICE_{s_upper}_HOST_PUBLIC"] = resolved_public_host

        # 1. Top-level global environment store (public, secret, private)
        all_global_env = {}
        all_global_env.update(app_config.global_env)
        all_global_env.update(app_config.secret_env)
        all_global_env.update(app_config.private_env)

        for k, v in all_global_env.items():
            merged[k] = cls.resolve_template_string(
                v,
                workspace_name,
                repo_name,
                slot,
                project_root=p_root,
                workspaces_dir=ws_dir,
                service_ports=ports_map,
                lan_ip=resolved_lan_ip,
                public_host=resolved_public_host,
                interface=interface,
            )

        # 2. Top-level dynamic environment store
        for k, v in app_config.dynamic_env.items():
            merged[k] = cls.resolve_template_string(
                v,
                workspace_name,
                repo_name,
                slot,
                project_root=p_root,
                workspaces_dir=ws_dir,
                service_ports=ports_map,
                lan_ip=resolved_lan_ip,
                public_host=resolved_public_host,
                interface=interface,
            )

        # 3. Repository-scoped environment overrides (public, secret, private)
        if repo_name in app_config.repositories:
            repo_cfg = app_config.repositories[repo_name]
            all_repo_env = {}
            all_repo_env.update(repo_cfg.env)
            all_repo_env.update(repo_cfg.secret_env)
            all_repo_env.update(repo_cfg.private_env)

            for k, v in all_repo_env.items():
                merged[k] = cls.resolve_template_string(
                    v,
                    workspace_name,
                    repo_name,
                    slot,
                    project_root=p_root,
                    workspaces_dir=ws_dir,
                    service_ports=ports_map,
                    lan_ip=resolved_lan_ip,
                    public_host=resolved_public_host,
                    interface=interface,
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
                # Ensure copied .env is always writable even if source .env.example was read-only
                try:
                    mode = target_env.stat().st_mode
                    os.chmod(target_env, mode | stat.S_IWUSR | stat.S_IRUSR)
                except Exception as pe:
                    logger.debug("Failed setting write permission on %s: %s", target_env, pe)
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
            # Ensure target .env has write permissions before attempting to read/update
            try:
                mode = target_env.stat().st_mode
                if not (mode & stat.S_IWUSR):
                    os.chmod(target_env, mode | stat.S_IWUSR)
            except Exception as pe:
                logger.debug("Failed restoring write permissions on %s: %s", target_env, pe)

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
            if target_env.exists():
                try:
                    mode = target_env.stat().st_mode
                    if not (mode & stat.S_IWUSR):
                        os.chmod(target_env, mode | stat.S_IWUSR)
                except Exception:
                    pass

            with open(target_env, "w", encoding="utf-8") as f:
                f.writelines(new_lines)
            actions_taken.append(f"synced {len(env_vars)} variables in {env_filename}")
        except Exception as e:
            return False, f"failed to write {env_filename}: {e}"

        return True, ", ".join(actions_taken)

