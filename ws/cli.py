"""Command Line Interface entrypoint and argument parser for ws."""

import argparse
import logging
from pathlib import Path
import sys
from typing import Sequence

from ws import __version__
from ws.commands import (
    cmd_add,
    cmd_antigravity,
    cmd_attach,
    cmd_bridge,
    cmd_completion,
    cmd_create,
    cmd_delete,
    cmd_doctor,
    cmd_env,
    cmd_exec,
    cmd_fetch,
    cmd_info,
    cmd_init,
    cmd_internal_complete,
    cmd_launch,
    cmd_list,
    cmd_logs,
    cmd_new,
    cmd_open,
    cmd_pull,
    cmd_push,
    cmd_remove,
    cmd_repo_add,
    cmd_repo_lock,
    cmd_repo_remove,
    cmd_repo_unlock,
    cmd_restart,
    cmd_setup,
    cmd_shell,
    cmd_start,
    cmd_status,
    cmd_stop,
    cmd_sync,
    cmd_workspace_add_repo,
    cmd_workspace_freeze,
    cmd_workspace_remove_repo,
    cmd_workspace_unfreeze,
    cmd_hub_login,
    cmd_hub_whoami,
    cmd_hub_logout,
    cmd_hub_clone,
    cmd_hub_publish,
    cmd_hub_push,
    cmd_hub_pull,
    cmd_hub_status,
    cmd_hub_sync,
    cmd_hub_state_save,
    cmd_hub_state_restore,
    cmd_hub_secret_list,
    cmd_hub_secret_set,
    cmd_hub_secret_get,
    cmd_hub_secret_delete,
    cmd_hub_secret_upload,
    cmd_hub_secret_pull,
)
from ws.config import ConfigLoader
from ws.exceptions import WSException
from ws.models import RepoConfig, RepoSpec
from ws.output import OutputHandler
from ws.workspace import WorkspaceManager

logger = logging.getLogger("ws.cli")

KNOWN_COMMANDS = {
    "create", "new", "list", "ls", "info", "delete", "rm", "remove",
    "status", "exec", "push", "pull", "start", "launch", "run",
    "attach", "stop", "kill", "restart", "logs", "shell", "enter", "open",
    "env", "setup", "bridge",
    "repo", "lock", "unlock", "workspace",
    "project", "init", "add", "fetch", "sync", "doctor", "antigravity",
    "completion", "_complete",
    "hub", "clone",
}



def clean_workspace(name: str | None) -> str | None:
    """Strip leading @ sigil from workspace name."""
    if name is None:
        return None
    return name.lstrip("@")


def clean_repo(name: str | None) -> str | None:
    """Strip leading %, +, :, #, $ sigil from repository or service name."""
    if name is None:
        return None
    return name.lstrip("%+:#$")


def clean_repos(repos: Sequence[str] | None) -> list[str] | None:
    """Strip leading sigils from list of repository names."""
    if repos is None:
        return None
    return [clean_repo(r) for r in repos if r is not None]



def normalize_cli_args(sys_args: Sequence[str]) -> list[str]:
    """Normalize CLI arguments to support both 'ws <command> @<workspace> ...' and 'ws @<workspace> <command> ...'."""
    args_list = list(sys_args)
    if not args_list:
        return args_list

    first = args_list[0]
    # Check if first argument is a workspace name with @ sigil (e.g. '@develop start --tmux')
    if first.startswith("@") and len(args_list) >= 2:
        second = args_list[1]
        if second in KNOWN_COMMANDS:
            return [second, first] + args_list[2:]

    # Check if first argument is a positional workspace name without flag (e.g. 'develop start')
    if first not in KNOWN_COMMANDS and not first.startswith("-") and len(args_list) >= 2:
        second = args_list[1]
        if second in KNOWN_COMMANDS:
            return [second, first] + args_list[2:]

    return args_list


