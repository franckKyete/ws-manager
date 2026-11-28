use std::fs;
use std::path::Path;

use rusqlite::params;

use super::config::ActivityConfig;
use super::db::ActivityDb;

fn legacy_project_id_for_path(path: &str) -> String {
    format!("path:{}", path)
}

#[derive(Debug, Clone)]
pub struct IdMigrationCandidate {
    pub name: String,
    pub path: String,
    pub canonical: String,
    pub legacy: String,
    pub legacy_events: i64,
    pub legacy_buckets: i64,
    pub legacy_state: i64,
    pub legacy_oldest_event: Option<i64>,
    pub legacy_min_bucket_hour: Option<i64>,
    pub canonical_oldest_event: Option<i64>,
}

pub fn list_candidates(db: &ActivityDb) -> Result<Vec<IdMigrationCandidate>, String> {
    let projects = db.list_projects()?;
    let mut out = Vec::new();
    for p in projects {
        let legacy = legacy_project_id_for_path(&p.path);
        let legacy_events: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE project_id = ?1",
                params![legacy],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let legacy_buckets: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM buckets WHERE project_id = ?1",
                params![legacy],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let legacy_state: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM rollup_state WHERE project_id = ?1",
                params![legacy],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let legacy_oldest_event: Option<i64> = db
            .conn
            .query_row(
                "SELECT MIN(ts) FROM events WHERE project_id = ?1",
                params![legacy],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let legacy_min_bucket_hour: Option<i64> = db
            .conn
            .query_row(
                "SELECT MIN(hour) FROM buckets WHERE project_id = ?1",
                params![legacy],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let canonical_oldest_event: Option<i64> = db
            .conn
            .query_row(
                "SELECT MIN(ts) FROM events WHERE project_id = ?1",
                params![p.project_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        out.push(IdMigrationCandidate {
            name: p.name,
            path: p.path,
            canonical: p.project_id,
            legacy,
            legacy_events,
            legacy_buckets,
            legacy_state,
            legacy_oldest_event,
            legacy_min_bucket_hour,
            canonical_oldest_event,
        });
    }
    Ok(out)
}

pub fn pending_candidates(db: &ActivityDb) -> Result<Vec<IdMigrationCandidate>, String> {
    Ok(list_candidates(db)?
        .into_iter()
        .filter(|c| {
            c.legacy != c.canonical
                && (c.legacy_events > 0 || c.legacy_buckets > 0 || c.legacy_state > 0)
        })
        .collect())
}

pub fn migrate(
    db: &ActivityDb,
    cfg: &ActivityConfig,
    keep_aggregates: bool,
    db_path: &Path,
) -> Result<(Vec<IdMigrationCandidate>, Option<std::path::PathBuf>), String> {
    let todo = pending_candidates(db)?;
    if todo.is_empty() {
        return Ok((todo, None));
    }

    // Preflight: rebuild requires raw events to cover legacy buckets
    if !keep_aggregates {
        for m in &todo {
            if m.legacy_buckets > 0 && m.legacy_events == 0 {
                return Err(format!(
                    "Cannot rebuild '{}' without raw events; rerun with --keep-aggregates.",
                    m.name
                ));
            }
        }
    }

    // Best-effort backup
    let bak = db_path.with_extension(format!(
        "bak-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let backup = fs::copy(db_path, &bak).ok().map(|_| bak);

    for m in &todo {
        let tx = db.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE events SET project_id = ?1 WHERE project_id = ?2",
            params![m.canonical, m.legacy],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM projects WHERE project_id = ?1",
            params![m.legacy],
        )
        .map_err(|e| e.to_string())?;

        if keep_aggregates {
            tx.execute(
                "INSERT INTO buckets(project_id,project_name,hour,workspace,repo,active_seconds,file_events,git_events,command_events,session_count,dirty,synced_hour)
                 SELECT ?1, project_name, hour, workspace, repo, active_seconds, file_events, git_events, command_events, session_count, 1, 0
                 FROM buckets WHERE project_id = ?2
                 ON CONFLICT(project_id,hour,workspace,repo) DO UPDATE SET
                   project_name=CASE WHEN excluded.project_name != '' THEN excluded.project_name ELSE buckets.project_name END,
                   active_seconds=buckets.active_seconds + excluded.active_seconds,
                   file_events=buckets.file_events + excluded.file_events,
                   git_events=buckets.git_events + excluded.git_events,
                   command_events=buckets.command_events + excluded.command_events,
                   session_count=buckets.session_count + excluded.session_count,
                   dirty=1",
                params![m.canonical, m.legacy],
            ).map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM buckets WHERE project_id = ?1",
                params![m.legacy],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM rollup_state WHERE project_id = ?1",
                params![m.legacy],
            )
            .map_err(|e| e.to_string())?;
            let max_id: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(id), 0) FROM events WHERE project_id = ?1",
                    params![m.canonical],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let max_hour: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(hour), 0) FROM buckets WHERE project_id = ?1",
                    params![m.canonical],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            tx.execute(
                "INSERT INTO rollup_state(project_id, through_hour, through_event_id, session_state) VALUES(?1,?2,?3,NULL)
                 ON CONFLICT(project_id) DO UPDATE SET through_hour=excluded.through_hour, through_event_id=excluded.through_event_id, session_state=NULL",
                params![m.canonical, max_hour, max_id],
            ).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        } else {
            tx.execute(
                "DELETE FROM buckets WHERE project_id = ?1 OR project_id = ?2",
                params![m.canonical, m.legacy],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM rollup_state WHERE project_id = ?1 OR project_id = ?2",
                params![m.canonical, m.legacy],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            super::rollup::rerollup_project(db, cfg, &m.canonical)?;
        }
    }

    Ok((todo, backup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::config::ActivityConfig;
    use crate::activity::db::ActivityDb;
    use crate::activity::model::ProjectState;

    fn temp_db() -> (tempfile::TempDir, ActivityDb) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("activity.db");
        let db = ActivityDb::open(&db_path).unwrap();
        (dir, db)
    }

    fn project(project_id: &str, path: &str, name: &str) -> ProjectState {
        ProjectState {
            project_id: project_id.to_string(),
            path: path.to_string(),
            name: name.to_string(),
            hub_project: None,
            hub_auto_created: false,
            sync_enabled: true,
            disabled_reason: None,
            linked_at: None,
            last_synced_hour: None,
            backoff_until: 0,
            last_error: None,
        }
    }

    #[test]
    fn lists_and_migrates_legacy_project_id_keep_aggregates() {
        let (_dir, db) = temp_db();
        let cfg = ActivityConfig::default();
        let path = "/tmp/example-project";
        let legacy = format!("path:{}", path);
        let canonical = "act_canonical".to_string();
        db.upsert_project(&project(canonical.as_str(), path, "example"))
            .unwrap();

        db.conn.execute(
            "INSERT INTO events(ts,kind,project_id,project_name,workspace,repo,rel_path,cmd,exit_code,duration_ms,count,meta) VALUES(1000,'file.modify',?1,'example','ws','repo',NULL,NULL,NULL,NULL,1,NULL)",
            params![legacy],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO buckets(project_id,project_name,hour,workspace,repo,active_seconds,file_events,git_events,command_events,session_count,dirty,synced_hour)
             VALUES(?1,'example',0,'ws','repo',60,1,0,0,1,1,0)",
            params![legacy],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO rollup_state(project_id,through_hour,through_event_id,session_state) VALUES(?1,0,1,NULL)",
            params![legacy],
        ).unwrap();

        let candidates = pending_candidates(&db).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].legacy_events, 1);
        assert_eq!(candidates[0].legacy_buckets, 1);

        let (migrated, _backup) = migrate(&db, &cfg, true, db.path.as_path()).unwrap();
        assert_eq!(migrated.len(), 1);

        let legacy_events: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE project_id=?1",
                params![legacy],
                |r| r.get(0),
            )
            .unwrap();
        let canonical_events: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE project_id=?1",
                params![canonical],
                |r| r.get(0),
            )
            .unwrap();
        let legacy_buckets: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM buckets WHERE project_id=?1",
                params![legacy],
                |r| r.get(0),
            )
            .unwrap();
        let canonical_buckets: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM buckets WHERE project_id=?1",
                params![canonical],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legacy_events, 0);
        assert_eq!(canonical_events, 1);
        assert_eq!(legacy_buckets, 0);
        assert_eq!(canonical_buckets, 1);

        let pending_after = pending_candidates(&db).unwrap();
        assert!(pending_after.is_empty());
    }
}
