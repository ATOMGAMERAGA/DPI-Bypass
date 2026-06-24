//! Reachability probing and network fingerprinting.
//!
//! Reachability is determined the same way `blockcheck` does: we hand-craft a
//! TLS ClientHello carrying the target SNI, send it over TCP/443, and watch what
//! comes back. A DPI box that censors by SNI typically injects a TCP RST right
//! after seeing the ClientHello, which we observe as a connection reset. A
//! reachable server replies with a TLS record (ServerHello `0x16` or an alert
//! `0x15`) — either proves the path is open.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::process::Command;
use std::time::Duration;

/// Outcome of probing a single domain over one transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reachability {
    /// Server responded with TLS bytes — path is open.
    Reachable,
    /// Name did not resolve (possible DNS-level block).
    DnsBlocked,
    /// TCP could not connect (refused / unreachable / filtered).
    TcpBlocked,
    /// Connection reset right after the ClientHello — classic SNI DPI block.
    TlsReset,
    /// No response in time — ambiguous, treated as blocked for solving.
    Timeout,
}

impl Reachability {
    /// Whether the domain is usable as-is (no bypass needed).
    pub fn is_open(self) -> bool {
        matches!(self, Reachability::Reachable)
    }
}

/// Combined text + voice verdict for a domain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DomainCheck {
    /// TCP/443 (text, API, gateway, CDN).
    pub text: Reachability,
    /// UDP/443 (QUIC / Discord voice). `None` when a voice probe was not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<Reachability>,
}

impl DomainCheck {
    pub fn is_fully_open(&self) -> bool {
        self.text.is_open() && self.voice.map(|v| v.is_open()).unwrap_or(true)
    }
}

/// Probe `domain` over TCP/443 with a SNI-bearing ClientHello.
pub fn check_text(domain: &str, timeout: Duration) -> Reachability {
    let addr = match (domain, 443u16).to_socket_addrs() {
        Ok(mut it) => match it.next() {
            Some(a) => a,
            None => return Reachability::DnsBlocked,
        },
        Err(_) => return Reachability::DnsBlocked,
    };

    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) => {
            return match e.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                    Reachability::Timeout
                }
                _ => Reachability::TcpBlocked,
            };
        }
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let hello = build_client_hello(domain);
    if stream.write_all(&hello).is_err() {
        return Reachability::TlsReset;
    }

    let mut buf = [0u8; 8];
    match stream.read(&mut buf) {
        Ok(0) => Reachability::TlsReset, // peer closed right after ClientHello
        Ok(_n) => {
            // 0x16 = handshake (ServerHello), 0x15 = alert. Either is a real
            // TLS response, so the path is open.
            if buf[0] == 0x16 || buf[0] == 0x15 {
                Reachability::Reachable
            } else {
                Reachability::TlsReset
            }
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::ConnectionReset => Reachability::TlsReset,
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Reachability::Timeout,
            _ => Reachability::TlsReset,
        },
    }
}

/// Best-effort voice/QUIC probe: send a QUIC Initial-shaped UDP datagram to
/// :443 and see whether anything comes back. Heuristic — UDP gives no
/// connection signal, so a silent timeout is reported as `Timeout` rather than a
/// definite block.
pub fn check_voice(domain: &str, timeout: Duration) -> Reachability {
    let addr = match (domain, 443u16).to_socket_addrs() {
        Ok(mut it) => match it.next() {
            Some(a) => a,
            None => return Reachability::DnsBlocked,
        },
        Err(_) => return Reachability::DnsBlocked,
    };

    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return Reachability::TcpBlocked,
    };
    let _ = sock.set_read_timeout(Some(timeout));
    if sock.connect(addr).is_err() {
        return Reachability::TcpBlocked;
    }
    // A minimal QUIC long-header Initial probe. Real servers answer with a
    // Version Negotiation or Initial; censorship often drops it silently.
    let probe = build_quic_probe();
    if sock.send(&probe).is_err() {
        return Reachability::TcpBlocked;
    }
    let mut buf = [0u8; 64];
    match sock.recv(&mut buf) {
        Ok(n) if n > 0 => Reachability::Reachable,
        Ok(_) => Reachability::Timeout,
        Err(_) => Reachability::Timeout,
    }
}

