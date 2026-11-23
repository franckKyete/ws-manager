use serde::Serialize;
use ws_core::activity::{open_db, ActivityConfig, ActivityPaths};
use ws_core::activity::model::{ActivityStatus, ActivitySeriesPoint, ActivityBreakdownRow, ReportRow};

#[derive(Debug, Clone, Default, Serialize)]
pub struct DashboardSnapshot {
    pub status: Option<ActivityStatus>,
    pub from_hour: i64,
    pub to_hour: i64,
    pub total_active_seconds: i64,
    pub wall_clock_active_seconds: i64,
    pub series: Vec<ActivitySeriesPoint>,
    pub project_breakdown: Vec<ActivityBreakdownRow>,
    pub event_breakdown: Vec<ActivityBreakdownRow>,
    pub rows: Vec<ReportRow>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeKind {
    Today,
    Week,
    Month,
}

impl RangeKind {
    pub fn label(self) -> &'static str {
        match self {
            RangeKind::Today => "Today",
            RangeKind::Week => "7 days",
            RangeKind::Month => "30 days",
        }
    }
}

pub fn load_snapshot(range: RangeKind, project: Option<String>) -> DashboardSnapshot {
    let result = (|| -> Result<DashboardSnapshot, String> {
        let paths = ActivityPaths::discover();
        paths.ensure_dirs()?;
        let db = open_db(&paths)?;
        let cfg = ActivityConfig::load().unwrap_or_default();
        let _ = ws_core::activity::rollup::rollup_all(&db, &cfg);
        let status = db.basic_status(&paths)?;

        let now = ws_core::activity::db::now_ms();
        let today_start = local_day_start_ms(now);
        let (from_ms, to_ms) = match range {
            RangeKind::Today => (today_start, now),
            RangeKind::Week => (today_start - 6 * 86_400_000, now),
            RangeKind::Month => (today_start - 29 * 86_400_000, now),
        };
        let from_hour = (from_ms / 1000 / 3600) * 3600;
        let to_hour = (to_ms / 1000 / 3600) * 3600 + 3600;
        let step = match range { RangeKind::Today => "hour", _ => "day" };
        let project_filter = project.as_deref();

        let rows = db.report_totals(from_hour, to_hour, project_filter)?;
        let total_active_seconds = rows.iter().map(|r| r.active_seconds).sum();
        let wall_clock_active_seconds = db.report_wall_clock(from_hour, to_hour, project_filter, None, None)?;
        let series = db.report_series(from_hour, to_hour, step, project_filter, None, None, false)?;
        let project_breakdown = db.report_breakdown(from_hour, to_hour, "project", project_filter, None, None)?;
        let event_breakdown = db.report_breakdown(from_hour, to_hour, "repo", project_filter, None, None)?;

        Ok(DashboardSnapshot {
            status: Some(status),
            from_hour,
            to_hour,
            total_active_seconds,
            wall_clock_active_seconds,
            series,
            project_breakdown,
            event_breakdown,
            rows,
            error: None,
        })
    })();

    match result {
        Ok(s) => s,
        Err(e) => DashboardSnapshot { error: Some(e), ..Default::default() },
    }
}

fn local_day_start_ms(now_ms: i64) -> i64 {
    use chrono::TimeZone;
    let now = chrono::Local::now();
    now.date_naive().and_hms_opt(0, 0, 0)
        .and_then(|dt| chrono::Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now_ms - (now_ms % 86_400_000))
}

pub fn format_duration_human(secs: i64) -> String {
    let s = secs.max(0);
    let h = s / 3600;
    let m = (s % 3600) / 60;
    if h > 0 { format!("{}h {:02}m", h, m) } else { format!("{}m", m) }
}
