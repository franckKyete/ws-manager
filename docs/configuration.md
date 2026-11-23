# ⚙️ Configuration Specification

`ws` is configured declaratively using YAML. This document details the schema
and behavior of the project configuration file (`ws-project.yml`), declarative
workspace templates (`workspace.yml`), environment variable scoping, and
lifecycle/setup configuration.

---

## 📄 `ws-project.yml` Schema

The `ws-project.yml` file resides in the root of your project directory and
defines the bare repository store, services, ports, environment variables, and
setup pipelines.

### Full Annotated Example (Lifecycle schema v3)

```yaml
schema_version: 3

# Global environment variables accessible across all workspaces and services
env:
  NODE_ENV: development
  LOG_LEVEL: debug
  GLOBAL_API_URL: http://localhost:8080

# Lifecycle (v3): built-in configure stage + ordered setup stages
lifecycle:
  configure:
    - run: "echo 'Preparing workspace: ${WORKSPACE_NAME}'"
      when: once
  setup:
    - name: install
      hooks:
        - run: "echo 'global install hooks (optional)'"
    - name: verify
      hooks:
        - run: "echo 'workspace verification'"

# Repositories & Services Definition
repositories:
  server:
    bare: bares/Renttik-server.git
    checkout: Renttik-server
    base_branch: main
    command: npm run dev
    port: 8080
    depends_on: []
    # File/env sync is handled by the built-in configure stage.
    copy_files:
      - from: files/.env.server
        to: .env
      - from: .env.example
        to: .env.local
    env:
      PORT: '8080'
      DATABASE_URL: 'postgresql://postgres:postgres@localhost:5432/db_${WORKSPACE_NAME}'
      SECRET_KEY: '${ENV:DEV_SECRET_KEY:-default_secret}'
    lifecycle:
      setup:
        - name: install
          hooks:
            - run: npm install
        - name: migrate
          hooks:
            - run: npm run db:migrate
        - name: verify
          hooks:
            - run: npm run check

  mobile:
    bare: bares/Renttik-mobile.git
    checkout: Renttik-mobile
    base_branch: main
    command: npx expo start --port 8081
    port: 8081
    depends_on:
      - server
    copy_files:
      - from: files/.env.mobile
        to: .env
    env:
      EXPO_PUBLIC_API_URL: 'http://localhost:8080'
      EXPO_PORT: '8081'
    lifecycle:
      setup:
        - name: install
          hooks:
            - run: npm install
```

> **Migration required:** legacy configs and v1 lifecycle configs must be
> migrated before use:
>
> ```bash
> ws project migrate lifecycle --yes
> ```

---

## 🧩 Schema Field Reference

### Top-Level Fields

| Field            | Type                    | Description                                                                                |
| :--------------- | :---------------------- | :----------------------------------------------------------------------------------------- |
| `schema_version` | `integer`               | Lifecycle/config schema version (v3).                                                      |
| `env`            | `dict[str, str]`        | Global public environment variables synchronized in blueprint revisions.                   |
| `secret`         | `dict[str, str]`        | Global sensitive secrets encrypted with AES-256-GCM in wshub Vault.                        |
| `private`        | `dict[str, str]`        | Global host-specific variables that **never leave the local machine**.                     |
| `lifecycle`      | `object`                | v3 lifecycle definition (`configure` hooks and ordered `setup` stages).                     |
| `tmux`           | `dict` or `str`         | Project Tmux session configuration for workspace windows (`session`, `command`, `switch`). |
| `hub`            | `dict`                  | wshub settings: default project identifier and background `auto_save` configuration.       |
| `workspace`      | `object`                | Workspace-root settings (project-wide): root `copy_files`, root `env`/env file sync, and hub context save options. |
| `repositories`   | `dict[str, RepoConfig]` | Map of repository definitions keyed by repository alias (`server`, `mobile`, etc.).        |

---

### 📁 Workspace Root (`workspace:`)

