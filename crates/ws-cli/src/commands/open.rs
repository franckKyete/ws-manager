use ws_core::WorkspaceManager;
use crate::helpers::resolve_ws_and_repo_args;

pub fn execute_open(
    manager: &WorkspaceManager,
    name: Option<&str>,
    worktree: Option<&str>,
) -> Result<(), String> {
    let (ws_name, repo_name, _) = resolve_ws_and_repo_args(manager, name, worktree, None, true, false)?;
    let target_wt = worktree.or(repo_name.as_deref());
    manager
        .open_workspace(&ws_name, target_wt)
        .map_err(|e| e.to_string())
}
