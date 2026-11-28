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

    #[command(about = "Run or inspect workspace lifecycle stages")]
    Lifecycle(LifecycleCommandArgs),

    // ==================== 6. Project & Bare Repositories ====================
    #[command(about = "Manage project-wide bare repositories")]
    Project(ProjectCommandArgs),

    #[command(about = "Initialize project and clone bare repositories")]
    Init {
        #[arg(help = "Git repository URLs")]
        urls: Vec<String>,
        #[arg(
            long = "yes",
            help = "Non-interactive: accept detected default base branches"
        )]
        yes: bool,
    },

    #[command(about = "Add and clone a new bare repository")]
    Add {
        #[arg(help = "Git repository URL or name=URL")]
        url: String,
        #[arg(
            long = "yes",
            help = "Non-interactive: accept detected default base branches"
        )]
        yes: bool,
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

    #[command(about = "Track local activity and sync time reports")]
    Activity(ActivityCommandArgs),
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

    #[arg(
        long = "yes",
        help = "Non-interactive: accept detected default base branches"
    )]
    pub yes: bool,

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

    #[arg(help = "Repository names (%<repo>)")]
    pub repos: Vec<String>,

    #[arg(long = "all", help = "Execute across all worktrees in workspace")]
    pub all: bool,

    #[arg(
        last = true,
        allow_hyphen_values = true,
        help = "Command to execute (after --)"
    )]
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
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository specification (%<repo>[:branch])")]
        repo: Option<String>,
        #[arg(help = "Target branch name")]
        branch: Option<String>,
        #[arg(
            long = "base",
            visible_aliases = ["target", "from"],
            help = "Base branch for a newly created branch (one-shot override)"
        )]
        base: Option<String>,
        #[arg(long = "existing")]
        existing: bool,
        #[arg(
            long = "yes",
            help = "Non-interactive: accept detected default base branches"
        )]
        yes: bool,
    },
    #[command(aliases = ["rm", "remove-repo"], about = "Remove a repository worktree from a workspace")]
    Remove {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository name (%<repo>)")]
        repo: Option<String>,
        #[arg(long = "delete-branch")]
        delete_branch: bool,
    },
    #[command(
        alias = "freeze",
        about = "Lock repository worktree (mark files read-only)"
    )]
    Lock {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository name (%<repo>)")]
        repo: Option<String>,
    },
    #[command(
        alias = "unfreeze",
        about = "Unlock repository worktree (restore write permissions)"
    )]
    Unlock {
        #[arg(help = "Workspace name (@<name>)")]
        name: Option<String>,
        #[arg(help = "Repository name (%<repo>)")]
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
    Init {
        urls: Vec<String>,
        #[arg(
            long = "yes",
            help = "Non-interactive: accept detected default base branches"
        )]
        yes: bool,
    },
    #[command(about = "Add and clone a new bare repository")]
    Add {
        url: String,
        #[arg(
            long = "yes",
            help = "Non-interactive: accept detected default base branches"
        )]
        yes: bool,
    },
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
    #[command(about = "Migrate project configuration/state")]
    Migrate {
        #[command(subcommand)]
        action: ProjectMigrateAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectMigrateAction {
    #[command(about = "Migrate ws-project.yml and workspace state to lifecycle schema v3")]
    Lifecycle {
        #[arg(long = "dry-run", help = "Show migration plan without writing changes")]
        dry_run: bool,
        #[arg(long = "yes", help = "Apply migration without confirmation prompt")]
        yes: bool,
        #[arg(
            long = "no-backup",
            help = "Do not create a backup of the configuration file"
        )]
        no_backup: bool,
        #[arg(
            long = "stage-name",
            default_value = "install",
            help = "Setup stage name for migrated legacy/setup commands"
        )]
        stage_name: String,
        #[arg(
            long = "source",
            default_value = "auto",
            help = "Migration source: auto|legacy|v1"
        )]
        source: String,
        #[arg(long = "no-state", help = "Do not migrate workspace lifecycle state")]
        no_state: bool,
        #[arg(long = "reset-state", help = "Reset lifecycle state before migration")]
        reset_state: bool,
    },
    #[command(about = "Rename legacy repositories.yml to ws-project.yml")]
    Config {
        #[arg(long = "dry-run", help = "Show rename plan without changing files")]
        dry_run: bool,
    },
}

