use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use ignore::WalkBuilder;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

use crate::config::ConfigLoader;
use crate::registry::list_registered_projects;

use super::db::{now_ms, ActivityDb, PRIVACY_NOTICE_VERSION};
use super::model::{ActivityEvent, ProjectState};
use super::ActivityConfig;

const MAX_WATCHES: usize = 8192;
const INOTIFY_BUF_SIZE: usize = 64 * 1024;

#[derive(Clone)]
struct FileState {
    mtime_ms: i64,
    len: u64,
}

#[derive(Clone, Default)]
struct GitState {
    head: String,
    index_mtime_ms: i64,
}

#[derive(Clone)]
struct RepoTarget {
    project_id: String,
    project_name: String,
    workspace: String,
    repo: String,
    path: PathBuf,
    active: bool,
}

#[derive(Clone)]
struct WatchCtx {
    target_idx: usize,
    dir: PathBuf,
}

#[derive(Clone, Debug)]
struct PendingEvent {
    kind: String, // file.create/file.modify/file.delete/file.rename
    ts_ms: i64,
    count: i64,
}

#[derive(Clone, Debug)]
struct MoveFrom {
    target_idx: usize,
    dir: PathBuf,
    name: OsString,
    ts: Instant,
}

pub struct ActivityCollector {
    cfg: ActivityConfig,
    paths: super::ActivityPaths,
    db: ActivityDb,
    last_heartbeat: HashMap<String, i64>,
    last_notice_check: Instant,
    file_state: HashMap<String, HashMap<String, FileState>>, // scope -> rel path -> state
    git_state: HashMap<String, GitState>,                   // scope -> git state
    last_reconcile: Instant,
    last_light_scan: Instant,
    last_discovery: Instant,
    last_rollup: Instant,
    socket_listener: Option<UnixListener>,

    // inotify state
    inotify: Option<Inotify>,
    watch_by_wd: HashMap<WatchDescriptor, WatchCtx>,
    wd_by_dir: HashMap<PathBuf, WatchDescriptor>,
    targets: Vec<RepoTarget>,
    watch_limited: bool,
    needs_rescan: bool,
    pending_by_key: HashMap<(usize, String, String), PendingEvent>, // (target_idx, rel_path, kind) -> pending
    recent_moves: VecDeque<(u32, MoveFrom)>, // cookie -> from

    // presence
    presence: super::presence::PresenceMonitor,
    focus: super::focus::FocusMonitor,
}

impl ActivityCollector {
    pub fn new() -> Result<Self, String> {
        let paths = super::ActivityPaths::discover();
        paths.ensure_dirs()?;
        let cfg = ActivityConfig::load().unwrap_or_default();
        let db = ActivityDb::open(&paths.db_path)?;
        let mut presence = super::presence::PresenceMonitor::new(&db, &cfg);
        if cfg.presence.enabled && (cfg.presence.source == "auto" || cfg.presence.source == "wayland") {
            presence.rx = super::presence_wayland::spawn_wayland_watcher(&cfg.presence);
        }
        let focus = super::focus::FocusMonitor::new(&db);
        let mut c = Self {
            cfg,
            paths,
            db,
            last_heartbeat: HashMap::new(),
            last_notice_check: Instant::now(),
            file_state: HashMap::new(),
            git_state: HashMap::new(),
            last_reconcile: Instant::now() - Duration::from_secs(3600),
            last_light_scan: Instant::now() - Duration::from_secs(3600),
            last_discovery: Instant::now() - Duration::from_secs(3600),
            last_rollup: Instant::now() - Duration::from_secs(3600),
            socket_listener: None,
            inotify: None,
            watch_by_wd: HashMap::new(),
            wd_by_dir: HashMap::new(),
            targets: Vec::new(),
            watch_limited: false,
            needs_rescan: true,
            pending_by_key: HashMap::new(),
            recent_moves: VecDeque::new(),
            presence,
            focus,
        };
        c.init_socket();
        c.init_inotify();
        Ok(c)
    }

    fn init_socket(&mut self) {
        let sock = &self.paths.socket_path;
        if sock.exists() {
            let _ = fs::remove_file(sock);
        }
        match UnixListener::bind(sock) {
            Ok(l) => {
                let _ = l.set_nonblocking(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(sock, fs::Permissions::from_mode(0o600));
                }
                self.socket_listener = Some(l);
            }
            Err(_) => {
                self.socket_listener = None;
            }
        }
    }

