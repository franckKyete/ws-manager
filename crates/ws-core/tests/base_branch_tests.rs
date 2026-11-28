use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use ws_core::base_branch::BaseBranchOptions;
use ws_core::config::ConfigLoader;
use ws_core::models::RepoSpec;
use ws_core::WorkspaceManager;

fn run_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed:\n{}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_source(dir: &Path, branches: &[&str]) {
    fs::create_dir_all(dir).unwrap();
    run_git(dir, &["init", "-q"]);
    fs::write(dir.join("README.md"), "hello\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-qm", "init"]);
    run_git(dir, &["branch", "-M", "main"]);
    for branch in branches {
        if *branch != "main" {
            run_git(dir, &["branch", branch]);
            run_git(dir, &["checkout", "-q", branch]);
            fs::write(dir.join(format!("{}.txt", branch)), "diverged\n").unwrap();
            run_git(dir, &["add", "."]);
            run_git(dir, &["commit", "-qm", &format!("commit on {}", branch)]);
        }
    }
    run_git(dir, &["checkout", "-q", "main"]);
}

fn clone_bare(source: &Path, target: &Path) {
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    run_git(
        target.parent().unwrap(),
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            target.to_str().unwrap(),
        ],
    );
}

fn head_of(worktree: &Path) -> String {
    run_git(worktree, &["rev-parse", "HEAD"])
}

fn branch_head(bare: &Path, branch: &str) -> String {
    run_git(
        bare.parent().unwrap(),
        &[
            "--git-dir",
            bare.to_str().unwrap(),
            "rev-parse",
            &format!("refs/heads/{}", branch),
        ],
    )
}

fn load_manager(project: &Path) -> WorkspaceManager {
    let config_path = project.join("ws.yml");
    let config = ConfigLoader::load_config(Some(&config_path), None, true).unwrap();
    WorkspaceManager::new(config, None)
}

fn spec(repo: &str, branch: &str, base: Option<&str>) -> RepoSpec {
    RepoSpec {
        name: repo.to_string(),
        branch: branch.to_string(),
        create: true,
        path: repo.to_string(),
        frozen: false,
        locked: false,
        base_branch: base.map(str::to_string),
    }
}

struct Fixture {
    _dir: TempDir,
    project: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();

    let src = dir.path().join("src");
    init_source(&src, &["develop"]);
    clone_bare(&src, &project.join("bares/src.git"));

    let web = dir.path().join("web");
    init_source(&web, &["develop"]);
    clone_bare(&web, &project.join("bares/web.git"));

    fs::write(
        project.join("ws.yml"),
        "schema_version: 3\nrepositories:\n  src:\n    bare: bares/src.git\n    checkout: src\n  web:\n    bare: bares/web.git\n    checkout: web\n",
    )
    .unwrap();

    Fixture { _dir: dir, project }
}

#[test]
fn init_detects_and_persists_base_for_cloned_repo() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("init-project");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("ws.yml"),
        "schema_version: 3\nrepositories: {}\n",
    )
    .unwrap();

    let source = dir.path().join("init-src");
    init_source(&source, &["develop"]);

    let mut manager = load_manager(&project);
    let mut options = BaseBranchOptions::non_interactive();
    manager
        .init_project_with_options(&[source.to_string_lossy().to_string()], &mut options)
        .unwrap();

    assert!(project.join("bares/init-src.git").is_dir());
    let saved = fs::read_to_string(project.join("ws.yml")).unwrap();
    assert!(
        saved.contains("base_branch: main"),
        "init should persist the detected default:\n{saved}"
    );
}

#[test]
fn create_persists_detected_base_and_uses_it_for_new_branch() {
    let fx = fixture();
    let manager = load_manager(&fx.project);

    let mut options = BaseBranchOptions::non_interactive();
    manager
        .create_workspace_with_options(
            "feat",
            &[spec("src", "feat", None)],
            None,
            true,
            &mut options,
        )
        .unwrap();

    let worktree = fx.project.join("workspaces/feat/src");
    assert_eq!(
        head_of(&worktree),
        branch_head(&fx.project.join("bares/src.git"), "main")
    );

    let saved = fs::read_to_string(fx.project.join("ws.yml")).unwrap();
    assert!(
        saved.contains("base_branch: main"),
        "detected base should be persisted:\n{saved}"
    );

    let metadata = fs::read_to_string(fx.project.join("workspaces/feat/workspace.yml")).unwrap();
    assert!(metadata.contains("base_branch: main"));
}

#[test]
fn explicit_base_override_is_one_shot_and_not_persisted() {
    let fx = fixture();
    let manager = load_manager(&fx.project);

    let mut options = BaseBranchOptions::non_interactive();
    manager
        .create_workspace_with_options(
            "feat",
            &[spec("src", "feat", Some("develop"))],
            None,
            true,
            &mut options,
        )
        .unwrap();

    let worktree = fx.project.join("workspaces/feat/src");
    assert_eq!(
        head_of(&worktree),
        branch_head(&fx.project.join("bares/src.git"), "develop")
    );

    let saved = fs::read_to_string(fx.project.join("ws.yml")).unwrap();
    assert!(
        !saved.contains("base_branch"),
        "one-shot override must not be persisted:\n{saved}"
    );
}

#[test]
fn repo_add_uses_explicit_base_and_records_metadata() {
    let fx = fixture();
    let manager = load_manager(&fx.project);

    let mut options = BaseBranchOptions::non_interactive();
    manager
        .create_workspace_with_options(
            "multi",
            &[spec("src", "multi", None)],
            None,
            true,
            &mut options,
        )
        .unwrap();

    manager
        .workspace_add_repo_with_options(
            "multi",
            "web",
            "feature/web",
            true,
            Some("develop"),
            &mut options,
        )
        .unwrap();

    let worktree = fx.project.join("workspaces/multi/web");
    assert_eq!(
        head_of(&worktree),
        branch_head(&fx.project.join("bares/web.git"), "develop")
    );

    let metadata = fs::read_to_string(fx.project.join("workspaces/multi/workspace.yml")).unwrap();
    assert!(metadata.contains("base_branch: develop"));
}

#[test]
fn declarative_file_requires_base_and_accepts_explicit_base() {
    let fx = fixture();
    let manager = load_manager(&fx.project);

    let missing = fx.project.join("missing.yml");
    fs::write(
        &missing,
        "name: missing\nrepositories:\n  src:\n    branch: missing\n    create: true\n",
    )
    .unwrap();
    let error = manager
        .create_workspace_from_config(&missing, None, true)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("must specify a base branch"),
        "unexpected error: {error}"
    );
    assert!(!fx.project.join("workspaces/missing").exists());

    let ok = fx.project.join("ok.yml");
    fs::write(
        &ok,
        "name: ok\nrepositories:\n  src:\n    branch: ok\n    create: true\n    base: develop\n",
    )
    .unwrap();
    manager
        .create_workspace_from_config(&ok, None, true)
        .unwrap();
    let worktree = fx.project.join("workspaces/ok/src");
    assert_eq!(
        head_of(&worktree),
        branch_head(&fx.project.join("bares/src.git"), "develop")
    );
}
