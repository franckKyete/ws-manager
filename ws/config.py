"""Configuration loading and validation module."""

import logging
from pathlib import Path
from typing import Any
import yaml

from ws.exceptions import ConfigException
from ws.models import (
    AppConfig,
    RepoConfig,
    clean_env_val,
    is_private_val,
    is_secret_val,
)

logger = logging.getLogger("ws.config")

DEFAULT_CONFIG_FILENAMES = [
    "repositories.yml",
    "repositories.yaml",
    "ws.yml",
    "ws.yaml",
    ".ws.yml",
]


class ConfigLoader:
    """Loader and validator for application repository configuration."""

    @classmethod
    def find_config_file(cls, explicit_path: Path | str | None = None) -> Path | None:
        """Find configuration file from explicit path, upward directory traversal, or default locations."""
        if explicit_path:
            p = Path(explicit_path).resolve()
            if p.exists() and p.is_file():
                return p
            raise ConfigException(f"Specified config file not found: {explicit_path}")

        # Traverse upwards from current working directory
        curr = Path.cwd().resolve()
        while True:
            for filename in DEFAULT_CONFIG_FILENAMES:
                candidate = curr / filename
                if candidate.exists() and candidate.is_file():
                    return candidate
            if curr.parent == curr:  # Reached filesystem root
                break
            curr = curr.parent

        # Search user home config directory
        user_config_dir = Path.home() / ".config" / "ws"
        for filename in ["repositories.yml", "config.yml", "ws.yml"]:
            candidate = user_config_dir / filename
            if candidate.exists() and candidate.is_file():
                return candidate

        return None

    @classmethod
    def load_config(
        cls,
        config_path: Path | str | None = None,
        workspaces_dir: Path | str | None = None,
        allow_empty: bool = False,
    ) -> AppConfig:
        """Load repository configuration from YAML file or infer defaults."""
        file_path = cls.find_config_file(config_path)

        repos: dict[str, RepoConfig] = {}
        project_root = file_path.parent.resolve() if file_path else Path.cwd().resolve()

        if file_path:
            logger.debug("Loading config from %s", file_path)
            try:
                with open(file_path, "r", encoding="utf-8") as f:
                    data = yaml.safe_load(f) or {}
            except Exception as e:
                raise ConfigException(f"Failed to parse YAML configuration file '{file_path}': {e}") from e

            repos_raw = data.get("repositories")
            if isinstance(repos_raw, dict):
                for name, repo_data in repos_raw.items():
                    if isinstance(repo_data, dict):
                        try:
                            repos[name] = RepoConfig.from_dict(name, repo_data)
                        except Exception as e:
                            raise ConfigException(f"Invalid repository configuration for '{name}': {e}") from e

            # Parse global env, secret, private blocks
            env_raw = data.get("env", {})
            global_env: dict[str, str] = {}
            global_secret_env: dict[str, str] = {}
            global_private_env: dict[str, str] = {}

            if isinstance(env_raw, dict):
                for k, v in env_raw.items():
                    k_str = str(k)
                    if is_secret_val(v):
                        global_secret_env[k_str] = clean_env_val(v)
                    elif is_private_val(v):
                        global_private_env[k_str] = clean_env_val(v)
                    else:
                        global_env[k_str] = clean_env_val(v)

            # Dedicated secret: / secrets: block
            secret_block = data.get("secret", data.get("secrets", {}))
            if isinstance(secret_block, dict):
                for k, v in secret_block.items():
                    global_secret_env[str(k)] = clean_env_val(v)

            # Dedicated private: / local_env: block
            private_block = data.get("private", data.get("local_env", {}))
            if isinstance(private_block, dict):
                for k, v in private_block.items():
                    global_private_env[str(k)] = clean_env_val(v)

            dyn_raw = data.get("dynamic_env", {})
            dynamic_env = {str(k): str(v) for k, v in dyn_raw.items()} if isinstance(dyn_raw, dict) else {}

            setup_raw = data.get("setup", [])
            if isinstance(setup_raw, str):
                global_setup = [setup_raw]
            elif isinstance(setup_raw, list):
                global_setup = [str(s) for s in setup_raw]
            else:
                global_setup = []

            secrets_raw = data.get("secrets", [])
            if isinstance(secrets_raw, str):
                global_secrets = [secrets_raw]
            elif isinstance(secrets_raw, list):
                global_secrets = [str(s) for s in secrets_raw]
            else:
                global_secrets = []

            copy_files_raw = data.get("copy_files", data.get("files", []))
            global_copy_files = list(copy_files_raw) if isinstance(copy_files_raw, list) else ([copy_files_raw] if copy_files_raw else [])
        else:
            global_env = {}
            global_secret_env = {}
            global_private_env = {}
            dynamic_env = {}
            global_setup = []
            global_secrets = []
            global_copy_files = []
            # Fallback auto-detection for .git bare repos in bares/ or current directory
            bares_dir = project_root / "bares"
            bare_dirs = sorted(bares_dir.glob("*.git")) if bares_dir.exists() else []
            if not bare_dirs:
                bare_dirs = sorted(project_root.glob("*.git"))

            if bare_dirs:
                logger.debug("Auto-detecting bare repositories")
                for bare in bare_dirs:
                    repo_name = bare.name[:-4] if bare.name.endswith(".git") else bare.name
                    key = repo_name.lower()
                    repos[key] = RepoConfig(
                        name=key,
                        bare=bare,
                        checkout=repo_name,
                    )

        if not repos and not allow_empty:
            raise ConfigException(
                "No repositories configured. Run 'ws init <git-url...>' to clone repositories or create 'repositories.yml'."
            )

        if workspaces_dir:
            ws_dir_path = Path(workspaces_dir)
            if not ws_dir_path.is_absolute():
                ws_dir_path = project_root / ws_dir_path
        else:
            ws_dir_path = project_root / "workspaces"

        return AppConfig(
            repositories=repos,
            workspaces_dir=ws_dir_path,
            config_file_path=file_path,
            global_env=global_env,
            secret_env=global_secret_env,
            private_env=global_private_env,
            dynamic_env=dynamic_env,
            setup=global_setup,
            secrets=global_secrets,
            copy_files=global_copy_files,
        )

    @classmethod
    def classify_project_assets(
        cls,
        app_config: AppConfig,
    ) -> tuple[str, dict[str, dict[str, str]], list[Path], int]:
        """Classify and sanitize project assets for wshub synchronization.

        Returns:
            - sanitized_blueprint_yaml: Public YAML with secrets as placeholders and private vars omitted.
            - extracted_secrets: Dict mapping scope ('global' or repo_name) to secret key/value pairs.
            - files_to_upload: List of Path objects to encrypt and upload to vault.
            - private_vars_count: Number of private variables omitted.
        """
        extracted_secrets: dict[str, dict[str, str]] = {}
        private_vars_count = 0

        # 1. Global secrets & private
        if app_config.secret_env:
            extracted_secrets["global"] = dict(app_config.secret_env)
        private_vars_count += len(app_config.private_env)

        # 2. Repo-level secrets & private
        sanitized_repos: dict[str, Any] = {}
        for r_name, r_cfg in app_config.repositories.items():
            r_dict = r_cfg.to_dict()
            # Remove private and secret from plain dict
            r_dict.pop("private", None)
            r_dict.pop("secret", None)

            if r_cfg.secret_env:
                extracted_secrets[r_name] = dict(r_cfg.secret_env)
                # In blueprint, mark secret keys with placeholder "secret"
                r_dict_env = dict(r_dict.get("env", {}))
                for s_key in r_cfg.secret_env:
                    r_dict_env[s_key] = "secret"
                r_dict["env"] = r_dict_env

            private_vars_count += len(r_cfg.private_env)
            sanitized_repos[r_name] = r_dict

        # 3. Build sanitized blueprint data structure
        sanitized_data: dict[str, Any] = {}
        if app_config.global_env or app_config.secret_env:
            sanitized_env = dict(app_config.global_env)
            for s_key in app_config.secret_env:
                sanitized_env[s_key] = "secret"
            sanitized_data["env"] = sanitized_env

        if app_config.dynamic_env:
            sanitized_data["dynamic_env"] = dict(app_config.dynamic_env)
        if app_config.setup:
            sanitized_data["setup"] = list(app_config.setup)
        if app_config.copy_files:
            sanitized_data["copy_files"] = list(app_config.copy_files)

        sanitized_data["repositories"] = sanitized_repos
        sanitized_yaml = yaml.dump(sanitized_data, sort_keys=False, default_flow_style=False)

        # 4. Collect sensitive files to encrypt & upload
        files_to_upload: list[Path] = []
        files_dir = app_config.project_root / "files"
        if files_dir.exists() and files_dir.is_dir():
            for f_path in files_dir.rglob("*"):
                if f_path.is_file():
                    files_to_upload.append(f_path)

        # Check explicit copy_files references
        for cf in app_config.copy_files:
            if isinstance(cf, dict) and "from" in cf:
                src_p = app_config.project_root / cf["from"]
                if src_p.exists() and src_p.is_file() and src_p not in files_to_upload:
                    files_to_upload.append(src_p)

        for r_cfg in app_config.repositories.values():
            for cf in r_cfg.copy_files:
                if isinstance(cf, dict) and "from" in cf:
                    src_p = app_config.project_root / cf["from"]
                    if src_p.exists() and src_p.is_file() and src_p not in files_to_upload:
                        files_to_upload.append(src_p)

        return sanitized_yaml, extracted_secrets, files_to_upload, private_vars_count

    @classmethod
    def save_config(cls, repositories: dict[str, RepoConfig], config_path: Path | str | None = None) -> Path:
        """Save repositories dictionary to YAML configuration file."""
        target_path = Path(config_path) if config_path else Path("repositories.yml")
        data = {
            "repositories": {k: v.to_dict() for k, v in repositories.items()}
        }
        with open(target_path, "w", encoding="utf-8") as f:
            yaml.dump(data, f, sort_keys=False, default_flow_style=False)
        return target_path

