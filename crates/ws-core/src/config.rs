use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use crate::errors::{Result, WSError};
use crate::lifecycle::LIFECYCLE_SCHEMA_VERSION;
use crate::models::{
    clean_env_val, is_private_val, is_secret_val, AppConfig, HubAutoSaveConfig, RepoConfig,
    TmuxConfig,
};

pub const DEFAULT_CONFIG_FILENAMES: &[&str] = &[
    "repositories.yml",
    "repositories.yaml",
    "ws.yml",
    "ws.yaml",
    ".ws.yml",
];

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn find_config_file(explicit_path: Option<&Path>) -> Result<Option<PathBuf>> {
        if let Some(p) = explicit_path {
            let abs_path = if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()?.join(p)
            };
            if abs_path.is_file() {
                return Ok(Some(abs_path));
            }
            return Err(WSError::Config(format!(
                "Specified config file not found: {}",
                p.display()
            )));
        }

        let mut curr = std::env::current_dir()?;
        loop {
            for filename in DEFAULT_CONFIG_FILENAMES {
                let candidate = curr.join(filename);
                if candidate.is_file() {
                    return Ok(Some(candidate));
                }
            }
            if !curr.pop() {
                break;
            }
        }

        if let Some(home) = dirs::home_dir() {
            let user_config_dir = home.join(".config").join("ws");
            for filename in &["repositories.yml", "config.yml", "ws.yml"] {
                let candidate = user_config_dir.join(filename);
                if candidate.is_file() {
                    return Ok(Some(candidate));
                }
            }
        }

        Ok(None)
    }

    pub fn load_config(
        config_path: Option<&Path>,
        workspaces_dir: Option<&Path>,
        allow_empty: bool,
    ) -> Result<AppConfig> {
        Self::load_config_with_options(config_path, workspaces_dir, allow_empty, true)
    }

    pub fn load_config_for_migration(
        config_path: Option<&Path>,
        workspaces_dir: Option<&Path>,
        allow_empty: bool,
    ) -> Result<AppConfig> {
        Self::load_config_with_options(config_path, workspaces_dir, allow_empty, false)
    }

    fn load_config_with_options(
        config_path: Option<&Path>,
        workspaces_dir: Option<&Path>,
        allow_empty: bool,
        enforce_lifecycle_schema: bool,
    ) -> Result<AppConfig> {
        let file_path = Self::find_config_file(config_path)?;
        let project_root = if let Some(ref fp) = file_path {
            fp.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
        } else {
            std::env::current_dir()?
        };

        if let Some(ref fp) = file_path {
            if fp.file_name().and_then(|n| n.to_str()) == Some("repositories.yml") {
                let _ = crate::registry::register_project(&project_root, None);
            }
        }

        let mut repos = HashMap::new();
        let mut global_env = HashMap::new();
        let mut global_secret_env = HashMap::new();
        let mut global_private_env = HashMap::new();
        let mut dynamic_env = HashMap::new();
        let mut global_setup = Vec::new();
        let mut global_secrets = Vec::new();
        let mut global_copy_files = Vec::new();
        let mut tmux_cfg = None;
        let mut hub_auto_save = None;
        let mut hub_project = None;
        let mut hub_val = serde_json::Value::Object(serde_json::Map::new());
        let mut lifecycle_val = None;
        let mut schema_version = None;
        let mut workspace_cfg = crate::models::WorkspaceRootConfig::default();

        if let Some(ref fp) = file_path {
            let content = fs::read_to_string(fp)?;
            let doc: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
                WSError::Config(format!("Failed to parse YAML '{}': {}", fp.display(), e))
            })?;

            if let Some(map) = doc.as_mapping() {
                // 1. Repositories
                if let Some(repos_val) =
                    map.get(serde_yaml::Value::String("repositories".to_string()))
                {
                    if let Some(repos_map) = repos_val.as_mapping() {
                        for (k, v) in repos_map {
                            if let Some(r_name) = k.as_str() {
                                let clean_r_name =
                                    r_name.trim_start_matches(['%', '+', ':', '#', '$']);
                                let cfg = RepoConfig::from_yaml_value(clean_r_name, v)
                                    .map_err(WSError::Config)?;
                                repos.insert(clean_r_name.to_string(), cfg);
                            }
                        }
                    }
                }

                // 2. Global env
                if let Some(env_val) = map.get(serde_yaml::Value::String("env".to_string())) {
                    if let Some(env_map) = env_val.as_mapping() {
                        for (k, v) in env_map {
                            if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                                if is_secret_val(v_str) {
                                    global_secret_env
                                        .insert(k_str.to_string(), clean_env_val(v_str));
                                } else if is_private_val(v_str) {
                                    global_private_env
                                        .insert(k_str.to_string(), clean_env_val(v_str));
                                } else {
                                    global_env.insert(k_str.to_string(), clean_env_val(v_str));
                                }
                            }
                        }
                    }
                }

                // 3. Global secret
                if let Some(sec_val) = map
                    .get(serde_yaml::Value::String("secret".to_string()))
                    .or_else(|| map.get(serde_yaml::Value::String("secrets".to_string())))
                {
                    if let Some(sec_map) = sec_val.as_mapping() {
                        for (k, v) in sec_map {
                            if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                                global_secret_env.insert(k_str.to_string(), clean_env_val(v_str));
                            }
                        }
                    } else if let Some(seq) = sec_val.as_sequence() {
                        for item in seq {
                            if let Some(s) = item.as_str() {
                                global_secrets.push(s.to_string());
                            }
                        }
                    } else if let Some(s) = sec_val.as_str() {
                        global_secrets.push(s.to_string());
                    }
                }

                // 4. Global private
                if let Some(priv_val) = map
                    .get(serde_yaml::Value::String("private".to_string()))
                    .or_else(|| map.get(serde_yaml::Value::String("local_env".to_string())))
                {
                    if let Some(priv_map) = priv_val.as_mapping() {
                        for (k, v) in priv_map {
                            if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                                global_private_env.insert(k_str.to_string(), clean_env_val(v_str));
                            }
                        }
                    }
                }

                // 5. Dynamic env
                if let Some(dyn_val) = map.get(serde_yaml::Value::String("dynamic_env".to_string()))
                {
                    if let Some(dyn_map) = dyn_val.as_mapping() {
                        for (k, v) in dyn_map {
                            if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                                dynamic_env.insert(k_str.to_string(), v_str.to_string());
                            }
                        }
                    }
                }

                // 6. Setup scripts
                if let Some(setup_val) = map.get(serde_yaml::Value::String("setup".to_string())) {
                    if let Some(s) = setup_val.as_str() {
                        global_setup.push(s.to_string());
                    } else if let Some(seq) = setup_val.as_sequence() {
                        for item in seq {
                            if let Some(s) = item.as_str() {
                                global_setup.push(s.to_string());
                            } else if let Ok(json) = serde_json::to_value(item) {
                                global_setup.push(json.to_string());
                            }
                        }
                    }
                }

                // 7. Copy files
                if let Some(cf_val) = map
                    .get(serde_yaml::Value::String("copy_files".to_string()))
                    .or_else(|| map.get(serde_yaml::Value::String("files".to_string())))
                {
                    if let Ok(json) = serde_json::to_value(cf_val) {
                        if let Some(arr) = json.as_array() {
                            global_copy_files = arr.clone();
                        } else {
                            global_copy_files.push(json);
                        }
                    }
                }

                // 8. Tmux
                if let Some(tmux_val) = map
                    .get(serde_yaml::Value::String("tmux".to_string()))
                    .or_else(|| map.get(serde_yaml::Value::String("tmux_session".to_string())))
                {
                    let tc = TmuxConfig::from_value(tmux_val).map_err(WSError::Config)?;
                    if let Some(ref launch) = tc.launch_session {
                        if &tc.session == launch {
                            return Err(WSError::Config(format!(
                                "Tmux work session ('{}') and launch session ('{}') must have different names to prevent collisions.",
                                tc.session, launch
                            )));
                        }
                    }
                    tmux_cfg = Some(tc);
                }

                // 9. Hub block
                if let Some(h_val) = map.get(serde_yaml::Value::String("hub".to_string())) {
                    if let Ok(json) = serde_json::to_value(h_val) {
                        hub_val = json;
                    }
                    if let Some(h_map) = h_val.as_mapping() {
                        hub_project = h_map
                            .get(serde_yaml::Value::String("project".to_string()))
                            .or_else(|| h_map.get(serde_yaml::Value::String("name".to_string())))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        if let Some(as_val) =
                            h_map.get(serde_yaml::Value::String("auto_save".to_string()))
                        {
                            if let Ok(as_cfg) = HubAutoSaveConfig::from_value(as_val) {
                                hub_auto_save = Some(as_cfg);
                            }
                        }
                    }
                }

                // 10. Fallback root auto_save
                if hub_auto_save.is_none() {
                    if let Some(as_val) =
                        map.get(serde_yaml::Value::String("auto_save".to_string()))
                    {
                        if let Ok(as_cfg) = HubAutoSaveConfig::from_value(as_val) {
                            hub_auto_save = Some(as_cfg);
                        }
                    }
                }

                // 11. Lifecycle
                if let Some(lv) = map.get(serde_yaml::Value::String("lifecycle".to_string())) {
                    lifecycle_val = serde_json::to_value(lv).ok();
                }
                if let Some(sv) = map.get(serde_yaml::Value::String("schema_version".to_string())) {
                    schema_version = sv.as_u64().map(|n| n as u32);
                }

                // 12. Workspace (additive schema v3 extension)
                if let Some(ws_val) = map.get(serde_yaml::Value::String("workspace".to_string())) {
                    workspace_cfg = crate::models::WorkspaceRootConfig::from_yaml_value(ws_val)
                        .map_err(WSError::Config)?;
                } else if let Some(ws_val) =
                    map.get(serde_yaml::Value::String("workspaces".to_string()))
                {
                    // Compat: accept workspace-root settings nested under "workspaces"
                    // (e.g. workspaces.copy_files/env/save) when "workspace" is absent.
                    if let Some(ws_map) = ws_val.as_mapping() {
                        let has_workspace_keys = [
                            "copy_files",
                            "files",
                            "env",
                            "env_file",
                            "env_example",
                            "save",
                        ]
                        .iter()
                        .any(|k| ws_map.contains_key(serde_yaml::Value::String((*k).to_string())));
                        if has_workspace_keys {
                            workspace_cfg =
                                crate::models::WorkspaceRootConfig::from_yaml_value(ws_val)
                                    .map_err(WSError::Config)?;
                        }
                    }
                }
            }
        } else {
            // Auto-detect bare repos in bares/ or project_root
            let bares_dir = project_root.join("bares");
            let mut bare_dirs = Vec::new();
            if bares_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&bares_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() && p.extension().is_some_and(|e| e == "git") {
                            bare_dirs.push(p);
                        }
                    }
                }
            }
            if bare_dirs.is_empty() {
                if let Ok(entries) = fs::read_dir(&project_root) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() && p.extension().is_some_and(|e| e == "git") {
                            bare_dirs.push(p);
                        }
                    }
                }
            }

            for bare in bare_dirs {
                let stem = bare.file_name().unwrap().to_string_lossy().to_string();
                let repo_name = if let Some(stripped) = stem.strip_suffix(".git") {
                    stripped.to_string()
                } else {
                    stem.clone()
                };
                let key = repo_name.to_lowercase();
                repos.insert(key.clone(), RepoConfig::new(key, bare, repo_name));
            }
        }

        if !repos.is_empty() && enforce_lifecycle_schema {
            if schema_version != Some(LIFECYCLE_SCHEMA_VERSION) {
                return Err(WSError::Config(format!(
                    "Project configuration '{}' uses unsupported lifecycle schema (found: {}, expected: {}).\\nRun 'ws project migrate lifecycle --yes' before using this project.",
                    file_path
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| project_root.join("repositories.yml").display().to_string()),
                    schema_version.unwrap_or(0),
                    LIFECYCLE_SCHEMA_VERSION
                )));
            }
            if let Some(lv) = lifecycle_val.as_ref().and_then(|v| v.as_object()) {
                if lv.contains_key("bootstrap")
                    || lv.contains_key("env")
                    || lv.contains_key("ready")
                {
                    return Err(WSError::Config(
                        "Legacy lifecycle stages (bootstrap/env/ready) detected. Run 'ws project migrate lifecycle --yes' before using this project.".to_string(),
                    ));
                }
            }
        }

        if repos.is_empty() && !allow_empty {
            return Err(WSError::Config(
                "No repositories configured. Run 'ws init <git-url...>' to clone repositories or create 'repositories.yml'.".to_string(),
            ));
        }

        let resolved_ws_dir = if let Some(ws_d) = workspaces_dir {
            if ws_d.is_absolute() {
                ws_d.to_path_buf()
            } else {
                project_root.join(ws_d)
            }
        } else {
            project_root.join("workspaces")
        };

        Ok(AppConfig {
            project_root: project_root.clone(),
            repositories: repos,
            workspaces_dir: resolved_ws_dir,
            config_file_path: file_path,
            global_env,
            secret_env: global_secret_env,
            private_env: global_private_env,
            dynamic_env,
            setup: global_setup,
            secrets: global_secrets,
            copy_files: global_copy_files,
            tmux: tmux_cfg,
            hub_auto_save,
            hub_project,
            hub: hub_val,
            lifecycle: lifecycle_val,
            schema_version,
            workspace: workspace_cfg,
        })
    }

    pub fn classify_project_assets(
        app_config: &AppConfig,
    ) -> (
        String,
        HashMap<String, HashMap<String, String>>,
        Vec<PathBuf>,
        usize,
    ) {
        let mut extracted_secrets: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut private_vars_count = 0;

        if !app_config.secret_env.is_empty() {
            extracted_secrets.insert("global".to_string(), app_config.secret_env.clone());
        }
        private_vars_count += app_config.private_env.len();

        let mut sanitized_repos = HashMap::new();
        for (r_name, r_cfg) in &app_config.repositories {
            let mut r_dict = serde_json::to_value(r_cfg).unwrap_or(serde_json::Value::Null);
            if let Some(map) = r_dict.as_object_mut() {
                map.remove("private");
                map.remove("secret");
                map.remove("private_env");
                map.remove("secret_env");

                if !r_cfg.secret_env.is_empty() {
                    extracted_secrets.insert(r_name.clone(), r_cfg.secret_env.clone());
                    let mut env_map = map
                        .get("env")
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    for s_key in r_cfg.secret_env.keys() {
                        env_map.insert(
                            s_key.clone(),
                            serde_json::Value::String("secret".to_string()),
                        );
                    }
                    map.insert("env".to_string(), serde_json::Value::Object(env_map));
                }
            }
            private_vars_count += r_cfg.private_env.len();
            sanitized_repos.insert(r_name.clone(), r_dict);
        }

        let mut sanitized_data = serde_json::Map::new();
        if !app_config.global_env.is_empty() || !app_config.secret_env.is_empty() {
            let mut env_map = serde_json::Map::new();
            for (k, v) in &app_config.global_env {
                env_map.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
            for s_key in app_config.secret_env.keys() {
                env_map.insert(
                    s_key.clone(),
                    serde_json::Value::String("secret".to_string()),
                );
            }
            sanitized_data.insert("env".to_string(), serde_json::Value::Object(env_map));
        }

        if !app_config.dynamic_env.is_empty() {
            sanitized_data.insert(
                "dynamic_env".to_string(),
                serde_json::to_value(&app_config.dynamic_env).unwrap_or_default(),
            );
        }
        if !app_config.setup.is_empty() {
            sanitized_data.insert(
                "setup".to_string(),
                serde_json::to_value(&app_config.setup).unwrap_or_default(),
            );
        }
        if !app_config.copy_files.is_empty() {
            sanitized_data.insert(
                "copy_files".to_string(),
                serde_json::Value::Array(app_config.copy_files.clone()),
            );
        }
        if !app_config.workspace.is_empty() {
            if let Ok(val) = serde_json::to_value(&app_config.workspace) {
                if !val.is_null() {
                    sanitized_data.insert("workspace".to_string(), val);
                }
            }
        }
        if let Some(ref tc) = app_config.tmux {
            sanitized_data.insert(
                "tmux".to_string(),
                serde_json::to_value(tc).unwrap_or_default(),
            );
        }

        let mut hub_map = if let Some(obj) = app_config.hub.as_object() {
            obj.clone()
        } else {
            serde_json::Map::new()
        };
        if let Some(ref p) = app_config.hub_project {
            hub_map.insert("project".to_string(), serde_json::Value::String(p.clone()));
        }
        if let Some(ref as_cfg) = app_config.hub_auto_save {
            hub_map.insert("auto_save".to_string(), as_cfg.to_map(true));
        }
        if !hub_map.is_empty() {
            sanitized_data.insert("hub".to_string(), serde_json::Value::Object(hub_map));
        }

        sanitized_data.insert(
            "repositories".to_string(),
            serde_json::to_value(sanitized_repos).unwrap_or_default(),
        );

        let sanitized_yaml = serde_yaml::to_string(&sanitized_data).unwrap_or_default();

        let mut files_to_upload = Vec::new();
        let files_dir = app_config.project_root().join("files");
        if files_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&files_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        files_to_upload.push(p);
                    }
                }
            }
        }

        (
            sanitized_yaml,
            extracted_secrets,
            files_to_upload,
            private_vars_count,
        )
    }

    pub fn update_hub_config(
        config_path: &Path,
        hub_project: Option<&str>,
        hub_auto_save: Option<&HubAutoSaveConfig>,
        hub_extra: Option<&serde_yaml::Value>,
    ) -> Result<()> {
        let mut data: serde_yaml::Value = if config_path.is_file() {
            let s = fs::read_to_string(config_path)?;
            serde_yaml::from_str(&s)
                .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        if !data.is_mapping() {
            data = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }

        let map = data.as_mapping_mut().unwrap();
        let hub_key = serde_yaml::Value::String("hub".to_string());
        let mut hub_map = map
            .get(&hub_key)
            .and_then(|v| v.as_mapping())
            .cloned()
            .unwrap_or_default();

        if let Some(extra) = hub_extra {
            if let Some(ex_map) = extra.as_mapping() {
                for (k, v) in ex_map {
                    hub_map.insert(k.clone(), v.clone());
                }
            }
        }

        if let Some(hp) = hub_project {
            hub_map.insert(
                serde_yaml::Value::String("project".to_string()),
                serde_yaml::Value::String(hp.to_string()),
            );
        }

        if let Some(as_cfg) = hub_auto_save {
            let json_map = as_cfg.to_map(true);
            if let Ok(yaml_val) = serde_yaml::to_value(json_map) {
                hub_map.insert(serde_yaml::Value::String("auto_save".to_string()), yaml_val);
            }
        }

        map.insert(hub_key, serde_yaml::Value::Mapping(hub_map));

        let updated_str =
            serde_yaml::to_string(&data).map_err(|e| WSError::Config(e.to_string()))?;
        fs::write(config_path, updated_str)?;
        Ok(())
    }

    pub fn save_config(
        repositories: &HashMap<String, RepoConfig>,
        config_path: Option<&Path>,
        tmux: Option<&TmuxConfig>,
        hub_auto_save: Option<&HubAutoSaveConfig>,
        hub_project: Option<&str>,
    ) -> Result<PathBuf> {
        let path = config_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("repositories.yml"));

        let mut data: serde_yaml::Mapping = if path.is_file() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                Ok(serde_yaml::Value::Mapping(m)) => m,
                _ => serde_yaml::Mapping::new(),
            }
        } else {
            serde_yaml::Mapping::new()
        };

        // Always emit v3 schema for saved configs.
        data.insert(
            serde_yaml::Value::String("schema_version".to_string()),
            serde_yaml::Value::Number(3.into()),
        );

        let repos_yaml = serde_yaml::to_value(repositories)?;
        data.insert(
            serde_yaml::Value::String("repositories".to_string()),
            repos_yaml,
        );

        if let Some(t) = tmux {
            let t_yaml = serde_yaml::to_value(t)?;
            data.insert(serde_yaml::Value::String("tmux".to_string()), t_yaml);
        }

        if hub_project.is_some() || hub_auto_save.is_some() {
            let hub_key = serde_yaml::Value::String("hub".to_string());
            let mut hub_map = data
                .get(&hub_key)
                .and_then(|v| v.as_mapping())
                .cloned()
                .unwrap_or_default();
            if let Some(p) = hub_project {
                hub_map.insert(
                    serde_yaml::Value::String("project".to_string()),
                    serde_yaml::Value::String(p.to_string()),
                );
            }
            if let Some(a) = hub_auto_save {
                let json_map = a.to_map(true);
                let yaml_val = serde_yaml::to_value(json_map)?;
                hub_map.insert(serde_yaml::Value::String("auto_save".to_string()), yaml_val);
            }
            data.insert(hub_key, serde_yaml::Value::Mapping(hub_map));
        }

        // Remove legacy top-level setup if present in preserved content is not safe to guess,
        // but v3 configs must not rely on it; leave lifecycle/global keys as-is.

        let content = serde_yaml::to_string(&serde_yaml::Value::Mapping(data))?;
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&path, content)?;
        Ok(path)
    }
}