    fn init_inotify(&mut self) {
        match Inotify::init() {
            Ok(ino) => {
                // best-effort nonblocking
                unsafe {
                    let fd = ino.as_raw_fd();
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    if flags >= 0 {
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                }
                self.inotify = Some(ino);
            }
            Err(_) => {
                self.inotify = None;
            }
        }
    }

    pub fn tick(&mut self) {
        self.cfg = ActivityConfig::load().unwrap_or(self.cfg.clone());
        let _ = self.accept_socket_records();
        let _ = self.drain_spool();

        // Presence runs even when activity collection is paused? No - tied to enabled.
        // But we still want presence to be tracked when enabled; pause only pauses event collection.
        if self.cfg.enabled {
            self.presence.tick(&self.db, &self.cfg);
            let pstate = self.presence.current_state();
            self.focus.tick(&self.db, &self.cfg, pstate);
        }

        if !self.cfg.enabled || self.cfg.paused {
            self.flush_pending_events(true);
            return;
        }

        self.maybe_show_notice();

        // fast path: inotify + debounced event flush
        let _ = self.poll_inotify();
        self.flush_pending_events(false);

        // periodic discovery/heartbeats
        if self.last_discovery.elapsed() >= Duration::from_secs(30) {
            let _ = self.discover_and_heartbeat();
            self.last_discovery = Instant::now();
        }

        // reconcile watches/scans
        let reconcile_every = Duration::from_secs(self.cfg.watch_reconcile.max(30));
        if self.needs_rescan || self.last_reconcile.elapsed() >= reconcile_every {
            let _ = self.reconcile();
            self.last_reconcile = Instant::now();
            self.needs_rescan = false;
        } else if self.last_light_scan.elapsed() >= Duration::from_secs(120) {
            // periodic git/light correctness scan for active targets
            let _ = self.scan_git_and_light(false);
            self.last_light_scan = Instant::now();
        }

        // rollup/sync on slower cadence
        if self.last_rollup.elapsed() >= Duration::from_secs(15) {
            let _ = super::rollup::rollup_all(&self.db, &self.cfg);
            self.maybe_sync();
            self.last_rollup = Instant::now();
        }

        // expire old move cookies
        let now_inst = Instant::now();
        while self.recent_moves.front().map(|(_, m)| now_inst.duration_since(m.ts) > Duration::from_secs(2)).unwrap_or(false) {
            if let Some((_, from)) = self.recent_moves.pop_front() {
                self.emit_move_from_as_delete(from);
            }
        }
    }

    fn maybe_sync(&mut self) {
        if !self.cfg.sync.enabled { return; }
        let now = super::db::now_ms();
        let last = self.db.get_meta_i64("last_sync_attempt_ms").ok().flatten().unwrap_or(0);
        let interval_ms = (self.cfg.sync.interval.max(60) * 1000) as i64;
        if now - last < interval_ms { return; }
        let _ = self.db.set_meta("last_sync_attempt_ms", &now.to_string());
        let _ = super::sync::sync_all(&self.db, &self.cfg, false);
    }

    fn maybe_show_notice(&mut self) {
        if self.last_notice_check.elapsed() < Duration::from_secs(60) {
            return;
        }
        self.last_notice_check = Instant::now();
        if self.db.get_meta_i64("privacy_notice_shown_at").ok().flatten().is_none() {
            crate::notify::send_dbus_notification(
                "Activity tracking",
                "Aggregates may sync to wshub when configured (time/counts/project/repo/workspace/device). No paths, command names, or contents are synced.",
                "dialog-information",
                8000,
            );
            let _ = self.db.mark_privacy_notice_shown(PRIVACY_NOTICE_VERSION);
        }
    }

    fn discover_and_heartbeat(&mut self) -> Result<(), String> {
        let (targets, known_ws, active_ws) = self.collect_targets()?;
        self.targets = targets;

        let _ = self.db.set_meta("known_workspaces", &known_ws.to_string());
        let _ = self.db.set_meta("watching_workspaces", &active_ws.to_string());

        // update projects
        for p in list_registered_projects(None, true) {
            let mut cfg_file = p.join("ws-project.yml");
            if !cfg_file.exists() { cfg_file = p.join("ws-project.yaml"); }
            if !cfg_file.exists() { continue; }
            let Ok(config) = ConfigLoader::load_config(Some(&cfg_file), None, true) else { continue; };
            let project_name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".to_string());
            let project_id = ensure_project_id(&config).unwrap_or_else(|_| format!("path:{}", p.to_string_lossy()));
            let hub_project = config.activity.get("hub_project").and_then(|v| v.as_str()).map(|s| s.to_string());
            let state = ProjectState {
                project_id: project_id.clone(),
                path: p.to_string_lossy().to_string(),
                name: project_name,
                hub_project: hub_project.clone(),
                hub_auto_created: false,
                sync_enabled: config.activity.get("sync").and_then(|v| v.as_bool()).unwrap_or(true),
                disabled_reason: None,
                linked_at: None,
                last_synced_hour: None,
                backoff_until: 0,
                last_error: None,
            };
            let _ = self.db.upsert_project(&state);
        }

        // heartbeats for active workspaces (gated by focus when attribution is available)
        let now = now_ms();
        let heartbeat_ms = (self.cfg.heartbeat_interval.max(15) * 1000) as i64;
        let mut emitted_ws: HashSet<String> = HashSet::new();
        let focus = self.focus.current().cloned();
        let presence_ok = self.presence.current_state().is_present();
        for t in &self.targets {
            if !t.active { continue; }
            let key = format!("{}::{}", t.project_id, t.workspace);
            if !emitted_ws.insert(key.clone()) { continue; }
            if self.cfg.presence.attribution && self.cfg.presence.enabled {
                let focused = focus.as_ref().map(|f| f.project_id == t.project_id && (f.workspace.is_empty() || f.workspace == t.workspace)).unwrap_or(false);
                if !focused || !presence_ok { continue; }
            }
            let last = self.last_heartbeat.get(&key).copied().unwrap_or(0);
            if now - last < heartbeat_ms { continue; }
            self.last_heartbeat.insert(key, now);
            let ev = ActivityEvent {
                ts: now,
                kind: "heartbeat".to_string(),
                project_id: t.project_id.clone(),
                project_name: t.project_name.clone(),
                workspace: t.workspace.clone(),
                repo: String::new(),
                rel_path: None, cmd: None, exit_code: None, duration_ms: None, count: 1, meta: None,
            };
            let _ = self.db.insert_event(&ev);
        }
        Ok(())
    }

    fn collect_targets(&self) -> Result<(Vec<RepoTarget>, usize, usize), String> {
        let mut targets = Vec::new();
        let mut known_ws_set: HashSet<String> = HashSet::new();
        let mut active_ws_set: HashSet<String> = HashSet::new();

        for p in list_registered_projects(None, true) {
            let mut cfg_file = p.join("ws-project.yml");
            if !cfg_file.exists() { cfg_file = p.join("ws-project.yaml"); }
            if !cfg_file.exists() { continue; }
            let Ok(config) = ConfigLoader::load_config(Some(&cfg_file), None, true) else { continue; };
            let project_name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".to_string());
            let project_id = ensure_project_id(&config).unwrap_or_else(|_| format!("path:{}", p.to_string_lossy()));
            let ws_root = config.workspaces_dir.clone();
            if !ws_root.is_dir() { continue; }
            let Ok(entries) = fs::read_dir(&ws_root) else { continue; };
            let manager = crate::workspace::WorkspaceManager::new(config.clone(), None);
            for entry in entries.flatten() {
                let ws_dir = entry.path();
                if !ws_dir.is_dir() { continue; }
                let ws_name = ws_dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let mut ws_file = ws_dir.join("workspace.yml");
                if !ws_file.exists() { ws_file = ws_dir.join("workspace.yaml"); }
                if !ws_file.exists() { continue; }
                let Ok(content) = fs::read_to_string(&ws_file) else { continue; };
                let Ok(meta) = crate::models::WorkspaceMetadata::from_yaml_str(&content) else { continue; };
                let ws_key = format!("{}::{}", project_id, ws_name);
                known_ws_set.insert(ws_key.clone());
                let active = manager.is_session_running(&ws_name);
                if active { active_ws_set.insert(ws_key.clone()); }
                for (repo_name, spec) in &meta.repositories {
                    let checkout = if spec.path.is_empty() { repo_name.clone() } else { spec.path.clone() };
                    let repo_path = ws_dir.join(&checkout);
                    if !repo_path.is_dir() { continue; }
                    targets.push(RepoTarget {
                        project_id: project_id.clone(),
                        project_name: project_name.clone(),
                        workspace: ws_name.clone(),
                        repo: repo_name.clone(),
                        path: repo_path,
                        active,
                    });
                }
            }
        }
        // prioritize active
        targets.sort_by_key(|t| (!t.active, t.project_name.clone(), t.workspace.clone(), t.repo.clone()));
        Ok((targets, known_ws_set.len(), active_ws_set.len()))
    }

    fn reconcile(&mut self) -> Result<(), String> {
        // refresh targets/watches and do a correctness scan
        let _ = self.discover_and_heartbeat();
        self.refresh_watches();
        // correctness scans (git + files) for all targets; active first
        self.scan_git_and_light(true)?;
        // full file scan (reconcile) for active targets; others opportunistic on later light scans
        self.scan_files_full(true)?;
        let _ = self.db.set_meta("watch_limited", if self.watch_limited { "1" } else { "0" });
        Ok(())
    }

    fn refresh_watches(&mut self) {
        let Some(ino) = self.inotify.as_mut() else {
            self.watch_limited = !self.targets.is_empty();
            let _ = self.db.set_meta("watch_limited", if self.watch_limited { "1" } else { "0" });
            return;
        };

        // drop existing watches and rebuild (simple and correct on dynamic changes)
        for wd in self.watch_by_wd.keys().cloned().collect::<Vec<_>>() {
            let _ = ino.watches().remove(wd);
        }
        self.watch_by_wd.clear();
        self.wd_by_dir.clear();
        self.watch_limited = false;

        let mask = WatchMask::CREATE
            | WatchMask::MODIFY
            | WatchMask::CLOSE_WRITE
            | WatchMask::DELETE
            | WatchMask::MOVED_FROM
            | WatchMask::MOVED_TO
            | WatchMask::MOVE_SELF
            | WatchMask::DELETE_SELF
            | WatchMask::ATTRIB;

        let mut watch_count = 0usize;
        for (idx, target) in self.targets.iter().enumerate() {
            if watch_count >= MAX_WATCHES {
                self.watch_limited = true;
                break;
            }
            // add recursive dir watches for this target repo
            let walker = WalkBuilder::new(&target.path)
                .hidden(false)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .ignore(true)
                .follow_links(false)
                .filter_entry(|e| {
                    let name = e.file_name();
                    name != ".git" && name != ".ws"
                })
                .build();
            for dent in walker.flatten() {
                if watch_count >= MAX_WATCHES { self.watch_limited = true; break; }
                let p = dent.path();
                if !p.is_dir() { continue; }
                // skip .git/.ws subtrees
                if p.components().any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".ws") { continue; }
                if self.wd_by_dir.contains_key(p) { continue; }
                match ino.watches().add(p, mask) {
                    Ok(wd) => {
                        let wd2 = wd.clone();
                        let wd3 = wd.clone();
                        self.wd_by_dir.insert(p.to_path_buf(), wd);
                        let _ = wd3;
                        self.watch_by_wd.insert(wd2, WatchCtx { target_idx: idx, dir: p.to_path_buf() });
                        watch_count += 1;
                    }
                    Err(_) => {
                        self.watch_limited = true;
                    }
                }
            }
        }
        let _ = self.db.set_meta("watch_limited", if self.watch_limited { "1" } else { "0" });
    }

