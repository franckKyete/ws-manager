use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use chrono::TimeZone;
use ws_core::activity::db::{now_ms, PRIVACY_NOTICE_VERSION};
use ws_core::activity::{open_db, ActivityConfig, ActivityPaths};
use ws_core::output::OutputHandler;

use crate::cli::{ActivityAction, ActivityDeviceAction, ActivityIdsAction, ActivityShellAction, ActivitySyncAction};

fn format_duration_human(secs: i64) -> String {
    let s = secs.max(0);
    let h = s / 3600;
    let m = (s % 3600) / 60;
    if h > 0 { format!("{}h {:02}m", h, m) } else { format!("{}m", m) }
}

fn parse_date_start_ms(s: &str) -> Result<i64, String> {
    let t = s.trim();
    // RFC3339
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Ok(dt.timestamp_millis());
    }
    // YYYY-MM-DD as local start
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        let dt = d.and_hms_opt(0, 0, 0).ok_or_else(|| "invalid date".to_string())?;
        let local = chrono::Local.from_local_datetime(&dt).single().unwrap_or_else(|| chrono::Utc.from_utc_datetime(&dt).with_timezone(&chrono::Local));
        return Ok(local.timestamp_millis());
    }
    Err(format!("Invalid date '{}'. Use YYYY-MM-DD or RFC3339.", t))
}

fn local_day_start_ms() -> i64 {
    let now = chrono::Local::now();
    now.date_naive().and_hms_opt(0,0,0)
        .and_then(|dt| chrono::Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now_ms() - (now_ms() % 86_400_000))
}

fn hour_floor_secs(ms: i64) -> i64 { (ms / 1000 / 3600) * 3600 }

pub fn execute_activity_status(json: bool) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let status = db.basic_status(&paths)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&status).map_err(|e| e.to_string())?);
        return Ok(());
    }

    println!("Activity status\n{}", "─".repeat(40));
    println!("  Enabled:          {}", if status.enabled { "yes" } else { "no" });
    println!("  Paused:           {}", if status.paused { "yes" } else { "no" });
    println!(
        "  Daemon:           {}{}",
        if status.daemon_running { "running" } else { "not running" },
        status.daemon_pid.map(|p| format!(" (pid {})", p)).unwrap_or_default()
    );
    println!("  DB:               {} ({} bytes)", status.db_path, status.db_size_bytes);
    println!("  Events:           {}", status.event_count);
    println!("  Buckets:          {}", status.bucket_count);
    println!(
        "  Device:           {} ({})",
        status.device_name,
        status.device_id.clone().unwrap_or_default()
    );
    let notice = if status.privacy_notice_pending {
        "pending".to_string()
    } else {
        status.privacy_notice_shown_at.map(|ts| format!("shown (unix ms {})", ts)).unwrap_or_else(|| "shown".to_string())
    };
    println!("  Privacy notice:   {}", notice);
    if let Some(ts) = status.last_event_at {
        let ago = (now_ms() - ts) / 1000;
        println!("  Last event:       {}s ago", ago.max(0));
    } else {
        println!("  Last event:       none");
    }
    println!(
        "  Watching:          {} of {} workspaces{}",
        status.watching_workspaces, status.known_workspaces,
        if status.watch_limited { " (inotify budget)" } else { "" }
    );
    // Presence
    let cfg = ActivityConfig::load().unwrap_or_default();
    let now = now_ms();
    let _ = ws_core::activity::presence::rollup_presence(&db, &cfg, now);
    let _ = ws_core::activity::focus::rollup_focus(&db, now);
    let today_start_ms = chrono::Local::now().date_naive().and_hms_opt(0,0,0).unwrap().and_local_timezone(chrono::Local).single().unwrap().timestamp_millis();
    let today_end_ms = (chrono::Local::now().date_naive() + chrono::Duration::days(1)).and_hms_opt(0,0,0).unwrap().and_local_timezone(chrono::Local).single().unwrap().timestamp_millis();
    let from_h = (today_start_ms / 1000 / 3600) * 3600;
    let to_h = ((today_end_ms + 999) / 1000 / 3600) * 3600;
    if let Ok(pt) = ws_core::activity::presence::presence_totals_for_range(&db, from_h, to_h, now) {
        let (pstate, psource, _) = db.latest_presence_state().ok().flatten().unwrap_or_else(|| ("unknown".to_string(), "none".to_string(), 0));
        println!("  Presence:          {} ({})  present {} · away {}", pstate, psource, format_duration_human(pt.present_seconds), format_duration_human(pt.away_seconds));
    }
    if !status.projects.is_empty() {
        println!("\nProjects:");
        for p in &status.projects {
            let hub = p.hub_project.clone().unwrap_or_else(|| "(not linked)".to_string());
            let sync = if p.sync_enabled { "sync on" } else { "sync off" };
            let short_id = if p.project_id.len() > 12 { &p.project_id[..12] } else { &p.project_id };
            println!("  - {} [{}] {} {}{}", p.name, short_id, hub, sync, p.disabled_reason.as_ref().map(|r| format!(" ({})", r)).unwrap_or_default());
        }
    }
    if status.privacy_notice_pending {
        OutputHandler::print_info("Privacy notice: sync sends aggregates only (time, counts, project/repo/workspace, device). No file paths, command names, arguments, or contents are synced.");
    }
    Ok(())
}

