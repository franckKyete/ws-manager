use std::fs;
use std::path::{Path, PathBuf};

use crate::config::ConfigLoader;
use crate::network::list_network_interfaces;

pub const ZSH_COMPLETION_TEMPLATE: &str = r#"#compdef ws

_ws() {
    local curcontext="$curcontext" state line
    typeset -A opt_args

    local -a commands
    commands=(
        'create:Create workspace with Git worktrees'
        'new:Create workspace with Git worktrees'
        'list:List all workspaces'
        'ls:List all workspaces'
        'info:Display workspace details & processes'
        'focus:Focus or switch to workspace tmux window'
        'switch:Switch to workspace tmux window'
        'end:Safely end and close workspace'
        'close:Safely close workspace'
        'delete:Delete workspace and prune worktrees'
        'rm:Delete workspace'
        'remove:Delete workspace'
        'status:Show combined Git status'
        'exec:Execute command across workspace worktrees'
        'start:Start services in TUI or multiplexer'
        'launch:Start services in TUI or multiplexer'
        'run:Start services in TUI or multiplexer'
        'attach:Attach to running daemon session'
        'stop:Stop running workspace session'
        'kill:Stop running workspace session'
        'restart:Restart services in active workspace'
        'logs:View or tail service logs'
        'bridge:Raw terminal PTY bridge'
        'shell:Open interactive subshell'
        'enter:Open interactive subshell'
        'open:Open interactive subshell'
        'env:Inspect or sync environment variables'
        'setup:Run setup scripts and sync .env'
        'repo:Manage repositories inside workspace'
        'lock:Lock worktree tracked files read-only'
        'unlock:Unlock worktree tracked files writable'
        'push:Push committed changes to remotes'
        'pull:Pull remote updates'
        'project:Manage project bare repository store'
        'init:Initialize project and clone bare repositories'
        'add:Add and clone a new bare repository'
        'fetch:Fetch updates in all bare repositories'
        'sync:Sync and prune worktrees'
        'doctor:Run health check diagnostics'
        'hub:Collaborate, clone, publish, sync, and manage secrets with wshub'
        'clone:Clone and replicate a project from wshub'
        'daemon:Manage or run global background daemon'
        'service:Manage systemd user service (ws.service)'
        'completion:Generate or install shell completion scripts'
    )

    _ws_commands() {
        _describe -t commands 'ws command' commands
    }

    _ws_workspaces() {
        local -a ws_list
        ws_list=(${(f)"$(ws _complete workspaces 2>/dev/null)"})
        if [[ -n "$ws_list" ]]; then
            _describe -t workspaces 'workspace' ws_list
        fi
    }

    _ws_repositories() {
        local -a repo_list
        repo_list=(${(f)"$(ws _complete repos 2>/dev/null)"})
        if [[ -n "$repo_list" ]]; then
            _describe -t repositories 'repository' repo_list
        fi
    }

    _ws_workspace_or_service() {
        _ws_workspaces
        _ws_repositories
    }

    _ws_interfaces() {
        local -a iface_list
        iface_list=(${(f)"$(ws _complete interfaces 2>/dev/null)"})
        if [[ -n "$iface_list" ]]; then
            _describe -t interfaces 'network interface' iface_list
        fi
    }

    if [[ "$words[2]" == @* ]]; then
        local target_ws="${words[2]}"
        local subcmd="${words[3]}"

        if [[ $CURRENT -eq 3 ]]; then
            _ws_commands
            return
        fi

        case "$subcmd" in
            start|launch|run)
                _arguments \
                    '--all[Start all services in workspace]' \
                    '--tmux[Launch in Tmux session with vertical panes]' \
                    '(-z --zellij)'{-z,--zellij}'[Launch in Zellij session]' \
                    '(-t --terminal)'{-t,--terminal}'[Launch in separate terminal windows]' \
                    '--stream[Stream raw stdout/stderr without interactive TUI]' \
                    '(-d --daemon)'{-d,--daemon}'[Launch detached in background daemon]' \
                    '(-s --switch)'{-s,--switch}'[Zero-downtime switch to presentation engine]' \
                    '(-m --mode)'{-m,--mode}'[Multiplexer mode]:mode:(tui tmux zellij terminal stream daemon)' \
                    '--interface[Network interface name or type]:interface:_ws_interfaces' \
                    '--iface[Network interface name or type]:interface:_ws_interfaces' \
                    '--ip[Explicit host LAN IP address override]:ip:' \
                    '--lan-ip[Explicit host LAN IP address override]:ip:' \
                    '--attach[Focus single service]:service:_ws_repositories' \
                    '*:service:_ws_repositories'
                ;;
            attach)
                _arguments \
                    '--all[Attach in multi-pane grid view]' \
                    '--tmux[Attach using Tmux backend]' \
                    '(-z --zellij)'{-z,--zellij}'[Attach using Zellij backend]' \
                    '(-s --switch)'{-s,--switch}'[Zero-downtime switch presentation engine]' \
                    '(-m --mode)'{-m,--mode}'[Engine backend]:mode:(tui tmux zellij)' \
                    '1:service:_ws_repositories'
                ;;
            focus|switch)
                return
                ;;
            restart|logs|bridge|shell|enter|open|lock|unlock)
                _arguments '*:service:_ws_repositories'
                ;;
            env)
                _arguments \
                    '--sync[Sync environment variables into .env files]' \
                    '--interface[Network interface name or type]:interface:_ws_interfaces' \
                    '--iface[Network interface name or type]:interface:_ws_interfaces' \
                    '--ip[Explicit host LAN IP address override]:ip:' \
                    '--lan-ip[Explicit host LAN IP address override]:ip:' \
                    '*:service:_ws_repositories'
                ;;
            setup)
                _arguments \
                    '--all[Setup all repositories in workspace]' \
                    '--dry-run[Print setup commands without running them]' \
                    '--skip-scripts[Only sync environment variables without running scripts]' \
                    '--interface[Network interface name or type]:interface:_ws_interfaces' \
                    '--iface[Network interface name or type]:interface:_ws_interfaces' \
                    '--ip[Explicit host LAN IP address override]:ip:' \
                    '--lan-ip[Explicit host LAN IP address override]:ip:' \
                    '*:service:_ws_repositories'
                ;;
            push|pull)
                _arguments \
                    '--remote[Git remote name]:remote:(origin upstream)' \
                    '*:service:_ws_repositories'
                ;;
            *)
                _arguments '*:arguments:_files'
                ;;
        esac
        return
    fi

    _arguments -C \
        '(-v --verbose)'{-v,--verbose}'[Enable debug logging]' \
        '(-c --config)'{-c,--config}'[Path to repositories configuration file]:config file:_files' \
        '(-w --workspaces-dir)'{-w,--workspaces-dir}'[Directory for storing workspaces]:directory:_files -/' \
        '--version[Show version information]' \
        '1: :->command_or_workspace' \
        '*:: :->args'

    case $state in
        command_or_workspace)
            _ws_commands
            _ws_workspaces
            ;;
        args)
            local cmd="${words[1]}"
            case "$cmd" in
                create|new)
                    _arguments \
                        '1:workspace name:_ws_workspaces' \
                        '(-f --file)'{-f,--file}'[Path to workspace YAML file]:YAML file:_files -g "*.yml *.yaml"' \
                        '--setup[Run setup scripts after creation]' \
                        '--cmd[Command to run in workspace tmux window]:command:' \
                        '--command[Command to run in workspace tmux window]:command:' \
                        '--no-tmux[Skip creating a tmux window for this workspace]' \
                        '--all[Include all repositories]' \
                        '--existing[Checkout existing branches]' \
                        '*:repository specification:_ws_repositories'
                    ;;
                focus|switch)
                    _arguments '1:workspace:_ws_workspaces'
                    ;;
                start|launch|run)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '--all[Start all services in workspace]' \
                        '--tmux[Launch in Tmux session with vertical panes]' \
                        '(-z --zellij)'{-z,--zellij}'[Launch in Zellij session]' \
                        '(-t --terminal)'{-t,--terminal}'[Launch in separate terminal windows]' \
                        '--stream[Stream raw stdout/stderr without interactive TUI]' \
                        '(-d --daemon)'{-d,--daemon}'[Launch detached in background daemon]' \
                        '(-s --switch)'{-s,--switch}'[Zero-downtime switch to presentation engine]' \
                        '(-m --mode)'{-m,--mode}'[Multiplexer mode]:mode:(tui tmux zellij terminal stream daemon)' \
                        '--interface[Network interface name or type]:interface:_ws_interfaces' \
                        '--iface[Network interface name or type]:interface:_ws_interfaces' \
                        '--ip[Explicit host LAN IP address override]:ip:' \
                        '--lan-ip[Explicit host LAN IP address override]:ip:' \
                        '--attach[Focus single service]:service:_ws_repositories' \
                        '*:services:_ws_repositories'
                    ;;
                attach)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '2:service:_ws_repositories' \
                        '--all[Attach in multi-pane grid view]' \
                        '--tmux[Attach using Tmux backend]' \
                        '(-z --zellij)'{-z,--zellij}'[Attach using Zellij backend]' \
                        '(-s --switch)'{-s,--switch}'[Zero-downtime switch presentation engine]' \
                        '(-m --mode)'{-m,--mode}'[Engine backend]:mode:(tui tmux zellij)'
                    ;;
                end|close|delete|rm|remove)
                    _arguments \
                        '1:workspace:_ws_workspaces' \
                        '--no-merge[Allow closing unmerged branches]' \
                        '(-f --force)'{-f,--force}'[Force close regardless of uncommitted or unmerged work]' \
                        '--delete-branch[Delete Git branch from bare store]' \
                        '(-t --target --target-branch)'{-t,--target,--target-branch}'[Target base branch]:target:' \
                        '--no-tmux[Skip removing the workspace tmux window]'
                    ;;
                info|status)
                    _arguments '1:workspace or service:_ws_workspace_or_service'
                    ;;
                exec)
                    _arguments \
                        '--all[Execute across all repositories]' \
                        '--repos[Comma-separated repository list]:repos:' \
                        '*:args:_files'
                    ;;
                restart)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '*:service:_ws_repositories'
                    ;;
                logs)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '(-f --follow)'{-f,--follow}'[Follow live log output]' \
                        '(-n --lines)'{-n,--lines}'[Number of lines]:lines:' \
                        '*:service:_ws_repositories'
                    ;;
                bridge|shell|enter|open|lock|unlock)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '2:service:_ws_repositories'
                    ;;
                env)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '2:service:_ws_repositories' \
                        '--sync[Sync environment variables into .env files]' \
                        '--interface[Network interface name or type]:interface:_ws_interfaces' \
                        '--iface[Network interface name or type]:interface:_ws_interfaces' \
                        '--ip[Explicit host LAN IP address override]:ip:' \
                        '--lan-ip[Explicit host LAN IP address override]:ip:'
                    ;;
                setup)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '--all[Setup all repositories in workspace]' \
                        '--dry-run[Print setup commands without running them]' \
                        '--skip-scripts[Only sync environment variables without running scripts]' \
                        '--interface[Network interface name or type]:interface:_ws_interfaces' \
                        '--iface[Network interface name or type]:interface:_ws_interfaces' \
                        '--ip[Explicit host LAN IP address override]:ip:' \
                        '--lan-ip[Explicit host LAN IP address override]:ip:' \
                        '*:service:_ws_repositories'
                    ;;
                push|pull)
                    _arguments \
                        '1:workspace or service:_ws_workspace_or_service' \
                        '--remote[Git remote name]:remote:(origin upstream)' \
                        '*:service:_ws_repositories'
                    ;;
                repo|workspace)
                    _arguments \
                        '1:action:(add remove lock unlock)' \
                        '2:workspace:_ws_workspaces' \
                        '3:repository:_ws_repositories' \
                        '--existing[Checkout existing branch]' \
                        '--delete-branch[Also delete branch from bare store]'
                    ;;
                project)
                    _arguments '1:action:(init add fetch sync list register unregister)' '*:args:_files'
                    ;;
                daemon)
                    _arguments \
                        '--tick[Worker loop tick interval in seconds]:tick:' \
                        '1:action:(run)'
                    ;;
                service)
                    _arguments \
                        '1:action:(install uninstall start stop restart enable disable status logs)' \
                        '(-f --follow)'{-f,--follow}'[Follow live journal logs]' \
                        '(-n --lines)'{-n,--lines}'[Number of lines to display]:lines:'
                    ;;
                hub)
                    local hub_action="${words[2]}"
                    case "$hub_action" in
                        auto-save|autosave)
                            _arguments \
                                '2:action:(status start stop run once daemon service)' \
                                '--interval[Auto-save interval duration]:interval:' \
                                '--project[Override project identifier]:project:' \
                                '--force[Force save even if no changes detected]'
                            ;;
                        service)
                            _arguments \
                                '2:action:(install uninstall start stop restart enable disable status logs)'
                            ;;
                        state|resume)
                            _arguments \
                                '2:action:(save restore)' \
                                '3:workspace:_ws_workspaces' \
                                '--no-wip[Skip uncommitted changes]'
                            ;;
                        secret)
                            _arguments \
                                '2:action:(list set get delete upload pull)' \
                                '--project[Override project identifier]:project:'
                            ;;
                        *)
                            _arguments '1:action:(login whoami logout clone publish push pull status sync state resume auto-save secret service)'
                            ;;
                    esac
                    ;;
                clone)
                    _arguments '1:project:' '2:target directory:_files -/'
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

