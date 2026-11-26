use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityEvent {
    pub ts: i64,
    pub kind: String,
    pub project_id: String,
    pub project_name: String,
    #[serde(default)] pub workspace: String,
    #[serde(default)] pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub rel_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub duration_ms: Option<i64>,
    #[serde(default = "default_count")] pub count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub meta: Option<String>,
}
fn default_count() -> i64 { 1 }

impl ActivityEvent {
    pub fn is_real(&self) -> bool { self.kind != "heartbeat" }
    pub fn category(&self) -> &'static str {
        if self.kind.starts_with("file.") { "file" }
        else if self.kind.starts_with("git.") { "git" }
        else if self.kind == "command" { "command" }
        else { "heartbeat" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Bucket {
    pub project_id: String,
    pub project_name: String,
    pub hour: i64,
    #[serde(default)] pub workspace: String,
    #[serde(default)] pub repo: String,
    pub active_seconds: i64,
    pub file_events: i64,
    pub git_events: i64,
    pub command_events: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub project_id: String,
    pub path: String,
    pub name: String,
    pub hub_project: Option<String>,
    #[serde(default)] pub hub_auto_created: bool,
    pub sync_enabled: bool,
    pub disabled_reason: Option<String>,
    pub linked_at: Option<i64>,
    pub last_synced_hour: Option<i64>,
    pub backoff_until: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivityStatus {
    pub enabled: bool,
    pub paused: bool,
    pub daemon_running: bool,
    pub daemon_pid: Option<i32>,
    pub db_path: String,
    pub db_size_bytes: u64,
    pub event_count: i64,
    pub bucket_count: i64,
    pub device_id: Option<String>,
    pub device_name: String,
    pub privacy_notice_shown_at: Option<i64>,
    pub privacy_notice_pending: bool,
    pub last_event_at: Option<i64>,
    pub watching_workspaces: usize,
    pub known_workspaces: usize,
    pub watch_limited: bool,
    pub projects: Vec<ProjectState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportRow {
    pub project_id: String,
    pub project_name: String,
    pub workspace: String,
    pub repo: String,
    pub active_seconds: i64,
    pub file_events: i64,
    pub git_events: i64,
    pub command_events: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivitySeriesPoint {
    pub bucket_start: i64,
    pub active_seconds: i64,
    pub file_events: i64,
    pub git_events: i64,
    pub command_events: i64,
    pub session_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivityBreakdownRow {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub active_seconds: i64,
    pub file_events: i64,
    pub git_events: i64,
    pub command_events: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivitySnapshotPoint {
    pub bucket_start: i64,
    pub label: String,
    pub active_seconds: i64,
    pub file_events: i64,
    pub git_events: i64,
    pub command_events: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivityTotals {
    /// Deduped event-work seconds (union of credited spans). Headline metric.
    pub active_seconds: i64,
    /// Raw per-session sum before overlap dedupe; diagnostic only.
    #[serde(default)]
    pub raw_active_seconds: i64,
    pub wall_clock_seconds: i64,
    pub file_events: i64,
    pub git_events: i64,
    pub command_events: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PresenceTotals {
    pub present_seconds: i64,
    pub active_seconds: i64,
    pub idle_seconds: i64,
    pub away_seconds: i64,
    pub locked_seconds: i64,
    pub asleep_seconds: i64,
    #[serde(default)]
    pub work_seconds: i64,
    #[serde(default)]
    pub active_work_seconds: i64,
    #[serde(default)]
    pub leisure_seconds: i64,
    #[serde(default)]
    pub neutral_seconds: i64,
    #[serde(default)]
    pub unattributed_seconds: i64,
    #[serde(default)]
    pub unassigned_work_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PresenceSnapshot {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_rule: Option<String>,
    #[serde(default)]
    pub category_source: String,
    #[serde(default)]
    pub totals: PresenceTotals,
    /// Raw event-session active time (headline). Deduped union of credited spans; not capped by presence.
    #[serde(default)]
    pub event_work_seconds: i64,
    /// Raw per-session sum before overlap dedupe; diagnostic only.
    #[serde(default)]
    pub raw_event_seconds: i64,
    /// Input-active time while focused on a work context.
    #[serde(default)]
    pub active_work_seconds: i64,
    /// Present (active+idle) time while focused on a work context.
    #[serde(default)]
    pub focused_work_seconds: i64,
    /// Compatibility alias for event_work_seconds.
    #[serde(default)]
    pub event_seconds: i64,
    #[serde(default)]
    pub presence_only_seconds: i64,
    #[serde(default)]
    pub background_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivitySnapshot {
    pub range: String,
    pub generated_at: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub status: ActivityStatus,
    pub totals: ActivityTotals,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<PresenceSnapshot>,
    pub series: Vec<ActivitySnapshotPoint>,
    pub projects: Vec<ActivityBreakdownRow>,
    pub repos: Vec<ActivityBreakdownRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_projects: Vec<ActivityBreakdownRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_repos: Vec<ActivityBreakdownRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInterval {
    pub id: i64,
    pub start_ts: i64,
    pub end_ts: Option<i64>,
    pub state: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PresenceBucket {
    pub hour: i64,
    pub active_seconds: i64,
    pub idle_seconds: i64,
    pub away_seconds: i64,
    pub locked_seconds: i64,
    pub asleep_seconds: i64,
    #[serde(default)]
    pub work_seconds: i64,
    #[serde(default)]
    pub active_work_seconds: i64,
    #[serde(default)]
    pub leisure_seconds: i64,
    #[serde(default)]
    pub neutral_seconds: i64,
    #[serde(default)]
    pub unattributed_seconds: i64,
    #[serde(default)]
    pub unassigned_work_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FocusInterval {
    pub id: i64,
    pub start_ts: i64,
    pub end_ts: Option<i64>,
    #[serde(default)] pub project_id: String,
    #[serde(default)] pub project_name: String,
    #[serde(default)] pub workspace: String,
    #[serde(default)] pub repo: String,
    #[serde(default)] pub source: String,      // cwd|process|rule|manual|unattributed|tmux|browser|file
    #[serde(default)] pub confidence: String,
    #[serde(default)] pub category: String,     // work|leisure|neutral|unknown
    #[serde(default)] pub category_source: String, // manual|target|work_rule|leisure_rule|default
    #[serde(default)] pub app: String,
    #[serde(default)] pub app_kind: String,    // terminal|browser|editor|media|game|chat|office|file_manager|system|unknown
    #[serde(default)] pub domain: String,
    #[serde(default)] pub rule: String,
    #[serde(default)] pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FocusBucket {
    pub project_id: String,
    pub project_name: String,
    pub hour: i64,
    #[serde(default)] pub workspace: String,
    #[serde(default)] pub repo: String,
    pub work_seconds: i64,
    pub input_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CategoryOverride {
    pub id: i64,
    pub start_ts: i64,
    pub end_ts: Option<i64>,
    pub category: String,
    pub scope: String, // global|focus
    #[serde(default)] pub match_project: String,
    #[serde(default)] pub match_repo: String,
    #[serde(default)] pub match_workspace: String,
    #[serde(default)] pub match_domain: String,
    #[serde(default)] pub match_app: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaybarStatus {
    pub text: String,
    pub alt: String,
    pub tooltip: String,
    #[serde(default)]
    pub class: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u8>,
}
