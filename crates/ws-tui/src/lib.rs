pub mod buffer;
pub mod clipboard;
pub mod daemon;
pub mod supervisor;
pub mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use pyo3::prelude::*;
use pyo3::exceptions::PyRuntimeError;

use daemon::{AttachedSessionClient, SessionDaemon};
use supervisor::{ProcessSupervisor, ServiceSpec};
use ui::WorkspaceTUI;

#[pyclass(name = "ServiceSpec")]
#[derive(Clone)]
pub struct PyServiceSpec {
    #[pyo3(get, set)]
    pub name: String,
    #[pyo3(get, set)]
    pub command: String,
    #[pyo3(get, set)]
    pub cwd: String,
    #[pyo3(get, set)]
    pub env: HashMap<String, String>,
}

#[pymethods]
impl PyServiceSpec {
    #[new]
    #[pyo3(signature = (name, command, cwd, env=None))]
    pub fn new(name: String, command: String, cwd: String, env: Option<HashMap<String, String>>) -> Self {
        Self {
            name,
            command,
            cwd,
            env: env.unwrap_or_default(),
        }
    }
}

#[pyfunction]
#[pyo3(signature = (workspace_name, services, log_dir=None, initial_focus=None, fullscreen=false))]
pub fn run_workspace_tui(
    workspace_name: String,
    services: Vec<PyServiceSpec>,
    log_dir: Option<String>,
    initial_focus: Option<String>,
    fullscreen: bool,
) -> PyResult<i32> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| PyRuntimeError::new_err(format!("Failed building Tokio runtime: {}", e)))?;

    rt.block_on(async move {
        let log_path = log_dir.map(PathBuf::from);
        let mut supervisor = ProcessSupervisor::new(workspace_name.clone(), log_path);

        for s in services {
            supervisor.register_service(ServiceSpec {
                name: s.name,
                command: s.command,
                cwd: s.cwd,
                env: s.env,
            });
        }

        supervisor.start_all().await;

        let mut tui = WorkspaceTUI::new(workspace_name, &supervisor, initial_focus, fullscreen);
        let exit_code = tui.run().await.unwrap_or(1);

        supervisor.stop_all().await;
        Ok(exit_code)
    })
}

#[pyfunction]
#[pyo3(signature = (workspace_name, services, socket_path, log_dir=None))]
pub fn start_workspace_daemon(
    workspace_name: String,
    services: Vec<PyServiceSpec>,
    socket_path: String,
    log_dir: Option<String>,
) -> PyResult<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| PyRuntimeError::new_err(format!("Failed building Tokio runtime: {}", e)))?;

    rt.block_on(async move {
        let log_path = log_dir.map(PathBuf::from);
        let mut supervisor = ProcessSupervisor::new(workspace_name.clone(), log_path);

        for s in services {
            supervisor.register_service(ServiceSpec {
                name: s.name,
                command: s.command,
                cwd: s.cwd,
                env: s.env,
            });
        }

        supervisor.start_all().await;

        let daemon = SessionDaemon::new(
            workspace_name,
            Arc::new(supervisor),
            PathBuf::from(socket_path),
        );

        daemon.run().await.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(())
    })
}

#[pyfunction]
#[pyo3(signature = (workspace_name, socket_path, initial_focus=None, fullscreen=false))]
pub fn attach_workspace_session(
    workspace_name: String,
    socket_path: String,
    initial_focus: Option<String>,
    fullscreen: bool,
) -> PyResult<i32> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| PyRuntimeError::new_err(format!("Failed building Tokio runtime: {}", e)))?;

    rt.block_on(async move {
        let mut client = AttachedSessionClient::new(
            workspace_name,
            PathBuf::from(socket_path),
            initial_focus,
            fullscreen,
        );
        let exit_code = client.run().await.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(exit_code)
    })
}


#[pyfunction]
pub fn is_session_active(socket_path: String) -> bool {
    let sock = PathBuf::from(&socket_path);
    if !sock.exists() {
        return false;
    }

    match std::os::unix::net::UnixStream::connect(&sock) {
        Ok(stream) => {
            use std::io::Write;
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(300)));
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(300)));
            let mut s = stream;
            if s.write_all(b"{\"type\":\"Ping\"}\n").is_ok() {
                true
            } else {
                let _ = std::fs::remove_file(&sock);
                false
            }
        }
        Err(_) => {
            let _ = std::fs::remove_file(&sock);
            false
        }
    }
}


#[pyfunction]
pub fn stop_workspace_session(socket_path: String) -> PyResult<bool> {
    let sock = PathBuf::from(socket_path);
    if !sock.exists() {
        return Ok(false);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| PyRuntimeError::new_err(format!("Failed building Tokio runtime: {}", e)))?;

    rt.block_on(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        if let Ok(stream) = tokio::net::UnixStream::connect(&sock).await {
            let mut reader = BufReader::new(stream);
            let req = serde_json::to_string(&daemon::DaemonRequest::StopAll).unwrap_or_default() + "\n";
            let _ = reader.get_mut().write_all(req.as_bytes()).await;
            let _ = reader.get_mut().flush().await;

            let mut resp_line = String::new();
            let _ = reader.read_line(&mut resp_line).await;
            let _ = std::fs::remove_file(&sock);
            Ok(true)
        } else {
            let _ = std::fs::remove_file(&sock);
            Ok(false)
        }
    })
}


#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyServiceSpec>()?;
    m.add_function(wrap_pyfunction!(run_workspace_tui, m)?)?;
    m.add_function(wrap_pyfunction!(start_workspace_daemon, m)?)?;
    m.add_function(wrap_pyfunction!(attach_workspace_session, m)?)?;
    m.add_function(wrap_pyfunction!(is_session_active, m)?)?;
    m.add_function(wrap_pyfunction!(stop_workspace_session, m)?)?;
    Ok(())
}

