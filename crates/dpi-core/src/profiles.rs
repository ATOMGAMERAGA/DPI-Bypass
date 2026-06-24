//! Profiles: a working strategy bound to the network it was found on, persisted
//! as JSON. Mirrors the schema in the project spec (§8).

use crate::network::NetworkFingerprint;
use crate::strategy::Strategy;
use crate::{CoreError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Last-known check results for a profile's target.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestResults {
    #[serde(default)]
    pub text: bool,
    #[serde(default)]
    pub voice: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked: Option<DateTime<Utc>>,
}

/// A saved, working configuration for one network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub network_fingerprint: NetworkFingerprint,
    pub domains: Vec<String>,
    pub strategy: Strategy,
    #[serde(default)]
    pub test_results: TestResults,
    #[serde(default = "default_interval")]
    pub auto_test_interval_min: u32,
}

fn default_interval() -> u32 {
    30
}

impl Profile {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        domains: Vec<String>,
        strategy: Strategy,
        fingerprint: NetworkFingerprint,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            created_at: Utc::now(),
            network_fingerprint: fingerprint,
            domains,
            strategy,
            test_results: TestResults::default(),
            auto_test_interval_min: default_interval(),
        }
    }
}

/// On-disk collection of profiles plus the id of the default one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileStore {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_id: Option<String>,
}

impl ProfileStore {
    /// Load from `path`, returning an empty store if the file does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist atomically (write to a temp file, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Allocate the next free `profile-N` id and `Profil N` default name.
    pub fn next_id(&self) -> (String, String) {
        let n = self.profiles.len() + 1;
        let mut idx = n;
        loop {
            let id = format!("profile-{idx}");
            if !self.profiles.iter().any(|p| p.id == id) {
                return (id, format!("Profil {idx}"));
            }
            idx += 1;
        }
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Profile> {
        self.profiles.iter_mut().find(|p| p.id == id)
    }

    pub fn add(&mut self, profile: Profile) {
        if self.default_id.is_none() {
            self.default_id = Some(profile.id.clone());
        }
        self.profiles.push(profile);
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        if self.profiles.len() == before {
            return Err(CoreError::ProfileNotFound(id.into()));
        }
        if self.default_id.as_deref() == Some(id) {
            self.default_id = self.profiles.first().map(|p| p.id.clone());
        }
        Ok(())
    }

    pub fn rename(&mut self, id: &str, new_name: &str) -> Result<()> {
        let p = self
            .get_mut(id)
            .ok_or_else(|| CoreError::ProfileNotFound(id.into()))?;
        p.name = new_name.to_string();
        Ok(())
    }

    pub fn set_default(&mut self, id: &str) -> Result<()> {
        if self.get(id).is_none() {
            return Err(CoreError::ProfileNotFound(id.into()));
        }
        self.default_id = Some(id.to_string());
        Ok(())
    }

    /// The default profile, falling back to the first if none is set.
    pub fn default_profile(&self) -> Option<&Profile> {
        self.default_id
            .as_ref()
            .and_then(|id| self.get(id))
            .or_else(|| self.profiles.first())
    }

    /// Find a profile whose stored fingerprint matches the current network.
    pub fn matching(&self, current: &NetworkFingerprint) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.network_fingerprint.matches(current))
    }
}

/// Export a single profile to a shareable JSON string.
pub fn export_profile(p: &Profile) -> Result<String> {
    Ok(serde_json::to_string_pretty(p)?)
}

/// Import a profile from JSON, assigning it a fresh id within `store`.
pub fn import_profile(store: &ProfileStore, json: &str) -> Result<Profile> {
    let mut p: Profile = serde_json::from_str(json)?;
    let (id, _) = store.next_id();
    p.id = id;
    Ok(p)
}

/// Default location for the profile store under the user's config dir.
pub fn default_store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("dpi-bypass").join("profiles.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::Strategy;
    use crate::strategy::TcpStrategy;

    fn sample() -> Profile {
        Profile::new(
            "profile-1",
            "Profil 1",
            vec!["discord.com".into()],
            Strategy {
                tcp: TcpStrategy::default(),
                udp_quic: None,
            },
            NetworkFingerprint {
                gateway_mac: Some("aa:bb:cc:dd:ee:ff".into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let mut store = ProfileStore::default();
        store.add(sample());
        store.save(&path).unwrap();

        let loaded = ProfileStore::load(&path).unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.default_id.as_deref(), Some("profile-1"));
        assert_eq!(loaded.profiles[0].name, "Profil 1");
    }

    #[test]
    fn next_id_is_unique() {
        let mut store = ProfileStore::default();
        store.add(sample());
        let (id, name) = store.next_id();
        assert_eq!(id, "profile-2");
        assert_eq!(name, "Profil 2");
    }

    #[test]
    fn remove_reassigns_default() {
        let mut store = ProfileStore::default();
        store.add(sample());
        let mut p2 = sample();
        p2.id = "profile-2".into();
        store.add(p2);
        store.remove("profile-1").unwrap();
        assert_eq!(store.default_id.as_deref(), Some("profile-2"));
    }

    #[test]
    fn export_import_assigns_new_id() {
        let mut store = ProfileStore::default();
        store.add(sample());
        let json = export_profile(&store.profiles[0]).unwrap();
        let imported = import_profile(&store, &json).unwrap();
        assert_eq!(imported.id, "profile-2");
    }
}
