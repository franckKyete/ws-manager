use ws_core::daemon::run_daemon_loop;

pub fn execute_daemon(interval: Option<u64>) -> Result<(), String> {
    let tick = interval.unwrap_or(3);
    run_daemon_loop(tick);
    Ok(())
}
