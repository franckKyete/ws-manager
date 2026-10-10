use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::ConfigLoader;
use crate::registry::list_registered_projects;
use crate::workspace::WorkspaceManager;

pub struct ConfigFileWatcher {
    registry_path: Option<PathBuf>,
    file_hashes: HashMap<String, String>,
    initialized: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WatcherCheckResult {
    pub pushed: Vec<String>,
    pub saved: Vec<String>,
}

impl ConfigFileWatcher {
    pub fn new(registry_path: Option<PathBuf>) -> Self {
        Self {
            registry_path,
            file_hashes: HashMap::new(),
            initialized: false,
        }
    }

    fn hash_file(file_path: &Path) -> Option<String> {
        if !file_path.exists() || !file_path.is_file() {
            return None;
        }
        let content = std::fs::read(file_path).ok()?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        Some(hex::encode(hasher.finalize()))
    }

    fn is_valid_yaml(file_path: &Path) -> bool {
        if let Ok(content) = std::fs::read_to_string(file_path) {
            serde_yaml::from_str::<serde_yaml::Value>(&content).is_ok()
        } else {
            false
        }
    }

    pub fn initialize(&mut self) {
        let projects = list_registered_projects(self.registry_path.as_deref(), true);
        for p in projects {
            let mut cfg_file = p.join("repositories.yml");
            if !cfg_file.exists() {
                cfg_file = p.join("repository.yml");
            }
            if cfg_file.exists() {
                if let Some(h) = Self::hash_file(&cfg_file) {
                    if let Ok(canon) = cfg_file.canonicalize() {
                        self.file_hashes
                            .insert(canon.to_string_lossy().to_string(), h);
                    }
                }
            }

            let ws_root = p.join("workspaces");
            if ws_root.exists() && ws_root.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&ws_root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let mut ws_file = path.join("workspace.yml");
                            if !ws_file.exists() {
                                ws_file = path.join("workspace.yaml");
                            }
                            if ws_file.exists() {
                                if let Some(h) = Self::hash_file(&ws_file) {
                                    if let Ok(canon) = ws_file.canonicalize() {
                                        self.file_hashes
                                            .insert(canon.to_string_lossy().to_string(), h);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.initialized = true;
    }

    pub fn check_changes(&mut self) -> WatcherCheckResult {
        if !self.initialized {
            self.initialize();
            return WatcherCheckResult::default();
        }

        let projects = list_registered_projects(self.registry_path.as_deref(), true);
        let mut pushed_projects = Vec::new();
        let mut saved_workspaces = Vec::new();
        let mut seen_paths = std::collections::HashSet::new();

        for p in projects {
            let proj_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut cfg_file = p.join("repositories.yml");
            if !cfg_file.exists() {
                cfg_file = p.join("repository.yml");
            }

            if cfg_file.exists() {
                if let Ok(canon) = cfg_file.canonicalize() {
                    let cfg_str = canon.to_string_lossy().to_string();
                    seen_paths.insert(cfg_str.clone());

                    if let Some(current_hash) = Self::hash_file(&canon) {
                        let last_hash = self.file_hashes.get(&cfg_str).cloned();
                        match last_hash {
                            None => {
                                self.file_hashes.insert(cfg_str, current_hash);
                            }
                            Some(prev) if prev != current_hash && Self::is_valid_yaml(&canon) => {
                                self.file_hashes.insert(cfg_str.clone(), current_hash);
                                if let Ok(config) =
                                    ConfigLoader::load_config(Some(&canon), None, true)
                                {
                                    let mut manager = WorkspaceManager::new(config, None);
                                    match manager.hub_push(
                                        &format!(
                                            "Auto-pushed blueprint from {} edit",
                                            canon.file_name().unwrap_or_default().to_string_lossy()
                                        ),
                                        None,
                                        true,
                                    ) {
                                        Ok(rev_res) => {
                                            let version = rev_res
                                                .get("revision")
                                                .and_then(|r| r.get("version"))
                                                .and_then(|v| v.as_i64())
                                                .map(|v| v.to_string())
                                                .unwrap_or_else(|| "?".to_string());
                                            pushed_projects.push(proj_name.clone());

                                            if let Some(refreshed) = Self::hash_file(&canon) {
                                                self.file_hashes.insert(cfg_str, refreshed);
                                            }

                                            let should_notify = manager
                                                .config
                                                .hub_auto_save
                                                .as_ref()
                                                .map(|a| a.notify)
                                                .unwrap_or(true);
                                            if should_notify {
                                                let (ns, p_n) =
                                                    manager.get_project_namespace_and_name(None);
                                                crate::notify::notify_blueprint_push_success(
                                                    &format!("{}/{}", ns, p_n),
                                                    Some(&version),
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            let should_notify = manager
                                                .config
                                                .hub_auto_save
                                                .as_ref()
                                                .map(|a| a.notify)
                                                .unwrap_or(true);
                                            if should_notify {
                                                crate::notify::notify_blueprint_push_failure(
                                                    &proj_name,
                                                    &e.to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Check workspace.yml files
            let ws_root = p.join("workspaces");
            if ws_root.exists() && ws_root.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&ws_root) {
                    for entry in entries.flatten() {
                        let ws_dir = entry.path();
                        if !ws_dir.is_dir() {
                            continue;
                        }

                        let mut ws_file = ws_dir.join("workspace.yml");
                        if !ws_file.exists() {
                            ws_file = ws_dir.join("workspace.yaml");
                        }
                        if !ws_file.exists() {
                            continue;
                        }

                        if let Ok(canon) = ws_file.canonicalize() {
                            let ws_str = canon.to_string_lossy().to_string();
                            seen_paths.insert(ws_str.clone());

                            if let Some(current_ws_hash) = Self::hash_file(&canon) {
                                let last_ws_hash = self.file_hashes.get(&ws_str).cloned();
                                match last_ws_hash {
                                    None => {
                                        self.file_hashes.insert(ws_str, current_ws_hash);
                                    }
                                    Some(prev)
                                        if prev != current_ws_hash
                                            && Self::is_valid_yaml(&canon) =>
                                    {
                                        self.file_hashes.insert(ws_str, current_ws_hash);
                                        let raw_name = ws_dir
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_default();
                                        let ws_name = raw_name.trim_start_matches('@').to_string();

                                        if let Ok(config) =
                                            ConfigLoader::load_config(Some(&cfg_file), None, true)
                                        {
                                            let manager = WorkspaceManager::new(config, None);
                                            if manager.has_workspace(&ws_name) {
                                                match manager.hub_state_save(
                                                    &ws_name, None, true, true, true,
                                                ) {
                                                    Ok(_) => {
                                                        if let Ok(fp) = manager
                                                            .get_workspace_fingerprint(
                                                                &ws_name, true,
                                                            )
                                                        {
                                                            let mut cache =
                                                                manager.load_auto_save_cache();
                                                            let mut entry_map =
                                                                serde_json::Map::new();
                                                            entry_map.insert(
                                                                "fingerprint".to_string(),
                                                                serde_json::Value::String(fp),
                                                            );
                                                            entry_map.insert(
                                                                "last_saved_at".to_string(),
                                                                serde_json::Value::String(
                                                                    crate::utils::get_iso_timestamp(
                                                                    ),
                                                                ),
                                                            );
                                                            cache.insert(
                                                                ws_name.clone(),
                                                                serde_json::Value::Object(
                                                                    entry_map,
                                                                ),
                                                            );
                                                            let _ = manager
                                                                .save_auto_save_cache(&cache);
                                                        }
                                                        saved_workspaces.push(format!(
                                                            "{}@{}",
                                                            proj_name, ws_name
                                                        ));

                                                        let should_notify = manager
                                                            .config
                                                            .hub_auto_save
                                                            .as_ref()
                                                            .map(|a| a.notify)
                                                            .unwrap_or(true);
                                                        if should_notify {
                                                            let (ns, p_n) = manager
                                                                .get_project_namespace_and_name(
                                                                    None,
                                                                );
                                                            crate::notify::notify_auto_save_success(
                                                                &ws_name,
                                                                Some(&format!("{}/{}", ns, p_n)),
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        let should_notify = manager
                                                            .config
                                                            .hub_auto_save
                                                            .as_ref()
                                                            .map(|a| a.notify)
                                                            .unwrap_or(true);
                                                        if should_notify {
                                                            crate::notify::notify_auto_save_failure(
                                                                &ws_name,
                                                                &e.to_string(),
                                                                Some(&proj_name),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // Prune deleted files
        self.file_hashes
            .retain(|k, _| seen_paths.contains(k) || Path::new(k).exists());

        WatcherCheckResult {
            pushed: pushed_projects,
            saved: saved_workspaces,
        }
    }
}
