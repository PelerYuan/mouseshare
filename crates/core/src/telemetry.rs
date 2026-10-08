//! Live state a frontend can poll every frame: per-device connection
//! status, latency, monitors learned during the handshake, who currently has
//! the cursor, and a queue of one-shot notices for toasts.
//!
//! This deliberately is *state to look at*, not a callback API: both the
//! CLI and the GUI keep emitting/reading it the same way, and `tracing`
//! stays the channel for free-form logs.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use mouseshare_protocol::MonitorInfo;

/// Connection state of one remote device (controller side).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerStatus {
    /// Looking for / dialing the device, or waiting to retry.
    Connecting,
    Connected,
    /// The device rejected our pairing code.
    AuthFailed,
    /// Carries a human-readable reason.
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct PeerState {
    pub status: PeerStatus,
    /// Round-trip time of the last ping, in milliseconds.
    pub latency_ms: Option<u32>,
    /// Monitors the device reported (empty until the first handshake).
    pub monitors: Vec<MonitorInfo>,
}

/// One-shot events worth a toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    PeerConnected(String),
    PeerLost(String),
    PeerAuthFailed(String),
    /// Target role: a controller authenticated and connected.
    ControllerConnected(String),
    ControllerLeft(String),
    /// Target role: a connection attempt was refused (wrong code / rate
    /// limit). Carries the remote IP.
    IncomingRejected(String),
    /// Control was pulled back to this computer with the hotkey.
    ReturnedByHotkey,
}

#[derive(Default)]
pub struct TelemetryInner {
    pub peers: HashMap<String, PeerState>,
    /// Target role: the controller currently connected.
    pub controller: Option<String>,
    /// Controller role: the remote device currently holding the cursor.
    pub active: Option<String>,
    pub local_monitors: Vec<MonitorInfo>,
    /// Set once the role's main loop has started.
    pub running: bool,
    notices: VecDeque<Notice>,
}

/// Cheap-to-clone shared handle.
#[derive(Clone, Default)]
pub struct Telemetry(Arc<Mutex<TelemetryInner>>);

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut TelemetryInner) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// A consistent copy of everything except the notice queue.
    pub fn snapshot(&self) -> TelemetrySnapshot {
        self.with(|t| TelemetrySnapshot {
            peers: t.peers.clone(),
            controller: t.controller.clone(),
            active: t.active.clone(),
            local_monitors: t.local_monitors.clone(),
            running: t.running,
        })
    }

    pub fn drain_notices(&self) -> Vec<Notice> {
        self.with(|t| t.notices.drain(..).collect())
    }

    pub fn clear(&self) {
        self.with(|t| *t = TelemetryInner::default());
    }

    pub(crate) fn notice(&self, n: Notice) {
        self.with(|t| {
            if t.notices.len() >= 32 {
                t.notices.pop_front();
            }
            t.notices.push_back(n);
        });
    }

    pub(crate) fn set_running(&self, running: bool) {
        self.with(|t| t.running = running);
    }

    pub(crate) fn set_status(&self, id: &str, status: PeerStatus) {
        self.with(|t| {
            let e = t.peers.entry(id.to_string()).or_insert(PeerState {
                status: PeerStatus::Connecting,
                latency_ms: None,
                monitors: Vec::new(),
            });
            if status != PeerStatus::Connected {
                e.latency_ms = None;
            }
            e.status = status;
        });
    }

    pub(crate) fn set_latency(&self, id: &str, ms: u32) {
        self.with(|t| {
            if let Some(p) = t.peers.get_mut(id) {
                p.latency_ms = Some(ms);
            }
        });
    }

    pub(crate) fn set_peer_monitors(&self, id: &str, monitors: Vec<MonitorInfo>) {
        self.with(|t| {
            if let Some(p) = t.peers.get_mut(id) {
                p.monitors = monitors;
            }
        });
    }

    pub(crate) fn set_local_monitors(&self, monitors: Vec<MonitorInfo>) {
        self.with(|t| t.local_monitors = monitors);
    }

    pub(crate) fn set_active(&self, id: Option<&str>) {
        self.with(|t| t.active = id.map(str::to_string));
    }

    pub(crate) fn set_controller(&self, id: Option<&str>) {
        self.with(|t| t.controller = id.map(str::to_string));
    }
}

#[derive(Clone, Debug, Default)]
pub struct TelemetrySnapshot {
    pub peers: HashMap<String, PeerState>,
    pub controller: Option<String>,
    pub active: Option<String>,
    pub local_monitors: Vec<MonitorInfo>,
    pub running: bool,
}
