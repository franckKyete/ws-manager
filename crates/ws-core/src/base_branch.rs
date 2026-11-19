use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::errors::WSError;
use crate::git::GitService;
use crate::output::OutputHandler;

/// A request for the user to choose a base branch.
pub struct BasePrompt<'a> {
    pub repo: &'a str,
    /// Detected default, if one could be found.
    pub suggested: Option<&'a str>,
    /// Validation error from the previous attempt, if any.
    pub error: Option<&'a str>,
    /// Number of repositories still waiting after this one.
    pub remaining: usize,
}

/// The user's answer to a [`BasePrompt`].
pub struct BaseChoice {
    pub branch: String,
    /// Reuse this branch for the remaining repositories where it exists.
    pub apply_to_all: bool,
}

/// Terminal-agnostic prompt hook. The CLI implements this with `dialoguer`;
/// core remains deterministic and testable.
pub trait BaseBranchResolver {
    fn resolve(&mut self, request: BasePrompt<'_>) -> Result<BaseChoice, WSError>;
}

pub struct BaseBranchOptions<'a> {
    pub interactive: bool,
    pub resolver: Option<&'a mut dyn BaseBranchResolver>,
}

impl<'a> BaseBranchOptions<'a> {
    pub fn non_interactive() -> Self {
        Self {
            interactive: false,
            resolver: None,
        }
    }

    pub fn interactive(resolver: &'a mut dyn BaseBranchResolver) -> Self {
        Self {
            interactive: true,
            resolver: Some(resolver),
        }
    }
}

/// A repository that needs a base branch resolved.
pub struct BaseCandidate {
    pub repo: String,
    pub bare: PathBuf,
    pub suggested: Option<String>,
}

pub fn detect_base_at(git: &GitService, bare: &Path) -> Option<String> {
    git.detect_default_base(bare)
}

pub fn validate_base_at(
    git: &GitService,
    bare: &Path,
    repo: &str,
    branch: &str,
) -> Result<String, WSError> {
    let normalized = git.normalize_base_branch(bare, branch).ok_or_else(|| {
        WSError::Validation(format!(
            "Base branch for repository '{}' cannot be empty",
            repo
        ))
    })?;
    if git.base_branch_exists(bare, &normalized, true) {
        Ok(normalized)
    } else {
        Err(WSError::BranchNotFound(format!(
            "Base branch '{}' does not exist locally or on the primary remote for repository '{}'",
            normalized, repo
        )))
    }
}

