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


def test_prepare_and_sync_env_file_with_readonly_example(tmp_path):
    """Test that copying a read-only .env.example (0444) results in a writable .env file."""
    worktree = tmp_path / "worktree"
    worktree.mkdir()

    example_file = worktree / ".env.example"
    example_file.write_text("FOO=bar\n")
    # Mark example file read-only (0444)
    example_file.chmod(0o444)

    env_vars = {"FOO": "updated_val"}
    ok, msg = EnvEngine.prepare_and_sync_env_file(
        worktree_path=worktree,
        env_vars=env_vars,
        env_filename=".env",
        example_filename=".env.example",
    )

    assert ok is True
    target_env = worktree / ".env"
    assert target_env.exists()
    assert target_env.stat().st_mode & 0o200 != 0  # User write bit is set
    assert "FOO=updated_val" in target_env.read_text()


def test_prepare_and_sync_env_file_existing_readonly(tmp_path):
    """Test that updating an existing read-only .env file (0444) restores write permissions and succeeds."""
    worktree = tmp_path / "worktree"
    worktree.mkdir()

    target_env = worktree / ".env"
    target_env.write_text("OLD_KEY=old_val\n")
    target_env.chmod(0o444)

    env_vars = {"OLD_KEY": "new_val", "NEW_KEY": "added"}
    ok, msg = EnvEngine.prepare_and_sync_env_file(
        worktree_path=worktree,
        env_vars=env_vars,
        env_filename=".env",
    )

    assert ok is True
    assert target_env.stat().st_mode & 0o200 != 0
    content = target_env.read_text()
    assert "OLD_KEY=new_val" in content
    assert "NEW_KEY=added" in content



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


def test_service_discovery_template_placeholders():
    """Test multi-network and cross-service discovery template substitutions."""
    service_ports = {"server": 8090, "mobile": 8091}
    lan_ip = "192.168.1.50"
    public_host = "app.example.com"

    template = (
        "API=${SERVICE_URL:server} LAN=${SERVICE_URL_LAN:server} "
        "PUB=${SERVICE_URL_PUBLIC:server} PORT=${SERVICE_PORT:mobile}"
    )

    res = EnvEngine.resolve_template_string(
        template_str=template,
        workspace_name="feat-auth",
        repo_name="mobile",
        slot=1,
        service_ports=service_ports,
        lan_ip=lan_ip,
        public_host=public_host,
    )

    assert "API=http://127.0.0.1:8090" in res
    assert "LAN=http://192.168.1.50:8090" in res
    assert "PUB=http://app.example.com:8090" in res
    assert "PORT=8091" in res

    # When public_host is unset, it should gracefully fall back to lan_ip
    res_no_pub = EnvEngine.resolve_template_string(
        template_str="PUB=${SERVICE_URL_PUBLIC:server}",
        workspace_name="feat-auth",
        repo_name="mobile",
        slot=1,
        service_ports=service_ports,
        lan_ip=lan_ip,
        public_host=None,
    )
    assert f"PUB=http://{lan_ip}:8090" in res_no_pub



def test_service_discovery_descriptor_and_env_injection(tmp_path):
    """Test writing and reading .ws/services.json and .ws/services.env descriptors."""
    ws_dir = tmp_path / "workspaces" / "develop"
    ws_dir.mkdir(parents=True)

    service_ports = {"server": 8080, "web": 3000}
    json_path = EnvEngine.write_service_discovery_files(
        workspace_dir=ws_dir,
        workspace_name="develop",
        slot=0,
        service_ports=service_ports,
        public_host="develop.tunnel.org",
    )

    assert json_path.exists()
    descriptor = EnvEngine.read_service_discovery_descriptor(ws_dir)
    assert descriptor is not None
    assert descriptor["workspace"] == "develop"
    assert descriptor["slot"] == 0
    assert descriptor["services"]["server"]["port"] == 8080
    assert descriptor["services"]["web"]["port"] == 3000

    env_path = ws_dir / ".ws" / "services.env"
    assert env_path.exists()
    env_content = env_path.read_text()
    assert "WS_SERVICE_SERVER_PORT=8080" in env_content
    assert "WS_SERVICE_WEB_PORT=3000" in env_content


