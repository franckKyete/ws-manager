use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use regex::Regex;

use crate::models::AppConfig;
use crate::network::{allocate_workspace_ports, compute_preferred_service_port, get_lan_ip};

lazy_static::lazy_static! {
    static ref PORT_EXPR_REGEX: Regex = Regex::new(r"\$\{PORT:(\d+)\}").unwrap();
    static ref PORT_OFFSET_EXPR_REGEX: Regex = Regex::new(r"\$\{PORT_OFFSET:(\d+):(\d+)\}").unwrap();
    static ref SVC_PORTS_REGEX: Regex = Regex::new(r"\$\{SERVICE_PORTS:([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_PORT_SUB_REGEX: Regex = Regex::new(r"\$\{SERVICE_PORT:([a-zA-Z0-9_-]+):([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_PORT_REGEX: Regex = Regex::new(r"\$\{SERVICE_PORT:([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_URL_SUB_REGEX: Regex = Regex::new(r"\$\{SERVICE_URL:([a-zA-Z0-9_-]+):([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_URL_REGEX: Regex = Regex::new(r"\$\{SERVICE_URL:([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_URL_LAN_SUB_REGEX: Regex = Regex::new(r"\$\{SERVICE_URL_LAN:([a-zA-Z0-9_-]+):([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_URL_LAN_REGEX: Regex = Regex::new(r"\$\{SERVICE_URL_LAN:([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_URL_PUB_SUB_REGEX: Regex = Regex::new(r"\$\{SERVICE_URL_PUBLIC:([a-zA-Z0-9_-]+):([a-zA-Z0-9_-]+)\}").unwrap();
    static ref SVC_URL_PUB_REGEX: Regex = Regex::new(r"\$\{SERVICE_URL_PUBLIC:([a-zA-Z0-9_-]+)\}").unwrap();
    static ref ENV_FALLBACK_REGEX: Regex = Regex::new(r"\$\{ENV:([a-zA-Z0-9_]+)(?::-([^}]*))?\}").unwrap();
}

pub struct EnvEngine;

impl EnvEngine {
    pub fn get_workspace_slot(workspaces_dir: &Path, workspace_name: &str) -> u16 {
        let ws_dir = workspaces_dir.join(workspace_name);
        let s_path = ws_dir.join(".ws").join("services.json");
        if s_path.is_file() {
            if let Ok(content) = fs::read_to_string(&s_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(slot) = val.get("slot").and_then(|v| v.as_u64()) {
                        return slot as u16;
                    }
                }
            }
        }

        let mut occupied = std::collections::HashSet::new();
        if let Ok(entries) = fs::read_dir(workspaces_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
                    let sp = p.join(".ws").join("services.json");
                    if sp.is_file() {
                        if let Ok(content) = fs::read_to_string(&sp) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(slot) = val.get("slot").and_then(|v| v.as_u64()) {
                                    occupied.insert(slot as u16);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut candidate = 1u16;
        while occupied.contains(&candidate) {
            candidate += 1;
        }
        candidate
    }

    pub fn resolve_template_string(
        template: &str,
        workspace_name: &str,
        repo_name: &str,
        slot: u16,
        project_root: Option<&Path>,
        workspaces_dir: Option<&Path>,
        service_ports: Option<&HashMap<String, u16>>,
        all_allocated_ports: Option<&HashMap<String, HashMap<String, u16>>>,
        lan_ip: Option<&str>,
        public_host: Option<&str>,
        interface: Option<&str>,
    ) -> String {
        let empty_ports = HashMap::new();
        let ports = service_ports.unwrap_or(&empty_ports);

        let empty_all_ports = HashMap::new();
        let all_ports = all_allocated_ports.unwrap_or(&empty_all_ports);

        let mut result = template.to_string();

        // 1. Workspace variables
        result = result.replace("${WORKSPACE_NAME}", workspace_name);
        result = result.replace("${REPO_NAME}", repo_name);
        result = result.replace("${WORKSPACE_SLOT}", &slot.to_string());

        // 2. Path variables
        if let Some(pr) = project_root {
            result = result.replace("${PROJECT_DIR}", &pr.display().to_string());
            result = result.replace("${PROJECT_ROOT}", &pr.display().to_string());
        }
        if let Some(wd) = workspaces_dir {
            result = result.replace("${WORKSPACES_DIR}", &wd.display().to_string());
            result = result.replace("${WORKSPACE_DIR}", &wd.join(workspace_name).display().to_string());
        }

        // 3. Port expressions ${PORT:3000} -> base + slot * 10
        result = PORT_EXPR_REGEX.replace_all(&result, |caps: &regex::Captures| {
            if let Ok(base) = caps[1].parse::<u16>() {
                compute_preferred_service_port(base, slot, 10).to_string()
            } else {
                caps[0].to_string()
            }
        }).to_string();

        // ${PORT_OFFSET:8000:5} -> base + slot * 5
        result = PORT_OFFSET_EXPR_REGEX.replace_all(&result, |caps: &regex::Captures| {
            if let (Ok(base), Ok(mult)) = (caps[1].parse::<u16>(), caps[2].parse::<u16>()) {
                compute_preferred_service_port(base, slot, mult).to_string()
            } else {
                caps[0].to_string()
            }
        }).to_string();

        // 4. Sibling service discovery
        result = SVC_PORTS_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            if let Some(sub) = all_ports.get(target) {
                let mut p_list: Vec<_> = sub.values().map(|p| p.to_string()).collect();
                p_list.sort();
                p_list.join(",")
            } else if let Some(p) = ports.get(target) {
                p.to_string()
            } else {
                "".to_string()
            }
        }).to_string();

        // ${SERVICE_PORT:<name>:<subport>}
        result = SVC_PORT_SUB_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            let subport = &caps[2];
            let combined = format!("{}:{}", target, subport);
            if let Some(p) = ports.get(&combined) {
                return p.to_string();
            }
            if let Some(sub) = all_ports.get(target) {
                if let Some(p) = sub.get(subport) {
                    return p.to_string();
                }
            }
            "".to_string()
        }).to_string();

        // ${SERVICE_PORT:<name>}
        result = SVC_PORT_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            if let Some(p) = ports.get(target) {
                p.to_string()
            } else {
                "".to_string()
            }
        }).to_string();

        // ${SERVICE_URL:<name>:<subport>}
        result = SVC_URL_SUB_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            let subport = &caps[2];
            let combined = format!("{}:{}", target, subport);
            if let Some(p) = ports.get(&combined) {
                return format!("http://127.0.0.1:{}", p);
            }
            if let Some(sub) = all_ports.get(target) {
                if let Some(p) = sub.get(subport) {
                    return format!("http://127.0.0.1:{}", p);
                }
            }
            "".to_string()
        }).to_string();

        // ${SERVICE_URL:<name>}
        result = SVC_URL_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            if let Some(p) = ports.get(target) {
                format!("http://127.0.0.1:{}", p)
            } else {
                "".to_string()
            }
        }).to_string();

        let resolved_lan_ip = match lan_ip {
            Some(ip) => ip.to_string(),
            None => get_lan_ip(interface),
        };

        let resolved_public_host = match public_host {
            Some(h) => h.to_string(),
            None => std::env::var("WS_PUBLIC_HOST")
                .or_else(|_| std::env::var("PUBLIC_HOST"))
                .unwrap_or_else(|_| resolved_lan_ip.clone()),
        };

        // ${SERVICE_URL_LAN:<name>:<subport>}
        result = SVC_URL_LAN_SUB_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            let subport = &caps[2];
            let combined = format!("{}:{}", target, subport);
            if let Some(p) = ports.get(&combined) {
                return format!("http://{}:{}", resolved_lan_ip, p);
            }
            if let Some(sub) = all_ports.get(target) {
                if let Some(p) = sub.get(subport) {
                    return format!("http://{}:{}", resolved_lan_ip, p);
                }
            }
            "".to_string()
        }).to_string();

        // ${SERVICE_URL_LAN:<name>}
        result = SVC_URL_LAN_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            if let Some(p) = ports.get(target) {
                format!("http://{}:{}", resolved_lan_ip, p)
            } else {
                "".to_string()
            }
        }).to_string();

        // ${SERVICE_URL_PUBLIC:<name>:<subport>}
        result = SVC_URL_PUB_SUB_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            let subport = &caps[2];
            let combined = format!("{}:{}", target, subport);
            if let Some(p) = ports.get(&combined) {
                return format!("https://{}:{}", resolved_public_host, p);
            }
            if let Some(sub) = all_ports.get(target) {
                if let Some(p) = sub.get(subport) {
                    return format!("https://{}:{}", resolved_public_host, p);
                }
            }
            "".to_string()
        }).to_string();

        // ${SERVICE_URL_PUBLIC:<name>}
        result = SVC_URL_PUB_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let target = &caps[1];
            if let Some(p) = ports.get(target) {
                format!("https://{}:{}", resolved_public_host, p)
            } else {
                "".to_string()
            }
        }).to_string();

        // 5. Host env fallbacks ${ENV:KEY:-default} or ${ENV:KEY}
        result = ENV_FALLBACK_REGEX.replace_all(&result, |caps: &regex::Captures| {
            let key = &caps[1];
            let fallback = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            std::env::var(key).unwrap_or_else(|_| fallback.to_string())
        }).to_string();

        result
    }

    pub fn expand_command(
        command: &str,
        env_vars: &HashMap<String, String>,
        workspace_name: &str,
        repo_name: &str,
        slot: u16,
        project_root: Option<&Path>,
        workspaces_dir: Option<&Path>,
        service_ports: Option<&HashMap<String, u16>>,
        lan_ip: Option<&str>,
        public_host: Option<&str>,
        interface: Option<&str>,
    ) -> String {
        let mut cmd = Self::resolve_template_string(
            command,
            workspace_name,
            repo_name,
            slot,
            project_root,
            workspaces_dir,
            service_ports,
            None,
            lan_ip,
            public_host,
            interface,
        );

        for (k, v) in env_vars {
            let placeholder = format!("${{{}}}", k);
            cmd = cmd.replace(&placeholder, v);
        }

        if let Some(pr) = project_root {
            let scripts_dir = pr.join("scripts");
            if scripts_dir.exists() && (cmd.starts_with("scripts/") || cmd.starts_with("./scripts/")) {
                let first_word = cmd.split_whitespace().next().unwrap_or("");
                let script_rel = first_word.trim_start_matches("./");
                let candidate = pr.join(script_rel);
                if candidate.exists() {
                    let rest = &cmd[first_word.len()..];
                    cmd = format!("{}{}", candidate.display(), rest);
                }
            }
        }

        cmd
    }

    pub fn resolve_repo_env(
        app_config: &AppConfig,
        workspace_name: &str,
        repo_name: &str,
        slot: u16,
        service_ports: Option<&HashMap<String, u16>>,
        lan_ip: Option<&str>,
        public_host: Option<&str>,
        interface: Option<&str>,
    ) -> HashMap<String, String> {
        let (auto_ports, _) = if service_ports.is_none() {
            let (p, _) = allocate_workspace_ports(&app_config.repositories, slot, None);
            (Some(p), false)
        } else {
            (None, false)
        };

        let ports_ref = service_ports.or(auto_ports.as_ref()).unwrap();
        let resolved_lan_ip = match lan_ip {
            Some(ip) => ip.to_string(),
            None => get_lan_ip(interface),
        };
        let resolved_public_host = public_host
            .map(|s| s.to_string())
            .or_else(|| app_config.global_env.get("PUBLIC_HOST").cloned())
            .or_else(|| std::env::var("WS_PUBLIC_HOST").ok())
            .or_else(|| std::env::var("PUBLIC_HOST").ok())
            .unwrap_or_else(|| resolved_lan_ip.clone());

        let mut merged = HashMap::new();

        // 1. Global env tiers
        for (k, v) in &app_config.global_env {
            merged.insert(k.clone(), v.clone());
        }
        for (k, v) in &app_config.secret_env {
            merged.insert(k.clone(), v.clone());
        }
        for (k, v) in &app_config.private_env {
            merged.insert(k.clone(), v.clone());
        }
        for (k, v) in &app_config.dynamic_env {
            merged.insert(k.clone(), v.clone());
        }

        // 2. Repo-level tiers
        if let Some(repo_cfg) = app_config.repositories.get(repo_name) {
            for (k, v) in &repo_cfg.env {
                merged.insert(k.clone(), v.clone());
            }
            for (k, v) in &repo_cfg.secret_env {
                merged.insert(k.clone(), v.clone());
            }
            for (k, v) in &repo_cfg.private_env {
                merged.insert(k.clone(), v.clone());
            }
        }

        // 3. Resolve template strings
        let mut resolved = HashMap::new();
        let pr = app_config.project_root();
        for (k, v) in merged {
            let res_v = Self::resolve_template_string(
                &v,
                workspace_name,
                repo_name,
                slot,
                Some(&pr),
                Some(&app_config.workspaces_dir),
                Some(ports_ref),
                None,
                Some(&resolved_lan_ip),
                Some(&resolved_public_host),
                interface,
            );
            resolved.insert(k, res_v);
        }

        // 4. Inject runtime discovery variables
        resolved.insert("WS_WORKSPACE".to_string(), workspace_name.to_string());
        resolved.insert("WS_SLOT".to_string(), slot.to_string());
        resolved.insert("WS_LAN_IP".to_string(), resolved_lan_ip.clone());
        resolved.insert("WS_PUBLIC_HOST".to_string(), resolved_public_host);

        for (svc_name, p) in ports_ref {
            if !svc_name.contains(':') {
                let upper = svc_name.to_uppercase().replace('-', "_");
                resolved.insert(format!("WS_SERVICE_{}_PORT", upper), p.to_string());
                resolved.insert(format!("WS_SERVICE_{}_URL", upper), format!("http://127.0.0.1:{}", p));
                resolved.insert(format!("WS_SERVICE_{}_URL_LAN", upper), format!("http://{}:{}", resolved_lan_ip, p));
            } else {
                let parts: Vec<&str> = svc_name.splitn(2, ':').collect();
                let upper = parts[0].to_uppercase().replace('-', "_");
                let sub_upper = parts[1].to_uppercase().replace('-', "_");
                resolved.insert(format!("WS_SERVICE_{}_PORT_{}", upper, sub_upper), p.to_string());
                resolved.insert(format!("WS_SERVICE_{}_URL_{}", upper, sub_upper), format!("http://127.0.0.1:{}", p));
                resolved.insert(format!("WS_SERVICE_{}_URL_LAN_{}", upper, sub_upper), format!("http://{}:{}", resolved_lan_ip, p));
            }
        }

        resolved
    }

    pub fn mask_secret_value(key: &str, val: &str) -> String {
        let upper = key.to_uppercase();
        if upper.contains("SECRET")
            || upper.contains("PASSWORD")
            || upper.contains("KEY")
            || upper.contains("TOKEN")
            || upper.contains("CREDENTIAL")
            || upper.contains("PRIVATE")
            || upper.contains("AUTH")
        {
            "*".repeat(val.len().max(8))
        } else {
            val.to_string()
        }
    }

    pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if ty.is_dir() {
                Self::copy_dir_recursive(&from, &to)?;
            } else {
                fs::copy(&from, &to)?;
            }
        }
        Ok(())
    }

    pub fn sync_copied_files(
        project_root: &Path,
        worktree_path: &Path,
        copy_files: &[serde_json::Value],
    ) -> (bool, String) {
        if copy_files.is_empty() {
            return (true, String::new());
        }

        let mut copied_count = 0;
        let mut errors = Vec::new();
        let files_dir = project_root.join("files");

        for item in copy_files {
            let (src_rel, dst_rel) = if let Some(s) = item.as_str() {
                let d = if s.starts_with("files/") { &s[6..] } else { s };
                (s.to_string(), d.to_string())
            } else if let Some(obj) = item.as_object() {
                let s = obj.get("source").or_else(|| obj.get("src")).and_then(|v| v.as_str()).unwrap_or("");
                let d = obj.get("dest").or_else(|| obj.get("dst")).and_then(|v| v.as_str()).unwrap_or(s);
                (s.to_string(), d.to_string())
            } else {
                continue;
            };

            if src_rel.is_empty() {
                continue;
            }

            let mut src_path = None;
            let c1 = files_dir.join(&src_rel);
            if files_dir.exists() && c1.exists() {
                src_path = Some(c1);
            } else {
                let c2 = project_root.join(&src_rel);
                if c2.exists() {
                    src_path = Some(c2);
                } else if src_rel.starts_with("files/") {
                    let c3 = project_root.join(&src_rel);
                    if c3.exists() {
                        src_path = Some(c3);
                    }
                }
            }

            let src_path = match src_path {
                Some(p) => p,
                None => {
                    errors.push(format!("source file not found: '{}'", src_rel));
                    continue;
                }
            };

            let dst_path = worktree_path.join(&dst_rel);
            if let Some(parent) = dst_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            if src_path.is_dir() {
                let _ = Self::copy_dir_recursive(&src_path, &dst_path);
                copied_count += 1;
            } else if fs::copy(&src_path, &dst_path).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = dst_path.metadata() {
                        let mut perms = meta.permissions();
                        let mode = perms.mode() | 0o600;
                        perms.set_mode(mode);
                        let _ = fs::set_permissions(&dst_path, perms);
                    }
                }
                copied_count += 1;
            } else {
                errors.push(format!("failed copying {} -> {}", src_rel, dst_rel));
            }
        }

        if errors.is_empty() {
            (true, format!("Copied {} files", copied_count))
        } else {
            (false, errors.join(", "))
        }
    }

    pub fn prepare_and_sync_env_file(
        worktree_path: &Path,
        env_vars: &HashMap<String, String>,
        env_file_name: &str,
        env_example_name: &str,
    ) -> std::io::Result<(bool, String)> {
        let target_env = worktree_path.join(env_file_name);

        // Step 1: Copy example if missing
        if !target_env.exists() {
            let example_path = worktree_path.join(env_example_name);
            if example_path.is_file() {
                let _ = fs::copy(&example_path, &target_env);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = target_env.metadata() {
                        let mut perms = meta.permissions();
                        let mode = perms.mode() | 0o600;
                        perms.set_mode(mode);
                        let _ = fs::set_permissions(&target_env, perms);
                    }
                }
            }
        }

        // Step 2: Read existing lines
        let mut existing_lines = Vec::new();
        let mut existing_keys = HashMap::new();

        if target_env.is_file() {
            let content = fs::read_to_string(&target_env)?;
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if !trimmed.starts_with('#') && trimmed.contains('=') {
                    let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                    let k = parts[0].trim().to_string();
                    existing_keys.insert(k, idx);
                }
                existing_lines.push(line.to_string());
            }
        }

        // Step 3: Update existing keys or append
        let mut sorted_vars: Vec<_> = env_vars.iter().collect();
        sorted_vars.sort_by_key(|(k, _)| *k);

        for (k, v) in sorted_vars {
            let quote_escaped = if v.contains(' ') || v.contains('"') || v.contains('#') {
                format!("\"{}\"", v.replace('"', "\\\""))
            } else {
                v.clone()
            };
            let formatted_line = format!("{}={}", k, quote_escaped);

            if let Some(&line_idx) = existing_keys.get(k) {
                existing_lines[line_idx] = formatted_line;
            } else {
                existing_lines.push(formatted_line);
            }
        }

        let new_content = existing_lines.join("\n") + "\n";
        fs::write(&target_env, new_content)?;

        Ok((true, format!("Synchronized {}", target_env.display())))
    }

    pub fn write_service_discovery_files(
        workspace_dir: &Path,
        workspace_name: &str,
        slot: u16,
        service_ports: &HashMap<String, u16>,
        public_host: Option<&str>,
        lan_ip: Option<&str>,
        interface: Option<&str>,
    ) -> std::io::Result<PathBuf> {
        let ws_meta_dir = workspace_dir.join(".ws");
        fs::create_dir_all(&ws_meta_dir)?;

        let resolved_lan_ip = match lan_ip {
            Some(ip) => ip.to_string(),
            None => get_lan_ip(interface),
        };
        let pub_host = public_host
            .map(|s| s.to_string())
            .or_else(|| std::env::var("WS_PUBLIC_HOST").ok())
            .or_else(|| std::env::var("PUBLIC_HOST").ok())
            .unwrap_or_else(|| resolved_lan_ip.clone());

        let mut services_data: HashMap<String, serde_json::Value> = HashMap::new();
        let mut env_lines = vec![
            format!("# Auto-generated service discovery for workspace @{}\n", workspace_name),
            format!("WS_WORKSPACE={}\n", workspace_name),
            format!("WS_SLOT={}\n", slot),
            format!("WS_LAN_IP={}\n", resolved_lan_ip),
            format!("WS_PUBLIC_HOST={}\n\n", pub_host),
        ];

        let mut base_services: Vec<String> = service_ports.keys().filter(|k| !k.contains(':')).cloned().collect();
        base_services.sort();
        if base_services.is_empty() && !service_ports.is_empty() {
            let mut unique_bases = std::collections::HashSet::new();
            for k in service_ports.keys() {
                unique_bases.insert(k.split(':').next().unwrap().to_string());
            }
            base_services = unique_bases.into_iter().collect();
            base_services.sort();
        }

        for s_name in base_services {
            let primary_port = service_ports.get(&s_name).copied().unwrap_or(0);
            let mut sub_ports: HashMap<String, u16> = HashMap::new();

            for (k, &v) in service_ports {
                if let Some(stripped) = k.strip_prefix(&format!("{}:", s_name)) {
                    sub_ports.insert(stripped.to_string(), v);
                }
            }
            if sub_ports.is_empty() && primary_port > 0 {
                sub_ports.insert("default".to_string(), primary_port);
            }

            let mut urls_data = serde_json::Map::new();
            let mut all_ports_list = Vec::new();
            for (p_label, &p_val) in &sub_ports {
                let p_str = p_val.to_string();
                if !all_ports_list.contains(&p_str) {
                    all_ports_list.push(p_str);
                }
                urls_data.insert(p_label.clone(), serde_json::json!({
                    "port": p_val,
                    "url_local": format!("http://127.0.0.1:{}", p_val),
                    "url_lan": format!("http://{}:{}", resolved_lan_ip, p_val),
                    "url_public": format!("http://{}:{}", pub_host, p_val),
                }));
            }
            if all_ports_list.is_empty() && primary_port > 0 {
                all_ports_list.push(primary_port.to_string());
            }

            let url_local = format!("http://127.0.0.1:{}", primary_port);
            let url_lan = format!("http://{}:{}", resolved_lan_ip, primary_port);
            let url_pub = format!("http://{}:{}", pub_host, primary_port);

            let s_obj = serde_json::json!({
                "port": primary_port,
                "ports": sub_ports,
                "url": url_local,
                "url_local": url_local,
                "url_lan": url_lan,
                "url_public": url_pub,
                "urls": urls_data,
                "host_local": "127.0.0.1",
                "host_lan": resolved_lan_ip,
                "host_public": pub_host,
            });
            services_data.insert(s_name.clone(), s_obj);

            let s_upper = s_name.to_uppercase().replace('-', "_");
            env_lines.push(format!("WS_SERVICE_{}_PORT={}\n", s_upper, primary_port));
            env_lines.push(format!("WS_SERVICE_{}_PORTS={}\n", s_upper, all_ports_list.join(",")));
            env_lines.push(format!("WS_SERVICE_{}_URL={}\n", s_upper, url_local));
            env_lines.push(format!("WS_SERVICE_{}_URL_LOCAL={}\n", s_upper, url_local));
            env_lines.push(format!("WS_SERVICE_{}_URL_LAN={}\n", s_upper, url_lan));
            env_lines.push(format!("WS_SERVICE_{}_URL_PUBLIC={}\n", s_upper, url_pub));
            env_lines.push(format!("WS_SERVICE_{}_HOST=127.0.0.1\n", s_upper));
            env_lines.push(format!("WS_SERVICE_{}_HOST_LAN={}\n", s_upper, resolved_lan_ip));

            for (p_label, &p_val) in &sub_ports {
                if p_label != "default" {
                    let lbl_upper = p_label.to_uppercase().replace('-', "_");
                    env_lines.push(format!("WS_SERVICE_{}_PORT_{}={}\n", s_upper, lbl_upper, p_val));
                    env_lines.push(format!("WS_SERVICE_{}_URL_{}=http://127.0.0.1:{}\n", s_upper, lbl_upper, p_val));
                    env_lines.push(format!("WS_SERVICE_{}_URL_LOCAL_{}=http://127.0.0.1:{}\n", s_upper, lbl_upper, p_val));
                    env_lines.push(format!("WS_SERVICE_{}_URL_LAN_{}=http://{}:{}\n", s_upper, lbl_upper, resolved_lan_ip, p_val));
                    env_lines.push(format!("WS_SERVICE_{}_URL_PUBLIC_{}=http://{}:{}\n", s_upper, lbl_upper, pub_host, p_val));
                }
            }
            env_lines.push("\n".to_string());
        }

        let json_path = ws_meta_dir.join("services.json");
        let descriptor = serde_json::json!({
            "workspace": workspace_name,
            "slot": slot,
            "lan_ip": resolved_lan_ip,
            "public_host": pub_host,
            "updated_at": crate::utils::get_iso_timestamp(),
            "services": services_data,
        });
        fs::write(&json_path, serde_json::to_string_pretty(&descriptor)?)?;

        let env_path = ws_meta_dir.join("services.env");
        fs::write(&env_path, env_lines.concat())?;

        Ok(json_path)
    }

    pub fn read_service_discovery_descriptor(workspace_dir: &Path) -> Option<HashMap<String, serde_json::Value>> {
        let json_path = workspace_dir.join(".ws").join("services.json");
        if json_path.exists() {
            if let Ok(content) = fs::read_to_string(&json_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(services) = val.get("services").and_then(|v| v.as_object()) {
                        let mut map = HashMap::new();
                        for (k, v) in services {
                            map.insert(k.clone(), v.clone());
                        }
                        return Some(map);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_placeholders() {
        let res = EnvEngine::resolve_template_string(
            "db_${WORKSPACE_NAME}_${REPO_NAME}_slot${WORKSPACE_SLOT}",
            "auth-flow",
            "server",
            2,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res, "db_auth-flow_server_slot2");

        let port_res = EnvEngine::resolve_template_string(
            "${PORT:3000}",
            "auth-flow",
            "web",
            3,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(port_res, "3030");

        let offset_res = EnvEngine::resolve_template_string(
            "${PORT_OFFSET:8000:5}",
            "auth-flow",
            "server",
            3,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(offset_res, "8015");
    }
}
