/// Concurrent Process Supervisor with PTY allocation and process group lifecycle.

use std::collections::HashMap;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use regex::Regex;
use tokio::sync::{Mutex, RwLock};

use crate::buffer::VirtualLineBuffer;

lazy_static::lazy_static! {
    static ref PORT_REGEX: Regex = Regex::new(
        r"(?i)(?:https?://(?:localhost|127\.0\.0\.1|0\.0\.0\.0):(\d{2,5})|localhost:(\d{2,5})|(?:port|PORT)\s*(?:=|:|\s)\s*(\d{2,5})|listening on\s*:?(\d{2,5}))"
    ).unwrap();
}

#[derive(Debug, Clone)]
pub struct ServiceSpec {
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Starting,
    Running,
    Stopped(i32),
    Failed(String),
}

pub struct ManagedService {
    pub spec: ServiceSpec,
    pub status: Arc<RwLock<ServiceStatus>>,
    pub buffer: Arc<RwLock<VirtualLineBuffer>>,
    pub detected_port: Arc<AtomicU32>,
    pub master_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    pub pid: Arc<Mutex<Option<u32>>>,
    pub start_time: Instant,
}

impl ManagedService {
    pub fn new(spec: ServiceSpec) -> Self {
        Self {
            spec,
            status: Arc::new(RwLock::new(ServiceStatus::Starting)),
            buffer: Arc::new(RwLock::new(VirtualLineBuffer::default())),
            detected_port: Arc::new(AtomicU32::new(0)),
            master_writer: Arc::new(Mutex::new(None)),
            pid: Arc::new(Mutex::new(None)),
            start_time: Instant::now(),
        }
    }

    pub async fn send_input(&self, data: &[u8]) -> bool {
        let mut writer_guard = self.master_writer.lock().await;
        if let Some(writer) = writer_guard.as_mut() {
            if writer.write_all(data).is_ok() && writer.flush().is_ok() {
                return true;
            }
        }
        false
    }
}

pub struct ProcessSupervisor {
    pub workspace_name: String,
    pub log_dir: Option<PathBuf>,
    pub services: HashMap<String, Arc<ManagedService>>,
}

impl ProcessSupervisor {
    pub fn new(workspace_name: String, log_dir: Option<PathBuf>) -> Self {
        if let Some(dir) = &log_dir {
            let _ = create_dir_all(dir);
        }
        Self {
            workspace_name,
            log_dir,
            services: HashMap::new(),
        }
    }

    pub fn register_service(&mut self, spec: ServiceSpec) -> Arc<ManagedService> {
        let service = Arc::new(ManagedService::new(spec.clone()));
        self.services.insert(spec.name.clone(), service.clone());
        service
    }