compdef _ws ws 2>/dev/null || true
"#;

pub const BASH_COMPLETION_TEMPLATE: &str = r#"# ------------------------------------------------------------------------------
# Bash completion script for `ws`
# Generated automatically by `ws completion bash`
# ------------------------------------------------------------------------------

_ws_completion() {
    local cur prev words cword
    if declare -F _init_completion >/dev/null 2>&1; then
        _init_completion || return
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
        prev="${COMP_WORDS[COMP_CWORD-1]}"
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    fi

    local commands="create new list ls info focus switch end close delete rm remove status exec push pull start launch run attach stop kill restart logs bridge shell enter open env setup repo lock unlock project init add fetch sync doctor antigravity hub clone daemon service completion"

    if [[ $cword -eq 1 ]]; then
        local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
        COMPREPLY=( $(compgen -W "${commands} ${workspaces}" -- "$cur") )
        return 0
    fi

    local subcmd="${words[1]}"
    local target_ws=""

    if [[ "$subcmd" == @* ]]; then
        target_ws="$subcmd"
        if [[ $cword -eq 2 ]]; then
            COMPREPLY=( $(compgen -W "${commands}" -- "$cur") )
            return 0
        fi
        subcmd="${words[2]}"
    elif [[ "${words[2]}" == @* ]]; then
        target_ws="${words[2]}"
    fi

    case "$subcmd" in
        create|new)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--file -f --setup --cmd --command --no-tmux --all --existing" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        start|launch|run)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --tmux --zellij -z --terminal -t --stream --daemon -d --switch -s --mode -m --interface --iface --ip --lan-ip --attach" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                local repos=$(ws _complete repos_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces} ${repos}" -- "$cur") )
            else
                local repos=$(ws _complete repos_all "${target_ws:-${words[2]}}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        env)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--sync --interface --iface --ip --lan-ip" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                local repos=$(ws _complete repos_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces} ${repos}" -- "$cur") )
            else
                local repos=$(ws _complete repos_all "${target_ws:-${words[2]}}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        setup)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --dry-run --skip-scripts --interface --iface --ip --lan-ip" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                local repos=$(ws _complete repos_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces} ${repos}" -- "$cur") )
            else
                local repos=$(ws _complete repos_all "${target_ws:-${words[2]}}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        end|close|delete|rm|remove|focus|switch|info|status)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--no-merge --force -f --delete-branch --target --target-branch -t --no-tmux" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            fi
            ;;
        attach|stop|kill|restart|logs|bridge|shell|enter|open|lock|unlock|push|pull)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --tmux -z --zellij --switch -s --follow -f --lines -n --remote" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                local repos=$(ws _complete repos_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces} ${repos}" -- "$cur") )
            else
                local repos=$(ws _complete repos_all "${target_ws:-${words[2]}}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        exec)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--all --repos" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                local repos=$(ws _complete repos_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces} ${repos}" -- "$cur") )
            fi
            ;;
        repo|workspace)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "add remove lock unlock" -- "$cur") )
            elif [[ $cword -eq 3 ]]; then
                local workspaces=$(ws _complete workspaces_all 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${workspaces}" -- "$cur") )
            else
                local repos=$(ws _complete repos_all "${words[3]}" 2>/dev/null | cut -d: -f1)
                COMPREPLY=( $(compgen -W "${repos}" -- "$cur") )
            fi
            ;;
        project)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "init add fetch sync list register unregister" -- "$cur") )
            fi
            ;;
        daemon)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--tick" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "run" -- "$cur") )
            fi
            ;;
        service)
            if [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "-f --follow -n --lines" -- "$cur") )
            elif [[ $cword -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "install uninstall start stop restart enable disable status logs" -- "$cur") )
            fi
            ;;
        hub)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "login whoami logout clone publish push pull status sync state resume auto-save secret service" -- "$cur") )
            elif [[ $cword -eq 3 ]]; then
                case "${words[2]}" in
                    auto-save|autosave)
                        COMPREPLY=( $(compgen -W "status start stop run once daemon service" -- "$cur") )
                        ;;
                    service)
                        COMPREPLY=( $(compgen -W "install uninstall start stop restart enable disable status logs" -- "$cur") )
                        ;;
                    secret)
                        COMPREPLY=( $(compgen -W "list set get delete upload pull" -- "$cur") )
                        ;;
                    state|resume)
                        COMPREPLY=( $(compgen -W "save restore" -- "$cur") )
                        ;;
                esac
            elif [[ $cword -ge 4 && ( "${words[2]}" == "auto-save" || "${words[2]}" == "autosave" ) ]]; then
                if [[ "${words[3]}" == "service" && $cword -eq 4 ]]; then
                    COMPREPLY=( $(compgen -W "install uninstall start stop restart enable disable status logs" -- "$cur") )
                elif [[ "$cur" == -* ]]; then
                    COMPREPLY=( $(compgen -W "--interval --project --force -d --daemon --tick" -- "$cur") )
                fi
            elif [[ "$cur" == -* ]]; then
                COMPREPLY=( $(compgen -W "--project --force --no-wip" -- "$cur") )
            fi
            ;;
        clone)
            ;;
        completion)
            COMPREPLY=( $(compgen -W "zsh bash fish install" -- "$cur") )
            ;;
        *)
            ;;
    esac
}

