use std::fs;
use std::path::{Path, PathBuf};

pub fn get_global_config_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".config").join("ws")
    } else {
        PathBuf::from(".ws_global")
    }
}

pub fn get_projects_registry_file() -> PathBuf {
    get_global_config_dir().join("projects.json")
}

pub fn list_registered_projects(registry_path: Option<&Path>, prune_missing: bool) -> Vec<PathBuf> {
    let reg_file = registry_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(get_projects_registry_file);

    if !reg_file.is_file() {
        return Vec::new();
    }

    let mut result = Vec::new();
    if let Ok(content) = fs::read_to_string(&reg_file) {
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(&content) {
            let mut valid_paths = Vec::new();
            for p_str in &paths {
                let p = PathBuf::from(p_str);
                if p.is_dir() || !prune_missing {
                    valid_paths.push(p);
                }
            }
            if prune_missing && valid_paths.len() < paths.len() {
                let save_list: Vec<String> = valid_paths
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let _ = serde_json::to_string_pretty(&save_list).map(|s| fs::write(&reg_file, s));
            }
            result = valid_paths;
        }
    }
    result
}

pub fn register_project(project_path: &Path, registry_path: Option<&Path>) -> bool {
    let abs_path = if let Ok(canon) = project_path.canonicalize() {
        canon
    } else {
        project_path.to_path_buf()
    };

    let reg_file = registry_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(get_projects_registry_file);

    if let Some(parent) = reg_file.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut projects = list_registered_projects(Some(&reg_file), false);
    if projects.iter().any(|p| p == &abs_path) {
        return false;
    }

    projects.push(abs_path);
    let save_list: Vec<String> = projects
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    if let Ok(json_str) = serde_json::to_string_pretty(&save_list) {
        fs::write(&reg_file, json_str).is_ok()
    } else {
        false
    }
}

pub fn unregister_project(project_path: &Path, registry_path: Option<&Path>) -> bool {
    let abs_path = if let Ok(canon) = project_path.canonicalize() {
        canon
    } else {
        project_path.to_path_buf()
    };

    let reg_file = registry_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(get_projects_registry_file);

    let projects = list_registered_projects(Some(&reg_file), false);
    let original_len = projects.len();
    let updated: Vec<PathBuf> = projects.into_iter().filter(|p| p != &abs_path).collect();

    if updated.len() == original_len {
        return false;
    }

    let save_list: Vec<String> = updated
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    if let Ok(json_str) = serde_json::to_string_pretty(&save_list) {
        fs::write(&reg_file, json_str).is_ok()
    } else {
        false
    }
}
