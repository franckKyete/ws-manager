use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_list(manager: &WorkspaceManager) -> Result<(), String> {
    let workspaces = manager.list_workspaces();
    OutputHandler::print_workspaces_list(&workspaces);
    Ok(())
}
