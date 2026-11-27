use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn resolve_binary(name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn command_for(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    // Avoid re-inheriting a huge PATH for trivial probes; keep PATH minimal.
    cmd.env("PATH", "/usr/bin:/bin");
    cmd
}

pub struct TmuxSnapshot {
    windows_by_session: HashMap<String, Vec<String>>,
}

impl TmuxSnapshot {
    pub fn capture() -> Self {
        let mut windows_by_session: HashMap<String, Vec<String>> = HashMap::new();
        let Some(bin) = TmuxLauncher::binary() else { return Self { windows_by_session }; };
        let Ok(out) = command_for(bin).args(["list-windows", "-a", "-F", "#{session_name}\t#{window_name}"]).output() else {
            return Self { windows_by_session };
        };
        if !out.status.success() { return Self { windows_by_session }; }
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut parts = line.splitn(2, '\t');
            let Some(session) = parts.next() else { continue; };
            let Some(window) = parts.next() else { continue; };
            windows_by_session.entry(session.to_string()).or_default().push(window.to_string());
        }
        Self { windows_by_session }
    }

    pub fn session_active(&self, session_name: &str) -> bool {
        self.windows_by_session.contains_key(session_name)
    }

    pub fn window_active(&self, session_name: &str, window_name: &str) -> bool {
        let Some(windows) = self.windows_by_session.get(session_name) else { return false; };
        let clean = window_name.trim_start_matches('@');
        let with_at = format!("@{clean}");
        windows.iter().any(|t| t == window_name || t == clean || t == &with_at)
    }
}

pub struct ZellijSnapshot {
    sessions: Vec<String>,
}

impl ZellijSnapshot {
    pub fn capture() -> Self {
        let Some(bin) = ZellijLauncher::binary() else { return Self { sessions: Vec::new() }; };
        let Ok(out) = command_for(bin).args(["list-sessions"]).output() else {
            return Self { sessions: Vec::new() };
        };
        if !out.status.success() { return Self { sessions: Vec::new() }; }
        let sessions = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                if t.is_empty() { return None; }
                // zellij list-sessions includes formatting/exit status; first token is session name
                let name = t.split_whitespace().next().unwrap_or(t);
                let name = name.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '@' && c != '.');
                if name.is_empty() { None } else { Some(name.to_string()) }
            })
            .collect();
        Self { sessions }
    }

    pub fn session_running(&self, session_name: &str) -> bool {
        self.sessions.iter().any(|s| s == session_name || s.contains(session_name))
    }
}

pub struct TmuxLauncher;

