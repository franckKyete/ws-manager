# 🌲 Worktree & Multi-Repository Git Management

Managing feature branches, code synchronization, and read-only worktree protections across multiple Git repositories is seamless with `ws`.

---

## 🌿 Feature Branching in Polyrepos

In multi-repository architectures, a single feature often requires synchronized changes across several repositories (e.g. backend API endpoints, mobile screens, shared TypeScript contracts).

`ws` provides three flexible branching patterns during workspace creation:

### 1. Unified Feature Branches across All Repositories
```bash
ws create @feat-oauth %server %mobile
```
- Creates `feature/feat-oauth` in `%server` and `%mobile`.
- Both worktrees are checked out under `workspaces/@feat-oauth/`.

---

### 2. Mixed Mode (Existing Branches + New Branches)
```bash
ws create @feat-oauth %server:main:existing %mobile:feature/auth-screen:new
```
- `%server` checks out existing `main` without creating a new branch.
- `%mobile` creates and checks out a new branch `feature/auth-screen`.

---

### 3. Full Project Synchronization
```bash
ws create @prod-debug --all --existing
```
- Checks out existing branches matching `@prod-debug` or default branches across every configured repository.

---

## 🔒 Tracked File Locking (`ws repo lock` / `ws lock`)

When working on a frontend or mobile feature, you may need a running backend server without wanting to accidentally edit backend code.

### The Problem with Traditional Worktrees
If you keep a backend repository in your editor workspace, accidental keystrokes or refactoring tools (e.g. IDE rename symbol) can modify backend files unintentionally.

### The `ws` Locking Solution
`ws lock` sets write permissions on all **Git-tracked files** to read-only (`chmod a-w` via `git ls-files`):

```bash
ws repo lock @develop %server
# Or using the direct shortcut:
ws lock @develop %server
```

### Why This Is Safe:
- **Git-Tracked Files**: Set to read-only (`r--r--r--`). Your editor will prevent saving edits.
- **Untracked & Build Files**: Directories such as `node_modules/`, `target/`, `dist/`, `.env`, and build caches **remain writable**.
- **Services Continue Running**: Build tools and compilers continue generating artifacts without error.
- **Git Operations Protected**: `ws push` automatically skips locked repositories so you never accidentally push unwanted changes.

### Unlocking a Worktree
When you need to make changes again:
```bash
ws repo unlock @develop %server
# Or shortcut:
ws unlock @develop %server
```

---

## 📊 Cross-Repository Status (`ws status`)

View unified Git status across all workspace worktrees:

```bash
ws status @develop
```

Output:
```
┏━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━┓
┃ Repository     ┃ Current Branch    ┃ Working Tree     ┃ Sync Status    ┃
┡━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━┩
│ Renttik-server │ feature/auth      │ [green]Clean[/green]     │ Up to date     │
│ Renttik-mobile │ feature/auth-ui   │ [yellow]2 modified[/yellow] │ 1 ahead        │
└────────────────┴───────────────────┴──────────────────┴────────────────┘
```

---

## ⬆️ Safe Multi-Repo Push (`ws push`)

Push committed changes across workspace repositories to their respective remotes:

```bash
# Push all active repositories:
ws push @develop

# Push specific repositories:
ws push @develop %mobile

# Target a specific remote:
ws push @develop --remote upstream
```

### Safety Guarantees:
- **Never Auto-Commits**: `ws` never stages or commits uncommitted changes.
- **Never Force Pushes**: Standard `git push` is executed safely.
- **Skips Locked Repositories**: Any repository marked `LOCKED` is automatically skipped.

---

## ⬇️ Smart Multi-Repo Pull (`ws pull`)

Pull upstream updates across all repositories in your workspace:

```bash
ws pull @develop
```

If a conflict, dirty worktree, or network error occurs in any repository, `ws` displays a structured diagnostic report showing the exact error and guidance for resolution.

---

## ➕ Adding and Removing Repositories Dynamically

You don't need to recreate your workspace to add or remove a repository.

### Adding a Repository:
```bash
# Add a repository with a new branch:
ws repo add @develop %web:feature/auth-page

# Add a repository on an existing branch:
ws repo add @develop %web:main --existing
```

### Removing a Repository:
```bash
# Remove worktree from workspace:
ws repo remove @develop %web

# Also delete the Git branch from the bare store:
ws repo remove @develop %web --delete-branch
```
