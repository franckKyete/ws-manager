"""Tests for Environment Variable Tiers (public, secret, private), Vault Packaging, and Sensitive Files Sync."""

from pathlib import Path
from unittest.mock import patch, MagicMock
import pytest
import yaml

from ws.config import ConfigLoader
from ws.env import EnvEngine
from ws.models import (
    AppConfig,
    RepoConfig,
    clean_env_val,
    is_private_val,
    is_secret_val,
)
from ws.workspace import WorkspaceManager
from ws.hub import HubClient


def test_env_classification_helpers():
    """Test prefix checkers and value sanitization helpers."""
    assert is_secret_val("secret:my-token") is True
    assert is_secret_val("vault:my-token") is True
    assert is_secret_val("plain-token") is False
    assert is_secret_val(123) is False

    assert is_private_val("private:/opt/local") is True
    assert is_private_val("local:192.168.1.5") is True
    assert is_private_val("public:standard") is False
    assert is_private_val("normal_val") is False

    assert clean_env_val("secret:sk_test_123") == "sk_test_123"
    assert clean_env_val("vault:token_abc") == "token_abc"
    assert clean_env_val("private:/path/dir") == "/path/dir"
    assert clean_env_val("local:127.0.0.1") == "127.0.0.1"
    assert clean_env_val("public:my_val") == "my_val"
    assert clean_env_val("ordinary") == "ordinary"


def test_config_loader_parses_three_tiers(tmp_path):
    """Test ConfigLoader parses public, secret, and private blocks and inline prefixes."""
    config_file = tmp_path / "repositories.yml"
    raw_yaml = """
env:
  NODE_ENV: development
  GLOBAL_API_KEY: secret:global_key_123
  LOCAL_CACHE_DIR: private:/tmp/cache

secret:
  JWT_SIGNING_KEY: super_jwt_secret

private:
  DEVELOPER_MACHINE_ID: macbook_pro_01

repositories:
  server:
    bare: bares/server.git
    checkout: server
    env:
      PORT: "8080"
      DB_PASSWORD: secret:pg_secret_pass
      LOCAL_DEBUG_IP: private:192.168.1.99
    secret:
      STRIPE_SECRET: sk_live_9999
    private:
      LOCAL_LOG_PATH: /var/log/custom.log
"""
    config_file.write_text(raw_yaml, encoding="utf-8")

    app_cfg = ConfigLoader.load_config(config_path=config_file)

    # 1. Global assertions
    assert app_cfg.global_env == {"NODE_ENV": "development"}
    assert app_cfg.secret_env == {
        "GLOBAL_API_KEY": "global_key_123",
        "JWT_SIGNING_KEY": "super_jwt_secret",
    }
    assert app_cfg.private_env == {
        "LOCAL_CACHE_DIR": "/tmp/cache",
        "DEVELOPER_MACHINE_ID": "macbook_pro_01",
    }

    # 2. Repo-level assertions
    server_cfg = app_cfg.repositories["server"]
    assert server_cfg.env == {"PORT": "8080"}
    assert server_cfg.secret_env == {
        "DB_PASSWORD": "pg_secret_pass",
        "STRIPE_SECRET": "sk_live_9999",
    }
    assert server_cfg.private_env == {
        "LOCAL_DEBUG_IP": "192.168.1.99",
        "LOCAL_LOG_PATH": "/var/log/custom.log",
    }


def test_classify_project_assets(tmp_path):
    """Test asset classification generates sanitized blueprint, extracted secrets, and files."""
    proj_dir = tmp_path / "test-project"
    proj_dir.mkdir()
    config_file = proj_dir / "repositories.yml"

    # Create dummy sensitive files
    files_dir = proj_dir / "files"
    files_dir.mkdir()
    cert_file = files_dir / "service-account.json"
    cert_file.write_text('{"type": "service_account"}', encoding="utf-8")

    raw_yaml = """
env:
  PUBLIC_VAR: hello
  TOP_SECRET: secret:shhh_token
  LOCAL_VAR: private:my_private_path

repositories:
  api:
    bare: bares/api.git
    checkout: api
    env:
      PORT: "3000"
      API_SECRET: secret:secret_api_key
      DEV_IP: private:127.0.0.1
    copy_files:
      - from: files/service-account.json
        to: credentials.json
"""
    config_file.write_text(raw_yaml, encoding="utf-8")

    app_cfg = ConfigLoader.load_config(config_path=config_file)
    sanitized_yaml, extracted_secrets, files_to_upload, private_count = ConfigLoader.classify_project_assets(app_cfg)

    # Verify sanitized blueprint
    parsed_blueprint = yaml.safe_load(sanitized_yaml)
    assert parsed_blueprint["env"]["PUBLIC_VAR"] == "hello"
    assert parsed_blueprint["env"]["TOP_SECRET"] == "secret"
    assert "LOCAL_VAR" not in parsed_blueprint["env"]

    api_env = parsed_blueprint["repositories"]["api"]["env"]
    assert api_env["PORT"] == "3000"
    assert api_env["API_SECRET"] == "secret"
    assert "DEV_IP" not in api_env

    # Verify extracted secrets
    assert extracted_secrets["global"] == {"TOP_SECRET": "shhh_token"}
    assert extracted_secrets["api"] == {"API_SECRET": "secret_api_key"}

    # Verify files
    assert cert_file in files_to_upload
    assert private_count == 2


