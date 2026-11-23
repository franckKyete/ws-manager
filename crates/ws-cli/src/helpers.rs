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
    "activity",
    "lifecycle",
];

pub fn clean_workspace(name: Option<&str>) -> Option<String> {
    name.map(|n| n.trim_start_matches('@').to_string())
}

/// Return true when `token` (already stripped of sigils) names a configured or
/// workspace repository/service.
pub fn is_known_repo(manager: &WorkspaceManager, token: &str) -> bool {
    let bare = token.trim();
    if bare.is_empty() {
        return false;
    }
    if manager.config.repositories.contains_key(bare) {
        return true;
    }
    if manager.config.repositories.values().any(|r| {
        r.checkout == bare
            || Path::new(&r.checkout)
                .file_name()
                .and_then(|f| f.to_str())
                .map(|f| f == bare)
                .unwrap_or(false)
    }) {
        return true;
    }
    let (detected_ws, _) = manager.detect_context(None);
    if let Some(ws) = detected_ws {
        if let Ok((meta, _)) = manager.get_workspace_info(&ws) {
            return meta.repositories.contains_key(bare)
                || meta.repositories.values().any(|s| {
                    s.path == bare
                        || Path::new(&s.path)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .map(|f| f == bare)
                            .unwrap_or(false)
                });
        }
    }
    false
}

/// Build a helpful error explaining the required `@`/`%` prefix for a bare
/// workspace or repository token.
pub fn sigil_hint(manager: &WorkspaceManager, token: &str) -> String {
    let bare = token.trim_start_matches(&['@', '%', '+', ':', '#', '$'][..]);
    if !bare.is_empty() && manager.has_workspace(bare) {
        format!(
            "Workspace names must start with '@': use '@{}' instead of '{}'.",
            bare, token
        )
    } else if !bare.is_empty() && is_known_repo(manager, bare) {
        format!(
            "Repository and service names must start with '%': use '%{}' instead of '{}'.",
            bare, token
        )
    } else {
        format!(
            "'{}' requires a prefix: workspace names start with '@' (e.g. '@{}'); repository and service names start with '%' (e.g. '%{}').",
            token, bare, bare
        )
    }
}

/// Parse a single CLI repository/service token. Only `%` is accepted.
pub fn parse_repo_token(manager: &WorkspaceManager, token: &str) -> Result<String, String> {
    let token = token.trim();
    if let Some(rest) = token.strip_prefix('%') {
        if rest.is_empty() {
            return Err("Repository name after '%' is empty.".to_string());
        }
        Ok(rest.to_string())
    } else {
        Err(sigil_hint(manager, token))
    }
}

/// Parse a comma-separated repository/service flag value. Every item must use
/// the `%` prefix.
pub fn parse_repo_list(manager: &WorkspaceManager, value: &str) -> Result<Vec<String>, String> {
    let mut repos = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let repo = parse_repo_token(manager, item)?;
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    Ok(repos)
}

pub fn clean_repo(name: Option<&str>) -> Option<String> {
    name.map(|n| {
        n.trim_start_matches(&['%', '+', ':', '#', '$'][..])
            .to_string()
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

    let mut explicit_ws: Option<String> = None;
    let mut positional_repo: Option<String> = None;

    if let Some(n) = name_arg {
        let n = n.trim();
        if let Some(rest) = n.strip_prefix('@') {
            if rest.is_empty() {
                return Err("Workspace name after '@' is empty.".to_string());
            }
            explicit_ws = Some(rest.to_string());
        } else if n.starts_with('%') {
            positional_repo = Some(parse_repo_token(manager, n)?);
        } else {
            return Err(sigil_hint(manager, n));
        }
    }

    let repo_from_flag = match repo_arg {
        Some(r) => Some(parse_repo_token(manager, r)?),
        None => None,
    };

    let mut repo_list: Vec<String> = Vec::new();
    if let Some(list) = repos_arg {
        for token in list {
            let repo = parse_repo_token(manager, token)?;
            if !repo.is_empty() && !repo_list.contains(&repo) {
                repo_list.push(repo);
            }
        }
    }

    let resolved_ws = explicit_ws.clone().or(detected_ws);
    let has_explicit_repos =
        positional_repo.is_some() || repo_from_flag.is_some() || !repo_list.is_empty();

    let mut resolved_repo = positional_repo.or(repo_from_flag);
    // Repository inference from cwd only when no workspace or repository was
    // supplied explicitly.
    if explicit_ws.is_none() && !has_explicit_repos {
        resolved_repo = detected_repo;
    }

    let mut resolved_repos: Vec<String> = Vec::new();
    if let Some(ref r) = resolved_repo {
        resolved_repos.push(r.clone());
    }
    for r in repo_list {
        if !resolved_repos.contains(&r) {
            resolved_repos.push(r);
        }
    }

    if require_ws && resolved_ws.is_none() {
        return Err(
            "Workspace name is required. Pass '@<workspace>' or run this command from inside a workspace directory."
                .to_string(),
        );
    }

    if require_repo && resolved_repo.is_none() {
        return Err(
            "Repository name is required. Pass '%<repo>' or run this command from inside a repository worktree."
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
    fn test_normalize_cli_args_bare_name_is_not_reordered() {
        let args = vec!["my-ws".to_string(), "start".to_string()];
        let norm = normalize_cli_args(&args);
        assert_eq!(norm, vec!["my-ws", "start"]);
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
            lifecycle: None,
            schema_version: Some(ws_core::lifecycle::LIFECYCLE_SCHEMA_VERSION),
            workspace: ws_core::models::WorkspaceRootConfig::default(),
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

    #[test]
    fn test_parse_repo_token_requires_percent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manager = create_test_manager(temp_dir.path());

        assert_eq!(
            parse_repo_token(&manager, "%server").unwrap(),
            "server".to_string()
        );
        assert!(parse_repo_token(&manager, "server").is_err());
        assert!(parse_repo_token(&manager, ":server").is_err());
        assert!(parse_repo_token(&manager, "+server").is_err());
        assert!(parse_repo_token(&manager, "server")
            .unwrap_err()
            .contains('%'));
    }

    #[test]
    fn test_parse_repo_list_requires_percent_per_item() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manager = create_test_manager(temp_dir.path());

        assert_eq!(
            parse_repo_list(&manager, "%a,%b").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(parse_repo_list(&manager, "%a,b").is_err());
    }

    #[test]
    fn test_resolve_ws_and_repo_args_rejects_bare_workspace() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manager = create_test_manager(temp_dir.path());
        let err =
            resolve_ws_and_repo_args(&manager, Some("feat"), None, None, true, false).unwrap_err();
        assert!(err.contains('@'));
    }
}
