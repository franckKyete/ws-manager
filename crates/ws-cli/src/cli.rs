use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "ws",
    about = "Multi-repository Git workspace manager using git worktree.",
    version
)]
pub struct Cli {
    #[arg(short = 'v', long = "verbose", help = "Enable debug log output")]
    pub verbose: bool,

    #[arg(
        short = 'c',
        long = "config",
        help = "Path to repositories configuration file"
    )]
    pub config: Option<PathBuf>,

    #[arg(
        short = 'w',
        long = "workspaces-dir",
        help = "Directory for storing workspaces (default: workspaces)"
    )]
    pub workspaces_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    // ==================== 1. Workspace Lifecycle ====================
    #[command(
        alias = "new",
        about = "Create a workspace from parameters or YAML file"
    )]
    Create(CreateArgs),

    #[command(alias = "ls", about = "List all workspaces")]
    List,

    #[command(about = "Display details and live process status for a workspace")]
    Info {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
    },

    #[command(alias = "switch", about = "Focus or switch to workspace tmux window")]
    Focus {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
    },

    #[command(
        aliases = ["close", "delete", "rm", "remove"],
        about = "Safely end and close a workspace, pruning all its worktrees"
    )]
    End(EndArgs),

    #[command(about = "Show Git status across all workspace worktrees")]
    Status {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
    },

    #[command(about = "Execute command inside each repo worktree of a workspace")]
    Exec(ExecArgs),

    // ==================== 2. Git Collaboration ====================
    #[command(about = "Push committed changes for workspace repositories to remotes")]
    Push(PushPullArgs),

    #[command(about = "Pull remote updates for workspace repositories")]
    Pull(PushPullArgs),

    // ==================== 3. Worktree & Repo Management ====================
    #[command(
        alias = "workspace",
        about = "Manage repositories inside an existing workspace"
    )]
    Repo(RepoCommandArgs),

    #[command(about = "Lock repository worktree (read-only)")]
    Lock {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository name (%<repo>)")]
        repo: Option<String>,
    },

    #[command(about = "Unlock repository worktree (writable)")]
    Unlock {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository name (%<repo>)")]
        repo: Option<String>,
    },

    // ==================== 4. Service Runtime & Multiplexers ====================
    #[command(aliases = ["launch", "run"], about = "Start workspace services concurrently")]
    Start(StartArgs),

    #[command(about = "Attach to a running workspace session")]
    Attach(AttachArgs),

    #[command(alias = "kill", about = "Stop running workspace background session")]
    Stop {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
    },

    #[command(about = "Restart running workspace services")]
    Restart {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Services to restart (%<repo>)")]
        repos: Vec<String>,
    },

    #[command(about = "View service logs")]
    Logs(LogsArgs),

    #[command(about = "Connect raw terminal I/O bridge to a running workspace service")]
    Bridge {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository service name (%<repo>)")]
        repo: Option<String>,
    },

    // ==================== 5. Developer Shell & Environment ====================
    #[command(aliases = ["enter", "open"], about = "Open interactive subshell inside workspace or worktree")]
    Shell {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository worktree name to open subshell into (%<repo>)")]
        worktree: Option<String>,
    },

    #[command(about = "Inspect or sync environment variables for a workspace")]
    Env(EnvArgs),

    #[command(about = "Run setup scripts and environment variable sync for a workspace")]
    Setup(SetupArgs),

    // ==================== 6. Project & Bare Repositories ====================
    #[command(about = "Manage project-wide bare repositories")]
    Project(ProjectCommandArgs),

    #[command(about = "Initialize project and clone bare repositories")]
    Init {
        #[arg(help = "Git repository URLs")]
        urls: Vec<String>,
    },

    #[command(about = "Add and clone a new bare repository")]
    Add {
        #[arg(help = "Git repository URL or name=URL")]
        url: String,
    },

    #[command(about = "Fetch updates in all bare repositories")]
    Fetch,

    #[command(about = "Sync and prune worktrees")]
    Sync,

    #[command(about = "Run system health checks and diagnostics")]
    Doctor,

    #[command(about = "Antigravity AI agent workspace health check")]
    Antigravity,

    // ==================== 7. wshub Cloud Integration ====================
    #[command(about = "Collaborate, clone, publish, sync, and manage secrets with wshub")]
    Hub(HubCommandArgs),

    #[command(about = "Clone and replicate a project from wshub")]
    Clone {
        #[arg(help = "Project identifier (e.g. 'org/project' or 'project')")]
        project: String,
        #[arg(help = "Target directory path (optional)")]
        target_dir: Option<PathBuf>,
    },

    // ==================== 8. Completions, Daemon & Service ====================
    #[command(about = "Generate or install shell completion scripts")]
    Completion {
        #[arg(help = "Shell type: zsh, bash, fish, install (default: zsh)")]
        shell: Option<String>,
        #[arg(long, help = "Install completion script into user shell configuration")]
        install: bool,
    },

    #[command(name = "_complete", hide = true)]
    InternalComplete {
        query_type: String,
        query_args: Vec<String>,
    },

    #[command(about = "Manage or run global background daemon")]
    Daemon {
        #[arg(
            long,
            default_value = "15",
            help = "Worker loop tick interval in seconds"
        )]
        tick: u64,
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },

    #[command(about = "Manage systemd user service (ws.service)")]
    Service {
        #[command(subcommand)]
        action: Option<ServiceAction>,
    },
}

