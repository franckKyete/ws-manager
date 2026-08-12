# 🖥️ Multiplexers, Presentation Engines & Runtime

`ws` provides a versatile suite of presentation engines for running and monitoring multi-service workspaces. Whether you prefer an interactive native TUI, side-by-side Tmux vertical panes, Zellij, or detached background daemons, `ws` supports them all with **zero-downtime switching**.

---

## 📊 Presentation Engine Comparison

| Engine | Launch Flag | Attach Flag | Best For |
| :--- | :--- | :--- | :--- |
| **Interactive Rust TUI** | `ws start @name` | `ws attach @name` | All-in-one terminal dashboard with scrollback, search, and service controls. |
| **Tmux Vertical Panes** | `ws start @name --tmux` | `ws attach @name --tmux` | Side-by-side vertical column panes inside standard Tmux. |
| **Zellij Split Grid** | `ws start @name -z` | `ws attach @name -z` | Modern tiled multi-service panes with Zellij tabs. |
| **Detached Daemon** | `ws start @name -d` | N/A | Headless background execution for CI, background servers, or remote dev. |
| **Streaming Output** | `ws start @name --stream` | N/A | Direct stdout/stderr multiplexing in standard terminal output. |
| **Native Terminal Tabs** | `ws start @name -t` | N/A | Spawning separate OS terminal windows/tabs per service. |

---

## 1. Interactive Rust TUI

The default presentation engine is a high-performance terminal UI built with Rust, `ratatui`, and `crossterm`.

```bash
ws start @develop
```

### Keybindings & Navigation

| Keybinding | Action | Description |
| :--- | :--- | :--- |
| `Tab` / `Shift+Tab` | **Cycle Services** | Switch focus between service panes. |
| `F` | **Toggle Fullscreen** | Maximize the focused service pane to fill the entire window. |
| `PageUp` / `PageDown` | **Scroll Logs** | Scroll up and down through the 10,000-line history buffer. |
| `Home` / `End` | **Jump Top/Bottom** | Jump directly to the oldest or newest log line. |
| `R` | **Restart Service** | Trigger an in-place restart of the currently selected service. |
| `B` | **Raw Bridge** | Open a direct raw interactive terminal bridge to the service PTY. |
| `S` | **Switch Engine** | Trigger zero-downtime presentation migration. |
| `Q` / `Ctrl+C` | **Exit / Detach** | Exit the TUI view (services continue running in the background daemon). |

---

## 2. Tmux Integration (Vertical Side-by-Side Panes)

For developers who live inside `tmux`, `ws` launches services in side-by-side **vertical columns** (`tmux split-window -h`) with an `even-horizontal` layout:

```bash
ws start @develop --tmux
```

### Features:
- **Automatic Session & Window Naming**: Named `ws-<workspace_name>` (e.g. `ws-develop`).
- **Vertical Columns**: Services sit side-by-side across your monitor, allowing easy horizontal comparison of frontend, backend, and database logs.
- **Tmux Native Keybindings**: Use standard `Ctrl+B + [o / Arrow Keys]` to navigate between panes.

---

## 3. Zellij Integration (Tiled Split Grid)

For users of [Zellij](https://zellij.dev), `ws` automatically generates a tailored KDL layout file and launches a tiled session:

```bash
ws start @develop --zellij
# or
ws start @develop -z
```

---

## 4. Detached Daemon Mode (`-d`)

Run services completely in the background without locking your current terminal shell:

```bash
ws start @develop -d
```

Check the status of running services at any time:
```bash
ws info @develop
```

---

## 5. Zero-Downtime Presentation Switching

You can switch between any presentation engine on-the-fly without killing or restarting running services.

### Recipe 1: Start Headless ➔ Attach in Tmux
```bash
# 1. Start services in the background
ws start @develop -d

# 2. Attach later in Tmux with side-by-side vertical panes
ws attach @develop --tmux --switch
```

### Recipe 2: From Tmux ➔ Switch to Rust TUI
```bash
# Attach using the interactive TUI without interrupting processes
ws attach @develop --switch
```

### Recipe 3: From TUI ➔ Switch to Zellij
```bash
# Attach in Zellij
ws attach @develop -z --switch
```

---

## 6. Direct Terminal Bridge (`ws bridge`)

When you need to send interactive stdin (e.g. debugging prompts, inputting 2FA codes, or running interactive CLI commands) directly to a service running under the daemon:

```bash
ws bridge @develop %mobile
```

Press `Ctrl+]` to detach from the bridge and return to your shell.

---

## 7. Service Log Management (`ws logs`)

View or follow persistent log files for workspace services:

```bash
# Tail live logs for the server service
ws logs @develop %server -f

# View the last 100 lines
ws logs @develop %server -n 100

# View logs for all services in the workspace
ws logs @develop
```
