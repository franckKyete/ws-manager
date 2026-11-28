use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::utils::parse_duration;

fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_idle_gap() -> u64 {
    300
}
fn default_session_cap() -> u64 {
    4 * 60 * 60
}
fn default_tail_credit() -> u64 {
    60
}
fn default_heartbeat_interval() -> u64 {
    60
}
fn default_heartbeat_window() -> u64 {
    15 * 60
}
fn default_debounce() -> u64 {
    2
}
fn default_raw_retention_days() -> u64 {
    365
}
fn default_watch_reconcile() -> u64 {
    60
}
fn default_sync_interval() -> u64 {
    600
}
fn default_backfill() -> String {
    "none".to_string()
}
fn default_idle_timeout() -> u64 {
    120
}
fn default_away_timeout() -> u64 {
    900
}
fn default_presence_source() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySyncConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_sync_interval")]
    pub interval: u64,
    #[serde(default = "default_backfill")]
    pub backfill: String,
}

impl Default for ActivitySyncConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: 600,
            backfill: "none".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum StringOrList {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::One(s) => s.trim().is_empty(),
            Self::Many(v) => v.is_empty() || v.iter().all(|s| s.trim().is_empty()),
        }
    }
    pub fn matches(&self, value: &str, glob: fn(&str, &str) -> bool) -> bool {
        match self {
            Self::None => true,
            Self::One(p) => glob(p, value),
            Self::Many(v) => v.iter().any(|p| glob(p, value)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PresenceRuleMatch {
    #[serde(default)]
    pub class: StringOrList,
    #[serde(default)]
    pub title: StringOrList,
    #[serde(default)]
    pub initial_class: StringOrList,
    #[serde(default)]
    pub domain: StringOrList,
    #[serde(default)]
    pub url: StringOrList,
    #[serde(default)]
    pub app_kind: StringOrList,
    #[serde(default)]
    pub tmux_session: StringOrList,
    #[serde(default)]
    pub tmux_window: StringOrList,
    #[serde(default)]
    pub path: StringOrList,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "match")]
    pub match_: PresenceRuleMatch,
    #[serde(default)]
    pub category: Option<String>, // work | leisure | neutral
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub require: Option<String>, // project
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceBrowserConfig {
    #[serde(default = "default_browser_inspect")]
    pub inspect: String, // auto|sessionstore|history|title|off
    #[serde(default = "default_browser_profiles")]
    pub profiles: String, // auto|<path>
}

fn default_browser_inspect() -> String {
    "auto".to_string()
}
fn default_browser_profiles() -> String {
    "auto".to_string()
}

impl Default for PresenceBrowserConfig {
    fn default() -> Self {
        Self {
            inspect: default_browser_inspect(),
            profiles: default_browser_profiles(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityPresenceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_presence_source")]
    pub source: String,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: u64,
    #[serde(default = "default_away_timeout")]
    pub away_timeout: u64,
    #[serde(default = "default_true")]
    pub lock_aware: bool,
    #[serde(default = "default_true")]
    pub sleep_aware: bool,
    #[serde(default = "default_true")]
    pub attribution: bool,
    #[serde(default = "default_true")]
    pub hyprland: bool,
    #[serde(default = "default_true")]
    pub terminal_process_tree: bool,
    #[serde(default = "default_true")]
    pub extend_sessions: bool,
    #[serde(default)]
    pub browser: PresenceBrowserConfig,
    #[serde(default)]
    pub rules: Vec<PresenceRule>,
}

impl Default for ActivityPresenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            source: default_presence_source(),
            idle_timeout: default_idle_timeout(),
            away_timeout: default_away_timeout(),
            lock_aware: true,
            sleep_aware: true,
            attribution: true,
            hyprland: true,
            terminal_process_tree: true,
            extend_sessions: true,
            browser: PresenceBrowserConfig::default(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_false")]
    pub paused: bool,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default = "default_idle_gap")]
    pub idle_gap: u64,
    #[serde(default = "default_session_cap")]
    pub session_cap: u64,
    #[serde(default = "default_tail_credit")]
    pub tail_credit: u64,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,
    #[serde(default = "default_heartbeat_window")]
    pub heartbeat_window: u64,
    #[serde(default = "default_debounce")]
    pub debounce: u64,
    #[serde(default = "default_raw_retention_days", alias = "raw_retention")]
    pub raw_retention_days: u64,
    #[serde(default = "default_watch_reconcile")]
    pub watch_reconcile: u64,
    #[serde(default)]
    pub sync: ActivitySyncConfig,
    #[serde(default)]
    pub presence: ActivityPresenceConfig,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            paused: false,
            device_name: None,
            idle_gap: default_idle_gap(),
            session_cap: default_session_cap(),
            tail_credit: default_tail_credit(),
            heartbeat_interval: default_heartbeat_interval(),
            heartbeat_window: default_heartbeat_window(),
            debounce: default_debounce(),
            raw_retention_days: default_raw_retention_days(),
            watch_reconcile: default_watch_reconcile(),
            sync: ActivitySyncConfig::default(),
            presence: ActivityPresenceConfig::default(),
            exclude: Vec::new(),
        }
    }
}

impl ActivityConfig {
    pub fn load() -> Result<Self, String> {
        let paths = ActivityPaths::discover();
        Self::load_from(&paths.config_path)
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        let mut cfg = if path.is_file() {
            let content = fs::read_to_string(path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let val: serde_yaml::Value = serde_yaml::from_str(&content)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            Self::from_yaml_value(&val)?
        } else {
            Self::default()
        };
        cfg.apply_env_overrides();
        Ok(cfg)
    }

    fn from_yaml_value(val: &serde_yaml::Value) -> Result<Self, String> {
        let mut json = serde_json::to_value(val).map_err(|e| e.to_string())?;
        if let Some(obj) = json.as_object_mut() {
            for key in [
                "idle_gap",
                "session_cap",
                "tail_credit",
                "heartbeat_interval",
                "heartbeat_window",
                "debounce",
                "watch_reconcile",
            ] {
                if let Some(v) = obj.get(key) {
                    if let Some(s) = v.as_str() {
                        obj.insert(key.to_string(), serde_json::Value::from(parse_duration(s)));
                    }
                }
            }
            if let Some(ret) = obj
                .get("raw_retention")
                .or_else(|| obj.get("raw_retention_days"))
                .cloned()
            {
                let days = match ret {
                    serde_json::Value::Number(n) => n.as_u64().unwrap_or(365),
                    serde_json::Value::String(s) => parse_retention_days(&s),
                    _ => 365,
                };
                obj.insert(
                    "raw_retention_days".to_string(),
                    serde_json::Value::from(days),
                );
            }
            if let Some(sync) = obj.get_mut("sync").and_then(|v| v.as_object_mut()) {
                if let Some(v) = sync.get("interval") {
                    if let Some(s) = v.as_str() {
                        sync.insert(
                            "interval".to_string(),
                            serde_json::Value::from(parse_duration(s)),
                        );
                    }
                }
            }
            if let Some(pres) = obj.get_mut("presence").and_then(|v| v.as_object_mut()) {
                for key in ["idle_timeout", "away_timeout"] {
                    if let Some(v) = pres.get(key) {
                        if let Some(s) = v.as_str() {
                            pres.insert(
                                key.to_string(),
                                serde_json::Value::from(parse_duration(s)),
                            );
                        }
                    }
                }
            }
        }
        serde_json::from_value(json).map_err(|e| format!("Invalid activity config: {e}"))
    }

    fn apply_env_overrides(&mut self) {
        if std::env::var("WS_ACTIVITY_DISABLE").ok().as_deref() == Some("1") {
            self.enabled = false;
        }
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let paths = ActivityPaths::discover();
        self.save_to(&paths.config_path)
    }

    pub fn save_to(&self, path: &Path) -> Result<PathBuf, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        }
        let content = serde_yaml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        Ok(path.to_path_buf())
    }

    pub fn effective_device_name(&self) -> String {
        if let Some(name) = &self.device_name {
            let t = name.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        hostname_fallback()
    }
}

fn parse_retention_days(s: &str) -> u64 {
    let t = s.trim().to_lowercase();
    if t.ends_with("mo") || t.ends_with("month") || t.ends_with("months") {
        let n: u64 = t
            .trim_end_matches("months")
            .trim_end_matches("month")
            .trim_end_matches("mo")
            .trim()
            .parse()
            .unwrap_or(12);
        return n.saturating_mul(30);
    }
    if t.ends_with('d') {
        let n: u64 = t.trim_end_matches('d').trim().parse().unwrap_or(365);
        return n.max(1);
    }
    let secs = parse_duration(&t);
    if secs == 0 {
        365
    } else {
        (secs / 86400).max(1)
    }
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "device".to_string())
}

#[derive(Debug, Clone)]
pub struct ActivityPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub spool_path: PathBuf,
    pub socket_path: PathBuf,
}

impl ActivityPaths {
    pub fn discover() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("ws");
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| home.join(".local/share"))
            .join("ws");
        let runtime_dir = dirs::runtime_dir()
            .unwrap_or_else(|| data_dir.clone())
            .join("ws");
        let config_path = std::env::var_os("WS_ACTIVITY_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("activity.yml"));
        let db_path = std::env::var_os("WS_ACTIVITY_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("activity.db"));
        let spool_path = data_dir.join("activity-spool.jsonl");
        let socket_path = runtime_dir.join("activity.sock");
        Self {
            config_dir,
            data_dir,
            runtime_dir,
            config_path,
            db_path,
            spool_path,
            socket_path,
        }
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        for dir in [&self.config_dir, &self.data_dir, &self.runtime_dir] {
            fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
            }
        }
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectActivityConfig {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub sync: Option<bool>,
    #[serde(default)]
    pub hub_project: Option<String>,
}
