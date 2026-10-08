use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use mouseshare_core::{Hotkey, SessionOptions};
use mouseshare_layout::{Device, LayoutConfig, Monitor};
use serde::{Deserialize, Serialize};

use crate::{config_dir, default_device_name, read_toml, write_atomic, ConfigError};

pub const DEFAULT_PORT: u16 = 7878;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// This computer owns the mouse and keyboard.
    #[default]
    Controller,
    /// This computer is driven by another one.
    Target,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Blue,
    Violet,
    Teal,
    Green,
    Orange,
    Pink,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Blue,
        Accent::Violet,
        Accent::Teal,
        Accent::Green,
        Accent::Orange,
        Accent::Pink,
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorRecord {
    #[serde(default)]
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A device in the arrangement, as remembered between runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub id: String,
    /// Origin in the shared virtual desktop.
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    /// Last known monitors (refreshed from the device on every connect).
    #[serde(default)]
    pub monitors: Vec<MonitorRecord>,
    /// Last known `host:port`, if the device is dialled by address rather
    /// than found by mDNS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    /// Pointer speed multiplier when this device is being controlled.
    #[serde(default = "one")]
    pub sensitivity: f32,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_hotkey() -> String {
    Hotkey::default().to_string()
}

fn default_clip_kb() -> u32 {
    256
}

fn default_scale() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// This computer's name in the arrangement (and on the network).
    pub device_name: String,
    pub role: Role,
    /// Set once the first-run wizard has been completed.
    pub onboarded: bool,
    pub language: Language,
    pub theme: ThemeMode,
    pub accent: Accent,
    /// UI zoom factor, 0.8 ..= 1.6.
    #[serde(default = "default_scale")]
    pub ui_scale: f32,
    #[serde(default = "default_port")]
    pub listen_port: u16,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    pub edge_dwell_ms: u32,
    #[serde(default = "yes")]
    pub hold_guard: bool,
    #[serde(default = "yes")]
    pub clipboard: bool,
    #[serde(default = "default_clip_kb")]
    pub clipboard_max_kb: u32,
    pub natural_scroll: bool,
    /// Launch the app when the user logs in.
    pub autostart: bool,
    /// Begin sharing as soon as the app opens.
    pub start_sharing_on_launch: bool,
    /// The arrangement, including this computer.
    pub devices: Vec<DeviceRecord>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: default_device_name(),
            role: Role::default(),
            onboarded: false,
            language: Language::default(),
            theme: ThemeMode::default(),
            accent: Accent::default(),
            ui_scale: 1.0,
            listen_port: DEFAULT_PORT,
            hotkey: default_hotkey(),
            edge_dwell_ms: 0,
            hold_guard: true,
            clipboard: true,
            clipboard_max_kb: 256,
            natural_scroll: false,
            autostart: false,
            start_sharing_on_launch: false,
            devices: Vec::new(),
        }
    }
}

impl Settings {
    pub fn default_path() -> PathBuf {
        config_dir().join("settings.toml")
    }

