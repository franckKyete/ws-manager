use colored::Colorize;
use ws_core::output::OutputHandler;
use ws_core::systemd::{
    control_service, get_service_status, install_service, stream_service_logs, uninstall_service,
};

pub fn execute_service_install(exec_path: Option<&str>) -> Result<(), String> {
    let (ok, msg) = install_service(exec_path).map_err(|e| e.to_string())?;
    if ok {
        OutputHandler::print_success(&msg);
    } else {
        OutputHandler::print_err(&msg);
    }
    Ok(())
}

pub fn execute_service_uninstall() -> Result<(), String> {
    let (ok, msg) = uninstall_service().map_err(|e| e.to_string())?;
    if ok {
        OutputHandler::print_success(&msg);
    } else {
        OutputHandler::print_err(&msg);
    }
    Ok(())
}

pub fn execute_service_control(action: &str) -> Result<(), String> {
    let (ok, msg) = control_service(action).map_err(|e| e.to_string())?;
    if ok {
        OutputHandler::print_success(&msg);
    } else {
        OutputHandler::print_err(&msg);
    }
    Ok(())
}

pub fn execute_service_status() -> Result<(), String> {
    let status = get_service_status();
    println!(
        "{}",
        "systemd User Service Status: ws.service".bold().cyan()
    );
    println!(
        "  Installed: {}",
        if status.installed {
            "Yes".green()
        } else {
            "No".red()
        }
    );
    println!(
        "  Active:    {}",
        if status.active {
            "active (running)".green()
        } else {
            "inactive".yellow()
        }
    );
    println!(
        "  Enabled:   {}",
        if status.enabled {
            "enabled".green()
        } else {
            "disabled".dimmed()
        }
    );
    if !status.unit_path.is_empty() {
        println!("  Unit Path: {}", status.unit_path);
    }
    if !status.details.is_empty() {
        println!("  Details:   {}", status.details);
    }
    Ok(())
}

pub fn execute_service_logs(follow: bool, lines: usize) -> Result<(), String> {
    let _ = stream_service_logs(follow, lines).map_err(|e| e.to_string())?;
    Ok(())
}