// Subcommand Argument Structs

#[derive(Args, Debug)]
pub struct CreateArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(
        short = 'f',
        long = "file",
        help = "Path to workspace YAML configuration file"
    )]
    pub file: Option<PathBuf>,

    #[arg(
        long = "setup",
        help = "Run setup scripts and sync environment variables after creation"
    )]
    pub setup: bool,

    #[arg(
        long = "cmd",
        visible_alias = "command",
        help = "Command to run in workspace tmux window"
    )]
    pub tmux_cmd: Option<String>,

    #[arg(
        long = "no-tmux",
        help = "Skip creating a tmux window for this workspace"
    )]
    pub no_tmux: bool,

    #[arg(
        short = 't',
        long = "target",
        visible_aliases = ["target-branch", "base", "from"],
        help = "Base branch to create workspace branches from"
    )]
    pub target_branch: Option<String>,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub extra_args: Vec<String>,
}

#[derive(Args, Debug)]
pub struct EndArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(
        long = "no-merge",
        help = "Allow closing even if committed branches are not merged"
    )]
    pub no_merge: bool,

    #[arg(
        short = 'f',
        long = "force",
        help = "Force close regardless of uncommitted changes"
    )]
    pub force: bool,

    #[arg(
        long = "delete-branch",
        help = "Also delete the Git branch from the bare repository store"
    )]
    pub delete_branch: bool,

    #[arg(
        short = 't',
        long = "target",
        visible_alias = "target-branch",
        help = "Target base branch"
    )]
    pub target_branch: Option<String>,

    #[arg(long = "no-tmux", help = "Skip removing the workspace tmux window")]
    pub no_tmux: bool,
}

#[derive(Args, Debug)]
pub struct ExecArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(long = "all", help = "Execute across all worktrees in workspace")]
    pub all: bool,

    #[arg(long = "repos", help = "Comma-separated list of repository names")]
    pub repos_flag: Option<String>,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Args, Debug)]
pub struct PushPullArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(help = "Repository names (%<repo>)")]
    pub repos: Vec<String>,

    #[arg(long = "repos", help = "Comma-separated list of repository names")]
    pub repos_flag: Option<String>,

    #[arg(long = "remote", default_value = "origin", help = "Git remote name")]
    pub remote: String,
}

#[derive(Args, Debug)]
pub struct RepoCommandArgs {
    #[command(subcommand)]
    pub action: Option<RepoAction>,
}

#[derive(Subcommand, Debug)]
pub enum RepoAction {
    #[command(
        alias = "add-repo",
        about = "Add a repository worktree to an existing workspace"
    )]
    Add {
        name: Option<String>,
        repo: Option<String>,
        branch: Option<String>,
        #[arg(long = "existing")]
        existing: bool,
    },
    #[command(aliases = ["rm", "remove-repo"], about = "Remove a repository worktree from a workspace")]
    Remove {
        name: Option<String>,
        repo: Option<String>,
        #[arg(long = "delete-branch")]
        delete_branch: bool,
    },
    #[command(
        alias = "freeze",
        about = "Lock repository worktree (mark files read-only)"
    )]
    Lock {
        name: Option<String>,
        repo: Option<String>,
    },
    #[command(
        alias = "unfreeze",
        about = "Unlock repository worktree (restore write permissions)"
    )]
    Unlock {
        name: Option<String>,
        repo: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct StartArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(help = "Services to start (%<repo>)")]
    pub repos: Vec<String>,

    #[arg(long = "all", help = "Start all services in workspace")]
    pub all: bool,

    #[arg(
        long = "repos",
        visible_alias = "only",
        help = "Comma-separated list of services"
    )]
    pub repos_flag: Option<String>,

    #[arg(
        long = "attach",
        help = "Focus or connect directly to a single service"
    )]
    pub attach: Option<String>,

    #[arg(long = "tmux", help = "Launch in tmux session")]
    pub tmux: bool,

    #[arg(short = 'z', long = "zellij", help = "Launch in Zellij session")]
    pub zellij: bool,

    #[arg(
        short = 't',
        long = "terminal",
        help = "Launch in separate terminal windows/tabs"
    )]
    pub terminal: bool,

    #[arg(
        long = "stream",
        help = "Stream raw stdout/stderr without interactive TUI"
    )]
    pub stream: bool,

    #[arg(
        short = 'd',
        long = "daemon",
        visible_alias = "background",
        help = "Launch detached in background daemon"
    )]
    pub daemon: bool,

    #[arg(
        short = 's',
        long = "switch",
        help = "Zero-downtime switch to target presentation engine"
    )]
    pub switch: bool,

    #[arg(short = 'm', long = "mode", help = "Multiplexer/UI mode")]
    pub mode: Option<String>,

    #[arg(long = "interface", visible_aliases = ["iface", "lan-interface"], help = "Network interface name or type")]
    pub interface: Option<String>,

    #[arg(
        long = "ip",
        visible_alias = "lan-ip",
        help = "Explicit host LAN IP override"
    )]
    pub lan_ip: Option<String>,
}

