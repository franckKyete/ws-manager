use crate::cli::LifecycleCommandArgs;
use crate::helpers::{parse_repo_token, sigil_hint};
use ws_core::lifecycle::{self, LifecycleState};
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

const LIFECYCLE_USAGE: &str = "\
Usage:
  ws lifecycle [@workspace] [%repo ...] run [--all] [+stage ...] [flags]
  ws lifecycle [@workspace] [%repo ...] status [+stage ...]

Examples:
  ws lifecycle @feat run
  ws lifecycle @feat %server run +configure +build
  ws lifecycle @feat %server status +install
  ws lifecycle %api run +post_launch

With no stage arguments, 'run' executes configure followed by every setup stage.
Stage names must be prefixed with '+' (for example '+configure' or '+install').";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleVerb {
    Run,
    Status,
}

impl LifecycleVerb {
    fn as_str(self) -> &'static str {
        match self {
            LifecycleVerb::Run => "run",
            LifecycleVerb::Status => "status",
        }
    }
}

#[derive(Debug)]
struct LifecycleInvocation {
    workspace: Option<String>,
    repos: Vec<String>,
    verb: LifecycleVerb,
    stages: Vec<String>,
}

struct ResolvedLifecycle {
    ws_name: String,
    target_repos: Option<Vec<String>>,
    selected_repos: Vec<String>,
    inferred_repo: Option<String>,
}

fn usage_error(message: &str) -> String {
    format!("{}\n\n{}", message, LIFECYCLE_USAGE)
}

fn invalid_verb(manager: &WorkspaceManager, token: &str) -> String {
    if token.starts_with('+') {
        return usage_error(&format!(
            "Stage token '{}' must come after the lifecycle action. Try 'run {}' or 'status {}'.",
            token, token, token
        ));
    }
    if token.starts_with('@') || token.starts_with('%') {
        return usage_error(&format!(
            "Unexpected '{}' after the lifecycle action slot. Workspace and repository arguments must come before 'run' or 'status'.",
            token
        ));
    }
    usage_error(&format!(
        "Expected lifecycle action 'run' or 'status', found '{}'.\n{}\nIf '{}' is a stage, run it with: ws lifecycle ... run +{}",
        token,
        sigil_hint(manager, token),
        token,
        token
    ))
}

fn parse_invocation(
    manager: &WorkspaceManager,
    raw: &[String],
) -> Result<LifecycleInvocation, String> {
    if raw.is_empty() {
        return Err(usage_error(
            "Lifecycle action required: specify 'run' or 'status'.",
        ));
    }

    let mut idx = 0;
    let mut workspace: Option<String> = None;

    if let Some(token) = raw.get(idx) {
        if let Some(rest) = token.strip_prefix('@') {
            if rest.is_empty() {
                return Err("Workspace name after '@' is empty.".to_string());
            }
            workspace = Some(rest.to_string());
            idx += 1;
        }
    }

    let mut repos: Vec<String> = Vec::new();
    while let Some(token) = raw.get(idx) {
        if !token.starts_with('%') {
            break;
        }
        let repo = parse_repo_token(manager, token)?;
        if !repos.contains(&repo) {
            repos.push(repo);
        }
        idx += 1;
    }

    let verb_token = match raw.get(idx) {
        Some(token) => token,
        None => {
            return Err(usage_error(
                "Lifecycle action required: specify 'run' or 'status'.",
            ))
        }
    };

    let verb = match verb_token.as_str() {
        "run" => LifecycleVerb::Run,
        "status" => LifecycleVerb::Status,
        other => return Err(invalid_verb(manager, other)),
    };
    idx += 1;

    let mut stages: Vec<String> = Vec::new();
    while let Some(token) = raw.get(idx) {
        if let Some(stage) = token.strip_prefix('+') {
            if stage.is_empty() {
                return Err("Stage name after '+' is empty.".to_string());
            }
            stages.push(stage.to_string());
        } else if token.starts_with('@') || token.starts_with('%') {
            return Err(usage_error(&format!(
                "Unexpected '{}' after '{}'. Workspace and repository arguments must come before the lifecycle action.",
                token,
                verb.as_str()
            )));
        } else {
            return Err(usage_error(&format!(
                "Unexpected argument '{}'. Lifecycle stage names must start with '+' (for example '+{}').",
                token, token
            )));
        }
        idx += 1;
    }

    Ok(LifecycleInvocation {
        workspace,
        repos,
        verb,
        stages,
    })
}

