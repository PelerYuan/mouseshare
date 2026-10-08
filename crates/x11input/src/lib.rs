//! Direct X11-protocol pointer/keyboard capture and injection for
//! mouseshare.
//!
//! This crate talks to an X server using raw core-protocol requests via
//! [`x11rb`] for everything except key *injection*, which needs the XTest
//! extension (there is no core-protocol way to synthesize input).
//!
//! There are two roles:
//!
//! * **Capture** (the machine that currently owns the physical mouse and
//!   keyboard): [`LocalCursor::begin_capture`] actively grabs both the
//!   pointer and the keyboard, confines the pointer to the root window, and
//!   warps it to a fixed center point. From then on,
//!   [`LocalCursor::poll_capture_delta`] drains queued `MotionNotify` events,
//!   turning each one into a `(dx, dy)` delta relative to that center point,
//!   and re-warps the pointer back to center after every event so the
//!   visible OS cursor never actually travels (the same trick Synergy/Barrier
//!   use on X11), while [`LocalCursor::poll_capture_keys`] drains queued
//!   `KeyPress`/`KeyRelease` events as `(keycode, pressed)` pairs.
//! * **Inject** (the machine currently receiving forwarded input):
//!   [`LocalCursor::warp_relative`] applies a relative pointer motion using
//!   the core `WarpPointer` request (source and destination windows both
//!   `None`), [`LocalCursor::warp_absolute`] places the pointer at an exact
//!   position on the root window (source window `None`, destination window
//!   the root window), and [`LocalCursor::inject_key`] synthesizes a key
//!   press/release via the XTest extension's `FakeInput` request.
//!
//! See the crate-level tests in `tests/` for an end-to-end exercise of both
//! roles against real (headless) `Xvfb` servers.
//!
//! A third, independent concern lives here too: [`clipboard::Clipboard`]
//! syncs the X11 `CLIPBOARD` selection (plain text only) via ICCCM
//! selection ownership, unrelated to pointer/keyboard capture.

mod clipboard;
pub use clipboard::Clipboard;

