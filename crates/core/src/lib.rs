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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mouseshare_discovery::Announcement;
use mouseshare_layout::{ControlState, EdgeDetector, LayoutConfig, Transition};
use mouseshare_net::{Connection, NetError};
use mouseshare_protocol::Message;
use mouseshare_x11input::{Clipboard, LocalCursor};

/// Per-target connection state, reported live so a GUI can show a status dot
/// on each device row instead of one opaque app-global message -- this is
/// real data backing that dot, not just a color token with nothing behind
/// it: it's written directly from the same code paths that already know
/// whether a given target connected, failed, or dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerStatus {
    /// Dial/handshake in flight; not yet known to succeed or fail.
    Connecting,
    Connected,
    /// Carries a human-readable reason (a failed dial, a failed handshake,
    /// or the error a live connection ended with).
    Failed(String),
}

/// Shared map a caller can poll to render per-device connection status.
/// Keyed by screen id. A GUI clones this `Arc` before calling
/// `run_controller`/`run_target` and reads it from the UI thread each frame;
/// callers that don't care (the CLI) can just make one and drop it.
pub type PeerStatusMap = Arc<Mutex<HashMap<String, PeerStatus>>>;

/// Shared cell a Target-role caller can poll to show which controller (if
/// any) currently has this machine, e.g. "Connected to desk-1" in a GUI.
/// `None` means no controller is currently connected.
pub type ControllerIdCell = Arc<Mutex<Option<String>>>;

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

/// Resolves an address for every remote (non-local) screen configured in
/// `layout`, so the controller can connect out to all of them. `overrides`
/// supplies an explicit address for any screen id that shouldn't rely on
/// mDNS (e.g. a user-typed "Connect address"); every remote screen without
/// an override is auto-discovered on the LAN in a single mDNS browse.
pub async fn resolve_target_addrs(
    layout: &LayoutConfig,
    overrides: &HashMap<String, SocketAddr>,
    timeout: Duration,
) -> anyhow::Result<HashMap<String, SocketAddr>> {
    let remote_ids: Vec<String> = layout
        .screens
        .iter()
        .map(|s| s.id.clone())
        .filter(|id| *id != layout.local_id)
        .collect();
    if remote_ids.is_empty() {
        anyhow::bail!("layout config has no remote screens besides the local one");
    }

    let mut resolved = HashMap::new();
    let mut need_discovery = Vec::new();
    for id in remote_ids {
        match overrides.get(&id) {
            Some(addr) => {
                resolved.insert(id, *addr);
            }
            None => need_discovery.push(id),
        }
    }

    if !need_discovery.is_empty() {
        tracing::info!(?need_discovery, ?timeout, "discovering targets via mDNS");
        let peers =
            tokio::task::spawn_blocking(move || mouseshare_discovery::discover(timeout)).await??;
        for id in need_discovery {
            let addr = peers
                .iter()
                .find(|p| p.screen_id == id)
                .map(|p| p.addr)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no mouseshare target announcing screen_id {id:?} was found on the LAN within {timeout:?}"
                    )
                })?;
            resolved.insert(id, addr);
        }
    }

    Ok(resolved)
}