    fn poll_inotify(&mut self) -> Result<(), String> {
        if self.inotify.is_none() { return Ok(()); }
        let mut overflow = false;
        let mut events_buf: Vec<(WatchDescriptor, EventMask, u32, Option<OsString>)> = Vec::new();
        {
            let ino = self.inotify.as_mut().unwrap();
            let mut buffer = [0u8; INOTIFY_BUF_SIZE];
            loop {
                match ino.read_events(&mut buffer) {
                    Ok(events) => {
                        let mut any = false;
                        for ev in events {
                            any = true;
                            if ev.mask.contains(EventMask::Q_OVERFLOW) {
                                overflow = true;
                                continue;
                            }
                            events_buf.push((ev.wd, ev.mask, ev.cookie, ev.name.map(|n| n.to_os_string())));
                        }
                        if !any { break; }
                        // continue reading until WouldBlock/error
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        if overflow {
            self.needs_rescan = true;
        }
        for (wd, mask, cookie, name) in events_buf {
            self.handle_inotify_event(wd, mask, cookie, name);
        }
        Ok(())
    }

    fn handle_inotify_event(&mut self, wd: WatchDescriptor, mask: EventMask, cookie: u32, name: Option<OsString>) {
        if mask.contains(EventMask::IGNORED) {
            if let Some(ctx) = self.watch_by_wd.remove(&wd) {
                self.wd_by_dir.remove(&ctx.dir);
            }
            return;
        }
        let Some(ctx) = self.watch_by_wd.get(&wd).cloned() else { return; };
        if ctx.target_idx >= self.targets.len() { return; }
        let Some(name) = name else {
            // event on watched dir itself (delete_self/move_self) -> rescan
            if mask.intersects(EventMask::DELETE_SELF | EventMask::MOVE_SELF) {
                self.needs_rescan = true;
            }
            return;
        };

        // ignore .git/.ws
        if name == ".git" || name == ".ws" { return; }
        let full_path = ctx.dir.join(&name);
        if full_path.components().any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".ws") { return; }

        // if directory created, try to add watch
        if mask.contains(EventMask::CREATE) && mask.contains(EventMask::ISDIR) {
            self.add_watch_recursive(ctx.target_idx, &full_path);
        }

        let target = &self.targets[ctx.target_idx];
        let rel = match full_path.strip_prefix(&target.path) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => return,
        };
        if rel.is_empty() { return; }
        // best-effort gitignore: rely on reconcile scans for full correctness; skip obvious non-file dir-only noise except deletes/moves
        let now = now_ms();

        if mask.contains(EventMask::MOVED_FROM) {
            self.recent_moves.push_back((cookie, MoveFrom { target_idx: ctx.target_idx, dir: ctx.dir.clone(), name, ts: Instant::now() }));
            return;
        }
        if mask.contains(EventMask::MOVED_TO) {
            // try pair
            if let Some(pos) = self.recent_moves.iter().position(|(c, _)| *c == cookie && cookie != 0) {
                let (_, from) = self.recent_moves.remove(pos).unwrap();
                self.emit_rename(from, ctx.target_idx, &ctx.dir, &full_path, &rel, now);
                return;
            } else {
                // treat as create/modify
                self.queue_file_event(ctx.target_idx, rel, "file.create", now);
                return;
            }
        }
        if mask.contains(EventMask::DELETE) {
            self.queue_file_event(ctx.target_idx, rel, "file.delete", now);
            return;
        }
        if mask.intersects(EventMask::CREATE | EventMask::MODIFY | EventMask::CLOSE_WRITE | EventMask::ATTRIB) {
            if mask.contains(EventMask::ISDIR) { return; }
            let kind = if mask.contains(EventMask::CREATE) { "file.create" } else { "file.modify" };
            self.queue_file_event(ctx.target_idx, rel, kind, now);
        }
    }

    fn add_watch_recursive(&mut self, target_idx: usize, dir: &Path) {
        let Some(ino) = self.inotify.as_mut() else { return; };
        if self.wd_by_dir.len() >= MAX_WATCHES { self.watch_limited = true; return; }
        let mask = WatchMask::CREATE
            | WatchMask::MODIFY
            | WatchMask::CLOSE_WRITE
            | WatchMask::DELETE
            | WatchMask::MOVED_FROM
            | WatchMask::MOVED_TO
            | WatchMask::MOVE_SELF
            | WatchMask::DELETE_SELF
            | WatchMask::ATTRIB;
        let walker = WalkBuilder::new(dir)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true)
            .follow_links(false)
            .filter_entry(|e| {
                let name = e.file_name();
                name != ".git" && name != ".ws"
            })
            .build();
        for dent in walker.flatten() {
            if self.wd_by_dir.len() >= MAX_WATCHES { self.watch_limited = true; break; }
            let p = dent.path();
            if !p.is_dir() { continue; }
            if p.components().any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".ws") { continue; }
            if self.wd_by_dir.contains_key(p) { continue; }
            match ino.watches().add(p, mask) {
                Ok(wd) => {
                    let wd2 = wd.clone();
                    let wd3 = wd.clone();
                    self.wd_by_dir.insert(p.to_path_buf(), wd);
                    let _ = wd3;
                    self.watch_by_wd.insert(wd2, WatchCtx { target_idx, dir: p.to_path_buf() });
                }
                Err(_) => { self.watch_limited = true; }
            }
        }
    }

