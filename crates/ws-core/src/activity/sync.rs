use serde_json::json;
use std::path::Path;

use crate::hub::HubClient;

use super::db::{now_ms, ActivityDb};
use super::model::ProjectState;
use super::ActivityConfig;

pub fn sync_all(db: &ActivityDb, cfg: &ActivityConfig, force: bool) -> Result<Vec<String>, String> {
    sync_projects(db, cfg, force, None)
}

pub fn sync_projects(db: &ActivityDb, cfg: &ActivityConfig, force: bool, project_filter: Option<&str>) -> Result<Vec<String>, String> {
    if !cfg.sync.enabled {
        return Ok(vec!["sync disabled in config".to_string()]);
    }
    let client = HubClient::default();
    if client.token.is_none() {
        return Ok(vec!["no hub token configured; skipping sync".to_string()]);
    }

    let device_id = db.device_id().unwrap_or_default();
    let device_name = db.get_meta("device_name").unwrap_or(None).unwrap_or_else(|| cfg.effective_device_name());
    let projects = db.list_projects().map_err(|e| e.to_string())?;
    let mut notes = Vec::new();

    for mut p in projects {
        if let Some(f) = project_filter {
            if p.project_id != f && p.name != f && p.path != f {
                continue;
            }
        }
        if !p.sync_enabled {
            continue;
        }
        if !force && p.backoff_until > now_ms() {
            continue;
        }

        let was_unlinked = p.hub_project.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true);

        let hub_project = match ensure_hub_project(&client, db, &mut p, was_unlinked) {
            Ok(h) => h,
            Err(e) => {
                let msg = e.to_string();
                notes.push(format!("{}: {}", p.name, msg));
                p.last_error = Some(msg.clone());
                p.backoff_until = next_backoff_until(&p, force);
                if is_not_found_error(&msg) && p.hub_auto_created {
                    p.sync_enabled = false;
                    p.disabled_reason = Some("auto-created hub project not found (deleted?)".to_string());
                }
                let _ = db.upsert_project(&p);
                continue;
            }
        };

        let (ns, name) = match HubClient::parse_project_identifier(&hub_project) {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!("{}: invalid hub project {}: {}", p.name, hub_project, e));
                p.last_error = Some(format!("invalid hub project: {}", e));
                p.sync_enabled = false;
                p.disabled_reason = Some("invalid hub project mapping".to_string());
                let _ = db.upsert_project(&p);
                continue;
            }
        };

        // first-link cutover: if never synced before, only push data from link hour onward
        if was_unlinked || p.last_synced_hour.is_none() {
            if let Some(linked_at) = p.linked_at {
                let cutoff_hour = hour_floor_secs(linked_at);
                let _ = db.conn.execute(
                    "UPDATE buckets SET dirty=0, synced_hour=hour WHERE project_id=?1 AND dirty=1 AND hour < ?2",
                    rusqlite::params![p.project_id, cutoff_hour],
                );
            }
        }

        // upsert device (best-effort)
        let _ = client.upsert_activity_device(&ns, &name, &device_id, &device_name);

        // push sanitized blueprint if project config changed since last push (best-effort)
        let _ = maybe_push_blueprint(&client, db, &ns, &name, &p);

        let buckets = match fetch_dirty_buckets(db, &p.project_id) {
            Ok(b) => b,
            Err(e) => { notes.push(format!("{}: {}", p.name, e)); continue; }
        };
        if buckets.is_empty() {
            continue;
        }

        let payload: Vec<serde_json::Value> = buckets.iter().map(|(hour, ws, repo, active, file_ev, git_ev, cmd_ev, sess)| {
            let hour_iso = chrono_like_iso(*hour);
            json!({
                "deviceId": device_id,
                "deviceName": device_name,
                "hourUtc": hour_iso,
                "repo": repo,
                "workspace": ws,
                "activeSeconds": active,
                "fileEvents": file_ev,
                "gitEvents": git_ev,
                "commandEvents": cmd_ev,
                "sessionCount": sess,
            })
        }).collect();

        // send in chunks to avoid huge payloads
        let mut ok_all = true;
        for chunk in payload.chunks(500) {
            match client.push_activity_buckets(&ns, &name, chunk) {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    notes.push(format!("{}: sync failed: {}", p.name, msg));
                    p.last_error = Some(msg.clone());
                    p.backoff_until = next_backoff_until(&p, force);
                    if is_not_found_error(&msg) && p.hub_auto_created {
                        p.sync_enabled = false;
                        p.disabled_reason = Some("auto-created hub project not found (deleted?)".to_string());
                    }
                    let _ = db.upsert_project(&p);
                    ok_all = false;
                    break;
                }
            }
        }
        if !ok_all { continue; }

        if let Err(e) = mark_buckets_synced(db, &p.project_id) {
            notes.push(format!("{}: pushed but failed to mark synced: {}", p.name, e));
        } else {
            p.last_error = None;
            p.backoff_until = 0;
            p.last_synced_hour = buckets.iter().map(|b| b.0).max().or(p.last_synced_hour);
            if p.linked_at.is_none() { p.linked_at = Some(now_ms()); }
            let _ = db.upsert_project(&p);
            notes.push(format!("{}: synced {} bucket(s)", p.name, buckets.len()));
        }
    }

    Ok(notes)
}

