//! The actual controller/target runtime loops, shared by every frontend
//! (the `mouseshare` CLI binary and the `mouseshare-gui` GUI binary) so
//! there's exactly one implementation of "what a controller/target
//! actually does" to get right and keep tested.
//!
//! Callers only need a `LayoutConfig` and an address; observability is
//! deliberately left to `tracing` rather than a bespoke callback/event API
//! -- both frontends can install whatever `tracing_subscriber` layer suits
//! them (stderr logging for the CLI, an in-memory ring buffer for a GUI log
//! panel) without this crate needing to know about it.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mouseshare_discovery::Announcement;
use mouseshare_layout::{ControlState, EdgeDetector, LayoutConfig};
use mouseshare_net::{Connection, NetError};
use mouseshare_protocol::Message;
use mouseshare_x11input::{Clipboard, LocalCursor};

/// Pixels of hysteresis applied when control might return from a remote
/// screen to local, to avoid flicker right at the seam.
const REENTRY_MARGIN: i32 = 4;

/// Poll interval for the controller's local-position/capture-delta loop.
const TICK: Duration = Duration::from_millis(8);

/// How often each side checks whether its own clipboard changed. Coarser
/// than `TICK` on purpose -- clipboard sync has no latency requirement even
/// close to mouse motion's, and polling `GetSelectionOwner` is a local X
/// round trip that's wasteful to repeat 125 times a second for no reason.
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Aborts the wrapped task when dropped. Used to make sure the background
/// task that owns a connection's read half never outlives the loop that
/// owns its write half -- on every exit path, including the *outer*
/// `run_controller`/`run_target` future itself being aborted (e.g. a GUI's
/// Stop button), not just a normal `return`. A bare `JoinHandle` wouldn't
/// do this: dropping it only detaches the task, it doesn't stop it.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How long a caller should wait for mDNS replies before giving up, when
/// auto-discovering a target instead of dialing a known address.
pub const DEFAULT_DISCOVER_TIMEOUT: Duration = Duration::from_secs(3);

/// Returns `connect` unchanged if given, otherwise browses the LAN via mDNS
/// for a mouseshare target announcing the layout's other configured screen.
///
/// MVP is single-peer: the only sensible auto-discovery target is whichever
/// other screen this layout has configured alongside the local one.
pub async fn resolve_target_addr(
    layout: &LayoutConfig,
    connect: Option<SocketAddr>,
    timeout: Duration,
) -> anyhow::Result<SocketAddr> {
    if let Some(addr) = connect {
        return Ok(addr);
    }

    let remote_id = layout
        .screens
        .iter()
        .map(|s| s.id.clone())
        .find(|id| *id != layout.local_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no target address was given but the layout config has no other screen to discover"
            )
        })?;

    tracing::info!(screen_id = %remote_id, ?timeout, "no target address given; discovering target via mDNS");
    let peers =
        tokio::task::spawn_blocking(move || mouseshare_discovery::discover(timeout)).await??;
    peers
        .into_iter()
        .find(|p| p.screen_id == remote_id)
        .map(|p| p.addr)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no mouseshare target announcing screen_id {remote_id:?} was found on the LAN within {timeout:?}"
            )
        })
}

/// Runs the target role forever: listens for controller connections,
/// applies received `MouseMove`/`KeyEvent` messages to the local X server,
/// syncs the clipboard in both directions, and announces itself over mDNS
/// so controllers can auto-discover it.
///
/// Only returns on an unrecoverable error (e.g. failing to bind
/// `listen_addr`); per-connection errors are logged and the loop continues
/// to accept the next controller.
pub async fn run_target(layout: LayoutConfig, listen_addr: SocketAddr) -> anyhow::Result<()> {
    let local = layout.local_screen().clone();
    let listener = mouseshare_net::listen(listen_addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!(addr = %bound_addr, "target listening");

    // Kept alive for the lifetime of this function: dropping it would send
    // an mDNS goodbye and stop controllers from finding us.
    let _announcement = match Announcement::start(&local.id, bound_addr.port()) {
        Ok(a) => Some(a),
        Err(e) => {
            tracing::warn!(
                "mDNS announcement failed ({e}); target is still reachable by address, \
                 just not auto-discoverable"
            );
            None
        }
    };

    loop {
        let mut conn = listener.accept().await?;
        let peer = match conn
            .handshake_as_listener(local.id.clone(), local.width, local.height)
            .await
        {
            Ok(peer) => peer,
            Err(e) => {
                tracing::warn!("handshake with incoming controller failed: {e}");
                continue;
            }
        };
        tracing::info!(?peer, "controller connected");

        let cursor = match LocalCursor::connect(None) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to connect to local X display: {e}");
                continue;
            }
        };
        let clipboard = match Clipboard::connect(None) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("clipboard sync unavailable ({e}); continuing without it");
                None
            }
        };

        if let Err(e) = handle_target_connection(conn, &cursor, clipboard).await {
            tracing::warn!("controller connection ended: {e}");
        }
    }
}