#[derive(Args, Debug)]
#[command(
    about = "Run or inspect workspace lifecycle stages",
    long_about = "Run or inspect workspace lifecycle stages.\n\n\
The workspace is inferred from the current directory when @workspace is omitted.\n\
When @workspace, %repo and --all are all omitted, the repository is inferred from\n\
the current directory. Stage names must be prefixed with '+' (for example\n\
'+configure', '+install', '+pre_launch'). With no stages, 'run' executes\n\
configure followed by every setup stage in order.",
    override_usage = "ws lifecycle [@workspace] [%repo ...] run [--all] [+stage ...] [flags]\n       ws lifecycle [@workspace] [%repo ...] status [+stage ...]"
)]
pub struct LifecycleCommandArgs {
    #[arg(
        value_name = "ARGS",
        num_args = 0..,
        help = "Positional arguments: [@workspace] [%repo ...] run|status [+stage ...]"
    )]
    pub raw: Vec<String>,

    #[arg(long = "all", help = "Run for all repositories in the workspace")]
    pub all: bool,

    #[arg(
        long = "force",
        help = "Force re-run even if stage is marked completed"
    )]
    pub force: bool,

    #[arg(long = "dry-run", help = "Show commands without executing them")]
    pub dry_run: bool,

    #[arg(
        long = "skip-configure",
        help = "Skip automatic configure stage before setup stages"
    )]
    pub skip_configure: bool,

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
pub struct HubCommandArgs {
    #[command(subcommand)]
    pub action: Option<HubAction>,
}

