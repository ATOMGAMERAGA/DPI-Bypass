//! Linux engine: nftables NFQUEUE + nfqws (zapret core).
//!
//! Model: locally-originated TCP/443 and UDP/443 packets are diverted into an
//! NFQUEUE; `nfqws` reads that queue, matches the SNI against a hostlist, and
//! desyncs only matching flows. The `bypass` flag on the queue rule is the
//! kill-switch: if `nfqws` is not attached, packets pass untouched, so a crash
//! never blackholes the connection.
//!
//! This helper is invoked per-action (via pkexec). `apply` spawns `nfqws`
//! detached and records its PID; `revert` kills it and removes the table.

use anyhow::{bail, Context, Result};
use dpi_core::engine::{DomainSet, Engine};
use dpi_core::strategy::Strategy;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `dpi_core::Engine` adapter so the prober can drive the real Linux engine
/// inside a single privileged invocation (one pkexec prompt for a whole solve).
/// Aliased as `PlatformEngine` so `main.rs` is platform-agnostic.
pub struct LinuxEngine {
    pub nfqws: Option<String>,
}

/// The engine type `main.rs` instantiates, identical name across platforms.
pub type PlatformEngine = LinuxEngine;

impl LinuxEngine {
    pub fn new(nfqws: Option<String>) -> Self {
        Self { nfqws }
    }
}

impl Engine for LinuxEngine {
    fn apply(&mut self, strategy: &Strategy, domains: &DomainSet) -> dpi_core::Result<()> {
        apply(strategy, &domains.domains, self.nfqws.as_deref())
            .map_err(|e| dpi_core::CoreError::Other(e.to_string()))
    }
    fn revert(&mut self) -> dpi_core::Result<()> {
        revert().map_err(|e| dpi_core::CoreError::Other(e.to_string()))
    }
    fn is_active(&self) -> bool {
        is_active()
    }
}

const TABLE: &str = "dpi_bypass";
const QUEUE_NUM: u32 = 200;
const RUN_DIR: &str = "/run/dpi-bypass";
const DEFAULT_NFQWS: &str = "/usr/lib/dpi-bypass/nfqws";
const ACTIVE_PROFILE: &str = "/etc/dpi-bypass/active-profile.json";

/// What the "always on" systemd daemon applies on boot. Persisted to
/// [`ACTIVE_PROFILE`] when the user enables the service.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ActiveProfile {
    pub strategy: Strategy,
    pub domains: Vec<String>,
    #[serde(default)]
    pub nfqws: Option<String>,
}

/// Persist the profile the always-on service should apply.
pub fn write_active_profile(
    strategy: &Strategy,
    domains: &[String],
    nfqws: Option<&str>,
) -> Result<()> {
    let ap = ActiveProfile {
        strategy: strategy.clone(),
        domains: domains.to_vec(),
        nfqws: nfqws.map(|s| s.to_string()),
    };
    fs::create_dir_all("/etc/dpi-bypass").context("creating /etc/dpi-bypass")?;
    fs::write(ACTIVE_PROFILE, serde_json::to_vec_pretty(&ap)?)?;
    Ok(())
}

/// systemd `ExecStart` body: apply the active profile and run nfqws in the
/// foreground so systemd owns its lifetime. Returns when nfqws exits; the unit's
/// `ExecStopPost` then calls `revert` to clean up nftables.
pub fn run_daemon() -> Result<()> {
    let data = fs::read(ACTIVE_PROFILE)
        .context("no active profile saved; enable 'Always On' from the app first")?;
    let ap: ActiveProfile = serde_json::from_slice(&data)?;
    let nfqws = nfqws_path(ap.nfqws.as_deref());
    if !Path::new(&nfqws).exists() {
        bail!("nfqws engine not found at {nfqws}");
    }
    ensure_run_dir()?;
    fs::write(hostlist_path(), hostlist_contents(&ap.domains))?;
    nft_apply(&nft_ruleset())?;

    let argv = nfqws_argv(&nfqws, &ap.strategy, &hostlist_path());
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .with_context(|| format!("running {nfqws}"))?;
    if !status.success() {
        bail!("nfqws exited with {status}");
    }
    Ok(())
}

fn run_dir() -> PathBuf {
    PathBuf::from(RUN_DIR)
}
fn pidfile() -> PathBuf {
    run_dir().join("nfqws.pid")
}
fn hostlist_path() -> PathBuf {
    run_dir().join("hostlist.txt")
}

/// Resolve the nfqws binary path: explicit arg > env > default install path.
pub fn nfqws_path(explicit: Option<&str>) -> String {
    if let Some(p) = explicit {
        return p.to_string();
    }
    std::env::var("DPI_NFQWS").unwrap_or_else(|_| DEFAULT_NFQWS.to_string())
}

/// The nftables ruleset that diverts 443 traffic into the queue.
pub fn nft_ruleset() -> String {
    format!(
        "table inet {TABLE} {{\n\
         \tchain output {{\n\
         \t\ttype filter hook output priority mangle; policy accept;\n\
         \t\tmeta l4proto tcp tcp dport 443 queue num {QUEUE_NUM} bypass\n\
         \t\tmeta l4proto udp udp dport 443 queue num {QUEUE_NUM} bypass\n\
         \t}}\n\
         }}\n"
    )
}

/// Build the full nfqws argv: queue binding + hostlist + strategy flags.
pub fn nfqws_argv(nfqws: &str, strategy: &Strategy, hostlist: &Path) -> Vec<String> {
    let mut argv = vec![
        nfqws.to_string(),
        format!("--qnum={QUEUE_NUM}"),
        format!("--hostlist={}", hostlist.display()),
    ];
    argv.extend(strategy.to_nfqws_args());
    argv
}

