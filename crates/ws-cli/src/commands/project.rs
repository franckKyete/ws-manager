use std::path::Path;
use ws_core::output::OutputHandler;
use ws_core::registry::{list_registered_projects, register_project, unregister_project};

pub fn execute_project_list() -> Result<(), String> {
    let projects = list_registered_projects(None, true);
    if projects.is_empty() {
        println!("No registered projects found.");
    } else {
        println!("Registered Projects:");
        for p in projects {
            println!("  {}", p.display());
        }
    }
    Ok(())
}

pub fn execute_project_register(path: Option<&Path>) -> Result<(), String> {
    let target = match path {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };

    if register_project(&target, None) {
        OutputHandler::print_success(&format!("Registered project at '{}'", target.display()));
    } else {
        OutputHandler::print_info(&format!("Project at '{}' is already registered", target.display()));
    }
    Ok(())
}

pub fn execute_project_unregister(path: Option<&Path>) -> Result<(), String> {
    let target = match path {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };

    if unregister_project(&target, None) {
        OutputHandler::print_success(&format!("Unregistered project at '{}'", target.display()));
    } else {
        OutputHandler::print_warning(&format!("Project at '{}' was not registered", target.display()));
    }
    Ok(())
}
