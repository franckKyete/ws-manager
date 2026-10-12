use crate::errors::WSError;
use crate::utils::ensure_directory;
use std::path::PathBuf;
use std::process::Command;

pub const SERVICE_NAME: &str = "ws.service";

pub fn get_user_systemd_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".config").join("systemd").join("user"))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn get_service_path() -> PathBuf {
    get_user_systemd_dir().join(SERVICE_NAME)
}

pub fn is_command_available(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn is_systemctl_available() -> bool {
    is_command_available("systemctl")
}

pub fn find_ws_binary() -> String {
    if is_command_available("ws") {
        if let Ok(output) = Command::new("which").arg("ws").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return path;
                }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let fallback = home.join(".local").join("bin").join("ws");
        if fallback.exists() {
            return fallback.to_string_lossy().to_string();
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        return current_exe.to_string_lossy().to_string();
    }

    "ws".to_string()
}

pub fn generate_service_unit(ws_exec: Option<&str>) -> String {
    let exec_path = match ws_exec {
        Some(p) => p.to_string(),
        None => find_ws_binary(),
    };

    format!(
        r#"[Unit]
Description=ws Global Background Daemon
Documentation=https://github.com/franckKyete/ws-manager
After=network.target

[Service]
Type=simple
ExecStart={} daemon
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
"#,
        exec_path
    )
}

pub fn install_service(ws_exec: Option<&str>) -> Result<(bool, String), WSError> {
    if !is_systemctl_available() {
        return Ok((
            false,
            "systemctl is not available on this system.".to_string(),
        ));
    }

    let service_dir = get_user_systemd_dir();
    ensure_directory(&service_dir)?;
    let service_path = get_service_path();
    let unit_content = generate_service_unit(ws_exec);
    std::fs::write(&service_path, unit_content)?;

    // Reload systemd user daemon
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    let res_enable = Command::new("systemctl")
        .args(["--user", "enable", "--now", SERVICE_NAME])
        .output()?;

    if !res_enable.status.success() {
        let err = String::from_utf8_lossy(&res_enable.stderr)
            .trim()
            .to_string();
        let out = String::from_utf8_lossy(&res_enable.stdout)
            .trim()
            .to_string();
        let msg = if !err.is_empty() { err } else { out };
        return Ok((false, format!("Failed enabling {}: {}", SERVICE_NAME, msg)));
    }

    Ok((
        true,
        format!("Installed and started {} successfully.", SERVICE_NAME),
    ))
}

pub fn uninstall_service() -> Result<(bool, String), WSError> {
    let service_path = get_service_path();
    if !is_systemctl_available() {
        if service_path.exists() {
            let _ = std::fs::remove_file(&service_path);
            return Ok((true, format!("Removed {}.", service_path.display())));
        }
        return Ok((
            false,
            "systemctl is not available on this system.".to_string(),
        ));
    }

    let _ = Command::new("systemctl")
        .args(["--user", "stop", SERVICE_NAME])
        .output();
    let _ = Command::new("systemctl")
        .args(["--user", "disable", SERVICE_NAME])
        .output();

    if service_path.exists() {
        let _ = std::fs::remove_file(&service_path);
    }

    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    Ok((true, format!("Uninstalled {} successfully.", SERVICE_NAME)))
}

pub fn control_service(action: &str) -> Result<(bool, String), WSError> {
    if !is_systemctl_available() {
        return Ok((
            false,
            "systemctl is not available on this system.".to_string(),
        ));
    }

    let service_path = get_service_path();
    if !service_path.exists() && matches!(action, "start" | "restart" | "enable") {
        return Ok((
            false,
            format!(
                "{} is not installed. Run 'ws service install' first.",
                SERVICE_NAME
            ),
        ));
    }

    let res = Command::new("systemctl")
        .args(["--user", action, SERVICE_NAME])
        .output()?;

    if !res.status.success() {
        let err = String::from_utf8_lossy(&res.stderr).trim().to_string();
        let out = String::from_utf8_lossy(&res.stdout).trim().to_string();
        let msg = if !err.is_empty() { err } else { out };
        return Ok((false, format!("systemctl {} failed: {}", action, msg)));
    }

    Ok((
        true,
        format!("Successfully executed '{}' on {}.", action, SERVICE_NAME),
    ))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub available: bool,
    pub installed: bool,
    pub active: bool,
    pub enabled: bool,
    pub details: String,
    pub unit_path: String,
}

pub fn get_service_status() -> ServiceStatus {
    let service_path = get_service_path();
    if !is_systemctl_available() {
        return ServiceStatus {
            available: false,
            installed: service_path.exists(),
            active: false,
            enabled: false,
            details: "systemctl is not available".to_string(),
            unit_path: service_path.to_string_lossy().to_string(),
        };
    }

    let installed = service_path.exists();
    if !installed {
        return ServiceStatus {
            available: true,
            installed: false,
            active: false,
            enabled: false,
            details: format!(
                "{} is not installed (run 'ws service install')",
                SERVICE_NAME
            ),
            unit_path: service_path.to_string_lossy().to_string(),
        };
    }

    let is_active = Command::new("systemctl")
        .args(["--user", "is-active", SERVICE_NAME])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false);

    let is_enabled = Command::new("systemctl")
        .args(["--user", "is-enabled", SERVICE_NAME])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false);

    let details = Command::new("systemctl")
        .args(["--user", "status", SERVICE_NAME])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    ServiceStatus {
        available: true,
        installed: true,
        active: is_active,
        enabled: is_enabled,
        details,
        unit_path: service_path.to_string_lossy().to_string(),
    }
}

pub fn stream_service_logs(follow: bool, lines: usize) -> Result<i32, WSError> {
    if !is_command_available("journalctl") {
        eprintln!("journalctl is not available on this system.");
        return Ok(1);
    }

    let lines_str = lines.to_string();
    let mut args = vec!["--user", "-u", SERVICE_NAME, "-n", &lines_str, "--no-pager"];
    if follow {
        args.push("-f");
    }

    let status = Command::new("journalctl").args(&args).status()?;

    Ok(status.code().unwrap_or(0))
}
