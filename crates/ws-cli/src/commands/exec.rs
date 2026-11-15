use crate::helpers::{parse_repo_token, sigil_hint};
use ws_core::output::OutputHandler;
use ws_core::WorkspaceManager;

pub fn execute_exec(
    manager: &WorkspaceManager,
    name: Option<&str>,
    repos: &[String],
    all: bool,
    command: &[String],
) -> Result<(), String> {
    let (detected_ws, _) = manager.detect_context(None);

    let mut ws_name: Option<String> = None;
    let mut target_repos: Vec<String> = Vec::new();

    if let Some(token) = name {
        let token = token.trim();
        if let Some(rest) = token.strip_prefix('@') {
            if rest.is_empty() {
                return Err("Workspace name after '@' is empty.".to_string());
            }
            ws_name = Some(rest.to_string());
        } else if token.starts_with('%') {
            target_repos.push(parse_repo_token(manager, token)?);
        } else {
            return Err(sigil_hint(manager, token));
        }
    }

    for token in repos {
        let repo = parse_repo_token(manager, token)?;
        if !repo.is_empty() && !target_repos.contains(&repo) {
            target_repos.push(repo);
        }
    }

    if all && !target_repos.is_empty() {
        return Err(
            "'--all' cannot be combined with explicit repository targets. Remove the '%repo' arguments or drop '--all'."
                .to_string(),
        );
    }

    let ws = match ws_name.or(detected_ws) {
        Some(w) => w,
        None => {
            return Err(
                "Workspace name (@<name>) required for 'ws exec'. Pass '@<workspace>' or run this command from inside a workspace directory."
                    .to_string(),
            )
        }
    };

    let mut exec_cmd: Vec<String> = command.to_vec();
    if exec_cmd.first().map(|s| s.as_str()) == Some("--") {
        exec_cmd.remove(0);
    }
    if exec_cmd.is_empty() {
        return Err("Command required for 'ws exec'. Pass it after '--'.".to_string());
    }

    let filter = if all || target_repos.is_empty() {
        None
    } else {
        Some(target_repos)
    };

    let results = manager
        .exec_workspace(&ws, &exec_cmd, filter.as_deref())
        .map_err(|e| e.to_string())?;

    let failed: Vec<_> = results
        .iter()
        .filter_map(|(r, code)| if *code != 0 { Some(r.as_str()) } else { None })
        .collect();

    if !failed.is_empty() {
        OutputHandler::print_warning(&format!(
            "Command exited with non-zero status in: {}",
            failed.join(", ")
        ));
        std::process::exit(1);
    }

    Ok(())
}
