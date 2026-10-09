use std::path::Path;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;
use crate::helpers::{clean_repo, clean_workspace};

pub fn execute_exec(
    manager: &WorkspaceManager,
    name: Option<&str>,
    command: &[String],
    all: bool,
    repos_flag: Option<&str>,
) -> Result<(), String> {
    let (detected_ws, _) = manager.detect_context(None);
    let mut tokens: Vec<String> = Vec::new();
    if let Some(n) = name {
        tokens.push(n.to_string());
    }
    tokens.extend_from_slice(command);

    let mut ws_name: Option<String> = None;
    let mut target_repos: Vec<String> = Vec::new();
    let mut exec_cmd: Vec<String> = Vec::new();
    let mut all_flag = all;

    if let Some(rf) = repos_flag {
        for r in rf.split(',') {
            if let Some(r_clean) = clean_repo(Some(r.trim())) {
                if !r_clean.is_empty() {
                    target_repos.push(r_clean);
                }
            }
        }
    }

    let mut idx = 0;
    // 1. Parse optional workspace name (@name or registered workspace name before command)
    if idx < tokens.len() {
        let t = &tokens[idx];
        if t.starts_with('@') {
            ws_name = clean_workspace(Some(t));
            idx += 1;
        } else if t == "--" {
            idx += 1;
            exec_cmd.extend_from_slice(&tokens[idx..]);
            idx = tokens.len();
        } else if !t.starts_with(&['%', '+', ':', '#', '$', '-'][..]) {
            if let Some(c_ws) = clean_workspace(Some(t)) {
                if manager.has_workspace(&c_ws) {
                    ws_name = Some(c_ws);
                    idx += 1;
                }
            }
        }
    }

    if ws_name.is_none() {
        ws_name = detected_ws;
    }

    // 2. Parse optional repository filters (%repo) before delimiter '--' or command
    while idx < tokens.len() {
        let t = &tokens[idx];
        if t == "--" {
            idx += 1;
            break;
        } else if t == "--all" || t == "-a" {
            all_flag = true;
            target_repos.clear();
            idx += 1;
        } else if t.starts_with(&['%', '+', ':', '#', '$'][..]) {
            for sub_t in t.split(',') {
                if let Some(sub_clean) = clean_repo(Some(sub_t.trim())) {
                    if !sub_clean.is_empty() {
                        target_repos.push(sub_clean);
                    }
                }
            }
            idx += 1;
        } else if let Some(ref current_ws) = ws_name {
            let clean_t = clean_repo(Some(t)).unwrap_or_else(|| t.to_string());
            let mut is_repo = false;
            if manager.config.repositories.contains_key(&clean_t) {
                is_repo = true;
            } else if manager.config.repositories.values().any(|r| r.checkout == clean_t) {
                is_repo = true;
            } else if let Ok((meta, _)) = manager.get_workspace_info(current_ws) {
                if meta.repositories.contains_key(&clean_t)
                    || meta.repositories.values().any(|s| {
                        s.path == clean_t || Path::new(&s.path).file_name().map(|f| f.to_str().unwrap() == clean_t).unwrap_or(false)
                    })
                {
                    is_repo = true;
                }
            }

            if is_repo {
                target_repos.push(clean_t);
                idx += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // 3. Remaining tokens form the command
    if idx < tokens.len() {
        exec_cmd.extend_from_slice(&tokens[idx..]);
    }

    if !exec_cmd.is_empty() && exec_cmd[0] == "--" {
        exec_cmd.remove(0);
    }

    let ws = match ws_name {
        Some(w) => w,
        None => return Err("Workspace name (@<name>) required for 'ws exec'. Specify a workspace or run this command from inside a workspace directory.".to_string()),
    };

    if exec_cmd.is_empty() {
        return Err("Command required for 'ws exec'.".to_string());
    }

    let filter = if all_flag || target_repos.iter().any(|r| r == "all" || r == "*") {
        None
    } else if !target_repos.is_empty() {
        Some(target_repos)
    } else {
        None
    };

    let results = manager
        .exec_workspace(&ws, &exec_cmd, filter.as_deref())
        .map_err(|e| e.to_string())?;

    let failed: Vec<_> = results
        .iter()
        .filter_map(|(r, code)| if *code != 0 { Some(r.as_str()) } else { None })
        .collect();

    if !failed.is_empty() {
        OutputHandler::print_warning(&format!("Command exited with non-zero status in: {}", failed.join(", ")));
        std::process::exit(1);
    }

    Ok(())
}
