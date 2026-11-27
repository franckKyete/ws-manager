use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use super::model::{ActivityEvent, ActivityStatus, Bucket, ProjectState, ReportRow};
use super::ActivityPaths;

const SCHEMA_VERSION: i64 = 5;
pub const PRIVACY_NOTICE_VERSION: &str = "1";

pub struct ActivityDb {
    pub path: std::path::PathBuf,
    pub conn: Connection,
}

impl ActivityDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700)); }
        }
        let conn = Connection::open(path).map_err(|e| format!("Failed to open DB {}: {e}", path.display()))?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600)); }
        let mut db = Self { path: path.to_path_buf(), conn };
        db.init()?;
        Ok(db)
    }

    pub fn open_readonly(path: &Path) -> Result<Self, String> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| format!("Failed to open DB read-only {}: {e}", path.display()))?;
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA query_only=ON;")
            .map_err(|e| e.to_string())?;
        Ok(Self { path: path.to_path_buf(), conn })
    }

    fn init(&mut self) -> Result<(), String> {
        self.conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;",
        ).map_err(|e| e.to_string())?;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, kind TEXT NOT NULL, project_id TEXT NOT NULL, project_name TEXT NOT NULL, workspace TEXT NOT NULL DEFAULT '', repo TEXT NOT NULL DEFAULT '', rel_path TEXT, cmd TEXT, exit_code INTEGER, duration_ms INTEGER, count INTEGER NOT NULL DEFAULT 1, meta TEXT);
             CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
             CREATE INDEX IF NOT EXISTS events_proj_ts ON events(project_id, ts);
             CREATE INDEX IF NOT EXISTS events_proj_id ON events(project_id, id);
             CREATE TABLE IF NOT EXISTS buckets(project_id TEXT NOT NULL, project_name TEXT NOT NULL, hour INTEGER NOT NULL, workspace TEXT NOT NULL DEFAULT '', repo TEXT NOT NULL DEFAULT '', active_seconds INTEGER NOT NULL DEFAULT 0, file_events INTEGER NOT NULL DEFAULT 0, git_events INTEGER NOT NULL DEFAULT 0, command_events INTEGER NOT NULL DEFAULT 0, session_count INTEGER NOT NULL DEFAULT 0, dirty INTEGER NOT NULL DEFAULT 1, synced_hour INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(project_id, hour, workspace, repo));
             CREATE INDEX IF NOT EXISTS buckets_dirty ON buckets(dirty, hour);
             CREATE TABLE IF NOT EXISTS projects(project_id TEXT PRIMARY KEY, path TEXT NOT NULL, name TEXT NOT NULL, hub_project TEXT, hub_auto_created INTEGER NOT NULL DEFAULT 0, sync_enabled INTEGER NOT NULL DEFAULT 1, disabled_reason TEXT, linked_at INTEGER, last_synced_hour INTEGER, backoff_until INTEGER NOT NULL DEFAULT 0, last_error TEXT);
             CREATE TABLE IF NOT EXISTS rollup_state(project_id TEXT PRIMARY KEY, through_hour INTEGER NOT NULL, through_event_id INTEGER NOT NULL DEFAULT 0, session_state TEXT);
             CREATE TABLE IF NOT EXISTS presence_intervals(id INTEGER PRIMARY KEY, start_ts INTEGER NOT NULL, end_ts INTEGER, state TEXT NOT NULL, source TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS presence_intervals_start ON presence_intervals(start_ts);
             CREATE INDEX IF NOT EXISTS presence_intervals_end ON presence_intervals(end_ts);
             CREATE TABLE IF NOT EXISTS presence_buckets(hour INTEGER PRIMARY KEY, active_seconds INTEGER NOT NULL DEFAULT 0, idle_seconds INTEGER NOT NULL DEFAULT 0, away_seconds INTEGER NOT NULL DEFAULT 0, locked_seconds INTEGER NOT NULL DEFAULT 0, asleep_seconds INTEGER NOT NULL DEFAULT 0, work_seconds INTEGER NOT NULL DEFAULT 0, active_work_seconds INTEGER NOT NULL DEFAULT 0, leisure_seconds INTEGER NOT NULL DEFAULT 0, neutral_seconds INTEGER NOT NULL DEFAULT 0, unattributed_seconds INTEGER NOT NULL DEFAULT 0, unassigned_work_seconds INTEGER NOT NULL DEFAULT 0, dirty INTEGER NOT NULL DEFAULT 1, synced_hour INTEGER NOT NULL DEFAULT 0);
             CREATE INDEX IF NOT EXISTS presence_buckets_dirty ON presence_buckets(dirty, hour);
             CREATE TABLE IF NOT EXISTS focus_intervals(id INTEGER PRIMARY KEY, start_ts INTEGER NOT NULL, end_ts INTEGER, project_id TEXT NOT NULL DEFAULT '', project_name TEXT NOT NULL DEFAULT '', workspace TEXT NOT NULL DEFAULT '', repo TEXT NOT NULL DEFAULT '', source TEXT NOT NULL DEFAULT '', confidence TEXT NOT NULL DEFAULT '', category TEXT NOT NULL DEFAULT 'neutral', category_source TEXT NOT NULL DEFAULT 'default', app TEXT NOT NULL DEFAULT '', app_kind TEXT NOT NULL DEFAULT '', domain TEXT NOT NULL DEFAULT '', rule TEXT NOT NULL DEFAULT '', evidence TEXT NOT NULL DEFAULT '');
             CREATE INDEX IF NOT EXISTS focus_intervals_start ON focus_intervals(start_ts);
             CREATE INDEX IF NOT EXISTS focus_intervals_end ON focus_intervals(end_ts);
             CREATE TABLE IF NOT EXISTS focus_buckets(project_id TEXT NOT NULL, project_name TEXT NOT NULL, hour INTEGER NOT NULL, workspace TEXT NOT NULL DEFAULT '', repo TEXT NOT NULL DEFAULT '', work_seconds INTEGER NOT NULL DEFAULT 0, input_seconds INTEGER NOT NULL DEFAULT 0, dirty INTEGER NOT NULL DEFAULT 1, synced_hour INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(project_id, hour, workspace, repo));
             CREATE INDEX IF NOT EXISTS focus_buckets_dirty ON focus_buckets(dirty, hour);
             CREATE TABLE IF NOT EXISTS category_overrides(id INTEGER PRIMARY KEY, start_ts INTEGER NOT NULL, end_ts INTEGER, category TEXT NOT NULL, scope TEXT NOT NULL DEFAULT 'global', created_at INTEGER NOT NULL DEFAULT 0, match_project TEXT NOT NULL DEFAULT '', match_repo TEXT NOT NULL DEFAULT '', match_workspace TEXT NOT NULL DEFAULT '', match_domain TEXT NOT NULL DEFAULT '', match_app TEXT NOT NULL DEFAULT '');
             CREATE TABLE IF NOT EXISTS event_spans(project_id TEXT NOT NULL, project_name TEXT NOT NULL DEFAULT '', workspace TEXT NOT NULL DEFAULT '', repo TEXT NOT NULL DEFAULT '', start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL, PRIMARY KEY(project_id, workspace, repo, start_ts));
             CREATE INDEX IF NOT EXISTS event_spans_range ON event_spans(start_ts, end_ts);
             CREATE INDEX IF NOT EXISTS event_spans_proj ON event_spans(project_id, start_ts);",
        ).map_err(|e| e.to_string())?;
        let _ = self.conn.execute("ALTER TABLE rollup_state ADD COLUMN through_event_id INTEGER NOT NULL DEFAULT 0", []);
        let _ = self.conn.execute("ALTER TABLE rollup_state ADD COLUMN session_state TEXT", []);
        let _ = self.conn.execute("ALTER TABLE projects ADD COLUMN hub_auto_created INTEGER NOT NULL DEFAULT 0", []);
        for sql in [
            "ALTER TABLE focus_intervals ADD COLUMN app TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE focus_intervals ADD COLUMN app_kind TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE focus_intervals ADD COLUMN domain TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE focus_intervals ADD COLUMN rule TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE focus_intervals ADD COLUMN evidence TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE category_overrides ADD COLUMN match_project TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE category_overrides ADD COLUMN match_repo TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE category_overrides ADD COLUMN match_workspace TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE category_overrides ADD COLUMN match_domain TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE category_overrides ADD COLUMN match_app TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE presence_buckets ADD COLUMN active_work_seconds INTEGER NOT NULL DEFAULT 0",
        ] { let _ = self.conn.execute(sql, []); }
        let current_ver = self.get_meta_i64("schema_version")?.unwrap_or(0);
        if current_ver < SCHEMA_VERSION {
            self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
        }
        if self.get_meta("device_id")?.is_none() { self.set_meta("device_id", &uuid::Uuid::new_v4().to_string())?; }
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>, String> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).optional().map_err(|e| e.to_string())
    }
    pub fn get_meta_i64(&self, key: &str) -> Result<Option<i64>, String> { Ok(self.get_meta(key)?.and_then(|v| v.parse().ok())) }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn device_id(&self) -> Result<String, String> { Ok(self.get_meta("device_id")?.unwrap_or_default()) }

    pub fn insert_event(&self, ev: &ActivityEvent) -> Result<i64, String> {
        self.conn.execute(
            "INSERT INTO events(ts,kind,project_id,project_name,workspace,repo,rel_path,cmd,exit_code,duration_ms,count,meta) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![ev.ts,ev.kind,ev.project_id,ev.project_name,ev.workspace,ev.repo,ev.rel_path,ev.cmd,ev.exit_code,ev.duration_ms,ev.count,ev.meta],
        ).map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn upsert_project(&self, p: &ProjectState) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO projects(project_id,path,name,hub_project,hub_auto_created,sync_enabled,disabled_reason,linked_at,last_synced_hour,backoff_until,last_error)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(project_id) DO UPDATE SET path=excluded.path,name=excluded.name,
               hub_project=CASE WHEN excluded.hub_project IS NOT NULL AND excluded.hub_project != '' THEN excluded.hub_project ELSE projects.hub_project END,
               hub_auto_created=CASE WHEN excluded.hub_project IS NOT NULL AND excluded.hub_project != '' THEN excluded.hub_auto_created ELSE projects.hub_auto_created END,
               sync_enabled=excluded.sync_enabled,disabled_reason=excluded.disabled_reason,
               linked_at=COALESCE(projects.linked_at, excluded.linked_at),
               last_synced_hour=CASE WHEN excluded.last_synced_hour IS NOT NULL THEN excluded.last_synced_hour ELSE projects.last_synced_hour END,
               backoff_until=excluded.backoff_until,last_error=excluded.last_error",
            params![p.project_id,p.path,p.name,p.hub_project, if p.hub_auto_created {1i64} else {0}, if p.sync_enabled {1i64} else {0}, p.disabled_reason,p.linked_at,p.last_synced_hour,p.backoff_until,p.last_error],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_project(&self, project_id: &str) -> Result<Option<ProjectState>, String> {
        self.conn.query_row(
            "SELECT project_id,path,name,hub_project,hub_auto_created,sync_enabled,disabled_reason,linked_at,last_synced_hour,backoff_until,last_error FROM projects WHERE project_id=?1",
            params![project_id], |r| Ok(ProjectState {
                project_id: r.get(0)?, path: r.get(1)?, name: r.get(2)?, hub_project: r.get(3)?,
                hub_auto_created: r.get::<_, i64>(4).unwrap_or(0) != 0,
                sync_enabled: r.get::<_, i64>(5)? != 0, disabled_reason: r.get(6)?,
                linked_at: r.get(7)?, last_synced_hour: r.get(8)?, backoff_until: r.get::<_, i64>(9).unwrap_or(0), last_error: r.get(10)?,
            })
        ).optional().map_err(|e| e.to_string())
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectState>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT project_id,path,name,hub_project,hub_auto_created,sync_enabled,disabled_reason,linked_at,last_synced_hour,backoff_until,last_error FROM projects ORDER BY name"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(ProjectState {
            project_id: r.get(0)?, path: r.get(1)?, name: r.get(2)?, hub_project: r.get(3)?,
            hub_auto_created: r.get::<_, i64>(4).unwrap_or(0) != 0,
            sync_enabled: r.get::<_, i64>(5)? != 0, disabled_reason: r.get(6)?,
            linked_at: r.get(7)?, last_synced_hour: r.get(8)?, backoff_until: r.get::<_, i64>(9).unwrap_or(0), last_error: r.get(10)?,
        })).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn event_count(&self) -> Result<i64, String> { self.conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).map_err(|e| e.to_string()) }
    pub fn bucket_count(&self) -> Result<i64, String> { self.conn.query_row("SELECT COUNT(*) FROM buckets", [], |r| r.get(0)).map_err(|e| e.to_string()) }
    pub fn last_event_at(&self) -> Result<Option<i64>, String> {
        self.conn.query_row("SELECT MAX(ts) FROM events", [], |r| r.get(0)).optional().map_err(|e| e.to_string()).map(|o| o.flatten())
    }
    pub fn db_size_bytes(&self) -> u64 { fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) }

    pub fn mark_privacy_notice_shown(&self, version: &str) -> Result<(), String> {
        let now = now_ms();
        self.set_meta("privacy_notice_version", version)?;
        self.set_meta("privacy_notice_shown_at", &now.to_string())
    }

    pub fn upsert_bucket(&self, b: &Bucket, dirty: bool) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO buckets(project_id,project_name,hour,workspace,repo,active_seconds,file_events,git_events,command_events,session_count,dirty,synced_hour)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0)
             ON CONFLICT(project_id,hour,workspace,repo) DO UPDATE SET
               project_name=excluded.project_name,
               active_seconds=excluded.active_seconds,file_events=excluded.file_events,git_events=excluded.git_events,command_events=excluded.command_events,session_count=excluded.session_count,
               dirty=excluded.dirty",
            params![b.project_id,b.project_name,b.hour,b.workspace,b.repo,b.active_seconds,b.file_events,b.git_events,b.command_events,b.session_count, if dirty {1} else {0}],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn report_totals(&self, from_hour: i64, to_hour: i64, project: Option<&str>) -> Result<Vec<ReportRow>, String> {
        let mut sql = String::from(
            "SELECT project_id, project_name, workspace, repo, \
             SUM(active_seconds), SUM(file_events), SUM(git_events), SUM(command_events), SUM(session_count) \
             FROM buckets WHERE hour >= ?1 AND hour < ?2"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from_hour), Box::new(to_hour)];
        if let Some(p) = project {
            sql.push_str(" AND (project_name = ?3 OR project_id = ?3)");
            params_vec.push(Box::new(p.to_string()));
        }
        sql.push_str(" GROUP BY project_id, project_name, workspace, repo ORDER BY project_name, workspace, repo");
        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |r| Ok(ReportRow {
            project_id: r.get(0)?, project_name: r.get(1)?, workspace: r.get::<_, String>(2).unwrap_or_default(), repo: r.get::<_, String>(3).unwrap_or_default(),
            active_seconds: r.get::<_, i64>(4).unwrap_or(0), file_events: r.get::<_, i64>(5).unwrap_or(0), git_events: r.get::<_, i64>(6).unwrap_or(0), command_events: r.get::<_, i64>(7).unwrap_or(0), session_count: r.get::<_, i64>(8).unwrap_or(0),
        })).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn prune_raw_events(&self, before_ts: i64) -> Result<usize, String> {
        self.conn.execute("DELETE FROM events WHERE ts < ?1", params![before_ts]).map_err(|e| e.to_string())
    }

    pub fn clear_all(&self) -> Result<(), String> {
        self.conn.execute_batch("DELETE FROM events; DELETE FROM buckets; DELETE FROM projects; DELETE FROM rollup_state; DELETE FROM event_spans;").map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_project_ids(&self) -> Result<Vec<String>, String> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT project_id FROM events ORDER BY project_id").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn get_rollup_state(&self, project_id: &str) -> Result<(i64, Option<String>), String> {
        let res = self.conn.query_row(
            "SELECT through_event_id, session_state FROM rollup_state WHERE project_id=?1",
            params![project_id],
            |r| Ok((r.get::<_, i64>(0).unwrap_or(0), r.get::<_, Option<String>>(1).unwrap_or(None))),
        ).optional().map_err(|e| e.to_string())?;
        Ok(res.unwrap_or((0, None)))
    }

    pub fn set_rollup_state(&self, project_id: &str, through_event_id: i64, through_hour: i64, session_state: Option<&str>) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO rollup_state(project_id, through_hour, through_event_id, session_state) VALUES(?1,?2,?3,?4)
             ON CONFLICT(project_id) DO UPDATE SET through_hour=excluded.through_hour, through_event_id=excluded.through_event_id, session_state=excluded.session_state",
            params![project_id, through_hour, through_event_id, session_state],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn fetch_events_after(&self, project_id: &str, after_id: i64, limit: i64) -> Result<Vec<(i64, ActivityEvent)>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, kind, project_id, project_name, workspace, repo, rel_path, cmd, exit_code, duration_ms, count, meta
             FROM events WHERE project_id=?1 AND id > ?2 ORDER BY id ASC LIMIT ?3"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![project_id, after_id, limit], |r| {
            let id: i64 = r.get(0)?;
            let ev = ActivityEvent {
                ts: r.get(1)?,
                kind: r.get(2)?,
                project_id: r.get(3)?,
                project_name: r.get(4)?,
                workspace: r.get::<_, String>(5).unwrap_or_default(),
                repo: r.get::<_, String>(6).unwrap_or_default(),
                rel_path: r.get(7)?,
                cmd: r.get(8)?,
                exit_code: r.get(9)?,
                duration_ms: r.get(10)?,
                count: r.get::<_, i64>(11).unwrap_or(1),
                meta: r.get(12)?,
            };
            Ok((id, ev))
        }).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn add_bucket_active(&self, b: &Bucket) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO buckets(project_id,project_name,hour,workspace,repo,active_seconds,file_events,git_events,command_events,session_count,dirty,synced_hour)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,0)
             ON CONFLICT(project_id,hour,workspace,repo) DO UPDATE SET
               project_name=excluded.project_name,
               active_seconds=buckets.active_seconds + excluded.active_seconds,
               file_events=buckets.file_events + excluded.file_events,
               git_events=buckets.git_events + excluded.git_events,
               command_events=buckets.command_events + excluded.command_events,
               session_count=buckets.session_count + excluded.session_count,
               dirty=1",
            params![b.project_id,b.project_name,b.hour,b.workspace,b.repo,b.active_seconds,b.file_events,b.git_events,b.command_events,b.session_count],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Insert a credited active span, merging with an adjacent/overlapping span for the same key.
    pub fn insert_event_span(&self, project_id: &str, project_name: &str, workspace: &str, repo: &str, start_ts: i64, end_ts: i64) -> Result<(), String> {
        if end_ts <= start_ts { return Ok(()); }
        let existing: Option<(i64, i64, i64)> = self.conn.query_row(
            "SELECT rowid, start_ts, end_ts FROM event_spans WHERE project_id=?1 AND workspace=?2 AND repo=?3 AND start_ts <= ?5 AND end_ts >= ?4 ORDER BY start_ts DESC LIMIT 1",
            params![project_id, workspace, repo, start_ts, end_ts],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(|e| e.to_string())?;
        if let Some((rowid, s, e)) = existing {
            let ns = s.min(start_ts);
            let ne = e.max(end_ts);
            self.conn.execute("UPDATE event_spans SET start_ts=?1, end_ts=?2, project_name=CASE WHEN ?3!='' THEN ?3 ELSE project_name END WHERE rowid=?4",
                params![ns, ne, project_name, rowid]).map_err(|e| e.to_string())?;
            return Ok(());
        }
        // also merge into a span that starts exactly at end_ts (adjacent forward) - rare, but keep compact
        let forward: Option<(i64, i64, i64)> = self.conn.query_row(
            "SELECT rowid, start_ts, end_ts FROM event_spans WHERE project_id=?1 AND workspace=?2 AND repo=?3 AND start_ts=?4 LIMIT 1",
            params![project_id, workspace, repo, end_ts],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(|e| e.to_string())?;
        if let Some((rowid, _s, e)) = forward {
            self.conn.execute("UPDATE event_spans SET start_ts=?1, project_name=CASE WHEN ?2!='' THEN ?2 ELSE project_name END WHERE rowid=?3",
                params![start_ts, project_name, rowid]).map_err(|e| e.to_string())?;
            let _ = e;
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO event_spans(project_id,project_name,workspace,repo,start_ts,end_ts) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(project_id,workspace,repo,start_ts) DO UPDATE SET end_ts=MAX(event_spans.end_ts, excluded.end_ts), project_name=CASE WHEN excluded.project_name!='' THEN excluded.project_name ELSE event_spans.project_name END",
            params![project_id, project_name, workspace, repo, start_ts, end_ts],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn event_spans_in_range(&self, from_ms: i64, to_ms: i64) -> Result<Vec<(String, String, String, String, i64, i64)>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT project_id, project_name, workspace, repo, start_ts, end_ts FROM event_spans WHERE start_ts < ?2 AND end_ts > ?1 ORDER BY start_ts ASC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![from_ms, to_ms], |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1).unwrap_or_default(),
            r.get::<_, String>(2).unwrap_or_default(),
            r.get::<_, String>(3).unwrap_or_default(),
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn delete_event_spans_in_range(&self, from_ms: i64, to_ms: i64) -> Result<usize, String> {
        // delete spans fully inside; for overlapping spans, trim or split
        // Simple approach: load overlapping, delete them, reinsert trimmed parts outside range.
        let spans = self.event_spans_in_range(from_ms, to_ms)?;
        let mut deleted = 0usize;
        for (pid, pname, ws, repo, s, e) in spans {
            self.conn.execute(
                "DELETE FROM event_spans WHERE project_id=?1 AND workspace=?2 AND repo=?3 AND start_ts=?4",
                params![pid, ws, repo, s],
            ).map_err(|e| e.to_string())?;
            deleted += 1;
            if s < from_ms {
                let _ = self.conn.execute(
                    "INSERT OR REPLACE INTO event_spans(project_id,project_name,workspace,repo,start_ts,end_ts) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![pid, pname, ws, repo, s, from_ms],
                );
            }
            if e > to_ms {
                let _ = self.conn.execute(
                    "INSERT OR REPLACE INTO event_spans(project_id,project_name,workspace,repo,start_ts,end_ts) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![pid, pname, ws, repo, to_ms, e],
                );
            }
        }
        Ok(deleted)
    }


    pub fn report_wall_clock(&self, from_hour: i64, to_hour: i64, project: Option<&str>, repo: Option<&str>, workspace: Option<&str>) -> Result<i64, String> {
        let mut sql = String::from("SELECT hour, SUM(active_seconds) FROM buckets WHERE hour >= ?1 AND hour < ?2");
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from_hour), Box::new(to_hour)];
        let mut idx = 3;
        if let Some(p) = project { sql.push_str(&format!(" AND (project_name = ?{idx} OR project_id = ?{idx})")); params_vec.push(Box::new(p.to_string())); idx += 1; }
        if let Some(rf) = repo { sql.push_str(&format!(" AND repo = ?{idx}")); params_vec.push(Box::new(rf.to_string())); idx += 1; }
        if let Some(wf) = workspace { sql.push_str(&format!(" AND workspace = ?{idx}")); params_vec.push(Box::new(wf.to_string())); let _ = idx; }
        sql.push_str(" GROUP BY hour");
        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |r| Ok(r.get::<_, i64>(1).unwrap_or(0))).map_err(|e| e.to_string())?;
        let mut total = 0i64;
        for row in rows { let secs = row.map_err(|e| e.to_string())?; total += secs.min(3600).max(0); }
        Ok(total)
    }

    pub fn basic_status(&self, _paths: &ActivityPaths) -> Result<ActivityStatus, String> {
        let cfg = super::config::ActivityConfig::load().unwrap_or_default();
        let (daemon_running, daemon_pid) = crate::daemon::is_standalone_daemon_running();
        let notice_shown = self.get_meta_i64("privacy_notice_shown_at")?;
        let watching_workspaces = self.get_meta("watching_workspaces")?.and_then(|v| v.parse().ok()).unwrap_or(0usize);
        let known_workspaces = self.get_meta("known_workspaces")?.and_then(|v| v.parse().ok()).unwrap_or(0usize);
        let watch_limited = self.get_meta("watch_limited")?.map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
        Ok(ActivityStatus {
            enabled: cfg.enabled, paused: cfg.paused, daemon_running, daemon_pid,
            db_path: self.path.to_string_lossy().to_string(), db_size_bytes: self.db_size_bytes(),
            event_count: self.event_count()?, bucket_count: self.bucket_count()?,
            device_id: self.get_meta("device_id")?,
            device_name: self.get_meta("device_name")?.unwrap_or_else(|| cfg.effective_device_name()),
            privacy_notice_shown_at: notice_shown, privacy_notice_pending: notice_shown.is_none(),
            last_event_at: self.last_event_at()?,
            watching_workspaces, known_workspaces, watch_limited,
            projects: self.list_projects()?,
        })
    }

    pub fn report_series(
        &self,
        from_hour: i64,
        to_hour: i64,
        step: &str,
        project: Option<&str>,
        repo: Option<&str>,
        workspace: Option<&str>,
        group_by_project: bool,
    ) -> Result<Vec<super::model::ActivitySeriesPoint>, String> {
        let bucket_expr = if step == "day" { "(hour / 86400) * 86400" } else { "hour" };
        let mut sql = String::from("SELECT ");
        sql.push_str(bucket_expr);
        if group_by_project {
            sql.push_str(", project_id, project_name, '', ''");
        } else {
            sql.push_str(", '', '', '', ''");
        }
        sql.push_str(
            ", SUM(active_seconds), SUM(file_events), SUM(git_events), SUM(command_events), SUM(session_count) \
             FROM buckets WHERE hour >= ?1 AND hour < ?2",
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from_hour), Box::new(to_hour)];
        let mut idx = 3;
        if let Some(p) = project {
            sql.push_str(&format!(" AND (project_name = ?{idx} OR project_id = ?{idx})"));
            params_vec.push(Box::new(p.to_string()));
            idx += 1;
        }
        if let Some(rf) = repo {
            sql.push_str(&format!(" AND repo = ?{idx}"));
            params_vec.push(Box::new(rf.to_string()));
            idx += 1;
        }
        if let Some(wf) = workspace {
            sql.push_str(&format!(" AND workspace = ?{idx}"));
            params_vec.push(Box::new(wf.to_string()));
            let _ = idx;
        }
        sql.push_str(" GROUP BY ");
        sql.push_str(bucket_expr);
        if group_by_project {
            sql.push_str(", project_id, project_name");
        }
        sql.push_str(" ORDER BY 1 ASC");
        if group_by_project {
            sql.push_str(", project_name ASC");
        }

        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let mut out: Vec<super::model::ActivitySeriesPoint> = stmt.query_map(params_refs.as_slice(), |r| {
            let project_id: String = r.get::<_, String>(1).unwrap_or_default();
            let project_name: String = r.get::<_, String>(2).unwrap_or_default();
            Ok(super::model::ActivitySeriesPoint {
                bucket_start: r.get(0)?,
                project_id: if group_by_project && !project_id.is_empty() { Some(project_id) } else { None },
                project_name: if group_by_project && !project_name.is_empty() { Some(project_name) } else { None },
                workspace: None,
                repo: None,
                active_seconds: r.get::<_, i64>(5).unwrap_or(0),
                file_events: r.get::<_, i64>(6).unwrap_or(0),
                git_events: r.get::<_, i64>(7).unwrap_or(0),
                command_events: r.get::<_, i64>(8).unwrap_or(0),
                session_count: r.get::<_, i64>(9).unwrap_or(0),
            })
        }).map_err(|e| e.to_string())?.map(|r| r.map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>()?;

        // Dedupe active_seconds across overlapping sessions. Legacy hours without spans fall back to raw.
        if repo.is_none() && workspace.is_none() {
            let union_by_hour = self.event_work_union_by_hour(from_hour, to_hour, project).unwrap_or_default();
            if !group_by_project {
                let mut by_bucket: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
                for (&h, &s) in &union_by_hour {
                    let b = if step == "day" { (h / 86400) * 86400 } else { h };
                    *by_bucket.entry(b).or_insert(0) += s;
                }
                for row in &mut out {
                    if let Some(s) = by_bucket.get(&row.bucket_start) { row.active_seconds = *s; }
                    else { row.active_seconds = 0; }
                }
            } else {
                // proportional allocation by raw share within bucket
                let mut bucket_union: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
                let mut bucket_raw: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
                for (&h, &s) in &union_by_hour { let b = if step == "day" { (h / 86400) * 86400 } else { h }; *bucket_union.entry(b).or_insert(0) += s; }
                for row in &out { *bucket_raw.entry(row.bucket_start).or_insert(0) += row.active_seconds.max(0); }
                for row in &mut out {
                    let u = bucket_union.get(&row.bucket_start).copied().unwrap_or(0);
                    let raw_sum = bucket_raw.get(&row.bucket_start).copied().unwrap_or(0);
                    row.active_seconds = if raw_sum > 0 { ((row.active_seconds.max(0) as f64 / raw_sum as f64) * u as f64).round() as i64 } else { 0 };
                }
                // reconcile rounding to exact union
                let mut bucket_alloc: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
                for row in &out { *bucket_alloc.entry(row.bucket_start).or_insert(0) += row.active_seconds; }
                for (b, u) in &bucket_union {
                    let alloc = bucket_alloc.get(b).copied().unwrap_or(0);
                    let mut diff = *u - alloc;
                    for row in out.iter_mut().filter(|r| r.bucket_start == *b) {
                        if diff == 0 { break; }
                        let adj = if diff > 0 { 1 } else { -1 };
                        if row.active_seconds + adj >= 0 { row.active_seconds += adj; diff -= adj; }
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn report_breakdown(
        &self,
        from_hour: i64,
        to_hour: i64,
        group_by: &str,
        project: Option<&str>,
        repo: Option<&str>,
        workspace: Option<&str>,
    ) -> Result<Vec<super::model::ActivityBreakdownRow>, String> {
        let (key_expr, group_expr) = match group_by {
            "workspace" => ("workspace", "workspace"),
            "repo" => ("repo", "repo"),
            _ => ("project_name", "project_id, project_name"),
        };
        let mut sql = format!(
            "SELECT {key_expr}, project_id, project_name, workspace, repo, \
             SUM(active_seconds), SUM(file_events), SUM(git_events), SUM(command_events), SUM(session_count) \
             FROM buckets WHERE hour >= ?1 AND hour < ?2"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from_hour), Box::new(to_hour)];
        let mut idx = 3;
        if let Some(p) = project {
            sql.push_str(&format!(" AND (project_name = ?{idx} OR project_id = ?{idx})"));
            params_vec.push(Box::new(p.to_string()));
            idx += 1;
        }
        if let Some(rf) = repo {
            sql.push_str(&format!(" AND repo = ?{idx}"));
            params_vec.push(Box::new(rf.to_string()));
            idx += 1;
        }
        if let Some(wf) = workspace {
            sql.push_str(&format!(" AND workspace = ?{idx}"));
            params_vec.push(Box::new(wf.to_string()));
            let _ = idx;
        }
        sql.push_str(&format!(" GROUP BY {group_expr} ORDER BY SUM(active_seconds) DESC, 1 ASC"));

        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let mut out: Vec<super::model::ActivityBreakdownRow> = stmt.query_map(params_refs.as_slice(), |r| {
            let key: String = r.get::<_, String>(0).unwrap_or_default();
            let project_id: String = r.get::<_, String>(1).unwrap_or_default();
            let project_name: String = r.get::<_, String>(2).unwrap_or_default();
            let ws: String = r.get::<_, String>(3).unwrap_or_default();
            let rp: String = r.get::<_, String>(4).unwrap_or_default();
            Ok(super::model::ActivityBreakdownRow {
                key: if key.is_empty() { match group_by { "workspace" => "(root)".to_string(), "repo" => "(none)".to_string(), _ => project_name.clone() } } else { key },
                project_id: if group_by == "project" { Some(project_id) } else { None },
                project_name: if group_by == "project" { Some(project_name) } else { None },
                workspace: if group_by == "workspace" { Some(ws) } else { None },
                repo: if group_by == "repo" { Some(rp) } else { None },
                active_seconds: r.get::<_, i64>(5).unwrap_or(0),
                file_events: r.get::<_, i64>(6).unwrap_or(0),
                git_events: r.get::<_, i64>(7).unwrap_or(0),
                command_events: r.get::<_, i64>(8).unwrap_or(0),
                session_count: r.get::<_, i64>(9).unwrap_or(0),
            })
        }).map_err(|e| e.to_string())?.map(|r| r.map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>()?;

        if repo.is_none() && workspace.is_none() {
            let dedup = self.event_work_report_totals(from_hour, to_hour, project).unwrap_or_default();
            let mut alloc: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
            for r in &dedup {
                let k = match group_by { "workspace" => if r.workspace.is_empty() { "(root)".to_string() } else { r.workspace.clone() }, "repo" => if r.repo.is_empty() { "(none)".to_string() } else { r.repo.clone() }, _ => r.project_name.clone() };
                *alloc.entry(k).or_insert(0) += r.active_seconds;
            }
            for row in &mut out { row.active_seconds = alloc.get(&row.key).copied().unwrap_or(0); }
            out.sort_by(|a,b| b.active_seconds.cmp(&a.active_seconds).then(a.key.cmp(&b.key)));
        }
        Ok(out)
    }

    // --- Event-work union (deduped spans) ---

    fn raw_hour_sums(&self, from_hour: i64, to_hour: i64, project: Option<&str>) -> Result<std::collections::BTreeMap<i64, i64>, String> {
        let mut sql = String::from("SELECT hour, SUM(active_seconds) FROM buckets WHERE hour >= ?1 AND hour < ?2");
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from_hour), Box::new(to_hour)];
        if let Some(p) = project { sql.push_str(" AND (project_name = ?3 OR project_id = ?3)"); params_vec.push(Box::new(p.to_string())); }
        sql.push_str(" GROUP BY hour");
        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1).unwrap_or(0)))).map_err(|e| e.to_string())?;
        let mut out = std::collections::BTreeMap::new();
        for r in rows { let (h, s) = r.map_err(|e| e.to_string())?; out.insert(h, s.max(0)); }
        Ok(out)
    }

    /// Returns per-hour deduped union seconds, falling back to raw bucket sum for hours with no spans.
    fn event_work_union_by_hour(&self, from_hour: i64, to_hour: i64, project: Option<&str>) -> Result<std::collections::BTreeMap<i64, i64>, String> {
        let from_ms = from_hour.saturating_mul(1000);
        let to_ms = to_hour.saturating_mul(1000);
        let spans = self.event_spans_in_range(from_ms, to_ms)?;
        let spans: Vec<_> = spans.into_iter().filter(|(pid, pname, _, _, _, _)| match project { None => true, Some(p) => pid == p || pname == p }).collect();
        let raw_hours = self.raw_hour_sums(from_hour, to_hour, project)?;
        let mut by_hour: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
        if spans.is_empty() {
            return Ok(raw_hours);
        }
        let mut boundaries: Vec<i64> = Vec::with_capacity(spans.len() * 4);
        let mut min_h = i64::MAX;
        let mut max_h = i64::MIN;
        for (_, _, _, _, s, e) in &spans {
            let s = (*s).max(from_ms); let e = (*e).min(to_ms);
            if e > s {
                boundaries.push(s); boundaries.push(e);
                let hs = (s / 1000 / 3600) * 3600;
                let he = ((e - 1) / 1000 / 3600) * 3600;
                if hs < min_h { min_h = hs; }
                if he > max_h { max_h = he; }
            }
        }
        if min_h <= max_h {
            let mut h = min_h;
            while h <= max_h.saturating_add(3600) && h < to_hour {
                boundaries.push((h * 1000).max(from_ms).min(to_ms));
                h = h.saturating_add(3600);
            }
        }
        boundaries.sort(); boundaries.dedup();
        let mut span_idx = 0usize;
        let mut active: Vec<usize> = Vec::new();
        for w in boundaries.windows(2) {
            let a = w[0]; let b = w[1];
            if b <= a { continue; }
            while span_idx < spans.len() && spans[span_idx].4 < b { let e = spans[span_idx].5.min(to_ms); let s = spans[span_idx].4.max(from_ms); if e > a && s < b { active.push(span_idx); } span_idx += 1; }
            active.retain(|&i| { let e = spans[i].5.min(to_ms); let s = spans[i].4.max(from_ms); e > a && s < b });
            if active.is_empty() { continue; }
            let hour = (a / 1000 / 3600) * 3600;
            *by_hour.entry(hour).or_insert(0) += b - a; // ms
        }
        // convert ms->s and fill legacy hours
        let mut out: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
        for (&h, &ms) in &by_hour {
            if ms > 0 { out.insert(h, (ms / 1000).max(0)); }
        }
        for (&h, &raw) in &raw_hours {
            if raw > 0 && !by_hour.contains_key(&h) {
                out.insert(h, raw);
            }
        }
        Ok(out)
    }

    pub fn event_work_union(&self, from_hour: i64, to_hour: i64, project: Option<&str>) -> Result<i64, String> {
        let by_hour = self.event_work_union_by_hour(from_hour, to_hour, project)?;
        Ok(by_hour.values().sum())
    }

    /// Compute deduped event-work totals with proportional per-key allocation.
    /// Hybrid: hours with span data are deduped/allocated; legacy hours without spans use raw bucket sums.
    pub fn event_work_report_totals(&self, from_hour: i64, to_hour: i64, project: Option<&str>) -> Result<Vec<ReportRow>, String> {
        let from_ms = from_hour.saturating_mul(1000);
        let to_ms = to_hour.saturating_mul(1000);
        let spans = self.event_spans_in_range(from_ms, to_ms)?;
        let spans: Vec<_> = spans.into_iter().filter(|(pid, pname, _, _, _, _)| match project { None => true, Some(p) => pid == p || pname == p }).collect();
        let raw = self.report_totals(from_hour, to_hour, project)?;
        if spans.is_empty() {
            return Ok(raw);
        }

        #[derive(Clone, Default)]
        struct Acc { project_id: String, project_name: String, workspace: String, repo: String, ms: i64 }
        let mut acc: std::collections::BTreeMap<(String,String,String), Acc> = std::collections::BTreeMap::new();
        let mut span_hours: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
        let mut boundaries: Vec<i64> = Vec::with_capacity(spans.len() * 4);
        let mut min_h = i64::MAX;
        let mut max_h = i64::MIN;
        for (_, _, _, _, s, e) in &spans {
            let s = (*s).max(from_ms); let e = (*e).min(to_ms);
            if e > s {
                boundaries.push(s); boundaries.push(e);
                let hs = (s / 1000 / 3600) * 3600;
                let he = ((e - 1) / 1000 / 3600) * 3600;
                span_hours.insert(hs);
                span_hours.insert(he);
                if hs < min_h { min_h = hs; }
                if he > max_h { max_h = he; }
            }
        }
        if min_h <= max_h {
            let mut h = min_h;
            while h <= max_h.saturating_add(3600) && h < to_hour {
                boundaries.push((h * 1000).max(from_ms).min(to_ms));
                h = h.saturating_add(3600);
            }
        }
        boundaries.sort(); boundaries.dedup();
        let mut span_idx = 0usize;
        let mut active: Vec<usize> = Vec::new();
        for w in boundaries.windows(2) {
            let a = w[0]; let b = w[1];
            if b <= a { continue; }
            while span_idx < spans.len() && spans[span_idx].4 < b { let e = spans[span_idx].5.min(to_ms); let s = spans[span_idx].4.max(from_ms); if e > a && s < b { active.push(span_idx); } span_idx += 1; }
            active.retain(|&i| { let e = spans[i].5.min(to_ms); let s = spans[i].4.max(from_ms); e > a && s < b });
            if active.is_empty() { continue; }
            let mut keys: Vec<(String,String,String)> = active.iter().map(|&i| (spans[i].0.clone(), spans[i].2.clone(), spans[i].3.clone())).collect();
            keys.sort(); keys.dedup();
            let n = keys.len() as i64;
            let seg_ms = b - a;
            let share = seg_ms / n;
            let mut rem = seg_ms % n;
            for k in &keys {
                let add = share + if rem > 0 { rem -= 1; 1 } else { 0 };
                let e = acc.entry(k.clone()).or_insert_with(|| {
                    let sp = active.iter().find_map(|&i| { let si = &spans[i]; if si.0 == k.0 && si.2 == k.1 && si.3 == k.2 { Some(si) } else { None } }).unwrap();
                    Acc { project_id: sp.0.clone(), project_name: sp.1.clone(), workspace: sp.2.clone(), repo: sp.3.clone(), ms: 0 }
                });
                e.ms += add;
            }
        }

        // start from raw for counts and for legacy-hour active_seconds
        let mut count_map: std::collections::BTreeMap<(String,String,String), ReportRow> = std::collections::BTreeMap::new();
        for r in &raw {
            let k = (r.project_id.clone(), r.workspace.clone(), r.repo.clone());
            let e = count_map.entry(k).or_insert_with(|| ReportRow { active_seconds: 0, ..r.clone() });
            e.file_events += r.file_events; e.git_events += r.git_events; e.command_events += r.command_events; e.session_count += r.session_count;
            if e.project_name.is_empty() { e.project_name = r.project_name.clone(); }
        }
        // For legacy hours without spans, keep raw active seconds; for span hours, use allocated.
        // We need per-key raw seconds by hour to split this. Query raw per hour per key.
        let mut raw_hour_key: std::collections::BTreeMap<(i64,String,String,String), i64> = std::collections::BTreeMap::new();
        {
            let mut sql = String::from("SELECT hour, project_id, workspace, repo, SUM(active_seconds) FROM buckets WHERE hour >= ?1 AND hour < ?2");
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(from_hour), Box::new(to_hour)];
            if let Some(p) = project { sql.push_str(" AND (project_name = ?3 OR project_id = ?3)"); params_vec.push(Box::new(p.to_string())); }
            sql.push_str(" GROUP BY hour, project_id, workspace, repo");
            let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
            let rows = stmt.query_map(params_refs.as_slice(), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2).unwrap_or_default(), r.get::<_, String>(3).unwrap_or_default(), r.get::<_, i64>(4).unwrap_or(0)))).map_err(|e| e.to_string())?;
            for row in rows { let (hh,pid,ws,rp,sec) = row.map_err(|e| e.to_string())?; raw_hour_key.insert((hh,pid,ws,rp), sec.max(0)); }
        }
        // zero active_seconds, then fill: span hours from alloc, legacy hours from raw
        for e in count_map.values_mut() { e.active_seconds = 0; }
        // convert allocated ms to seconds, distributing remainder so sum == union seconds
        let total_alloc_ms: i64 = acc.values().map(|a| a.ms).sum();
        let target_s = (total_alloc_ms / 1000).max(0);
        let mut alloc_secs: Vec<((String,String,String), i64, i64)> = acc.iter().map(|(k,a)| (k.clone(), (a.ms/1000).max(0), a.ms % 1000)).collect();
        let mut sum_s: i64 = alloc_secs.iter().map(|x| x.1).sum();
        alloc_secs.sort_by(|a,b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        for x in &mut alloc_secs { if sum_s >= target_s { break; } x.1 += 1; sum_s += 1; }
        for (k, secs, _) in alloc_secs {
            let a = acc.get(&k).unwrap();
            let e = count_map.entry(k.clone()).or_insert_with(|| ReportRow { project_id: a.project_id.clone(), project_name: a.project_name.clone(), workspace: a.workspace.clone(), repo: a.repo.clone(), active_seconds: 0, file_events: 0, git_events: 0, command_events: 0, session_count: 0 });
            e.active_seconds += secs;
            if e.project_name.is_empty() { e.project_name = a.project_name.clone(); }
        }
        for ((hh,pid,ws,rp), sec) in &raw_hour_key {
            if span_hours.contains(hh) { continue; } // span hour handled by allocation
            let k = (pid.clone(), ws.clone(), rp.clone());
            let e = count_map.entry(k.clone()).or_insert_with(|| ReportRow { project_id: pid.clone(), project_name: String::new(), workspace: ws.clone(), repo: rp.clone(), active_seconds: 0, file_events: 0, git_events: 0, command_events: 0, session_count: 0 });
            e.active_seconds += *sec;
        }
        // fix project names for legacy-only keys
        for r in &raw { let k = (r.project_id.clone(), r.workspace.clone(), r.repo.clone()); if let Some(e) = count_map.get_mut(&k) { if e.project_name.is_empty() { e.project_name = r.project_name.clone(); } } }

        let mut rows: Vec<ReportRow> = count_map.into_values().collect();
        rows.sort_by(|a,b| a.project_name.cmp(&b.project_name).then(a.workspace.cmp(&b.workspace)).then(a.repo.cmp(&b.repo)));
        Ok(rows)
    }

    pub fn current_presence_interval(&self) -> Result<Option<super::model::PresenceInterval>, String> {
        self.conn.query_row(
            "SELECT id, start_ts, end_ts, state, source FROM presence_intervals WHERE end_ts IS NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok(super::model::PresenceInterval {
                id: r.get(0)?,
                start_ts: r.get(1)?,
                end_ts: r.get(2)?,
                state: r.get(3)?,
                source: r.get(4)?,
            }),
        ).optional().map_err(|e| e.to_string())
    }

    pub fn open_presence_interval(&self, start_ts: i64, state: &str, source: &str) -> Result<i64, String> {
        self.conn.execute(
            "INSERT INTO presence_intervals(start_ts, end_ts, state, source) VALUES(?1, NULL, ?2, ?3)",
            params![start_ts, state, source],
        ).map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn close_presence_interval(&self, id: i64, end_ts: i64) -> Result<(), String> {
        self.conn.execute(
            "UPDATE presence_intervals SET end_ts = ?2 WHERE id = ?1 AND (end_ts IS NULL OR end_ts > ?2)",
            params![id, end_ts],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn close_open_presence_intervals(&self, end_ts: i64) -> Result<usize, String> {
        self.conn.execute(
            "UPDATE presence_intervals SET end_ts = ?1 WHERE end_ts IS NULL",
            params![end_ts],
        ).map_err(|e| e.to_string())
    }

    pub fn latest_presence_state(&self) -> Result<Option<(String, String, i64)>, String> {
        self.conn.query_row(
            "SELECT state, source, COALESCE(end_ts, start_ts) as ts FROM presence_intervals ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
        ).optional().map_err(|e| e.to_string())
    }

    pub fn presence_intervals_in_range(&self, from_ms: i64, to_ms: i64) -> Result<Vec<super::model::PresenceInterval>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id, start_ts, end_ts, state, source FROM presence_intervals \
             WHERE start_ts < ?2 AND (end_ts IS NULL OR end_ts > ?1) ORDER BY start_ts ASC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![from_ms, to_ms], |r| Ok(super::model::PresenceInterval {
            id: r.get(0)?,
            start_ts: r.get(1)?,
            end_ts: r.get(2)?,
            state: r.get(3)?,
            source: r.get(4)?,
        })).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn upsert_presence_bucket(&self, b: &super::model::PresenceBucket) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO presence_buckets(hour, active_seconds, idle_seconds, away_seconds, locked_seconds, asleep_seconds, work_seconds, active_work_seconds, leisure_seconds, neutral_seconds, unattributed_seconds, unassigned_work_seconds, dirty, synced_hour)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,0)
             ON CONFLICT(hour) DO UPDATE SET
               active_seconds=excluded.active_seconds, idle_seconds=excluded.idle_seconds, away_seconds=excluded.away_seconds,
               locked_seconds=excluded.locked_seconds, asleep_seconds=excluded.asleep_seconds,
               work_seconds=excluded.work_seconds, active_work_seconds=excluded.active_work_seconds, leisure_seconds=excluded.leisure_seconds, neutral_seconds=excluded.neutral_seconds,
               unattributed_seconds=excluded.unattributed_seconds, unassigned_work_seconds=excluded.unassigned_work_seconds,
               dirty=CASE WHEN
                 presence_buckets.active_seconds!=excluded.active_seconds OR presence_buckets.idle_seconds!=excluded.idle_seconds OR
                 presence_buckets.away_seconds!=excluded.away_seconds OR presence_buckets.locked_seconds!=excluded.locked_seconds OR
                 presence_buckets.asleep_seconds!=excluded.asleep_seconds OR presence_buckets.work_seconds!=excluded.work_seconds OR
                 presence_buckets.active_work_seconds!=excluded.active_work_seconds OR
                 presence_buckets.leisure_seconds!=excluded.leisure_seconds OR presence_buckets.neutral_seconds!=excluded.neutral_seconds OR
                 presence_buckets.unattributed_seconds!=excluded.unattributed_seconds OR presence_buckets.unassigned_work_seconds!=excluded.unassigned_work_seconds
               THEN 1 ELSE presence_buckets.dirty END",
            params![b.hour, b.active_seconds, b.idle_seconds, b.away_seconds, b.locked_seconds, b.asleep_seconds, b.work_seconds, b.active_work_seconds, b.leisure_seconds, b.neutral_seconds, b.unattributed_seconds, b.unassigned_work_seconds],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn presence_totals(&self, from_hour: i64, to_hour: i64) -> Result<super::model::PresenceTotals, String> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(active_seconds),0), COALESCE(SUM(idle_seconds),0), COALESCE(SUM(away_seconds),0), \
                    COALESCE(SUM(locked_seconds),0), COALESCE(SUM(asleep_seconds),0), \
                    COALESCE(SUM(work_seconds),0), COALESCE(SUM(active_work_seconds),0), COALESCE(SUM(leisure_seconds),0), COALESCE(SUM(neutral_seconds),0), \
                    COALESCE(SUM(unattributed_seconds),0), COALESCE(SUM(unassigned_work_seconds),0) \
             FROM presence_buckets WHERE hour >= ?1 AND hour < ?2",
            params![from_hour, to_hour],
            |r| {
                let active: i64 = r.get(0)?;
                let idle: i64 = r.get(1)?;
                Ok(super::model::PresenceTotals {
                    active_seconds: active,
                    idle_seconds: idle,
                    away_seconds: r.get(2)?,
                    locked_seconds: r.get(3)?,
                    asleep_seconds: r.get(4)?,
                    work_seconds: r.get(5)?,
                    active_work_seconds: r.get(6)?,
                    leisure_seconds: r.get(7)?,
                    neutral_seconds: r.get(8)?,
                    unattributed_seconds: r.get(9)?,
                    unassigned_work_seconds: r.get(10)?,
                    present_seconds: active + idle,
                })
            },
        ).map_err(|e| e.to_string())
    }

    pub fn presence_buckets_in_range(&self, from_hour: i64, to_hour: i64) -> Result<Vec<super::model::PresenceBucket>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT hour, active_seconds, idle_seconds, away_seconds, locked_seconds, asleep_seconds, work_seconds, active_work_seconds, leisure_seconds, neutral_seconds, unattributed_seconds, unassigned_work_seconds \
             FROM presence_buckets WHERE hour >= ?1 AND hour < ?2 ORDER BY hour ASC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![from_hour, to_hour], |r| Ok(super::model::PresenceBucket {
            hour: r.get(0)?,
            active_seconds: r.get(1)?,
            idle_seconds: r.get(2)?,
            away_seconds: r.get(3)?,
            locked_seconds: r.get(4)?,
            asleep_seconds: r.get(5)?,
            work_seconds: r.get(6)?,
            active_work_seconds: r.get(7)?,
            leisure_seconds: r.get(8)?,
            neutral_seconds: r.get(9)?,
            unattributed_seconds: r.get(10)?,
            unassigned_work_seconds: r.get(11)?,
        })).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    // --- Focus / Category ---

    fn focus_interval_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<super::model::FocusInterval> {
        Ok(super::model::FocusInterval {
            id: r.get(0)?, start_ts: r.get(1)?, end_ts: r.get(2)?,
            project_id: r.get(3)?, project_name: r.get(4)?, workspace: r.get(5)?, repo: r.get(6)?,
            source: r.get(7)?, confidence: r.get(8)?, category: r.get(9)?, category_source: r.get(10)?,
            app: r.get(11).unwrap_or_default(), app_kind: r.get(12).unwrap_or_default(),
            domain: r.get(13).unwrap_or_default(), rule: r.get(14).unwrap_or_default(),
            evidence: r.get(15).unwrap_or_default(),
        })
    }

    pub fn current_focus_interval(&self) -> Result<Option<super::model::FocusInterval>, String> {
        self.conn.query_row(
            "SELECT id,start_ts,end_ts,project_id,project_name,workspace,repo,source,confidence,category,category_source,app,app_kind,domain,rule,evidence \
             FROM focus_intervals WHERE end_ts IS NULL ORDER BY id DESC LIMIT 1",
            [],
            Self::focus_interval_row,
        ).optional().map_err(|e| e.to_string())
    }

    pub fn focus_interval_at(&self, ts_ms: i64) -> Result<Option<super::model::FocusInterval>, String> {
        self.conn.query_row(
            "SELECT id,start_ts,end_ts,project_id,project_name,workspace,repo,source,confidence,category,category_source,app,app_kind,domain,rule,evidence \
             FROM focus_intervals WHERE start_ts <= ?1 AND (end_ts IS NULL OR end_ts > ?1) ORDER BY id DESC LIMIT 1",
            rusqlite::params![ts_ms],
            Self::focus_interval_row,
        ).optional().map_err(|e| e.to_string())
    }

    pub fn open_focus_interval(&self, f: &super::model::FocusInterval) -> Result<i64, String> {
        self.conn.execute(
            "INSERT INTO focus_intervals(start_ts,end_ts,project_id,project_name,workspace,repo,source,confidence,category,category_source,app,app_kind,domain,rule,evidence) \
             VALUES(?1,NULL,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![f.start_ts, f.project_id, f.project_name, f.workspace, f.repo, f.source, f.confidence, f.category, f.category_source, f.app, f.app_kind, f.domain, f.rule, f.evidence],
        ).map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn close_focus_interval(&self, id: i64, end_ts: i64) -> Result<(), String> {
        self.conn.execute(
            "UPDATE focus_intervals SET end_ts=?2 WHERE id=?1 AND (end_ts IS NULL OR end_ts > ?2)",
            params![id, end_ts],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn close_open_focus_intervals(&self, end_ts: i64) -> Result<usize, String> {
        self.conn.execute("UPDATE focus_intervals SET end_ts=?1 WHERE end_ts IS NULL", params![end_ts]).map_err(|e| e.to_string())
    }

    pub fn focus_intervals_in_range(&self, from_ms: i64, to_ms: i64) -> Result<Vec<super::model::FocusInterval>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id,start_ts,end_ts,project_id,project_name,workspace,repo,source,confidence,category,category_source,app,app_kind,domain,rule,evidence \
             FROM focus_intervals WHERE start_ts < ?2 AND (end_ts IS NULL OR end_ts > ?1) ORDER BY start_ts ASC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![from_ms, to_ms], Self::focus_interval_row).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn upsert_focus_bucket(&self, b: &super::model::FocusBucket) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO focus_buckets(project_id,project_name,hour,workspace,repo,work_seconds,input_seconds,dirty,synced_hour) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,1,0) \
             ON CONFLICT(project_id,hour,workspace,repo) DO UPDATE SET \
               project_name=excluded.project_name, \
               work_seconds=excluded.work_seconds, input_seconds=excluded.input_seconds, \
               dirty=CASE WHEN focus_buckets.work_seconds!=excluded.work_seconds OR focus_buckets.input_seconds!=excluded.input_seconds THEN 1 ELSE focus_buckets.dirty END",
            params![b.project_id,b.project_name,b.hour,b.workspace,b.repo,b.work_seconds,b.input_seconds],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn focus_totals(&self, from_hour: i64, to_hour: i64) -> Result<(i64, i64), String> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(work_seconds),0), COALESCE(SUM(input_seconds),0) FROM focus_buckets WHERE hour >= ?1 AND hour < ?2",
            params![from_hour, to_hour],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ).map_err(|e| e.to_string())
    }

    pub fn report_focus_breakdown(&self, from_hour: i64, to_hour: i64, group_by: &str) -> Result<Vec<super::model::ActivityBreakdownRow>, String> {
        let (key_expr, group_expr) = match group_by {
            "repo" => ("repo", "repo"),
            _ => ("project_name", "project_id, project_name"),
        };
        let sql = format!(
            "SELECT {key_expr}, project_id, project_name, workspace, repo, \
             SUM(work_seconds), SUM(input_seconds) \
             FROM focus_buckets WHERE hour >= ?1 AND hour < ?2 AND work_seconds > 0 \
             GROUP BY {group_expr} ORDER BY SUM(work_seconds) DESC, 1 ASC"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![from_hour, to_hour], |r| {
            let key: String = r.get::<_, String>(0).unwrap_or_default();
            let project_id: String = r.get::<_, String>(1).unwrap_or_default();
            let project_name: String = r.get::<_, String>(2).unwrap_or_default();
            let ws: String = r.get::<_, String>(3).unwrap_or_default();
            let rp: String = r.get::<_, String>(4).unwrap_or_default();
            Ok(super::model::ActivityBreakdownRow {
                key: if key.is_empty() { if group_by == "repo" { "(none)".to_string() } else { project_name.clone() } } else { key },
                project_id: if group_by == "project" { Some(project_id) } else { None },
                project_name: if group_by == "project" { Some(project_name) } else { None },
                workspace: if group_by == "workspace" { Some(ws) } else { None },
                repo: if group_by == "repo" { Some(rp) } else { None },
                active_seconds: r.get::<_, i64>(5).unwrap_or(0), // work_seconds
                file_events: r.get::<_, i64>(6).unwrap_or(0),    // input_seconds (reused field)
                git_events: 0,
                command_events: 0,
                session_count: 0,
            })
        }).map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }

    pub fn active_category_override(&self, now_ms: i64, focus: Option<&super::model::FocusInterval>) -> Result<Option<super::model::CategoryOverride>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id,start_ts,end_ts,category,scope,created_at,match_project,match_repo,match_workspace,match_domain,match_app \
             FROM category_overrides WHERE start_ts <= ?1 AND (end_ts IS NULL OR end_ts > ?1) ORDER BY id DESC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![now_ms], |r| Ok(super::model::CategoryOverride {
            id: r.get(0)?, start_ts: r.get(1)?, end_ts: r.get(2)?, category: r.get(3)?, scope: r.get(4)?,
            match_project: r.get::<_, String>(6).unwrap_or_default(),
            match_repo: r.get::<_, String>(7).unwrap_or_default(),
            match_workspace: r.get::<_, String>(8).unwrap_or_default(),
            match_domain: r.get::<_, String>(9).unwrap_or_default(),
            match_app: r.get::<_, String>(10).unwrap_or_default(),
        })).map_err(|e| e.to_string())?;
        for row in rows {
            let o = row.map_err(|e| e.to_string())?;
            if o.scope == "global" { return Ok(Some(o)); }
            // focus-scoped: match stored fields against current focus
            let Some(f) = focus else { continue; };
            let ok = (o.match_project.is_empty() || o.match_project == f.project_id || o.match_project == f.project_name)
                && (o.match_repo.is_empty() || o.match_repo == f.repo)
                && (o.match_workspace.is_empty() || o.match_workspace == f.workspace)
                && (o.match_domain.is_empty() || o.match_domain.eq_ignore_ascii_case(&f.domain))
                && (o.match_app.is_empty() || o.match_app.eq_ignore_ascii_case(&f.app));
            if ok { return Ok(Some(o)); }
        }
        Ok(None)
    }

    pub fn insert_category_override(&self, o: &super::model::CategoryOverride) -> Result<i64, String> {
        self.conn.execute(
            "INSERT INTO category_overrides(start_ts,end_ts,category,scope,created_at,match_project,match_repo,match_workspace,match_domain,match_app) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![o.start_ts, o.end_ts, o.category, o.scope, now_ms(), o.match_project, o.match_repo, o.match_workspace, o.match_domain, o.match_app],
        ).map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn clear_active_category_overrides(&self, now_ms: i64) -> Result<usize, String> {
        self.conn.execute(
            "UPDATE category_overrides SET end_ts=?1 WHERE end_ts IS NULL OR end_ts > ?1",
            params![now_ms],
        ).map_err(|e| e.to_string())
    }
}

