# 💻 CLI Reference Manual

`ws` provides a clean, intentional command hierarchy organized across these
domains:

1. [**Workspace Lifecycle**](#1-workspace-lifecycle) (`create`, `list`, `info`,
   `end`, `close`, `status`, `exec`, `push`, `pull`)
2. [**Worktree & Repository Operations**](#2-worktree--repository-operations)
   (`repo add`, `repo remove`, `repo lock`, `repo unlock`, `lock`, `unlock`)
3. [**Service Runtime & Multiplexers**](#3-service-runtime--multiplexers)
   (`start`, `attach`, `stop`, `restart`, `logs`, `bridge`)
4. [**Developer Shell & Environment**](#4-developer-shell--environment)
   (`shell`, `env`, `setup`)
5. [**Project Store Management**](#5-project-store-management) (`project init`,
   `project add`, `project fetch`, `project sync`, `doctor`, `antigravity`)
6. [**wshub Cloud Collaboration & Replication**](#6-wshub-cloud-collaboration--replication)
   (`hub`, `clone`, `service`, `daemon`)
7. [**Universal Inverted Syntax**](#7-universal-inverted-syntax)
   (`ws @<name> <command>`)

---

## 🏷️ Sigil Conventions

To ensure absolute clarity in command arguments:

- **Workspaces** use the **`@`** prefix: `@develop`, `@feat-auth`,
  `@hotfix-401`.
- **Repositories & Services** use the **`%`** prefix: `%server`, `%mobile`,
  `%frontend`.

The prefixes are mandatory for both positional arguments and flag values.
Bare names are rejected with a hint, and legacy repository prefixes (`+`, `:`,
`#`, `$`) are no longer accepted by the CLI. Internal workspace directories and
`workspace.yml` names remain bare (for example `workspaces/feat-auth`).

---

## 1. Workspace Lifecycle

### `ws create` / `ws new`

Creates a new workspace containing Git worktrees for specified repositories.

```bash
ws create @<name> [%repo[:branch[:mode]] ...] [--all] [--existing] [-f <file.yml>] [--setup] [--cmd <command>] [--no-tmux] [--base <branch>] [--yes]
```

#### Arguments & Options

| Argument / Flag            | Type       | Description                                                                                   |
| :------------------------- | :--------- | :-------------------------------------------------------------------------------------------- |
| `@<name>`                  | Positional | Target workspace name (required unless `-f` is used).                                         |
| `%<repo>[:branch[:mode]]`  | Positional | Repository name with optional target branch and branch creation mode (`new` or `existing`).   |
| `--all`                    | Flag       | Include all repositories defined in `ws-project.yml`.                                       |
| `--existing`               | Flag       | Default to checking out existing branches rather than creating new `feature/<name>` branches. For `:existing`/`--existing`, `ws` fetches the branch from the primary remote (preferring `origin`) if needed and creates a local branch ref when the branch exists only remotely. |
| `--base <branch>`          | Option     | One-shot base branch for newly created branches (`--target` / `--from` are aliases). Overrides `base_branch` from `ws-project.yml` for this command only; not persisted. |
| `-f`, `--file <file.yml>`  | Option     | Path to a declarative workspace YAML configuration file. Every `create: true` entry must include a base (`base`/`target`/`from`/`base_branch`); no prompt or fallback is used. |
| `--setup`                  | Flag       | Run the default lifecycle flow after creation (`configure` + all setup stages).                |
| `--cmd`, `--command <cmd>` | Option     | Command to run inside the workspace Tmux window upon creation (overrides config default).     |
| `--no-tmux`                | Flag       | Skip opening a dedicated Tmux window for this workspace.                                      |
| `--yes`                    | Flag       | Non-interactive: accept each repository's detected default base branch and persist it, instead of prompting. |

#### Examples

```bash
# Create workspace with feature/auth branches across server and mobile:
ws create @feat-auth %server %mobile

# Create workspace and launch custom editor inside Tmux window:
ws create @feat-auth %server %mobile --cmd "nvim"

# Checkout existing develop branch for server, create new feature/auth branch for mobile:
ws create @feat-auth %server:develop:existing %mobile:feature/auth:new

# Create workspace with all project repositories on existing branches:
ws create @prod-repro --all --existing

# Create workspace from a declarative YAML spec and run setup scripts:
ws create -f team-setup.yml --setup

# Create new branches from develop without changing ws-project.yml:
ws create @feat-auth %server %mobile --base develop

# Non-interactive: accept and persist each repo's detected default base:
ws create @feat-auth %server %mobile --yes
```

---

### `ws list` / `ws ls`

Lists all existing workspaces with their active branch mappings, creation
timestamps, and filesystem paths.

```bash
ws list
# or
ws ls
```

---

### `ws info`

Displays detailed inspection information for a workspace, including worktree
paths, branch names, file lock status, active presentation engine, and live
process statuses with listening ports.

```bash
ws info @<name>
```

#### Example

```bash
ws info @develop
```

---

### `ws focus` / `ws switch`

Focuses or switches to the workspace's Tmux window. If the window is not yet
open in the Tmux session, `ws focus` automatically spawns it in the workspace
directory with the configured command (e.g. `nvim`). If run inside Tmux, selects
and switches client to that window. If run outside Tmux, attaches to the project
session focused on that window.

```bash
ws focus @<name>
# or
ws switch @<name>
```

#### Example

```bash
ws focus @feat-auth
```

---

### `ws end` / `ws close`

Safely ends and closes a workspace. Before pruning Git worktrees and removing
the workspace directory, `ws end` verifies that:

1. **All work has been committed**: Checks for uncommitted changes (staged,
   unstaged, and untracked files). Prevents closing if uncommitted work is
   found.
2. **All work has been merged**: Checks if feature branches have been merged
   into their base branch (defaulting to the bare repo default branch, e.g.
   `main` or `master`). Prevents closing if unmerged commits exist.
3. **Session successfully stopped**: If services are running under the
   background daemon, ensures the session is cleanly terminated before files are
   deleted.
4. **Tmux window closed**: If configured, cleanly terminates the workspace's
   Tmux window.

```bash
ws end @<name> [--no-merge] [-f|--force] [--delete-branch] [-t|--target <branch>] [--no-tmux]
# or alias
ws close @<name> [--no-merge] [-f|--force] [--delete-branch] [-t|--target <branch>] [--no-tmux]
```

#### Options

| Flag                                         | Description                                                                                                                                 |
| :------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------ |
| `--no-merge`                                 | Allow closing even if committed changes have not been merged into the base branch. _(Note: `--no-merge` does NOT bypass uncommitted work!)_ |
| `-f`, `--force`                              | Force close regardless of uncommitted changes, unmerged branches, or active sessions.                                                       |
| `--delete-branch`                            | Also delete the Git feature branch from the bare repository store.                                                                          |
| `-t`, `--target`, `--target-branch <branch>` | Base branch to check merge status against (default: repo default branch, e.g. `main` or `master`).                                          |
| `--no-tmux`                                  | Skip closing the workspace's Tmux window.                                                                                                   |

#### Examples

```bash
# Safely end workspace (verifies all work committed and merged, stops active daemon):
ws end @feat-auth

# Close workspace even if branch is not yet merged:
ws end @feat-auth --no-merge

# Check merge status against develop branch instead of main:
ws end @feat-auth --target develop

# Safely close and delete feature branch from bare repository:
ws end @feat-auth --delete-branch

# Force close discarding any uncommitted changes:
ws end @feat-auth --force
```

---

### `ws status`

Runs `git status` across all repository worktrees in the workspace and renders a
combined overview table highlighting uncommitted changes, untracked files, and
branch divergence.

```bash
ws status @<name>
```

#### Example

```bash
ws status @develop
```

---

### `ws exec`

Executes an arbitrary shell command across all or a subset of repository
worktrees inside the workspace. The command must be separated from `ws`
arguments with `--`.

```bash
ws exec [@<name>] [%repos...] [--all] -- <command...>
```

#### Options

| Argument / Flag | Default | Description                                             |
| :-------------- | :------ | :------------------------------------------------------ |
| `@<name>`       | Current | Workspace name (defaults to active detected workspace). |
| `%repos...`     | All     | Specific repositories to execute in (`%hub %web`).      |
| `--all`         | All     | Explicitly target all repositories in the workspace.    |
| `--`            | -       | Required delimiter separating `ws` arguments from the child command. |

`--all` cannot be combined with explicit `%repo` targets.

#### Examples

```bash
# Run command across all repos in current workspace:
ws exec -- git status

# Run command across a subset of repos:
ws exec %hub %web -- ga .

# Run command across all repos in @develop:
ws exec @develop -- git clean -fd

# Run command across a subset of repos in a specific workspace:
ws exec @feat-auth %server %mobile -- npm test
```

---

### `ws push`

Pushes committed changes across workspace repositories to their upstream Git
remotes. Skips locked/frozen repositories automatically.

```bash
ws push @<name> [%repos...] [--repos %r1,%r2] [--remote <name>]
```

#### Options

| Argument / Flag   | Default  | Description                                              |
| :---------------- | :------- | :------------------------------------------------------- |
| `%repos...`       | All      | Specific repositories to push.                           |
| `--repos <list>`  | -        | Comma-separated repositories; each value requires `%`.   |
| `--remote <name>` | `origin` | Target Git remote name.                                  |

#### Example

```bash
ws push @feat-auth %server %mobile --remote origin
```

---

### `ws pull`

Pulls remote updates for workspace repositories. Automatically reports
conflicts, dirty worktree states, and network errors in formatted diagnostic
tables.

```bash
ws pull @<name> [%repos...] [--repos %r1,%r2] [--remote <name>]
```

#### Example

```bash
ws pull @develop %server
```

---

## 2. Worktree & Repository Operations

### `ws repo add`

Adds a new repository worktree to an existing workspace.

```bash
ws repo add @<workspace> %<repo>[:branch] [--existing] [--base <branch>] [--yes]
```

#### Options

| Argument / Flag       | Type       | Description                                                                                     |
| :-------------------- | :--------- | :---------------------------------------------------------------------------------------------- |
| `@<workspace>`        | Positional | Target workspace (required).                                                                    |
| `%<repo>[:branch]`    | Positional | Repository name and optional branch.                                                            |
| `--existing`          | Flag       | Check out an existing branch instead of creating a new one.                                     |
| `--base <branch>`     | Option     | One-shot base branch when creating a new branch (`--target` / `--from` aliases). Not persisted. |
| `--yes`               | Flag       | Non-interactive: accept the detected default base and persist it instead of prompting.          |

#### Examples

```bash
# Add mobile repo with a new feature branch:
ws repo add @develop %mobile:feature/auth-screen

# Add server repo checking out existing main:
ws repo add @develop %server:main --existing

# Create a new branch from develop just this once (ws-project.yml unchanged):
ws repo add @develop %server:feature/api --base develop
```

---

### `ws repo remove` / `ws repo rm`

Removes a repository worktree from an existing workspace and cleans up its Git
worktree metadata.

```bash
ws repo remove @<workspace> %<repo> [--delete-branch]
```

#### Options

| Option            | Description                                                |
| :---------------- | :--------------------------------------------------------- |
| `--delete-branch` | Also delete the Git branch from the bare repository store. |

#### Example

```bash
ws repo remove @develop %mobile --delete-branch
```

---

### `ws repo lock` / `ws lock`

Locks a repository worktree by setting all Git-tracked files to read-only
(`chmod a-w`). Keeps untracked build artifacts and environment files (`.env`,
`node_modules/`, `target/`) writable.

```bash
ws repo lock @<workspace> %<repo>
# or top-level shortcut:
ws lock @<workspace> %<repo>
```

#### Example

```bash
ws lock @automatic-401-logout %server
```

---

### `ws repo unlock` / `ws unlock`

Unlocks a previously locked repository worktree, restoring standard write
permissions (`chmod u+w`) to tracked files.

```bash
ws repo unlock @<workspace> %<repo>
# or top-level shortcut:
ws unlock @<workspace> %<repo>
```

#### Example

```bash
ws unlock @automatic-401-logout %server
```

---

## 3. Service Runtime & Multiplexers

### `ws start` / `ws launch` / `ws run`

Starts workspace services concurrently under a persistent background supervisor
daemon and displays the requested presentation interface.

```bash
ws start @<name> [%repos...] [--all] [--repos %r1,%r2] [--tmux] [-z|--zellij] [-t|--terminal] [--stream] [-d|--daemon] [--attach %repo] [--switch] [--mode <mode>] [--interface <iface>] [--ip <ip>]
```

#### Options

| Flag                                        | Description                                                                                                                   |
| :------------------------------------------ | :---------------------------------------------------------------------------------------------------------------------------- |
| `%repos...`                                 | Specific services to start (starts all configured services if omitted or `--all` is passed).                                  |
| `--all`                                     | Explicitly target every repository. Conflicts with explicit `%repo` / `--repos` targets.                                      |
| `--repos %r1,%r2`                           | Comma-separated services to start; each value requires `%`.                                                                   |
| `--tmux`                                    | Launch services in a Tmux session with side-by-side vertical panes.                                                           |
| `-z`, `--zellij`                            | Launch services in a Zellij tiled split grid session.                                                                         |
| `-t`, `--terminal`                          | Launch each service in a separate native OS terminal window.                                                                  |
| `--stream`                                  | Stream raw multiplexed stdout/stderr directly to terminal without interactive TUI.                                            |
| `-d`, `--daemon`                            | Launch services detached in background daemon without opening a UI.                                                           |
| `--attach %repo`                            | Launch and immediately focus single service terminal output.                                                                  |
| `-s`, `--switch`                            | Migrate presentation engine on-the-fly with **zero downtime**.                                                                |
| `-m`, `--mode <mode>`                       | Presentation engine mode: `tui`, `tmux`, `zellij`, `terminal`, `stream`, `daemon`, `attach`.                                  |
| `--interface`, `--iface`, `--lan-interface` | Select network interface (`wlan0`, `eno1`) or type (`wifi`, `ethernet`) to resolve `${LAN_IP}`. Prioritizes Wi-Fi by default. |
| `--ip`, `--lan-ip`                          | Explicit host LAN IP address override (e.g. `192.168.24.178`).                                                                |

#### Examples

```bash
# Start in interactive Rust TUI (prioritizes Wi-Fi adapter for LAN IP by default):
ws start @develop

# Start with explicit Ethernet interface override:
ws start @develop --interface eno1

# Start with explicit IP address override:
ws start @develop --ip 192.168.1.55

# Start in Tmux side-by-side vertical panes:
ws start @develop --tmux

# Start in Zellij:
ws start @develop -z

# Start detached in background daemon:
ws start @develop -d
```

---

### `ws attach`

Connects to a running workspace daemon session.

```bash
ws attach @<name> [%service] [--all] [--tmux] [-z|--zellij] [--switch] [--mode <mode>]
```

#### Options

| Flag             | Description                                                    |
| :--------------- | :------------------------------------------------------------- |
| `%service`       | Service name to focus directly.                                |
| `--all`          | Attach in multi-pane grid view.                                |
| `--tmux`         | Attach using Tmux presentation backend.                        |
| `-z`, `--zellij` | Attach using Zellij presentation backend.                      |
| `-s`, `--switch` | Migrate presentation engine on-the-fly with **zero downtime**. |

#### Examples

```bash
# Attach in interactive TUI:
ws attach @develop

# Attach in Tmux with zero downtime switch:
ws attach @develop --tmux --switch

# Attach directly to mobile service logs:
ws attach @develop %mobile
```

---

### `ws stop` / `ws kill`

Gracefully terminates all running services and shuts down the workspace
background daemon.

```bash
ws stop @<name>
```

#### Example

```bash
ws stop @develop
```

---

### `ws restart`

Restarts running services inside an active workspace session without terminating
the daemon.

```bash
ws restart @<name> [%repos...]
```

#### Example

```bash
ws restart @develop %server
```

---

### `ws logs`

Views or tails persistent log files for workspace services stored in
`.ws/logs/`.

```bash
ws logs @<name> [%repo] [-f|--follow] [-n <lines>]
```

#### Options

| Option           | Default | Description                              |
| :--------------- | :------ | :--------------------------------------- |
| `-f`, `--follow` | `false` | Follow live log output (tail -f).        |
| `-n`, `--lines`  | `50`    | Number of previous log lines to display. |

#### Example

```bash
ws logs @develop %server -f -n 100
```

---

### `ws bridge`

Connects a raw terminal I/O bridge directly into a running service's Master PTY
inside the daemon.

```bash
ws bridge @<name> %<repo>
```

#### Example

```bash
ws bridge @develop %mobile
```

---

## 4. Developer Shell & Environment

### `ws shell` / `ws enter` / `ws open`

Spawns an interactive subshell configured specifically for the target workspace
or worktree.

```bash
ws shell @<name> [%worktree]
```

#### Example

```bash
ws shell @develop %server
```

---

### `ws env`

Inspects or synchronizes environment variables configured for workspace
repositories.

```bash
ws env @<name> [%repo] [--sync] [--interface <iface>] [--ip <ip>]
```

#### Options

| Flag                                        | Description                                                                                                                   |
| :------------------------------------------ | :---------------------------------------------------------------------------------------------------------------------------- |
| `--sync`                                    | Write resolved environment variables into worktree `.env` files.                                                              |
| `--interface`, `--iface`, `--lan-interface` | Select network interface (`wlan0`, `eno1`) or type (`wifi`, `ethernet`) to resolve `${LAN_IP}`. Prioritizes Wi-Fi by default. |
| `--ip`, `--lan-ip`                          | Explicit host LAN IP address override (e.g. `192.168.24.178`).                                                                |

#### Examples

```bash
# Inspect environment variables (uses default Wi-Fi priority):
ws env @develop %mobile

# Inspect with Ethernet interface override:
ws env @develop %mobile --interface eno1

# Sync variables into .env files:
ws env @develop --sync
```

---

### `ws setup`

Runs the workspace lifecycle default flow (configure + all setup stages) as
defined in `ws-project.yml` (schema v3).

```bash
ws setup @<name> [%repos...] [--all] [--dry-run] [--skip-scripts] [--interface <iface>] [--ip <ip>]
```

- Repositories are selected positionally as `%repo`; `--all` is the only
  repo-selection flag and conflicts with explicit `%repo` targets.
- `--all` (or default full-workspace setup) runs the default lifecycle flow:
  `configure` then all setup stages in order (including repo-only stages).
- `configure` is the built-in stage that copies files and syncs env/templates;
  named setup stages (e.g. `install`, `build`, `migrate`, `verify`) run
  project-defined hooks in order.
- `--skip-scripts` skips hook execution but still performs built-in
  configure/sync behavior where applicable.
- `--dry-run` prints planned actions/commands without executing hooks.

#### Examples

```bash
# Run default lifecycle flow for every repository (configure + all setup stages):
ws setup @develop --all

# Run setup for selected repositories only:
ws setup @develop %server %mobile

# Dry-run setup flow:
ws setup @develop --all --dry-run
```

### `ws lifecycle`

Inspect or run lifecycle stages for a workspace.

```bash
ws lifecycle [@<name>] [%repo ...] run [--all] [+stage ...] [--force] [--dry-run] [--skip-configure] [--interface <iface>] [--ip <ip>]
ws lifecycle [@<name>] [%repo ...] status [+stage ...]
```

The arguments are positional and must appear in this order: optional
workspace (`@name`), zero or more repositories (`%repo`), the required action
(`run` or `status`), then zero or more stages (`+stage`). Stage names require
the `+` prefix; omitting it is a usage error.

- **Workspace inference:** when `@<name>` is omitted, the workspace is detected
  from the current directory. If no workspace can be detected, pass
  `@<workspace>`.
- **Repository inference:** when `@<name>`, `%repo`, and `--all` are all
  omitted, the repository is inferred from the current directory and restricts
  the run to that repository. Workspace-scope hooks still run.
- `--all` targets every repository, suppresses repository inference, and
  conflicts with explicit `%repo` arguments.
- **Stages:** `+configure` is the built-in configure stage. `+<name>` maps to a
  defined setup stage (for example `+install` or `+build`). Event stages use
  their lifecycle names: `+pre_launch`, `+post_launch`, `+pre_stop`,
  `+post_stop`, `+pre_end`, `+post_end`, `+repo_add`, `+repo_remove`. Unknown
  stage names are hard errors that list the known stages.
- **No stages:** `run` executes `configure` followed by every setup stage in
  root order and then repo-only setup stages.
- **Configure auto-run:** explicit setup stages automatically run `configure`
  first unless `--skip-configure` is passed. An explicit `+configure` always
  runs, even with `--skip-configure`. Repeated `+stage` tokens run each time
  they appear.
- `status` accepts only `%repo` and `+stage` filters; run-only flags and
  `--all` are rejected.
- `ws start` fails if required setup stages are incomplete (stages whose hooks
  are all `always` do not block), with a hint to run `ws lifecycle @<name> run`.

#### Examples

```bash
ws lifecycle @develop status
ws lifecycle @develop status +install
ws lifecycle @develop run
ws lifecycle @develop run +install
ws lifecycle @develop %server run +configure +build
ws lifecycle @develop %server %mobile status
ws lifecycle @develop run +install --skip-configure
ws lifecycle %api run +post_launch   # workspace and repo inferred from cwd
```

---

## 5. Project Store Management

> **Lifecycle schema v3:** Legacy or v1 lifecycle configurations must be
> migrated before normal use.
>
> ```bash
> ws project migrate lifecycle --yes
> ```
>
> Migration is comment-preserving and supports `--dry-run`, `--no-backup`,
> `--stage-name <name>`, `--source auto|legacy|v1`, `--no-state`, and
> `--reset-state`.

### `ws project init` / `ws init`

Initializes bare Git repositories in `bares/` and generates `ws-project.yml`
with sensible defaults for `tmux` (`session` set to project folder,
`command: nvim`, `switch: true`) and `hub` (`auto_save` every 5m). Existing
configurations are preserved if already present.

```bash
ws project init [alias=URL ...] [--yes]
# Or top-level shortcut:
ws init [alias=URL ...] [--yes]
```

#### Example

```bash
ws project init server=git@github.com:org/api.git mobile=git@github.com:org/app.git
```

With an interactive terminal, `ws` prompts for a `base_branch` per repository,
prefilled with the detected default. `--yes` skips the prompt, uses the detected
default, and persists it.

---

### `ws project add` / `ws add`

Clones an additional bare repository into `bares/` and registers it in
`ws-project.yml`.

```bash
ws project add <alias=URL> [--yes]
# Or top-level shortcut:
ws add <alias=URL> [--yes]
```

#### Example

```bash
ws project add frontend=git@github.com:org/web.git
```

When the repository has no `base_branch`, `ws` prompts with the detected
default (or accepts it non-interactively with `--yes`) and saves the value.

---

### `ws project fetch` / `ws fetch`

Fetches remote branches and tags across all bare repositories in `bares/`.

```bash
ws project fetch
```

---

### `ws project sync` / `ws sync`

Runs `git worktree prune` across all bare repositories and cleans up stale
references.

```bash
ws project sync
```

---

### `ws project migrate lifecycle`

Migrates legacy or v1 lifecycle configuration to schema v3 (`schema_version: 3`)
with a comment-preserving YAML rewrite.

```bash
ws project migrate lifecycle [--dry-run] [--yes] [--no-backup] [--stage-name <name>] [--source auto|legacy|v1] [--no-state] [--reset-state]
```

#### Options

| Flag                 | Description                                                                        |
| :------------------- | :--------------------------------------------------------------------------------- |
| `--dry-run`          | Preview migration changes without writing files.                                   |
| `--yes`              | Apply migration without prompting.                                                 |
| `--no-backup`        | Do not write a `.bak` backup before applying migration.                            |
| `--stage-name <name>`| Stage name for migrated setup scripts (default: `install`).                        |
| `--source <mode>`    | Migration source preference: `auto`, `legacy`, or `v1`.                            |
| `--no-state`         | Migrate config only; skip workspace lifecycle state migration.                     |
| `--reset-state`      | Reset workspace lifecycle state instead of migrating it.                           |

---

### `ws project migrate config`

Renames a legacy `repositories.yml` project configuration to `ws-project.yml`.

```bash
ws project migrate config [--dry-run]
```

#### Options

| Flag        | Description                                |
| :---------- | :----------------------------------------- |
| `--dry-run` | Preview the rename without changing files. |

---

### `ws doctor`

Runs an automated system diagnostic check verifying Git version, multiplexer
availability (`tmux`, `zellij`), bare repository integrity, socket permissions,
and environment health.

```bash
ws doctor
```

---

### `ws completion`

Generates or installs shell autocompletion scripts for Zsh, Bash, and Fish,
providing tab-completion for subcommands, workspaces (`@<name>`), repositories
(`%<repo>`), and flags.

```bash
ws completion [zsh|bash|fish|install]
```

#### Examples

```bash
# Evaluate Zsh completions directly in current shell:
eval "$(ws completion zsh)"

# Install completions permanently into ~/.zsh/completions/_ws:
ws completion install

# Generate Bash completion script:
ws completion bash
```

---

---

## 6. wshub Cloud Collaboration & Replication

### `ws hub login`

Authenticates with the `wshub` server and saves an encrypted session token in
`~/.config/ws/hub.yml`.

```bash
ws hub login [--url <url>] [--token <pat_token>] [-u <username>] [-p <password>]
```

### `ws hub whoami`

Displays the currently authenticated wshub user profile and active server.

```bash
ws hub whoami
```

### `ws hub clone` / `ws clone`

Clones project blueprint, downloads secret files from the vault, and clones all
bare repositories in one command.

```bash
ws clone <org/project> [target_dir]
ws hub clone <org/project> [target_dir]
```

### `ws hub publish`

Publishes a local workspace project blueprint (`ws-project.yml`) and
automation scripts to wshub.

```bash
ws hub publish [<org/project>] [-d <description>]
```

### `ws hub push`

Pushes local project configuration updates as a new linear blueprint revision.

```bash
ws hub push [-m <changelog_message>]
```

### `ws hub pull`

Pulls the latest blueprint from wshub and clones any newly added bare
repositories.

```bash
ws hub pull
```

### `ws hub sync`

Synchronizes project blueprint, clones missing bare repositories, and updates
vault secrets/files in one step.

```bash
ws hub sync
```

### `ws hub state save` & `ws hub resume`

Snapshots active workspace branches, locks, and local configuration for
cross-machine resumption.

```bash
# Save workspace state to hub
ws hub state save @develop

# Re-hydrate exact workspace state on another machine
ws hub resume @develop
```

### `ws service` & `ws daemon`

Manages the machine-wide `ws.service` user systemd unit which continuously
monitors all registered projects on your machine and performs periodic Hub
auto-saving.

```bash
# Install, enable, and start ws.service (runs on boot)
ws service install

# Check status of ws.service and see all monitored projects
ws service status

# Start, stop, or restart the background service
ws service start
ws service stop
ws service restart

# Stream live systemd journal logs
ws service logs

# Run the global daemon in foreground (used by ws.service)
# Includes real-time configuration file watcher for ws-project.yml and workspace.yml
ws daemon run [--tick 15]
```

The daemon automatically watches `ws-project.yml` and `workspace.yml` in real
time, auto-pushing blueprint revisions on `ws-project.yml` edits and
auto-saving workspace states on `workspace.yml` edits. Projects not yet
published on `wshub` are automatically published on first save or push.

### `ws project`

Manages project bare repositories and the global project registry for the
background daemon.

```bash
# List all registered projects monitored by ws.service
ws project list

# Register a project directory in the global registry
ws project register [/path/to/project]

# Unregister a project directory
ws project unregister [/path/to/project]
```

### `ws hub auto-save`

Controls periodic auto-saving of workspace states to `wshub`. Works with
change-detection caching to skip redundant uploads when worktrees are unchanged.

```bash
# Check daemon status, interval, and workspace snapshot history
ws hub auto-save status

# Trigger a one-time auto-save pass across configured workspaces
ws hub auto-save once [--force]

# Manage systemd service via alias
ws hub auto-save service install
ws hub auto-save service status
```

### `ws hub secret`

Manages zero-Git encrypted project secrets and sensitive files (`.pem`, `.json`,
certificates) in the wshub AES-256-GCM vault.

```bash
# List secrets
ws hub secret list

# Set a secret key
ws hub secret set <KEY> <VALUE> [--repo %<repo>]

# Get a secret value
ws hub secret get <KEY> [--repo %<repo>]

# Upload a sensitive file
ws hub secret upload <file_path>

# Pull and decrypt files to local files/ directory
ws hub secret pull
```

---


---

## 8. Activity Tracking

Tracks local development activity (file changes, terminal commands via shell hooks, heartbeats, and git events where available), derives active-time sessions/buckets, and can sync hourly aggregates to `wshub`.

- **Local storage**: global SQLite DB at `~/.local/share/ws/activity.db` (WAL) and config at `~/.config/ws/activity.yml`.
- **Privacy**: sync sends aggregates only (time/counts/project/repo/workspace/device). No file paths, command names, command arguments, or file contents are synced. Command events are recorded locally as command names only (no arguments).
- **Project config** (`ws-project.yml`): `activity.project_id`, `activity.sync`, `activity.hub_project` (optional mapping/overrides).

```bash
# Status and control
ws activity status [--json] [--waybar]
ws activity enable [--yes]
ws activity disable
ws activity pause
ws activity resume
ws activity clear [--yes]

# Reports, series, snapshot, and export
ws activity report [--day|--week|--month] [--from <date>] [--to <date>] [--project <name|id>] [--repo <name>] [--workspace <name>] [--json|--csv]
ws activity series [--day|--week|--month] [--from <date>] [--to <date>] [--step hour|day] [--project <name|id>] [--repo <name>] [--workspace <name>] [--group-by none|project] [--json]
ws activity snapshot [--range today|week|month] [--project <name|id>] [--json]
ws activity export --raw [--from <date>] [--to <date>] [--json|--csv]
ws activity prune [--before <date>] [--dry-run]

# Shell command hooks (opt-in)
ws activity shell install [--shell bash|zsh|fish] [--file <path>]
ws activity shell uninstall [--shell bash|zsh|fish] [--file <path>]

# Hub sync
ws activity sync [--project <name|id>] [--force]
ws activity sync enable|disable [--project <name|id>]
ws activity sync purge --yes [--project <name|id>]

# Device identity
ws activity device rename <name>

# Dashboard
ws activity dashboard
```

### Waybar Module

`ws activity status --waybar` emits compact Waybar JSON (`text`, `alt`, `tooltip`, `class`, `percentage`). The module is intended as the always-visible trigger in Waybar; a Quickshell overlay can show a rich hover panel.

Example module (`~/.config/waybar/modules/custom-ws.jsonc`):

```jsonc
{
  "custom/ws": {
    "exec": "ws activity status --waybar",
    "return-type": "json",
    "format": "{text}",
    "interval": 60,
    "signal": 10,
    "tooltip": false,
    "on-click": "ws activity sync --force; pkill -RTMIN+10 waybar"
  }
}
```

Place `custom/ws` first in `modules-left` and give it a fixed slot so Quickshell can align a transparent hover catcher over it. The reference geometry is defined in `~/.config/waybar/user-style.css` and mirrored in `~/.config/quickshell/ws-activity/Geometry.qml` (`x=12 y=6 w=120 h=26`, bar height `38`).

Style classes: `active`, `paused`, `disabled`, `stale`, `sync-error`, `watch-limited`, `synced`.

### Quickshell Hover Panel

`ws activity snapshot --range today|week|month [--project <name|id>] --json` returns a dense, local-time-aligned snapshot for the popup: status, totals, series, project breakdown, and repo breakdown. Series points are 24 hourly slots for `today`, 7 local-day slots for `week`, and 30 local-day slots for `month`.

The Quickshell config lives at `~/.config/quickshell/ws-activity/`. It creates a transparent overlay `PanelWindow` over the Waybar trigger, uses an input mask so only the module slot captures pointer events, and opens a `PopupWindow` with charts/metrics on hover.

Setup:

```bash
sudo pacman -S quickshell
quickshell -c ws-activity
```

Hyprland autostart (`~/.config/hypr/userprefs.conf`):

```ini
exec-once = quickshell -c ws-activity
```

The `dashboard` command still launches the optional GTK `ws-dashboard` (`cargo build -p ws-dashboard`; install/copy it next to `ws` or on `PATH`).

> Some advanced collection/transport behaviors (full inotify/git-metadata, unix socket transport, and complete hub linkage/blueprint-push automation) may be partially implemented depending on version; local rollup/buckets and baseline sync are supported.

## 7. Universal Inverted Syntax

All workspace-scoped commands support the intuitive **`ws @<name> <verb>`**
inverted syntax. The `@` and `%` prefixes remain mandatory:

```bash
ws @develop start --tmux
ws @develop shell %mobile
ws @develop status
ws @develop info
ws @develop lock %server
ws @develop unlock %server
ws @develop restart %server
ws @develop logs %server -f
ws @develop push
ws @develop pull
ws @develop stop
```


### ID Migration

Reconcile legacy `path:<dir>` project IDs with canonical `activity.project_id` values.

```bash
# Inspect canonical vs legacy IDs
ws activity ids list

# Dry run
ws activity ids migrate --dry-run

# Rebuild buckets from raw events (recommended when raw history is complete)
ws activity ids migrate --yes

# Keep/merge existing aggregates instead of rebuilding (approximate when raw history is incomplete)
ws activity ids migrate --keep-aggregates --yes
```

- Rebuild mode deletes affected buckets/rollup state and rerolls from raw events; it refuses if legacy buckets exist without raw events.
- A best-effort DB backup (`activity.db.bak-*`) is created before migration.