    fn queue_file_event(&mut self, target_idx: usize, rel: String, kind: &str, ts_ms: i64) {
        if target_idx >= self.targets.len() { return; }
        let key = (target_idx, rel, kind.to_string());
        let entry = self.pending_by_key.entry(key).or_insert(PendingEvent { kind: kind.to_string(), ts_ms, count: 0 });
        entry.ts_ms = ts_ms.max(entry.ts_ms);
        entry.count = entry.count.saturating_add(1);
    }

    fn emit_move_from_as_delete(&mut self, from: MoveFrom) {
        if from.target_idx >= self.targets.len() { return; }
        let target = &self.targets[from.target_idx];
        let full = from.dir.join(&from.name);
        if let Ok(rel) = full.strip_prefix(&target.path) {
            let rels = rel.to_string_lossy().to_string();
            if !rels.is_empty() {
                self.queue_file_event(from.target_idx, rels, "file.delete", now_ms());
            }
        }
    }

    fn emit_rename(&mut self, from: MoveFrom, to_target_idx: usize, _to_dir: &Path, _to_full: &Path, to_rel: &str, ts_ms: i64) {
        // represent rename as delete+create for compatibility with existing rollup/categories
        self.emit_move_from_as_delete(from);
        if to_target_idx < self.targets.len() {
            self.queue_file_event(to_target_idx, to_rel.to_string(), "file.create", ts_ms);
        }
    }