def parse_create_workspace_args(
    workspace_name: str,
    raw_args: list[str],
    repositories: dict[str, RepoConfig],
) -> list[RepoSpec]:
    """Parse parameters for 'ws create @<name> [#repo[:branch[:mode]] ...] [--all] [--existing]'."""
    clean_ws = clean_workspace(workspace_name) or workspace_name
    global_existing = False
    include_all = False
    selected_repos: set[str] | None = None
    explicit_specs: dict[str, RepoSpec] = {}

    idx = 0
    while idx < len(raw_args):
        arg = raw_args[idx]

        if arg == "--all":
            include_all = True
            idx += 1
            continue
        elif arg in ("--repos", "--only"):
            if idx + 1 >= len(raw_args):
                raise WSException(f"Option '{arg}' requires a comma-separated list of repository names")
            repos_str = raw_args[idx + 1]
            selected_repos = set(clean_repo(r.strip()) for r in repos_str.split(",") if r.strip())
            idx += 2
            continue
        elif arg.startswith("--repos=") or arg.startswith("--only="):
            repos_str = arg.split("=", 1)[1]
            selected_repos = set(clean_repo(r.strip()) for r in repos_str.split(",") if r.strip())
            idx += 1
            continue
        elif arg == "--existing":
            global_existing = True
            idx += 1
            continue
        elif arg == "--new":
            global_existing = False
            idx += 1
            continue

        # Check colon tag syntax (e.g. #server:main, #server:main:existing, server:main:new)
        clean_arg = clean_repo(arg)
        if ":" in clean_arg and not clean_arg.startswith("-"):
            parts = clean_arg.split(":")
            repo_key = parts[0]
            if repo_key not in repositories:
                raise WSException(
                    f"Unknown repository '{repo_key}' in argument '{arg}'. "
                    f"Configured repositories: {', '.join(repositories.keys())}"
                )
            branch_val = parts[1] if len(parts) > 1 and parts[1] else clean_ws
            create_mode = not global_existing
            if len(parts) > 2:
                if parts[2] == "existing":
                    create_mode = False
                elif parts[2] == "new":
                    create_mode = True

            explicit_specs[repo_key] = RepoSpec(
                name=repo_key,
                branch=branch_val,
                create=create_mode,
                path=repositories[repo_key].checkout,
            )
            idx += 1
            continue

        # Check key=value format (e.g. #server=main, server=main:existing)
        if "=" in clean_arg and not clean_arg.startswith("-"):
            repo_key, branch_val = clean_arg.split("=", 1)
            if repo_key not in repositories:
                raise WSException(
                    f"Unknown repository '{repo_key}' in argument '{arg}'. "
                    f"Configured repositories: {', '.join(repositories.keys())}"
                )

            create_mode = not global_existing
            if branch_val.endswith(":existing"):
                branch_name = branch_val[:-9]
                create_mode = False
            elif branch_val.endswith(":new"):
                branch_name = branch_val[:-4]
                create_mode = True
            else:
                branch_name = branch_val
                if idx + 1 < len(raw_args):
                    if raw_args[idx + 1] == "--existing":
                        create_mode = False
                        idx += 1
                    elif raw_args[idx + 1] == "--new":
                        create_mode = True
                        idx += 1

            explicit_specs[repo_key] = RepoSpec(
                name=repo_key,
                branch=branch_name,
                create=create_mode,
                path=repositories[repo_key].checkout,
            )
            idx += 1
            continue

        # Check positional repo name (e.g. #server, #mobile, server)
        if clean_arg in repositories and not clean_arg.startswith("-"):
            repo_key = clean_arg
            create_mode = not global_existing
            if idx + 1 < len(raw_args):
                if raw_args[idx + 1] == "--existing":
                    create_mode = False
                    idx += 1
                elif raw_args[idx + 1] == "--new":
                    create_mode = True
                    idx += 1

            branch_name = clean_ws if not create_mode else f"feature/{clean_ws}"
            explicit_specs[repo_key] = RepoSpec(
                name=repo_key,
                branch=branch_name,
                create=create_mode,
                path=repositories[repo_key].checkout,
            )
            idx += 1
            continue

        # Check legacy / explicit flags (--server-new, --server-existing, --server)
        matched = False
        for repo_name in repositories:
            new_flag = f"--{repo_name}-new"
            exist_flag = f"--{repo_name}-existing"
            plain_flag = f"--{repo_name}"

            if arg == new_flag:
                if idx + 1 >= len(raw_args):
                    raise WSException(f"Option '{new_flag}' requires a branch argument")
                branch_name = raw_args[idx + 1]
                explicit_specs[repo_name] = RepoSpec(
                    name=repo_name,
                    branch=branch_name,
                    create=True,
                    path=repositories[repo_name].checkout,
                )
                idx += 2
                matched = True
                break

            elif arg == exist_flag:
                if idx + 1 >= len(raw_args):
                    raise WSException(f"Option '{exist_flag}' requires a branch argument")
                branch_name = raw_args[idx + 1]
                explicit_specs[repo_name] = RepoSpec(
                    name=repo_name,
                    branch=branch_name,
                    create=False,
                    path=repositories[repo_name].checkout,
                )
                idx += 2
                matched = True
                break

            elif arg == plain_flag:
                if idx + 1 >= len(raw_args):
                    raise WSException(f"Option '{plain_flag}' requires a branch argument")
                branch_name = raw_args[idx + 1]
                explicit_specs[repo_name] = RepoSpec(
                    name=repo_name,
                    branch=branch_name,
                    create=not global_existing,
                    path=repositories[repo_name].checkout,
                )
                idx += 2
                matched = True
                break

        if not matched:
            raise WSException(f"Unknown argument or option: '{arg}'")

    if not include_all and selected_repos is None and not explicit_specs:
        raise WSException(
            "Explicit repository selection required. "
            "Specify repositories as '%repo[:branch]' (e.g. 'ws create @name %mobile %server'), "
            "or specify '--all'."
        )


    if include_all:
        target_names = set(repositories.keys())
    elif selected_repos is not None:
        target_names = selected_repos
        for r in target_names:
            if r not in repositories:
                raise WSException(
                    f"Repository '{r}' is not in project configuration. "
                    f"Available repositories: {', '.join(repositories.keys())}"
                )
    else:
        target_names = set(explicit_specs.keys())

    final_specs: list[RepoSpec] = []
    for repo_name in sorted(target_names):
        if repo_name in explicit_specs:
            final_specs.append(explicit_specs[repo_name])
        else:
            repo_cfg = repositories[repo_name]
            if global_existing:
                branch = clean_ws
                create = False
            else:
                branch = f"feature/{clean_ws}"
                create = True

            final_specs.append(
                RepoSpec(
                    name=repo_name,
                    branch=branch,
                    create=create,
                    path=repo_cfg.checkout,
                )
            )

    return final_specs

parse_new_workspace_args = parse_create_workspace_args


