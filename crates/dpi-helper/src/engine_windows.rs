//! Windows engine: WinDivert + GoodbyeDPI (bundled sidecar).
//!
//! GoodbyeDPI is a mature, Turkey-proven desync engine that drives the WinDivert
//! driver itself, so this module's job is process lifecycle, not packet
//! manipulation: write a domain blacklist, spawn `goodbyedpi.exe` with the flags
//! a [`Strategy`] renders to, record its PID, and kill it on revert. Because
//! GoodbyeDPI makes no persistent firewall changes, a crash simply lets traffic
//! flow normally — that is the kill-switch (§15).
//!
//! "Always on" is a Scheduled Task (`schtasks`) that runs the helper's `daemon`
//! verb at logon with highest privileges, mirroring the Linux systemd unit.
//!
//! Exposes the same module-level API as `engine_linux` so `main.rs` is
//! platform-agnostic.

use anyhow::{bail, Context, Result};
use dpi_core::engine::{DomainSet, Engine};
use dpi_core::strategy::Strategy;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `dpi_core::Engine` adapter so the prober can drive GoodbyeDPI in a single
/// elevated invocation. Aliased as `PlatformEngine` for `main.rs`.
pub struct WindowsEngine {
    pub engine: Option<String>,
}

/// The engine type `main.rs` instantiates, identical name across platforms.
pub type PlatformEngine = WindowsEngine;

impl WindowsEngine {
    pub fn new(engine: Option<String>) -> Self {
        Self { engine }
    }
}

impl Engine for WindowsEngine {
    fn apply(&mut self, strategy: &Strategy, domains: &DomainSet) -> dpi_core::Result<()> {
        apply(strategy, &domains.domains, self.engine.as_deref())
            .map_err(|e| dpi_core::CoreError::Other(e.to_string()))
    }
    fn revert(&mut self) -> dpi_core::Result<()> {
        revert().map_err(|e| dpi_core::CoreError::Other(e.to_string()))
    }
    fn is_active(&self) -> bool {
        is_active()
    }
}

const TASK_NAME: &str = "DPI-Bypass";
const DEFAULT_EXE: &str = "goodbyedpi.exe";

/// What the always-on scheduled task applies on logon. Identical schema to the
/// Linux engine so exported profiles stay portable.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ActiveProfile {
    pub strategy: Strategy,
    pub domains: Vec<String>,
    #[serde(default)]
    pub nfqws: Option<String>,
}

/// `%ProgramData%\DPI-Bypass` — writable only by admins, survives reboot.
fn data_dir() -> PathBuf {
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(base).join("DPI-Bypass")
}
fn run_dir() -> PathBuf {
    data_dir().join("run")
}
fn pidfile() -> PathBuf {
    run_dir().join("goodbyedpi.pid")
}
fn blacklist_path() -> PathBuf {
    run_dir().join("blacklist.txt")
}
fn active_profile_path() -> PathBuf {
    data_dir().join("active-profile.json")
}

fn ensure_run_dir() -> Result<()> {
    fs::create_dir_all(run_dir()).with_context(|| format!("creating {}", run_dir().display()))?;
    Ok(())
}

/// Resolve the GoodbyeDPI binary: explicit arg > env > next to the helper exe >
/// the installed default. GoodbyeDPI needs its WinDivert .dll/.sys in the same
/// directory, which the installer guarantees.
pub fn engine_path(explicit: Option<&str>) -> String {
    if let Some(p) = explicit {
        return p.to_string();
    }
    if let Ok(p) = std::env::var("DPI_GOODBYEDPI") {
        return p;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(DEFAULT_EXE);
            if cand.exists() {
                return cand.to_string_lossy().into_owned();
            }
        }
    }
    let installed = PathBuf::from(
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string()),
    )
    .join("DPI-Bypass")
    .join(DEFAULT_EXE);
    installed.to_string_lossy().into_owned()
}

/// Normalise a domain set into a GoodbyeDPI blacklist (one host per line,
/// wildcards stripped — GoodbyeDPI matches a host and its subdomains).
fn blacklist_contents(domains: &[String]) -> String {
    let mut lines: Vec<String> = domains
        .iter()
        .map(|d| d.trim_start_matches("*.").trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();
    lines.sort();
    lines.dedup();
    lines.join("\r\n") + "\r\n"
}

/// Build the GoodbyeDPI argv: strategy flags + the domain blacklist. An empty
/// domain set means system-wide (no `--blacklist`).
pub fn goodbyedpi_argv(
    exe: &str,
    strategy: &Strategy,
    blacklist: &Path,
    domains: &[String],
) -> Vec<String> {
    let mut argv = vec![exe.to_string()];
    argv.extend(strategy.to_goodbyedpi_args());
    if !domains.is_empty() {
        argv.push("--blacklist".to_string());
        argv.push(blacklist.display().to_string());
    }
    argv
}

/// Print, without applying, exactly what `apply` would do.
pub fn plan(strategy: &Strategy, domains: &[String], engine: Option<&str>) -> String {
    let exe = engine_path(engine);
    let bl = blacklist_path();
    let argv = goodbyedpi_argv(&exe, strategy, &bl, domains);
    format!(
        "# blacklist ({}):\n{}\n# goodbyedpi command:\n{}\n",
        bl.display(),
        blacklist_contents(domains),
        argv.join(" ")
    )
}

/// Apply a strategy: write blacklist, spawn GoodbyeDPI detached, record PID.
pub fn apply(strategy: &Strategy, domains: &[String], engine: Option<&str>) -> Result<()> {
    ensure_run_dir()?;
    let exe = engine_path(engine);
    if !Path::new(&exe).exists() {
        bail!("GoodbyeDPI engine not found at {exe} (install or build it first)");
    }

    // Always start clean so re-apply is idempotent.
    let _ = revert();

    let bl = blacklist_path();
    if !domains.is_empty() {
        fs::write(&bl, blacklist_contents(domains)).context("writing blacklist")?;
    }

    let argv = goodbyedpi_argv(&exe, strategy, &bl, domains);
    let exe_dir = Path::new(&exe)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let child = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&exe_dir) // so WinDivert.dll/.sys are found
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawning {exe}"))?;

    fs::write(pidfile(), child.id().to_string()).context("writing pidfile")?;
    Ok(())
}

