"""Unit tests for environment variable resolution and .env synchronization engine."""

from pathlib import Path
import pytest

from ws.env import EnvEngine
from ws.models import AppConfig, RepoConfig


def test_resolve_template_string():
    """Test dynamic placeholder substitutions."""
    # Standard placeholders
    res = EnvEngine.resolve_template_string(
        template_str="db_${WORKSPACE_NAME}_${REPO_NAME}_slot${WORKSPACE_SLOT}",
        workspace_name="auth-flow",
        repo_name="server",
        slot=2,
    )
    assert res == "db_auth-flow_server_slot2"

    # Port allocation: base + slot * 10
    port_res = EnvEngine.resolve_template_string(
        template_str="${PORT:3000}",
        workspace_name="auth-flow",
        repo_name="web",
        slot=3,
    )
    assert port_res == "3030"

    # Custom offset multiplier
    offset_res = EnvEngine.resolve_template_string(
        template_str="${PORT_OFFSET:8000:5}",
        workspace_name="auth-flow",
        repo_name="server",
        slot=3,
    )
    assert offset_res == "8015"


def test_resolve_repo_env_scoping_and_overrides(tmp_path):
    """Test merging global store with repo-scoped overrides."""
    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(
                name="server",
                bare=tmp_path / "server.git",
                checkout="server",
                env={
                    "PORT": "${PORT:4000}",
                    "DB_NAME": "renttik_${WORKSPACE_NAME}_backend",
                },
            ),
            "web": RepoConfig(
                name="web",
                bare=tmp_path / "web.git",
                checkout="web",
                env={
                    "PORT": "${PORT:3000}",
                    "VITE_API_URL": "http://localhost:4000",
                },
            ),
        },
        workspaces_dir=tmp_path / "workspaces",
        global_env={
            "NODE_ENV": "development",
            "JWT_SECRET": "secret123",
            "DB_NAME": "renttik_${WORKSPACE_NAME}",
        },
        dynamic_env={
            "GLOBAL_APP": "${WORKSPACE_NAME}_app",
        },
    )

    # Server env at slot 1
    server_env = EnvEngine.resolve_repo_env(app_cfg, "auth-flow", "server", slot=1)
    assert server_env["NODE_ENV"] == "development"
    assert server_env["JWT_SECRET"] == "secret123"
    assert server_env["GLOBAL_APP"] == "auth-flow_app"
    assert server_env["PORT"] == "4010"  # 4000 + 1 * 10
    assert server_env["DB_NAME"] == "renttik_auth-flow_backend"  # Scoped override

    # Web env at slot 1
    web_env = EnvEngine.resolve_repo_env(app_cfg, "auth-flow", "web", slot=1)
    assert web_env["NODE_ENV"] == "development"
    assert web_env["PORT"] == "3010"  # 3000 + 1 * 10
    assert web_env["VITE_API_URL"] == "http://localhost:4000"
    assert web_env["DB_NAME"] == "renttik_auth-flow"  # Inherited from global


def test_prepare_and_sync_env_file_copy_example(tmp_path):
    """Test Step 1: Copy .env.example if .env is missing."""
    worktree = tmp_path / "worktree"
    worktree.mkdir()

    example_file = worktree / ".env.example"
    example_file.write_text("FOO=bar\nBAZ=qux\n")

    env_vars = {"FOO": "new_bar", "CUSTOM_KEY": "custom_val"}
    ok, msg = EnvEngine.prepare_and_sync_env_file(
        worktree_path=worktree,
        env_vars=env_vars,
        env_filename=".env",
        example_filename=".env.example",
    )

    assert ok is True
    target_env = worktree / ".env"
    assert target_env.exists()
    content = target_env.read_text()
    assert "FOO=new_bar" in content
    assert "BAZ=qux" in content  # Preserved from example
    assert "CUSTOM_KEY=custom_val" in content  # Appended


