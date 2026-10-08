//! Start-on-login via an XDG autostart `.desktop` entry.

use std::path::PathBuf;

use crate::ConfigError;

pub fn autostart_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config")
        });
    base.join("autostart").join("mouseshare.desktop")
}

/// Creates or removes the autostart entry. `exec` is the command to run.
pub fn set_autostart(enabled: bool, exec: &str) -> Result<(), ConfigError> {
    let path = autostart_path();
    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ConfigError::Io(path, e)),
        };
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=mouseshare\n\
         Comment=Share one mouse and keyboard across your computers\n\
         Exec={exec}\nIcon=mouseshare\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    crate::write_atomic(&path, &entry, false)
}