/// Forwards messages arriving on `reader` to `tx`, ending the loop (and
/// sending the terminal error) once the connection closes or breaks.
/// Runs as its own task so the main per-connection loop below never has to
/// race a raw socket read inside `select!` -- see the cancellation-safety
/// note on `mouseshare_net::Connection::recv`.
async fn forward_incoming(
    mut reader: mouseshare_net::ConnReader,
    tx: tokio::sync::mpsc::UnboundedSender<Result<Message, NetError>>,
) {
    loop {
        let result = reader.recv().await;
        let is_err = result.is_err();
        if tx.send(result).is_err() || is_err {
            return;
        }
    }
}

async fn handle_target_connection(
    conn: Connection,
    cursor: &LocalCursor,
    mut clipboard: Option<Clipboard>,
) -> Result<(), NetError> {
    let (reader, mut writer) = conn.into_split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _reader_guard = AbortOnDrop(tokio::spawn(forward_incoming(reader, tx)));

    let mut clipboard_tick = tokio::time::interval(CLIPBOARD_POLL_INTERVAL);
    clipboard_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(Ok(Message::MouseMove { dx, dy })) => {
                        if let Err(e) = cursor.warp_relative(dx, dy) {
                            tracing::warn!("warp_relative failed: {e}");
                        }
                    }
                    Some(Ok(Message::KeyEvent { keycode, pressed })) => {
                        if let Err(e) = cursor.inject_key(keycode, pressed) {
                            tracing::warn!("inject_key failed: {e}");
                        }
                    }
                    Some(Ok(Message::ClipboardText(text))) => {
                        if let Some(clipboard) = clipboard.as_mut() {
                            // Logged by length, not content: clipboard text
                            // can be sensitive (passwords, etc).
                            let bytes = text.len();
                            match clipboard.set_text(text) {
                                Ok(()) => tracing::info!(bytes, "clipboard updated from controller"),
                                Err(e) => tracing::warn!("failed to apply clipboard update: {e}"),
                            }
                        }
                    }
                    Some(Ok(Message::Heartbeat)) => {}
                    Some(Ok(other)) => tracing::warn!(?other, "unexpected message from controller"),
                    Some(Err(NetError::ConnectionClosed)) => return Ok(()),
                    Some(Err(e)) => return Err(e),
                    // The reader task always sends a terminal Err before
                    // ending, so a closed channel with nothing received
                    // means it was aborted/dropped, not a clean close.
                    None => return Ok(()),
                }
            }
            _ = clipboard_tick.tick() => {
                if let Some(clipboard) = clipboard.as_mut() {
                    match clipboard.poll() {
                        Ok(Some(text)) => {
                            tracing::info!(bytes = text.len(), "sending local clipboard change to controller");
                            writer.send(&Message::ClipboardText(text)).await?;
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!("clipboard poll failed: {e}"),
                    }
                }
            }
        }
    }
}

