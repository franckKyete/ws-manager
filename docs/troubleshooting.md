# 🩺 Troubleshooting & Diagnostics

This guide helps diagnose and resolve common issues when working with `ws`.

---

## 🩺 Quick Diagnostic Check (`ws doctor`)

Run the built-in diagnostic suite to verify your environment health:

```bash
ws doctor
```

`ws doctor` checks:
- ✅ Python version (>= 3.10)
- ✅ Git binary version and `git worktree` support (>= 2.20)
- ✅ Multiplexer binaries (`tmux`, `zellij`)
- ✅ Bare repositories directory integrity in `bares/`
- ✅ Configuration syntax in `repositories.yml`
- ✅ Active Unix domain sockets and permissions in `workspaces/`

---

## 🔍 Common Issues & Resolutions

### 1. Shell Stripping Repository Arguments (`#` vs `%`)

#### Problem:
Running `ws lock @develop #server` results in:
`error: the following arguments are required: repo`

#### Cause:
In Bash and Zsh, `#` is the shell comment character when preceded by a space. The shell strips `#server` before passing arguments to Python.

#### Resolution:
Use the **`%`** sigil for repositories (or use plain names):
```bash
ws lock @develop %server
# or
ws lock @develop server
```

---

### 2. Stale Unix Domain Socket (`.ws/session.sock`)

#### Problem:
After a machine crash or ungraceful shutdown, `ws start` reports that a session is already active, but no processes are running.

#### Resolution:
1. `ws` automatically validates PID liveness on startup and cleans up dead sockets.
2. If needed, you can force stop the workspace:
   ```bash
   ws stop @<name>
   ```
3. Or manually delete the socket file:
   ```bash
   rm workspaces/@<name>/.ws/session.sock
   ```

---

### 3. Port Already in Use

#### Problem:
A service fails to start with `EADDRINUSE: address already in use :8080`.

#### Resolution:
1. Check which service is currently occupying the port:
   ```bash
   ws info @<name>
   # or check system-wide
   lsof -i :8080
   ```
2. If another workspace session is holding the port, stop it with `ws stop @other-workspace`.

---

### 4. Git Worktree Desynchronization

#### Problem:
Git reports `fatal: '<path>' is already checked out by worktree at '<path>'`.

#### Resolution:
Prune stale worktree references across all bare repositories:
```bash
ws project sync
# or directly via git
git -C bares/<repo>.git worktree prune
```

---

### 5. Remote Branch Not Found During Workspace Creation

#### Problem:
Running `ws create @feat %server:feature/new-api:existing` fails with `Branch 'feature/new-api' does not exist`.

#### Resolution:
Fetch the latest remote branches into the bare store:
```bash
ws project fetch
```
Then retry creating the workspace.

---

### 6. Subshell Prompt Not Showing Workspace Name

#### Problem:
When running `ws shell @develop %server`, your prompt does not display `[@develop] server $`.

#### Resolution:
`ws shell` injects `$WS_WORKSPACE` and `$WS_REPO` into the subshell environment. Ensure your shell config (`.bashrc` or `.zshrc`) supports customized prompts, or check the variables directly:
```bash
echo "Workspace: $WS_WORKSPACE, Repo: $WS_REPO"
```