pub fn execute_activity_status_waybar() -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let cfg = ActivityConfig::load().unwrap_or_default();
    let _ = ws_core::activity::rollup::rollup_all(&db, &cfg);
    let status = db.basic_status(&paths)?;

    let now = now_ms();
    let _ = ws_core::activity::presence::rollup_presence(&db, &cfg, now);
    let _ = ws_core::activity::focus::rollup_focus(&db, now);
    let from_ms = local_day_start_ms();
    let from_hour = hour_floor_secs(from_ms);
    let to_hour = hour_floor_secs(now) + 3600;
    let rows = db.report_totals(from_hour, to_hour, None)?;
    let total_active: i64 = rows.iter().map(|r| r.active_seconds).sum();

    let week_from_ms = local_day_start_ms() - 6 * 86_400_000;
    let week_from_hour = hour_floor_secs(week_from_ms);
    let week_rows = db.report_totals(week_from_hour, to_hour, None)?;
    let week_active: i64 = week_rows.iter().map(|r| r.active_seconds).sum();

    let today_presence = ws_core::activity::presence::presence_totals_for_range(&db, from_hour, to_hour, now).unwrap_or_default();
    let week_presence = ws_core::activity::presence::presence_totals_for_range(&db, week_from_hour, to_hour, now).unwrap_or_default();
    let pstate = db.latest_presence_state().ok().flatten().map(|(s,_,_)| s).unwrap_or_else(|| "unknown".to_string());

    let mut classes: Vec<String> = Vec::new();
    let headline_secs = total_active; // raw event work (presence-confirmed sessions included)
    let (text, alt) = if !status.enabled {
        classes.push("disabled".to_string());
        ("󱎫 off".to_string(), "disabled".to_string())
    } else if status.paused {
        classes.push("paused".to_string());
        ("󱎫 paused".to_string(), "paused".to_string())
    } else {
        classes.push("active".to_string());
        (format!("󱎫 {}", format_duration_human(headline_secs)), "active".to_string())
    };

    if status.watch_limited { classes.push("watch-limited".to_string()); }

    let sync_errors = status.projects.iter().filter(|p| p.last_error.is_some() || p.backoff_until > now).count();
    if sync_errors > 0 { classes.push("sync-error".to_string()); } else { classes.push("synced".to_string()); }
    classes.push(format!("presence-{}", pstate));

    let last_event = status.last_event_at
        .map(|ts| {
            let ago = ((now - ts).max(0)) / 1000;
            if ago < 60 { format!("{}s ago", ago) }
            else if ago < 3600 { format!("{}m ago", ago / 60) }
            else { format!("{}h {}m ago", ago / 3600, (ago % 3600) / 60) }
        })
        .unwrap_or_else(|| "none".to_string());
    if let Some(ts) = status.last_event_at {
        if status.enabled && !status.paused && now - ts > 30 * 60 * 1000 {
            classes.push("stale".to_string());
        }
    }

    let mut tooltip = Vec::new();
    tooltip.push(format!("Work (event): {} · active {}", format_duration_human(total_active), format_duration_human(today_presence.active_work_seconds)));
    tooltip.push(format!("Focus work: {}  present {}", format_duration_human(today_presence.work_seconds), format_duration_human(today_presence.present_seconds)));
    tooltip.push(format!("Week work (event): {} · active {}", format_duration_human(week_active), format_duration_human(week_presence.active_work_seconds)));
    tooltip.push(format!("Presence: {}", pstate));
    tooltip.push(format!("Daemon: {}", if status.daemon_running { format!("running{}", status.daemon_pid.map(|p| format!(" (pid {})", p)).unwrap_or_default()) } else { "not running".to_string() }));
    tooltip.push(format!("Watching: {}/{} workspaces{}", status.watching_workspaces, status.known_workspaces, if status.watch_limited { " (limited)" } else { "" }));
    tooltip.push(format!("Last event: {}", last_event));
    if sync_errors > 0 {
        tooltip.push(format!("Sync: {} project(s) with errors/backoff", sync_errors));
    } else {
        tooltip.push("Sync: ok".to_string());
    }

    let pct = ((headline_secs.min(8 * 3600) as f64 / (8.0 * 3600.0)) * 100.0).round().clamp(0.0, 100.0) as u8;
    let out = ws_core::activity::model::WaybarStatus {
        text,
        alt,
        tooltip: tooltip.join("\n"),
        class: classes,
        percentage: Some(pct),
    };
    println!("{}", serde_json::to_string(&out).map_err(|e| e.to_string())?);
    Ok(())
}

pub fn execute_activity_snapshot(range: &str, project: Option<&str>, json: bool) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let cfg = ActivityConfig::load().unwrap_or_default();
    let _ = ws_core::activity::rollup::rollup_all(&db, &cfg);
    let _ = ws_core::activity::presence::rollup_presence(&db, &cfg, now_ms());
    let _ = ws_core::activity::focus::rollup_focus(&db, now_ms());
    let status = db.basic_status(&paths)?;
    let snapshot = ws_core::activity::snapshot::build_snapshot(&db, status, range, now_ms(), project)?;

    if json {
        println!("{}", serde_json::to_string(&snapshot).map_err(|e| e.to_string())?);
        return Ok(());
    }

    let t = &snapshot.totals;
    println!("Activity snapshot · {}\n{}", snapshot.range, "─".repeat(40));
    if let Some(p) = &snapshot.presence {
        println!(
            "  Work (event): {}   Active work: {}   Focus work: {}",
            format_duration_human(p.event_work_seconds),
            format_duration_human(p.active_work_seconds),
            format_duration_human(p.focused_work_seconds),
        );
        println!(
            "  Present:      {}   Presence-only: {}   Background: {}   Away: {}",
            format_duration_human(p.totals.present_seconds),
            format_duration_human(p.presence_only_seconds),
            format_duration_human(p.background_seconds),
            format_duration_human(p.totals.away_seconds),
        );
    } else {
        println!(
            "  Active:      {}   Wall clock: {}   Sessions: {}",
            format_duration_human(t.active_seconds),
            format_duration_human(t.wall_clock_seconds),
            t.session_count
        );
    }
    println!(
        "  Events:      {} total (file {} · git {} · command {})",
        t.file_events + t.git_events + t.command_events,
        t.file_events,
        t.git_events,
        t.command_events
    );
    println!(
        "  Daemon:      {}{}",
        if snapshot.status.daemon_running { "running" } else { "not running" },
        snapshot.status.daemon_pid.map(|p| format!(" (pid {})", p)).unwrap_or_default()
    );

    if let Some(p) = &snapshot.presence {
        let pt = &p.totals;
        println!(
            "  Presence:    {} present (active {} · idle {})  away {}  locked {}  asleep {}  [{}]",
            format_duration_human(pt.present_seconds),
            format_duration_human(pt.active_seconds),
            format_duration_human(pt.idle_seconds),
            format_duration_human(pt.away_seconds),
            format_duration_human(pt.locked_seconds),
            format_duration_human(pt.asleep_seconds),
            p.state,
        );
        println!(
            "  Focus work:  {} (active {})  leisure {}  neutral {}  unattributed {}{}",
            format_duration_human(pt.work_seconds),
            format_duration_human(pt.active_work_seconds),
            format_duration_human(pt.leisure_seconds),
            format_duration_human(pt.neutral_seconds),
            format_duration_human(pt.unattributed_seconds),
            p.override_category.as_ref().map(|c| format!("  [override: {}]", c)).unwrap_or_default()
        );
        if let Some(fp) = &p.focus_project {
            println!("  Focus:       {}{}{}", fp, p.focus_workspace.as_ref().map(|w| format!(" / {}", w)).unwrap_or_default(), p.focus_repo.as_ref().map(|r| format!(" / {}", r)).unwrap_or_default());
        }
    }

    if !snapshot.series.is_empty() {
        let values: Vec<i64> = snapshot.series.iter().map(|p| p.active_seconds).collect();
        println!("\n  {}", sparkline(&values));
        println!("  {}..{}", snapshot.series[0].label, snapshot.series[snapshot.series.len() - 1].label);
    }

    if !snapshot.projects.is_empty() {
        println!("\nTop projects:");
        for row in snapshot.projects.iter().take(6) {
            println!("  {:<24} {}", row.key, format_duration_human(row.active_seconds));
        }
    }
    if !snapshot.repos.is_empty() {
        println!("\nTop repos:");
        for row in snapshot.repos.iter().take(6) {
            println!("  {:<24} {}", row.key, format_duration_human(row.active_seconds));
        }
    }
    Ok(())
}

