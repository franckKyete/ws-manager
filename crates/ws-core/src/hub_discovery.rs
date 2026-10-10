use crate::network::list_network_interfaces;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredHub {
    pub url: String,
    pub port: u16,
    pub version: String,
    pub hostname: String,
    pub ips: Vec<String>,
    pub latency_ms: u64,
}

pub fn compute_broadcast_address(ip_str: &str) -> Option<String> {
    if let Ok(ipv4) = ip_str.parse::<Ipv4Addr>() {
        let octets = ipv4.octets();
        if octets[0] == 10 {
            // Typical /24 or /8 - /24 is most common in home/office subnets
            return Some(format!("{}.{}.{}.255", octets[0], octets[1], octets[2]));
        } else if octets[0] == 172 && (16..=31).contains(&octets[1]) {
            return Some(format!("{}.{}.{}.255", octets[0], octets[1], octets[2]));
        } else if octets[0] == 192 && octets[1] == 168 {
            return Some(format!("192.168.{}.255", octets[2]));
        }
        return Some(format!("{}.{}.{}.255", octets[0], octets[1], octets[2]));
    }
    None
}

pub fn quick_ping_hub(url: &str, timeout_ms: u64) -> bool {
    let clean = url.trim_end_matches('/');
    let target = format!("{}/healthz", clean);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build();

    if let Ok(c) = client {
        if let Ok(resp) = c.get(&target).send() {
            return resp.status().is_success();
        }
        // Fallback check on /api/v1/health
        let health_ep = format!("{}/api/v1/health", clean);
        if let Ok(resp) = c.get(&health_ep).send() {
            return resp.status().is_success();
        }
    }
    false
}

pub fn discover_local_hubs(timeout_ms: u64, discovery_port: u16) -> Vec<DiscoveredHub> {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let _ = socket.set_broadcast(true);
    let _ = socket.set_read_timeout(Some(Duration::from_millis(timeout_ms.min(1000))));

    let probe_payload = serde_json::json!({
        "type": "wshub_discover",
        "service": "wshub",
        "client": "ws"
    })
    .to_string();

    let probe_bytes = probe_payload.as_bytes();

    // 1. Broadcast targets: 255.255.255.255, interface subnets, and loopback
    let mut targets: HashSet<SocketAddr> = HashSet::new();

    if let Ok(addr) = format!("255.255.255.255:{}", discovery_port).parse() {
        targets.insert(addr);
    }
    if let Ok(addr) = format!("127.0.0.1:{}", discovery_port).parse() {
        targets.insert(addr);
    }

    for iface in list_network_interfaces() {
        if let Some(bcast) = compute_broadcast_address(&iface.ip) {
            if let Ok(addr) = format!("{}:{}", bcast, discovery_port).parse() {
                targets.insert(addr);
            }
        }
    }

    for target in &targets {
        let _ = socket.send_to(probe_bytes, target);
    }

    let start = Instant::now();
    let max_duration = Duration::from_millis(timeout_ms);
    let mut discovered = Vec::new();
    let mut seen_urls = HashSet::new();

    let mut buf = [0u8; 4096];

    while start.elapsed() < max_duration {
        match socket.recv_from(&mut buf) {
            Ok((size, src_addr)) => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&buf[..size]) {
                    let service = val.get("service").and_then(|v| v.as_str()).unwrap_or("");
                    if service == "wshub" || val.get("port").is_some() {
                        let port = val.get("port").and_then(|v| v.as_u64()).unwrap_or(8787) as u16;
                        let version = val
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0.1.0")
                            .to_string();
                        let hostname = val
                            .get("hostname")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let ips = val
                            .get("ips")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();

                        // Candidate URLs: primaryUrl or from packet source address
                        let mut candidates = Vec::new();
                        if let Some(p_url) = val.get("primaryUrl").and_then(|v| v.as_str()) {
                            candidates.push(p_url.trim_end_matches('/').to_string());
                        }

                        let src_ip = src_addr.ip().to_string();
                        candidates.push(format!("http://{}:{}", src_ip, port));

                        for ip in &ips {
                            candidates.push(format!("http://{}:{}", ip, port));
                        }

                        for candidate in candidates {
                            if seen_urls.insert(candidate.clone()) {
                                let ping_start = Instant::now();
                                if quick_ping_hub(&candidate, 500) {
                                    let latency_ms = ping_start.elapsed().as_millis() as u64;
                                    discovered.push(DiscoveredHub {
                                        url: candidate,
                                        port,
                                        version: version.clone(),
                                        hostname: hostname.clone(),
                                        ips: ips.clone(),
                                        latency_ms,
                                    });
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Timeout slice elapsed, continue until total max_duration
                if start.elapsed() >= max_duration {
                    break;
                }
            }
            Err(_) => {
                break;
            }
        }
    }

    discovered.sort_by_key(|h| h.latency_ms);
    discovered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_broadcast_address() {
        assert_eq!(
            compute_broadcast_address("192.168.1.15").unwrap(),
            "192.168.1.255"
        );
        assert_eq!(compute_broadcast_address("10.0.0.2").unwrap(), "10.0.0.255");
        assert_eq!(
            compute_broadcast_address("172.20.10.3").unwrap(),
            "172.20.10.255"
        );
        assert!(compute_broadcast_address("not_an_ip").is_none());
    }

    #[test]
    fn test_quick_ping_invalid_host() {
        // Ping a non-existent port should return false without crashing
        assert!(!quick_ping_hub("http://127.0.0.1:59999", 50));
    }
}
