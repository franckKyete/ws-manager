"""Unit tests for CLI parser and parameter handling."""

import pytest
from pathlib import Path

from ws.cli import parse_new_workspace_args
from ws.models import RepoConfig, RepoSpec
from ws.exceptions import WSException


@pytest.fixture
def sample_repositories() -> dict[str, RepoConfig]:
    return {
        "server": RepoConfig(name="server", bare=Path("Renttik-server.git"), checkout="Renttik-server"),
        "mobile": RepoConfig(name="mobile", bare=Path("Renttik-mobile.git"), checkout="Renttik-mobile"),
    }


def test_parse_new_default(sample_repositories):
    """Test 'ws new auth --all' (default create feature/auth for both)."""
    specs = parse_new_workspace_args("auth", ["--all"], sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="feature/auth", create=True, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="feature/auth", create=True, path="Renttik-mobile")


def test_parse_new_existing_global(sample_repositories):
    """Test 'ws new auth --all --existing'."""
    specs = parse_new_workspace_args("auth", ["--all", "--existing"], sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="auth", create=False, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="auth", create=False, path="Renttik-mobile")



def test_parse_new_repo_existing_specific(sample_repositories):
    """Test 'ws new auth --server-existing develop --mobile-existing main'."""
    raw_args = ["--server-existing", "develop", "--mobile-existing", "main"]
    specs = parse_new_workspace_args("auth", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert spec_dict["server"] == RepoSpec(name="server", branch="develop", create=False, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="main", create=False, path="Renttik-mobile")


def test_parse_new_repo_new_specific(sample_repositories):
    """Test 'ws new auth --server-new feature/auth-api --mobile-new feature/auth-ui'."""
    raw_args = ["--server-new", "feature/auth-api", "--mobile-new", "feature/auth-ui"]
    specs = parse_new_workspace_args("auth", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert spec_dict["server"] == RepoSpec(name="server", branch="feature/auth-api", create=True, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="feature/auth-ui", create=True, path="Renttik-mobile")


def test_parse_new_mixed_mode(sample_repositories):
    """Test 'ws new auth --server-new feature/auth-api --mobile-existing develop'."""
    raw_args = ["--server-new", "feature/auth-api", "--mobile-existing", "develop"]
    specs = parse_new_workspace_args("auth", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert spec_dict["server"] == RepoSpec(name="server", branch="feature/auth-api", create=True, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="develop", create=False, path="Renttik-mobile")


def test_parse_new_unknown_arg_raises(sample_repositories):
    """Test unknown parameter raises WSException."""
    with pytest.raises(WSException, match="Unknown argument"):
        parse_new_workspace_args("auth", ["--all", "--invalid-flag"], sample_repositories)


def test_parse_new_subset(sample_repositories):
    """Test 'ws new auth --repos server' creates workspace with subset of repos."""
    specs = parse_new_workspace_args("auth", ["--repos", "server"], sample_repositories)
    assert len(specs) == 1
    assert specs[0].name == "server"
    assert specs[0].branch == "feature/auth"


def test_parse_new_no_flag_raises(sample_repositories):
    """Test missing --all or --repos/--only raises WSException."""
    with pytest.raises(WSException, match="Explicit repository selection required"):
        parse_new_workspace_args("auth", [], sample_repositories)


def test_parse_new_positional_repo_branch(sample_repositories):
    """Test 'ws new auth-ui server=main --existing mobile=main --existing'."""
    raw_args = ["server=main", "--existing", "mobile=main", "--existing"]
    specs = parse_new_workspace_args("auth-ui", raw_args, sample_repositories)
    spec_dict = {s.name: s for s in specs}

    assert len(specs) == 2
    assert spec_dict["server"] == RepoSpec(name="server", branch="main", create=False, path="Renttik-server")
    assert spec_dict["mobile"] == RepoSpec(name="mobile", branch="main", create=False, path="Renttik-mobile")