fn sparkline(values: &[i64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0);
    if max <= 0 {
        return "·".repeat(values.len());
    }
    values
        .iter()
        .map(|&v| {
            let idx = ((v.max(0) as f64 / max as f64) * (BARS.len() - 1) as f64).round() as usize;
            BARS[idx.min(BARS.len() - 1)]
        })
        .collect()
}

pub fn execute_activity_presence(range: &str, json: bool) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let cfg = ActivityConfig::load().unwrap_or_default();
    let now = now_ms();
    let _ = ws_core::activity::presence::rollup_presence(&db, &cfg, now);
    let _ = ws_core::activity::focus::rollup_focus(&db, now);

    let today = chrono::Local::now().date_naive();
    let (start_date, label) = match range {
        "today" | "day" | "" => (today, "Today"),
        "week" => (today - chrono::Duration::days(6), "Last 7 days"),
        "month" => (today - chrono::Duration::days(29), "Last 30 days"),
        _ => return Err("Invalid --range. Use 'today', 'week' or 'month'.".to_string()),
    };
    let from_ms = start_date.and_hms_opt(0,0,0).unwrap().and_local_timezone(chrono::Local).single().unwrap().timestamp_millis();
    let to_ms = (today + chrono::Duration::days(1)).and_hms_opt(0,0,0).unwrap().and_local_timezone(chrono::Local).single().unwrap().timestamp_millis();
    let from_hour = (from_ms / 1000 / 3600) * 3600;
    let to_hour = ((to_ms + 999) / 1000 / 3600) * 3600;

    let totals = ws_core::activity::presence::presence_totals_for_range(&db, from_hour, to_hour, now)?;
    let (state, source, _ts) = db.latest_presence_state()?.unwrap_or_else(|| ("unknown".to_string(), "none".to_string(), 0));

    if json {
        let out = serde_json::json!({
            "range": range,
            "from_ms": from_ms,
            "to_ms": to_ms,
            "state": state,
            "source": source,
            "totals": totals,
        });
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }

    println!("Presence ({})", label);
    println!("  State:    {} ({})", state, source);
    let focus = db.current_focus_interval().ok().flatten();
    let ovr = db.active_category_override(now, focus.as_ref()).ok().flatten();
    println!("  Present:      {}", format_duration_human(totals.present_seconds));
    println!("  Active:       {}", format_duration_human(totals.active_seconds));
    println!("  Idle:         {}", format_duration_human(totals.idle_seconds));
    println!("  Away:         {}", format_duration_human(totals.away_seconds));
    println!("  Locked:       {}", format_duration_human(totals.locked_seconds));
    println!("  Asleep:       {}", format_duration_human(totals.asleep_seconds));
    println!("  Focus work:   {} (active {})", format_duration_human(totals.work_seconds), format_duration_human(totals.active_work_seconds));
    println!("  Leisure:      {}", format_duration_human(totals.leisure_seconds));
    println!("  Neutral:      {}", format_duration_human(totals.neutral_seconds));
    println!("  Unattributed: {}", format_duration_human(totals.unattributed_seconds));
    if let Some(o) = ovr { println!("  Override:     {} ({})", o.category, o.scope); }
    if let Some(f) = focus {
        if !f.project_name.is_empty() || !f.category.is_empty() || !f.app.is_empty() {
            let ctx = if !f.domain.is_empty() { format!(" · {}", f.domain) } else if !f.app.is_empty() { format!(" · {}", f.app) } else { String::new() };
            println!("  Focus:        {}{}{}{} [{} / {}]", f.project_name, if f.workspace.is_empty() { String::new() } else { format!(" / {}", f.workspace) }, if f.repo.is_empty() { String::new() } else { format!(" / {}", f.repo) }, ctx, f.category, f.category_source);
        }
    }
    Ok(())
}

pub fn execute_activity_presence_set(state: &str) -> Result<(), String> {
    let valid = ["active", "idle", "away", "locked", "asleep"];
    if !valid.contains(&state) {
        return Err(format!("Invalid state '{}'. Use: {}", state, valid.join(", ")));
    }
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let record = serde_json::json!({
        "type": "presence_set",
        "state": state,
        "ts": now_ms(),
    });
    let line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::net::UnixStream;
        if let Ok(mut stream) = UnixStream::connect(&paths.socket_path) {
            let _ = stream.write_all(line.as_bytes());
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
            OutputHandler::print_success(&format!("Presence set to '{}'.", state));
            return Ok(());
        }
    }
    // Fallback: write directly to DB (daemon will reconcile on next start; may conflict with running monitor)
    let db = open_db(&paths)?;
    let now = now_ms();
    if let Ok(Some(cur)) = db.current_presence_interval() {
        let _ = db.close_presence_interval(cur.id, now);
    }
    let _ = db.open_presence_interval(now, state, "manual");
    OutputHandler::print_success(&format!("Presence set to '{}' (direct DB write; restart daemon if running for consistency).", state));
    Ok(())
}

