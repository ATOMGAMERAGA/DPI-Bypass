//! Shared application state: the profile store, user settings, and their paths.

use dpi_core::profiles::{default_store_path, ProfileStore};
use dpi_core::Result as CoreResult;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// User-facing settings, persisted alongside the profile store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// `"tr"` or `"en"`.
    pub language: String,
    /// `"dark"` or `"light"`.
    pub theme: String,
    /// Windows scope mode (`discord` / `browsers` / `all_browsers` / `system`).
    /// Ignored on Linux, which is always system-wide.
    pub scope: String,
    /// Minutes between silent background re-tests.
    pub auto_test_interval_min: u32,
    /// Launch the GUI at login.
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "tr".into(),
            theme: "dark".into(),
            scope: "discord".into(),
            auto_test_interval_min: 30,
            autostart: false,
        }
    }
}

impl Settings {
    fn path(config_dir: &std::path::Path) -> PathBuf {
        config_dir.join("dpi-bypass").join("settings.json")
    }

    fn load(config_dir: &std::path::Path) -> Self {
        match std::fs::read(Self::path(config_dir)) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn save(&self, config_dir: &std::path::Path) -> std::io::Result<()> {
        let path = Self::path(config_dir);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap())
    }
}

/// Global app state managed by Tauri.
pub struct AppState {
    pub config_dir: PathBuf,
    pub store_path: PathBuf,
    pub store: Mutex<ProfileStore>,
    pub settings: Mutex<Settings>,
}

impl AppState {
    /// Initialise from the OS config directory, loading persisted data.
    pub fn init() -> Self {
        let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let store_path = default_store_path(&config_dir);
        let store = ProfileStore::load(&store_path).unwrap_or_default();
        let settings = Settings::load(&config_dir);
        Self {
            config_dir,
            store_path,
            store: Mutex::new(store),
            settings: Mutex::new(settings),
        }
    }

    /// Persist the current profile store.
    pub fn save_store(&self) -> CoreResult<()> {
        let store = self.store.lock().unwrap();
        store.save(&self.store_path)
    }

    /// Persist the current settings.
    pub fn save_settings(&self) -> std::io::Result<()> {
        let settings = self.settings.lock().unwrap();
        settings.save(&self.config_dir)
    }
}
