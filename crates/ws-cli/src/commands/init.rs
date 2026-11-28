use crate::prompts::{interactive_tty, DialoguerBaseBranchResolver};
use ws_core::base_branch::BaseBranchOptions;
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_init(
    manager: &mut WorkspaceManager,
    repo_inputs: &[String],
    yes: bool,
) -> Result<(), String> {
    let mut resolver = DialoguerBaseBranchResolver;
    let mut options = if !yes && interactive_tty() {
        BaseBranchOptions::interactive(&mut resolver)
    } else {
        BaseBranchOptions::non_interactive()
    };
    manager
        .init_project_with_options(repo_inputs, &mut options)
        .map_err(|e| e.to_string())?;
    OutputHandler::print_success("Initialized project configuration in ws-project.yml");
    Ok(())
}

pub fn execute_add(
    manager: &mut WorkspaceManager,
    repo_input: &str,
    yes: bool,
) -> Result<(), String> {
    let mut resolver = DialoguerBaseBranchResolver;
    let mut options = if !yes && interactive_tty() {
        BaseBranchOptions::interactive(&mut resolver)
    } else {
        BaseBranchOptions::non_interactive()
    };
    manager
        .init_project_with_options(&[repo_input.to_string()], &mut options)
        .map_err(|e| e.to_string())?;
    let (name, _, _, _) = WorkspaceManager::parse_repo_url(repo_input);
    OutputHandler::print_success(&format!("Added repository '{}' to ws-project.yml", name));
    Ok(())
}
