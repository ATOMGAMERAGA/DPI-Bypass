//! DPI-Bypass Tauri application entry point.

mod commands;
mod helper;
mod monitor;
mod state;

use state::AppState;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState::init())
        .setup(|app| {
            build_tray(app)?;
            // Silent background monitor: network-change + reachability drift.
            monitor::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::check_domain_cmd,
            commands::network_fingerprint,
            commands::solve,
            commands::create_profile,
            commands::list_profiles,
            commands::default_profile_id,
            commands::rename_profile,
            commands::delete_profile,
            commands::set_default_profile,
            commands::export_profile_cmd,
            commands::import_profile_cmd,
            commands::update_profile_strategy,
            commands::update_profile_domains,
            commands::engine_apply,
            commands::engine_revert,
            commands::engine_status,
            commands::set_always_on,
            commands::service_status,
            commands::get_settings,
            commands::set_settings,
            commands::discord_domains,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DPI-Bypass");
}

/// System tray: status icon + quick menu (show / toggle / quit).
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Göster", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Atlatmayı Kapat", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("DPI-Bypass")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "toggle" => {
                // Best-effort turn-off from the tray; the UI reflects real state
                // on next refresh.
                let _ = commands::engine_revert();
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // Regression guard for the "localhost refused to connect" bug.
    //
    // Tauri serves devUrl (http://localhost:1420) whenever it is in dev mode,
    // and dev mode is exactly `!cfg!(feature = "custom-protocol")`. So a
    // production build (custom-protocol on) MUST report not-dev; if it doesn't,
    // the shipped webview would load localhost and users get
    // ERR_CONNECTION_REFUSED. This test only exists when the feature is enabled,
    // so `cargo test --workspace` (no feature) skips it; CI runs it explicitly
    // with `--features dpi-bypass/custom-protocol`.
    #[cfg(feature = "custom-protocol")]
    #[test]
    fn production_build_serves_embedded_frontend() {
        assert!(
            !tauri::is_dev(),
            "custom-protocol is enabled but tauri::is_dev() is still true: the \
             webview would load http://localhost:1420 instead of the bundled UI."
        );
    }
}
