//! Pairing codes. Kept apart from `settings.toml` so the preferences file
//! can be shared or posted in a bug report without leaking a secret.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mouseshare_core::PairingCode;
use serde::{Deserialize, Serialize};

use crate::{config_dir, read_toml, write_atomic, ConfigError};

#[derive(Default, Serialize, Deserialize)]
struct Raw {
    /// The code other devices must enter to control this one.
    #[serde(default)]
    own: Option<String>,
    /// Codes this device uses to reach each remote device, by device id.
    #[serde(default)]
    peers: BTreeMap<String, String>,
}

pub struct PairingStore {
    path: PathBuf,
    raw: Raw,
}

impl PairingStore {
    pub fn default_path() -> PathBuf {
        config_dir().join("pairing.toml")
    }

    pub fn load() -> Self {
        Self::load_from(Self::default_path())
    }

    pub fn load_from(path: PathBuf) -> Self {
        let raw = read_toml(&path).unwrap_or_default();
        Self { path, raw }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// This device's own code, generating and persisting one on first use.
    pub fn own_code(&mut self) -> PairingCode {
        if let Some(code) = self
            .raw
            .own
            .as_deref()
            .and_then(|s| PairingCode::parse(s).ok())
        {
            return code;
        }
        let code = PairingCode::generate();
        self.raw.own = Some(code.display());
        let _ = self.save();
        code
    }

    /// Replaces this device's own code with a fresh one.
    pub fn regenerate_own(&mut self) -> PairingCode {
        let code = PairingCode::generate();
        self.raw.own = Some(code.display());
        let _ = self.save();
        code
    }

    pub fn peer_code(&self, id: &str) -> Option<PairingCode> {
        self.raw
            .peers
            .get(id)
            .and_then(|s| PairingCode::parse(s).ok())
    }

    pub fn set_peer_code(&mut self, id: &str, code: &PairingCode) {
        self.raw.peers.insert(id.to_string(), code.display());
        let _ = self.save();
    }

    pub fn forget_peer(&mut self, id: &str) {
        if self.raw.peers.remove(id).is_some() {
            let _ = self.save();
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        write_atomic(&self.path, &toml::to_string_pretty(&self.raw)?, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_code_is_stable_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pairing.toml");
        let mut a = PairingStore::load_from(path.clone());
        let c1 = a.own_code();
        let mut b = PairingStore::load_from(path.clone());
        assert_eq!(b.own_code().display(), c1.display());
        b.set_peer_code("desk", &c1);
        let c = PairingStore::load_from(path.clone());
        assert_eq!(c.peer_code("desk").unwrap().display(), c1.display());
        assert!(c.peer_code("nope").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn regenerate_changes_the_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = PairingStore::load_from(dir.path().join("p.toml"));
        let a = s.own_code().display();
        let b = s.regenerate_own().display();
        assert_ne!(a, b);
    }
}