    fn flush_pending_events(&mut self, force: bool) {
        if self.pending_by_key.is_empty() { return; }
        let debounce_ms = (self.cfg.debounce.max(1) * 1000) as i64;
        let now = now_ms();
        let keys: Vec<(usize, String, String)> = self.pending_by_key.keys().cloned().collect();
        for key in keys {
            let Some(p) = self.pending_by_key.get(&key).cloned() else { continue; };
            if !force && now - p.ts_ms < debounce_ms { continue; }
            self.pending_by_key.remove(&key);
            let (target_idx, rel, _k) = key;
            if target_idx >= self.targets.len() { continue; }
            let t = &self.targets[target_idx];
            let _ = self.db.insert_event(&ActivityEvent {
                ts: p.ts_ms.max(now - 1),
                kind: p.kind,
                project_id: t.project_id.clone(),
                project_name: t.project_name.clone(),
                workspace: t.workspace.clone(),
                repo: t.repo.clone(),
                rel_path: Some(rel),
                cmd: None, exit_code: None, duration_ms: None,
                count: p.count.max(1),
                meta: None,
            });
        }
    }

    fn scan_git_and_light(&mut self, full: bool) -> Result<(), String> {
        let now = now_ms();
        for (idx, t) in self.targets.clone().iter().enumerate() {
            if !full && !t.active { continue; }
            let scope_key = format!("{}::{}::{}", t.project_id, t.workspace, t.repo);
            self.scan_git(&scope_key, t, now);
            let _ = idx;
        }
        Ok(())
    }

