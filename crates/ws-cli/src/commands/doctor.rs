use colored::Colorize;
use ws_core::network::{get_lan_ip, list_network_interfaces};
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_doctor(manager: &WorkspaceManager) -> Result<(), String> {
    println!("{}", "System Diagnostics & Health Check".bold().cyan());
    println!();

    let results = manager.doctor();
    let mut all_ok = true;
    let mut keys: Vec<_> = results.keys().collect();
    keys.sort();

    for k in keys {
        let ok = results[k];
        let badge = if ok {
            "PASS".bold().green()
        } else {
            all_ok = false;
            "FAIL".bold().red()
        };
        println!("  {:32} : {}", k, badge);
    }

    println!();
    println!("{}", "Network Discovery & Interfaces".bold().cyan());
    let ifaces = list_network_interfaces();
    if ifaces.is_empty() {
        println!("  No active network interfaces detected.");
    } else {
        for iface in ifaces {
            let w_badge = if iface.iface_type == ws_core::network::InterfaceType::Wireless {
                " (Wi-Fi)".cyan()
            } else {
                "".white()
            };
            println!(
                "  {:12} : {} ({:?}){}",
                iface.name.bold(),
                iface.ip,
                iface.iface_type,
                w_badge
            );
        }
    }

    let default_lan = get_lan_ip(None);
    println!("  Default Resolved LAN IP: {}", default_lan.bold().green());

    println!();
    if all_ok {
        OutputHandler::print_success("All diagnostic checks passed.");
    } else {
        OutputHandler::print_warning("Some diagnostic checks reported warnings or failures.");
    }

    Ok(())
}

pub fn execute_antigravity(manager: &WorkspaceManager) -> Result<(), String> {
    println!(
        "{}",
        "🚀 Google Antigravity Agent Workspace Diagnostic"
            .bold()
            .green()
    );
    println!("Workspace Manager version: {}", "0.1.0".cyan());
    println!(
        "Repositories active: {}",
        manager.config.repositories.len().to_string().yellow()
    );
    println!(
        "Workspaces registered: {}",
        manager.list_workspaces().len().to_string().yellow()
    );
    OutputHandler::print_success("Antigravity engine nominal.");
    Ok(())
}