fn ensure_hub_project(client: &HubClient, db: &ActivityDb, p: &mut ProjectState, was_unlinked: bool) -> Result<String, String> {
    if let Some(hp) = &p.hub_project {
        if !hp.trim().is_empty() {
            // mapped (possibly from config); ensure device/blueprint sync can proceed
            return Ok(hp.trim().to_string());
        }
    }
    let username = client.current_username().map_err(|e| format!("cannot determine hub user: {}", e))?;
    let slug = sanitize_name(&p.name);
    let identifier = format!("{}/{}", username, slug);
    let (ns, name) = HubClient::parse_project_identifier(&identifier).map_err(|e| e.to_string())?;

    let sanitized = load_sanitized_blueprint(&p.path);
    let exists = client.get_project(&ns, &name).is_ok();
    if !exists {
        client.create_project(&ns, &name, Some("Auto-created by ws activity"), true, sanitized.as_deref())
            .map_err(|e| format!("failed to auto-create hub project: {}", e))?;
        if let Some(bp) = sanitized.as_deref() {
            let _ = client.push_revision(&ns, &name, bp, Some("Initial blueprint (activity auto-link)"), None);
            let meta_key = format!("blueprint_pushed_mtime:{}", p.project_id);
            let _ = db.set_meta(&meta_key, &now_ms().to_string());
        }
    } else if let Some(bp) = sanitized.as_deref() {
        let _ = client.push_revision(&ns, &name, bp, Some("Sync blueprint (activity link)"), None);
    }

    p.hub_project = Some(identifier.clone());
    // mark auto-created only when we generated mapping because none existed
    if was_unlinked {
        p.hub_auto_created = true;
    }
    if p.linked_at.is_none() { p.linked_at = Some(now_ms()); }
    db.upsert_project(p).map_err(|e| e.to_string())?;
    let _ = persist_hub_project_mapping(&p.path, &identifier);
    Ok(identifier)
}

fn maybe_push_blueprint(client: &HubClient, db: &ActivityDb, ns: &str, name: &str, p: &ProjectState) -> Result<(), String> {
    let root = Path::new(&p.path);
    let mut cfg_path = root.join("ws-project.yml");
    if !cfg_path.exists() { cfg_path = root.join("ws-project.yaml"); }
    if !cfg_path.exists() { return Ok(()); }
    let md = std::fs::metadata(&cfg_path).map_err(|e| e.to_string())?;
    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    let meta_key = format!("blueprint_pushed_mtime:{}", p.project_id);
    let last = db.get_meta_i64(&meta_key).unwrap_or(None).unwrap_or(0);
    if mtime <= last && last > 0 { return Ok(()); }
    if let Some(bp) = load_sanitized_blueprint(&p.path) {
        let _ = client.push_revision(ns, name, &bp, Some("Sync blueprint (activity)"), None);
    }
    let _ = db.set_meta(&meta_key, &mtime.to_string());
    Ok(())
}