fn resolve_invocation(
    manager: &WorkspaceManager,
    invocation: &LifecycleInvocation,
    all: bool,
) -> Result<ResolvedLifecycle, String> {
    let (detected_ws, detected_repo) = manager.detect_context(None);

    let ws_name = match &invocation.workspace {
        Some(ws) => ws.clone(),
        None => detected_ws.ok_or_else(|| {
            "Could not detect a workspace from the current directory. Pass '@<workspace>' (for example '@feat') or run this command from inside a workspace."
                .to_string()
        })?,
    };

    if all && !invocation.repos.is_empty() {
        return Err(
            "'--all' cannot be combined with explicit repository targets. Remove the '%repo' arguments or drop '--all'."
                .to_string(),
        );
    }

    let (meta, _) = manager
        .get_workspace_info(&ws_name)
        .map_err(|e| e.to_string())?;
    let mut all_repos: Vec<String> = meta.repositories.keys().cloned().collect();
    all_repos.sort();

    // Repository inference only when no @workspace, %repo, or --all was given.
    let inferred_repo = if invocation.workspace.is_none() && invocation.repos.is_empty() && !all {
        detected_repo
    } else {
        None
    };

    let target_repos: Option<Vec<String>> = if all {
        None
    } else if !invocation.repos.is_empty() {
        Some(invocation.repos.clone())
    } else {
        inferred_repo.clone().map(|repo| vec![repo])
    };

    let selected_repos = match &target_repos {
        Some(repos) => repos.clone(),
        None => all_repos,
    };

    Ok(ResolvedLifecycle {
        ws_name,
        target_repos,
        selected_repos,
        inferred_repo,
    })
}

fn display_stage(key: &str) -> String {
    format!("+{}", key.strip_prefix("setup:").unwrap_or(key))
}

fn map_stage_tokens(
    manager: &WorkspaceManager,
    selected_repos: &[String],
    stages: &[String],
) -> Result<Vec<String>, String> {
    let known = lifecycle::known_stages_for_repos(&manager.config, selected_repos);
    let mut keys = Vec::new();

    for stage in stages {
        let key = if stage == "configure" {
            "configure".to_string()
        } else if lifecycle::is_event_stage(stage) {
            stage.clone()
        } else {
            format!("setup:{}", stage)
        };

        if known.iter().any(|candidate| candidate == &key) {
            keys.push(key);
        } else {
            let known_display: Vec<String> = known.iter().map(|k| display_stage(k)).collect();
            return Err(format!(
                "Unknown lifecycle stage '+{}'.\nKnown stages: {}",
                stage,
                known_display.join(", ")
            ));
        }
    }

    Ok(keys)
}

