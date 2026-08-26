"""Shell completion generator and dynamic query resolver for ws.

Provides autocompletion scripts and dynamic runtime completions for:
- Zsh (with rich descriptions, tag groupings, sigils @ and %, and inverted syntax support)
- Bash (with compgen and dynamic fallback)
- Fish (with complete -c ws)
"""

import json
import logging
from pathlib import Path
import sys
from typing import Sequence

logger = logging.getLogger("ws.completion")


def find_project_root_and_workspaces_dir() -> tuple[Path | None, Path | None]:
    """Lightweight upward traversal to find project root containing repositories.yml and workspaces dir."""
    curr = Path.cwd().resolve()
    for directory in [curr] + list(curr.parents):
        if (directory / "repositories.yml").exists():
            ws_dir = directory / "workspaces"
            return directory, ws_dir
    return None, None


def query_workspaces(include_sigil: bool = True) -> list[tuple[str, str]]:
    """Query available workspaces for completion.

    Returns list of (candidate, description).
    """
    _, ws_dir = find_project_root_and_workspaces_dir()
    if not ws_dir or not ws_dir.exists():
        return []

    candidates: list[tuple[str, str]] = []
    try:
        for item in sorted(ws_dir.iterdir()):
            if item.is_dir() and not item.name.startswith("."):
                name = item.name.lstrip("@")
                meta_file = item / ".ws" / "metadata.json"
                desc = "workspace"
                if meta_file.exists():
                    try:
                        with open(meta_file, "r", encoding="utf-8") as f:
                            data = json.load(f)
                            repo_count = len(data.get("repositories", {}))
                            desc = f"{repo_count} repo{'s' if repo_count != 1 else ''}"
                    except Exception:
                        pass
                
                sigil_name = f"@{name}" if include_sigil else name
                candidates.append((sigil_name, desc))
    except Exception as e:
        logger.debug("Failed querying workspaces for completion: %s", e)

    return candidates


def query_repositories(workspace_name: str | None = None, include_sigil: bool = True) -> list[tuple[str, str]]:
    """Query available repositories for completion.

    If workspace_name is provided, returns repositories present in that workspace.
    Otherwise returns all repositories defined in repositories.yml.
    """
    proj_root, ws_dir = find_project_root_and_workspaces_dir()
    if not proj_root:
        return []

    candidates: list[tuple[str, str]] = []
    
    # 1. If workspace specified, inspect workspace metadata
    if workspace_name and ws_dir:
        clean_ws = workspace_name.lstrip("@")
        meta_file = ws_dir / f"@{clean_ws}" / ".ws" / "metadata.json"
        if not meta_file.exists():
            meta_file = ws_dir / clean_ws / ".ws" / "metadata.json"

        if meta_file.exists():
            try:
                with open(meta_file, "r", encoding="utf-8") as f:
                    data = json.load(f)
                    for r_k, r_v in sorted(data.get("repositories", {}).items()):
                        br = r_v.get("branch", "main")
                        sigil_r = f"%{r_k}" if include_sigil else r_k
                        candidates.append((sigil_r, f"branch: {br}"))
                    return candidates
            except Exception as e:
                logger.debug("Failed querying workspace repositories: %s", e)

    # 2. Fallback to repositories.yml
    config_file = proj_root / "repositories.yml"
    if config_file.exists():
        try:
            import yaml
            with open(config_file, "r", encoding="utf-8") as f:
                data = yaml.safe_load(f) or {}
            repos = data.get("repositories", {})
            for r_k, r_v in sorted(repos.items()):
                cmd = r_v.get("command") or r_v.get("launch") or ""
                desc = cmd[:30] if cmd else "repository"
                sigil_r = f"%{r_k}" if include_sigil else r_k
                candidates.append((sigil_r, desc))
        except Exception as e:
            logger.debug("Failed querying repositories.yml for completion: %s", e)

    return candidates


