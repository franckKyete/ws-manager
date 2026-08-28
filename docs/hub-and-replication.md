# wshub & Replication Architecture

`wshub` is the centralized cloud and team collaboration hub for the `ws` multi-repository ecosystem. It bridges local developer workspaces with remote teams and allows seamless cross-machine replication without exposing secrets in Git repositories.

---

## Key Capabilities

1. **Zero-Git Vault (Envelope Encryption)**:
   - Secret environment variables (`.env`) and sensitive keys (`.pem`, `.json`, certificates) are stored in `wshub` using **AES-256-GCM envelope encryption** with project-specific HKDF keys derived from a master vault key.
   - **Secrets never enter Git commits or repositories**.

2. **Project Blueprint Registry & Versioning**:
   - Stores `repositories.yml` blueprints and automation scripts with linear versioning (`v1`, `v2`, ...).
   - Enables one-command project cloning: `ws clone <org/project>`.

3. **Cross-Machine Session Resumption**:
   - Snapshot active workspace branch checkouts, file locks, and configuration on Machine A: `ws hub state save @develop`.
   - Re-hydrate the exact same branch checkouts and worktrees on Machine B: `ws hub resume @develop`.

4. **Provider-Agnostic Backend (Clean Architecture)**:
   - Built with **Hono (TypeScript)** and **NestJS-style Clean Architecture** (Controllers, Services, Repositories, DI Container).
   - Hexagonal Ports & Adapters support **Cloudflare Workers / Pages (D1, R2, KV)** as well as **Self-Hosted Node.js / Docker (SQLite/Postgres, S3/Local Blob)**.

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

When you run `ws hub publish`, `ws` automatically performs 3-tier asset classification:
1. **Public variables** (`env:`) are preserved in the published blueprint.
2. **Secrets** (`secret:` block or `secret:<value>`) are masked with `"secret"` in the blueprint and automatically **encrypted with AES-256-GCM** into the wshub Vault.
3. **Private variables** (`private:` block or `private:<value>`) are **completely stripped** and never leave your local machine.
4. **Sensitive files** (`files/` directory or `copy_files:`) are **encrypted and uploaded** to the wshub encrypted blob store.

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

### 5. Resuming Work from Another Machine

```bash
# Machine A (before leaving):
ws hub state save @feature-checkout

# Machine B (at home/office):
ws hub resume @feature-checkout
```
