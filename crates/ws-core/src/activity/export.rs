use serde::Serialize;

use super::db::ActivityDb;

#[derive(Serialize)]
pub struct ExportRow {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub project_id: String,
    pub project_name: String,
    pub workspace: String,
    pub repo: String,
    pub rel_path: Option<String>,
    pub cmd: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub count: i64,
}

pub fn export_events(db: &ActivityDb, from_ms: i64, to_ms: i64) -> Result<Vec<ExportRow>, String> {
    let mut stmt = db.conn.prepare(
        "SELECT id, ts, kind, project_id, project_name, workspace, repo, rel_path, cmd, exit_code, duration_ms, count
         FROM events WHERE ts >= ?1 AND ts < ?2 ORDER BY ts ASC, id ASC"
    ).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![from_ms, to_ms], |r| {
            Ok(ExportRow {
                id: r.get(0)?,
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
            })
        })
        .map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}