pub fn execute_activity_presence_category(category: &str, duration: Option<&str>, current: bool) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let now = now_ms();

    if category == "auto" || category == "clear" {
        let n = db.clear_active_category_overrides(now)?;
        OutputHandler::print_success(&format!("Cleared {} active category override(s).", n));
        return Ok(());
    }

    let valid = ["work", "leisure", "neutral"];
    if !valid.contains(&category) {
        return Err(format!("Invalid category '{}'. Use: work, leisure, neutral, auto", category));
    }

    let end_ts = if let Some(dur) = duration {
        let secs = ws_core::utils::parse_duration(dur);
        if secs == 0 { return Err("Invalid --for duration".to_string()); }
        Some(now + (secs as i64 * 1000))
    } else {
        None
    };

    let scope = if current { "focus" } else { "global" };
    let focus = db.current_focus_interval().ok().flatten();
    // Clear existing active overrides first (simple behavior)
    let _ = db.clear_active_category_overrides(now);
    let mut ovr = ws_core::activity::model::CategoryOverride {
        start_ts: now, end_ts, category: category.to_string(), scope: scope.to_string(), ..Default::default()
    };
    if current {
        if let Some(f) = focus {
            ovr.match_project = f.project_id.clone();
            ovr.match_repo = f.repo.clone();
            ovr.match_workspace = f.workspace.clone();
            ovr.match_domain = f.domain.clone();
            ovr.match_app = f.app.clone();
        }
    }
    let _ = db.insert_category_override(&ovr)?;
    OutputHandler::print_success(&format!(
        "Category override: {} (scope={}, {})",
        category, scope,
        end_ts.map(|e| format!("until {}s from now", (e-now)/1000)).unwrap_or_else(|| "until cleared".to_string())
    ));
    Ok(())
}

pub fn execute_activity_presence_explain(json: bool) -> Result<(), String> {
    let cfg = ActivityConfig::load().unwrap_or_default();
    let val = ws_core::activity::focus::explain_current(&cfg)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?);
    } else {
        let w = &val["window"];
        let c = &val["context"];
        let r = &val["resolved"];
        let cat = &val["category"];
        println!("Focus explain");
        println!("  Window:   {} / {}  pid={}", w["class"].as_str().unwrap_or(""), w["initial_class"].as_str().unwrap_or(""), w["pid"]);
        println!("  Title:    {}", w["title"].as_str().unwrap_or(""));
        println!("  App:      {} ({})", c["app"].as_str().unwrap_or(""), c["app_kind"].as_str().unwrap_or(""));
        if !c["domain"].as_str().unwrap_or("").is_empty() { println!("  Domain:   {}", c["domain"].as_str().unwrap_or("")); }
        if !c["tmux_session"].as_str().unwrap_or("").is_empty() { println!("  Tmux:     {} / {}", c["tmux_session"].as_str().unwrap_or(""), c["tmux_window"].as_str().unwrap_or("")); }
        if !r["project_name"].as_str().unwrap_or("").is_empty() {
            println!("  Project:  {}{}{}", r["project_name"].as_str().unwrap_or(""), if r["workspace"].as_str().unwrap_or("").is_empty() { "".to_string() } else { format!(" / {}", r["workspace"].as_str().unwrap_or("")) }, if r["repo"].as_str().unwrap_or("").is_empty() { "".to_string() } else { format!(" / {}", r["repo"].as_str().unwrap_or("")) });
        }
        println!("  Source:   {} ({})", r["source"].as_str().unwrap_or(""), r["confidence"].as_str().unwrap_or(""));
        println!("  Category: {} [{}] rule={}", cat["value"].as_str().unwrap_or(""), cat["source"].as_str().unwrap_or(""), cat["rule"].as_str().unwrap_or("-"));
        if let Some(hits) = val["matches"].as_array() {
            if !hits.is_empty() {
                println!("  Matches:");
                for h in hits { println!("    - {} {} ({})", h["kind"].as_str().unwrap_or(""), h["name"].as_str().unwrap_or(""), h["category"].as_str().unwrap_or("")); }
            }
        }
    }
    Ok(())
}

pub fn execute_activity_series(
    day: bool, week: bool, month: bool,
    from: Option<&str>, to: Option<&str>,
    step: &str, project: Option<&str>, repo: Option<&str>, workspace: Option<&str>,
    group_by: &str, _json: bool,
) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let cfg = ActivityConfig::load().unwrap_or_default();
    let _ = ws_core::activity::rollup::rollup_all(&db, &cfg);

    let now = now_ms();
    let (from_ms, to_ms) = if let (Some(f), Some(t)) = (from, to) {
        (parse_date_start_ms(f)?, parse_date_start_ms(t)?)
    } else if let Some(f) = from {
        (parse_date_start_ms(f)?, now)
    } else if week {
        (local_day_start_ms() - 6 * 86_400_000, now)
    } else if month {
        (local_day_start_ms() - 29 * 86_400_000, now)
    } else {
        let _ = day;
        (local_day_start_ms(), now)
    };
    if to_ms < from_ms { return Err("Invalid range: --to must be >= --from".to_string()); }
    let step = match step {
        "day" | "hour" => step,
        _ => return Err("Invalid --step. Use 'hour' or 'day'.".to_string()),
    };
    let group_by_project = match group_by {
        "project" => true,
        "none" | "" => false,
        _ => return Err("Invalid --group-by. Use 'none' or 'project'.".to_string()),
    };

    let from_hour = hour_floor_secs(from_ms);
    let to_hour = hour_floor_secs(to_ms) + 3600;
    let series = db.report_series(from_hour, to_hour, step, project, repo, workspace, group_by_project)?;
    let out = serde_json::json!({
        "from_hour": from_hour,
        "to_hour": to_hour,
        "step": step,
        "group_by": if group_by_project { "project" } else { "none" },
        "series": series,
    });
    println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    Ok(())
}

pub fn execute_activity_dashboard() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let sibling = exe.with_file_name("ws-dashboard");
    let candidate = if sibling.is_file() { sibling } else { PathBuf::from("ws-dashboard") };
    let status = std::process::Command::new(&candidate)
        .status()
        .map_err(|e| format!("Failed to launch ws-dashboard ({}). Build it with `cargo build -p ws-dashboard` in the ws-manager workspace. Error: {}", candidate.display(), e))?;
    if !status.success() {
        return Err(format!("ws-dashboard exited with status {}", status));
    }
    Ok(())
}

