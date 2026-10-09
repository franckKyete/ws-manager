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
    pub name: String,
    pub bare: PathBuf,
    pub checkout: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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
    #[serde(default = "default_env_file")]
    pub env_file: String,
    #[serde(default = "default_env_example")]
    pub env_example: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_files: Vec<serde_json::Value>,
}

fn default_env_file() -> String {
    ".env".to_string()
}

fn default_env_example() -> String {
    ".env.example".to_string()
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

        let bare_str = mapping
            .get(serde_yaml::Value::String("bare".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Repository '{}' missing required 'bare'", name))?;

        let checkout_str = mapping
            .get(serde_yaml::Value::String("checkout".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Repository '{}' missing required 'checkout'", name))?;

        let url = mapping
            .get(serde_yaml::Value::String("url".to_string()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

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

        let env_file = mapping
            .get(serde_yaml::Value::String("env_file".to_string()))
            .and_then(|v| v.as_str())
            .unwrap_or(".env")
            .to_string();

        let env_example = mapping
            .get(serde_yaml::Value::String("env_example".to_string()))
            .and_then(|v| v.as_str())
            .unwrap_or(".env.example")
            .to_string();

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

        Ok(Self {
            name: name.to_string(),
            bare: PathBuf::from(bare_str),
            checkout: checkout_str.to_string(),
            url,
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
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoSpec {
    #[serde(default, skip_serializing)]
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
}

impl AppConfig {
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}
