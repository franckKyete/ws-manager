use dialoguer::{Confirm, Input};
use std::io::IsTerminal;
use ws_core::base_branch::{BaseBranchResolver, BaseChoice, BasePrompt};
use ws_core::WSError;

/// `dialoguer`-backed resolver used by the CLI. Core owns detection,
/// normalization, validation, and persistence; this type only talks to the
/// terminal.
pub struct DialoguerBaseBranchResolver;

impl BaseBranchResolver for DialoguerBaseBranchResolver {
    fn resolve(&mut self, request: BasePrompt<'_>) -> Result<BaseChoice, WSError> {
        if let Some(error) = request.error {
            eprintln!("{error}");
        }

        let prompt = match request.suggested {
            Some(suggested) => format!(
                "Base branch for %{} (detected: {})",
                request.repo, suggested
            ),
            None => format!("Base branch for %{} (no default detected)", request.repo),
        };
        let mut input = Input::<String>::new().with_prompt(prompt);

        if let Some(suggested) = request.suggested {
            input = input.with_initial_text(suggested);
        } else {
            input = input.allow_empty(false).validate_with(|value: &String| {
                if value.trim().is_empty() {
                    Err("A branch name is required.")
                } else {
                    Ok(())
                }
            });
        }

        let raw = input.interact_text().map_err(prompt_error)?;
        let branch = if raw.trim().is_empty() {
            request.suggested.unwrap_or_default().to_string()
        } else {
            raw
        };

        let mut apply_to_all = false;
        if request.remaining > 0 && !branch.trim().is_empty() {
            apply_to_all = Confirm::new()
                .with_prompt(format!(
                    "Use '{}' for the remaining {} repositor{}?",
                    branch.trim(),
                    request.remaining,
                    if request.remaining == 1 { "y" } else { "ies" }
                ))
                .default(false)
                .interact()
                .map_err(prompt_error)?;
        }

        Ok(BaseChoice {
            branch,
            apply_to_all,
        })
    }
}

fn prompt_error(error: dialoguer::Error) -> WSError {
    match &error {
        dialoguer::Error::IO(io) if io.kind() == std::io::ErrorKind::Interrupted => {
            WSError::General("Base branch prompt aborted".to_string())
        }
        _ => WSError::General(format!("Base branch prompt failed: {error}")),
    }
}

pub fn interactive_tty() -> bool {
    std::io::stdin().is_terminal()
}