fn load_sanitized_blueprint(project_root: &str) -> Option<String> {
    let root = std::path::Path::new(project_root);
    let mut cfg_path = root.join("ws-project.yml");
    if !cfg_path.exists() { cfg_path = root.join("ws-project.yaml"); }
    if !cfg_path.exists() { return None; }
    let config = crate::config::ConfigLoader::load_config(Some(&cfg_path), None, true).ok()?;
    let (sanitized_yaml, _secrets, _files, _private) = crate::config::ConfigLoader::classify_project_assets(&config);
    if sanitized_yaml.trim().is_empty() { None } else { Some(sanitized_yaml) }
}

fn persist_hub_project_mapping(project_root: &str, hub_project: &str) -> Result<(), String> {
    let root = std::path::Path::new(project_root);
    let mut cfg_path = root.join("ws-project.yml");
    if !cfg_path.exists() { cfg_path = root.join("ws-project.yaml"); }
    if !cfg_path.exists() { return Ok(()); }
    // targeted update via ConfigLoader helper if available, else minimal YAML edit preserving as much as possible
    if let Ok(content) = std::fs::read_to_string(&cfg_path) {
        // try to update existing hub_project line in activity block or append; avoid full serde_yaml round-trip when possible
        if let Some(updated) = update_yaml_scalar_in_block_pub(&content, "activity", "hub_project", hub_project) {
            std::fs::write(&cfg_path, updated).map_err(|e| e.to_string())?;
            return Ok(());
        }
        // fallback: parse/modify (may drop comments)
        let mut doc = serde_yaml::from_str::<serde_yaml::Value>(&content).map_err(|e| e.to_string())?;
        if let Some(map) = doc.as_mapping_mut() {
            let act_key = serde_yaml::Value::String("activity".to_string());
            let mut act_map = map.get(&act_key).and_then(|v| v.as_mapping()).cloned().unwrap_or_default();
            act_map.insert(serde_yaml::Value::String("hub_project".to_string()), serde_yaml::Value::String(hub_project.to_string()));
            map.insert(act_key, serde_yaml::Value::Mapping(act_map));
            let out = serde_yaml::to_string(&serde_yaml::Value::Mapping(map.clone())).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn update_yaml_scalar_in_block_pub(content: &str, block: &str, key: &str, value: &str) -> Option<String> {
    // simple line-based targeted update: find `block:` at column 0, then replace or insert `  key: value`
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);
    let mut i = 0;
    let mut found_block = false;
    let mut replaced = false;
    while i < lines.len() {
        let line = lines[i];
        out.push(line.to_string());
        if !found_block && (line == format!("{}:", block) || line.starts_with(&format!("{}:", block))) && !line.starts_with(' ') && !line.starts_with('\t') {
            found_block = true;
            i += 1;
            let mut inserted = false;
            // scan block
            while i < lines.len() {
                let l = lines[i];
                let is_top = !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.trim_start().starts_with('#');
                if is_top { break; }
                let trimmed = l.trim_start();
                if trimmed.starts_with(&format!("{}:", key)) {
                    let indent_len = l.len() - trimmed.len();
                    let indent = &l[..indent_len];
                    out.push(format!("{}{}: {}", indent, key, value));
                    replaced = true;
                    i += 1;
                    inserted = true;
                    continue;
                }
                out.push(l.to_string());
                i += 1;
            }
            if !inserted && !replaced {
                out.insert(out.len() - (lines.len().saturating_sub(i)), format!("  {}: {}", key, value));
                replaced = true;
            }
            continue;
        }
        i += 1;
    }
    if !found_block {
        // append block at end
        if !content.ends_with('\n') { out.push(String::new()); }
        out.push(format!("{}:", block));
        out.push(format!("  {}: {}", key, value));
        replaced = true;
    }
    if replaced { Some(out.join("\n") + if content.ends_with('\n') { "\n" } else { "" }) } else { None }
}

fn next_backoff_until(p: &ProjectState, force: bool) -> i64 {
    if force { return 0; }
    let now = now_ms();
    // exponential-ish: parse previous backoff interval from error state heuristic not stored; derive from last_error/backoff_until
    let prev_wait = if p.backoff_until > now { p.backoff_until - now } else { 0 };
    let base: i64 = 60_000; // 1m
    let next = if prev_wait <= 0 { 5 * 60_000 } else { (prev_wait.saturating_mul(2)).min(6 * 60 * 60_000).max(base) };
    now + next
}

fn is_not_found_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("404") || m.contains("not found") || m.contains("no such")
}

pub fn mark_all_dirty_for_project(db: &ActivityDb, project_id: &str) -> Result<(), String> {
    db.conn.execute("UPDATE buckets SET dirty=1 WHERE project_id=?1", rusqlite::params![project_id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn purge_remote(project_filter: Option<&str>) -> Result<usize, String> {
    let paths = super::ActivityPaths::discover();
    paths.ensure_dirs()?;
    let db = super::open_db(&paths)?;
    let client = HubClient::default();
    let projects = db.list_projects()?;
    let mut purged = 0;
    for p in projects {
        if let Some(name) = project_filter { if p.name != name && p.project_id != name { continue; } }
        if let Some(hp) = &p.hub_project {
            if let Ok((ns, n)) = HubClient::parse_project_identifier(hp) {
                if client.purge_activity(&ns, &n).is_ok() {
                    purged += 1;
                    let _ = mark_all_dirty_for_project(&db, &p.project_id);
                }
            }
        }
    }
    Ok(purged)
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() || lc == '-' || lc == '_' { out.push(lc); }
        else if lc.is_whitespace() || lc == '.' { out.push('-'); }
    }
    while out.contains("--") { out = out.replace("--", "-"); }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "project".to_string() } else { out }
}

fn hour_floor_secs(ms: i64) -> i64 { (ms / 1000 / 3600) * 3600 }

fn chrono_like_iso(hour_unix_secs: i64) -> String {
    let secs = hour_unix_secs;
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let hh = sod / 3600;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:00:00Z", y, m, d, hh)
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2)/153;
    let d = doy - (153*mp + 2)/5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

fn fetch_dirty_buckets(db: &ActivityDb, project_id: &str) -> Result<Vec<(i64, String, String, i64, i64, i64, i64, i64)>, String> {
    let mut stmt = db.conn.prepare(
        "SELECT hour, workspace, repo, active_seconds, file_events, git_events, command_events, session_count
         FROM buckets WHERE project_id=?1 AND dirty=1 ORDER BY hour ASC"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params![project_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1).unwrap_or_default(),
            r.get::<_, String>(2).unwrap_or_default(),
            r.get::<_, i64>(3).unwrap_or(0),
            r.get::<_, i64>(4).unwrap_or(0),
            r.get::<_, i64>(5).unwrap_or(0),
            r.get::<_, i64>(6).unwrap_or(0),
            r.get::<_, i64>(7).unwrap_or(0),
        ))
    }).map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}

fn mark_buckets_synced(db: &ActivityDb, project_id: &str) -> Result<(), String> {
    db.conn.execute("UPDATE buckets SET dirty=0, synced_hour=hour WHERE project_id=?1 AND dirty=1", rusqlite::params![project_id]).map_err(|e| e.to_string())?;
    Ok(())
}
