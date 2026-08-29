# ⚙️ Configuration Specification

`ws` is configured declaratively using YAML. This document details the schema and behavior of the project configuration file (`repositories.yml`), declarative workspace templates (`workspace.yml`), environment variable scoping, and automated setup scripts.

---

## 📄 `repositories.yml` Schema

The `repositories.yml` file resides in the root of your project directory and defines the bare repository store, services, ports, environment variables, and setup pipelines.

### Full Annotated Example

```yaml
# Global environment variables accessible across all workspaces and services
env:
  NODE_ENV: development
  LOG_LEVEL: debug
  GLOBAL_API_URL: http://localhost:8080

# Global workspace setup scripts (executed in workspace root)
setup:
  scripts:
    - name: "Global Pre-flight Check"
      command: "echo 'Preparing workspace: ${WORKSPACE_NAME}'"

# Repositories & Services Definition
repositories:
  server:
    bare: bares/Renttik-server.git
    checkout: Renttik-server
    command: npm run dev
    port: 8080
    depends_on: []
    setup:
      # Copy files from files/ directory or template examples into worktree
      copy_files:
        - from: files/.env.server
          to: .env
        - from: .env.example
          to: .env.local
      # Static & scoped environment variables
      env:
        PORT: "8080"
        DATABASE_URL: "postgresql://postgres:postgres@localhost:5432/db_${WORKSPACE_NAME}"
        SECRET_KEY: "${ENV:DEV_SECRET_KEY:-default_secret}"
      # Repository-specific setup scripts executed inside worktree
      scripts:
        - name: "Install Server Dependencies"
          command: "npm install"
        - name: "Run Database Migrations"
          command: "npm run db:migrate"

  mobile:
    bare: bares/Renttik-mobile.git
    checkout: Renttik-mobile
    command: npx expo start --port 8081
    port: 8081
    depends_on:
      - server
    setup:
      copy_files:
        - from: files/.env.mobile
          to: .env
      env:
        EXPO_PUBLIC_API_URL: "http://localhost:8080"
        EXPO_PORT: "8081"
      scripts:
        - name: "Install Mobile Dependencies"
          command: "npm install"
```

---

## 🧩 Schema Field Reference

### Top-Level Fields

| Field           | Type                    | Description                                                                                 |
| :-------------- | :---------------------- | :------------------------------------------------------------------------------------------ |
| `env`           | `dict[str, str]`        | Global public environment variables synchronized in blueprint revisions.                    |
| `secret`        | `dict[str, str]`        | Global sensitive secrets encrypted with AES-256-GCM in wshub Vault.                         |
| `private`       | `dict[str, str]`        | Global host-specific variables that **never leave the local machine**.                      |
| `setup.scripts` | `list[ScriptSpec]`      | Global setup scripts executed in the workspace root directory.                              |
| `repositories`  | `dict[str, RepoConfig]` | Map of repository definitions keyed by repository alias (`server`, `mobile`, etc.).         |

---

### Environment Variable Tiers & Scoping

`ws` supports three tiers of environment variables to guarantee Zero-Git secrets and local machine isolation:

1. **`public` (Default in `env:`)**: Standard non-sensitive configuration (ports, URLs, feature flags) synchronized in plaintext blueprint revisions.
2. **`secret` (in `secret:` block or `secret:<value>` prefix)**: Sensitive credentials (tokens, passwords, API keys) automatically stripped from the public blueprint, encrypted with **AES-256-GCM** in the wshub Vault, and re-hydrated on clone/sync.
3. **`private` (in `private:` block or `private:<value>` prefix)**: Developer-only or machine-specific overrides (local tool paths, hardware IPs) that **never leave the local machine**.

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
      PORT: "8080"
      DATABASE_PASSWORD: "secret:postgres_super_pass"  # Encrypted in Vault
      DEBUG_CACHE: "private:/tmp/my-server-cache"       # Stays local only
