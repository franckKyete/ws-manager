use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TmuxLauncher;

impl TmuxLauncher {
    pub fn is_available() -> bool {
        Command::new("tmux").arg("-V").output().map_or(false, |o| o.status.success())
    }

    pub fn session_name(project_name: &str) -> String {
        format!("ws-{}", project_name)
    }

    pub fn is_session_active(session_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        Command::new("tmux")
            .args(["has-session", "-t", session_name])
            .output()
            .map_or(false, |o| o.status.success())
    }

    pub fn is_window_active(session_name: &str, window_name: &str) -> bool {
        if !Self::is_session_active(session_name) {
            return false;
        }
        if let Ok(out) = Command::new("tmux")
            .args(["list-windows", "-t", session_name, "-F", "#{window_name}"])
            .output()
        {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                return stdout.lines().any(|l| l.trim() == window_name);
            }
        }
        false
    }

    pub fn is_window_running(project_name: &str, window_name: &str) -> bool {
        let sess = Self::session_name(project_name);
        Self::is_window_active(&sess, window_name)
    }

    pub fn create_workspace_window(
        session_name: &str,
        window_name: &str,
        cwd: &Path,
        command: Option<&str>,
        switch: bool,
    ) -> bool {
        if !Self::is_available() {
            return false;
        }
        let cwd_str = cwd.to_string_lossy();

        if !Self::is_session_active(session_name) {
            let mut args = vec!["new-session", "-d", "-s", session_name, "-n", window_name, "-c", &cwd_str];
            if let Some(cmd) = command {
                args.push(cmd);
            }
            if let Ok(out) = Command::new("tmux").args(&args).output() {
                if !out.status.success() {
                    return false;
                }
            } else {
                return false;
            }
        } else if !Self::is_window_active(session_name, window_name) {
            let target = format!("{}:", session_name);
            let mut args = vec!["new-window", "-d", "-t", &target, "-n", window_name, "-c", &cwd_str];
            if let Some(cmd) = command {
                args.push(cmd);
            }
            if let Ok(out) = Command::new("tmux").args(&args).output() {
                if !out.status.success() {
                    return false;
                }
            } else {
                return false;
            }
        }

        if switch {
            let target = format!("{}:{}", session_name, window_name);
            if std::env::var("TMUX").is_ok() {
                let _ = Command::new("tmux").args(["switch-client", "-t", &target]).status();
            } else {
                let _ = Command::new("tmux").args(["select-window", "-t", &target]).status();
            }
        }

        true
    }

    pub fn focus_workspace_window(session_name: &str, window_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        let target = format!("{}:{}", session_name, window_name);
        if std::env::var("TMUX").is_ok() {
            let _ = Command::new("tmux").args(["select-window", "-t", &target]).status();
            let _ = Command::new("tmux").args(["switch-client", "-t", &target]).status();
            true
        } else {
            Command::new("tmux")
                .args(["attach-session", "-t", &target])
                .status()
                .map_or(false, |s| s.success())
        }
    }

    pub fn kill_workspace_window(session_name: &str, window_name: &str) -> bool {
        if !Self::is_session_active(session_name) {
            return false;
        }
        let target = format!("{}:{}", session_name, window_name);
        Command::new("tmux")
            .args(["kill-window", "-t", &target])
            .output()
            .map_or(false, |o| o.status.success())
    }

    pub fn kill_workspace(window_name: &str, project_name: Option<&str>) -> bool {
        if let Some(p) = project_name {
            let sess = Self::session_name(p);
            Self::kill_workspace_window(&sess, window_name)
        } else {
            false
        }
    }

    pub fn launch(
        workspace_name: &str,
        launch_entries: &[(String, String, String)], // (name, cwd, cmd)
        session_name: &str,
    ) -> bool {
        if !Self::is_available() || launch_entries.is_empty() {
            return false;
        }

        let _ = Self::create_workspace_window(
            session_name,
            workspace_name,
            Path::new(&launch_entries[0].1),
            Some(&launch_entries[0].2),
            true,
        );

        let target_window = format!("{}:{}", session_name, workspace_name);
        for (_, cwd, cmd) in &launch_entries[1..] {
            let _ = Command::new("tmux")
                .args(["split-window", "-h", "-t", &target_window, "-c", cwd, cmd])
                .status();
        }

        let _ = Command::new("tmux")
            .args(["select-layout", "-t", &target_window, "even-horizontal"])
            .status();

        true
    }

    pub fn launch_services_horizontal_panes(
        session_name: &str,
        window_name: &str,
        services: &[(String, String, PathBuf, HashMap<String, String>)],
    ) -> bool {
        if !Self::is_available() || services.is_empty() {
            return false;
        }

        let _ = Command::new("tmux").args(["kill-session", "-t", session_name]).output();

        let (_first_name, first_cmd, first_cwd, first_env) = &services[0];
        let cwd_str = first_cwd.to_string_lossy();

        let build_cmd = |cmd: &str, env: &HashMap<String, String>| -> String {
            let mut exports = Vec::new();
            for (k, v) in env {
                exports.push(format!("export {}={:?}", k, v));
            }
            if exports.is_empty() {
                cmd.to_string()
            } else {
                format!("{}; {}", exports.join("; "), cmd)
            }
        };

        let initial_shell = build_cmd(first_cmd, first_env);
        let status = Command::new("tmux")
            .args([
                "new-session", "-d",
                "-s", session_name,
                "-n", window_name,
                "-c", &cwd_str,
                &initial_shell,
            ])
            .status();

        if !status.map_or(false, |s| s.success()) {
            return false;
        }

        let target_window = format!("{}:{}", session_name, window_name);
        for (_, cmd, cwd, env) in &services[1..] {
            let pane_cwd = cwd.to_string_lossy();
            let pane_shell = build_cmd(cmd, env);
            let _ = Command::new("tmux")
                .args([
                    "split-window", "-h",
                    "-t", &target_window,
                    "-c", &pane_cwd,
                    &pane_shell,
                ])
                .status();
        }

        let _ = Command::new("tmux")
            .args(["select-layout", "-t", &target_window, "even-horizontal"])
            .status();

        true
    }

    pub fn list_panes(session_name: &str, window_name: &str) -> Vec<String> {
        let target = format!("{}:{}", session_name, window_name);
        if let Ok(out) = Command::new("tmux")
            .args(["list-panes", "-t", &target, "-F", "#{pane_title}"])
            .output()
        {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
            }
        }
        Vec::new()
    }

    pub fn attach(
        workspace_name: &str,
        session_name: &str,
        project_name: &str,
        _repo_name: Option<&str>,
        _all_panes: bool,
    ) -> std::io::Result<std::process::ExitStatus> {
        let target = if Self::is_window_active(session_name, workspace_name) {
            format!("{}:{}", session_name, workspace_name)
        } else {
            format!("{}:{}", project_name, workspace_name)
        };
        if std::env::var("TMUX").is_ok() {
            let _ = Command::new("tmux").args(["select-window", "-t", &target]).status();
            Command::new("tmux").args(["switch-client", "-t", &target]).status()
        } else {
            Command::new("tmux").args(["attach-session", "-t", &target]).status()
        }
    }

    pub fn attach_session(session_name: &str) -> std::io::Result<std::process::ExitStatus> {
        if std::env::var("TMUX").is_ok() {
            Command::new("tmux").args(["switch-client", "-t", session_name]).status()
        } else {
            Command::new("tmux").args(["attach-session", "-t", session_name]).status()
        }
    }
}