complete -F _ws_completion ws
"#;

pub const FISH_COMPLETION_TEMPLATE: &str = r#"# ------------------------------------------------------------------------------
# Fish completion script for `ws`
# Generated automatically by `ws completion fish`
# ------------------------------------------------------------------------------

function __fish_ws_workspaces
    ws _complete workspaces_all 2>/dev/null | string replace -r ':(.*)' '\t$1'
end

function __fish_ws_repos
    ws _complete repos_all 2>/dev/null | string replace -r ':(.*)' '\t$1'
end

function __fish_ws_interfaces
    ws _complete interfaces 2>/dev/null | string replace -r ':(.*)' '\t$1'
end

complete -c ws -f
complete -c ws -n "__fish_use_subcommand" -a "create" -d "Create workspace with Git worktrees"
complete -c ws -n "__fish_use_subcommand" -a "new" -d "Create workspace with Git worktrees"
complete -c ws -n "__fish_use_subcommand" -a "list" -d "List all workspaces"
complete -c ws -n "__fish_use_subcommand" -a "ls" -d "List all workspaces"
complete -c ws -n "__fish_use_subcommand" -a "info" -d "Display workspace details & processes"
complete -c ws -n "__fish_use_subcommand" -a "focus" -d "Focus or switch to workspace tmux window"
complete -c ws -n "__fish_use_subcommand" -a "switch" -d "Switch to workspace tmux window"
complete -c ws -n "__fish_use_subcommand" -a "end" -d "Safely end and close workspace"
complete -c ws -n "__fish_use_subcommand" -a "close" -d "Safely close workspace"
complete -c ws -n "__fish_use_subcommand" -a "delete" -d "Delete workspace and prune worktrees"
complete -c ws -n "__fish_use_subcommand" -a "rm" -d "Delete workspace"
complete -c ws -n "__fish_use_subcommand" -a "remove" -d "Delete workspace"
complete -c ws -n "__fish_use_subcommand" -a "status" -d "Show combined Git status"
complete -c ws -n "__fish_use_subcommand" -a "exec" -d "Execute command across workspace worktrees"
complete -c ws -n "__fish_use_subcommand" -a "start" -d "Start services in TUI or multiplexer"
complete -c ws -n "__fish_use_subcommand" -a "launch" -d "Start services in TUI or multiplexer"
complete -c ws -n "__fish_use_subcommand" -a "run" -d "Start services in TUI or multiplexer"
complete -c ws -n "__fish_use_subcommand" -a "attach" -d "Attach to running daemon session"
complete -c ws -n "__fish_use_subcommand" -a "stop" -d "Stop running workspace session"
complete -c ws -n "__fish_use_subcommand" -a "kill" -d "Stop running workspace session"
complete -c ws -n "__fish_use_subcommand" -a "restart" -d "Restart services in active workspace"
complete -c ws -n "__fish_use_subcommand" -a "logs" -d "View or tail service logs"
complete -c ws -n "__fish_use_subcommand" -a "bridge" -d "Raw terminal PTY bridge"
complete -c ws -n "__fish_use_subcommand" -a "shell" -d "Open interactive subshell"
complete -c ws -n "__fish_use_subcommand" -a "enter" -d "Open interactive subshell"
complete -c ws -n "__fish_use_subcommand" -a "open" -d "Open interactive subshell"
complete -c ws -n "__fish_use_subcommand" -a "env" -d "Inspect or sync environment variables"
complete -c ws -n "__fish_use_subcommand" -a "setup" -d "Run setup scripts and sync .env"
complete -c ws -n "__fish_use_subcommand" -a "repo" -d "Manage repositories inside workspace"
complete -c ws -n "__fish_use_subcommand" -a "lock" -d "Lock worktree tracked files read-only"
complete -c ws -n "__fish_use_subcommand" -a "unlock" -d "Unlock worktree tracked files writable"
complete -c ws -n "__fish_use_subcommand" -a "push" -d "Push committed changes to remotes"
complete -c ws -n "__fish_use_subcommand" -a "pull" -d "Pull remote updates"
complete -c ws -n "__fish_use_subcommand" -a "project" -d "Manage project bare repository store"
complete -c ws -n "__fish_use_subcommand" -a "init" -d "Initialize project and clone bare repositories"
complete -c ws -n "__fish_use_subcommand" -a "add" -d "Add and clone a new bare repository"
complete -c ws -n "__fish_use_subcommand" -a "fetch" -d "Fetch updates in all bare repositories"
complete -c ws -n "__fish_use_subcommand" -a "sync" -d "Sync and prune worktrees"
complete -c ws -n "__fish_use_subcommand" -a "doctor" -d "Run health check diagnostics"
complete -c ws -n "__fish_use_subcommand" -a "hub" -d "Collaborate, clone, publish, sync, and manage secrets with wshub"
complete -c ws -n "__fish_use_subcommand" -a "clone" -d "Clone and replicate a project from wshub"
complete -c ws -n "__fish_use_subcommand" -a "daemon" -d "Manage or run global background daemon"
complete -c ws -n "__fish_use_subcommand" -a "service" -d "Manage systemd user service (ws.service)"
complete -c ws -n "__fish_use_subcommand" -a "completion" -d "Generate completion scripts"