impl TmuxLauncher {
    fn binary() -> Option<&'static PathBuf> {
        static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
        BIN.get_or_init(|| resolve_binary("tmux")).as_ref()
    }

    fn raw_cmd() -> Command {
        match Self::binary() {
            Some(b) => command_for(b),
            None => {
                let mut c = Self::raw_cmd();
                c.env("PATH", "/usr/bin:/bin");
                c
            }
        }
    }

    pub fn is_available() -> bool {
        Self::binary().is_some()
    }

    pub fn session_name(project_name: &str) -> String {
        format!("ws-{}", project_name)
    }

    pub fn is_session_active(session_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        Self::raw_cmd()
            .args(["has-session", "-t", session_name])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    pub fn find_window_name(session_name: &str, window_name: &str) -> Option<String> {
        if !Self::is_session_active(session_name) {
            return None;
        }
        if let Ok(out) = Self::raw_cmd()
            .args(["list-windows", "-t", session_name, "-F", "#{window_name}"])
            .output()
        {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let clean = window_name.trim_start_matches('@');
                let with_at = format!("@{}", clean);
                for line in stdout.lines() {
                    let t = line.trim();
                    if t == window_name || t == clean || t == with_at {
                        return Some(t.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn is_window_active(session_name: &str, window_name: &str) -> bool {
        Self::find_window_name(session_name, window_name).is_some()
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
            let mut args = vec![
                "new-session",
                "-d",
                "-s",
                session_name,
                "-n",
                window_name,
                "-c",
                &cwd_str,
            ];
            if let Some(cmd) = command {
                args.push(cmd);
            }
            if let Ok(out) = Self::raw_cmd().args(&args).output() {
                if !out.status.success() {
                    return false;
                }
            } else {
                return false;
            }
        } else if !Self::is_window_active(session_name, window_name) {
            let target = format!("{}:", session_name);
            let mut args = vec![
                "new-window",
                "-d",
                "-t",
                &target,
                "-n",
                window_name,
                "-c",
                &cwd_str,
            ];
            if let Some(cmd) = command {
                args.push(cmd);
            }
            if let Ok(out) = Self::raw_cmd().args(&args).output() {
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
                let _ = Self::raw_cmd()
                    .args(["switch-client", "-t", &target])
                    .status();
            } else {
                let _ = Self::raw_cmd()
                    .args(["select-window", "-t", &target])
                    .status();
            }
        }

        true
    }

    pub fn focus_workspace_window(session_name: &str, window_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        let target_win = match Self::find_window_name(session_name, window_name) {
            Some(w) => w,
            None => window_name.to_string(),
        };
        let target = format!("{}:{}", session_name, target_win);
        let _ = Self::raw_cmd()
            .args(["select-window", "-t", &target])
            .status();
        if std::env::var("TMUX").is_ok() {
            let _ = Self::raw_cmd()
                .args(["switch-client", "-t", &target])
                .status();
            true
        } else {
            Self::raw_cmd()
                .args(["attach-session", "-t", &target])
                .status()
                .is_ok_and(|s| s.success())
        }
    }

    pub fn is_current_window(session_name: &str, window_name: &str) -> bool {
        if std::env::var_os("TMUX").is_none() || !Self::is_available() {
            return false;
        }
        let target_win = match Self::find_window_name(session_name, window_name) {
            Some(w) => w,
            None => window_name.to_string(),
        };
        let output = match Self::raw_cmd()
            .args(["display-message", "-p", "#S\t#{window_name}"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return false,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut parts = stdout.trim_end().splitn(2, '\t');
        let cur_session = parts.next().unwrap_or_default().trim();
        let cur_window = parts.next().unwrap_or_default().trim();
        if cur_session != session_name {
            return false;
        }
        let clean = window_name.trim_start_matches('@');
        let with_at = format!("@{}", clean);
        cur_window == target_win
            || cur_window == window_name
            || cur_window == clean
            || cur_window == with_at
    }

    pub fn kill_workspace_window(session_name: &str, window_name: &str) -> bool {
        if !Self::is_session_active(session_name) {
            return false;
        }
        let target_win = match Self::find_window_name(session_name, window_name) {
            Some(w) => w,
            None => window_name.to_string(),
        };
        let target = format!("{}:{}", session_name, target_win);
        Self::raw_cmd()
            .args(["kill-window", "-t", &target])
            .output()
            .is_ok_and(|o| o.status.success())
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
            let _ = Self::raw_cmd()
                .args(["split-window", "-h", "-t", &target_window, "-c", cwd, cmd])
                .status();
        }

        let _ = Self::raw_cmd()
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

        let _ = Self::raw_cmd()
            .args(["kill-session", "-t", session_name])
            .output();

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
        let status = Self::raw_cmd()
            .args([
                "new-session",
                "-d",
                "-s",
                session_name,
                "-n",
                window_name,
                "-c",
                &cwd_str,
                &initial_shell,
            ])
            .status();

        if !status.is_ok_and(|s| s.success()) {
            return false;
        }

        let target_window = format!("{}:{}", session_name, window_name);
        for (_, cmd, cwd, env) in &services[1..] {
            let pane_cwd = cwd.to_string_lossy();
            let pane_shell = build_cmd(cmd, env);
            let _ = Self::raw_cmd()
                .args([
                    "split-window",
                    "-h",
                    "-t",
                    &target_window,
                    "-c",
                    &pane_cwd,
                    &pane_shell,
                ])
                .status();
        }

        let _ = Self::raw_cmd()
            .args(["select-layout", "-t", &target_window, "even-horizontal"])
            .status();

        true
    }

    pub fn list_panes(session_name: &str, window_name: &str) -> Vec<String> {
        let target = format!("{}:{}", session_name, window_name);
        if let Ok(out) = Self::raw_cmd()
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
            let _ = Self::raw_cmd()
                .args(["select-window", "-t", &target])
                .status();
            Self::raw_cmd()
                .args(["switch-client", "-t", &target])
                .status()
        } else {
            Self::raw_cmd()
                .args(["attach-session", "-t", &target])
                .status()
        }
    }

    pub fn attach_session(session_name: &str) -> std::io::Result<std::process::ExitStatus> {
        if std::env::var("TMUX").is_ok() {
            Self::raw_cmd()
                .args(["switch-client", "-t", session_name])
                .status()
        } else {
            Self::raw_cmd()
                .args(["attach-session", "-t", session_name])
                .status()
        }
    }
}

pub struct ZellijLauncher;

impl ZellijLauncher {
    fn binary() -> Option<&'static PathBuf> {
        static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
        BIN.get_or_init(|| resolve_binary("zellij")).as_ref()
    }

    fn raw_cmd() -> Command {
        match Self::binary() {
            Some(b) => command_for(b),
            None => {
                let mut c = Self::raw_cmd();
                c.env("PATH", "/usr/bin:/bin");
                c
            }
        }
    }

    pub fn attach(
        _workspace_name: &str,
        project_name: &str,
        _repo_name: Option<&str>,
        _all_panes: bool,
        _ws_dir: &Path,
    ) -> std::io::Result<std::process::ExitStatus> {
        Self::raw_cmd()
            .args(["attach", project_name])
            .status()
    }

    pub fn is_available() -> bool {
        Self::binary().is_some()
    }

    pub fn is_session_running(session_name: &str) -> bool {
        if !Self::is_available() {
            return false;
        }
        if let Ok(out) = Self::raw_cmd().args(["list-sessions"]).output() {
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
        Self::raw_cmd()
            .args(["kill-session", project_name])
            .output()
            .is_ok_and(|o| o.status.success())
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

        Self::raw_cmd()
            .args([
                "--session",
                project_name,
                "--layout",
                &layout_file.to_string_lossy(),
            ])
            .status()
            .is_ok_and(|s| s.success())
    }
}
