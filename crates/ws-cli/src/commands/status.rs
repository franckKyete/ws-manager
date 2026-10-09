use ws_core::WorkspaceManager;
use crate::helpers::resolve_ws_and_repo_args;

pub fn execute_status(manager: &WorkspaceManager, name: Option<&str>) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    let statuses = manager.status_workspace(&ws_name).map_err(|e| e.to_string())?;
    for (repo, status) in statuses {
        println!("{} ({}):", repo, ws_name);
        if status.is_empty() {
            println!("  clean");
        } else {
            for line in status.lines() {
                println!("  {}", line);
            }
        }
    }
    Ok(())
}
