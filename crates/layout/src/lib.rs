use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ScreenRect {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ScreenRect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// Like `contains`, but the rect is treated as expanded by `margin` on
    /// every side. Used to debounce control handoff right at a seam.
    pub fn contains_with_margin(&self, x: i32, y: i32, margin: i32) -> bool {
        x >= self.x - margin
            && x < self.x + self.width + margin
            && y >= self.y - margin
            && y < self.y + self.height + margin
    }

    pub fn clamp(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x.clamp(self.x, self.x + self.width - 1),
            y.clamp(self.y, self.y + self.height - 1),
        )
    }

    /// Converts a point in shared virtual-desktop coordinates to coordinates
    /// local to this screen (i.e. relative to its own top-left corner).
    pub fn to_local(&self, vx: i32, vy: i32) -> (i32, i32) {
        (vx - self.x, vy - self.y)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LayoutConfig {
    pub local_id: String,
    #[serde(rename = "screens")]
    pub screens: Vec<ScreenRect>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("failed to read layout file {0}: {1}")]
    Io(String, std::io::Error),
    #[error("failed to parse layout file: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("local_id {0:?} is not present in the configured screens")]
    UnknownLocalId(String),
    #[error("duplicate screen id {0:?}")]
    DuplicateId(String),
}

impl LayoutConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, LayoutError> {
        let config: LayoutConfig = toml::from_str(s)?;
        config.validate()?;
        Ok(config)
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, LayoutError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| LayoutError::Io(path.display().to_string(), e))?;
        Self::from_toml_str(&raw)
    }

    fn validate(&self) -> Result<(), LayoutError> {
        let mut seen = std::collections::HashSet::new();
        for screen in &self.screens {
            if !seen.insert(&screen.id) {
                return Err(LayoutError::DuplicateId(screen.id.clone()));
            }
        }
        if self.screen(&self.local_id).is_none() {
            return Err(LayoutError::UnknownLocalId(self.local_id.clone()));
        }
        Ok(())
    }

    pub fn screen(&self, id: &str) -> Option<&ScreenRect> {
        self.screens.iter().find(|s| s.id == id)
    }

    pub fn local_screen(&self) -> &ScreenRect {
        self.screen(&self.local_id)
            .expect("validated at construction: local_id exists")
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
    /// Absolute local-screen coordinates to warp the OS cursor to. Only
    /// meaningful when `new_state` is `ControlState::Local`.
    pub local_target: Option<(i32, i32)>,
}

/// Tracks a single virtual cursor position across the whole configured
/// virtual desktop and decides when control should hand off between
/// screens. The node that owns the physical mouse runs this continuously;
/// peers never need to report their cursor position back, because every
/// delta forwarded to them originated here.
pub struct EdgeDetector {
    layout: LayoutConfig,
    state: ControlState,
    virtual_pos: (i32, i32),
    reentry_margin: i32,
}

impl EdgeDetector {
    pub fn new(layout: LayoutConfig, reentry_margin: i32) -> Self {
        let local = layout.local_screen();
        let start = (local.x + local.width / 2, local.y + local.height / 2);
        Self {
            layout,
            state: ControlState::Local,
            virtual_pos: start,
            reentry_margin,
        }
    }

    pub fn state(&self) -> &ControlState {
        &self.state
    }

    pub fn virtual_pos(&self) -> (i32, i32) {
        self.virtual_pos
    }

    /// Feed an absolute local-screen cursor position while `state()` is
    /// `Local`. Returns `Some` when control should hand off to a neighbor.
    ///
    /// A real OS cursor is clamped by the display server to
    /// `[0, width-1] x [0, height-1]` and can never actually be reported
    /// *beyond* the screen — so resting at the boundary pixel is the
    /// strongest signal a real caller can ever give us that the user is
    /// trying to leave in that direction. Treat it as having already
    /// crossed one pixel past the edge (into whatever neighbor is
    /// configured there, if any) rather than requiring a position that a
    /// real query_pointer() could never produce.
    pub fn on_local_move(&mut self, local_x: i32, local_y: i32) -> Option<Transition> {
        debug_assert_eq!(self.state, ControlState::Local);
        let local = self.layout.local_screen().clone();
        let vx = if local_x <= 0 {
            local.x - 1
        } else if local_x >= local.width - 1 {
            local.x + local.width
        } else {
            local.x + local_x
        };
        let vy = if local_y <= 0 {
            local.y - 1
        } else if local_y >= local.height - 1 {
            local.y + local.height
        } else {
            local.y + local_y
        };
        self.virtual_pos = (vx, vy);
        if local.contains(vx, vy) {
            return None;
        }
        self.resolve_new_screen(&local.id, false)
    }

    /// Feed a relative delta captured locally while `state()` is
    /// `Remote(_)`. Returns `Some` when control should hand off (back to
    /// `Local`, or to a different remote peer for a >2-screen layout).
    pub fn on_remote_delta(&mut self, dx: i32, dy: i32) -> Option<Transition> {
        let ControlState::Remote(current_id) = self.state.clone() else {
            debug_assert!(false, "on_remote_delta called while state is Local");
            return None;
        };
        let (vx, vy) = (self.virtual_pos.0 + dx, self.virtual_pos.1 + dy);
        self.virtual_pos = (vx, vy);
        let current = self
            .layout
            .screen(&current_id)
            .expect("current remote screen id must exist in layout")
            .clone();
        if current.contains_with_margin(vx, vy, self.reentry_margin) {
            return None;
        }
        self.resolve_new_screen(&current.id, true)
    }

    fn resolve_new_screen(&mut self, exclude_id: &str, is_remote: bool) -> Option<Transition> {
        let (vx, vy) = self.virtual_pos;
        for screen in &self.layout.screens {
            if screen.id == exclude_id {
                continue;
            }
            if screen.contains(vx, vy) {
                let new_state = if screen.id == self.layout.local_id {
                    ControlState::Local
                } else {
                    ControlState::Remote(screen.id.clone())
                };
                let local_target = if screen.id == self.layout.local_id {
                    Some(screen.to_local(vx, vy))
                } else {
                    None
                };
                self.state = new_state.clone();
                return Some(Transition {
                    new_state,
                    local_target,
                });
            }
        }
        // No neighbor configured on that side: clamp to the wall, stay put.
        let current = self
            .layout
            .screen(exclude_id)
            .expect("exclude_id must exist in layout");
        self.virtual_pos = current.clamp(vx, vy);
        let _ = is_remote;
        None
    }
}

#[cfg(test)]
mod tests {
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
    fn remote_delta_clamps_at_far_wall() {
        let mut d = EdgeDetector::new(two_screen_layout(), 2);
        d.on_local_move(999, 400).unwrap();
        assert_eq!(d.on_remote_delta(5000, 0), None);
        assert_eq!(*d.state(), ControlState::Remote("B".into()));
        assert_eq!(d.virtual_pos(), (1999, 400));
    }
}