# Subcommands
complete -c ws -n "__fish_seen_subcommand_from project" -a "init add fetch sync list register unregister"
complete -c ws -n "__fish_seen_subcommand_from daemon" -a "run" -d "Run global daemon in foreground"
complete -c ws -n "__fish_seen_subcommand_from service" -a "install uninstall start stop restart enable disable status logs"
complete -c ws -n "__fish_seen_subcommand_from hub" -a "login whoami logout clone publish push pull status sync state resume auto-save secret service"
complete -c ws -n "__fish_seen_subcommand_from repo" -a "add remove lock unlock"

# Dynamic workspace and repo arguments
complete -c ws -n "__fish_seen_subcommand_from start attach info end close delete status restart logs bridge shell env setup lock unlock push pull" -a "(__fish_ws_workspaces)"
complete -c ws -n "__fish_seen_subcommand_from start attach restart logs bridge shell env setup lock unlock push pull" -a "(__fish_ws_repos)"

# Flags
complete -c ws -n "__fish_seen_subcommand_from end close delete rm remove" -l no-merge -d "Allow closing unmerged branches"
complete -c ws -n "__fish_seen_subcommand_from end close delete rm remove" -s f -l force -d "Force close regardless of uncommitted or unmerged work"
complete -c ws -n "__fish_seen_subcommand_from end close delete rm remove" -l delete-branch -d "Delete branch from bare store"
complete -c ws -n "__fish_seen_subcommand_from end close delete rm remove" -s t -l target -d "Target base branch to check merge status against"
complete -c ws -n "__fish_seen_subcommand_from end close delete rm remove" -l target-branch -d "Target base branch to check merge status against"
complete -c ws -n "__fish_seen_subcommand_from start" -l tmux -d "Launch in Tmux vertical panes"
complete -c ws -n "__fish_seen_subcommand_from start" -s z -l zellij -d "Launch in Zellij session"
complete -c ws -n "__fish_seen_subcommand_from start" -s d -l daemon -d "Launch in background daemon"
complete -c ws -n "__fish_seen_subcommand_from start" -s s -l switch -d "Zero-downtime presentation switch"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l interface -a "(__fish_ws_interfaces)" -d "Network interface name or type"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l iface -a "(__fish_ws_interfaces)" -d "Network interface name or type"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l ip -d "Explicit LAN IP address override"
complete -c ws -n "__fish_seen_subcommand_from start setup env" -l lan-ip -d "Explicit LAN IP address override"
complete -c ws -n "__fish_seen_subcommand_from attach" -s s -l switch -d "Zero-downtime presentation switch"
complete -c ws -n "__fish_seen_subcommand_from daemon" -l tick -d "Worker loop tick interval in seconds"
complete -c ws -n "__fish_seen_subcommand_from service" -s f -l follow -d "Follow live logs"
complete -c ws -n "__fish_seen_subcommand_from service" -s n -l lines -d "Number of lines to display"
complete -c ws -n "__fish_seen_subcommand_from completion" -a "zsh bash fish install"
"#;

