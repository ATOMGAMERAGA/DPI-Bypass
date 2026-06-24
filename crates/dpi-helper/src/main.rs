//! `dpi-bypass-helper` — the privileged worker.
//!
//! The unprivileged GUI invokes this for the few operations that need elevation:
//! installing packet filters, launching the desync engine, and toggling the
//! "always on" service. On Linux it is run via `pkexec` (nftables + nfqws); on
//! Windows it runs elevated (WinDivert + GoodbyeDPI) — the GUI is marked
//! `requireAdministrator`. Keeping these in a tiny, auditable binary is the
//! privilege separation the spec (§15) calls for.
//!
//! Usage:
//!   dpi-bypass-helper plan   --strategy <json> --domains a,b,c [--nfqws PATH]
//!   dpi-bypass-helper apply  --strategy <json> --domains a,b,c [--nfqws PATH]
//!   dpi-bypass-helper revert
//!   dpi-bypass-helper solve  --domains a,b,c [--voice 1] [--nfqws PATH]
//!   dpi-bypass-helper status
//!   dpi-bypass-helper daemon
//!   dpi-bypass-helper enable-service  [--strategy <json> --domains a,b,c]
//!   dpi-bypass-helper disable-service
//!   dpi-bypass-helper service-status
//!
//! `status` / `service-status` / `solve` print a JSON line so the GUI can parse
//! them. `--nfqws` names the engine binary (nfqws on Linux, goodbyedpi.exe on
//! Windows); it is optional and defaults to the installed path.

// The platform engine: Linux uses nftables + nfqws (zapret core); Windows uses
// WinDivert + GoodbyeDPI. Both expose the same module-level API consumed below.
#[cfg(unix)]
#[path = "engine_linux.rs"]
mod engine;
#[cfg(windows)]
#[path = "engine_windows.rs"]
mod engine;

use anyhow::{bail, Context, Result};
use dpi_core::engine::DomainSet;
use dpi_core::network::check_domain;
use dpi_core::prober::find_strategy;
use dpi_core::strategy::Strategy;
use std::collections::HashMap;
use std::time::Duration;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let verb = args.next().unwrap_or_default();
    let opts = parse_opts(args.collect());

    match verb.as_str() {
        "plan" => {
            let (strategy, domains) = strategy_and_domains(&opts)?;
            print!(
                "{}",
                engine::plan(&strategy, &domains, opts.get("nfqws").map(|s| s.as_str()))
            );
        }
        "apply" => {
            let (strategy, domains) = strategy_and_domains(&opts)?;
            engine::apply(&strategy, &domains, opts.get("nfqws").map(|s| s.as_str()))?;
            println!("{{\"ok\":true}}");
        }
        "revert" => {
            engine::revert()?;
            println!("{{\"ok\":true}}");
        }
        "solve" => {
            // Run the whole auto-solver privileged, in one shot: apply each
            // candidate, probe, revert, return the winner as JSON.
            let domains: Vec<String> = opts
                .get("domains")
                .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            let with_voice = opts.contains_key("voice");
            let mut eng = engine::PlatformEngine::new(opts.get("nfqws").cloned());
            let ds = DomainSet::new(domains);
            let outcome = find_strategy(&mut eng, &ds, with_voice, |d, v| {
                check_domain(d, Duration::from_secs(3), v)
            })?;
            println!("{}", serde_json::to_string(&outcome)?);
        }
        "status" => {
            println!("{{\"active\":{}}}", engine::is_active());
        }
        "daemon" => {
            // Service ExecStart / scheduled-task entrypoint (ignores any trailing
            // flags such as --profile-default, kept for unit readability).
            engine::run_daemon()?;
        }
        "enable-service" => {
            // Persist the profile the service should apply on boot, when given.
            if opts.contains_key("strategy") {
                let (strategy, domains) = strategy_and_domains(&opts)?;
                engine::write_active_profile(
                    &strategy,
                    &domains,
                    opts.get("nfqws").map(|s| s.as_str()),
                )?;
            }
            engine::enable_service()?;
            println!("{{\"ok\":true}}");
        }
        "disable-service" => {
            engine::disable_service()?;
            println!("{{\"ok\":true}}");
        }
        "service-status" => {
            let (enabled, active) = engine::service_status();
            println!("{{\"enabled\":{enabled},\"active\":{active}}}");
        }
        other => bail!("unknown verb: {other}"),
    }
    Ok(())
}

fn strategy_and_domains(opts: &HashMap<String, String>) -> Result<(Strategy, Vec<String>)> {
    let json = opts
        .get("strategy")
        .context("--strategy <json> is required")?;
    let strategy: Strategy = serde_json::from_str(json).context("parsing --strategy JSON")?;
    let domains = opts
        .get("domains")
        .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    Ok((strategy, domains))
}

/// Minimal `--key value` parser (values may be JSON, so we don't use `=`).
fn parse_opts(args: Vec<String>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            if let Some(val) = args.get(i + 1) {
                map.insert(key.to_string(), val.clone());
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    map
}
