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
use std::time::Duration;

use mouseshare_discovery::Announcement;
use mouseshare_layout::{ControlState, EdgeDetector, LayoutConfig};
use mouseshare_net::{Connection, NetError};
use mouseshare_protocol::Message;
use mouseshare_x11input::LocalCursor;

/// Pixels of hysteresis applied when control might return from a remote
/// screen to local, to avoid flicker right at the seam.
const REENTRY_MARGIN: i32 = 4;

/// Poll interval for the controller's local-position/capture-delta loop.
const TICK: Duration = Duration::from_millis(8);

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
    let peers = tokio::task::spawn_blocking(move || mouseshare_discovery::discover(timeout)).await??;
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
/// and announces itself over mDNS so controllers can auto-discover it.
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

        if let Err(e) = handle_target_connection(&mut conn, &cursor).await {
            tracing::warn!("controller connection ended: {e}");
        }
    }
}

async fn handle_target_connection(
    conn: &mut Connection,
    cursor: &LocalCursor,
) -> Result<(), NetError> {
    loop {
        match conn.recv().await {
            Ok(Message::MouseMove { dx, dy }) => {
                if let Err(e) = cursor.warp_relative(dx, dy) {
                    tracing::warn!("warp_relative failed: {e}");
                }
            }
            Ok(Message::KeyEvent { keycode, pressed }) => {
                if let Err(e) = cursor.inject_key(keycode, pressed) {
                    tracing::warn!("inject_key failed: {e}");
                }
            }
            Ok(Message::Heartbeat) => {}
            Ok(other) => tracing::warn!(?other, "unexpected message from controller"),
            Err(NetError::ConnectionClosed) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

/// Runs the controller role forever: connects to `target_addr`, then polls
/// the local cursor position/capture deltas at a fixed tick rate, handing
/// mouse and keyboard control off to the target and back based on
/// `mouseshare_layout::EdgeDetector`.
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

    let mut cursor = LocalCursor::connect(None)?;
    let mut detector = EdgeDetector::new(layout, REENTRY_MARGIN);

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
                            conn.send(&Message::MouseMove { dx, dy }).await?;
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
                        conn.send(&Message::KeyEvent { keycode, pressed }).await?;
                    }
                }
            }
        }
        tokio::time::sleep(TICK).await;
    }
}