/// Runs the target role forever: listens for controller connections,
/// applies received `MouseMove`/`KeyEvent` messages to the local X server,
/// syncs the clipboard in both directions, and announces itself over mDNS
/// so controllers can auto-discover it.
///
/// Only returns on an unrecoverable error (e.g. failing to bind
/// `listen_addr`); per-connection errors are logged and the loop continues
/// to accept the next controller.
///
/// `controller_status` is set to the connected controller's screen id for as
/// long as a controller connection is live, and back to `None` once it ends
/// -- a GUI can poll this to show "Connected to `<id>`" without this crate
/// needing to know anything about how that's rendered.
pub async fn run_target(
    layout: LayoutConfig,
    listen_addr: SocketAddr,
    controller_status: ControllerIdCell,
) -> anyhow::Result<()> {
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

        *controller_status.lock().unwrap() = Some(peer.screen_id.clone());
        let result = handle_target_connection(conn, &cursor, clipboard).await;
        *controller_status.lock().unwrap() = None;
        if let Err(e) = result {
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

/// Same as `forward_incoming`, but tags every message with which peer it
/// came from -- the controller multiplexes reads from every connected
/// target onto a single channel, since clipboard updates (and, once
/// bidirectional control exists, other events) can arrive from any of them
/// independent of which one currently has the mouse.
async fn forward_incoming_tagged(
    mut reader: mouseshare_net::ConnReader,
    screen_id: String,
    tx: tokio::sync::mpsc::UnboundedSender<(String, Result<Message, NetError>)>,
) {
    loop {
        let result = reader.recv().await;
        let is_err = result.is_err();
        if tx.send((screen_id.clone(), result)).is_err() || is_err {
            return;
        }
    }
}

/// One connected target: the write half kept for forwarding input to it,
/// plus the guard that keeps its background reader task alive for exactly
/// as long as this peer is considered connected.
struct PeerHandle {
    writer: mouseshare_net::ConnWriter,
    _reader_guard: AbortOnDrop,
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

/// Runs the controller role forever: connects to every address in `targets`
/// (keyed by the screen id it belongs to), then polls the local cursor
/// position/capture deltas at a fixed tick rate, handing mouse and keyboard
/// control off between the local screen and whichever remote screen the
/// cursor is currently over, based on `mouseshare_layout::EdgeDetector`.
/// Clipboard sync runs alongside this, independent of which side currently
/// has control, and is broadcast to every connected target.
///
/// `targets` must have an entry for every non-local screen in `layout`.
///
/// Every target is dialed independently: one bad address or a refused
/// connection only takes that target out of the session (recorded in
/// `peer_status` as `Failed`), it does not prevent the others from
/// connecting. Only returns an error up front if *none* of the targets
/// could be connected -- with nothing connected there is nothing to
/// control, same as the empty-targets case.
///
/// Returns once every target has disconnected (nothing left to control), or
/// on an unrecoverable error (a fatal X11 error). Losing just one of several
/// targets does not end the session -- control simply can't be handed off to
/// that screen anymore until it reconnects.
pub async fn run_controller(
    layout: LayoutConfig,
    targets: HashMap<String, SocketAddr>,
    peer_status: PeerStatusMap,
) -> anyhow::Result<()> {
    let local = layout.local_screen().clone();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut peers: HashMap<String, PeerHandle> = HashMap::new();
    for (screen_id, addr) in targets {
        peer_status
            .lock()
            .unwrap()
            .insert(screen_id.clone(), PeerStatus::Connecting);

        let connected = async {
            let mut conn = mouseshare_net::connect(addr).await?;
            let peer = conn
                .handshake_as_dialer(local.id.clone(), local.width, local.height)
                .await?;
            Ok::<_, anyhow::Error>((conn, peer))
        }
        .await;

        let (conn, peer) = match connected {
            Ok(ok) => ok,
            Err(e) => {
                tracing::warn!(target = %screen_id, %addr, "failed to connect: {e}");
                peer_status
                    .lock()
                    .unwrap()
                    .insert(screen_id, PeerStatus::Failed(e.to_string()));
                continue;
            }
        };
        tracing::info!(?peer, target = %screen_id, "connected to target");
        peer_status
            .lock()
            .unwrap()
            .insert(screen_id.clone(), PeerStatus::Connected);

        let (reader, writer) = conn.into_split();
        let reader_guard = AbortOnDrop(tokio::spawn(forward_incoming_tagged(
            reader,
            screen_id.clone(),
            tx.clone(),
        )));
        peers.insert(
            screen_id,
            PeerHandle {
                writer,
                _reader_guard: reader_guard,
            },
        );
    }
    // Only the reader tasks' clones should keep the channel open now.
    drop(tx);

    if peers.is_empty() {
        anyhow::bail!("could not connect to any target; nothing to control");
    }

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
        if peers.is_empty() {
            tracing::info!("every target has disconnected; nothing left to control");
            return Ok(());
        }

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
            ControlState::Remote(mut peer_id) => {
                let mut still_remote = true;
                for (dx, dy) in cursor.poll_capture_delta()? {
                    match detector.on_remote_delta(dx, dy) {
                        None => {
                            if let Some(peer) = peers.get_mut(&peer_id) {
                                peer.writer.send(&Message::MouseMove { dx, dy }).await?;
                            }
                            // No connection to this screen anymore: the
                            // delta is silently dropped rather than treated
                            // as an error, same as moving the mouse over a
                            // screen that was never configured.
                        }
                        Some(Transition {
                            new_state: ControlState::Local,
                            local_target,
                        }) => {
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
                            if let Some((lx, ly)) = local_target {
                                cursor.warp_absolute(lx, ly)?;
                            }
                            tracing::info!("control returned to local");
                            still_remote = false;
                            break;
                        }
                        Some(Transition {
                            new_state: ControlState::Remote(new_peer_id),
                            ..
                        }) => {
                            tracing::info!(
                                from = %peer_id, to = %new_peer_id,
                                "control handed off to a different remote screen"
                            );
                            peer_id = new_peer_id;
                        }
                    }
                }
                // Only poll for keys if capture is still active: if the loop
                // above just ended it (control returned to local), the
                // keyboard grab is already released.
                if still_remote {
                    let keys = cursor.poll_capture_keys()?;
                    if let Some(peer) = peers.get_mut(&peer_id) {
                        for (keycode, pressed) in keys {
                            peer.writer
                                .send(&Message::KeyEvent { keycode, pressed })
                                .await?;
                        }
                    }
                }
            }
        }

        loop {
            match rx.try_recv() {
                Ok((screen_id, Ok(Message::ClipboardText(text)))) => {
                    if let Some(clipboard) = clipboard.as_mut() {
                        let bytes = text.len();
                        match clipboard.set_text(text) {
                            Ok(()) => {
                                tracing::info!(bytes, from = %screen_id, "clipboard updated from target")
                            }
                            Err(e) => tracing::warn!("failed to apply clipboard update: {e}"),
                        }
                    }
                }
                Ok((_, Ok(Message::Heartbeat))) => {}
                Ok((screen_id, Ok(other))) => {
                    tracing::warn!(?other, from = %screen_id, "unexpected message from target")
                }
                Ok((screen_id, Err(NetError::ConnectionClosed))) => {
                    tracing::warn!(target = %screen_id, "target disconnected");
                    peers.remove(&screen_id);
                    peer_status.lock().unwrap().insert(
                        screen_id.clone(),
                        PeerStatus::Failed("disconnected".to_string()),
                    );
                    if detector.state() == &ControlState::Remote(screen_id) {
                        let (lx, ly) = detector.force_local();
                        cursor.end_capture()?;
                        cursor.warp_absolute(lx, ly)?;
                    }
                }
                Ok((screen_id, Err(e))) => {
                    tracing::warn!(target = %screen_id, "connection error: {e}");
                    peers.remove(&screen_id);
                    peer_status
                        .lock()
                        .unwrap()
                        .insert(screen_id.clone(), PeerStatus::Failed(e.to_string()));
                    if detector.state() == &ControlState::Remote(screen_id) {
                        let (lx, ly) = detector.force_local();
                        cursor.end_capture()?;
                        cursor.warp_absolute(lx, ly)?;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                // Reader tasks always send a terminal Err before ending, so
                // this means every one of them was aborted/dropped instead
                // (i.e. `peers` is already empty) rather than a clean close.
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
                            targets = peers.len(),
                            "sending local clipboard change to all targets"
                        );
                        for peer in peers.values_mut() {
                            if let Err(e) = peer
                                .writer
                                .send(&Message::ClipboardText(text.clone()))
                                .await
                            {
                                tracing::warn!("failed to send clipboard update: {e}");
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("clipboard poll failed: {e}"),
                }
            }
        }

        tokio::time::sleep(TICK).await;
    }
}