def query_interfaces(include_desc: bool = True) -> list[tuple[str, str]]:
    """Query available network interfaces and shortcut types for completion."""
    from ws.network import list_network_interfaces
    candidates = [
        ("wifi", "Prioritize wireless Wi-Fi adapter"),
        ("ethernet", "Prioritize wired Ethernet adapter"),
    ]
    try:
        ifaces = list_network_interfaces()
        for item in ifaces:
            name = item["name"]
            ip = item["ip"]
            itype = item["type"]
            desc = f"{itype} interface ({ip})" if include_desc else ""
            candidates.append((name, desc))
    except Exception as e:
        logger.debug("Failed querying network interfaces: %s", e)
    return candidates


def query_completions(query_type: str, *args: str) -> list[str]:
    """Dispatch dynamic completion queries from shell hooks."""
    if query_type == "workspaces":
        res = query_workspaces(include_sigil=True)
        return [f"{c}:{d}" if d else c for c, d in res]
    elif query_type == "workspaces_plain":
        res = query_workspaces(include_sigil=False)
        return [f"{c}:{d}" if d else c for c, d in res]
    elif query_type == "repos":
        ws_name = args[0] if args else None
        res = query_repositories(workspace_name=ws_name, include_sigil=True)
        return [f"{c}:{d}" if d else c for c, d in res]
    elif query_type == "repos_plain":
        ws_name = args[0] if args else None
        res = query_repositories(workspace_name=ws_name, include_sigil=False)
        return [f"{c}:{d}" if d else c for c, d in res]
    elif query_type == "interfaces":
        res = query_interfaces(include_desc=True)
        return [f"{c}:{d}" if d else c for c, d in res]
    elif query_type == "interfaces_plain":
        res = query_interfaces(include_desc=False)
        return [c for c, _ in res]
    return []


# ==============================================================================
# Zsh Completion Script Generator
# ==============================================================================