/// Runs the controller role forever: connects to `target_addr`, then polls
/// the local cursor position/capture deltas at a fixed tick rate, handing
/// mouse and keyboard control off to the target and back based on
/// `mouseshare_layout::EdgeDetector`. Clipboard sync runs alongside this,
/// independent of which side currently has control.
///
/// Only returns on an unrecoverable error (failing to connect, a fatal X11
/// or network error).
pub async fn run_controller(layout: LayoutConfig, target_addr: SocketAddr) -> anyhow::Result<()> {
    let local = layout.local_screen().clone();

    let mut conn = mouseshare_net::connect(target_addr).await?;
    let peer = conn
        .handshake_as_dialer(local.id.clone(), local.width, local.height)
        .await?;
    tracing::info!(?peer, "connected to target");

    let (reader, mut writer) = conn.into_split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _reader_guard = AbortOnDrop(tokio::spawn(forward_incoming(reader, tx)));

    let mut cursor = LocalCursor::connect(None)?;
    let mut detector = EdgeDetector::new(layout, REENTRY_MARGIN);

    let mut clipboard = match Clipboard::connect(None) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!("clipboard sync unavailable ({e}); continuing without it");
            None
        }
    };
    let mut next_clipboard_poll = Instant::now();

    loop {
        match detector.state().clone() {
            ControlState::Local => {
                let (lx, ly) = cursor.query_pointer()?;
                if let Some(transition) = detector.on_local_move(lx, ly) {
                    match transition.new_state {
                        ControlState::Remote(peer_id) => {
                            cursor.begin_capture()?;
                            tracing::info!(target = %peer_id, "control handed off to remote");
                        }
                        ControlState::Local => unreachable!(
                            "on_local_move only returns Some(..) when the state actually changed"
                        ),
                    }
                }
            }
            ControlState::Remote(_) => {
                let mut still_remote = true;
                for (dx, dy) in cursor.poll_capture_delta()? {
                    match detector.on_remote_delta(dx, dy) {
                        None => {
                            writer.send(&Message::MouseMove { dx, dy }).await?;
                        }
                        Some(transition) if transition.new_state == ControlState::Local => {
                            // This delta is the one that crossed back onto
                            // the local screen: it isn't forwarded (control
                            // is ours again), the pointer is placed exactly
                            // at the crossing point, and the OS cursor is
                            // ungrabbed. Any further deltas already queued
                            // in this same batch are relative to the old
                            // (now irrelevant) capture center and are
                            // dropped — at the tick rate used here that's at
                            // most a fraction of a pixel of local movement.
                            cursor.end_capture()?;
                            if let Some((lx, ly)) = transition.local_target {
                                cursor.warp_absolute(lx, ly)?;
                            }
                            tracing::info!("control returned to local");
                            still_remote = false;
                            break;
                        }
                        Some(transition) => {
                            tracing::warn!(
                                ?transition,
                                "hand-off to a different remote screen isn't supported yet \
                                 (single-peer MVP); dropping this delta"
                            );
                        }
                    }
                }
                // Only poll for keys if capture is still active: if the loop
                // above just ended it (control returned to local), the
                // keyboard grab is already released.
                if still_remote {
                    for (keycode, pressed) in cursor.poll_capture_keys()? {
                        writer.send(&Message::KeyEvent { keycode, pressed }).await?;
                    }
                }
            }
        }

        loop {
            match rx.try_recv() {
                Ok(Ok(Message::ClipboardText(text))) => {
                    if let Some(clipboard) = clipboard.as_mut() {
                        let bytes = text.len();
                        match clipboard.set_text(text) {
                            Ok(()) => tracing::info!(bytes, "clipboard updated from target"),
                            Err(e) => tracing::warn!("failed to apply clipboard update: {e}"),
                        }
                    }
                }
                Ok(Ok(Message::Heartbeat)) => {}
                Ok(Ok(other)) => tracing::warn!(?other, "unexpected message from target"),
                Ok(Err(NetError::ConnectionClosed)) => return Ok(()),
                Ok(Err(e)) => return Err(e.into()),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                // Reader task always sends a terminal Err before its loop
                // ends, so this means it was aborted/dropped instead.
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        if let Some(clipboard) = clipboard.as_mut() {
            if Instant::now() >= next_clipboard_poll {
                next_clipboard_poll = Instant::now() + CLIPBOARD_POLL_INTERVAL;
                match clipboard.poll() {
                    Ok(Some(text)) => {
                        tracing::info!(
                            bytes = text.len(),
                            "sending local clipboard change to target"
                        );
                        writer.send(&Message::ClipboardText(text)).await?;
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("clipboard poll failed: {e}"),
                }
            }
        }

        tokio::time::sleep(TICK).await;
    }
}
