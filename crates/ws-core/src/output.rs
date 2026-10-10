use colored::Colorize;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, Table};
use std::collections::HashMap;
use std::path::Path;

use crate::models::{RepoSpec, WorkspaceMetadata};
use crate::utils::format_relative_time;

pub struct OutputHandler;

impl OutputHandler {
    /// Dynamically determine current terminal width, checking COLUMNS, crossterm, and ioctl.
    pub fn get_terminal_width() -> u16 {
        // 1. Check COLUMNS environment variable override (useful for testing and scripting)
        if let Ok(cols_str) = std::env::var("COLUMNS") {
            if let Ok(cols) = cols_str.trim().parse::<u16>() {
                if cols >= 20 {
                    return cols;
                }
            }
        }

        // 2. Try crossterm terminal size
        if let Ok((width, _)) = crossterm::terminal::size() {
            if width >= 20 {
                return width;
            }
        }

        // 3. Fallback on Unix: check ioctl TIOCGWINSZ on standard fds and /dev/tty
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;

            for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO, libc::STDIN_FILENO] {
                unsafe {
                    let mut ws = libc::winsize {
                        ws_row: 0,
                        ws_col: 0,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col >= 20 {
                        return ws.ws_col;
                    }
                }
            }

            if let Ok(tty) = std::fs::File::open("/dev/tty") {
                unsafe {
                    let mut ws = libc::winsize {
                        ws_row: 0,
                        ws_col: 0,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    if libc::ioctl(tty.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) == 0
                        && ws.ws_col >= 20
                    {
                        return ws.ws_col;
                    }
                }
            }
        }

        // 4. Default safe fallback
        80
    }

