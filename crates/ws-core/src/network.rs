use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceType {
    Wireless,
    Ethernet,
    Other,
}

#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub name: String,
    pub ip: String,
    pub iface_type: InterfaceType,
}

pub fn is_valid_ipv4(ip: &str) -> bool {
    ip.parse::<Ipv4Addr>().is_ok()
}

pub fn is_virtual_docker_ip(ip: &str) -> bool {
    if let Ok(ipv4) = ip.parse::<Ipv4Addr>() {
        let octets = ipv4.octets();
        if octets[0] == 172 && (17..=31).contains(&octets[1]) {
            return true;
        }
    }
    false
}

pub fn is_wireless_interface(iface: &str) -> bool {
    let p1 = format!("/sys/class/net/{}/wireless", iface);
    let p2 = format!("/sys/class/net/{}/phy80211", iface);
    if Path::new(&p1).exists() || Path::new(&p2).exists() {
        return true;
    }
    for prefix in &["wl", "wlan", "wifi", "ath", "ra"] {
        if iface.starts_with(prefix) {
            return true;
        }
    }
    false
}

pub fn is_ethernet_interface(iface: &str) -> bool {
    if is_wireless_interface(iface) {
        return false;
    }
    for prefix in &["eth", "en", "em"] {
        if iface.starts_with(prefix) {
            return true;
        }
    }
    false
}

pub fn list_network_interfaces() -> Vec<NetworkInterface> {
    let mut interfaces = Vec::new();
    let mut seen_ifaces = HashSet::new();

    // 1. Run 'ip -4 -o addr show' on Linux
    if let Ok(out) = Command::new("ip").args(&["-4", "-o", "addr", "show"]).output() {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let iface = parts[1];
                    let ip_with_mask = parts[3];
                    let ip = ip_with_mask.split('/').next().unwrap_or("");

                    if ip.starts_with("127.") || ip.starts_with("169.254.") || is_virtual_docker_ip(ip) {
                        continue;
                    }
                    if iface.starts_with("docker")
                        || iface.starts_with("br-")
                        || iface.starts_with("veth")
                        || iface.starts_with("virbr")
                        || iface.starts_with("tun")
                        || iface.starts_with("tap")
                        || iface == "lo"
                        || iface.starts_with("dummy")
                    {
                        continue;
                    }

                    if seen_ifaces.insert(iface.to_string()) {
                        let iface_type = if is_wireless_interface(iface) {
                            InterfaceType::Wireless
                        } else if is_ethernet_interface(iface) {
                            InterfaceType::Ethernet
                        } else {
                            InterfaceType::Other
                        };
                        interfaces.push(NetworkInterface {
                            name: iface.to_string(),
                            ip: ip.to_string(),
                            iface_type,
                        });
                    }
                }
            }
        }
    }

    // Sort by type: Wireless first, then Ethernet, then Other
    interfaces.sort_by(|a, b| {
        let rank = |t: &InterfaceType| match t {
            InterfaceType::Wireless => 0,
            InterfaceType::Ethernet => 1,
            InterfaceType::Other => 2,
        };
        rank(&a.iface_type).cmp(&rank(&b.iface_type))
    });

    interfaces
}

pub fn get_lan_ip(preferred_interface: Option<&str>) -> String {
    if let Ok(ip) = std::env::var("WS_LAN_IP") {
        let trimmed = ip.trim();
        if is_valid_ipv4(trimmed) && !trimmed.starts_with("127.") {
            return trimmed.to_string();
        }
    }

    let ifaces = list_network_interfaces();

    if let Some(pref) = preferred_interface {
        let p_lower = pref.to_lowercase();
        if p_lower == "wifi" || p_lower == "wlan" || p_lower == "wireless" {
            if let Some(iface) = ifaces.iter().find(|i| i.iface_type == InterfaceType::Wireless) {
                return iface.ip.clone();
            }
        } else if p_lower == "eth" || p_lower == "ethernet" {
            if let Some(iface) = ifaces.iter().find(|i| i.iface_type == InterfaceType::Ethernet) {
                return iface.ip.clone();
            }
        } else if let Some(iface) = ifaces.iter().find(|i| i.name.to_lowercase() == p_lower) {
            return iface.ip.clone();
        }
    }

    if let Some(first) = ifaces.first() {
        return first.ip.clone();
    }

    // Fallback: connect UDP socket to probe routing table
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                let ip_str = addr.ip().to_string();
                if is_valid_ipv4(&ip_str) && !ip_str.starts_with("127.") {
                    return ip_str;
                }
            }
        }
    }

    "127.0.0.1".to_string()
}

