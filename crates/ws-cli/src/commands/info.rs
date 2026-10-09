use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;
use crate::helpers::resolve_ws_and_repo_args;

pub fn execute_info(manager: &WorkspaceManager, name: Option<&str>) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    let (meta, ws_path) = manager.get_workspace_info(&ws_name).map_err(|e| e.to_string())?;
    let active_engine = manager.get_active_engine(&ws_name);
    let running_services = manager.get_running_services_status(&ws_name);

    OutputHandler::print_workspace_info(
        &meta,
        &ws_path,
        active_engine.as_deref(),
        running_services.as_ref(),
    );
    Ok(())
}
