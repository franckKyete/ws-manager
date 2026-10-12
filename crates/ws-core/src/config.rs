use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::{Result, WSError};
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
        let file_path = Self::find_config_file(config_path)?;
        let project_root = if let Some(ref fp) = file_path {
            fp.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
        } else {
            std::env::current_dir()?
        };

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
                                let cfg = RepoConfig::from_yaml_value(r_name, v)
                                    .map_err(WSError::Config)?;
                                repos.insert(r_name.to_string(), cfg);
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

        let mut data = serde_yaml::Mapping::new();
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
            let mut hub_map = serde_yaml::Mapping::new();
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
            data.insert(
                serde_yaml::Value::String("hub".to_string()),
                serde_yaml::Value::Mapping(hub_map),
            );
        }

        let content = serde_yaml::to_string(&serde_yaml::Value::Mapping(data))?;
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&path, content)?;
        Ok(path)
    }
}
