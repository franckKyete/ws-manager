"""Unit tests for CLI parser and parameter handling."""

import pytest
from pathlib import Path

from ws.cli import clean_workspace, clean_repo, clean_repos, normalize_cli_args, parse_create_workspace_args
from ws.models import RepoConfig, RepoSpec
from ws.exceptions import WSException


@pytest.fixture
def sample_repositories() -> dict[str, RepoConfig]:
    return {
        "server": RepoConfig(name="server", bare=Path("Renttik-server.git"), checkout="Renttik-server"),
        "mobile": RepoConfig(name="mobile", bare=Path("Renttik-mobile.git"), checkout="Renttik-mobile"),
    }


def test_clean_sigil_helpers():
    """Test @ and % sigil stripping helpers (including forgiving stripping of +, :, #, $)."""
    assert clean_workspace("@develop") == "develop"
    assert clean_workspace("develop") == "develop"
    assert clean_repo("%mobile") == "mobile"
    assert clean_repo("mobile") == "mobile"
    assert clean_repo("+mobile") == "mobile"
    assert clean_repo(":mobile") == "mobile"
    assert clean_repo("#mobile") == "mobile"
    assert clean_repos(["%server", "mobile", "+mobile"]) == ["server", "mobile", "mobile"]


def test_parse_create_default(sample_repositories):
    """Test 'ws create @auth --all' (default create feature/auth for both)."""
    specs = parse_create_workspace_args("@auth", ["--all"], sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="feature/auth", create=True, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="feature/auth", create=True, path="Renttik-mobile")


def test_parse_create_existing_global(sample_repositories):
    """Test 'ws create @auth --all --existing'."""
    specs = parse_create_workspace_args("@auth", ["--all", "--existing"], sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="auth", create=False, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="auth", create=False, path="Renttik-mobile")


def test_parse_create_colon_syntax(sample_repositories):
    """Test 'ws create @auth %server:main:existing %mobile:feature/auth-ui:new'."""
    raw_args = ["%server:main:existing", "%mobile:feature/auth-ui:new"]
    specs = parse_create_workspace_args("@auth", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="main", create=False, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="feature/auth-ui", create=True, path="Renttik-mobile")


def test_parse_create_positional_repos_with_sigil(sample_repositories):
    """Test 'ws create @auth %server %mobile'."""
    raw_args = ["%server", "%mobile"]
    specs = parse_create_workspace_args("@auth", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="feature/auth", create=True, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="feature/auth", create=True, path="Renttik-mobile")


def test_parse_create_mixed_colon_and_equal(sample_repositories):
    """Test 'ws create @auth %server:develop %mobile=feature/ui'."""
    raw_args = ["%server:develop", "%mobile=feature/ui"]
    specs = parse_create_workspace_args("@auth", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert spec_dict["server"] == RepoSpec(name="server", branch="develop", create=True, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="feature/ui", create=True, path="Renttik-mobile")


def test_parse_create_unknown_arg_raises(sample_repositories):
    """Test unknown parameter raises WSException."""
    with pytest.raises(WSException, match="Unknown argument"):
        parse_create_workspace_args("@auth", ["--all", "--invalid-flag"], sample_repositories)


def test_parse_create_no_flag_raises(sample_repositories):
    """Test missing selection raises WSException."""
    with pytest.raises(WSException, match="Explicit repository selection required"):
        parse_create_workspace_args("@auth", [], sample_repositories)


def test_normalize_cli_args_with_sigils():
    """Test flexible CLI argument normalization with @ sigil and standard names."""
    assert normalize_cli_args(["@develop", "start"]) == ["start", "@develop"]
    assert normalize_cli_args(["@develop", "start", "--tmux"]) == ["start", "@develop", "--tmux"]
    assert normalize_cli_args(["@develop", "shell", "%server"]) == ["shell", "@develop", "%server"]
    assert normalize_cli_args(["@develop", "status"]) == ["status", "@develop"]
    assert normalize_cli_args(["@develop", "stop"]) == ["stop", "@develop"]
    assert normalize_cli_args(["develop", "attach"]) == ["attach", "develop"]
    assert normalize_cli_args(["start", "@develop", "%mobile"]) == ["start", "@develop", "%mobile"]
    assert normalize_cli_args([]) == []



def test_cli_commands_imported():
    """Verify all subcommands and command functions are imported in cli.py."""
    import ws.cli as cli

    assert callable(cli.cmd_start)
    assert callable(cli.cmd_attach)
    assert callable(cli.cmd_shell)
    assert callable(cli.cmd_delete)
    assert callable(cli.cmd_stop)
    assert callable(cli.cmd_restart)
    assert callable(cli.cmd_logs)
    assert callable(cli.cmd_repo_add)
    assert callable(cli.cmd_repo_lock)
    assert callable(cli.cmd_repo_unlock)