/// Run both probes for a domain.
pub fn check_domain(domain: &str, timeout: Duration, with_voice: bool) -> DomainCheck {
    DomainCheck {
        text: check_text(domain, timeout),
        voice: if with_voice {
            Some(check_voice(domain, timeout))
        } else {
            None
        },
    }
}

/// Build a minimal but valid TLS 1.2 ClientHello carrying `sni` as the
/// server_name extension. Enough for a server to respond and for an SNI DPI to
/// trip.
fn build_client_hello(sni: &str) -> Vec<u8> {
    let sni_bytes = sni.as_bytes();

    // server_name extension body.
    let mut sni_ext = Vec::new();
    let host_len = sni_bytes.len() as u16;
    let list_len = host_len + 3; // name_type(1) + name_len(2) + host
    sni_ext.extend_from_slice(&list_len.to_be_bytes());
    sni_ext.push(0x00); // name_type = host_name
    sni_ext.extend_from_slice(&host_len.to_be_bytes());
    sni_ext.extend_from_slice(sni_bytes);

    let mut extensions = Vec::new();
    extensions.extend_from_slice(&0x0000u16.to_be_bytes()); // ext type: server_name
    extensions.extend_from_slice(&(sni_ext.len() as u16).to_be_bytes());
    extensions.extend_from_slice(&sni_ext);

    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x03]); // client_version TLS 1.2
    body.extend_from_slice(&[0x11; 32]); // random (fixed is fine for a probe)
    body.push(0x00); // session_id length
    body.extend_from_slice(&0x0002u16.to_be_bytes()); // cipher_suites length
    body.extend_from_slice(&[0x13, 0x01]); // TLS_AES_128_GCM_SHA256
    body.push(0x01); // compression methods length
    body.push(0x00); // null compression
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);

    let mut handshake = Vec::new();
    handshake.push(0x01); // client_hello
    let len = body.len();
    handshake.push((len >> 16) as u8);
    handshake.push((len >> 8) as u8);
    handshake.push(len as u8);
    handshake.extend_from_slice(&body);

    let mut record = Vec::new();
    record.push(0x16); // handshake
    record.extend_from_slice(&[0x03, 0x01]); // record version TLS 1.0 (compat)
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

/// A tiny QUIC long-header datagram used only to elicit a response.
fn build_quic_probe() -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    p.push(0xC0); // long header, Initial
    p.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // version 1
    p.push(0x08); // DCID len
    p.extend_from_slice(&[0xAB; 8]);
    p.push(0x00); // SCID len
    p.resize(64, 0x00); // pad to a plausible Initial size
    p
}

/// Identifies the network the device is currently attached to, so a profile can
/// be tied to it (a strategy that works on one ISP/DPI box rarely works on
/// another). All fields are best-effort and read from the OS without elevation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkFingerprint {
    /// MAC of the default gateway (most stable per-network identifier).
    #[serde(default)]
    pub gateway_mac: Option<String>,
    /// `ethernet` / `wifi` / `other`.
    #[serde(default)]
    pub link_type: Option<String>,
    /// Local subnet in CIDR form, e.g. `192.168.1.0/24`.
    #[serde(default)]
    pub subnet: Option<String>,
    /// Default route interface name.
    #[serde(default)]
    pub iface: Option<String>,
}

impl NetworkFingerprint {
    /// Two fingerprints "match" (same network) when the gateway MAC matches, or
    /// — if MAC is unavailable — when subnet + iface match.
    pub fn matches(&self, other: &NetworkFingerprint) -> bool {
        match (&self.gateway_mac, &other.gateway_mac) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => self.subnet == other.subnet && self.iface == other.iface,
        }
    }
}

