"""Workspace management service and business logic."""

import base64
import json
import logging
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
from typing import Callable, Sequence
import yaml

from ws.env import EnvEngine
from ws.exceptions import (
    BranchAlreadyExistsException,
    BranchNotFoundException,
    RepoAlreadyInWorkspaceException,
    RepoFrozenException,
    RepoNotInWorkspaceException,
    RepositoryNotFoundException,
    RollbackException,
    ValidationException,
    WorkspaceExistsException,
    WorkspaceNotFoundException,
    WSException,
)
from ws.git import GitService
from ws.models import AppConfig, RepoConfig, RepoSpec, WorkspaceMetadata
from ws.output import OutputHandler
from ws.utils import ensure_directory, get_iso_timestamp

logger = logging.getLogger("ws.workspace")


class RollbackStack:
    """Stack for tracking and executing rollback actions during creation failure."""

    def __init__(self):
        self._actions: list[tuple[str, Callable[[], None]]] = []

    def add(self, description: str, action: Callable[[], None]) -> None:
        """Add a rollback step to the stack."""
        self._actions.append((description, action))

    def clear(self) -> None:
        """Clear all rollback steps when process succeeds."""
        self._actions.clear()

    def execute(self) -> list[str]:
        """Execute all recorded rollback actions in reverse order."""
        executed: list[str] = []
        for description, action in reversed(self._actions):
            try:
                logger.info("Rollback step: %s", description)
                action()
                executed.append(description)
            except Exception as e:
                logger.error("Failed to execute rollback step '%s': %s", description, e)
        self._actions.clear()
        return executed