pub fn execute_activity_enable(_yes: bool) -> Result<(), String> {
    let mut cfg = ActivityConfig::load().unwrap_or_default();
    cfg.enabled = true;
    cfg.paused = false;
    cfg.save()?;
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    if db.get_meta_i64("privacy_notice_shown_at")?.is_none() {
        OutputHandler::print_info("Activity tracking enabled. Aggregates may sync to wshub when configured (time/counts/project-repo-workspace/device only; no paths, command names, or contents).");
        db.mark_privacy_notice_shown(PRIVACY_NOTICE_VERSION)?;
    }
    OutputHandler::print_success("Activity collection enabled.");
    Ok(())
}
pub fn execute_activity_disable() -> Result<(), String> {
    let mut cfg = ActivityConfig::load().unwrap_or_default();
    cfg.enabled = false;
    cfg.save()?;
    OutputHandler::print_success("Activity collection disabled.");
    Ok(())
}
pub fn execute_activity_pause() -> Result<(), String> {
    let mut cfg = ActivityConfig::load().unwrap_or_default();
    cfg.paused = true;
    cfg.save()?;
    OutputHandler::print_success("Activity collection paused.");
    Ok(())
}
pub fn execute_activity_resume() -> Result<(), String> {
    let mut cfg = ActivityConfig::load().unwrap_or_default();
    cfg.paused = false;
    cfg.enabled = true;
    cfg.save()?;
    OutputHandler::print_success("Activity collection resumed.");
    Ok(())
}
pub fn execute_activity_device_rename(name: &str) -> Result<(), String> {
    let mut cfg = ActivityConfig::load().unwrap_or_default();
    let trimmed = name.trim().to_string();
    cfg.device_name = Some(trimmed.clone());
    cfg.save()?;
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    db.set_meta("device_name", &trimmed)?;
    OutputHandler::print_success(&format!("Device renamed to '{}'.", trimmed));
    Ok(())
}
pub fn execute_activity_clear(yes: bool) -> Result<(), String> {
    if !yes {
        eprint!("This will delete all local activity events and aggregates. Type 'clear' to confirm: ");
        io::stderr().flush().ok();
        let mut input = String::new();
        io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;
        if input.trim() != "clear" { return Err("Aborted.".to_string()); }
    }
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let device_id = db.device_id().ok();
    let device_name = db.get_meta("device_name").ok().flatten();
    let notice = db.get_meta_i64("privacy_notice_shown_at").ok().flatten();
    db.clear_all()?;
    if let Some(id) = device_id { let _ = db.set_meta("device_id", &id); }
    if let Some(name) = device_name { let _ = db.set_meta("device_name", &name); }
    if let Some(ts) = notice { let _ = db.set_meta("privacy_notice_shown_at", &ts.to_string()); let _ = db.set_meta("privacy_notice_version", PRIVACY_NOTICE_VERSION); }
    OutputHandler::print_success("Local activity data cleared.");
    Ok(())
}

pub fn execute_activity_prune(before: Option<&str>, dry_run: bool) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let cfg = ActivityConfig::load().unwrap_or_default();
    let cutoff_ms = if let Some(b) = before {
        parse_date_start_ms(b)?
    } else {
        now_ms() - (cfg.raw_retention_days as i64) * 86_400_000
    };
    if dry_run {
        let count: i64 = db.conn.query_row("SELECT COUNT(*) FROM events WHERE ts < ?1", [cutoff_ms], |r| r.get(0)).map_err(|e| e.to_string())?;
        OutputHandler::print_info(&format!("Would delete {} raw event(s).", count));
        return Ok(());
    }
    let deleted = db.prune_raw_events(cutoff_ms)?;
    OutputHandler::print_success(&format!("Pruned {} raw event(s). Aggregates retained.", deleted));
    Ok(())
}

pub fn execute_activity_report(action: &ActivityAction) -> Result<(), String> {
    let ActivityAction::Report { day, week, month, from, to, project, repo, workspace, device, tz, json, csv } = action else {
        return execute_activity_report_basic_range(None, None, None, None, None, false, false, None, None);
    };

    if let Some(tzname) = tz {
        if !tzname.trim().is_empty() && tzname.trim() != "local" {
            OutputHandler::print_info(&format!("Note: --tz '{}' is not applied to local CLI bucketing; using local time boundaries.", tzname));
        }
    }
    if let Some(d) = device {
        if !d.trim().is_empty() {
            OutputHandler::print_info("Note: local bucket reports are aggregated for this device; --device filtering is not applied to local bucketing.");
        }
    }

    let now = now_ms();
    let (from_ms, to_ms) = if let (Some(f), Some(t)) = (from.as_deref(), to.as_deref()) {
        let a = parse_date_start_ms(f)?;
        let b = parse_date_start_ms(t)?;
        (a, b)
    } else if let Some(f) = from.as_deref() {
        (parse_date_start_ms(f)?, now)
    } else if *week {
        (local_day_start_ms() - 6 * 86_400_000, now)
    } else if *month {
        (local_day_start_ms() - 29 * 86_400_000, now)
    } else {
        let _ = day;
        (local_day_start_ms(), now)
    };
    if to_ms < from_ms {
        return Err("Invalid range: --to must be >= --from".to_string());
    }

    execute_activity_report_basic_range(Some(from_ms), Some(to_ms), project.as_deref(), repo.as_deref(), workspace.as_deref(), *json, *csv, tz.as_deref(), device.as_deref())
}

fn execute_activity_report_basic_range(from_ms: Option<i64>, to_ms: Option<i64>, project: Option<&str>, repo: Option<&str>, workspace: Option<&str>, json: bool, csv: bool, _tz: Option<&str>, _device: Option<&str>) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    // ensure latest events are rolled up before reporting
    let cfg = ActivityConfig::load().unwrap_or_default();
    let _ = ws_core::activity::rollup::rollup_all(&db, &cfg);

    let now = now_ms();
    let from_ms = from_ms.unwrap_or_else(local_day_start_ms);
    let to_ms = to_ms.unwrap_or(now);
    let from_hour = hour_floor_secs(from_ms);
    let to_hour = hour_floor_secs(to_ms) + 3600;
    let mut rows = db.report_totals(from_hour, to_hour, project)?;
    if let Some(rf) = repo { rows.retain(|r| r.repo == rf); }
    if let Some(wf) = workspace { rows.retain(|r| r.workspace == wf); }

    let total_active: i64 = rows.iter().map(|r| r.active_seconds).sum();
    let wall_clock = db.report_wall_clock(from_hour, to_hour, project, repo, workspace)?;

    if json {
        let out = serde_json::json!({
            "from_hour": from_hour,
            "to_hour": to_hour,
            "total_active_seconds": total_active,
            "wall_clock_active_seconds": wall_clock,
            "rows": rows,
        });
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }
    if csv {
        println!("project_id,project_name,workspace,repo,active_seconds,file_events,git_events,command_events,session_count");
        for r in &rows {
            println!("{},{},{},{},{},{},{},{},{}", esc_csv(&r.project_id), esc_csv(&r.project_name), esc_csv(&r.workspace), esc_csv(&r.repo), r.active_seconds, r.file_events, r.git_events, r.command_events, r.session_count);
        }
        return Ok(());
    }
    if rows.is_empty() {
        OutputHandler::print_info("No activity buckets for the selected period.");
        return Ok(());
    }
    println!("Activity report (bucket totals)\n{}", "─".repeat(40));
    println!("  Total active:     {} ({}s)", format_duration_human(total_active), total_active);
    println!("  Wall-clock (est): {} ({}s)", format_duration_human(wall_clock), wall_clock);
    let mut by_project: BTreeMap<String, i64> = BTreeMap::new();
    for r in &rows { *by_project.entry(r.project_name.clone()).or_default() += r.active_seconds; }
    println!("\nBy project:");
    for (proj, secs) in by_project { println!("  {:<30} {}", proj, format_duration_human(secs)); }
    println!("\nBy workspace/repo:");
    for r in &rows {
        if r.active_seconds == 0 && r.file_events == 0 && r.git_events == 0 && r.command_events == 0 { continue; }
        println!("  {:<18} {:<18} {:<12}  files={} git={} cmd={}", r.project_name, if r.workspace.is_empty() { "(root)" } else { &r.workspace }, if r.repo.is_empty() { "(none)" } else { &r.repo }, r.file_events, r.git_events, r.command_events);
    }
    Ok(())
}


