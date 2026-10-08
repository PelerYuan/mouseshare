use serde::{Deserialize, Serialize};

/// Wire protocol revision. Bumped to 2 for: monitor lists in the handshake,
/// mouse buttons/scroll, absolute warps, latency probes, and the encrypted
/// transport (see `mouseshare-net`).
pub const PROTOCOL_VERSION: u32 = 2;

/// Max accepted frame body size. Guards against a corrupt length prefix
/// causing an unbounded allocation.
pub const MAX_FRAME_LEN: u32 = 1024 * 1024;

/// One physical monitor of a device, in that device's own root-window
/// coordinate space (the top-left of the leftmost/topmost monitor is not
/// necessarily `(0, 0)` -- X11 lets users position monitors freely).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Message {
    Hello {
        version: u32,
        device_id: String,
        monitors: Vec<MonitorInfo>,
    },
    HelloAck {
        version: u32,
        device_id: String,
        monitors: Vec<MonitorInfo>,
    },
    /// Relative pointer movement, in target-device pixels.
    MouseMove {
        dx: i32,
        dy: i32,
    },
    /// Place the pointer at an absolute position in the target device's root
    /// coordinate space. Sent when control enters a device so the cursor
    /// appears where it crossed the seam instead of wherever it last was.
    MouseWarp {
        x: i32,
        y: i32,
    },
    /// A pointer button press/release. `button` is the X11 button number
    /// (1 = left, 2 = middle, 3 = right, 8/9 = back/forward). Wheel buttons
    /// (4-7) are never sent as buttons; see [`Message::Scroll`].
    MouseButton {
        button: u8,
        pressed: bool,
    },
    /// Wheel movement in notches. Positive `dy` scrolls down, positive `dx`
    /// scrolls right.
    Scroll {
        dx: i32,
        dy: i32,
    },
    /// A key press or release. `keycode` is a raw X11 keycode (the X11
    /// protocol defines these as 8-bit values) — both ends are X11 for
    /// now, so it's passed through unmapped. A cross-platform key
    /// representation is out of scope until a non-X11 backend exists.
    KeyEvent {
        keycode: u8,
        pressed: bool,
    },
    /// The sender's clipboard changed to this plain-text content. Sent in
    /// either direction, independent of which side currently has mouse/
    /// keyboard control -- clipboard sync isn't gated by `ControlState`.
    ClipboardText(String),
    /// Latency probe; the peer answers with `Pong` carrying the same token.
    Ping(u64),
    Pong(u64),
    /// The sender's monitor configuration changed (hot-plug, resolution
    /// change).
    MonitorsChanged(Vec<MonitorInfo>),
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame of {0} bytes exceeds MAX_FRAME_LEN")]
    FrameTooLarge(u32),
    #[error(transparent)]
    Encode(#[from] bincode::Error),
}

/// Encodes a message as a 4-byte big-endian length prefix followed by the
/// bincode-serialized payload, ready to write to a stream.
pub fn encode_frame(msg: &Message) -> Result<Vec<u8>, ProtocolError> {
    let body = bincode::serialize(msg)?;
    let len: u32 = body
        .len()
        .try_into()
        .map_err(|_| ProtocolError::FrameTooLarge(u32::MAX))?;
    if len > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge(len));
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(&body);
    Ok(framed)
}

/// Decodes a message body (length prefix already stripped by the caller).
pub fn decode_body(body: &[u8]) -> Result<Message, ProtocolError> {
    Ok(bincode::deserialize(body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, x: i32) -> MonitorInfo {
        MonitorInfo {
            name: name.into(),
            x,
            y: 0,
            width: 1920,
            height: 1080,
            primary: x == 0,
        }
    }

    #[test]
    fn round_trips_every_variant() {
        let messages = [
            Message::Hello {
                version: PROTOCOL_VERSION,
                device_id: "a".into(),
                monitors: vec![mon("DP-1", 0), mon("HDMI-1", 1920)],
            },
            Message::HelloAck {
                version: PROTOCOL_VERSION,
                device_id: "b".into(),
                monitors: vec![mon("eDP-1", 0)],
            },
            Message::MouseMove { dx: -5, dy: 12 },
            Message::MouseWarp { x: 100, y: 200 },
            Message::MouseButton {
                button: 1,
                pressed: true,
            },
            Message::Scroll { dx: 0, dy: -3 },
            Message::KeyEvent {
                keycode: 38,
                pressed: true,
            },
            Message::KeyEvent {
                keycode: 38,
                pressed: false,
            },
            Message::ClipboardText("hello, clipboard".to_string()),
            Message::Ping(42),
            Message::Pong(42),
            Message::MonitorsChanged(vec![mon("DP-2", 0)]),
        ];
        for msg in messages {
            let framed = encode_frame(&msg).unwrap();
            let len = u32::from_be_bytes(framed[0..4].try_into().unwrap());
            assert_eq!(len as usize, framed.len() - 4);
            let decoded = decode_body(&framed[4..]).unwrap();
            assert_eq!(decoded, msg);
        }
    }

    #[test]
    fn rejects_oversized_frame() {
        let huge = Message::HelloAck {
            version: 1,
            device_id: "x".repeat(MAX_FRAME_LEN as usize + 1),
            monitors: vec![],
        };
        assert!(matches!(
            encode_frame(&huge),
            Err(ProtocolError::FrameTooLarge(_))
        ));
    }
}
