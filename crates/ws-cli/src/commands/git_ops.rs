use crate::helpers::{clean_repo, resolve_ws_and_repo_args};
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_push(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repos: Option<&[String]>,
    repos_flag: Option<&str>,
    remote: Option<&str>,
) -> Result<(), String> {
    let (ws_name, _, resolved_repos) =
        resolve_ws_and_repo_args(manager, name, None, repos, true, false)?;
    let mut target_repos: Option<Vec<String>> = None;

    if let Some(rf) = repos_flag {
        let set: Vec<String> = rf
            .split(',')
            .filter(|r| !r.trim().is_empty())
            .map(|r| clean_repo(Some(r.trim())).unwrap())
            .collect();
        target_repos = Some(set);
    } else if !resolved_repos.is_empty() {
        target_repos = Some(resolved_repos);
    }

    let rem = remote.unwrap_or("origin");
    let results = manager
        .push_workspace(&ws_name, target_repos.as_deref(), rem)
        .map_err(|e| e.to_string())?;

    OutputHandler::print_push_summary(&ws_name, &results);
    Ok(())
}

pub fn execute_pull(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repos: Option<&[String]>,
    repos_flag: Option<&str>,
    remote: Option<&str>,
) -> Result<(), String> {
    let (ws_name, _, resolved_repos) =
        resolve_ws_and_repo_args(manager, name, None, repos, true, false)?;
    let mut target_repos: Option<Vec<String>> = None;

    if let Some(rf) = repos_flag {
        let set: Vec<String> = rf
            .split(',')
            .filter(|r| !r.trim().is_empty())
            .map(|r| clean_repo(Some(r.trim())).unwrap())
            .collect();
        target_repos = Some(set);
    } else if !resolved_repos.is_empty() {
        target_repos = Some(resolved_repos);
    }

    let rem = remote.unwrap_or("origin");
    let results = manager
        .pull_workspace(&ws_name, target_repos.as_deref(), rem)
        .map_err(|e| e.to_string())?;

    OutputHandler::print_pull_summary(&ws_name, &results);
    Ok(())
}

pub fn execute_fetch(manager: &WorkspaceManager) -> Result<(), String> {
    let _ = manager.fetch_repositories();
    OutputHandler::print_success("Fetched all bare repositories");
    Ok(())
}

pub fn execute_sync(manager: &WorkspaceManager) -> Result<(), String> {
    let _ = manager.fetch_repositories();
    OutputHandler::print_success("Synced and pruned repositories");
    Ok(())
}
