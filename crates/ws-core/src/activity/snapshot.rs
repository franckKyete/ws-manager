use chrono::{Duration, Local, NaiveDate, TimeZone, Timelike};

use super::db::ActivityDb;
use super::model::{
    ActivitySeriesPoint, ActivitySnapshot, ActivitySnapshotPoint, ActivityStatus, ActivityTotals,
    PresenceSnapshot,
};

/// Build a dense, local-time-aligned activity snapshot for the requested range.
///
/// The database stores buckets aligned to UTC hours. To keep day boundaries
/// intuitive for the user, buckets are re-grouped into local hourly or daily
/// slots here instead of using the UTC-aligned `report_series` bucketing.
pub fn build_snapshot(
    db: &ActivityDb,
    status: ActivityStatus,
    range: &str,
    now: i64,
    project: Option<&str>,
) -> Result<ActivitySnapshot, String> {
    let range = match range {
        "day" | "" => "today",
        other @ ("today" | "week" | "month") => other,
        _ => return Err("Invalid --range. Use 'today', 'week' or 'month'.".to_string()),
    };

    let today = Local
        .timestamp_opt(now / 1000, 0)
        .single()
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| Local::now().date_naive());

    let (start_date, count, hourly) = match range {
        "today" => (today, 24usize, true),
        "week" => (today - Duration::days(6), 7usize, false),
        _ => (today - Duration::days(29), 30usize, false),
    };

    let slots = build_slots(start_date, count, hourly, range);
    let from_ms = slots
        .first()
        .map(|s| s.start_ms)
        .unwrap_or_else(|| local_ms(start_date, 0));
    let to_ms = local_ms(today + Duration::days(1), 0);

    let from_hour = (from_ms / 1000 / 3600) * 3600;
    let to_hour = ((to_ms + 999) / 1000 / 3600) * 3600;

    let rows = db.report_series(from_hour, to_hour, "hour", project, None, None, false)?;
    let mut series: Vec<ActivitySnapshotPoint> = slots
        .iter()
        .map(|slot| ActivitySnapshotPoint {
            bucket_start: slot.start_ms,
            label: slot.label.clone(),
            ..Default::default()
        })
        .collect();

    for row in rows {
        let Some(index) = slot_index(start_date, hourly, count, row.bucket_start) else {
            continue;
        };
        let point = &mut series[index];
        accumulate(point, &row);
    }

    let raw_rows = db
        .report_totals(from_hour, to_hour, project)
        .unwrap_or_default();
    let raw_active_seconds: i64 = raw_rows.iter().map(|r| r.active_seconds).sum();
    let totals = ActivityTotals {
        active_seconds: series.iter().map(|p| p.active_seconds).sum(),
        raw_active_seconds,
        wall_clock_seconds: db.report_wall_clock(from_hour, to_hour, project, None, None)?,
        file_events: series.iter().map(|p| p.file_events).sum(),
        git_events: series.iter().map(|p| p.git_events).sum(),
        command_events: series.iter().map(|p| p.command_events).sum(),
        session_count: series.iter().map(|p| p.session_count).sum(),
    };

    let projects = db.report_breakdown(from_hour, to_hour, "project", project, None, None)?;
    let repos = db.report_breakdown(from_hour, to_hour, "repo", project, None, None)?;
    let work_projects = db
        .report_focus_breakdown(from_hour, to_hour, "project")
        .unwrap_or_default();
    let work_repos = db
        .report_focus_breakdown(from_hour, to_hour, "repo")
        .unwrap_or_default();

    let presence = build_presence_snapshot(
        db,
        from_hour,
        to_hour,
        now,
        totals.active_seconds,
        totals.raw_active_seconds,
    );

    Ok(ActivitySnapshot {
        range: range.to_string(),
        generated_at: now,
        from_ms,
        to_ms,
        status,
        totals,
        presence: Some(presence),
        series,
        projects,
        repos,
        work_projects,
        work_repos,
    })
}

struct Slot {
    start_ms: i64,
    label: String,
}

fn build_slots(start_date: NaiveDate, count: usize, hourly: bool, range: &str) -> Vec<Slot> {
    (0..count)
        .map(|i| {
            if hourly {
                let hour = i as u32;
                Slot {
                    start_ms: local_ms(start_date, hour),
                    label: format!("{:02}:00", hour),
                }
            } else {
                let date = start_date + Duration::days(i as i64);
                Slot {
                    start_ms: local_ms(date, 0),
                    label: if range == "month" {
                        date.format("%b %d").to_string()
                    } else {
                        date.format("%a %d").to_string()
                    },
                }
            }
        })
        .collect()
}

