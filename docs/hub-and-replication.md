# wshub & Replication Architecture

`wshub` is the centralized cloud and team collaboration hub for the `ws`
multi-repository ecosystem. It bridges local developer workspaces with remote
teams and allows seamless cross-machine replication without exposing secrets in
Git repositories.

---

## Key Capabilities

1. **Zero-Git Vault (Envelope Encryption)**:
   - Secret environment variables (`.env`) and sensitive keys (`.pem`, `.json`,
     certificates) are stored in `wshub` using **AES-256-GCM envelope
     encryption** with project-specific HKDF keys derived from a master vault
     key.
   - **Secrets never enter Git commits or repositories**.

2. **Project Blueprint Registry & Versioning**:
   - Stores `repositories.yml` blueprints and automation scripts with linear
     versioning (`v1`, `v2`, ...).
   - Enables one-command project cloning: `ws clone <org/project>`.

3. **Cross-Machine Session Resumption**:
   - Snapshot active workspace branch checkouts, file locks, and configuration
     on Machine A: `ws hub state save @develop`.
   - Re-hydrate the exact same branch checkouts and worktrees on Machine B:
     `ws hub resume @develop`.

4. **Provider-Agnostic Backend (Clean Architecture)**:
   - Built with **Hono (TypeScript)** and **NestJS-style Clean Architecture**
     (Controllers, Services, Repositories, DI Container).
   - Hexagonal Ports & Adapters support **Cloudflare Workers / Pages (D1, R2,
     KV)** as well as **Self-Hosted Node.js / Docker (SQLite/Postgres, S3/Local
     Blob)**.

---

## Quick Start Workflow

### 1. Authenticate with wshub

```bash
# Log in interactively
ws hub login --url http://127.0.0.1:8787

# Or log in with a Personal Access Token
ws hub login --url http://127.0.0.1:8787 --token wshub_pat_abcdef...

# Verify your session
ws hub whoami
```

### 2. Publishing an Existing Project

When you run `ws hub publish`, `ws` automatically performs 3-tier asset
classification:

1. **Public variables** (`env:`) are preserved in the published blueprint.
2. **Secrets** (`secret:` block or `secret:<value>`) are masked with `"secret"`
   in the blueprint and automatically **encrypted with AES-256-GCM** into the
   wshub Vault.
3. **Private variables** (`private:` block or `private:<value>`) are
   **completely stripped** and never leave your local machine.
4. **Sensitive files** (`files/` directory or `copy_files:`) are **encrypted and
   uploaded** to the wshub encrypted blob store.

```bash
cd my-project-workspaces
ws hub publish kyete/renttik -d "Production polyrepo ecosystem"
# Output:
# ✔ Published project kyete/renttik (Revision v1)
# 🔒 Stored and encrypted 4 secret(s) in Vault
# 📁 Encrypted and uploaded 2 sensitive file(s)
# 🚫 Skipped 2 private variable(s) (kept local)
```

### 3. Cloning a Project on a New Machine

```bash
# Clone blueprint, download & decrypt sensitive files, re-hydrate secrets, and clone all bare repos
ws clone kyete/renttik

cd renttik-workspaces
ws create @develop --all
ws start @develop
```

### 4. Managing Vault Secrets & Sensitive Files

```bash
# Set a secret key
ws hub secret set PAWAPAY_JWT_TOKEN "eyJraWQiOiIx..." --repo server

# Upload a sensitive file (e.g. RSA private key or service account JSON)
ws hub secret upload files/pawapay-private.pem

# List encrypted secrets
ws hub secret list

# Pull secrets & files into local workspace
ws hub secret pull
```

### 5. Resuming Work from Another Machine (Automatic WIP Sync)

`ws hub state save` automatically captures both your checked-out branch topology
and any **uncommitted work** (modified tracked files, staged changes, and new
untracked files):

