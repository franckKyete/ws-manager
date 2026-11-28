use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::db::ActivityDb;
use super::model::Bucket;
use super::ActivityConfig;

#[derive(Debug, Clone, Default)]
struct PendingSpan {
    project_id: String,
    project_name: String,
    workspace: String,
    repo: String,
    start_ts: i64,
    end_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionState {
    last_real_ts: i64,
    last_activity_ts: i64, // credited up to this timestamp
    session_start_ts: i64,
    session_credited_ms: i64,
    session_open: bool,
    cur_workspace: String,
    cur_repo: String,
    project_name: String,
    #[serde(default)]
    last_app: String,
    #[serde(default)]
    last_app_kind: String,
}

pub fn rollup_all(db: &ActivityDb, cfg: &ActivityConfig) -> Result<(), String> {
    let project_ids = db.list_project_ids()?;
    for pid in project_ids {
        let _ = rollup_project(db, cfg, &pid);
    }
    Ok(())
}

pub fn rerollup_project(
    db: &ActivityDb,
    cfg: &ActivityConfig,
    project_id: &str,
) -> Result<(), String> {
    db.set_rollup_state(project_id, 0, 0, None)?;
    rollup_project(db, cfg, project_id)
}

fn rollup_project(db: &ActivityDb, cfg: &ActivityConfig, project_id: &str) -> Result<(), String> {
    let idle_gap_ms = (cfg.idle_gap.max(1) * 1000) as i64;
    let session_cap_ms = (cfg.session_cap.max(60) * 1000) as i64;
    let tail_ms = (cfg.tail_credit * 1000) as i64;
    let heartbeat_window_ms = (cfg.heartbeat_window.max(1) * 1000) as i64;

    let (mut through_id, state_json) = db.get_rollup_state(project_id)?;
    let mut st: SessionState = state_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    loop {
        let batch = db.fetch_events_after(project_id, through_id, 1000)?;
        if batch.is_empty() {
            break;
        }
        let mut last_batch_id = through_id;
        let mut pending: HashMap<(i64, String, String), Bucket> = HashMap::new();
        let mut pending_spans: Vec<PendingSpan> = Vec::new();

        for (eid, ev) in &batch {
            last_batch_id = *eid;
            if st.project_name.is_empty() {
                st.project_name = ev.project_name.clone();
            }

            let is_real = ev.is_real();
            let eligible = if is_real {
                true
            } else {
                st.last_real_ts > 0
                    && ev.ts.saturating_sub(st.last_real_ts) <= heartbeat_window_ms
                    && st.session_open
            };

            // event counts attributed to event's own partition
            if eligible {
                let (fe, ge, ce) = match ev.category() {
                    "file" => (ev.count.max(1), 0, 0),
                    "git" => (0, ev.count.max(1), 0),
                    "command" => (0, 0, ev.count.max(1)),
                    _ => (0, 0, 0),
                };
                if fe != 0 || ge != 0 || ce != 0 {
                    let hour = hour_floor(ev.ts);
                    let key = (hour, ev.workspace.clone(), ev.repo.clone());
                    let b = pending.entry(key.clone()).or_insert_with(|| Bucket {
                        project_id: project_id.to_string(),
                        project_name: ev.project_name.clone(),
                        hour: key.0,
                        workspace: key.1.clone(),
                        repo: key.2.clone(),
                        active_seconds: 0,
                        file_events: 0,
                        git_events: 0,
                        command_events: 0,
                        session_count: 0,
                    });
                    b.file_events += fe;
                    b.git_events += ge;
                    b.command_events += ce;
                    if b.project_name.is_empty() {
                        b.project_name = ev.project_name.clone();
                    }
                }
            }

            if !eligible {
                continue;
            }

            // close session if gap exceeded or capped; try presence extension first
            if st.session_open {
                let gap = ev.ts.saturating_sub(st.last_activity_ts);
                if st.session_credited_ms >= session_cap_ms {
                    close_session(
                        &mut st,
                        tail_ms,
                        session_cap_ms,
                        &mut pending,
                        &mut pending_spans,
                        project_id,
                    );
                } else if gap > idle_gap_ms {
                    let extended = extend_session_with_presence(
                        db,
                        cfg,
                        project_id,
                        &mut st,
                        &mut pending,
                        &mut pending_spans,
                        ev.ts,
                        idle_gap_ms,
                        session_cap_ms,
                    );
                    let gap2 = ev.ts.saturating_sub(st.last_activity_ts);
                    if gap2 > idle_gap_ms && !extended {
                        close_session(
                            &mut st,
                            tail_ms,
                            session_cap_ms,
                            &mut pending,
                            &mut pending_spans,
                            project_id,
                        );
                    } else if gap2 > idle_gap_ms && extended {
                        // extended partially; close remainder
                        close_session(
                            &mut st,
                            tail_ms,
                            session_cap_ms,
                            &mut pending,
                            &mut pending_spans,
                            project_id,
                        );
                    }
                }
            }

            if !st.session_open {
                // new session starts at this event; partition defaults to event's own for initial state
                st.session_open = true;
                st.session_start_ts = ev.ts;
                st.last_activity_ts = ev.ts;
                st.session_credited_ms = 0;
                st.cur_workspace = ev.workspace.clone();
                st.cur_repo = ev.repo.clone();
                st.project_name = ev.project_name.clone();
                if let Ok(Some(fi)) = db.focus_interval_at(ev.ts) {
                    st.last_app = fi.app.clone();
                    st.last_app_kind = fi.app_kind.clone();
                }
                let hour = hour_floor(ev.ts);
                let key = (hour, st.cur_workspace.clone(), st.cur_repo.clone());
                let b = pending.entry(key.clone()).or_insert_with(|| Bucket {
                    project_id: project_id.to_string(),
                    project_name: st.project_name.clone(),
                    hour: key.0,
                    workspace: key.1.clone(),
                    repo: key.2.clone(),
                    active_seconds: 0,
                    file_events: 0,
                    git_events: 0,
                    command_events: 0,
                    session_count: 0,
                });
                b.session_count += 1;
            } else {
                // credit time from last_activity_ts to ev.ts to previous/current partition
                let mut target = ev.ts;
                // don't extend beyond idle gap from last credited point
                if target.saturating_sub(st.last_activity_ts) > idle_gap_ms {
                    target = st.last_activity_ts.saturating_add(idle_gap_ms);
                }
                let mut delta = target.saturating_sub(st.last_activity_ts);
                let remaining = session_cap_ms.saturating_sub(st.session_credited_ms);
                if delta > remaining {
                    delta = remaining;
                }
                if delta > 0 {
                    add_active_duration(
                        &mut pending,
                        &mut pending_spans,
                        project_id,
                        &st.project_name,
                        &st.cur_workspace,
                        &st.cur_repo,
                        st.last_activity_ts,
                        st.last_activity_ts + delta,
                    );
                    st.session_credited_ms += delta;
                    st.last_activity_ts += delta;
                }

                if st.session_credited_ms >= session_cap_ms {
                    close_session(
                        &mut st,
                        0,
                        session_cap_ms,
                        &mut pending,
                        &mut pending_spans,
                        project_id,
                    );
                    // start new session on this real event
                    if is_real {
                        st.session_open = true;
                        st.session_start_ts = ev.ts;
                        st.last_activity_ts = ev.ts;
                        st.session_credited_ms = 0;
                        st.cur_workspace = ev.workspace.clone();
                        st.cur_repo = ev.repo.clone();
                        st.project_name = ev.project_name.clone();
                        if let Ok(Some(fi)) = db.focus_interval_at(ev.ts) {
                            st.last_app = fi.app.clone();
                            st.last_app_kind = fi.app_kind.clone();
                        }
                        let hour = hour_floor(ev.ts);
                        let key = (hour, st.cur_workspace.clone(), st.cur_repo.clone());
                        let b = pending.entry(key.clone()).or_insert_with(|| Bucket {
                            project_id: project_id.to_string(),
                            project_name: st.project_name.clone(),
                            hour: key.0,
                            workspace: key.1.clone(),
                            repo: key.2.clone(),
                            active_seconds: 0,
                            file_events: 0,
                            git_events: 0,
                            command_events: 0,
                            session_count: 0,
                        });
                        b.session_count += 1;
                    }
                }
            }

            if is_real {
                st.last_real_ts = ev.ts;
                // subsequent time attributed to this event's partition
                st.cur_workspace = ev.workspace.clone();
                st.cur_repo = ev.repo.clone();
                st.project_name = ev.project_name.clone();
                if let Ok(Some(fi)) = db.focus_interval_at(ev.ts) {
                    st.last_app = fi.app.clone();
                    st.last_app_kind = fi.app_kind.clone();
                }
            }
            // heartbeats extend time (already credited above if within window) but keep partition
        }

        for b in pending.values() {
            let _ = db.add_bucket_active(b);
        }
        for s in &pending_spans {
            let _ = db.insert_event_span(
                &s.project_id,
                &s.project_name,
                &s.workspace,
                &s.repo,
                s.start_ts,
                s.end_ts,
            );
        }
        through_id = last_batch_id;
        let through_hour = hour_floor(if st.last_activity_ts > 0 {
            st.last_activity_ts
        } else {
            0
        });
        let state_str = serde_json::to_string(&st).ok();
        db.set_rollup_state(project_id, through_id, through_hour, state_str.as_deref())?;

        if batch.len() < 1000 {
            break;
        }
    }

    if st.session_open && cfg.presence.extend_sessions {
        let now = super::db::now_ms();
        let mut pending2: HashMap<(i64, String, String), Bucket> = HashMap::new();
        let mut spans2: Vec<PendingSpan> = Vec::new();
        let extended = extend_session_with_presence(
            db,
            cfg,
            project_id,
            &mut st,
            &mut pending2,
            &mut spans2,
            now,
            idle_gap_ms,
            session_cap_ms,
        );
        if extended || !pending2.is_empty() {
            for b in pending2.values() {
                let _ = db.add_bucket_active(b);
            }
            for s in &spans2 {
                let _ = db.insert_event_span(
                    &s.project_id,
                    &s.project_name,
                    &s.workspace,
                    &s.repo,
                    s.start_ts,
                    s.end_ts,
                );
            }
            let through_hour = hour_floor(st.last_activity_ts);
            let state_str = serde_json::to_string(&st).ok();
            let _ = db.set_rollup_state(project_id, through_id, through_hour, state_str.as_deref());
        }
    }

    Ok(())
}

fn extend_session_with_presence(
    db: &ActivityDb,
    cfg: &ActivityConfig,
    project_id: &str,
    st: &mut SessionState,
    pending: &mut HashMap<(i64, String, String), Bucket>,
    pending_spans: &mut Vec<PendingSpan>,
    next_event_ts: i64,
    idle_gap_ms: i64,
    session_cap_ms: i64,
) -> bool {
    if !cfg.presence.extend_sessions {
        return false;
    }
    if !st.session_open {
        return false;
    }
    let start = st.last_activity_ts;
    let end = next_event_ts;
    if end <= start {
        return false;
    }

    let focus = match db.focus_intervals_in_range(start, end) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let presence = match db.presence_intervals_in_range(start, end) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if focus.is_empty() || presence.is_empty() {
        return false;
    }

    let mut extended_any = false;
    let mut cursor = start;
    // Walk by focus interval boundaries to credit covered segments
    let mut boundaries: Vec<i64> = vec![start, end];
    for fi in &focus {
        boundaries.push(fi.start_ts.max(start).min(end));
        boundaries.push(fi.end_ts.unwrap_or(end).min(end).max(start));
    }
    for pi in &presence {
        boundaries.push(pi.start_ts.max(start).min(end));
        boundaries.push(pi.end_ts.unwrap_or(end).min(end).max(start));
    }
    boundaries.sort();
    boundaries.dedup();

    for w in boundaries.windows(2) {
        let a = w[0];
        let b = w[1];
        if b <= a {
            continue;
        }
        if a < cursor {
            continue;
        }
        // if there's an uncovered gap larger than idle_gap, stop extending
        if a.saturating_sub(st.last_activity_ts) > idle_gap_ms {
            break;
        }

        let present = presence.iter().any(|p| {
            matches!(p.state.as_str(), "active" | "idle")
                && p.start_ts < b
                && p.end_ts.unwrap_or(end) >= a
        });
        if !present {
            continue;
        }

        let covered = focus.iter().any(|fi| {
            if fi.start_ts >= b || fi.end_ts.unwrap_or(end) <= a {
                return false;
            }
            if fi.category != "work" {
                return false;
            }
            let same_project = !fi.project_id.is_empty() && fi.project_id == project_id;
            let same_repo = same_project
                && (fi.repo.is_empty() || st.cur_repo.is_empty() || fi.repo == st.cur_repo);
            // Only extend by work-capable app when the focus interval is not mapped to a different project.
            let unmapped_focus = fi.project_id.is_empty();
            let same_work_app = unmapped_focus
                && !fi.app.is_empty()
                && !st.last_app.is_empty()
                && fi.app.eq_ignore_ascii_case(&st.last_app)
                && matches!(fi.app_kind.as_str(), "terminal" | "editor" | "office");
            same_project || same_repo || same_work_app
        });
        if !covered {
            continue;
        }

        let mut delta = b.saturating_sub(st.last_activity_ts);
        let remaining = session_cap_ms.saturating_sub(st.session_credited_ms);
        if delta > remaining {
            delta = remaining;
        }
        if delta <= 0 {
            break;
        }
        add_active_duration(
            pending,
            pending_spans,
            project_id,
            &st.project_name,
            &st.cur_workspace,
            &st.cur_repo,
            st.last_activity_ts,
            st.last_activity_ts + delta,
        );
        st.session_credited_ms += delta;
        st.last_activity_ts += delta;
        cursor = st.last_activity_ts;
        extended_any = true;
        if st.session_credited_ms >= session_cap_ms {
            break;
        }
    }
    extended_any
}
fn close_session(
    st: &mut SessionState,
    tail_ms: i64,
    session_cap_ms: i64,
    pending: &mut HashMap<(i64, String, String), Bucket>,
    pending_spans: &mut Vec<PendingSpan>,
    project_id: &str,
) {
    if !st.session_open {
        return;
    }
    let remaining = session_cap_ms.saturating_sub(st.session_credited_ms);
    let tail = tail_ms.min(remaining).max(0);
    if tail > 0 {
        add_active_duration(
            pending,
            pending_spans,
            project_id,
            &st.project_name,
            &st.cur_workspace,
            &st.cur_repo,
            st.last_activity_ts,
            st.last_activity_ts + tail,
        );
        st.session_credited_ms += tail;
        st.last_activity_ts += tail;
    }
    st.session_open = false;
}

fn hour_floor(ts_ms: i64) -> i64 {
    if ts_ms <= 0 {
        return 0;
    }
    let secs = ts_ms / 1000;
    (secs / 3600) * 3600
}

fn add_active_duration(
    pending: &mut HashMap<(i64, String, String), Bucket>,
    pending_spans: &mut Vec<PendingSpan>,
    project_id: &str,
    project_name: &str,
    workspace: &str,
    repo: &str,
    start_ms: i64,
    end_ms: i64,
) {
    if end_ms <= start_ms {
        return;
    }
    pending_spans.push(PendingSpan {
        project_id: project_id.to_string(),
        project_name: project_name.to_string(),
        workspace: workspace.to_string(),
        repo: repo.to_string(),
        start_ts: start_ms,
        end_ts: end_ms,
    });
    let mut cur = start_ms;
    while cur < end_ms {
        let hour = hour_floor(cur);
        let next_hour_ms = (hour + 3600) * 1000;
        let seg_end = end_ms.min(next_hour_ms);
        let secs = ((seg_end - cur + 999) / 1000).max(0); // ceil
        if secs > 0 {
            let key = (hour, workspace.to_string(), repo.to_string());
            let b = pending.entry(key.clone()).or_insert_with(|| Bucket {
                project_id: project_id.to_string(),
                project_name: project_name.to_string(),
                hour: key.0,
                workspace: key.1.clone(),
                repo: key.2.clone(),
                active_seconds: 0,
                file_events: 0,
                git_events: 0,
                command_events: 0,
                session_count: 0,
            });
            b.active_seconds += secs;
            if b.project_name.is_empty() {
                b.project_name = project_name.to_string();
            }
        }
        cur = seg_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::config::{ActivityConfig, ActivitySyncConfig};
    use crate::activity::db::ActivityDb;
    use crate::activity::model::ActivityEvent;

    fn test_db() -> (tempfile::TempDir, ActivityDb) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.db");
        let db = ActivityDb::open(&path).unwrap();
        (dir, db)
    }

    fn cfg() -> ActivityConfig {
        ActivityConfig {
            enabled: true,
            paused: false,
            device_name: None,
            idle_gap: 300,
            session_cap: 14400,
            tail_credit: 60,
            heartbeat_interval: 60,
            heartbeat_window: 900,
            debounce: 2,
            raw_retention_days: 365,
            watch_reconcile: 60,
            sync: ActivitySyncConfig::default(),
            presence: Default::default(),
            exclude: vec![],
        }
    }

    fn ev(ts: i64, kind: &str, pid: &str, ws: &str, repo: &str) -> ActivityEvent {
        ActivityEvent {
            ts,
            kind: kind.to_string(),
            project_id: pid.to_string(),
            project_name: "proj".to_string(),
            workspace: ws.to_string(),
            repo: repo.to_string(),
            rel_path: None,
            cmd: None,
            exit_code: None,
            duration_ms: None,
            count: 1,
            meta: None,
        }
    }

    #[test]
    fn rollup_creates_bucket_and_credits_tail() {
        let (_d, db) = test_db();
        let c = cfg();
        // two events 10s apart in same hour -> should create session and credit active time including tail (capped by events/gap logic)
        db.insert_event(&ev(1_700_000_000_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(1_700_000_010_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        rollup_all(&db, &c).unwrap();
        let rows = db.report_totals(0, i64::MAX / 2, None).unwrap();
        assert!(!rows.is_empty());
        let total: i64 = rows.iter().map(|r| r.active_seconds).sum();
        assert!(total >= 10, "expected >=10s credited, got {}", total);
        let sessions: i64 = rows.iter().map(|r| r.session_count).sum();
        assert!(sessions >= 1);
    }

    #[test]
    fn idle_gap_starts_new_session() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 60;
        c.tail_credit = 0;
        db.insert_event(&ev(1_700_000_000_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        // > idle gap later
        db.insert_event(&ev(
            1_700_000_000_000 + 120_000,
            "file.modify",
            "p1",
            "ws1",
            "r1",
        ))
        .unwrap();
        rollup_all(&db, &c).unwrap();
        let rows = db.report_totals(0, i64::MAX / 2, None).unwrap();
        let sessions: i64 = rows.iter().map(|r| r.session_count).sum();
        assert!(sessions >= 2, "expected >=2 sessions, got {}", sessions);
    }

    fn seed_focus(
        db: &ActivityDb,
        start_ms: i64,
        end_ms: i64,
        project_id: &str,
        category: &str,
        app: &str,
        app_kind: &str,
    ) {
        use crate::activity::model::FocusInterval;
        let id = db
            .open_focus_interval(&FocusInterval {
                start_ts: start_ms,
                end_ts: None,
                project_id: project_id.to_string(),
                project_name: "proj".to_string(),
                workspace: "ws1".to_string(),
                repo: "r1".to_string(),
                source: "test".to_string(),
                confidence: "high".to_string(),
                category: category.to_string(),
                category_source: "test".to_string(),
                app: app.to_string(),
                app_kind: app_kind.to_string(),
                ..Default::default()
            })
            .unwrap();
        db.close_focus_interval(id, end_ms).unwrap();
    }

    fn seed_presence(db: &ActivityDb, start_ms: i64, end_ms: i64, state: &str) {
        let id = db.open_presence_interval(start_ms, state, "test").unwrap();
        db.close_presence_interval(id, end_ms).unwrap();
    }

    fn total_active(db: &ActivityDb) -> i64 {
        db.report_totals(0, i64::MAX / 2, None)
            .unwrap()
            .iter()
            .map(|r| r.active_seconds)
            .sum()
    }

    fn total_sessions(db: &ActivityDb) -> i64 {
        db.report_totals(0, i64::MAX / 2, None)
            .unwrap()
            .iter()
            .map(|r| r.session_count)
            .sum()
    }

    #[test]
    fn idle_presence_extends_work_session() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        c.presence.extend_sessions = true;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        // second event after idle gap; presence+focus cover the gap
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        seed_focus(&db, t0, t0 + 700_000, "p1", "work", "kitty", "terminal");
        seed_presence(&db, t0, t0 + 700_000, "idle");
        rollup_all(&db, &c).unwrap();
        let total = total_active(&db);
        assert!(
            total >= 540,
            "idle work focus should extend session, got {total}s"
        );
    }

    #[test]
    fn away_locked_asleep_do_not_extend() {
        for state in ["away", "locked", "asleep"] {
            let (_d, db) = test_db();
            let mut c = cfg();
            c.idle_gap = 300;
            c.tail_credit = 0;
            c.presence.extend_sessions = true;
            let t0 = 1_700_000_000_000i64;
            db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
                .unwrap();
            db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
                .unwrap();
            seed_focus(&db, t0, t0 + 700_000, "p1", "work", "kitty", "terminal");
            seed_presence(&db, t0, t0 + 700_000, state);
            rollup_all(&db, &c).unwrap();
            let total = total_active(&db);
            assert_eq!(
                total, 0,
                "{state} presence must not extend session, got {total}s"
            );
        }
    }

    #[test]
    fn leisure_focus_does_not_extend() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        c.presence.extend_sessions = true;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        seed_focus(&db, t0, t0 + 700_000, "", "leisure", "mpv", "media");
        seed_presence(&db, t0, t0 + 700_000, "active");
        rollup_all(&db, &c).unwrap();
        let total = total_active(&db);
        assert_eq!(
            total, 0,
            "leisure focus must not extend session, got {total}s"
        );
    }

    #[test]
    fn work_capable_app_extends_unmapped_focus() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        c.presence.extend_sessions = true;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        // unmapped focus, work-capable app: may extend
        seed_focus(&db, t0, t0 + 700_000, "", "work", "code", "editor");
        seed_presence(&db, t0, t0 + 700_000, "active");
        rollup_all(&db, &c).unwrap();
        let total = total_active(&db);
        assert!(
            total >= 540,
            "unmapped work-capable app should extend work session, got {total}s"
        );
    }

    #[test]
    fn different_project_focus_does_not_extend() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        c.presence.extend_sessions = true;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        // focus is mapped to a different project - must not extend p1
        seed_focus(&db, t0, t0 + 700_000, "other", "work", "code", "editor");
        seed_presence(&db, t0, t0 + 700_000, "active");
        rollup_all(&db, &c).unwrap();
        let total = total_active(&db);
        assert_eq!(
            total, 0,
            "different-project focus must not extend session, got {total}s"
        );
    }

    #[test]
    fn non_work_capable_app_does_not_extend_without_same_project() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        c.presence.extend_sessions = true;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        seed_focus(&db, t0, t0 + 700_000, "other", "work", "firefox", "browser");
        seed_presence(&db, t0, t0 + 700_000, "active");
        rollup_all(&db, &c).unwrap();
        let total = total_active(&db);
        assert_eq!(
            total, 0,
            "browser app should not extend by app match, got {total}s"
        );
    }

