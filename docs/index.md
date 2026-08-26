# 📖 `ws` Documentation Portal

Welcome to the comprehensive documentation for **`ws`** (Workspace Manager) — the modern multi-repository Git workspace and service management tool designed for polyrepo development.

---

## 🧭 Documentation Map

| Guide                                                        | Description                                                                                                                                |
| :----------------------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------- |
| [**🚀 Getting Started**](getting-started.md)                 | System requirements, installation instructions, and a 3-minute onboarding tutorial.                                                        |
| [**💻 CLI Reference**](cli-reference.md)                     | Exhaustive command manual detailing syntax, options, aliases, and real-world examples for all commands.                                    |
| [**⚙️ Configuration Specification**](configuration.md)       | Complete schema reference for `repositories.yml` and declarative `workspace.yml` spec files.                                               |
| [**🏛️ Architecture & Internals**](architecture.md)           | Deep dive into the Git worktree model, bare repositories store, daemon architecture, Unix socket IPC, and the Rust `_native` vt100 engine. |
| [**🖥️ Multiplexers & Runtime**](multiplexers-and-runtime.md) | Operating the Interactive TUI, Tmux vertical panes, Zellij split grid, Background Daemon, and **Zero-Downtime Presentation Switching**.    |
| [**🌲 Worktree & Git Management**](worktree-management.md)   | Worktree lifecycles, branch coordination, tracked file locking (`ws repo lock`), and safe multi-repo push/pull.                            |
| [**🩺 Troubleshooting & Diagnostics**](troubleshooting.md)   | Diagnostic workflows with `ws doctor`, socket debugging, rollback recovery, and common resolutions.                                        |

---

## 🌟 Quick Overview: The `ws` Philosophy

Modern development teams often organize related services across multiple Git repositories (e.g. backend API, frontend web, mobile app, microservices). Working across multiple repositories traditionally introduces significant friction:

- Managing multiple branches and clones manually.
- Context switching between divergent feature setups.
- Disk duplication and lengthy clone times.
- Starting and monitoring multiple development servers across different terminals.

**`ws` solves this by making the _Workspace_ the primary unit of work:**

1. **Shared Bare Store**: Repositories are cloned **once** as bare repositories (`bares/<repo>.git`), consuming minimal disk space.
2. **Instant Git Worktrees**: Workspaces are isolated directories where lightweight Git worktrees are checked out in milliseconds without duplicating Git history.
3. **Coordinated Multi-Repo Lifecycle**: Create, branch, status, lock, pull, and push across all workspace repositories with unified commands.
4. **Supervised Runtime with Zero-Downtime Switching**: Run all services simultaneously under a persistent background daemon, and switch seamlessly between an interactive Rust TUI, Tmux vertical panes, and Zellij without restarting child processes.

---

## 🚀 Quick Command Reference

```bash
# Initialize a project bare store
ws project init server=git@github.com:org/server.git mobile=git@github.com:org/mobile.git

# Create a feature workspace with coordinated branches
ws create @feat-auth %server:main:existing %mobile:feature/auth:new

# Start all services in Tmux with side-by-side vertical panes
ws start @feat-auth --tmux

# Switch live presentation to the interactive Rust TUI without downtime
ws attach @feat-auth --switch

# Open an interactive shell inside a specific worktree
ws shell @feat-auth %server

# Lock tracked files in a worktree to prevent accidental edits
ws lock @feat-auth %server

# View live service and process status
ws info @feat-auth
```