Configures workspace-root behavior applied to each workspace directory (`workspaces/<name>/`) during the built-in `configure` stage (before workspace-scope configure hooks):

- `copy_files` / `files`: copy specs (same format as repository `copy_files`) from project sources (`files/` or project root) into the workspace root.
- `env`: workspace-specific environment variables (merged with global env, workspace values override on key collision) resolved with placeholder expansion in workspace context.
- `env_file` / `env_example`: target and template filenames for generated/synced workspace env file (defaults `.env` / `.env.example`).
- `save.exclude`: glob patterns to exclude additional workspace-root context files from hub state snapshots.
- `save.max_file_size`: per-file size limit in bytes for hub context snapshots (default `1048576`).

Hub state save/restore captures eligible workspace-root context files (implicit, excluding repo worktrees, `.ws/`, `workspace.yml`/`workspace.yaml`, and the configured workspace env files by default), skips symlinks and oversized files, and restores snapshot paths by upsert (no deletions). Workspace-scope lifecycle hooks continue to run at the workspace root and receive resolved workspace env vars.

---

### 🪟 Tmux Workspace Integration (`tmux:`)

When `tmux` is configured, `ws` automatically manages a project-wide tmux
session where each window represents a workspace.

```yaml
# Mapping notation
tmux:
  session: 'Workspace' # Required: Tmux session name
  command: 'nvim' # Optional: command to run in newly created windows (default: None -> default shell)
  switch: false # Optional: auto-focus new workspace window upon creation (default: false)

# Or shorthand notation
tmux: 'Workspace'
```

- On `ws create @feat`: creates a new window named `@feat` in the session,
  working directory set to `workspaces/feat`.
- On `ws end @feat`: cleanly terminates the window
  `tmux kill-window -t Workspace:feat`.
- Switch/focus anytime with `ws focus @feat` or `ws switch @feat`.

---

### Environment Variable Tiers & Scoping

`ws` supports three tiers of environment variables to guarantee Zero-Git secrets
and local machine isolation:

1. **`public` (Default in `env:`)**: Standard non-sensitive configuration
   (ports, URLs, feature flags) synchronized in plaintext blueprint revisions.
2. **`secret` (in `secret:` block or `secret:<value>` prefix)**: Sensitive
   credentials (tokens, passwords, API keys) automatically stripped from the
   public blueprint, encrypted with **AES-256-GCM** in the wshub Vault, and
   re-hydrated on clone/sync.
3. **`private` (in `private:` block or `private:<value>` prefix)**:
   Developer-only or machine-specific overrides (local tool paths, hardware IPs)
   that **never leave the local machine**.

#### Example Syntax

```yaml
# 1. Block notation
env:
  NODE_ENV: development
  LOG_LEVEL: debug

secret:
  JWT_SECRET: super_secure_token
  STRIPE_KEY: sk_live_12345

private:
  DEV_TOOL_PATH: /opt/custom/bin
  LOCAL_HARDWARE_IP: 192.168.1.50

# 2. Inline prefix notation (shorthand)
repositories:
  server:
    bare: bares/server.git
    checkout: server
    env:
      PORT: '8080'
      DATABASE_PASSWORD: 'secret:postgres_super_pass' # Encrypted in Vault
      DEBUG_CACHE: 'private:/tmp/my-server-cache' # Stays local only
```

---

### ☁️ Hub & Periodic Auto-Save (`hub:`)

When collaborating via `wshub`, you can define project associations and
automatic background state synchronization:

```yaml
hub:
  project: 'kyete/renttik' # Default project identifier for hub push/pull/state
  auto_save:
    enabled: true # Enable or disable auto-saving (default: false)
    interval: '15m' # Periodic interval (e.g. 5m, 15m, 1h, 30s, or 'never')
    include_wip: true # Snapshot uncommitted edits and untracked files (default: true)
    workspaces: 'all' # "all", "active" (only workspaces with running services), or list ["dev", "feat-auth"]
    notify: true # Desktop notifications via D-Bus on success and failure (default: true)
```