def test_expand_command():
    """Test inline dynamic variable expansion in setup script commands."""
    env_vars = {
        "DB_NAME": "renttik_auth-flow",
        "PORT": "4010",
    }
    cmd = "createdb -h localhost ${DB_NAME} --port ${PORT}"
    expanded = EnvEngine.expand_command(cmd, env_vars, "auth-flow", "server", slot=1)
    assert expanded == "createdb -h localhost renttik_auth-flow --port 4010"

    # Template placeholder within command
    cmd2 = "echo ${WORKSPACE_NAME}_db on ${PORT:3000}"
    expanded2 = EnvEngine.expand_command(cmd2, env_vars, "auth-flow", "web", slot=1)
    assert expanded2 == "echo auth-flow_db on 3010"


def test_secret_masking():
    """Test heuristic and explicit secret detection and masking."""
    assert EnvEngine.is_secret_key("JWT_SECRET") is True
    assert EnvEngine.is_secret_key("DB_PASSWORD") is True
    assert EnvEngine.is_secret_key("STRIPE_API_KEY") is True
    assert EnvEngine.is_secret_key("AUTH_TOKEN") is True
    assert EnvEngine.is_secret_key("PORT") is False
    assert EnvEngine.is_secret_key("DB_NAME") is False

    # Explicit secret list
    assert EnvEngine.is_secret_key("CUSTOM_VAL", explicit_secrets=["CUSTOM_VAL"]) is True
    assert EnvEngine.mask_secret_value("my_super_secret_val") == "********"


def test_sync_copied_files(tmp_path):
    """Test copying configuration files directly into worktrees."""
    project_root = tmp_path / "project"
    worktree = tmp_path / "worktree"
    project_root.mkdir()
    worktree.mkdir()

    # Source files
    config_dir = project_root / "config"
    config_dir.mkdir()
    (config_dir / "firebase.json").write_text('{"app": "renttik"}')
    (project_root / "schema.prisma").write_text("// prisma schema")

    copy_spec = [
        "config/firebase.json",
        {"source": "schema.prisma", "dest": "prisma/schema.prisma"},
    ]

    ok, msg = EnvEngine.sync_copied_files(project_root, worktree, copy_spec)
    assert ok is True
    assert (worktree / "config" / "firebase.json").exists()
    assert (worktree / "config" / "firebase.json").read_text() == '{"app": "renttik"}'
    assert (worktree / "prisma" / "schema.prisma").exists()
    assert (worktree / "prisma" / "schema.prisma").read_text() == "// prisma schema"


def test_sync_copied_files_from_files_directory(tmp_path):
    """Test copying file from project files/ directory with relative syntax like storage/app/pawapay-private.pem."""
    project_root = tmp_path / "project"
    worktree = tmp_path / "worktree"
    project_root.mkdir()
    worktree.mkdir()

    # Create project_root/files/storage/app/pawapay-private.pem
    files_dir = project_root / "files"
    storage_app_dir = files_dir / "storage" / "app"
    storage_app_dir.mkdir(parents=True)
    pem_file = storage_app_dir / "pawapay-private.pem"
    pem_file.write_text("-----BEGIN RSA PRIVATE KEY-----\nsecret_key\n-----END RSA PRIVATE KEY-----\n")

    copy_spec = ["storage/app/pawapay-private.pem"]

    ok, msg = EnvEngine.sync_copied_files(project_root, worktree, copy_spec)
    assert ok is True

    # Verify destination worktree has storage/app/pawapay-private.pem
    dest_pem = worktree / "storage" / "app" / "pawapay-private.pem"
    assert dest_pem.exists()
    assert "BEGIN RSA PRIVATE KEY" in dest_pem.read_text()



def test_global_script_path_expansion(tmp_path):
    """Test expanding ${PROJECT_ROOT} and ${SCRIPTS_DIR}."""
    project_root = tmp_path / "root"
    workspaces_dir = tmp_path / "workspaces"
    project_root.mkdir()
    workspaces_dir.mkdir()

    scripts_dir = project_root / "scripts"
    scripts_dir.mkdir()
    init_script = scripts_dir / "init_db.sh"
    init_script.write_text("#!/bin/bash\n")

    cmd = "${SCRIPTS_DIR}/init_db.sh ${WORKSPACE_NAME}"
    expanded = EnvEngine.expand_command(
        cmd,
        {},
        "auth-flow",
        "server",
        project_root=project_root,
        workspaces_dir=workspaces_dir,
    )
    assert str(init_script.resolve()) in expanded
    assert "auth-flow" in expanded


