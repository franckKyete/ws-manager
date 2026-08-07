# 🛠️ `ws` — Multi-Repository Git Workspace Manager

`ws` is a production-grade Python CLI tool that manages multi-repository development environments using **Git worktrees**.

It elevates the **workspace** (a synchronized set of Git repositories) as the primary unit of work. Software engineers working across microservices or multi-repo projects (e.g. backend API, frontend web, mobile app) can create, pull, push, freeze, and teardown linked worktrees with zero manual overhead.

---

## 🌟 Key Features

* 🚀 **Workspace-Centric Workflow**: Create and manage synchronized feature branches across arbitrary Git repositories simultaneously.
* 📦 **Flexible Syntax & Repository Selection**: Specify branch modes per repo using intuitive positional key-value pairs (`ws new auth server=main --existing web=feature/auth`), subset selection (`--repos server,mobile`), or full project setup (`--all`).
* 🔒 **Selective Tracked File Freeze**: Lock tracked repository files (`chmod a-w` via `git ls-files`) to prevent editing, while keeping untracked files (`.env`, `node_modules`, build artifacts) writable.
* ⬇️ **Smart Pull (`ws pull`)**: Pull remote updates across workspace worktrees. Automatically detects and reports Git errors (merge conflicts, dirty uncommitted files, network failures) in styled tables.
* ⬆️ **Safe Push (`ws push`)**: Push committed changes to remotes (`git push`). Never auto-adds, never auto-commits, and **never force pushes**.
* 🌐 **Global Execution & Upward Discovery**: Install `ws` globally (`pip install .` or `pipx install .`) and run commands from **any subdirectory** inside your project tree.
* 🛡️ **Atomic Rollback Engine**: Guaranteed filesystem and Git resource cleanup if workspace creation encounters failures mid-flight.
* 🎨 **Rich Terminal UX**: Formatted tables, tree diagrams, spinners, and syntax highlighting powered by `rich`.

---

## 🏗️ Project Architecture

```
workspaces/
├── pyproject.toml              # Build & dependency metadata
├── README.md                   # Project documentation
├── .gitignore                  # Git ignore rules
│
└── ws/                         # Main Python package
    ├── __init__.py
    ├── __main__.py             # Executable module entrypoint
    ├── cli.py                  # CLI argument parser & parameter resolution
    ├── commands.py             # High-level command handlers
    ├── config.py               # Config loader, validator & upward directory traversal
    ├── git.py                  # Low-level Subprocess Git service abstraction
    ├── workspace.py            # WorkspaceManager business logic & rollback engine
    ├── models.py               # Immutable Dataclass models (RepoSpec, WorkspaceMetadata)
    ├── output.py               # Rich visual layout and formatting handler
    ├── exceptions.py           # Domain exception hierarchy
    ├── utils.py                # Timestamp formatting & filesystem helpers
    │
    ├── templates/              # Workspace metadata templates
    │   └── workspace.yml
    │
    └── tests/                  # Pytest unit & integration test suite
        ├── test_cli.py
        └── test_workspace.py
```

---

## 📦 Installation Guide

### Option 1: Global Installation via `pip` / `pipx`

Install `ws` globally on your system to run it from any directory:

```bash
# Clone the repository
git clone https://github.com/your-org/workspaces.git
cd workspaces

# Install globally using pip
pip install .

# Or install in editable mode for development
pip install -e .

# Or install using pipx
pipx install .
```

Verify installation:

```bash
ws --version
ws --help
```

### Option 2: Running Unit Tests

Run the test suite using `pytest`:

```bash
python3 -m pytest ws/tests/ -v
```

---

## 🚀 Quick Start & Walkthrough

### 1. Initialize Project Bare Repositories (`ws init`)

Initialize bare Git repositories in `bares/` and create `repositories.yml`:

```bash
ws init server=git@github.com:myorg/server.git \
        mobile=git@github.com:myorg/mobile.git \
        web=git@github.com:myorg/web.git
```

This clones bare repositories into `bares/server.git`, `bares/mobile.git`, and `bares/web.git`, generating a `repositories.yml` configuration:

```yaml
repositories:
  server:
    bare: bares/server.git
    checkout: server
  mobile:
    bare: bares/mobile.git
    checkout: mobile
  web:
    bare: bares/web.git
    checkout: web
```

---

### 2. Create a Workspace (`ws new`)

Create a workspace named `auth-flow` with custom branch mappings:

```bash
# Checkout existing main for server & mobile, create new branch feature/auth for web:
ws new auth-flow server=main --existing web=feature/auth mobile=main --existing
```

Or create new `feature/auth-flow` branches across all repositories:

```bash
ws new auth-flow --all
```

Or create a workspace with a subset of repositories:

```bash
ws new auth-flow --repos server,web
```

---

### 3. Add or Remove Repositories in a Workspace (`ws workspace ...`)

Add a repository worktree to an existing workspace:

```bash
ws workspace add-repo auth-flow web feature/auth-web
```

Remove a repository worktree from a workspace (optionally deleting its branch):

```bash
ws workspace remove-repo auth-flow web --delete-branch
```

---

### 4. Freeze & Unfreeze Repositories (`ws workspace freeze` / `unfreeze`)

Freeze a repository to mark git-tracked files read-only (`chmod a-w` via `git ls-files`):

```bash
# Freeze repository (prevents modifying tracked files)
ws workspace freeze auth-flow mobile

# Unfreeze repository (restores write permissions)
ws workspace unfreeze auth-flow mobile
```

Untracked files like `.env`, `node_modules/`, and temporary build outputs remain writable!

---

### 5. Pull Remote Updates (`ws pull`)

Pull remote updates across non-frozen workspace repositories:

```bash
# Pull updates for all repos in workspace:
ws pull auth-flow

# Pull updates for specific subset:
ws pull auth-flow --repos server,web
```

Output displays a Rich summary table:

| REPOSITORY | STATUS | BRANCH | REMOTE | DETAILS / REASON |
| :--- | :--- | :--- | :--- | :--- |
| `server` | `ℹ UP TO DATE` | `main` | `origin` | Already up to date |
| `web` | `✔ PULLED (UPDATED)` | `feature/auth` | `origin` | successfully pulled updates |
| `mobile` | `⏭ SKIPPED` | `main` | `origin` | frozen repository (read-only) |

---

### 6. Push Committed Changes (`ws push`)

Push committed changes across workspace repositories:

```bash
ws push auth-flow
```

*`ws push` only pushes committed changes (`git push`). It never attempts `git add` or `git commit`, and **never force pushes**.*

---

### 7. Inspect & Teardown Workspaces (`ws info` / `ws list` / `ws remove`)

```bash
# List all workspaces
ws list

# Detailed tree view of a workspace
ws info auth-flow

# Remove workspace and all associated worktrees
ws remove auth-flow
```

---

## 📖 CLI Command Reference

| Command | Usage | Description |
| :--- | :--- | :--- |
| `init` | `ws init [url...]` | Clone bare repositories into `bares/` and generate `repositories.yml` |
| `add` | `ws add <url>` | Add a new bare repository to `repositories.yml` |
| `new` | `ws new <name> [spec...]` | Create a workspace with custom or default branches |
| `create` | `ws create <config.yml>` | Create a workspace from a YAML configuration file |
| `pull` | `ws pull <name> [--repos r1,r2]` | Pull remote updates across workspace worktrees |
| `push` | `ws push <name> [--repos r1,r2]` | Push committed changes to remotes (no force push) |
| `list` | `ws list` | List active workspaces and frozen badges `[🔒 FROZEN]` |
| `info` | `ws info <name>` | Display detailed Rich tree view of a workspace |
| `workspace` | `ws workspace <action> ...` | Manage repos inside workspace (`add-repo`, `remove-repo`, `freeze`, `unfreeze`) |
| `remove` | `ws remove <name>` | Remove a workspace and all its worktrees |
| `open` | `ws open <name>` | Open interactive shell inside workspace directory |
| `status` | `ws status <name>` | Display Git status across all workspace worktrees |
| `exec` | `ws exec <name> -- <cmd>` | Run command across all workspace repository worktrees |
| `doctor` | `ws doctor` | Run system health and environment diagnostics |