pub fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64 }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::model::Bucket;

    fn temp_db() -> (tempfile::TempDir, ActivityDb) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("activity.db");
        let db = ActivityDb::open(&db_path).unwrap();
        (dir, db)
    }

    fn bucket(project_id: &str, project_name: &str, hour: i64, workspace: &str, repo: &str, active: i64, f: i64, g: i64, c: i64, s: i64) -> Bucket {
        Bucket {
            project_id: project_id.to_string(), project_name: project_name.to_string(),
            hour, workspace: workspace.to_string(), repo: repo.to_string(),
            active_seconds: active, file_events: f, git_events: g, command_events: c, session_count: s,
        }
    }

    #[test]
    fn report_series_empty() {
        let (_dir, db) = temp_db();
        let rows = db.report_series(0, 3600 * 24, "hour", None, None, None, false).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn report_series_hour_and_filters() {
        let (_dir, db) = temp_db();
        db.upsert_bucket(&bucket("p1", "proj", 3600, "ws", "repo", 100, 1, 2, 3, 1), false).unwrap();
        db.upsert_bucket(&bucket("p1", "proj", 7200, "ws", "repo", 50, 0, 0, 1, 0), false).unwrap();
        db.upsert_bucket(&bucket("p2", "other", 3600, "ws2", "repo2", 200, 5, 0, 0, 1), false).unwrap();

        let all = db.report_series(0, 10_000, "hour", None, None, None, false).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].bucket_start, 3600);
        assert_eq!(all[0].active_seconds, 300); // p1+p2 same hour
        assert_eq!(all[1].bucket_start, 7200);
        assert_eq!(all[1].active_seconds, 50);

        let p1 = db.report_series(0, 10_000, "hour", Some("p1"), None, None, false).unwrap();
        assert_eq!(p1.len(), 2);
        assert_eq!(p1[0].active_seconds, 100);

        let repo_filter = db.report_series(0, 10_000, "hour", None, Some("repo2"), None, false).unwrap();
        assert_eq!(repo_filter.len(), 1);
        assert_eq!(repo_filter[0].active_seconds, 200);
    }

    #[test]
    fn report_series_group_by_project() {
        let (_dir, db) = temp_db();
        db.upsert_bucket(&bucket("p1", "proj", 3600, "ws", "repo", 100, 1, 0, 0, 1), false).unwrap();
        db.upsert_bucket(&bucket("p2", "other", 3600, "ws", "repo", 200, 0, 1, 0, 1), false).unwrap();

        let rows = db.report_series(0, 10_000, "hour", None, None, None, true).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].project_name.as_deref(), Some("other")); // same hour, ordered by project_name asc
        assert_eq!(rows[1].project_name.as_deref(), Some("proj"));
        assert_eq!(rows[0].active_seconds, 200);
        assert_eq!(rows[1].active_seconds, 100);
    }

    #[test]
    fn report_series_day_step() {
        let (_dir, db) = temp_db();
        let day = 86400i64;
        db.upsert_bucket(&bucket("p1", "proj", day, "ws", "repo", 100, 1, 0, 0, 1), false).unwrap();
        db.upsert_bucket(&bucket("p1", "proj", day + 3600, "ws", "repo", 50, 1, 0, 0, 0), false).unwrap();
        db.upsert_bucket(&bucket("p1", "proj", day * 2, "ws", "repo", 30, 0, 0, 1, 0), false).unwrap();

        let rows = db.report_series(0, day * 3, "day", None, None, None, false).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].bucket_start, day);
        assert_eq!(rows[0].active_seconds, 150);
        assert_eq!(rows[1].bucket_start, day * 2);
        assert_eq!(rows[1].active_seconds, 30);
    }

    #[test]
    fn report_breakdown_project_repo_workspace() {
        let (_dir, db) = temp_db();
        db.upsert_bucket(&bucket("p1", "proj", 3600, "ws1", "repo1", 100, 1, 0, 0, 1), false).unwrap();
        db.upsert_bucket(&bucket("p1", "proj", 3600, "ws2", "repo2", 50, 0, 1, 0, 0), false).unwrap();
        db.upsert_bucket(&bucket("p2", "other", 3600, "ws1", "repo1", 200, 0, 0, 1, 1), false).unwrap();

        let by_project = db.report_breakdown(0, 10_000, "project", None, None, None).unwrap();
        assert_eq!(by_project.len(), 2);
        assert_eq!(by_project[0].key, "other");
        assert_eq!(by_project[0].active_seconds, 200);
        assert_eq!(by_project[1].key, "proj");
        assert_eq!(by_project[1].active_seconds, 150);

        let by_repo = db.report_breakdown(0, 10_000, "repo", None, None, None).unwrap();
        assert_eq!(by_repo.len(), 2);
        assert_eq!(by_repo[0].key, "repo1");
        assert_eq!(by_repo[0].active_seconds, 300);

        let by_ws = db.report_breakdown(0, 10_000, "workspace", Some("p1"), None, None).unwrap();
        assert_eq!(by_ws.len(), 2);
        assert_eq!(by_ws[0].key, "ws1");
        assert_eq!(by_ws[0].active_seconds, 100);
    }
}
