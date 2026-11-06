use crate::helpers::resolve_ws_and_repo_args;
use colored::Colorize;
use comfy_table::{Cell, Color};
use std::fs;
use std::path::Path;
use ws_core::hub::HubClient;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_hub_discover(timeout_ms: u64, set: bool) -> Result<(), String> {
    OutputHandler::print_info(&format!(
        "Scanning for wshub servers on local network (LAN) (timeout: {}ms)...",
        timeout_ms
    ));
    let hubs = ws_core::hub_discovery::discover_local_hubs(timeout_ms, 8788);
    if hubs.is_empty() {
        OutputHandler::print_info("No wshub servers discovered on local network.");
        return Ok(());
    }

    let mut table = OutputHandler::create_table();
    table.set_header(vec![
        Cell::new("Hostname").fg(Color::Cyan),
        Cell::new("Primary URL").fg(Color::Cyan),
        Cell::new("Version").fg(Color::Cyan),
        Cell::new("Latency").fg(Color::Cyan),
        Cell::new("IPs").fg(Color::Cyan),
    ]);

    for hub in &hubs {
        table.add_row(vec![
            Cell::new(&hub.hostname).fg(Color::Green),
            Cell::new(&hub.url).fg(Color::White),
            Cell::new(&hub.version).fg(Color::Yellow),
            Cell::new(format!("{} ms", hub.latency_ms)).fg(Color::Blue),
            Cell::new(hub.ips.join(", ")),
        ]);
    }

    println!("{table}");

    if set {
        let best_hub = &hubs[0];
        let client = HubClient::default();
        client.update_active_url(&best_hub.url);
        OutputHandler::print_success(&format!(
            "Set active wshub URL to '{}' in configuration.",
            best_hub.url
        ));
    }

    Ok(())
}