fn esc_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

pub fn execute_activity_export(raw: bool, from: Option<&str>, to: Option<&str>, json: bool, csv: bool) -> Result<(), String> {
    if !raw { return Err("`ws activity export` requires --raw.".to_string()); }
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let from_ms = from.map(parse_date_start_ms).transpose()?.unwrap_or(0);
    let to_ms = to.map(parse_date_start_ms).transpose()?.unwrap_or_else(now_ms);
    let rows = ws_core::activity::export::export_events(&db, from_ms, to_ms)?;
    if csv {
        println!("id,ts,kind,project_id,project_name,workspace,repo,rel_path,cmd,exit_code,duration_ms,count");
        for r in &rows {
            println!("{},{},{},{},{},{},{},{},{},{},{},{}", r.id, r.ts, esc_csv(&r.kind), esc_csv(&r.project_id), esc_csv(&r.project_name), esc_csv(&r.workspace), esc_csv(&r.repo), esc_csv(r.rel_path.as_deref().unwrap_or("")), esc_csv(r.cmd.as_deref().unwrap_or("")), r.exit_code.map(|v| v.to_string()).unwrap_or_default(), r.duration_ms.map(|v| v.to_string()).unwrap_or_default(), r.count);
        }
    } else {
        for r in &rows { println!("{}", serde_json::to_string(r).map_err(|e| e.to_string())?); }
    }
    let _ = json; // json lines default
    Ok(())
}

fn hook_block(shell: &str) -> String {
    let begin = "# >>> ws activity >>>";
    let end = "# <<< ws activity <<<";
    match shell {
        "zsh" => format!(
            "{begin}\n\
zmodload zsh/datetime 2>/dev/null\n\
__ws_activity_now_ms() {{ if [[ -n \"$EPOCHREALTIME\" ]]; then printf '%d' $((EPOCHREALTIME * 1000)); else printf '%s000' \"$(date +%s)\"; fi; }}\n\
__ws_activity_preexec() {{ command -v ws >/dev/null 2>&1 || return 0; __WS_ACTIVITY_CMD=\"$1\"; __WS_ACTIVITY_TS=$(__ws_activity_now_ms); __WS_ACTIVITY_START=$__WS_ACTIVITY_TS; }}\n\
__ws_activity_precmd() {{ local ec=$?; if [[ -n \"${{__WS_ACTIVITY_CMD+x}}\" ]]; then local now=$(__ws_activity_now_ms); local dur=$((now - __WS_ACTIVITY_START)); [[ $dur -lt 0 ]] && dur=0; (ws activity record --cmd \"$__WS_ACTIVITY_CMD\" --cwd \"$PWD\" --exit \"$ec\" --dur \"$dur\" --ts \"$__WS_ACTIVITY_TS\" >/dev/null 2>&1 &); unset __WS_ACTIVITY_CMD __WS_ACTIVITY_TS __WS_ACTIVITY_START; fi; }}\n\
typeset -ga preexec_functions precmd_functions\n\
[[ -z \"${{preexec_functions[(r)__ws_activity_preexec]}}\" ]] && preexec_functions+=(__ws_activity_preexec)\n\
[[ -z \"${{precmd_functions[(r)__ws_activity_precmd]}}\" ]] && precmd_functions+=(__ws_activity_precmd)\n\
{end}\n"
        ),
        "fish" => format!(
            "{begin}\n\
function __ws_activity_now_ms\n  set -l ms (date +%s%3N 2>/dev/null)\n  if string match -qr '^[0-9]+$' -- $ms; echo $ms; else; math (date +%s) \\* 1000; end\nend\n\
function __ws_activity_preexec --on-event fish_preexec\n  if not type -q ws; return; end\n  set -g __WS_ACTIVITY_CMD $argv[1]\n  set -g __WS_ACTIVITY_TS (__ws_activity_now_ms)\n  set -g __WS_ACTIVITY_START $__WS_ACTIVITY_TS\nend\n\
function __ws_activity_postexec --on-event fish_postexec\n  set -l ec $status\n  if set -q __WS_ACTIVITY_CMD\n    set -l now (__ws_activity_now_ms)\n    set -l dur (math $now - $__WS_ACTIVITY_START 2>/dev/null; or echo 0)\n    if test $dur -lt 0; set dur 0; end\n    ws activity record --cmd \"$__WS_ACTIVITY_CMD\" --cwd \"$PWD\" --exit $ec --dur $dur --ts $__WS_ACTIVITY_TS >/dev/null 2>&1 &\n    set -e __WS_ACTIVITY_CMD __WS_ACTIVITY_TS __WS_ACTIVITY_START\n  end\nend\n\
{end}\n"
        ),
        _ => format!(
            "{begin}\n\
__ws_activity_now_ms() {{ local ms; ms=$(date +%s%3N 2>/dev/null); if [[ \"$ms\" =~ ^[0-9]+$ ]]; then printf '%s' \"$ms\"; else printf '%s000' \"$(date +%s)\"; fi; }}\n\
__ws_activity_preexec() {{ command -v ws >/dev/null 2>&1 || return 0; __WS_ACTIVITY_CMD=\"$BASH_COMMAND\"; __WS_ACTIVITY_TS=$(__ws_activity_now_ms); __WS_ACTIVITY_START=$__WS_ACTIVITY_TS; }}\n\
__ws_activity_precmd() {{ local ec=$?; if [[ -n \"${{__WS_ACTIVITY_CMD+x}}\" ]]; then local now dur; now=$(__ws_activity_now_ms); dur=$((now - __WS_ACTIVITY_START)); [[ $dur -lt 0 ]] && dur=0; (ws activity record --cmd \"$__WS_ACTIVITY_CMD\" --cwd \"$PWD\" --exit \"$ec\" --dur \"$dur\" --ts \"$__WS_ACTIVITY_TS\" >/dev/null 2>&1 &); unset __WS_ACTIVITY_CMD __WS_ACTIVITY_TS __WS_ACTIVITY_START; fi; }}\n\
__ws_activity_debug_trap() {{ local prev=$?; [[ -n \"${{__WS_ACTIVITY_IN_PROMPT:-}}\" ]] && return $prev; __ws_activity_preexec; return $prev; }}\n\
__ws_activity_prompt_wrap() {{ __WS_ACTIVITY_IN_PROMPT=1; __ws_activity_precmd; unset __WS_ACTIVITY_IN_PROMPT; }}\n\
trap '__ws_activity_debug_trap' DEBUG\n\
case \";$PROMPT_COMMAND;\" in *\";__ws_activity_prompt_wrap;\"*) ;; *) PROMPT_COMMAND=\"__ws_activity_prompt_wrap${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\" ;; esac\n\
{end}\n"
        ),
    }
}

