use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::env::EnvEngine;
use crate::errors::{Result, WSError};
use crate::models::AppConfig;
use crate::output::OutputHandler;
use crate::utils::get_iso_timestamp;
use crate::workspace::run_shell_command;

pub const LIFECYCLE_SCHEMA_VERSION: u32 = 3;
pub const PHASE_CONFIGURE: &str = "configure";
pub const PHASE_SETUP: &str = "setup";
pub const DEFAULT_SETUP_STAGE: &str = "install";
pub const DEFAULT_READY_STAGE: &str = "verify";

/// Lifecycle event stages that can also be invoked explicitly by `ws lifecycle`.
pub const EVENT_STAGES: &[&str] = &[
    "pre_launch",
    "post_launch",
    "pre_stop",
    "post_stop",
    "pre_end",
    "post_end",
    "repo_add",
    "repo_remove",
];

pub fn is_event_stage(stage: &str) -> bool {
    EVENT_STAGES.contains(&stage)
}

/// When a hook or stage should run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum HookWhen {
    /// Run only if not previously completed successfully for the same fingerprint/context.
    #[default]
    Once,
    /// Always run when stage is executed.
    Always,
    /// Re-run when inputs/fingerprint changed (best-effort; currently treated as once+fingerprint).
    IfStale,
}

/// A single lifecycle hook/command.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LifecycleHook {
    /// Shell command to execute.
    #[serde(default, alias = "command", alias = "cmd")]
    pub run: Option<String>,
    /// Optional display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Execution condition.
    #[serde(default)]
    pub when: HookWhen,
    /// Optional working directory relative to target cwd.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Continue executing remaining hooks in stage on failure.
    #[serde(default)]
    pub continue_on_error: bool,
}