ZSH_COMPLETION_TEMPLATE = """#compdef ws

# ------------------------------------------------------------------------------
# Zsh completion script for `ws` (Multi-repository Git Workspace Manager)
# Generated automatically by `ws completion zsh`
# ------------------------------------------------------------------------------

_ws_workspaces() {
    local -a workspaces
    local raw_output
    raw_output=$(ws _complete workspaces 2>/dev/null)
    if [[ -n "$raw_output" ]]; then
        while IFS= read -r line; do
            workspaces+=("$line")
        done <<< "$raw_output"
        _describe -t workspaces 'workspace' workspaces -S ''
    fi
}

_ws_workspaces_all() {
    local -a workspaces
    local raw_output
    raw_output=$(ws _complete workspaces 2>/dev/null)
    if [[ -n "$raw_output" ]]; then
        while IFS= read -r line; do
            workspaces+=("$line")
        done <<< "$raw_output"
    fi
    raw_output_plain=$(ws _complete workspaces_plain 2>/dev/null)
    if [[ -n "$raw_output_plain" ]]; then
        while IFS= read -r line; do
            workspaces+=("$line")
        done <<< "$raw_output_plain"
    fi
    if [[ ${#workspaces[@]} -gt 0 ]]; then
        _describe -t workspaces 'workspace' workspaces -S ''
    fi
}

_ws_interfaces() {
    local -a ifaces
    local raw_output
    raw_output=$(ws _complete interfaces 2>/dev/null)
    if [[ -n "$raw_output" ]]; then
        while IFS= read -r line; do
            ifaces+=("$line")
        done <<< "$raw_output"
        _describe -t interfaces 'network interface' ifaces
    fi
}

_ws_repositories() {
    local ws_target=""
    # Scan command line for any @workspace argument
    for word in "${words[@]}"; do
        if [[ "$word" == @* ]]; then
            ws_target="$word"
            break
        fi
    done

    local -a repos
    local raw_output
    raw_output=$(ws _complete repos "$ws_target" 2>/dev/null)
    if [[ -n "$raw_output" ]]; then
        while IFS= read -r line; do
            repos+=("$line")
        done <<< "$raw_output"
        _describe -t repositories 'repository / service' repos -S ''
    fi
}

_ws_commands() {
    local -a commands=(
        'create:Create a new workspace with Git worktrees'
        'new:Create a new workspace (alias for create)'
        'list:List all active workspaces'
        'ls:List all active workspaces (alias for list)'
        'info:Display workspace details, ports, and live processes'
        'delete:Safely delete a workspace and prune worktrees'
        'rm:Delete a workspace (alias for delete)'
        'remove:Delete a workspace (alias for delete)'
        'status:Show combined Git status across all workspace worktrees'
        'exec:Execute an arbitrary command across all worktrees'
        'push:Push committed changes to Git remotes'
        'pull:Pull remote updates across all worktrees'
        'start:Start workspace services concurrently in TUI or multiplexer'
        'launch:Start workspace services (alias for start)'
        'run:Start workspace services (alias for start)'
        'attach:Connect to a running workspace daemon session'
        'stop:Gracefully stop running services and daemon'
        'kill:Stop running services and daemon (alias for stop)'
        'restart:Restart running services inside active workspace'
        'logs:View or tail persistent service logs'
        'bridge:Open raw interactive PTY bridge to a service'
        'shell:Open interactive subshell inside workspace worktree'
        'enter:Open interactive subshell (alias for shell)'
        'open:Open interactive subshell (alias for shell)'
        'env:Inspect or synchronize environment variables'
        'setup:Run dependency setup scripts and sync .env files'
        'repo:Manage repository worktrees in an existing workspace'
        'lock:Lock worktree tracked files as read-only'
        'unlock:Unlock worktree tracked files as writable'
        'project:Manage project bare repository store'
        'init:Initialize project and bare repositories'
        'add:Add a new bare repository'
        'fetch:Fetch updates across all bare repositories'
        'sync:Prune stale worktrees and sync repository refs'
        'doctor:Run system diagnostics and health checks'
        'completion:Generate or install shell completion scripts'
    )
    _describe -t commands 'command' commands
}

_ws() {
    local curcontext="$curcontext" state line
    typeset -A opt_args

    # Check for inverted syntax: ws @workspace <command> ...
    if [[ $CURRENT -ge 3 && "${words[2]}" == @* ]]; then
        local target_ws="${words[2]}"
        local subcmd="${words[3]}"

        if [[ $CURRENT -eq 3 ]]; then
            _ws_commands
            return
        fi

        case "$subcmd" in
            start|launch|run)
                _arguments \\
                    '--all[Start all services in workspace]' \\
                    '--tmux[Launch in Tmux session with vertical panes]' \\
                    '(-z --zellij)'{-z,--zellij}'[Launch in Zellij session]' \\
                    '(-t --terminal)'{-t,--terminal}'[Launch in separate terminal windows]' \\
                    '--stream[Stream raw stdout/stderr without interactive TUI]' \\
                    '(-d --daemon)'{-d,--daemon}'[Launch detached in background daemon]' \\
                    '(-s --switch)'{-s,--switch}'[Zero-downtime switch to presentation engine]' \\
                    '(-m --mode)'{-m,--mode}'[Multiplexer mode]:mode:(tui tmux zellij terminal stream daemon)' \\
                    '--interface[Network interface name or type]:interface:_ws_interfaces' \\
                    '--iface[Network interface name or type]:interface:_ws_interfaces' \\
                    '--ip[Explicit host LAN IP address override]:ip:' \\
                    '--lan-ip[Explicit host LAN IP address override]:ip:' \\
                    '--attach[Focus single service]:service:_ws_repositories' \\
                    '*:service:_ws_repositories'
                ;;
            attach)
                _arguments \\
                    '--all[Attach in multi-pane grid view]' \\
                    '--tmux[Attach using Tmux backend]' \\
                    '(-z --zellij)'{-z,--zellij}'[Attach using Zellij backend]' \\
                    '(-s --switch)'{-s,--switch}'[Zero-downtime switch presentation engine]' \\
                    '(-m --mode)'{-m,--mode}'[Engine backend]:mode:(tui tmux zellij)' \\
                    '1:service:_ws_repositories'
                ;;
            restart|logs|bridge|shell|enter|open|lock|unlock)
                _arguments '*:service:_ws_repositories'
                ;;
            env)
                _arguments \\
                    '--sync[Sync environment variables into .env files]' \\
                    '--interface[Network interface name or type]:interface:_ws_interfaces' \\
                    '--iface[Network interface name or type]:interface:_ws_interfaces' \\
                    '--ip[Explicit host LAN IP address override]:ip:' \\
                    '--lan-ip[Explicit host LAN IP address override]:ip:' \\
                    '*:service:_ws_repositories'
                ;;
            setup)
                _arguments \\
                    '--all[Setup all repositories in workspace]' \\
                    '--dry-run[Print setup commands without running them]' \\
                    '--skip-scripts[Only sync environment variables without running scripts]' \\
                    '--interface[Network interface name or type]:interface:_ws_interfaces' \\
                    '--iface[Network interface name or type]:interface:_ws_interfaces' \\
                    '--ip[Explicit host LAN IP address override]:ip:' \\
                    '--lan-ip[Explicit host LAN IP address override]:ip:' \\
                    '*:service:_ws_repositories'
                ;;
            push|pull)
                _arguments \\
                    '--remote[Git remote name]:remote:(origin upstream)' \\
                    '*:service:_ws_repositories'
                ;;
            *)
                _arguments '*:arguments:_files'
                ;;
        esac
        return
    fi

    _arguments -C \\
        '(-v --verbose)'{-v,--verbose}'[Enable debug logging]' \\
        '(-c --config)'{-c,--config}'[Path to repositories configuration file]:config file:_files' \\
        '(-w --workspaces-dir)'{-w,--workspaces-dir}'[Directory for storing workspaces]:directory:_files -/' \\
        '--version[Show version information]' \\
        '1: :->command_or_workspace' \\
        '*:: :->args'

    case $state in
        command_or_workspace)
            _ws_commands
            _ws_workspaces
            ;;
        args)
            local cmd="${words[2]}"
            case "$cmd" in
                create|new)
                    _arguments \\
                        '1:workspace name:_ws_workspaces_all' \\
                        '(-f --file)'{-f,--file}'[Path to workspace YAML file]:YAML file:_files -g "*.yml *.yaml"' \\
                        '--setup[Run setup scripts after creation]' \\
                        '--all[Include all repositories]' \\
                        '--existing[Checkout existing branches]' \\
                        '*:repository specification:_ws_repositories'
                    ;;
                start|launch|run)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '--all[Start all services in workspace]' \\
                        '--tmux[Launch in Tmux session with vertical panes]' \\
                        '(-z --zellij)'{-z,--zellij}'[Launch in Zellij session]' \\
                        '(-t --terminal)'{-t,--terminal}'[Launch in separate terminal windows]' \\
                        '--stream[Stream raw stdout/stderr without interactive TUI]' \\
                        '(-d --daemon)'{-d,--daemon}'[Launch detached in background daemon]' \\
                        '(-s --switch)'{-s,--switch}'[Zero-downtime switch to presentation engine]' \\
                        '(-m --mode)'{-m,--mode}'[Multiplexer mode]:mode:(tui tmux zellij terminal stream daemon)' \\
                        '--interface[Network interface name or type]:interface:_ws_interfaces' \\
                        '--iface[Network interface name or type]:interface:_ws_interfaces' \\
                        '--ip[Explicit host LAN IP address override]:ip:' \\
                        '--lan-ip[Explicit host LAN IP address override]:ip:' \\
                        '--attach[Focus single service]:service:_ws_repositories' \\
                        '*:services:_ws_repositories'
                    ;;
                attach)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '2:service:_ws_repositories' \\
                        '--all[Attach in multi-pane grid view]' \\
                        '--tmux[Attach using Tmux backend]' \\
                        '(-z --zellij)'{-z,--zellij}'[Attach using Zellij backend]' \\
                        '(-s --switch)'{-s,--switch}'[Zero-downtime switch presentation engine]' \\
                        '(-m --mode)'{-m,--mode}'[Engine backend]:mode:(tui tmux zellij)'
                    ;;
                info|status|stop|kill|delete|rm|remove)
                    _arguments '1:workspace:_ws_workspaces_all'
                    ;;
                restart|logs)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '(-f --follow)'{-f,--follow}'[Follow live log output]' \\
                        '(-n --lines)'{-n,--lines}'[Number of lines]:lines:' \\
                        '*:service:_ws_repositories'
                    ;;
                bridge|shell|enter|open|lock|unlock)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '2:service:_ws_repositories'
                    ;;
                env)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '2:service:_ws_repositories' \\
                        '--sync[Sync environment variables into .env files]' \\
                        '--interface[Network interface name or type]:interface:_ws_interfaces' \\
                        '--iface[Network interface name or type]:interface:_ws_interfaces' \\
                        '--ip[Explicit host LAN IP address override]:ip:' \\
                        '--lan-ip[Explicit host LAN IP address override]:ip:'
                    ;;
                setup)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '--all[Setup all repositories in workspace]' \\
                        '--dry-run[Print setup commands without running them]' \\
                        '--skip-scripts[Only sync environment variables without running scripts]' \\
                        '--interface[Network interface name or type]:interface:_ws_interfaces' \\
                        '--iface[Network interface name or type]:interface:_ws_interfaces' \\
                        '--ip[Explicit host LAN IP address override]:ip:' \\
                        '--lan-ip[Explicit host LAN IP address override]:ip:' \\
                        '*:service:_ws_repositories'
                    ;;
                push|pull)
                    _arguments \\
                        '1:workspace:_ws_workspaces_all' \\
                        '--remote[Git remote name]:remote:(origin upstream)' \\
                        '*:service:_ws_repositories'
                    ;;
                repo|workspace)
                    _arguments \\
                        '1:action:(add remove lock unlock)' \\
                        '2:workspace:_ws_workspaces_all' \\
                        '3:repository:_ws_repositories' \\
                        '--existing[Checkout existing branch]' \\
                        '--delete-branch[Also delete branch from bare store]'
                    ;;
                project)
                    _arguments '1:action:(init add fetch sync)' '*:args:_files'
                    ;;
                completion)
                    _arguments '1:shell:(zsh bash fish install)'
                    ;;
                *)
                    _files
                    ;;
            esac
            ;;
    esac
}

_ws "$@"
"""


