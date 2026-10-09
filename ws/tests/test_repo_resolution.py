import pytest
from pathlib import Path
from ws.models import AppConfig, RepoConfig, RepoSpec, WorkspaceMetadata
from ws.workspace import WorkspaceManager
from ws.exceptions import RepositoryNotFoundException, RepoNotInWorkspaceException


@pytest.fixture
def mock_ws_env(tmp_path):
    bares = tmp_path / "bares"
    bares.mkdir()
    workspaces = tmp_path / "workspaces"
    workspaces.mkdir()

    # Create dummy bare repositories
    server_bare = bares / "server.git"
    server_bare.mkdir()
    mobile_bare = bares / "mobile.git"
    mobile_bare.mkdir()

    config = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        workspaces_dir=workspaces,
        repositories={
            "server": RepoConfig(name="server", bare=server_bare, checkout="api-server"),
            "mobile": RepoConfig(name="mobile", bare=mobile_bare, checkout="tools-react-native"),
        },
    )

    manager = WorkspaceManager(config=config)

    # Create workspace on disk
    ws_dir = workspaces / "test-ws"
    ws_dir.mkdir()
    (ws_dir / "api-server").mkdir()
    (ws_dir / "tools-react-native").mkdir()

    meta = WorkspaceMetadata(
        name="test-ws",
        created="2026-10-05T00:00:00Z",
        status="active",
        repositories={
            "server": RepoSpec(name="server", branch="main", create=False, path="api-server"),
            "mobile": RepoSpec(name="mobile", branch="main", create=False, path="tools-react-native"),
        },
    )
    manager._save_metadata(ws_dir, meta)

    return {"manager": manager, "ws_dir": ws_dir, "meta": meta}


def test_resolve_repo_spec_by_alias(mock_ws_env):
    manager = mock_ws_env["manager"]
    ws_dir = mock_ws_env["ws_dir"]

    # Resolve by alias
    r_key, spec, path = manager.resolve_repo_spec("test-ws", "mobile")
    assert r_key == "mobile"
    assert spec.path == "tools-react-native"
    assert path == ws_dir / "tools-react-native"

    # With sigil
    r_key2, spec2, path2 = manager.resolve_repo_spec("test-ws", "%mobile")
    assert r_key2 == "mobile"
    assert path2 == ws_dir / "tools-react-native"


def test_resolve_repo_spec_by_checkout_folder(mock_ws_env):
    manager = mock_ws_env["manager"]
    ws_dir = mock_ws_env["ws_dir"]

    # Resolve by checkout name
    r_key, spec, path = manager.resolve_repo_spec("test-ws", "tools-react-native")
    assert r_key == "mobile"
    assert spec.path == "tools-react-native"
    assert path == ws_dir / "tools-react-native"

    # With sigil
    r_key2, spec2, path2 = manager.resolve_repo_spec("test-ws", "%tools-react-native")
    assert r_key2 == "mobile"
    assert path2 == ws_dir / "tools-react-native"


def test_resolve_repo_spec_when_meta_has_checkout_key(mock_ws_env):
    manager = mock_ws_env["manager"]
    ws_dir = mock_ws_env["ws_dir"]

    # Re-save metadata where key is checkout name instead of alias
    meta = WorkspaceMetadata(
        name="test-ws",
        created="2026-10-05T00:00:00Z",
        status="active",
        repositories={
            "tools-react-native": RepoSpec(name="tools-react-native", branch="main", create=False, path="tools-react-native"),
        },
    )
    manager._save_metadata(ws_dir, meta)

    # Resolving via alias "mobile" should still work!
    r_key, spec, path = manager.resolve_repo_spec("test-ws", "mobile")
    assert r_key == "tools-react-native"
    assert path == ws_dir / "tools-react-native"

    # Resolving via "tools-react-native" should also work
    r_key2, spec2, path2 = manager.resolve_repo_spec("test-ws", "tools-react-native")
    assert r_key2 == "tools-react-native"
    assert path2 == ws_dir / "tools-react-native"


def test_resolve_repo_spec_not_found(mock_ws_env):
    manager = mock_ws_env["manager"]
    with pytest.raises(RepositoryNotFoundException):
        manager.resolve_repo_spec("test-ws", "non-existent")