    /// Loads the saved settings, or defaults if none/unreadable.
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        let mut s: Settings = read_toml(path).unwrap_or_default();
        s.normalize();
        s
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::default_path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        write_atomic(path, &toml::to_string_pretty(self)?, false)
    }

    /// Clamps hand-edited values into sane ranges.
    pub fn normalize(&mut self) {
        self.ui_scale = if self.ui_scale.is_finite() {
            self.ui_scale.clamp(0.8, 1.6)
        } else {
            1.0
        };
        let name = crate::sanitize_device_name(&self.device_name);
        self.device_name = if name.is_empty() {
            default_device_name()
        } else {
            name
        };
        if self.listen_port == 0 {
            self.listen_port = DEFAULT_PORT;
        }
        self.clipboard_max_kb = self.clipboard_max_kb.clamp(1, 900);
        self.edge_dwell_ms = self.edge_dwell_ms.min(2000);
        for d in &mut self.devices {
            d.sensitivity = if d.sensitivity.is_finite() {
                d.sensitivity.clamp(0.25, 4.0)
            } else {
                1.0
            };
        }
        // Exactly one record per id.
        let mut seen = std::collections::HashSet::new();
        self.devices.retain(|d| seen.insert(d.id.clone()));
    }

    pub fn device(&self, id: &str) -> Option<&DeviceRecord> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn device_mut(&mut self, id: &str) -> Option<&mut DeviceRecord> {
        self.devices.iter_mut().find(|d| d.id == id)
    }

    /// Hotkey from the setting, falling back to the default if unparsable.
    pub fn hotkey(&self) -> Hotkey {
        Hotkey::parse(&self.hotkey).unwrap_or_default()
    }

    pub fn session_options(&self) -> SessionOptions {
        let sensitivity: HashMap<String, f32> = self
            .devices
            .iter()
            .map(|d| (d.id.clone(), d.sensitivity))
            .collect();
        SessionOptions {
            hotkey: Some(self.hotkey()),
            edge_dwell: Duration::from_millis(self.edge_dwell_ms as u64),
            hold_guard: self.hold_guard,
            clipboard: self.clipboard,
            clipboard_max_bytes: self.clipboard_max_kb as usize * 1024,
            natural_scroll: self.natural_scroll,
            sensitivity,
        }
    }

    /// The arrangement as a [`LayoutConfig`] with this computer as the
    /// local device. Devices without known monitors get a 1920x1080
    /// placeholder until they connect and report the real ones.
    pub fn to_layout(&self) -> Result<LayoutConfig, mouseshare_layout::LayoutError> {
        let mut devices: Vec<Device> = self
            .devices
            .iter()
            .map(|d| Device {
                id: d.id.clone(),
                x: d.x,
                y: d.y,
                monitors: if d.monitors.is_empty() {
                    vec![Monitor::new("", 0, 0, 1920, 1080)]
                } else {
                    d.monitors
                        .iter()
                        .map(|m| Monitor::new(m.name.clone(), m.x, m.y, m.width, m.height))
                        .collect()
                },
            })
            .collect();
        if !devices.iter().any(|d| d.id == self.device_name) {
            devices.insert(
                0,
                Device::single(self.device_name.clone(), 0, 0, 1920, 1080),
            );
        }
        let layout = LayoutConfig {
            local_id: self.device_name.clone(),
            devices,
        };
        layout.validate()?;
        Ok(layout)
    }

    /// Dial address for a remote device, if one is saved and valid.
    pub fn addr_of(&self, id: &str) -> Option<SocketAddr> {
        self.device(id)?.addr.as_deref()?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_survives_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut s = Settings {
            device_name: "desk".into(),
            theme: ThemeMode::Dark,
            accent: Accent::Teal,
            language: Language::ZhCn,
            ..Settings::default()
        };
        s.devices.push(DeviceRecord {
            id: "desk".into(),
            x: 0,
            y: 0,
            monitors: vec![MonitorRecord {
                name: "DP-1".into(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            }],
            addr: None,
            sensitivity: 1.0,
        });
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);

        // A file written by an older/newer version with few fields loads.
        std::fs::write(&path, "device_name = \"x\"\nfuture_field = 1\n").unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.device_name, "x");
        assert_eq!(loaded.listen_port, DEFAULT_PORT);
        assert!(loaded.clipboard);
    }

    #[test]
    fn corrupt_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let s = Settings::load_from(&path);
        assert!(!s.onboarded);
        assert!(dir.path().join("settings.bak").exists());
    }

    #[test]
    fn normalize_clamps_wild_values() {
        let mut s = Settings {
            ui_scale: 99.0,
            clipboard_max_kb: 0,
            listen_port: 0,
            device_name: "bad name!".into(),
            ..Settings::default()
        };
        s.normalize();
        assert_eq!(s.ui_scale, 1.6);
        assert_eq!(s.clipboard_max_kb, 1);
        assert_eq!(s.listen_port, DEFAULT_PORT);
        assert_eq!(s.device_name, "bad-name");
    }

    #[test]
    fn layout_includes_local_device_even_if_unsaved() {
        let s = Settings {
            device_name: "me".into(),
            ..Settings::default()
        };
        let l = s.to_layout().unwrap();
        assert_eq!(l.local_id, "me");
        assert_eq!(l.devices.len(), 1);
    }

    #[test]
    fn options_carry_sensitivity() {
        let mut s = Settings::default();
        s.devices.push(DeviceRecord {
            id: "b".into(),
            x: 0,
            y: 0,
            monitors: vec![],
            addr: Some("10.0.0.2:7878".into()),
            sensitivity: 2.0,
        });
        assert_eq!(s.session_options().sensitivity["b"], 2.0);
        assert_eq!(s.addr_of("b").unwrap().port(), 7878);
    }
}