#[derive(Args, Debug)]
pub struct AttachArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(help = "Service name to focus (%<repo>)")]
    pub repo: Option<String>,

    #[arg(long = "all", help = "Attach in multi-pane grid view")]
    pub all: bool,

    #[arg(
        short = 's',
        long = "switch",
        help = "Zero-downtime switch presentation engine"
    )]
    pub switch: bool,

    #[arg(long = "tmux", help = "Attach using tmux backend")]
    pub tmux: bool,

    #[arg(short = 'z', long = "zellij", help = "Attach using Zellij backend")]
    pub zellij: bool,

    #[arg(short = 'm', long = "mode", help = "Multiplexer engine backend")]
    pub mode: Option<String>,
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(help = "Service name (%<repo>)")]
    pub repo: Option<String>,

    #[arg(short = 'f', long = "follow", help = "Follow log output")]
    pub follow: bool,

    #[arg(
        short = 'n',
        long = "lines",
        default_value = "50",
        help = "Number of lines to display"
    )]
    pub lines: usize,
}

#[derive(Args, Debug)]
pub struct EnvArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(help = "Repository name (%<repo>)")]
    pub repo: Option<String>,

    #[arg(
        long = "sync",
        help = "Sync resolved environment variables into worktree .env files"
    )]
    pub sync: bool,

    #[arg(long = "interface", visible_aliases = ["iface", "lan-interface"], help = "Network interface name or type")]
    pub interface: Option<String>,

    #[arg(
        long = "ip",
        visible_alias = "lan-ip",
        help = "Explicit host LAN IP override"
    )]
    pub lan_ip: Option<String>,
}

#[derive(Args, Debug)]
pub struct SetupArgs {
    #[arg(help = "Workspace name (@<name>)")]
    pub name: Option<String>,

    #[arg(help = "Repository names to setup (%<repo>)")]
    pub repos: Vec<String>,

    #[arg(long = "all", help = "Setup all repositories in the workspace")]
    pub all: bool,

    #[arg(
        long = "repos",
        visible_alias = "only",
        help = "Comma-separated list of repository names to setup"
    )]
    pub repos_flag: Option<String>,

    #[arg(long = "dry-run", help = "Print setup commands without executing them")]
    pub dry_run: bool,

    #[arg(
        long = "skip-scripts",
        help = "Only sync environment variables without running setup scripts"
    )]
    pub skip_scripts: bool,

    #[arg(long = "interface", visible_aliases = ["iface", "lan-interface"], help = "Network interface name or type")]
    pub interface: Option<String>,

    #[arg(
        long = "ip",
        visible_alias = "lan-ip",
        help = "Explicit host LAN IP override"
    )]
    pub lan_ip: Option<String>,
}

#[derive(Args, Debug)]
pub struct ProjectCommandArgs {
    #[command(subcommand)]
    pub action: Option<ProjectAction>,
}

#[derive(Subcommand, Debug)]
pub enum ProjectAction {
    #[command(about = "Initialize project and clone bare repositories")]
    Init { urls: Vec<String> },
    #[command(about = "Add and clone a new bare repository")]
    Add { url: String },
    #[command(about = "Fetch updates in all bare repositories")]
    Fetch,
    #[command(about = "Sync and prune worktrees")]
    Sync,
    #[command(alias = "ls", about = "List all registered projects")]
    List,
    #[command(about = "Register a project in the global registry")]
    Register { path: Option<PathBuf> },
    #[command(about = "Unregister a project from the global registry")]
    Unregister { path: Option<PathBuf> },
}

#[derive(Args, Debug)]
pub struct HubCommandArgs {
    #[command(subcommand)]
    pub action: Option<HubAction>,
}

