"""Unit tests for shell completion generators and dynamic query resolver."""

from pathlib import Path
import pytest

from ws.cli import build_parser, normalize_cli_args
from ws.completion import (
    generate_completion_script,
    install_completion,
    query_completions,
    query_repositories,
    query_workspaces,
)


def test_generate_completion_scripts():
    """Test generating completion scripts for all supported shells."""
    zsh_script = generate_completion_script("zsh")
    assert "#compdef ws" in zsh_script
    assert "_ws_workspaces" in zsh_script
    assert "_ws_repositories" in zsh_script
    assert "_ws_workspace_or_service" in zsh_script
    assert 'local cmd="${words[1]}"' in zsh_script
    assert "repos_all" in zsh_script
    assert "start|launch|run" in zsh_script
    assert "compdef _ws ws" in zsh_script

    bash_script = generate_completion_script("bash")
    assert "complete -F _ws_completion ws" in bash_script
    assert "_ws_completion" in bash_script
    assert "declare -F _init_completion" in bash_script
    assert "workspaces_all" in bash_script
    assert "repos_all" in bash_script

    fish_script = generate_completion_script("fish")
    assert "complete -c ws" in fish_script
    assert "__fish_ws_workspaces" in fish_script
    assert "workspaces_all" in fish_script
    assert "repos_all" in fish_script

    with pytest.raises(ValueError, match="Unsupported shell"):
        generate_completion_script("unsupported_shell")


def test_query_workspaces_and_repositories(tmp_path, monkeypatch):
    """Test querying workspaces and repositories dynamically."""
    proj_dir = tmp_path / "my_project"
    proj_dir.mkdir()
    ws_dir = proj_dir / "workspaces"
    ws_dir.mkdir()

    # Create workspace @feature-1 with workspace.yml
    feat1_dir = ws_dir / "feature-1"
    feat1_dir.mkdir()
    (feat1_dir / "workspace.yml").write_text(
        """
name: feature-1
status: active
repositories:
  server:
    branch: feature/auth
    path: server
  mobile:
    branch: feature/auth-ui
    path: mobile
"""
    )

    # Create workspace @feature-2 without workspace.yml
    (ws_dir / "@feature-2").mkdir()

    # Create repositories.yml
    (proj_dir / "repositories.yml").write_text(
        """
repositories:
  server:
    bare: bares/server.git
    checkout: server
    command: npm run dev
  mobile:
    bare: bares/mobile.git
    checkout: mobile
    command: npx expo start
  worker:
    bare: bares/worker.git
    checkout: worker
    command: python worker.py
"""
    )

    monkeypatch.chdir(proj_dir)

    # Test querying workspaces
    workspaces = query_workspaces(include_sigil=True)
    names = [w[0] for w in workspaces]
    assert "@feature-1" in names
    assert "@feature-2" in names

    # Verify description from workspace.yml
    feat1_desc = next(d for c, d in workspaces if c == "@feature-1")
    assert "2 repos" in feat1_desc
    assert "active" in feat1_desc

    # Test querying repositories without workspace (falls back to repositories.yml)
    repos = query_repositories(include_sigil=True)
    repo_names = [r[0] for r in repos]
    assert "%server" in repo_names
    assert "%mobile" in repo_names
    assert "%worker" in repo_names

    # Test querying repositories for specific workspace (reads workspace.yml)
    ws_repos = query_repositories(workspace_name="feature-1", include_sigil=True)
    ws_repo_names = [r[0] for r in ws_repos]
    assert "%server" in ws_repo_names
    assert "%mobile" in ws_repo_names
    assert "%worker" not in ws_repo_names
    assert next(d for c, d in ws_repos if c == "%server") == "branch: feature/auth"

    # Test query_repositories plain (no sigil)
    plain_repos = query_repositories(workspace_name="feature-1", include_sigil=False)
    assert ("server", "branch: feature/auth") in plain_repos

    # Test query_completions dispatcher
    ws_output = query_completions("workspaces")
    assert any(item.startswith("@feature-1") for item in ws_output)

    ws_all_output = query_completions("workspaces_all")
    assert any(item.startswith("@feature-1") for item in ws_all_output)
    assert any(item.startswith("feature-1") for item in ws_all_output)

    repo_output = query_completions("repos", "feature-1")
    assert any(item.startswith("%server") for item in repo_output)

    repo_all_output = query_completions("repos_all", "feature-1")
    assert any(item.startswith("%server") for item in repo_all_output)
    assert any(item.startswith("server") for item in repo_all_output)


def test_query_repositories_context_detection(tmp_path, monkeypatch):
    """Test query_repositories auto-detects active workspace from cwd context."""
    proj_dir = tmp_path / "my_project"
    proj_dir.mkdir()
    ws_dir = proj_dir / "workspaces"
    ws_dir.mkdir()

    feat_dir = ws_dir / "feature-ctx"
    feat_dir.mkdir()
    (feat_dir / "workspace.yml").write_text(
        """
name: feature-ctx
status: active
repositories:
  custom-svc:
    branch: feature/ctx-branch
    path: custom-svc
"""
    )
    svc_dir = feat_dir / "custom-svc"
    svc_dir.mkdir()

    # Create root repositories.yml
    (proj_dir / "repositories.yml").write_text(
        """
repositories:
  server:
    bare: bares/server.git
  custom-svc:
    bare: bares/custom.git
"""
    )

    # Change cwd to inside workspace subdirectory
    monkeypatch.chdir(svc_dir)

    # Querying repos without explicit workspace argument should detect feature-ctx
    repos = query_repositories(workspace_name=None, include_sigil=True)
    repo_names = [r[0] for r in repos]
    assert repo_names == ["%custom-svc"]
    assert repos[0][1] == "branch: feature/ctx-branch"


def test_cli_completion_subcommand():
    """Test CLI parser recognizes completion and _complete subcommands."""
    parser = build_parser()
    
    args_zsh = parser.parse_args(["completion", "zsh"])
    assert args_zsh.subcommand == "completion"
    assert args_zsh.shell == "zsh"

    args_install = parser.parse_args(["completion", "install"])
    assert args_install.subcommand == "completion"
    assert args_install.shell == "install"

    args_int = parser.parse_args(["_complete", "workspaces"])
    assert args_int.subcommand == "_complete"
    assert args_int.query_type == "workspaces"

    args_repos_all = parser.parse_args(["_complete", "repos_all", "@main"])
    assert args_repos_all.subcommand == "_complete"
    assert args_repos_all.query_type == "repos_all"
