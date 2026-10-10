use crate::helpers::resolve_ws_and_repo_args;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_start(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    repos: Option<&[String]>,
    all: bool,
    repos_flag: Option<&str>,
    mode: Option<&str>,
    attach_repo: Option<&str>,
    daemon: bool,
    switch: bool,
    interface: Option<&str>,
    lan_ip: Option<&str>,
) -> Result<(), String> {
    let (ws_name, _, resolved_repos) =
        resolve_ws_and_repo_args(manager, name, None, repos, true, false)?;
    let mut target_repos = if !resolved_repos.is_empty() {
        Some(resolved_repos)
    } else {
        None
    };

    if let Some(rf) = repos_flag {
        let set: Vec<String> = rf
            .split(',')
            .filter(|r| !r.trim().is_empty())
            .map(|r| crate::helpers::clean_repo(Some(r.trim())).unwrap())
            .collect();
        target_repos = Some(set);
    }

    if all {
        target_repos = None;
    }

    let launch_mode = mode.unwrap_or("tui");

    manager
        .launch_workspace(
            &ws_name,
            target_repos.as_deref(),
            launch_mode,
            attach_repo,
            daemon,
            switch,
            interface,
            lan_ip,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn execute_stop(manager: &WorkspaceManager, name: Option<&str>) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    OutputHandler::print_info(&format!(
        "Stopping services for workspace '@{}'...",
        ws_name
    ));
    let stopped = manager
        .stop_workspace(&ws_name)
        .map_err(|e| e.to_string())?;
    if stopped {
        OutputHandler::print_success(&format!("Workspace session for '@{}' terminated.", ws_name));
    } else {
        OutputHandler::print_info(&format!("No active session found for '@{}'.", ws_name));
    }
    Ok(())
}

pub fn execute_attach(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
    all_panes: bool,
    mode: Option<&str>,
    switch: bool,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, false)?;
    let target_repo = repo.or(repo_name.as_deref());
    manager
        .attach_workspace(&ws_name, target_repo, all_panes, mode, switch)
        .map_err(|e| e.to_string())
}

pub fn execute_restart(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
    repos: Option<&[String]>,
) -> Result<(), String> {
    let (ws_name, _, resolved_repos) =
        resolve_ws_and_repo_args(manager, name, repo, repos, true, false)?;
    let sock_path = manager.get_session_socket_path(&ws_name);

    if !manager.is_session_running(&ws_name) {
        OutputHandler::print_warning(&format!(
            "No running session for workspace '@{}'. Starting...",
            ws_name
        ));
        return execute_start(
            manager,
            Some(&ws_name),
            repos,
            false,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
    }

    let target_repos = if !resolved_repos.is_empty() {
        resolved_repos
    } else {
        let (meta, _) = manager
            .get_workspace_info(&ws_name)
            .map_err(|e| e.to_string())?;
        meta.repositories.keys().cloned().collect()
    };

    for r in &target_repos {
        match UnixStream::connect(&sock_path) {
            Ok(mut stream) => {
                let req = serde_json::json!({
                    "type": "RestartService",
                    "service": r,
                });
                let req_bytes = format!("{}\n", req);
                let _ = stream.write_all(req_bytes.as_bytes());
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                OutputHandler::print_success(&format!(
                    "Restarted service '%{}' in workspace '@{}'",
                    r, ws_name
                ));
            }
            Err(e) => {
                OutputHandler::print_err(&format!("Failed restarting service '%{}': {}", r, e));
            }
        }
    }

    Ok(())
}

pub fn execute_logs(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
    follow: bool,
    lines: usize,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, false)?;
    let ws_dir = manager.get_workspace_dir(&ws_name);
    let log_dir = ws_dir.join(".ws").join("logs");

    if !log_dir.exists() {
        OutputHandler::print_warning(&format!(
            "No log directory found for workspace '@{}'.",
            ws_name
        ));
        return Ok(());
    }

    let target_repo = repo.or(repo_name.as_deref());
    let mut log_files = Vec::new();

    if let Some(r) = target_repo {
        let direct = log_dir.join(format!("{}.log", r));
        if direct.exists() {
            log_files.push(direct);
        } else {
            let alt_name = manager
                .config
                .repositories
                .get(r)
                .map(|c| c.checkout.clone())
                .or_else(|| {
                    manager
                        .config
                        .repositories
                        .iter()
                        .find(|(_, c)| c.checkout == r)
                        .map(|(k, _)| k.clone())
                });
            if let Some(alt) = alt_name {
                let alt_path = log_dir.join(format!("{}.log", alt));
                if alt_path.exists() {
                    log_files.push(alt_path);
                }
            }
        }

        if log_files.is_empty() {
            OutputHandler::print_err(&format!(
                "No log file found for service '%{}' in {}",
                r,
                log_dir.display()
            ));
            return Ok(());
        }
    } else if let Ok(entries) = fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("log") {
                log_files.push(path);
            }
        }
        log_files.sort();
    }

    if log_files.is_empty() {
        OutputHandler::print_info(&format!("No log files found in '{}'.", log_dir.display()));
        return Ok(());
    }

    for lf in log_files {
        let fname = lf.file_name().unwrap_or_default().to_string_lossy();
        OutputHandler::print_info(&format!("Log: {}", fname));
        if let Ok(content) = fs::read_to_string(&lf) {
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(lines);
            for l in &all_lines[start..] {
                println!("{}", l);
            }
        }
    }

    let _ = follow; // follow mode placeholder or tail
    Ok(())
}

pub fn execute_bridge(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, false)?;
    let target_repo = repo.or(repo_name.as_deref());
    manager
        .run_raw_bridge(&ws_name, target_repo)
        .map_err(|e| e.to_string())
}