#[derive(Subcommand, Debug)]
pub enum HubAction {
    #[command(about = "Authenticate with wshub")]
    Login {
        #[arg(long = "url")]
        url: Option<String>,
        #[arg(long = "token")]
        token: Option<String>,
        #[arg(short = 'u', long = "username")]
        username: Option<String>,
        #[arg(short = 'p', long = "password")]
        password: Option<String>,
    },
    #[command(about = "Display active wshub user profile and session")]
    Whoami,
    #[command(about = "Log out and clear saved wshub credentials")]
    Logout,
    #[command(about = "Clone and replicate a project from wshub")]
    Clone {
        project: String,
        target_dir: Option<PathBuf>,
    },
    #[command(about = "Publish local workspace project definition to wshub")]
    Publish {
        project: Option<String>,
        #[arg(short = 'd', long = "description")]
        description: Option<String>,
    },
    #[command(about = "Push updated project blueprint to wshub")]
    Push {
        #[arg(short = 'm', long = "message", default_value = "Update configuration")]
        message: String,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Pull latest project blueprint and clone new bare repos from wshub")]
    Pull {
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Check project revision status against wshub")]
    Status {
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Synchronize project blueprint, bare repos, and secrets from wshub")]
    Sync {
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Manage cross-machine workspace session state")]
    State {
        #[command(subcommand)]
        action: HubStateAction,
    },
    #[command(about = "Restore workspace state from wshub on this machine")]
    Resume {
        workspace: String,
        #[arg(long = "project")]
        project: Option<String>,
        #[arg(long = "no-wip")]
        no_wip: bool,
    },
    #[command(
        alias = "autosave",
        about = "Manage periodic automatic workspace saving to wshub"
    )]
    AutoSave {
        #[command(subcommand)]
        action: Option<HubAutoSaveAction>,
    },
    #[command(about = "Manage zero-Git encrypted secrets in wshub vault")]
    Secret {
        #[command(subcommand)]
        action: HubSecretAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum HubStateAction {
    #[command(about = "Save workspace state to wshub")]
    Save {
        workspace: String,
        #[arg(long = "project")]
        project: Option<String>,
        #[arg(long = "no-wip")]
        no_wip: bool,
        #[arg(long = "auto")]
        auto: bool,
    },
    #[command(about = "Restore workspace state from wshub")]
    Restore {
        workspace: String,
        #[arg(long = "project")]
        project: Option<String>,
        #[arg(long = "no-wip")]
        no_wip: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum HubAutoSaveAction {
    #[command(about = "Show status of automatic hub saving")]
    Status,
    #[command(about = "Start background auto-save daemon")]
    Start {
        #[arg(long = "interval")]
        interval: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
        #[arg(short = 'd', long = "daemon", default_value = "true")]
        daemon: bool,
    },
    #[command(about = "Stop background auto-save daemon")]
    Stop,
    #[command(about = "Run auto-save loop in the foreground")]
    Run {
        #[arg(long = "interval")]
        interval: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Check and save any changed workspaces immediately")]
    Once {
        #[arg(long = "project")]
        project: Option<String>,
        #[arg(long = "force")]
        force: bool,
    },
    #[command(about = "Run global auto-save daemon in foreground")]
    Daemon {
        #[arg(long = "tick", default_value = "15")]
        tick: u64,
    },
    #[command(about = "Manage systemd user service")]
    Service {
        #[command(subcommand)]
        action: Option<ServiceAction>,
    },
}

#[derive(Subcommand, Debug)]
pub enum HubSecretAction {
    #[command(about = "List project secrets in vault")]
    List {
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Set an encrypted secret in wshub vault")]
    Set {
        key: String,
        value: String,
        #[arg(long = "repo")]
        repo: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Get a secret value from wshub vault")]
    Get {
        key: String,
        #[arg(long = "repo")]
        repo: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Delete a secret from wshub vault")]
    Delete {
        key: String,
        #[arg(long = "repo")]
        repo: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Upload and encrypt a sensitive file")]
    Upload {
        file_path: String,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Download and decrypt sensitive files")]
    Pull {
        #[arg(long = "project")]
        project: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum DaemonAction {
    #[command(about = "Run global daemon in foreground")]
    Run,
}

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    #[command(about = "Install, enable, and start ws.service")]
    Install,
    #[command(about = "Stop, disable, and remove ws.service")]
    Uninstall,
    #[command(about = "Start ws.service via systemctl")]
    Start,
    #[command(about = "Stop ws.service via systemctl")]
    Stop,
    #[command(about = "Restart ws.service via systemctl")]
    Restart,
    #[command(about = "Enable ws.service on boot")]
    Enable,
    #[command(about = "Disable ws.service on boot")]
    Disable,
    #[command(about = "Show systemd status of ws.service")]
    Status,
    #[command(about = "Stream journalctl logs for ws.service")]
    Logs {
        #[arg(short = 'f', long = "follow", default_value = "true")]
        follow: bool,
        #[arg(short = 'n', long = "lines", default_value = "50")]
        lines: usize,
    },
}