fn default_stage_keys(manager: &WorkspaceManager, selected_repos: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = vec!["configure".to_string()];

    for stage in lifecycle::setup_stage_names(&manager.config, None) {
        let key = format!("setup:{}", stage);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    for repo in selected_repos {
        for stage in lifecycle::setup_stage_names(&manager.config, Some(repo)) {
            let key = format!("setup:{}", stage);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }

    keys
}

fn validate_flags(args: &LifecycleCommandArgs, verb: LifecycleVerb) -> Result<(), String> {
    if verb == LifecycleVerb::Status {
        if args.all {
            return Err("'--all' is only valid with 'run'. Status always reports every selected repository.".to_string());
        }
        if args.force || args.dry_run || args.skip_configure {
            return Err(
                "'--force', '--dry-run', and '--skip-configure' are only valid with 'run'."
                    .to_string(),
            );
        }
        if args.interface.is_some() || args.lan_ip.is_some() {
            return Err("'--interface' and '--ip' are only valid with 'run'.".to_string());
        }
    }
    Ok(())
}

fn execute_status(
    manager: &WorkspaceManager,
    resolved: &ResolvedLifecycle,
    stage_keys: &[String],
) -> Result<(), String> {
    let ws_dir = manager.get_workspace_dir(&resolved.ws_name);
    let state = LifecycleState::load(&ws_dir);

    println!("Lifecycle status for @{}:", resolved.ws_name);
    let mut printed = false;

    for stage_key in stage_keys {
        let stage_label = if let Some(stage) = stage_key.strip_prefix("setup:") {
            format!("setup/{}", stage)
        } else {
            stage_key.clone()
        };

        let workspace_hooks = lifecycle::workspace_stage_hooks(&manager.config, stage_key);
        if !workspace_hooks.is_empty() {
            printed = true;
            let key = LifecycleState::key(None, stage_key);
            let status = state
                .records
                .get(&key)
                .map(|r| r.status.as_str())
                .unwrap_or("pending");
            println!("▶ {}", stage_label);
            for (i, hook) in workspace_hooks.iter().enumerate() {
                if hook.run.is_none() {
                    continue;
                }
                let display = hook
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("hook {}", i + 1));
                let details = hook.run.clone().unwrap_or_default();
                OutputHandler::print_setup_step(
                    i + 1,
                    &format!("[{}] {}", display, status),
                    &details,
                    if status == "success" {
                        "success"
                    } else {
                        "info"
                    },
                );
            }
        }

        for repo_name in &resolved.selected_repos {
            let hooks =
                lifecycle::stage_hooks_for_repo(&manager.config, Some(repo_name), stage_key);
            if hooks.is_empty() {
                continue;
            }
            printed = true;
            let key = LifecycleState::key(Some(repo_name), stage_key);
            let status = state
                .records
                .get(&key)
                .map(|r| r.status.as_str())
                .unwrap_or("pending");
            println!("  {} ({})", repo_name, stage_label);
            for hook in hooks {
                if hook.run.is_none() {
                    continue;
                }
                let display = hook.name.clone().unwrap_or_else(|| "hook".to_string());
                let details = hook.run.clone().unwrap_or_default();
                OutputHandler::print_setup_step(
                    1,
                    &format!("[{}] {}", display, status),
                    &details,
                    if status == "success" {
                        "success"
                    } else {
                        "info"
                    },
                );
            }
        }
    }

    if !printed {
        println!("  (no lifecycle stages configured)");
    }

    Ok(())
}

fn execute_run(
    manager: &mut WorkspaceManager,
    resolved: &ResolvedLifecycle,
    stage_keys: &[String],
    args: &LifecycleCommandArgs,
) -> Result<(), String> {
    if let Some(inferred) = &resolved.inferred_repo {
        let has_repo_hooks = stage_keys.iter().any(|stage_key| {
            !lifecycle::stage_hooks_for_repo(&manager.config, Some(inferred), stage_key).is_empty()
        });
        if !has_repo_hooks {
            OutputHandler::print_info(&format!(
                "Repository '%{}' has no lifecycle hooks; running workspace-scope stages only.",
                inferred
            ));
        }
    }

    let target = resolved.target_repos.as_deref();
    for stage_key in stage_keys {
        manager
            .setup_workspace_stage_with_force(
                &resolved.ws_name,
                target,
                args.dry_run,
                false,
                false,
                args.interface.as_deref(),
                args.lan_ip.as_deref(),
                stage_key,
                args.force,
            )
            .map_err(|e| e.to_string())?;
    }

    OutputHandler::print_success(&format!(
        "Lifecycle run completed for @{}",
        resolved.ws_name
    ));
    Ok(())
}

pub fn execute_lifecycle(
    manager: &mut WorkspaceManager,
    args: &LifecycleCommandArgs,
) -> Result<(), String> {
    let invocation = parse_invocation(manager, &args.raw)?;
    validate_flags(args, invocation.verb)?;
    let resolved = resolve_invocation(manager, &invocation, args.all)?;

    let mut stage_keys = if invocation.stages.is_empty() {
        default_stage_keys(manager, &resolved.selected_repos)
    } else {
        map_stage_tokens(manager, &resolved.selected_repos, &invocation.stages)?
    };

    match invocation.verb {
        LifecycleVerb::Run => {
            // Explicit '+configure' always runs. Automatic configure is inserted
            // before setup stages unless '--skip-configure' suppresses it.
            if !invocation.stages.is_empty()
                && !args.skip_configure
                && stage_keys.iter().any(|key| key.starts_with("setup:"))
                && !stage_keys.iter().any(|key| key == "configure")
            {
                stage_keys.insert(0, "configure".to_string());
            }
            execute_run(manager, &resolved, &stage_keys, args)
        }
        LifecycleVerb::Status => execute_status(manager, &resolved, &stage_keys),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_manager(lifecycle: Option<serde_json::Value>) -> (tempfile::TempDir, WorkspaceManager) {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().to_path_buf();
        let config = ws_core::models::AppConfig {
            project_root: root.clone(),
            repositories: HashMap::new(),
            workspaces_dir: root.join("workspaces"),
            config_file_path: None,
            global_env: HashMap::new(),
            secret_env: HashMap::new(),
            private_env: HashMap::new(),
            dynamic_env: HashMap::new(),
            setup: Vec::new(),
            secrets: Vec::new(),
            copy_files: Vec::new(),
            tmux: None,
            hub_auto_save: None,
            hub_project: None,
            hub: serde_json::Value::Null,
            lifecycle,
            schema_version: Some(ws_core::lifecycle::LIFECYCLE_SCHEMA_VERSION),
        };
        (temp_dir, WorkspaceManager::new(config, None))
    }

    fn raw(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn test_lifecycle_requires_action() {
        let (_tmp, manager) = test_manager(None);
        let err = parse_invocation(&manager, &[]).unwrap_err();
        assert!(err.contains("Lifecycle action required"));
        let err = parse_invocation(&manager, &raw(&["@feat"])).unwrap_err();
        assert!(err.contains("Lifecycle action required"));
    }

    #[test]
    fn test_lifecycle_rejects_stage_before_action() {
        let (_tmp, manager) = test_manager(None);
        let err = parse_invocation(&manager, &raw(&["+install", "run"])).unwrap_err();
        assert!(err.contains("must come after the lifecycle action"));
    }

    #[test]
    fn test_lifecycle_rejects_bare_stage_after_action() {
        let (_tmp, manager) = test_manager(None);
        let err = parse_invocation(&manager, &raw(&["run", "install"])).unwrap_err();
        assert!(err.contains("must start with '+'"));
    }

    #[test]
    fn test_lifecycle_parses_positional_order() {
        let (_tmp, manager) = test_manager(None);
        let parsed = parse_invocation(
            &manager,
            &raw(&["@feat", "%server", "%mobile", "run", "+install"]),
        )
        .unwrap();
        assert_eq!(parsed.workspace.as_deref(), Some("feat"));
        assert_eq!(parsed.repos, vec!["server", "mobile"]);
        assert_eq!(parsed.verb, LifecycleVerb::Run);
        assert_eq!(parsed.stages, vec!["install"]);
    }

    #[test]
    fn test_lifecycle_maps_known_stages() {
        let (_tmp, manager) = test_manager(Some(serde_json::json!({
            "setup": {
                "install": "echo install",
                "verify": "echo verify"
            }
        })));
        let keys =
            map_stage_tokens(&manager, &[], &raw(&["configure", "install", "pre_launch"])).unwrap();
        assert_eq!(
            keys,
            vec![
                "configure".to_string(),
                "setup:install".to_string(),
                "pre_launch".to_string()
            ]
        );

        let err = map_stage_tokens(&manager, &[], &raw(&["nope"])).unwrap_err();
        assert!(err.contains("Unknown lifecycle stage"));
        assert!(err.contains("+install"));
    }
}
