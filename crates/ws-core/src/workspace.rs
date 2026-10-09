use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::config::ConfigLoader;
use crate::env::EnvEngine;
use crate::errors::WSError;
use crate::git::GitService;
use crate::hub::HubClient;
use crate::models::{
    AppConfig, HubAutoSaveConfig, RepoConfig, RepoSpec, TmuxConfig, WorkspaceMetadata,
    WorkspacesSelector,
};
use crate::multiplexer::{TmuxLauncher, ZellijLauncher};
use crate::network::{allocate_workspace_ports, get_lan_ip, list_network_interfaces};
use crate::output::OutputHandler;
use crate::rollback::RollbackStack;
use crate::utils::{ensure_directory, get_iso_timestamp};

fn quote_shell(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn run_shell_command(cmd_str: &str, cwd: &Path, env: Option<&HashMap<String, String>>) -> i32 {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut cmd = Command::new(&shell);

    if shell.contains("zsh") {
        let script = format!(
            "setopt aliases 2>/dev/null; [ -f \"$ZDOTDIR/.zshrc\" ] && source \"$ZDOTDIR/.zshrc\" 2>/dev/null || [ -f \"$HOME/.zshrc\" ] && source \"$HOME/.zshrc\" 2>/dev/null; eval {}",
            quote_shell(cmd_str)
        );
        cmd.args(["-c", &script]);
    } else if shell.contains("bash") {
        let script = format!(
            "shopt -s expand_aliases 2>/dev/null; [ -f \"$HOME/.bashrc\" ] && source \"$HOME/.bashrc\" 2>/dev/null; eval {}",
            quote_shell(cmd_str)
        );
        cmd.args(["-c", &script]);
    } else {
        cmd.args(["-c", cmd_str]);
    }

    cmd.current_dir(cwd);
    if let Some(envs) = env {
        cmd.envs(envs);
    }

    cmd.status().map(|s| s.code().unwrap_or(1)).unwrap_or(1)
}

pub struct WorkspaceManager {
    pub config: AppConfig,
    pub git: GitService,
}

impl WorkspaceManager {
    pub fn new(config: AppConfig, git_service: Option<GitService>) -> Self {
        Self {
            config,
            git: git_service.unwrap_or_default(),
        }
    }

    pub fn resolve_bare_path(&self, bare: &Path) -> PathBuf {
        if bare.is_absolute() {
            bare.to_path_buf()
        } else {
            self.config.project_root.join(bare)
        }
    }

    pub fn get_workspace_dir(&self, name: &str) -> PathBuf {
        let clean = name.trim_start_matches('@');
        self.config.workspaces_dir.join(clean)
    }

    pub fn has_workspace(&self, name: &str) -> bool {
        let ws_dir = self.get_workspace_dir(name);
        ws_dir.exists()
            && (ws_dir.join("workspace.yml").exists() || ws_dir.join("workspace.yaml").exists())
    }

    pub fn detect_context(&self, cwd: Option<&Path>) -> (Option<String>, Option<String>) {
        let current = cwd
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let current_canon = current.canonicalize().unwrap_or(current.clone());
        let ws_dir_canon = self
            .config
            .workspaces_dir
            .canonicalize()
            .unwrap_or_else(|_| self.config.workspaces_dir.clone());

        // 1. Check if current is inside workspaces_dir
        if let Ok(rel) = current_canon.strip_prefix(&ws_dir_canon) {
            let mut parts = rel.iter();
            if let Some(ws_part) = parts.next() {
                let ws_name = ws_part
                    .to_string_lossy()
                    .trim_start_matches('@')
                    .to_string();
                if let Some(repo_part) = parts.next() {
                    let folder_name = repo_part.to_string_lossy().to_string();
                    for (r_name, r_cfg) in &self.config.repositories {
                        let checkout_name = Path::new(&r_cfg.checkout)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| r_name.clone());
                        if folder_name == *r_name || folder_name == checkout_name {
                            return (Some(ws_name), Some(r_name.clone()));
                        }
                    }
                    return (Some(ws_name), Some(folder_name));
                }
                return (Some(ws_name), None);
            }
        }

        // 2. Fallback: Search upward for workspace.yml
        let mut curr = current_canon.as_path();
        while let Some(parent) = curr.parent() {
            if curr.join("workspace.yml").exists() || curr.join("workspace.yaml").exists() {
                let ws_name = curr
                    .file_name()
                    .map(|n| n.to_string_lossy().trim_start_matches('@').to_string())
                    .unwrap_or_default();
                if current_canon != curr {
                    if let Ok(rel) = current_canon.strip_prefix(curr) {
                        if let Some(first) = rel.iter().next() {
                            let f_name = first.to_string_lossy().to_string();
                            for (r_name, r_cfg) in &self.config.repositories {
                                let checkout_name = Path::new(&r_cfg.checkout)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| r_name.clone());
                                if f_name == *r_name || f_name == checkout_name {
                                    return (Some(ws_name), Some(r_name.clone()));
                                }
                            }
                            return (Some(ws_name), Some(f_name));
                        }
                    }
                }
                return (Some(ws_name), None);
            }
            if curr == parent {
                break;
            }
            curr = parent;
        }

        (None, None)
    }

    pub fn validate_environment(&self) -> Result<(), WSError> {
        if !self.git.is_git_installed() {
            return Err(WSError::Validation(
                "Git is not installed or not available in PATH.".to_string(),
            ));
        }
        Ok(())
    }

    pub fn validate_repository_config(&self, repo_name: &str) -> Result<RepoConfig, WSError> {
        let repo_cfg = self.config.repositories.get(repo_name).ok_or_else(|| {
            WSError::RepositoryNotFound(format!(
                "Repository '{}' is not defined in configuration. Configured repositories: {}",
                repo_name,
                self.config
                    .repositories
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

        let bare_path = self.resolve_bare_path(&repo_cfg.bare);
        if !self.git.is_bare_repo(&bare_path) {
            return Err(WSError::RepositoryNotFound(format!(
                "Bare Git repository for '{}' not found or invalid at: {}",
                repo_name,
                bare_path.display()
            )));
        }

        let mut out_cfg = repo_cfg.clone();
        out_cfg.bare = bare_path;
        Ok(out_cfg)
    }

    pub fn validate_creation(&self, name: &str, repo_specs: &[RepoSpec]) -> Result<(), WSError> {
        self.validate_environment()?;
        let ws_dir = self.get_workspace_dir(name);
        if ws_dir.exists() {
            return Err(WSError::WorkspaceExists(format!(
                "Workspace directory already exists: {}",
                ws_dir.display()
            )));
        }

        for spec in repo_specs {
            let repo_cfg = self.validate_repository_config(&spec.name)?;
            let branch_exists = self.git.branch_exists(&repo_cfg.bare, &spec.branch);

            if spec.create && branch_exists {
                return Err(WSError::BranchAlreadyExists(format!(
                    "Cannot create branch '{}' for repository '{}': branch already exists in bare repo {}",
                    spec.branch,
                    spec.name,
                    repo_cfg.bare.display()
                )));
            }

            if !spec.create && !branch_exists {
                return Err(WSError::BranchNotFound(format!(
                    "Branch '{}' does not exist in repository '{}' ({})",
                    spec.branch,
                    spec.name,
                    repo_cfg.bare.display()
                )));
            }
        }
        Ok(())
    }

    pub fn prepare_and_sync_target_branch(
        &self,
        repo_name: &str,
        bare_path: &Path,
        target_branch: Option<&str>,
    ) -> Result<Option<String>, WSError> {
        let is_explicit = target_branch.map(|t| !t.trim().is_empty()).unwrap_or(false);
        let mut resolved_target = if is_explicit {
            target_branch.unwrap().trim().to_string()
        } else {
            self.git.resolve_main_branch(bare_path)
        };

        if resolved_target.starts_with("refs/heads/") {
            resolved_target = resolved_target
                .strip_prefix("refs/heads/")
                .unwrap()
                .to_string();
        }

        let remotes = self.git.get_remotes(bare_path, None);
        let primary_remote = if remotes.iter().any(|r| r == "origin") {
            Some("origin".to_string())
        } else {
            remotes.first().cloned()
        };

        if let Some(remote) = primary_remote {
            self.git
                .fetch_remote_branch(bare_path, &resolved_target, &remote, None);
            let remote_ref = format!("refs/remotes/{}/{}", remote, resolved_target);
            let has_remote = self.git.ref_exists(bare_path, &remote_ref, None);
            let has_local = self.git.branch_exists(bare_path, &resolved_target);

            if !has_remote && !has_local {
                if is_explicit {
                    return Err(WSError::Validation(format!(
                        "Target branch '{}' does not exist in repository '{}'",
                        resolved_target, repo_name
                    )));
                }
                return Ok(Some(self.git.get_default_branch(bare_path)));
            }

            if has_remote {
                let (ahead, behind) =
                    self.git
                        .get_branch_divergence(bare_path, &resolved_target, &remote, None);
                if ahead > 0 && behind > 0 {
                    return Err(WSError::Validation(format!(
                        "Repository '{}' branch '{}' has diverged from '{}/{}' ({} commit(s) ahead, {} commit(s) behind). Please resolve divergence before creating workspace.",
                        repo_name, resolved_target, remote, resolved_target, ahead, behind
                    )));
                } else if ahead == 0 && behind > 0 {
                    let worktrees = self.git.list_worktrees(bare_path);
                    let wt_match = worktrees.iter().find(|(_, b)| b == &resolved_target);
                    if let Some((wt_str, _)) = wt_match {
                        let wt_path = PathBuf::from(wt_str);
                        let uncommitted = self.git.check_worktree_uncommitted(&wt_path);
                        if uncommitted.has_uncommitted {
                            return Err(WSError::Validation(format!(
                                "Cannot pull latest changes for repository '{}' branch '{}': worktree at '{}' has uncommitted changes in tracked files. Please commit or stash changes before creating workspace.",
                                repo_name, resolved_target, wt_path.display()
                            )));
                        }
                        OutputHandler::print_info(&format!(
                            "Pulling latest changes for '{}' branch '{}' at '{}'...",
                            repo_name,
                            resolved_target,
                            wt_path.display()
                        ));
                        self.git
                            .pull_branch(&wt_path, &remote, Some(&resolved_target))?;
                    } else {
                        let updated = self.git.update_bare_branch(
                            bare_path,
                            &resolved_target,
                            &remote_ref,
                        )?;
                        if !updated {
                            return Err(WSError::Validation(format!(
                                "Failed to update branch '{}' in bare repository '{}' to '{}'.",
                                resolved_target, repo_name, remote_ref
                            )));
                        }
                    }
                }
            }
            Ok(Some(resolved_target))
        } else {
            if self.git.branch_exists(bare_path, &resolved_target) {
                return Ok(Some(resolved_target));
            }
            if is_explicit {
                return Err(WSError::Validation(format!(
                    "Target branch '{}' does not exist in repository '{}'",
                    resolved_target, repo_name
                )));
            }
            Ok(Some(self.git.get_default_branch(bare_path)))
        }
    }

    pub fn create_workspace(
        &self,
        name: &str,
        repo_specs: &[RepoSpec],
        tmux_cmd: Option<&str>,
        no_tmux: bool,
    ) -> Result<WorkspaceMetadata, WSError> {
        self.validate_creation(name, repo_specs)?;

        let mut resolved_bases = HashMap::new();
        for spec in repo_specs {
            let repo_cfg = self.validate_repository_config(&spec.name)?;
            let target_to_sync = if spec.create {
                spec.base_branch.as_deref()
            } else {
                Some(spec.branch.as_str())
            };
            let resolved_base =
                self.prepare_and_sync_target_branch(&spec.name, &repo_cfg.bare, target_to_sync)?;
            resolved_bases.insert(spec.name.clone(), resolved_base);
        }

        let ws_dir = self.get_workspace_dir(name);
        let mut rollback = RollbackStack::new();

        OutputHandler::print_creation_header(name, repo_specs);

        let ws_dir_clone = ws_dir.clone();
        rollback.add(
            format!("Remove directory {}", ws_dir.display()),
            Box::new(move || {
                let _ = std::fs::remove_dir_all(&ws_dir_clone);
            }),
        );
        ensure_directory(&ws_dir)?;

        let mut spec_dict = HashMap::new();

        for spec in repo_specs {
            let repo_cfg = self.validate_repository_config(&spec.name)?;
            let worktree_path = ws_dir.join(&repo_cfg.checkout);
            let start_point = if spec.create {
                resolved_bases.get(&spec.name).cloned().flatten()
            } else {
                None
            };

            self.git.create_worktree(
                &repo_cfg.bare,
                &worktree_path,
                &spec.branch,
                spec.create,
                start_point.as_deref(),
            )?;

            let b_path = repo_cfg.bare.clone();
            let wt_path = worktree_path.clone();
            let br_name = spec.branch.clone();
            let was_created = spec.create;

            let git_service = self.git.clone();
            rollback.add(
                format!("Remove worktree {}", wt_path.display()),
                Box::new(move || {
                    let _ = git_service.remove_worktree(&b_path, &wt_path, true);
                }),
            );

            if was_created {
                let b_path2 = repo_cfg.bare.clone();
                let git_service2 = self.git.clone();
                rollback.add(
                    format!("Delete created branch {} in {}", br_name, b_path2.display()),
                    Box::new(move || {
                        let _ = git_service2.delete_branch(&b_path2, &br_name, true);
                    }),
                );
            }

            if spec.frozen || spec.locked {
                self.git.set_tracked_files_readonly(&worktree_path, true);
            }

            spec_dict.insert(
                spec.name.clone(),
                RepoSpec {
                    name: spec.name.clone(),
                    branch: spec.branch.clone(),
                    create: spec.create,
                    path: repo_cfg.checkout.clone(),
                    frozen: spec.frozen || spec.locked,
                    locked: spec.frozen || spec.locked,
                    base_branch: if spec.create {
                        resolved_bases.get(&spec.name).cloned().flatten()
                    } else {
                        None
                    },
                },
            );
        }

        let metadata = WorkspaceMetadata {
            name: name.to_string(),
            created: get_iso_timestamp(),
            status: "active".to_string(),
            repositories: spec_dict,
        };

        self.save_metadata(&ws_dir, &metadata)?;

        // Open tmux workspace window if configured
        if let Some(tmux_cfg) = &self.config.tmux {
            if !no_tmux {
                let sess_name = &tmux_cfg.session;
                let win_cmd = tmux_cmd.or(tmux_cfg.command.as_deref());
                let do_switch = tmux_cfg.switch;

                let opened = TmuxLauncher::create_workspace_window(
                    sess_name, name, &ws_dir, win_cmd, do_switch,
                );
                if opened {
                    OutputHandler::print_info(&format!(
                        "Opened tmux window '@{}' in session '{}'",
                        name, sess_name
                    ));
                }
            }
        }

        // Register project in global registry
        let _ = crate::registry::register_project(&self.config.project_root, None);

        rollback.clear();
        OutputHandler::print_creation_success(name, &ws_dir);
        Ok(metadata)
    }

    pub fn create_workspace_from_config(
        &self,
        config_file: &Path,
        tmux_cmd: Option<&str>,
        no_tmux: bool,
    ) -> Result<WorkspaceMetadata, WSError> {
        if !config_file.exists() || !config_file.is_file() {
            return Err(WSError::Validation(format!(
                "Configuration file not found: {}",
                config_file.display()
            )));
        }

        let content = std::fs::read_to_string(config_file)?;
        let val: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
            WSError::Validation(format!(
                "Invalid YAML file '{}': {}",
                config_file.display(),
                e
            ))
        })?;

        let name = val.get("name").and_then(|n| n.as_str()).ok_or_else(|| {
            WSError::Validation(format!(
                "YAML config '{}' must contain a 'name' field",
                config_file.display()
            ))
        })?;

        let repos_map = val
            .get("repositories")
            .and_then(|r| r.as_mapping())
            .ok_or_else(|| {
                WSError::Validation(format!(
                    "YAML config '{}' must contain a 'repositories' section",
                    config_file.display()
                ))
            })?;

        let mut specs = Vec::new();
        for (k, v) in repos_map {
            let r_name = k.as_str().unwrap_or_default().to_string();
            let branch = v
                .get("branch")
                .and_then(|b| b.as_str())
                .ok_or_else(|| {
                    WSError::Validation(format!(
                        "Repository '{}' missing 'branch' in '{}'",
                        r_name,
                        config_file.display()
                    ))
                })?
                .to_string();

            let create = v.get("create").and_then(|c| c.as_bool()).unwrap_or(true);
            let mut checkout_path = r_name.clone();
            if let Some(r_cfg) = self.config.repositories.get(&r_name) {
                checkout_path = r_cfg.checkout.clone();
            }

            let base = v
                .get("base")
                .or_else(|| v.get("target"))
                .or_else(|| v.get("from"))
                .and_then(|b| b.as_str())
                .map(|s| s.to_string());

            let frozen = v
                .get("frozen")
                .or_else(|| v.get("locked"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);

            specs.push(RepoSpec {
                name: r_name,
                branch,
                create,
                path: checkout_path,
                frozen,
                locked: frozen,
                base_branch: base,
            });
        }

        self.create_workspace(name, &specs, tmux_cmd, no_tmux)
    }

    pub fn remove_workspace(&self, name: &str, quiet: bool) -> Result<(), WSError> {
        self.validate_environment()?;
        let ws_dir = self.get_workspace_dir(name);

        if !ws_dir.exists() || !ws_dir.is_dir() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found at: {}",
                name,
                ws_dir.display()
            )));
        }

        let meta_res = self.get_workspace_info(name);
        if let Ok((metadata, _)) = meta_res {
            for (r_name, spec) in &metadata.repositories {
                if let Some(repo_cfg) = self.config.repositories.get(r_name) {
                    let wt_path = ws_dir.join(&spec.path);
                    if wt_path.exists() {
                        let bare_path = self.resolve_bare_path(&repo_cfg.bare);
                        let _ = self.git.remove_worktree(&bare_path, &wt_path, true);
                    }
                }
            }
        } else {
            for repo_cfg in self.config.repositories.values() {
                let bare_path = self.resolve_bare_path(&repo_cfg.bare);
                let wt_path = ws_dir.join(&repo_cfg.checkout);
                if wt_path.exists() {
                    let _ = self.git.remove_worktree(&bare_path, &wt_path, true);
                }
            }
        }

        for repo_cfg in self.config.repositories.values() {
            let bare_path = self.resolve_bare_path(&repo_cfg.bare);
            if self.git.is_bare_repo(&bare_path) {
                let _ = self.git.prune_worktrees(&bare_path);
            }
        }

        let _ = std::fs::remove_dir_all(&ws_dir);
        if !quiet {
            OutputHandler::print_success(&format!("Removed workspace '{}'", name));
        }
        Ok(())
    }

    pub fn inspect_workspace_safety(
        &self,
        name: &str,
        target_branch: Option<&str>,
    ) -> Result<serde_json::Value, WSError> {
        self.validate_environment()?;
        let ws_dir = self.get_workspace_dir(name);

        if !ws_dir.exists() || !ws_dir.is_dir() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found at: {}",
                name,
                ws_dir.display()
            )));
        }

        let metadata = self.get_workspace_info(name).map(|(m, _)| m).ok();
        let mut repos_map: HashMap<String, RepoSpec> = HashMap::new();
        if let Some(meta) = metadata {
            repos_map = meta.repositories;
        } else {
            for (r_name, repo_cfg) in &self.config.repositories {
                repos_map.insert(
                    r_name.clone(),
                    RepoSpec {
                        name: r_name.clone(),
                        branch: "HEAD".to_string(),
                        create: false,
                        path: repo_cfg.checkout.clone(),
                        frozen: false,
                        locked: false,
                        base_branch: None,
                    },
                );
            }
        }

        let mut has_uncommitted = false;
        let mut has_unmerged = false;
        let mut repos_safety = serde_json::Map::new();

        for (r_name, spec) in repos_map {
            if !self.config.repositories.contains_key(&r_name) {
                continue;
            }
            let repo_cfg = self.validate_repository_config(&r_name)?;
            let wt_path = ws_dir.join(&spec.path);
            let bare_path = self.resolve_bare_path(&repo_cfg.bare);

            if !wt_path.exists() {
                let mut r_obj = serde_json::Map::new();
                r_obj.insert(
                    "worktree_exists".to_string(),
                    serde_json::Value::Bool(false),
                );
                let mut unc = serde_json::Map::new();
                unc.insert(
                    "has_uncommitted".to_string(),
                    serde_json::Value::Bool(false),
                );
                unc.insert("modified".to_string(), serde_json::Value::Array(vec![]));
                unc.insert("untracked".to_string(), serde_json::Value::Array(vec![]));
                r_obj.insert("uncommitted".to_string(), serde_json::Value::Object(unc));
                let mut m_info = serde_json::Map::new();
                m_info.insert("is_merged".to_string(), serde_json::Value::Bool(true));
                m_info.insert(
                    "target_branch".to_string(),
                    serde_json::Value::String(String::new()),
                );
                m_info.insert(
                    "unmerged_commits".to_string(),
                    serde_json::Value::Number(0.into()),
                );
                r_obj.insert("merged_info".to_string(), serde_json::Value::Object(m_info));
                r_obj.insert("branch".to_string(), serde_json::Value::String(spec.branch));
                repos_safety.insert(r_name, serde_json::Value::Object(r_obj));
                continue;
            }

            // 1. Uncommitted changes check
            let unc_info = self.git.check_worktree_uncommitted(&wt_path);
            if unc_info.has_uncommitted {
                has_uncommitted = true;
            }

            // 2. Merged check
            let cur_branch = self.git.get_current_branch(&wt_path);
            let branch_to_check = if !spec.branch.is_empty() && spec.branch != "HEAD" {
                spec.branch.clone()
            } else {
                cur_branch
            };

            let (is_merged, resolved_tgt, unmerged_count) = self.git.is_branch_merged(
                &bare_path,
                &branch_to_check,
                target_branch,
                Some(&wt_path),
                true,
            );
            if !is_merged {
                has_unmerged = true;
            }

            let mut r_obj = serde_json::Map::new();
            r_obj.insert("worktree_exists".to_string(), serde_json::Value::Bool(true));

            let mut unc = serde_json::Map::new();
            unc.insert(
                "has_uncommitted".to_string(),
                serde_json::Value::Bool(unc_info.has_uncommitted),
            );
            unc.insert("modified".to_string(), serde_json::json!(unc_info.modified));
            unc.insert(
                "untracked".to_string(),
                serde_json::json!(unc_info.untracked),
            );
            r_obj.insert("uncommitted".to_string(), serde_json::Value::Object(unc));

            let mut m_info = serde_json::Map::new();
            m_info.insert("is_merged".to_string(), serde_json::Value::Bool(is_merged));
            m_info.insert(
                "target_branch".to_string(),
                serde_json::Value::String(resolved_tgt),
            );
            m_info.insert(
                "unmerged_commits".to_string(),
                serde_json::Value::Number(unmerged_count.into()),
            );
            r_obj.insert("merged_info".to_string(), serde_json::Value::Object(m_info));
            r_obj.insert(
                "branch".to_string(),
                serde_json::Value::String(branch_to_check),
            );

            repos_safety.insert(r_name, serde_json::Value::Object(r_obj));
        }

        let mut res = serde_json::Map::new();
        res.insert(
            "workspace".to_string(),
            serde_json::Value::String(name.to_string()),
        );
        res.insert(
            "has_uncommitted".to_string(),
            serde_json::Value::Bool(has_uncommitted),
        );
        res.insert(
            "has_unmerged".to_string(),
            serde_json::Value::Bool(has_unmerged),
        );
        res.insert("repos".to_string(), serde_json::Value::Object(repos_safety));

        Ok(serde_json::Value::Object(res))
    }

    pub fn end_workspace(
        &self,
        name: &str,
        force: bool,
        no_merge: bool,
        delete_branch: bool,
        target_branch: Option<&str>,
        no_tmux: bool,
    ) -> Result<(), WSError> {
        self.validate_environment()?;
        let ws_dir = self.get_workspace_dir(name);

        if !ws_dir.exists() || !ws_dir.is_dir() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found at: {}",
                name,
                ws_dir.display()
            )));
        }

        let metadata = self.get_workspace_info(name).map(|(m, _)| m).ok();

        if !force {
            let safety = self.inspect_workspace_safety(name, target_branch)?;
            if safety["has_uncommitted"].as_bool().unwrap_or(false) {
                let mut details = Vec::new();
                if let Some(repos) = safety["repos"].as_object() {
                    for (r_name, r_val) in repos {
                        let unc = &r_val["uncommitted"];
                        if unc["has_uncommitted"].as_bool().unwrap_or(false) {
                            let mut parts = Vec::new();
                            let mod_files =
                                unc["modified"].as_array().map(|a| a.len()).unwrap_or(0);
                            let untr_files =
                                unc["untracked"].as_array().map(|a| a.len()).unwrap_or(0);
                            if mod_files > 0 {
                                parts.push(format!("{} modified/staged", mod_files));
                            }
                            if untr_files > 0 {
                                parts.push(format!("{} untracked", untr_files));
                            }
                            details.push(format!("  • %{}: {}", r_name, parts.join(", ")));
                        }
                    }
                }
                return Err(WSError::WorkspaceUncommitted(format!(
                    "Cannot end workspace '@{}': uncommitted changes detected.\n{}\n\nPlease commit or stash your changes before closing.\nTo discard changes and close anyway, use: ws end @{} --force",
                    name, details.join("\n"), name
                )));
            }

            if safety["has_unmerged"].as_bool().unwrap_or(false) && !no_merge {
                let mut details = Vec::new();
                if let Some(repos) = safety["repos"].as_object() {
                    for (r_name, r_val) in repos {
                        let m_info = &r_val["merged_info"];
                        if !m_info["is_merged"].as_bool().unwrap_or(true) {
                            let br = r_val["branch"].as_str().unwrap_or("unknown");
                            let tgt = m_info["target_branch"].as_str().unwrap_or("main");
                            let cnt = m_info["unmerged_commits"].as_i64().unwrap_or(0);
                            details.push(format!(
                                "  • %{} (branch '{}'): {} commit(s) not merged into '{}'",
                                r_name, br, cnt, tgt
                            ));
                        }
                    }
                }
                return Err(WSError::WorkspaceUnmerged(format!(
                    "Cannot end workspace '@{}': unmerged work detected.\n{}\n\nTo close without merging, re-run with: ws end @{} --no-merge\nTo force close regardless of work status, use: ws end @{} --force",
                    name, details.join("\n"), name, name
                )));
            }
        }

        if self.is_session_running(name) {
            OutputHandler::print_info(&format!(
                "Stopping active services for workspace '@{}'...",
                name
            ));
            let _ = self.stop_workspace(name);
            if self.is_session_running(name) && !force {
                return Err(WSError::SessionStop(format!(
                    "Failed to terminate active daemon session for workspace '@{}'. Workspace closing aborted.",
                    name
                )));
            }
        }

        let mut branches_to_delete = Vec::new();
        if delete_branch {
            if let Some(meta) = &metadata {
                for (r_name, spec) in &meta.repositories {
                    if let Some(repo_cfg) = self.config.repositories.get(r_name) {
                        let bare_path = self.resolve_bare_path(&repo_cfg.bare);
                        let default_br = self.git.get_default_branch(&bare_path);
                        if spec.branch != default_br {
                            branches_to_delete.push((bare_path, spec.branch.clone()));
                        }
                    }
                }
            }
        }

        self.remove_workspace(name, true)?;

        for (b_path, br_name) in branches_to_delete {
            let _ = self.git.delete_branch(&b_path, &br_name, true);
            OutputHandler::print_info(&format!(
                "Deleted branch '{}' from {}",
                br_name,
                b_path.display()
            ));
        }

        if let Some(tmux_cfg) = &self.config.tmux {
            if !no_tmux {
                let sess_name = &tmux_cfg.session;
                if TmuxLauncher::is_window_active(sess_name, name) {
                    let _ = TmuxLauncher::kill_workspace_window(sess_name, name);
                    OutputHandler::print_info(&format!(
                        "Closed tmux window '@{}' in session '{}'",
                        name, sess_name
                    ));
                }
            }
        }

        OutputHandler::print_success(&format!("Safely closed workspace '@{}'", name));
        Ok(())
    }

    pub fn list_workspaces(&self) -> Vec<WorkspaceMetadata> {
        let ws_root = &self.config.workspaces_dir;
        if !ws_root.exists() || !ws_root.is_dir() {
            return Vec::new();
        }

        let mut workspaces = Vec::new();
        if let Ok(entries) = std::fs::read_dir(ws_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let child_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let meta_file = path.join("workspace.yml");
                    if meta_file.exists() && meta_file.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&meta_file) {
                            if let Ok(meta) = WorkspaceMetadata::from_yaml_str(&content) {
                                workspaces.push(meta);
                                continue;
                            }
                        }
                        workspaces.push(WorkspaceMetadata {
                            name: child_name,
                            created: "unknown".to_string(),
                            status: "unknown".to_string(),
                            repositories: HashMap::new(),
                        });
                    } else {
                        workspaces.push(WorkspaceMetadata {
                            name: child_name,
                            created: "unknown".to_string(),
                            status: "active".to_string(),
                            repositories: HashMap::new(),
                        });
                    }
                }
            }
        }
        workspaces
    }

    pub fn get_workspace_info(&self, name: &str) -> Result<(WorkspaceMetadata, PathBuf), WSError> {
        let ws_dir = self.get_workspace_dir(name);
        if !ws_dir.exists() || !ws_dir.is_dir() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found at: {}",
                name,
                ws_dir.display()
            )));
        }

        let mut meta_file = ws_dir.join("workspace.yml");
        if !meta_file.exists() {
            meta_file = ws_dir.join("workspace.yaml");
        }
        if !meta_file.exists() {
            return Err(WSError::Validation(format!(
                "Workspace '{}' does not contain a workspace.yml file",
                name
            )));
        }

        let content = std::fs::read_to_string(&meta_file)?;
        let meta = WorkspaceMetadata::from_yaml_str(&content)
            .map_err(|e| WSError::Validation(format!("Invalid metadata: {}", e)))?;

        Ok((meta, ws_dir))
    }

    pub fn get_session_socket_path(&self, name: &str) -> PathBuf {
        let ws_dir = self.get_workspace_dir(name);
        ws_dir.join(".ws").join("session.sock")
    }

    pub fn is_daemon_active(&self, name: &str) -> bool {
        let sock_path = self.get_session_socket_path(name);
        ws_tui::is_session_active(&sock_path)
    }

    pub fn get_active_engine(&self, name: &str) -> Option<String> {
        let project_name = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let launch_sess = self.get_launch_session_name();

        if TmuxLauncher::is_window_active(&launch_sess, name)
            || TmuxLauncher::is_window_running(&project_name, name)
        {
            return Some("tmux".to_string());
        }

        if ZellijLauncher::is_session_running(&project_name)
            && ZellijLauncher::is_tab_running(&project_name, name)
        {
            return Some("zellij".to_string());
        }

        if self.is_daemon_active(name) {
            return Some("tui".to_string());
        }

        None
    }

    pub fn is_session_running(&self, name: &str) -> bool {
        self.get_active_engine(name).is_some()
    }

    pub fn get_running_services_status(
        &self,
        name: &str,
    ) -> Option<HashMap<String, serde_json::Value>> {
        let sock_path = self.get_session_socket_path(name);
        if !sock_path.exists() {
            return None;
        }
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(&sock_path).ok()?;
        let _ = stream.write_all(b"{\"type\":\"GetState\"}\n");
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).ok()?;
        let resp: serde_json::Value = serde_json::from_slice(&buf[..n]).ok()?;
        if resp.get("type").and_then(|v| v.as_str()) == Some("State") {
            let mut map = HashMap::new();
            if let Some(svcs) = resp.get("services").and_then(|v| v.as_array()) {
                for s in svcs {
                    if let Some(s_name) = s.get("name").and_then(|v| v.as_str()) {
                        map.insert(s_name.to_string(), s.clone());
                    }
                }
            }
            return Some(map);
        }
        None
    }

    pub fn attach_workspace(
        &self,
        workspace_name: &str,
        repo_name: Option<&str>,
        all_panes: bool,
        mode: Option<&str>,
        _switch: bool,
    ) -> Result<(), WSError> {
        let active_engine = self.get_active_engine(workspace_name);
        let target_engine = mode
            .map(|s| s.to_string())
            .or_else(|| active_engine.clone())
            .unwrap_or_else(|| "tui".to_string());

        let project_name = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if active_engine.as_deref() == Some("tmux")
            || (active_engine.is_none() && target_engine == "tmux")
        {
            let launch_sess = self.get_launch_session_name();
            OutputHandler::print_info(&format!(
                "Attaching to running Tmux window for workspace: @{}",
                workspace_name
            ));
            let _ = TmuxLauncher::attach(
                workspace_name,
                &launch_sess,
                &project_name,
                repo_name,
                all_panes,
            );
            return Ok(());
        }

        if active_engine.as_deref() == Some("zellij")
            || (active_engine.is_none() && target_engine == "zellij")
        {
            OutputHandler::print_info(&format!(
                "Attaching to running Zellij session for workspace: @{}",
                workspace_name
            ));
            let ws_dir = self.get_workspace_dir(workspace_name);
            let _ = ZellijLauncher::attach(
                workspace_name,
                &project_name,
                repo_name,
                all_panes,
                &ws_dir,
            );
            return Ok(());
        }

        let sock_path = self.get_session_socket_path(workspace_name);
        let fullscreen = !all_panes;
        OutputHandler::print_info(&format!(
            "Attaching to running native session for workspace: @{}",
            workspace_name
        ));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(async {
            ws_tui::attach_workspace_session(
                workspace_name.to_string(),
                sock_path,
                repo_name.map(|s| s.to_string()),
                fullscreen,
            )
            .await
        });
        Ok(())
    }

    pub fn run_raw_bridge(
        &self,
        workspace_name: &str,
        repo_name: Option<&str>,
    ) -> Result<(), WSError> {
        let sock_path = self.get_session_socket_path(workspace_name);
        if !self.is_session_running(workspace_name) {
            return Err(WSError::General(format!(
                "No active session found for workspace '{}'. Start services first using 'ws start @{}'.",
                workspace_name, workspace_name
            )));
        }
        let r = repo_name.unwrap_or(workspace_name);
        ws_tui::run_raw_bridge(&sock_path, r).map_err(WSError::General)?;
        Ok(())
    }

    pub fn stop_workspace(&self, name: &str) -> Result<bool, WSError> {
        let project_name = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut stopped_any = false;

        let launch_sess = self.get_launch_session_name();
        if TmuxLauncher::is_window_active(&launch_sess, name) {
            stopped_any = TmuxLauncher::kill_workspace_window(&launch_sess, name) || stopped_any;
        } else if TmuxLauncher::is_window_running(&project_name, name) {
            stopped_any = TmuxLauncher::kill_workspace(name, Some(&project_name)) || stopped_any;
        }

        if ZellijLauncher::is_session_running(&project_name) {
            stopped_any = ZellijLauncher::kill_workspace(name, &project_name) || stopped_any;
        }

        let sock_path = self.get_session_socket_path(name);
        if sock_path.exists() {
            let stopped = match tokio::runtime::Handle::try_current() {
                Ok(h) => tokio::task::block_in_place(|| {
                    h.block_on(ws_tui::stop_workspace_session(&sock_path))
                        .unwrap_or(false)
                }),
                Err(_) => tokio::runtime::Runtime::new()
                    .map(|rt| {
                        rt.block_on(ws_tui::stop_workspace_session(&sock_path))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false),
            };
            stopped_any = stopped || stopped_any;
        }

        Ok(stopped_any)
    }

    pub fn focus_workspace(&self, name: &str) -> Result<bool, WSError> {
        let tmux_cfg = self.config.tmux.as_ref().ok_or_else(|| {
            WSError::Config(
                "Tmux integration is not configured. Add 'tmux:' to repositories.yml.".to_string(),
            )
        })?;

        if !TmuxLauncher::is_available() {
            return Err(WSError::Workspace(
                "tmux executable not found on PATH.".to_string(),
            ));
        }

        let sess_name = &tmux_cfg.session;
        if !TmuxLauncher::is_window_active(sess_name, name) {
            return Err(WSError::WorkspaceNotFound(format!(
                "Tmux window '@{}' not found in session '{}'.",
                name, sess_name
            )));
        }

        Ok(TmuxLauncher::focus_workspace_window(sess_name, name))
    }

    pub fn get_launch_session_name(&self) -> String {
        if let Some(tmux) = &self.config.tmux {
            if let Some(ls) = &tmux.launch_session {
                return ls.clone();
            }
        }
        let proj = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        format!("running-{}", proj)
    }

    pub fn get_or_create_launch_session(&mut self) -> Result<String, WSError> {
        if let Some(tmux) = &self.config.tmux {
            if let Some(ls) = &tmux.launch_session {
                if tmux.session == *ls {
                    return Err(WSError::Config(format!(
                        "Tmux work session ('{}') and launch session ('{}') must have different names to prevent collisions.",
                        tmux.session, ls
                    )));
                }
                return Ok(ls.clone());
            }
        }

        let proj_name = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let work_session = self.config.tmux.as_ref().map(|t| t.session.clone());

        let candidate = loop {
            let rand_val: u16 = rand_or_time_u16();
            let c = format!("running-{}-{:04x}", proj_name, rand_val);
            if Some(&c) != work_session.as_ref() {
                break c;
            }
        };

        self.persist_launch_session(&candidate);
        OutputHandler::print_info(&format!(
            "Configured tmux launch session as '{}' in repositories.yml",
            candidate
        ));
        Ok(candidate)
    }

    fn persist_launch_session(&mut self, launch_session_name: &str) {
        if let Some(tmux) = &mut self.config.tmux {
            tmux.launch_session = Some(launch_session_name.to_string());
        } else {
            let proj_name = self
                .config
                .project_root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            self.config.tmux = Some(TmuxConfig {
                session: proj_name,
                launch_session: Some(launch_session_name.to_string()),
                command: None,
                switch: true,
            });
        }

        let cfg_path = self
            .config
            .config_file_path
            .clone()
            .unwrap_or_else(|| self.config.project_root.join("repositories.yml"));
        if cfg_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&cfg_path) {
                if let Ok(mut val) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    if let Some(map) = val.as_mapping_mut() {
                        let tmux_key = serde_yaml::Value::String("tmux".to_string());
                        if let Some(t_val) = map.get_mut(&tmux_key) {
                            if let Some(t_map) = t_val.as_mapping_mut() {
                                t_map.insert(
                                    serde_yaml::Value::String("launch_session".to_string()),
                                    serde_yaml::Value::String(launch_session_name.to_string()),
                                );
                            }
                        }
                    }
                    if let Ok(new_yaml) = serde_yaml::to_string(&val) {
                        let _ = std::fs::write(&cfg_path, new_yaml);
                    }
                }
            }
        }
    }

    pub fn resolve_repo_spec(
        &self,
        workspace_name: &str,
        repo_or_worktree: &str,
    ) -> Result<(String, RepoSpec, PathBuf), WSError> {
        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;
        let clean_target = repo_or_worktree.trim_start_matches(['%', '+', ':', '#', '$']);

        // 1. Direct match in meta.repositories
        if let Some(spec) = meta.repositories.get(clean_target) {
            let p = ws_dir.join(&spec.path);
            return Ok((clean_target.to_string(), spec.clone(), p));
        }

        // 2. Match by spec.path in meta.repositories
        for (r_name, spec) in &meta.repositories {
            if spec.path == clean_target
                || Path::new(&spec.path)
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    == Some(clean_target.into())
            {
                let p = ws_dir.join(&spec.path);
                return Ok((r_name.clone(), spec.clone(), p));
            }
        }

        // 3. Match via project config alias
        if let Some(r_cfg) = self.config.repositories.get(clean_target) {
            for (r_name, spec) in &meta.repositories {
                if spec.path == r_cfg.checkout || *r_name == r_cfg.checkout {
                    let p = ws_dir.join(&spec.path);
                    return Ok((r_name.clone(), spec.clone(), p));
                }
            }
            let wt_path = ws_dir.join(&r_cfg.checkout);
            if wt_path.is_dir() {
                return Ok((
                    clean_target.to_string(),
                    RepoSpec {
                        name: clean_target.to_string(),
                        branch: "HEAD".to_string(),
                        create: false,
                        path: r_cfg.checkout.clone(),
                        frozen: false,
                        locked: false,
                        base_branch: None,
                    },
                    wt_path,
                ));
            }
        }

        // 4. Directory exists on disk inside workspace
        let wt_path = ws_dir.join(clean_target);
        if wt_path.is_dir() {
            return Ok((
                clean_target.to_string(),
                RepoSpec {
                    name: clean_target.to_string(),
                    branch: "HEAD".to_string(),
                    create: false,
                    path: clean_target.to_string(),
                    frozen: false,
                    locked: false,
                    base_branch: None,
                },
                wt_path,
            ));
        }

        let available = meta
            .repositories
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        Err(WSError::RepositoryNotFound(format!(
            "Repository or worktree '{}' not found in workspace '{}'. Available worktrees: {}",
            repo_or_worktree, workspace_name, available
        )))
    }

    pub fn open_workspace(&self, name: &str, worktree: Option<&str>) -> Result<(), WSError> {
        let ws_dir = self.get_workspace_dir(name);
        if !ws_dir.exists() || !ws_dir.is_dir() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found at: {}",
                name,
                ws_dir.display()
            )));
        }

        let target_dir = if let Some(wt) = worktree {
            let (_, _, p) = self.resolve_repo_spec(name, wt)?;
            p
        } else {
            ws_dir
        };

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        OutputHandler::print_info(&format!("Opening shell inside: {}", target_dir.display()));

        let _ = std::env::set_current_dir(&target_dir);
        let _ = Command::new(&shell).status();
        Ok(())
    }

    pub fn status_workspace(&self, name: &str) -> Result<HashMap<String, String>, WSError> {
        let ws_dir = self.get_workspace_dir(name);
        if !ws_dir.exists() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found",
                name
            )));
        }

        let (meta, _) = self.get_workspace_info(name)?;
        let mut statuses = HashMap::new();
        for (r_name, spec) in &meta.repositories {
            let wt_path = ws_dir.join(&spec.path);
            if wt_path.exists() {
                statuses.insert(r_name.clone(), self.git.get_status(&wt_path));
            } else {
                statuses.insert(r_name.clone(), "missing worktree".to_string());
            }
        }
        Ok(statuses)
    }

    pub fn exec_workspace(
        &self,
        name: &str,
        command: &[String],
        repos: Option<&[String]>,
    ) -> Result<HashMap<String, i32>, WSError> {
        let ws_dir = self.get_workspace_dir(name);
        if !ws_dir.exists() {
            return Err(WSError::WorkspaceNotFound(format!(
                "Workspace '{}' not found",
                name
            )));
        }

        let cmd_str = command.join(" ");
        let (meta, _) = self.get_workspace_info(name)?;
        let mut target_items = Vec::new();

        if let Some(r_filter) = repos {
            for r in r_filter {
                let (r_key, _, wt_path) = self.resolve_repo_spec(name, r)?;
                target_items.push((r_key, wt_path));
            }
        } else {
            for (r_name, spec) in &meta.repositories {
                target_items.push((r_name.clone(), ws_dir.join(&spec.path)));
            }
        }

        let mut results = HashMap::new();
        for (r_name, wt_path) in target_items {
            if wt_path.exists() {
                OutputHandler::print_info(&format!("Executing in %{}...", r_name));
                let code = run_shell_command(&cmd_str, &wt_path, None);
                results.insert(r_name, code);
            } else {
                OutputHandler::print_warning(&format!("Skipping {} (worktree missing)", r_name));
                results.insert(r_name, -1);
            }
        }
        Ok(results)
    }

    pub fn fetch_repositories(&self) -> Result<(), WSError> {
        for (r_name, repo_cfg) in &self.config.repositories {
            let bare_path = self.resolve_bare_path(&repo_cfg.bare);
            if self.git.is_bare_repo(&bare_path) {
                OutputHandler::print_info(&format!(
                    "Fetching bare repo {} ({})...",
                    r_name,
                    bare_path.display()
                ));
                let _ = self.git.fetch_repo(&bare_path);
            }
        }
        Ok(())
    }

    pub fn doctor(&self) -> HashMap<String, bool> {
        let mut results = HashMap::new();
        results.insert("git_installed".to_string(), self.git.is_git_installed());

        for (r_name, repo_cfg) in &self.config.repositories {
            let bare_path = self.resolve_bare_path(&repo_cfg.bare);
            results.insert(
                format!("repo_{}", r_name),
                self.git.is_bare_repo(&bare_path),
            );
        }

        results.insert(
            "workspaces_dir_exists".to_string(),
            self.config.workspaces_dir.exists(),
        );

        let active_interfaces = list_network_interfaces();
        let detected_ip = get_lan_ip(None);
        results.insert(
            "network_interfaces_detected".to_string(),
            !active_interfaces.is_empty() || detected_ip != "127.0.0.1",
        );
        results
    }

    pub fn parse_repo_url(input_str: &str) -> (String, String, PathBuf, String) {
        let input_str = input_str.trim();
        let (mut name, url) = if input_str.contains('=')
            && !input_str.starts_with("http://")
            && !input_str.starts_with("https://")
            && !input_str.starts_with("git@")
        {
            let parts: Vec<&str> = input_str.splitn(2, '=').collect();
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (String::new(), input_str.to_string())
        };

        let clean_url = url.trim_end_matches('/');
        let base_url = clean_url.strip_suffix(".git").unwrap_or(clean_url);
        let base_name = base_url
            .split('/')
            .next_back()
            .unwrap_or("")
            .split(':')
            .next_back()
            .unwrap_or("");

        if name.is_empty() {
            name = base_name.to_lowercase();
        }

        let checkout = base_name.to_string();
        let bare_name = if base_name.ends_with(".git") {
            base_name.to_string()
        } else {
            format!("{}.git", base_name)
        };
        let bare_path = PathBuf::from("bares").join(bare_name);

        (name, url, bare_path, checkout)
    }

    pub fn init_project(&mut self, repo_inputs: &[String]) -> Result<AppConfig, WSError> {
        self.validate_environment()?;
        let mut updated_repos = self.config.repositories.clone();

        for item in repo_inputs {
            let (name, url, bare_path, checkout) = Self::parse_repo_url(item);
            let resolved_bare = self.resolve_bare_path(&bare_path);
            if let Some(parent) = resolved_bare.parent() {
                ensure_directory(parent)?;
            }

            if !self.git.is_bare_repo(&resolved_bare) {
                OutputHandler::print_info(&format!(
                    "Cloning bare repository {} from {}...",
                    name, url
                ));
                self.git.clone_bare(&url, &resolved_bare)?;
                OutputHandler::print_success(&format!("Cloned bare repo {}", bare_path.display()));
            } else {
                OutputHandler::print_info(&format!(
                    "Using existing bare repository at {}",
                    bare_path.display()
                ));
            }

            updated_repos.insert(
                name.clone(),
                RepoConfig {
                    name,
                    bare: bare_path,
                    checkout,
                    url: Some(url),
                    port: None,
                    ports: HashMap::new(),
                    env: HashMap::new(),
                    secret_env: HashMap::new(),
                    private_env: HashMap::new(),
                    env_file: ".env".to_string(),
                    env_example: ".env.example".to_string(),
                    setup: Vec::new(),
                    launch: None,
                    secrets: Vec::new(),
                    copy_files: Vec::new(),
                },
            );
        }

        let root_dir_name = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let tmux_cfg = self.config.tmux.clone().unwrap_or_else(|| TmuxConfig {
            session: root_dir_name,
            launch_session: None,
            command: Some("nvim".to_string()),
            switch: true,
        });

        let hub_auto_save =
            self.config
                .hub_auto_save
                .clone()
                .unwrap_or_else(|| HubAutoSaveConfig {
                    enabled: true,
                    interval: 300,
                    include_wip: true,
                    workspaces: WorkspacesSelector::Mode("all".to_string()),
                    notify: true,
                });

        let saved_path = ConfigLoader::save_config(
            &updated_repos,
            self.config.config_file_path.as_deref(),
            Some(&tmux_cfg),
            Some(&hub_auto_save),
            self.config.hub_project.as_deref(),
        )?;

        OutputHandler::print_success(&format!("Saved configuration to {}", saved_path.display()));
        self.config.repositories = updated_repos;
        self.config.tmux = Some(tmux_cfg);
        self.config.hub_auto_save = Some(hub_auto_save);
        self.config.config_file_path = Some(saved_path);

        Ok(self.config.clone())
    }

    pub fn save_metadata(
        &self,
        ws_dir: &Path,
        metadata: &WorkspaceMetadata,
    ) -> Result<(), WSError> {
        let metadata_path = ws_dir.join("workspace.yml");
        let content = serde_yaml::to_string(metadata)?;
        std::fs::write(metadata_path, content)?;
        Ok(())
    }

    pub fn workspace_add_repo(
        &self,
        workspace_name: &str,
        repo_name: &str,
        branch: &str,
        create: bool,
    ) -> Result<(), WSError> {
        self.validate_environment()?;
        let (mut meta, ws_dir) = self.get_workspace_info(workspace_name)?;

        if meta.repositories.contains_key(repo_name) {
            return Err(WSError::RepoAlreadyInWorkspace(format!(
                "Repository '{}' is already in workspace '{}'",
                repo_name, workspace_name
            )));
        }

        let repo_cfg = self.validate_repository_config(repo_name)?;
        let branch_exists = self.git.branch_exists(&repo_cfg.bare, branch);

        if create && branch_exists {
            return Err(WSError::BranchAlreadyExists(format!(
                "Cannot create branch '{}' for repository '{}': branch already exists",
                branch, repo_name
            )));
        }
        if !create && !branch_exists {
            return Err(WSError::BranchNotFound(format!(
                "Branch '{}' does not exist in repository '{}'",
                branch, repo_name
            )));
        }

        let worktree_path = ws_dir.join(&repo_cfg.checkout);
        self.git
            .create_worktree(&repo_cfg.bare, &worktree_path, branch, create, None)?;

        meta.repositories.insert(
            repo_name.to_string(),
            RepoSpec {
                name: repo_name.to_string(),
                branch: branch.to_string(),
                create,
                path: repo_cfg.checkout,
                frozen: false,
                locked: false,
                base_branch: None,
            },
        );

        self.save_metadata(&ws_dir, &meta)?;
        OutputHandler::print_success(&format!(
            "Added repository '{}' ({}) to workspace '{}'",
            repo_name, branch, workspace_name
        ));
        Ok(())
    }

    pub fn workspace_remove_repo(
        &self,
        workspace_name: &str,
        repo_name: &str,
        delete_branch: bool,
    ) -> Result<(), WSError> {
        self.validate_environment()?;
        let (mut meta, ws_dir) = self.get_workspace_info(workspace_name)?;

        let spec = meta.repositories.get(repo_name).ok_or_else(|| {
            WSError::RepoNotInWorkspace(format!(
                "Repository '{}' is not in workspace '{}'",
                repo_name, workspace_name
            ))
        })?;

        if spec.frozen || spec.locked {
            return Err(WSError::RepoFrozen(format!(
                "Cannot remove repository '{}': it is locked/frozen in workspace '{}'. Unlock it first.",
                repo_name, workspace_name
            )));
        }

        let repo_cfg = self.validate_repository_config(repo_name)?;
        let worktree_path = ws_dir.join(&spec.path);

        if worktree_path.exists() {
            self.git.set_tracked_files_readonly(&worktree_path, false);
            self.git
                .remove_worktree(&repo_cfg.bare, &worktree_path, true)?;
        }

        if delete_branch {
            let _ = self.git.delete_branch(&repo_cfg.bare, &spec.branch, true);
        }

        meta.repositories.remove(repo_name);
        self.save_metadata(&ws_dir, &meta)?;
        OutputHandler::print_success(&format!(
            "Removed repository '{}' from workspace '{}'{}",
            repo_name,
            workspace_name,
            if delete_branch {
                " (deleted branch)"
            } else {
                ""
            }
        ));
        Ok(())
    }

    pub fn lock_repo(&self, workspace_name: &str, repo_name: &str) -> Result<(), WSError> {
        let (mut meta, ws_dir) = self.get_workspace_info(workspace_name)?;
        let (r_key, spec, worktree_path) = self.resolve_repo_spec(workspace_name, repo_name)?;

        if spec.frozen || spec.locked {
            OutputHandler::print_info(&format!("Repository '%{}' is already locked", r_key));
            return Ok(());
        }

        if worktree_path.exists() {
            self.git.set_tracked_files_readonly(&worktree_path, true);
        }

        if let Some(s) = meta.repositories.get_mut(&r_key) {
            s.frozen = true;
            s.locked = true;
        }
        self.save_metadata(&ws_dir, &meta)?;
        OutputHandler::print_success(&format!(
            "Locked repository '%{}' in workspace '@{}'",
            r_key, workspace_name
        ));
        Ok(())
    }

    pub fn freeze_repo(&self, workspace_name: &str, repo_name: &str) -> Result<(), WSError> {
        self.lock_repo(workspace_name, repo_name)
    }

    pub fn unlock_repo(&self, workspace_name: &str, repo_name: &str) -> Result<(), WSError> {
        let (mut meta, ws_dir) = self.get_workspace_info(workspace_name)?;
        let (r_key, spec, worktree_path) = self.resolve_repo_spec(workspace_name, repo_name)?;

        let is_already_unlocked = !spec.frozen && !spec.locked;

        if worktree_path.exists() {
            self.git.set_tracked_files_readonly(&worktree_path, false);
        }

        if let Some(s) = meta.repositories.get_mut(&r_key) {
            s.frozen = false;
            s.locked = false;
        }
        self.save_metadata(&ws_dir, &meta)?;

        if is_already_unlocked {
            OutputHandler::print_success(&format!(
                "Restored write permissions for repository '%{}' in workspace '@{}'",
                r_key, workspace_name
            ));
        } else {
            OutputHandler::print_success(&format!(
                "Unlocked repository '%{}' in workspace '@{}'",
                r_key, workspace_name
            ));
        }
        Ok(())
    }

    pub fn unfreeze_repo(&self, workspace_name: &str, repo_name: &str) -> Result<(), WSError> {
        self.unlock_repo(workspace_name, repo_name)
    }

    pub fn push_workspace(
        &self,
        workspace_name: &str,
        repos: Option<&[String]>,
        remote: &str,
    ) -> Result<HashMap<String, HashMap<String, String>>, WSError> {
        self.validate_environment()?;
        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;

        let target_repos: Vec<String> = if let Some(r_list) = repos {
            let mut tr = Vec::new();
            for r in r_list {
                let (r_name, _, _) = self.resolve_repo_spec(workspace_name, r)?;
                if !tr.contains(&r_name) {
                    tr.push(r_name);
                }
            }
            tr
        } else {
            meta.repositories.keys().cloned().collect()
        };

        let mut results = HashMap::new();
        for r_name in target_repos {
            let spec = &meta.repositories[&r_name];
            let wt_path = ws_dir.join(&spec.path);

            if spec.frozen || spec.locked {
                let mut m = HashMap::new();
                m.insert("status".to_string(), "skipped".to_string());
                m.insert(
                    "reason".to_string(),
                    "frozen repository (read-only)".to_string(),
                );
                m.insert("branch".to_string(), spec.branch.clone());
                m.insert("remote".to_string(), remote.to_string());
                results.insert(r_name, m);
                continue;
            }

            if !wt_path.exists() {
                let mut m = HashMap::new();
                m.insert("status".to_string(), "skipped".to_string());
                m.insert("reason".to_string(), "missing worktree".to_string());
                m.insert("branch".to_string(), spec.branch.clone());
                m.insert("remote".to_string(), remote.to_string());
                results.insert(r_name, m);
                continue;
            }

            let mut m = HashMap::new();
            m.insert("branch".to_string(), spec.branch.clone());
            m.insert("remote".to_string(), remote.to_string());

            match self.git.push_branch(&wt_path, remote, Some(&spec.branch)) {
                Ok((was_pushed, msg)) => {
                    m.insert(
                        "status".to_string(),
                        if was_pushed {
                            "pushed".to_string()
                        } else {
                            "up-to-date".to_string()
                        },
                    );
                    m.insert("reason".to_string(), msg);
                }
                Err(e) => {
                    m.insert("status".to_string(), "failed".to_string());
                    m.insert("reason".to_string(), e.to_string());
                }
            }
            results.insert(r_name, m);
        }

        Ok(results)
    }

    pub fn pull_workspace(
        &self,
        workspace_name: &str,
        repos: Option<&[String]>,
        remote: &str,
    ) -> Result<HashMap<String, HashMap<String, String>>, WSError> {
        self.validate_environment()?;
        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;

        let target_repos: Vec<String> = if let Some(r_list) = repos {
            let mut tr = Vec::new();
            for r in r_list {
                let (r_name, _, _) = self.resolve_repo_spec(workspace_name, r)?;
                if !tr.contains(&r_name) {
                    tr.push(r_name);
                }
            }
            tr
        } else {
            meta.repositories.keys().cloned().collect()
        };

        let mut results = HashMap::new();
        for r_name in target_repos {
            let spec = &meta.repositories[&r_name];
            let wt_path = ws_dir.join(&spec.path);

            if spec.frozen || spec.locked {
                let mut m = HashMap::new();
                m.insert("status".to_string(), "skipped".to_string());
                m.insert(
                    "reason".to_string(),
                    "frozen repository (read-only)".to_string(),
                );
                m.insert("branch".to_string(), spec.branch.clone());
                m.insert("remote".to_string(), remote.to_string());
                results.insert(r_name, m);
                continue;
            }

            if !wt_path.exists() {
                let mut m = HashMap::new();
                m.insert("status".to_string(), "skipped".to_string());
                m.insert("reason".to_string(), "missing worktree".to_string());
                m.insert("branch".to_string(), spec.branch.clone());
                m.insert("remote".to_string(), remote.to_string());
                results.insert(r_name, m);
                continue;
            }

            let mut m = HashMap::new();
            m.insert("branch".to_string(), spec.branch.clone());
            m.insert("remote".to_string(), remote.to_string());

            match self.git.pull_branch(&wt_path, remote, Some(&spec.branch)) {
                Ok((was_updated, msg)) => {
                    m.insert(
                        "status".to_string(),
                        if was_updated {
                            "pulled".to_string()
                        } else {
                            "up-to-date".to_string()
                        },
                    );
                    m.insert("reason".to_string(), msg);
                }
                Err(e) => {
                    m.insert("status".to_string(), "failed".to_string());
                    m.insert("reason".to_string(), e.to_string());
                }
            }
            results.insert(r_name, m);
        }

        Ok(results)
    }

    pub fn setup_workspace(
        &mut self,
        workspace_name: &str,
        repos: Option<&[String]>,
        dry_run: bool,
        skip_scripts: bool,
        verbose: bool,
        interface: Option<&str>,
        lan_ip: Option<&str>,
    ) -> Result<HashMap<String, serde_json::Value>, WSError> {
        self.validate_environment()?;
        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;

        let target_repos: Vec<String> = if let Some(r_list) = repos {
            let mut tr = Vec::new();
            for r in r_list {
                let (r_name, _, _) = self.resolve_repo_spec(workspace_name, r)?;
                if !tr.contains(&r_name) {
                    tr.push(r_name);
                }
            }
            tr
        } else {
            meta.repositories.keys().cloned().collect()
        };

        let slot = EnvEngine::get_workspace_slot(&self.config.workspaces_dir, workspace_name);
        let resolved_lan_ip = lan_ip
            .map(|s| s.to_string())
            .unwrap_or_else(|| get_lan_ip(interface));

        let public_host = self
            .config
            .global_env
            .get("PUBLIC_HOST")
            .cloned()
            .or_else(|| std::env::var("WS_PUBLIC_HOST").ok())
            .or_else(|| std::env::var("PUBLIC_HOST").ok())
            .unwrap_or_else(|| resolved_lan_ip.clone());

        let (service_ports, _) = allocate_workspace_ports(&self.config.repositories, slot, None);
        let _ = EnvEngine::write_service_discovery_files(
            &ws_dir,
            workspace_name,
            slot,
            &service_ports,
            Some(&public_host),
            Some(&resolved_lan_ip),
            interface,
        );

        if !dry_run
            && self
                .config
                .tmux
                .as_ref()
                .and_then(|t| t.launch_session.as_ref())
                .is_none()
        {
            let _ = self.get_or_create_launch_session();
        }

        let mut results = HashMap::new();

        // Top level infrastructure setup
        if repos.is_none() && !self.config.setup.is_empty() && !skip_scripts {
            OutputHandler::print_setup_repo_start("WORKSPACE INFRASTRUCTURE", &ws_dir);
            let global_vars = EnvEngine::resolve_repo_env(
                &self.config,
                workspace_name,
                "",
                slot,
                Some(&service_ports),
                Some(&resolved_lan_ip),
                Some(&public_host),
                interface,
            );

            if verbose {
                OutputHandler::print_env_resolution_details(
                    &global_vars,
                    Some(&self.config.secrets),
                );
            }

            for g_cmd in &self.config.setup {
                let expanded = EnvEngine::expand_command(
                    g_cmd,
                    &global_vars,
                    workspace_name,
                    "",
                    slot,
                    Some(&self.config.project_root),
                    Some(&self.config.workspaces_dir),
                    Some(&service_ports),
                    Some(&resolved_lan_ip),
                    Some(&public_host),
                    interface,
                );

                if dry_run {
                    OutputHandler::print_setup_step(
                        0,
                        &format!("[DRY-RUN] {}", expanded),
                        "skipped execution",
                        "info",
                    );
                    continue;
                }

                OutputHandler::print_command_start(&expanded);
                let t0 = Instant::now();
                let mut env_map = global_vars.clone();
                env_map.insert("WORKSPACE_NAME".to_string(), workspace_name.to_string());
                env_map.insert(
                    "PROJECT_ROOT".to_string(),
                    self.config.project_root.display().to_string(),
                );
                env_map.insert("WORKSPACE_DIR".to_string(), ws_dir.display().to_string());

                let ret = run_shell_command(&expanded, &ws_dir, Some(&env_map));
                let elapsed = t0.elapsed().as_secs_f64();
                OutputHandler::print_command_done(&expanded, elapsed, ret == 0, ret);
            }
        }

        for r_name in target_repos {
            let spec = &meta.repositories[&r_name];
            let wt_path = ws_dir.join(&spec.path);
            let repo_cfg = self.config.repositories.get(&r_name);

            OutputHandler::print_setup_repo_start(&r_name, &wt_path);

            let repo_cfg = match repo_cfg {
                Some(rc) => rc,
                None => {
                    OutputHandler::print_setup_step(
                        1,
                        "Config Validation",
                        "Repository missing from project configuration",
                        "error",
                    );
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "status".to_string(),
                        serde_json::Value::String("failed".to_string()),
                    );
                    obj.insert(
                        "reason".to_string(),
                        serde_json::Value::String("missing from config".to_string()),
                    );
                    results.insert(r_name, serde_json::Value::Object(obj));
                    continue;
                }
            };

            if !wt_path.exists() {
                OutputHandler::print_setup_step(
                    1,
                    "Worktree Validation",
                    "Worktree directory does not exist",
                    "warning",
                );
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String("skipped".to_string()),
                );
                obj.insert(
                    "reason".to_string(),
                    serde_json::Value::String("missing worktree".to_string()),
                );
                results.insert(r_name, serde_json::Value::Object(obj));
                continue;
            }

            // Step 1: Copy files
            let mut all_copy = self.config.copy_files.clone();
            all_copy.extend(repo_cfg.copy_files.clone());
            if !all_copy.is_empty() {
                let (f_ok, f_msg) =
                    EnvEngine::sync_copied_files(&self.config.project_root, &wt_path, &all_copy);
                OutputHandler::print_setup_step(
                    1,
                    "File Copy",
                    &f_msg,
                    if f_ok { "success" } else { "warning" },
                );
            }

            let env_vars = EnvEngine::resolve_repo_env(
                &self.config,
                workspace_name,
                &r_name,
                slot,
                Some(&service_ports),
                Some(&resolved_lan_ip),
                Some(&public_host),
                interface,
            );

            let (env_ok, env_msg) = EnvEngine::prepare_and_sync_env_file(
                &wt_path,
                &env_vars,
                &repo_cfg.env_file,
                &repo_cfg.env_example,
            )?;

            if !env_ok {
                OutputHandler::print_setup_step(
                    1,
                    "Environment Setup",
                    &format!("Failed: {}", env_msg),
                    "error",
                );
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String("failed".to_string()),
                );
                obj.insert("reason".to_string(), serde_json::Value::String(env_msg));
                results.insert(r_name, serde_json::Value::Object(obj));
                continue;
            }

            OutputHandler::print_setup_step(
                1,
                "Template Setup",
                &format!(
                    "processed {} -> {}",
                    repo_cfg.env_example, repo_cfg.env_file
                ),
                "success",
            );
            OutputHandler::print_setup_step(2, "Env Resolution", &env_msg, "success");

            if verbose {
                let mut all_sec = self.config.secrets.clone();
                all_sec.extend(repo_cfg.secrets.clone());
                OutputHandler::print_env_resolution_details(&env_vars, Some(&all_sec));
            }

            // Step 3: Run setup commands
            if skip_scripts || repo_cfg.setup.is_empty() {
                OutputHandler::print_setup_step(
                    3,
                    "Setup Scripts",
                    "No setup scripts configured (skipped)",
                    "info",
                );
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String("completed".to_string()),
                );
                obj.insert(
                    "reason".to_string(),
                    serde_json::Value::String("environment synced (no setup commands)".to_string()),
                );
                obj.insert("env_status".to_string(), serde_json::Value::String(env_msg));
                results.insert(r_name, serde_json::Value::Object(obj));
                continue;
            }

            let mut script_failures = Vec::new();
            let mut executed_cmds = Vec::new();

            for cmd in &repo_cfg.setup {
                let expanded = EnvEngine::expand_command(
                    cmd,
                    &env_vars,
                    workspace_name,
                    &r_name,
                    slot,
                    Some(&self.config.project_root),
                    Some(&self.config.workspaces_dir),
                    Some(&service_ports),
                    Some(&resolved_lan_ip),
                    Some(&public_host),
                    interface,
                );
                executed_cmds.push(expanded.clone());

                if dry_run {
                    OutputHandler::print_setup_step(
                        3,
                        &format!("[DRY-RUN] {}", expanded),
                        "skipped execution",
                        "info",
                    );
                    continue;
                }

                OutputHandler::print_command_start(&expanded);
                let t0 = Instant::now();
                let mut proc_env = env_vars.clone();
                proc_env.insert("WORKSPACE_NAME".to_string(), workspace_name.to_string());
                proc_env.insert("REPO_NAME".to_string(), r_name.clone());
                proc_env.insert(
                    "PROJECT_ROOT".to_string(),
                    self.config.project_root.display().to_string(),
                );
                proc_env.insert("WORKSPACE_DIR".to_string(), ws_dir.display().to_string());
                proc_env.insert("WORKTREE_DIR".to_string(), wt_path.display().to_string());

                let ret = run_shell_command(&expanded, &wt_path, Some(&proc_env));
                let elapsed = t0.elapsed().as_secs_f64();
                OutputHandler::print_command_done(&expanded, elapsed, ret == 0, ret);

                if ret != 0 {
                    script_failures.push(format!("'{}' failed: exit code {}", expanded, ret));
                    break;
                }
            }

            let mut obj = serde_json::Map::new();
            if script_failures.is_empty() {
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String("completed".to_string()),
                );
                obj.insert(
                    "reason".to_string(),
                    serde_json::Value::String(format!(
                        "ran {} setup command(s)",
                        executed_cmds.len()
                    )),
                );
            } else {
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String("failed".to_string()),
                );
                obj.insert(
                    "reason".to_string(),
                    serde_json::Value::String(script_failures.join("; ")),
                );
            }
            obj.insert("env_status".to_string(), serde_json::Value::String(env_msg));
            obj.insert("commands_run".to_string(), serde_json::json!(executed_cmds));
            results.insert(r_name, serde_json::Value::Object(obj));
        }

        Ok(results)
    }

    pub fn sync_env(
        &mut self,
        workspace_name: &str,
        repos: Option<&[String]>,
        interface: Option<&str>,
        lan_ip: Option<&str>,
    ) -> Result<HashMap<String, serde_json::Value>, WSError> {
        self.setup_workspace(workspace_name, repos, false, true, false, interface, lan_ip)
    }

    pub fn get_env_vars(
        &self,
        workspace_name: &str,
        repo_name: &str,
        interface: Option<&str>,
        lan_ip: Option<&str>,
    ) -> HashMap<String, String> {
        let slot = EnvEngine::get_workspace_slot(&self.config.workspaces_dir, workspace_name);
        EnvEngine::resolve_repo_env(
            &self.config,
            workspace_name,
            repo_name,
            slot,
            None,
            lan_ip,
            None,
            interface,
        )
    }

    pub fn launch_workspace(
        &mut self,
        workspace_name: &str,
        repos: Option<&[String]>,
        mode: &str,
        attach_repo: Option<&str>,
        daemon: bool,
        switch: bool,
        interface: Option<&str>,
        lan_ip: Option<&str>,
    ) -> Result<Vec<(String, String, String, HashMap<String, String>)>, WSError> {
        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;
        let target_repos: Vec<String> = repos
            .map(|r| r.to_vec())
            .unwrap_or_else(|| meta.repositories.keys().cloned().collect());

        let slot = EnvEngine::get_workspace_slot(&self.config.workspaces_dir, workspace_name);
        let resolved_lan_ip = lan_ip
            .map(|s| s.to_string())
            .unwrap_or_else(|| get_lan_ip(interface));

        let public_host = self
            .config
            .global_env
            .get("PUBLIC_HOST")
            .cloned()
            .or_else(|| std::env::var("WS_PUBLIC_HOST").ok())
            .or_else(|| std::env::var("PUBLIC_HOST").ok())
            .unwrap_or_else(|| resolved_lan_ip.clone());

        let recorded_leases = EnvEngine::read_service_discovery_descriptor(&ws_dir);
        let (service_ports, has_shifted) =
            allocate_workspace_ports(&self.config.repositories, slot, recorded_leases.as_ref());
        if has_shifted {
            OutputHandler::print_warning(
                &format!("Active socket collision detected for workspace '@{}'. Dynamically re-allocated free ports and synchronized worktree .env files.", workspace_name)
            );
        }

        // Always re-evaluate and sync .env
        for r_k in meta.repositories.keys() {
            if let Some(spec) = meta.repositories.get(r_k) {
                if let Some(repo_cfg) = self.config.repositories.get(r_k) {
                    let wt_p = ws_dir.join(&spec.path);
                    if wt_p.exists() {
                        let r_env = EnvEngine::resolve_repo_env(
                            &self.config,
                            workspace_name,
                            r_k,
                            slot,
                            Some(&service_ports),
                            Some(&resolved_lan_ip),
                            Some(&public_host),
                            interface,
                        );
                        let _ = EnvEngine::prepare_and_sync_env_file(
                            &wt_p,
                            &r_env,
                            &repo_cfg.env_file,
                            &repo_cfg.env_example,
                        );
                    }
                }
            }
        }

        let _ = EnvEngine::write_service_discovery_files(
            &ws_dir,
            workspace_name,
            slot,
            &service_ports,
            Some(&public_host),
            Some(&resolved_lan_ip),
            interface,
        );

        let mut launch_entries = Vec::new();
        for r_name in &target_repos {
            if let Some(spec) = meta.repositories.get(r_name) {
                if let Some(repo_cfg) = self.config.repositories.get(r_name) {
                    if let Some(launch_cmd) = &repo_cfg.launch {
                        let wt_path = ws_dir.join(&spec.path);
                        let env_vars = EnvEngine::resolve_repo_env(
                            &self.config,
                            workspace_name,
                            r_name,
                            slot,
                            Some(&service_ports),
                            Some(&resolved_lan_ip),
                            Some(&public_host),
                            interface,
                        );
                        let expanded_cmd = EnvEngine::expand_command(
                            launch_cmd,
                            &env_vars,
                            workspace_name,
                            r_name,
                            slot,
                            Some(&self.config.project_root),
                            Some(&self.config.workspaces_dir),
                            Some(&service_ports),
                            Some(&resolved_lan_ip),
                            Some(&public_host),
                            interface,
                        );
                        launch_entries.push((
                            r_name.clone(),
                            wt_path.display().to_string(),
                            expanded_cmd,
                            env_vars,
                        ));
                    }
                }
            }
        }

        if launch_entries.is_empty() || matches!(mode, "summary" | "list") {
            return Ok(launch_entries);
        }

        let active_engine = self.get_active_engine(workspace_name);
        let req_engine = if matches!(mode, "tui" | "daemon") {
            "tui"
        } else {
            mode
        };

        if let Some(eng) = &active_engine {
            if req_engine != eng && !matches!(mode, "summary" | "list" | "attach") {
                if switch {
                    OutputHandler::print_info(&format!(
                        "Switching workspace '@{}' from {} to {}...",
                        workspace_name, eng, req_engine
                    ));
                    if eng == "tmux" {
                        let launch_sess = self.get_launch_session_name();
                        if TmuxLauncher::is_window_active(&launch_sess, workspace_name) {
                            let _ =
                                TmuxLauncher::kill_workspace_window(&launch_sess, workspace_name);
                        } else {
                            let proj_name = self
                                .config
                                .project_root
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy();
                            let _ = TmuxLauncher::kill_workspace(workspace_name, Some(&proj_name));
                        }
                    } else if eng == "zellij" {
                        let proj_name = self
                            .config
                            .project_root
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        let _ = ZellijLauncher::kill_workspace(workspace_name, &proj_name);
                    }
                } else {
                    OutputHandler::print_error(
                        &format!("Workspace '{}' is already running in {}.\nTo switch engines, use '--switch' (e.g. 'ws launch {} --{} --switch').", workspace_name, eng, workspace_name, req_engine),
                        None,
                    );
                    return Ok(launch_entries);
                }
            }
        }

        // Ensure daemon running helper
        let sock_path = self.get_session_socket_path(workspace_name);
        let log_dir = ws_dir.join(".ws").join("logs");

        let ensure_daemon_running = || -> Result<(), WSError> {
            if !self.is_daemon_active(workspace_name) {
                ensure_directory(sock_path.parent().unwrap())?;
                if sock_path.exists() {
                    let _ = std::fs::remove_file(&sock_path);
                }

                let specs: Vec<ws_tui::ServiceSpec> = launch_entries
                    .iter()
                    .map(|(name, cwd, cmd, env)| ws_tui::ServiceSpec {
                        name: name.clone(),
                        command: cmd.clone(),
                        cwd: cwd.clone(),
                        env: env.clone(),
                    })
                    .collect();

                let w_name = workspace_name.to_string();
                let s_path = sock_path.clone();
                let l_dir = Some(log_dir.clone());

                // Spawn daemon in a background thread with tokio runtime
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(async {
                        let _ = ws_tui::start_workspace_daemon(w_name, specs, s_path, l_dir).await;
                    });
                });

                for _ in 0..60 {
                    if self.is_daemon_active(workspace_name) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            Ok(())
        };

        if daemon || mode == "daemon" {
            ensure_daemon_running()?;
            OutputHandler::print_success(&format!(
                "Workspace daemon active for '{}' ({} services).\nAttach anytime using: ws attach {} or ws {} attach",
                workspace_name,
                launch_entries.len(),
                workspace_name,
                workspace_name
            ));
            return Ok(launch_entries);
        }

        let proj_name = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if mode == "zellij" {
            if !ZellijLauncher::is_available() {
                OutputHandler::print_error(
                    "Zellij executable is not found on $PATH.\nInstall Zellij using 'cargo install zellij' or via your system package manager.",
                    None,
                );
                return Ok(launch_entries);
            }
            ensure_daemon_running()?;
            let entries_conv: Vec<(String, String, String)> = launch_entries
                .iter()
                .map(|(a, b, c, _)| (a.clone(), b.clone(), c.clone()))
                .collect();
            let _ = ZellijLauncher::launch(workspace_name, &entries_conv, &proj_name, &ws_dir);
            return Ok(launch_entries);
        }

        if mode == "tmux" {
            if !TmuxLauncher::is_available() {
                OutputHandler::print_error(
                    "tmux executable is not found on $PATH.\nInstall tmux via your system package manager (e.g. apt install tmux).",
                    None,
                );
                return Ok(launch_entries);
            }
            ensure_daemon_running()?;
            let launch_sess = self.get_or_create_launch_session()?;
            let entries_conv: Vec<(String, String, String)> = launch_entries
                .iter()
                .map(|(a, b, c, _)| (a.clone(), b.clone(), c.clone()))
                .collect();
            let _ = TmuxLauncher::launch(workspace_name, &entries_conv, &launch_sess);
            return Ok(launch_entries);
        }

        if mode == "attach" || (attach_repo.is_some() && launch_entries.len() == 1) {
            let target_repo = attach_repo
                .map(|s| s.to_string())
                .unwrap_or_else(|| launch_entries[0].0.clone());
            ensure_daemon_running()?;
            let _ = ws_tui::run_raw_bridge(&sock_path, &target_repo);
            return Ok(launch_entries);
        }

        if mode == "tui" {
            ensure_daemon_running()?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            let _ = rt.block_on(async {
                ws_tui::attach_workspace_session(
                    workspace_name.to_string(),
                    sock_path,
                    attach_repo.map(|s| s.to_string()),
                    false,
                )
                .await
            });
            return Ok(launch_entries);
        }

        Ok(launch_entries)
    }

    pub fn get_project_namespace_and_name(
        &self,
        override_identifier: Option<&str>,
    ) -> (String, String) {
        let client = HubClient::default();
        let target_id = override_identifier.or(self.config.hub_project.as_deref());
        if let Some(tid) = target_id {
            if let Ok(pair) = HubClient::parse_project_identifier(tid) {
                return pair;
            }
        }

        let proj_dir = self
            .config
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let clean = proj_dir
            .replace("-workspaces", "")
            .replace("_workspaces", "")
            .to_lowercase();

        let namespace = client
            .whoami()
            .ok()
            .and_then(|u| {
                u.get("username")
                    .and_then(|un| un.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "personal".to_string());

        (namespace, clean)
    }

    pub fn clone_from_hub(
        &self,
        project_identifier: &str,
        target_dir: Option<&Path>,
    ) -> Result<PathBuf, WSError> {
        let client = HubClient::default();
        let (namespace, name) = HubClient::parse_project_identifier(project_identifier)?;

        OutputHandler::print_info(&format!(
            "Connecting to wshub for {}/{}...",
            namespace, name
        ));
        let data = client.get_project(&namespace, &name)?;

        let latest_rev = data.get("latestRevision").ok_or_else(|| {
            WSError::Config(format!(
                "Project '{}/{}' has no valid blueprint revisions.",
                namespace, name
            ))
        })?;

        let blueprint_yaml = latest_rev
            .get("blueprintYaml")
            .and_then(|y| y.as_str())
            .ok_or_else(|| {
                WSError::Config("Latest revision is missing blueprintYaml".to_string())
            })?;

        let dest_dir = if let Some(td) = target_dir {
            td.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_default()
                .join(format!("{}-workspaces", name))
        };

        ensure_directory(&dest_dir)?;
        ensure_directory(&dest_dir.join("bares"))?;
        ensure_directory(&dest_dir.join("workspaces"))?;

        let config_path = dest_dir.join("repositories.yml");
        std::fs::write(&config_path, blueprint_yaml)?;
        OutputHandler::print_success(&format!("Wrote {}", config_path.display()));

        if let Some(scripts_raw) = latest_rev.get("scriptsJson").and_then(|s| s.as_str()) {
            if let Ok(scripts_dict) = serde_json::from_str::<HashMap<String, String>>(scripts_raw) {
                let scripts_dir = dest_dir.join("scripts");
                let _ = ensure_directory(&scripts_dir);
                for (s_name, s_content) in &scripts_dict {
                    let s_file = scripts_dir.join(s_name);
                    let _ = std::fs::write(&s_file, s_content);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(
                            &s_file,
                            std::fs::Permissions::from_mode(0o755),
                        );
                    }
                }
                OutputHandler::print_success(&format!(
                    "Restored {} automation script(s)",
                    scripts_dict.len()
                ));
            }
        }

        // Download sensitive files
        if let Ok(files_list_val) = client.list_files(&namespace, &name) {
            if let Some(files_list) = files_list_val.as_array() {
                let files_dir = dest_dir.join("files");
                let _ = ensure_directory(&files_dir);
                for f_info in files_list {
                    if let Some(rel_path) = f_info.get("filePath").and_then(|p| p.as_str()) {
                        let clean_rel =
                            if rel_path.starts_with("files/") || rel_path.starts_with("files\\") {
                                &rel_path[6..]
                            } else {
                                rel_path
                            };
                        let target_file = files_dir.join(clean_rel);
                        if let Some(parent) = target_file.parent() {
                            let _ = ensure_directory(parent);
                        }
                        if let Ok(file_bytes) = client.download_file(&namespace, &name, rel_path) {
                            let _ = std::fs::write(target_file, file_bytes);
                        }
                    }
                }
                OutputHandler::print_success(&format!(
                    "Downloaded {} secret file(s) from vault",
                    files_list.len()
                ));
            }
        }

        // Restore secrets
        if let Ok(secrets_list_val) = client.list_secrets(&namespace, &name) {
            if let Some(secrets_list) = secrets_list_val.as_array() {
                if let Ok(mut cfg) = ConfigLoader::load_config(Some(&config_path), None, true) {
                    for s_item in secrets_list {
                        let s_key = s_item.get("key").and_then(|k| k.as_str());
                        let s_val = s_item.get("value").and_then(|v| v.as_str());
                        let s_repo = s_item.get("repoName").and_then(|r| r.as_str());
                        if let (Some(k), Some(v)) = (s_key, s_val) {
                            if s_repo.is_none() || s_repo == Some("global") {
                                cfg.secret_env.insert(k.to_string(), v.to_string());
                            } else if let Some(r_name) = s_repo {
                                if let Some(r_cfg) = cfg.repositories.get_mut(r_name) {
                                    r_cfg.secret_env.insert(k.to_string(), v.to_string());
                                }
                            }
                        }
                    }
                    let _ = ConfigLoader::save_config(
                        &cfg.repositories,
                        Some(&config_path),
                        cfg.tmux.as_ref(),
                        cfg.hub_auto_save.as_ref(),
                        cfg.hub_project.as_deref(),
                    );
                    OutputHandler::print_success(&format!(
                        "Restored {} secret(s) from vault into local config",
                        secrets_list.len()
                    ));
                }
            }
        }

        // Clone bare repos
        let loaded_cfg = ConfigLoader::load_config(
            Some(&config_path),
            Some(&dest_dir.join("workspaces")),
            true,
        )?;
        for (r_name, r_cfg) in &loaded_cfg.repositories {
            if let Some(url) = &r_cfg.url {
                let bare_path = dest_dir.join(&r_cfg.bare);
                if let Some(parent) = bare_path.parent() {
                    let _ = ensure_directory(parent);
                }
                if !self.git.is_bare_repo(&bare_path) {
                    OutputHandler::print_info(&format!(
                        "Cloning bare repository {} from {}...",
                        r_name, url
                    ));
                    self.git.clone_bare(url, &bare_path)?;
                    OutputHandler::print_success(&format!("Cloned {}", bare_path.display()));
                }
            }
        }

        Ok(dest_dir)
    }

    pub fn hub_publish(
        &mut self,
        project_identifier: Option<&str>,
        description: Option<&str>,
        silent: bool,
    ) -> Result<serde_json::Value, WSError> {
        let client = HubClient::default();
        let (namespace, name) = self.get_project_namespace_and_name(project_identifier);

        let config_file = self
            .config
            .config_file_path
            .clone()
            .unwrap_or_else(|| self.config.project_root.join("repositories.yml"));
        if !config_file.exists() {
            return Err(WSError::Config(
                "No 'repositories.yml' found in project root to publish.".to_string(),
            ));
        }

        let proj_ident = format!("{}/{}", namespace, name);
        if self.config.hub_project.as_deref() != Some(&proj_ident) {
            let _ = ConfigLoader::update_hub_config(&config_file, Some(&proj_ident), None, None);
            self.config.hub_project = Some(proj_ident);
        }

        let (sanitized_yaml, extracted_secrets, files_to_upload, private_count) =
            ConfigLoader::classify_project_assets(&self.config);

        if !silent {
            OutputHandler::print_info(&format!(
                "Publishing project {}/{} to wshub...",
                namespace, name
            ));
        }

        let result = client.create_project(&namespace, &name, description, false)?;

        let _ = client.push_revision(
            &namespace,
            &name,
            &sanitized_yaml,
            Some("Initial publish from local workspace"),
            None,
        );

        if !silent {
            OutputHandler::print_success(&format!(
                "Published project {}/{} (Revision v1)",
                namespace, name
            ));
        }

        // Sync secrets
        let mut total_secrets = 0;
        let mut flat_secrets = Vec::new();
        for (scope, sec_dict) in &extracted_secrets {
            let repo_param = if scope == "global" {
                None
            } else {
                Some(scope.clone())
            };
            for (k, v) in sec_dict {
                flat_secrets.push((k.clone(), v.clone(), repo_param.clone()));
                total_secrets += 1;
            }
        }
        if !flat_secrets.is_empty() {
            let _ = client.set_secrets_bulk(&namespace, &name, &flat_secrets);
        }

        if !silent && total_secrets > 0 {
            OutputHandler::print_success(&format!(
                "Stored and encrypted {} secret(s) in Vault",
                total_secrets
            ));
        }

        // Sync files
        for f_path in &files_to_upload {
            let rel_path = f_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Ok(file_bytes) = std::fs::read(f_path) {
                let _ = client.upload_file(&namespace, &name, &rel_path, file_bytes);
            }
        }
        if !silent && !files_to_upload.is_empty() {
            OutputHandler::print_success(&format!(
                "Encrypted and uploaded {} sensitive file(s)",
                files_to_upload.len()
            ));
        }

        if !silent && private_count > 0 {
            OutputHandler::print_info(&format!(
                "Skipped {} private variable(s) (kept local)",
                private_count
            ));
        }

        Ok(result)
    }

    pub fn hub_push(
        &mut self,
        message: &str,
        project_identifier: Option<&str>,
        silent: bool,
    ) -> Result<serde_json::Value, WSError> {
        let client = HubClient::default();
        let (namespace, name) = self.get_project_namespace_and_name(project_identifier);

        let config_file = self
            .config
            .config_file_path
            .clone()
            .unwrap_or_else(|| self.config.project_root.join("repositories.yml"));
        if !config_file.exists() {
            return Err(WSError::Config(
                "No 'repositories.yml' found in project root.".to_string(),
            ));
        }

        let (sanitized_yaml, extracted_secrets, files_to_upload, private_count) =
            ConfigLoader::classify_project_assets(&self.config);

        let result =
            match client.push_revision(&namespace, &name, &sanitized_yaml, Some(message), None) {
                Ok(r) => r,
                Err(e) => {
                    if e.to_string().contains("404")
                        || e.to_string().to_lowercase().contains("not found")
                    {
                        return self.hub_publish(
                            project_identifier,
                            Some("Auto-published on push"),
                            silent,
                        );
                    }
                    return Err(e);
                }
            };

        let version = result
            .get("revision")
            .and_then(|r| r.get("version"))
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".to_string());

        if !silent {
            OutputHandler::print_success(&format!(
                "Pushed revision v{} to {}/{}",
                version, namespace, name
            ));
        }

        // Sync secrets
        let mut flat_secrets = Vec::new();
        for (scope, sec_dict) in &extracted_secrets {
            let repo_param = if scope == "global" {
                None
            } else {
                Some(scope.clone())
            };
            for (k, v) in sec_dict {
                flat_secrets.push((k.clone(), v.clone(), repo_param.clone()));
            }
        }
        if !flat_secrets.is_empty() {
            let _ = client.set_secrets_bulk(&namespace, &name, &flat_secrets);
        }

        // Upload files
        for f_path in &files_to_upload {
            let rel_path = f_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Ok(file_bytes) = std::fs::read(f_path) {
                let _ = client.upload_file(&namespace, &name, &rel_path, file_bytes);
            }
        }

        if !silent && private_count > 0 {
            OutputHandler::print_info(&format!(
                "Skipped {} private variable(s) (kept local)",
                private_count
            ));
        }

        Ok(result)
    }

    pub fn hub_pull(
        &mut self,
        project_identifier: Option<&str>,
    ) -> Result<serde_json::Value, WSError> {
        let client = HubClient::default();
        let (namespace, name) = self.get_project_namespace_and_name(project_identifier);

        let data = client.get_project(&namespace, &name)?;
        let latest_rev = data.get("latestRevision").ok_or_else(|| {
            WSError::Config(format!(
                "Project '{}/{}' has no revisions.",
                namespace, name
            ))
        })?;

        let blueprint_yaml = latest_rev
            .get("blueprintYaml")
            .and_then(|y| y.as_str())
            .unwrap_or("");
        let config_file = self
            .config
            .config_file_path
            .clone()
            .unwrap_or_else(|| self.config.project_root.join("repositories.yml"));

        std::fs::write(&config_file, blueprint_yaml)?;
        OutputHandler::print_success(&format!("Updated {} to revision", config_file.display()));

        Ok(data)
    }

    pub fn hub_state_save(
        &self,
        workspace_name: &str,
        project_identifier: Option<&str>,
        include_wip: bool,
        silent: bool,
        is_auto: bool,
    ) -> Result<serde_json::Value, WSError> {
        let client = HubClient::default();
        let (namespace, name) = self.get_project_namespace_and_name(project_identifier);

        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;
        let mut state_val = serde_json::to_value(&meta)?;
        let state_map = state_val.as_object_mut().unwrap();

        if is_auto {
            state_map.insert("auto_saved".to_string(), serde_json::Value::Bool(true));
            state_map.insert(
                "saved_at".to_string(),
                serde_json::Value::String(get_iso_timestamp()),
            );
        }

        let mut wip_summary = Vec::new();
        if include_wip {
            let mut wip_dict = serde_json::Map::new();
            for (r_name, spec) in &meta.repositories {
                let wt_path = ws_dir.join(&spec.path);
                if !wt_path.exists() {
                    continue;
                }

                let diff = self.git.get_uncommitted_diff(&wt_path);
                let untracked = self.git.get_untracked_files(&wt_path);
                let mut untracked_map = serde_json::Map::new();

                for u in &untracked {
                    let file_p = wt_path.join(u);
                    if file_p.is_file() {
                        if let Ok(bytes) = std::fs::read(&file_p) {
                            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                            untracked_map.insert(u.clone(), serde_json::Value::String(encoded));
                        }
                    }
                }

                if !diff.trim().is_empty() || !untracked_map.is_empty() {
                    let diff_file_count =
                        diff.lines().filter(|l| l.starts_with("diff --git")).count();
                    let mut r_wip = serde_json::Map::new();
                    r_wip.insert("diff".to_string(), serde_json::Value::String(diff));
                    r_wip.insert(
                        "untracked".to_string(),
                        serde_json::Value::Object(untracked_map.clone()),
                    );
                    wip_dict.insert(r_name.clone(), serde_json::Value::Object(r_wip));
                    wip_summary.push((r_name.clone(), diff_file_count, untracked_map.len()));
                }
            }

            if !wip_dict.is_empty() {
                state_map.insert("wip".to_string(), serde_json::Value::Object(wip_dict));
            }
        }

        let result = client.save_workspace_state(&namespace, &name, &meta.name, &state_val)?;

        if !silent {
            let prefix = if is_auto { "Auto-saved" } else { "Saved" };
            OutputHandler::print_success(&format!(
                "{} workspace state @{} to {}/{}",
                prefix, meta.name, namespace, name
            ));
            for (r_name, mod_cnt, untr_cnt) in wip_summary {
                OutputHandler::print_info(&format!(
                    "  Captured uncommitted work in %{} ({} modified, {} untracked)",
                    r_name, mod_cnt, untr_cnt
                ));
            }
        }

        Ok(result)
    }

    pub fn get_auto_save_cache_file(&self) -> PathBuf {
        self.config.workspaces_dir.join(".auto_save_cache.json")
    }

    pub fn load_auto_save_cache(&self) -> HashMap<String, serde_json::Value> {
        let p = self.get_auto_save_cache_file();
        if p.exists() {
            if let Ok(c) = std::fs::read_to_string(p) {
                if let Ok(val) = serde_json::from_str(&c) {
                    return val;
                }
            }
        }
        HashMap::new()
    }

    pub fn save_auto_save_cache(
        &self,
        cache: &HashMap<String, serde_json::Value>,
    ) -> Result<(), WSError> {
        let p = self.get_auto_save_cache_file();
        if let Some(parent) = p.parent() {
            ensure_directory(parent)?;
        }
        let content = serde_json::to_string_pretty(cache)?;
        std::fs::write(p, content)?;
        Ok(())
    }

    pub fn get_workspace_fingerprint(
        &self,
        workspace_name: &str,
        include_wip: bool,
    ) -> Result<String, WSError> {
        let (meta, ws_dir) = self.get_workspace_info(workspace_name)?;
        let mut hasher = Sha256::new();
        hasher.update(meta.name.as_bytes());

        let mut sorted_repos: Vec<(&String, &RepoSpec)> = meta.repositories.iter().collect();
        sorted_repos.sort_by_key(|(k, _)| (*k).clone());

        for (r_name, spec) in sorted_repos {
            hasher.update(
                format!("{}:{}:{}:{}", r_name, spec.branch, spec.frozen, spec.path).as_bytes(),
            );
            let wt_path = ws_dir.join(&spec.path);
            if wt_path.exists() {
                let head = self.git.get_head_commit(&wt_path).unwrap_or_default();
                hasher.update(format!("head:{}", head).as_bytes());

                if include_wip {
                    let uncommitted = self.git.check_worktree_uncommitted(&wt_path);
                    hasher.update(
                        format!("has_uncommitted:{}", uncommitted.has_uncommitted).as_bytes(),
                    );
                    for m in &uncommitted.modified {
                        hasher.update(format!("mod:{}", m).as_bytes());
                    }
                    for u in &uncommitted.untracked {
                        let fp = wt_path.join(u);
                        if let Ok(md) = fp.metadata() {
                            hasher.update(format!("untr:{}:{}", u, md.len()).as_bytes());
                        } else {
                            hasher.update(format!("untr:{}", u).as_bytes());
                        }
                    }
                }
            }
        }

        Ok(hex::encode(hasher.finalize()))
    }

    pub fn hub_auto_save_workspace(
        &self,
        workspace_name: &str,
        project_identifier: Option<&str>,
        include_wip: bool,
        force: bool,
        silent: bool,
    ) -> Result<bool, WSError> {
        if !self.has_workspace(workspace_name) {
            return Ok(false);
        }

        let current_fp = self.get_workspace_fingerprint(workspace_name, include_wip)?;
        let mut cache = self.load_auto_save_cache();
        let last_fp = cache
            .get(workspace_name)
            .and_then(|v| v.get("fingerprint"))
            .and_then(|f| f.as_str());

        if !force && last_fp == Some(&current_fp) {
            return Ok(false);
        }

        let should_notify = self
            .config
            .hub_auto_save
            .as_ref()
            .map(|a| a.notify)
            .unwrap_or(true);
        let (namespace, p_name) = self.get_project_namespace_and_name(project_identifier);

        match self.hub_state_save(
            workspace_name,
            project_identifier,
            include_wip,
            silent,
            true,
        ) {
            Ok(_) => {
                let mut map = serde_json::Map::new();
                map.insert(
                    "fingerprint".to_string(),
                    serde_json::Value::String(current_fp),
                );
                map.insert(
                    "last_saved_at".to_string(),
                    serde_json::Value::String(get_iso_timestamp()),
                );
                cache.insert(workspace_name.to_string(), serde_json::Value::Object(map));
                let _ = self.save_auto_save_cache(&cache);

                if should_notify {
                    crate::notify::notify_auto_save_success(
                        workspace_name,
                        Some(&format!("{}/{}", namespace, p_name)),
                    );
                }
                Ok(true)
            }
            Err(e) => {
                if should_notify {
                    crate::notify::notify_auto_save_failure(
                        workspace_name,
                        &e.to_string(),
                        Some(&format!("{}/{}", namespace, p_name)),
                    );
                }
                Err(e)
            }
        }
    }

    pub fn hub_auto_save_all_workspaces(
        &self,
        project_identifier: Option<&str>,
        force: bool,
        silent: bool,
    ) -> HashMap<String, bool> {
        let auto_cfg = self.config.hub_auto_save.as_ref();
        let target_setting = auto_cfg
            .map(|a| match &a.workspaces {
                WorkspacesSelector::Mode(s) => s.as_str(),
                WorkspacesSelector::List(_) => "list",
            })
            .unwrap_or("all");
        let include_wip = auto_cfg.map(|a| a.include_wip).unwrap_or(true);

        let all_meta = self.list_workspaces();
        let mut target_names = Vec::new();

        for m in all_meta {
            if target_setting == "active" {
                if self.is_session_running(&m.name) {
                    target_names.push(m.name);
                }
            } else if target_setting == "list" {
                if let Some(HubAutoSaveConfig {
                    workspaces: WorkspacesSelector::List(l),
                    ..
                }) = auto_cfg
                {
                    if l.contains(&m.name) {
                        target_names.push(m.name);
                    }
                }
            } else {
                target_names.push(m.name);
            }
        }

        let mut results = HashMap::new();
        for w_name in target_names {
            match self.hub_auto_save_workspace(
                &w_name,
                project_identifier,
                include_wip,
                force,
                silent,
            ) {
                Ok(saved) => {
                    results.insert(w_name, saved);
                }
                Err(_) => {
                    results.insert(w_name, false);
                }
            }
        }
        results
    }

    pub fn get_auto_save_pid_file(&self) -> PathBuf {
        self.config.workspaces_dir.join(".auto_save.pid")
    }

    pub fn is_auto_save_daemon_active(&self) -> (bool, Option<i32>) {
        let pid_file = self.get_auto_save_pid_file();
        if !pid_file.exists() {
            return (false, None);
        }
        if let Ok(c) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = c.trim().parse::<i32>() {
                let res = unsafe { libc::kill(pid, 0) };
                if res == 0 {
                    return (true, Some(pid));
                } else {
                    let _ = std::fs::remove_file(&pid_file);
                }
            }
        }
        (false, None)
    }

    pub fn start_auto_save_daemon(
        &self,
        interval: Option<u64>,
        project_identifier: Option<&str>,
        detached: bool,
    ) -> Result<i32, WSError> {
        let (active, existing_pid) = self.is_auto_save_daemon_active();
        let cur_pid = std::process::id() as i32;
        if active && existing_pid != Some(cur_pid) {
            return Err(WSError::Workspace(format!(
                "Auto-save daemon is already running (PID {}).",
                existing_pid.unwrap_or(0)
            )));
        }

        let eff_interval = interval.unwrap_or_else(|| {
            self.config
                .hub_auto_save
                .as_ref()
                .map(|a| a.interval)
                .unwrap_or(900)
        });

        let pid_file = self.get_auto_save_pid_file();
        ensure_directory(pid_file.parent().unwrap())?;

        if detached {
            let log_file = self.config.workspaces_dir.join(".auto_save.log");
            let log_out = File::options().create(true).append(true).open(&log_file)?;

            let current_exe = std::env::current_exe()?;
            let mut cmd = Command::new(current_exe);
            if let Some(cf) = &self.config.config_file_path {
                cmd.args(["-c", &cf.display().to_string()]);
            }
            cmd.args([
                "hub",
                "auto-save",
                "run",
                "--interval",
                &eff_interval.to_string(),
            ]);
            if let Some(pi) = project_identifier {
                cmd.args(["--project", pi]);
            }

            let child = cmd
                .current_dir(&self.config.project_root)
                .stdout(log_out.try_clone()?)
                .stderr(log_out)
                .spawn()?;

            let pid = child.id() as i32;
            std::fs::write(&pid_file, pid.to_string())?;
            Ok(pid)
        } else {
            std::fs::write(&pid_file, cur_pid.to_string())?;
            self.run_auto_save_loop(eff_interval, project_identifier);
            if pid_file.exists() {
                let _ = std::fs::remove_file(&pid_file);
            }
            Ok(cur_pid)
        }
    }

    pub fn stop_auto_save_daemon(&self) -> Result<bool, WSError> {
        let (active, pid) = self.is_auto_save_daemon_active();
        if !active || pid.is_none() {
            return Ok(false);
        }
        let target_pid = pid.unwrap();
        unsafe {
            libc::kill(target_pid, libc::SIGTERM);
        }
        let pid_file = self.get_auto_save_pid_file();
        if pid_file.exists() {
            let _ = std::fs::remove_file(&pid_file);
        }
        Ok(true)
    }

    pub fn run_auto_save_loop(&self, interval: u64, project_identifier: Option<&str>) {
        let dur = std::time::Duration::from_secs(interval);
        loop {
            let _ = self.hub_auto_save_all_workspaces(project_identifier, false, true);
            std::thread::sleep(dur);
        }
    }

    pub fn get_auto_save_status(&self) -> serde_json::Value {
        let auto_cfg = self.config.hub_auto_save.as_ref();
        let (active, pid) = self.is_auto_save_daemon_active();
        let cache = self.load_auto_save_cache();

        let all_ws = self.list_workspaces();
        let mut ws_info = serde_json::Map::new();

        for m in all_ws {
            let c_entry = cache.get(&m.name);
            let last_saved = c_entry.and_then(|v| v.get("last_saved_at")).cloned();
            let mut dirty = false;
            if let Ok((_, ws_dir)) = self.get_workspace_info(&m.name) {
                for spec in m.repositories.values() {
                    let wt_p = ws_dir.join(&spec.path);
                    if wt_p.exists() {
                        let u = self.git.check_worktree_uncommitted(&wt_p);
                        if u.has_uncommitted {
                            dirty = true;
                            break;
                        }
                    }
                }
            }

            let mut w_map = serde_json::Map::new();
            w_map.insert(
                "last_saved_at".to_string(),
                last_saved.unwrap_or(serde_json::Value::Null),
            );
            w_map.insert(
                "has_uncommitted".to_string(),
                serde_json::Value::Bool(dirty),
            );
            w_map.insert(
                "active_session".to_string(),
                serde_json::Value::Bool(self.is_session_running(&m.name)),
            );
            ws_info.insert(m.name, serde_json::Value::Object(w_map));
        }

        serde_json::json!({
            "enabled": auto_cfg.map(|a| a.enabled).unwrap_or(false),
            "interval": auto_cfg.map(|a| a.interval).unwrap_or(900),
            "include_wip": auto_cfg.map(|a| a.include_wip).unwrap_or(true),
            "workspaces_setting": auto_cfg.map(|a| a.workspaces.clone()).unwrap_or_else(|| WorkspacesSelector::Mode("all".to_string())),
            "notify": auto_cfg.map(|a| a.notify).unwrap_or(true),
            "daemon_active": active,
            "daemon_pid": pid,
            "workspaces": ws_info,
        })
    }

    pub fn hub_state_restore(
        &self,
        workspace_name: &str,
        project_identifier: Option<&str>,
        apply_wip: bool,
    ) -> Result<(), WSError> {
        let client = HubClient::default();
        let (namespace, name) = self.get_project_namespace_and_name(project_identifier);

        let state_val = client.get_workspace_state(&namespace, &name, workspace_name)?;
        let meta: WorkspaceMetadata = serde_json::from_value(state_val.clone()).map_err(|e| {
            WSError::Config(format!(
                "Invalid saved state for '{}': {}",
                workspace_name, e
            ))
        })?;

        let repo_specs: Vec<RepoSpec> = meta.repositories.values().cloned().collect();
        OutputHandler::print_info(&format!(
            "Recreating workspace @{} from hub state...",
            meta.name
        ));
        self.create_workspace(&meta.name, &repo_specs, None, false)?;

        if apply_wip {
            if let Some(wip_data) = state_val.get("wip").and_then(|w| w.as_object()) {
                let (_, ws_dir) = self.get_workspace_info(&meta.name)?;
                for (r_name, r_wip) in wip_data {
                    let spec = meta.repositories.get(r_name);
                    let wt_path = ws_dir.join(spec.map(|s| s.path.as_str()).unwrap_or(r_name));
                    if !wt_path.exists() {
                        continue;
                    }

                    if let Some(untr_map) = r_wip.get("untracked").and_then(|u| u.as_object()) {
                        for (rel_p, b64_val) in untr_map {
                            if let Some(b64_str) = b64_val.as_str() {
                                if let Ok(bytes) =
                                    base64::engine::general_purpose::STANDARD.decode(b64_str)
                                {
                                    let dest = wt_path.join(rel_p);
                                    if let Some(parent) = dest.parent() {
                                        let _ = ensure_directory(parent);
                                    }
                                    let _ = std::fs::write(dest, bytes);
                                }
                            }
                        }
                    }

                    if let Some(diff_str) = r_wip.get("diff").and_then(|d| d.as_str()) {
                        if !diff_str.trim().is_empty() {
                            let _ = self.git.apply_patch(&wt_path, diff_str);
                        }
                    }
                    OutputHandler::print_success(&format!(
                        "Restored uncommitted work in %{}",
                        r_name
                    ));
                }
            }
        }

        OutputHandler::print_success(&format!("Restored workspace @{} successfully", meta.name));
        Ok(())
    }
}

fn rand_or_time_u16() -> u16 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_millis() & 0xFFFF) as u16
}