impl LifecycleHook {
    pub fn from_value(val: &serde_json::Value) -> Option<Self> {
        if let Some(s) = val.as_str() {
            return Some(Self {
                run: Some(s.to_string()),
                name: None,
                when: HookWhen::Once,
                cwd: None,
                continue_on_error: false,
            });
        }
        if let Some(obj) = val.as_object() {
            let run = obj
                .get("run")
                .or_else(|| obj.get("command"))
                .or_else(|| obj.get("cmd"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if run.is_none() {
                return None;
            }
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let when = match obj.get("when").and_then(|v| v.as_str()).unwrap_or("once") {
                "always" => HookWhen::Always,
                "if-stale" | "if_stale" | "stale" => HookWhen::IfStale,
                _ => HookWhen::Once,
            };
            let cwd = obj
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let continue_on_error = obj
                .get("continue_on_error")
                .or_else(|| obj.get("allow_failure"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            return Some(Self {
                run,
                name,
                when,
                cwd,
                continue_on_error,
            });
        }
        None
    }

    pub fn hooks_from_value(val: Option<&serde_json::Value>) -> Vec<Self> {
        let Some(val) = val else {
            return Vec::new();
        };
        if let Some(s) = val.as_str() {
            return vec![Self {
                run: Some(s.to_string()),
                name: None,
                when: HookWhen::Once,
                cwd: None,
                continue_on_error: false,
            }];
        }
        if let Some(arr) = val.as_array() {
            let mut out = Vec::new();
            for item in arr {
                if let Some(h) = Self::from_value(item) {
                    out.push(h);
                } else if let Some(s) = item.as_str() {
                    out.push(Self {
                        run: Some(s.to_string()),
                        name: None,
                        when: HookWhen::Once,
                        cwd: None,
                        continue_on_error: false,
                    });
                }
            }
            return out;
        }
        if let Some(h) = Self::from_value(val) {
            return vec![h];
        }
        Vec::new()
    }
}

/// A named setup stage within the lifecycle `setup` phase.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetupStage {
    pub name: String,
    #[serde(default)]
    pub when: HookWhen,
    #[serde(default)]
    pub hooks: Vec<LifecycleHook>,
}

/// Parsed lifecycle stage map for a scope.
#[derive(Debug, Clone, Default)]
pub struct LifecycleScope {
    /// Stage keys are canonical internal keys, notably:
    /// - `configure`
    /// - `setup:<stage-name>`
    /// - lifecycle event names such as `pre_launch`, `repo_add`, etc.
    pub stages: HashMap<String, Vec<LifecycleHook>>,
    /// Ordered setup stage names discovered from the sequence-based `setup` phase.
    pub setup_order: Vec<String>,
}

impl LifecycleScope {
    pub fn from_value(val: Option<&serde_json::Value>) -> Self {
        let mut stages = HashMap::new();
        let mut setup_order = Vec::new();
        let Some(val) = val else {
            return Self {
                stages,
                setup_order,
            };
        };
        let Some(obj) = val.as_object() else {
            return Self {
                stages,
                setup_order,
            };
        };

        for (k, v) in obj {
            if k == "configure" {
                let hooks = LifecycleHook::hooks_from_value(Some(v));
                if !hooks.is_empty() {
                    stages.insert("configure".to_string(), hooks);
                }
                continue;
            }
            if k == "setup" {
                if let Some(arr) = v.as_array() {
                    for item in arr {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            let name = name.trim().to_string();
                            if name.is_empty() || name == "setup" {
                                continue;
                            }
                            let stage_hooks = item.get("hooks").or_else(|| item.get("command"));
                            let hooks = LifecycleHook::hooks_from_value(stage_hooks);
                            stages.insert(format!("setup:{name}"), hooks);
                            setup_order.push(name);
                        }
                    }
                } else if let Some(map) = v.as_object() {
                    for (stage_name, stage_val) in map {
                        let stage_name_str = stage_name.as_str();
                        let name = stage_name_str.trim().to_string();
                        if name.is_empty() || name == "setup" {
                            continue;
                        }
                        let hooks = LifecycleHook::hooks_from_value(Some(stage_val));
                        stages.insert(format!("setup:{name}"), hooks);
                        setup_order.push(name);
                    }
                }
                continue;
            }

            let hooks = LifecycleHook::hooks_from_value(Some(v));
            if !hooks.is_empty() {
                stages.insert(k.clone(), hooks);
            }
        }

        Self {
            stages,
            setup_order,
        }
    }

    pub fn hooks_for(&self, stage: &str) -> &[LifecycleHook] {
        self.stages.get(stage).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Persisted lifecycle state under .ws/state/lifecycle.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LifecycleState {
    pub version: u32,
    /// key -> record. Keys are phase-qualified, e.g. `configure`, `setup:install`,
    /// or lifecycle event names such as `pre_launch`.
    #[serde(default)]
    pub records: HashMap<String, StageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StageRecord {
    pub status: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LifecycleState {
    pub fn path(ws_dir: &Path) -> PathBuf {
        ws_dir.join(".ws").join("state").join("lifecycle.json")
    }

    pub fn load(ws_dir: &Path) -> Self {
        let path = Self::path(ws_dir);
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(state) = serde_json::from_str::<Self>(&content) {
                return state;
            }
        }
        Self {
            version: LIFECYCLE_SCHEMA_VERSION,
            records: HashMap::new(),
        }
    }

    pub fn save(&self, ws_dir: &Path) -> Result<()> {
        let path = Self::path(ws_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content =
            serde_json::to_string_pretty(self).map_err(|e| WSError::Validation(e.to_string()))?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn key(repo: Option<&str>, stage: &str) -> String {
        match repo {
            Some(r) if !r.is_empty() => format!("repo:{}:{}", r, stage),
            _ => format!("workspace:{}", stage),
        }
    }

    pub fn is_done(&self, key: &str, fingerprint: Option<&str>, when: &HookWhen) -> bool {
        let Some(rec) = self.records.get(key) else {
            return false;
        };
        if rec.status != "success" {
            return false;
        }
        match when {
            HookWhen::Always => false,
            HookWhen::Once => true,
            HookWhen::IfStale => match (fingerprint, rec.fingerprint.as_deref()) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            },
        }
    }

    pub fn mark(
        &mut self,
        key: String,
        status: &str,
        fingerprint: Option<String>,
        message: Option<String>,
    ) {
        self.records.insert(
            key,
            StageRecord {
                status: status.to_string(),
                updated_at: get_iso_timestamp(),
                fingerprint,
                message,
            },
        );
    }
}

#[derive(Debug, Clone, Default)]
pub struct StageRunOptions {
    pub dry_run: bool,
    pub force: bool,
    pub verbose: bool,
    pub interface: Option<String>,
    pub lan_ip: Option<String>,
}

pub struct StageContext<'a> {
    pub config: &'a AppConfig,
    pub workspace_name: &'a str,
    pub ws_dir: &'a Path,
    pub repo_name: Option<&'a str>,
    pub work_dir: &'a Path,
    pub env_vars: &'a HashMap<String, String>,
}

/// Compute a stable fingerprint for a stage/repo context.
pub fn compute_stage_fingerprint(
    stage: &str,
    repo_name: Option<&str>,
    config: &AppConfig,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(stage.to_string());
    if let Some(r) = repo_name {
        parts.push(r.to_string());
        if let Some(rc) = config.repositories.get(r) {
            parts.push(format!("{:?}", rc.setup));
            parts.push(format!("{:?}", rc.copy_files));
            parts.push(format!("{:?}", rc.env));
            parts.push(format!("{:?}", rc.secret_env));
            parts.push(format!("{:?}", rc.private_env));
            parts.push(format!("{:?}", rc.lifecycle));
        }
    } else {
        parts.push(format!("{:?}", config.setup));
        parts.push(format!("{:?}", config.copy_files));
        parts.push(format!("{:?}", config.global_env));
        parts.push(format!("{:?}", config.lifecycle));
    }
    let joined = parts.join("|");
    let mut h: u64 = 1469598103934665603;
    for b in joined.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{:016x}", h)
}

fn expanded_command(
    ctx: &StageContext<'_>,
    opts: &StageRunOptions,
    hook: &LifecycleHook,
) -> (String, String) {
    let expanded = EnvEngine::expand_command(
        hook.run.as_deref().unwrap_or(""),
        ctx.env_vars,
        ctx.workspace_name,
        ctx.repo_name.unwrap_or(""),
        EnvEngine::get_workspace_slot(&ctx.config.workspaces_dir, ctx.workspace_name),
        Some(&ctx.config.project_root),
        Some(&ctx.config.workspaces_dir),
        None,
        opts.lan_ip
            .as_deref()
            .or(ctx.env_vars.get("WS_LAN_IP").map(|s| s.as_str())),
        ctx.env_vars.get("PUBLIC_HOST").map(|s| s.as_str()),
        opts.interface.as_deref(),
    );
    let label = hook
        .name
        .clone()
        .unwrap_or_else(|| format!("hook {}", ctx.repo_name.unwrap_or("workspace")));
    (label, expanded)
}

/// Execute a list of hooks for a lifecycle stage, updating lifecycle state.
pub fn run_hooks(
    stage: &str,
    hooks: &[LifecycleHook],
    state: &mut LifecycleState,
    ws_dir: &Path,
    ctx: &StageContext<'_>,
    opts: &StageRunOptions,
) -> Result<bool> {
    if hooks.is_empty() {
        return Ok(true);
    }
    let key = LifecycleState::key(ctx.repo_name, stage);
    let fingerprint = Some(compute_stage_fingerprint(stage, ctx.repo_name, ctx.config));

    let all_skippable = hooks.iter().all(|h| {
        h.when != HookWhen::Always && state.is_done(&key, fingerprint.as_deref(), &h.when)
    });
    if all_skippable && !opts.force {
        OutputHandler::print_setup_step(
            0,
            &format!("Lifecycle: {}", stage),
            "skipped (already completed)",
            "info",
        );
        for (i, hook) in hooks.iter().enumerate() {
            if hook.run.is_some() {
                let (label, expanded) = expanded_command(ctx, opts, hook);
                OutputHandler::print_setup_step(
                    i + 1,
                    &format!("[skipped] {}", label),
                    &expanded,
                    "info",
                );
            }
        }
        return Ok(true);
    }

    let mut any_ran = false;
    for (i, hook) in hooks.iter().enumerate() {
        if hook.run.is_none() {
            continue;
        }
        let (label, expanded) = expanded_command(ctx, opts, hook);
        let hook_done = hook.when != HookWhen::Always
            && state.is_done(&key, fingerprint.as_deref(), &hook.when);
        if hook_done && !opts.force {
            OutputHandler::print_setup_step(
                i + 1,
                &format!("[skipped] {}", label),
                &expanded,
                "info",
            );
            continue;
        }

        if opts.dry_run {
            OutputHandler::print_setup_step(
                i + 1,
                &format!("[DRY-RUN] {}", label),
                &expanded,
                "info",
            );
            continue;
        }

        let cwd = if let Some(rel) = &hook.cwd {
            ctx.work_dir.join(rel)
        } else {
            ctx.work_dir.to_path_buf()
        };

        OutputHandler::print_command_start(&expanded);
        let t0 = Instant::now();
        let ret = run_shell_command(&expanded, &cwd, Some(ctx.env_vars));
        let elapsed = t0.elapsed().as_secs_f64();
        OutputHandler::print_command_done(&expanded, elapsed, ret == 0, ret);
        any_ran = true;

        if ret != 0 && !hook.continue_on_error {
            state.mark(
                key,
                "failed",
                fingerprint.clone(),
                Some(format!("hook '{}' failed", label)),
            );
            let _ = state.save(ws_dir);
            return Err(WSError::Validation(format!(
                "Lifecycle stage '{}' failed at hook '{}'",
                stage, label
            )));
        }
    }

    if opts.dry_run {
        return Ok(true);
    }

    if any_ran || !all_skippable {
        state.mark(key, "success", fingerprint, None);
        let _ = state.save(ws_dir);
    }
    Ok(true)
}

/// Get lifecycle hooks for a stage with no legacy runtime fallback.
pub fn stage_hooks_for_repo(
    config: &AppConfig,
    repo_name: Option<&str>,
    stage: &str,
) -> Vec<LifecycleHook> {
    let scope = if let Some(r) = repo_name {
        let Some(rc) = config.repositories.get(r) else {
            return Vec::new();
        };
        LifecycleScope::from_value(rc.lifecycle.as_ref())
    } else {
        LifecycleScope::from_value(config.lifecycle.as_ref())
    };
    scope.hooks_for(stage).to_vec()
}

pub fn workspace_stage_hooks(config: &AppConfig, stage: &str) -> Vec<LifecycleHook> {
    stage_hooks_for_repo(config, None, stage)
}

/// Return ordered setup stage names for a workspace scope or a single repo.
pub fn setup_stage_names(config: &AppConfig, repo_name: Option<&str>) -> Vec<String> {
    let val = match repo_name {
        Some(r) => config
            .repositories
            .get(r)
            .and_then(|rc| rc.lifecycle.as_ref()),
        None => config.lifecycle.as_ref(),
    };
    let scope = LifecycleScope::from_value(val);
    let mut names: Vec<String> = Vec::new();
    for key in &scope.setup_order {
        if let Some(name) = key.strip_prefix("setup:") {
            let name = name.trim();
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    if names.is_empty() {
        let mut seen: HashSet<String> = HashSet::new();
        for key in scope.stages.keys() {
            if let Some(name) = key.strip_prefix("setup:") {
                let name = name.trim();
                if !name.is_empty() && seen.insert(name.to_string()) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

/// Collect all known lifecycle stage keys for the given repository selection.
pub fn known_stages_for_repos(config: &AppConfig, repos: &[String]) -> Vec<String> {
    let mut set = HashSet::new();
    set.insert("configure".to_string());
    for stage in setup_stage_names(config, None) {
        set.insert(format!("setup:{}", stage));
    }
    for repo in repos {
        for stage in setup_stage_names(config, Some(repo)) {
            set.insert(format!("setup:{}", stage));
        }
    }
    for stage in EVENT_STAGES {
        set.insert(stage.to_string());
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

/// Collect all known lifecycle stage names for status/reporting.
pub fn known_stages(config: &AppConfig) -> Vec<String> {
    let repos: Vec<String> = config.repositories.keys().cloned().collect();
    known_stages_for_repos(config, &repos)
}

/// Compute pending setup stages that block start/launch.
/// Stages whose hooks are all `always` do not block.
pub fn pending_setup_stages(
    config: &AppConfig,
    ws_dir: &Path,
    workspace_repos: &[String],
) -> Vec<(String, Vec<String>)> {
    let state = LifecycleState::load(ws_dir);
    let mut pending: Vec<(String, Vec<String>)> = Vec::new();

    for repo in workspace_repos {
        let mut repo_pending: Vec<String> = Vec::new();
        for stage in setup_stage_names(config, Some(repo)) {
            let stage_key = format!("setup:{}", stage);
            let hooks = stage_hooks_for_repo(config, Some(repo), &stage_key);
            if hooks.is_empty() {
                continue;
            }
            if hooks.iter().all(|h| h.when == HookWhen::Always) {
                continue;
            }
            let fp = Some(compute_stage_fingerprint(&stage_key, Some(repo), config));
            let done = hooks.iter().all(|h| {
                h.when == HookWhen::Always
                    || state.is_done(
                        &LifecycleState::key(Some(repo), &stage_key),
                        fp.as_deref(),
                        &h.when,
                    )
            });
            if !done {
                repo_pending.push(stage);
            }
        }
        // Also consider workspace-scope setup stages as applying? They may not be repo-specific; include if any workspace hooks exist and pending.
        if !repo_pending.is_empty() {
            pending.push((repo.clone(), repo_pending));
        }
    }

    // Workspace-level setup stages (block if not done and not all-always)
    let mut ws_pending: Vec<String> = Vec::new();
    for stage in setup_stage_names(config, None) {
        let stage_key = format!("setup:{}", stage);
        let hooks = workspace_stage_hooks(config, &stage_key);
        if hooks.is_empty() {
            continue;
        }
        if hooks.iter().all(|h| h.when == HookWhen::Always) {
            continue;
        }
        let fp = Some(compute_stage_fingerprint(&stage_key, None, config));
        let done = hooks.iter().all(|h| {
            h.when == HookWhen::Always
                || state.is_done(
                    &LifecycleState::key(None, &stage_key),
                    fp.as_deref(),
                    &h.when,
                )
        });
        if !done {
            ws_pending.push(stage);
        }
    }
    if !ws_pending.is_empty() {
        pending.insert(0, ("workspace".to_string(), ws_pending));
    }

    pending
}

pub fn format_pending_setup_error(
    workspace_name: &str,
    pending: &[(String, Vec<String>)],
) -> String {
    let mut msg = format!(
        "Workspace '@{}' setup is incomplete.\n\nPending setup stages:\n",
        workspace_name
    );
    for (scope, stages) in pending {
        if scope == "workspace" {
            msg.push_str(&format!("  workspace: {}\n", stages.join(", ")));
        } else {
            msg.push_str(&format!("  %{}: {}\n", scope, stages.join(", ")));
        }
    }
    msg.push_str("\nRun the lifecycle first:\n\n");
    msg.push_str(&format!("  ws lifecycle @{} run\n\n", workspace_name));
    msg.push_str("Or run a specific stage:\n\n");
    // show a couple of concrete examples if available
    let mut examples: Vec<String> = Vec::new();
    for (scope, stages) in pending {
        for st in stages {
            if scope == "workspace" {
                examples.push(format!("  ws lifecycle @{} run +{}", workspace_name, st));
            } else {
                examples.push(format!(
                    "  ws lifecycle @{} %{} run +{}",
                    workspace_name, scope, st
                ));
            }
            if examples.len() >= 3 {
                break;
            }
        }
        if examples.len() >= 3 {
            break;
        }
    }
    if examples.is_empty() {
        msg.push_str(&format!("  ws lifecycle @{} run\n", workspace_name));
    } else {
        for ex in examples {
            msg.push_str(&ex);
            msg.push('\n');
        }
    }
    msg
}
