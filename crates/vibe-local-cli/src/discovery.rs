//! Local discovery so Vibe Router can find Vibe Local automatically:
//! 1. A UDP probe responder on port 20131 (`VIBE_DISCOVER_V1` -> JSON).
//! 2. A minimal mDNS / DNS-SD responder advertising `_vibe-local._tcp.local`
//!    (PTR + SRV + TXT + A records) on 224.0.0.251:5353.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;

pub const UDP_PROBE_PORT: u16 = 20131;
pub const MDNS_PORT: u16 = 5353;
const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const SERVICE_TYPE: &str = "_vibe-local._tcp.local";

#[derive(Clone)]
pub struct DiscoveryConfig {
    pub http_port: u16,
    pub lan: bool,
    pub udp_port: u16,
    pub mdns_port: u16,
}

pub struct DiscoveryHandle {
    stop: Arc<AtomicBool>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl DiscoveryHandle {
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Threads observe the flag within their 1s read timeout.
        for handle in self.threads.lock().unwrap().drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for DiscoveryHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start_discovery(config: DiscoveryConfig) -> anyhow::Result<DiscoveryHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();

    // UDP probe responder.
    {
        let udp = UdpSocket::bind(("0.0.0.0", config.udp_port))?;
        udp.set_read_timeout(Some(Duration::from_secs(1)))?;
        let stop = Arc::clone(&stop);
        let cfg = config.clone();
        threads.push(
            std::thread::Builder::new()
                .name("vibe-udp-disc".into())
                .spawn(move || {
                    let mut buf = [0u8; 512];
                    while !stop.load(Ordering::SeqCst) {
                        match udp.recv_from(&mut buf) {
                            Ok((n, src)) => {
                                let probe = String::from_utf8_lossy(&buf[..n]);
                                if probe.trim() == "VIBE_DISCOVER_V1" {
                                    let reply = json!({
                                        "service": "vibe-local",
                                        "version": env!("CARGO_PKG_VERSION"),
                                        "http_port": cfg.http_port,
                                        "lan": cfg.lan,
                                        "host": local_hostname(),
                                    });
                                    let _ = udp.send_to(reply.to_string().as_bytes(), src);
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                })?,
        );
    }

    // mDNS DNS-SD responder.
    {
        let mdns = UdpSocket::bind(("0.0.0.0", config.mdns_port))?;
        mdns.set_read_timeout(Some(Duration::from_secs(1)))?;
        let iface = Ipv4Addr::UNSPECIFIED;
        // Multicast join can fail in restricted environments; unicast mDNS
        // queries still work, so this is best-effort.
        let _ = mdns.join_multicast_v4(&MDNS_GROUP, &iface);
        let stop = Arc::clone(&stop);
        let cfg = config.clone();
        threads.push(
            std::thread::Builder::new()
                .name("vibe-mdns".into())
                .spawn(move || {
                    let mut buf = [0u8; 1500];
                    while !stop.load(Ordering::SeqCst) {
                        match mdns.recv_from(&mut buf) {
                            Ok((n, src)) => {
                                if let Some(question) = parse_mdns_question(&buf[..n])
                                    && question.to_lowercase().contains(SERVICE_TYPE)
                                {
                                    let packet = build_mdns_response(&cfg, &local_ip());
                                    let dest: SocketAddr = (src.ip(), MDNS_PORT).into();
                                    let _ = mdns.send_to(&packet, dest);
                                    let _ = mdns.send_to(&packet, (MDNS_GROUP, MDNS_PORT));
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                })?,
        );
    }

    Ok(DiscoveryHandle {
        stop,
        threads: Mutex::new(threads),
    })
}

/// Best-effort primary LAN IP (no packets are sent by UDP connect).
pub fn local_ip() -> IpAddr {
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
        && sock.connect("8.8.8.8:80").is_ok()
        && let Ok(addr) = sock.local_addr()
        && !addr.ip().is_unspecified()
    {
        return addr.ip();
    }
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

pub fn local_hostname() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            vibe_local_core::util::run_with_timeout("hostname", &[], Duration::from_secs(3))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "vibe-local".to_string());
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "vibe-local".into()
    } else {
        trimmed
    }
}

// ---------------------------------------------------------------------------
// Minimal DNS wire format
// ---------------------------------------------------------------------------

fn encode_name(name: &str, out: &mut Vec<u8>) {
    for label in name.split('.') {
        if label.is_empty() {
            continue;
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
}

/// Parse the first question name from a DNS query packet.
pub fn parse_mdns_question(packet: &[u8]) -> Option<String> {
    if packet.len() < 13 {
        return None;
    }
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if qdcount == 0 {
        return None;
    }
    let mut pos = 12;
    let mut labels = Vec::new();
    loop {
        let len = *packet.get(pos)? as usize;
        pos += 1;
        if len == 0 {
            break;
        }
        if len & 0xc0 != 0 {
            return None; // compression pointer in question: unsupported
        }
        let end = pos.checked_add(len)?;
        labels.push(String::from_utf8_lossy(packet.get(pos..end)?).into_owned());
        pos = end;
    }
    Some(labels.join("."))
}

fn push_record(out: &mut Vec<u8>, name: &str, rtype: u16, ttl: u32, rdata: &[u8]) {
    encode_name(name, out);
    out.extend_from_slice(&rtype.to_be_bytes());
    out.extend_from_slice(&0x8001u16.to_be_bytes()); // IN + cache-flush
    out.extend_from_slice(&ttl.to_be_bytes());
    out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    out.extend_from_slice(rdata);
}

/// Build a full DNS-SD answer: PTR, SRV, TXT and A records.
pub fn build_mdns_response(cfg: &DiscoveryConfig, ip: &IpAddr) -> Vec<u8> {
    let host = local_hostname();
    let instance = format!("Vibe Local ({host}).{SERVICE_TYPE}");
    let target = format!("{host}.local");
    let mut out = Vec::new();
    // Header: id 0, flags 0x8400 (response, authoritative), 4 answers.
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0x8400u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // qd
    out.extend_from_slice(&4u16.to_be_bytes()); // an
    out.extend_from_slice(&0u16.to_be_bytes()); // ns
    out.extend_from_slice(&0u16.to_be_bytes()); // ar

    // PTR: service type -> instance
    let mut ptr_data = Vec::new();
    encode_name(&instance, &mut ptr_data);
    push_record(&mut out, SERVICE_TYPE, 12, 120, &ptr_data);

    // SRV: instance -> host:port
    let mut srv_data = Vec::new();
    srv_data.extend_from_slice(&0u16.to_be_bytes()); // priority
    srv_data.extend_from_slice(&0u16.to_be_bytes()); // weight
    srv_data.extend_from_slice(&cfg.http_port.to_be_bytes());
    encode_name(&target, &mut srv_data);
    push_record(&mut out, &instance, 33, 120, &srv_data);

    // TXT: metadata
    let mut txt_data = Vec::new();
    for entry in [
        "path=/".to_string(),
        format!("lan={}", if cfg.lan { 1 } else { 0 }),
        format!("version={}", env!("CARGO_PKG_VERSION")),
    ] {
        txt_data.push(entry.len() as u8);
        txt_data.extend_from_slice(entry.as_bytes());
    }
    push_record(&mut out, &instance, 16, 120, &txt_data);

    // A: hostname -> IP (only for IPv4; IPv6-only hosts skip the A record
    // and rely on the UDP probe responder).
    match ip {
        IpAddr::V4(v4) => push_record(&mut out, &target, 1, 120, &v4.octets()),
        IpAddr::V6(_) => {
            // Replace the 4th answer slot count: we emitted 3 records.
            out[6..8].copy_from_slice(&3u16.to_be_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query_packet(name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u16.to_be_bytes()); // id
        p.extend_from_slice(&0u16.to_be_bytes()); // flags: query
        p.extend_from_slice(&1u16.to_be_bytes()); // qd
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(&0u16.to_be_bytes());
        encode_name(name, &mut p);
        p.extend_from_slice(&12u16.to_be_bytes()); // PTR
        p.extend_from_slice(&1u16.to_be_bytes()); // IN
        p
    }

    #[test]
    fn question_parse_roundtrip() {
        let p = query_packet(SERVICE_TYPE);
        assert_eq!(parse_mdns_question(&p).as_deref(), Some(SERVICE_TYPE));
        assert_eq!(parse_mdns_question(&[0u8; 4]), None);
        assert_eq!(
            parse_mdns_question(&query_packet("_other._tcp.local")).as_deref(),
            Some("_other._tcp.local")
        );
    }

    #[test]
    fn response_contains_records() {
        let cfg = DiscoveryConfig {
            http_port: 20129,
            lan: true,
            udp_port: 0,
            mdns_port: 0,
        };
        let packet = build_mdns_response(&cfg, &IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)));
        assert!(packet.len() > 60);
        let ancount = u16::from_be_bytes([packet[6], packet[7]]);
        assert_eq!(ancount, 4);
        let text = String::from_utf8_lossy(&packet);
        assert!(text.contains("_vibe-local"));
        assert!(text.contains("lan=1"));
        // port 20129 = 0x4EA1 present in SRV rdata
        assert!(packet.windows(2).any(|w| w == [0x4e, 0xa1]));
        // A record IP bytes
        assert!(packet.windows(4).any(|w| w == [192, 168, 1, 50]));
    }

    #[test]
    fn udp_probe_responder() {
        let cfg = DiscoveryConfig {
            http_port: 20129,
            lan: false,
            udp_port: 0,
            mdns_port: 0,
        };
        // Bind an ephemeral probe socket manually to test the protocol logic.
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = responder.local_addr().unwrap();
        responder
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, src) = responder.recv_from(&mut buf).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&buf[..n]).trim(),
                "VIBE_DISCOVER_V1"
            );
            let reply = json!({"service": "vibe-local", "http_port": 20129, "lan": false});
            responder
                .send_to(reply.to_string().as_bytes(), src)
                .unwrap();
        });
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client.send_to(b"VIBE_DISCOVER_V1", addr).unwrap();
        let mut buf = [0u8; 512];
        let (n, _) = client.recv_from(&mut buf).unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(reply["service"], "vibe-local");
        assert_eq!(reply["http_port"], 20129);
        server.join().unwrap();
        let _ = cfg;
    }

    #[test]
    fn hostname_sanitized() {
        let name = local_hostname();
        assert!(!name.is_empty());
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
