use ws_core::daemon::{
    get_daemon_pid_file, is_standalone_daemon_running, run_daemon_loop,
};
use ws_core::utils::ensure_directory;

pub fn execute_daemon(interval: Option<u64>) -> Result<(), String> {
    let tick = interval.unwrap_or(3);

    let (active, existing_pid) = is_standalone_daemon_running();
    let current_pid = std::process::id() as i32;
    if active && existing_pid != Some(current_pid) {
        return Err(format!(
            "ws daemon is already running (PID {}).",
            existing_pid.unwrap_or(0)
        ));
    }

    let pid_file = get_daemon_pid_file();
    if let Some(parent) = pid_file.parent() {
        ensure_directory(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&pid_file, current_pid.to_string()).map_err(|e| e.to_string())?;

    run_daemon_loop(tick);

    // clean up pid file on exit (only if it's still ours)
    if pid_file.is_file() {
        if let Ok(content) = std::fs::read_to_string(&pid_file) {
            if content.trim().parse::<i32>().ok() == Some(current_pid) {
                let _ = std::fs::remove_file(&pid_file);
            }
        }
    }
    Ok(())
}
