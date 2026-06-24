//! Tauri command bridges — the only surface the frontend can call.
//!
//! Unprivileged work (reachability probes, network fingerprint, profile CRUD)
//! runs in-process via `dpi-core`. Privileged work (solve, apply, revert,
//! service toggle) is delegated to `dpi-bypass-helper` through [`crate::helper`].

use crate::helper;
use crate::state::{AppState, Settings};
use dpi_core::engine::DomainSet;
use dpi_core::network::{check_domain, current_fingerprint, DomainCheck, NetworkFingerprint};
use dpi_core::prober::ProbeOutcome;
use dpi_core::profiles::{export_profile, import_profile, Profile, TestResults};
use dpi_core::strategy::Strategy;
use std::time::Duration;
use tauri::State;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Probe a domain's reachability (text + optional voice). No privilege needed.
#[tauri::command]
pub fn check_domain_cmd(domain: String, with_voice: bool) -> DomainCheck {
    check_domain(&domain, PROBE_TIMEOUT, with_voice)
}

/// The current network fingerprint.
#[tauri::command]
pub fn network_fingerprint() -> NetworkFingerprint {
    current_fingerprint()
}

/// Run the auto-solver for a domain set via the privileged helper. Returns the
/// solver outcome (already-open / found strategy / not found).
#[tauri::command]
pub fn solve(
    domains: Vec<String>,
    with_voice: bool,
    nfqws: Option<String>,
) -> Result<ProbeOutcome, String> {
    let mut args = vec!["--domains".to_string(), domains.join(",")];
    if with_voice {
        args.push("--voice".into());
        args.push("1".into());
    }
    if let Some(p) = nfqws {
        args.push("--nfqws".into());
        args.push(p);
    }
    let out = helper::run("solve", &args)?;
    serde_json::from_str(out.trim()).map_err(|e| format!("bad solver output: {e}\n{out}"))
}

/// Persist a freshly found strategy as a new profile bound to the current
/// network. Used right after `solve` returns `Found`.
#[tauri::command]
pub fn create_profile(
    state: State<AppState>,
    domains: Vec<String>,
    strategy: Strategy,
    check: DomainCheck,
) -> Result<Profile, String> {
    let fp = current_fingerprint();
    let mut store = state.store.lock().unwrap();
    let (id, name) = store.next_id();
    let mut profile = Profile::new(id, name, domains, strategy, fp);
    profile.test_results = TestResults {
        text: check.text.is_open(),
        voice: check.voice.map(|v| v.is_open()).unwrap_or(false),
        last_checked: Some(chrono::Utc::now()),
    };
    store.add(profile.clone());
    drop(store);
    state.save_store().map_err(|e| e.to_string())?;
    Ok(profile)
}

#[tauri::command]
pub fn list_profiles(state: State<AppState>) -> Vec<Profile> {
    state.store.lock().unwrap().profiles.clone()
}

#[tauri::command]
pub fn default_profile_id(state: State<AppState>) -> Option<String> {
    state.store.lock().unwrap().default_id.clone()
}

