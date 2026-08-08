"""Unit tests for workspace setup pipeline and project launcher."""

from pathlib import Path
import pytest

from ws.models import AppConfig, RepoConfig, RepoSpec
from ws.workspace import WorkspaceManager


def test_setup_workspace_execution_pipeline(tmp_path, monkeypatch):
    """Test 3-step setup: .env.example copy, env resolution, and setup command execution."""
    bare1 = tmp_path / "server.git"
    bare1.mkdir()

    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(
                name="server",
                bare=bare1,
                checkout="server",
                env={"PORT": "${PORT:8000}", "DB_NAME": "test_${WORKSPACE_NAME}"},
                setup=["echo setup_done > setup.log"],
                launch="python -m app",
            ),
        },
        workspaces_dir=tmp_path / "workspaces",
        global_env={"GLOBAL_KEY": "global_val"},
    )

    manager = WorkspaceManager(config=app_cfg)
    monkeypatch.setattr(manager.git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(manager.git, "branch_exists", lambda bare, br: False)
    monkeypatch.setattr(
        manager.git,
        "create_worktree",
        lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True),
    )

    specs = [RepoSpec(name="server", branch="feature/test", create=True, path="server")]
    manager.create_workspace("my-ws", specs)

    # Place .env.example in the worktree
    server_wt = tmp_path / "workspaces" / "my-ws" / "server"
    example_env = server_wt / ".env.example"
    example_env.write_text("BASE_SETTING=true\n")

    # Run setup
    results = manager.setup_workspace("my-ws")

    assert results["server"]["status"] == "completed"

    # Verify .env generated
    env_file = server_wt / ".env"
    assert env_file.exists()
    env_content = env_file.read_text()
    assert "BASE_SETTING=true" in env_content
    assert "GLOBAL_KEY=global_val" in env_content
    assert "DB_NAME=test_my-ws" in env_content

    # Verify setup command executed
    log_file = server_wt / "setup.log"
    assert log_file.exists()
    assert "setup_done" in log_file.read_text()


def test_launch_workspace_commands(tmp_path, monkeypatch):
    """Test retrieving launch commands for workspace services."""
    bare1 = tmp_path / "server.git"
    bare2 = tmp_path / "web.git"
    bare1.mkdir()
    bare2.mkdir()

    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(
                name="server",
                bare=bare1,
                checkout="server",
                launch="python server.py",
            ),
            "web": RepoConfig(
                name="web",
                bare=bare2,
                checkout="web",
                launch="npm run dev",
            ),
        },
        workspaces_dir=tmp_path / "workspaces",
    )

    manager = WorkspaceManager(config=app_cfg)
    monkeypatch.setattr(manager.git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(manager.git, "branch_exists", lambda bare, br: True)
    monkeypatch.setattr(
        manager.git,
        "create_worktree",
        lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True),
    )

    specs = [
        RepoSpec(name="server", branch="main", create=False, path="server"),
        RepoSpec(name="web", branch="main", create=False, path="web"),
    ]
    manager.create_workspace("app-ws", specs)

    entries = manager.launch_workspace("app-ws")
    assert len(entries) == 2
    repo_names = [e[0] for e in entries]
    launch_cmds = [e[2] for e in entries]
    assert "server" in repo_names
    assert "web" in repo_names
    assert "python server.py" in launch_cmds
    assert "npm run dev" in launch_cmds


def test_setup_workspace_subset_selection(tmp_path, monkeypatch):
    """Test running setup on a specific subset of repositories."""
    bare1 = tmp_path / "server.git"
    bare2 = tmp_path / "web.git"
    bare1.mkdir()
    bare2.mkdir()

    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(
                name="server",
                bare=bare1,
                checkout="server",
                setup=["touch server_setup.txt"],
            ),
            "web": RepoConfig(
                name="web",
                bare=bare2,
                checkout="web",
                setup=["touch web_setup.txt"],
            ),
        },
        workspaces_dir=tmp_path / "workspaces",
    )

    manager = WorkspaceManager(config=app_cfg)
    monkeypatch.setattr(manager.git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(manager.git, "branch_exists", lambda bare, br: True)
    monkeypatch.setattr(
        manager.git,
        "create_worktree",
        lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True),
    )

    specs = [
        RepoSpec(name="server", branch="main", create=False, path="server"),
        RepoSpec(name="web", branch="main", create=False, path="web"),
    ]
    manager.create_workspace("subset-ws", specs)

    # Setup ONLY server
    results = manager.setup_workspace("subset-ws", repos=["server"])
    assert "server" in results
    assert "web" not in results
    assert results["server"]["status"] == "completed"

    server_wt = tmp_path / "workspaces" / "subset-ws" / "server"
    web_wt = tmp_path / "workspaces" / "subset-ws" / "web"
    assert (server_wt / "server_setup.txt").exists()
    assert not (web_wt / "web_setup.txt").exists()


