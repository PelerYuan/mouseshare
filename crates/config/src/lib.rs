//! Everything mouseshare remembers between runs, in plain TOML under
//! `~/.config/mouseshare/` (or `$XDG_CONFIG_HOME/mouseshare/`):
//!
//! * `settings.toml` -- preferences, the device book and the arrangement.
//! * `pairing.toml`  -- pairing codes (secrets; written mode 0600).
//!
//! The CLI and the GUI read and write the very same files, so a layout set
//! up in the GUI can be run headless later with `mouseshare controller`.

mod autostart;
mod pairing_store;
mod settings;

pub use autostart::{autostart_path, set_autostart};
pub use pairing_store::PairingStore;
pub use settings::{
    Accent, DeviceRecord, Language, MonitorRecord, Role, Settings, ThemeMode, DEFAULT_PORT,
};

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error on {0}: {1}")]
    Io(PathBuf, io::Error),
    #[error("could not serialize: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// The directory all mouseshare state lives in. Not created here.
pub fn config_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(x).join("mouseshare");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config").join("mouseshare")
}

/// Best-effort machine name used as the default device id.
pub fn default_device_name() -> String {
    let raw = std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_default();
    let cleaned = sanitize_device_name(&raw);
    if cleaned.is_empty() {
        "my-computer".into()
    } else {
        cleaned
    }
}

/// Device ids end up in mDNS names and TOML keys, so keep them tame:
/// letters, digits, `-`, `_` and `.` only, at most 40 chars.
pub fn sanitize_device_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Writes atomically (temp file in the same directory, then rename) so a
/// crash mid-write can never leave a truncated settings file behind.
pub(crate) fn write_atomic(path: &Path, contents: &str, secret: bool) -> Result<(), ConfigError> {
    let io_err = |e| ConfigError::Io(path.to_path_buf(), e);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(io_err)?;
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(io_err)?;
    }
    #[cfg(not(unix))]
    let _ = secret;
    std::fs::rename(&tmp, path).map_err(io_err)
}

/// Reads a TOML file, tolerating absence (returns `None`) and corruption
/// (moves the broken file aside to `*.bak` and returns `None`) so a bad
/// file never keeps the app from starting.
pub(crate) fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    match toml::from_str(&raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("{} is unreadable ({e}); starting fresh", path.display());
            let _ = std::fs::rename(path, path.with_extension("bak"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names_are_sanitized() {
        assert_eq!(sanitize_device_name("  My Laptop\n"), "My-Laptop");
        assert_eq!(sanitize_device_name("a/b c"), "a-b-c");
        assert_eq!(sanitize_device_name("!!!"), "");
        assert_eq!(sanitize_device_name(&"x".repeat(100)).len(), 40);
    }
}
