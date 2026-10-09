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
        eprintln!();
        eprintln!("╭─ {} ────────────────────────────╮", "Error".bold().red());
        eprintln!("│ {}", message.bold().red());
        if let Some(d) = details {
            if !d.is_empty() {
                eprintln!("│");
                for line in d.lines() {
                    eprintln!("│ {}", line.bright_red());
                }
            }
        }
        eprintln!("╰──────────────────────────────────────────╯");
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
        eprintln!();
        eprintln!(
            "╭─ {} ───────────────────╮",
            "Rollback Executed".bold().yellow()
        );
        eprintln!("│ {}: {}", "Workspace Creation Failed".bold().red(), reason);
        if restored {
            eprintln!("│");
            eprintln!(
                "│ {} Filesystem and Git branches restored.",
                "↺ Automatic rollback executed.".bold().yellow()
            );
        }
        eprintln!("╰──────────────────────────────────────────╯");
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

        println!("{}", "─".repeat(60).dimmed());
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

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["NAME", "STATUS", "CREATED", "REPOSITORIES"]);

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
        println!();
        println!(
            "╭─ {} ───────────────────────",
            format!("Workspace Info: @{}", metadata.name).bold().green()
        );
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
        println!("╰──────────────────────────────────────────╯");
        println!();
    }

    pub fn print_push_summary(
        workspace_name: &str,
        results: &HashMap<String, HashMap<String, String>>,
    ) {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
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
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
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
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
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
        let border = if returncode == 0 { "green" } else { "red" };
        println!(
            "╭─ Command: {} (exit {}) ──────────────────────────",
            command, returncode
        );
        if !stdout.trim().is_empty() {
            println!("Output:\n{}", stdout.trim());
        }
        if !stderr.trim().is_empty() {
            println!("Errors/Warnings:\n{}", stderr.trim().red());
        }
        if stdout.trim().is_empty() && stderr.trim().is_empty() {
            println!("(no console output)");
        }
        println!("╰───────────────────────────────────────────────────");
        let _ = border;
    }

    pub fn print_env_table(
        workspace_name: &str,
        repo_name: &str,
        env_vars: &HashMap<String, String>,
        explicit_secrets: Option<&[String]>,
    ) {
        use crate::models::is_secret_val;

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["VARIABLE", "RESOLVED VALUE"]);

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
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["REPOSITORY", "WORKING DIRECTORY", "LAUNCH COMMAND"]);

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
