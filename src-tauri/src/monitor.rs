//! Silent background monitor (spec §10.3 / §10.4).
//!
//! A single low-frequency thread that, **without any privilege**, watches two
//! things and reports them to the UI via Tauri events:
//!
//! * **Network change** — the device's network fingerprint changing means the
//!   active profile may no longer apply; we emit `monitor://network-changed` so
//!   the UI can offer to find a new profile.
//! * **Reachability drift** — every `auto_test_interval_min` we re-probe the
//!   default profile's target (text + voice), update its stored `test_results`,
//!   and emit `monitor://retest` so the home screen reflects reality.
//!
//! The loop deliberately performs no privileged work (no helper/pkexec/UAC), so
//! it never pops an auth dialog in the background.

use crate::state::AppState;
use dpi_core::network::{check_domain, current_fingerprint, NetworkFingerprint};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const TICK: Duration = Duration::from_secs(30);

/// Spawn the monitor thread. Safe to call once at startup.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_fp = current_fingerprint();
        // Run the first reachability test ~one tick in, not immediately at boot.
        let mut last_test = Instant::now();
        loop {
            std::thread::sleep(TICK);

            // --- network change detection ---
            let fp = current_fingerprint();
            if fp != last_fp {
                last_fp = fp.clone();
                let _ = app.emit("monitor://network-changed", &fp);
            }

            // --- periodic reachability re-test ---
            let interval_min = {
                let st = app.state::<AppState>();
                let s = st.settings.lock().unwrap();
                s.auto_test_interval_min.max(1)
            };
            if last_test.elapsed() >= Duration::from_secs(interval_min as u64 * 60) {
                last_test = Instant::now();
                retest_default(&app, &fp);
            }
        }
    });
}

/// Re-probe the default profile's target and persist the result.
fn retest_default(app: &AppHandle, current_fp: &NetworkFingerprint) {
    let st = app.state::<AppState>();

    // Snapshot what we need, then drop the lock before any network IO.
    let (id, domain, with_voice, profile_fp) = {
        let store = st.store.lock().unwrap();
        match store.default_profile() {
            Some(p) => (
                p.id.clone(),
                p.domains.first().cloned().unwrap_or_default(),
                p.domains.iter().any(|d| d.contains("discord")),
                p.network_fingerprint.clone(),
            ),
            None => return,
        }
    };
    if domain.is_empty() {
        return;
    }

    // If we're on a different network than the profile was made for, a failing
    // probe is expected — skip the rewrite so we don't clobber its results.
    if !profile_fp.matches(current_fp) {
        return;
    }

    let check = check_domain(&domain, PROBE_TIMEOUT, with_voice);
    {
        let mut store = st.store.lock().unwrap();
        if let Some(p) = store.get_mut(&id) {
            p.test_results.text = check.text.is_open();
            p.test_results.voice = check.voice.map(|v| v.is_open()).unwrap_or(false);
            p.test_results.last_checked = Some(chrono::Utc::now());
        }
    }
    let _ = st.save_store();
    let _ = app.emit(
        "monitor://retest",
        serde_json::json!({ "id": id, "check": check }),
    );
}
