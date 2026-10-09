use ws_core::WorkspaceManager;
use crate::helpers::resolve_ws_and_repo_args;

pub fn execute_end(
    manager: &WorkspaceManager,
    name: Option<&str>,
    force: bool,
    no_merge: bool,
    delete_branch: bool,
    target_branch: Option<&str>,
    no_tmux: bool,
) -> Result<(), String> {
    let (ws_name, _, _) = resolve_ws_and_repo_args(manager, name, None, None, true, false)?;
    manager
        .end_workspace(&ws_name, force, no_merge, delete_branch, target_branch, no_tmux)
        .map_err(|e| e.to_string())
}
