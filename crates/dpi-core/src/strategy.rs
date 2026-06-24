//! DPI-bypass strategy model and translation to `nfqws` (zapret) command-line
//! arguments.
//!
//! A [`Strategy`] is an engine-agnostic description of how packets should be
//! manipulated. The Linux engine renders it into `nfqws` flags; the Windows
//! engine (scaffolded) renders it into GoodbyeDPI flags. Keeping the model
//! abstract lets profiles be portable and lets the prober enumerate candidates
//! without caring about the underlying tool.

use serde::{Deserialize, Serialize};

/// TCP / TLS desync parameters (the part that fixes Discord text, API, gateway,
/// CDN — anything over TCP/443).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpStrategy {
    /// nfqws `--dpi-desync` mode list, e.g. `"fake,split2"` or `"fakeddisorder"`.
    pub desync: String,
    /// Split position. Either a number (`"1"`, `"2"`) or a marker understood by
    /// nfqws. Valid markers (zapret v71+): `method`, `host`, `endhost`, `sld`,
    /// `endsld`, `midsld`, `sniext` (optionally with `+N`/`-N`). To split inside
    /// the TLS SNI use `midsld` or `sniext` — note: bare `"sni"` is NOT valid.
    pub split_pos: String,
    /// TTL used for injected fake packets (so they reach the DPI box but not the
    /// real server). `0` means "let the engine pick / autottl".
    pub ttl: u8,
    /// nfqws `--dpi-desync-fooling` mode, e.g. `"md5sig"`, `"badsum"`, `"none"`.
    pub fooling: String,
    /// How many times to repeat the desync packet(s).
    pub repeats: u8,
}

impl Default for TcpStrategy {
    fn default() -> Self {
        Self {
            desync: "fake,split2".into(),
            split_pos: "midsld".into(),
            ttl: 0,
            fooling: "md5sig".into(),
            repeats: 1,
        }
    }
}

/// UDP / QUIC desync parameters (the part that fixes Discord **voice**, which is
/// the most common Turkey complaint when only TCP is handled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpStrategy {
    /// nfqws `--dpi-desync` mode for UDP, typically `"fake"` or `"fake,udplen"`.
    pub desync: String,
    /// TTL for fake UDP/QUIC packets.
    pub ttl: u8,
    /// Repeat count.
    pub repeats: u8,
}

impl Default for UdpStrategy {
    fn default() -> Self {
        Self {
            desync: "fake".into(),
            ttl: 0,
            repeats: 2,
        }
    }
}

/// A complete strategy: the TCP part is mandatory, the UDP/QUIC part is optional
/// (only Discord-voice-style targets need it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strategy {
    pub tcp: TcpStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_quic: Option<UdpStrategy>,
}

impl Strategy {
    /// Build the `nfqws` argument vector for this strategy.
    ///
    /// nfqws separates independent traffic profiles with `--new`. We emit one
    /// profile for TCP/443 and, when present, one for UDP/443 (QUIC + Discord
    /// voice). `tcp_queue`/`udp_queue` are informational; queue binding is done
    /// by the nftables rules, nfqws reads from whichever queue it is told to via
    /// `--qnum` (set by the engine, not here).
    pub fn to_nfqws_args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        // --- TCP / TLS profile ---
        args.push("--filter-tcp=443".into());
        args.push(format!("--dpi-desync={}", self.tcp.desync));
        if self.tcp.ttl > 0 {
            args.push(format!("--dpi-desync-ttl={}", self.tcp.ttl));
        } else {
            // Let nfqws derive a TTL that lands between us and the DPI box.
            args.push("--dpi-desync-autottl=2".into());
        }
        if self.tcp.fooling != "none" {
            args.push(format!("--dpi-desync-fooling={}", self.tcp.fooling));
        }
        if !self.tcp.split_pos.is_empty() {
            args.push(format!("--dpi-desync-split-pos={}", self.tcp.split_pos));
        }
        if self.tcp.repeats > 1 {
            args.push(format!("--dpi-desync-repeats={}", self.tcp.repeats));
        }

