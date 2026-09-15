"""Network discovery, LAN IP resolution, and dynamic port allocation utilities."""

import logging
from pathlib import Path
import socket
from typing import Any, Mapping, Sequence


logger = logging.getLogger("ws.network")


def _is_virtual_docker_ip(ip: str) -> bool:
    """Check if IP address belongs to common Docker / container virtual bridge ranges."""
    if ip.startswith(
        (
            "172.17.",
            "172.18.",
            "172.19.",
            "172.20.",
            "172.21.",
            "172.22.",
            "172.23.",
            "172.24.",
            "172.25.",
            "172.26.",
            "172.27.",
            "172.28.",
            "172.29.",
            "172.30.",
            "172.31.",
        )
    ):
        return True
    return False


def is_valid_ipv4(ip: str) -> bool:
    """Validate if string is a valid non-loopback IPv4 address."""
    parts = ip.split(".")
    if len(parts) != 4:
        return False
    try:
        return all(0 <= int(p) <= 255 for p in parts)
    except ValueError:
        return False


def is_wireless_interface(iface: str) -> bool:
    """Determine if a network interface is a wireless/Wi-Fi adapter."""
    # 1. Linux sysfs check
    sysfs_paths = [
        Path(f"/sys/class/net/{iface}/wireless"),
        Path(f"/sys/class/net/{iface}/phy80211"),
    ]
    for p in sysfs_paths:
        try:
            if p.exists():
                return True
        except Exception:
            pass

    # 2. Standard wireless interface naming prefixes
    if iface.startswith(("wl", "wlan", "wifi", "ath", "ra")):
        return True

    return False


def is_ethernet_interface(iface: str) -> bool:
    """Determine if a network interface is a wired Ethernet adapter."""
    if is_wireless_interface(iface):
        return False
    return iface.startswith(("eth", "en", "em"))


def list_network_interfaces() -> list[dict[str, Any]]:
    """List non-loopback active network interfaces with their IPv4 addresses, interface type, and status."""
    interfaces: list[dict[str, Any]] = []
    seen_ifaces: set[str] = set()

    # 1. Inspect active network interfaces via 'ip -4 -o addr show' on Linux
    try:
        import subprocess

        res = subprocess.run(
            ["ip", "-4", "-o", "addr", "show"],
            capture_output=True,
            text=True,
            check=False,
        )
        for line in res.stdout.splitlines():
            parts = line.split()
            if len(parts) >= 4:
                iface = parts[1]
                ip_with_mask = parts[3]
                ip = ip_with_mask.split("/")[0]

                if ip.startswith("127.") or ip.startswith("169.254.") or _is_virtual_docker_ip(ip):
                    continue
                if iface.startswith(("docker", "br-", "veth", "virbr", "tun", "tap", "lo", "dummy")):
                    continue

                if iface in seen_ifaces:
                    continue
                seen_ifaces.add(iface)

                is_wl = is_wireless_interface(iface)
                is_eth = is_ethernet_interface(iface)
                iface_type = "wireless" if is_wl else ("ethernet" if is_eth else "other")

                interfaces.append({
                    "name": iface,
                    "ip": ip,
                    "type": iface_type,
                    "is_wireless": is_wl,
                })
    except Exception as e:
        logger.debug("Failed discovering network interfaces via ip addr: %s", e)

    return interfaces


