//! Bridge from the unprivileged GUI to the privileged `dpi-bypass-helper`.
//!
//! Privileged operations (packet filters, the desync engine, the always-on
//! service) are delegated to the small `dpi-bypass-helper` binary so the bulk of
//! the GUI never runs with elevation.
//!
//! * **Linux** — the helper is invoked through `pkexec`, so the OS shows its own
//!   authentication dialog; a polkit rule (`AUTH_ADMIN_KEEP`) caches the grant
//!   for the session. If the GUI already runs as root (systemd/`sudo` during
//!   dev) we skip `pkexec`.
//! * **Windows** — the GUI is shipped with a `requireAdministrator` manifest, so
//!   it (and therefore the helper) already runs elevated; the helper is invoked
//!   directly.

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
const HELPER_EXE: &str = "dpi-bypass-helper.exe";
#[cfg(unix)]
const HELPER_EXE: &str = "dpi-bypass-helper";

/// Locate the helper binary: explicit env override, installed path, or the dev
/// build output next to the running GUI.
fn helper_path() -> PathBuf {
    if let Ok(p) = std::env::var("DPI_HELPER") {
        return PathBuf::from(p);
    }

    #[cfg(unix)]
    let installed = PathBuf::from("/usr/lib/dpi-bypass/dpi-bypass-helper");
    #[cfg(windows)]
    let installed = PathBuf::from(
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string()),
    )
    .join("DPI-Bypass")
    .join(HELPER_EXE);

    if installed.exists() {
        return installed;
    }
    // Dev fallback: helper next to this binary.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(HELPER_EXE);
            if cand.exists() {
                return cand;
            }
        }
    }
    installed
}

/// Whether the GUI itself already runs elevated, in which case `pkexec` is not
/// needed. On Windows the manifest guarantees elevation, so this is always true.
#[cfg(unix)]
fn is_elevated() -> bool {
    // SAFETY: geteuid is always safe to call.
    unsafe { libc_geteuid() == 0 }
}
#[cfg(windows)]
fn is_elevated() -> bool {
    true
}

// Avoid pulling the whole libc crate for a single call.
#[cfg(unix)]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Run a helper verb, returning its stdout on success.
pub fn run(verb: &str, extra: &[String]) -> Result<String, String> {
    let helper = helper_path();

    #[cfg(unix)]
    let mut cmd = if is_elevated() {
        let mut c = Command::new(&helper);
        c.arg(verb).args(extra);
        c
    } else {
        let mut c = Command::new("pkexec");
        c.arg(&helper).arg(verb).args(extra);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let _ = is_elevated();
        let mut c = Command::new(&helper);
        c.arg(verb).args(extra);
        c
    };

    let out = cmd
        .output()
        .map_err(|e| format!("failed to run helper: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(if stderr.trim().is_empty() {
            format!("helper '{verb}' failed (exit {:?})", out.status.code())
        } else {
            stderr.into_owned()
        })
    }
}