pub fn generate_completion_script(shell: &str) -> Result<String, String> {
    match shell.to_lowercase().as_str() {
        "zsh" | "z" => Ok(ZSH_COMPLETION_TEMPLATE.to_string()),
        "bash" | "sh" => Ok(BASH_COMPLETION_TEMPLATE.to_string()),
        "fish" => Ok(FISH_COMPLETION_TEMPLATE.to_string()),
        _ => Err(format!(
            "Unsupported shell: '{}'. Supported shells: zsh, bash, fish",
            shell
        )),
    }
}

pub fn find_project_root_and_workspaces_dir() -> (Option<PathBuf>, Option<PathBuf>) {
    if let Ok(mut curr) = std::env::current_dir() {
        loop {
            if curr.join("repositories.yml").exists() || curr.join("repositories.yaml").exists() {
                let ws_dir = curr.join("workspaces");
                return (Some(curr), Some(ws_dir));
            }
            if !curr.pop() {
                break;
            }
        }
    }
    (None, None)
}

pub fn detect_active_workspace_name(ws_dir: Option<&Path>) -> Option<String> {
    if let Ok(curr) = std::env::current_dir() {
        if let Some(wd) = ws_dir {
            if let Ok(rel) = curr.strip_prefix(wd) {
                if let Some(first) = rel.components().next() {
                    let s = first.as_os_str().to_string_lossy().to_string();
                    return Some(s.trim_start_matches('@').to_string());
                }
            }
        }
        let mut p = curr;
        loop {
            if p.join("workspace.yml").exists() {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                return Some(name.trim_start_matches('@').to_string());
            }
            if !p.pop() {
                break;
            }
        }
    }
    None
}

