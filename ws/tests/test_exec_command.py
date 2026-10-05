import pytest
from pathlib import Path
from unittest.mock import patch, MagicMock
from ws.models import AppConfig, RepoConfig, RepoSpec, WorkspaceMetadata
from ws.workspace import WorkspaceManager, run_shell_command
from ws.cli import main, resolve_ws_and_repo_args


@pytest.fixture
def exec_test_env(tmp_path):
    bares = tmp_path / "bares"
    bares.mkdir()
    workspaces = tmp_path / "workspaces"
    workspaces.mkdir()

    server_bare = bares / "server.git"
    server_bare.mkdir()
    hub_bare = bares / "hub.git"
    hub_bare.mkdir()

    config = AppConfig(
        config_file_path=tmp_path / "repositories.yml",
        workspaces_dir=workspaces,
        repositories={
            "server": RepoConfig(name="server", bare=server_bare, checkout="api-server"),
            "hub": RepoConfig(name="hub", bare=hub_bare, checkout="wshub"),
        },
    )

    manager = WorkspaceManager(config=config)

    # Create workspace on disk
    ws_dir = workspaces / "my-ws"
    ws_dir.mkdir()
    (ws_dir / "api-server").mkdir()
    (ws_dir / "wshub").mkdir()

    meta = WorkspaceMetadata(
        name="my-ws",
        created="2026-10-05T00:00:00Z",
        status="active",
        repositories={
            "server": RepoSpec(name="server", branch="main", create=False, path="api-server"),
            "hub": RepoSpec(name="hub", branch="main", create=False, path="wshub"),
        },
    )
    manager._save_metadata(ws_dir, meta)

    return {"manager": manager, "ws_dir": ws_dir, "meta": meta, "config": config}


def test_exec_workspace_all_repos(exec_test_env):
    manager = exec_test_env["manager"]

    with patch("ws.workspace.run_shell_command", return_value=0) as mock_run:
        results = manager.exec_workspace("my-ws", "git status")
        assert results == {"server": 0, "hub": 0}
        assert mock_run.call_count == 2


def test_exec_workspace_single_repo(exec_test_env):
    manager = exec_test_env["manager"]
    ws_dir = exec_test_env["ws_dir"]

    with patch("ws.workspace.run_shell_command", return_value=0) as mock_run:
        results = manager.exec_workspace("my-ws", "ga .", repos=["hub"])
        assert results == {"hub": 0}
        assert mock_run.call_count == 1
        mock_run.assert_called_once_with("ga .", cwd=ws_dir / "wshub")


def test_exec_workspace_by_checkout_name(exec_test_env):
    manager = exec_test_env["manager"]
    ws_dir = exec_test_env["ws_dir"]

    with patch("ws.workspace.run_shell_command", return_value=0) as mock_run:
        results = manager.exec_workspace("my-ws", "git status", repos=["wshub"])
        assert results == {"hub": 0}
        mock_run.assert_called_once_with("git status", cwd=ws_dir / "wshub")


def test_exec_cli_parsing_single_repo_with_sigil(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=("my-ws", None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "%hub", "ga", "."])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["ga", "."]
                    assert call_kwargs["repos"] == ["hub"]


def test_exec_cli_parsing_explicit_workspace_and_repo(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=(None, None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "@my-ws", "%hub", "--", "git", "status"])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["git", "status"]
                    assert call_kwargs["repos"] == ["hub"]


def test_exec_cli_parsing_all_repos_delimiter(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=(None, None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "@my-ws", "--", "npm", "test"])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["npm", "test"]
                    assert call_kwargs["repos"] is None


def test_exec_workspace_quoted_arguments(exec_test_env):
    manager = exec_test_env["manager"]
    ws_dir = exec_test_env["ws_dir"]

    with patch("ws.workspace.run_shell_command", return_value=0) as mock_run:
        results = manager.exec_workspace(
            "my-ws",
            ["gc", "-m", "chore: Setup skill and add initial docs"],
            repos=["hub"],
        )
        assert results == {"hub": 0}
        assert mock_run.call_count == 1
        mock_run.assert_called_once_with(
            "gc -m 'chore: Setup skill and add initial docs'",
            cwd=ws_dir / "wshub",
        )


def test_exec_workspace_single_string_in_list(exec_test_env):
    manager = exec_test_env["manager"]
    ws_dir = exec_test_env["ws_dir"]

    with patch("ws.workspace.run_shell_command", return_value=0) as mock_run:
        results = manager.exec_workspace(
            "my-ws",
            ["git status"],
            repos=["hub"],
        )
        assert results == {"hub": 0}
        assert mock_run.call_count == 1
        mock_run.assert_called_once_with("git status", cwd=ws_dir / "wshub")


def test_exec_cli_parsing_delimiter_quoted_message(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=("my-ws", None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "%hub", "--", "gc", "-m", "chore: Setup skill and add initial docs"])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["gc", "-m", "chore: Setup skill and add initial docs"]
                    assert call_kwargs["repos"] == ["hub"]


def test_exec_cli_parsing_multiple_repos(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=("my-ws", None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "%hub", "%server", "--", "ga", "."])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["ga", "."]
                    assert call_kwargs["repos"] == ["hub", "server"]


def test_exec_cli_parsing_comma_separated_repos(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=("my-ws", None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "%hub,%server", "--", "ga", "."])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["ga", "."]
                    assert call_kwargs["repos"] == ["hub", "server"]


def test_exec_cli_parsing_all_flag(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=("my-ws", None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "--all", "--", "ga", "."])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["ga", "."]
                    assert call_kwargs["repos"] is None


def test_exec_cli_parsing_repos_flag(exec_test_env):
    with patch("ws.cli.ConfigLoader.load_config", return_value=exec_test_env["config"]):
        with patch.object(WorkspaceManager, "detect_context", return_value=("my-ws", None)):
            with patch.object(WorkspaceManager, "has_workspace", side_effect=lambda name: name == "my-ws"):
                with patch("ws.cli.cmd_exec") as mock_cmd_exec:
                    exit_code = main(["exec", "--repos", "hub,server", "--", "ga", "."])
                    assert exit_code == 0
                    mock_cmd_exec.assert_called_once()
                    call_kwargs = mock_cmd_exec.call_args[1]
                    assert call_kwargs["name"] == "my-ws"
                    assert call_kwargs["command"] == ["ga", "."]
                    assert call_kwargs["repos"] == ["hub", "server"]


def test_cmd_exec_failure_exit_code(exec_test_env):
    from ws.commands import cmd_exec
    manager = exec_test_env["manager"]

    with patch.object(manager, "exec_workspace", return_value={"hub": 0, "server": 1}):
        code = cmd_exec(manager=manager, name="my-ws", command=["ga", "."])
        assert code == 1

    with patch.object(manager, "exec_workspace", return_value={"hub": 0, "server": 0}):
        code = cmd_exec(manager=manager, name="my-ws", command=["ga", "."])
        assert code == 0