#[tauri::command]
pub fn rename_profile(state: State<AppState>, id: String, name: String) -> Result<(), String> {
    state
        .store
        .lock()
        .unwrap()
        .rename(&id, &name)
        .map_err(|e| e.to_string())?;
    state.save_store().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<(), String> {
    state
        .store
        .lock()
        .unwrap()
        .remove(&id)
        .map_err(|e| e.to_string())?;
    state.save_store().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_default_profile(state: State<AppState>, id: String) -> Result<(), String> {
    state
        .store
        .lock()
        .unwrap()
        .set_default(&id)
        .map_err(|e| e.to_string())?;
    state.save_store().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_profile_cmd(state: State<AppState>, id: String) -> Result<String, String> {
    let store = state.store.lock().unwrap();
    let p = store
        .get(&id)
        .ok_or_else(|| format!("profile not found: {id}"))?;
    export_profile(p).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_profile_cmd(state: State<AppState>, json: String) -> Result<Profile, String> {
    let mut store = state.store.lock().unwrap();
    let p = import_profile(&store, &json).map_err(|e| e.to_string())?;
    store.add(p.clone());
    drop(store);
    state.save_store().map_err(|e| e.to_string())?;
    Ok(p)
}

/// Turn a profile on: apply its strategy via the helper.
#[tauri::command]
pub fn engine_apply(state: State<AppState>, id: String) -> Result<(), String> {
    let store = state.store.lock().unwrap();
    let p = store
        .get(&id)
        .ok_or_else(|| format!("profile not found: {id}"))?;
    let strategy_json = serde_json::to_string(&p.strategy).map_err(|e| e.to_string())?;
    let domains = p.domains.join(",");
    drop(store);
    helper::run(
        "apply",
        &[
            "--strategy".into(),
            strategy_json,
            "--domains".into(),
            domains,
        ],
    )?;
    Ok(())
}

/// Turn the engine off (full revert / kill-switch).
#[tauri::command]
pub fn engine_revert() -> Result<(), String> {
    helper::run("revert", &[])?;
    Ok(())
}

/// Whether a strategy is currently applied.
#[tauri::command]
pub fn engine_status() -> bool {
    helper::run("status", &[])
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s.trim()).ok())
        .and_then(|v| v.get("active").and_then(|a| a.as_bool()))
        .unwrap_or(false)
}

/// "Always on": enable/disable the systemd service. When enabling, the default
/// profile's strategy is handed to the helper so the boot-time daemon can apply
/// it.
#[tauri::command]
pub fn set_always_on(state: State<AppState>, enabled: bool) -> Result<(), String> {
    if !enabled {
        helper::run("disable-service", &[])?;
        return Ok(());
    }
    let store = state.store.lock().unwrap();
    let p = store
        .default_profile()
        .ok_or_else(|| "no default profile to enable".to_string())?;
    let strategy_json = serde_json::to_string(&p.strategy).map_err(|e| e.to_string())?;
    let domains = p.domains.join(",");
    drop(store);
    helper::run(
        "enable-service",
        &[
            "--strategy".into(),
            strategy_json,
            "--domains".into(),
            domains,
        ],
    )?;
    Ok(())
}

#[tauri::command]
pub fn service_status() -> serde_json::Value {
    helper::run("service-status", &[])
        .ok()
        .and_then(|s| serde_json::from_str(s.trim()).ok())
        .unwrap_or_else(|| serde_json::json!({"enabled": false, "active": false}))
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_settings(
    app: tauri::AppHandle,
    state: State<AppState>,
    settings: Settings,
) -> Result<(), String> {
    // Apply the OS-level autostart registration to match the setting.
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    let res = if settings.autostart {
        autostart.enable()
    } else {
        autostart.disable()
    };
    if let Err(e) = res {
        // Non-fatal: persist the setting anyway, but report the failure.
        log::warn!("autostart toggle failed: {e}");
    }

    *state.settings.lock().unwrap() = settings;
    state.save_settings().map_err(|e| e.to_string())
}

/// Edit a profile's strategy in place (advanced editor, spec §8 / Faz 4).
#[tauri::command]
pub fn update_profile_strategy(
    state: State<AppState>,
    id: String,
    strategy: Strategy,
) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        let p = store
            .get_mut(&id)
            .ok_or_else(|| format!("profile not found: {id}"))?;
        p.strategy = strategy;
    }
    state.save_store().map_err(|e| e.to_string())
}

/// Replace a profile's domain set (advanced editor).
#[tauri::command]
pub fn update_profile_domains(
    state: State<AppState>,
    id: String,
    domains: Vec<String>,
) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        let p = store
            .get_mut(&id)
            .ok_or_else(|| format!("profile not found: {id}"))?;
        p.domains = domains;
    }
    state.save_store().map_err(|e| e.to_string())
}

/// The default Discord domain set, surfaced to the UI as a starting point.
#[tauri::command]
pub fn discord_domains() -> Vec<String> {
    DomainSet::discord().domains
}