    fn scan_files_full(&mut self, active_only: bool) -> Result<(), String> {
        let now = now_ms();
        for t in self.targets.clone() {
            if active_only && !t.active { continue; }
            self.scan_target_files(&t, now)?;
        }
        Ok(())
    }

    fn scan_target_files(&mut self, t: &RepoTarget, now: i64) -> Result<(), String> {
        let scope_key = format!("{}::{}::{}", t.project_id, t.workspace, t.repo);
        let state = self.file_state.entry(scope_key.clone()).or_insert_with(HashMap::new);
        let mut seen: HashMap<String, FileState> = HashMap::new();
        let debounce_ms = (self.cfg.debounce.max(1) * 1000) as i64;

        let walker = WalkBuilder::new(&t.path)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true)
            .follow_links(false)
            .build();
        for dent in walker.flatten() {
            let path = dent.path();
            if !path.is_file() { continue; }
            if path.components().any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".ws") { continue; }
            let Ok(md) = fs::metadata(path) else { continue; };
            let mtime_ms = md.modified().ok().and_then(|ts| ts.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
            let rel = path.strip_prefix(&t.path).unwrap_or(path).to_string_lossy().to_string();
            seen.insert(rel.clone(), FileState { mtime_ms, len: md.len() });
            match state.get(&rel) {
                None => {
                    if now - mtime_ms <= 300_000 {
                        // avoid initial flood: only recent creates; reconcile may see lots on first run, but bounded by recency
                        let _ = self.db.insert_event(&ActivityEvent {
                            ts: now.max(mtime_ms),
                            kind: "file.create".to_string(),
                            project_id: t.project_id.clone(),
                            project_name: t.project_name.clone(),
                            workspace: t.workspace.clone(),
                            repo: t.repo.clone(),
                            rel_path: Some(rel.clone()),
                            cmd: None, exit_code: None, duration_ms: None, count: 1, meta: None,
                        });
                    }
                }
                Some(prev) => {
                    if prev.mtime_ms != mtime_ms || prev.len != md.len() {
                        if mtime_ms >= prev.mtime_ms && now - mtime_ms <= debounce_ms.saturating_mul(30).max(120_000) {
                            let _ = self.db.insert_event(&ActivityEvent {
                                ts: now.max(mtime_ms),
                                kind: "file.modify".to_string(),
                                project_id: t.project_id.clone(),
                                project_name: t.project_name.clone(),
                                workspace: t.workspace.clone(),
                                repo: t.repo.clone(),
                                rel_path: Some(rel.clone()),
                                cmd: None, exit_code: None, duration_ms: None, count: 1, meta: None,
                            });
                        }
                    }
                }
            }
        }
        let deleted: Vec<String> = state.keys().filter(|k| !seen.contains_key(*k)).cloned().collect();
        for rel in deleted {
            state.remove(&rel);
            let _ = self.db.insert_event(&ActivityEvent {
                ts: now,
                kind: "file.delete".to_string(),
                project_id: t.project_id.clone(),
                project_name: t.project_name.clone(),
                workspace: t.workspace.clone(),
                repo: t.repo.clone(),
                rel_path: Some(rel),
                cmd: None, exit_code: None, duration_ms: None, count: 1, meta: None,
            });
        }
        *state = seen;
        Ok(())
    }

    fn scan_git(&mut self, scope_key: &str, t: &RepoTarget, now: i64) {
        let git_dir = resolve_git_dir(&t.path);
        let Some(git_dir) = git_dir else { return; };
        let head_path = git_dir.join("HEAD");
        let head = fs::read_to_string(&head_path).unwrap_or_default().trim().to_string();
        let index_mtime_ms = fs::metadata(git_dir.join("index")).ok()
            .and_then(|m| m.modified().ok())
            .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64).unwrap_or(0);
        let prev = self.git_state.get(scope_key).cloned().unwrap_or_default();
        let mut changed_head = false;
        let mut changed_index = false;
        if !prev.head.is_empty() && prev.head != head && !head.is_empty() {
            changed_head = true;
        }
        if prev.index_mtime_ms != 0 && index_mtime_ms != 0 && prev.index_mtime_ms != index_mtime_ms {
            changed_index = true;
        }
        if changed_head {
            let _ = self.db.insert_event(&ActivityEvent {
                ts: now, kind: "git.head".to_string(),
                project_id: t.project_id.clone(), project_name: t.project_name.clone(),
                workspace: t.workspace.clone(), repo: t.repo.clone(),
                rel_path: None, cmd: None, exit_code: None, duration_ms: None, count: 1,
                meta: Some(head.clone()),
            });
        }
        if changed_index {
            let _ = self.db.insert_event(&ActivityEvent {
                ts: now, kind: "git.index".to_string(),
                project_id: t.project_id.clone(), project_name: t.project_name.clone(),
                workspace: t.workspace.clone(), repo: t.repo.clone(),
                rel_path: None, cmd: None, exit_code: None, duration_ms: None, count: 1, meta: None,
            });
        }
        if !head.is_empty() || index_mtime_ms != 0 {
            self.git_state.insert(scope_key.to_string(), GitState { head, index_mtime_ms });
        }
    }

    fn accept_socket_records(&mut self) -> Result<(), String> {
        let Some(listener) = self.socket_listener.as_ref() else { return Ok(()); };
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let reader = BufReader::new(&stream);
                    for line in reader.lines().flatten() {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                            if v.get("type").and_then(|x| x.as_str()) == Some("presence_set") {
                                let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("active");
                                let ts = v.get("ts").and_then(|x| x.as_i64()).unwrap_or_else(now_ms);
                                let ps = super::presence::PresenceState::from_str(state);
                                self.presence.set_manual_state(&self.db, ts, ps);
                                continue;
                            }
                        }
                        let _ = self.ingest_record_line(&line);
                    }
                    let _ = stream.write_all(b"ok\n");
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        Ok(())
    }

    fn ingest_record_line(&self, line: &str) -> Result<(), String> {
        let line = line.trim();
        if line.is_empty() { return Ok(()); }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return Ok(()); };
        if v.get("type").and_then(|x| x.as_str()) == Some("command") {
            let cmd_raw = v.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
            let cmd = command_name_only(cmd_raw);
            let cwd = v.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let ts = v.get("ts").and_then(|x| x.as_i64()).unwrap_or_else(now_ms);
            let exit = v.get("exit").and_then(|x| x.as_i64()).unwrap_or(0);
            let dur = v.get("dur_ms").and_then(|x| x.as_i64()).unwrap_or(0);
            let (project_id, project_name, workspace, repo) = attribute_cwd(&cwd);
            if project_id.is_empty() { return Ok(()); }
            let ev = ActivityEvent {
                ts, kind: "command".to_string(), project_id, project_name, workspace, repo,
                rel_path: None,
                cmd: if cmd.is_empty() { None } else { Some(cmd) },
                exit_code: Some(exit), duration_ms: Some(dur), count: 1, meta: None,
            };
            let _ = self.db.insert_event(&ev);
        }
        Ok(())
    }

    fn drain_spool(&mut self) -> Result<(), String> {
        let spool = &self.paths.spool_path;
        let processing = spool.with_extension("jsonl.processing");

        // Recover any leftover processing file (e.g. crash mid-drain)
        if processing.is_file() {
            if let Ok(content) = fs::read_to_string(&processing) {
                for line in content.lines() { let _ = self.ingest_record_line(line); }
            }
            let _ = fs::remove_file(&processing);
        }

        if !spool.is_file() { return Ok(()); }
        // Move current spool aside so concurrent appends go to a fresh file
        if fs::rename(spool, &processing).is_err() { return Ok(()); }
        if let Ok(content) = fs::read_to_string(&processing) {
            for line in content.lines() { let _ = self.ingest_record_line(line); }
        }
        let _ = fs::remove_file(&processing);
        Ok(())
    }
}

