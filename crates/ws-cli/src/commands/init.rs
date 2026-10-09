use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_init(manager: &mut WorkspaceManager, repo_inputs: &[String]) -> Result<(), String> {
    manager.init_project(repo_inputs).map_err(|e| e.to_string())?;
    OutputHandler::print_success("Initialized project configuration in repositories.yml");
    Ok(())
}

pub fn execute_add(manager: &mut WorkspaceManager, repo_input: &str) -> Result<(), String> {
    manager.init_project(&[repo_input.to_string()]).map_err(|e| e.to_string())?;
    OutputHandler::print_success(&format!("Added repository '{}' to repositories.yml", repo_input));
    Ok(())
}
