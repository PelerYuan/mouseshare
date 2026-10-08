//! Screen geometry and edge-crossing control handoff.
//!
//! The model: a **device** is one computer. It owns one or more
//! **monitors**, laid out the way that computer's own display server lays
//! them out (device-local coordinates). Devices themselves are placed in a
//! shared **virtual desktop** by the position of their origin. The cursor
//! hands off between devices when it walks off the *union* of the current
//! device's monitors onto a monitor of another device.
//!
//! Config files may use the original single-rectangle form (`[[screens]]`,
//! one rectangle per device) or the multi-monitor form (`[[devices]]` with
//! nested `[[devices.monitors]]`); both load into the same model.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A device-local monitor rectangle (root-window coordinates of its owner).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Monitor {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Monitor {
    pub fn new(name: impl Into<String>, x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            name: name.into(),
            x,
            y,
            width,
            height,
        }
    }

    fn contains(&self, x: i32, y: i32, margin: i32) -> bool {
        x >= self.x - margin
            && x < self.x + self.width + margin
            && y >= self.y - margin
            && y < self.y + self.height + margin
    }

    fn clamp(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x.clamp(self.x, self.x + self.width - 1),
            y.clamp(self.y, self.y + self.height - 1),
        )
    }
}

/// One computer: an origin in the virtual desktop plus its monitors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    /// Where this device's root-window origin `(0, 0)` sits in the virtual
    /// desktop.
    pub x: i32,
    pub y: i32,
    pub monitors: Vec<Monitor>,
}