/// A single-document, format-preserving edit of `repositories.yml`.
///
/// Unlike [`ConfigLoader::save_config`], this keeps unknown per-repo keys and
/// comments intact. Edits are built in memory and written once by [`save`].
pub struct ProjectConfigEdit<'a> {
    path: &'a Path,
    doc: Document,
}

impl<'a> ProjectConfigEdit<'a> {
    pub fn open(path: &'a Path) -> Result<Self> {
        let content = if path.is_file() {
            fs::read_to_string(path)?
        } else {
            String::new()
        };
        let content = if content.trim().is_empty() {
            format!("schema_version: {}\n", LIFECYCLE_SCHEMA_VERSION)
        } else {
            content
        };
        let doc = Document::from_str(&content).map_err(|e| WSError::Config(e.to_string()))?;
        if doc.as_mapping().is_none() {
            return Err(WSError::Config(
                "Top-level repositories.yml must be a YAML mapping".to_string(),
            ));
        }
        Ok(Self { path, doc })
    }

    fn root(&self) -> Result<Mapping> {
        self.doc.as_mapping().ok_or_else(|| {
            WSError::Config("Top-level repositories.yml must be a YAML mapping".to_string())
        })
    }

    fn repositories(&self) -> Result<Mapping> {
        let root = self.root()?;
        if let Some(repos) = root.get_mapping("repositories") {
            // An empty `repositories: {}` is flow-style. Replace it with a
            // pending block mapping so newly added entries render in the
            // block style used by the rest of the file.
            if repos.iter().count() > 0 {
                return Ok(repos);
            }
            root.set("repositories", Mapping::new());
            return root.get_mapping("repositories").ok_or_else(|| {
                WSError::Config("Failed to create 'repositories' mapping".to_string())
            });
        }
        root.set("repositories", Mapping::new());
        root.get_mapping("repositories")
            .ok_or_else(|| WSError::Config("Failed to create 'repositories' mapping".to_string()))
    }

