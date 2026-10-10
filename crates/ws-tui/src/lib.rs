pub mod bridge;
pub mod buffer;
pub mod clipboard;
pub mod daemon;
pub mod supervisor;
pub mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use daemon::{AttachedSessionClient, DaemonRequest, DaemonResponse, SessionDaemon};
pub use supervisor::{ManagedService, ProcessSupervisor, ServiceSpec, ServiceStatus};
pub use ui::WorkspaceTUI;

pub async fn run_workspace_tui(
    workspace_name: String,
    services: Vec<ServiceSpec>,
    log_dir: Option<PathBuf>,
    initial_focus: Option<String>,
    fullscreen: bool,
) -> Result<i32, String> {
    let mut supervisor = ProcessSupervisor::new(workspace_name.clone(), log_dir);

    for s in services {
        supervisor.register_service(s);
    }

    supervisor.start_all().await;

    let mut tui = WorkspaceTUI::new(workspace_name, &supervisor, initial_focus, fullscreen);
    let exit_code = tui.run().await.unwrap_or(1);

    supervisor.stop_all().await;
    Ok(exit_code)
}

pub async fn start_workspace_daemon(
    workspace_name: String,
    services: Vec<ServiceSpec>,
    socket_path: PathBuf,
    log_dir: Option<PathBuf>,
) -> Result<(), String> {
    let mut supervisor = ProcessSupervisor::new(workspace_name.clone(), log_dir);

    for s in services {
        supervisor.register_service(s);
    }

    supervisor.start_all().await;

    let daemon = SessionDaemon::new(workspace_name, Arc::new(supervisor), socket_path);

    daemon.run().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn attach_workspace_session(
    workspace_name: String,
    socket_path: PathBuf,
    initial_focus: Option<String>,
    fullscreen: bool,
) -> Result<i32, String> {
    let mut client =
        AttachedSessionClient::new(workspace_name, socket_path, initial_focus, fullscreen);
    client.run().await.map_err(|e| e.to_string())
}

pub fn is_session_active(socket_path: &Path) -> bool {
    if !socket_path.exists() {
        return false;
    }

    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(stream) => {
            use std::io::Write;
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(300)));
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(300)));
            let mut s = stream;
            if s.write_all(b"{\"type\":\"Ping\"}\n").is_ok() {
                true
            } else {
                let _ = std::fs::remove_file(socket_path);
                false
            }
        }
        Err(_) => {
            let _ = std::fs::remove_file(socket_path);
            false
        }
    }
}

pub async fn stop_workspace_session(socket_path: &Path) -> Result<bool, String> {
    if !socket_path.exists() {
        return Ok(false);
    }

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    if let Ok(stream) = tokio::net::UnixStream::connect(socket_path).await {
        let mut reader = BufReader::new(stream);
        let req = serde_json::to_string(&daemon::DaemonRequest::StopAll).unwrap_or_default() + "\n";
        let _ = reader.get_mut().write_all(req.as_bytes()).await;
        let _ = reader.get_mut().flush().await;

        let mut resp_line = String::new();
        let _ = reader.read_line(&mut resp_line).await;
        let _ = std::fs::remove_file(socket_path);
        Ok(true)
    } else {
        let _ = std::fs::remove_file(socket_path);
        Ok(false)
    }
}

pub fn run_raw_bridge(socket_path: &Path, service_name: &str) -> Result<i32, String> {
    bridge::run_raw_bridge(
        socket_path.to_string_lossy().to_string(),
        service_name.to_string(),
    )
}
