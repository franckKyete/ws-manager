use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;
use crate::helpers::resolve_ws_and_repo_args;

pub fn execute_env(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    repo: Option<&str>,
    sync: bool,
    interface: Option<&str>,
    lan_ip: Option<&str>,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, repo, None, true, false)?;
    let target_repo = repo.or(repo_name.as_deref());

    if sync {
        let repos = target_repo.map(|r| vec![r.to_string()]);
        let results = manager
            .sync_env(&ws_name, repos.as_deref(), interface, lan_ip)
            .map_err(|e| e.to_string())?;
        OutputHandler::print_setup_summary(&ws_name, &results);
    } else {
        let (meta, _) = manager.get_workspace_info(&ws_name).map_err(|e| e.to_string())?;
        let target_repos: Vec<String> = if let Some(r) = target_repo {
            vec![r.to_string()]
        } else {
            meta.repositories.keys().cloned().collect()
        };

        for r in target_repos {
            let env_vars = manager.get_env_vars(&ws_name, &r, interface, lan_ip);
            OutputHandler::print_env_summary(&ws_name, &r, &env_vars);
        }
    }

    Ok(())
}
