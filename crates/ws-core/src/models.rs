use crate::utils::{format_duration, get_iso_timestamp, parse_duration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn is_secret_val(val: &str) -> bool {
    val.starts_with("secret:") || val.starts_with("vault:")
}

pub fn is_private_val(val: &str) -> bool {
    val.starts_with("private:") || val.starts_with("local:")
}

pub fn clean_env_val(val: &str) -> String {
    if let Some(stripped) = val.strip_prefix("secret:") {
        stripped.to_string()
    } else if let Some(stripped) = val.strip_prefix("vault:") {
        stripped.to_string()
    } else if let Some(stripped) = val.strip_prefix("private:") {
        stripped.to_string()
    } else if let Some(stripped) = val.strip_prefix("local:") {
        stripped.to_string()
    } else if let Some(stripped) = val.strip_prefix("public:") {
        stripped.to_string()
    } else {
        val.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RepoConfig {
    #[serde(default, skip_serializing)]
    pub name: String,
    pub bare: PathBuf,
    pub checkout: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ports: HashMap<String, u16>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub secret_env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub private_env: HashMap<String, String>,
    #[serde(
        default = "default_env_file",
        skip_serializing_if = "is_default_env_file"
    )]
    pub env_file: String,
    #[serde(
        default = "default_env_example",
        skip_serializing_if = "is_default_env_example"
    )]
    pub env_example: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_files: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<serde_json::Value>,
}

fn default_env_file() -> String {
    ".env".to_string()
}

fn is_default_env_file(s: &str) -> bool {
    s == ".env"
}

fn default_env_example() -> String {
    ".env.example".to_string()
}

fn is_default_env_example(s: &str) -> bool {
    s == ".env.example"
}