def get_lan_ip(preferred_interface: str | None = None, explicit_ip: str | None = None) -> str:
    """Discover the host's primary non-loopback local area network (LAN) IP address.

    Prioritizes physical wireless (Wi-Fi) network interfaces (e.g. wlan0, wlp2s0) by default
    so that physical mobile devices on the same Wi-Fi network can discover and connect to backend services.
    Falls back to Ethernet (eno1, eth0, enp*), then other non-virtual adapters.

    Args:
        preferred_interface: Optional interface name (e.g. 'wlan0', 'eno1') or category ('wifi', 'wireless', 'ethernet', 'eth').
        explicit_ip: Optional direct IP address override (e.g. '192.168.24.178').
    """
    import os

    # 1. Direct explicit IP argument
    if explicit_ip and is_valid_ipv4(explicit_ip) and not explicit_ip.startswith("127."):
        return explicit_ip

    # If preferred_interface was passed an IP directly (e.g. --interface 192.168.24.178)
    if preferred_interface and is_valid_ipv4(preferred_interface) and not preferred_interface.startswith("127."):
        return preferred_interface

    # 2. Check explicit environment override
    env_ip = os.environ.get("WS_LAN_IP") or os.environ.get("LAN_IP")
    if env_ip and is_valid_ipv4(env_ip) and not env_ip.startswith("127."):
        return env_ip

    pref = preferred_interface or os.environ.get("WS_INTERFACE") or os.environ.get("WS_IFACE")
    pref_norm = pref.strip().lower() if pref else None

    # 3. Discover active network interfaces
    interfaces = list_network_interfaces()

    if interfaces:
        # If user explicitly requested an interface name or type
        if pref_norm:
            if pref_norm in ("wifi", "wireless", "wlan"):
                for item in interfaces:
                    if item["is_wireless"]:
                        return item["ip"]
                logger.warning("No active wireless interface found; falling back to default network interface.")
            elif pref_norm in ("ethernet", "eth", "wired", "lan"):
                for item in interfaces:
                    if item["type"] == "ethernet":
                        return item["ip"]
                logger.warning("No active ethernet interface found; falling back to default network interface.")
            else:
                for item in interfaces:
                    if item["name"].lower() == pref_norm:
                        return item["ip"]
                logger.warning("Specified interface '%s' not found or has no active IPv4; falling back to default.", pref)

        # Default hierarchy:
        # 1. Wireless interfaces first (Wi-Fi)
        for item in interfaces:
            if item["is_wireless"]:
                return item["ip"]

        # 2. Ethernet interfaces second
        for item in interfaces:
            if item["type"] == "ethernet":
                return item["ip"]

        # 3. Any other non-virtual interface
        return interfaces[0]["ip"]

    # 4. Outbound UDP socket probe towards public DNS (does not send data)
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
        if ip and not ip.startswith("127.") and not _is_virtual_docker_ip(ip):
            return ip
    except Exception as e:
        logger.debug("Failed discovering LAN IP via UDP probe: %s", e)
    finally:
        s.close()

    # 5. Hostname resolution fallback
    try:
        hostname = socket.gethostname()
        ip = socket.gethostbyname(hostname)
        if ip and not ip.startswith("127.") and not _is_virtual_docker_ip(ip):
            return ip
    except Exception as e:
        logger.debug("Failed discovering LAN IP via hostname resolution: %s", e)

    return "127.0.0.1"




def is_port_available(port: int, host: str = "0.0.0.0") -> bool:
    """Check if a TCP port is currently free and bindable across IPv4 and IPv6 interfaces.

    Performs active connection tests to catch running services and clean socket binding
    and listening checks without SO_REUSEADDR to prevent false availability reporting.
    """
    if port <= 0 or port > 65535:
        return False

    import errno

    # 1. Active connection probes to detect listening services (e.g. on 127.0.0.1 or ::1)
    targets_to_probe: list[tuple[int, str]] = []
    if host in ("0.0.0.0", "127.0.0.1", "localhost", ""):
        targets_to_probe.append((socket.AF_INET, "127.0.0.1"))
        if socket.has_ipv6:
            targets_to_probe.append((socket.AF_INET6, "::1"))
    elif host == "::":
        if socket.has_ipv6:
            targets_to_probe.append((socket.AF_INET6, "::1"))
        targets_to_probe.append((socket.AF_INET, "127.0.0.1"))
    else:
        af = socket.AF_INET6 if ":" in host else socket.AF_INET
        targets_to_probe.append((af, host))

    for family, ip in targets_to_probe:
        try:
            with socket.socket(family, socket.SOCK_STREAM) as probe:
                probe.settimeout(0.05)
                if probe.connect_ex((ip, port)) == 0:
                    return False
        except (OSError, socket.error):
            pass

    # 2. IPv4 Bind & Listen probe (without SO_REUSEADDR)
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.bind((host if host not in ("::", "") else "0.0.0.0", port))
            s.listen(1)
    except OSError:
        return False

    # If checking 0.0.0.0, also verify 127.0.0.1 bind
    if host in ("0.0.0.0", ""):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
                s.bind(("127.0.0.1", port))
                s.listen(1)
        except OSError:
            return False

    # 3. IPv6 Bind & Listen probe (if IPv6 supported)
    if socket.has_ipv6 and host in ("0.0.0.0", "::", "127.0.0.1", "localhost", ""):
        try:
            with socket.socket(socket.AF_INET6, socket.SOCK_STREAM) as s6:
                s6.bind(("::1", port))
                s6.listen(1)
        except OSError as e:
            if getattr(e, "errno", None) in (errno.EADDRINUSE, errno.EACCES):
                return False
        except Exception:
            pass

    return True