impl Device {
    /// A device with a single monitor covering `width`x`height`.
    pub fn single(id: impl Into<String>, x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            id: id.into(),
            x,
            y,
            monitors: vec![Monitor::new("", 0, 0, width, height)],
        }
    }

    /// Whether the virtual-desktop point lies on one of this device's
    /// monitors, with every monitor grown by `margin` pixels.
    pub fn contains_virtual(&self, vx: i32, vy: i32, margin: i32) -> bool {
        let (lx, ly) = (vx - self.x, vy - self.y);
        self.monitors.iter().any(|m| m.contains(lx, ly, margin))
    }

    /// The nearest in-bounds point (device-local) to a virtual point.
    pub fn clamp_virtual(&self, vx: i32, vy: i32) -> (i32, i32) {
        let (lx, ly) = (vx - self.x, vy - self.y);
        let mut best = (i64::MAX, (lx, ly));
        for m in &self.monitors {
            let (cx, cy) = m.clamp(lx, ly);
            let d = (cx - lx) as i64 * (cx - lx) as i64 + (cy - ly) as i64 * (cy - ly) as i64;
            if d < best.0 {
                best = (d, (cx, cy));
            }
        }
        best.1
    }

    /// Bounding box `(x, y, w, h)` of all monitors, device-local.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let x0 = self.monitors.iter().map(|m| m.x).min().unwrap_or(0);
        let y0 = self.monitors.iter().map(|m| m.y).min().unwrap_or(0);
        let x1 = self
            .monitors
            .iter()
            .map(|m| m.x + m.width)
            .max()
            .unwrap_or(0);
        let y1 = self
            .monitors
            .iter()
            .map(|m| m.y + m.height)
            .max()
            .unwrap_or(0);
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// A point guaranteed to be on some monitor (centre of the first one).
    pub fn center_local(&self) -> (i32, i32) {
        self.monitors
            .first()
            .map(|m| (m.x + m.width / 2, m.y + m.height / 2))
            .unwrap_or((0, 0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutConfig {
    /// The device this machine is.
    pub local_id: String,
    pub devices: Vec<Device>,
}

// ---- on-disk shape -------------------------------------------------------

#[derive(Deserialize, Serialize)]
struct RawScreen {
    id: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Deserialize, Serialize)]
struct RawDevice {
    id: String,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    monitors: Vec<Monitor>,
    // Shorthand for a one-monitor device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    height: Option<i32>,
}

#[derive(Deserialize, Serialize)]
struct RawConfig {
    local_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    screens: Vec<RawScreen>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    devices: Vec<RawDevice>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("failed to read layout file {0}: {1}")]
    Io(String, std::io::Error),
    #[error("failed to parse layout file: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to write layout: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("local_id {0:?} is not present in the configured devices")]
    UnknownLocalId(String),
    #[error("duplicate device id {0:?}")]
    DuplicateId(String),
    #[error("device {0:?} has no monitors (give it `monitors`, or `width` and `height`)")]
    NoMonitors(String),
    #[error("device {0:?} has a monitor with a non-positive size")]
    BadMonitor(String),
}

impl LayoutConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, LayoutError> {
        let raw: RawConfig = toml::from_str(s)?;
        let mut devices = Vec::new();
        for sc in raw.screens {
            devices.push(Device::single(sc.id, sc.x, sc.y, sc.width, sc.height));
        }
        for d in raw.devices {
            let monitors = if !d.monitors.is_empty() {
                d.monitors
            } else if let (Some(w), Some(h)) = (d.width, d.height) {
                vec![Monitor::new("", 0, 0, w, h)]
            } else {
                return Err(LayoutError::NoMonitors(d.id));
            };
            devices.push(Device {
                id: d.id,
                x: d.x,
                y: d.y,
                monitors,
            });
        }
        let config = LayoutConfig {
            local_id: raw.local_id,
            devices,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, LayoutError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| LayoutError::Io(path.display().to_string(), e))?;
        Self::from_toml_str(&raw)
    }

    /// Serializes to the multi-monitor `[[devices]]` form (what the GUI's
    /// "export layout" writes for the CLI to consume).
    pub fn to_toml_string(&self) -> Result<String, LayoutError> {
        let raw = RawConfig {
            local_id: self.local_id.clone(),
            screens: Vec::new(),
            devices: self
                .devices
                .iter()
                .map(|d| RawDevice {
                    id: d.id.clone(),
                    x: d.x,
                    y: d.y,
                    monitors: d.monitors.clone(),
                    width: None,
                    height: None,
                })
                .collect(),
        };
        Ok(toml::to_string_pretty(&raw)?)
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        let mut seen = std::collections::HashSet::new();
        for d in &self.devices {
            if !seen.insert(&d.id) {
                return Err(LayoutError::DuplicateId(d.id.clone()));
            }
            if d.monitors.is_empty() {
                return Err(LayoutError::NoMonitors(d.id.clone()));
            }
            if d.monitors.iter().any(|m| m.width <= 0 || m.height <= 0) {
                return Err(LayoutError::BadMonitor(d.id.clone()));
            }
        }
        if self.device(&self.local_id).is_none() {
            return Err(LayoutError::UnknownLocalId(self.local_id.clone()));
        }
        Ok(())
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn device_mut(&mut self, id: &str) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.id == id)
    }

    pub fn local_device(&self) -> &Device {
        self.device(&self.local_id)
            .expect("validated at construction: local_id exists")
    }

    /// Ids of every device except this machine.
    pub fn remote_ids(&self) -> impl Iterator<Item = &str> {
        self.devices
            .iter()
            .map(|d| d.id.as_str())
            .filter(|id| *id != self.local_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlState {
    Local,
    Remote(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub new_state: ControlState,
    /// Absolute local root coordinates to warp the OS cursor to. Only
    /// meaningful when `new_state` is `ControlState::Local`.
    pub local_target: Option<(i32, i32)>,
    /// Where on the *newly controlled remote device* (its root coordinates)
    /// the cursor should appear. Only set when `new_state` is `Remote`.
    pub remote_target: Option<(i32, i32)>,
}

/// Tracks a single virtual cursor position across the whole configured
/// virtual desktop and decides when control should hand off between
/// devices. The node that owns the physical mouse runs this continuously;
/// peers never need to report their cursor position back, because every
/// delta forwarded to them originated here.
#[derive(Clone)]
pub struct EdgeDetector {
    layout: LayoutConfig,
    state: ControlState,
    virtual_pos: (i32, i32),
    reentry_margin: i32,
    hold: bool,
}

impl EdgeDetector {
    pub fn new(layout: LayoutConfig, reentry_margin: i32) -> Self {
        let local = layout.local_device();
        let (cx, cy) = local.center_local();
        let start = (local.x + cx, local.y + cy);
        Self {
            layout,
            state: ControlState::Local,
            virtual_pos: start,
            reentry_margin,
            hold: false,
        }
    }

    /// While `true`, the cursor is pinned to the device it is on: it can
    /// still move, but running off the edge clamps instead of handing off.
    /// Used so dragging with a button held can't switch machines mid-drag.
    pub fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
    }

    pub fn state(&self) -> &ControlState {
        &self.state
    }

    pub fn virtual_pos(&self) -> (i32, i32) {
        self.virtual_pos
    }

    pub fn layout(&self) -> &LayoutConfig {
        &self.layout
    }

    /// Replaces one device's monitor list (hot-plug / resolution change).
    /// If that is the remote currently being controlled, the virtual cursor
    /// is pulled back onto its new monitors.
    pub fn update_monitors(&mut self, device_id: &str, monitors: Vec<Monitor>) {
        if monitors.is_empty() {
            return;
        }
        if let Some(d) = self.layout.device_mut(device_id) {
            d.monitors = monitors;
        }
        if self.state == ControlState::Remote(device_id.to_string()) {
            let d = self.layout.device(device_id).expect("just updated").clone();
            let (lx, ly) = d.clamp_virtual(self.virtual_pos.0, self.virtual_pos.1);
            self.virtual_pos = (d.x + lx, d.y + ly);
        }
    }

    /// Feed an absolute local root cursor position while `state()` is
    /// `Local`. Returns `Some` when control should hand off to a neighbor.
    ///
    /// A real OS cursor is clamped by the display server to the union of
    /// the monitors and can never actually be reported *beyond* it -- so
    /// resting on a boundary pixel is the strongest signal a real caller can
    /// ever give us that the user is trying to leave in that direction.
    /// Treat it as having already crossed one pixel past the edge (into
    /// whatever neighbor is configured there, if any).
    pub fn on_local_move(&mut self, local_x: i32, local_y: i32) -> Option<Transition> {
        debug_assert_eq!(self.state, ControlState::Local);
        let local = self.layout.local_device().clone();
        let (cx, cy) = local.clamp_virtual(local.x + local_x, local.y + local_y);
        let p = (local.x + cx, local.y + cy);
        self.virtual_pos = p;
        // Only act on the boundary of the *union*: an internal seam between
        // two of this device's own monitors must never hand off.
        for (ddx, ddy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let q = (p.0 + ddx, p.1 + ddy);
            if local.contains_virtual(q.0, q.1, 0) {
                continue;
            }
            if let Some(t) = self.enter_device_at(q, &local.id) {
                return Some(t);
            }
        }
        None
    }

    /// Feed a relative delta captured locally while `state()` is
    /// `Remote(_)`. Returns `Some` when control should hand off (back to
    /// `Local`, or to a different remote device).
    pub fn on_remote_delta(&mut self, dx: i32, dy: i32) -> Option<Transition> {
        let ControlState::Remote(current_id) = self.state.clone() else {
            debug_assert!(false, "on_remote_delta called while state is Local");
            return None;
        };
        let pos = (self.virtual_pos.0 + dx, self.virtual_pos.1 + dy);
        self.virtual_pos = pos;
        let current = self
            .layout
            .device(&current_id)
            .expect("current remote device id must exist in layout")
            .clone();
        if current.contains_virtual(pos.0, pos.1, self.reentry_margin) {
            return None;
        }
        if !self.hold {
            if let Some(t) = self.enter_device_at(pos, &current_id) {
                return Some(t);
            }
        }
        // No neighbour (or hand-off suppressed) there: clamp to the nearest point of this device.
        let (lx, ly) = current.clamp_virtual(pos.0, pos.1);
        self.virtual_pos = (current.x + lx, current.y + ly);
        None
    }

    /// Forces control back to `Local` immediately, e.g. because the
    /// currently-controlled remote's connection was just lost or the user
    /// pressed the emergency-return hotkey.
    ///
    /// Returns the absolute local root coordinates the OS cursor should be
    /// warped to (the current virtual position, clamped onto the local
    /// monitors).
    pub fn force_local(&mut self) -> (i32, i32) {
        let local = self.layout.local_device().clone();
        let (lx, ly) = local.clamp_virtual(self.virtual_pos.0, self.virtual_pos.1);
        self.virtual_pos = (local.x + lx, local.y + ly);
        self.state = ControlState::Local;
        (lx, ly)
    }

    fn enter_device_at(&mut self, pos: (i32, i32), exclude_id: &str) -> Option<Transition> {
        let target = self
            .layout
            .devices
            .iter()
            .find(|d| d.id != exclude_id && d.contains_virtual(pos.0, pos.1, 0))?
            .clone();
        self.virtual_pos = pos;
        let local = (pos.0 - target.x, pos.1 - target.y);
        if target.id == self.layout.local_id {
            self.state = ControlState::Local;
            Some(Transition {
                new_state: ControlState::Local,
                local_target: Some(local),
                remote_target: None,
            })
        } else {
            self.state = ControlState::Remote(target.id.clone());
            Some(Transition {
                new_state: ControlState::Remote(target.id),
                local_target: None,
                remote_target: Some(local),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_layout_file_parses() {
        let cfg = LayoutConfig::from_toml_str(include_str!("../../../layout.example.toml"))
            .expect("layout.example.toml must stay valid");
        assert_eq!(cfg.local_id, "desk");
        assert_eq!(cfg.devices.len(), 2);
        assert_eq!(cfg.device("desk").unwrap().monitors.len(), 2);
    }

    use super::*;

    fn two_screen_layout() -> LayoutConfig {
        LayoutConfig::from_toml_str(
            r#"
            local_id = "A"

            [[screens]]
            id = "A"
            x = 0
            y = 0
            width = 1000
            height = 800

            [[screens]]
            id = "B"
            x = 1000
            y = 0
            width = 1000
            height = 800
            "#,
        )
        .unwrap()
    }

    #[test]
    fn rejects_unknown_local_id() {
        let err = LayoutConfig::from_toml_str(
            r#"
            local_id = "Z"
            [[screens]]
            id = "A"
            x = 0
            y = 0
            width = 100
            height = 100
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, LayoutError::UnknownLocalId(_)));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let err = LayoutConfig::from_toml_str(
            r#"
            local_id = "A"
            [[screens]]
            id = "A"
            x = 0
            y = 0
            width = 100
            height = 100
            [[screens]]
            id = "A"
            x = 100
            y = 0
            width = 100
            height = 100
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, LayoutError::DuplicateId(_)));
    }

    #[test]
    fn stays_local_within_bounds() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        assert_eq!(d.on_local_move(500, 400), None);
        assert_eq!(*d.state(), ControlState::Local);
    }

    #[test]
    fn crosses_right_edge_into_neighbor() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        // Screen A is 1000 wide; a real cursor can be reported at x=999
        // (the last valid pixel) but never at x=1000 or beyond, since the
        // display server itself clamps it there.
        let t = d.on_local_move(999, 400).expect("should hand off");
        assert_eq!(t.new_state, ControlState::Remote("B".into()));
        assert_eq!(*d.state(), ControlState::Remote("B".into()));
    }

    #[test]
    fn does_not_cross_one_pixel_before_the_edge() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        assert_eq!(d.on_local_move(998, 400), None);
        assert_eq!(*d.state(), ControlState::Local);
    }

    #[test]
    fn clamps_when_no_neighbor_configured() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        // Left edge of A has no neighbor.
        assert_eq!(d.on_local_move(-50, 400), None);
        assert_eq!(*d.state(), ControlState::Local);
        assert_eq!(d.virtual_pos(), (0, 400));
    }

    #[test]
    fn remote_delta_returns_to_local_with_margin() {
        let mut d = EdgeDetector::new(two_screen_layout(), 5);
        d.on_local_move(999, 400).unwrap(); // hand off to B, virtual_pos = (1000, 400)
                                            // Small negative delta stays within the margin around B's left edge.
        assert_eq!(d.on_remote_delta(-3, 0), None);
        assert_eq!(*d.state(), ControlState::Remote("B".into()));
        // Crossing past the margin hands control back to Local.
        let t = d.on_remote_delta(-10, 0).expect("should return to local");
        assert_eq!(t.new_state, ControlState::Local);
        let (lx, ly) = t.local_target.expect("local target must be set");
        assert_eq!((lx, ly), (987, 400));
    }

    #[test]
    fn force_local_recovers_from_a_dead_remote() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        d.on_local_move(999, 400).unwrap(); // hand off to B, virtual_pos = (1000, 400)
        assert_eq!(d.on_remote_delta(50, 30), None); // still remote, virtual_pos = (1050, 430)
        let (lx, ly) = d.force_local();
        assert_eq!(*d.state(), ControlState::Local);
        // virtual_pos (1050, 430) is inside B, not A -- clamped onto A's
        // right wall (last valid pixel column) at the same y.
        assert_eq!((lx, ly), (999, 430));
    }

    #[test]
    fn remote_delta_clamps_at_far_wall() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        d.on_local_move(999, 400).unwrap();
        assert_eq!(d.on_remote_delta(5000, 0), None);
        assert_eq!(*d.state(), ControlState::Remote("B".into()));
        assert_eq!(d.virtual_pos(), (1999, 400));
    }

    fn dual_monitor_layout() -> LayoutConfig {
        // A: two 1000x800 monitors side by side (2000 wide). B: one monitor
        // to the right of A's union. C: below A's first monitor only.
        LayoutConfig::from_toml_str(
            r#"
            local_id = "A"

            [[devices]]
            id = "A"
            x = 0
            y = 0
            [[devices.monitors]]
            x = 0
            y = 0
            width = 1000
            height = 800
            [[devices.monitors]]
            x = 1000
            y = 0
            width = 1000
            height = 800

            [[devices]]
            id = "B"
            x = 2000
            y = 0
            width = 1000
            height = 800

            [[devices]]
            id = "C"
            x = 0
            y = 800
            width = 1000
            height = 600
            "#,
        )
        .unwrap()
    }

    #[test]
    fn seam_between_own_monitors_never_hands_off() {
        let mut d = EdgeDetector::new(dual_monitor_layout(), 2);
        assert_eq!(d.on_local_move(999, 400), None);
        assert_eq!(d.on_local_move(1000, 400), None);
        assert_eq!(*d.state(), ControlState::Local);
    }

    #[test]
    fn crosses_union_edge_into_neighbour_with_entry_point() {
        let mut d = EdgeDetector::new(dual_monitor_layout(), 2);
        let t = d.on_local_move(1999, 123).expect("hand off to B");
        assert_eq!(t.new_state, ControlState::Remote("B".into()));
        assert_eq!(t.remote_target, Some((0, 123)));
    }

    #[test]
    fn only_the_adjacent_monitor_borders_a_neighbour() {
        let mut d = EdgeDetector::new(dual_monitor_layout(), 2);
        // Bottom of A's second monitor has nothing below it.
        assert_eq!(d.on_local_move(1500, 799), None);
        // Bottom of A's first monitor borders C.
        let t = d.on_local_move(500, 799).expect("hand off to C");
        assert_eq!(t.new_state, ControlState::Remote("C".into()));
        assert_eq!(t.remote_target, Some((500, 0)));
    }

    #[test]
    fn hold_pins_the_cursor_to_its_device() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        d.on_local_move(999, 400).unwrap();
        d.set_hold(true);
        assert_eq!(d.on_remote_delta(-300, 0), None);
        assert_eq!(*d.state(), ControlState::Remote("B".into()));
        assert_eq!(d.virtual_pos(), (1000, 400));
        d.set_hold(false);
        assert!(d.on_remote_delta(-300, 0).is_some());
    }

    #[test]
    fn remote_walls_clamp() {
        let mut d = EdgeDetector::new(dual_monitor_layout(), 2);
        d.on_local_move(500, 799).unwrap(); // into C at virtual (500, 800)
        assert_eq!(d.on_remote_delta(2000, 0), None);
        assert_eq!(d.virtual_pos().0, 999);
    }

    #[test]
    fn legacy_screens_and_devices_forms_agree() {
        let legacy = two_screen_layout();
        let rt = LayoutConfig::from_toml_str(&legacy.to_toml_string().unwrap()).unwrap();
        assert_eq!(legacy, rt);
    }

    #[test]
    fn device_without_monitors_is_rejected() {
        let err = LayoutConfig::from_toml_str(
            r#"
            local_id = "A"
            [[devices]]
            id = "A"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, LayoutError::NoMonitors(_)));
    }

    #[test]
    fn monitor_hotplug_pulls_cursor_back() {
        let mut d = EdgeDetector::new(dual_monitor_layout(), 2);
        d.on_local_move(1999, 100).unwrap(); // into B
        d.on_remote_delta(900, 0);
        d.update_monitors("B", vec![Monitor::new("", 0, 0, 500, 800)]);
        assert!(d.virtual_pos().0 < 2500);
    }
}
