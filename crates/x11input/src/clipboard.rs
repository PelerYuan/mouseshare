//! Shared clipboard sync via the X11 ICCCM `CLIPBOARD` selection.
//!
//! Only plain text (`UTF8_STRING`) is supported -- MVP scope, matching the
//! project's lightweight stance. Rich formats or anything larger than
//! [`MAX_CLIPBOARD_BYTES`] would need the `INCR` transfer protocol, which
//! this module deliberately doesn't implement.
//!
//! Detecting a local clipboard change (someone copied something on *this*
//! machine) is done by periodically comparing `GetSelectionOwner`'s answer
//! against what we last saw -- a single fast round trip to the local X
//! server, not to another client, so it's safe to call from an async loop
//! like the rest of this crate's methods. Fetching the actual text once an
//! ownership change is observed goes through a real `ConvertSelection`
//! round trip with another client, which *can* be slow or never answer;
//! that path is spread across multiple non-blocking `poll` calls instead of
//! blocking on it, via `awaiting_transfer`.
//!
//! Serving *our* clipboard content to other local apps (when this machine
//! is the one offering text received from the peer) requires answering
//! `SelectionRequest` events for as long as we're the owner -- also handled
//! by `poll`, so callers just need to invoke it regularly.

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    SelectionRequestEvent, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::COPY_DEPTH_FROM_PARENT;

use crate::X11Error;

/// Max clipboard text we'll transfer in one shot (no `INCR` support).
/// Generous for anything a human actually copy-pastes.
const MAX_CLIPBOARD_BYTES: u32 = 4 * 1024 * 1024;

/// A connection to a single X display dedicated to `CLIPBOARD` selection
/// handling, independent of [`crate::LocalCursor`]'s pointer/keyboard
/// connection since the two have unrelated lifecycles (clipboard sync runs
/// regardless of capture state).
pub struct Clipboard {
    conn: RustConnection,
    window: Window,
    atom_clipboard: u32,
    atom_utf8_string: u32,
    atom_targets: u32,
    atom_transfer: u32,

    /// The text we're currently offering while we own the selection (set by
    /// `set_text`), `None` if we don't currently own it.
    owned_text: Option<String>,
    /// The last text content seen from either direction, used to suppress
    /// redundant sends: applying a peer's update locally would otherwise
    /// look like a fresh local copy on the very next ownership check and
    /// get echoed straight back.
    last_known: Option<String>,
    /// The selection owner window last observed by `check_owner`, to only
    /// kick off a `ConvertSelection` when ownership actually changes.
    last_seen_owner: Window,
    /// Set after issuing `ConvertSelection`, cleared once its
    /// `SelectionNotify` reply has been drained by `poll` -- lets the wait
    /// for another client's (possibly slow) response span multiple
    /// non-blocking `poll` calls instead of blocking this one.
    awaiting_transfer: bool,
}

impl Clipboard {
    /// Connects to an X display. `display` overrides `$DISPLAY`, same as
    /// [`crate::LocalCursor::connect`].
    pub fn connect(display: Option<&str>) -> Result<Self, X11Error> {
        let (conn, screen_num) = x11rb::connect(display)?;
        let screen = &conn.setup().roots[screen_num];

        let window = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            screen.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )?;

        let atom_clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        let atom_utf8_string = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
        let atom_targets = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;
        let atom_transfer = conn.intern_atom(false, b"MOUSESHARE_CLIPBOARD")?.reply()?.atom;
        conn.flush()?;