pub fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

pub fn find_next_free_port(start_port: u16, max_attempts: u16) -> u16 {
    for port in start_port..(start_port.saturating_add(max_attempts)) {
        if is_port_available(port) {
            return port;
        }
    }
    start_port
}

pub fn compute_preferred_service_port(base_port: u16, slot: u16, offset_multiplier: u16) -> u16 {
    base_port.saturating_add(slot.saturating_mul(offset_multiplier))
}

pub fn find_available_port(
    preferred_port: u16,
    max_attempts: u16,
    exclude_ports: &HashSet<u16>,
) -> u16 {
    let mut candidate = preferred_port;
    for _ in 0..max_attempts {
        if !exclude_ports.contains(&candidate) && is_port_available(candidate) {
            return candidate;
        }
        candidate = candidate.saturating_add(1);
    }
    preferred_port
}

pub fn allocate_workspace_ports(
    repositories: &HashMap<String, crate::models::RepoConfig>,
    slot: u16,
    recorded_leases: Option<&HashMap<String, serde_json::Value>>,
) -> (HashMap<String, u16>, bool) {
    let mut allocated: HashMap<String, u16> = HashMap::new();
    let mut used_ports: HashSet<u16> = HashSet::new();
    let mut has_shifted = false;
    let mut global_port_idx = 0u16;

    let mut sorted_repos: Vec<_> = repositories.keys().cloned().collect();
    sorted_repos.sort();

    for r_name in sorted_repos {
        let repo_cfg = &repositories[&r_name];
        let mut ports_dict: HashMap<String, u16> = HashMap::new();
        if !repo_cfg.ports.is_empty() {
            ports_dict = repo_cfg.ports.clone();
        } else if let Some(p) = repo_cfg.port {
            ports_dict.insert("default".to_string(), p);
        } else {
            ports_dict.insert("default".to_string(), 0);
        }

        let mut sorted_labels: Vec<_> = ports_dict.keys().cloned().collect();
        sorted_labels.sort();

        let mut repo_allocated_ports: Vec<(String, u16)> = Vec::new();
        for (sub_idx, port_label) in sorted_labels.iter().enumerate() {
            let b_port = ports_dict[port_label];
            let mut rec_port = None;

            if let Some(leases) = recorded_leases {
                if let Some(val) = leases.get(&r_name) {
                    if let Some(map) = val.as_object() {
                        if let Some(pv) = map.get(port_label).and_then(|v| v.as_u64()) {
                            rec_port = Some(pv as u16);
                        }
                    } else if sub_idx == 0 {
                        if let Some(pv) = val.as_u64() {
                            rec_port = Some(pv as u16);
                        }
                    }
                }
                if rec_port.is_none() {
                    let full_key = format!("{}:{}", r_name, port_label);
                    if let Some(pv) = leases.get(&full_key).and_then(|v| v.as_u64()) {
                        rec_port = Some(pv as u16);
                    }
                }
            }

            let preferred = if let Some(rp) = rec_port {
                rp
            } else if b_port > 0 {
                b_port.saturating_add(slot.saturating_mul(10))
            } else {
                8000u16.saturating_add(slot.saturating_mul(10)).saturating_add(global_port_idx)
            };
            global_port_idx = global_port_idx.saturating_add(1);

            let live_port = find_available_port(preferred, 50, &used_ports);

            if let Some(rp) = rec_port {
                if live_port != rp {
                    has_shifted = true;
                }
            }

            used_ports.insert(live_port);
            repo_allocated_ports.push((port_label.clone(), live_port));

            allocated.insert(format!("{}:{}", r_name, port_label), live_port);
            allocated.insert(format!("{}:{}", r_name, sub_idx), live_port);
        }

        if let Some((_, primary_port)) = repo_allocated_ports.iter().find(|(k, _)| k == "default").or_else(|| repo_allocated_ports.first()) {
            allocated.insert(r_name, *primary_port);
        }
    }

    (allocated, has_shifted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_ipv4() {
        assert!(is_valid_ipv4("192.168.1.1"));
        assert!(!is_valid_ipv4("192.168.1.999"));
        assert!(!is_valid_ipv4("abc"));
    }

    #[test]
    fn test_docker_ip() {
        assert!(is_virtual_docker_ip("172.17.0.1"));
        assert!(is_virtual_docker_ip("172.20.0.5"));
        assert!(!is_virtual_docker_ip("192.168.1.5"));
    }
}