fn detect_shell() -> String {
    std::env::var("SHELL").ok().and_then(|s| PathBuf::from(s).file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "bash".to_string())
}

fn default_rc(shell: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match shell {
        "zsh" => home.join(".zshrc"),
        "fish" => home.join(".config/fish/config.fish"),
        _ => home.join(".bashrc"),
    }
}

pub fn execute_activity_shell_install(shell: Option<&str>, file: Option<&str>) -> Result<(), String> {
    let sh = shell.map(|s| s.to_string()).unwrap_or_else(detect_shell);
    let sh = if sh.contains("zsh") { "zsh" } else if sh.contains("fish") { "fish" } else { "bash" };
    let path = file.map(PathBuf::from).unwrap_or_else(|| default_rc(sh));
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let mut content = if path.is_file() { fs::read_to_string(&path).map_err(|e| e.to_string())? } else { String::new() };
    let begin = "# >>> ws activity >>>";
    let end = "# <<< ws activity <<<";
    if content.contains(begin) {
        // replace existing block
        if let (Some(s), Some(e)) = (content.find(begin), content.find(end)) {
            let e = e + end.len();
            content.replace_range(s..e, hook_block(sh).trim_end());
        }
    } else {
        if !content.is_empty() && !content.ends_with('\n') { content.push('\n'); }
        content.push_str(&hook_block(sh));
    }
    fs::write(&path, content).map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!("Installed activity shell hooks for {} in {}.", sh, path.display()));
    OutputHandler::print_info("Restart your shell (or source the rc file) for hooks to take effect. Requires `ws` on PATH.");
    Ok(())
}

pub fn execute_activity_shell_uninstall(shell: Option<&str>, file: Option<&str>) -> Result<(), String> {
    let sh = shell.map(|s| s.to_string()).unwrap_or_else(detect_shell);
    let sh = if sh.contains("zsh") { "zsh" } else if sh.contains("fish") { "fish" } else { "bash" };
    let path = file.map(PathBuf::from).unwrap_or_else(|| default_rc(sh));
    if !path.is_file() { OutputHandler::print_info("No shell rc file found."); return Ok(()); }
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let begin = "# >>> ws activity >>>";
    let end = "# <<< ws activity <<<";
    if let (Some(s), Some(e)) = (content.find(begin), content.find(end)) {
        let e = e + end.len();
        let mut newc = content.clone();
        newc.replace_range(s..e, "");
        fs::write(&path, newc.trim_end().to_string() + "\n").map_err(|e| e.to_string())?;
        OutputHandler::print_success(&format!("Removed activity shell hooks from {}.", path.display()));
    } else {
        OutputHandler::print_info("No ws activity hook block found.");
    }
    Ok(())
}


fn execute_activity_ids_list() -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let candidates = ws_core::activity::ids::list_candidates(&db)?;
    if candidates.is_empty() {
        OutputHandler::print_info("No projects tracked in activity DB.");
        return Ok(());
    }
    println!("Activity project IDs\n{}", "─".repeat(40));
    for c in &candidates {
        let canonical_events: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE project_id = ?1", [c.canonical.as_str()], |r| r.get(0)
        ).unwrap_or(0);
        println!("  {}  ({})", c.name, c.path);
        println!("    canonical: {}  events={}", c.canonical, canonical_events);
        if c.legacy != c.canonical && (c.legacy_events > 0 || c.legacy_buckets > 0 || c.legacy_state > 0) {
            println!("    legacy:    {}  events={} buckets={}", c.legacy, c.legacy_events, c.legacy_buckets);
        } else {
            println!("    legacy:    (none)");
        }
    }
    Ok(())
}

fn execute_activity_ids_migrate(dry_run: bool, keep_aggregates: bool, yes: bool) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let cfg = ActivityConfig::load().unwrap_or_default();
    let todo = ws_core::activity::ids::pending_candidates(&db)?;

    if todo.is_empty() {
        OutputHandler::print_info("No legacy path:* activity IDs found to migrate.");
        return Ok(());
    }

    println!("Legacy activity ID migration{}\n{}", if dry_run { " (dry run)" } else { "" }, "─".repeat(40));
    for m in &todo {
        println!("  {}:", m.name);
        println!("    {} -> {}", m.legacy, m.canonical);
        println!("    legacy events={} buckets={}", m.legacy_events, m.legacy_buckets);
        if !keep_aggregates {
            if let (Some(min_hour), Some(oldest_ev)) = (m.legacy_min_bucket_hour, m.legacy_oldest_event) {
                if min_hour * 1000 < oldest_ev {
                    println!("    warning: legacy buckets predate retained raw events; rebuild may be lossy, consider --keep-aggregates");
                }
            } else if m.legacy_min_bucket_hour.is_some() && m.legacy_events == 0 {
                println!("    warning: legacy buckets exist without raw events; rebuild not possible, use --keep-aggregates");
                if !dry_run { return Err(format!("Cannot rebuild '{}' without raw events; rerun with --keep-aggregates.", m.name)); }
            }
        }
    }

    if dry_run {
        OutputHandler::print_info("Dry run complete. No changes made.");
        return Ok(());
    }

    if !yes {
        eprint!("This will rewrite local activity data. Type 'migrate' to confirm: ");
        io::stderr().flush().ok();
        let mut input = String::new();
        io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;
        if input.trim() != "migrate" { return Err("Aborted.".to_string()); }
    }

    let (migrated, backup) = ws_core::activity::ids::migrate(&db, &cfg, keep_aggregates, &paths.db_path)?;
    if let Some(bak) = backup {
        OutputHandler::print_info(&format!("Backup created: {}", bak.display()));
    }
    for m in &migrated {
        OutputHandler::print_success(&format!("Migrated {} -> {}", m.legacy, m.canonical));
    }
    if migrated.is_empty() {
        OutputHandler::print_info("No legacy path:* activity IDs found to migrate.");
    }
    Ok(())
}


