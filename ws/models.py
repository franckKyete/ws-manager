"""Dataclasses and data models for workspace manager."""

from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class RepoConfig:
    """Configuration for a managed repository."""

    name: str
    bare: Path
    checkout: str
    url: str | None = None

    def to_dict(self) -> dict[str, str]:
        res = {
            "bare": str(self.bare),
            "checkout": self.checkout,
        }
        if self.url:
            res["url"] = self.url
        return res

    @classmethod
    def from_dict(cls, name: str, data: dict[str, Any]) -> "RepoConfig":
        bare_val = data.get("bare")
        checkout_val = data.get("checkout")
        url_val = data.get("url")
        if not bare_val or not checkout_val:
            raise ValueError(f"Repository '{name}' definition must include 'bare' and 'checkout'")
        return cls(
            name=name,
            bare=Path(bare_val),
            checkout=str(checkout_val),
            url=str(url_val) if url_val else None,
        )


@dataclass
class RepoSpec:
    """Workspace-specific repository target specification."""

    name: str
    branch: str
    create: bool
    path: str
    frozen: bool = False

    def to_dict(self) -> dict[str, Any]:
        return {
            "branch": self.branch,
            "create": self.create,
            "path": self.path,
            "frozen": self.frozen,
        }

    @classmethod
    def from_dict(cls, name: str, data: dict[str, Any]) -> "RepoSpec":
        branch = data.get("branch")
        create = data.get("create", True)
        path = data.get("path", name)
        frozen = data.get("frozen", False)
        if not branch:
            raise ValueError(f"Repository specification '{name}' missing required 'branch'")
        return cls(
            name=name,
            branch=str(branch),
            create=bool(create),
            path=str(path),
            frozen=bool(frozen),
        )


@dataclass
class WorkspaceMetadata:
    """Workspace metadata saved in workspace.yml."""

    name: str
    created: str
    status: str = "active"
    repositories: dict[str, RepoSpec] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "created": self.created,
            "status": self.status,
            "repositories": {k: v.to_dict() for k, v in self.repositories.items()},
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "WorkspaceMetadata":
        name = data.get("name")
        if not name:
            raise ValueError("Workspace metadata missing required 'name'")
        created = data.get("created", datetime.now(timezone.utc).isoformat())
        status = data.get("status", "active")

        repos_raw = data.get("repositories", {})
        repos: dict[str, RepoSpec] = {}
        for r_name, r_data in repos_raw.items():
            repos[r_name] = RepoSpec.from_dict(r_name, r_data)

        return cls(
            name=str(name),
            created=str(created),
            status=str(status),
            repositories=repos,
        )


@dataclass
class AppConfig:
    """Application-wide configuration."""

    repositories: dict[str, RepoConfig]
    workspaces_dir: Path = Path("workspaces")
    config_file_path: Path | None = None

    @property
    def project_root(self) -> Path:
        """Root directory of the project containing configuration file."""
        return self.config_file_path.parent.resolve() if self.config_file_path else Path.cwd().resolve()