    pub fn ensure_schema_version(&self) -> Result<()> {
        self.root()?
            .set("schema_version", LIFECYCLE_SCHEMA_VERSION as i64);
        Ok(())
    }

    pub fn ensure_tmux(&self, tmux: &TmuxConfig) -> Result<()> {
        let root = self.root()?;
        if root.get("tmux").is_none() && root.get("tmux_session").is_none() {
            root.set("tmux", serialized_mapping(tmux)?);
        }
        Ok(())
    }

    pub fn ensure_hub(&self, auto_save: &HubAutoSaveConfig, project: Option<&str>) -> Result<()> {
        let root = self.root()?;
        if root.get_mapping("hub").is_none() {
            root.set("hub", Mapping::new());
        }
        let hub = root
            .get_mapping("hub")
            .ok_or_else(|| WSError::Config("Failed to create 'hub' mapping".to_string()))?;
        if hub.get("auto_save").is_none() {
            hub.set("auto_save", serialized_mapping(&auto_save.to_map(true))?);
        }
        if let Some(project) = project {
            if hub.get("project").is_none() {
                hub.set("project", project);
            }
        }
        Ok(())
    }

    /// Insert a new repository entry. Existing entries are left untouched
    /// apart from filling a missing `base_branch`.
    pub fn upsert_repository(&self, name: &str, repo: &RepoConfig) -> Result<()> {
        let repos = self.repositories()?;
        if repos.get(name).is_none() {
            repos.set(name, serialized_mapping(repo)?);
            return Ok(());
        }
        if let Some(base) = repo.base_branch.as_deref().map(str::trim) {
            if !base.is_empty() {
                let repo_map = repos.get_mapping(name).ok_or_else(|| {
                    WSError::Config(format!("Repository '{}' is not a YAML mapping", name))
                })?;
                let missing = match repo_map.get("base_branch") {
                    Some(node) => node
                        .as_scalar()
                        .map(|scalar| {
                            let text = scalar.to_string();
                            let text = text.trim();
                            text.is_empty() || text == "null" || text == "~"
                        })
                        .unwrap_or(true),
                    None => true,
                };
                if missing {
                    repo_map.set("base_branch", base);
                }
            }
        }
        Ok(())
    }

