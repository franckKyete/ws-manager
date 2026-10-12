use crate::helpers::{clean_repo, clean_workspace};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use ws_core::models::{RepoConfig, RepoSpec};
use ws_core::WorkspaceManager;

pub fn parse_create_workspace_args(
    workspace_name: &str,
    raw_args: &[String],
    repositories: &HashMap<String, RepoConfig>,
    target_branch: Option<&str>,
) -> Result<Vec<RepoSpec>, String> {
    let clean_ws =
        clean_workspace(Some(workspace_name)).unwrap_or_else(|| workspace_name.to_string());
    let mut global_existing = false;
    let mut include_all = false;
    let mut selected_repos: Option<HashSet<String>> = None;
    let mut explicit_specs: HashMap<String, RepoSpec> = HashMap::new();
    let mut target_branch = target_branch.map(|s| s.to_string());

    let mut idx = 0;
    while idx < raw_args.len() {
        let arg = &raw_args[idx];

        if arg == "--all" {
            include_all = true;
            idx += 1;
            continue;
        } else if arg == "--repos" || arg == "--only" {
            if idx + 1 >= raw_args.len() {
                return Err(format!(
                    "Option '{}' requires a comma-separated list of repository names",
                    arg
                ));
            }
            let repos_str = &raw_args[idx + 1];
            let set: HashSet<String> = repos_str
                .split(',')
                .filter(|r| !r.trim().is_empty())
                .map(|r| clean_repo(Some(r.trim())).unwrap())
                .collect();
            selected_repos = Some(set);
            idx += 2;
            continue;
        } else if arg.starts_with("--repos=") || arg.starts_with("--only=") {
            let repos_str = arg.split_once('=').unwrap().1;
            let set: HashSet<String> = repos_str
                .split(',')
                .filter(|r| !r.trim().is_empty())
                .map(|r| clean_repo(Some(r.trim())).unwrap())
                .collect();
            selected_repos = Some(set);
            idx += 1;
            continue;
        } else if arg == "--existing" {
            global_existing = true;
            idx += 1;
            continue;
        } else if arg == "--new" {
            global_existing = false;
            idx += 1;
            continue;
        } else if arg == "--target"
            || arg == "--target-branch"
            || arg == "--base"
            || arg == "--from"
            || arg == "-t"
        {
            if idx + 1 >= raw_args.len() {
                return Err(format!("Option '{}' requires a branch argument", arg));
            }
            target_branch = Some(raw_args[idx + 1].clone());
            idx += 2;
            continue;
        } else if arg.starts_with("--target=")
            || arg.starts_with("--target-branch=")
            || arg.starts_with("--base=")
            || arg.starts_with("--from=")
        {
            target_branch = Some(arg.split_once('=').unwrap().1.to_string());
            idx += 1;
            continue;
        } else if arg == "--no-tmux" {
            idx += 1;
            continue;
        } else if arg == "--cmd" || arg == "--command" {
            idx += 2;
            continue;
        } else if arg.starts_with("--cmd=") || arg.starts_with("--command=") {
            idx += 1;
            continue;
        }

        let clean_arg = clean_repo(Some(arg)).unwrap();

        // 1. Colon tag syntax (#server:main, server:feat:existing, server:feat:new:develop)
        if clean_arg.contains(':') && !clean_arg.starts_with('-') {
            let parts: Vec<&str> = clean_arg.split(':').collect();
            let repo_key = parts[0];
            if !repositories.contains_key(repo_key) {
                return Err(format!(
                    "Unknown repository '{}' in argument '{}'. Configured repositories: {}",
                    repo_key,
                    arg,
                    repositories.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }

            let branch_val = if parts.len() > 1 && !parts[1].is_empty() {
                parts[1].to_string()
            } else {
                clean_ws.clone()
            };

            let mut create_mode = !global_existing;
            let mut repo_base = target_branch.clone();

            if parts.len() > 2 {
                if parts[2] == "existing" || parts[2] == "exist" {
                    create_mode = false;
                    if parts.len() > 3 {
                        repo_base = Some(parts[3].to_string());
                    }
                } else if parts[2] == "new" || parts[2] == "create" {
                    create_mode = true;
                    if parts.len() > 3 {
                        repo_base = Some(parts[3].to_string());
                    }
                } else {
                    repo_base = Some(parts[2].to_string());
                }
            }

            let mut spec = RepoSpec::new(
                repo_key,
                branch_val,
                create_mode,
                &repositories[repo_key].checkout,
            );
            spec.base_branch = repo_base;
            explicit_specs.insert(repo_key.to_string(), spec);
            idx += 1;
            continue;
        }

        // 2. Key=value syntax (server=main, server=main:existing)
        if clean_arg.contains('=') && !clean_arg.starts_with('-') {
            let parts: Vec<&str> = clean_arg.splitn(2, '=').collect();
            let repo_key = parts[0];
            let branch_val = parts[1];

            if !repositories.contains_key(repo_key) {
                return Err(format!(
                    "Unknown repository '{}' in argument '{}'. Configured repositories: {}",
                    repo_key,
                    arg,
                    repositories.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }

            let mut create_mode = !global_existing;
            let branch_name = if let Some(stripped) = branch_val.strip_suffix(":existing") {
                create_mode = false;
                stripped.to_string()
            } else if let Some(stripped) = branch_val.strip_suffix(":new") {
                create_mode = true;
                stripped.to_string()
            } else {
                if idx + 1 < raw_args.len() {
                    if raw_args[idx + 1] == "--existing" {
                        create_mode = false;
                        idx += 1;
                    } else if raw_args[idx + 1] == "--new" {
                        create_mode = true;
                        idx += 1;
                    }
                }
                branch_val.to_string()
            };

            let mut spec = RepoSpec::new(
                repo_key,
                branch_name,
                create_mode,
                &repositories[repo_key].checkout,
            );
            spec.base_branch = target_branch.clone();
            explicit_specs.insert(repo_key.to_string(), spec);
            idx += 1;
            continue;
        }

        // 3. Positional repo name (#server, server)
        if repositories.contains_key(&clean_arg) && !clean_arg.starts_with('-') {
            let repo_key = clean_arg;
            let mut create_mode = !global_existing;
            if idx + 1 < raw_args.len() {
                if raw_args[idx + 1] == "--existing" {
                    create_mode = false;
                    idx += 1;
                } else if raw_args[idx + 1] == "--new" {
                    create_mode = true;
                    idx += 1;
                }
            }

            let branch_name = clean_ws.clone();
            let mut spec = RepoSpec::new(
                &repo_key,
                branch_name,
                create_mode,
                &repositories[&repo_key].checkout,
            );
            spec.base_branch = target_branch.clone();
            explicit_specs.insert(repo_key, spec);
            idx += 1;
            continue;
        }

        // 4. Legacy flags (--server-new, --server-existing, --server)
        let mut matched = false;
        for repo_name in repositories.keys() {
            let new_flag = format!("--{}-new", repo_name);
            let exist_flag = format!("--{}-existing", repo_name);
            let plain_flag = format!("--{}", repo_name);

            if arg == &new_flag {
                if idx + 1 >= raw_args.len() {
                    return Err(format!("Option '{}' requires a branch argument", new_flag));
                }
                let branch_name = &raw_args[idx + 1];
                let mut spec = RepoSpec::new(
                    repo_name,
                    branch_name,
                    true,
                    &repositories[repo_name].checkout,
                );
                spec.base_branch = target_branch.clone();
                explicit_specs.insert(repo_name.clone(), spec);
                idx += 2;
                matched = true;
                break;
            } else if arg == &exist_flag {
                if idx + 1 >= raw_args.len() {
                    return Err(format!(
                        "Option '{}' requires a branch argument",
                        exist_flag
                    ));
                }
                let branch_name = &raw_args[idx + 1];
                let mut spec = RepoSpec::new(
                    repo_name,
                    branch_name,
                    false,
                    &repositories[repo_name].checkout,
                );
                spec.base_branch = target_branch.clone();
                explicit_specs.insert(repo_name.clone(), spec);
                idx += 2;
                matched = true;
                break;
            } else if arg == &plain_flag {
                if idx + 1 >= raw_args.len() {
                    return Err(format!(
                        "Option '{}' requires a branch argument",
                        plain_flag
                    ));
                }
                let branch_name = &raw_args[idx + 1];
                let mut spec = RepoSpec::new(
                    repo_name,
                    branch_name,
                    !global_existing,
                    &repositories[repo_name].checkout,
                );
                spec.base_branch = target_branch.clone();
                explicit_specs.insert(repo_name.clone(), spec);
                idx += 2;
                matched = true;
                break;
            }
        }

        if !matched {
            return Err(format!("Unknown argument or option: '{}'", arg));
        }
    }

    if !include_all && selected_repos.is_none() && explicit_specs.is_empty() {
        return Err(
            "Explicit repository selection required. Specify repositories as '%repo[:branch]' (e.g. 'ws create @name %mobile %server'), or specify '--all'.".to_string(),
        );
    }

    let target_names: HashSet<String> = if include_all {
        repositories.keys().cloned().collect()
    } else if let Some(ref sel) = selected_repos {
        for r in sel {
            if !repositories.contains_key(r) {
                return Err(format!(
                    "Repository '{}' is not in project configuration. Available repositories: {}",
                    r,
                    repositories.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        sel.clone()
    } else {
        explicit_specs.keys().cloned().collect()
    };

    let mut sorted_targets: Vec<_> = target_names.into_iter().collect();
    sorted_targets.sort();

    let mut final_specs = Vec::new();
    for repo_name in sorted_targets {
        if let Some(mut spec) = explicit_specs.remove(&repo_name) {
            if spec.base_branch.is_none() && target_branch.is_some() {
                spec.base_branch = target_branch.clone();
            }
            final_specs.push(spec);
        } else {
            let repo_cfg = &repositories[&repo_name];
            let (branch_name, create) = if global_existing {
                (clean_ws.clone(), false)
            } else {
                (format!("feature/{}", clean_ws), true)
            };
            let mut spec = RepoSpec::new(&repo_name, &branch_name, create, &repo_cfg.checkout);
            spec.base_branch = target_branch.clone();
            final_specs.push(spec);
        }
    }

    Ok(final_specs)
}

pub fn execute_create(
    manager: &mut WorkspaceManager,
    name: Option<&str>,
    file: Option<&Path>,
    setup: bool,
    tmux_cmd: Option<&str>,
    no_tmux: bool,
    raw_args: &[String],
    target_branch: Option<&str>,
) -> Result<(), String> {
    if let Some(cfg_file) = file {
        let meta = manager
            .create_workspace_from_config(cfg_file, tmux_cmd, no_tmux)
            .map_err(|e| e.to_string())?;
        if setup {
            let _ = manager.setup_workspace(&meta.name, None, false, false, false, None, None);
        }
        return Ok(());
    }

    let ws_name =
        match name {
            Some(n) => clean_workspace(Some(n)).unwrap(),
            None => return Err(
                "Workspace name (@<name>) is required for 'ws create' unless '-f/--file' is used."
                    .to_string(),
            ),
        };

    let repo_specs = parse_create_workspace_args(
        &ws_name,
        raw_args,
        &manager.config.repositories,
        target_branch,
    )?;

    manager
        .create_workspace(&ws_name, &repo_specs, tmux_cmd, no_tmux)
        .map_err(|e| e.to_string())?;

    if setup {
        let _ = manager.setup_workspace(&ws_name, None, false, false, false, None, None);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repos() -> HashMap<String, RepoConfig> {
        let mut repos = HashMap::new();
        repos.insert(
            "backend".to_string(),
            RepoConfig {
                url: Some("git@github.com:org/backend.git".to_string()),
                checkout: "backend".to_string(),
                ..Default::default()
            },
        );
        repos.insert(
            "frontend".to_string(),
            RepoConfig {
                url: Some("git@github.com:org/frontend.git".to_string()),
                checkout: "frontend".to_string(),
                ..Default::default()
            },
        );
        repos
    }

    #[test]
    fn test_parse_create_all() {
        let repos = sample_repos();
        let specs =
            parse_create_workspace_args("feat", &["--all".to_string()], &repos, None).unwrap();
        assert_eq!(specs.len(), 2);
        assert!(specs
            .iter()
            .any(|s| s.name == "backend" && s.branch == "feature/feat" && s.create));
        assert!(specs
            .iter()
            .any(|s| s.name == "frontend" && s.branch == "feature/feat" && s.create));
    }

    #[test]
    fn test_parse_create_explicit_tag() {
        let repos = sample_repos();
        let specs = parse_create_workspace_args(
            "feat",
            &[
                "%backend:v1".to_string(),
                "%frontend:main:existing".to_string(),
            ],
            &repos,
            None,
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        let backend = specs.iter().find(|s| s.name == "backend").unwrap();
        assert_eq!(backend.branch, "v1");
        assert!(backend.create);

        let frontend = specs.iter().find(|s| s.name == "frontend").unwrap();
        assert_eq!(frontend.branch, "main");
        assert!(!frontend.create);
    }

    #[test]
    fn test_parse_create_repos_filter() {
        let repos = sample_repos();
        let specs = parse_create_workspace_args(
            "feat",
            &["--repos=backend".to_string()],
            &repos,
            Some("develop"),
        )
        .unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "backend");
        assert_eq!(specs[0].branch, "feature/feat");
        assert_eq!(specs[0].base_branch, Some("develop".to_string()));
    }
}