def test_env_engine_runtime_injection(tmp_path):
    """Test EnvEngine merges public, secret, and private variables for runtime worktree execution."""
    config_file = tmp_path / "repositories.yml"
    raw_yaml = """
env:
  GLOBAL_PUB: global_value
  GLOBAL_SEC: secret:secret_global

repositories:
  server:
    bare: bares/server.git
    checkout: server
    env:
      REPO_PUB: repo_value
      REPO_SEC: secret:secret_repo
      REPO_PRIV: private:local_private_val
"""
    config_file.write_text(raw_yaml, encoding="utf-8")

    app_cfg = ConfigLoader.load_config(config_path=config_file)
    merged = EnvEngine.resolve_repo_env(
        app_config=app_cfg,
        workspace_name="develop",
        repo_name="server",
        slot=0,
    )

    assert merged["GLOBAL_PUB"] == "global_value"
    assert merged["GLOBAL_SEC"] == "secret_global"
    assert merged["REPO_PUB"] == "repo_value"
    assert merged["REPO_SEC"] == "secret_repo"
    assert merged["REPO_PRIV"] == "local_private_val"


@patch("ws.hub.HubClient.create_project")
@patch("ws.hub.HubClient.set_secrets_bulk")
@patch("ws.hub.HubClient.upload_file")
def test_hub_publish_syncs_secrets_and_files(
    mock_upload_file,
    mock_set_secrets_bulk,
    mock_create_project,
    tmp_path,
):
    """Test manager.hub_publish strips private vars, stores secrets in vault, and uploads files."""
    proj_dir = tmp_path / "renttik-ws"
    proj_dir.mkdir()
    config_file = proj_dir / "repositories.yml"

    files_dir = proj_dir / "files"
    files_dir.mkdir()
    secret_pem = files_dir / "private.pem"
    secret_pem.write_text("-----BEGIN RSA PRIVATE KEY-----", encoding="utf-8")

    raw_yaml = """
env:
  NODE_ENV: production
  GLOBAL_JWT: secret:super_secret_jwt
  LOCAL_TEMP: private:/tmp/local-only

repositories:
  server:
    bare: bares/server.git
    checkout: server
    env:
      PORT: "8080"
      DB_PASSWORD: secret:my_db_pass
      DEBUG_PORT: private:9229
"""
    config_file.write_text(raw_yaml, encoding="utf-8")

    app_cfg = ConfigLoader.load_config(config_path=config_file)
    manager = WorkspaceManager(config=app_cfg)

    mock_create_project.return_value = {"project": {"name": "renttik-ws"}}

    with patch("ws.hub.HubClient.whoami", return_value={"username": "kyete"}):
        result = manager.hub_publish(project_identifier="kyete/renttik-ws")

    # 1. Verify create_project received sanitized blueprint
    mock_create_project.assert_called_once()
    called_yaml = mock_create_project.call_args.kwargs["blueprint_yaml"]
    parsed_sent = yaml.safe_load(called_yaml)
    assert parsed_sent["env"]["NODE_ENV"] == "production"
    assert parsed_sent["env"]["GLOBAL_JWT"] == "secret"
    assert "LOCAL_TEMP" not in parsed_sent["env"]
    assert "DEBUG_PORT" not in parsed_sent["repositories"]["server"]["env"]

    # 2. Verify set_secrets_bulk was called for global and server secrets
    assert mock_set_secrets_bulk.call_count == 2

    # 3. Verify file upload was called for private.pem
    mock_upload_file.assert_called_once()