    pub async fn start_service(&self, name: &str) -> bool {
        let service = match self.services.get(name) {
            Some(s) => s.clone(),
            None => return false,
        };

        // Stop if running
        self.stop_service(name).await;

        {
            let mut status_guard = service.status.write().await;
            *status_guard = ServiceStatus::Starting;
        }

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: 50,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        }) {

            Ok(p) => p,
            Err(e) => {
                let mut status_guard = service.status.write().await;
                *status_guard = ServiceStatus::Failed(format!("Failed to open PTY: {}", e));
                return false;
            }
        };

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["-c", &service.spec.command]);
        cmd.cwd(&service.spec.cwd);

        for (k, v) in &service.spec.env {
            cmd.env(k, v);
        }
        cmd.env("WORKSPACE_NAME", &self.workspace_name);
        cmd.env("REPO_NAME", &service.spec.name);
        cmd.env("FORCE_COLOR", "1");
        cmd.env("PYTHONUNBUFFERED", "1");

        let child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(e) => {
                let mut status_guard = service.status.write().await;
                *status_guard = ServiceStatus::Failed(format!("Failed to spawn command: {}", e));
                return false;
            }
        };

        let pid = child.process_id();
        {
            let mut pid_guard = service.pid.lock().await;
            *pid_guard = pid;
        }

        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(e) => {
                let mut status_guard = service.status.write().await;
                *status_guard = ServiceStatus::Failed(format!("Failed to take PTY writer: {}", e));
                return false;
            }
        };

        {
            let mut writer_guard = service.master_writer.lock().await;
            *writer_guard = Some(writer);
        }

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                let mut status_guard = service.status.write().await;
                *status_guard = ServiceStatus::Failed(format!("Failed to take PTY reader: {}", e));
                return false;
            }
        };

        {
            let mut status_guard = service.status.write().await;
            *status_guard = ServiceStatus::Running;
        }

        // Spawn background reader thread
        let buffer_clone = service.buffer.clone();
        let port_clone = service.detected_port.clone();
        let status_clone = service.status.clone();
        let log_file_path = self.log_dir.as_ref().map(|d| d.join(format!("{}.log", service.spec.name)));

        tokio::task::spawn_blocking(move || {
            let mut file_handle = log_file_path.and_then(|p| {
                OpenOptions::new().create(true).append(true).open(p).ok()
            });

            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buf[..n]);

                // Port sniffing
                if port_clone.load(Ordering::Relaxed) == 0 {
                    if let Some(captures) = PORT_REGEX.captures(&text) {
                        for i in 1..=4 {
                            if let Some(m) = captures.get(i) {
                                if let Ok(port) = m.as_str().parse::<u32>() {
                                    if port > 80 {
                                        port_clone.store(port, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // Feed VT100 virtual terminal line buffer
                if let Ok(mut buf_guard) = buffer_clone.try_write() {
                    buf_guard.feed_bytes(&buf[..n]);
                }


                if let Some(fh) = file_handle.as_mut() {
                    let _ = fh.write_all(&buf[..n]);
                }
            }

            // Mark stopped
            let mut mut_child = child;
            let exit_code = mut_child.wait().map(|status| status.exit_code() as i32).unwrap_or(0);
            if let Ok(mut status_guard) = status_clone.try_write() {
                *status_guard = ServiceStatus::Stopped(exit_code);
            }
        });

        true
    }

    pub async fn start_all(&self) {
        for name in self.services.keys() {
            self.start_service(name).await;
        }
    }

    pub async fn stop_service(&self, name: &str) -> bool {
        let service = match self.services.get(name) {
            Some(s) => s,
            None => return true,
        };

        let pid_val = {
            let mut pid_guard = service.pid.lock().await;
            pid_guard.take()
        };

        if let Some(pid) = pid_val {
            let pid_i32 = pid as i32;
            let target_pid = Pid::from_raw(pid_i32);

            // 1. Send SIGINT and SIGTERM to process group and direct PID
            let _ = killpg(target_pid, Signal::SIGINT);
            let _ = nix::sys::signal::kill(target_pid, Signal::SIGINT);
            let _ = killpg(target_pid, Signal::SIGTERM);
            let _ = nix::sys::signal::kill(target_pid, Signal::SIGTERM);

            // 2. Poll for termination for up to 3.5 seconds
            let start = tokio::time::Instant::now();
            let timeout = tokio::time::Duration::from_millis(3500);
            let mut exited = false;

            while tokio::time::Instant::now().duration_since(start) < timeout {
                // Check if process still exists
                if nix::sys::signal::kill(target_pid, None).is_err() {
                    exited = true;
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            // 3. Fallback to SIGKILL if still running
            if !exited {
                let _ = killpg(target_pid, Signal::SIGKILL);
                let _ = nix::sys::signal::kill(target_pid, Signal::SIGKILL);
            }
        }

        {
            let mut status_guard = service.status.write().await;
            *status_guard = ServiceStatus::Stopped(0);
        }
        true
    }

    pub async fn stop_all(&self) {
        let mut handles = Vec::new();
        for service in self.services.values() {
            let s = service.clone();
            handles.push(tokio::spawn(async move {
                let pid_val = {
                    let mut pid_guard = s.pid.lock().await;
                    pid_guard.take()
                };

                if let Some(pid) = pid_val {
                    let pid_i32 = pid as i32;
                    let target_pid = Pid::from_raw(pid_i32);

                    let _ = killpg(target_pid, Signal::SIGINT);
                    let _ = nix::sys::signal::kill(target_pid, Signal::SIGINT);
                    let _ = killpg(target_pid, Signal::SIGTERM);
                    let _ = nix::sys::signal::kill(target_pid, Signal::SIGTERM);

                    let start = tokio::time::Instant::now();
                    let timeout = tokio::time::Duration::from_millis(3500);
                    let mut exited = false;

                    while tokio::time::Instant::now().duration_since(start) < timeout {
                        if nix::sys::signal::kill(target_pid, None).is_err() {
                            exited = true;
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }

                    if !exited {
                        let _ = killpg(target_pid, Signal::SIGKILL);
                        let _ = nix::sys::signal::kill(target_pid, Signal::SIGKILL);
                    }
                }

                if let Ok(mut status_guard) = s.status.try_write() {
                    *status_guard = ServiceStatus::Stopped(0);
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }
    }


    pub async fn send_input(&self, name: &str, data: &[u8]) -> bool {
        if let Some(service) = self.services.get(name) {
            return service.send_input(data).await;
        }
        false
    }
}
