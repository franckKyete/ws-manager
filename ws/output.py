"""Rich output and terminal formatting module."""

import logging
from pathlib import Path
from typing import Sequence

from rich.console import Console
from rich.panel import Panel
from rich.progress import Progress, SpinnerColumn, TextColumn
from rich.rule import Rule
from rich.syntax import Syntax
from rich.table import Table
from rich.text import Text
from rich.tree import Tree

from ws.models import RepoSpec, WorkspaceMetadata
from ws.utils import format_relative_time

# Shared rich console instance
console = Console()
error_console = Console(stderr=True)


class OutputHandler:
    """Formatter and printer for terminal output using Rich."""

    @staticmethod
    def print_success(message: str) -> None:
        """Print a success message with green checkmark."""
        console.print(f"[bold green]✔[/bold green] [bold white]{message}[/bold white]")

    @staticmethod
    def print_info(message: str) -> None:
        """Print an informational message."""
        console.print(f"[bold blue]ℹ[/bold blue] {message}")

    @staticmethod
    def print_warning(message: str) -> None:
        """Print a warning message with yellow alert icon."""
        console.print(f"[bold yellow]⚠[/bold yellow] [yellow]{message}[/yellow]")

    @staticmethod
    def print_error(message: str, details: str | None = None) -> None:
        """Print a styled error panel."""
        err_text = f"[bold red]{message}[/bold red]"
        if details:
            err_text += f"\n\n[dim red]{details}[/dim red]"
        error_console.print(
            Panel(
                err_text,
                title="[bold red]Error[/bold red]",
                border_style="red",
                expand=False,
            )
        )

    @staticmethod
    def print_rollback_notice(reason: str, restored: bool = True) -> None:
        """Print a notice about automatic rollback."""
        text = f"[bold red]Workspace Creation Failed:[/bold red] {reason}\n"
        if restored:
            text += "\n[bold yellow]↺ Automatic rollback executed.[/bold yellow] Filesystem and Git branches restored."
        error_console.print(
            Panel(
                text,
                title="[bold yellow]Rollback Executed[/bold yellow]",
                border_style="yellow",
                expand=False,
            )
        )

    @staticmethod
    def print_creation_header(name: str, repo_specs: Sequence[RepoSpec]) -> None:
        """Print header preview when starting workspace creation."""
        console.print()
        console.print(f"[bold green]✔[/bold green] [bold white]Creating workspace [cyan]{name}[/cyan][/bold white]")
        console.print()
        console.print("[bold cyan]Repositories[/bold cyan]")
        console.print()

        for spec in repo_specs:
            mode_str = "[green]NEW[/green]" if spec.create else "[yellow]EXISTING[/yellow]"
            console.print(f"  [bold underline]{spec.name.capitalize()}[/bold underline]")
            console.print(f"      Branch : [bold white]{spec.branch}[/bold white]")
            console.print(f"      Mode   : {mode_str}")
            console.print()

        console.print(Rule(style="dim white"))
        console.print()

    @staticmethod
    def print_creation_success(name: str, workspace_path: Path) -> None:
        """Print success summary after workspace creation."""
        console.print(f"[bold green]✔ Workspace created[/bold green]")
        console.print()
        console.print("[bold cyan]Location[/bold cyan]")
        console.print(f"  [bold bright_blue]{workspace_path.resolve()}[/bold bright_blue]")
        console.print()

    @staticmethod
    def print_workspace_list(workspaces: list[WorkspaceMetadata]) -> None:
        """Print list of workspaces formatted as a Rich table."""
        if not workspaces:
            console.print("[dim]No workspaces found.[/dim]")
            return

        table = Table(
            title="Workspaces",
            title_style="bold cyan",
            header_style="bold magenta",
            border_style="dim white",
            expand=False,
        )

        table.add_column("NAME", style="bold cyan", no_wrap=True)
        table.add_column("STATUS", style="green", no_wrap=True)
        table.add_column("CREATED", style="dim white", no_wrap=True)
        table.add_column("REPOSITORIES", style="white")

        for ws in sorted(workspaces, key=lambda w: w.name):
            rel_time = format_relative_time(ws.created)
            repo_parts = []
            for name, spec in ws.repositories.items():
                part = f"{name}:{spec.branch}"
                if spec.frozen:
                    part += " [🔒 frozen]"
                repo_parts.append(part)
            repo_summary = ", ".join(repo_parts)
            status_style = "[green]active[/green]" if ws.status == "active" else f"[yellow]{ws.status}[/yellow]"
            table.add_row(ws.name, status_style, rel_time, repo_summary)

        console.print(table)

    @staticmethod
    def print_workspace_info(metadata: WorkspaceMetadata, workspace_path: Path) -> None:
        """Print detailed workspace metadata in a Rich table/tree."""
        tree = Tree(f"[bold cyan]Workspace: [yellow]{metadata.name}[/yellow][/bold cyan]")
        tree.add(f"[bold white]Created:[/bold white] {metadata.created} ({format_relative_time(metadata.created)})")
        tree.add(f"[bold white]Status:[/bold white] [green]{metadata.status}[/green]")
        tree.add(f"[bold white]Path:[/bold white] {workspace_path.resolve()}")

        repo_branch = tree.add("[bold cyan]Repositories[/bold cyan]")
        for repo_name, spec in metadata.repositories.items():
            mode_badge = "[green]new[/green]" if spec.create else "[yellow]existing[/yellow]"
            frozen_badge = " [bold yellow]🔒 FROZEN[/bold yellow]" if spec.frozen else ""
            r_node = repo_branch.add(f"[bold magenta]{repo_name}[/bold magenta] ({mode_badge}){frozen_badge}")
            r_node.add(f"Branch: [bold white]{spec.branch}[/bold white]")
            r_node.add(f"Worktree Path: [dim]{spec.path}[/dim]")
            if spec.frozen:
                r_node.add("Status: [yellow]Read-only (frozen)[/yellow]")

        console.print(
            Panel(
                tree,
                title=f"[bold green]Workspace Info: {metadata.name}[/bold green]",
                border_style="cyan",
            )
        )

    @staticmethod
    def print_push_summary(workspace_name: str, results: dict[str, dict[str, str]]) -> None:
        """Print summary table of push results across workspace repositories."""
        table = Table(
            title=f"Push Summary for Workspace '{workspace_name}'",
            title_style="bold cyan",
            header_style="bold magenta",
            border_style="dim white",
            expand=False,
        )

        table.add_column("REPOSITORY", style="bold white", no_wrap=True)
        table.add_column("STATUS", no_wrap=True)
        table.add_column("BRANCH", style="bold white", no_wrap=True)
        table.add_column("REMOTE", style="dim white", no_wrap=True)
        table.add_column("DETAILS / REASON", style="dim white")

        for repo_name, res in results.items():
            status = res.get("status", "unknown")
            branch = res.get("branch", "-")
            remote = res.get("remote", "origin")
            reason = res.get("reason", "")

            if status == "pushed":
                status_formatted = "[bold green]✔ PUSHED (COMMITS)[/bold green]"
            elif status == "up-to-date":
                status_formatted = "[bold cyan]ℹ UP TO DATE[/bold cyan]"
            elif status == "skipped":
                status_formatted = "[bold yellow]⏭ SKIPPED[/bold yellow]"
            else:
                status_formatted = "[bold red]✘ FAILED[/bold red]"

            table.add_row(repo_name, status_formatted, branch, remote, reason)

        console.print(table)

    @staticmethod
    def print_pull_summary(workspace_name: str, results: dict[str, dict[str, str]]) -> None:
        """Print summary table of pull results across workspace repositories."""
        table = Table(
            title=f"Pull Summary for Workspace '{workspace_name}'",
            title_style="bold cyan",
            header_style="bold magenta",
            border_style="dim white",
            expand=False,
        )

        table.add_column("REPOSITORY", style="bold white", no_wrap=True)
        table.add_column("STATUS", no_wrap=True)
        table.add_column("BRANCH", style="bold white", no_wrap=True)
        table.add_column("REMOTE", style="dim white", no_wrap=True)
        table.add_column("DETAILS / REASON", style="dim white")

        for repo_name, res in results.items():
            status = res.get("status", "unknown")
            branch = res.get("branch", "-")
            remote = res.get("remote", "origin")
            reason = res.get("reason", "")

            if status == "pulled":
                status_formatted = "[bold green]✔ PULLED (UPDATED)[/bold green]"
            elif status == "up-to-date":
                status_formatted = "[bold cyan]ℹ UP TO DATE[/bold cyan]"
            elif status == "skipped":
                status_formatted = "[bold yellow]⏭ SKIPPED[/bold yellow]"
            else:
                status_formatted = "[bold red]✘ FAILED[/bold red]"

            table.add_row(repo_name, status_formatted, branch, remote, reason)

        console.print(table)

    @staticmethod
    def print_setup_summary(workspace_name: str, results: dict[str, dict[str, Any]]) -> None:
        """Print summary table of setup execution and env sync across workspace repositories."""
        table = Table(
            title=f"Setup Summary for Workspace '{workspace_name}'",
            title_style="bold cyan",
            header_style="bold magenta",
            border_style="dim white",
            expand=False,
        )

        table.add_column("REPOSITORY", style="bold white", no_wrap=True)
        table.add_column("STATUS", no_wrap=True)
        table.add_column("ENV SYNC", style="dim white", no_wrap=True)
        table.add_column("DETAILS / COMMANDS", style="dim white")

        for repo_name, res in results.items():
            status = res.get("status", "unknown")
            env_status = res.get("env_status", "-")
            reason = res.get("reason", "")
            cmds = res.get("commands_run", [])

            if status == "completed":
                status_formatted = "[bold green]✔ COMPLETED[/bold green]"
            elif status == "skipped":
                status_formatted = "[bold yellow]⏭ SKIPPED[/bold yellow]"
            else:
                status_formatted = "[bold red]✘ FAILED[/bold red]"

            cmd_details = reason
            if cmds and status == "completed":
                cmd_details = f"{reason} ([dim]{', '.join(cmds)}[/dim])"

            table.add_row(repo_name, status_formatted, str(env_status), cmd_details)

        console.print(table)

    @staticmethod
    def print_setup_repo_start(repo_name: str, path: Path) -> None:
        """Print header when starting setup on a repository worktree."""
        console.print()
        console.print(f"[bold cyan]📦 [{repo_name.upper()}][/bold cyan] [dim white]({path})[/dim white]")

    @staticmethod
    def print_step_start(step_number: int, action: str) -> None:
        """Print start notice for a setup pipeline step."""
        console.print(f"  [cyan]❯[/cyan] [bold white]Step {step_number}:[/bold white] [dim]{action}...[/dim]")

    @staticmethod
    def print_setup_step(step_number: int, title: str, details: str, status: str = "success") -> None:
        """Print a step in the setup pipeline."""
        if status == "success":
            icon = "[bold green]✔[/bold green]"
            title_style = "[bold white]"
        elif status == "info":
            icon = "[bold blue]ℹ[/bold blue]"
            title_style = "[white]"
        elif status == "warning":
            icon = "[bold yellow]⚠[/bold yellow]"
            title_style = "[yellow]"
        else:
            icon = "[bold red]✘[/bold red]"
            title_style = "[bold red]"

        console.print(f"  {icon} {title_style}Step {step_number}: {title}[/{title_style}] [dim]— {details}[/dim]")

    @staticmethod
    def print_command_start(command: str) -> None:
        """Print start notification right before executing a setup command."""
        console.print(f"  [cyan]❯ Running:[/cyan] [bold cyan]{command}[/bold cyan] [dim]...[/dim]")

    @staticmethod
    def print_command_done(command: str, elapsed_seconds: float, success: bool = True, returncode: int = 0) -> None:
        """Print completion summary for a setup command."""
        if success:
            console.print(f"  [bold green]✔[/bold green] [bold white]Completed:[/bold white] [cyan]{command}[/cyan] [dim]({elapsed_seconds:.2f}s)[/dim]")
        else:
            console.print(f"  [bold red]✘[/bold red] [bold red]Failed:[/bold red] [cyan]{command}[/cyan] [bold red](exit {returncode} in {elapsed_seconds:.2f}s)[/bold red]")


    @staticmethod
    def print_env_resolution_details(env_vars: dict[str, str], explicit_secrets: Sequence[str] | None = None) -> None:
        """Print indented breakdown of all resolved environment variables in verbose mode with secret masking."""
        from ws.env import EnvEngine
        console.print("    [dim cyan]Resolved Environment Variables:[/dim cyan]")
        if not env_vars:
            console.print("      [dim](none)[/dim]")
            return

        for k, v in sorted(env_vars.items()):
            display_val = "[yellow]********[/yellow]" if EnvEngine.is_secret_key(k, explicit_secrets) else f"[green]{v}[/green]"
            console.print(f"      [dim white]•[/dim white] [cyan]{k}[/cyan] = {display_val}")

    @staticmethod
    def print_command_output(command: str, stdout: str, stderr: str, returncode: int) -> None:
        """Print styled panel with stdout/stderr from a setup command."""
        output = ""
        if stdout.strip():
            output += f"[bold white]Output:[/bold white]\n{stdout.strip()}\n"
        if stderr.strip():
            output += f"\n[bold red]Errors/Warnings:[/bold red]\n{stderr.strip()}\n"
        if not output.strip():
            output = "[dim](no console output)[/dim]"

        border_col = "green" if returncode == 0 else "red"
        status_tag = f"[bold green]exit {returncode}[/bold green]" if returncode == 0 else f"[bold red]exit {returncode}[/bold red]"

        console.print(
            Panel(
                output.strip(),
                title=f"[{border_col}]Command: {command}[/{border_col}] ({status_tag})",
                border_style=border_col,
                expand=False,
            )
        )

    @staticmethod
    def print_env_table(
        workspace_name: str,
        repo_name: str,
        env_vars: dict[str, str],
        explicit_secrets: Sequence[str] | None = None,
    ) -> None:
        """Print formatted table of resolved environment variables with secret masking."""
        from ws.env import EnvEngine
        table = Table(
            title=f"Environment Variables for '{repo_name}' in '{workspace_name}'",
            title_style="bold cyan",
            header_style="bold magenta",
            border_style="dim white",
            expand=False,
        )

        table.add_column("VARIABLE", style="bold cyan", no_wrap=True)
        table.add_column("RESOLVED VALUE", style="bold green")

        if not env_vars:
            table.add_row("[dim](none)[/dim]", "[dim]No environment variables configured[/dim]")
        else:
            for k, v in sorted(env_vars.items()):
                val_formatted = "[yellow]******** (masked secret)[/yellow]" if EnvEngine.is_secret_key(k, explicit_secrets) else v
                table.add_row(k, val_formatted)

        console.print(table)


    @staticmethod
    def print_launch_summary(workspace_name: str, launch_entries: list[tuple[str, str, str]]) -> None:
        """Print launch command table for workspace services."""
        table = Table(
            title=f"Launch Commands for Workspace '{workspace_name}'",
            title_style="bold cyan",
            header_style="bold magenta",
            border_style="dim white",
            expand=False,
        )

        table.add_column("REPOSITORY", style="bold white", no_wrap=True)
        table.add_column("WORKING DIRECTORY", style="dim white")
        table.add_column("LAUNCH COMMAND", style="bold green")

        if not launch_entries:
            table.add_row("[dim](none)[/dim]", "-", "[dim]No launch commands configured in repositories.yml[/dim]")
        else:
            for repo, wt_dir, cmd in launch_entries:
                table.add_row(repo, wt_dir, cmd)

        console.print(table)

    @staticmethod
    def print_yaml_highlighted(content: str, title: str = "workspace.yml") -> None:
        """Print YAML syntax highlighted content."""
        syntax = Syntax(content, "yaml", theme="monokai", line_numbers=True)
        console.print(Panel(syntax, title=title, border_style="dim cyan"))

    @staticmethod
    def spinner(description: str):
        """Create a Rich progress spinner context manager."""
        return Progress(
            SpinnerColumn(spinner_name="dots"),
            TextColumn("[bold cyan]{task.description}[/bold cyan]"),
            transient=True,
        )