```

---

### `RepoConfig` Fields

| Field              | Type                 | Required | Description                                                                                        |
| :----------------- | :------------------- | :------- | :------------------------------------------------------------------------------------------------- |
| `bare`             | `string`             | **Yes**  | Relative or absolute path to the bare Git repository (e.g. `bares/server.git`).                    |
| `checkout`         | `string`             | **Yes**  | Subdirectory name where the worktree is checked out inside each workspace (e.g. `Renttik-server`). |
| `command`          | `string`             | No       | Service launch command (e.g. `npm run dev`, `cargo run`).                                          |
| `port`             | `integer`            | No       | Network port the service listens on (used in `ws info` inspection and process health monitoring).  |
| `depends_on`       | `list[str]`          | No       | List of service aliases that must start before this service.                                       |
| `setup.copy_files` | `list[FileCopySpec]` | No       | File copy specifications to execute during `ws setup` or workspace creation.                       |
| `setup.env`        | `dict[str, str]`     | No       | Environment variables specific to this repository worktree.                                        |
| `setup.scripts`    | `list[ScriptSpec]`   | No       | Setup commands executed sequentially inside the repository worktree directory.                     |

---

## 🔄 Dynamic Variable Interpolation & Service Discovery

`ws` provides dynamic template variables and cross-service discovery placeholders inside commands, environment variable values, and setup scripts:

### Standard Template Variables

| Variable                   | Description                                       | Example Value                                   |
| :------------------------- | :------------------------------------------------ | :---------------------------------------------- |
| `${WORKSPACE_NAME}`        | Name of the active workspace                      | `feat-auth`                                     |
| `${WORKSPACE_SLOT}`        | Deterministic integer slot index of the workspace | `0`, `1`, `2`                                   |
| `${WORKSPACE_PATH}`        | Absolute path to the workspace root               | `/path/to/workspaces/@feat-auth`                |
| `${REPO_NAME}`             | Alias of the target repository                    | `server`                                        |
| `${REPO_PATH}`             | Absolute path to the repository worktree          | `/path/to/workspaces/@feat-auth/Renttik-server` |
| `${LAN_IP}`                | Auto-detected host LAN Wi-Fi IP address           | `192.168.1.45`                                  |
| `${PUBLIC_HOST}`           | Configured public or tunnel hostname              | `myproject.loca.lt`                             |
| `${ENV:VAR_NAME:-default}` | Host environment variable with optional fallback  | `${ENV:API_KEY:-dev_key}`                       |
| `${PORT:3000}`             | Dynamic port offset (`3000 + slot * 10`)          | `3000`, `3010`, `3020`                          |

---

### 🌐 Cross-Service Discovery Placeholders

Services running in the same workspace can reference sibling services without hardcoding ports or IP addresses:

| Placeholder                    | Target Scope        | Example Value (Slot 1)           | Best For                                               |
| :----------------------------- | :------------------ | :------------------------------- | :----------------------------------------------------- |
| `${SERVICE_PORT:server}`       | Dynamic Port        | `8090`                           | Injecting target port into configs or CLI flags.       |
| `${SERVICE_URL:server}`        | Localhost URL       | `http://127.0.0.1:8090`          | Local intra-machine communication (web ➔ API).         |
| `${SERVICE_URL_LAN:server}`    | LAN Wi-Fi URL       | `http://192.168.1.45:8090`       | Physical mobile devices (Expo/React Native on phones). |
| `${SERVICE_URL_PUBLIC:server}` | Public / Tunnel URL | `https://myproject.loca.lt:8090` | External webhooks, OAuth callbacks, remote staging.    |

### Concrete Cross-Service Example

