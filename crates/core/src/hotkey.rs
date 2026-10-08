//! The emergency-return hotkey: pulls the cursor and keyboard back to this
//! computer no matter what the remote is doing. It is detected locally on
//! the captured key stream and swallowed (never forwarded).

use std::collections::HashSet;
use std::fmt;

use mouseshare_x11input::LocalCursor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    /// Lower-case key name, e.g. `esc`, `f12`, `pause`, `q`.
    pub key: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            ctrl: true,
            alt: true,
            shift: false,
            meta: false,
            key: "esc".into(),
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.meta {
            write!(f, "Super+")?;
        }
        let mut name = self.key.clone();
        if let Some(c) = name.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        write!(f, "{name}")
    }
}

impl Hotkey {
    /// Parses strings like `Ctrl+Alt+Esc` or `ctrl+shift+f12`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut hk = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            key: String::new(),
        };
        for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            match part.as_str() {
                "" => return Err(format!("empty component in {s:?}")),
                "ctrl" | "control" => hk.ctrl = true,
                "alt" => hk.alt = true,
                "shift" => hk.shift = true,
                "super" | "meta" | "win" | "cmd" => hk.meta = true,
                other => {
                    if !hk.key.is_empty() {
                        return Err(format!("more than one non-modifier key in {s:?}"));
                    }
                    if keysym_for_name(other).is_none() {
                        return Err(format!("unknown key {other:?}"));
                    }
                    hk.key = other.to_string();
                }
            }
        }
        if hk.key.is_empty() {
            return Err(format!("no key in {s:?}"));
        }
        if !(hk.ctrl || hk.alt || hk.shift || hk.meta) {
            return Err("a hotkey needs at least one modifier".into());
        }
        Ok(hk)
    }
}

pub fn keysym_for_name(name: &str) -> Option<u32> {
    match name {
        "esc" | "escape" => Some(0xff1b),
        "pause" => Some(0xff13),
        "scrolllock" => Some(0xff14),
        "home" => Some(0xff50),
        "end" => Some(0xff57),
        "menu" => Some(0xff67),
        "backspace" => Some(0xff08),
        "space" => Some(0x20),
        n if n.len() == 1 => {
            let c = n.as_bytes()[0];
            (c.is_ascii_lowercase() || c.is_ascii_digit()).then_some(c as u32)
        }
        n if n.starts_with('f') => {
            let num: u32 = n[1..].parse().ok()?;
            (1..=12).contains(&num).then(|| 0xffbe + num - 1)
        }
        _ => None,
    }
}

/// A [`Hotkey`] resolved to concrete keycodes on this X server.
pub struct HotkeyMatcher {
    /// Each required modifier -> its acceptable keycodes (left/right).
    groups: Vec<Vec<u8>>,
    key: u8,
}

impl HotkeyMatcher {
    pub fn resolve(hk: &Hotkey, cursor: &LocalCursor) -> Option<Self> {
        let lookup = |syms: &[u32]| -> Vec<u8> {
            syms.iter()
                .filter_map(|s| cursor.keycode_for_keysym(*s).ok().flatten())
                .collect()
        };
        let mut groups = Vec::new();
        for (needed, syms) in [
            (hk.ctrl, [0xffe3, 0xffe4]),
            (hk.alt, [0xffe9, 0xffea]),
            (hk.shift, [0xffe1, 0xffe2]),
            (hk.meta, [0xffeb, 0xffec]),
        ] {
            if needed {
                let codes = lookup(&syms);
                if codes.is_empty() {
                    return None;
                }
                groups.push(codes);
            }
        }
        let key = cursor
            .keycode_for_keysym(keysym_for_name(&hk.key)?)
            .ok()
            .flatten()?;
        Some(Self { groups, key })
    }

    /// Whether the key event `(keycode, pressed)` completes the hotkey,
    /// given the keys currently held (not including this one).
    pub fn triggered(&self, keycode: u8, pressed: bool, held: &HashSet<u8>) -> bool {
        pressed
            && keycode == self.key
            && self
                .groups
                .iter()
                .all(|g| g.iter().any(|k| held.contains(k)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        let hk = Hotkey::parse("ctrl+Alt+Esc").unwrap();
        assert_eq!(hk, Hotkey::default());
        assert_eq!(hk.to_string(), "Ctrl+Alt+Esc");
        assert!(Hotkey::parse("esc").is_err());
        assert!(Hotkey::parse("ctrl+").is_err());
        assert!(Hotkey::parse("ctrl+a+b").is_err());
        assert!(Hotkey::parse("ctrl+f13").is_err());
        assert!(Hotkey::parse("ctrl+shift+f12").is_ok());
    }

    #[test]
    fn matcher_needs_modifiers_held() {
        let m = HotkeyMatcher {
            groups: vec![vec![37, 105], vec![64, 108]],
            key: 9,
        };
        let mut held: HashSet<u8> = [37].into();
        assert!(!m.triggered(9, true, &held));
        held.insert(108);
        assert!(m.triggered(9, true, &held));
        assert!(!m.triggered(9, false, &held));
        assert!(!m.triggered(10, true, &held));
    }
}