/// Normalise a domain set into an nfqws hostlist (one host per line, wildcards
/// stripped — nfqws matches a host and all its subdomains).
fn hostlist_contents(domains: &[String]) -> String {
    let mut lines: Vec<String> = domains
        .iter()
        .map(|d| d.trim_start_matches("*.").trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();
    lines.sort();
    lines.dedup();
    lines.join("\n") + "\n"
}

/// Print, without applying, exactly what `apply` would do. Used for verification
/// in environments without root.
pub fn plan(strategy: &Strategy, domains: &[String], nfqws: Option<&str>) -> String {
    let nfqws = nfqws_path(nfqws);
    let hl = hostlist_path();
    let argv = nfqws_argv(&nfqws, strategy, &hl);
    format!(
        "# nftables ruleset:\n{}\n# hostlist ({}):\n{}\n# nfqws command:\n{}\n",
        nft_ruleset(),
        hl.display(),
        hostlist_contents(domains),
        argv.join(" ")
    )
}

fn ensure_run_dir() -> Result<()> {
    fs::create_dir_all(run_dir()).with_context(|| format!("creating {RUN_DIR}"))?;
    Ok(())
}

/// Apply a strategy: write hostlist, install nft rules, spawn nfqws detached.
pub fn apply(strategy: &Strategy, domains: &[String], nfqws: Option<&str>) -> Result<()> {
    ensure_run_dir()?;
    let nfqws = nfqws_path(nfqws);
    if !Path::new(&nfqws).exists() {
        bail!("nfqws engine not found at {nfqws} (build/install it first)");
    }

    // Always start from a clean slate so re-apply is idempotent.
    let _ = revert();

    let hl = hostlist_path();
    fs::write(&hl, hostlist_contents(domains)).context("writing hostlist")?;

    nft_apply(&nft_ruleset())?;

    let argv = nfqws_argv(&nfqws, strategy, &hl);
    let child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawning {nfqws}"))?;

    fs::write(pidfile(), child.id().to_string()).context("writing pidfile")?;
    Ok(())
}

/// Revert everything: kill nfqws, delete the nft table, remove run files.
pub fn revert() -> Result<()> {
    if let Ok(pid) = fs::read_to_string(pidfile()) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            // SIGTERM the nfqws process; ignore if already gone.
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        let _ = fs::remove_file(pidfile());
    }
    nft_delete();
    let _ = fs::remove_file(hostlist_path());
    Ok(())
}

/// Active when both the nft table exists and the nfqws process is alive.
pub fn is_active() -> bool {
    let table_present = Command::new("nft")
        .args(["list", "table", "inet", TABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let proc_alive = fs::read_to_string(pidfile())
        .ok()
        .and_then(|p| p.trim().parse::<i32>().ok())
        .map(|pid| Path::new(&format!("/proc/{pid}")).exists())
        .unwrap_or(false);

    table_present && proc_alive
}

const SERVICE: &str = "dpi-bypass.service";

/// Enable + start the always-on systemd service.
pub fn enable_service() -> Result<()> {
    systemctl(&["enable", "--now", SERVICE])
}

/// Stop + disable the always-on systemd service.
pub fn disable_service() -> Result<()> {
    systemctl(&["disable", "--now", SERVICE])
}

/// `(enabled, active)` for the always-on service.
pub fn service_status() -> (bool, bool) {
    let enabled = systemctl(&["is-enabled", "--quiet", SERVICE]).is_ok();
    let active = systemctl(&["is-active", "--quiet", SERVICE]).is_ok();
    (enabled, active)
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .context("running systemctl")?;
    if !status.success() {
        bail!("systemctl {:?} failed", args);
    }
    Ok(())
}

fn nft_apply(ruleset: &str) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .context("spawning nft")?;
    child
        .stdin
        .as_mut()
        .context("nft stdin")?
        .write_all(ruleset.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("nft failed to apply ruleset");
    }
    Ok(())
}

fn nft_delete() {
    let _ = Command::new("nft")
        .args(["delete", "table", "inet", TABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpi_core::strategy::{Strategy, TcpStrategy, UdpStrategy};

    #[test]
    fn ruleset_has_queue_and_bypass() {
        let r = nft_ruleset();
        assert!(r.contains("queue num 200 bypass"));
        assert!(r.contains("tcp dport 443"));
        assert!(r.contains("udp dport 443"));
    }

    #[test]
    fn argv_contains_qnum_and_hostlist() {
        let s = Strategy {
            tcp: TcpStrategy::default(),
            udp_quic: Some(UdpStrategy::default()),
        };
        let argv = nfqws_argv("/usr/lib/dpi-bypass/nfqws", &s, Path::new("/tmp/hl.txt"));
        assert_eq!(argv[0], "/usr/lib/dpi-bypass/nfqws");
        assert!(argv.iter().any(|a| a == "--qnum=200"));
        assert!(argv.iter().any(|a| a == "--hostlist=/tmp/hl.txt"));
        assert!(argv.iter().any(|a| a == "--filter-tcp=443"));
    }

    #[test]
    fn hostlist_strips_wildcards_and_dedups() {
        let hl = hostlist_contents(&[
            "*.discord.com".into(),
            "discord.com".into(),
            "Discord.GG".into(),
        ]);
        let lines: Vec<&str> = hl.lines().collect();
        assert!(lines.contains(&"discord.com"));
        assert!(lines.contains(&"discord.gg"));
        // "*.discord.com" and "discord.com" collapse to one entry.
        assert_eq!(lines.iter().filter(|l| **l == "discord.com").count(), 1);
    }
}
