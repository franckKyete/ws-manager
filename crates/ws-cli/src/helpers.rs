use std::collections::HashSet;
use std::path::Path;
use ws_core::WorkspaceManager;

pub const KNOWN_COMMANDS: &[&str] = &[
    "create",
    "new",
    "list",
    "ls",
    "info",
    "end",
    "close",
    "delete",
    "rm",
    "remove",
    "status",
    "exec",
    "push",
    "pull",
    "start",
    "launch",
    "run",
    "attach",
    "stop",
    "kill",
    "restart",
    "logs",
    "shell",
    "enter",
    "open",
    "env",
    "setup",
    "bridge",
    "focus",
    "switch",
    "repo",
    "lock",
    "unlock",
    "workspace",
    "project",
    "init",
    "add",
    "fetch",
    "sync",
    "doctor",
    "antigravity",
    "completion",
    "_complete",
    "hub",
    "clone",
    "daemon",
    "service",
];

pub fn clean_workspace(name: Option<&str>) -> Option<String> {
    name.map(|n| n.trim_start_matches('@').to_string())
}

pub fn clean_repo(name: Option<&str>) -> Option<String> {
    name.map(|n| {
        n.trim_start_matches(&['%', '+', ':', '#', '$'][..])
            .to_string()
    })
}

pub fn clean_repos(repos: Option<&[String]>) -> Option<Vec<String>> {
    repos.map(|r_list| {
        r_list
            .iter()
            .map(|r| {
                r.trim_start_matches(&['%', '+', ':', '#', '$'][..])
                    .to_string()
            })
            .collect()
    })
}

pub fn normalize_cli_args(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        return Vec::new();
    }

    let mut result = args.to_vec();
    let known: HashSet<&str> = KNOWN_COMMANDS.iter().copied().collect();

    let first = &result[0];

    // Case 1: @workspace <cmd> -> <cmd> @workspace
    if first.starts_with('@') && result.len() >= 2 {
        let second = &result[1];
        if known.contains(second.as_str()) {
            let cmd = result.remove(1);
            result.insert(0, cmd);
            return result;
        }
    }

    // Case 2: %repo <cmd> -> <cmd> %repo
    if first.starts_with('%') && result.len() >= 2 {
        let second = &result[1];
        if known.contains(second.as_str()) {
            let cmd = result.remove(1);
            result.insert(0, cmd);
            return result;
        }
    }

    // Case 3: workspace_name <cmd> -> <cmd> workspace_name
    if !known.contains(first.as_str()) && !first.starts_with('-') && result.len() >= 2 {
        let second = &result[1];
        if known.contains(second.as_str()) {
            let cmd = result.remove(1);
            result.insert(0, cmd);
            return result;
        }
    }

    result
}

