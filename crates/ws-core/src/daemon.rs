use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::ConfigLoader;
use crate::errors::WSError;
use crate::registry::list_registered_projects;
use crate::utils::{ensure_directory, get_iso_timestamp};
use crate::watcher::ConfigFileWatcher;
use crate::workspace::WorkspaceManager;

pub fn get_global_config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".config").join("ws"))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn get_daemon_pid_file() -> PathBuf {
    get_global_config_dir().join("daemon.pid")
}

pub fn get_daemon_log_file() -> PathBuf {
    get_global_config_dir().join("daemon.log")
}

pub fn get_global_cache_file() -> PathBuf {
    get_global_config_dir().join("global_auto_save_cache.json")
}

pub fn get_global_cache() -> HashMap<String, serde_json::Value> {
    let path = get_global_cache_file();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&content) {
                return val;
            }
        }
    }
    HashMap::new()
}

pub fn save_global_cache(cache: &HashMap<String, serde_json::Value>) {
    let path = get_global_cache_file();
    if let Some(parent) = path.parent() {
        let _ = ensure_directory(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(&path, content);
    }
}

pub fn is_standalone_daemon_running() -> (bool, Option<i32>) {
    let pid_file = get_daemon_pid_file();
    if !pid_file.exists() {
        return (false, None);
    }

    if let Ok(content) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            let res = unsafe { libc::kill(pid, 0) };
            if res == 0 {
                return (true, Some(pid));
            } else {
                let _ = std::fs::remove_file(&pid_file);
                return (false, None);
            }
        }
    }
    (false, None)
}

pub struct GlobalAutoSaveWorker {
    cache: HashMap<String, serde_json::Value>,
}

impl Default for GlobalAutoSaveWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalAutoSaveWorker {
    pub fn new() -> Self {
        Self {
            cache: get_global_cache(),
        }
    }

    pub fn process_all_projects(&mut self) -> HashMap<String, HashMap<String, bool>> {
        let project_paths = list_registered_projects(None, true);
        let mut results = HashMap::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let mut cache_updated = false;

        for p in project_paths {
            let cfg_file = p.join("repositories.yml");
            if !cfg_file.exists() {
                continue;
            }

            let config = match ConfigLoader::load_config(Some(&cfg_file), None, true) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let auto_cfg = match &config.hub_auto_save {
                Some(a) if a.enabled => a,
                _ => continue,
            };

            let interval = if auto_cfg.interval > 0 {
                auto_cfg.interval as f64
            } else {
                300.0
            };

            let p_str = p.to_string_lossy().to_string();
            let mut p_entry = self
                .cache
                .get(&p_str)
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();

            let last_checked = p_entry
                .get("last_checked_at")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            if now - last_checked < interval {
                continue;
            }

            p_entry.insert("last_checked_at".to_string(), serde_json::Value::from(now));
            cache_updated = true;

            let manager = WorkspaceManager::new(config, None);
            let ws_results = manager.hub_auto_save_all_workspaces(None, false, true);

            let proj_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            results.insert(proj_name.clone(), ws_results.clone());

            let any_saved = ws_results.values().any(|&s| s);
            if any_saved {
                p_entry.insert(
                    "last_saved_at".to_string(),
                    serde_json::Value::String(get_iso_timestamp()),
                );
            }

            self.cache.insert(p_str, serde_json::Value::Object(p_entry));
        }

        if cache_updated {
            save_global_cache(&self.cache);
        }

        results
    }
}

pub fn run_daemon_loop(tick_seconds: u64) {
    let running = Arc::new(AtomicBool::new(true));
    let r_clone = Arc::clone(&running);

    // Register signal handlers for clean shutdown
    ctrlc_compat(move || {
        r_clone.store(false, Ordering::SeqCst);
    });

    let mut auto_save_worker = GlobalAutoSaveWorker::new();
    let mut file_watcher = ConfigFileWatcher::new(None);
    file_watcher.initialize();

    // Initial check on startup
    let _ = auto_save_worker.process_all_projects();

    let mut last_auto_save_run = Instant::now();
    let tick_dur = Duration::from_secs(tick_seconds);

    while running.load(Ordering::SeqCst) {
        // 1. Check for immediate config / workspace file modifications
        file_watcher.check_changes();

        // 2. Check for periodic auto-save passes
        if last_auto_save_run.elapsed() >= tick_dur {
            let _ = auto_save_worker.process_all_projects();
            last_auto_save_run = Instant::now();
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

fn ctrlc_compat<F: Fn() + Send + Sync + 'static>(handler: F) {
    let handler = Arc::new(handler);
    unsafe {
        // Simple SIGINT and SIGTERM handlers
        libc::signal(libc::SIGINT, libc_sig_handler as *const () as usize);
        libc::signal(libc::SIGTERM, libc_sig_handler as *const () as usize);
    }
    HANDLER.lock().unwrap().replace(handler);
}

lazy_static::lazy_static! {
    static ref HANDLER: std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = std::sync::Mutex::new(None);
}

extern "C" fn libc_sig_handler(_sig: libc::c_int) {
    if let Ok(guard) = HANDLER.lock() {
        if let Some(h) = guard.as_ref() {
            h();
        }
    }
}

pub fn start_standalone_daemon(detached: bool) -> Result<i32, WSError> {
    let (active, existing_pid) = is_standalone_daemon_running();
    let current_pid = std::process::id() as i32;
    if active && existing_pid != Some(current_pid) {
        return Err(WSError::Workspace(format!(
            "ws daemon is already running (PID {}).",
            existing_pid.unwrap_or(0)
        )));
    }

    let pid_file = get_daemon_pid_file();
    ensure_directory(pid_file.parent().unwrap())?;

    if detached {
        let log_file = get_daemon_log_file();
        let log_out = File::options().create(true).append(true).open(&log_file)?;

        let current_exe = std::env::current_exe()?;
        let child = std::process::Command::new(current_exe)
            .args(["daemon", "run"])
            .stdout(log_out.try_clone()?)
            .stderr(log_out)
            .spawn()?;

        let pid = child.id() as i32;
        std::fs::write(&pid_file, pid.to_string())?;
        Ok(pid)
    } else {
        std::fs::write(&pid_file, current_pid.to_string())?;
        run_daemon_loop(15);
        if pid_file.exists() {
            let _ = std::fs::remove_file(&pid_file);
        }
        Ok(current_pid)
    }
}

pub fn stop_standalone_daemon() -> Result<bool, WSError> {
    let (active, pid) = is_standalone_daemon_running();
    if !active || pid.is_none() {
        return Ok(false);
    }

    let target_pid = pid.unwrap();
    unsafe {
        libc::kill(target_pid, libc::SIGTERM);
    }

    let pid_file = get_daemon_pid_file();
    if pid_file.exists() {
        let _ = std::fs::remove_file(&pid_file);
    }
    Ok(true)
}