def test_network_port_allocation_and_collision_auto_healing(tmp_path):
    """Test dynamic socket probing and collision auto-healing in ws/network.py."""
    import socket
    from ws.models import RepoConfig
    from ws.network import allocate_workspace_ports, is_port_available

    # Bind a real TCP socket to an available ephemeral port to simulate a collision
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("0.0.0.0", 0))
    busy_port = sock.getsockname()[1]
    sock.listen(1)

    try:
        repos = {
            "server": RepoConfig(name="server", bare=tmp_path / "s.git", checkout="s", port=busy_port),
            "web": RepoConfig(name="web", bare=tmp_path / "w.git", checkout="w", port=busy_port + 10),
        }

        # Preferred server port is busy_port (which is occupied by sock)
        allocated, shifted = allocate_workspace_ports(repos, slot=0, recorded_leases={"server": busy_port})
        # Server port must auto-heal to a different free port
        assert allocated["server"] != busy_port
        assert shifted is True
    finally:
        sock.close()


def test_network_interface_discovery_and_wireless_priority(monkeypatch):
    """Test get_lan_ip prioritizes Wi-Fi adapter and supports explicit interface/ip selection."""
    from ws.network import get_lan_ip, list_network_interfaces, is_wireless_interface, is_ethernet_interface

    assert is_wireless_interface("wlan0") is True
    assert is_wireless_interface("wlp2s0") is True
    assert is_wireless_interface("wifi0") is True
    assert is_wireless_interface("eno1") is False

    assert is_ethernet_interface("eno1") is True
    assert is_ethernet_interface("eth0") is True
    assert is_ethernet_interface("enp3s0") is True
    assert is_ethernet_interface("wlan0") is False

    # Test explicit IP override
    assert get_lan_ip(explicit_ip="192.168.1.100") == "192.168.1.100"

    # Test mocking list_network_interfaces
    mock_interfaces = [
        {"name": "eno1", "ip": "10.0.0.1", "type": "ethernet", "is_wireless": False},
        {"name": "wlan0", "ip": "192.168.24.178", "type": "wireless", "is_wireless": True},
    ]
    monkeypatch.setattr("ws.network.list_network_interfaces", lambda: mock_interfaces)

    # 1. Default should pick wireless adapter (wlan0 -> 192.168.24.178)
    assert get_lan_ip() == "192.168.24.178"

    # 2. Preferred interface 'eno1' should pick ethernet
    assert get_lan_ip(preferred_interface="eno1") == "10.0.0.1"

    # 3. Preferred interface 'ethernet' / 'eth' should pick ethernet
    assert get_lan_ip(preferred_interface="ethernet") == "10.0.0.1"

    # 4. Preferred interface 'wifi' / 'wireless' should pick wifi
    assert get_lan_ip(preferred_interface="wifi") == "192.168.24.178"

    # 5. Environment variable override
    monkeypatch.setenv("WS_LAN_IP", "172.20.10.5")
    assert get_lan_ip() == "172.20.10.5"


def test_resolve_repo_env_interface_and_ip_override(tmp_path):
    """Test EnvEngine.resolve_repo_env with interface and lan_ip parameters."""
    app_cfg = AppConfig(
        repositories={
            "mobile": RepoConfig(
                name="mobile",
                bare=tmp_path / "mobile.git",
                checkout="mobile",
                env={
                    "API_URL": "${SERVICE_URL_LAN:server}/api",
                },
            ),
            "server": RepoConfig(
                name="server",
                bare=tmp_path / "server.git",
                checkout="server",
                port=4000,
            ),
        },
        workspaces_dir=tmp_path / "workspaces",
    )

    # With explicit IP override
    resolved_ip = EnvEngine.resolve_repo_env(
        app_cfg,
        "feature-test",
        "mobile",
        slot=1,
        lan_ip="192.168.50.200",
    )
    assert resolved_ip["WS_LAN_IP"] == "192.168.50.200"
    assert resolved_ip["API_URL"] == "http://192.168.50.200:4010/api"
    assert resolved_ip["WS_SERVICE_SERVER_URL_LAN"] == "http://192.168.50.200:4010"