/// Revert: kill GoodbyeDPI (which unloads WinDivert) and remove run files.
pub fn revert() -> Result<()> {
    if let Ok(pid) = fs::read_to_string(pidfile()) {
        if let Ok(pid) = pid.trim().parse::<u32>() {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_file(pidfile());
    }
    // Belt-and-braces: kill any stray engine by image name too.
    let _ = Command::new("taskkill")
        .args(["/IM", DEFAULT_EXE, "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = fs::remove_file(blacklist_path());
    Ok(())
}

/// Active when the recorded GoodbyeDPI process is still alive.
pub fn is_active() -> bool {
    fs::read_to_string(pidfile())
        .ok()
        .and_then(|p| p.trim().parse::<u32>().ok())
        .map(pid_alive)
        .unwrap_or(false)
}

/// Whether a PID is currently running (via `tasklist`).
fn pid_alive(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
        Err(_) => false,
    }
}

/// Persist the profile the always-on task should apply.
pub fn write_active_profile(
    strategy: &Strategy,
    domains: &[String],
    engine: Option<&str>,
) -> Result<()> {
    let ap = ActiveProfile {
        strategy: strategy.clone(),
        domains: domains.to_vec(),
        nfqws: engine.map(|s| s.to_string()),
    };
    fs::create_dir_all(data_dir()).with_context(|| format!("creating {}", data_dir().display()))?;
    fs::write(active_profile_path(), serde_json::to_vec_pretty(&ap)?)?;
    Ok(())
}

/// Scheduled-task entrypoint: apply the saved profile and block on GoodbyeDPI so
/// the task host owns its lifetime.
pub fn run_daemon() -> Result<()> {
    let data = fs::read(active_profile_path())
        .context("no active profile saved; enable 'Always On' from the app first")?;
    let ap: ActiveProfile = serde_json::from_slice(&data)?;
    let exe = engine_path(ap.nfqws.as_deref());
    if !Path::new(&exe).exists() {
        bail!("GoodbyeDPI engine not found at {exe}");
    }
    ensure_run_dir()?;
    let bl = blacklist_path();
    if !ap.domains.is_empty() {
        fs::write(&bl, blacklist_contents(&ap.domains))?;
    }
    let argv = goodbyedpi_argv(&exe, &ap.strategy, &bl, &ap.domains);
    let exe_dir = Path::new(&exe)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&exe_dir)
        .status()
        .with_context(|| format!("running {exe}"))?;
    if !status.success() {
        bail!("GoodbyeDPI exited with {status}");
    }
    Ok(())
}

/// Create + start the always-on scheduled task (runs `daemon` at logon, highest
/// privileges).
pub fn enable_service() -> Result<()> {
    let exe = std::env::current_exe()
        .context("locating helper exe")?
        .to_string_lossy()
        .into_owned();
    // /RL HIGHEST = run elevated; /SC ONLOGON = at user logon; /F = overwrite.
    let cmd = format!("\"{exe}\" daemon");
    let status = Command::new("schtasks")
        .args([
            "/Create", "/TN", TASK_NAME, "/TR", &cmd, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F",
        ])
        .status()
        .context("running schtasks /Create")?;
    if !status.success() {
        bail!("schtasks /Create failed");
    }
    // Start it now so "Always On" takes effect without a re-logon.
    let _ = Command::new("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .status();
    Ok(())
}

/// Stop + delete the always-on scheduled task, and revert any live engine.
pub fn disable_service() -> Result<()> {
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", TASK_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = revert();
    Ok(())
}

/// `(enabled, active)` for the always-on task.
pub fn service_status() -> (bool, bool) {
    let out = Command::new("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            // schtasks reports "Running" while the daemon is up.
            (true, text.contains("Running"))
        }
        _ => (false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpi_core::strategy::{Strategy, TcpStrategy, UdpStrategy};

    #[test]
    fn argv_has_blacklist_when_domains_present() {
        let s = Strategy {
            tcp: TcpStrategy::default(),
            udp_quic: Some(UdpStrategy::default()),
        };
        let argv = goodbyedpi_argv(
            "goodbyedpi.exe",
            &s,
            Path::new(r"C:\tmp\bl.txt"),
            &["discord.com".into()],
        );
        assert_eq!(argv[0], "goodbyedpi.exe");
        assert!(argv.iter().any(|a| a == "--blacklist"));
    }

    #[test]
    fn argv_systemwide_has_no_blacklist() {
        let s = Strategy {
            tcp: TcpStrategy::default(),
            udp_quic: None,
        };
        let argv = goodbyedpi_argv("goodbyedpi.exe", &s, Path::new(r"C:\tmp\bl.txt"), &[]);
        assert!(!argv.iter().any(|a| a == "--blacklist"));
    }

    #[test]
    fn blacklist_strips_wildcards_and_dedups() {
        let bl = blacklist_contents(&[
            "*.discord.com".into(),
            "discord.com".into(),
            "Discord.GG".into(),
        ]);
        let lines: Vec<&str> = bl.lines().collect();
        assert!(lines.contains(&"discord.com"));
        assert!(lines.contains(&"discord.gg"));
        assert_eq!(lines.iter().filter(|l| **l == "discord.com").count(), 1);
    }
}