fn resolve_git_dir(repo_path: &Path) -> Option<PathBuf> {
    let dot_git = repo_path.join(".git");
    if dot_git.is_dir() { return Some(dot_git); }
    if dot_git.is_file() {
        if let Ok(content) = fs::read_to_string(&dot_git) {
            for line in content.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("gitdir:") {
                    let p = PathBuf::from(rest.trim());
                    if p.is_absolute() { return Some(p); }
                    return Some(repo_path.join(p));
                }
            }
        }
    }
    None
}

fn ensure_project_id(config: &crate::models::AppConfig) -> Result<String, String> {
    if let Some(id) = config.activity.get("project_id").and_then(|v| v.as_str()) {
        if !id.trim().is_empty() { return Ok(id.trim().to_string()); }
    }
    let id = format!("act_{}", uuid::Uuid::new_v4());
    if let Some(cfg_path) = &config.config_file_path {
        if let Ok(content) = fs::read_to_string(cfg_path) {
            if let Some(updated) = super::sync::update_yaml_scalar_in_block_pub(&content, "activity", "project_id", &id) {
                let _ = fs::write(cfg_path, updated);
                return Ok(id);
            }
            if let Ok(mut doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                if let Some(map) = doc.as_mapping_mut() {
                    let act_key = serde_yaml::Value::String("activity".to_string());
                    let mut act_map = map.get(&act_key).and_then(|v| v.as_mapping()).cloned().unwrap_or_default();
                    act_map.insert(serde_yaml::Value::String("project_id".to_string()), serde_yaml::Value::String(id.clone()));
                    map.insert(act_key, serde_yaml::Value::Mapping(act_map));
                    if let Ok(out) = serde_yaml::to_string(&serde_yaml::Value::Mapping(map.clone())) {
                        let _ = fs::write(cfg_path, out);
                    }
                }
            }
        }
    }
    Ok(id)
}

