"""Configuration loading and validation module."""

import logging
from pathlib import Path
from typing import Any
import yaml

from ws.exceptions import ConfigException
from ws.models import AppConfig, RepoConfig

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
        else:
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
        )

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