def find_available_port(
    preferred_port: int,
    max_attempts: int = 100,
    exclude_ports: set[int] | None = None,
    host: str = "0.0.0.0",
) -> int:
    """Find an available port starting from preferred_port, checking socket binding in real-time."""
    excluded = exclude_ports or set()
    candidate = preferred_port

    for _ in range(max_attempts):
        if candidate not in excluded and is_port_available(candidate, host=host):
            return candidate
        candidate += 1

    raise RuntimeError(
        f"Unable to find an available TCP port starting from {preferred_port} after {max_attempts} attempts."
    )


def compute_preferred_service_port(base_port: int, slot: int, repo_index: int = 0) -> int:
    """Compute deterministic preferred port for a service in a workspace slot.

    Offset formula: base_port + (slot * 10)
    If base_port is 0 or unassigned, defaults to 8000 + (slot * 10) + repo_index.
    """
    if base_port > 0:
        return base_port + (slot * 10)
    return 8000 + (slot * 10) + repo_index


def allocate_workspace_ports(
    repositories: Mapping[str, Any],
    slot: int = 0,
    recorded_leases: dict[str, Any] | None = None,
) -> tuple[dict[str, int], bool]:
    """Allocate non-conflicting, verified bindable ports for all services in a workspace.

    Supports both single-port and multi-port service configurations.

    Returns:
        tuple[dict[key, allocated_port], bool shifted]
        Dictionary contains primary ports (e.g. 'server'), named sub-ports (e.g. 'server:http', 'server:ws'),
        and indexed sub-ports (e.g. 'server:0', 'server:1').
        shifted is True if any port differed from recorded_leases due to collision auto-healing.
    """
    allocated: dict[str, int] = {}
    used_ports: set[int] = set()
    recorded = recorded_leases or {}
    has_shifted = False
    global_port_idx = 0

    sorted_repos = sorted(repositories.keys())
    for r_name in sorted_repos:
        repo_cfg = repositories[r_name]

        # Extract ports configuration
        ports_dict: dict[str, int] = {}
        if hasattr(repo_cfg, "ports") and isinstance(repo_cfg.ports, dict) and repo_cfg.ports:
            ports_dict = dict(repo_cfg.ports)
        elif isinstance(repo_cfg, dict) and isinstance(repo_cfg.get("ports"), dict) and repo_cfg.get("ports"):
            ports_dict = {str(k): int(v) for k, v in repo_cfg["ports"].items()}
        elif hasattr(repo_cfg, "ports") and isinstance(repo_cfg.ports, list) and repo_cfg.ports:
            ports_dict = {"default" if i == 0 else f"port_{i}": int(v) for i, v in enumerate(repo_cfg.ports)}
        elif isinstance(repo_cfg, dict) and isinstance(repo_cfg.get("ports"), list) and repo_cfg.get("ports"):
            ports_dict = {"default" if i == 0 else f"port_{i}": int(v) for i, v in enumerate(repo_cfg["ports"])}

        base_port = getattr(repo_cfg, "port", None) if not isinstance(repo_cfg, dict) else repo_cfg.get("port")
        if base_port is not None and "default" not in ports_dict and not ports_dict:
            ports_dict["default"] = int(base_port)
        elif not ports_dict:
            ports_dict["default"] = int(base_port or 0)

        # Iterate over all defined sub-ports for this repo
        repo_allocated_ports: list[tuple[str, int]] = []
        for sub_idx, (port_label, b_port) in enumerate(ports_dict.items()):
            # Check recorded leases
            rec_port = None
            if isinstance(recorded.get(r_name), dict):
                rec_port = recorded[r_name].get(port_label)
            elif f"{r_name}:{port_label}" in recorded:
                rec_port = recorded[f"{r_name}:{port_label}"]
            elif sub_idx == 0 and isinstance(recorded.get(r_name), int):
                rec_port = recorded[r_name]

            preferred = rec_port or compute_preferred_service_port(b_port, slot, global_port_idx)
            global_port_idx += 1

            # Probe candidate port for availability
            live_port = find_available_port(
                preferred_port=preferred,
                max_attempts=50,
                exclude_ports=used_ports,
            )

            if rec_port and live_port != rec_port:
                has_shifted = True
                logger.info(
                    "Service '%s' (port '%s') shifted from %d to %d due to active socket collision.",
                    r_name,
                    port_label,
                    rec_port,
                    live_port,
                )

            used_ports.add(live_port)
            repo_allocated_ports.append((port_label, live_port))

            # Store aliases
            allocated[f"{r_name}:{port_label}"] = live_port
            allocated[f"{r_name}:{sub_idx}"] = live_port

        # Primary port is the first allocated port
        if repo_allocated_ports:
            allocated[r_name] = repo_allocated_ports[0][1]

    return allocated, has_shifted
