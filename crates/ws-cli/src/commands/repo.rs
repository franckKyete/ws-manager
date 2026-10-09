use ws_core::WorkspaceManager;
use crate::helpers::{clean_repo, resolve_ws_and_repo_args};

pub fn execute_repo_add(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo_input: Option<&str>,
    branch: Option<&str>,
    existing: bool,
) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, repo_input, None, true, false)?;
    let raw_input = repo_input
        .or(name)
        .ok_or_else(|| "Repository specification (%<repo>[:branch]) required.".to_string())?;
    let clean_input = clean_repo(Some(raw_input)).unwrap();
    let mut create_mode = !existing;
    let r_name: String;
    let mut r_branch: Option<String> = None;

    if clean_input.contains(':') {
        let parts: Vec<&str> = clean_input.split(':').collect();
        r_name = parts[0].to_string();
        if parts.len() > 1 && !parts[1].is_empty() {
            r_branch = Some(parts[1].to_string());
        }
        if parts.len() > 2 {
            if parts[2] == "existing" {
                create_mode = false;
            } else if parts[2] == "new" {
                create_mode = true;
            }
        }
    } else if clean_input.contains('=') {
        let parts: Vec<&str> = clean_input.splitn(2, '=').collect();
        r_name = parts[0].to_string();
        let mut b_str = parts[1].to_string();
        if let Some(stripped) = b_str.strip_suffix(":existing") {
            b_str = stripped.to_string();
            create_mode = false;
        } else if let Some(stripped) = b_str.strip_suffix(":new") {
            b_str = stripped.to_string();
            create_mode = true;
        }
        r_branch = Some(b_str);
    } else {
        r_name = clean_input;
        r_branch = branch.map(|s| s.to_string());
    }

    let final_branch = r_branch.unwrap_or_else(|| {
        if !create_mode {
            ws_name.clone()
        } else {
            format!("feature/{}", ws_name)
        }
    });

    manager
        .workspace_add_repo(&ws_name, &r_name, &final_branch, create_mode)
        .map_err(|e| e.to_string())
}

pub fn execute_repo_remove(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
    delete_branch: bool,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, true)?;
    let r_name = repo_name.unwrap();
    manager
        .workspace_remove_repo(&ws_name, &r_name, delete_branch)
        .map_err(|e| e.to_string())
}

pub fn execute_repo_lock(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, true)?;
    let r_name = repo_name.unwrap();
    manager.lock_repo(&ws_name, &r_name).map_err(|e| e.to_string())
}

pub fn execute_repo_unlock(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, true)?;
    let r_name = repo_name.unwrap();
    manager.unlock_repo(&ws_name, &r_name).map_err(|e| e.to_string())
}