    pub fn set_repo_base_branch(&self, name: &str, branch: &str) -> Result<()> {
        let repos = self.repositories()?;
        let repo_map = repos.get_mapping(name).ok_or_else(|| {
            WSError::Config(format!(
                "Repository '{}' is not present in configuration",
                name
            ))
        })?;
        repo_map.set("base_branch", branch);
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(self.path, self.doc.to_string())?;
        Ok(())
    }
}

impl ConfigLoader {
    /// Persist one or more resolved base branches, preserving the rest of the
    /// configuration file byte-for-byte where possible.
    pub fn set_repo_base_branches(
        config_path: &Path,
        values: &HashMap<String, String>,
    ) -> Result<()> {
        if values.is_empty() {
            return Ok(());
        }
        let edit = ProjectConfigEdit::open(config_path)?;
        for (repo, branch) in values {
            edit.set_repo_base_branch(repo, branch)?;
        }
        edit.save()
    }

    /// Merge local `base_branch` values into a remote blueprint. Remote
    /// values win when present; local values fill gaps.
    pub fn merge_local_bases_into_yaml(local_yaml: &str, remote_yaml: &str) -> Result<String> {
        let local: serde_yaml::Value = serde_yaml::from_str(local_yaml).map_err(WSError::Yaml)?;
        let remote_value: serde_yaml::Value =
            serde_yaml::from_str(remote_yaml).map_err(WSError::Yaml)?;

        let Some(local_repos) = local.get("repositories").and_then(|v| v.as_mapping()) else {
            return Ok(remote_yaml.to_string());
        };
        let Some(remote_repos) = remote_value
            .get("repositories")
            .and_then(|v| v.as_mapping())
        else {
            return Ok(remote_yaml.to_string());
        };

        let doc = Document::from_str(remote_yaml).map_err(|e| WSError::Config(e.to_string()))?;
        let root = doc.as_mapping().ok_or_else(|| {
            WSError::Config("Remote blueprint must be a YAML mapping".to_string())
        })?;
        let remote_edit_repos = root.get_mapping("repositories").ok_or_else(|| {
            WSError::Config("Remote blueprint has no 'repositories' mapping".to_string())
        })?;

        for (key, value) in local_repos {
            let Some(name) = key.as_str() else {
                continue;
            };
            let Some(local_base) = value
                .get("base_branch")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
            else {
                continue;
            };
            let Some(remote_repo) = remote_repos.get(serde_yaml::Value::String(name.to_string()))
            else {
                continue;
            };
            let remote_has_base = remote_repo
                .get("base_branch")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .is_some_and(|v| !v.is_empty());
            if remote_has_base {
                continue;
            }
            if let Some(repo_map) = remote_edit_repos.get_mapping(name) {
                repo_map.set("base_branch", local_base);
            }
        }

        Ok(doc.to_string())
    }
}