#### Auto-Save Configuration Options

| Option        | Type                    | Default | Description                                                                                                                              |
| :------------ | :---------------------- | :------ | :--------------------------------------------------------------------------------------------------------------------------------------- |
| `enabled`     | `boolean`               | `false` | Enables periodic background state auto-saving.                                                                                           |
| `interval`    | `string` or `int`       | `15m`   | Interval between auto-save checks. Supports human durations (`"10m"`, `"1h"`, `"300s"`).                                                 |
| `include_wip` | `boolean`               | `true`  | When true, includes uncommitted worktree modifications and untracked files.                                                              |
| `workspaces`  | `string` or `list[str]` | `"all"` | `"all"` saves all workspaces, `"active"` saves only workspaces with active sessions/supervisors, or an explicit list of workspace names. |
| `notify`      | `boolean`               | `true`  | Dispatches desktop notifications via D-Bus upon auto-save success and failure.                                                           |

> [!NOTE] `ws` employs smart fingerprint caching. If a workspace's branch heads
> and uncommitted files have not changed since the last save, the upload is
> skipped to save network bandwidth and hub storage.

---

### `RepoConfig` Fields

| Field        | Type                            | Required | Description                                                                                         |
| :----------- | :------------------------------ | :------- | :-------------------------------------------------------------------------------------------------- |
| `bare`       | `string`                        | **Yes**  | Relative or absolute path to the bare Git repository (e.g. `bares/server.git`).                     |
| `checkout`   | `string`                        | **Yes**  | Subdirectory name where the worktree is checked out inside each workspace (e.g. `Renttik-server`).  |
| `base_branch` | `string`                        | No       | Default branch new workspace branches are created from. Prompted/detected when missing.              |
| `command`    | `string`                        | No       | Service launch command (e.g. `npm run dev`, `cargo run`).                                           |
| `port`       | `integer`                       | No       | Primary base network port the service listens on (used as default port in discovery and status).    |
| `ports`      | `list[int]` or `dict[str, int]` | No       | Multiple base network ports (e.g. `[8080, 8081]` or `{http: 8080, ws: 8081, metrics: 9090}`).       |
| `depends_on` | `list[str]`                     | No       | List of service aliases that must start before this service.                                        |
| `copy_files` | `list[FileCopySpec]`            | No       | File copy specifications applied during the built-in `configure` stage.                             |
| `env`        | `dict[str, str]`                | No       | Environment variables specific to this repository worktree (resolved/synced during `configure`).    |
| `lifecycle`  | `object`                        | No       | Repository lifecycle definition (e.g. ordered `setup` stages/hooks) for v3 lifecycle execution.      |

> Legacy `setup.scripts`/`setup.env`/bootstrap/env/ready lifecycle forms are
> not used at runtime in schema v3; migrate with `ws project migrate lifecycle --yes`.

---

### 🌿 Default Base Branches (`base_branch`)

`base_branch` is the branch a repository's new workspace branch starts from.
It only applies when a branch is **created** (`create: true`); checking out an
existing branch ignores it.

```yaml
repositories:
  server:
    bare: bares/server.git
    checkout: server
    base_branch: develop
```

- **Optional and additive.** `schema_version` stays `3`; existing configs remain
  valid without the field.
- **Prompted when missing.** `ws init`, `ws add`, `ws create`, and
  `ws repo add` ask for a base branch when the repository has none and the
  terminal is interactive. The prompt is prefilled with the detected default;
  pressing Enter accepts it, and an unknown branch re-prompts.
- **Persisted on choice.** The chosen value is written back to
  `ws-project.yml` before any worktree is created. If persistence fails, the
  workspace is not created. A base persisted before a later failure remains.
- **Non-interactive behavior.** With `--yes` (or no TTY), each repository uses
  its detected default, which is saved with a warning. If no default can be
  detected, the command fails.
- **Detection order.** `origin/HEAD`, the bare repository's `HEAD`, then
  `main` → `develop` → `master`. No branch is invented when none exists.
