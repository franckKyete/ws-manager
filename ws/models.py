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
    port: int | None = None
    ports: dict[str, int] = field(default_factory=dict)
    env: dict[str, str] = field(default_factory=dict)
    env_file: str = ".env"
    env_example: str = ".env.example"
    setup: list[str] = field(default_factory=list)
    launch: str | None = None
    secrets: list[str] = field(default_factory=list)
    copy_files: list[Any] = field(default_factory=list)

    @property
    def ports_list(self) -> list[int]:
        """Return list of all configured base ports in deterministic order."""
        if self.ports:
            return list(self.ports.values())
        if self.port is not None:
            return [self.port]
        return []

    def to_dict(self) -> dict[str, Any]:
        res: dict[str, Any] = {
            "bare": str(self.bare),
            "checkout": self.checkout,
        }
        if self.url:
            res["url"] = self.url
        if self.port:
            res["port"] = self.port
        if self.ports:
            res["ports"] = dict(self.ports)
        if self.env:
            res["env"] = dict(self.env)
        if self.env_file != ".env":
            res["env_file"] = self.env_file
        if self.env_example != ".env.example":
            res["env_example"] = self.env_example
        if self.setup:
            res["setup"] = list(self.setup)
        if self.launch:
            res["launch"] = self.launch
        if self.secrets:
            res["secrets"] = list(self.secrets)
        if self.copy_files:
            res["copy_files"] = list(self.copy_files)
        return res

    @classmethod
    def from_dict(cls, name: str, data: dict[str, Any]) -> "RepoConfig":
        bare_val = data.get("bare")
        checkout_val = data.get("checkout")
        url_val = data.get("url")
        if not bare_val or not checkout_val:
            raise ValueError(f"Repository '{name}' definition must include 'bare' and 'checkout'")

        env_raw = data.get("env", {})
        env_dict = {str(k): str(v) for k, v in env_raw.items()} if isinstance(env_raw, dict) else {}

        setup_raw = data.get("setup", [])
        if isinstance(setup_raw, str):
            setup_list = [setup_raw]
        elif isinstance(setup_raw, list):
            setup_list = [str(s) for s in setup_raw]
        else:
            setup_list = []

        secrets_raw = data.get("secrets", [])
        if isinstance(secrets_raw, str):
            secrets_list = [secrets_raw]
        elif isinstance(secrets_raw, list):
            secrets_list = [str(s) for s in secrets_raw]
        else:
            secrets_list = []

        copy_files_raw = data.get("copy_files", data.get("files", []))
        copy_files_list = list(copy_files_raw) if isinstance(copy_files_raw, list) else ([copy_files_raw] if copy_files_raw else [])

        ports_dict: dict[str, int] = {}
        ports_raw = data.get("ports")
        if isinstance(ports_raw, dict):
            for k, v in ports_raw.items():
                try:
                    ports_dict[str(k)] = int(v)
                except (ValueError, TypeError):
                    pass
        elif isinstance(ports_raw, list):
            for idx, v in enumerate(ports_raw):
                try:
                    p_val = int(v)
                    k_name = "default" if idx == 0 else f"port_{idx}"
                    ports_dict[k_name] = p_val
                except (ValueError, TypeError):
                    pass
        elif isinstance(ports_raw, (int, str)):
            try:
                ports_dict["default"] = int(ports_raw)
            except (ValueError, TypeError):
                pass

        port_val = None
        if data.get("port") is not None:
            try:
                port_val = int(data["port"])
                if "default" not in ports_dict and not ports_dict:
                    ports_dict["default"] = port_val
                elif "default" not in ports_dict:
                    ports_dict = {"default": port_val, **ports_dict}
            except (ValueError, TypeError):
                pass
        elif ports_dict:
            port_val = next(iter(ports_dict.values()))

        launch_val = data.get("launch", data.get("command"))
        return cls(
            name=name,
            bare=Path(bare_val),
            checkout=str(checkout_val),
            url=str(url_val) if url_val else None,
            port=port_val,
            ports=ports_dict,
            env=env_dict,
            env_file=str(data.get("env_file", ".env")),
            env_example=str(data.get("env_example", ".env.example")),
            setup=setup_list,
            launch=str(launch_val) if launch_val else None,
            secrets=secrets_list,
            copy_files=copy_files_list,
        )





@dataclass
class RepoSpec:
    """Workspace-specific repository target specification."""

    name: str
    branch: str
    create: bool
    path: str
    frozen: bool = False

    @property
    def locked(self) -> bool:
        return self.frozen

    @locked.setter
    def locked(self, value: bool) -> None:
        self.frozen = value

    def to_dict(self) -> dict[str, Any]:
        return {
            "branch": self.branch,
            "create": self.create,
            "path": self.path,
            "locked": self.frozen,
            "frozen": self.frozen,
        }

    @classmethod
    def from_dict(cls, name: str, data: dict[str, Any]) -> "RepoSpec":
        branch = data.get("branch")
        create = data.get("create", True)
        path = data.get("path", name)
        frozen = data.get("locked", data.get("frozen", False))
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
    global_env: dict[str, str] = field(default_factory=dict)
    dynamic_env: dict[str, str] = field(default_factory=dict)
    setup: list[str] = field(default_factory=list)
    secrets: list[str] = field(default_factory=list)
    copy_files: list[Any] = field(default_factory=list)

    @property
    def project_root(self) -> Path:
        """Root directory of the project containing configuration file."""

        return self.config_file_path.parent.resolve() if self.config_file_path else Path.cwd().resolve()



