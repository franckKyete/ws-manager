use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::errors::{Result, WSError};

#[derive(Debug, Clone, Default)]
pub struct UncommittedChanges {
    pub has_uncommitted: bool,
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GitService {
    pub timeout_secs: u64,
}

impl Default for GitService {
    fn default() -> Self {
        Self::new(60)
    }
}

impl GitService {
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }

    pub fn run_raw(&self, args: &[&str], cwd: Option<&Path>) -> std::io::Result<std::process::Output> {
        let mut cmd = Command::new("git");
        cmd.args(args);
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        cmd.output()
    }

    pub fn run_git(&self, args: &[&str], cwd: Option<&Path>, check: bool) -> Result<std::process::Output> {
        let mut cmd = Command::new("git");
        cmd.args(args);
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        let output = cmd.output().map_err(|e| WSError::Git {
            message: format!("Failed to spawn git command: {}", e),
            command: Some(args.join(" ")),
            returncode: None,
            stderr: None,
        })?;

        if check && !output.status.success() {
            let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let err_msg = if !stderr_str.is_empty() {
                stderr_str.clone()
            } else {
                stdout_str
            };
            return Err(WSError::Git {
                message: err_msg,
                command: Some(args.join(" ")),
                returncode: output.status.code(),
                stderr: Some(stderr_str),
            });
        }
        Ok(output)
    }

    pub fn is_git_installed(&self) -> bool {
        self.run_git(&["--version"], None, false).map_or(false, |out| out.status.success())
    }

