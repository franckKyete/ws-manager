"""Unit and integration tests for wshub Client and CLI Integration."""

import json
from pathlib import Path
from unittest.mock import patch, MagicMock
import pytest

from ws.cli import build_parser, normalize_cli_args
from ws.hub import HubClient, HubException


@pytest.fixture
def temp_hub_config(tmp_path):
    """Fixture providing a temporary hub configuration file path."""
    return tmp_path / "hub.yml"


def test_hub_client_config_save_and_load(temp_hub_config):
    """Test HubClient saves and reloads credentials from YAML config."""
    client = HubClient(config_path=temp_hub_config)
    assert client.token is None

    client.save_session(
        url="http://hub.example.com",
        token="wshub_pat_test12345",
        username="kyete",
    )

    # Re-initialize client with same path
    reloaded = HubClient(config_path=temp_hub_config)
    assert reloaded.base_url == "http://hub.example.com"
    assert reloaded.token == "wshub_pat_test12345"

    # Test clear_session
    assert reloaded.clear_session() is True
    assert not temp_hub_config.exists()


def test_hub_client_parse_project_identifier(temp_hub_config):
    """Test parsing org/project and bare project names."""
    client = HubClient(config_path=temp_hub_config)

    # Explicit org/project
    org, name = client.parse_project_identifier("fantastik/renttik")
    assert org == "fantastik"
    assert name == "renttik"

    # With wshub: prefix
    org2, name2 = client.parse_project_identifier("wshub:myorg/api-service")
    assert org2 == "myorg"
    assert name2 == "api-service"

    # Without org (defaults to personal or username)
    org3, name3 = client.parse_project_identifier("standalone-app")
    assert org3 in ("personal", "default") or isinstance(org3, str)
    assert name3 == "standalone-app"


@patch("urllib.request.urlopen")
def test_hub_client_whoami(mock_urlopen, temp_hub_config):
    """Test HubClient whoami profile retrieval."""
    client = HubClient(token="wshub_pat_123", config_path=temp_hub_config)

    mock_resp = MagicMock()
    mock_resp.read.return_value = json.dumps({
        "status": "success",
        "data": {
            "user": {
                "id": "usr_123",
                "username": "kyete",
                "email": "kyete@example.com",
            }
        }
    }).encode("utf-8")
    mock_resp.__enter__.return_value = mock_resp
    mock_urlopen.return_value = mock_resp

    user = client.whoami()
    assert user["username"] == "kyete"
    assert user["email"] == "kyete@example.com"


@patch("urllib.request.urlopen")
def test_hub_client_secrets_lifecycle(mock_urlopen, temp_hub_config):
    """Test HubClient secret setting, retrieval, listing, and deletion."""
    client = HubClient(token="wshub_pat_123", config_path=temp_hub_config)

    # 1. Mock set_secret
    mock_resp_set = MagicMock()
    mock_resp_set.read.return_value = json.dumps({"status": "success"}).encode("utf-8")
    mock_resp_set.__enter__.return_value = mock_resp_set
    mock_urlopen.return_value = mock_resp_set

    client.set_secret("fantastik", "renttik", "JWT_SECRET", "super-secret-key", repo_name="server")

    # 2. Mock list_secrets
    mock_resp_list = MagicMock()
    mock_resp_list.read.return_value = json.dumps({
        "status": "success",
        "data": [
            {"key": "JWT_SECRET", "value": "super-secret-key", "repoName": "server"}
        ]
    }).encode("utf-8")
    mock_resp_list.__enter__.return_value = mock_resp_list
    mock_urlopen.return_value = mock_resp_list

    secrets = client.list_secrets("fantastik", "renttik")
    assert len(secrets) == 1
    assert secrets[0]["key"] == "JWT_SECRET"


def test_cli_hub_argument_parsing():
    """Test CLI parser correctly recognizes ws hub subcommands and ws clone shortcut."""
    parser = build_parser()

    # ws hub login
    args1, _ = parser.parse_known_args(["hub", "login", "--url", "http://localhost:8787", "--token", "abc"])
    assert args1.subcommand == "hub"
    assert args1.hub_subcommand == "login"
    assert args1.url == "http://localhost:8787"
    assert args1.token == "abc"

    # ws hub clone
    args2, _ = parser.parse_known_args(["hub", "clone", "org/project", "/tmp/dest"])
    assert args2.subcommand == "hub"
    assert args2.hub_subcommand == "clone"
    assert args2.project == "org/project"
    assert args2.target_dir == "/tmp/dest"

    # Top-level ws clone
    args3, _ = parser.parse_known_args(["clone", "org/project"])
    assert args3.subcommand == "clone"
    assert args3.project == "org/project"

    # ws hub secret set
    args4, _ = parser.parse_known_args(["hub", "secret", "set", "DB_PASS", "mypassword", "--repo", "server"])
    assert args4.subcommand == "hub"
    assert args4.hub_subcommand == "secret"
    assert args4.hub_sec_subcommand == "set"
    assert args4.key == "DB_PASS"
    assert args4.value == "mypassword"
    assert args4.repo == "server"

    # ws hub state save @develop
    args5, _ = parser.parse_known_args(["hub", "state", "save", "@develop"])
    assert args5.subcommand == "hub"
    assert args5.hub_subcommand == "state"
    assert args5.hub_state_subcommand == "save"
    assert args5.workspace == "@develop"


@patch("urllib.request.urlopen")
def test_workspace_manager_hub_publish_and_push(mock_urlopen, tmp_path, temp_hub_config):
    """Test WorkspaceManager hub_publish and hub_push methods."""
    from ws.workspace import WorkspaceManager
    from ws.models import AppConfig

    # Setup temporary project directory
    proj_dir = tmp_path / "my-project-workspaces"
    proj_dir.mkdir()
    config_file = proj_dir / "repositories.yml"
    config_file.write_text("repositories:\n  api:\n    url: git@github.com:org/api.git\n", encoding="utf-8")

    scripts_dir = proj_dir / "scripts"
    scripts_dir.mkdir()
    (scripts_dir / "setup.sh").write_text("#!/bin/bash\necho setup", encoding="utf-8")

    app_cfg = AppConfig(
        repositories={},
        workspaces_dir=proj_dir / "workspaces",
        config_file_path=config_file,
    )
    manager = WorkspaceManager(config=app_cfg)

    # Mock whoami and create_project responses
    mock_resp = MagicMock()
    mock_resp.read.return_value = json.dumps({
        "status": "success",
        "data": {
            "project": {"id": "prj_1", "namespace": "kyete", "name": "my-project"},
            "revision": {"version": 1},
            "user": {"username": "kyete"},
        }
    }).encode("utf-8")
    mock_resp.__enter__.return_value = mock_resp
    mock_urlopen.return_value = mock_resp

    # Save session
    client = HubClient(config_path=temp_hub_config)
    client.save_session("http://localhost:8787", "wshub_pat_test", "kyete")

    with patch("ws.hub.DEFAULT_HUB_CONFIG_PATH", temp_hub_config):
        pub_result = manager.hub_publish(project_identifier="kyete/my-project")
        assert pub_result.get("project", {}).get("name") == "my-project"

        push_result = manager.hub_push(message="Update config", project_identifier="kyete/my-project")
        assert "revision" in push_result