def test_setup_workspace_dynamic_command_expansion(tmp_path, monkeypatch):
    """Test dynamic variables in setup commands like ${DB_NAME} and top-level setup."""
    bare1 = tmp_path / "server.git"
    bare1.mkdir()

    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(
                name="server",
                bare=bare1,
                checkout="server",
                env={"DB_NAME": "db_${WORKSPACE_NAME}"},
                setup=["echo creating_${DB_NAME} > db.log"],
            ),
        },
        workspaces_dir=tmp_path / "workspaces",
        setup=["echo global_setup_for_${WORKSPACE_NAME} > global.log"],
    )

    manager = WorkspaceManager(config=app_cfg)
    monkeypatch.setattr(manager.git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(manager.git, "branch_exists", lambda bare, br: True)
    monkeypatch.setattr(
        manager.git,
        "create_worktree",
        lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True),
    )

    specs = [RepoSpec(name="server", branch="main", create=False, path="server")]
    manager.create_workspace("dyn-ws", specs)

    # Run full setup
    results = manager.setup_workspace("dyn-ws")

    assert results["server"]["status"] == "completed"

    ws_root = tmp_path / "workspaces" / "dyn-ws"
    global_log = ws_root / "global.log"
    assert global_log.exists()
    assert "global_setup_for_dyn-ws" in global_log.read_text()

    server_wt = ws_root / "server"
    db_log = server_wt / "db.log"
    assert db_log.exists()
    assert "creating_db_dyn-ws" in db_log.read_text()


def test_setup_workspace_file_copying_and_global_scripts(tmp_path, monkeypatch):
    """Test copying project-level configuration files into worktrees during setup."""
    bare1 = tmp_path / "server.git"
    bare1.mkdir()

    # Create a project root with shared configs and a global script
    project_root = tmp_path / "project"
    project_root.mkdir()
    shared_dir = project_root / "shared"
    shared_dir.mkdir()
    (shared_dir / "firebase.json").write_text('{"apiKey": "xyz"}')

    scripts_dir = project_root / "scripts"
    scripts_dir.mkdir()
    global_script = scripts_dir / "seed.sh"
    global_script.write_text("#!/bin/bash\necho seeded > seed.txt\n")
    global_script.chmod(0o755)

    app_cfg = AppConfig(
        repositories={
            "server": RepoConfig(
                name="server",
                bare=bare1,
                checkout="server",
                copy_files=["shared/firebase.json"],
                setup=["bash ${SCRIPTS_DIR}/seed.sh"],
            ),
        },
        workspaces_dir=project_root / "workspaces",
        config_file_path=project_root / "repositories.yml",
    )

    manager = WorkspaceManager(config=app_cfg)
    monkeypatch.setattr(manager.git, "is_bare_repo", lambda path: True)
    monkeypatch.setattr(manager.git, "branch_exists", lambda bare, br: True)
    monkeypatch.setattr(
        manager.git,
        "create_worktree",
        lambda bare_path, worktree_path, branch, create_branch: worktree_path.mkdir(parents=True, exist_ok=True),
    )

    specs = [RepoSpec(name="server", branch="main", create=False, path="server")]
    manager.create_workspace("copy-ws", specs)

    # Run setup
    results = manager.setup_workspace("copy-ws")
    assert results["server"]["status"] == "completed"

    server_wt = project_root / "workspaces" / "copy-ws" / "server"
    # Verify file was copied
    assert (server_wt / "shared" / "firebase.json").exists()
    assert (server_wt / "shared" / "firebase.json").read_text() == '{"apiKey": "xyz"}'
    # Verify global script ran in the worktree
    assert (server_wt / "seed.txt").exists()
    assert "seeded" in (server_wt / "seed.txt").read_text()



