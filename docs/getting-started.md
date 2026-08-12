# 🚀 Getting Started with `ws`

This guide walks you through system requirements, installation, initial project setup, and creating your first multi-repository workspace.

---

## 📋 System Requirements

- **Operating System**: Linux (Ubuntu, Debian, Fedora, Arch, etc.) or macOS.
- **Python**: Python 3.10 or higher.
- **Git**: Git 2.20 or higher (with `git worktree` support).
- **Rust / Cargo** *(Optional, for building the native terminal engine)*: Rust 1.75+.
- **Terminal Multiplexers** *(Optional)*:
  - `tmux` 3.0+ (for `ws start --tmux`)
  - `zellij` 0.39+ (for `ws start --zellij`)

---

## 📦 Installation

### Option 1: Global Installation via `pip` / `pipx` (Recommended)

Installing `ws` globally makes the `ws` binary available across your entire system:

```bash
# Clone the repository
git clone https://github.com/franckKyete/ws-manager.git
cd ws-manager

# Install using pip
pip install .

# Or install using pipx (isolated virtual environment)
pipx install .
```

Verify your installation:
```bash
ws --version
ws doctor
```

---

### Option 2: Editable Installation for Development

If you are developing or customizing `ws`:

```bash
# Clone and enter the repository
git clone https://github.com/franckKyete/ws-manager.git
cd ws-manager

# Install in editable mode with development dependencies
pip install -e ".[dev]"

# (Optional) Compile native Rust engine in development mode
cargo build --workspace
```

Run the automated test suite to ensure everything is operational:
```bash
python3 -m pytest ws/tests/ -v
cargo test --workspace
```

---

## 🎓 3-Minute Tutorial: Your First Workspace

Follow these steps to set up a project containing a backend API server and a mobile app.

### Step 1: Initialize Project Bare Repositories

Inside your project root directory (e.g. `~/my-polyrepo-project`), run `ws project init` with your remote Git URLs:

```bash
ws project init server=git@github.com:example-org/api-server.git \
                mobile=git@github.com:example-org/mobile-app.git
```

This command:
1. Clones bare repositories into `bares/server.git` and `bares/mobile.git`.
2. Generates a root `repositories.yml` configuration file.

Inspect the generated `repositories.yml`:
```yaml
repositories:
  server:
    bare: bares/server.git
    checkout: server
    command: npm run dev
    port: 8080
  mobile:
    bare: bares/mobile.git
    checkout: mobile
    command: npm start
    port: 8081
```

---

### Step 2: Create a Workspace (`ws create`)

Now create a coordinated feature workspace named `@feat-auth`:

```bash
# Create feature/auth branch for both server and mobile:
ws create @feat-auth %server %mobile
```

Or checkout an existing `main` branch for `%server` while creating a new `feature/auth-ui` branch for `%mobile`:

```bash
ws create @feat-auth %server:main:existing %mobile:feature/auth-ui:new
```

Your directory structure now contains lightweight Git worktrees:
```
my-polyrepo-project/
├── bares/
│   ├── server.git/
│   └── mobile.git/
├── repositories.yml
└── workspaces/
    └── @feat-auth/
        ├── .ws/
        │   └── metadata.json
        ├── server/             <-- Worktree on main
        └── mobile/             <-- Worktree on feature/auth-ui
```

---

### Step 3: Run Setup Scripts & Environment Sync

Sync `.env` files and run setup scripts (e.g. `npm install`) inside all repositories:

```bash
ws setup @feat-auth --all
```

---

### Step 4: Start Services in Tmux or TUI

Start all workspace services concurrently:

```bash
# Launch in Tmux with side-by-side vertical panes:
ws start @feat-auth --tmux

# Or launch in the interactive Rust TUI:
ws start @feat-auth
```

---

### Step 5: Seamless Zero-Downtime Engine Switching

While your services are running in Tmux, switch directly to the Rust TUI without restarting child processes:

```bash
ws attach @feat-auth --switch
```

---

### Step 6: Interactive Developer Subshell

Open an interactive subshell inside the `%mobile` repository worktree:

```bash
ws shell @feat-auth %mobile
```

Your shell prompt updates to `[@feat-auth] mobile $`, and relevant environment variables (`$WS_WORKSPACE`, `$WS_REPO`) are automatically populated.

---

### Step 7: Inspect and Teardown

Check Git status and process metrics across all repositories:

```bash
# View Git status across worktrees
ws status @feat-auth

# View live process statuses and ports
ws info @feat-auth

# Stop running services
ws stop @feat-auth

# Safely delete the workspace when work is finished
ws delete @feat-auth
```

---

## 📚 Next Steps

- Explore all command options in the [**CLI Reference**](cli-reference.md).
- Learn how to configure environment variables and file sync in [**Configuration Specification**](configuration.md).
- Master multiplexers in [**Multiplexers & Runtime**](multiplexers-and-runtime.md).
