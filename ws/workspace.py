"""Workspace management service and business logic."""

import logging
import os
from pathlib import Path
import shutil
import sys
from typing import Callable, Sequence
import yaml

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

    def open_workspace(self, name: str) -> None:
        """Spawn an interactive subshell inside the workspace directory."""
        ws_dir = self._get_workspace_dir(name)
        if not ws_dir.exists() or not ws_dir.is_dir():
            raise WorkspaceNotFoundException(f"Workspace '{name}' not found at: {ws_dir}")

        shell = os.environ.get("SHELL", "/bin/bash")
        OutputHandler.print_info(f"Opening shell inside workspace: [bold cyan]{ws_dir}[/bold cyan]")

        # Change working directory and execute shell
        os.chdir(ws_dir)
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

    def freeze_repo(self, workspace_name: str, repo_name: str) -> None:
        """Freeze a repository in a workspace, marking tracked files read-only."""
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repo_name not in meta.repositories:
            raise RepoNotInWorkspaceException(
                f"Repository '{repo_name}' is not in workspace '{workspace_name}'"
            )

        spec = meta.repositories[repo_name]
        if spec.frozen:
            OutputHandler.print_info(f"Repository '{repo_name}' is already frozen")
            return

        worktree_path = ws_dir / spec.path
        if worktree_path.exists():
            self.git.set_tracked_files_readonly(worktree_path, readonly=True)

        meta.repositories[repo_name].frozen = True
        self._save_metadata(ws_dir, meta)
        OutputHandler.print_success(f"Frozen repository '{repo_name}' in workspace '{workspace_name}'")

    def unfreeze_repo(self, workspace_name: str, repo_name: str) -> None:
        """Unfreeze a repository in a workspace, restoring write permissions on tracked files."""
        meta, ws_dir = self.get_workspace_info(workspace_name)

        if repo_name not in meta.repositories:
            raise RepoNotInWorkspaceException(
                f"Repository '{repo_name}' is not in workspace '{workspace_name}'"
            )

        spec = meta.repositories[repo_name]
        if not spec.frozen:
            OutputHandler.print_info(f"Repository '{repo_name}' is not frozen")
            return

        worktree_path = ws_dir / spec.path
        if worktree_path.exists():
            self.git.set_tracked_files_readonly(worktree_path, readonly=False)

        meta.repositories[repo_name].frozen = False
        self._save_metadata(ws_dir, meta)
        OutputHandler.print_success(f"Unfrozen repository '{repo_name}' in workspace '{workspace_name}'")

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