pub fn execute_hub_login(
    url: Option<&str>,
    token: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
    discover: bool,
) -> Result<(), String> {
    let effective_url = if discover {
        OutputHandler::print_info("Discovering local wshub on LAN...");
        let discovered = ws_core::hub_discovery::discover_local_hubs(1500, 8788);
        if let Some(hub) = discovered.first() {
            OutputHandler::print_success(&format!(
                "Discovered wshub at '{}' (host: {})",
                hub.url, hub.hostname
            ));
            Some(hub.url.clone())
        } else {
            OutputHandler::print_info(
                "No local wshub found on LAN; using specified or default URL.",
            );
            url.map(|s| s.to_string())
        }
    } else if url.is_none() {
        let default_client = HubClient::default();
        if default_client.token.is_none() && default_client.base_url == "https://hub.ws.dev" {
            let discovered = ws_core::hub_discovery::discover_local_hubs(800, 8788);
            if let Some(hub) = discovered.first() {
                OutputHandler::print_info(&format!(
                    "Auto-detected local wshub at '{}' (host: {})",
                    hub.url, hub.hostname
                ));
                Some(hub.url.clone())
            } else {
                None
            }
        } else {
            Some(default_client.base_url)
        }
    } else {
        url.map(|s| s.to_string())
    };

    let mut client = HubClient::new(effective_url.as_deref(), token, None);
    if let Some(tok) = token {
        let base = client.base_url.clone();
        client
            .save_session(&base, tok, username)
            .map_err(|e| e.to_string())?;
        OutputHandler::print_success("wshub authentication token saved successfully.");
        return Ok(());
    }

    match (username, password) {
        (Some(u), Some(p)) => {
            let res = client.login(u, p).map_err(|e| e.to_string())?;
            let tok = client.token.clone().or_else(|| {
                res.get("token")
                    .or_else(|| res.get("access_token"))
                    .or_else(|| {
                        res.get("data")
                            .and_then(|d| d.get("token").or_else(|| d.get("access_token")))
                    })
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

            if let Some(ref t) = tok {
                let base = client.base_url.clone();
                client
                    .save_session(&base, t, Some(u))
                    .map_err(|e| e.to_string())?;
                OutputHandler::print_success(&format!(
                    "Successfully logged in to wshub as '{}'",
                    u
                ));
                Ok(())
            } else {
                Err(
                    "Login succeeded but server response did not contain an authentication token."
                        .to_string(),
                )
            }
        }
        (Some(_), None) => Err("Missing --password for username authentication.".to_string()),
        (None, Some(_)) => Err("Missing --username for password authentication.".to_string()),
        (None, None) => Err(
            "Either --token or (--username and --password) must be specified for 'ws hub login'."
                .to_string(),
        ),
    }
}

pub fn execute_hub_logout() -> Result<(), String> {
    let mut client = HubClient::default();
    if client.clear_session() {
        OutputHandler::print_success("Logged out from wshub. Saved session credentials removed.");
    } else {
        OutputHandler::print_info("No active wshub session found.");
    }
    Ok(())
}

pub fn execute_hub_whoami() -> Result<(), String> {
    let client = HubClient::default();
    let res = client.whoami().map_err(|e| e.to_string())?;
    println!("{}", "wshub Current Session:".bold().cyan());

    let user_obj = res
        .get("user")
        .or_else(|| res.get("data").and_then(|d| d.get("user")))
        .unwrap_or(&res);

    let username = user_obj
        .get("username")
        .and_then(|v| v.as_str())
        .or_else(|| res.get("username").and_then(|v| v.as_str()))
        .or(client.username.as_deref());

    let email = user_obj
        .get("email")
        .and_then(|v| v.as_str())
        .or_else(|| res.get("email").and_then(|v| v.as_str()));

    if let Some(u) = username {
        println!("  User:  {}", u.green());
    }
    if let Some(e) = email {
        println!("  Email: {}", e);
    }
    println!("  URL:   {}", client.active_url());
    Ok(())
}

pub fn execute_hub_clone(
    manager: &WorkspaceManager,
    identifier: &str,
    destination: Option<&Path>,
) -> Result<(), String> {
    let dest = manager
        .clone_from_hub(identifier, destination)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!(
        "Successfully cloned workspace project to '{}'",
        dest.display()
    ));
    Ok(())
}

pub fn execute_hub_push(
    manager: &mut WorkspaceManager,
    message: Option<&str>,
    project: Option<&str>,
    silent: bool,
) -> Result<(), String> {
    let msg = message.unwrap_or("Pushed blueprint update from ws CLI");
    manager
        .hub_push(msg, project, silent)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn execute_hub_pull(
    manager: &mut WorkspaceManager,
    project: Option<&str>,
) -> Result<(), String> {
    let res = manager.hub_pull(project).map_err(|e| e.to_string())?;
    let ver = res
        .get("latestRevision")
        .and_then(|r| r.get("version"))
        .or_else(|| res.get("version"))
        .and_then(|v| v.as_i64());
    if let Some(rev) = ver {
        OutputHandler::print_success(&format!(
            "Updated local repositories.yml to wshub blueprint v{}",
            rev
        ));
    } else {
        OutputHandler::print_success("Pulled latest blueprint from wshub");
    }
    Ok(())
}

pub fn execute_hub_status(manager: &WorkspaceManager) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(None)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    println!(
        "{}",
        format!("wshub Project Status: {}/{}", ns, p_name)
            .bold()
            .cyan()
    );
    match client.get_project(&ns, &p_name) {
        Ok(proj) => {
            println!("  Remote Project:  {}", "Connected".green());
            let proj_obj = proj.get("project").unwrap_or(&proj);
            if let Some(desc) = proj_obj.get("description").and_then(|d| d.as_str()) {
                if !desc.trim().is_empty() {
                    println!("  Description:     {}", desc);
                }
            }
            let is_private = proj_obj
                .get("isPrivate")
                .or_else(|| proj.get("isPrivate"))
                .and_then(|p| p.as_bool());
            if let Some(priv_val) = is_private {
                println!(
                    "  Visibility:      {}",
                    if priv_val { "private" } else { "public" }
                );
            } else if let Some(vis) = proj.get("visibility").and_then(|v| v.as_str()) {
                println!("  Visibility:      {}", vis);
            }
            let rev = proj
                .get("latestRevision")
                .and_then(|r| r.get("version"))
                .or_else(|| proj.get("current_version"))
                .or_else(|| proj.get("version"))
                .and_then(|v| v.as_i64());
            if let Some(ver) = rev {
                println!("  Latest Version:  v{}", ver);
            }
            if let Some(created_at) = proj
                .get("latestRevision")
                .and_then(|r| r.get("createdAt"))
                .and_then(|c| c.as_str())
            {
                println!("  Last Updated:    {}", created_at);
            }
        }
        Err(e) => {
            println!("  Remote Project:  {}", "Not found or unreachable".yellow());
            println!("  Details:         {}", e);
        }
    }
    Ok(())
}

pub fn execute_hub_state_save(
    manager: &WorkspaceManager,
    name: Option<&str>,
    project: Option<&str>,
    no_wip: bool,
    auto: bool,
) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    manager
        .hub_state_save(&ws_name, project, !no_wip, false, auto)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!(
        "Saved workspace state for '@{}' to wshub",
        ws_name
    ));
    Ok(())
}

