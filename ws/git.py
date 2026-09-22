"""Git abstraction service for executing git commands safely."""

import logging
import os
from pathlib import Path
import stat
import subprocess
from typing import Sequence

from ws.exceptions import GitException

logger = logging.getLogger("ws.git")


class GitService:
    """Service wrapping low-level Git CLI commands."""

    def __init__(self, timeout: float = 60.0):
        self.timeout = timeout

    def _run(
        self,
        args: Sequence[str],
        cwd: Path | str | None = None,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        """Execute a git command via subprocess."""
        cmd = ["git"] + list(args)
        logger.debug("Executing command: %s (cwd=%s)", " ".join(cmd), cwd)
        try:
            res = subprocess.run(
                cmd,
                cwd=cwd,
                capture_output=True,
                text=True,
                timeout=self.timeout,
            )
            if check and res.returncode != 0:
                raise GitException(
                    message=f"Git command failed with exit code {res.returncode}: {res.stderr.strip() or res.stdout.strip()}",
                    command=" ".join(cmd),
                    returncode=res.returncode,
                    stderr=res.stderr.strip(),
                )
            return res
        except subprocess.TimeoutExpired as e:
            raise GitException(
                message=f"Git command timed out after {self.timeout} seconds",
                command=" ".join(cmd),
            ) from e
        except FileNotFoundError as e:
            raise GitException(
                message="Git binary not found. Please ensure Git is installed and available in PATH.",
            ) from e

    def is_git_installed(self) -> bool:
        """Check if Git CLI is available in system PATH."""
        try:
            res = self._run(["--version"], check=False)
            return res.returncode == 0
        except Exception:
            return False

    def clone_bare(self, url: str, target_bare_path: Path) -> None:
        """Clone a remote repository as a bare git repository."""
        self._run(["clone", "--bare", url, str(target_bare_path)])
        self._run(
            ["--git-dir", str(target_bare_path), "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
            check=False,
        )

    def is_bare_repo(self, bare_path: Path) -> bool:
        """Check if specified path is a valid bare git repository."""
        if not bare_path.exists():
            return False
        res = self._run(["--git-dir", str(bare_path), "rev-parse", "--is-bare-repository"], check=False)
        return res.returncode == 0 and res.stdout.strip() == "true"

    def branch_exists(self, bare_path: Path, branch: str) -> bool:
        """Check if a branch exists in the bare repository."""
        res = self._run(
            ["--git-dir", str(bare_path), "rev-parse", "--verify", "--quiet", f"refs/heads/{branch}"],
            check=False,
        )
        return res.returncode == 0

    def get_default_branch_or_head(self, bare_path: Path) -> str | None:
        """Get valid branch/commit reference for creating new branches in bare repo."""
        res = self._run(
            ["--git-dir", str(bare_path), "rev-parse", "--verify", "--quiet", "HEAD"],
            check=False,
        )
        if res.returncode == 0:
            return "HEAD"

        # If HEAD is invalid/orphaned, check existing local branches in bare repo
        res_branches = self._run(
            ["--git-dir", str(bare_path), "branch", "--format=%(refname:short)"],
            check=False,
        )
        if res_branches.returncode == 0:
            branches = [b.strip() for b in res_branches.stdout.splitlines() if b.strip()]
            if branches:
                start = branches[0]
                # Fix HEAD symbolic ref for bare repo
                self._run(
                    ["--git-dir", str(bare_path), "symbolic-ref", "HEAD", f"refs/heads/{start}"],
                    check=False,
                )
                return start
        return None

    def create_worktree(
        self,
        bare_path: Path,
        worktree_path: Path,
        branch: str,
        create_branch: bool = True,
    ) -> None:
        """Create a new worktree from a bare repository.

        If create_branch is True, creates new branch `-b branch`.
        Otherwise checks out existing branch `branch`.
        """
        worktree_path_str = str(worktree_path)
        bare_path_str = str(bare_path)

        if create_branch:
            start_point = self.get_default_branch_or_head(bare_path)
            args = ["--git-dir", bare_path_str, "worktree", "add", "-b", branch, worktree_path_str]
            if start_point:
                args.append(start_point)
        else:
            args = ["--git-dir", bare_path_str, "worktree", "add", worktree_path_str, branch]

        self._run(args)


    def remove_worktree(self, bare_path: Path, worktree_path: Path, force: bool = True) -> None:
        """Remove a worktree associated with a bare repository."""
        args = ["--git-dir", str(bare_path), "worktree", "remove"]
        if force:
            args.append("--force")
        args.append(str(worktree_path))

        res = self._run(args, check=False)
        if res.returncode != 0:
            logger.warning(
                "Git worktree remove failed for '%s': %s",
                worktree_path,
                res.stderr.strip() or res.stdout.strip(),
            )

    def delete_branch(self, bare_path: Path, branch: str, force: bool = True) -> None:
        """Delete a branch from a bare repository."""
        flag = "-D" if force else "-d"
        args = ["--git-dir", str(bare_path), "branch", flag, branch]
        res = self._run(args, check=False)
        if res.returncode != 0:
            logger.warning(
                "Failed to delete branch '%s' in '%s': %s",
                branch,
                bare_path,
                res.stderr.strip(),
            )

    def fetch_repo(self, bare_path: Path) -> None:
        """Fetch updates in bare repository."""
        self._run(
            ["--git-dir", str(bare_path), "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
            check=False,
        )
        self._run(["--git-dir", str(bare_path), "fetch", "--all"])

    def get_remotes(self, bare_path: Path, worktree_path: Path | None = None) -> list[str]:
        """Get list of configured git remotes."""
        if worktree_path and worktree_path.exists():
            res = self._run(["remote"], cwd=worktree_path, check=False)
        else:
            res = self._run(["--git-dir", str(bare_path), "remote"], check=False)
        if res.returncode != 0:
            return []
        return [r.strip() for r in res.stdout.splitlines() if r.strip()]

    def ref_exists(
        self,
        bare_path: Path,
        ref: str,
        worktree_path: Path | None = None,
    ) -> bool:
        """Check if a git reference exists."""
        if worktree_path and worktree_path.exists():
            res = self._run(["rev-parse", "--verify", "--quiet", ref], cwd=worktree_path, check=False)
        else:
            res = self._run(["--git-dir", str(bare_path), "rev-parse", "--verify", "--quiet", ref], check=False)
        return res.returncode == 0

    def fetch_remote_branch(
        self,
        bare_path: Path,
        branch: str,
        remote: str = "origin",
        worktree_path: Path | None = None,
    ) -> bool:
        """Fetch updates for a target branch from remote into remote tracking ref.

        Returns True if fetch succeeded, False otherwise (e.g. offline, branch not on remote).
        """
        clean_branch = branch.removeprefix("refs/heads/").removeprefix("refs/remotes/").removeprefix(f"{remote}/")

        # Ensure fetch refspec is configured for future operations
        self._run(
            ["--git-dir", str(bare_path), "config", f"remote.{remote}.fetch", f"+refs/heads/*:refs/remotes/{remote}/*"],
            check=False,
        )

        refspec = f"+refs/heads/{clean_branch}:refs/remotes/{remote}/{clean_branch}"
        if worktree_path and worktree_path.exists():
            res = self._run(["fetch", remote, refspec], cwd=worktree_path, check=False)
        else:
            res = self._run(["--git-dir", str(bare_path), "fetch", remote, refspec], check=False)
        return res.returncode == 0

    def prune_worktrees(self, bare_path: Path) -> None:
        """Prune working tree information in bare repository."""
        self._run(["--git-dir", str(bare_path), "worktree", "prune"])

    def get_status(self, worktree_path: Path) -> str:
        """Get git status output inside worktree."""
        res = self._run(["status", "--short"], cwd=worktree_path, check=False)
        return res.stdout if res.returncode == 0 else "status unavailable"

    def get_current_branch(self, worktree_path: Path) -> str:
        """Get current checked out branch in worktree."""
        res = self._run(["rev-parse", "--abbrev-ref", "HEAD"], cwd=worktree_path, check=False)
        return res.stdout.strip() if res.returncode == 0 else "unknown"

    def list_worktrees(self, bare_path: Path) -> list[tuple[str, str]]:
        """List registered worktrees for a bare repository.

        Returns list of (path, branch/commit) tuples.
        """
        res = self._run(["--git-dir", str(bare_path), "worktree", "list", "--porcelain"], check=False)
        if res.returncode != 0:
            return []

        worktrees = []
        current_path = ""
        current_branch = ""

        for line in res.stdout.splitlines():
            line = line.strip()
            if line.startswith("worktree "):
                current_path = line[9:]
            elif line.startswith("branch "):
                current_branch = line[7:].replace("refs/heads/", "")
            elif line == "":
                if current_path:
                    worktrees.append((current_path, current_branch or "detached"))
                    current_path = ""
                    current_branch = ""

        if current_path:
            worktrees.append((current_path, current_branch or "detached"))

        return worktrees

    def push_branch(
        self,
        worktree_path: Path,
        remote: str = "origin",
        branch: str | None = None,
    ) -> tuple[bool, str]:
        """Push current committed branch of a worktree to remote (never force).

        Returns (was_pushed, message).
        """
        cmd = ["push", remote]
        if branch:
            cmd.append(branch)

        res = self._run(cmd, cwd=worktree_path, check=False)
        if res.returncode != 0:
            raise GitException(
                message=f"Git push failed: {res.stderr.strip() or res.stdout.strip()}",
                command=" ".join(["git"] + cmd),
                returncode=res.returncode,
                stderr=res.stderr.strip(),
            )

        output = (res.stderr + "\n" + res.stdout).strip()
        if "Everything up-to-date" in output or "Everything up to date" in output:
            return False, "up to date (no new commits)"

        return True, "successfully pushed committed changes"

    def pull_branch(
        self,
        worktree_path: Path,
        remote: str = "origin",
        branch: str | None = None,
    ) -> tuple[bool, str]:
        """Pull updates for current branch of a worktree from remote.

        Returns (was_updated, message).
        Raises GitException with detailed error message if git pull fails.
        """
        cmd = ["pull", remote]
        if branch:
            cmd.append(branch)

        res = self._run(cmd, cwd=worktree_path, check=False)
        output = (res.stderr + "\n" + res.stdout).strip()

        if res.returncode != 0:
            err_line = res.stderr.strip() or res.stdout.strip()
            first_err = err_line.splitlines()[0] if err_line else "Git pull failed"
            raise GitException(
                message=f"Git pull failed: {first_err}",
                command=" ".join(["git"] + cmd),
                returncode=res.returncode,
                stderr=res.stderr.strip(),
            )

        if "Already up to date" in output or "Already up-to-date" in output:
            return False, "Already up to date"

        return True, "successfully pulled updates"



    def get_remote_url(self, worktree_path: Path, remote: str = "origin") -> str:
        """Get remote URL for a worktree."""
        res = self._run(["remote", "get-url", remote], cwd=worktree_path, check=False)
        return res.stdout.strip() if res.returncode == 0 else "unknown"

    def list_tracked_files(self, worktree_path: Path) -> list[Path]:
        """Return paths of all git-tracked files in a worktree using git ls-files."""
        res = self._run(["ls-files"], cwd=worktree_path, check=False)
        if res.returncode != 0:
            return []
        files = []
        for line in res.stdout.splitlines():
            line = line.strip()
            if line:
                file_path = worktree_path / line
                if file_path.exists() and file_path.is_file():
                    files.append(file_path)
        return files

    def set_tracked_files_readonly(self, worktree_path: Path, readonly: bool) -> None:
        """chmod a-w (freeze) or u+w (unfreeze) on git-tracked files only."""
        tracked_files = self.list_tracked_files(worktree_path)
        for file_path in tracked_files:
            # Keep runtime env files writable even if tracked in git
            if readonly and (file_path.name == ".env" or file_path.name.startswith(".env.")):
                continue

            try:
                mode = file_path.stat().st_mode
                if readonly:
                    # Remove write bits for user, group, other
                    new_mode = mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH)
                else:
                    # Add write bit for user
                    new_mode = mode | stat.S_IWUSR
                os.chmod(file_path, new_mode)
            except Exception as e:
                logger.warning("Failed to change permission for '%s': %s", file_path, e)

    def get_uncommitted_diff(self, worktree_path: Path) -> str:
        """Get unified diff of all uncommitted (staged + unstaged) changes in worktree."""
        if not worktree_path.exists():
            return ""
        res_head = self._run(["rev-parse", "--verify", "HEAD"], cwd=worktree_path, check=False)
        if res_head.returncode == 0:
            res = self._run(["diff", "--binary", "HEAD"], cwd=worktree_path, check=False)
        else:
            res = self._run(["diff", "--binary"], cwd=worktree_path, check=False)
        return res.stdout if res.returncode == 0 else ""

    def get_untracked_files(self, worktree_path: Path) -> list[str]:
        """Return relative paths of untracked files (excluding gitignored)."""
        if not worktree_path.exists():
            return []
        res = self._run(["ls-files", "--others", "--exclude-standard"], cwd=worktree_path, check=False)
        if res.returncode != 0:
            return []
        files = []
        for line in res.stdout.splitlines():
            line = line.strip()
            if line:
                if line == ".env" or line.startswith(".env."):
                    continue
                files.append(line)
        return files

    def apply_patch(self, worktree_path: Path, patch_content: str) -> bool:
        """Apply unified diff patch to worktree."""
        if not worktree_path.exists() or not patch_content.strip():
            return True
        cmd = ["apply", "--whitespace=nowarn", "--allow-empty", "-"]
        try:
            res = subprocess.run(
                ["git"] + cmd,
                input=patch_content,
                cwd=worktree_path,
                capture_output=True,
                text=True,
                timeout=self.timeout,
            )
            if res.returncode != 0:
                res_3way = subprocess.run(
                    ["git", "apply", "--3way", "--whitespace=nowarn", "-"],
                    input=patch_content,
                    cwd=worktree_path,
                    capture_output=True,
                    text=True,
                    timeout=self.timeout,
                )
                if res_3way.returncode != 0:
                    logger.warning("Git apply patch failed in '%s': %s", worktree_path, res.stderr.strip() or res_3way.stderr.strip())
                    return False
            return True
        except Exception as e:
            logger.warning("Failed to apply patch in '%s': %s", worktree_path, e)
            return False

    def check_worktree_uncommitted(self, worktree_path: Path) -> dict[str, Any]:
        """Check if worktree has uncommitted changes (staged, unstaged, untracked).

        Returns:
            {
                "has_uncommitted": bool,
                "modified": list[str],
                "untracked": list[str],
            }
        """
        if not worktree_path.exists() or not worktree_path.is_dir():
            return {"has_uncommitted": False, "modified": [], "untracked": []}

        res = self._run(["status", "--porcelain"], cwd=worktree_path, check=False)
        if res.returncode != 0:
            return {"has_uncommitted": False, "modified": [], "untracked": []}

        modified: list[str] = []
        untracked: list[str] = []

        for line in res.stdout.splitlines():
            line_str = line.strip()
            if not line_str:
                continue
            status_code = line[:2]
            filename = line[3:].strip()
            if " -> " in filename:
                filename = filename.split(" -> ", 1)[1]

            if status_code == "??" or status_code.startswith("?"):
                if filename == ".env" or filename.startswith(".env."):
                    continue
                untracked.append(filename)
            else:
                modified.append(filename)

        has_uncommitted = bool(modified or untracked)
        return {
            "has_uncommitted": has_uncommitted,
            "modified": modified,
            "untracked": untracked,
        }

    def get_default_branch(self, bare_path: Path) -> str:
        """Resolve the default branch name of a bare repository."""
        # 1. Try symbolic-ref HEAD
        res = self._run(
            ["--git-dir", str(bare_path), "symbolic-ref", "--short", "HEAD"],
            check=False,
        )
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()

        # 2. Check main
        if self.branch_exists(bare_path, "main"):
            return "main"

        # 3. Check master
        if self.branch_exists(bare_path, "master"):
            return "master"

        # 4. Fallback to start point or any branch
        fallback = self.get_default_branch_or_head(bare_path)
        if fallback and fallback != "HEAD":
            return fallback

        res_branches = self._run(
            ["--git-dir", str(bare_path), "branch", "--format=%(refname:short)"],
            check=False,
        )
        if res_branches.returncode == 0 and res_branches.stdout.strip():
            lines = res_branches.stdout.splitlines()
            if lines:
                first_branch = lines[0].strip()
                if first_branch:
                    return first_branch

        return "main"

    def is_branch_merged(
        self,
        bare_path: Path,
        branch: str,
        target_branch: str | None = None,
        worktree_path: Path | None = None,
        prefer_remote: bool = True,
    ) -> tuple[bool, str, int]:
        """Check if branch is merged into target_branch.

        Always compares with the remote version of the target branch if a remote exists,
        fetching the latest changes from the remote to ensure up-to-date merge detection.

        Uses multi-tier detection to reliably handle:
        1. Direct ancestry (fast-forward or standard merge commits) via `merge-base --is-ancestor`.
        2. Patch equivalence (cherry-picks, rebase merges, PR merges with rewritten commit IDs) via `git cherry`.
        3. Tree equivalence (squash merges, single or multi-commit) via `git merge-tree --write-tree`.

        Returns (is_merged, resolved_target_branch, unmerged_commit_count).
        """
        def _exec(git_args: list[str]):
            if worktree_path and worktree_path.exists():
                return self._run(git_args, cwd=worktree_path, check=False)
            return self._run(["--git-dir", str(bare_path)] + git_args, check=False)

        raw_target = target_branch or self.get_default_branch(bare_path)
        clean_target = raw_target.removeprefix("refs/heads/").removeprefix("refs/remotes/")
        clean_branch = branch.removeprefix("refs/heads/").removeprefix("refs/remotes/")

        # Identify available remotes
        remotes = self.get_remotes(bare_path, worktree_path=worktree_path)
        primary_remote = "origin" if "origin" in remotes else (remotes[0] if remotes else None)

        target_ref = None
        target_display = clean_target

        if prefer_remote and primary_remote:
            # If raw_target is already a remote ref (e.g. origin/main or refs/remotes/origin/main)
            if raw_target.startswith(f"{primary_remote}/") or raw_target.startswith(f"refs/remotes/{primary_remote}/"):
                remote_branch_name = raw_target.removeprefix(f"refs/remotes/{primary_remote}/").removeprefix(f"{primary_remote}/")
                self.fetch_remote_branch(bare_path, remote_branch_name, remote=primary_remote, worktree_path=worktree_path)
                candidate_ref = f"refs/remotes/{primary_remote}/{remote_branch_name}"
                if self.ref_exists(bare_path, candidate_ref, worktree_path=worktree_path):
                    target_ref = candidate_ref
                    target_display = f"{primary_remote}/{remote_branch_name}"
            else:
                # Attempt to fetch remote target branch
                self.fetch_remote_branch(bare_path, clean_target, remote=primary_remote, worktree_path=worktree_path)
                candidate_remote_ref = f"refs/remotes/{primary_remote}/{clean_target}"
                if self.ref_exists(bare_path, candidate_remote_ref, worktree_path=worktree_path):
                    target_ref = candidate_remote_ref
                    target_display = f"{primary_remote}/{clean_target}"

        # If no remote target resolved, fall back to local target branch
        if not target_ref:
            if raw_target.startswith("refs/") or raw_target == "HEAD":
                target_ref = raw_target
            else:
                target_ref = f"refs/heads/{clean_target}"
            target_display = clean_target

        # Resolve branch ref
        if branch.startswith("refs/") or branch == "HEAD":
            branch_ref = branch
        elif "/" in branch and any(branch.startswith(f"{r}/") for r in remotes):
            branch_ref = f"refs/remotes/{branch}"
        else:
            branch_ref = f"refs/heads/{clean_branch}"

        # If checking exact same ref against itself
        if branch_ref == target_ref:
            return True, target_display, 0

        # 1. Tier 1: Direct ancestry check (fast-path for standard merge commits or fast-forwards)
        res_ancestor = _exec(["merge-base", "--is-ancestor", branch_ref, target_ref])
        if res_ancestor.returncode == 0:
            return True, target_display, 0

        # 2. Tier 2: Patch equivalence check via git cherry
        # git cherry outputs '-' for commits whose patch-id exists in target, and '+' for unmerged commits.
        res_cherry = _exec(["cherry", target_ref, branch_ref])
        plus_commits: list[str] = []
        if res_cherry.returncode == 0:
            cherry_lines = [line.strip() for line in res_cherry.stdout.splitlines() if line.strip()]
            plus_commits = [line for line in cherry_lines if line.startswith("+")]
            if not plus_commits:
                # All commits have an equivalent changeset in target (e.g. cherry-picked / PR rebase-merge)
                return True, target_display, 0

        # 3. Tier 3: Tree equivalence check via git merge-tree
        # Catches squash merges where multiple branch commits were squashed into a single commit in target,
        # or subsequent target commits that already incorporated all code changes from branch.
        res_merge_tree = _exec(["merge-tree", "--write-tree", target_ref, branch_ref])
        if res_merge_tree.returncode == 0 and res_merge_tree.stdout.strip():
            merged_tree = res_merge_tree.stdout.strip().splitlines()[0].strip()
            res_target_tree = _exec(["rev-parse", f"{target_ref}^{{tree}}"])
            if res_target_tree.returncode == 0:
                target_tree = res_target_tree.stdout.strip()
                if merged_tree and merged_tree == target_tree:
                    return True, target_display, 0

        # If not merged, calculate unmerged commit count
        if res_cherry.returncode == 0 and plus_commits:
            unmerged_count = len(plus_commits)
        else:
            res_count = _exec(["rev-list", "--count", f"{target_ref}..{branch_ref}"])
            if res_count.returncode == 0:
                try:
                    unmerged_count = int(res_count.stdout.strip())
                except ValueError:
                    unmerged_count = 1
            else:
                unmerged_count = 1

        return False, target_display, max(unmerged_count, 1)