# ==============================================================================
# Bash Completion Script Generator
# ==============================================================================

BASH_COMPLETION_TEMPLATE = """# ------------------------------------------------------------------------------
# Bash completion script for `ws`
# Generated automatically by `ws completion bash`
# ------------------------------------------------------------------------------

_ws_completion() {
    local cur prev words cword
    _init_completion || return

    local commands="create new list ls info delete rm remove status exec push pull start launch run attach stop kill restart logs bridge shell enter open env setup repo lock unlock project init add fetch sync doctor completion"

    # Top-level command completion
    if [[ $cword -eq 1 ]]; then
        local workspaces=$(ws _complete workspaces 2>/dev/null | cut -d: -f1)
        COMPREPLY=( $(compgen -W "${commands} ${workspaces}" -- "$cur") )
        return 0
    fi

    local subcmd="${words[1]}"

    # Inverted syntax: ws @workspace <command> ...
    if [[ "$subcmd" == @* ]]; then
        if [[ $cword -eq 2 ]]; then
            COMPREPLY=( $(compgen -W "${commands}" -- "$cur") )
            return 0
        fi
        subcmd="${words[2]}"
    fi

    case "$subcmd" in
        create|new)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--file -f --setup --all --existing" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        start|launch|run)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --tmux --zellij -z --terminal -t --stream --daemon -d --switch -s --mode -m --interface --iface --ip --lan-ip --attach" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos "${words[2]}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        env)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--sync --interface --iface --ip --lan-ip" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos "${words[2]}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        setup)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --dry-run --skip-scripts --interface --iface --ip --lan-ip" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos "${words[2]}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        attach|stop|kill|restart|logs|bridge|shell|enter|open|lock|unlock|push|pull|status|info|delete|rm|remove)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --tmux -z --zellij --switch -s --follow -f --lines -n --remote" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos "${words[2]}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        completion)
            COMPREPLY=( $(compgen -W "zsh bash fish install" -- "$cur") )
            ;;
        *)
            ;;
    esac
}

complete -F _ws_completion ws
"""