    pub fn clone_bare(&self, url: &str, target_bare_path: &Path) -> Result<()> {
        self.run_git(&["clone", "--bare", url, &target_bare_path.to_string_lossy()], None, true)?;
        let bare_str = target_bare_path.to_string_lossy();
        let _ = self.run_git(
            &["--git-dir", &bare_str, "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
            None,
            false,
        );
        Ok(())
    }

    pub fn is_bare_repo(&self, bare_path: &Path) -> bool {
        if !bare_path.exists() {
            return false;
        }
        let bare_str = bare_path.to_string_lossy();
        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "rev-parse", "--is-bare-repository"], None, false) {
            String::from_utf8_lossy(&out.stdout).trim() == "true"
        } else {
            false
        }
    }

    pub fn branch_exists(&self, bare_path: &Path, branch: &str) -> bool {
        let bare_str = bare_path.to_string_lossy();
        let ref_str = format!("refs/heads/{}", branch);
        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "rev-parse", "--verify", "--quiet", &ref_str], None, false) {
            out.status.success()
        } else {
            false
        }
    }

    pub fn get_default_branch_or_head(&self, bare_path: &Path) -> Option<String> {
        let bare_str = bare_path.to_string_lossy();
        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "rev-parse", "--verify", "--quiet", "HEAD"], None, false) {
            if out.status.success() {
                return Some("HEAD".to_string());
            }
        }

        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "branch", "--format=%(refname:short)"], None, false) {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    let b = line.trim();
                    if !b.is_empty() {
                        let ref_str = format!("refs/heads/{}", b);
                        let _ = self.run_git(&["--git-dir", &bare_str, "symbolic-ref", "HEAD", &ref_str], None, false);
                        return Some(b.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn create_worktree(
        &self,
        bare_path: &Path,
        worktree_path: &Path,
        branch: &str,
        create_branch: bool,
        start_point: Option<&str>,
    ) -> Result<()> {
        let bare_str = bare_path.to_string_lossy();
        let wt_str = worktree_path.to_string_lossy();

        if create_branch {
            let resolved_start = match start_point {
                Some(s) => Some(s.to_string()),
                None => self.get_default_branch_or_head(bare_path),
            };
            let mut args = vec!["--git-dir", &bare_str, "worktree", "add", "-b", branch, &wt_str];
            if let Some(ref st) = resolved_start {
                args.push(st);
            }
            self.run_git(&args, None, true)?;
        } else {
            let args = vec!["--git-dir", &bare_str, "worktree", "add", &wt_str, branch];
            self.run_git(&args, None, true)?;
        }
        Ok(())
    }

    pub fn remove_worktree(&self, bare_path: &Path, worktree_path: &Path, force: bool) -> Result<()> {
        let bare_str = bare_path.to_string_lossy();
        let wt_str = worktree_path.to_string_lossy();
        let mut args = vec!["--git-dir", &bare_str, "worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&wt_str);
        let _ = self.run_git(&args, None, false);
        Ok(())
    }

    pub fn delete_branch(&self, bare_path: &Path, branch: &str, force: bool) -> Result<()> {
        let bare_str = bare_path.to_string_lossy();
        let flag = if force { "-D" } else { "-d" };
        let args = vec!["--git-dir", &bare_str, "branch", flag, branch];
        let _ = self.run_git(&args, None, false);
        Ok(())
    }

    pub fn fetch_repo(&self, bare_path: &Path) -> Result<()> {
        let bare_str = bare_path.to_string_lossy();
        let _ = self.run_git(
            &["--git-dir", &bare_str, "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
            None,
            false,
        );
        self.run_git(&["--git-dir", &bare_str, "fetch", "--all"], None, true)?;
        Ok(())
    }

    pub fn get_remotes(&self, bare_path: &Path, worktree_path: Option<&Path>) -> Vec<String> {
        let bare_str = bare_path.to_string_lossy();
        let out = if let Some(wt) = worktree_path {
            if wt.is_dir() {
                self.run_git(&["remote"], Some(wt), false)
            } else {
                self.run_git(&["--git-dir", &bare_str, "remote"], None, false)
            }
        } else {
            self.run_git(&["--git-dir", &bare_str, "remote"], None, false)
        };

        if let Ok(res) = out {
            String::from_utf8_lossy(&res.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn ref_exists(&self, bare_path: &Path, ref_name: &str, worktree_path: Option<&Path>) -> bool {
        let bare_str = bare_path.to_string_lossy();
        let out = if let Some(wt) = worktree_path {
            if wt.is_dir() {
                self.run_git(&["rev-parse", "--verify", "--quiet", ref_name], Some(wt), false)
            } else {
                self.run_git(&["--git-dir", &bare_str, "rev-parse", "--verify", "--quiet", ref_name], None, false)
            }
        } else {
            self.run_git(&["--git-dir", &bare_str, "rev-parse", "--verify", "--quiet", ref_name], None, false)
        };
        out.map_or(false, |r| r.status.success())
    }

    pub fn fetch_remote_branch(
        &self,
        bare_path: &Path,
        branch: &str,
        remote: &str,
        worktree_path: Option<&Path>,
    ) -> bool {
        let bare_str = bare_path.to_string_lossy();
        let clean_branch = branch
            .trim_start_matches("refs/heads/")
            .trim_start_matches("refs/remotes/")
            .trim_start_matches(&format!("{}/", remote));

        let cfg_key = format!("remote.{}.fetch", remote);
        let cfg_val = format!("+refs/heads/*:refs/remotes/{}/*", remote);
        let _ = self.run_git(&["--git-dir", &bare_str, "config", &cfg_key, &cfg_val], None, false);

        let refspec = format!("+refs/heads/{}:refs/remotes/{}/{}", clean_branch, remote, clean_branch);
        let out = if let Some(wt) = worktree_path {
            if wt.is_dir() {
                self.run_git(&["fetch", remote, &refspec], Some(wt), false)
            } else {
                self.run_git(&["--git-dir", &bare_str, "fetch", remote, &refspec], None, false)
            }
        } else {
            self.run_git(&["--git-dir", &bare_str, "fetch", remote, &refspec], None, false)
        };
        out.map_or(false, |r| r.status.success())
    }

    pub fn prune_worktrees(&self, bare_path: &Path) -> Result<()> {
        let bare_str = bare_path.to_string_lossy();
        self.run_git(&["--git-dir", &bare_str, "worktree", "prune"], None, true)?;
        Ok(())
    }

    pub fn get_status(&self, worktree_path: &Path) -> String {
        if let Ok(out) = self.run_git(&["status", "--short"], Some(worktree_path), false) {
            String::from_utf8_lossy(&out.stdout).to_string()
        } else {
            "status unavailable".to_string()
        }
    }

    pub fn get_current_branch(&self, worktree_path: &Path) -> String {
        if let Ok(out) = self.run_git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(worktree_path), false) {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            "unknown".to_string()
        }
    }

    pub fn get_head_commit(&self, worktree_path: &Path) -> Option<String> {
        if let Ok(out) = self.run_git(&["rev-parse", "HEAD"], Some(worktree_path), false) {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }

    pub fn list_worktrees(&self, bare_path: &Path) -> Vec<(String, String)> {
        let bare_str = bare_path.to_string_lossy();
        let mut worktrees = Vec::new();
        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "worktree", "list", "--porcelain"], None, false) {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut curr_path = String::new();
            let mut curr_branch = String::new();

            for line in stdout.lines() {
                let l = line.trim();
                if let Some(p) = l.strip_prefix("worktree ") {
                    curr_path = p.to_string();
                } else if let Some(b) = l.strip_prefix("branch ") {
                    curr_branch = b.trim_start_matches("refs/heads/").to_string();
                } else if l.is_empty() && !curr_path.is_empty() {
                    worktrees.push((
                        curr_path.clone(),
                        if curr_branch.is_empty() { "detached".to_string() } else { curr_branch.clone() },
                    ));
                    curr_path.clear();
                    curr_branch.clear();
                }
            }
            if !curr_path.is_empty() {
                worktrees.push((
                    curr_path,
                    if curr_branch.is_empty() { "detached".to_string() } else { curr_branch },
                ));
            }
        }
        worktrees
    }

    pub fn push_branch(&self, worktree_path: &Path, remote: &str, branch: Option<&str>) -> Result<(bool, String)> {
        let mut args = vec!["push", remote];
        if let Some(b) = branch {
            args.push(b);
        }
        let out = self.run_git(&args, Some(worktree_path), false)?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let combined = format!("{}\n{}", stderr, stdout);

        if !out.status.success() {
            return Err(WSError::Git {
                message: format!("Git push failed: {}", combined.trim()),
                command: Some(args.join(" ")),
                returncode: out.status.code(),
                stderr: Some(stderr.to_string()),
            });
        }

        if combined.contains("Everything up-to-date") || combined.contains("Everything up to date") {
            Ok((false, "up to date (no new commits)".to_string()))
        } else {
            Ok((true, "successfully pushed committed changes".to_string()))
        }
    }

    pub fn pull_branch(&self, worktree_path: &Path, remote: &str, branch: Option<&str>) -> Result<(bool, String)> {
        let mut args = vec!["pull", remote];
        if let Some(b) = branch {
            args.push(b);
        }
        let out = self.run_git(&args, Some(worktree_path), false)?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let combined = format!("{}\n{}", stderr, stdout);

        if !out.status.success() {
            let first_err = stderr.lines().next().or_else(|| stdout.lines().next()).unwrap_or("Git pull failed");
            return Err(WSError::Git {
                message: format!("Git pull failed: {}", first_err),
                command: Some(args.join(" ")),
                returncode: out.status.code(),
                stderr: Some(stderr.to_string()),
            });
        }

        if combined.contains("Already up to date") || combined.contains("Already up-to-date") {
            Ok((false, "Already up to date".to_string()))
        } else {
            Ok((true, "successfully pulled updates".to_string()))
        }
    }

    pub fn get_remote_url(&self, worktree_path: &Path, remote: &str) -> String {
        if let Ok(out) = self.run_git(&["remote", "get-url", remote], Some(worktree_path), false) {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            "unknown".to_string()
        }
    }

    pub fn list_tracked_files(&self, worktree_path: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(out) = self.run_git(&["ls-files"], Some(worktree_path), false) {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let l = line.trim();
                if !l.is_empty() {
                    let fp = worktree_path.join(l);
                    if fp.is_file() {
                        files.push(fp);
                    }
                }
            }
        }
        files
    }

    pub fn set_tracked_files_readonly(&self, worktree_path: &Path, readonly: bool) {
        let files = self.list_tracked_files(worktree_path);
        for file_path in files {
            if let Some(name) = file_path.file_name().and_then(|n| n.to_str()) {
                if readonly && (name == ".env" || name.starts_with(".env.")) {
                    continue;
                }
            }
            if let Ok(meta) = fs::metadata(&file_path) {
                let mode = meta.permissions().mode();
                let new_mode = if readonly {
                    mode & !(0o222)
                } else {
                    mode | 0o200
                };
                let _ = fs::set_permissions(&file_path, fs::Permissions::from_mode(new_mode));
            }
        }
    }

    pub fn get_uncommitted_diff(&self, worktree_path: &Path) -> String {
        if !worktree_path.exists() {
            return String::new();
        }
        let has_head = self.run_git(&["rev-parse", "--verify", "HEAD"], Some(worktree_path), false)
            .map_or(false, |r| r.status.success());

        let out = if has_head {
            self.run_git(&["diff", "--binary", "HEAD"], Some(worktree_path), false)
        } else {
            self.run_git(&["diff", "--binary"], Some(worktree_path), false)
        };

        out.map(|r| String::from_utf8_lossy(&r.stdout).to_string()).unwrap_or_default()
    }

    pub fn get_untracked_files(&self, worktree_path: &Path) -> Vec<String> {
        if !worktree_path.exists() {
            return Vec::new();
        }
        let mut files = Vec::new();
        if let Ok(out) = self.run_git(&["ls-files", "--others", "--exclude-standard"], Some(worktree_path), false) {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let l = line.trim();
                if !l.is_empty() && l != ".env" && !l.starts_with(".env.") {
                    files.push(l.to_string());
                }
            }
        }
        files
    }

    pub fn apply_patch(&self, worktree_path: &Path, patch_content: &str) -> bool {
        if !worktree_path.exists() || patch_content.trim().is_empty() {
            return true;
        }

        let mut child = match Command::new("git")
            .args(&["apply", "--whitespace=nowarn", "--allow-empty", "-"])
            .current_dir(worktree_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(patch_content.as_bytes());
        }

        if let Ok(status) = child.wait() {
            if status.success() {
                return true;
            }
        }

        // Fallback with 3way
        let mut child3way = match Command::new("git")
            .args(&["apply", "--3way", "--whitespace=nowarn", "-"])
            .current_dir(worktree_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };

        if let Some(mut stdin) = child3way.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(patch_content.as_bytes());
        }

        child3way.wait().map_or(false, |s| s.success())
    }

    pub fn check_worktree_uncommitted(&self, worktree_path: &Path) -> UncommittedChanges {
        if !worktree_path.is_dir() {
            return UncommittedChanges::default();
        }

        let out = match self.run_git(&["status", "--porcelain"], Some(worktree_path), false) {
            Ok(o) if o.status.success() => o,
            _ => return UncommittedChanges::default(),
        };

        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut modified = Vec::new();
        let mut untracked = Vec::new();

        for line in stdout.lines() {
            let l = line.trim();
            if l.len() < 3 {
                continue;
            }
            let status_code = &l[..2];
            let mut filename = l[3..].trim().to_string();
            if let Some(idx) = filename.find(" -> ") {
                filename = filename[idx + 4..].to_string();
            }

            if status_code.starts_with('?') {
                if filename != ".env" && !filename.starts_with(".env.") {
                    untracked.push(filename);
                }
            } else {
                modified.push(filename);
            }
        }

        let has_uncommitted = !modified.is_empty() || !untracked.is_empty();
        UncommittedChanges {
            has_uncommitted,
            modified,
            untracked,
        }
    }

    pub fn get_default_branch(&self, bare_path: &Path) -> String {
        let bare_str = bare_path.to_string_lossy();
        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "symbolic-ref", "--short", "HEAD"], None, false) {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }

        if self.branch_exists(bare_path, "main") {
            return "main".to_string();
        }
        if self.branch_exists(bare_path, "master") {
            return "master".to_string();
        }

        if let Some(fb) = self.get_default_branch_or_head(bare_path) {
            if fb != "HEAD" {
                return fb;
            }
        }

        if let Ok(out) = self.run_git(&["--git-dir", &bare_str, "branch", "--format=%(refname:short)"], None, false) {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let b = line.trim();
                if !b.is_empty() {
                    return b.to_string();
                }
            }
        }

        "main".to_string()
    }

    pub fn resolve_main_branch(&self, bare_path: &Path) -> String {
        for candidate in &["main", "develop", "master"] {
            if self.branch_exists(bare_path, candidate) {
                return candidate.to_string();
            }
            if self.ref_exists(bare_path, &format!("refs/remotes/origin/{}", candidate), None) {
                return candidate.to_string();
            }
        }
        self.get_default_branch(bare_path)
    }

    pub fn update_bare_branch(
        &self,
        bare_path: &Path,
        branch: &str,
        new_ref_or_commit: &str,
    ) -> Result<bool> {
        let clean_branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
        let target_ref = format!("refs/heads/{}", clean_branch);
        let bare_str = bare_path.to_string_lossy().to_string();
        let res = self.run_git(&["--git-dir", &bare_str, "update-ref", &target_ref, new_ref_or_commit], None, false);
        Ok(res.map(|o| o.status.success()).unwrap_or(false))
    }

    pub fn get_branch_divergence(
        &self,
        bare_path: &Path,
        branch: &str,
        remote: &str,
        worktree_path: Option<&Path>,
    ) -> (i64, i64) {
        let bare_str = bare_path.to_string_lossy();
        let clean_branch = branch
            .trim_start_matches("refs/heads/")
            .trim_start_matches("refs/remotes/")
            .trim_start_matches(&format!("{}/", remote));

        let local_ref = format!("refs/heads/{}", clean_branch);
        let remote_ref = format!("refs/remotes/{}/{}", remote, clean_branch);

        if !self.ref_exists(bare_path, &remote_ref, worktree_path) {
            return (0, 0);
        }

        if !self.ref_exists(bare_path, &local_ref, worktree_path) {
            let count_out = if let Some(wt) = worktree_path {
                self.run_git(&["rev-list", "--count", &remote_ref], Some(wt), false)
            } else {
                self.run_git(&["--git-dir", &bare_str, "rev-list", "--count", &remote_ref], None, false)
            };
            let behind = count_out
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<i64>().ok())
                .unwrap_or(1);
            return (0, behind);
        }

        let out = if let Some(wt) = worktree_path {
            self.run_git(
                &["rev-list", "--left-right", "--count", &format!("{}...{}", local_ref, remote_ref)],
                Some(wt),
                false,
            )
        } else {
            self.run_git(
                &["--git-dir", &bare_str, "rev-list", "--left-right", "--count", &format!("{}...{}", local_ref, remote_ref)],
                None,
                false,
            )
        };

        if let Ok(res) = out {
            let stdout = String::from_utf8_lossy(&res.stdout);
            let parts: Vec<&str> = stdout.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(ahead), Ok(behind)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
                    return (ahead, behind);
                }
            }
        }
        (0, 0)
    }

    pub fn is_branch_merged(
        &self,
        bare_path: &Path,
        branch: &str,
        target_branch: Option<&str>,
        worktree_path: Option<&Path>,
        prefer_remote: bool,
    ) -> (bool, String, usize) {
        let bare_str = bare_path.to_string_lossy();
        let exec = |args: &[&str]| -> Result<std::process::Output> {
            if let Some(wt) = worktree_path {
                if wt.is_dir() {
                    return self.run_git(args, Some(wt), false);
                }
            }
            let mut full = vec!["--git-dir", &bare_str];
            full.extend_from_slice(args);
            self.run_git(&full, None, false)
        };

        let raw_target = match target_branch {
            Some(t) => t.to_string(),
            None => self.get_default_branch(bare_path),
        };

        let clean_target = raw_target
            .trim_start_matches("refs/heads/")
            .trim_start_matches("refs/remotes/");
        let clean_branch = branch
            .trim_start_matches("refs/heads/")
            .trim_start_matches("refs/remotes/");

        let remotes = self.get_remotes(bare_path, worktree_path);
        let primary_remote = if remotes.contains(&"origin".to_string()) {
            Some("origin".to_string())
        } else {
            remotes.first().cloned()
        };

        let mut target_ref = None;
        let mut target_display = clean_target.to_string();

        if prefer_remote {
            if let Some(ref rem) = primary_remote {
                let rem_prefix = format!("{}/", rem);
                let rem_refs_prefix = format!("refs/remotes/{}/", rem);
                if raw_target.starts_with(&rem_prefix) || raw_target.starts_with(&rem_refs_prefix) {
                    let remote_branch_name = raw_target
                        .trim_start_matches(&rem_refs_prefix)
                        .trim_start_matches(&rem_prefix);
                    self.fetch_remote_branch(bare_path, remote_branch_name, rem, worktree_path);
                    let candidate_ref = format!("refs/remotes/{}/{}", rem, remote_branch_name);
                    if self.ref_exists(bare_path, &candidate_ref, worktree_path) {
                        target_ref = Some(candidate_ref);
                        target_display = format!("{}/{}", rem, remote_branch_name);
                    }
                } else {
                    self.fetch_remote_branch(bare_path, clean_target, rem, worktree_path);
                    let candidate_remote_ref = format!("refs/remotes/{}/{}", rem, clean_target);
                    if self.ref_exists(bare_path, &candidate_remote_ref, worktree_path) {
                        target_ref = Some(candidate_remote_ref);
                        target_display = format!("{}/{}", rem, clean_target);
                    }
                }
            }
        }

        let final_target_ref = match target_ref {
            Some(r) => r,
            None => {
                if raw_target.starts_with("refs/") || raw_target == "HEAD" {
                    raw_target.clone()
                } else {
                    format!("refs/heads/{}", clean_target)
                }
            }
        };

        let branch_ref = if branch.starts_with("refs/") || branch == "HEAD" {
            branch.to_string()
        } else if branch.contains('/') && remotes.iter().any(|r| branch.starts_with(&format!("{}/", r))) {
            format!("refs/remotes/{}", branch)
        } else {
            format!("refs/heads/{}", clean_branch)
        };

        if branch_ref == final_target_ref {
            return (true, target_display, 0);
        }

        // Tier 1: Direct ancestry check
        if let Ok(out) = exec(&["merge-base", "--is-ancestor", &branch_ref, &final_target_ref]) {
            if out.status.success() {
                return (true, target_display, 0);
            }
        }

        // Tier 2: Patch equivalence check via git cherry
        let mut plus_commits = Vec::new();
        let mut cherry_ok = false;
        if let Ok(out) = exec(&["cherry", &final_target_ref, &branch_ref]) {
            if out.status.success() {
                cherry_ok = true;
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    let l = line.trim();
                    if l.starts_with('+') {
                        plus_commits.push(l.to_string());
                    }
                }
                if plus_commits.is_empty() {
                    return (true, target_display, 0);
                }
            }
        }

        // Tier 3: Tree equivalence check via git merge-tree
        if let Ok(out) = exec(&["merge-tree", "--write-tree", &final_target_ref, &branch_ref]) {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let merged_tree = stdout.lines().next().unwrap_or("").trim();
                let tree_spec = format!("{}^{{tree}}", final_target_ref);
                if let Ok(target_out) = exec(&["rev-parse", &tree_spec]) {
                    if target_out.status.success() {
                        let target_tree = String::from_utf8_lossy(&target_out.stdout).trim().to_string();
                        if !merged_tree.is_empty() && merged_tree == target_tree {
                            return (true, target_display, 0);
                        }
                    }
                }
            }
        }

        let unmerged_count = if cherry_ok && !plus_commits.is_empty() {
            plus_commits.len()
        } else {
            let rev_range = format!("{}..{}", final_target_ref, branch_ref);
            if let Ok(out) = exec(&["rev-list", "--count", &rev_range]) {
                String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(1)
            } else {
                1
            }
        };

        (false, target_display, unmerged_count.max(1))
    }
}
