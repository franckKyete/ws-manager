use crate::helpers::resolve_ws_and_repo_args;
use ws_core::WorkspaceManager;

pub fn execute_focus(manager: &WorkspaceManager, name: Option<&str>) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    manager
        .focus_workspace(&ws_name)
        .map_err(|e| e.to_string())?;
    Ok(())
}
