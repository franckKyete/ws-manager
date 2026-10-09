use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;
use crate::helpers::{clean_repo, resolve_ws_and_repo_args};

pub fn execute_setup(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    repos: Option<&[String]>,
    all: bool,
    repos_flag: Option<&str>,
    dry_run: bool,
    skip_scripts: bool,
    verbose: bool,
    interface: Option<&str>,
    lan_ip: Option<&str>,
) -> Result<(), String> {
    let (ws_name, _, resolved_repos) = resolve_ws_and_repo_args(manager, name, None, repos, true, false)?;
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
    } else if !all {
        let (_, detected_repo) = manager.detect_context(None);
        if let Some(dr) = detected_repo {
            target_repos = Some(vec![dr]);
        }
    }

    if !all && target_repos.is_none() {
        return Err(format!(
            "Explicit repository selection required for setup in workspace '@{}'. Specify '--all' to setup all repositories, or specify repositories using '%repo1 %repo2' or '--repos r1,r2'.",
            ws_name
        ));
    }

    let filter = if all { None } else { target_repos };

    let results = manager
        .setup_workspace(
            &ws_name,
            filter.as_deref(),
            dry_run,
            skip_scripts,
            verbose,
            interface,
            lan_ip,
        )
        .map_err(|e| e.to_string())?;

    OutputHandler::print_setup_summary(&ws_name, &results);
    Ok(())
}