    #[test]
    fn extension_is_idempotent() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        c.presence.extend_sessions = true;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        seed_focus(&db, t0, t0 + 700_000, "p1", "work", "kitty", "terminal");
        seed_presence(&db, t0, t0 + 700_000, "idle");
        rollup_all(&db, &c).unwrap();
        let total1 = total_active(&db);
        let sessions1 = total_sessions(&db);
        rollup_all(&db, &c).unwrap();
        let total2 = total_active(&db);
        let sessions2 = total_sessions(&db);
        assert_eq!(
            total1, total2,
            "active seconds changed on second rollup: {total1} -> {total2}"
        );
        assert_eq!(
            sessions1, sessions2,
            "session count changed on second rollup: {sessions1} -> {sessions2}"
        );
    }

    #[test]
    fn overlapping_projects_are_deduped_in_union() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 3600;
        c.tail_credit = 0;
        let t0 = 1_700_000_000_000i64;
        // two projects active in same 10-minute window
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 600_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        let mut ev2 = ev(t0, "file.modify", "p2", "ws2", "r2");
        ev2.project_name = "proj2".to_string();
        db.insert_event(&ev2).unwrap();
        let mut ev2b = ev(t0 + 600_000, "file.modify", "p2", "ws2", "r2");
        ev2b.project_name = "proj2".to_string();
        db.insert_event(&ev2b).unwrap();
        rollup_all(&db, &c).unwrap();
        let raw: i64 = db
            .report_totals(0, i64::MAX / 2, None)
            .unwrap()
            .iter()
            .map(|r| r.active_seconds)
            .sum();
        let union = db.event_work_union(0, i64::MAX / 2, None).unwrap();
        assert!(raw >= 1200, "raw should count both sessions, got {raw}");
        assert!(
            (590..=610).contains(&union),
            "union should be ~600s, got {union}"
        );
        assert!(
            union < raw,
            "union should be less than raw when overlapping"
        );
    }

    #[test]
    fn rerollup_does_not_duplicate_spans() {
        let (_d, db) = test_db();
        let mut c = cfg();
        c.idle_gap = 300;
        c.tail_credit = 0;
        let t0 = 1_700_000_000_000i64;
        db.insert_event(&ev(t0, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        db.insert_event(&ev(t0 + 60_000, "file.modify", "p1", "ws1", "r1"))
            .unwrap();
        rollup_all(&db, &c).unwrap();
        let u1 = db.event_work_union(0, i64::MAX / 2, None).unwrap();
        // rerollup from scratch: clear rollup state and buckets/spans for clean replay
        db.conn.execute("DELETE FROM buckets", []).unwrap();
        db.conn.execute("DELETE FROM event_spans", []).unwrap();
        db.set_rollup_state("p1", 0, 0, None).unwrap();
        rollup_all(&db, &c).unwrap();
        let u2 = db.event_work_union(0, i64::MAX / 2, None).unwrap();
        assert_eq!(u1, u2, "rerollup produced different union: {u1} vs {u2}");
    }
}