impl RepoConfig {
    pub fn new(
        name: impl Into<String>,
        bare: impl Into<PathBuf>,
        checkout: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            bare: bare.into(),
            checkout: checkout.into(),
            url: None,
            base_branch: None,
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
            lifecycle: None,
        }
    }

    pub fn ports_list(&self) -> Vec<u16> {
        if !self.ports.is_empty() {
            let mut list: Vec<_> = self.ports.values().copied().collect();
            list.sort();
            list
        } else if let Some(p) = self.port {
            vec![p]
        } else {
            Vec::new()
        }
    }

    pub fn from_yaml_value(name: &str, val: &serde_yaml::Value) -> Result<Self, String> {
        let mapping = val
            .as_mapping()
            .ok_or_else(|| format!("Repo '{}' must be a mapping", name))?;

        let clean_name = name.trim_start_matches(['%', '+', ':', '#', '$']);

        let bare_str = mapping
            .get(serde_yaml::Value::String("bare".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Repository '{}' missing required 'bare'", clean_name))?;

        let checkout_str = mapping
            .get(serde_yaml::Value::String("checkout".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Repository '{}' missing required 'checkout'", clean_name))?;
        let clean_checkout = checkout_str.trim_start_matches(['%', '+', ':', '#', '$']);

        let url = mapping
            .get(serde_yaml::Value::String("url".to_string()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let base_branch = match mapping.get(serde_yaml::Value::String("base_branch".to_string())) {
            Some(v) if v.is_null() => None,
            Some(v) => {
                let raw = v.as_str().ok_or_else(|| {
                    format!(
                        "Repository '{}' field 'base_branch' must be a string",
                        clean_name
                    )
                })?;
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            None => None,
        };

        let mut public_env = HashMap::new();
        let mut secret_env = HashMap::new();
        let mut private_env = HashMap::new();

        // 1. env block
        if let Some(env_val) = mapping.get(serde_yaml::Value::String("env".to_string())) {
            if let Some(env_map) = env_val.as_mapping() {
                for (k, v) in env_map {
                    let k_str = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        _ => k.as_str().unwrap_or("").to_string(),
                    };
                    let v_str = match v {
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Number(n) => n.to_string(),
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        _ => "".to_string(),
                    };
                    if is_secret_val(&v_str) {
                        secret_env.insert(k_str, clean_env_val(&v_str));
                    } else if is_private_val(&v_str) {
                        private_env.insert(k_str, clean_env_val(&v_str));
                    } else {
                        public_env.insert(k_str, clean_env_val(&v_str));
                    }
                }
            }
        }

        // 2. secret block
        if let Some(secret_val) = mapping
            .get(serde_yaml::Value::String("secret".to_string()))
            .or_else(|| mapping.get(serde_yaml::Value::String("secrets".to_string())))
        {
            if let Some(sec_map) = secret_val.as_mapping() {
                for (k, v) in sec_map {
                    if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                        secret_env.insert(k_str.to_string(), clean_env_val(v_str));
                    }
                }
            }
        }

        // 3. private block
        if let Some(private_val) = mapping
            .get(serde_yaml::Value::String("private".to_string()))
            .or_else(|| mapping.get(serde_yaml::Value::String("local_env".to_string())))
        {
            if let Some(priv_map) = private_val.as_mapping() {
                for (k, v) in priv_map {
                    if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                        private_env.insert(k_str.to_string(), clean_env_val(v_str));
                    }
                }
            }
        }

        let mut ports_dict = HashMap::new();
        if let Some(ports_val) = mapping.get(serde_yaml::Value::String("ports".to_string())) {
            if let Some(ports_map) = ports_val.as_mapping() {
                for (k, v) in ports_map {
                    if let (Some(k_str), Some(v_num)) = (k.as_str(), v.as_u64()) {
                        ports_dict.insert(k_str.to_string(), v_num as u16);
                    }
                }
            } else if let Some(ports_seq) = ports_val.as_sequence() {
                for (idx, v) in ports_seq.iter().enumerate() {
                    if let Some(p) = v.as_u64() {
                        let k_name = if idx == 0 {
                            "default".to_string()
                        } else {
                            format!("port_{}", idx)
                        };
                        ports_dict.insert(k_name, p as u16);
                    }
                }
            } else if let Some(p) = ports_val.as_u64() {
                ports_dict.insert("default".to_string(), p as u16);
            }
        }

        let mut port_val = mapping
            .get(serde_yaml::Value::String("port".to_string()))
            .and_then(|v| v.as_u64())
            .map(|p| p as u16);

        if let Some(p) = port_val {
            if !ports_dict.contains_key("default") {
                ports_dict.insert("default".to_string(), p);
            }
        } else if let Some(p) = ports_dict
            .get("default")
            .or_else(|| ports_dict.values().next())
        {
            port_val = Some(*p);
        }

        let launch = mapping
            .get(serde_yaml::Value::String("launch".to_string()))
            .or_else(|| mapping.get(serde_yaml::Value::String("command".to_string())))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let env_file = {
            let v = mapping
                .get(serde_yaml::Value::String("env_file".to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or(".env")
                .trim()
                .trim_start_matches("./");
            if v.is_empty() || v == "." {
                ".env".to_string()
            } else {
                v.to_string()
            }
        };

        let env_example = {
            let v = mapping
                .get(serde_yaml::Value::String("env_example".to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or(".env.example")
                .trim()
                .trim_start_matches("./");
            if v.is_empty() || v == "." {
                ".env.example".to_string()
            } else {
                v.to_string()
            }
        };

        let mut setup_list = Vec::new();
        if let Some(setup_val) = mapping.get(serde_yaml::Value::String("setup".to_string())) {
            if let Some(s) = setup_val.as_str() {
                setup_list.push(s.to_string());
            } else if let Some(seq) = setup_val.as_sequence() {
                for item in seq {
                    if let Some(s) = item.as_str() {
                        setup_list.push(s.to_string());
                    } else if let Ok(json) = serde_json::to_value(item) {
                        setup_list.push(json.to_string());
                    }
                }
            }
        }

        let mut copy_files = Vec::new();
        if let Some(files_val) = mapping
            .get(serde_yaml::Value::String("copy_files".to_string()))
            .or_else(|| mapping.get(serde_yaml::Value::String("files".to_string())))
        {
            if let Ok(json) = serde_json::to_value(files_val) {
                if let Some(arr) = json.as_array() {
                    copy_files = arr.clone();
                } else {
                    copy_files.push(json);
                }
            }
        }

        let lifecycle = mapping
            .get(serde_yaml::Value::String("lifecycle".to_string()))
            .and_then(|v| serde_json::to_value(v).ok());

        Ok(Self {
            name: clean_name.to_string(),
            bare: PathBuf::from(bare_str),
            checkout: clean_checkout.to_string(),
            url,
            base_branch,
            port: port_val,
            ports: ports_dict,
            env: public_env,
            secret_env,
            private_env,
            env_file,
            env_example,
            setup: setup_list,
            launch,
            secrets: Vec::new(),
            copy_files,
            lifecycle,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoSpec {
    #[serde(default)]
    pub name: String,
    pub branch: String,
    #[serde(default = "default_true")]
    pub create: bool,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub frozen: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(
        skip_serializing_if = "Option::is_none",
        alias = "base",
        alias = "target",
        alias = "from"
    )]
    pub base_branch: Option<String>,
}

fn default_true() -> bool {
    true
}

impl RepoSpec {
    pub fn new(
        name: impl Into<String>,
        branch: impl Into<String>,
        create: bool,
        path: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            branch: branch.into(),
            create,
            path: path.into(),
            frozen: false,
            locked: false,
            base_branch: None,
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked || self.frozen
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.frozen = locked;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub name: String,
    #[serde(default = "get_iso_timestamp")]
    pub created: String,
    #[serde(default = "default_active_status")]
    pub status: String,
    #[serde(default)]
    pub repositories: HashMap<String, RepoSpec>,
}

fn default_active_status() -> String {
    "active".to_string()
}

impl WorkspaceMetadata {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            created: get_iso_timestamp(),
            status: "active".to_string(),
            repositories: HashMap::new(),
        }
    }

    pub fn from_yaml_str(content: &str) -> Result<Self, String> {
        let val: serde_yaml::Value = serde_yaml::from_str(content).map_err(|e| e.to_string())?;
        Self::from_yaml_value(val)
    }

    pub fn from_yaml_value(val: serde_yaml::Value) -> Result<Self, String> {
        let json_val = serde_json::to_value(val).map_err(|e| e.to_string())?;
        let mut meta: WorkspaceMetadata =
            serde_json::from_value(json_val).map_err(|e| e.to_string())?;
        meta.normalize();
        Ok(meta)
    }

    pub fn from_json_value(val: serde_json::Value) -> Result<Self, String> {
        let mut meta: WorkspaceMetadata = serde_json::from_value(val).map_err(|e| e.to_string())?;
        meta.normalize();
        Ok(meta)
    }

    pub fn from_json_str(content: &str) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_str(content).map_err(|e| e.to_string())?;
        Self::from_json_value(val)
    }

    pub fn normalize(&mut self) {
        for (k, spec) in self.repositories.iter_mut() {
            if spec.name.is_empty() {
                spec.name = k.clone();
            }
            if spec.path.is_empty() {
                spec.path = k.clone();
            }
            if spec.frozen {
                spec.locked = true;
            }
            if spec.locked {
                spec.frozen = true;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TmuxConfig {
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "cmd")]
    pub command: Option<String>,
    #[serde(default)]
    pub switch: bool,
}

impl TmuxConfig {
    pub fn from_value(val: &serde_yaml::Value) -> Result<Self, String> {
        if let Some(s) = val.as_str() {
            return Ok(Self {
                session: s.trim().to_string(),
                launch_session: None,
                command: None,
                switch: false,
            });
        }
        if let Some(map) = val.as_mapping() {
            let session = map
                .get(serde_yaml::Value::String("session".to_string()))
                .or_else(|| map.get(serde_yaml::Value::String("session_name".to_string())))
                .or_else(|| map.get(serde_yaml::Value::String("name".to_string())))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Tmux configuration must include 'session'".to_string())?
                .trim()
                .to_string();

            let launch_session = map
                .get(serde_yaml::Value::String("launch_session".to_string()))
                .or_else(|| map.get(serde_yaml::Value::String("launch".to_string())))
                .or_else(|| map.get(serde_yaml::Value::String("launch_name".to_string())))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string());

            let command = map
                .get(serde_yaml::Value::String("command".to_string()))
                .or_else(|| map.get(serde_yaml::Value::String("cmd".to_string())))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string());

            let switch = map
                .get(serde_yaml::Value::String("switch".to_string()))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            return Ok(Self {
                session,
                launch_session,
                command,
                switch,
            });
        }
        Err("Invalid tmux configuration format".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkspacesSelector {
    Mode(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubAutoSaveConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_true")]
    pub include_wip: bool,
    #[serde(default = "default_workspaces_selector")]
    pub workspaces: WorkspacesSelector,
    #[serde(default = "default_true")]
    pub notify: bool,
}

fn default_interval() -> u64 {
    900
}

fn default_workspaces_selector() -> WorkspacesSelector {
    WorkspacesSelector::Mode("all".to_string())
}

impl HubAutoSaveConfig {
    pub fn from_value(val: &serde_yaml::Value) -> Result<Self, String> {
        if let Some(b) = val.as_bool() {
            return Ok(Self {
                enabled: b,
                interval: 900,
                include_wip: true,
                workspaces: WorkspacesSelector::Mode("all".to_string()),
                notify: true,
            });
        }
        if let Some(map) = val.as_mapping() {
            let mut enabled = map
                .get(serde_yaml::Value::String("enabled".to_string()))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let interval_raw = map.get(serde_yaml::Value::String("interval".to_string()));
            let interval = if let Some(i_val) = interval_raw {
                if let Some(n) = i_val.as_u64() {
                    n
                } else if let Some(s) = i_val.as_str() {
                    let sec = parse_duration(s);
                    if sec == 0 {
                        enabled = false;
                    }
                    sec
                } else {
                    900
                }
            } else {
                900
            };

            let include_wip = map
                .get(serde_yaml::Value::String("include_wip".to_string()))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let workspaces = if let Some(ws_val) =
                map.get(serde_yaml::Value::String("workspaces".to_string()))
            {
                if let Some(s) = ws_val.as_str() {
                    WorkspacesSelector::Mode(s.to_string())
                } else if let Some(seq) = ws_val.as_sequence() {
                    let list = seq
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    WorkspacesSelector::List(list)
                } else {
                    WorkspacesSelector::Mode("all".to_string())
                }
            } else {
                WorkspacesSelector::Mode("all".to_string())
            };

            let notify = map
                .get(serde_yaml::Value::String("notify".to_string()))
                .or_else(|| map.get(serde_yaml::Value::String("notifications".to_string())))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            return Ok(Self {
                enabled,
                interval: if interval > 0 { interval } else { 900 },
                include_wip,
                workspaces,
                notify,
            });
        }
        Err("Invalid hub auto_save configuration format".to_string())
    }

    pub fn to_map(&self, human_interval: bool) -> serde_json::Value {
        let interval_val = if human_interval {
            serde_json::Value::String(format_duration(self.interval))
        } else {
            serde_json::Value::Number(self.interval.into())
        };
        serde_json::json!({
            "enabled": self.enabled,
            "interval": interval_val,
            "include_wip": self.include_wip,
            "workspaces": self.workspaces,
            "notify": self.notify,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkspaceSaveConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(
        default = "default_workspace_save_max_file_size",
        skip_serializing_if = "is_default_workspace_save_max_file_size"
    )]
    pub max_file_size: u64,
}

fn default_workspace_save_max_file_size() -> u64 {
    1024 * 1024
}

fn is_default_workspace_save_max_file_size(v: &u64) -> bool {
    *v == default_workspace_save_max_file_size()
}

impl WorkspaceSaveConfig {
    pub fn from_value(val: Option<&serde_json::Value>) -> Self {
        let mut cfg = Self {
            exclude: Vec::new(),
            max_file_size: default_workspace_save_max_file_size(),
        };
        let Some(val) = val else {
            return cfg;
        };
        let Some(map) = val.as_object() else {
            return cfg;
        };
        if let Some(ex) = map.get("exclude") {
            if let Some(arr) = ex.as_array() {
                cfg.exclude = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
            } else if let Some(s) = ex.as_str() {
                cfg.exclude = vec![s.to_string()];
            }
        }
        if let Some(m) = map.get("max_file_size").and_then(|v| v.as_u64()) {
            if m > 0 {
                cfg.max_file_size = m;
            }
        }
        cfg
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkspaceRootConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_files: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(
        default = "default_env_file",
        skip_serializing_if = "is_default_env_file"
    )]
    pub env_file: String,
    #[serde(
        default = "default_env_example",
        skip_serializing_if = "is_default_env_example"
    )]
    pub env_example: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<WorkspaceSaveConfig>,
}

impl WorkspaceRootConfig {
    pub fn is_empty(&self) -> bool {
        self.copy_files.is_empty()
            && self.env.is_empty()
            && self.env_file == default_env_file()
            && self.env_example == default_env_example()
            && self
                .save
                .as_ref()
                .map(|s| {
                    s.exclude.is_empty()
                        && s.max_file_size == default_workspace_save_max_file_size()
                })
                .unwrap_or(true)
    }

    pub fn from_yaml_value(val: &serde_yaml::Value) -> Result<Self, String> {
        let mapping = val
            .as_mapping()
            .ok_or_else(|| "workspace config must be a mapping".to_string())?;
        let mut cfg = WorkspaceRootConfig {
            copy_files: Vec::new(),
            env: HashMap::new(),
            env_file: default_env_file(),
            env_example: default_env_example(),
            save: None,
        };

        if let Some(files_val) = mapping
            .get(serde_yaml::Value::String("copy_files".to_string()))
            .or_else(|| mapping.get(serde_yaml::Value::String("files".to_string())))
        {
            if let Ok(json) = serde_json::to_value(files_val) {
                if let Some(arr) = json.as_array() {
                    cfg.copy_files = arr.clone();
                } else {
                    cfg.copy_files.push(json);
                }
            }
        }

        if let Some(env_val) = mapping.get(serde_yaml::Value::String("env".to_string())) {
            if let Some(env_map) = env_val.as_mapping() {
                for (k, v) in env_map {
                    let k_str = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        _ => k.as_str().unwrap_or("").to_string(),
                    };
                    let v_str = match v {
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Number(n) => n.to_string(),
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        _ => "".to_string(),
                    };
                    cfg.env.insert(k_str, clean_env_val(&v_str));
                }
            }
        }

        if let Some(s) = mapping
            .get(serde_yaml::Value::String("env_file".to_string()))
            .and_then(|v| v.as_str())
        {
            let v = s.trim();
            if !v.is_empty() && v != "." {
                cfg.env_file = v.trim_start_matches("./").to_string();
            }
        }
        if let Some(s) = mapping
            .get(serde_yaml::Value::String("env_example".to_string()))
            .and_then(|v| v.as_str())
        {
            let v = s.trim();
            if !v.is_empty() && v != "." {
                cfg.env_example = v.trim_start_matches("./").to_string();
            }
        }

        if let Some(save_val) = mapping.get(serde_yaml::Value::String("save".to_string())) {
            let json = serde_json::to_value(save_val).ok();
            let save = WorkspaceSaveConfig::from_value(json.as_ref());
            if !save.exclude.is_empty()
                || save.max_file_size != default_workspace_save_max_file_size()
            {
                cfg.save = Some(save);
            } else if save_val.as_mapping().is_some() {
                // preserve explicit empty save block awareness not required; keep None for empty
                cfg.save = None;
            }
        }

        Ok(cfg)
    }

    pub fn save_max_file_size(&self) -> u64 {
        self.save
            .as_ref()
            .map(|s| s.max_file_size)
            .unwrap_or_else(default_workspace_save_max_file_size)
    }

    pub fn save_excludes(&self) -> Vec<String> {
        self.save
            .as_ref()
            .map(|s| s.exclude.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub project_root: PathBuf,
    pub repositories: HashMap<String, RepoConfig>,
    pub workspaces_dir: PathBuf,
    pub config_file_path: Option<PathBuf>,
    pub global_env: HashMap<String, String>,
    pub secret_env: HashMap<String, String>,
    pub private_env: HashMap<String, String>,
    pub dynamic_env: HashMap<String, String>,
    pub setup: Vec<String>,
    pub secrets: Vec<String>,
    pub copy_files: Vec<serde_json::Value>,
    pub tmux: Option<TmuxConfig>,
    pub hub_auto_save: Option<HubAutoSaveConfig>,
    pub hub_project: Option<String>,
    pub hub: serde_json::Value,
    pub lifecycle: Option<serde_json::Value>,
    pub schema_version: Option<u32>,
    pub workspace: WorkspaceRootConfig,
}

impl AppConfig {
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}