```bash
# 🖥️ Machine A (before leaving):
# Automatically snapshots branches AND uncommitted edits in all repositories
ws hub state save @feature-checkout

# Output:
# ✔ Saved workspace state @feature-checkout to kyete/renttik
# ℹ 🔒 Captured uncommitted work in %server (2 modified files, 1 untracked file)

# 💻 Machine B (at home/office):
# Re-provisions worktrees, branches, and re-applies all uncommitted edits
ws hub resume @feature-checkout

# Output:
# ℹ Recreating workspace @feature-checkout from hub state...
# ✔ Creating workspace feature-checkout
# ✔ Restored uncommitted work in %server
# ✔ Restored workspace @feature-checkout successfully
```

#### Skipping Uncommitted Work

If you only want to sync the branch references without uncommitted code:

```bash
ws hub state save @develop --no-wip
ws hub resume @develop --no-wip
```

---

### 6. Automatic Workspace State Saving (`ws hub auto-save`)

Instead of remembering to manually run `ws hub state save`, `ws` can
periodically snapshot and save your workspaces in the background.

#### Smart Deduplication

Auto-save continuously computes a workspace fingerprint incorporating:

- Current branch `HEAD` commit SHA for each repository.
- Modified / staged files detected via Git status.
- Untracked file timestamps and sizes.

If nothing has changed since the last snapshot, the upload is **completely
skipped**, ensuring zero wasteful network calls.

#### Configuration in `repositories.yml`

```yaml
hub:
  project: 'kyete/renttik'
  auto_save:
    enabled: true # Enable auto-save (default: false)
    interval: '15m' # e.g., "5m", "15m", "1h", "300s", or "never"
    include_wip: true # include uncommitted / untracked work (default: true)
    workspaces: 'all' # "all", "active" (workspaces with active sessions), or list of names
```

#### Global Multi-Project Auto-Save & Systemd Service (`ws.service`)

`ws` runs a single machine-wide background daemon that automatically monitors
all registered projects with `hub.auto_save.enabled: true`:

```bash
# Install and enable the systemd user service (starts on boot)
ws service install

# Check status of ws.service and see all monitored projects
ws service status

# Start / Stop / Restart the service
ws service start
ws service stop
ws service restart

# Stream live journal logs
ws service logs

# View auto-save status and workspace topology for the current project
ws hub auto-save status

# Trigger an immediate one-time auto-save pass right now
ws hub auto-save once [--force]
```

#### Project Registry

Projects are automatically registered whenever you run `ws` inside them. You can
also manage the registry explicitly:

```bash
# List all registered projects monitored by the daemon
ws project list

# Register a project directory
ws project register [/path/to/project]

# Unregister a project directory
ws project unregister [/path/to/project]
```

#### D-Bus Desktop Notifications

Auto-save automatically sends desktop notifications over D-Bus
(`org.freedesktop.Notifications`) when snapshots occur:

- **Success (`document-save` icon)**: Confirms when a workspace has been safely
  snapshotted and uploaded to `wshub`.
- **Failure (`dialog-error` icon)**: Alerts you immediately if an upload fails
  (e.g. `wshub` server offline or connection lost), without interrupting your
  terminal or local work.
- Notifications can be muted anytime by setting `notify: false` under
  `hub.auto_save` in `repositories.yml`.

#### Automatic Publishing on First Save

When saving a workspace state (`ws hub state save`, `ws hub auto-save once`, or
background auto-save) or pushing revisions for a project that has never been
registered on `wshub`, `ws` automatically:

1. Detects that the project is new on the hub.
2. Performs 3-tier asset classification and publishes the project blueprint,
   encrypted vault secrets, and sensitive files.
3. Automatically completes the workspace state save or blueprint push.

#### Real-Time Configuration File Watcher

The background daemon (`ws.service`) includes a lightweight real-time file
watcher that continuously monitors all registered projects:

- **`repositories.yml` Edited**: Automatically pushes an updated blueprint
  revision to `wshub` and issues a desktop notification.
- **`workspace.yml` Edited**: Automatically saves the workspace state (branches,
  locks, uncommitted WIP) to `wshub` and updates the deduplication fingerprint
  cache.
- **Syntax Safety**: Detects partial or incomplete YAML syntax while editing and
  only triggers once valid configuration is saved.
