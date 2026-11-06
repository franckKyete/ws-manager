use colored::Colorize;
use comfy_table::{Cell, Color};
use std::path::Path;
use ws_core::config::ConfigLoader;
use ws_core::daemon::get_global_cache;
use ws_core::output::OutputHandler;
use ws_core::registry::{list_registered_projects, register_project, unregister_project};
use ws_core::utils::format_relative_time;

pub fn print_registered_projects_table(title: &str) {
    let projects = list_registered_projects(None, true);
    if projects.is_empty() {
        OutputHandler::print_info("No projects registered in global registry (~/.config/ws/projects.json).");
        return;
    }

    let cache = get_global_cache();
    let mut table = OutputHandler::create_table();
    table.set_header(vec![
        Cell::new("Project").fg(Color::Cyan),
        Cell::new("Directory").fg(Color::DarkGrey),
        Cell::new("Auto-Save").fg(Color::White),
        Cell::new("Interval").fg(Color::White),
        Cell::new("Last Saved").fg(Color::Green),
    ]);

    for p in projects {
        let p_name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string());
        let p_str = p.to_string_lossy().to_string();
        let cfg_path = p.join("repositories.yml");

        let mut auto_save_cell = Cell::new("disabled").fg(Color::DarkGrey);
        let mut interval_cell = Cell::new("-").fg(Color::DarkGrey);

        if cfg_path.is_file() {
            if let Ok(cfg) = ConfigLoader::load_config(Some(&cfg_path), None, true) {
                if let Some(ref auto_cfg) = cfg.hub_auto_save {
                    if auto_cfg.enabled {
                        auto_save_cell = Cell::new("enabled").fg(Color::Green);
                        let sec = auto_cfg.interval;
                        let interval_str = if sec % 60 == 0 {
                            format!("{}m", sec / 60)
                        } else {
                            format!("{}s", sec)
                        };
                        interval_cell = Cell::new(interval_str).fg(Color::White);
                    }
                }
            }
        }

        let p_entry = cache.get(&p_str).and_then(|v| v.as_object());
        let last_saved_str = p_entry
            .and_then(|e| e.get("last_saved_at"))
            .and_then(|v| v.as_str())
            .map(format_relative_time)
            .unwrap_or_else(|| "never".to_string());

        let last_saved_cell = if last_saved_str == "never" {
            Cell::new(last_saved_str).fg(Color::DarkGrey)
        } else {
            Cell::new(last_saved_str).fg(Color::Green)
        };

        table.add_row(vec![
            Cell::new(p_name).fg(Color::Cyan),
            Cell::new(p_str).fg(Color::DarkGrey),
            auto_save_cell,
            interval_cell,
            last_saved_cell,
        ]);
    }

    println!("{}", title.bold().cyan());
    println!("{}", table);
}

pub fn execute_project_list() -> Result<(), String> {
    print_registered_projects_table("Registered Projects");
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
        OutputHandler::print_info(&format!(
            "Project at '{}' is already registered",
            target.display()
        ));
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
        OutputHandler::print_warning(&format!(
            "Project at '{}' was not registered",
            target.display()
        ));
    }
    Ok(())
}