pub struct ZellijLauncher;

impl ZellijLauncher {
    pub fn attach(
        _workspace_name: &str,
        project_name: &str,
        _repo_name: Option<&str>,
        _all_panes: bool,
        _ws_dir: &Path,
    ) -> std::io::Result<std::process::ExitStatus> {
        Command::new("zellij").args(["attach", project_name]).status()
    }

    pub fn is_available() -> bool {
        Command::new("zellij").arg("--version").output().map_or(false, |o| o.status.success())
    }

    pub fn is_session_running(session_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        if let Ok(out) = Command::new("zellij").args(["list-sessions"]).output() {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                return stdout.lines().any(|l| l.contains(session_name));
            }
        }
        false
    }

    pub fn is_tab_running(session_name: &str, _tab_name: &str) -> bool {
        Self::is_session_running(session_name)
    }

    pub fn kill_workspace(_workspace_name: &str, project_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        Command::new("zellij")
            .args(["kill-session", project_name])
            .output()
            .map_or(false, |o| o.status.success())
    }

    pub fn generate_kdl_layout(
        services: &[(String, String, PathBuf)], // (name, command, cwd)
    ) -> String {
        let mut kdl = String::from("layout {\n    pane split_direction=\"vertical\" {\n");
        for (name, cmd, cwd) in services {
            kdl.push_str(&format!(
                "        pane name=\"{}\" cwd=\"{}\" command=\"bash\" {{\n            args \"-c\" \"{}\"\n        }}\n",
                name, cwd.display(), cmd.replace('"', "\\\"")
            ));
        }
        kdl.push_str("    }\n}\n");
        kdl
    }

    pub fn launch(
        workspace_name: &str,
        launch_entries: &[(String, String, String)],
        project_name: &str,
        ws_dir: &Path,
    ) -> bool {
        if !Self::is_available() {
            return false;
        }
        let layout_services: Vec<(String, String, PathBuf)> = launch_entries
            .iter()
            .map(|(name, cwd, cmd)| (name.clone(), cmd.clone(), PathBuf::from(cwd)))
            .collect();
        let kdl = Self::generate_kdl_layout(&layout_services);
        let layout_file = ws_dir.join(".ws").join(format!("{}.kdl", workspace_name));
        if let Some(p) = layout_file.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::write(&layout_file, kdl);

        Command::new("zellij")
            .args(["--session", project_name, "--layout", &layout_file.to_string_lossy()])
            .status()
            .map_or(false, |s| s.success())
    }
}
