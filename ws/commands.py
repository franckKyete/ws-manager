"""Command handler implementations decoupling CLI from WorkspaceManager logic."""

import logging
from pathlib import Path
from typing import Sequence

from ws.models import RepoSpec
from ws.output import console, OutputHandler
from ws.workspace import WorkspaceManager

logger = logging.getLogger("ws.commands")


def cmd_new(manager: WorkspaceManager, name: str, repo_specs: Sequence[RepoSpec], run_setup: bool = False) -> None:
    """Execute 'ws create' / 'ws new' command."""
    manager.create_workspace(name=name, repo_specs=repo_specs)
    if run_setup:
        results = manager.setup_workspace(workspace_name=name)
        OutputHandler.print_setup_summary(workspace_name=name, results=results)


def cmd_create(manager: WorkspaceManager, config_file: Path | str, run_setup: bool = False) -> None:
    """Execute 'ws create -f <config.yml>' command using a YAML configuration file."""
    meta = manager.create_workspace_from_config(config_file=config_file)
    if run_setup:
        results = manager.setup_workspace(workspace_name=meta.name)
        OutputHandler.print_setup_summary(workspace_name=meta.name, results=results)



def cmd_list(manager: WorkspaceManager) -> None:
    """Execute 'ws list' command."""
    workspaces = manager.list_workspaces()
    OutputHandler.print_workspace_list(workspaces)