def build_parser() -> argparse.ArgumentParser:
    """Build the top-level CLI argument parser with coherent command semantics."""
    parser = argparse.ArgumentParser(
        prog="ws",
        description="Multi-repository Git workspace manager using git worktree.",
        formatter_class=argparse.RawTextHelpFormatter,
    )

    parser.add_argument("-v", "--verbose", action="store_true", help="Enable debug log output")
    parser.add_argument("-c", "--config", type=str, help="Path to repositories configuration file")
    parser.add_argument("-w", "--workspaces-dir", type=str, help="Directory for storing workspaces (default: workspaces)")
    parser.add_argument("--version", action="version", version=f"%(prog)s {__version__}")

    subparsers = parser.add_subparsers(dest="subcommand", title="subcommands", metavar="COMMAND")

    # ==================== 1. Workspace Lifecycle ====================
    # Command: ws create @<name> [%repo[:branch] ...] / ws create -f <config.yml>
    p_create = subparsers.add_parser("create", aliases=["new"], help="Create a workspace from parameters or YAML file")
    p_create.add_argument("name", nargs="?", default=None, help="Workspace name (@<name>)")
    p_create.add_argument("-f", "--file", type=str, default=None, help="Path to workspace YAML configuration file")
    p_create.add_argument("--setup", action="store_true", help="Run setup scripts and sync environment variables after creation")

    # Command: ws list / ws ls
    subparsers.add_parser("list", aliases=["ls"], help="List all workspaces")

    # Command: ws info @<name>
    p_info = subparsers.add_parser("info", help="Display details and live process status for a workspace")
    p_info.add_argument("name", help="Workspace name (@<name>)")

    # Command: ws delete @<name>
    p_delete = subparsers.add_parser("delete", aliases=["rm", "remove"], help="Delete a workspace and prune all its worktrees")
    p_delete.add_argument("name", help="Workspace name (@<name>)")

    # Command: ws status @<name>
    p_status = subparsers.add_parser("status", help="Show Git status across all workspace worktrees")
    p_status.add_argument("name", help="Workspace name (@<name>)")

    # Command: ws exec @<name> -- <command...>
    p_exec = subparsers.add_parser("exec", help="Execute command inside each repo worktree of a workspace")
    p_exec.add_argument("name", help="Workspace name (@<name>)")
    p_exec.add_argument("command", nargs=argparse.REMAINDER, help="Command to execute")

    # Command: ws push @<name> [%repos...] [--remote origin]
    p_push = subparsers.add_parser("push", help="Push committed changes for workspace repositories to remotes")
    p_push.add_argument("name", help="Workspace name (@<name>)")
    p_push.add_argument("repos", nargs="*", help="Repository names to push (%%<repo>)")
    p_push.add_argument("--repos", dest="repos_flag", type=str, help="Comma-separated list of repository names")
    p_push.add_argument("--remote", type=str, default="origin", help="Git remote name (default: origin)")

    # Command: ws pull @<name> [%repos...] [--remote origin]
    p_pull = subparsers.add_parser("pull", help="Pull remote updates for workspace repositories")
    p_pull.add_argument("name", help="Workspace name (@<name>)")
    p_pull.add_argument("repos", nargs="*", help="Repository names to pull (%%<repo>)")
    p_pull.add_argument("--repos", dest="repos_flag", type=str, help="Comma-separated list of repository names")
    p_pull.add_argument("--remote", type=str, default="origin", help="Git remote name (default: origin)")

    # ==================== 2. Worktree & Repo Management ====================
    # Command: ws repo <add|remove|lock|unlock>
    p_repo = subparsers.add_parser("repo", aliases=["workspace"], help="Manage repositories inside an existing workspace")
    repo_subparsers = p_repo.add_subparsers(dest="repo_subcommand", title="repo actions", metavar="ACTION")

    p_repo_add = repo_subparsers.add_parser("add", aliases=["add-repo"], help="Add a repository worktree to an existing workspace")
    p_repo_add.add_argument("name", help="Workspace name (@<name>)")
    p_repo_add.add_argument("repo", help="Repository specification (%%<repo>[:branch])")
    p_repo_add.add_argument("branch", nargs="?", default=None, help="Git branch name (optional if specified in repo)")
    p_repo_add.add_argument("--existing", action="store_true", help="Checkout existing branch instead of creating new branch")

    p_repo_rm = repo_subparsers.add_parser("remove", aliases=["rm", "remove-repo"], help="Remove a repository worktree from a workspace")
    p_repo_rm.add_argument("name", help="Workspace name (@<name>)")
    p_repo_rm.add_argument("repo", help="Repository name (%%<repo>)")
    p_repo_rm.add_argument("--delete-branch", action="store_true", help="Also delete the branch from bare repository")

    p_repo_lock = repo_subparsers.add_parser("lock", aliases=["freeze"], help="Lock repository worktree (mark files read-only)")
    p_repo_lock.add_argument("name", help="Workspace name (@<name>)")
    p_repo_lock.add_argument("repo", help="Repository name (%%<repo>)")

    p_repo_unlock = repo_subparsers.add_parser("unlock", aliases=["unfreeze"], help="Unlock repository worktree (restore write permissions)")
    p_repo_unlock.add_argument("name", help="Workspace name (@<name>)")
    p_repo_unlock.add_argument("repo", help="Repository name (%%<repo>)")

    # Direct top-level shortcuts for lock/unlock
    p_lock = subparsers.add_parser("lock", help="Lock repository worktree (read-only)")
    p_lock.add_argument("name", help="Workspace name (@<name>)")
    p_lock.add_argument("repo", help="Repository name (%%<repo>)")

    p_unlock = subparsers.add_parser("unlock", help="Unlock repository worktree (writable)")
    p_unlock.add_argument("name", help="Workspace name (@<name>)")
    p_unlock.add_argument("repo", help="Repository name (%%<repo>)")

    # ==================== 3. Service Runtime & Multiplexers ====================
    # Command: ws start @<name> [%repos...] [--tmux|-z|-d|-t|--stream] [--switch]
    p_start = subparsers.add_parser("start", aliases=["launch", "run"], help="Start workspace services concurrently")
    p_start.add_argument("name", help="Workspace name (@<name>)")
    p_start.add_argument("repos", nargs="*", help="Services to start (%%<repo>)")
    p_start.add_argument("--all", action="store_true", help="Start all services in workspace")
    p_start.add_argument("--repos", "--only", dest="repos_flag", type=str, help="Comma-separated list of services")
    p_start.add_argument("--attach", type=str, default=None, help="Focus or connect directly to a single service")
    p_start.add_argument("--tmux", action="store_true", help="Launch in tmux session with side-by-side vertical panes")
    p_start.add_argument("--zellij", "-z", action="store_true", help="Launch in Zellij session")
    p_start.add_argument("--terminal", "-t", action="store_true", help="Launch in separate terminal windows/tabs")
    p_start.add_argument("--stream", action="store_true", help="Stream raw stdout/stderr without interactive TUI")
    p_start.add_argument("--daemon", "-d", "--background", dest="daemon", action="store_true", help="Launch detached in background daemon")
    p_start.add_argument("--switch", "-s", action="store_true", help="Zero-downtime switch to target presentation engine")
    p_start.add_argument("--mode", "-m", choices=["tui", "zellij", "tmux", "terminal", "stream", "attach", "daemon", "summary", "list"], default=None, help="Multiplexer/UI mode")
    p_start.add_argument("--interface", "--iface", "--lan-interface", dest="interface", default=None, help="Network interface name (e.g. wlan0, eno1) or type (wifi, ethernet) to populate LAN IP")
    p_start.add_argument("--ip", "--lan-ip", dest="lan_ip", default=None, help="Explicit host LAN IP address override")

    # Command: ws attach @<name> [%repo]
    p_attach = subparsers.add_parser("attach", help="Attach to a running workspace session")
    p_attach.add_argument("name", help="Workspace name (@<name>)")
    p_attach.add_argument("repo", nargs="?", default=None, help="Service name to focus (%%<repo>)")
    p_attach.add_argument("--all", action="store_true", help="Attach in multi-pane grid view")
    p_attach.add_argument("--switch", "-s", action="store_true", help="Zero-downtime switch presentation engine")
    p_attach.add_argument("--tmux", action="store_true", help="Attach using tmux backend")
    p_attach.add_argument("--zellij", "-z", action="store_true", help="Attach using Zellij backend")
    p_attach.add_argument("--mode", "-m", choices=["tui", "zellij", "tmux"], default=None, help="Multiplexer engine backend")

    # Command: ws stop @<name>
    p_stop = subparsers.add_parser("stop", aliases=["kill"], help="Stop running workspace background session")
    p_stop.add_argument("name", help="Workspace name (@<name>)")

    # Command: ws restart @<name> [%repos...]
    p_restart = subparsers.add_parser("restart", help="Restart running workspace services")
    p_restart.add_argument("name", help="Workspace name (@<name>)")
    p_restart.add_argument("repos", nargs="*", help="Services to restart (%%<repo>)")

    # Command: ws logs @<name> [%repo] [-f]
    p_logs = subparsers.add_parser("logs", help="View service logs")
    p_logs.add_argument("name", help="Workspace name (@<name>)")
    p_logs.add_argument("repo", nargs="?", default=None, help="Service name (%%<repo>)")
    p_logs.add_argument("-f", "--follow", action="store_true", help="Follow log output")
    p_logs.add_argument("-n", "--lines", type=int, default=50, help="Number of lines to display (default: 50)")

    # Command: ws bridge @<name> %<repo>
    p_bridge = subparsers.add_parser("bridge", help="Connect raw terminal I/O bridge to a running workspace service")
    p_bridge.add_argument("name", help="Workspace name (@<name>)")
    p_bridge.add_argument("repo", help="Repository service name (%%<repo>)")

    # ==================== 4. Developer Shell & Environment ====================
    # Command: ws shell @<name> [%worktree]
    p_shell = subparsers.add_parser("shell", aliases=["enter", "open"], help="Open interactive subshell inside workspace or worktree")
    p_shell.add_argument("name", help="Workspace name (@<name>)")
    p_shell.add_argument("worktree", nargs="?", default=None, help="Repository worktree name to open subshell into (%%<repo>)")

    # Command: ws env @<name> [%repo]
    p_env = subparsers.add_parser("env", help="Inspect or sync environment variables for a workspace")
    p_env.add_argument("name", help="Workspace name (@<name>)")
    p_env.add_argument("repo", nargs="?", default=None, help="Repository name (%%<repo>)")
    p_env.add_argument("--sync", action="store_true", help="Sync resolved environment variables into worktree .env files")
    p_env.add_argument("--interface", "--iface", "--lan-interface", dest="interface", default=None, help="Network interface name (e.g. wlan0, eno1) or type (wifi, ethernet) to populate LAN IP")
    p_env.add_argument("--ip", "--lan-ip", dest="lan_ip", default=None, help="Explicit host LAN IP address override")

    # Command: ws setup @<name> [%repos...]
    p_setup = subparsers.add_parser("setup", help="Run setup scripts and environment variable sync for a workspace")
    p_setup.add_argument("name", help="Workspace name (@<name>)")
    p_setup.add_argument("repos", nargs="*", help="Repository names to setup (%%<repo>)")
    p_setup.add_argument("--all", action="store_true", help="Setup all repositories in the workspace")
    p_setup.add_argument("--repos", "--only", dest="repos_flag", type=str, help="Comma-separated list of repository names to setup")
    p_setup.add_argument("--dry-run", action="store_true", help="Print setup commands without executing them")
    p_setup.add_argument("--skip-scripts", action="store_true", help="Only sync environment variables without running setup scripts")
    p_setup.add_argument("--interface", "--iface", "--lan-interface", dest="interface", default=None, help="Network interface name (e.g. wlan0, eno1) or type (wifi, ethernet) to populate LAN IP")
    p_setup.add_argument("--ip", "--lan-ip", dest="lan_ip", default=None, help="Explicit host LAN IP address override")



    # ==================== 5. Project & Bare Repositories ====================
    # Command: ws project <init|add|fetch|sync>
    p_project = subparsers.add_parser("project", help="Manage project-wide bare repositories")
    proj_subparsers = p_project.add_subparsers(dest="proj_subcommand", title="project actions", metavar="ACTION")

    p_proj_init = proj_subparsers.add_parser("init", help="Initialize project and clone bare repositories")
    p_proj_init.add_argument("urls", nargs="*", help="Git repository URLs")

    p_proj_add = proj_subparsers.add_parser("add", help="Add and clone a new bare repository")
    p_proj_add.add_argument("url", help="Git repository URL or name=URL")

    proj_subparsers.add_parser("fetch", help="Fetch updates in all bare repositories")
    proj_subparsers.add_parser("sync", help="Sync and prune worktrees")

    # Direct top-level shortcuts for project commands
    p_init = subparsers.add_parser("init", help="Initialize project and clone bare repositories")
    p_init.add_argument("urls", nargs="*", help="Git repository URLs")

    p_add = subparsers.add_parser("add", help="Add and clone a new bare repository")
    p_add.add_argument("url", help="Git repository URL or name=URL")

    subparsers.add_parser("fetch", help="Fetch updates in all bare repositories")
    subparsers.add_parser("sync", help="Sync and prune worktrees")
    subparsers.add_parser("doctor", help="Run system health checks and diagnostics")
    subparsers.add_parser("antigravity", help="Antigravity AI agent workspace health check")

    # ==================== 6. wshub Cloud Integration ====================
    # Command: ws hub <login|whoami|logout|clone|publish|push|pull|status|sync|state|secret>
    p_hub = subparsers.add_parser("hub", help="Collaborate, clone, publish, sync, and manage secrets with wshub")
    hub_subparsers = p_hub.add_subparsers(dest="hub_subcommand", title="hub actions", metavar="ACTION")

    # ws hub login
    p_hub_login = hub_subparsers.add_parser("login", help="Authenticate with wshub")
    p_hub_login.add_argument("--url", help="wshub server URL (default: http://127.0.0.1:8787)")
    p_hub_login.add_argument("--token", help="Personal Access Token")
    p_hub_login.add_argument("-u", "--username", help="Username or email")
    p_hub_login.add_argument("-p", "--password", help="Password")

    # ws hub whoami
    hub_subparsers.add_parser("whoami", help="Display active wshub user profile and session")

    # ws hub logout
    hub_subparsers.add_parser("logout", help="Log out and clear saved wshub credentials")

    # ws hub clone <org/project> [target_dir]
    p_hub_clone = hub_subparsers.add_parser("clone", help="Clone and replicate a project from wshub")
    p_hub_clone.add_argument("project", help="Project identifier (e.g. 'org/project' or 'project')")
    p_hub_clone.add_argument("target_dir", nargs="?", help="Target directory path (optional)")

    # ws hub publish [org/project]
    p_hub_pub = hub_subparsers.add_parser("publish", help="Publish local workspace project definition to wshub")
    p_hub_pub.add_argument("project", nargs="?", help="Project identifier (e.g. 'org/project')")
    p_hub_pub.add_argument("-d", "--description", help="Project description")

    # ws hub push
    p_hub_push = hub_subparsers.add_parser("push", help="Push updated project blueprint to wshub (creates new revision)")
    p_hub_push.add_argument("-m", "--message", default="Update configuration", help="Revision changelog message")
    p_hub_push.add_argument("--project", help="Override project identifier")

    # ws hub pull
    p_hub_pull = hub_subparsers.add_parser("pull", help="Pull latest project blueprint and clone new bare repos from wshub")
    p_hub_pull.add_argument("--project", help="Override project identifier")

    # ws hub status
    p_hub_status = hub_subparsers.add_parser("status", help="Check project revision status against wshub")
    p_hub_status.add_argument("--project", help="Override project identifier")

    # ws hub sync
    p_hub_sync = hub_subparsers.add_parser("sync", help="Synchronize project blueprint, bare repos, and secrets from wshub")
    p_hub_sync.add_argument("--project", help="Override project identifier")

    # ws hub state <save|restore>
    p_hub_state = hub_subparsers.add_parser("state", help="Manage cross-machine workspace session state")
    hub_state_sub = p_hub_state.add_subparsers(dest="hub_state_subcommand", metavar="STATE_ACTION")
    p_state_save = hub_state_sub.add_parser("save", help="Save workspace state (branches, locks, local env) to wshub")
    p_state_save.add_argument("workspace", help="Workspace name (@name)")
    p_state_save.add_argument("--project", help="Override project identifier")
    p_state_restore = hub_state_sub.add_parser("restore", help="Restore workspace state from wshub on this machine")
    p_state_restore.add_argument("workspace", help="Workspace name (@name)")
    p_state_restore.add_argument("--project", help="Override project identifier")

    # ws hub resume @<workspace>
    p_hub_resume = hub_subparsers.add_parser("resume", help="Restore workspace state from wshub on this machine")
    p_hub_resume.add_argument("workspace", help="Workspace name (@name)")
    p_hub_resume.add_argument("--project", help="Override project identifier")

    # ws hub secret <list|set|get|delete|upload|pull>
    p_hub_sec = hub_subparsers.add_parser("secret", help="Manage zero-Git encrypted secrets in wshub vault")
    hub_sec_sub = p_hub_sec.add_subparsers(dest="hub_sec_subcommand", metavar="SECRET_ACTION")
    p_sec_list = hub_sec_sub.add_parser("list", help="List project secrets in vault")
    p_sec_list.add_argument("--project", help="Override project identifier")

    p_sec_set = hub_sec_sub.add_parser("set", help="Set an encrypted secret in wshub vault")
    p_sec_set.add_argument("key", help="Secret key name")
    p_sec_set.add_argument("value", help="Secret value")
    p_sec_set.add_argument("--repo", help="Optional repository scope (e.g. 'server')")
    p_sec_set.add_argument("--project", help="Override project identifier")

    p_sec_get = hub_sec_sub.add_parser("get", help="Get a secret value from wshub vault")
    p_sec_get.add_argument("key", help="Secret key name")
    p_sec_get.add_argument("--repo", help="Optional repository scope")
    p_sec_get.add_argument("--project", help="Override project identifier")

    p_sec_del = hub_sec_sub.add_parser("delete", help="Delete a secret from wshub vault")
    p_sec_del.add_argument("key", help="Secret key name")
    p_sec_del.add_argument("--repo", help="Optional repository scope")
    p_sec_del.add_argument("--project", help="Override project identifier")

    p_sec_upload = hub_sec_sub.add_parser("upload", help="Upload and encrypt a sensitive file (.pem, .json) to wshub vault")
    p_sec_upload.add_argument("file_path", help="Local file path to upload")
    p_sec_upload.add_argument("--project", help="Override project identifier")

    p_sec_pull = hub_sec_sub.add_parser("pull", help="Download and decrypt sensitive files into local files/ directory")
    p_sec_pull.add_argument("--project", help="Override project identifier")

    # Top-level shortcut: ws clone <org/project> [target_dir]
    p_top_clone = subparsers.add_parser("clone", help="Clone and replicate a project from wshub")
    p_top_clone.add_argument("project", help="Project identifier (e.g. 'org/project' or 'project')")
    p_top_clone.add_argument("target_dir", nargs="?", help="Target directory path (optional)")

    # Command: ws completion [zsh|bash|fish|install]
    p_comp = subparsers.add_parser("completion", help="Generate or install shell completion scripts")
    p_comp.add_argument("shell", nargs="?", default="zsh", choices=["zsh", "bash", "fish", "install"], help="Shell type: zsh, bash, fish, install (default: zsh)")
    p_comp.add_argument("--install", action="store_true", help="Install completion script into user shell configuration")

    # Hidden Command: ws _complete <type> [args...]
    p_int = subparsers.add_parser("_complete", help=argparse.SUPPRESS)
    p_int.add_argument("query_type", help="Query type")
    p_int.add_argument("query_args", nargs="*", help="Query arguments")

    return parser