fn attribute_cwd(cwd: &str) -> (String, String, String, String) {
    if cwd.is_empty() { return (String::new(), String::new(), String::new(), String::new()); }
    let cwd_path = PathBuf::from(cwd);
    let project_paths = list_registered_projects(None, true);
    for p in project_paths {
        let mut cfg_file = p.join("ws-project.yml");
        if !cfg_file.exists() { cfg_file = p.join("ws-project.yaml"); }
        let Ok(config) = ConfigLoader::load_config(Some(&cfg_file), None, true) else { continue; };
        let project_name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let ws_root = config.workspaces_dir.clone();
        if cwd_path.starts_with(&ws_root) {
            if let Ok(rel) = cwd_path.strip_prefix(&ws_root) {
                let comps: Vec<_> = rel.components().collect();
                if !comps.is_empty() {
                    let ws_name = comps[0].as_os_str().to_string_lossy().to_string();
                    let repo = if comps.len() >= 2 { comps[1].as_os_str().to_string_lossy().to_string() } else { String::new() };
                    let project_id = ensure_project_id(&config).unwrap_or_else(|_| format!("path:{}", p.to_string_lossy()));
                    return (project_id, project_name, ws_name, repo);
                }
            }
        }
        if cwd_path.starts_with(&p) {
            let project_id = ensure_project_id(&config).unwrap_or_else(|_| format!("path:{}", p.to_string_lossy()));
            return (project_id, project_name, String::new(), String::new());
        }
    }
    (String::new(), String::new(), String::new(), String::new())
}

pub(crate) fn command_name_only(s: &str) -> String {
    let tokens: Vec<&str> = s.trim().split_whitespace().collect();
    if tokens.is_empty() { return String::new(); }
    let wrappers = ["sudo", "env", "nohup", "nice", "time", "command", "builtin", "doas"];
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        let base = std::path::Path::new(t)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| t.to_string());
        // env-style assignments: FOO=bar
        if t.contains('=') && !t.starts_with('-') && !base.contains('=') == false {
            // if it's like VAR=val (base may equal t), skip
        }
        if t.contains('=') && !t.starts_with('-') {
            i += 1;
            continue;
        }
        if wrappers.contains(&base.as_str()) {
            i += 1;
            // skip simple sudo flags like -u <user>, -n, -E, -H (best-effort)
            while i < tokens.len() && tokens[i].starts_with('-') {
                let flag = tokens[i];
                i += 1;
                if matches!(flag, "-u" | "-g" | "-C" | "-D" | "-h" | "-p" | "-r" | "-t" | "-U") && i < tokens.len() {
                    i += 1;
                }
            }
            continue;
        }
        if t.starts_with('-') {
            i += 1;
            continue;
        }
        return std::path::Path::new(t)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| t.to_string());
    }
    String::new()
}


#[cfg(test)]
mod command_tests {
    use super::command_name_only;

    #[test]
    fn extracts_basic_name() {
        assert_eq!(command_name_only("cargo build --release"), "cargo");
        assert_eq!(command_name_only("/usr/bin/git status"), "git");
        assert_eq!(command_name_only("  ./script.sh arg  "), "script.sh");
    }

    #[test]
    fn strips_common_wrappers() {
        assert_eq!(command_name_only("sudo cargo build"), "cargo");
        assert_eq!(command_name_only("sudo -u root cargo build"), "cargo");
        assert_eq!(command_name_only("env FOO=bar make test"), "make");
        assert_eq!(command_name_only("nohup ./run.sh"), "run.sh");
        assert_eq!(command_name_only("FOO=1 BAR=2 cargo test"), "cargo");
    }

    #[test]
    fn empty_or_blank() {
        assert_eq!(command_name_only(""), "");
        assert_eq!(command_name_only("   "), "");
    }
}
