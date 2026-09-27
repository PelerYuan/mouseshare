//! Direct X11-protocol pointer capture/injection for mouseshare.
//!
//! This crate talks to an X server using raw core-protocol requests via
//! [`x11rb`] — no XTest extension is required for anything implemented here.
//!
//! There are two roles:
//!
//! * **Capture** (the machine that currently owns the physical mouse):
//!   [`LocalCursor::begin_capture`] actively grabs the pointer, confines it to
//!   the root window, and warps it to a fixed center point. From then on,
//!   [`LocalCursor::poll_capture_delta`] drains queued `MotionNotify` events,
//!   turning each one into a `(dx, dy)` delta relative to that center point,
//!   and re-warps the pointer back to center after every event so the
//!   visible OS cursor never actually travels. This is the same trick
//!   Synergy/Barrier use on X11.
//! * **Inject** (the machine currently receiving forwarded deltas):
//!   [`LocalCursor::warp_relative`] applies a relative pointer motion using
//!   the core `WarpPointer` request (source and destination windows both
//!   `None`), and [`LocalCursor::warp_absolute`] places the pointer at an
//!   exact position on the root window (source window `None`, destination
//!   window the root window).
//!
//! See the crate-level tests in `tests/` for an end-to-end exercise of both
//! roles against real (headless) `Xvfb` servers.

use x11rb::connection::Connection;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::protocol::xproto::{ConnectionExt, EventMask, GrabMode, GrabStatus, Window};
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

    /// `GrabPointer` did not return `Success` (e.g. some other client
    /// already has an active pointer grab).
    #[error("pointer grab failed with status {0:?}")]
    GrabFailed(GrabStatus),

    /// [`LocalCursor::poll_capture_delta`] or [`LocalCursor::end_capture`]
    /// was called while not in capture mode.
    #[error("not currently capturing the pointer")]
    NotCapturing,
}

/// A reference point plus grab bookkeeping used while in "capture" mode.
struct CaptureState {
    center: (i32, i32),
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

    /// Grabs the pointer (confined to the root window) and warps it to a
    /// fixed center reference point, entering "capture" mode. Calling this
    /// again while already capturing is a no-op that re-confirms the grab.
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

        let center = (self.width / 2, self.height / 2);
        self.warp_absolute(center.0, center.1)?;
        // Drain and discard the MotionNotify that our own warp just queued
        // up, so the first real poll_capture_delta() starts from a clean
        // slate (its delta relative to `center` would be zero anyway, but
        // this also flushes the connection).
        while self.conn.poll_for_event()?.is_some() {}

        self.capture = Some(CaptureState { center });
        Ok(())
    }

    /// Drains all pointer-motion events received since the last call, each
    /// already converted to a delta from the center reference point, and
    /// re-centers the pointer after reading. Returns an empty `Vec` if
    /// nothing moved. Must be called after [`LocalCursor::begin_capture`].
    ///
    /// This is non-blocking: it only looks at events already buffered by the
    /// connection (see the crate docs / integration notes on driving this
    /// from an async context).
    pub fn poll_capture_delta(&mut self) -> Result<Vec<(i32, i32)>, X11Error> {
        let center = self.capture.as_ref().ok_or(X11Error::NotCapturing)?.center;

        let mut deltas = Vec::new();
        let mut needs_recenter = false;
        while let Some(event) = self.conn.poll_for_event()? {
            if let Event::MotionNotify(motion) = event {
                let dx = motion.event_x as i32 - center.0;
                let dy = motion.event_y as i32 - center.1;
                if dx != 0 || dy != 0 {
                    deltas.push((dx, dy));
                    needs_recenter = true;
                }
            }
            // Other event kinds (button presses, etc.) are outside this
            // crate's scope and are intentionally dropped.
        }

        if needs_recenter {
            // Requests are processed by the server strictly in the order
            // they were sent on this connection, so it's safe to queue this
            // warp without waiting for a reply for every single event above
            // — the next batch of MotionNotify events will already be
            // relative to the new center by the time we read them.
            self.conn
                .warp_pointer(NONE, self.root, 0, 0, 0, 0, center.0 as i16, center.1 as i16)?
                .ignore_error();
            self.conn.flush()?;
        }

        Ok(deltas)
    }

    /// Ungrabs the pointer, leaving capture mode.
    pub fn end_capture(&mut self) -> Result<(), X11Error> {
        if self.capture.take().is_none() {
            return Err(X11Error::NotCapturing);
        }
        self.conn.ungrab_pointer(x11rb::CURRENT_TIME)?.check()?;
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
}