def main(sys_args: Sequence[str] | None = None) -> int:
    """Main CLI entrypoint function."""
    if sys_args is None:
        sys_args = sys.argv[1:]

    normalized_args = normalize_cli_args(sys_args)

    parser = build_parser()
    args, unknown = parser.parse_known_args(normalized_args)

    if args.verbose:
        logging.basicConfig(level=logging.DEBUG, format="%(levelname)s [%(name)s]: %(message)s")
    else:
        logging.basicConfig(level=logging.WARNING, format="%(levelname)s: %(message)s")

    if not args.subcommand:
        parser.print_help()
        return 0

    # Fast path for completion queries & generation without loading app configs
    if args.subcommand == "_complete":
        cmd_internal_complete(args.query_type, getattr(args, "query_args", []))
        return 0

    if args.subcommand == "completion":
        cmd_completion(shell=args.shell, install=getattr(args, "install", False))
        return 0

    try:
        allow_empty_config = args.subcommand in ("init", "add", "doctor", "project", "hub", "clone")
        app_config = ConfigLoader.load_config(
            config_path=args.config,
            workspaces_dir=args.workspaces_dir,
            allow_empty=allow_empty_config,
        )
        manager = WorkspaceManager(config=app_config)

        # 1. Project commands
        if args.subcommand in ("init", "add", "fetch", "sync", "project"):

            if args.subcommand == "project":
                sub = getattr(args, "proj_subcommand", None)
                if sub == "init":
                    cmd_init(manager=manager, repo_inputs=args.urls)
                elif sub == "add":
                    cmd_add(manager=manager, repo_input=args.url)
                elif sub == "fetch":
                    cmd_fetch(manager=manager)
                elif sub == "sync":
                    cmd_sync(manager=manager)
                else:
                    OutputHandler.print_error("Please specify a project action: init, add, fetch, sync")
                    return 1
            elif args.subcommand == "init":
                cmd_init(manager=manager, repo_inputs=args.urls)
            elif args.subcommand == "add":
                cmd_add(manager=manager, repo_input=args.url)
            elif args.subcommand == "fetch":
                cmd_fetch(manager=manager)
            elif args.subcommand == "sync":
                cmd_sync(manager=manager)

        # 2. Workspace creation
        elif args.subcommand in ("create", "new"):
            if getattr(args, "file", None):
                cmd_create(manager=manager, config_file=args.file, run_setup=args.setup)
            else:
                if not args.name:
                    raise WSException("Workspace name (@<name>) is required for 'ws create' unless '-f/--file' is used.")
                ws_name = clean_workspace(args.name)
                raw_create_args = list(unknown)
                repo_specs = parse_create_workspace_args(
                    workspace_name=ws_name,
                    raw_args=raw_create_args,
                    repositories=app_config.repositories,
                )
                cmd_new(manager=manager, name=ws_name, repo_specs=repo_specs, run_setup=args.setup)

        # 3. Workspace listing & inspection
        elif args.subcommand in ("list", "ls"):
            cmd_list(manager=manager)

        elif args.subcommand == "info":
            cmd_info(manager=manager, name=clean_workspace(args.name))

        elif args.subcommand in ("delete", "rm", "remove"):
            cmd_delete(manager=manager, name=clean_workspace(args.name))

        elif args.subcommand == "status":
            cmd_status(manager=manager, name=clean_workspace(args.name))

        elif args.subcommand == "exec":
            cmd_exec(manager=manager, name=clean_workspace(args.name), command=args.command)

        # 4. Worktree & Repo management
        elif args.subcommand in ("repo", "workspace"):
            ws_cmd = getattr(args, "repo_subcommand", None)
            if not ws_cmd:
                OutputHandler.print_error("Please specify a repo action: add, remove, lock, unlock")
                return 1

            ws_name = clean_workspace(args.name)
            repo_val = clean_repo(args.repo)
            if ws_cmd in ("add", "add-repo"):
                cmd_repo_add(
                    manager=manager,
                    workspace_name=ws_name,
                    repo_input=args.repo,
                    branch=args.branch,
                    existing=args.existing,
                )
            elif ws_cmd in ("remove", "rm", "remove-repo"):
                cmd_repo_remove(
                    manager=manager,
                    workspace_name=ws_name,
                    repo_name=repo_val,
                    delete_branch=args.delete_branch,
                )
            elif ws_cmd in ("lock", "freeze"):
                cmd_repo_lock(manager=manager, workspace_name=ws_name, repo_name=repo_val)
            elif ws_cmd in ("unlock", "unfreeze"):
                cmd_repo_unlock(manager=manager, workspace_name=ws_name, repo_name=repo_val)

        elif args.subcommand == "lock":
            cmd_repo_lock(manager=manager, workspace_name=clean_workspace(args.name), repo_name=clean_repo(args.repo))

        elif args.subcommand == "unlock":
            cmd_repo_unlock(manager=manager, workspace_name=clean_workspace(args.name), repo_name=clean_repo(args.repo))

        # 5. Service runtime
        elif args.subcommand in ("start", "launch", "run"):
            target_repos = None
            if getattr(args, "repos_flag", None):
                target_repos = [clean_repo(r.strip()) for r in args.repos_flag.split(",") if r.strip()]
            elif getattr(args, "repos", None):
                target_repos = clean_repos(args.repos)

            mode = getattr(args, "mode", None) or "tui"
            if getattr(args, "zellij", False):
                mode = "zellij"
            elif getattr(args, "tmux", False):
                mode = "tmux"
            elif getattr(args, "terminal", False):
                mode = "terminal"
            elif getattr(args, "stream", False):
                mode = "stream"
            elif getattr(args, "attach", None):
                mode = "attach"

            cmd_launch(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repos=target_repos if not getattr(args, "all", False) else None,
                mode=mode,
                attach_repo=clean_repo(getattr(args, "attach", None)),
                daemon=getattr(args, "daemon", False),
                switch=getattr(args, "switch", False),
                interface=getattr(args, "interface", None),
                lan_ip=getattr(args, "lan_ip", None),
            )

        elif args.subcommand == "attach":
            attach_mode = getattr(args, "mode", None)
            if getattr(args, "zellij", False):
                attach_mode = "zellij"
            elif getattr(args, "tmux", False):
                attach_mode = "tmux"

            cmd_attach(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repo_name=clean_repo(getattr(args, "repo", None)),
                all_panes=getattr(args, "all", False),
                mode=attach_mode,
                switch=getattr(args, "switch", False),
            )

        elif args.subcommand in ("stop", "kill"):
            cmd_stop(manager=manager, name=clean_workspace(args.name))

        elif args.subcommand == "restart":
            cmd_restart(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repos=clean_repos(getattr(args, "repos", None)),
            )

        elif args.subcommand == "logs":
            cmd_logs(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repo_name=clean_repo(getattr(args, "repo", None)),
                follow=args.follow,
                lines=args.lines,
            )

        elif args.subcommand == "bridge":
            cmd_bridge(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repo_name=clean_repo(args.repo),
            )

        # 6. Developer shell & environment
        elif args.subcommand in ("shell", "enter", "open"):
            cmd_shell(
                manager=manager,
                name=clean_workspace(args.name),
                worktree=clean_repo(getattr(args, "worktree", None)),
            )

        elif args.subcommand == "env":
            cmd_env(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repo_name=clean_repo(args.repo),
                sync=args.sync,
                interface=getattr(args, "interface", None),
                lan_ip=getattr(args, "lan_ip", None),
            )

        elif args.subcommand == "setup":
            target_repos = None
            if getattr(args, "repos_flag", None):
                target_repos = [clean_repo(r.strip()) for r in args.repos_flag.split(",") if r.strip()]
            elif getattr(args, "repos", None):
                target_repos = clean_repos(args.repos)

            if not args.all and not target_repos:
                raise WSException(
                    f"Explicit repository selection required for setup in workspace '@{clean_workspace(args.name)}'. "
                    "Specify '--all' to setup all repositories, or specify repositories using '%repo1 %repo2' or '--repos r1,r2'."
                )


            cmd_setup(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repos=target_repos if not args.all else None,
                dry_run=args.dry_run,
                skip_scripts=args.skip_scripts,
                verbose=args.verbose,
                interface=getattr(args, "interface", None),
                lan_ip=getattr(args, "lan_ip", None),
            )

        # 7. Git Collaboration
        elif args.subcommand == "push":
            target_repos = None
            if getattr(args, "repos_flag", None):
                target_repos = [clean_repo(r.strip()) for r in args.repos_flag.split(",") if r.strip()]
            elif getattr(args, "repos", None):
                target_repos = clean_repos(args.repos)

            cmd_push(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repos=target_repos,
                remote=args.remote,
            )

        elif args.subcommand == "pull":
            target_repos = None
            if getattr(args, "repos_flag", None):
                target_repos = [clean_repo(r.strip()) for r in args.repos_flag.split(",") if r.strip()]
            elif getattr(args, "repos", None):
                target_repos = clean_repos(args.repos)

            cmd_pull(
                manager=manager,
                workspace_name=clean_workspace(args.name),
                repos=target_repos,
                remote=args.remote,
            )

        elif args.subcommand == "doctor":
            cmd_doctor(manager=manager)

        elif args.subcommand == "antigravity":
            cmd_antigravity(manager=manager)

        # ==================== wshub Commands ====================
        elif args.subcommand == "clone":
            cmd_hub_clone(manager=manager, project=args.project, target_dir=getattr(args, "target_dir", None))

        elif args.subcommand == "hub":
            hub_action = getattr(args, "hub_subcommand", None)
            if not hub_action:
                p_hub.print_help()
                return 0

            if hub_action == "login":
                cmd_hub_login(
                    url=getattr(args, "url", None),
                    token=getattr(args, "token", None),
                    username=getattr(args, "username", None),
                    password=getattr(args, "password", None),
                )
            elif hub_action == "whoami":
                cmd_hub_whoami()
            elif hub_action == "logout":
                cmd_hub_logout()
            elif hub_action == "clone":
                cmd_hub_clone(manager=manager, project=args.project, target_dir=getattr(args, "target_dir", None))
            elif hub_action == "publish":
                cmd_hub_publish(manager=manager, project=getattr(args, "project", None), description=getattr(args, "description", None))
            elif hub_action == "push":
                cmd_hub_push(manager=manager, message=args.message, project=getattr(args, "project", None))
            elif hub_action == "pull":
                cmd_hub_pull(manager=manager, project=getattr(args, "project", None))
            elif hub_action == "status":
                cmd_hub_status(manager=manager, project=getattr(args, "project", None))
            elif hub_action == "sync":
                cmd_hub_sync(manager=manager, project=getattr(args, "project", None))
            elif hub_action in ("state", "resume"):
                state_action = getattr(args, "hub_state_subcommand", None)
                if hub_action == "resume" or state_action == "restore":
                    cmd_hub_state_restore(manager=manager, workspace=args.workspace, project=getattr(args, "project", None))
                elif state_action == "save":
                    cmd_hub_state_save(manager=manager, workspace=args.workspace, project=getattr(args, "project", None))
                else:
                    OutputHandler.print_error("Please specify a state action: save or restore")
                    return 1
            elif hub_action == "secret":
                sec_action = getattr(args, "hub_sec_subcommand", None)
                proj = getattr(args, "project", None)
                if sec_action == "list":
                    cmd_hub_secret_list(manager=manager, project=proj)
                elif sec_action == "set":
                    cmd_hub_secret_set(manager=manager, key=args.key, value=args.value, repo=getattr(args, "repo", None), project=proj)
                elif sec_action == "get":
                    cmd_hub_secret_get(manager=manager, key=args.key, repo=getattr(args, "repo", None), project=proj)
                elif sec_action == "delete":
                    cmd_hub_secret_delete(manager=manager, key=args.key, repo=getattr(args, "repo", None), project=proj)
                elif sec_action == "upload":
                    cmd_hub_secret_upload(manager=manager, file_path=args.file_path, project=proj)
                elif sec_action == "pull":
                    cmd_hub_secret_pull(manager=manager, project=proj)
                else:
                    OutputHandler.print_error("Please specify a secret action: list, set, get, delete, upload, pull")
                    return 1
            else:
                p_hub.print_help()

        else:
            parser.print_help()

        return 0

    except WSException as e:
        OutputHandler.print_error(message=str(e))
        return 1
    except Exception as e:
        logger.exception("Unexpected error occurred: %s", e)
        OutputHandler.print_error(message=f"An unexpected error occurred: {e}")
        return 1


if __name__ == "__main__":
    sys.exit(main())

