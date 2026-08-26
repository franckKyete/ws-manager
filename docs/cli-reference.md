# 💻 CLI Reference Manual

`ws` provides a clean, intentional command hierarchy organized across 5 core domains:

1. [**Workspace Lifecycle**](#1-workspace-lifecycle) (`create`, `list`, `info`, `delete`, `status`, `exec`, `push`, `pull`)
2. [**Worktree & Repository Operations**](#2-worktree--repository-operations) (`repo add`, `repo remove`, `repo lock`, `repo unlock`, `lock`, `unlock`)
3. [**Service Runtime & Multiplexers**](#3-service-runtime--multiplexers) (`start`, `attach`, `stop`, `restart`, `logs`, `bridge`)
4. [**Developer Shell & Environment**](#4-developer-shell--environment) (`shell`, `env`, `setup`)
5. [**Project Store Management**](#5-project-store-management) (`project init`, `project add`, `project fetch`, `project sync`, `doctor`, `antigravity`)
6. [**Universal Inverted Syntax**](#6-universal-inverted-syntax) (`ws @<name> <command>`)

---

## 🏷️ Sigil Conventions

To ensure absolute clarity in command arguments:

- **Workspaces** use the **`@`** prefix: `@develop`, `@feat-auth`, `@hotfix-401`.
- **Repositories & Services** use the **`%`** prefix: `%server`, `%mobile`, `%frontend`.

_(Note: The CLI is forgiving and also accepts un-prefixed names or `+repo`, `:repo`)_

---

## 1. Workspace Lifecycle

### `ws create` / `ws new`

Creates a new workspace containing Git worktrees for specified repositories.

```bash
ws create @<name> [%repo[:branch[:mode]] ...] [--all] [--existing] [-f <file.yml>] [--setup]
```

#### Arguments & Options

| Argument / Flag           | Type       | Description                                                                                   |
| :------------------------ | :--------- | :-------------------------------------------------------------------------------------------- |
| `@<name>`                 | Positional | Target workspace name (required unless `-f` is used).                                         |
| `%<repo>[:branch[:mode]]` | Positional | Repository name with optional target branch and branch creation mode (`new` or `existing`).   |
| `--all`                   | Flag       | Include all repositories defined in `repositories.yml`.                                       |
| `--existing`              | Flag       | Default to checking out existing branches rather than creating new `feature/<name>` branches. |
| `-f`, `--file <file.yml>` | Option     | Path to a declarative workspace YAML configuration file.                                      |
| `--setup`                 | Flag       | Automatically run setup scripts and environment sync immediately after workspace creation.    |

#### Examples

```bash
# Create workspace with feature/auth branches across server and mobile:
ws create @feat-auth %server %mobile

# Checkout existing develop branch for server, create new feature/auth branch for mobile:
ws create @feat-auth %server:develop:existing %mobile:feature/auth:new

# Create workspace with all project repositories on existing branches:
ws create @prod-repro --all --existing

# Create workspace from a declarative YAML spec and run setup scripts:
ws create -f team-setup.yml --setup
```

---

### `ws list` / `ws ls`

Lists all existing workspaces with their active branch mappings, creation timestamps, and filesystem paths.

```bash
ws list
# or
ws ls
```

---

### `ws info`

Displays detailed inspection information for a workspace, including worktree paths, branch names, file lock status, active presentation engine, and live process statuses with listening ports.

```bash
ws info @<name>
```

#### Example

```bash
ws info @develop
```

---

### `ws delete` / `ws rm` / `ws remove`

Safely terminates running background daemon processes, prunes all associated Git worktrees, and removes the workspace directory from disk.

```bash
ws delete @<name>
# or
ws rm @<name>
```

#### Example

```bash
ws delete @feat-auth
```

---

### `ws status`

Runs `git status` across all repository worktrees in the workspace and renders a combined overview table highlighting uncommitted changes, untracked files, and branch divergence.

```bash
ws status @<name>
```

#### Example

```bash
ws status @develop
```

---

### `ws exec`

Executes an arbitrary shell command across every repository worktree inside the workspace.

```bash
ws exec @<name> -- <command...>
```

#### Examples

```bash
# Run git clean across all repos in @develop:
ws exec @develop -- git clean -fd

# Run linter across all repos:
ws exec @feat-auth -- npm run lint
```

---

### `ws push`

Pushes committed changes across workspace repositories to their upstream Git remotes. Skips locked/frozen repositories automatically.

```bash
ws push @<name> [%repos...] [--repos r1,r2] [--remote <name>]
```

#### Options

| Argument / Flag   | Default  | Description                    |
| :---------------- | :------- | :----------------------------- |
| `%repos...`       | All      | Specific repositories to push. |
| `--remote <name>` | `origin` | Target Git remote name.        |

#### Example

```bash
ws push @feat-auth %server %mobile --remote origin
```

---

### `ws pull`

Pulls remote updates for workspace repositories. Automatically reports conflicts, dirty worktree states, and network errors in formatted diagnostic tables.

```bash
ws pull @<name> [%repos...] [--repos r1,r2] [--remote <name>]
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
ws repo add @<workspace> %<repo>[:branch] [--existing]
```

#### Examples

```bash
# Add mobile repo with a new feature branch:
ws repo add @develop %mobile:feature/auth-screen

# Add server repo checking out existing main:
ws repo add @develop %server:main --existing
```

---

### `ws repo remove` / `ws repo rm`

Removes a repository worktree from an existing workspace and cleans up its Git worktree metadata.

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

Locks a repository worktree by setting all Git-tracked files to read-only (`chmod a-w`). Keeps untracked build artifacts and environment files (`.env`, `node_modules/`, `target/`) writable.

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

Unlocks a previously locked repository worktree, restoring standard write permissions (`chmod u+w`) to tracked files.

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

Starts workspace services concurrently under a persistent background supervisor daemon and displays the requested presentation interface.

```bash
ws start @<name> [%repos...] [--all] [--tmux] [-z|--zellij] [-t|--terminal] [--stream] [-d|--daemon] [--attach %repo] [--switch] [--mode <mode>] [--interface <iface>] [--ip <ip>]
```

#### Options

| Flag                                         | Description                                                                                                    |
| :------------------------------------------- | :------------------------------------------------------------------------------------------------------------- |
| `%repos...`                                  | Specific services to start (starts all configured services if omitted or `--all` is passed).                   |
| `--tmux`                                     | Launch services in a Tmux session with side-by-side vertical panes.                                            |
| `-z`, `--zellij`                             | Launch services in a Zellij tiled split grid session.                                                          |
| `-t`, `--terminal`                           | Launch each service in a separate native OS terminal window.                                                   |
| `--stream`                                   | Stream raw multiplexed stdout/stderr directly to terminal without interactive TUI.                             |
| `-d`, `--daemon`                             | Launch services detached in background daemon without opening a UI.                                            |
| `--attach %repo`                             | Launch and immediately focus single service terminal output.                                                   |
| `-s`, `--switch`                             | Migrate presentation engine on-the-fly with **zero downtime**.                                                 |
| `-m`, `--mode <mode>`                        | Presentation engine mode: `tui`, `tmux`, `zellij`, `terminal`, `stream`, `daemon`, `attach`.                   |
| `--interface`, `--iface`, `--lan-interface`  | Select network interface (`wlan0`, `eno1`) or type (`wifi`, `ethernet`) to resolve `${LAN_IP}`. Prioritizes Wi-Fi by default. |
| `--ip`, `--lan-ip`                           | Explicit host LAN IP address override (e.g. `192.168.24.178`).                                                 |

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

Gracefully terminates all running services and shuts down the workspace background daemon.

```bash
ws stop @<name>
```

#### Example

```bash
ws stop @develop
```

---

### `ws restart`

Restarts running services inside an active workspace session without terminating the daemon.

```bash
ws restart @<name> [%repos...]
```

#### Example

```bash
ws restart @develop %server
```

---

### `ws logs`

Views or tails persistent log files for workspace services stored in `.ws/logs/`.

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

Connects a raw terminal I/O bridge directly into a running service's Master PTY inside the daemon.

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

Spawns an interactive subshell configured specifically for the target workspace or worktree.

```bash
ws shell @<name> [%worktree]
```

#### Example

```bash
ws shell @develop %server
```

---

### `ws env`

Inspects or synchronizes environment variables configured for workspace repositories.

```bash
ws env @<name> [%repo] [--sync] [--interface <iface>] [--ip <ip>]
```

#### Options

| Flag                                         | Description                                                                                                    |
| :------------------------------------------- | :------------------------------------------------------------------------------------------------------------- |
| `--sync`                                     | Write resolved environment variables into worktree `.env` files.                                               |
| `--interface`, `--iface`, `--lan-interface`  | Select network interface (`wlan0`, `eno1`) or type (`wifi`, `ethernet`) to resolve `${LAN_IP}`. Prioritizes Wi-Fi by default. |
| `--ip`, `--lan-ip`                           | Explicit host LAN IP address override (e.g. `192.168.24.178`).                                                 |

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

Runs dependency installation and configuration scripts defined in `repositories.yml`.

```bash
ws setup @<name> [%repos...] [--all] [--dry-run] [--skip-scripts] [--interface <iface>] [--ip <ip>]
```

#### Options

| Flag                                         | Description                                                                                                    |
| :------------------------------------------- | :------------------------------------------------------------------------------------------------------------- |
| `--all`                                      | Run setup for all repositories in workspace.                                                                   |
| `--dry-run`                                  | Print commands that would be executed without running them.                                                    |
| `--skip-scripts`                             | Only sync environment variables and file copies without executing scripts.                                     |
| `--interface`, `--iface`, `--lan-interface`  | Select network interface (`wlan0`, `eno1`) or type (`wifi`, `ethernet`) to resolve `${LAN_IP}`. Prioritizes Wi-Fi by default. |
| `--ip`, `--lan-ip`                           | Explicit host LAN IP address override (e.g. `192.168.24.178`).                                                 |

#### Examples

```bash
# Setup all repositories (prioritizes Wi-Fi adapter for mobile discovery):
ws setup @develop --all

# Setup with explicit network interface:
ws setup @develop --all --interface wlan0
```

---

## 5. Project Store Management

### `ws project init` / `ws init`

Initializes bare Git repositories in `bares/` and generates `repositories.yml`.

```bash
ws project init [alias=URL ...]
```

#### Example

```bash
ws project init server=git@github.com:org/api.git mobile=git@github.com:org/app.git
```

---

### `ws project add` / `ws add`

Clones an additional bare repository into `bares/` and registers it in `repositories.yml`.

```bash
ws project add <alias=URL>
```

#### Example

```bash
ws project add frontend=git@github.com:org/web.git
```

---

### `ws project fetch` / `ws fetch`

Fetches remote branches and tags across all bare repositories in `bares/`.

```bash
ws project fetch
```

---

### `ws project sync` / `ws sync`

Runs `git worktree prune` across all bare repositories and cleans up stale references.

```bash
ws project sync
```

---

### `ws doctor`

Runs an automated system diagnostic check verifying Git version, multiplexer availability (`tmux`, `zellij`), bare repository integrity, socket permissions, and environment health.

```bash
ws doctor
```

---

### `ws completion`

Generates or installs shell autocompletion scripts for Zsh, Bash, and Fish, providing tab-completion for subcommands, workspaces (`@<name>`), repositories (`%<repo>`), and flags.

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


## 6. Universal Inverted Syntax

All workspace-scoped commands support the intuitive **`ws @<name> <verb>`** inverted syntax:

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