use x11rb::connection::Connection;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::xproto::{
    ConnectionExt, EventMask, GrabMode, GrabStatus, Window, BUTTON_PRESS_EVENT,
    BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as XtestConnectionExt;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::NONE;

/// Errors that can occur while talking to the X server.
#[derive(Debug, thiserror::Error)]
pub enum X11Error {
    /// Failed to establish the initial connection to the X server.
    #[error("failed to connect to X server: {0}")]
    Connect(#[from] ConnectError),

    /// A lower-level connection error (I/O, parsing, protocol violation, ...).
    #[error("X11 connection error: {0}")]
    Connection(#[from] ConnectionError),

    /// A request completed with an X11 error, or the connection broke while
    /// waiting for a reply.
    #[error("X11 request failed: {0}")]
    Reply(#[from] ReplyError),

    /// Same as [`X11Error::Reply`], but for requests that may also fail due
    /// to resource-ID exhaustion.
    #[error("X11 request failed: {0}")]
    ReplyOrId(#[from] ReplyOrIdError),

    /// `GrabPointer` or `GrabKeyboard` did not return `Success` (e.g. some
    /// other client already has an active grab).
    #[error("grab failed with status {0:?}")]
    GrabFailed(GrabStatus),

    /// [`LocalCursor::poll_capture_delta`], [`LocalCursor::poll_capture_keys`]
    /// or [`LocalCursor::end_capture`] was called while not in capture mode.
    #[error("not currently capturing the pointer")]
    NotCapturing,
}

/// One physical monitor, in root-window coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

/// One input event observed while capturing, in the order the server sent
/// them (so a button press followed by motion stays a press-then-drag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvent {
    /// Relative pointer motion since the previous event.
    Motion {
        dx: i32,
        dy: i32,
    },
    Key {
        keycode: u8,
        pressed: bool,
    },
    /// Pointer button (X11 numbering: 1 left, 2 middle, 3 right, 8/9
    /// back/forward). Wheel notches are reported as [`CaptureEvent::Scroll`].
    Button {
        button: u8,
        pressed: bool,
    },
    /// Wheel notches; positive `dy` is down, positive `dx` is right.
    Scroll {
        dx: i32,
        dy: i32,
    },
}

/// A reference point, plus buffered events not yet claimed by the
/// corresponding `poll_capture_*` method, used while in "capture" mode.
///
/// Both `poll_capture_delta` and `poll_capture_keys` read from the same
/// underlying X connection event queue, so whichever one is called first in
/// a given tick has to see (and stash) the other kind of event too, rather
/// than silently dropping it. See `drain_queued_events` for the shared drain
/// logic.
struct CaptureState {
    center: (i32, i32),
    pending: Vec<CaptureEvent>,
}

/// A connection to a single X display, with the local root window's geometry
/// cached, plus optional pointer-capture state.
///
/// Cheap operations (`query_pointer`, `warp_relative`, `warp_absolute`) can
/// be called at any time; `begin_capture` / `poll_capture_delta` /
/// `end_capture` are a stateful trio that must be called in that order.
pub struct LocalCursor {
    conn: RustConnection,
    root: Window,
    width: i32,
    height: i32,
    capture: Option<CaptureState>,
}

impl LocalCursor {
    /// Connects to an X display. `display` overrides `$DISPLAY` (e.g.
    /// `Some(":97")`) — this is what lets tests target a specific `Xvfb`
    /// instance instead of whatever display the process happens to inherit.
    pub fn connect(display: Option<&str>) -> Result<Self, X11Error> {
        let (conn, screen_num) = x11rb::connect(display)?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let width = screen.width_in_pixels as i32;
        let height = screen.height_in_pixels as i32;
        Ok(Self {
            conn,
            root,
            width,
            height,
            capture: None,
        })
    }

    /// Absolute pointer position on the root window right now.
    pub fn query_pointer(&self) -> Result<(i32, i32), X11Error> {
        let reply = self.conn.query_pointer(self.root)?.reply()?;
        Ok((reply.root_x as i32, reply.root_y as i32))
    }

    /// The root window's size in pixels, as reported at connect time.
    pub fn screen_size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// Grabs the pointer (confined to the root window) and the keyboard, and
    /// warps the pointer to a fixed center reference point, entering
    /// "capture" mode. Calling this again while already capturing is a
    /// no-op that re-confirms both grabs.
    ///
    /// If the keyboard grab fails after the pointer grab already succeeded,
    /// the pointer grab is released before returning the error, so a failed
    /// call never leaves a dangling pointer grab behind.
    pub fn begin_capture(&mut self) -> Result<(), X11Error> {
        let reply = self
            .conn
            .grab_pointer(
                false,
                self.root,
                EventMask::POINTER_MOTION | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                self.root,
                NONE,
                x11rb::CURRENT_TIME,
            )?
            .reply()?;
        if reply.status != GrabStatus::SUCCESS {
            return Err(X11Error::GrabFailed(reply.status));
        }

        if let Err(err) = self.grab_keyboard_only() {
            // Don't leak the pointer grab we just took if the keyboard grab
            // fails for any reason.
            let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
            let _ = self.conn.flush();
            return Err(err);
        }

        let center = (self.width / 2, self.height / 2);
        self.warp_absolute(center.0, center.1)?;
        // Drain and discard the MotionNotify that our own warp just queued
        // up, so the first real poll_capture_delta() starts from a clean
        // slate (its delta relative to `center` would be zero anyway, but
        // this also flushes the connection).
        while self.conn.poll_for_event()?.is_some() {}

        self.capture = Some(CaptureState {
            center,
            pending: Vec::new(),
        });
        Ok(())
    }

    /// Just the `GrabKeyboard` half of `begin_capture`, split out so the
    /// pointer-grab cleanup on failure has a single call site to wrap.
    fn grab_keyboard_only(&self) -> Result<(), X11Error> {
        let reply = self
            .conn
            .grab_keyboard(
                false,
                self.root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .reply()?;
        if reply.status != GrabStatus::SUCCESS {
            return Err(X11Error::GrabFailed(reply.status));
        }
        Ok(())
    }

    /// Drains every event currently buffered on the connection, exactly
    /// once, sorting `MotionNotify` into `pending_deltas` (converted to
    /// incremental deltas, same as before) and `KeyPress`/`KeyRelease` into
    /// `pending_keys`. Re-centers the pointer afterwards if it moved.
    ///
    /// This exists because `poll_capture_delta` and `poll_capture_keys` both
    /// need to read from the single underlying X connection event queue:
    /// `poll_for_event` is destructive, so if each method read the
    /// connection directly, whichever one happened to be called first in a
    /// given tick would silently steal (and drop) the other kind of event.
    /// Routing both through this one drain and buffering into two separate
    /// `Vec`s means it doesn't matter which of the two public methods is
    /// called first, or how often either is called relative to the other.
    ///
    /// Deltas are computed incrementally rather than each being measured
    /// against the fixed center: the server reports each `MotionNotify`'s
    /// *absolute* position, and the recenter warp is only sent once after
    /// the whole batch is drained (queuing a warp per event would flood the
    /// server and generate self-inflicted motion events during fast
    /// movement). If every event's delta were instead computed against the
    /// same stale center, a batch of more than one event would double-count
    /// the earlier events' displacement — e.g. two queued moves to
    /// center+50 and then center+100 would wrongly report deltas of +50 and
    /// +100 (summing to 150 for an actual 100px move) instead of +50 and
    /// +50.
    fn drain_queued_events(&mut self) -> Result<(), X11Error> {
        let center = self.capture.as_ref().ok_or(X11Error::NotCapturing)?.center;

        let mut last = center;
        let mut new_events = Vec::new();
        while let Some(event) = self.conn.poll_for_event()? {
            match event {
                Event::MotionNotify(motion) => {
                    let pos = (motion.event_x as i32, motion.event_y as i32);
                    let dx = pos.0 - last.0;
                    let dy = pos.1 - last.1;
                    if dx != 0 || dy != 0 {
                        new_events.push(CaptureEvent::Motion { dx, dy });
                    }
                    last = pos;
                }
                Event::KeyPress(key) => new_events.push(CaptureEvent::Key {
                    keycode: key.detail,
                    pressed: true,
                }),
                Event::KeyRelease(key) => new_events.push(CaptureEvent::Key {
                    keycode: key.detail,
                    pressed: false,
                }),
                Event::ButtonPress(b) => match b.detail {
                    4 => new_events.push(CaptureEvent::Scroll { dx: 0, dy: -1 }),
                    5 => new_events.push(CaptureEvent::Scroll { dx: 0, dy: 1 }),
                    6 => new_events.push(CaptureEvent::Scroll { dx: -1, dy: 0 }),
                    7 => new_events.push(CaptureEvent::Scroll { dx: 1, dy: 0 }),
                    n => new_events.push(CaptureEvent::Button {
                        button: n,
                        pressed: true,
                    }),
                },
                // Wheel "buttons" only generate a press we already turned
                // into a notch; their release carries no information.
                Event::ButtonRelease(b) if !(4..=7).contains(&b.detail) => {
                    new_events.push(CaptureEvent::Button {
                        button: b.detail,
                        pressed: false,
                    });
                }
                _ => {}
            }
        }

        if last != center {
            // Requests are processed by the server strictly in the order
            // they were sent on this connection, so it's safe to queue this
            // warp without waiting for a reply — the next batch of
            // MotionNotify events will already be relative to the new
            // center by the time we read them.
            self.conn
                .warp_pointer(
                    NONE,
                    self.root,
                    0,
                    0,
                    0,
                    0,
                    center.0 as i16,
                    center.1 as i16,
                )?
                .ignore_error();
            self.conn.flush()?;
        }

        // Recheck capture state: `?` above returns early if it was cleared
        // mid-drain, which can't actually happen since we hold `&mut self`,
        // but keeps this robust to future refactors.
        let state = self.capture.as_mut().ok_or(X11Error::NotCapturing)?;
        state.pending.extend(new_events);
        Ok(())
    }

    /// Drains all pointer-motion events received since the last call to
    /// this or [`LocalCursor::poll_capture_keys`], each converted to a delta
    /// *from the previous event in this same batch* (or from the center
    /// reference point, for the first event in the batch), and re-centers
    /// the pointer after reading if it moved. Returns an empty `Vec` if
    /// nothing moved. Must be called after [`LocalCursor::begin_capture`].
    ///
    /// This is non-blocking: it only looks at events already buffered by the
    /// connection (see the crate docs / integration notes on driving this
    /// from an async context).
    pub fn poll_capture_delta(&mut self) -> Result<Vec<(i32, i32)>, X11Error> {
        self.drain_queued_events()?;
        let state = self.capture.as_mut().ok_or(X11Error::NotCapturing)?;
        let mut deltas = Vec::new();
        state.pending.retain(|ev| match ev {
            CaptureEvent::Motion { dx, dy } => {
                deltas.push((*dx, *dy));
                false
            }
            _ => true,
        });
        Ok(deltas)
    }

    /// Drains *every* event received since the last poll, motion, keys,
    /// buttons and wheel notches alike, in the exact order the server
    /// reported them. This is what a controller should use: it preserves
    /// press-then-drag ordering that the per-kind pollers cannot.
    pub fn poll_capture_events(&mut self) -> Result<Vec<CaptureEvent>, X11Error> {
        self.drain_queued_events()?;
        let state = self.capture.as_mut().ok_or(X11Error::NotCapturing)?;
        Ok(std::mem::take(&mut state.pending))
    }

    /// Drains all `KeyPress`/`KeyRelease` events received since the last
    /// call to this or [`LocalCursor::poll_capture_delta`], each reported
    /// as-is (no delta math needed, unlike pointer motion) as a
    /// `(keycode, pressed)` pair, in the order the server sent them. Returns
    /// an empty `Vec` if no keys were pressed or released. Must be called
    /// after [`LocalCursor::begin_capture`].
    ///
    /// This is non-blocking, exactly like `poll_capture_delta`: it only
    /// looks at events already buffered by the connection. It's safe to
    /// call this and `poll_capture_delta` in either order, at any relative
    /// frequency: both methods drain the same underlying connection through
    /// a shared internal buffer, so neither one can steal events that
    /// belong to the other.
    pub fn poll_capture_keys(&mut self) -> Result<Vec<(u8, bool)>, X11Error> {
        self.drain_queued_events()?;
        let state = self.capture.as_mut().ok_or(X11Error::NotCapturing)?;
        let mut keys = Vec::new();
        state.pending.retain(|ev| match ev {
            CaptureEvent::Key { keycode, pressed } => {
                keys.push((*keycode, *pressed));
                false
            }
            _ => true,
        });
        Ok(keys)
    }

    /// Ungrabs the pointer and the keyboard, leaving capture mode.
    pub fn end_capture(&mut self) -> Result<(), X11Error> {
        if self.capture.take().is_none() {
            return Err(X11Error::NotCapturing);
        }
        self.conn.ungrab_pointer(x11rb::CURRENT_TIME)?.check()?;
        self.conn.ungrab_keyboard(x11rb::CURRENT_TIME)?.check()?;
        self.conn.flush()?;
        Ok(())
    }

    /// Injects a relative pointer motion (the "inject" role) using the core
    /// `WarpPointer` request with both source and destination windows set to
    /// `None`.
    pub fn warp_relative(&self, dx: i32, dy: i32) -> Result<(), X11Error> {
        self.conn
            .warp_pointer(NONE, NONE, 0, 0, 0, 0, dx as i16, dy as i16)?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }

    /// Places the pointer at an absolute position on the root window (used
    /// when handing control back to local after a capture session).
    pub fn warp_absolute(&self, x: i32, y: i32) -> Result<(), X11Error> {
        self.conn
            .warp_pointer(NONE, self.root, 0, 0, 0, 0, x as i16, y as i16)?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }

    /// Injects a key press or release (the "inject" role) using the XTest
    /// extension's `FakeInput` request. Unlike `warp_relative`/
    /// `warp_absolute`, this cannot be done with a core-protocol request —
    /// there's no core way for a client to synthesize input — hence this
    /// crate's `x11rb` dependency needing the `"xtest"` feature.
    ///
    /// `keycode` is a raw X11 keycode, passed straight through with no
    /// remapping (matches `protocol::Message::KeyEvent`, which is likewise
    /// unmapped for now since both ends are X11-only).
    pub fn inject_key(&self, keycode: u8, pressed: bool) -> Result<(), X11Error> {
        let event_type = if pressed {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(event_type, keycode, 0, self.root, 0, 0, 0)?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }

    /// Injects a pointer button press or release via XTest.
    pub fn inject_button(&self, button: u8, pressed: bool) -> Result<(), X11Error> {
        let event_type = if pressed {
            BUTTON_PRESS_EVENT
        } else {
            BUTTON_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(event_type, button, 0, self.root, 0, 0, 0)?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }

    /// Injects wheel notches. X11 models the wheel as buttons 4/5
    /// (up/down) and 6/7 (left/right); each notch is a press+release.
    /// Counts are capped so a corrupt message can't spin the server.
    pub fn inject_scroll(&self, dx: i32, dy: i32) -> Result<(), X11Error> {
        const MAX_NOTCHES: i32 = 64;
        let notches = |count: i32, button: u8| -> Result<(), X11Error> {
            for _ in 0..count.clamp(0, MAX_NOTCHES) {
                self.inject_button(button, true)?;
                self.inject_button(button, false)?;
            }
            Ok(())
        };
        notches(-dy, 4)?;
        notches(dy, 5)?;
        notches(-dx, 6)?;
        notches(dx, 7)?;
        Ok(())
    }

    /// The physical monitors currently configured on this display, from
    /// RandR 1.5 `GetMonitors`. Falls back to one monitor covering the
    /// whole root window when RandR is unavailable or reports nothing
    /// (e.g. a bare `Xvfb`), so callers can always rely on at least one
    /// entry. Sorted left-to-right, top-to-bottom for stable output.
    pub fn monitors(&self) -> Result<Vec<Monitor>, X11Error> {
        let mut out = Vec::new();
        if let Ok(cookie) = self.conn.randr_get_monitors(self.root, true) {
            if let Ok(reply) = cookie.reply() {
                for m in reply.monitors {
                    if m.width == 0 || m.height == 0 {
                        continue;
                    }
                    let name = self
                        .conn
                        .get_atom_name(m.name)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                        .unwrap_or_default();
                    out.push(Monitor {
                        name,
                        x: m.x as i32,
                        y: m.y as i32,
                        width: m.width as i32,
                        height: m.height as i32,
                        primary: m.primary,
                    });
                }
            }
        }
        if out.is_empty() {
            out.push(Monitor {
                name: "screen".to_string(),
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
                primary: true,
            });
        }
        out.sort_by_key(|m| (m.x, m.y));
        Ok(out)
    }

    /// The keycode currently mapped to X keysym `keysym` (e.g. `0xff1b`
    /// for Escape), if any. Used to recognise hotkeys in captured input
    /// without hard-coding a keyboard layout's keycodes.
    pub fn keycode_for_keysym(&self, keysym: u32) -> Result<Option<u8>, X11Error> {
        let setup = self.conn.setup();
        let min = setup.min_keycode;
        let count = setup.max_keycode - min + 1;
        let mapping = self.conn.get_keyboard_mapping(min, count)?.reply()?;
        let per = mapping.keysyms_per_keycode as usize;
        if per == 0 {
            return Ok(None);
        }
        for (i, syms) in mapping.keysyms.chunks(per).enumerate() {
            if syms.contains(&keysym) {
                return Ok(Some(min + i as u8));
            }
        }
        Ok(None)
    }
}