def cmd_info(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws info' command."""
    metadata, ws_path = manager.get_workspace_info(name=name)
    active_engine = manager.get_active_engine(name)
    running_services = manager.get_running_services_status(name)
    OutputHandler.print_workspace_info(
        metadata=metadata,
        workspace_path=ws_path,
        active_engine=active_engine,
        running_services=running_services,
    )



def cmd_delete(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws delete' / 'ws rm' command."""
    manager.remove_workspace(name=name)

cmd_remove = cmd_delete


def cmd_shell(
    manager: WorkspaceManager,
    name: str,
    worktree: str | None = None,
) -> None:
    """Execute 'ws shell' / 'ws enter' command to open an interactive subshell inside workspace or worktree."""
    clean_wt = worktree.lstrip("%+:#$") if worktree else None
    manager.open_workspace(name=name, worktree=clean_wt)

cmd_enter = cmd_shell
cmd_open = cmd_shell



def cmd_stop(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws stop' command to terminate a running workspace session."""
    OutputHandler.print_info(f"Stopping services for workspace '[bold cyan]@{name}[/bold cyan]'...")
    stopped = manager.stop_workspace(name)
    if stopped:
        OutputHandler.print_success(f"Workspace session for '@{name}' terminated.")
    else:
        OutputHandler.print_info(f"No active session found for '@{name}'.")


def cmd_restart(
    manager: WorkspaceManager,
    workspace_name: str,
    repos: Sequence[str] | None = None,
) -> None:
    """Restart services inside a running daemon session."""
    sock_path = manager.get_session_socket_path(workspace_name)
    if not manager.is_session_running(workspace_name):
        OutputHandler.print_warning(f"No running session for workspace '@{workspace_name}'. Starting...")
        cmd_launch(manager=manager, workspace_name=workspace_name, repos=repos)
        return

    target_repos = [r.lstrip("%+:#$") for r in repos] if repos else []
    if not target_repos:
        meta, _ = manager.get_workspace_info(workspace_name)
        target_repos = list(meta.repositories.keys())

    import socket, json
    for r in target_repos:
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                s.settimeout(1.0)
                s.connect(str(sock_path))
                req = json.dumps({"type": "RestartService", "service": r}) + "\n"
                s.sendall(req.encode("utf-8"))
                _ = s.recv(1024)
                OutputHandler.print_success(f"Restarted service '%{r}' in workspace '@{workspace_name}'")
        except Exception as e:
            OutputHandler.print_error(f"Failed restarting service '%{r}': {e}")


def cmd_logs(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str | None = None,
    follow: bool = False,
    lines: int = 50,
) -> None:
    """View logs for workspace services."""
    ws_dir = manager._get_workspace_dir(workspace_name)
    log_dir = ws_dir / ".ws" / "logs"
    if not log_dir.exists():
        OutputHandler.print_warning(f"No log directory found for workspace '@{workspace_name}'.")
        return

    clean_repo = repo_name.lstrip("%+:#$") if repo_name else None
    if clean_repo:
        target_log = log_dir / f"{clean_repo}.log"
        if not target_log.exists():
            OutputHandler.print_error(f"No log file found for service '%{clean_repo}' at {target_log}")
            return
        log_files = [target_log]
    else:
        log_files = sorted(log_dir.glob("*.log"))
        if not log_files:
            OutputHandler.print_info(f"No log files found in '{log_dir}'.")
            return

    for lf in log_files:
        OutputHandler.print_info(f"[bold cyan]Log: {lf.name}[/bold cyan]")
        try:
            with open(lf, "r", encoding="utf-8", errors="replace") as f:
                content = f.readlines()
                for line in content[-lines:]:
                    console.print(line, end="")
        except Exception as e:
            OutputHandler.print_error(f"Error reading {lf}: {e}")


def cmd_status(manager: WorkspaceManager, name: str) -> None:
    """Execute 'ws status' command across all repository worktrees in a workspace."""
    manager.status_workspace(name=name)


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
    from ws.network import get_lan_ip, list_network_interfaces
    results = manager.doctor()
    console.print("[bold cyan]System Diagnostics & Health Check[/bold cyan]\n")
    all_ok = True
    for check, ok in results.items():
        badge = "[bold green]PASS[/bold green]" if ok else "[bold red]FAIL[/bold red]"
        if not ok:
            all_ok = False
        console.print(f"  {check:<32} : {badge}")

    console.print("\n[bold cyan]Network Discovery & Interfaces[/bold cyan]")
    ifaces = list_network_interfaces()
    if ifaces:
        for iface in ifaces:
            wl_badge = "[green](Wireless Wi-Fi)[/green]" if iface["is_wireless"] else f"({iface['type']})"
            console.print(f"  Interface: [bold]{iface['name']}[/bold] -> {iface['ip']} {wl_badge}")
    else:
        console.print("  No physical network interfaces detected.")
    console.print(f"  Active LAN IP (Prioritized) : [bold green]{get_lan_ip()}[/bold green]\n")

    if all_ok:
        OutputHandler.print_success("All system health checks passed!")
    else:
        OutputHandler.print_warning("Some health checks failed. Please inspect repository configurations.")


def cmd_init(manager: WorkspaceManager, repo_inputs: Sequence[str]) -> None:
    """Execute 'ws init' / 'ws project init' command."""
    manager.init_project(repo_inputs=repo_inputs)


def cmd_add(manager: WorkspaceManager, repo_input: str) -> None:
    """Execute 'ws add' / 'ws project add' command."""
    manager.init_project(repo_inputs=[repo_input])


def cmd_antigravity(manager: WorkspaceManager) -> None:
    """Easter egg / AI agent health overview."""
    console.print("[bold green]🚀 Google Antigravity Agent Workspace Diagnostic[/bold green]")
    console.print("Workspace Manager version: [cyan]0.1.0[/cyan]")
    console.print("Repositories active: [yellow]" + str(len(manager.config.repositories)) + "[/yellow]")
    console.print("Workspaces registered: [yellow]" + str(len(manager.list_workspaces())) + "[/yellow]")
    OutputHandler.print_success("Antigravity engine nominal.")


def cmd_repo_add(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_input: str,
    branch: str | None = None,
    existing: bool = False,
) -> None:
    """Execute 'ws repo add @<workspace> %<repo>:[branch]'."""
    clean_input = repo_input.lstrip("%+:#$")
    create_mode = not existing

    if ":" in clean_input:
        parts = clean_input.split(":")
        r_name = parts[0]
        r_branch = parts[1] if len(parts) > 1 and parts[1] else None
        if len(parts) > 2 and parts[2] == "existing":
            create_mode = False
        elif len(parts) > 2 and parts[2] == "new":
            create_mode = True
    elif "=" in clean_input:
        parts = clean_input.split("=", 1)
        r_name = parts[0]
        r_branch = parts[1]
        if r_branch.endswith(":existing"):
            r_branch = r_branch[:-9]
            create_mode = False
        elif r_branch.endswith(":new"):
            r_branch = r_branch[:-4]
            create_mode = True
    else:
        r_name = clean_input
        r_branch = branch

    if not r_branch:
        r_branch = workspace_name if not create_mode else f"feature/{workspace_name}"

    manager.workspace_add_repo(
        workspace_name=workspace_name,
        repo_name=r_name,
        branch=r_branch,
        create=create_mode,
    )

cmd_workspace_add_repo = cmd_repo_add


def cmd_repo_remove(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
    delete_branch: bool = False,
) -> None:
    """Execute 'ws repo remove @<workspace> %<repo>'."""
    clean_repo = repo_name.lstrip("%+:#$")
    manager.workspace_remove_repo(
        workspace_name=workspace_name,
        repo_name=clean_repo,
        delete_branch=delete_branch,
    )

cmd_workspace_remove_repo = cmd_repo_remove


def cmd_repo_lock(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
) -> None:
    """Execute 'ws repo lock @<workspace> %<repo>' (alias: 'ws lock')."""
    clean_repo = repo_name.lstrip("%+:#$")
    manager.lock_repo(workspace_name=workspace_name, repo_name=clean_repo)

cmd_lock = cmd_repo_lock
cmd_workspace_freeze = cmd_repo_lock


def cmd_repo_unlock(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str,
) -> None:
    """Execute 'ws repo unlock @<workspace> %<repo>' (alias: 'ws unlock')."""
    clean_repo = repo_name.lstrip("%+:#$")
    manager.unlock_repo(workspace_name=workspace_name, repo_name=clean_repo)

cmd_unlock = cmd_repo_unlock
cmd_workspace_unfreeze = cmd_repo_unlock


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


def cmd_setup(
    manager: WorkspaceManager,
    workspace_name: str,
    repos: Sequence[str] | None = None,
    dry_run: bool = False,
    skip_scripts: bool = False,
    verbose: bool = False,
    interface: str | None = None,
    lan_ip: str | None = None,
) -> None:
    """Execute 'ws setup' command."""
    results = manager.setup_workspace(
        workspace_name=workspace_name,
        repos=repos,
        dry_run=dry_run,
        skip_scripts=skip_scripts,
        verbose=verbose,
        interface=interface,
        lan_ip=lan_ip,
    )
    OutputHandler.print_setup_summary(workspace_name=workspace_name, results=results)



def cmd_env(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str | None = None,
    sync: bool = False,
    interface: str | None = None,
    lan_ip: str | None = None,
) -> None:
    """Execute 'ws env' command to inspect or sync environment variables."""
    if sync:
        repos = [repo_name] if repo_name else None
        results = manager.sync_env(
            workspace_name=workspace_name,
            repos=repos,
            interface=interface,
            lan_ip=lan_ip,
        )
        OutputHandler.print_setup_summary(workspace_name=workspace_name, results=results)
    else:
        meta, _ = manager.get_workspace_info(workspace_name)
        target_repos = [repo_name] if repo_name else list(meta.repositories.keys())
        for r in target_repos:
            env_vars = manager.get_env_vars(
                workspace_name=workspace_name,
                repo_name=r,
                interface=interface,
                lan_ip=lan_ip,
            )
            repo_cfg = manager.config.repositories.get(r)
            repo_secrets = repo_cfg.secrets if repo_cfg else []
            all_secrets = list(set(manager.config.secrets + repo_secrets))
            OutputHandler.print_env_table(
                workspace_name=workspace_name,
                repo_name=r,
                env_vars=env_vars,
                explicit_secrets=all_secrets,
            )
def cmd_launch(
    manager: WorkspaceManager,
    workspace_name: str,
    repos: Sequence[str] | None = None,
    mode: str = "tui",
    attach_repo: str | None = None,
    daemon: bool = False,
    switch: bool = False,
    interface: str | None = None,
    lan_ip: str | None = None,
) -> None:
    """Execute 'ws launch' command to start services concurrently."""
    manager.launch_workspace(
        workspace_name=workspace_name,
        repos=repos,
        mode=mode,
        attach_repo=attach_repo,
        daemon=daemon,
        switch=switch,
        interface=interface,
        lan_ip=lan_ip,
    )



def cmd_attach(
    manager: WorkspaceManager,
    workspace_name: str,
    repo_name: str | None = None,
    all_panes: bool = False,
    mode: str | None = None,
    switch: bool = False,
) -> None:
    """Interactively attach to a running workspace session."""
    active_engine = manager.get_active_engine(workspace_name)
    target_engine = mode or active_engine or "tui"

    if active_engine and target_engine != active_engine:
        if switch:
            manager.launch_workspace(
                workspace_name=workspace_name,
                mode=target_engine,
                attach_repo=repo_name,
                switch=True,
            )
            return
        else:
            OutputHandler.print_error(
                f"Workspace '{workspace_name}' is currently running in {active_engine}.\n"
                f"To switch engines, use '--switch' (e.g. 'ws attach {workspace_name} --{target_engine} --switch')."
            )
            return

    project_name = manager.config.project_root.name
    from ws.multiplexer import TmuxLauncher, ZellijLauncher

    if active_engine == "tmux" or (not active_engine and mode == "tmux"):
        OutputHandler.print_info(
            f"Attaching to running Tmux window for workspace: [bold cyan]{workspace_name}[/bold cyan] "
            f"({'all panes' if all_panes else (repo_name or 'fullscreen pane')})"
        )
        TmuxLauncher.attach(
            workspace_name=workspace_name,
            project_name=project_name,
            repo_name=repo_name,
            all_panes=all_panes,
        )
        return

    if active_engine == "zellij" or (not active_engine and mode == "zellij"):
        OutputHandler.print_info(
            f"Attaching to running Zellij session for workspace: [bold cyan]{workspace_name}[/bold cyan]"
        )
        ZellijLauncher.attach(
            workspace_name=workspace_name,
            project_name=project_name,
            repo_name=repo_name,
            all_panes=all_panes,
        )
        return

    socket_path = manager.get_session_socket_path(workspace_name)
    try:
        from ws._native import attach_workspace_session
        fullscreen = not all_panes
        OutputHandler.print_info(
            f"Attaching to running native session for workspace: [bold cyan]{workspace_name}[/bold cyan] "
            f"({'grid view' if all_panes else (repo_name or 'fullscreen pane')})"
        )
        exit_code = attach_workspace_session(
            workspace_name=workspace_name,
            socket_path=str(socket_path),
            initial_focus=repo_name,
            fullscreen=fullscreen,
        )
        if exit_code != 0:
            logger.info("Native TUI session finished with exit code %d", exit_code)
    except ImportError:
        OutputHandler.print_error(
            "Native TUI engine is not compiled.\n"
            "Build with 'cargo build --workspace' or run with --stream / --terminal."
        )
    except Exception as e:
        OutputHandler.print_error(f"Failed attaching to session: {e}")


def cmd_bridge(manager: WorkspaceManager, workspace_name: str, repo_name: str) -> None:
    """Execute high-performance raw PTY bridge to background daemon service."""
    from ws.cli import clean_workspace, clean_repo
    clean_ws = clean_workspace(workspace_name)
    clean_r = clean_repo(repo_name)
    sock_path = manager.get_session_socket_path(clean_ws)
    if not manager.is_daemon_active(clean_ws):
        OutputHandler.print_error(
            f"No active session found for workspace '{clean_ws}'.\n"
            f"Start services first using: [bold yellow]ws start @{clean_ws}[/bold yellow]"
        )
        return
    try:
        from ws._native import run_raw_bridge
        run_raw_bridge(str(sock_path), clean_r)
    except Exception as e:
        OutputHandler.print_error(f"Bridge connection error: {e}")



cmd_start = cmd_launch
cmd_run = cmd_launch


def cmd_completion(shell: str | None = None, install: bool = False) -> None:
    """Execute 'ws completion [zsh|bash|fish|install]' command."""
    from ws.completion import generate_completion_script, install_completion
    target = (shell or "zsh").lower()
    if target == "install" or install:
        ok, msg = install_completion(shell=None)
        if ok:
            console.print(f"[bold green]{msg}[/bold green]")
        else:
            OutputHandler.print_error(msg)
    else:
        try:
            script = generate_completion_script(target)
            print(script)
        except Exception as e:
            OutputHandler.print_error(str(e))


def cmd_internal_complete(query_type: str, args: list[str]) -> None:
    """Hidden command handler for shell completion hook queries: 'ws _complete <type> [args]'."""
    from ws.completion import query_completions
    results = query_completions(query_type, *args)
    for r in results:
        print(r)