pub fn execute_activity_sync(project: Option<&str>, force: bool, action: Option<&ActivitySyncAction>) -> Result<(), String> {
    match action {
        Some(ActivitySyncAction::Enable { project }) => {
            set_project_sync(project.as_deref(), true)?;
            OutputHandler::print_success("Activity sync enabled.");
            Ok(())
        }
        Some(ActivitySyncAction::Disable { project }) => {
            set_project_sync(project.as_deref(), false)?;
            OutputHandler::print_success("Activity sync disabled.");
            Ok(())
        }
        Some(ActivitySyncAction::Purge { project, yes }) => {
            if !*yes { return Err("Pass --yes to confirm remote purge.".to_string()); }
            match ws_core::activity::sync::purge_remote(project.as_deref()) {
                Ok(purged) => {
                    OutputHandler::print_success(&format!("Requested remote activity purge for {} project(s). Local buckets marked dirty for re-sync if desired.", purged));
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        None => {
            let paths = ActivityPaths::discover();
            paths.ensure_dirs()?;
            let db = open_db(&paths)?;
            let cfg = ActivityConfig::load().unwrap_or_default();
            match ws_core::activity::sync::sync_projects(&db, &cfg, force, project) {
                Ok(notes) => {
                    if notes.is_empty() { OutputHandler::print_info("Nothing to sync."); }
                    else { for n in notes { OutputHandler::print_info(&n); } }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    }
}

fn set_project_sync(project: Option<&str>, enabled: bool) -> Result<(), String> {
    // Update local DB project state flags for all or matching project(s)
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = open_db(&paths)?;
    let projects = db.list_projects()?;
    for mut p in projects {
        if let Some(name) = project {
            if p.name != name && p.project_id != name { continue; }
        }
        p.sync_enabled = enabled;
        if enabled { p.disabled_reason = None; }
        db.upsert_project(&p)?;
    }
    Ok(())
}

pub fn execute_activity(action: Option<ActivityAction>) -> Result<(), String> {
    match action.unwrap_or(ActivityAction::Status { json: false, waybar: false }) {
        ActivityAction::Status { json, waybar } => {
            if waybar { execute_activity_status_waybar() } else { execute_activity_status(json) }
        }
        ActivityAction::Enable { yes } => execute_activity_enable(yes),
        ActivityAction::Disable => execute_activity_disable(),
        ActivityAction::Pause => execute_activity_pause(),
        ActivityAction::Resume => execute_activity_resume(),
        ActivityAction::Clear { yes } => execute_activity_clear(yes),
        ActivityAction::Prune { before, dry_run } => execute_activity_prune(before.as_deref(), dry_run),
        act @ ActivityAction::Report { .. } => execute_activity_report(&act),
        ActivityAction::Snapshot { range, project, json } => {
            execute_activity_snapshot(&range, project.as_deref(), json)
        }
        ActivityAction::Presence { range, json, action } => {
            match action {
                Some(crate::cli::ActivityPresenceAction::Set { state }) => execute_activity_presence_set(&state),
                Some(crate::cli::ActivityPresenceAction::Category { category, r#for, current }) => {
                    execute_activity_presence_category(&category, r#for.as_deref(), current)
                }
                Some(crate::cli::ActivityPresenceAction::Explain { json: ej }) => execute_activity_presence_explain(json || ej),
                None => execute_activity_presence(&range, json),
            }
        }
        ActivityAction::Series { day, week, month, from, to, step, project, repo, workspace, group_by, json } => {
            execute_activity_series(day, week, month, from.as_deref(), to.as_deref(), &step, project.as_deref(), repo.as_deref(), workspace.as_deref(), &group_by, json)
        }
        ActivityAction::Dashboard => execute_activity_dashboard(),
        ActivityAction::Sync { project, force, action } => execute_activity_sync(project.as_deref(), force, action.as_ref()),
        ActivityAction::Export { raw, from, to, json, csv } => execute_activity_export(raw, from.as_deref(), to.as_deref(), json, csv),
        ActivityAction::Shell { action } => match action {
            ActivityShellAction::Install { shell, file } => execute_activity_shell_install(shell.as_deref(), file.as_deref()),
            ActivityShellAction::Uninstall { shell, file } => execute_activity_shell_uninstall(shell.as_deref(), file.as_deref()),
        },
        ActivityAction::Device { action } => match action {
            ActivityDeviceAction::Rename { name } => execute_activity_device_rename(&name),
        },
        ActivityAction::Ids { action } => match action {
            ActivityIdsAction::List => execute_activity_ids_list(),
            ActivityIdsAction::Migrate { dry_run, keep_aggregates, yes } => execute_activity_ids_migrate(dry_run, keep_aggregates, yes),
        },
        ActivityAction::Record { cmd, cwd, exit, dur, ts } => execute_activity_record(cmd.as_deref(), cwd.as_deref(), exit, dur, ts),
    }
}

pub fn execute_activity_record(cmd: Option<&str>, cwd: Option<&str>, exit: Option<i64>, dur: Option<i64>, ts: Option<i64>) -> Result<(), String> {
    let paths = ActivityPaths::discover();
    paths.ensure_dirs()?;
    let cmd_raw = cmd.unwrap_or("");
    let cmd_name = cmd_raw
        .trim()
        .split_whitespace()
        .next()
        .and_then(|s| std::path::Path::new(s).file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    let record = serde_json::json!({
        "type": "command",
        "cmd": cmd_name,
        "cwd": cwd.unwrap_or(""),
        "exit": exit.unwrap_or(0),
        "dur_ms": dur.unwrap_or(0),
        "ts": ts.unwrap_or_else(now_ms),
    });
    let line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    // try unix socket first
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::net::UnixStream;
        if let Ok(mut stream) = UnixStream::connect(&paths.socket_path) {
            let _ = stream.write_all(line.as_bytes());
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
            return Ok(());
        }
    }
    // fallback to spool
    let mut f = fs::OpenOptions::new().create(true).append(true).open(&paths.spool_path).map_err(|e| e.to_string())?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = fs::set_permissions(&paths.spool_path, fs::Permissions::from_mode(0o600)); }
    writeln!(f, "{}", line).map_err(|e| e.to_string())?;
    Ok(())
}

