use crate::helpers::resolve_ws_and_repo_args;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use ws_core::hub::HubClient;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_hub_login(
    url: Option<&str>,
    token: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    let mut client = HubClient::new(url, token, None);
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
    println!("  URL:   {}", client.base_url);
    Ok(())
}

pub fn execute_hub_clone(identifier: &str, destination: Option<&Path>) -> Result<(), String> {
    let client = HubClient::default();
    let (ns, name) = HubClient::parse_project_identifier(identifier).map_err(|e| e.to_string())?;

    OutputHandler::print_info(&format!(
        "Fetching project blueprint '{}/{}' from wshub...",
        ns, name
    ));
    let proj = client.get_project(&ns, &name).map_err(|e| e.to_string())?;

    let dest_dir = destination
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(&name));

    fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;

    if let Some(raw_yaml) = proj
        .get("blueprint")
        .and_then(|b| b.get("yaml"))
        .and_then(|y| y.as_str())
    {
        let repo_yml_path = dest_dir.join("repositories.yml");
        fs::write(&repo_yml_path, raw_yaml).map_err(|e| e.to_string())?;
        OutputHandler::print_success(&format!(
            "Saved project blueprint to '{}'",
            repo_yml_path.display()
        ));
    } else {
        return Err("Project does not have an active blueprint on wshub.".to_string());
    }

    Ok(())
}

pub fn execute_hub_push(
    manager: &mut WorkspaceManager,
    message: Option<&str>,
    silent: bool,
) -> Result<(), String> {
    let msg = message.unwrap_or("Pushed blueprint update from ws CLI");
    let res = manager
        .hub_push(msg, None, silent)
        .map_err(|e| e.to_string())?;
    if let Some(rev) = res.get("revision") {
        let ver = rev.get("version").and_then(|v| v.as_i64()).unwrap_or(1);
        OutputHandler::print_success(&format!("Pushed blueprint revision v{}", ver));
    }
    Ok(())
}

pub fn execute_hub_pull(
    manager: &mut WorkspaceManager,
    project: Option<&str>,
) -> Result<(), String> {
    let res = manager.hub_pull(project).map_err(|e| e.to_string())?;
    if let Some(rev) = res.get("version").and_then(|v| v.as_i64()) {
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
    let (ns, p_name) = manager.get_project_namespace_and_name(None);
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
            if let Some(vis) = proj.get("visibility").and_then(|v| v.as_str()) {
                println!("  Visibility:      {}", vis);
            }
            if let Some(rev) = proj.get("current_version").and_then(|v| v.as_i64()) {
                println!("  Latest Version:  v{}", rev);
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
    _message: Option<&str>,
) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    manager
        .hub_state_save(&ws_name, None, true, false, false)
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
) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    manager
        .hub_state_restore(&ws_name, None, true)
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

pub fn execute_hub_secret_list(manager: &WorkspaceManager) -> Result<(), String> {
    let (ns, p_name) = manager.get_project_namespace_and_name(None);
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
    if let Some(arr) = res.as_array() {
        if arr.is_empty() {
            println!("  No secrets stored.");
        } else {
            for item in arr {
                let k = item.get("key").and_then(|v| v.as_str()).unwrap_or("");
                let scope = item
                    .get("scope")
                    .and_then(|v| v.as_str())
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
) -> Result<(), String> {
    let (ns, p_name) = manager.get_project_namespace_and_name(None);
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
) -> Result<(), String> {
    let (ns, p_name) = manager.get_project_namespace_and_name(None);
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
) -> Result<(), String> {
    let (ns, p_name) = manager.get_project_namespace_and_name(None);
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
    let (ns, p_name) = manager.get_project_namespace_and_name(project);
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
    let (ns, p_name) = manager.get_project_namespace_and_name(project);
    let client = HubClient::default();
    let res = client.list_files(&ns, &p_name).map_err(|e| e.to_string())?;
    let files_dir = manager.config.project_root.join("files");
    fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;

    if let Some(arr) = res.as_array() {
        for item in arr {
            if let Some(fname) = item.get("filename").and_then(|v| v.as_str()) {
                if let Ok(bytes) = client.download_file(&ns, &p_name, fname) {
                    let out_path = files_dir.join(fname);
                    let _ = fs::write(&out_path, bytes);
                    OutputHandler::print_success(&format!(
                        "Downloaded '{}' to files/{}",
                        fname, fname
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn execute_hub_auto_save_status(manager: &WorkspaceManager) -> Result<(), String> {
    let (active, pid) = manager.is_auto_save_daemon_active();
    if active {
        OutputHandler::print_success(&format!(
            "Auto-save background daemon is ACTIVE (PID {})",
            pid.unwrap_or(0)
        ));
    } else {
        OutputHandler::print_info("Auto-save background daemon is INACTIVE.");
    }
    Ok(())
}

pub fn execute_hub_auto_save_start(
    manager: &WorkspaceManager,
    interval: Option<u64>,
    project: Option<&str>,
    detached: bool,
) -> Result<(), String> {
    let pid = manager
        .start_auto_save_daemon(interval, project, detached)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!("Started wshub auto-save daemon (PID {})", pid));
    Ok(())
}

pub fn execute_hub_auto_save_stop(manager: &WorkspaceManager) -> Result<(), String> {
    let (active, pid) = manager.is_auto_save_daemon_active();
    if !active {
        OutputHandler::print_info("No active auto-save daemon found.");
        return Ok(());
    }
    match manager.stop_auto_save_daemon() {
        Ok(true) => OutputHandler::print_success(&format!(
            "Stopped auto-save daemon (PID {})",
            pid.unwrap_or(0)
        )),
        _ => OutputHandler::print_err(&format!(
            "Failed stopping auto-save daemon (PID {})",
            pid.unwrap_or(0)
        )),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_hub_login_arg_validation() {
        let res_no_args = execute_hub_login(None, None, None, None);
        assert_eq!(
            res_no_args.unwrap_err(),
            "Either --token or (--username and --password) must be specified for 'ws hub login'."
        );

        let res_user_only = execute_hub_login(None, None, Some("alice"), None);
        assert_eq!(
            res_user_only.unwrap_err(),
            "Missing --password for username authentication."
        );

        let res_pass_only = execute_hub_login(None, None, None, Some("secret"));
        assert_eq!(
            res_pass_only.unwrap_err(),
            "Missing --username for password authentication."
        );
    }
}
