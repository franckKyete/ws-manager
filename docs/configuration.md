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

| Field | Type | Description |
| :--- | :--- | :--- |
| `env` | `dict[str, str]` | Global environment variables injected into all workspace commands, subshells, and services. |
| `setup.scripts` | `list[ScriptSpec]` | Global setup scripts executed in the workspace root directory. |
| `repositories` | `dict[str, RepoConfig]` | Map of repository definitions keyed by repository alias (`server`, `mobile`, etc.). |

---

### `RepoConfig` Fields

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `bare` | `string` | **Yes** | Relative or absolute path to the bare Git repository (e.g. `bares/server.git`). |
| `checkout` | `string` | **Yes** | Subdirectory name where the worktree is checked out inside each workspace (e.g. `Renttik-server`). |
| `command` | `string` | No | Service launch command (e.g. `npm run dev`, `cargo run`). |
| `port` | `integer` | No | Network port the service listens on (used in `ws info` inspection and process health monitoring). |
| `depends_on` | `list[str]` | No | List of service aliases that must start before this service. |
| `setup.copy_files` | `list[FileCopySpec]` | No | File copy specifications to execute during `ws setup` or workspace creation. |
| `setup.env` | `dict[str, str]` | No | Environment variables specific to this repository worktree. |
| `setup.scripts` | `list[ScriptSpec]` | No | Setup commands executed sequentially inside the repository worktree directory. |

---

## 🔄 Dynamic Variable Interpolation

`ws` supports dynamic template variables inside commands, environment variable values, and setup scripts:

| Variable | Description | Example Value |
| :--- | :--- | :--- |
| `${WORKSPACE_NAME}` | Name of the active workspace | `feat-auth` |
| `${WORKSPACE_PATH}` | Absolute path to the workspace root | `/home/user/project/workspaces/@feat-auth` |
| `${REPO_NAME}` | Alias of the target repository | `server` |
| `${REPO_PATH}` | Absolute path to the repository worktree | `/home/user/project/workspaces/@feat-auth/Renttik-server` |
| `${ENV:VAR_NAME}` | Host environment variable value with optional default fallback | `${ENV:PORT:-3000}` |

### Example
```yaml
env:
  DATABASE_URL: "postgresql://localhost:5432/app_${WORKSPACE_NAME}"
  STORAGE_DIR: "${WORKSPACE_PATH}/shared_storage"
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
    locked: true   # Automatically lock tracked files as read-only
```

Create the workspace directly from the file:
```bash
ws create -f feature-auth.yml --setup
```