pub fn query_workspaces(include_sigil: bool) -> Vec<(String, String)> {
    let (_, ws_dir) = find_project_root_and_workspaces_dir();
    let mut candidates = Vec::new();
    if let Some(wd) = ws_dir {
        if let Ok(entries) = fs::read_dir(wd) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = p.file_name().unwrap().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let clean = name.trim_start_matches('@');
                    let meta_file = p.join("workspace.yml");
                    let mut desc = "workspace".to_string();
                    if meta_file.is_file() {
                        if let Ok(content) = fs::read_to_string(&meta_file) {
                            if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                                let repos = doc.get("repositories").and_then(|v| v.as_mapping());
                                let repo_count = repos.map_or(0, |m| m.len());
                                let status =
                                    doc.get("status").and_then(|v| v.as_str()).unwrap_or("");
                                desc = if !status.is_empty() {
                                    format!("{} repos, {}", repo_count, status)
                                } else {
                                    format!("{} repos", repo_count)
                                };
                            }
                        }
                    }
                    let display_name = if include_sigil {
                        format!("@{}", clean)
                    } else {
                        clean.to_string()
                    };
                    candidates.push((display_name, desc));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates
}

pub fn query_repositories(
    workspace_name: Option<&str>,
    include_sigil: bool,
) -> Vec<(String, String)> {
    let (proj_root, ws_dir) = find_project_root_and_workspaces_dir();
    let mut candidates = Vec::new();

    let target_ws = match workspace_name {
        Some(s) => Some(s.trim_start_matches('@').to_string()),
        None => detect_active_workspace_name(ws_dir.as_deref()),
    };

    if let (Some(ref tws), Some(ref wd)) = (target_ws, &ws_dir) {
        let mut meta_file = wd.join(tws).join("workspace.yml");
        if !meta_file.exists() {
            meta_file = wd.join(format!("@{}", tws)).join("workspace.yml");
        }
        if meta_file.is_file() {
            if let Ok(content) = fs::read_to_string(&meta_file) {
                if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    if let Some(map) = doc.get("repositories").and_then(|v| v.as_mapping()) {
                        for (k, v) in map {
                            if let Some(r_name) = k.as_str() {
                                let br = v.get("branch").and_then(|b| b.as_str()).unwrap_or("main");
                                let disp = if include_sigil {
                                    format!("%{}", r_name)
                                } else {
                                    r_name.to_string()
                                };
                                candidates.push((disp, format!("branch: {}", br)));
                            }
                        }
                        if !candidates.is_empty() {
                            candidates.sort_by(|a, b| a.0.cmp(&b.0));
                            return candidates;
                        }
                    }
                }
            }
        }
    }

    if let Some(pr) = proj_root {
        if let Ok(cfg) = ConfigLoader::load_config(Some(&pr.join("repositories.yml")), None, true) {
            for (k, v) in cfg.repositories {
                let disp = if include_sigil { format!("%{}", k) } else { k };
                let desc = v.launch.clone().unwrap_or_else(|| "repository".to_string());
                candidates.push((disp, desc));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates
}

pub fn query_interfaces() -> Vec<(String, String)> {
    let mut candidates = vec![
        (
            "wifi".to_string(),
            "Prioritize wireless Wi-Fi adapter".to_string(),
        ),
        (
            "ethernet".to_string(),
            "Prioritize wired Ethernet adapter".to_string(),
        ),
    ];
    let ifaces = list_network_interfaces();
    for i in ifaces {
        let type_str = match i.iface_type {
            crate::network::InterfaceType::Wireless => "wireless",
            crate::network::InterfaceType::Ethernet => "ethernet",
            crate::network::InterfaceType::Other => "other",
        };
        candidates.push((i.name, format!("{} interface ({})", type_str, i.ip)));
    }
    candidates
}

pub fn query_completions(query_type: &str, target: Option<&str>) -> Vec<String> {
    match query_type {
        "workspaces" => query_workspaces(true)
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        "workspaces_all" => query_workspaces(true)
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        "workspaces_plain" => query_workspaces(false)
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        "repos" => query_repositories(target, true)
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        "repos_all" => query_repositories(target, true)
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        "repos_plain" => query_repositories(target, false)
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        "interfaces" => query_interfaces()
            .into_iter()
            .map(|(c, d)| format!("{}:{}", c, d))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn install_completion(shell: Option<&str>) -> std::io::Result<(bool, String)> {
    let detected_shell = shell
        .map(|s| s.to_string())
        .or_else(|| {
            std::env::var("SHELL").ok().and_then(|s| {
                Path::new(&s)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
            })
        })
        .unwrap_or_else(|| "zsh".to_string());

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    if detected_shell.contains("zsh") {
        let zsh_dir = home.join(".zsh").join("completions");
        fs::create_dir_all(&zsh_dir)?;
        let comp_file = zsh_dir.join("_ws");
        fs::write(&comp_file, ZSH_COMPLETION_TEMPLATE)?;
        Ok((
            true,
            format!(
                "✔ Installed Zsh completions to {}.\n\nTo activate immediately in your current terminal session, run:\n  source <(ws completion zsh)\n\nTo ensure completions are permanently loaded, add this to your ~/.zshrc:\n  fpath=(~/.zsh/completions $fpath)\n  autoload -Uz compinit && compinit\n",
                comp_file.display()
            ),
        ))
    } else if detected_shell.contains("bash") {
        let bash_dir = home
            .join(".local")
            .join("share")
            .join("bash-completion")
            .join("completions");
        fs::create_dir_all(&bash_dir)?;
        let comp_file = bash_dir.join("ws");
        fs::write(&comp_file, BASH_COMPLETION_TEMPLATE)?;
        Ok((
            true,
            format!(
                "✔ Installed Bash completions to {}.\n\nTo activate in your current session, run:\n  eval \"$(ws completion bash)\"\n",
                comp_file.display()
            ),
        ))
    } else if detected_shell.contains("fish") {
        let fish_dir = home.join(".config").join("fish").join("completions");
        fs::create_dir_all(&fish_dir)?;
        let comp_file = fish_dir.join("ws.fish");
        fs::write(&comp_file, FISH_COMPLETION_TEMPLATE)?;
        Ok((
            true,
            format!("✔ Installed Fish completions to {}.", comp_file.display()),
        ))
    } else {
        Ok((false, format!("Unknown shell '{}'", detected_shell)))
    }
}