```yaml
repositories:
  server:
    bare: bares/server.git
    checkout: server
    port: 8080
    command: npm run dev -- --port ${PORT:8080}

  mobile:
    bare: bares/mobile.git
    checkout: mobile
    port: 8081
    command: npx expo start --port ${PORT:8081}
    depends_on:
      - server
    setup:
      env:
        # Physical phone automatically talks to the backend over Wi-Fi:
        EXPO_PUBLIC_API_URL: "${SERVICE_URL_LAN:server}"
        EXPO_PUBLIC_LOCAL_API_URL: "${SERVICE_URL:server}"
```

---

### 💉 Auto-Injected Runtime Discovery Variables

When `ws` launches any service or interactive subshell (`ws shell @name %repo`), it automatically injects discovery variables for all services in the workspace:

- `WS_WORKSPACE`: Active workspace name (`feat-auth`).
- `WS_SLOT`: Workspace integer slot (`1`).
- `WS_LAN_IP`: Host LAN IP (`192.168.1.45`).
- `WS_SERVICE_<NAME>_PORT`: Resolved port of target service (e.g. `WS_SERVICE_SERVER_PORT=8090`).
- `WS_SERVICE_<NAME>_URL`: Base localhost URL (e.g. `WS_SERVICE_SERVER_URL=http://127.0.0.1:8090`).
- `WS_SERVICE_<NAME>_URL_LAN`: Base LAN URL (e.g. `WS_SERVICE_SERVER_URL_LAN=http://192.168.1.45:8090`).

---

### 📶 Wireless (Wi-Fi) Adapter Prioritization & Interface Selection

To ensure physical mobile devices running client applications (e.g. Expo / React Native on Android & iOS) can reliably connect to local API backend services, `ws` employs smart network adapter prioritization:

1. **Active Wi-Fi Adapter (Default)**: `ws` scans physical network interfaces (`/sys/class/net/*/wireless`, `wlan*`, `wl*`, `wifi*`) and prioritizes active wireless IPv4 addresses.
2. **Ethernet Fallback**: If no active Wi-Fi interface is detected, physical Ethernet adapters (`eno*`, `eth*`, `enp*`) are selected.
3. **CLI Interface Flag**: You can select a specific interface or adapter type using `--interface <name|type>` (or `--iface`, `--lan-interface`):
   ```bash
   # Select Ethernet explicitly:
   ws start @develop --interface eno1
   # Select Wi-Fi explicitly:
   ws start @develop --interface wifi
   ```
4. **Explicit IP Override**: You can override the host IP directly using `--ip <ip>` (or `--lan-ip`) or the `WS_LAN_IP` environment variable:
   ```bash
   ws start @develop --ip 192.168.1.55
   ```

### 📄 Live Service Registry File (`.ws/services.json`)

On startup, `ws` writes a machine-readable discovery descriptor to `workspaces/@<name>/.ws/services.json`:

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
      "url_local": "http://127.0.0.1:8090",
      "url_lan": "http://192.168.1.45:8090",
      "url_public": "https://myproject.loca.lt:8090",
      "status": "running"
    }
  }
}
```

---

## 🔒 Secret Masking

When inspecting environment variables using `ws env` or printing debug logs, `ws` automatically masks sensitive values containing keywords such as:

- `SECRET`, `PASSWORD`, `KEY`, `TOKEN`, `CREDENTIAL`, `PRIVATE`, `AUTH`

Masked output in terminal:

```
DATABASE_URL: postgresql://postgres:postgres@localhost:5432/db_develop
JWT_SECRET: ********************
```

---

## 📋 Declarative `workspace.yml` Spec Files

In addition to CLI arguments, you can define workspaces declaratively in a YAML file for team consistency and CI pipelines:

```yaml
# feature-auth.yml
name: feat-auth
description: "Authentication and user session revamp"

repositories:
  server:
    branch: feature/auth-api
    create: true
  mobile:
    branch: feature/auth-ui
    create: true
  frontend:
    branch: main
    create: false
    locked: true # Automatically lock tracked files as read-only
```

Create the workspace directly from the file:

```bash
ws create -f feature-auth.yml --setup
```
