// On Windows, prevent a console window from appearing in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// A release binary MUST embed the frontend via the `custom-protocol` feature.
// Without it, tauri::is_dev() is true and the webview loads devUrl
// (http://localhost:1420) — which doesn't exist on a user's machine, so the
// window shows "localhost refused to connect" (ERR_CONNECTION_REFUSED). The
// Tauri CLI enables this feature automatically; a raw `cargo build --release`
// does not. Turn that silent runtime failure into a loud build failure so no
// broken bundle can ever ship (Windows or Linux).
#[cfg(all(not(debug_assertions), not(feature = "custom-protocol")))]
compile_error!(
    "release builds require the `custom-protocol` feature. Build with \
     `cargo build --release -p dpi-bypass --features dpi-bypass/custom-protocol` \
     or `cargo tauri build`. Without it the app loads http://localhost:1420 and \
     fails with ERR_CONNECTION_REFUSED."
);

fn main() {
    dpi_bypass_lib::run();
}
