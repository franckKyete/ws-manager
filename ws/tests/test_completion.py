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
    assert "start|launch|run" in zsh_script
    assert "compdef _ws ws" in zsh_script

    bash_script = generate_completion_script("bash")
    assert "complete -F _ws_completion ws" in bash_script
    assert "_ws_completion" in bash_script

    fish_script = generate_completion_script("fish")
    assert "complete -c ws" in fish_script
    assert "__fish_ws_workspaces" in fish_script

    with pytest.raises(ValueError, match="Unsupported shell"):
        generate_completion_script("unsupported_shell")


def test_query_workspaces_and_repositories(tmp_path, monkeypatch):
    """Test querying workspaces and repositories dynamically."""
    proj_dir = tmp_path / "my_project"
    proj_dir.mkdir()
    ws_dir = proj_dir / "workspaces"
    ws_dir.mkdir()

    # Create workspace @feature-1 and @feature-2
    (ws_dir / "@feature-1").mkdir()
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
"""
    )

    monkeypatch.chdir(proj_dir)

    # Test querying workspaces
    workspaces = query_workspaces(include_sigil=True)
    names = [w[0] for w in workspaces]
    assert "@feature-1" in names
    assert "@feature-2" in names

    # Test querying repositories
    repos = query_repositories(include_sigil=True)
    repo_names = [r[0] for r in repos]
    assert "%server" in repo_names
    assert "%mobile" in repo_names

    # Test query_completions dispatcher
    ws_output = query_completions("workspaces")
    assert any(item.startswith("@feature-1") for item in ws_output)

    repo_output = query_completions("repos")
    assert any(item.startswith("%server") for item in repo_output)


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