#[derive(Subcommand, Debug)]
pub enum HubAction {
    #[command(about = "Discover wshub instances on local network (LAN)")]
    Discover {
        #[arg(
            short = 't',
            long = "timeout",
            default_value = "1500",
            help = "Discovery listen timeout in milliseconds"
        )]
        timeout: u64,
        #[arg(
            long = "set",
            help = "Save the discovered hub as the active endpoint in configuration"
        )]
        set: bool,
    },
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
        #[arg(
            long = "discover",
            help = "Auto-discover wshub on LAN if --url is not provided"
        )]
        discover: bool,
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
        #[arg(help = "Workspace name (@<name>)")]
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
        #[arg(help = "Workspace name (@<name>)")]
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
        #[arg(help = "Workspace name (@<name>)")]
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
        #[arg(long = "repo", help = "Repository name (%<repo>)")]
        repo: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Get a secret value from wshub vault")]
    Get {
        key: String,
        #[arg(long = "repo", help = "Repository name (%<repo>)")]
        repo: Option<String>,
        #[arg(long = "project")]
        project: Option<String>,
    },
    #[command(about = "Delete a secret from wshub vault")]
    Delete {
        key: String,
        #[arg(long = "repo", help = "Repository name (%<repo>)")]
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

#[derive(Args, Debug)]
pub struct ActivityCommandArgs {
    #[command(subcommand)]
    pub action: Option<ActivityAction>,
}

#[derive(Subcommand, Debug)]
pub enum ActivityAction {
    #[command(about = "Show activity collector, daemon, DB and sync status")]
    Status {
        #[arg(long, help = "Output machine-readable JSON")]
        json: bool,
        #[arg(long, help = "Output compact Waybar JSON")]
        waybar: bool,
        #[arg(long, help = "Recompute rollups before reading (default: read-only)")]
        fresh: bool,
        #[arg(long, help = "Include daemon performance counters")]
        stats: bool,
    },
    #[command(about = "Report time spent by project, workspace and repo")]
    Report {
        #[arg(long, help = "Report today (default)")]
        day: bool,
        #[arg(long, help = "Report the last 7 days")]
        week: bool,
        #[arg(long, help = "Report the last 30 days")]
        month: bool,
        #[arg(long, help = "Start date/time (YYYY-MM-DD or RFC3339)")]
        from: Option<String>,
        #[arg(long, help = "End date/time (exclusive, YYYY-MM-DD or RFC3339)")]
        to: Option<String>,
        #[arg(long, help = "Filter by project name or id")]
        project: Option<String>,
        #[arg(long, help = "Filter by repository name")]
        repo: Option<String>,
        #[arg(long, help = "Filter by workspace name")]
        workspace: Option<String>,
        #[arg(
            long,
            help = "Filter by device id (not applied to local bucketing; hub views only)"
        )]
        device: Option<String>,
        #[arg(
            long,
            help = "Display timezone (not applied to local bucketing; hub views only)"
        )]
        tz: Option<String>,
        #[arg(long, help = "Output JSON")]
        json: bool,
        #[arg(long, help = "Output CSV")]
        csv: bool,
        #[arg(long, help = "Recompute rollups before reading (default: read-only)")]
        fresh: bool,
    },
    #[command(about = "Show a dense local-aligned activity snapshot")]
    Snapshot {
        #[arg(long, default_value = "today", help = "Range: today, week or month")]
        range: String,
        #[arg(long, help = "Filter by project name or id")]
        project: Option<String>,
        #[arg(long, help = "Output JSON")]
        json: bool,
        #[arg(long, help = "Recompute rollups before reading (default: read-only)")]
        fresh: bool,
    },
    #[command(about = "Show presence state and computer usage totals")]
    Presence {
        #[arg(long, default_value = "today", help = "Range: today, week or month")]
        range: String,
        #[arg(long, help = "Output JSON")]
        json: bool,
        #[arg(long, help = "Recompute rollups before reading (default: read-only)")]
        fresh: bool,
        #[command(subcommand)]
        action: Option<ActivityPresenceAction>,
    },
    #[command(about = "Show time-series activity buckets")]
    Series {
        #[arg(long, help = "Report today (default)")]
        day: bool,
        #[arg(long, help = "Report the last 7 days")]
        week: bool,
        #[arg(long, help = "Report the last 30 days")]
        month: bool,
        #[arg(long, help = "Start date/time (YYYY-MM-DD or RFC3339)")]
        from: Option<String>,
        #[arg(long, help = "End date/time (exclusive, YYYY-MM-DD or RFC3339)")]
        to: Option<String>,
        #[arg(long, help = "Bucket step: hour or day", default_value = "hour")]
        step: String,
        #[arg(long, help = "Filter by project name or id")]
        project: Option<String>,
        #[arg(long, help = "Filter by repository name")]
        repo: Option<String>,
        #[arg(long, help = "Filter by workspace name")]
        workspace: Option<String>,
        #[arg(
            long,
            help = "Group series by: none or project",
            default_value = "none"
        )]
        group_by: String,
        #[arg(long, help = "Output JSON (default)")]
        json: bool,
        #[arg(long, help = "Recompute rollups before reading (default: read-only)")]
        fresh: bool,
    },
    #[command(about = "Open the GTK activity dashboard (requires ws-dashboard build)")]
    Dashboard,
    #[command(about = "Sync pending activity aggregates to wshub")]
    Sync {
        #[arg(long, help = "Sync only this project")]
        project: Option<String>,
        #[arg(long, help = "Ignore backoff and force a sync pass")]
        force: bool,
        #[command(subcommand)]
        action: Option<ActivitySyncAction>,
    },
    #[command(about = "Export raw local activity events")]
    Export {
        #[arg(long, help = "Export raw events (required)")]
        raw: bool,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        csv: bool,
    },
    #[command(about = "Prune old raw events")]
    Prune {
        #[arg(long, help = "Delete raw events before this date")]
        before: Option<String>,
        #[arg(long, help = "Show what would be deleted")]
        dry_run: bool,
    },
    #[command(
        about = "Recompute activity buckets/spans from raw events (fixes overlap double-counting)"
    )]
    Recompute {
        #[arg(long, default_value = "today", help = "Range to recompute: today")]
        range: String,
        #[arg(long, help = "Show what would change without writing")]
        dry_run: bool,
        #[arg(long, help = "Proceed even if the daemon appears to be running")]
        force: bool,
    },
    #[command(about = "Install or uninstall shell command hooks")]
    Shell {
        #[command(subcommand)]
        action: ActivityShellAction,
    },
    #[command(about = "Enable local activity collection")]
    Enable {
        #[arg(long)]
        yes: bool,
    },
    #[command(about = "Disable local activity collection")]
    Disable,
    #[command(about = "Pause local activity collection")]
    Pause,
    #[command(about = "Resume local activity collection")]
    Resume,
    #[command(about = "Delete local activity data")]
    Clear {
        #[arg(long)]
        yes: bool,
    },
    #[command(about = "Manage this device identity")]
    Device {
        #[command(subcommand)]
        action: ActivityDeviceAction,
    },
    #[command(about = "Inspect and migrate legacy activity project IDs")]
    Ids {
        #[command(subcommand)]
        action: ActivityIdsAction,
    },
    #[command(
        about = "Record a shell command event (used by shell hooks)",
        hide = true
    )]
    Record {
        #[arg(long)]
        cmd: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        exit: Option<i64>,
        #[arg(long)]
        dur: Option<i64>,
        #[arg(long)]
        ts: Option<i64>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ActivitySyncAction {
    #[command(about = "Enable hub sync for a project or all projects")]
    Enable {
        #[arg(long)]
        project: Option<String>,
    },
    #[command(about = "Disable hub sync for a project or all projects")]
    Disable {
        #[arg(long)]
        project: Option<String>,
    },
    #[command(about = "Delete synced activity history from wshub for this user")]
    Purge {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ActivityShellAction {
    #[command(about = "Install shell hooks for command tracking")]
    Install {
        #[arg(long, help = "Shell: bash, zsh, fish (default: auto-detect)")]
        shell: Option<String>,
        #[arg(long, help = "Target rc/config file path")]
        file: Option<String>,
    },
    #[command(about = "Uninstall shell hooks")]
    Uninstall {
        #[arg(long)]
        shell: Option<String>,
        #[arg(long)]
        file: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ActivityPresenceAction {
    #[command(about = "Set manual presence state (for hooks/testing)")]
    Set {
        #[arg(help = "State: active, idle, away, locked, asleep")]
        state: String,
    },
    #[command(about = "Set or clear a work/leisure/neutral category override")]
    Category {
        #[arg(help = "Category: work, leisure, neutral, auto")]
        category: String,
        #[arg(long, help = "Duration (e.g. 30m, 1h, 2h). Omit for until cleared")]
        r#for: Option<String>,
        #[arg(long, help = "Apply to current focus interval only (default: global)")]
        current: bool,
    },
    #[command(about = "Explain how the current focused window is classified")]
    Explain {
        #[arg(long, help = "Output JSON")]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ActivityDeviceAction {
    #[command(about = "Rename this device as shown in reports and wshub")]
    Rename {
        #[arg(help = "New device name")]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ActivityIdsAction {
    #[command(about = "List canonical and legacy project IDs found in local activity data")]
    List,
    #[command(about = "Migrate legacy path:<dir> IDs to canonical activity.project_id values")]
    Migrate {
        #[arg(long, help = "Show what would change without modifying data")]
        dry_run: bool,
        #[arg(
            long,
            help = "Keep legacy aggregates instead of rebuilding from raw events"
        )]
        keep_aggregates: bool,
        #[arg(long, help = "Skip confirmation prompt")]
        yes: bool,
    },
}
