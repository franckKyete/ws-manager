use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use yaml_edit::{Document, Mapping, Sequence};

use crate::errors::{Result, WSError};
use crate::lifecycle::{
    LifecycleState, DEFAULT_READY_STAGE, DEFAULT_SETUP_STAGE, LIFECYCLE_SCHEMA_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationSourceKind {
    Auto,
    Legacy,
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetectedSource {
    #[default]
    None,
    Legacy,
    V1,
    Mixed,
    V3,
}

#[derive(Debug, Clone)]
pub struct MigrationOptions {
    pub dry_run: bool,
    pub backup: bool,
    pub stage_name: String,
    pub source: MigrationSourceKind,
    pub no_state: bool,
    pub reset_state: bool,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            backup: true,
            stage_name: DEFAULT_SETUP_STAGE.to_string(),
            source: MigrationSourceKind::Auto,
            no_state: false,
            reset_state: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MigrationReport {
    pub source: DetectedSource,
    pub changed: bool,
    pub backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ConfigFilenameMigrationReport {
    pub from: PathBuf,
    pub to: PathBuf,
    pub changed: bool,
}

pub fn find_migratable_config_file(project_root: &Path) -> Option<PathBuf> {
    for name in [
        "ws-project.yml",
        "ws-project.yaml",
        "repositories.yml",
        "repositories.yaml",
    ] {
        let candidate = project_root.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn migrate_project_config_filename(
    project_root: &Path,
    dry_run: bool,
) -> Result<ConfigFilenameMigrationReport> {
    let new_path = project_root.join("ws-project.yml");
    if new_path.exists() {
        return Err(WSError::Config(format!(
            "Target configuration '{}' already exists",
            new_path.display()
        )));
    }

    let old_path = ["repositories.yml", "repositories.yaml"]
        .iter()
        .map(|n| project_root.join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            WSError::Config(format!(
                "No legacy repositories.yml found in {}",
                project_root.display()
            ))
        })?;

    if !dry_run {
        fs::rename(&old_path, &new_path)?;
    }

    Ok(ConfigFilenameMigrationReport {
        from: old_path,
        to: new_path,
        changed: !dry_run,
    })
}

pub fn detect_project_source(config_path: &Path) -> Result<DetectedSource> {
    let content = fs::read_to_string(config_path)?;
    let doc = Document::from_str(&content).map_err(|e| WSError::Config(e.to_string()))?;
    let root = doc.as_mapping().ok_or_else(|| {
        WSError::Config("Top-level project configuration must be a YAML mapping".to_string())
    })?;
    Ok(detect_source(&root))
}

pub fn migrate_project_config(
    config_path: &Path,
    workspaces_dir: Option<&Path>,
    opts: &MigrationOptions,
) -> Result<MigrationReport> {
    let content = fs::read_to_string(config_path)?;
    let doc = Document::from_str(&content).map_err(|e| WSError::Config(e.to_string()))?;
    let root = doc.as_mapping().ok_or_else(|| {
        WSError::Config("Top-level project configuration must be a YAML mapping".to_string())
    })?;

    let detected = detect_source(&root);
    let effective = match opts.source {
        MigrationSourceKind::Auto => detected,
        MigrationSourceKind::Legacy => DetectedSource::Legacy,
        MigrationSourceKind::V1 => DetectedSource::V1,
    };

    if matches!(detected, DetectedSource::V3) && !opts.reset_state {
        return Ok(MigrationReport {
            source: detected,
            changed: false,
            backup_path: None,
        });
    }

    if let Err(msg) = validate_source_selection(detected, effective, opts.source) {
        return Err(WSError::Config(msg));
    }

    let mut changed = false;
    ensure_schema_version(&root, &mut changed);
    migrate_scope_mapping(&root, opts, &mut changed);
    if let Some(repos) = root.get_mapping("repositories") {
        for (_, repo_val) in &repos {
            if let Some(repo_map) = repo_val.as_mapping() {
                migrate_scope_mapping(repo_map, opts, &mut changed);
            }
        }
    }

    let mut backup_path = None;
    if changed && !opts.dry_run {
        if opts.backup {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let bak = config_path.with_file_name(format!(
                "{}.bak.{}",
                config_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repositories.yml"),
                ts
            ));
            fs::copy(config_path, &bak)?;
            backup_path = Some(bak);
        }
        fs::write(config_path, doc.to_string().as_bytes())?;
    }

    if !opts.no_state {
        if let Some(ws_dir) = workspaces_dir {
            migrate_workspace_states(ws_dir, opts)?;
        }
    }

    Ok(MigrationReport {
        source: detected,
        changed,
        backup_path,
    })
}

fn validate_source_selection(
    detected: DetectedSource,
    effective: DetectedSource,
    requested: MigrationSourceKind,
) -> std::result::Result<(), String> {
    if requested == MigrationSourceKind::Auto {
        return Ok(());
    }
    match (requested, detected) {
        (MigrationSourceKind::Legacy, DetectedSource::Legacy)
        | (MigrationSourceKind::Legacy, DetectedSource::Mixed)
        | (MigrationSourceKind::V1, DetectedSource::V1)
        | (MigrationSourceKind::V1, DetectedSource::Mixed) => Ok(()),
        _ if effective == detected => Ok(()),
        _ => Err(format!(
            "Requested migration source {:?} does not match detected source {:?}",
            requested, detected
        )),
    }
}

fn migrate_workspace_states(workspaces_dir: &Path, opts: &MigrationOptions) -> Result<()> {
    if !workspaces_dir.is_dir() {
        return Ok(());
    }
    let Ok(entries) = fs::read_dir(workspaces_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if opts.reset_state {
            let p = LifecycleState::path(&path);
            if p.exists() && !opts.dry_run {
                let _ = fs::remove_file(p);
            }
        }
        let mut state = LifecycleState::load(&path);
        state.version = LIFECYCLE_SCHEMA_VERSION;
        // Best-effort state key migration from legacy/v1 to v3 naming.
        migrate_state_keys(&mut state);
        if !opts.dry_run {
            let _ = state.save(&path);
        }
    }
    Ok(())
}

fn migrate_state_keys(state: &mut LifecycleState) {
    let keys: Vec<String> = state.records.keys().cloned().collect();
    for key in keys {
        let Some(rec) = state.records.remove(&key) else {
            continue;
        };
        let new_key = if let Some(rest) = key.strip_prefix("workspace:") {
            match rest {
                "bootstrap" => format!("workspace:setup:{}", DEFAULT_SETUP_STAGE),
                "ready" => format!("workspace:setup:{}", DEFAULT_READY_STAGE),
                "env" => "workspace:configure".to_string(),
                other => format!("workspace:{}", other),
            }
        } else if let Some(rest) = key.strip_prefix("repo:") {
            // repo:<repo>:<stage>
            let mut parts = rest.splitn(2, ':');
            let repo = parts.next().unwrap_or("");
            let stage = parts.next().unwrap_or("");
            let mapped = match stage {
                "bootstrap" => format!("setup:{}", DEFAULT_SETUP_STAGE),
                "ready" => format!("setup:{}", DEFAULT_READY_STAGE),
                "env" => "configure".to_string(),
                other => other.to_string(),
            };
            format!("repo:{}:{}", repo, mapped)
        } else {
            key.clone()
        };
        state.records.insert(new_key, rec);
    }
}

fn detect_source(root: &Mapping) -> DetectedSource {
    let mut legacy = false;
    let mut v1 = false;
    let mut v3 = false;

    if root.get("setup").is_some() {
        legacy = true;
    }
    if let Some(lc) = root.get_mapping("lifecycle") {
        if lc.get("bootstrap").is_some() || lc.get("env").is_some() || lc.get("ready").is_some() {
            v1 = true;
        }
        if lc.get("configure").is_some()
            || lc.get_sequence("setup").is_some()
            || lc.get_mapping("setup").is_some()
        {
            v3 = true;
        }
    }
    if let Some(repos) = root.get_mapping("repositories") {
        for (_, repo_val) in &repos {
            if let Some(repo_map) = repo_val.as_mapping() {
                if repo_map.get("setup").is_some() {
                    legacy = true;
                }
                if let Some(lc) = repo_map.get_mapping("lifecycle") {
                    if lc.get("bootstrap").is_some()
                        || lc.get("env").is_some()
                        || lc.get("ready").is_some()
                    {
                        v1 = true;
                    }
                    if lc.get("configure").is_some()
                        || lc.get_sequence("setup").is_some()
                        || lc.get_mapping("setup").is_some()
                    {
                        v3 = true;
                    }
                }
            }
        }
    }

    let schema_v3 = root
        .get("schema_version")
        .and_then(|n| n.as_scalar().map(|s| s.to_string() == "3"))
        .unwrap_or(false);

    if schema_v3 && !legacy && !v1 {
        return DetectedSource::V3;
    }
    if v3 && !legacy && !v1 {
        return DetectedSource::V3;
    }
    match (legacy, v1) {
        (true, true) => DetectedSource::Mixed,
        (true, false) => DetectedSource::Legacy,
        (false, true) => DetectedSource::V1,
        (false, false) => DetectedSource::None,
    }
}

fn ensure_schema_version(root: &Mapping, changed: &mut bool) {
    let is_v3 = root
        .get("schema_version")
        .and_then(|n| n.as_scalar().map(|s| s.to_string() == "3"))
        .unwrap_or(false);
    if !is_v3 {
        root.set("schema_version", 3);
        *changed = true;
    }
}

fn migrate_scope_mapping(owner: &Mapping, opts: &MigrationOptions, changed: &mut bool) {
    let has_legacy_setup = owner.get("setup").is_some();
    let lc_map = owner.get_mapping("lifecycle");
    let has_v1 = lc_map
        .as_ref()
        .map(|lc| {
            lc.get("bootstrap").is_some() || lc.get("env").is_some() || lc.get("ready").is_some()
        })
        .unwrap_or(false);
    let has_v3 = lc_map
        .as_ref()
        .map(|lc| {
            lc.get("configure").is_some()
                || lc.get_sequence("setup").is_some()
                || lc.get_mapping("setup").is_some()
        })
        .unwrap_or(false);

    if !has_legacy_setup && !has_v1 && !has_v3 {
        return;
    }

    if owner.get_mapping("lifecycle").is_none() {
        owner.set("lifecycle", Mapping::new());
        *changed = true;
    }
    let lifecycle = owner.get_mapping("lifecycle").unwrap();

    if let Some(legacy_setup) = owner.get("setup") {
        if lifecycle.get_sequence("setup").is_none() && lifecycle.get_mapping("setup").is_none() {
            let seq = Sequence::new();
            let stage = Mapping::new();
            stage.set("name", opts.stage_name.as_str());
            stage.set("hooks", hooks_sequence_from_node(legacy_setup));
            seq.push(stage);
            lifecycle.set("setup", seq);
            *changed = true;
        }
        owner.remove("setup");
        *changed = true;
    }

    if let Some(bootstrap) = lifecycle.get("bootstrap") {
        ensure_setup_stage(&lifecycle, &opts.stage_name, bootstrap, changed);
        lifecycle.remove("bootstrap");
        *changed = true;
    }
    if let Some(env_hooks) = lifecycle.get("env") {
        if lifecycle.get("configure").is_none() {
            lifecycle.set("configure", hooks_sequence_from_node(env_hooks));
        } else {
            let merged = hooks_sequence_from_node(env_hooks);
            if let Some(existing) = lifecycle.get("configure") {
                let existing_seq = hooks_sequence_from_node(existing);
                for v in existing_seq.values() {
                    merged.push(v);
                }
            }
            lifecycle.set("configure", merged);
        }
        lifecycle.remove("env");
        *changed = true;
    }
    if let Some(ready) = lifecycle.get("ready") {
        ensure_setup_stage(&lifecycle, DEFAULT_READY_STAGE, ready, changed);
        lifecycle.remove("ready");
        *changed = true;
    }

    // Clean empty lifecycle mapping? keep it if it has content; harmless either way.
}

fn ensure_setup_stage(
    lifecycle: &Mapping,
    stage_name: &str,
    hooks_val: yaml_edit::YamlNode,
    changed: &mut bool,
) {
    let hooks_seq = hooks_sequence_from_node(hooks_val);
    if let Some(seq) = lifecycle.get_sequence("setup") {
        for item in seq.values() {
            if let Some(stage_map) = item.as_mapping() {
                let name = stage_map
                    .get("name")
                    .and_then(|n| n.as_scalar().map(|s| s.to_string()));
                if name.as_deref() == Some(stage_name) {
                    if let Some(existing_hooks) = stage_map.get("hooks") {
                        let merged = hooks_sequence_from_node(existing_hooks);
                        for h in hooks_seq.values() {
                            merged.push(h);
                        }
                        stage_map.set("hooks", merged);
                    } else {
                        stage_map.set("hooks", hooks_seq);
                    }
                    *changed = true;
                    return;
                }
            }
        }
        let stage = Mapping::new();
        stage.set("name", stage_name);
        stage.set("hooks", hooks_seq);
        seq.push(stage);
        *changed = true;
        return;
    }

    if let Some(map) = lifecycle.get_mapping("setup") {
        if let Some(existing) = map.get(stage_name) {
            let merged = hooks_sequence_from_node(existing);
            for h in hooks_seq.values() {
                merged.push(h);
            }
            map.set(stage_name, merged);
        } else {
            map.set(stage_name, hooks_seq);
        }
        *changed = true;
        return;
    }

    let seq = Sequence::new();
    let stage = Mapping::new();
    stage.set("name", stage_name);
    stage.set("hooks", hooks_seq);
    seq.push(stage);
    lifecycle.set("setup", seq);
    *changed = true;
}

fn hooks_sequence_from_node(val: yaml_edit::YamlNode) -> Sequence {
    if let Some(seq) = val.as_sequence() {
        let out = Sequence::new();
        for item in seq.values() {
            if let Some(scalar) = item.as_scalar() {
                let hook = Mapping::new();
                hook.set("run", scalar.to_string().as_str());
                hook.set("when", "once");
                out.push(hook);
            } else {
                out.push(item);
            }
        }
        return out;
    }
    let out = Sequence::new();
    if let Some(scalar) = val.as_scalar() {
        let hook = Mapping::new();
        hook.set("run", scalar.to_string().as_str());
        hook.set("when", "once");
        out.push(hook);
        return out;
    }
    if val.as_mapping().is_some() || val.as_tagged().is_some() {
        out.push(val);
        return out;
    }
    out
}