pub fn resolve_ws_and_repo_args(
    manager: &WorkspaceManager,
    name_arg: Option<&str>,
    repo_arg: Option<&str>,
    repos_arg: Option<&[String]>,
    require_ws: bool,
    require_repo: bool,
) -> Result<(String, Option<String>, Vec<String>), String> {
    let (detected_ws, detected_repo) = manager.detect_context(None);

    let mut is_repo_spec = false;
    if let Some(n) = name_arg {
        if n.starts_with(&['%', '+', ':', '#', '$'][..]) {
            is_repo_spec = true;
        } else if detected_ws.is_some()
            && !manager.has_workspace(&clean_workspace(Some(n)).unwrap())
        {
            let c_n = clean_repo(Some(n)).unwrap();
            if manager.config.repositories.contains_key(&c_n)
                || manager.config.repositories.values().any(|r| {
                    r.checkout == c_n
                        || Path::new(&r.checkout)
                            .file_name()
                            .map(|f| f.to_str().unwrap() == c_n)
                            .unwrap_or(false)
                })
            {
                is_repo_spec = true;
            } else if let Some(ref d_ws) = detected_ws {
                if let Ok((meta, _)) = manager.get_workspace_info(d_ws) {
                    if meta.repositories.contains_key(&c_n)
                        || meta.repositories.values().any(|s| {
                            s.path == c_n
                                || Path::new(&s.path)
                                    .file_name()
                                    .map(|f| f.to_str().unwrap() == c_n)
                                    .unwrap_or(false)
                        })
                    {
                        is_repo_spec = true;
                    }
                }
            }
        }
    }

    let resolved_ws: Option<String>;
    let mut resolved_repo: Option<String> = None;
    let mut resolved_repos: Vec<String> = Vec::new();

    if is_repo_spec {
        resolved_ws = detected_ws;
        resolved_repo = clean_repo(name_arg);
    } else {
        resolved_ws = clean_workspace(name_arg).or(detected_ws);
        if let Some(r) = repo_arg {
            resolved_repo = clean_repo(Some(r));
        } else if resolved_ws.is_some() && detected_repo.is_some() && name_arg.is_none() {
            resolved_repo = detected_repo;
        }
    }

    if let Some(r_list) = repos_arg {
        let mut list = clean_repos(Some(r_list)).unwrap_or_default();
        if is_repo_spec {
            if let Some(ref r) = resolved_repo {
                if !list.contains(r) {
                    list.insert(0, r.clone());
                }
            }
        }
        if list.is_empty() {
            if let Some(ref r) = resolved_repo {
                resolved_repos = vec![r.clone()];
            }
        } else {
            resolved_repos = list;
        }
    } else if let Some(ref r) = resolved_repo {
        resolved_repos = vec![r.clone()];
    }

    if require_ws && resolved_ws.is_none() {
        return Err(
            "Workspace name is required, or command must be run inside a workspace directory."
                .to_string(),
        );
    }

    if require_repo && resolved_repo.is_none() {
        return Err(
            "Repository name is required, or command must be run inside a repository worktree."
                .to_string(),
        );
    }

    Ok((
        resolved_ws.unwrap_or_default(),
        resolved_repo,
        resolved_repos,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_workspace() {
        assert_eq!(clean_workspace(Some("@feat")), Some("feat".to_string()));
        assert_eq!(clean_workspace(Some("feat")), Some("feat".to_string()));
        assert_eq!(clean_workspace(None), None);
    }

    #[test]
    fn test_clean_repo() {
        assert_eq!(clean_repo(Some("%server")), Some("server".to_string()));
        assert_eq!(clean_repo(Some("+client")), Some("client".to_string()));
        assert_eq!(clean_repo(Some(":db")), Some("db".to_string()));
        assert_eq!(clean_repo(Some("#worker")), Some("worker".to_string()));
        assert_eq!(clean_repo(Some("$api")), Some("api".to_string()));
        assert_eq!(clean_repo(Some("plain")), Some("plain".to_string()));
        assert_eq!(clean_repo(None), None);
    }

    #[test]
    fn test_normalize_cli_args_inverted_workspace() {
        let args = vec![
            "@feat".to_string(),
            "start".to_string(),
            "--tmux".to_string(),
        ];
        let norm = normalize_cli_args(&args);
        assert_eq!(norm, vec!["start", "@feat", "--tmux"]);
    }

    #[test]
    fn test_normalize_cli_args_inverted_repo() {
        let args = vec!["%backend".to_string(), "logs".to_string(), "-f".to_string()];
        let norm = normalize_cli_args(&args);
        assert_eq!(norm, vec!["logs", "%backend", "-f"]);
    }

    #[test]
    fn test_normalize_cli_args_positional_ws() {
        let args = vec!["my-ws".to_string(), "start".to_string()];
        let norm = normalize_cli_args(&args);
        assert_eq!(norm, vec!["start", "my-ws"]);
    }

    #[test]
    fn test_normalize_cli_args_standard_order() {
        let args = vec![
            "create".to_string(),
            "@feat".to_string(),
            "--all".to_string(),
        ];
        let norm = normalize_cli_args(&args);
        assert_eq!(norm, vec!["create", "@feat", "--all"]);
    }

    fn create_test_manager(root: &std::path::Path) -> WorkspaceManager {
        let config = ws_core::models::AppConfig {
            project_root: root.to_path_buf(),
            repositories: std::collections::HashMap::new(),
            workspaces_dir: root.join("workspaces"),
            config_file_path: None,
            global_env: std::collections::HashMap::new(),
            secret_env: std::collections::HashMap::new(),
            private_env: std::collections::HashMap::new(),
            dynamic_env: std::collections::HashMap::new(),
            setup: Vec::new(),
            secrets: Vec::new(),
            copy_files: Vec::new(),
            tmux: None,
            hub_auto_save: None,
            hub_project: None,
            hub: serde_json::Value::Null,
        };
        WorkspaceManager::new(config, None)
    }

    #[test]
    fn test_resolve_ws_and_repo_args_omitted_workspace() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ws_root = temp_dir.path();
        let manager = create_test_manager(ws_root);

        let (_ws, repo, repos) =
            resolve_ws_and_repo_args(&manager, Some("%manager"), None, Some(&[]), false, false)
                .unwrap();
        assert_eq!(repo, Some("manager".to_string()));
        assert_eq!(repos, vec!["manager".to_string()]);
    }

    #[test]
    fn test_resolve_ws_and_repo_args_omitted_ws_multi_repo() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ws_root = temp_dir.path();
        let manager = create_test_manager(ws_root);

        let (_ws, repo, repos) = resolve_ws_and_repo_args(
            &manager,
            Some("%manager"),
            None,
            Some(&["%hub".to_string()]),
            false,
            false,
        )
        .unwrap();
        assert_eq!(repo, Some("manager".to_string()));
        assert_eq!(repos, vec!["manager".to_string(), "hub".to_string()]);
    }
}