fn serialized_mapping<T: serde::Serialize>(value: &T) -> Result<Mapping> {
    let yaml = serde_yaml::to_string(value).map_err(WSError::Yaml)?;
    let doc = Document::from_str(&yaml).map_err(|e| WSError::Config(e.to_string()))?;
    doc.as_mapping()
        .ok_or_else(|| WSError::Config("Serialized value is not a YAML mapping".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn targeted_base_branch_edit_preserves_comments_and_unknown_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("repositories.yml");
        fs::write(
            &path,
            "schema_version: 3\n# keep this comment\nrepositories:\n  api:\n    bare: bares/api.git\n    checkout: api\n    depends_on: db  # unknown key\n",
        )
        .unwrap();

        let mut values = HashMap::new();
        values.insert("api".to_string(), "main".to_string());
        ConfigLoader::set_repo_base_branches(&path, &values).unwrap();

        let output = fs::read_to_string(&path).unwrap();
        assert!(output.contains("# keep this comment"));
        assert!(output.contains("depends_on: db"));
        assert!(output.contains("base_branch: main"));
    }

    #[test]
    fn upsert_new_repository_writes_block_mapping_and_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("repositories.yml");
        let edit = ProjectConfigEdit::open(&path).unwrap();
        edit.ensure_schema_version().unwrap();
        let tmux = TmuxConfig {
            session: "demo".to_string(),
            launch_session: None,
            command: Some("nvim".to_string()),
            switch: true,
        };
        edit.ensure_tmux(&tmux).unwrap();
        let hub = HubAutoSaveConfig {
            enabled: true,
            interval: 300,
            include_wip: true,
            workspaces: crate::models::WorkspacesSelector::Mode("all".to_string()),
            notify: true,
        };
        edit.ensure_hub(&hub, None).unwrap();

        let mut repo = RepoConfig::new("api", "bares/api.git", "api");
        repo.url = Some("git@example.com:org/api.git".to_string());
        repo.base_branch = Some("main".to_string());
        edit.upsert_repository("api", &repo).unwrap();
        edit.save().unwrap();

        let output = fs::read_to_string(&path).unwrap();
        assert!(output.contains("schema_version: 3"));
        assert!(output.contains("repositories:\n  api:\n"));
        assert!(output.contains("base_branch: main"));
        assert!(output.contains("tmux:"));
        assert!(output.contains("auto_save:"));
    }

    #[test]
    fn merge_local_bases_preserves_gaps_but_remote_wins() {
        let local = "schema_version: 3\nrepositories:\n  api:\n    bare: bares/api.git\n    checkout: api\n    base_branch: develop\n  web:\n    bare: bares/web.git\n    checkout: web\n    base_branch: main\n";
        let remote = "schema_version: 3\nrepositories:\n  api:\n    bare: bares/api.git\n    checkout: api\n    base_branch: release\n  web:\n    bare: bares/web.git\n    checkout: web\n";

        let merged = ConfigLoader::merge_local_bases_into_yaml(local, remote).unwrap();
        assert!(merged.contains("base_branch: release"));
        assert!(merged.contains("base_branch: main"));
        assert!(!merged.contains("develop"));
    }
}
