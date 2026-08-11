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
    cmd_create,

    cmd_doctor,
    cmd_env,
    cmd_exec,
    cmd_fetch,
    cmd_info,
    cmd_init,
    cmd_launch,
    cmd_list,
    cmd_new,
    cmd_open,
    cmd_pull,
    cmd_push,
    cmd_remove,
    cmd_setup,
    cmd_status,
    cmd_stop,
    cmd_sync,
    cmd_workspace_add_repo,
    cmd_workspace_freeze,
    cmd_workspace_remove_repo,
    cmd_workspace_unfreeze,
)
from ws.config import ConfigLoader
from ws.exceptions import WSException
from ws.models import RepoConfig, RepoSpec
from ws.output import OutputHandler
from ws.workspace import WorkspaceManager

logger = logging.getLogger("ws.cli")

KNOWN_COMMANDS = {
    "init", "add", "new", "create", "setup", "env", "launch", "start", "run",
    "attach", "list", "info", "remove", "rm", "open", "stop", "kill", "workspace", "push", "pull",
    "status", "exec", "fetch", "sync", "doctor", "antigravity",
}



def normalize_cli_args(sys_args: Sequence[str]) -> list[str]:
    """Normalize CLI arguments to support both 'ws <command> <workspace> ...' and 'ws <workspace> <command> ...'."""
    args_list = list(sys_args)
    if not args_list:
        return args_list

    first = args_list[0]
    # If first argument is not a known command and not a flag (e.g. 'develop'),
    # but the second argument IS a known command (e.g. 'open', 'launch', 'status', etc.)
    if first not in KNOWN_COMMANDS and not first.startswith("-") and len(args_list) >= 2:
        second = args_list[1]
        if second in KNOWN_COMMANDS:
            # Swap: ['develop', 'open', '--all'] -> ['open', 'develop', '--all']
            normalized = [second, first] + args_list[2:]
            return normalized

    return args_list