- **Validation.** A configured base must exist locally or on the primary remote
  (`origin`, otherwise the first remote). A missing branch is fetched once
  before failing.
- **One-shot overrides.** `--base`, `--target`, and `--from` on `ws create` and
  `ws repo add` override the configured value for that command only and are
  never persisted.
- **Declarative files are strict.** `ws create --file` does not prompt, detect,
  or fall back: every `create: true` entry must supply `base`, `target`, `from`,
  or `base_branch` in the file, or the command fails before touching Git.
- **Editing defaults.** Change `base_branch` directly in `ws-project.yml`;
  there is no dedicated config command.

---

## 🔄 Dynamic Variable Interpolation & Service Discovery

`ws` provides dynamic template variables and cross-service discovery
placeholders inside commands, environment variable values, and lifecycle hook
scripts:

### Standard Template Variables

| Variable                   | Description                                       | Example Value                                   |
| :------------------------- | :------------------------------------------------ | :---------------------------------------------- |
| `${WORKSPACE_NAME}`        | Name of the active workspace                      | `feat-auth`                                     |
| `${WORKSPACE_SLOT}`        | Deterministic integer slot index of the workspace | `0`, `1`, `2`                                   |
| `${WORKSPACE_PATH}`        | Absolute path to the workspace root               | `/path/to/workspaces/feat-auth`                |
| `${WORKSPACE_DIR}`         | Absolute path to the workspace root directory     | `/path/to/workspaces/feat-auth`                |
| `${PROJECT_ROOT}`          | Absolute path to the project root directory       | `/path/to/project`                              |
| `${PROJECT_DIR}`           | Alias for `${PROJECT_ROOT}`                       | `/path/to/project`                              |
| `${SCRIPTS_DIR}`           | Path to project root automation scripts directory | `/path/to/project/scripts`                      |
| `${REPO_NAME}`             | Alias of the target repository                    | `server`                                        |
| `${REPO_PATH}`             | Absolute path to the repository worktree          | `/path/to/workspaces/feat-auth/Renttik-server` |
| `${LAN_IP}`                | Auto-detected host LAN Wi-Fi IP address           | `192.168.1.45`                                  |
| `${PUBLIC_HOST}`           | Configured public or tunnel hostname              | `myproject.loca.lt`                             |
| `${ENV:VAR_NAME:-default}` | Host environment variable with optional fallback  | `${ENV:API_KEY:-dev_key}`                       |
| `${PORT:3000}`             | Dynamic port offset (`3000 + slot * 10`)          | `3000`, `3010`, `3020`                          |

---

### 🌐 Cross-Service Discovery Placeholders

Services running in the same workspace can reference sibling services without
hardcoding ports or IP addresses:

| Placeholder                       | Target Scope         | Example Value (Slot 1)           | Best For                                               |
| :-------------------------------- | :------------------- | :------------------------------- | :----------------------------------------------------- |
| `${SERVICE_PORT:server}`          | Primary Dynamic Port | `8090`                           | Injecting target primary port into configs / flags.    |
| `${SERVICE_PORT:server:ws}`       | Named Sub-Port       | `8091`                           | Referencing a specific named port of a multi-port svc. |
| `${SERVICE_PORTS:server}`         | All Ports (List)     | `8090,8091`                      | Comma-separated list of all allocated ports.           |
| `${SERVICE_URL:server}`           | Localhost URL        | `http://127.0.0.1:8090`          | Local intra-machine communication (web ➔ API).         |
| `${SERVICE_URL:server:ws}`        | Localhost Sub-Port   | `http://127.0.0.1:8091`          | Local URL targeting specific sub-port endpoint.        |
| `${SERVICE_URL_LAN:server}`       | LAN Wi-Fi URL        | `http://192.168.1.45:8090`       | Physical mobile devices (Expo/React Native on phones). |
| `${SERVICE_URL_LAN:server:ws}`    | LAN Wi-Fi Sub-Port   | `http://192.168.1.45:8091`       | Physical mobile devices connecting to sub-port.        |
| `${SERVICE_URL_PUBLIC:server}`    | Public / Tunnel URL  | `https://myproject.loca.lt:8090` | External webhooks, OAuth callbacks, remote staging.    |
| `${SERVICE_URL_PUBLIC:server:ws}` | Public Sub-Port URL  | `https://myproject.loca.lt:8091` | External webhooks targeting specific sub-port.         |