    /// Helper to initialize tables with responsive dynamic arrangement and detected terminal width.
    pub fn create_table() -> Table {
        let width = Self::get_terminal_width();
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic)
            .set_width(width);
        table
    }

    /// Helper to word-wrap text to a given maximum visual width.
    pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![text.to_string()];
        }
        let mut lines = Vec::new();
        for paragraph in text.lines() {
            if paragraph.trim().is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut current_line = String::new();
            for word in paragraph.split_whitespace() {
                if current_line.is_empty() {
                    current_line.push_str(word);
                } else if current_line.chars().count() + 1 + word.chars().count() <= width {
                    current_line.push(' ');
                    current_line.push_str(word);
                } else {
                    lines.push(current_line);
                    current_line = word.to_string();
                }
            }
            if !current_line.is_empty() {
                lines.push(current_line);
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    pub fn print_success(message: &str) {
        println!("{} {}", "✔".bold().green(), message.bold().white());
    }

    pub fn print_info(message: &str) {
        println!("{} {}", "ℹ".bold().blue(), message);
    }

    pub fn print_warning(message: &str) {
        println!("{} {}", "⚠".bold().yellow(), message.yellow());
    }

    pub fn print_err(message: &str) {
        Self::print_error(message, None);
    }

    pub fn print_error(message: &str, details: Option<&str>) {
        let term_width = Self::get_terminal_width();
        let prefix = format!("╭─ {} ", "Error".bold().red());
        let prefix_len = 3 + "Error".len() + 1;
        let dashes = term_width.saturating_sub(prefix_len as u16).max(3);
        let bottom_dashes = term_width.saturating_sub(1).max(3);
        let content_width = (term_width as usize).saturating_sub(4).max(20);

        eprintln!();
        eprintln!("{}{}", prefix, "─".repeat(dashes as usize).red());
        for line in Self::wrap_text(message, content_width) {
            eprintln!("│ {}", line.bold().red());
        }
        if let Some(d) = details {
            if !d.is_empty() {
                eprintln!("│");
                for line in d.lines() {
                    for wrapped in Self::wrap_text(line, content_width) {
                        eprintln!("│ {}", wrapped.bright_red());
                    }
                }
            }
        }
        eprintln!("╰{}", "─".repeat(bottom_dashes as usize).red());
        eprintln!();
    }

    pub fn print_workspaces_list(workspaces: &[WorkspaceMetadata]) {
        Self::print_workspace_list(workspaces);
    }

    pub fn print_env_summary(
        workspace_name: &str,
        repo_name: &str,
        env_vars: &HashMap<String, String>,
    ) {
        Self::print_env_table(workspace_name, repo_name, env_vars, None);
    }

    pub fn print_rollback_notice(reason: &str, restored: bool) {
        let term_width = Self::get_terminal_width();
        let title = "Rollback Executed";
        let prefix = format!("╭─ {} ", title.bold().yellow());
        let prefix_len = 3 + title.len() + 1;
        let dashes = term_width.saturating_sub(prefix_len as u16).max(3);
        let bottom_dashes = term_width.saturating_sub(1).max(3);
        let content_width = (term_width as usize).saturating_sub(4).max(20);

        eprintln!();
        eprintln!("{}{}", prefix, "─".repeat(dashes as usize).yellow());
        let full_reason = format!("Workspace Creation Failed: {}", reason);
        for line in Self::wrap_text(&full_reason, content_width) {
            eprintln!("│ {}", line.bold().red());
        }
        if restored {
            eprintln!("│");
            let notice = "↺ Automatic rollback executed. Filesystem and Git branches restored.";
            for line in Self::wrap_text(notice, content_width) {
                eprintln!("│ {}", line.bold().yellow());
            }
        }
        eprintln!("╰{}", "─".repeat(bottom_dashes as usize).yellow());
        eprintln!();
    }

    pub fn print_creation_header(name: &str, repo_specs: &[RepoSpec]) {
        println!();
        println!(
            "{} {}",
            "✔".bold().green(),
            format!("Creating workspace {}", name.cyan()).bold().white()
        );
        println!();
        println!("{}", "Repositories".bold().cyan());
        println!();

        for spec in repo_specs {
            let mode_str = if spec.create {
                "NEW".green()
            } else {
                "EXISTING".yellow()
            };
            println!("  {}", spec.name.to_uppercase().bold().underline());
            println!("      Branch : {}", spec.branch.bold().white());
            println!("      Mode   : {}", mode_str);
            println!();
        }

        let term_width = Self::get_terminal_width();
        println!("{}", "─".repeat(term_width as usize).dimmed());
        println!();
    }

    pub fn print_creation_success(name: &str, workspace_path: &Path) {
        println!("{}", "✔ Workspace created".bold().green());
        println!();
        println!("{}", "Location".bold().cyan());
        let _ = name;
        println!(
            "  {}",
            workspace_path.display().to_string().bold().bright_blue()
        );
        println!();
    }

    pub fn print_workspace_list(workspaces: &[WorkspaceMetadata]) {
        if workspaces.is_empty() {
            println!("{}", "No workspaces found.".dimmed());
            return;
        }

        let mut table = Self::create_table();
        table.set_header(vec!["NAME", "STATUS", "CREATED", "REPOSITORIES"]);

        let mut sorted = workspaces.to_vec();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));

        for ws in sorted {
            let rel_time = format_relative_time(&ws.created);

            let mut repo_parts = Vec::new();
            for (name, spec) in &ws.repositories {
                let mut part = format!("{}:{}", name, spec.branch);
                if spec.frozen || spec.locked {
                    part.push_str(" [🔒 frozen]");
                }
                repo_parts.push(part);
            }
            let repo_summary = repo_parts.join(", ");

            let status_cell = if ws.status == "active" {
                Cell::new(&ws.status).fg(Color::Green)
            } else {
                Cell::new(&ws.status).fg(Color::Yellow)
            };

            table.add_row(vec![
                Cell::new(&ws.name).fg(Color::Cyan),
                status_cell,
                Cell::new(rel_time).fg(Color::DarkGrey),
                Cell::new(repo_summary).fg(Color::White),
            ]);
        }

        println!("{}", table);
    }

    pub fn print_workspace_info(
        metadata: &WorkspaceMetadata,
        workspace_path: &Path,
        active_engine: Option<&str>,
        running_services: Option<&HashMap<String, serde_json::Value>>,
    ) {
        let term_width = Self::get_terminal_width();
        let title = format!("Workspace Info: @{}", metadata.name);
        let prefix = format!("╭─ {} ", title.bold().green());
        let prefix_len = 3 + title.chars().count() + 1;
        let dashes = term_width.saturating_sub(prefix_len as u16).max(3);
        let bottom_dashes = term_width.saturating_sub(1).max(3);

        println!();
        println!("{}{}", prefix, "─".repeat(dashes as usize).dimmed());
        let created_str = if metadata.created.is_empty() {
            "-"
        } else {
            &metadata.created
        };
        let rel_time = format_relative_time(&metadata.created);
        println!("│ Created: {} ({})", created_str.bold(), rel_time.dimmed());

        if let Some(engine) = active_engine {
            println!("│ Engine Session: Active ({})", engine.bold().green());
        } else {
            println!("│ Engine Session: {}", "Inactive".dimmed());
        }

        println!("│ Status: {}", metadata.status.green());
        println!("│ Path: {}", workspace_path.display().to_string().cyan());
        println!("│");
        println!("│ {}", "Repositories & Services:".bold().cyan());

        let default_services = HashMap::new();
        let running_services = running_services.unwrap_or(&default_services);

        for (repo_name, spec) in &metadata.repositories {
            let mode_badge = if spec.create {
                "new".green()
            } else {
                "existing".yellow()
            };
            let locked_badge = if spec.frozen || spec.locked {
                " 🔒 LOCKED".bold().yellow()
            } else {
                "".clear()
            };

            let svc_info = running_services.get(repo_name);
            let process_badge = if let Some(info) = svc_info {
                let port_text = if let Some(ports) = info.get("ports").and_then(|p| p.as_array()) {
                    let p_str: Vec<String> = ports
                        .iter()
                        .filter_map(|v| v.as_i64())
                        .map(|p| format!(":{}", p))
                        .collect();
                    if !p_str.is_empty() {
                        format!(" (ports {})", p_str.join(", ").cyan())
                    } else {
                        String::new()
                    }
                } else if let Some(port) = info.get("port").and_then(|p| p.as_i64()) {
                    if port > 0 {
                        format!(" (port :{})", port.to_string().cyan())
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                format!(" ● RUNNING{}", port_text).bold().green()
            } else if active_engine.is_some() {
                " ○ stopped".dimmed()
            } else {
                "".clear()
            };

            println!(
                "│   • %{} ({}){}{}",
                repo_name.bold().magenta(),
                mode_badge,
                locked_badge,
                process_badge
            );
            println!("│       Branch: {}", spec.branch.bold().white());
            println!("│       Worktree Path: {}", spec.path.dimmed());

            if let Some(info) = svc_info {
                let status_val = info
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("running");
                println!("│       Process Status: {}", status_val.green());
                if let Some(url_local) = info.get("url_local").and_then(|u| u.as_str()) {
                    println!("│       Local URL: {}", url_local.bold().cyan());
                }
                if let Some(url_lan) = info.get("url_lan").and_then(|u| u.as_str()) {
                    if !url_lan.starts_with("http://127.") {
                        println!("│       LAN Wi-Fi URL: {}", url_lan.bold().yellow());
                    }
                }
            }

            if spec.frozen || spec.locked {
                println!("│       File Mode: {}", "Read-only (locked)".yellow());
            }
        }
        println!("╰{}", "─".repeat(bottom_dashes as usize).dimmed());
        println!();
    }

    pub fn print_push_summary(
        workspace_name: &str,
        results: &HashMap<String, HashMap<String, String>>,
    ) {
        let mut table = Self::create_table();
        table.set_header(vec![
            "REPOSITORY",
            "STATUS",
            "BRANCH",
            "REMOTE",
            "DETAILS / REASON",
        ]);

        for (repo_name, res) in results {
            let status = res.get("status").map(|s| s.as_str()).unwrap_or("unknown");
            let branch = res.get("branch").map(|s| s.as_str()).unwrap_or("-");
            let remote = res.get("remote").map(|s| s.as_str()).unwrap_or("origin");
            let reason = res.get("reason").map(|s| s.as_str()).unwrap_or("");

            let status_cell = match status {
                "pushed" => Cell::new("✔ PUSHED (COMMITS)").fg(Color::Green),
                "up-to-date" => Cell::new("ℹ UP TO DATE").fg(Color::Cyan),
                "skipped" => Cell::new("⏭ SKIPPED").fg(Color::Yellow),
                _ => Cell::new("✘ FAILED").fg(Color::Red),
            };

            table.add_row(vec![
                Cell::new(repo_name).fg(Color::White),
                status_cell,
                Cell::new(branch).fg(Color::White),
                Cell::new(remote).fg(Color::DarkGrey),
                Cell::new(reason).fg(Color::DarkGrey),
            ]);
        }

        println!("Push Summary for Workspace '{}'", workspace_name);
        println!("{}", table);
    }

    pub fn print_pull_summary(
        workspace_name: &str,
        results: &HashMap<String, HashMap<String, String>>,
    ) {
        let mut table = Self::create_table();
        table.set_header(vec![
            "REPOSITORY",
            "STATUS",
            "BRANCH",
            "REMOTE",
            "DETAILS / REASON",
        ]);

        for (repo_name, res) in results {
            let status = res.get("status").map(|s| s.as_str()).unwrap_or("unknown");
            let branch = res.get("branch").map(|s| s.as_str()).unwrap_or("-");
            let remote = res.get("remote").map(|s| s.as_str()).unwrap_or("origin");
            let reason = res.get("reason").map(|s| s.as_str()).unwrap_or("");

            let status_cell = match status {
                "pulled" => Cell::new("✔ PULLED (UPDATED)").fg(Color::Green),
                "up-to-date" => Cell::new("ℹ UP TO DATE").fg(Color::Cyan),
                "skipped" => Cell::new("⏭ SKIPPED").fg(Color::Yellow),
                _ => Cell::new("✘ FAILED").fg(Color::Red),
            };

            table.add_row(vec![
                Cell::new(repo_name).fg(Color::White),
                status_cell,
                Cell::new(branch).fg(Color::White),
                Cell::new(remote).fg(Color::DarkGrey),
                Cell::new(reason).fg(Color::DarkGrey),
            ]);
        }

        println!("Pull Summary for Workspace '{}'", workspace_name);
        println!("{}", table);
    }

    pub fn print_setup_summary(workspace_name: &str, results: &HashMap<String, serde_json::Value>) {
        let mut table = Self::create_table();
        table.set_header(vec![
            "REPOSITORY",
            "STATUS",
            "ENV SYNC",
            "DETAILS / COMMANDS",
        ]);

        for (repo_name, res) in results {
            let status = res
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            let env_status = res
                .get("env_status")
                .and_then(|s| s.as_str())
                .unwrap_or("-");
            let reason = res.get("reason").and_then(|s| s.as_str()).unwrap_or("");
            let cmds: Vec<String> = res
                .get("commands_run")
                .and_then(|c| c.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            let status_cell = match status {
                "completed" => Cell::new("✔ COMPLETED").fg(Color::Green),
                "skipped" => Cell::new("⏭ SKIPPED").fg(Color::Yellow),
                _ => Cell::new("✘ FAILED").fg(Color::Red),
            };

            let mut cmd_details = reason.to_string();
            if !cmds.is_empty() && status == "completed" {
                cmd_details = format!("{} ({})", reason, cmds.join(", "));
            }

            table.add_row(vec![
                Cell::new(repo_name).fg(Color::White),
                status_cell,
                Cell::new(env_status).fg(Color::DarkGrey),
                Cell::new(cmd_details).fg(Color::DarkGrey),
            ]);
        }

        println!("Setup Summary for Workspace '{}'", workspace_name);
        println!("{}", table);
    }

    pub fn print_setup_repo_start(repo_name: &str, path: &Path) {
        println!();
        println!(
            "📦 [{}] {}",
            repo_name.to_uppercase().bold().cyan(),
            format!("({})", path.display()).dimmed()
        );
    }

    pub fn print_step_start(step_number: usize, action: &str) {
        println!(
            "  {} Step {}: {}...",
            "❯".cyan(),
            step_number.to_string().bold().white(),
            action.dimmed()
        );
    }

    pub fn print_setup_step(step_number: usize, title: &str, details: &str, status: &str) {
        let (icon, title_styled) = match status {
            "success" => ("✔".bold().green(), title.bold().white()),
            "info" => ("ℹ".bold().blue(), title.white()),
            "warning" => ("⚠".bold().yellow(), title.yellow()),
            _ => ("✘".bold().red(), title.bold().red()),
        };

        println!(
            "  {} Step {}: {} — {}",
            icon,
            step_number,
            title_styled,
            details.dimmed()
        );
    }

    pub fn print_command_start(command: &str) {
        println!("  ❯ Running: {} ...", command.bold().cyan());
    }

    pub fn print_command_done(command: &str, elapsed_seconds: f64, success: bool, returncode: i32) {
        if success {
            println!(
                "  {} Completed: {} ({:.2}s)",
                "✔".bold().green(),
                command.cyan(),
                elapsed_seconds
            );
        } else {
            println!(
                "  {} Failed: {} (exit {} in {:.2}s)",
                "✘".bold().red(),
                command.cyan(),
                returncode,
                elapsed_seconds
            );
        }
    }

    pub fn print_env_resolution_details(
        env_vars: &HashMap<String, String>,
        explicit_secrets: Option<&[String]>,
    ) {
        use crate::models::is_secret_val;

        println!("    Resolved Environment Variables:");
        if env_vars.is_empty() {
            println!("      (none)");
            return;
        }

        let mut sorted: Vec<(&String, &String)> = env_vars.iter().collect();
        sorted.sort_by_key(|(k, _)| (*k).clone());

        for (k, v) in sorted {
            let is_sec = explicit_secrets
                .map(|s| s.iter().any(|sec| sec == k))
                .unwrap_or(false)
                || is_secret_val(k);
            let display_val = if is_sec {
                "********".yellow()
            } else {
                v.green()
            };
            println!("      • {} = {}", k.cyan(), display_val);
        }
    }

    pub fn print_command_output(command: &str, stdout: &str, stderr: &str, returncode: i32) {
        println!();
        let term_width = Self::get_terminal_width();
        let title = format!("Command: {} (exit {})", command, returncode);
        let prefix = if returncode == 0 {
            format!("╭─ {} ", title.bold().green())
        } else {
            format!("╭─ {} ", title.bold().red())
        };
        let prefix_len = 3 + title.chars().count() + 1;
        let dashes = term_width.saturating_sub(prefix_len as u16).max(3);
        let bottom_dashes = term_width.saturating_sub(1).max(3);

        if returncode == 0 {
            println!("{}{}", prefix, "─".repeat(dashes as usize).dimmed());
        } else {
            println!("{}{}", prefix, "─".repeat(dashes as usize).red());
        }

        let content_width = (term_width as usize).saturating_sub(4).max(20);

        if !stdout.trim().is_empty() {
            println!("│ {}", "Output:".bold().white());
            for line in stdout.trim().lines() {
                for wrapped in Self::wrap_text(line, content_width) {
                    println!("│ {}", wrapped);
                }
            }
        }
        if !stderr.trim().is_empty() {
            println!("│ {}", "Errors/Warnings:".bold().red());
            for line in stderr.trim().lines() {
                for wrapped in Self::wrap_text(line, content_width) {
                    println!("│ {}", wrapped.red());
                }
            }
        }
        if stdout.trim().is_empty() && stderr.trim().is_empty() {
            println!("│ {}", "(no console output)".dimmed());
        }

        if returncode == 0 {
            println!("╰{}", "─".repeat(bottom_dashes as usize).dimmed());
        } else {
            println!("╰{}", "─".repeat(bottom_dashes as usize).red());
        }
    }

    pub fn print_env_table(
        workspace_name: &str,
        repo_name: &str,
        env_vars: &HashMap<String, String>,
        explicit_secrets: Option<&[String]>,
    ) {
        use crate::models::is_secret_val;

        let mut table = Self::create_table();
        table.set_header(vec!["VARIABLE", "RESOLVED VALUE"]);

        if env_vars.is_empty() {
            table.add_row(vec![
                Cell::new("(none)").fg(Color::DarkGrey),
                Cell::new("No environment variables configured").fg(Color::DarkGrey),
            ]);
        } else {
            let mut sorted: Vec<(&String, &String)> = env_vars.iter().collect();
            sorted.sort_by_key(|(k, _)| (*k).clone());

            for (k, v) in sorted {
                let is_sec = explicit_secrets
                    .map(|s| s.iter().any(|sec| sec == k))
                    .unwrap_or(false)
                    || is_secret_val(k);
                let val_cell = if is_sec {
                    Cell::new("******** (masked secret)").fg(Color::Yellow)
                } else {
                    Cell::new(v).fg(Color::Green)
                };
                table.add_row(vec![Cell::new(k).fg(Color::Cyan), val_cell]);
            }
        }

        println!(
            "Environment Variables for '{}' in '{}'",
            repo_name, workspace_name
        );
        println!("{}", table);
    }

    pub fn print_launch_summary(workspace_name: &str, launch_entries: &[(String, String, String)]) {
        let mut table = Self::create_table();
        table.set_header(vec!["REPOSITORY", "WORKING DIRECTORY", "LAUNCH COMMAND"]);

        if launch_entries.is_empty() {
            table.add_row(vec![
                Cell::new("(none)").fg(Color::DarkGrey),
                Cell::new("-").fg(Color::DarkGrey),
                Cell::new("No launch commands configured in repositories.yml").fg(Color::DarkGrey),
            ]);
        } else {
            for (repo, wt_dir, cmd) in launch_entries {
                table.add_row(vec![
                    Cell::new(repo).fg(Color::White),
                    Cell::new(wt_dir).fg(Color::DarkGrey),
                    Cell::new(cmd).fg(Color::Green),
                ]);
            }
        }

        println!("Launch Commands for Workspace '{}'", workspace_name);
        println!("{}", table);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_terminal_width_env_override() {
        std::env::set_var("COLUMNS", "65");
        assert_eq!(OutputHandler::get_terminal_width(), 65);

        std::env::set_var("COLUMNS", "120");
        assert_eq!(OutputHandler::get_terminal_width(), 120);

        std::env::remove_var("COLUMNS");
        let detected = OutputHandler::get_terminal_width();
        assert!(detected >= 20);
    }

    #[test]
    fn test_create_table_uses_terminal_width() {
        std::env::set_var("COLUMNS", "72");
        let table = OutputHandler::create_table();
        assert_eq!(table.width(), Some(72));
        assert!(matches!(
            table.content_arrangement(),
            comfy_table::ContentArrangement::Dynamic
        ));
        std::env::remove_var("COLUMNS");
    }

    #[test]
    fn test_table_dynamic_width() {
        for width in [65, 80, 100, 120] {
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_content_arrangement(comfy_table::ContentArrangement::Dynamic)
                .set_width(width)
                .set_header(vec!["NAME", "STATUS", "CREATED", "REPOSITORIES"]);

            table.add_row(vec![
                Cell::new("full-rust-migration"),
                Cell::new("active"),
                Cell::new("3h ago"),
                Cell::new("web:feature/full-rust-migration, hub:feature/full-rust-migration, manager:feature/full-rust-migration"),
            ]);

            table.add_row(vec![
                Cell::new("main"),
                Cell::new("active"),
                Cell::new("16 days ago"),
                Cell::new("manager:main, web:main, hub:main"),
            ]);

            let rendered = table.to_string();
            println!("\n=== RENDERED (width {}) ===\n{}", width, rendered);
            for line in rendered.lines() {
                assert!(
                    line.chars().count() <= width as usize,
                    "Line exceeds width {}: {} (len {})",
                    width,
                    line,
                    line.chars().count()
                );
            }
        }
    }

    #[test]
    fn test_wrap_text() {
        let msg = "Explicit repository selection required for setup in workspace '@full-rust-migration'. Specify '--all' to setup all repositories, or specify repositories using '%repo1 %repo2' or '--repos r1,r2'.";
        let wrapped = OutputHandler::wrap_text(msg, 60);
        assert!(wrapped.len() > 1);
        for line in &wrapped {
            assert!(
                line.chars().count() <= 60,
                "Line exceeds width 60: '{}' ({})",
                line,
                line.chars().count()
            );
        }
    }
}
