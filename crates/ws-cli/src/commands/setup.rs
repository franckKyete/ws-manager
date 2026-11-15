use crate::helpers::resolve_ws_and_repo_args;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_setup(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    repos: &[String],
    all: bool,
    dry_run: bool,
    skip_scripts: bool,
    verbose: bool,
    interface: Option<&str>,
    lan_ip: Option<&str>,
) -> Result<(), String> {
    // `--all` must suppress repository inference from the current directory.
    let detected_name: Option<String> = if all && name.is_none() {
        manager.detect_context(None).0.map(|ws| format!("@{}", ws))
    } else {
        None
    };
    let name_arg = name.or(detected_name.as_deref());

    let (ws_name, _, resolved_repos) =
        resolve_ws_and_repo_args(manager, name_arg, None, Some(repos), true, false)?;

    if all && !resolved_repos.is_empty() {
        return Err(
            "'--all' cannot be combined with explicit repository targets. Remove the '%repo' arguments or drop '--all'."
                .to_string(),
        );
    }

    let target_repos: Option<Vec<String>> = if all {
        None
    } else if !resolved_repos.is_empty() {
        Some(resolved_repos)
    } else {
        let (_, detected_repo) = manager.detect_context(None);
        detected_repo.map(|repo| vec![repo])
    };

    if !all && target_repos.is_none() {
        return Err(format!(
            "Explicit repository selection required for setup in workspace '@{}'. Specify '%repo ...' or '--all'.",
            ws_name
        ));
    }

    let results = manager
        .setup_workspace(
            &ws_name,
            target_repos.as_deref(),
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
