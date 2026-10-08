//! The controller/target runtime shared by every frontend (the `mouseshare`
//! CLI and the GUI), so there is exactly one implementation of "what a
//! controller/target actually does" to get right and keep tested.
//!
//! Frontends hand in a layout, pairing codes and [`SessionOptions`], and
//! observe progress through [`Telemetry`] (state to poll) plus `tracing`
//! (free-form logs).

mod controller;
mod hotkey;
mod target;
mod telemetry;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use mouseshare_layout::Monitor;
use mouseshare_protocol::MonitorInfo;

pub use controller::{run_controller, TargetSpec};
pub use hotkey::{Hotkey, HotkeyMatcher};
pub use mouseshare_net::{PairingCode, PairingCodeError};
pub use target::run_target;
pub use telemetry::{Notice, PeerState, PeerStatus, Telemetry, TelemetryInner, TelemetrySnapshot};

/// Pixels of hysteresis applied when control might return from a remote
/// device to local, to avoid flicker right at the seam.
pub(crate) const REENTRY_MARGIN: i32 = 4;

/// Poll interval for the controller's local-position/capture loop.
pub(crate) const TICK: Duration = Duration::from_millis(8);

/// How often each side checks whether its own clipboard changed.
pub(crate) const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// How often a side checks whether its monitor arrangement changed.
pub(crate) const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How often the controller pings each target.
pub(crate) const PING_INTERVAL: Duration = Duration::from_secs(2);

/// A connection with no traffic at all for this long is considered dead.
pub(crate) const DEAD_AFTER: Duration = Duration::from_secs(10);

/// Handshake deadline (both roles), so a silent peer cannot wedge anything.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a caller should wait for mDNS replies when auto-discovering.
pub const DEFAULT_DISCOVER_TIMEOUT: Duration = Duration::from_secs(3);

/// Largest clipboard payload we will ever forward (the wire frame limit is
/// 1 MiB; leave room for framing and AEAD overhead).
pub const CLIPBOARD_HARD_CAP: usize = 900 * 1024;

/// User-tunable behaviour of a running session.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// Emergency-return hotkey; `None` disables it.
    pub hotkey: Option<Hotkey>,
    /// Require the cursor to rest on a bordering edge this long before
    /// control is handed to a neighbour (zero = immediate).
    pub edge_dwell: Duration,
    /// Do not switch devices while a mouse button is held (dragging).
    pub hold_guard: bool,
    /// Sync clipboard text between devices.
    pub clipboard: bool,
    /// Clipboard payloads larger than this are not synced.
    pub clipboard_max_bytes: usize,
    /// Invert the direction of forwarded wheel scrolling.
    pub natural_scroll: bool,
    /// Per-remote-device pointer speed multiplier (default 1.0).
    pub sensitivity: HashMap<String, f32>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            hotkey: Some(Hotkey::default()),
            edge_dwell: Duration::ZERO,
            hold_guard: true,
            clipboard: true,
            clipboard_max_bytes: 256 * 1024,
            natural_scroll: false,
            sensitivity: HashMap::new(),
        }
    }
}

impl SessionOptions {
    pub(crate) fn clipboard_cap(&self) -> usize {
        self.clipboard_max_bytes.min(CLIPBOARD_HARD_CAP)
    }
}

pub(crate) fn to_monitors(infos: &[MonitorInfo]) -> Vec<Monitor> {
    infos
        .iter()
        .map(|m| Monitor::new(m.name.clone(), m.x, m.y, m.width, m.height))
        .collect()
}

/// This machine's monitors as the wire type, falling back to a single
/// monitor covering the whole screen if RandR yields nothing.
pub(crate) fn detect_monitors(cursor: &mouseshare_x11input::LocalCursor) -> Vec<MonitorInfo> {
    let mons = cursor.monitors().unwrap_or_default();
    if mons.is_empty() {
        let (w, h) = cursor.screen_size();
        return vec![MonitorInfo {
            name: "screen".into(),
            x: 0,
            y: 0,
            width: w,
            height: h,
            primary: true,
        }];
    }
    mons.into_iter()
        .map(|m| MonitorInfo {
            name: m.name,
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
            primary: m.primary,
        })
        .collect()
}

/// Looks up the address of each device in `ids` that has no explicit
/// address, with one mDNS browse. Devices that are not found are simply
/// absent from the result.
pub async fn discover_addrs(
    ids: &[String],
    timeout: Duration,
) -> anyhow::Result<HashMap<String, SocketAddr>> {
    let peers =
        tokio::task::spawn_blocking(move || mouseshare_discovery::discover(timeout)).await??;
    Ok(ids
        .iter()
        .filter_map(|id| {
            peers
                .iter()
                .find(|p| &p.screen_id == id)
                .map(|p| (id.clone(), p.addr))
        })
        .collect())
}

/// Aborts the wrapped task when dropped, so a task can never outlive the
/// loop that owns it -- including when the outer future itself is aborted
/// (a GUI's Stop button), not just on a normal return.
pub(crate) struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Pumps messages from a connection's read half into `tx`, ending after the
/// first error. Running the read in its own task keeps `select!` loops
/// cancellation-safe (see `mouseshare_net::ConnReader::recv`).
pub(crate) async fn forward_incoming(
    mut reader: mouseshare_net::ConnReader,
    tx: tokio::sync::mpsc::UnboundedSender<Result<mouseshare_protocol::Message, mouseshare_net::NetError>>,
) {
    loop {
        let result = reader.recv().await;
        let is_err = result.is_err();
        if tx.send(result).is_err() || is_err {
            return;
        }
    }
}
