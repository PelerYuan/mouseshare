use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// Max accepted frame body size. Guards against a corrupt length prefix
/// causing an unbounded allocation.
pub const MAX_FRAME_LEN: u32 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Message {
    Hello {
        version: u32,
        screen_id: String,
        width: i32,
        height: i32,
    },
    HelloAck {
        version: u32,
        screen_id: String,
        width: i32,
        height: i32,
    },
    /// Relative pointer movement, in target-screen pixels.
    MouseMove {
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
    Heartbeat,
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

    #[test]
    fn round_trips_every_variant() {
        let messages = [
            Message::Hello {
                version: PROTOCOL_VERSION,
                screen_id: "a".into(),
                width: 1920,
                height: 1080,
            },
            Message::HelloAck {
                version: PROTOCOL_VERSION,
                screen_id: "b".into(),
                width: 1280,
                height: 720,
            },
            Message::MouseMove { dx: -5, dy: 12 },
            Message::KeyEvent { keycode: 38, pressed: true },
            Message::KeyEvent { keycode: 38, pressed: false },
            Message::ClipboardText("hello, clipboard".to_string()),
            Message::Heartbeat,
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
            screen_id: "x".repeat(MAX_FRAME_LEN as usize + 1),
            width: 0,
            height: 0,
        };
        assert!(matches!(
            encode_frame(&huge),
            Err(ProtocolError::FrameTooLarge(_))
        ));
    }
}