        // --- UDP / QUIC profile (Discord voice) ---
        if let Some(udp) = &self.udp_quic {
            args.push("--new".into());
            args.push("--filter-udp=443".into());
            args.push(format!("--dpi-desync={}", udp.desync));
            if udp.ttl > 0 {
                args.push(format!("--dpi-desync-ttl={}", udp.ttl));
            } else {
                args.push("--dpi-desync-autottl=2".into());
            }
            if udp.repeats > 1 {
                args.push(format!("--dpi-desync-repeats={}", udp.repeats));
            }
        }

        args
    }

    /// Render the strategy into GoodbyeDPI flags (Windows engine, scaffolded).
    /// Only the TCP part maps cleanly; GoodbyeDPI's UDP/QUIC handling differs and
    /// is left for the Windows milestone.
    pub fn to_goodbyedpi_args(&self) -> Vec<String> {
        // -5 is GoodbyeDPI's strongest preset; we additionally express split.
        let mut args = vec!["-5".to_string()];
        if self.tcp.split_pos == "sni" || self.tcp.split_pos == "host" {
            args.push("--frag-by-sni".into());
        }
        args
    }
}

/// The ordered list of candidate strategies the prober tries, from least to most
/// aggressive. Curated for Turkish ISPs / Discord; refined over time. The first
/// one that passes both text and (when applicable) voice checks wins.
pub fn candidate_strategies() -> Vec<Strategy> {
    let voice = || {
        Some(UdpStrategy {
            desync: "fake".into(),
            ttl: 0,
            repeats: 2,
        })
    };
    vec![
        // 1. Plain split inside the SNI — cheapest, sometimes enough.
        Strategy {
            tcp: TcpStrategy {
                desync: "split2".into(),
                split_pos: "midsld".into(),
                ttl: 0,
                fooling: "none".into(),
                repeats: 1,
            },
            udp_quic: voice(),
        },
        // 2. Fake + split with md5sig fooling — common TR winner.
        Strategy {
            tcp: TcpStrategy::default(),
            udp_quic: voice(),
        },
        // 3. Fake + disorder, lower fixed TTL.
        Strategy {
            tcp: TcpStrategy {
                desync: "fakeddisorder".into(),
                split_pos: "midsld".into(),
                ttl: 4,
                fooling: "md5sig".into(),
                repeats: 2,
            },
            udp_quic: voice(),
        },
        // 4. Aggressive: badsum fooling, repeats.
        Strategy {
            tcp: TcpStrategy {
                desync: "fake,multisplit".into(),
                split_pos: "1".into(),
                ttl: 3,
                fooling: "badsum".into(),
                repeats: 3,
            },
            udp_quic: voice(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_strategy_renders_tcp_args() {
        let s = Strategy {
            tcp: TcpStrategy::default(),
            udp_quic: None,
        };
        let args = s.to_nfqws_args();
        assert!(args.iter().any(|a| a == "--filter-tcp=443"));
        assert!(args.iter().any(|a| a == "--dpi-desync=fake,split2"));
        assert!(args.iter().any(|a| a == "--dpi-desync-split-pos=midsld"));
        // No UDP profile when udp_quic is None.
        assert!(!args.iter().any(|a| a == "--new"));
    }

    #[test]
    fn voice_strategy_adds_udp_profile() {
        let s = Strategy {
            tcp: TcpStrategy::default(),
            udp_quic: Some(UdpStrategy::default()),
        };
        let args = s.to_nfqws_args();
        assert!(args.iter().any(|a| a == "--new"));
        assert!(args.iter().any(|a| a == "--filter-udp=443"));
    }

    #[test]
    fn candidates_nonempty_and_all_have_voice() {
        let c = candidate_strategies();
        assert!(!c.is_empty());
        assert!(c.iter().all(|s| s.udp_quic.is_some()));
    }
}