# ==============================================================================
# Fish Completion Script Generator
# ==============================================================================

FISH_COMPLETION_TEMPLATE = """# ------------------------------------------------------------------------------
# Fish completion script for `ws`
# Generated automatically by `ws completion fish`
# ------------------------------------------------------------------------------

function __fish_ws_workspaces
    ws _complete workspaces 2>/dev/null | string replace -r ':(.*)' '\t$1'
end

function __fish_ws_repos
    ws _complete repos 2>/dev/null | string replace -r ':(.*)' '\t$1'
end

function __fish_ws_interfaces
    ws _complete interfaces 2>/dev/null | string replace -r ':(.*)' '\t$1'
end

complete -c ws -f
complete -c ws -n "__fish_use_subcommand" -a "create" -d "Create workspace with Git worktrees"
complete -c ws -n "__fish_use_subcommand" -a "list" -d "List all workspaces"
complete -c ws -n "__fish_use_subcommand" -a "info" -d "Display workspace details & processes"
complete -c ws -n "__fish_use_subcommand" -a "delete" -d "Delete workspace and prune worktrees"
complete -c ws -n "__fish_use_subcommand" -a "status" -d "Show combined Git status"
complete -c ws -n "__fish_use_subcommand" -a "start" -d "Start services in TUI or multiplexer"
complete -c ws -n "__fish_use_subcommand" -a "attach" -d "Attach to running daemon session"
complete -c ws -n "__fish_use_subcommand" -a "stop" -d "Stop running workspace session"
complete -c ws -n "__fish_use_subcommand" -a "restart" -d "Restart services in active workspace"
complete -c ws -n "__fish_use_subcommand" -a "logs" -d "View or tail service logs"
complete -c ws -n "__fish_use_subcommand" -a "bridge" -d "Raw terminal PTY bridge"
complete -c ws -n "__fish_use_subcommand" -a "shell" -d "Open interactive subshell"
complete -c ws -n "__fish_use_subcommand" -a "env" -d "Inspect or sync environment variables"
complete -c ws -n "__fish_use_subcommand" -a "setup" -d "Run setup scripts and sync .env"
complete -c ws -n "__fish_use_subcommand" -a "lock" -d "Lock worktree tracked files read-only"
complete -c ws -n "__fish_use_subcommand" -a "unlock" -d "Unlock worktree tracked files writable"
complete -c ws -n "__fish_use_subcommand" -a "push" -d "Push committed changes to remotes"
complete -c ws -n "__fish_use_subcommand" -a "pull" -d "Pull remote updates"
complete -c ws -n "__fish_use_subcommand" -a "doctor" -d "Run health check diagnostics"
complete -c ws -n "__fish_use_subcommand" -a "completion" -d "Generate completion scripts"

# Dynamic workspace and repo arguments
complete -c ws -n "__fish_seen_subcommand_from start attach info delete status restart logs bridge shell env setup lock unlock push pull" -a "(__fish_ws_workspaces)"
complete -c ws -n "__fish_seen_subcommand_from start attach restart logs bridge shell env setup lock unlock push pull" -a "(__fish_ws_repos)"

# Flags
complete -c ws -n "__fish_seen_subcommand_from start" -l tmux -d "Launch in Tmux vertical panes"
complete -c ws -n "__fish_seen_subcommand_from start" -s z -l zellij -d "Launch in Zellij session"
complete -c ws -n "__fish_seen_subcommand_from start" -s d -l daemon -d "Launch in background daemon"
complete -c ws -n "__fish_seen_subcommand_from start" -s s -l switch -d "Zero-downtime presentation switch"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l interface -a "(__fish_ws_interfaces)" -d "Network interface name or type"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l iface -a "(__fish_ws_interfaces)" -d "Network interface name or type"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l ip -d "Explicit LAN IP address override"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l lan-ip -d "Explicit LAN IP address override"
complete -c ws -n "__fish_seen_subcommand_from attach" -s s -l switch -d "Zero-downtime presentation switch"
"""