> [!NOTE] Configured ports are **Base Ports**. At runtime, `ws` automatically
> computes slot offsets (`base_port + slot * 10`) and conducts real-time TCP
> socket probing to auto-heal any collisions before launching services.
> Discovery placeholders always resolve to the **actual live allocated ports**.

### Concrete Cross-Service Example

```yaml
repositories:
  server:
    bare: bares/server.git
    checkout: server
    ports:
      http: 8080
      ws: 8081
    command:
      npm run dev -- --http-port ${SERVICE_PORT:server:http} --ws-port
      ${SERVICE_PORT:server:ws}

  mobile:
    bare: bares/mobile.git
    checkout: mobile
    port: 8082
    command: npx expo start --port ${PORT:8082}
    depends_on:
      - server
    setup:
      env:
        # Physical phone automatically talks to backend API & WebSocket over Wi-Fi:
        EXPO_PUBLIC_API_URL: '${SERVICE_URL_LAN:server:http}'
        EXPO_PUBLIC_WS_URL: '${SERVICE_URL_LAN:server:ws}'
```

---

### 💉 Auto-Injected Runtime Discovery Variables

When `ws` launches any service or interactive subshell (`ws shell @name %repo`),
it automatically injects discovery variables for all services in the workspace:

- `WS_WORKSPACE`: Active workspace name (`feat-auth`).
- `WS_SLOT`: Workspace integer slot (`1`).
- `WS_LAN_IP`: Host LAN IP (`192.168.1.45`).
- `WS_SERVICE_<NAME>_PORT`: Resolved primary port (e.g.
  `WS_SERVICE_SERVER_PORT=8090`).
- `WS_SERVICE_<NAME>_PORTS`: Comma-separated list of all allocated ports (e.g.
  `WS_SERVICE_SERVER_PORTS=8090,8091`).
- `WS_SERVICE_<NAME>_PORT_<SUBPORT>`: Specific sub-port (e.g.
  `WS_SERVICE_SERVER_PORT_WS=8091`).
- `WS_SERVICE_<NAME>_URL`: Base localhost URL (e.g.
  `WS_SERVICE_SERVER_URL=http://127.0.0.1:8090`).
- `WS_SERVICE_<NAME>_URL_<SUBPORT>`: Sub-port localhost URL (e.g.
  `WS_SERVICE_SERVER_URL_WS=http://127.0.0.1:8091`).
- `WS_SERVICE_<NAME>_URL_LAN`: Base LAN URL (e.g.
  `WS_SERVICE_SERVER_URL_LAN=http://192.168.1.45:8090`).
- `WS_SERVICE_<NAME>_URL_LAN_<SUBPORT>`: Sub-port LAN URL (e.g.
  `WS_SERVICE_SERVER_URL_LAN_WS=http://192.168.1.45:8091`).

---

### 📶 Wireless (Wi-Fi) Adapter Prioritization & Interface Selection

To ensure physical mobile devices running client applications (e.g. Expo / React
Native on Android & iOS) can reliably connect to local API backend services,
`ws` employs smart network adapter prioritization:

1. **Active Wi-Fi Adapter (Default)**: `ws` scans physical network interfaces
   (`/sys/class/net/*/wireless`, `wlan*`, `wl*`, `wifi*`) and prioritizes active
   wireless IPv4 addresses.
2. **Ethernet Fallback**: If no active Wi-Fi interface is detected, physical
   Ethernet adapters (`eno*`, `eth*`, `enp*`) are selected.