/// Map a UTC-aligned bucket start (seconds) into a dense local slot index.
fn slot_index(
    start_date: NaiveDate,
    hourly: bool,
    count: usize,
    bucket_start_secs: i64,
) -> Option<usize> {
    let dt = Local.timestamp_opt(bucket_start_secs, 0).single()?;
    let index = if hourly {
        if dt.date_naive() != start_date {
            return None;
        }
        dt.hour() as usize
    } else {
        let days = (dt.date_naive() - start_date).num_days();
        if days < 0 {
            return None;
        }
        days as usize
    };
    (index < count).then_some(index)
}

fn accumulate(point: &mut ActivitySnapshotPoint, row: &ActivitySeriesPoint) {
    point.active_seconds += row.active_seconds;
    point.file_events += row.file_events;
    point.git_events += row.git_events;
    point.command_events += row.command_events;
    point.session_count += row.session_count;
}

fn local_ms(date: NaiveDate, hour: u32) -> i64 {
    let naive = date
        .and_hms_opt(hour, 0, 0)
        .unwrap_or_else(|| date.and_hms_opt(0, 0, 0).expect("midnight is always valid"));
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|| Local.from_utc_datetime(&naive).timestamp_millis())
}

/// Kept public so callers can derive local day boundaries with the same rules.
pub fn local_day_start_ms(now: i64) -> i64 {
    let today = Local
        .timestamp_opt(now / 1000, 0)
        .single()
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| Local::now().date_naive());
    local_ms(today, 0)
}

pub const SNAPSHOT_RANGES: [&str; 3] = ["today", "week", "month"];

