"""Command handler implementations decoupling CLI from WorkspaceManager logic."""

import logging
from pathlib import Path
from typing import Sequence

from ws.models import RepoSpec
from ws.output import console, OutputHandler
from ws.workspace import WorkspaceManager

logger = logging.getLogger("ws.commands")


def cmd_new(manager: WorkspaceManager, name: str, repo_specs: Sequence[RepoSpec]) -> None:
    """Execute 'ws new' command."""
    manager.create_workspace(name=name, repo_specs=repo_specs)


def cmd_create(manager: WorkspaceManager, config_file: Path | str) -> None:
    """Execute 'ws create' command using a YAML configuration file."""
    manager.create_workspace_from_config(config_file=config_file)


def cmd_list(manager: WorkspaceManager) -> None:
    """Execute 'ws list' command."""
    workspaces = manager.list_workspaces()
    OutputHandler.print_workspace_list(workspaces)


def cmd_info(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws info' command."""
    metadata, ws_path = manager.get_workspace_info(name=name)
    OutputHandler.print_workspace_info(metadata=metadata, workspace_path=ws_path)


def cmd_remove(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws remove' command."""
    manager.remove_workspace(name=name)


def cmd_open(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws open' command."""
    manager.open_workspace(name=name)


def cmd_status(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws status' command to check git status across worktrees."""
    statuses = manager.status_workspace(name=name)
    console.print(f"[bold cyan]Status for workspace: [yellow]{name}[/yellow][/bold cyan]\n")
    for repo, status in statuses.items():
        console.print(f"[bold magenta]{repo}[/bold magenta]:")
        if status.strip():
            console.print(f"  {status.strip()}")
        else:
            console.print("  [dim green]working tree clean[/dim green]")
        console.print()


def cmd_exec(manager: WorkspaceManager, name: str, command: list[str]) -> None:
    """Execute command across all repository worktrees in a workspace."""
    manager.exec_workspace(name=name, command=command)


def cmd_fetch(manager: WorkspaceManager) -> None:
    """Fetch updates across all bare repositories."""
    manager.fetch_repositories()
    OutputHandler.print_success("Fetched all bare repositories")


def cmd_sync(manager: WorkspaceManager) -> None:
    """Sync and prune worktrees."""
    manager.fetch_repositories()
    OutputHandler.print_success("Synced and pruned repositories")


def cmd_doctor(manager: WorkspaceManager) -> None:
    """Run diagnostics and display report."""
    results = manager.doctor()
    console.print("[bold cyan]System Diagnostics & Health Check[/bold cyan]\n")
    all_ok = True
    for check, ok in results.items():
        badge = "[bold green]PASS[/bold green]" if ok else "[bold red]FAIL[/bold red]"
        if not ok:
            all_ok = False
        console.print(f"  {check:<30} : {badge}")
    console.print()
    if all_ok:
        OutputHandler.print_success("All system health checks passed!")
    else:
        OutputHandler.print_warning("Some health checks failed. Please inspect repository configurations.")


def cmd_init(manager: WorkspaceManager, repo_inputs: Sequence[str]) -> None:
    """Execute 'ws init' command to initialize project and clone repositories."""
    manager.init_project(repo_inputs=repo_inputs)


def cmd_add(manager: WorkspaceManager, repo_input: str) -> None:
    """Execute 'ws add' command to clone and add a single repository."""
    manager.init_project(repo_inputs=[repo_input])


def cmd_antigravity(manager: WorkspaceManager) -> None:
    """Easter egg / AI agent health overview."""
    console.print("[bold green]🚀 Google Antigravity Agent Workspace Diagnostic[/bold green]")
    console.print("Workspace Manager version: [cyan]0.1.0[/cyan]")
    console.print("Repositories active: [yellow]" + str(len(manager.config.repositories)) + "[/yellow]")
    console.print("Workspaces registered: [yellow]" + str(len(manager.list_workspaces())) + "[/yellow]")
    OutputHandler.print_success("Antigravity engine nominal.")


def cmd_workspace_add_repo(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
    branch: str,
    create: bool = True,
) -> None:
    """Execute 'ws workspace add-repo' command."""
    manager.workspace_add_repo(
        workspace_name=workspace_name,
        repo_name=repo_name,
        branch=branch,
        create=create,
    )


def cmd_workspace_remove_repo(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
    delete_branch: bool = False,
) -> None:
    """Execute 'ws workspace remove-repo' command."""
    manager.workspace_remove_repo(
        workspace_name=workspace_name,
        repo_name=repo_name,
        delete_branch=delete_branch,
    )


def cmd_workspace_freeze(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
) -> None:
    """Execute 'ws workspace freeze' command."""
    manager.freeze_repo(workspace_name=workspace_name, repo_name=repo_name)


def cmd_workspace_unfreeze(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
) -> None:
    """Execute 'ws workspace unfreeze' command."""
    manager.unfreeze_repo(workspace_name=workspace_name, repo_name=repo_name)


def cmd_push(
    manager: WorkspaceManager,
    workspace_name: str,
    repos: Sequence[str] | None = None,
    remote: str = "origin",
) -> None:
    """Execute 'ws push' command."""
    results = manager.push_workspace(
        workspace_name=workspace_name,
        repos=repos,
        remote=remote,
    )
    OutputHandler.print_push_summary(workspace_name=workspace_name, results=results)


def cmd_pull(
    manager: WorkspaceManager,
    workspace_name: str,
    repos: Sequence[str] | None = None,
    remote: str = "origin",
) -> None:
    """Execute 'ws pull' command."""
    results = manager.pull_workspace(
        workspace_name=workspace_name,
        repos=repos,
        remote=remote,
    )
    OutputHandler.print_pull_summary(workspace_name=workspace_name, results=results)