pub fn execute_hub_state_restore(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    project: Option<&str>,
    no_wip: bool,
) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    manager
        .hub_state_restore(&ws_name, project, !no_wip)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!(
        "Restored workspace state for '@{}' from wshub",
        ws_name
    ));
    Ok(())
}

pub fn execute_hub_auto_save_run(manager: &WorkspaceManager) -> Result<(), String> {
    let results = manager.hub_auto_save_all_workspaces(None, false, false);
    for (ws, ok) in results {
        if ok {
            OutputHandler::print_success(&format!("Auto-saved workspace '@{}'", ws));
        } else {
            OutputHandler::print_info(&format!("Workspace '@{}' unchanged (skipped)", ws));
        }
    }
    Ok(())
}

pub fn execute_hub_secret_list(
    manager: &WorkspaceManager,
    project: Option<&str>,
) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(project)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    let res = client
        .list_secrets(&ns, &p_name)
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        format!("wshub Secrets for {}/{}:", ns, p_name)
            .bold()
            .cyan()
    );
    let arr_opt = res
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| res.as_array());
    if let Some(arr) = arr_opt {
        if arr.is_empty() {
            println!("  No secrets stored.");
        } else {
            for item in arr {
                let k = item.get("key").and_then(|v| v.as_str()).unwrap_or("");
                let scope = item
                    .get("repoName")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("scope").and_then(|v| v.as_str()))
                    .unwrap_or("global");
                println!("  {:30} (scope: {})", k.bold(), scope);
            }
        }
    }
    Ok(())
}

pub fn execute_hub_secret_set(
    manager: &WorkspaceManager,
    key: &str,
    value: &str,
    repo: Option<&str>,
    project: Option<&str>,
) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(project)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    client
        .set_secret(&ns, &p_name, key, value, repo)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!("Secret '{}' set in wshub", key));
    Ok(())
}

pub fn execute_hub_secret_get(
    manager: &WorkspaceManager,
    key: &str,
    repo: Option<&str>,
    project: Option<&str>,
) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(project)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    let res = client
        .get_secret(&ns, &p_name, key, repo)
        .map_err(|e| e.to_string())?;
    println!("{}", res);
    Ok(())
}

pub fn execute_hub_secret_delete(
    manager: &WorkspaceManager,
    key: &str,
    repo: Option<&str>,
    project: Option<&str>,
) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(project)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    client
        .delete_secret(&ns, &p_name, key, repo)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!("Secret '{}' deleted from wshub", key));
    Ok(())
}

pub fn execute_hub_publish(
    manager: &mut WorkspaceManager,
    project: Option<&str>,
    description: Option<&str>,
) -> Result<(), String> {
    manager
        .hub_publish(project, description, false)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success("Project published to wshub successfully.");
    Ok(())
}

pub fn execute_hub_sync(
    manager: &mut WorkspaceManager,
    project: Option<&str>,
) -> Result<(), String> {
    execute_hub_pull(manager, project)?;
    execute_hub_secret_pull(manager, project)?;
    Ok(())
}

pub fn execute_hub_secret_upload(
    manager: &WorkspaceManager,
    file_path: &str,
    project: Option<&str>,
) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(project)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    let path = Path::new(file_path);
    if !path.exists() {
        return Err(format!("File '{}' not found.", file_path));
    }
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    client
        .upload_file(&ns, &p_name, &filename, data)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!(
        "Uploaded and encrypted '{}' to wshub vault",
        filename
    ));
    Ok(())
}