def parse_new_workspace_args(
    workspace_name: str,
    raw_args: list[str],
    repositories: dict[str, RepoConfig],
) -> list[RepoSpec]:

    """Parse dynamic parameters for 'ws new <name>'."""
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
            selected_repos = set(r.strip() for r in repos_str.split(",") if r.strip())
            idx += 2
            continue
        elif arg.startswith("--repos=") or arg.startswith("--only="):
            repos_str = arg.split("=", 1)[1]
            selected_repos = set(r.strip() for r in repos_str.split(",") if r.strip())
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

        # Check key=value positional format (e.g. server=main, web=feature/auth, server=main:existing)
        if "=" in arg and not arg.startswith("-"):
            repo_key, branch_val = arg.split("=", 1)
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

        # Check positional repo name without branch (e.g. server, web)
        if arg in repositories and not arg.startswith("-"):
            repo_key = arg
            create_mode = not global_existing
            if idx + 1 < len(raw_args):
                if raw_args[idx + 1] == "--existing":
                    create_mode = False
                    idx += 1
                elif raw_args[idx + 1] == "--new":
                    create_mode = True
                    idx += 1

            branch_name = workspace_name if not create_mode else f"feature/{workspace_name}"
            explicit_specs[repo_key] = RepoSpec(
                name=repo_key,
                branch=branch_name,
                create=create_mode,
                path=repositories[repo_key].checkout,
            )
            idx += 1
            continue

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
            "Specify repositories as 'repo=branch [--existing]', use '--all', or '--repos repo1,repo2'."
        )

    if include_all:
        target_names = set(repositories.keys())
    elif selected_repos is not None:
        target_names = selected_repos
        for r in target_names:
            if r not in repositories:
                raise WSException(
                    f"Repository '{r}' specified in --repos/--only is not in project configuration. "
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
                branch = workspace_name
                create = False
            else:
                branch = f"feature/{workspace_name}"
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


def build_parser() -> argparse.ArgumentParser:
    """Build the top-level CLI argument parser."""
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

    # Command: ws init [URL ...]
    p_init = subparsers.add_parser("init", help="Initialize project and clone bare repositories from Git URLs")
    p_init.add_argument("urls", nargs="*", help="Git repository URLs (e.g. git@github.com:user/repo.git or name=URL)")

    # Command: ws add <URL>
    p_add = subparsers.add_parser("add", help="Add and clone a new bare repository from a Git URL")
    p_add.add_argument("url", help="Git repository URL or name=URL")

    # Command: ws new <name> ...
    p_new = subparsers.add_parser("new", help="Create a new workspace from CLI parameters")
    p_new.add_argument("name", help="Workspace name")
    p_new.add_argument("--setup", action="store_true", help="Run setup scripts and sync environment variables after creation")

    # Command: ws create <config.yml>
    p_create = subparsers.add_parser("create", help="Create a workspace from a YAML configuration file")
    p_create.add_argument("file", help="Path to workspace YAML configuration file")
    p_create.add_argument("--setup", action="store_true", help="Run setup scripts and sync environment variables after creation")

    # Command: ws setup <name> [repos...] [--all] [--repos r1,r2]
    p_setup = subparsers.add_parser("setup", help="Run setup scripts and environment variable sync for a workspace")
    p_setup.add_argument("name", help="Workspace name")
    p_setup.add_argument("repos", nargs="*", help="Repository names to setup (or use --all)")
    p_setup.add_argument("--all", action="store_true", help="Setup all repositories in the workspace")
    p_setup.add_argument("--repos", "--only", dest="repos_flag", type=str, help="Comma-separated list of repository names to setup")
    p_setup.add_argument("--dry-run", action="store_true", help="Print setup commands without executing them")
    p_setup.add_argument("--skip-scripts", action="store_true", help="Only sync environment variables without running setup scripts")

    # Command: ws env <name> [repo]
    p_env = subparsers.add_parser("env", help="Inspect or sync environment variables for a workspace")
    p_env.add_argument("name", help="Workspace name")
    p_env.add_argument("repo", nargs="?", default=None, help="Repository name (optional)")
    p_env.add_argument("--sync", action="store_true", help="Sync resolved environment variables into worktree .env files")

    # Command: ws launch <name> [repos...] [--all] [--repos r1,r2] [--zellij] [--tmux] [--terminal] [--attach repo] [--daemon] [--mode]
    p_launch = subparsers.add_parser("launch", aliases=["start", "run"], help="Launch workspace services concurrently")
    p_launch.add_argument("name", help="Workspace name")
    p_launch.add_argument("repos", nargs="*", help="Repository names to launch (or use --all)")
    p_launch.add_argument("--all", action="store_true", help="Launch all repositories in the workspace")
    p_launch.add_argument("--repos", "--only", dest="repos_flag", type=str, help="Comma-separated list of repository names to launch")
    p_launch.add_argument("--attach", type=str, default=None, help="Focus or connect directly to a single service output")
    p_launch.add_argument("--zellij", "-z", action="store_true", help="Launch services in tiled split panes inside a Zellij session")
    p_launch.add_argument("--tmux", action="store_true", help="Launch services in tiled split panes inside a tmux session")
    p_launch.add_argument("--terminal", "-t", action="store_true", help="Launch services in separate terminal windows/tabs")
    p_launch.add_argument("--stream", action="store_true", help="Stream raw multiplexed stdout/stderr without interactive TUI")
    p_launch.add_argument("--daemon", "-d", "--background", dest="daemon", action="store_true", help="Launch services in a detached background daemon")
    p_launch.add_argument("--switch", "-s", action="store_true", help="Switch multiplexer engine for a running workspace")
    p_launch.add_argument("--mode", "-m", choices=["tui", "zellij", "tmux", "terminal", "stream", "attach", "daemon", "summary", "list"], default=None, help="Multiplexer/UI mode")


    # Command: ws attach <name> [repo] [--all]
    p_attach = subparsers.add_parser("attach", help="Attach to a running workspace session")
    p_attach.add_argument("name", help="Workspace name")
    p_attach.add_argument("repo", nargs="?", default=None, help="Repository service name to focus (optional)")
    p_attach.add_argument("--all", action="store_true", help="Attach to all workspace services")
    p_attach.add_argument("--switch", "-s", action="store_true", help="Switch multiplexer engine for a running workspace")
    p_attach.add_argument("--tmux", action="store_true", help="Attach using tmux backend")
    p_attach.add_argument("--zellij", "-z", action="store_true", help="Attach using Zellij backend")
    p_attach.add_argument("--mode", "-m", choices=["tui", "zellij", "tmux"], default=None, help="Multiplexer engine backend")


    # Command: ws list
    p_list = subparsers.add_parser("list", help="List all workspaces")

    # Command: ws info <name>
    p_info = subparsers.add_parser("info", help="Display details for a workspace")
    p_info.add_argument("name", help="Workspace name")

    # Command: ws remove <name>
    p_remove = subparsers.add_parser("remove", aliases=["rm"], help="Remove a workspace and all its worktrees")
    p_remove.add_argument("name", help="Workspace name")

    # Command: ws open <name> [worktree]
    p_open = subparsers.add_parser("open", help="Open an interactive subshell inside a workspace or repository directory")
    p_open.add_argument("name", help="Workspace name")
    p_open.add_argument("worktree", nargs="?", default=None, help="Repository worktree name to open subshell into")



    # Command: ws workspace ...
    p_workspace = subparsers.add_parser("workspace", help="Manage repositories and state inside an existing workspace")
    ws_subparsers = p_workspace.add_subparsers(dest="ws_subcommand", title="workspace actions", metavar="ACTION")

    p_ws_add = ws_subparsers.add_parser("add-repo", help="Add a repository worktree to an existing workspace")
    p_ws_add.add_argument("name", help="Workspace name")
    p_ws_add.add_argument("repo", help="Repository name (must be in project configuration)")
    p_ws_add.add_argument("branch", help="Git branch name")
    p_ws_add.add_argument("--existing", action="store_true", help="Checkout existing branch instead of creating new branch")

    p_ws_rm = ws_subparsers.add_parser("remove-repo", help="Remove a repository worktree from a workspace")
    p_ws_rm.add_argument("name", help="Workspace name")
    p_ws_rm.add_argument("repo", help="Repository name")
    p_ws_rm.add_argument("--delete-branch", action="store_true", help="Also delete the branch from bare repository")

    p_ws_freeze = ws_subparsers.add_parser("freeze", help="Freeze repository worktree (mark git-tracked files read-only)")
    p_ws_freeze.add_argument("name", help="Workspace name")
    p_ws_freeze.add_argument("repo", help="Repository name")

    p_ws_unfreeze = ws_subparsers.add_parser("unfreeze", help="Unfreeze repository worktree (restore write permissions)")
    p_ws_unfreeze.add_argument("name", help="Workspace name")
    p_ws_unfreeze.add_argument("repo", help="Repository name")

    # Command: ws push <name> [--repos r1,r2] [--remote origin]
    p_push = subparsers.add_parser("push", help="Push committed changes for workspace repositories to remotes")
    p_push.add_argument("name", help="Workspace name")
    p_push.add_argument("--repos", type=str, help="Comma-separated list of repository names to push")
    p_push.add_argument("--remote", type=str, default="origin", help="Git remote name (default: origin)")

    # Command: ws pull <name> [--repos r1,r2] [--remote origin]
    p_pull = subparsers.add_parser("pull", help="Pull remote updates for workspace repositories")
    p_pull.add_argument("name", help="Workspace name")
    p_pull.add_argument("--repos", type=str, help="Comma-separated list of repository names to pull")
    p_pull.add_argument("--remote", type=str, default="origin", help="Git remote name (default: origin)")



    # Command: ws stop <name>
    p_stop = subparsers.add_parser("stop", aliases=["kill"], help="Stop running workspace background session")
    p_stop.add_argument("name", help="Workspace name")

    # Extension commands
    p_status = subparsers.add_parser("status", help="Show Git status across all workspace worktrees")
    p_status.add_argument("name", help="Workspace name")

    p_exec = subparsers.add_parser("exec", help="Execute command inside each repo worktree of a workspace")
    p_exec.add_argument("name", help="Workspace name")
    p_exec.add_argument("command", nargs=argparse.REMAINDER, help="Command to execute")

    subparsers.add_parser("fetch", help="Fetch updates in all bare repositories")
    subparsers.add_parser("sync", help="Sync and prune worktrees")
    subparsers.add_parser("doctor", help="Run system health checks and diagnostics")
    subparsers.add_parser("antigravity", help="Antigravity AI agent workspace health check")


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

    try:
        allow_empty_config = args.subcommand in ("init", "add", "doctor")
        app_config = ConfigLoader.load_config(
            config_path=args.config,
            workspaces_dir=args.workspaces_dir,
            allow_empty=allow_empty_config,
        )
        manager = WorkspaceManager(config=app_config)

        if args.subcommand == "init":
            cmd_init(manager=manager, repo_inputs=args.urls)

        elif args.subcommand == "add":
            cmd_add(manager=manager, repo_input=args.url)

        elif args.subcommand == "new":
            raw_new_args = list(unknown)
            repo_specs = parse_new_workspace_args(
                workspace_name=args.name,
                raw_args=raw_new_args,
                repositories=app_config.repositories,
            )
            cmd_new(manager=manager, name=args.name, repo_specs=repo_specs, run_setup=args.setup)

        elif args.subcommand == "create":
            cmd_create(manager=manager, config_file=args.file, run_setup=args.setup)

        elif args.subcommand == "setup":
            target_repos: list[str] | None = None
            if args.repos_flag:
                target_repos = [r.strip() for r in args.repos_flag.split(",") if r.strip()]
            elif args.repos:
                target_repos = list(args.repos)

            if not args.all and not target_repos:
                raise WSException(
                    f"Explicit repository selection required for setup in workspace '{args.name}'. "
                    "Specify '--all' to setup all repositories, or specify repositories using '--repos repo1,repo2' or positional arguments."
                )

            cmd_setup(
                manager=manager,
                workspace_name=args.name,
                repos=target_repos if not args.all else None,
                dry_run=args.dry_run,
                skip_scripts=args.skip_scripts,
                verbose=args.verbose,
            )

        elif args.subcommand == "env":
            cmd_env(
                manager=manager,
                workspace_name=args.name,
                repo_name=args.repo,
                sync=args.sync,
            )

        elif args.subcommand in ("launch", "start", "run"):
            target_repos = None
            if getattr(args, "repos_flag", None):
                target_repos = [r.strip() for r in args.repos_flag.split(",") if r.strip()]
            elif getattr(args, "repos", None):
                target_repos = list(args.repos)

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
                workspace_name=args.name,
                repos=target_repos if not getattr(args, "all", False) else None,
                mode=mode,
                attach_repo=getattr(args, "attach", None),
                daemon=getattr(args, "daemon", False),
                switch=getattr(args, "switch", False),
            )

        elif args.subcommand == "attach":
            attach_mode = getattr(args, "mode", None)
            if getattr(args, "zellij", False):
                attach_mode = "zellij"
            elif getattr(args, "tmux", False):
                attach_mode = "tmux"

            cmd_attach(
                manager=manager,
                workspace_name=args.name,
                repo_name=getattr(args, "repo", None),
                all_panes=getattr(args, "all", False),
                mode=attach_mode,
                switch=getattr(args, "switch", False),
            )



        elif args.subcommand == "list":
            cmd_list(manager=manager)

        elif args.subcommand == "info":
            cmd_info(manager=manager, name=args.name)

        elif args.subcommand in ("remove", "rm"):
            cmd_remove(manager=manager, name=args.name)

        elif args.subcommand == "open":
            cmd_open(
                manager=manager,
                name=args.name,
                worktree=getattr(args, "worktree", None),
            )


        elif args.subcommand in ("stop", "kill"):
            cmd_stop(manager=manager, name=args.name)



        elif args.subcommand == "workspace":
            if not getattr(args, "ws_subcommand", None):
                OutputHandler.print_error("Please specify a workspace action: add-repo, remove-repo, freeze, unfreeze")
                return 1

            ws_cmd = args.ws_subcommand
            if ws_cmd == "add-repo":
                cmd_workspace_add_repo(
                    manager=manager,
                    workspace_name=args.name,
                    repo_name=args.repo,
                    branch=args.branch,
                    create=not args.existing,
                )
            elif ws_cmd == "remove-repo":
                cmd_workspace_remove_repo(
                    manager=manager,
                    workspace_name=args.name,
                    repo_name=args.repo,
                    delete_branch=args.delete_branch,
                )
            elif ws_cmd == "freeze":
                cmd_workspace_freeze(
                    manager=manager,
                    workspace_name=args.name,
                    repo_name=args.repo,
                )
            elif ws_cmd == "unfreeze":
                cmd_workspace_unfreeze(
                    manager=manager,
                    workspace_name=args.name,
                    repo_name=args.repo,
                )

        elif args.subcommand == "push":
            repo_list = [r.strip() for r in args.repos.split(",")] if args.repos else None
            cmd_push(
                manager=manager,
                workspace_name=args.name,
                repos=repo_list,
                remote=args.remote,
            )

        elif args.subcommand == "pull":
            repo_list = [r.strip() for r in args.repos.split(",")] if args.repos else None
            cmd_pull(
                manager=manager,
                workspace_name=args.name,
                repos=repo_list,
                remote=args.remote,
            )



        elif args.subcommand == "status":
            cmd_status(manager=manager, name=args.name)

        elif args.subcommand == "exec":
            cmd_exec(manager=manager, name=args.name, command=args.command)

        elif args.subcommand == "fetch":
            cmd_fetch(manager=manager)

        elif args.subcommand == "sync":
            cmd_sync(manager=manager)

        elif args.subcommand == "doctor":
            cmd_doctor(manager=manager)

        elif args.subcommand == "antigravity":
            cmd_antigravity(manager=manager)

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