/// Resolve every candidate that has no configured base branch.
///
/// Interactive mode prompts with the detected default and supports
/// apply-to-all. Non-interactive mode accepts each repository's detected
/// default and warns; a repository with no detectable default is a hard error.
pub fn resolve_missing_bases(
    git: &GitService,
    candidates: &[BaseCandidate],
    options: &mut BaseBranchOptions<'_>,
) -> Result<HashMap<String, String>, WSError> {
    let mut resolved = HashMap::new();
    if candidates.is_empty() {
        return Ok(resolved);
    }

    if !options.interactive || options.resolver.is_none() {
        for candidate in candidates {
            let Some(branch) = candidate.suggested.as_deref() else {
                return Err(WSError::Validation(format!(
                    "No base branch is configured for repository '{}' and no default branch could be detected. Set 'base_branch' in ws-project.yml or run interactively.",
                    candidate.repo
                )));
            };
            OutputHandler::print_warning(&format!(
                "No base_branch configured for repository '{}'; using detected default '{}' and saving it to ws-project.yml.",
                candidate.repo, branch
            ));
            resolved.insert(candidate.repo.clone(), branch.to_string());
        }
        return Ok(resolved);
    }

    let resolver = options
        .resolver
        .as_mut()
        .expect("interactive mode requires a resolver");
    let mut shared: Option<String> = None;

    for (idx, candidate) in candidates.iter().enumerate() {
        let remaining = candidates.len().saturating_sub(idx + 1);
        let mut error: Option<String> = None;

        if let Some(ref branch) = shared {
            match validate_base_at(git, &candidate.bare, &candidate.repo, branch) {
                Ok(normalized) => {
                    OutputHandler::print_info(&format!(
                        "Using base branch '{}' for repository '{}' (apply to all).",
                        normalized, candidate.repo
                    ));
                    resolved.insert(candidate.repo.clone(), normalized);
                    continue;
                }
                Err(_) => {
                    let message = format!(
                        "Base branch '{}' does not exist in repository '{}'; choose a branch for this repository.",
                        branch, candidate.repo
                    );
                    OutputHandler::print_warning(&message);
                    error = Some(message);
                }
            }
        }

        loop {
            let choice = resolver.resolve(BasePrompt {
                repo: &candidate.repo,
                suggested: candidate.suggested.as_deref(),
                error: error.as_deref(),
                remaining,
            })?;

            let raw = choice.branch.trim();
            let branch = if raw.is_empty() {
                candidate.suggested.clone().ok_or_else(|| {
                    WSError::Validation(format!(
                        "No base branch was provided for repository '{}' and no default could be detected.",
                        candidate.repo
                    ))
                })?
            } else {
                raw.to_string()
            };

            match validate_base_at(git, &candidate.bare, &candidate.repo, &branch) {
                Ok(normalized) => {
                    resolved.insert(candidate.repo.clone(), normalized.clone());
                    if choice.apply_to_all && remaining > 0 {
                        shared = Some(normalized);
                    }
                    break;
                }
                Err(e) => {
                    error = Some(e.to_string());
                }
            }
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn run_bare(bare: &Path, args: &[&str]) -> String {
        let mut full = vec!["--git-dir".to_string(), bare.to_string_lossy().to_string()];
        full.extend(args.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        run_git(&std::env::current_dir().unwrap(), &refs)
    }

    fn bare_fixture(dir: &Path, head: &str, branches: &[&str]) -> PathBuf {
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        run_git(&src, &["init", "-q"]);
        run_git(&src, &["config", "user.email", "test@example.com"]);
        run_git(&src, &["config", "user.name", "Test"]);
        fs::write(src.join("README.md"), "hello").unwrap();
        run_git(&src, &["add", "."]);
        run_git(&src, &["commit", "-qm", "init"]);
        run_git(&src, &["branch", "-M", head]);
        for branch in branches {
            if *branch != head {
                run_git(&src, &["branch", branch]);
            }
        }

        let bare = dir.join("repo.git");
        run_git(
            dir,
            &[
                "clone",
                "--bare",
                src.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        run_bare(
            &bare,
            &["symbolic-ref", "HEAD", &format!("refs/heads/{}", head)],
        );
        bare
    }

    struct FakeResolver {
        answers: Vec<(String, bool)>,
        calls: usize,
        saw_error: bool,
    }

    impl BaseBranchResolver for FakeResolver {
        fn resolve(&mut self, request: BasePrompt<'_>) -> Result<BaseChoice, WSError> {
            self.calls += 1;
            if request.error.is_some() {
                self.saw_error = true;
            }
            let (branch, apply_to_all) = self.answers.remove(0);
            Ok(BaseChoice {
                branch,
                apply_to_all,
            })
        }
    }

    #[test]
    fn normalizes_remote_and_refs_prefixes() {
        let dir = tempdir().unwrap();
        let bare = bare_fixture(dir.path(), "main", &["develop"]);
        let git = GitService::default();

        assert_eq!(
            git.normalize_base_branch(&bare, "origin/main").as_deref(),
            Some("main")
        );
        assert_eq!(
            git.normalize_base_branch(&bare, "refs/heads/main")
                .as_deref(),
            Some("main")
        );
        assert_eq!(
            git.normalize_base_branch(&bare, "refs/remotes/origin/develop")
                .as_deref(),
            Some("develop")
        );
        assert_eq!(git.normalize_base_branch(&bare, "   "), None);
    }

    #[test]
    fn detects_origin_head_before_bare_head() {
        let dir = tempdir().unwrap();
        let bare = bare_fixture(dir.path(), "main", &["develop"]);
        let git = GitService::default();

        assert_eq!(git.detect_default_base(&bare).as_deref(), Some("main"));

        let develop_sha = run_bare(&bare, &["rev-parse", "refs/heads/develop"]);
        run_bare(
            &bare,
            &["update-ref", "refs/remotes/origin/develop", &develop_sha],
        );
        run_bare(
            &bare,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ],
        );

        assert_eq!(git.detect_default_base(&bare).as_deref(), Some("develop"));
    }

    #[test]
    fn validates_local_and_remote_tracking_branches() {
        let dir = tempdir().unwrap();
        let bare = bare_fixture(dir.path(), "main", &["develop"]);
        let git = GitService::default();

        assert!(git.base_branch_exists(&bare, "main", false));
        assert!(git.base_branch_exists(&bare, "develop", false));

        let main_sha = run_bare(&bare, &["rev-parse", "refs/heads/main"]);
        run_bare(&bare, &["branch", "-D", "develop"]);
        run_bare(
            &bare,
            &["update-ref", "refs/remotes/origin/develop", &main_sha],
        );
        assert!(git.base_branch_exists(&bare, "develop", false));
        assert!(!git.base_branch_exists(&bare, "missing", false));
    }

    #[test]
    fn apply_to_all_reuses_branch_and_reprompts_only_failures() {
        let dir = tempdir().unwrap();
        let bare_a = bare_fixture(&dir.path().join("a"), "main", &["develop"]);
        let bare_b = bare_fixture(&dir.path().join("b"), "develop", &[]);
        let git = GitService::default();

        let candidates = vec![
            BaseCandidate {
                repo: "a".to_string(),
                bare: bare_a,
                suggested: Some("main".to_string()),
            },
            BaseCandidate {
                repo: "b".to_string(),
                bare: bare_b,
                suggested: Some("develop".to_string()),
            },
        ];

        let mut resolver = FakeResolver {
            answers: vec![("main".to_string(), true), ("develop".to_string(), false)],
            calls: 0,
            saw_error: false,
        };

        let resolved = {
            let mut options = BaseBranchOptions::interactive(&mut resolver);
            resolve_missing_bases(&git, &candidates, &mut options).unwrap()
        };
        assert_eq!(resolved.get("a").map(String::as_str), Some("main"));
        assert_eq!(resolved.get("b").map(String::as_str), Some("develop"));
        assert_eq!(resolver.calls, 2);
        assert!(resolver.saw_error);
    }

    #[test]
    fn non_interactive_uses_detected_defaults_and_fails_without_one() {
        let dir = tempdir().unwrap();
        let bare = bare_fixture(dir.path(), "main", &[]);
        let git = GitService::default();

        let candidates = vec![BaseCandidate {
            repo: "api".to_string(),
            bare: bare.clone(),
            suggested: Some("main".to_string()),
        }];
        let mut options = BaseBranchOptions::non_interactive();
        let resolved = resolve_missing_bases(&git, &candidates, &mut options).unwrap();
        assert_eq!(resolved.get("api").map(String::as_str), Some("main"));

        let missing = vec![BaseCandidate {
            repo: "api".to_string(),
            bare,
            suggested: None,
        }];
        let err = resolve_missing_bases(&git, &missing, &mut options).unwrap_err();
        assert!(err.to_string().contains("No base branch is configured"));
    }
}