fn build_presence_snapshot(
    db: &ActivityDb,
    from_hour: i64,
    to_hour: i64,
    now: i64,
    event_seconds: i64,
    raw_event_seconds: i64,
) -> PresenceSnapshot {
    let totals =
        super::presence::presence_totals_for_range(db, from_hour, to_hour, now).unwrap_or_default();
    let (state, source) = db
        .latest_presence_state()
        .ok()
        .flatten()
        .map(|(s, src, _)| (s, src))
        .unwrap_or_else(|| ("unknown".to_string(), "none".to_string()));
    let focus = db.current_focus_interval().ok().flatten();
    let (category, focus_project, focus_workspace, focus_repo) = if let Some(ref f) = focus {
        (
            f.category.clone(),
            if f.project_name.is_empty() {
                None
            } else {
                Some(f.project_name.clone())
            },
            if f.workspace.is_empty() {
                None
            } else {
                Some(f.workspace.clone())
            },
            if f.repo.is_empty() {
                None
            } else {
                Some(f.repo.clone())
            },
        )
    } else {
        (String::new(), None, None, None)
    };
    let override_category = db
        .active_category_override(now, focus.as_ref())
        .ok()
        .flatten()
        .map(|o| o.category);
    let category_source = focus
        .as_ref()
        .map(|f| f.category_source.clone())
        .unwrap_or_default();
    let focused_work_seconds = totals.work_seconds;
    let active_work_seconds = totals.active_work_seconds;
    let event_work_seconds = event_seconds;
    let presence_only_seconds = (focused_work_seconds - event_work_seconds).max(0);
    let background_seconds = (event_work_seconds - focused_work_seconds).max(0);
    PresenceSnapshot {
        state,
        source,
        category,
        override_category,
        focus_project,
        focus_workspace,
        focus_repo,
        focus_app: focus.as_ref().and_then(|f| {
            if f.app.is_empty() {
                None
            } else {
                Some(f.app.clone())
            }
        }),
        focus_domain: focus.as_ref().and_then(|f| {
            if f.domain.is_empty() {
                None
            } else {
                Some(f.domain.clone())
            }
        }),
        focus_rule: focus.as_ref().and_then(|f| {
            if f.rule.is_empty() {
                None
            } else {
                Some(f.rule.clone())
            }
        }),
        category_source,
        totals,
        event_work_seconds,
        raw_event_seconds,
        active_work_seconds,
        focused_work_seconds,
        event_seconds: event_work_seconds,
        presence_only_seconds,
        background_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::db::ActivityDb;
    use crate::activity::model::Bucket;

    const DAY_MS: i64 = 86_400_000;

    fn temp_db() -> (tempfile::TempDir, ActivityDb) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("activity.db");
        let db = ActivityDb::open(&db_path).unwrap();
        (dir, db)
    }

    fn status() -> ActivityStatus {
        ActivityStatus::default()
    }

    fn hour_floor(ms: i64) -> i64 {
        (ms / 1000 / 3600) * 3600
    }

    fn add(
        db: &ActivityDb,
        ts_ms: i64,
        active: i64,
        files: i64,
        git: i64,
        commands: i64,
        sessions: i64,
    ) {
        db.add_bucket_active(&Bucket {
            project_id: "p1".to_string(),
            project_name: "kitchen".to_string(),
            hour: hour_floor(ts_ms),
            workspace: "ws".to_string(),
            repo: "repo".to_string(),
            active_seconds: active,
            file_events: files,
            git_events: git,
            command_events: commands,
            session_count: sessions,
        })
        .unwrap();
    }

    #[test]
    fn today_has_24_dense_hours_and_future_zeroes() {
        let (_dir, db) = temp_db();
        add(&db, crate::activity::db::now_ms(), 600, 3, 2, 1, 1);

        let snap =
            build_snapshot(&db, status(), "today", crate::activity::db::now_ms(), None).unwrap();

        assert_eq!(snap.range, "today");
        assert_eq!(snap.series.len(), 24);
        assert_eq!(snap.series[0].label, "00:00");
        assert_eq!(snap.series[23].label, "23:00");
        assert_eq!(snap.totals.active_seconds, 600);
        assert_eq!(snap.totals.file_events, 3);
        assert_eq!(snap.totals.git_events, 2);
        assert_eq!(snap.totals.command_events, 1);
        assert_eq!(snap.totals.session_count, 1);
        assert_eq!(snap.projects.len(), 1);
        assert_eq!(snap.projects[0].key, "kitchen");
        assert_eq!(snap.repos.len(), 1);
        assert_eq!(snap.repos[0].key, "repo");
    }

    #[test]
    fn week_and_month_are_dense_and_end_today() {
        let (_dir, db) = temp_db();
        let now = crate::activity::db::now_ms();
        add(&db, now - 2 * DAY_MS, 1200, 1, 0, 0, 1);

        let week = build_snapshot(&db, status(), "week", now, None).unwrap();
        assert_eq!(week.series.len(), 7);
        assert_eq!(week.totals.active_seconds, 1200);
        // start_date is six days ago, so two days ago is index 4.
        assert_eq!(week.series[4].active_seconds, 1200);

        let month = build_snapshot(&db, status(), "month", now, None).unwrap();
        assert_eq!(month.series.len(), 30);
        assert_eq!(month.totals.active_seconds, 1200);
        assert_eq!(month.series[27].active_seconds, 1200);
    }

    #[test]
    fn project_filter_applies_to_series_and_breakdowns() {
        let (_dir, db) = temp_db();
        let now = crate::activity::db::now_ms();
        add(&db, now, 600, 1, 0, 0, 1);
        db.add_bucket_active(&Bucket {
            project_id: "p2".to_string(),
            project_name: "other".to_string(),
            hour: hour_floor(now),
            workspace: String::new(),
            repo: String::new(),
            active_seconds: 300,
            file_events: 2,
            git_events: 0,
            command_events: 0,
            session_count: 0,
        })
        .unwrap();

        let snap = build_snapshot(&db, status(), "today", now, Some("kitchen")).unwrap();
        assert_eq!(snap.totals.active_seconds, 600);
        assert_eq!(snap.projects.len(), 1);
        assert_eq!(snap.projects[0].key, "kitchen");
    }

    #[test]
    fn invalid_range_is_rejected() {
        let (_dir, db) = temp_db();
        let err =
            build_snapshot(&db, status(), "year", crate::activity::db::now_ms(), None).unwrap_err();
        assert!(err.contains("--range"));
    }

    #[test]
    fn day_alias_maps_to_today() {
        let (_dir, db) = temp_db();
        let snap =
            build_snapshot(&db, status(), "day", crate::activity::db::now_ms(), None).unwrap();
        assert_eq!(snap.range, "today");
    }
}