pub fn execute_hub_secret_pull(
    manager: &WorkspaceManager,
    project: Option<&str>,
) -> Result<(), String> {
    let (ns, p_name) = manager
        .get_project_namespace_and_name(project)
        .map_err(|e| e.to_string())?;
    let client = HubClient::default();
    let res = client.list_files(&ns, &p_name).map_err(|e| e.to_string())?;
    let files_dir = manager.config.project_root.join("files");
    fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;

    let arr_opt = res
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| res.as_array());

    if let Some(arr) = arr_opt {
        for item in arr {
            let fname_opt = item
                .get("filePath")
                .or_else(|| item.get("filename"))
                .and_then(|v| v.as_str());
            if let Some(fname) = fname_opt {
                let clean_fname = if fname.starts_with("files/") || fname.starts_with("files\\") {
                    &fname[6..]
                } else {
                    fname
                };
                if let Ok(bytes) = client.download_file(&ns, &p_name, fname) {
                    let out_path = files_dir.join(clean_fname);
                    if let Some(parent) = out_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&out_path, bytes);
                    OutputHandler::print_success(&format!(
                        "Downloaded '{}' to files/{}",
                        fname, clean_fname
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn execute_hub_auto_save_status(manager: &WorkspaceManager) -> Result<(), String> {
    use ws_core::daemon::is_standalone_daemon_running;
    use ws_core::systemd::get_service_status;
    use ws_core::utils::format_relative_time;

    let status = manager.get_auto_save_status();
    let enabled = status
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval_sec = status
        .get("interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(900);
    let notify = status
        .get("notify")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let (active_l, pid_l) = manager.is_auto_save_daemon_active();

    let svc = get_service_status();
    let (active_s, pid_s) = is_standalone_daemon_running();

    let daemon_str = if svc.active {
        "active (systemd: ws.service)".green().to_string()
    } else if active_s {
        format!("active (standalone PID {})", pid_s.unwrap_or(0))
            .green()
            .to_string()
    } else if active_l {
        format!("active (local PID {})", pid_l.unwrap_or(0))
            .green()
            .to_string()
    } else {
        "inactive (run 'ws service start' or 'ws service install')"
            .yellow()
            .to_string()
    };

    let interval_str = if interval_sec % 60 == 0 {
        format!("{}m", interval_sec / 60)
    } else {
        format!("{}s", interval_sec)
    };
    let enabled_str = if enabled {
        "enabled".green().bold().to_string()
    } else {
        "disabled".yellow().to_string()
    };
    let notify_str = if notify {
        "on".green().to_string()
    } else {
        "off".dimmed().to_string()
    };

    println!("{}", "wshub Auto-Save Status:".bold().cyan());
    println!(
        "  Project Config:  {} (Interval: {}, Notifications: {})",
        enabled_str, interval_str, notify_str
    );
    println!("  Daemon Status:   {}", daemon_str);

    if let Some(workspaces) = status.get("workspaces").and_then(|w| w.as_object()) {
        if !workspaces.is_empty() {
            let mut table = OutputHandler::create_table();
            table.set_header(vec![
                Cell::new("Workspace").fg(Color::Cyan),
                Cell::new("Last Saved").fg(Color::White),
                Cell::new("Uncommitted WIP").fg(Color::Magenta),
                Cell::new("Session").fg(Color::DarkGrey),
            ]);

            for (w_name, info) in workspaces {
                let last_saved_str = info
                    .get("last_saved_at")
                    .and_then(|v| v.as_str())
                    .map(format_relative_time)
                    .unwrap_or_else(|| "never".to_string());
                let has_uncommitted = info
                    .get("has_uncommitted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let active_session = info
                    .get("active_session")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let dirty_cell = if has_uncommitted {
                    Cell::new("dirty (uncommitted)").fg(Color::Yellow)
                } else {
                    Cell::new("clean").fg(Color::Green)
                };
                let sess_cell = if active_session {
                    Cell::new("active").fg(Color::Green)
                } else {
                    Cell::new("idle").fg(Color::DarkGrey)
                };

                let last_saved_cell = if last_saved_str == "never" {
                    Cell::new(last_saved_str).fg(Color::DarkGrey)
                } else {
                    Cell::new(last_saved_str).fg(Color::White)
                };

                table.add_row(vec![
                    Cell::new(format!("@{}", w_name)).fg(Color::Cyan),
                    last_saved_cell,
                    dirty_cell,
                    sess_cell,
                ]);
            }
            println!("\n{}", "Workspace Topology:".bold().cyan());
            println!("{}", table);
        }
    }

    Ok(())
}

pub fn execute_hub_auto_save_start(
    manager: &WorkspaceManager,
    interval: Option<u64>,
    project: Option<&str>,
    detached: bool,
) -> Result<(), String> {
    use ws_core::daemon::{is_standalone_daemon_running, start_standalone_daemon};
    use ws_core::systemd::{
        control_service, get_service_status, install_service, is_systemctl_available,
    };

    if let Some(pi) = project {
        let pid = manager
            .start_auto_save_daemon(interval, Some(pi), detached)
            .map_err(|e| e.to_string())?;
        OutputHandler::print_success(&format!(
            "Started isolated wshub auto-save daemon for project '{}' (PID {})",
            pi, pid
        ));
        return Ok(());
    }

    // Global machine-wide auto-save daemon management
    let svc = get_service_status();
    if svc.active {
        OutputHandler::print_success(
            "Global auto-save daemon is already running (ws.service).\nAll registered projects with hub.auto_save.enabled: true are monitored.",
        );
        return Ok(());
    }

    let (active_s, pid_s) = is_standalone_daemon_running();
    if active_s {
        OutputHandler::print_success(&format!(
            "Standalone global auto-save daemon is already running (PID {}).\nAll registered projects with hub.auto_save.enabled: true are monitored.",
            pid_s.unwrap_or(0)
        ));
        return Ok(());
    }

    if is_systemctl_available() {
        if !svc.installed {
            let (ok, msg) = install_service(None).map_err(|e| e.to_string())?;
            if ok {
                OutputHandler::print_success(&format!(
                    "Installed and started {}!",
                    ws_core::systemd::SERVICE_NAME
                ));
            } else {
                return Err(msg);
            }
        } else {
            let (ok, msg) = control_service("start").map_err(|e| e.to_string())?;
            if ok {
                OutputHandler::print_success(&format!(
                    "Started {}!",
                    ws_core::systemd::SERVICE_NAME
                ));
            } else {
                return Err(msg);
            }
        }
        OutputHandler::print_info(
            "The daemon will automatically monitor and save all workspaces across all registered projects.",
        );
    } else {
        let pid = start_standalone_daemon(true).map_err(|e| e.to_string())?;
        OutputHandler::print_success(&format!(
            "Started standalone global auto-save daemon (PID {}). Monitoring all registered projects.",
            pid
        ));
    }

    // Check current project setting
    let auto_cfg = manager.config.hub_auto_save.as_ref();
    if !auto_cfg.map(|a| a.enabled).unwrap_or(false) {
        OutputHandler::print_warning(
            "Note: 'hub.auto_save.enabled' is currently false or missing in repositories.yml for this project.\nSet 'enabled: true' under 'hub.auto_save' in repositories.yml to include this project.",
        );
    }

    Ok(())
}

pub fn execute_hub_auto_save_stop(manager: &WorkspaceManager) -> Result<(), String> {
    use ws_core::daemon::{is_standalone_daemon_running, stop_standalone_daemon};
    use ws_core::systemd::{control_service, get_service_status};

    let (active_l, pid_l) = manager.is_auto_save_daemon_active();
    if active_l {
        let _ = manager.stop_auto_save_daemon();
        OutputHandler::print_success(&format!(
            "Stopped local auto-save daemon (PID {})",
            pid_l.unwrap_or(0)
        ));
    }

    let svc = get_service_status();
    if svc.active {
        let (ok, msg) = control_service("stop").map_err(|e| e.to_string())?;
        if ok {
            OutputHandler::print_success("Stopped ws.service.");
        } else {
            OutputHandler::print_err(&msg);
        }
    }

    let (active_s, _) = is_standalone_daemon_running();
    if active_s {
        let _ = stop_standalone_daemon();
        OutputHandler::print_success("Stopped standalone global auto-save daemon.");
    }

    if !active_l && !svc.active && !active_s {
        OutputHandler::print_info("No active auto-save daemon found.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_hub_login_arg_validation() {
        let res_no_args = execute_hub_login(None, None, None, None, false);
        assert_eq!(
            res_no_args.unwrap_err(),
            "Either --token or (--username and --password) must be specified for 'ws hub login'."
        );

        let res_user_only = execute_hub_login(None, None, Some("alice"), None, false);
        assert_eq!(
            res_user_only.unwrap_err(),
            "Missing --password for username authentication."
        );

        let res_pass_only = execute_hub_login(None, None, None, Some("secret"), false);
        assert_eq!(
            res_pass_only.unwrap_err(),
            "Missing --username for password authentication."
        );
    }
}
