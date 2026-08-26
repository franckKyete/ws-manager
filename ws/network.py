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
    """Check if a TCP port is currently free and bindable on the specified interface."""
    if port <= 0 or port > 65535:
        return False

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            s.bind((host, port))
            return True
        except OSError:
            return False


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
    recorded_leases: dict[str, int] | None = None,
) -> tuple[dict[str, int], bool]:
    """Allocate non-conflicting, verified bindable ports for all services in a workspace.

    Returns:
        tuple[dict[repo_name, allocated_port], bool shifted]
        shifted is True if any port differed from recorded_leases due to collision auto-healing.
    """
    allocated: dict[str, int] = {}
    used_ports: set[int] = set()
    recorded = recorded_leases or {}
    has_shifted = False

    sorted_repos = sorted(repositories.keys())
    for idx, r_name in enumerate(sorted_repos):
        repo_cfg = repositories[r_name]
        base_port = getattr(repo_cfg, "port", None) or 0
        preferred = recorded.get(r_name) or compute_preferred_service_port(base_port, slot, idx)

        # Probe candidate port for availability
        live_port = find_available_port(
            preferred_port=preferred,
            max_attempts=50,
            exclude_ports=used_ports,
        )

        if recorded.get(r_name) and live_port != recorded[r_name]:
            has_shifted = True
            logger.info(
                "Service '%s' port shifted from %d to %d due to active socket collision.",
                r_name,
                recorded[r_name],
                live_port,
            )

        allocated[r_name] = live_port
        used_ports.add(live_port)

    return allocated, has_shifted