        Ok(Self {
            conn,
            window,
            atom_clipboard,
            atom_utf8_string,
            atom_targets,
            atom_transfer,
            owned_text: None,
            last_known: None,
            last_seen_owner: x11rb::NONE,
            awaiting_transfer: false,
        })
    }

    /// Makes `text` available on this machine's clipboard for any app to
    /// paste, and records it as already-known so the next `poll` doesn't
    /// mistake our own new ownership for a local copy to report back.
    pub fn set_text(&mut self, text: String) -> Result<(), X11Error> {
        self.last_known = Some(text.clone());
        self.owned_text = Some(text);
        self.conn
            .set_selection_owner(self.window, self.atom_clipboard, x11rb::CURRENT_TIME)?;
        self.conn.flush()?;
        // We just became the owner; the next check_owner call would
        // otherwise see this as "someone else took it" for one tick.
        self.last_seen_owner = self.window;
        Ok(())
    }

    /// Services pending clipboard-related X11 events and checks for a new
    /// local selection owner, returning `Some(text)` exactly when this
    /// machine's clipboard now holds different text than the peer last
    /// knew about (call this regularly, e.g. once per tick).
    pub fn poll(&mut self) -> Result<Option<String>, X11Error> {
        let new_text = self.drain_events()?;
        self.check_owner()?;
        Ok(new_text)
    }

    fn drain_events(&mut self) -> Result<Option<String>, X11Error> {
        let mut new_text = None;
        while let Some(event) = self.conn.poll_for_event()? {
            match event {
                Event::SelectionNotify(ev)
                    if self.awaiting_transfer
                        && ev.selection == self.atom_clipboard
                        && ev.requestor == self.window =>
                {
                    self.awaiting_transfer = false;
                    if ev.property != x11rb::NONE {
                        if let Some(text) = self.read_transfer_property()? {
                            if self.last_known.as_deref() != Some(text.as_str()) {
                                self.last_known = Some(text.clone());
                                new_text = Some(text);
                            }
                        }
                    }
                }
                Event::SelectionRequest(req) if req.selection == self.atom_clipboard => {
                    self.service_request(req)?;
                }
                Event::SelectionClear(ev) if ev.selection == self.atom_clipboard => {
                    self.owned_text = None;
                }
                _ => {}
            }
        }
        Ok(new_text)
    }

    /// Cheap, server-local query (no other client involved) for who
    /// currently owns `CLIPBOARD`; kicks off a `ConvertSelection` transfer
    /// when it's changed to someone other than us.
    fn check_owner(&mut self) -> Result<(), X11Error> {
        if self.awaiting_transfer {
            // Don't start a second transfer while one's still in flight.
            return Ok(());
        }
        let owner = self.conn.get_selection_owner(self.atom_clipboard)?.reply()?.owner;
        if owner == self.last_seen_owner {
            return Ok(());
        }
        self.last_seen_owner = owner;
        if owner != x11rb::NONE && owner != self.window {
            self.conn.convert_selection(
                self.window,
                self.atom_clipboard,
                self.atom_utf8_string,
                self.atom_transfer,
                x11rb::CURRENT_TIME,
            )?;
            self.conn.flush()?;
            self.awaiting_transfer = true;
        }
        Ok(())
    }

    fn read_transfer_property(&self) -> Result<Option<String>, X11Error> {
        let reply = self
            .conn
            .get_property(
                true,
                self.window,
                self.atom_transfer,
                AtomEnum::ANY,
                0,
                MAX_CLIPBOARD_BYTES / 4,
            )?
            .reply()?;
        Ok(String::from_utf8(reply.value).ok())
    }

    /// Answers a `SelectionRequest` from some other local client asking for
    /// our clipboard content (we only get these while we're the owner).
    fn service_request(&mut self, req: SelectionRequestEvent) -> Result<(), X11Error> {
        let property = if req.property == x11rb::NONE { req.target } else { req.property };

        let accepted = if req.target == self.atom_targets {
            let targets = [self.atom_utf8_string, self.atom_targets];
            self.conn
                .change_property32(PropMode::REPLACE, req.requestor, property, AtomEnum::ATOM, &targets)?;
            true
        } else if req.target == self.atom_utf8_string {
            let text = self.owned_text.clone().unwrap_or_default();
            self.conn.change_property8(
                PropMode::REPLACE,
                req.requestor,
                property,
                self.atom_utf8_string,
                text.as_bytes(),
            )?;
            true
        } else {
            false
        };

        let notify = SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property: if accepted { property } else { x11rb::NONE },
        };
        self.conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }
}