3. **CLI Interface Flag**: You can select a specific interface or adapter type
   using `--interface <name|type>` (or `--iface`, `--lan-interface`):
   ```bash
   # Select Ethernet explicitly:
   ws start @develop --interface eno1
   # Select Wi-Fi explicitly:
   ws start @develop --interface wifi
   ```
4. **Explicit IP Override**: You can override the host IP directly using
   `--ip <ip>` (or `--lan-ip`) or the `WS_LAN_IP` environment variable:
   ```bash
   ws start @develop --ip 192.168.1.55
   ```

### 📄 Live Service Registry File (`.ws/services.json`)

On startup, `ws` writes a machine-readable discovery descriptor to
`workspaces/<name>/.ws/services.json`:

```json
{
  "workspace": "feat-auth",
  "slot": 1,
  "lan_ip": "192.168.1.45",
  "public_host": "myproject.loca.lt",
  "updated_at": "2026-08-12T15:30:00Z",
  "services": {
    "server": {
      "port": 8090,
      "ports": {
        "http": 8090,
        "ws": 8091
      },
      "url": "http://127.0.0.1:8090",
      "url_local": "http://127.0.0.1:8090",
      "url_lan": "http://192.168.1.45:8090",
      "url_public": "https://myproject.loca.lt:8090",
      "urls": {
        "http": {
          "port": 8090,
          "url_local": "http://127.0.0.1:8090",
          "url_lan": "http://192.168.1.45:8090",
          "url_public": "https://myproject.loca.lt:8090"
        },
        "ws": {
          "port": 8091,
          "url_local": "http://127.0.0.1:8091",
          "url_lan": "http://192.168.1.45:8091",
          "url_public": "https://myproject.loca.lt:8091"
        }
      },
      "status": "running"
    }
  }
}
```

---

## 🔒 Secret Masking

When inspecting environment variables using `ws env` or printing debug logs,
`ws` automatically masks sensitive values containing keywords such as:

- `SECRET`, `PASSWORD`, `KEY`, `TOKEN`, `CREDENTIAL`, `PRIVATE`, `AUTH`

Masked output in terminal:

```
DATABASE_URL: postgresql://postgres:postgres@localhost:5432/db_develop
JWT_SECRET: ********************
```

---

## 📋 Declarative `workspace.yml` Spec Files

In addition to CLI arguments, you can define workspaces declaratively in a YAML
file for team consistency and CI pipelines:

```yaml
# feature-auth.yml
name: feat-auth
description: 'Authentication and user session revamp'

repositories:
  server:
    branch: feature/auth-api
    create: true
    base: develop # Required for create: true (base/target/from/base_branch)
  mobile:
    branch: feature/auth-ui
    create: true
    base: main
  frontend:
    branch: main
    create: false
    locked: true # Automatically lock tracked files as read-only
```

Create the workspace directly from the file:

```bash
ws create -f feature-auth.yml --setup
```

> `ws create --file` is strict: any `create: true` repository without a base in
> the file is a hard error. It never prompts, detects a default, falls back to
> `ws-project.yml`, or writes the file.


---

## Activity Tracking (`~/.config/ws/activity.yml`)

Global activity tracking is configured in `~/.config/ws/activity.yml` (created/managed by `ws activity ...`). Common fields:

```yaml
enabled: true
paused: false
device_name: null        # defaults to hostname
idle_gap: 300            # seconds (or duration string)
session_cap: 14400
tail_credit: 60
heartbeat_interval: 60
heartbeat_window: 900
debounce: 2
raw_retention: 365d
watch_reconcile: 60
sync:
  enabled: true
  interval: 600
  backfill: none
exclude: []
```

Project-level overrides/mapping in `ws-project.yml` (no schema_version bump):

```yaml
activity:
  project_id: <uuid>      # generated/persisted locally
  sync: true              # per-project sync enable/disable
  hub_project: org/name   # hub mapping (may be auto-created/persisted)
```