def generate_completion_script(shell: str) -> str:
    """Generate shell completion script for specified shell."""
    shell_lower = shell.lower()
    if shell_lower in ("zsh", "z"):
        return ZSH_COMPLETION_TEMPLATE
    elif shell_lower in ("bash", "sh"):
        return BASH_COMPLETION_TEMPLATE
    elif shell_lower in ("fish",):
        return FISH_COMPLETION_TEMPLATE
    else:
        raise ValueError(f"Unsupported shell: '{shell}'. Supported shells: zsh, bash, fish")


def install_completion(shell: str | None = None) -> tuple[bool, str]:
    """Provide automated setup or instructions for shell completions."""
    detected_shell = shell or (Path(sys.executable).stem if "zsh" in sys.executable else "zsh")
    import os
    user_shell = os.environ.get("SHELL", "")
    if "zsh" in user_shell:
        target_shell = "zsh"
    elif "bash" in user_shell:
        target_shell = "bash"
    elif "fish" in user_shell:
        target_shell = "fish"
    else:
        target_shell = detected_shell or "zsh"

    home = Path.home()
    if target_shell == "zsh":
        zsh_dir = home / ".zsh" / "completions"
        zsh_dir.mkdir(parents=True, exist_ok=True)
        comp_file = zsh_dir / "_ws"
        comp_file.write_text(ZSH_COMPLETION_TEMPLATE, encoding="utf-8")
        
        return True, (
            f"✔ Installed Zsh completions to {comp_file}.\n\n"
            "To activate immediately in your current terminal session, run:\n"
            "  source <(ws completion zsh)\n\n"
            "To ensure completions are permanently loaded, add this to your ~/.zshrc:\n"
            "  fpath=(~/.zsh/completions $fpath)\n"
            "  autoload -Uz compinit && compinit\n"
        )
    elif target_shell == "bash":
        bash_comp_dir = home / ".local" / "share" / "bash-completion" / "completions"
        bash_comp_dir.mkdir(parents=True, exist_ok=True)
        comp_file = bash_comp_dir / "ws"
        comp_file.write_text(BASH_COMPLETION_TEMPLATE, encoding="utf-8")
        return True, (
            f"✔ Installed Bash completions to {comp_file}.\n\n"
            "To activate in your current session, run:\n"
            "  eval \"$(ws completion bash)\"\n"
        )
    elif target_shell == "fish":
        fish_dir = home / ".config" / "fish" / "completions"
        fish_dir.mkdir(parents=True, exist_ok=True)
        comp_file = fish_dir / "ws.fish"
        comp_file.write_text(FISH_COMPLETION_TEMPLATE, encoding="utf-8")
        return True, f"✔ Installed Fish completions to {comp_file}."

    return False, f"Unknown shell '{target_shell}'"