class WorkspaceManager:
    """Service managing workspace operations (create, remove, list, info)."""

    def __init__(self, config: AppConfig, git_service: GitService | None = None):
        self.config = config
        self.git = git_service or GitService()

    def _get_workspace_dir(self, name: str) -> Path:
        """Get absolute path to a workspace directory."""
        return (self.config.workspaces_dir / name).resolve()

    def validate_environment(self) -> None:
        """Ensure Git is installed and available."""
        if not self.git.is_git_installed():
            raise ValidationException("Git is not installed or not available in PATH.")

    def validate_repository_config(self, repo_name: str) -> RepoConfig:
        """Validate that a repository configuration exists and its bare repo is valid."""
        if repo_name not in self.config.repositories:
            raise RepositoryNotFoundException(
                f"Repository '{repo_name}' is not defined in configuration. "
                f"Configured repositories: {', '.join(self.config.repositories.keys())}"
            )
        repo_cfg = self.config.repositories[repo_name]

        # Resolve bare repo path relative to cwd if relative
        bare_path = repo_cfg.bare.resolve() if repo_cfg.bare.is_absolute() else (Path.cwd() / repo_cfg.bare).resolve()

        if not self.git.is_bare_repo(bare_path):
            raise RepositoryNotFoundException(
                f"Bare Git repository for '{repo_name}' not found or invalid at: {bare_path}"
            )

        return RepoConfig(
            name=repo_cfg.name,
            bare=bare_path,
            checkout=repo_cfg.checkout,
        )

    def validate_creation(self, name: str, repo_specs: Sequence[RepoSpec]) -> None:
        """Pre-creation validation for workspace and branches."""
        self.validate_environment()

        ws_dir = self._get_workspace_dir(name)
        if ws_dir.exists():
            raise WorkspaceExistsException(f"Workspace directory already exists: {ws_dir}")

        for spec in repo_specs:
            repo_cfg = self.validate_repository_config(spec.name)
            branch_exists = self.git.branch_exists(repo_cfg.bare, spec.branch)

            if spec.create and branch_exists:
                raise BranchAlreadyExistsException(
                    f"Cannot create branch '{spec.branch}' for repository '{spec.name}': "
                    f"branch already exists in bare repo {repo_cfg.bare.name}"
                )

            if not spec.create and not branch_exists:
                raise BranchNotFoundException(
                    f"Branch '{spec.branch}' does not exist in repository '{spec.name}' ({repo_cfg.bare.name})"
                )

    def create_workspace(self, name: str, repo_specs: Sequence[RepoSpec]) -> WorkspaceMetadata:
        """Create a new workspace with git worktrees and metadata."""
        # 1. Validation
        self.validate_creation(name, repo_specs)

        ws_dir = self._get_workspace_dir(name)
        rollback = RollbackStack()

        OutputHandler.print_creation_header(name, repo_specs)

        try:
            # Step A: Create workspace directory
            logger.info("Creating directory: %s", ws_dir)
            ws_dir.mkdir(parents=True, exist_ok=False)
            rollback.add(f"Remove directory {ws_dir}", lambda: shutil.rmtree(ws_dir, ignore_errors=True))

            # Step B: Create worktrees for each repository
            spec_dict: dict[str, RepoSpec] = {}

            for spec in repo_specs:
                repo_cfg = self.validate_repository_config(spec.name)
                worktree_path = ws_dir / repo_cfg.checkout

                mode_str = "Creating branch and worktree" if spec.create else "Checking out worktree"
                logger.info("%s for '%s' at %s", mode_str, spec.name, worktree_path)

                with OutputHandler.spinner(f"Setting up worktree for {spec.name} ({spec.branch})..."):
                    self.git.create_worktree(
                        bare_path=repo_cfg.bare,
                        worktree_path=worktree_path,
                        branch=spec.branch,
                        create_branch=spec.create,
                    )

                # Register rollback for worktree removal
                b_path = repo_cfg.bare
                wt_path = worktree_path
                br_name = spec.branch
                was_created = spec.create

                rollback.add(
                    f"Remove worktree {wt_path}",
                    lambda b=b_path, w=wt_path: self.git.remove_worktree(b, w, force=True),
                )

                if was_created:
                    rollback.add(
                        f"Delete created branch {br_name} in {b_path.name}",
                        lambda b=b_path, br=br_name: self.git.delete_branch(b, br, force=True),
                    )

                spec_dict[spec.name] = RepoSpec(
                    name=spec.name,
                    branch=spec.branch,
                    create=spec.create,
                    path=repo_cfg.checkout,
                )

            # Step C: Write workspace.yml metadata
            metadata = WorkspaceMetadata(
                name=name,
                created=get_iso_timestamp(),
                status="active",
                repositories=spec_dict,
            )

            self._save_metadata(ws_dir, metadata)

            # Success! Clear rollback stack
            rollback.clear()
            OutputHandler.print_creation_success(name, ws_dir)
            return metadata

        except Exception as e:
            logger.error("Workspace creation failed. Initiating rollback: %s", e)
            rollback.execute()
            OutputHandler.print_rollback_notice(str(e), restored=True)
            raise RollbackException(f"Failed to create workspace '{name}': {e}") from e

    def create_workspace_from_config(self, config_file: Path | str) -> WorkspaceMetadata:
        """Create a workspace defined by a YAML configuration file."""
        cfg_path = Path(config_file).resolve()
        if not cfg_path.exists() or not cfg_path.is_file():
            raise ValidationException(f"Configuration file not found: {cfg_path}")

        try:
            with open(cfg_path, "r", encoding="utf-8") as f:
                data = yaml.safe_load(f) or {}
        except Exception as e:
            raise ValidationException(f"Invalid YAML file '{cfg_path}': {e}") from e

        name = data.get("name")
        if not name:
            raise ValidationException(f"YAML config '{cfg_path}' must contain a 'name' field")

        repos_raw = data.get("repositories", {})
        if not isinstance(repos_raw, dict) or not repos_raw:
            raise ValidationException(f"YAML config '{cfg_path}' must contain a 'repositories' section")

        specs: list[RepoSpec] = []
        for r_name, r_data in repos_raw.items():
            if not isinstance(r_data, dict):
                raise ValidationException(f"Invalid repository spec for '{r_name}' in config")
            branch = r_data.get("branch")
            create = r_data.get("create", True)
            if not branch:
                raise ValidationException(f"Repository '{r_name}' missing 'branch' in '{cfg_path}'")

            # Match repo checkout path from global app config if available
            checkout_path = r_name
            if r_name in self.config.repositories:
                checkout_path = self.config.repositories[r_name].checkout

            specs.append(
                RepoSpec(
                    name=r_name,
                    branch=str(branch),
                    create=bool(create),
                    path=checkout_path,
                )
            )

        return self.create_workspace(name=str(name), repo_specs=specs)

    def remove_workspace(self, name: str) -> None:
        """Remove a workspace, removing all its git worktrees and metadata."""
        self.validate_environment()
        ws_dir = self._get_workspace_dir(name)

        if not ws_dir.exists() or not ws_dir.is_dir():
            raise WorkspaceNotFoundException(f"Workspace '{name}' not found at: {ws_dir}")

        metadata_file = ws_dir / "workspace.yml"
        metadata: WorkspaceMetadata | None = None

        if metadata_file.exists():
            try:
                with open(metadata_file, "r", encoding="utf-8") as f:
                    data = yaml.safe_load(f)
                    metadata = WorkspaceMetadata.from_dict(data)
            except Exception as e:
                logger.warning("Could not read workspace metadata: %s", e)

        # Remove worktrees based on metadata or config repositories
        if metadata and metadata.repositories:
            for r_name, spec in metadata.repositories.items():
                if r_name in self.config.repositories:
                    repo_cfg = self.validate_repository_config(r_name)
                    wt_path = ws_dir / spec.path
                    if wt_path.exists():
                        with OutputHandler.spinner(f"Removing worktree for {r_name}..."):
                            self.git.remove_worktree(repo_cfg.bare, wt_path, force=True)
        else:
            # Fallback to configured repositories
            for r_name, repo_cfg in self.config.repositories.items():
                bare_path = repo_cfg.bare.resolve() if repo_cfg.bare.is_absolute() else (Path.cwd() / repo_cfg.bare).resolve()
                wt_path = ws_dir / repo_cfg.checkout
                if wt_path.exists():
                    with OutputHandler.spinner(f"Removing worktree for {r_name}..."):
                        self.git.remove_worktree(bare_path, wt_path, force=True)

        # Prune worktrees in all bare repositories
        for r_name, repo_cfg in self.config.repositories.items():
            bare_path = repo_cfg.bare.resolve() if repo_cfg.bare.is_absolute() else (Path.cwd() / repo_cfg.bare).resolve()
            if self.git.is_bare_repo(bare_path):
                self.git.prune_worktrees(bare_path)

        # Delete workspace directory
        shutil.rmtree(ws_dir, ignore_errors=True)
        OutputHandler.print_success(f"Removed workspace '{name}'")

    def list_workspaces(self) -> list[WorkspaceMetadata]:
        """List all managed workspaces."""
        ws_root = self.config.workspaces_dir.resolve()
        if not ws_root.exists() or not ws_root.is_dir():
            return []

        workspaces: list[WorkspaceMetadata] = []
        for child in ws_root.iterdir():
            if child.is_dir():
                meta_file = child / "workspace.yml"
                if meta_file.exists() and meta_file.is_file():
                    try:
                        with open(meta_file, "r", encoding="utf-8") as f:
                            data = yaml.safe_load(f)
                            workspaces.append(WorkspaceMetadata.from_dict(data))
                    except Exception as e:
                        logger.warning("Invalid metadata file in workspace %s: %s", child.name, e)
                        workspaces.append(
                            WorkspaceMetadata(
                                name=child.name,
                                created="unknown",
                                status="unknown",
                            )
                        )
                else:
                    # Workspace directory without workspace.yml
                    workspaces.append(
                        WorkspaceMetadata(
                            name=child.name,
                            created="unknown",
                            status="active",
                        )
                    )

        return workspaces

    def get_workspace_info(self, name: str) -> tuple[WorkspaceMetadata, Path]:
        """Retrieve metadata and path for a workspace."""
        ws_dir = self._get_workspace_dir(name)
        if not ws_dir.exists() or not ws_dir.is_dir():
            raise WorkspaceNotFoundException(f"Workspace '{name}' not found at: {ws_dir}")

        meta_file = ws_dir / "workspace.yml"
        if not meta_file.exists():
            raise ValidationException(f"Workspace '{name}' does not contain a workspace.yml file")

        with open(meta_file, "r", encoding="utf-8") as f:
            data = yaml.safe_load(f)

        return WorkspaceMetadata.from_dict(data), ws_dir

    def get_session_socket_path(self, name: str) -> Path:
        """Return socket path for workspace session daemon."""
        ws_dir = self._get_workspace_dir(name)
        return ws_dir / ".ws" / "session.sock"

    def is_daemon_active(self, name: str) -> bool:
        """Check if the background supervisor daemon is currently active for workspace."""
        sock_path = self.get_session_socket_path(name)
        if not sock_path.exists():
            return False
        try:
            from ws._native import is_session_active
            if is_session_active(str(sock_path)):
                return True
            else:
                sock_path.unlink(missing_ok=True)
                return False
        except ImportError:
            import socket
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                    s.settimeout(0.3)
                    s.connect(str(sock_path))
                    return True
            except (OSError, ConnectionRefusedError):
                sock_path.unlink(missing_ok=True)
                return False

    def get_active_engine(self, name: str) -> str | None:
        """Detect which multiplexer backend is currently running for this workspace."""
        project_name = self.config.project_root.name
        from ws.multiplexer import TmuxLauncher, ZellijLauncher

        if TmuxLauncher.is_window_running(project_name, name):
            return "tmux"

        if ZellijLauncher.is_session_running(project_name):
            if ZellijLauncher.is_tab_running(project_name, name):
                return "zellij"

        if self.is_daemon_active(name):
            return "tui"

        return None



    def get_running_services_status(self, name: str) -> dict[str, dict[str, Any]]:
        """Return live status of running services in the workspace (engine, status, port, urls)."""
        active_engine = self.get_active_engine(name)
        if not active_engine:
            return {}

        results: dict[str, dict[str, Any]] = {}
        ws_dir = self._get_workspace_dir(name)
        descriptor = EnvEngine.read_service_discovery_descriptor(ws_dir)
        discovery_services = descriptor.get("services", {}) if descriptor else {}

        sock_path = self.get_session_socket_path(name)
        if sock_path.exists():
            import socket, json
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                    s.settimeout(0.4)
                    s.connect(str(sock_path))
                    s.sendall(b'{"type":"GetState"}\n')
                    data = s.recv(4096)
                    resp = json.loads(data.decode("utf-8").strip())
                    if resp.get("type") == "State":
                        for svc in resp.get("services", []):
                            s_name = svc["name"]
                            s_disc = discovery_services.get(s_name, {})
                            results[s_name] = {
                                "status": svc.get("status", "running"),
                                "port": svc.get("port") or s_disc.get("port", 0),
                                "url_local": s_disc.get("url_local"),
                                "url_lan": s_disc.get("url_lan"),
                                "url_public": s_disc.get("url_public"),
                                "engine": active_engine,
                            }
                        return results
            except Exception:
                pass

        project_name = self.config.project_root.name
        from ws.multiplexer import TmuxLauncher
        if active_engine == "tmux":
            panes = TmuxLauncher.list_panes(project_name, name)
            for p in panes:
                s_disc = discovery_services.get(p, {})
                results[p] = {
                    "status": "running",
                    "port": s_disc.get("port", 0),
                    "url_local": s_disc.get("url_local"),
                    "url_lan": s_disc.get("url_lan"),
                    "url_public": s_disc.get("url_public"),
                    "engine": "tmux",
                }

        return results



    def is_session_running(self, name: str) -> bool:
        """Check if background daemon session, tmux window, or zellij session is active."""
        return self.get_active_engine(name) is not None


    def stop_workspace(self, name: str) -> bool:
        """Stop running background daemon session, tmux window, or zellij session for workspace."""
        project_name = self.config.project_root.name
        from ws.multiplexer import TmuxLauncher, ZellijLauncher
        stopped_any = False

        if TmuxLauncher.is_window_running(project_name, name):
            TmuxLauncher.kill_workspace(name, project_name)
            stopped_any = True

        if ZellijLauncher.is_session_running(project_name):
            ZellijLauncher.kill_workspace(name, project_name)
            stopped_any = True

        sock_path = self.get_session_socket_path(name)
        if not sock_path.exists():
            return stopped_any
        try:
            from ws._native import stop_workspace_session
            stopped = stop_workspace_session(str(sock_path))
            return stopped or stopped_any
        except ImportError:
            if sock_path.exists():
                sock_path.unlink(missing_ok=True)
            return True




    def open_workspace(self, name: str, worktree: str | None = None) -> None:
        """Spawn an interactive subshell inside the workspace or a specific worktree directory."""
        ws_dir = self._get_workspace_dir(name)
        if not ws_dir.exists() or not ws_dir.is_dir():
            raise WorkspaceNotFoundException(f"Workspace '{name}' not found at: {ws_dir}")

        target_dir = ws_dir
        if worktree:
            meta, _ = self.get_workspace_info(name)
            if worktree in meta.repositories:
                spec = meta.repositories[worktree]
                target_dir = ws_dir / spec.path
            elif (ws_dir / worktree).is_dir():
                target_dir = ws_dir / worktree
            else:
                raise WorkspaceNotFoundException(
                    f"Worktree '{worktree}' not found in workspace '{name}'. "
                    f"Available worktrees: {', '.join(meta.repositories.keys())}"
                )

        shell = os.environ.get("SHELL", "/bin/bash")
        OutputHandler.print_info(f"Opening shell inside: [bold cyan]{target_dir}[/bold cyan]")

        os.chdir(target_dir)
        os.execv(shell, [shell])


    def status_workspace(self, name: str) -> dict[str, str]:
        """Get git status of all worktrees in a workspace."""
        ws_dir = self._get_workspace_dir(name)
        if not ws_dir.exists():
            raise WorkspaceNotFoundException(f"Workspace '{name}' not found")

        meta, _ = self.get_workspace_info(name)
        statuses: dict[str, str] = {}
        for r_name, spec in meta.repositories.items():
            wt_path = ws_dir / spec.path
            if wt_path.exists():
                statuses[r_name] = self.git.get_status(wt_path)
            else:
                statuses[r_name] = "missing worktree"
        return statuses

    def exec_workspace(self, name: str, command: list[str]) -> dict[str, int]:
        """Execute shell command in each repository worktree of a workspace."""
        import subprocess

        ws_dir = self._get_workspace_dir(name)
        if not ws_dir.exists():
            raise WorkspaceNotFoundException(f"Workspace '{name}' not found")

        meta, _ = self.get_workspace_info(name)
        results: dict[str, int] = {}
        for r_name, spec in meta.repositories.items():
            wt_path = ws_dir / spec.path
            if wt_path.exists():
                OutputHandler.print_info(f"Executing in [bold magenta]{r_name}[/bold magenta]...")
                res = subprocess.run(command, cwd=wt_path)
                results[r_name] = res.returncode
            else:
                OutputHandler.print_warning(f"Skipping {r_name} (worktree missing)")
                results[r_name] = -1
        return results

    def fetch_repositories(self) -> None:
        """Fetch all bare repositories."""
        for r_name, repo_cfg in self.config.repositories.items():
            bare_path = repo_cfg.bare.resolve() if repo_cfg.bare.is_absolute() else (Path.cwd() / repo_cfg.bare).resolve()
            if self.git.is_bare_repo(bare_path):
                OutputHandler.print_info(f"Fetching bare repo [bold cyan]{r_name}[/bold cyan] ({bare_path.name})...")
                self.git.fetch_repo(bare_path)

    def doctor(self) -> dict[str, bool]:
        """Run system diagnostics and health checks."""
        results: dict[str, bool] = {}
        results["git_installed"] = self.git.is_git_installed()

        for r_name, repo_cfg in self.config.repositories.items():
            bare_path = repo_cfg.bare.resolve() if repo_cfg.bare.is_absolute() else (Path.cwd() / repo_cfg.bare).resolve()
            results[f"repo_{r_name}"] = self.git.is_bare_repo(bare_path)

        ws_root = self.config.workspaces_dir.resolve()
        results["workspaces_dir_exists"] = ws_root.exists()

        # Network interface and LAN IP detection
        from ws.network import get_lan_ip, list_network_interfaces
        active_interfaces = list_network_interfaces()
        detected_ip = get_lan_ip()
        results["network_interfaces_detected"] = len(active_interfaces) > 0 or detected_ip != "127.0.0.1"
        return results

    @staticmethod
    def parse_repo_url(input_str: str) -> tuple[str, str, Path, str]:
        """Parse repository input string into (name, url, bare_path, checkout)."""
        input_str = input_str.strip()
        name = ""
        url = ""

        if "=" in input_str and not input_str.startswith("http://") and not input_str.startswith("https://") and not input_str.startswith("git@"):
            name, url = input_str.split("=", 1)
        else:
            url = input_str

        clean_url = url.rstrip("/")
        if clean_url.endswith(".git"):
            base_name = clean_url[:-4].split("/")[-1].split(":")[-1]
        else:
            base_name = clean_url.split("/")[-1].split(":")[-1]

        if not name:
            name = base_name.lower()

        checkout = base_name
        bare_name = f"{base_name}.git" if not base_name.endswith(".git") else base_name
        bare_path = Path("bares") / bare_name

        return name, url, bare_path, checkout

    def init_project(self, repo_inputs: Sequence[str]) -> AppConfig:
        """Initialize project configuration and clone bare repositories from Git URLs."""
        from ws.config import ConfigLoader

        self.validate_environment()

        updated_repos = dict(self.config.repositories)

        for item in repo_inputs:
            name, url, bare_path, checkout = self.parse_repo_url(item)

            resolved_bare = bare_path.resolve() if bare_path.is_absolute() else (Path.cwd() / bare_path).resolve()
            ensure_directory(resolved_bare.parent)

            if not self.git.is_bare_repo(resolved_bare):
                OutputHandler.print_info(f"Cloning bare repository [bold cyan]{name}[/bold cyan] from [dim]{url}[/dim]...")
                with OutputHandler.spinner(f"Cloning {name} into {bare_path}..."):
                    self.git.clone_bare(url=url, target_bare_path=resolved_bare)
                OutputHandler.print_success(f"Cloned bare repo [cyan]{bare_path}[/cyan]")
            else:
                OutputHandler.print_info(f"Using existing bare repository at [cyan]{bare_path}[/cyan]")

            updated_repos[name] = RepoConfig(
                name=name,
                bare=bare_path,
                checkout=checkout,
                url=url,
            )

        saved_path = ConfigLoader.save_config(repositories=updated_repos)
        OutputHandler.print_success(f"Saved configuration to [bold white]{saved_path}[/bold white]")

        self.config.repositories = updated_repos
        return self.config

    def _save_metadata(self, ws_dir: Path, metadata: WorkspaceMetadata) -> None:
        """Save workspace metadata to workspace.yml."""
        metadata_path = ws_dir / "workspace.yml"
        with open(metadata_path, "w", encoding="utf-8") as f:
            yaml.dump(metadata.to_dict(), f, sort_keys=False, default_flow_style=False)

    def workspace_add_repo(
        self,
        workspace_name: str,
        repo_name: str,
        branch: str,
        create: bool = True,
    ) -> None:
        """Add a new repository worktree to an existing workspace."""
        self.validate_environment()
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repo_name in meta.repositories:
            raise RepoAlreadyInWorkspaceException(
                f"Repository '{repo_name}' is already in workspace '{workspace_name}'"
            )

        repo_cfg = self.validate_repository_config(repo_name)
        branch_exists = self.git.branch_exists(repo_cfg.bare, branch)

        if create and branch_exists:
            raise BranchAlreadyExistsException(
                f"Cannot create branch '{branch}' for repository '{repo_name}': branch already exists"
            )
        if not create and not branch_exists:
            raise BranchNotFoundException(
                f"Branch '{branch}' does not exist in repository '{repo_name}'"
            )

        worktree_path = ws_dir / repo_cfg.checkout
        logger.info("Adding worktree for '%s' (%s) at %s", repo_name, branch, worktree_path)

        with OutputHandler.spinner(f"Setting up worktree for {repo_name} ({branch})..."):
            self.git.create_worktree(
                bare_path=repo_cfg.bare,
                worktree_path=worktree_path,
                branch=branch,
                create_branch=create,
            )

        meta.repositories[repo_name] = RepoSpec(
            name=repo_name,
            branch=branch,
            create=create,
            path=repo_cfg.checkout,
            frozen=False,
        )
        self._save_metadata(ws_dir, meta)
        OutputHandler.print_success(f"Added repository '{repo_name}' ({branch}) to workspace '{workspace_name}'")

    def workspace_remove_repo(
        self,
        workspace_name: str,
        repo_name: str,
        delete_branch: bool = False,
    ) -> None:
        """Remove a repository worktree from an existing workspace."""
        self.validate_environment()
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repo_name not in meta.repositories:
            raise RepoNotInWorkspaceException(
                f"Repository '{repo_name}' is not in workspace '{workspace_name}'"
            )

        spec = meta.repositories[repo_name]
        if spec.frozen:
            raise RepoFrozenException(
                f"Cannot remove repository '{repo_name}': it is frozen in workspace '{workspace_name}'. Unfreeze it first."
            )

        repo_cfg = self.validate_repository_config(repo_name)
        worktree_path = ws_dir / spec.path

        if worktree_path.exists():
            # If tracked files were readonly, restore write permissions before removing worktree
            self.git.set_tracked_files_readonly(worktree_path, readonly=False)
            with OutputHandler.spinner(f"Removing worktree for {repo_name}..."):
                self.git.remove_worktree(repo_cfg.bare, worktree_path, force=True)

        if delete_branch:
            with OutputHandler.spinner(f"Deleting branch {spec.branch} in {repo_cfg.bare.name}..."):
                self.git.delete_branch(repo_cfg.bare, spec.branch, force=True)

        del meta.repositories[repo_name]
        self._save_metadata(ws_dir, meta)
        OutputHandler.print_success(
            f"Removed repository '{repo_name}' from workspace '{workspace_name}'"
            + (" (deleted branch)" if delete_branch else "")
        )

    def lock_repo(self, workspace_name: str, repo_name: str) -> None:
        """Lock a repository in a workspace, marking tracked files read-only."""
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repo_name not in meta.repositories:
            raise RepoNotInWorkspaceException(
                f"Repository '{repo_name}' is not in workspace '{workspace_name}'"
            )

        spec = meta.repositories[repo_name]
        if spec.frozen or spec.locked:
            OutputHandler.print_info(f"Repository '#{repo_name}' is already locked")
            return

        worktree_path = ws_dir / spec.path
        if worktree_path.exists():
            self.git.set_tracked_files_readonly(worktree_path, readonly=True)
            # Ensure env files remain writable
            repo_cfg = self.config.repositories.get(repo_name)
            env_candidates = [".env", ".env.local", ".env.development", ".env.test"]
            if repo_cfg:
                env_candidates.append(repo_cfg.env_file)
            for env_name in set(env_candidates):
                env_file = worktree_path / env_name
                if env_file.exists() and env_file.is_file():
                    try:
                        mode = env_file.stat().st_mode
                        if not (mode & stat.S_IWUSR):
                            os.chmod(env_file, mode | stat.S_IWUSR)
                    except Exception as e:
                        logger.debug("Failed ensuring %s is writable: %s", env_file, e)

        meta.repositories[repo_name].frozen = True
        self._save_metadata(ws_dir, meta)
        OutputHandler.print_success(f"Locked repository '#{repo_name}' in workspace '@{workspace_name}'")

    def freeze_repo(self, workspace_name: str, repo_name: str) -> None:
        """Backward-compatible alias for lock_repo."""
        return self.lock_repo(workspace_name, repo_name)

    def unlock_repo(self, workspace_name: str, repo_name: str) -> None:
        """Unlock a repository in a workspace, restoring write permissions on tracked and untracked env files."""
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repo_name not in meta.repositories:
            raise RepoNotInWorkspaceException(
                f"Repository '{repo_name}' is not in workspace '{workspace_name}'"
            )

        spec = meta.repositories[repo_name]
        is_already_unlocked = not spec.frozen and not spec.locked

        worktree_path = ws_dir / spec.path
        if worktree_path.exists():
            self.git.set_tracked_files_readonly(worktree_path, readonly=False)
            # Also restore permissions on untracked env files and copied files
            repo_cfg = self.config.repositories.get(repo_name)
            env_candidates = [".env", ".env.local", ".env.development", ".env.test"]
            if repo_cfg:
                env_candidates.append(repo_cfg.env_file)
            for env_name in set(env_candidates):
                env_file = worktree_path / env_name
                if env_file.exists() and env_file.is_file():
                    try:
                        mode = env_file.stat().st_mode
                        if not (mode & stat.S_IWUSR):
                            os.chmod(env_file, mode | stat.S_IWUSR)
                    except Exception as e:
                        logger.debug("Failed unlocking %s: %s", env_file, e)

        meta.repositories[repo_name].frozen = False
        self._save_metadata(ws_dir, meta)
        if is_already_unlocked:
            OutputHandler.print_success(f"Restored write permissions for repository '#{repo_name}' in workspace '@{workspace_name}'")
        else:
            OutputHandler.print_success(f"Unlocked repository '#{repo_name}' in workspace '@{workspace_name}'")

    def unfreeze_repo(self, workspace_name: str, repo_name: str) -> None:
        """Backward-compatible alias for unlock_repo."""
        return self.unlock_repo(workspace_name, repo_name)



    def push_workspace(
        self,
        workspace_name: str,
        repos: Sequence[str] | None = None,
        remote: str = "origin",
    ) -> dict[str, dict[str, Any]]:
        """Push committed branches for repositories in a workspace to their remotes."""
        self.validate_environment()
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repos:
            target_repos = list(repos)
            for r in target_repos:
                if r not in meta.repositories:
                    raise RepoNotInWorkspaceException(
                        f"Repository '{r}' is not in workspace '{workspace_name}'"
                    )
        else:
            target_repos = list(meta.repositories.keys())

        results: dict[str, dict[str, Any]] = {}

        for r_name in target_repos:
            spec = meta.repositories[r_name]
            wt_path = ws_dir / spec.path

            if spec.frozen:
                results[r_name] = {
                    "status": "skipped",
                    "reason": "frozen repository (read-only)",
                    "branch": spec.branch,
                    "remote": remote,
                }
                continue

            if not wt_path.exists():
                results[r_name] = {
                    "status": "skipped",
                    "reason": "missing worktree",
                    "branch": spec.branch,
                    "remote": remote,
                }
                continue

            try:
                with OutputHandler.spinner(f"Pushing {r_name} ({spec.branch}) to {remote}..."):
                    was_pushed, message = self.git.push_branch(
                        worktree_path=wt_path,
                        remote=remote,
                        branch=spec.branch,
                    )
                results[r_name] = {
                    "status": "pushed" if was_pushed else "up-to-date",
                    "reason": message,
                    "branch": spec.branch,
                    "remote": remote,
                }
            except Exception as e:
                results[r_name] = {
                    "status": "failed",
                    "reason": str(e),
                    "branch": spec.branch,
                    "remote": remote,
                }

        return results

    def pull_workspace(
        self,
        workspace_name: str,
        repos: Sequence[str] | None = None,
        remote: str = "origin",
    ) -> dict[str, dict[str, Any]]:
        """Pull updates for repositories in a workspace from their remotes."""
        self.validate_environment()
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repos:
            target_repos = list(repos)
            for r in target_repos:
                if r not in meta.repositories:
                    raise RepoNotInWorkspaceException(
                        f"Repository '{r}' is not in workspace '{workspace_name}'"
                    )
        else:
            target_repos = list(meta.repositories.keys())

        results: dict[str, dict[str, Any]] = {}

        for r_name in target_repos:
            spec = meta.repositories[r_name]
            wt_path = ws_dir / spec.path

            if spec.frozen:
                results[r_name] = {
                    "status": "skipped",
                    "reason": "frozen repository (read-only)",
                    "branch": spec.branch,
                    "remote": remote,
                }
                continue

            if not wt_path.exists():
                results[r_name] = {
                    "status": "skipped",
                    "reason": "missing worktree",
                    "branch": spec.branch,
                    "remote": remote,
                }
                continue

            try:
                with OutputHandler.spinner(f"Pulling {r_name} ({spec.branch}) from {remote}..."):
                    was_updated, message = self.git.pull_branch(
                        worktree_path=wt_path,
                        remote=remote,
                        branch=spec.branch,
                    )
                results[r_name] = {
                    "status": "pulled" if was_updated else "up-to-date",
                    "reason": message,
                    "branch": spec.branch,
                    "remote": remote,
                }
            except Exception as e:
                results[r_name] = {
                    "status": "failed",
                    "reason": str(e),
                    "branch": spec.branch,
                    "remote": remote,
                }

        return results

    def setup_workspace(
        self,
        workspace_name: str,
        repos: Sequence[str] | None = None,
        dry_run: bool = False,
        skip_scripts: bool = False,
        verbose: bool = False,
        interface: str | None = None,
        lan_ip: str | None = None,
    ) -> dict[str, dict[str, Any]]:
        """Run 3-step setup pipeline for repositories in a workspace.

        Step 1: Copy .env.example -> .env if present and .env is missing.
        Step 2: Resolve global, dynamic, and repository-scoped environment variables and sync into .env.
        Step 3: Run repository setup script/commands sequentially.
        """
        import time
        self.validate_environment()
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repos:
            target_repos = list(repos)
            for r in target_repos:
                if r not in meta.repositories:
                    raise RepoNotInWorkspaceException(
                        f"Repository '{r}' is not in workspace '{workspace_name}'"
                    )
        else:
            target_repos = list(meta.repositories.keys())

        from ws.network import allocate_workspace_ports, get_lan_ip
        slot = EnvEngine.get_workspace_slot(self.config.workspaces_dir, workspace_name)
        resolved_lan_ip = lan_ip or get_lan_ip(preferred_interface=interface)
        public_host = (
            self.config.global_env.get("PUBLIC_HOST")
            or os.environ.get("WS_PUBLIC_HOST")
            or os.environ.get("PUBLIC_HOST")
            or resolved_lan_ip
        )

        service_ports, _ = allocate_workspace_ports(self.config.repositories, slot=slot)
        EnvEngine.write_service_discovery_files(
            workspace_dir=ws_dir,
            workspace_name=workspace_name,
            slot=slot,
            service_ports=service_ports,
            public_host=public_host,
            lan_ip=resolved_lan_ip,
            interface=interface,
        )

        results: dict[str, dict[str, Any]] = {}

        # 0. Optional Top-Level Workspace Infrastructure Setup (e.g. create database, start local services)
        if not repos and self.config.setup and not skip_scripts:
            OutputHandler.print_setup_repo_start("WORKSPACE INFRASTRUCTURE", ws_dir)
            global_vars = EnvEngine.resolve_repo_env(
                self.config,
                workspace_name,
                "",
                slot=slot,
                service_ports=service_ports,
                lan_ip=resolved_lan_ip,
                public_host=public_host,
                interface=interface,
            )
            if verbose:
                all_global_secrets = list(set(self.config.secrets))
                OutputHandler.print_env_resolution_details(global_vars, explicit_secrets=all_global_secrets)

            for g_cmd in self.config.setup:
                expanded_g_cmd = EnvEngine.expand_command(
                    g_cmd,
                    global_vars,
                    workspace_name,
                    "",
                    slot=slot,
                    project_root=self.config.project_root,
                    workspaces_dir=self.config.workspaces_dir,
                    service_ports=service_ports,
                    lan_ip=resolved_lan_ip,
                    public_host=public_host,
                    interface=interface,
                )

                if dry_run:
                    OutputHandler.print_setup_step(0, f"[DRY-RUN] {expanded_g_cmd}", "skipped execution", status="info")
                    continue

                OutputHandler.print_command_start(expanded_g_cmd)
                t0 = time.time()
                try:
                    proc_env = os.environ.copy()
                    proc_env.update(global_vars)
                    proc_env["WORKSPACE_NAME"] = workspace_name
                    proc_env["PROJECT_ROOT"] = str(self.config.project_root.resolve())
                    proc_env["SCRIPTS_DIR"] = str(self.config.project_root.resolve() / "scripts")
                    proc_env["WORKSPACE_DIR"] = str(ws_dir.resolve())

                    proc = subprocess.run(
                        expanded_g_cmd,
                        shell=True,
                        cwd=ws_dir,
                        env=proc_env,
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    elapsed = time.time() - t0

                    if proc.returncode != 0:
                        err = proc.stderr.strip() or proc.stdout.strip()
                        first_err = err.splitlines()[0] if err else f"exit code {proc.returncode}"
                        OutputHandler.print_command_done(expanded_g_cmd, elapsed, success=False, returncode=proc.returncode)
                        OutputHandler.print_command_output(expanded_g_cmd, proc.stdout, proc.stderr, proc.returncode)
                    else:
                        OutputHandler.print_command_done(expanded_g_cmd, elapsed, success=True, returncode=0)
                        if verbose:
                            OutputHandler.print_command_output(expanded_g_cmd, proc.stdout, proc.stderr, proc.returncode)
                except Exception as e:
                    elapsed = time.time() - t0
                    OutputHandler.print_command_done(expanded_g_cmd, elapsed, success=False, returncode=-1)
                    OutputHandler.print_error(f"Failed to execute workspace setup command '{expanded_g_cmd}'", str(e))

        for r_name in target_repos:
            spec = meta.repositories[r_name]
            wt_path = ws_dir / spec.path
            repo_cfg = self.config.repositories.get(r_name)

            OutputHandler.print_setup_repo_start(r_name, wt_path)

            if not repo_cfg:
                OutputHandler.print_setup_step(1, "Config Validation", "Repository missing from project configuration", status="error")
                results[r_name] = {
                    "status": "failed",
                    "reason": f"Repository '{r_name}' missing from project configuration",
                    "env_status": "skipped",
                    "commands_run": [],
                }
                continue

            if not wt_path.exists():
                OutputHandler.print_setup_step(1, "Worktree Validation", "Worktree directory does not exist", status="warning")
                results[r_name] = {
                    "status": "skipped",
                    "reason": "missing worktree",
                    "env_status": "skipped",
                    "commands_run": [],
                }
                continue

            # Step 1: Copy example env & Sync configured global project files
            if verbose:
                OutputHandler.print_step_start(1, f"Preparing templates and shared project files")

            # Copy any shared configuration files
            all_copy_files = list(self.config.copy_files) + list(repo_cfg.copy_files)
            if all_copy_files:
                file_ok, file_msg = EnvEngine.sync_copied_files(
                    project_root=self.config.project_root,
                    worktree_path=wt_path,
                    copy_files=all_copy_files,
                )
                if not file_ok:
                    OutputHandler.print_setup_step(1, "File Copy", f"Warning: {file_msg}", status="warning")
                else:
                    OutputHandler.print_setup_step(1, "File Copy", file_msg, status="success")

            env_vars = EnvEngine.resolve_repo_env(
                self.config,
                workspace_name,
                r_name,
                slot=slot,
                service_ports=service_ports,
                lan_ip=resolved_lan_ip,
                public_host=public_host,
                interface=interface,
            )
            env_ok, env_msg = EnvEngine.prepare_and_sync_env_file(

                worktree_path=wt_path,
                env_vars=env_vars,
                env_filename=repo_cfg.env_file,
                example_filename=repo_cfg.env_example,
            )

            if not env_ok:
                OutputHandler.print_setup_step(1, "Environment Setup", f"Failed: {env_msg}", status="error")
                results[r_name] = {
                    "status": "failed",
                    "reason": f"Env sync failed: {env_msg}",
                    "env_status": env_msg,
                    "commands_run": [],
                }
                continue

            OutputHandler.print_setup_step(1, "Template Setup", f"processed {repo_cfg.env_example} -> {repo_cfg.env_file}", status="success")
            OutputHandler.print_setup_step(2, "Env Resolution", f"{env_msg}", status="success")

            if verbose:
                all_secrets = list(set(self.config.secrets + repo_cfg.secrets))
                OutputHandler.print_env_resolution_details(env_vars, explicit_secrets=all_secrets)

            # Step 3: Run setup script/commands
            if skip_scripts or not repo_cfg.setup:
                OutputHandler.print_setup_step(3, "Setup Scripts", "No setup scripts configured (skipped)", status="info")
                results[r_name] = {
                    "status": "completed",
                    "reason": "environment synced (no setup commands)",
                    "env_status": env_msg,
                    "commands_run": [],
                }
                continue

            script_failures: list[str] = []
            executed_cmds: list[str] = []

            for cmd in repo_cfg.setup:
                expanded_cmd = EnvEngine.expand_command(
                    cmd,
                    env_vars,
                    workspace_name,
                    r_name,
                    project_root=self.config.project_root,
                    workspaces_dir=self.config.workspaces_dir,
                )
                executed_cmds.append(expanded_cmd)
                if dry_run:
                    OutputHandler.print_setup_step(3, f"[DRY-RUN] {expanded_cmd}", "skipped execution", status="info")
                    continue

                OutputHandler.print_command_start(expanded_cmd)
                t0 = time.time()
                try:
                    proc_env = os.environ.copy()
                    proc_env.update(env_vars)
                    proc_env["WORKSPACE_NAME"] = workspace_name
                    proc_env["REPO_NAME"] = r_name
                    proc_env["PROJECT_ROOT"] = str(self.config.project_root.resolve())
                    proc_env["SCRIPTS_DIR"] = str(self.config.project_root.resolve() / "scripts")
                    proc_env["WORKSPACE_DIR"] = str(ws_dir.resolve())
                    proc_env["WORKTREE_DIR"] = str(wt_path.resolve())

                    proc = subprocess.run(
                        expanded_cmd,
                        shell=True,
                        cwd=wt_path,
                        env=proc_env,
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    elapsed = time.time() - t0

                    if proc.returncode != 0:
                        err = proc.stderr.strip() or proc.stdout.strip()
                        first_err = err.splitlines()[0] if err else f"exit code {proc.returncode}"
                        script_failures.append(f"'{expanded_cmd}' failed: {first_err}")
                        OutputHandler.print_command_done(expanded_cmd, elapsed, success=False, returncode=proc.returncode)
                        OutputHandler.print_command_output(expanded_cmd, proc.stdout, proc.stderr, proc.returncode)
                        break
                    else:
                        OutputHandler.print_command_done(expanded_cmd, elapsed, success=True, returncode=0)
                        if verbose:
                            OutputHandler.print_command_output(expanded_cmd, proc.stdout, proc.stderr, proc.returncode)
                except Exception as e:
                    elapsed = time.time() - t0
                    script_failures.append(f"'{expanded_cmd}' execution error: {e}")
                    OutputHandler.print_command_done(expanded_cmd, elapsed, success=False, returncode=-1)
                    OutputHandler.print_error(f"Failed to execute command '{expanded_cmd}'", str(e))
                    break




            if script_failures:
                results[r_name] = {
                    "status": "failed",
                    "reason": "; ".join(script_failures),
                    "env_status": env_msg,
                    "commands_run": executed_cmds,
                }
            else:
                results[r_name] = {
                    "status": "completed",
                    "reason": f"ran {len(executed_cmds)} setup command(s)",
                    "env_status": env_msg,
                    "commands_run": executed_cmds,
                }

        return results

    def sync_env(
        self,
        workspace_name: str,
        repos: Sequence[str] | None = None,
        interface: str | None = None,
        lan_ip: str | None = None,
    ) -> dict[str, dict[str, Any]]:
        """Sync environment variables only without running setup commands."""
        return self.setup_workspace(
            workspace_name=workspace_name,
            repos=repos,
            skip_scripts=True,
            interface=interface,
            lan_ip=lan_ip,
        )

    def get_env_vars(
        self,
        workspace_name: str,
        repo_name: str,
        interface: str | None = None,
        lan_ip: str | None = None,
    ) -> dict[str, str]:
        """Get resolved environment variables dictionary for a repository."""
        return EnvEngine.resolve_repo_env(
            self.config,
            workspace_name,
            repo_name,
            interface=interface,
            lan_ip=lan_ip,
        )

    def launch_workspace(
        self,
        workspace_name: str,
        repos: Sequence[str] | None = None,
        mode: str = "summary",
        attach_repo: str | None = None,
        daemon: bool = False,
        switch: bool = False,
        interface: str | None = None,
        lan_ip: str | None = None,
    ) -> list[tuple[str, str, str, dict[str, str]]]:
        """Launch workspace services concurrently with TUI, daemon, tmux, or terminal window multiplexing.

        Returns list of (repo_name, worktree_path_str, launch_command, env_vars).
        """
        from ws.multiplexer import TerminalLauncher, TmuxLauncher, ZellijLauncher
        from ws.process import ProcessSupervisor
        from ws.tui import WorkspaceTUI

        meta, ws_dir = self.get_workspace_info(workspace_name)
        target_repos = list(repos) if repos else list(meta.repositories.keys())

        # Pre-flight real-time socket availability check and collision auto-healing
        from ws.network import allocate_workspace_ports, get_lan_ip
        slot = EnvEngine.get_workspace_slot(self.config.workspaces_dir, workspace_name)

        descriptor = EnvEngine.read_service_discovery_descriptor(ws_dir)
        recorded_leases = (
            {s_k: s_v["port"] for s_k, s_v in descriptor.get("services", {}).items() if "port" in s_v}
            if descriptor
            else None
        )

        # Freshly re-evaluate live LAN IP, Public Host, and bindable ports on each run
        resolved_lan_ip = lan_ip or get_lan_ip(preferred_interface=interface)
        public_host = (
            self.config.global_env.get("PUBLIC_HOST")
            or os.environ.get("WS_PUBLIC_HOST")
            or os.environ.get("PUBLIC_HOST")
            or resolved_lan_ip
        )

        service_ports, has_shifted = allocate_workspace_ports(
            self.config.repositories,
            slot=slot,
            recorded_leases=recorded_leases,
        )

        if has_shifted:
            OutputHandler.print_warning(
                f"Active socket collision detected for workspace '@{workspace_name}'. "
                "Dynamically re-allocated free ports and synchronized worktree .env files."
            )

        # Always re-evaluate and synchronize worktree .env files on each run
        for r_k in meta.repositories:
            spec = meta.repositories.get(r_k)
            repo_cfg = self.config.repositories.get(r_k)
            if spec and repo_cfg:
                wt_p = ws_dir / spec.path
                if wt_p.exists():
                    r_env = EnvEngine.resolve_repo_env(
                        self.config,
                        workspace_name,
                        r_k,
                        slot=slot,
                        service_ports=service_ports,
                        lan_ip=resolved_lan_ip,
                        public_host=public_host,
                        interface=interface,
                    )
                    EnvEngine.prepare_and_sync_env_file(
                        worktree_path=wt_p,
                        env_vars=r_env,
                        env_filename=repo_cfg.env_file,
                        example_filename=repo_cfg.env_example,
                    )

        # Write / update live service descriptor files (.ws/services.json & .ws/services.env)
        EnvEngine.write_service_discovery_files(
            workspace_dir=ws_dir,
            workspace_name=workspace_name,
            slot=slot,
            service_ports=service_ports,
            public_host=public_host,
            lan_ip=resolved_lan_ip,
            interface=interface,
        )

        launch_entries: list[tuple[str, str, str, dict[str, str]]] = []
        for r_name in target_repos:
            spec = meta.repositories.get(r_name)
            repo_cfg = self.config.repositories.get(r_name)
            if spec and repo_cfg and repo_cfg.launch:
                wt_path = ws_dir / spec.path
                env_vars = EnvEngine.resolve_repo_env(
                    self.config,
                    workspace_name,
                    r_name,
                    slot=slot,
                    service_ports=service_ports,
                    lan_ip=resolved_lan_ip,
                    public_host=public_host,
                    interface=interface,
                )
                expanded_cmd = EnvEngine.expand_command(
                    repo_cfg.launch,
                    env_vars,
                    workspace_name,
                    r_name,
                    slot=slot,
                    project_root=self.config.project_root,
                    workspaces_dir=self.config.workspaces_dir,
                    service_ports=service_ports,
                    lan_ip=resolved_lan_ip,
                    public_host=public_host,
                    interface=interface,
                )
                launch_entries.append((r_name, str(wt_path), expanded_cmd, env_vars))



        if not launch_entries or mode in ("summary", "list"):
            return launch_entries

        # Check cross-engine conflicts / switching
        active_engine = self.get_active_engine(workspace_name)
        req_engine = "tui" if mode in ("tui", "daemon") else mode
        if active_engine and req_engine not in (active_engine, "summary", "list", "attach"):
            if switch:
                OutputHandler.print_info(
                    f"Switching workspace '[bold cyan]{workspace_name}[/bold cyan]' from {active_engine} to {req_engine}..."
                )
                from ws.multiplexer import TmuxLauncher, ZellijLauncher
                if active_engine == "tmux":
                    TmuxLauncher.kill_workspace(workspace_name, self.config.project_root.name)
                elif active_engine == "zellij":
                    ZellijLauncher.kill_workspace(workspace_name, self.config.project_root.name)
            else:
                OutputHandler.print_error(
                    f"Workspace '{workspace_name}' is already running in {active_engine}.\n"
                    f"To switch engines, use '--switch' (e.g. 'ws launch {workspace_name} --{req_engine} --switch')."
                )
                return launch_entries

        # Helper to ensure background daemon is running to host the live services
        def _ensure_daemon_running():
            sock_path = self.get_session_socket_path(workspace_name)
            if not self.is_daemon_active(workspace_name):
                log_dir = ws_dir / ".ws" / "logs"
                sock_path.parent.mkdir(parents=True, exist_ok=True)
                if sock_path.exists():
                    sock_path.unlink(missing_ok=True)

                daemon_script = (
                    "import sys, os; "
                    "from ws._native import ServiceSpec, start_workspace_daemon; "
                    f"specs = [ServiceSpec(s[0], s[2], s[1], s[3]) for s in {launch_entries!r}]; "
                    f"start_workspace_daemon({workspace_name!r}, specs, {str(sock_path)!r}, {str(log_dir)!r})"
                )
                proc = subprocess.Popen(
                    [sys.executable, "-c", daemon_script],
                    start_new_session=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    stdin=subprocess.DEVNULL,
                )
                import time
                for _ in range(60):
                    if self.is_daemon_active(workspace_name):
                        break
                    if proc.poll() is not None:
                        break
                    time.sleep(0.05)


        # Mode 0: Detached Background Daemon
        if daemon or mode == "daemon":
            try:
                _ensure_daemon_running()
                OutputHandler.print_success(
                    f"Workspace daemon active for '{workspace_name}' ({len(launch_entries)} services).\n"
                    f"Attach anytime using: [bold yellow]ws attach {workspace_name}[/bold yellow] or [bold yellow]ws {workspace_name} attach[/bold yellow]"
                )
                return launch_entries
            except Exception as e:
                OutputHandler.print_error(f"Failed starting background daemon: {e}")
                return launch_entries

        project_name = self.config.project_root.name

        # Mode 1: Zellij session
        if mode == "zellij":
            if not ZellijLauncher.is_available():
                OutputHandler.print_error(
                    "Zellij executable is not found on $PATH.\n"
                    "Install Zellij using 'cargo install zellij' or via your system package manager."
                )
                return launch_entries
            _ensure_daemon_running()
            if ZellijLauncher.launch(workspace_name, launch_entries, project_name=project_name, ws_dir=ws_dir):
                return launch_entries


        # Mode 2: tmux session
        if mode == "tmux":
            if not TmuxLauncher.is_available():
                OutputHandler.print_error(
                    "tmux executable is not found on $PATH.\n"
                    "Install tmux via your system package manager (e.g. apt install tmux or brew install tmux)."
                )
                return launch_entries
            _ensure_daemon_running()
            if TmuxLauncher.launch(workspace_name, launch_entries, project_name=project_name):
                return launch_entries


        # Mode 3: Separate terminal windows/tabs
        if mode == "terminal":
            if TerminalLauncher.launch(workspace_name, launch_entries):
                return launch_entries


        # Mode 3: Single service attach / direct execution
        if mode == "attach" or (attach_repo and len(launch_entries) == 1):
            target_repo = attach_repo or launch_entries[0][0]
            _ensure_daemon_running()
            sock_path = self.get_session_socket_path(workspace_name)
            try:
                from ws._native import run_raw_bridge
                run_raw_bridge(str(sock_path), target_repo)
            except Exception as e:
                OutputHandler.print_error(f"Failed attaching to service '{target_repo}': {e}")
            return launch_entries

        # Mode 4: Interactive Multi-Pane TUI (Default for interactive CLI launch)
        if mode == "tui":
            sock_path = self.get_session_socket_path(workspace_name)
            try:
                from ws._native import attach_workspace_session
                _ensure_daemon_running()

                if self.is_session_running(workspace_name):
                    # Attach client TUI directly to the daemon session
                    exit_code = attach_workspace_session(
                        workspace_name=workspace_name,
                        socket_path=str(sock_path),
                        initial_focus=attach_repo,
                        fullscreen=False,
                    )
                    if exit_code != 0:
                        logger.info("Native TUI session finished with exit code %d", exit_code)
                return launch_entries

            except ImportError:
                OutputHandler.print_warning(
                    "Compiled native TUI extension not found.\n"
                    "Please compile using 'cargo build --workspace'."
                )
                return launch_entries
            except Exception as e:
                OutputHandler.print_error(f"Failed starting TUI session: {e}")
                return launch_entries



        return launch_entries

    # ==================== Hub Cloud Operations ====================

    def _get_project_namespace_and_name(self, override_identifier: str | None = None) -> tuple[str, str]:
        """Resolve (namespace, name) for active project."""
        from ws.hub import HubClient
        client = HubClient()

        if override_identifier:
            return client.parse_project_identifier(override_identifier)

        # Check if project name can be inferred from config or directory name
        proj_dir_name = self.config.project_root.name
        clean_name = proj_dir_name.replace("-workspaces", "").replace("_workspaces", "").lower()

        # Try getting whoami username
        try:
            user = client.whoami()
            namespace = user.get("username", "personal")
        except Exception:
            namespace = "personal"

        return namespace, clean_name

    def clone_from_hub(self, project_identifier: str, target_dir: Path | str | None = None) -> Path:
        """Clone project blueprint, bare repositories, and secrets from wshub."""
        from ws.hub import HubClient
        from ws.config import ConfigLoader

        client = HubClient()
        namespace, name = client.parse_project_identifier(project_identifier)

        OutputHandler.print_info(f"Connecting to wshub for [bold cyan]{namespace}/{name}[/bold cyan]...")
        with OutputHandler.spinner(f"Fetching blueprint for {namespace}/{name}..."):
            data = client.get_project(namespace, name)

        project = data.get("project", {})
        latest_rev = data.get("latestRevision")
        if not latest_rev or not latest_rev.get("blueprintYaml"):
            raise ConfigException(f"Project '{namespace}/{name}' has no valid blueprint revisions.")

        # Determine target directory
        if target_dir:
            dest_dir = Path(target_dir).resolve()
        else:
            dest_dir = (Path.cwd() / f"{name}-workspaces").resolve()

        ensure_directory(dest_dir)
        ensure_directory(dest_dir / "bares")
        ensure_directory(dest_dir / "workspaces")

        # 1. Write repositories.yml
        config_path = dest_dir / "repositories.yml"
        blueprint_content = latest_rev["blueprintYaml"]
        with open(config_path, "w", encoding="utf-8") as f:
            f.write(blueprint_content)
        OutputHandler.print_success(f"Wrote [bold white]{config_path}[/bold white]")

        # 2. Write scripts if any
        scripts_raw = latest_rev.get("scriptsJson")
        if scripts_raw:
            try:
                scripts_dict = json.loads(scripts_raw)
                scripts_dir = dest_dir / "scripts"
                ensure_directory(scripts_dir)
                for script_name, script_content in scripts_dict.items():
                    s_file = scripts_dir / script_name
                    with open(s_file, "w", encoding="utf-8") as f:
                        f.write(script_content)
                    try:
                        s_file.chmod(0o755)
                    except Exception:
                        pass
                OutputHandler.print_success(f"Restored {len(scripts_dict)} automation script(s)")
            except Exception as e:
                logger.debug("Failed to write scripts: %e", e)

        # 3. Download sensitive files from hub
        try:
            files_list = client.list_files(namespace, name)
            if files_list:
                files_dir = dest_dir / "files"
                ensure_directory(files_dir)
                for f_info in files_list:
                    rel_path = f_info["filePath"]
                    target_file = files_dir / rel_path
                    ensure_directory(target_file.parent)
                    file_bytes = client.download_file(namespace, name, rel_path)
                    with open(target_file, "wb") as f:
                        f.write(file_bytes)
                OutputHandler.print_success(f"Downloaded {len(files_list)} secret file(s) from vault")
        except Exception as e:
            logger.debug("No files downloaded or error: %e", e)

        # 3.5. Fetch decrypted secrets from vault and re-hydrate local secret blocks
        try:
            secrets_list = client.list_secrets(namespace, name)
            if secrets_list:
                with open(config_path, "r", encoding="utf-8") as f:
                    cfg_data = yaml.safe_load(f) or {}

                for s_item in secrets_list:
                    s_key = s_item.get("key")
                    s_val = s_item.get("value")
                    s_repo = s_item.get("repoName")
                    if not s_key or not s_val:
                        continue

                    if not s_repo or s_repo == "global":
                        if "secret" not in cfg_data or not isinstance(cfg_data["secret"], dict):
                            cfg_data["secret"] = {}
                        cfg_data["secret"][s_key] = s_val
                    else:
                        repos_data = cfg_data.get("repositories", {})
                        if s_repo in repos_data and isinstance(repos_data[s_repo], dict):
                            if "secret" not in repos_data[s_repo] or not isinstance(repos_data[s_repo]["secret"], dict):
                                repos_data[s_repo]["secret"] = {}
                            repos_data[s_repo]["secret"][s_key] = s_val

                with open(config_path, "w", encoding="utf-8") as f:
                    yaml.dump(cfg_data, f, sort_keys=False, default_flow_style=False)
                OutputHandler.print_success(f"Restored {len(secrets_list)} secret(s) from vault into local config")
        except Exception as e:
            logger.debug("Failed restoring secrets from vault: %e", e)

        # 4. Clone all bare repositories
        loaded_cfg = ConfigLoader.load_config(config_path=config_path, workspaces_dir=dest_dir / "workspaces")
        for r_name, repo_cfg in loaded_cfg.repositories.items():
            if not repo_cfg.url:
                OutputHandler.print_warning(f"Skipping {r_name}: no Git URL configured in blueprint")
                continue

            bare_path = dest_dir / repo_cfg.bare
            ensure_directory(bare_path.parent)

            if not self.git.is_bare_repo(bare_path):
                OutputHandler.print_info(f"Cloning bare repository [bold cyan]{r_name}[/bold cyan] from [dim]{repo_cfg.url}[/dim]...")
                with OutputHandler.spinner(f"Cloning {r_name} into {bare_path.name}..."):
                    self.git.clone_bare(url=repo_cfg.url, target_bare_path=bare_path)
                OutputHandler.print_success(f"Cloned [cyan]{bare_path.name}[/cyan]")
            else:
                OutputHandler.print_info(f"Using existing bare repo [cyan]{bare_path.name}[/cyan]")

        return dest_dir

    def hub_publish(self, project_identifier: str | None = None, description: str | None = None) -> dict[str, Any]:
        """Publish local workspace project definition, encrypted vault secrets, and sensitive files to wshub."""
        from ws.hub import HubClient
        from ws.config import ConfigLoader
        client = HubClient()
        namespace, name = self._get_project_namespace_and_name(project_identifier)

        config_file = self.config.config_file_path or (self.config.project_root / "repositories.yml")
        if not config_file.exists():
            raise ConfigException("No 'repositories.yml' found in project root to publish.")

        # Classify assets into sanitized blueprint, vault secrets, sensitive files, and private vars
        sanitized_yaml, extracted_secrets, files_to_upload, private_count = ConfigLoader.classify_project_assets(self.config)

        # Collect scripts
        scripts_dict: dict[str, str] = {}
        scripts_dir = self.config.project_root / "scripts"
        if scripts_dir.exists() and scripts_dir.is_dir():
            for s_path in scripts_dir.glob("*"):
                if s_path.is_file():
                    try:
                        with open(s_path, "r", encoding="utf-8") as sf:
                            scripts_dict[s_path.name] = sf.read()
                    except Exception:
                        pass

        scripts_json = json.dumps(scripts_dict) if scripts_dict else None

        OutputHandler.print_info(f"Publishing project [bold cyan]{namespace}/{name}[/bold cyan] to wshub...")
        with OutputHandler.spinner(f"Registering project blueprint {namespace}/{name}..."):
            result = client.create_project(
                namespace=namespace,
                name=name,
                blueprint_yaml=sanitized_yaml,
                description=description,
                scripts_json=scripts_json,
                changelog="Initial publish from local workspace",
            )
        OutputHandler.print_success(f"Published project [bold green]{namespace}/{name}[/bold green] (Revision v1)")

        # 1. Sync Vault Secrets
        total_secrets = 0
        if extracted_secrets:
            with OutputHandler.spinner("Encrypting and storing secrets in wshub vault..."):
                for scope, sec_dict in extracted_secrets.items():
                    repo_param = None if scope == "global" else scope
                    client.set_secrets_bulk(namespace, name, sec_dict, repo_name=repo_param)
                    total_secrets += len(sec_dict)
            OutputHandler.print_success(f"🔒 Stored and encrypted [bold cyan]{total_secrets}[/bold cyan] secret(s) in Vault")

        # 2. Sync Sensitive Files
        if files_to_upload:
            with OutputHandler.spinner(f"Encrypting and uploading {len(files_to_upload)} file(s)..."):
                for f_path in files_to_upload:
                    try:
                        rel_path = str(f_path.relative_to(self.config.project_root))
                    except ValueError:
                        rel_path = f_path.name
                    with open(f_path, "rb") as f:
                        file_bytes = f.read()
                    client.upload_file(namespace, name, rel_file_path=rel_path, content_bytes=file_bytes)
            OutputHandler.print_success(f"📁 Encrypted and uploaded [bold cyan]{len(files_to_upload)}[/bold cyan] sensitive file(s)")

        # 3. Report private vars omitted
        if private_count > 0:
            OutputHandler.print_info(f"🚫 Skipped [dim]{private_count}[/dim] private variable(s) (kept local)")

        return result

    def hub_push(self, message: str = "Update configuration", project_identifier: str | None = None) -> dict[str, Any]:
        """Push local project blueprint changes, secrets, and files to wshub as a new revision."""
        from ws.hub import HubClient
        from ws.config import ConfigLoader
        client = HubClient()
        namespace, name = self._get_project_namespace_and_name(project_identifier)

        config_file = self.config.config_file_path or (self.config.project_root / "repositories.yml")
        if not config_file.exists():
            raise ConfigException("No 'repositories.yml' found in project root.")

        sanitized_yaml, extracted_secrets, files_to_upload, private_count = ConfigLoader.classify_project_assets(self.config)

        scripts_dict: dict[str, str] = {}
        scripts_dir = self.config.project_root / "scripts"
        if scripts_dir.exists() and scripts_dir.is_dir():
            for s_path in scripts_dir.glob("*"):
                if s_path.is_file():
                    try:
                        with open(s_path, "r", encoding="utf-8") as sf:
                            scripts_dict[s_path.name] = sf.read()
                    except Exception:
                        pass
        scripts_json = json.dumps(scripts_dict) if scripts_dict else None

        with OutputHandler.spinner(f"Pushing revision to {namespace}/{name}..."):
            result = client.push_revision(
                namespace=namespace,
                name=name,
                blueprint_yaml=sanitized_yaml,
                scripts_json=scripts_json,
                changelog=message,
            )
        version = result.get("revision", {}).get("version", "?")
        OutputHandler.print_success(f"Pushed revision [bold green]v{version}[/bold green] to [cyan]{namespace}/{name}[/cyan]")

        # 1. Update Vault Secrets
        total_secrets = 0
        if extracted_secrets:
            with OutputHandler.spinner("Updating encrypted secrets in vault..."):
                for scope, sec_dict in extracted_secrets.items():
                    repo_param = None if scope == "global" else scope
                    client.set_secrets_bulk(namespace, name, sec_dict, repo_name=repo_param)
                    total_secrets += len(sec_dict)
            OutputHandler.print_success(f"🔒 Synced [bold cyan]{total_secrets}[/bold cyan] secret(s) in Vault")

        # 2. Update Files
        if files_to_upload:
            with OutputHandler.spinner(f"Uploading {len(files_to_upload)} file(s)..."):
                for f_path in files_to_upload:
                    try:
                        rel_path = str(f_path.relative_to(self.config.project_root))
                    except ValueError:
                        rel_path = f_path.name
                    with open(f_path, "rb") as f:
                        file_bytes = f.read()
                    client.upload_file(namespace, name, rel_file_path=rel_path, content_bytes=file_bytes)
            OutputHandler.print_success(f"📁 Synced [bold cyan]{len(files_to_upload)}[/bold cyan] sensitive file(s)")

        if private_count > 0:
            OutputHandler.print_info(f"🚫 Skipped [dim]{private_count}[/dim] private variable(s) (kept local)")

        return result

    def hub_pull(self, project_identifier: str | None = None) -> dict[str, Any]:
        """Pull latest blueprint and bare repositories from wshub."""
        from ws.hub import HubClient
        client = HubClient()
        namespace, name = self._get_project_namespace_and_name(project_identifier)

        with OutputHandler.spinner(f"Fetching latest blueprint from {namespace}/{name}..."):
            data = client.get_project(namespace, name)

        latest_rev = data.get("latestRevision")
        if not latest_rev:
            raise ConfigException(f"Project '{namespace}/{name}' has no revisions.")

        config_file = self.config.config_file_path or (self.config.project_root / "repositories.yml")
        with open(config_file, "w", encoding="utf-8") as f:
            f.write(latest_rev["blueprintYaml"])
        OutputHandler.print_success(f"Updated [bold white]{config_file.name}[/bold white] to revision v{latest_rev['version']}")

        # Merge vault secrets and restore local private variables
        try:
            secrets_list = client.list_secrets(namespace, name)
            with open(config_file, "r", encoding="utf-8") as f:
                cfg_data = yaml.safe_load(f) or {}

            for s_item in secrets_list:
                s_key = s_item.get("key")
                s_val = s_item.get("value")
                s_repo = s_item.get("repoName")
                if not s_key or not s_val:
                    continue
                if not s_repo or s_repo == "global":
                    if "secret" not in cfg_data or not isinstance(cfg_data["secret"], dict):
                        cfg_data["secret"] = {}
                    cfg_data["secret"][s_key] = s_val
                else:
                    repos_data = cfg_data.get("repositories", {})
                    if s_repo in repos_data and isinstance(repos_data[s_repo], dict):
                        if "secret" not in repos_data[s_repo] or not isinstance(repos_data[s_repo]["secret"], dict):
                            repos_data[s_repo]["secret"] = {}
                        repos_data[s_repo]["secret"][s_key] = s_val

            if self.config.private_env:
                if "private" not in cfg_data or not isinstance(cfg_data["private"], dict):
                    cfg_data["private"] = {}
                for pk, pv in self.config.private_env.items():
                    cfg_data["private"][pk] = pv

            for r_name, r_cfg in self.config.repositories.items():
                if r_cfg.private_env:
                    repos_data = cfg_data.get("repositories", {})
                    if r_name in repos_data and isinstance(repos_data[r_name], dict):
                        if "private" not in repos_data[r_name] or not isinstance(repos_data[r_name]["private"], dict):
                            repos_data[r_name]["private"] = {}
                        for pk, pv in r_cfg.private_env.items():
                            repos_data[r_name]["private"][pk] = pv

            with open(config_file, "w", encoding="utf-8") as f:
                yaml.dump(cfg_data, f, sort_keys=False, default_flow_style=False)
        except Exception as e:
            logger.debug("Error merging secrets and private vars during pull: %e", e)

        # Clone any missing bare repos
        from ws.config import ConfigLoader
        reloaded_cfg = ConfigLoader.load_config(config_path=config_file)
        for r_name, r_cfg in reloaded_cfg.repositories.items():
            bare_path = r_cfg.bare.resolve() if r_cfg.bare.is_absolute() else (self.config.project_root / r_cfg.bare).resolve()
            if not self.git.is_bare_repo(bare_path) and r_cfg.url:
                OutputHandler.print_info(f"Cloning newly added bare repo [cyan]{r_name}[/cyan]...")
                with OutputHandler.spinner(f"Cloning {r_name}..."):
                    self.git.clone_bare(url=r_cfg.url, target_bare_path=bare_path)
                OutputHandler.print_success(f"Cloned [cyan]{bare_path.name}[/cyan]")

        return data

    def hub_state_save(
        self,
        workspace_name: str,
        project_identifier: str | None = None,
        include_wip: bool = True,
    ) -> dict[str, Any]:
        """Save active workspace state (branches, locks, local env, and uncommitted WIP) to wshub."""
        from ws.hub import HubClient
        client = HubClient()
        namespace, name = self._get_project_namespace_and_name(project_identifier)

        meta, ws_dir = self.get_workspace_info(workspace_name)
        state_dict = meta.to_dict()

        wip_summary: list[tuple[str, int, int]] = []
        if include_wip:
            wip_dict: dict[str, Any] = {}
            for r_name, spec in meta.repositories.items():
                wt_path = ws_dir / spec.path
                if not wt_path.exists():
                    continue

                diff = self.git.get_uncommitted_diff(wt_path)
                untracked_files = self.git.get_untracked_files(wt_path)
                untracked_contents: dict[str, str] = {}

                for rel_p in untracked_files:
                    file_p = wt_path / rel_p
                    if file_p.is_file():
                        try:
                            encoded = base64.b64encode(file_p.read_bytes()).decode("ascii")
                            untracked_contents[rel_p] = encoded
                        except Exception as e:
                            logger.warning("Could not read untracked file '%s': %s", file_p, e)

                if diff.strip() or untracked_contents:
                    diff_file_count = len([line for line in diff.splitlines() if line.startswith("diff --git")])
                    wip_dict[r_name] = {
                        "diff": diff,
                        "untracked": untracked_contents,
                    }
                    wip_summary.append((r_name, diff_file_count, len(untracked_contents)))

            if wip_dict:
                state_dict["wip"] = wip_dict

        with OutputHandler.spinner(f"Saving state for @{meta.name} to wshub..."):
            result = client.save_workspace_state(
                namespace=namespace,
                name=name,
                workspace_name=meta.name,
                state_dict=state_dict,
            )
        OutputHandler.print_success(f"Saved workspace state [bold cyan]@{meta.name}[/bold cyan] to [cyan]{namespace}/{name}[/cyan]")
        for r_name, mod_cnt, untr_cnt in wip_summary:
            parts = []
            if mod_cnt > 0:
                parts.append(f"{mod_cnt} modified file{'s' if mod_cnt != 1 else ''}")
            if untr_cnt > 0:
                parts.append(f"{untr_cnt} untracked file{'s' if untr_cnt != 1 else ''}")
            OutputHandler.print_info(f"  🔒 Captured uncommitted work in [cyan]%{r_name}[/cyan] ({', '.join(parts)})")
        return result

    def hub_state_restore(
        self,
        workspace_name: str,
        project_identifier: str | None = None,
        apply_wip: bool = True,
    ) -> None:
        """Restore workspace state on another machine."""
        from ws.hub import HubClient
        client = HubClient()
        namespace, name = self._get_project_namespace_and_name(project_identifier)

        with OutputHandler.spinner(f"Fetching state for @{workspace_name} from wshub..."):
            state_dict = client.get_workspace_state(namespace, name, workspace_name)

        if not state_dict or "name" not in state_dict:
            raise ConfigException(f"No saved state found for workspace '{workspace_name}' on wshub.")

        meta = WorkspaceMetadata.from_dict(state_dict)
        repo_specs = list(meta.repositories.values())

        OutputHandler.print_info(f"Recreating workspace [bold cyan]@{meta.name}[/bold cyan] from hub state...")
        self.create_workspace(name=meta.name, repo_specs=repo_specs)

        # Restore uncommitted WIP if present
        wip_data = state_dict.get("wip", {})
        if apply_wip and isinstance(wip_data, dict) and wip_data:
            _, ws_dir = self.get_workspace_info(meta.name)
            for r_name, r_wip in wip_data.items():
                if not isinstance(r_wip, dict):
                    continue
                spec = meta.repositories.get(r_name)
                wt_path = ws_dir / (spec.path if spec else r_name)
                if not wt_path.exists():
                    continue

                # 1. Restore untracked files
                untracked_dict = r_wip.get("untracked", {})
                if isinstance(untracked_dict, dict):
                    for rel_p, b64_content in untracked_dict.items():
                        try:
                            file_dest = wt_path / rel_p
                            file_dest.parent.mkdir(parents=True, exist_ok=True)
                            file_dest.write_bytes(base64.b64decode(b64_content.encode("ascii")))
                        except Exception as e:
                            logger.warning("Failed to restore untracked file '%s': %s", rel_p, e)

                # 2. Apply git diff patch
                diff_patch = r_wip.get("diff", "")
                if isinstance(diff_patch, str) and diff_patch.strip():
                    self.git.apply_patch(wt_path, diff_patch)

                OutputHandler.print_success(f"Restored uncommitted work in [cyan]%{r_name}[/cyan]")

        OutputHandler.print_success(f"Restored workspace [bold green]@{meta.name}[/bold green] successfully")