/// Collect the current network fingerprint using the `ip` tool (read-only).
/// Falls back gracefully if `ip` is unavailable or output is unexpected.
pub fn current_fingerprint() -> NetworkFingerprint {
    let mut fp = NetworkFingerprint::default();

    // Default route: `ip route show default` -> "default via <gw> dev <iface> ..."
    if let Some(out) = run_ok("ip", &["route", "show", "default"]) {
        let mut gw_ip = None;
        for tok in out.split_whitespace().collect::<Vec<_>>().windows(2) {
            match tok[0] {
                "via" => gw_ip = Some(tok[1].to_string()),
                "dev" => fp.iface = Some(tok[1].to_string()),
                _ => {}
            }
        }
        // Gateway MAC from the neighbour table.
        if let Some(gw) = gw_ip {
            if let Some(neigh) = run_ok("ip", &["neigh", "show", &gw]) {
                // "<ip> dev <if> lladdr <mac> ..."
                let toks: Vec<&str> = neigh.split_whitespace().collect();
                if let Some(pos) = toks.iter().position(|t| *t == "lladdr") {
                    fp.gateway_mac = toks.get(pos + 1).map(|s| s.to_string());
                }
            }
        }
    }

    // Link type + subnet from the chosen interface.
    if let Some(iface) = fp.iface.clone() {
        fp.link_type = Some(detect_link_type(&iface));
        if let Some(out) = run_ok("ip", &["-o", "-f", "inet", "addr", "show", &iface]) {
            // "... inet 192.168.1.34/24 brd ..."
            for tok in out.split_whitespace() {
                if tok.contains('/') && tok.split('.').count() == 4 {
                    fp.subnet = Some(to_subnet(tok));
                    break;
                }
            }
        }
    }

    fp
}

fn detect_link_type(iface: &str) -> String {
    if std::path::Path::new(&format!("/sys/class/net/{iface}/wireless")).exists() {
        "wifi".into()
    } else if std::path::Path::new(&format!("/sys/class/net/{iface}")).exists() {
        "ethernet".into()
    } else {
        "other".into()
    }
}

/// Convert `192.168.1.34/24` into the network address `192.168.1.0/24`.
fn to_subnet(cidr: &str) -> String {
    let (ip, prefix) = match cidr.split_once('/') {
        Some((i, p)) => (i, p),
        None => return cidr.to_string(),
    };
    let prefix_len: u32 = prefix.parse().unwrap_or(24);
    let octets: Vec<u32> = ip.split('.').filter_map(|o| o.parse().ok()).collect();
    if octets.len() != 4 {
        return cidr.to_string();
    }
    let addr = (octets[0] << 24) | (octets[1] << 16) | (octets[2] << 8) | octets[3];
    let mask = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };
    let net = addr & mask;
    format!(
        "{}.{}.{}.{}/{}",
        (net >> 24) & 0xff,
        (net >> 16) & 0xff,
        (net >> 8) & 0xff,
        net & 0xff,
        prefix_len
    )
}

fn run_ok(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_is_well_formed() {
        let h = build_client_hello("discord.com");
        assert_eq!(h[0], 0x16); // TLS handshake record
        assert_eq!(h[5], 0x01); // client_hello
                                // SNI bytes appear verbatim in the extension.
        let needle = b"discord.com";
        assert!(h.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn subnet_math() {
        assert_eq!(to_subnet("192.168.1.34/24"), "192.168.1.0/24");
        assert_eq!(to_subnet("10.5.6.7/8"), "10.0.0.0/8");
        assert_eq!(to_subnet("172.16.5.9/16"), "172.16.0.0/16");
    }

    #[test]
    fn fingerprint_matches_on_mac() {
        let a = NetworkFingerprint {
            gateway_mac: Some("AA:BB:CC:DD:EE:FF".into()),
            ..Default::default()
        };
        let b = NetworkFingerprint {
            gateway_mac: Some("aa:bb:cc:dd:ee:ff".into()),
            subnet: Some("10.0.0.0/24".into()),
            ..Default::default()
        };
        assert!(a.matches(&b));
    }

    #[test]
    fn unreachable_tcp_is_classified() {
        // 203.0.113.0/24 (TEST-NET-3) is reserved and unroutable.
        let r = check_text("203.0.113.1", Duration::from_millis(800));
        assert!(!r.is_open());
    }
}
