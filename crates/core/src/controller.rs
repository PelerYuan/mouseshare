//! The controller role: owns the physical mouse/keyboard, decides which
//! device they drive, and forwards input to it.
//!
//! Layout: one supervisor task per target keeps a connection alive
//! (resolve, dial, authenticate, ping, reconnect with backoff) and talks to
//! the single input loop in [`run_controller`] through channels, so the input
//! loop never awaits the network.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mouseshare_layout::{ControlState, EdgeDetector, LayoutConfig, Transition};
use mouseshare_net::{NetError, PairingCode, PeerInfo};
use mouseshare_protocol::{Message, MonitorInfo};
use mouseshare_x11input::{CaptureEvent, Clipboard, LocalCursor};
use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::hotkey::HotkeyMatcher;
use crate::telemetry::{Notice, PeerStatus, Telemetry};
use crate::{
    detect_monitors, forward_incoming, to_monitors, AbortOnDrop, SessionOptions,
    CLIPBOARD_POLL_INTERVAL, DEAD_AFTER, HANDSHAKE_TIMEOUT, MONITOR_POLL_INTERVAL, PING_INTERVAL,
    REENTRY_MARGIN, TICK,
};

/// How to reach and authenticate to one target.
#[derive(Clone)]
pub struct TargetSpec {
    /// Explicit address; `None` means "find it by mDNS on every attempt".
    pub addr: Option<SocketAddr>,
    pub code: PairingCode,
}

enum PeerEvent {
    Connected {
        id: String,
        session: u64,
        monitors: Vec<MonitorInfo>,
        tx: UnboundedSender<Message>,
    },
    Message {
        id: String,
        session: u64,
        msg: Message,
    },
    Disconnected {
        id: String,
        session: u64,
    },
}

struct PeerLink {
    session: u64,
    tx: UnboundedSender<Message>,
}

fn net_status(e: &NetError) -> PeerStatus {
    match e {
        NetError::AuthFailed => PeerStatus::AuthFailed,
        other => PeerStatus::Failed(other.to_string()),
    }
}

/// Keeps one target connected forever.
async fn supervise_target(
    id: String,
    spec: TargetSpec,
    local: PeerInfo,
    ev_tx: UnboundedSender<PeerEvent>,
    tele: Telemetry,
) {
    let mut backoff = Duration::from_secs(1);
    let mut session: u64 = 0;
    loop {
        tele.set_status(&id, PeerStatus::Connecting);
        let started = Instant::now();
        let attempt = connect_once(&id, &spec, &local).await;
        let (conn, peer) = match attempt {
            Ok(ok) => ok,
            Err(e) => {
                tracing::warn!(target = %id, "connect failed: {e}");
                let status = net_status(&e);
                let wait = if status == PeerStatus::AuthFailed {
                    tele.notice(Notice::PeerAuthFailed(id.clone()));
                    // Never hammer a device that rejected our code.
                    Duration::from_secs(30)
                } else {
                    backoff
                };
                tele.set_status(&id, status);
                tokio::time::sleep(wait).await;
                backoff = (backoff * 2).min(Duration::from_secs(15));
                continue;
            }
        };
        session += 1;
        tracing::info!(target = %id, monitors = peer.monitors.len(), "connected");
        let (out_tx, out_rx) = unbounded_channel();
        tele.set_peer_monitors(&id, peer.monitors.clone());
        tele.set_status(&id, PeerStatus::Connected);
        tele.notice(Notice::PeerConnected(id.clone()));
        let _ = ev_tx.send(PeerEvent::Connected {
            id: id.clone(),
            session,
            monitors: peer.monitors,
            tx: out_tx,
        });

        let result = run_session(conn, &id, session, out_rx, &ev_tx, &tele).await;
        let _ = ev_tx.send(PeerEvent::Disconnected {
            id: id.clone(),
            session,
        });
        let reason = match result {
            Ok(()) => "disconnected".to_string(),
            Err(e) => e.to_string(),
        };
        tracing::warn!(target = %id, "session ended: {reason}");
        tele.set_status(&id, PeerStatus::Failed(reason));
        tele.notice(Notice::PeerLost(id.clone()));

        if started.elapsed() > Duration::from_secs(10) {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

async fn connect_once(
    id: &str,
    spec: &TargetSpec,
    local: &PeerInfo,
) -> Result<(mouseshare_net::Connection, PeerInfo), NetError> {
    let addr = match spec.addr {
        Some(a) => a,
        None => {
            let ids = [id.to_string()];
            let found = crate::discover_addrs(&ids, crate::DEFAULT_DISCOVER_TIMEOUT)
                .await
                .map_err(|e| NetError::Io(std::io::Error::other(e.to_string())))?;
            *found.get(id).ok_or_else(|| {
                NetError::Io(std::io::Error::other(format!(
                    "{id:?} was not found on the network"
                )))
            })?
        }
    };
    let timed_out = || NetError::Io(std::io::ErrorKind::TimedOut.into());
    let mut conn = tokio::time::timeout(Duration::from_secs(5), mouseshare_net::connect(addr))
        .await
        .map_err(|_| timed_out())??;
    let peer = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        conn.handshake_as_dialer(&spec.code, local),
    )
    .await
    .map_err(|_| timed_out())??;
    Ok((conn, peer))
}

/// Runs one live connection until it breaks, the controller drops it, or it
/// goes silent.
async fn run_session(
    conn: mouseshare_net::Connection,
    id: &str,
    session: u64,
    mut out_rx: UnboundedReceiver<Message>,
    ev_tx: &UnboundedSender<PeerEvent>,
    tele: &Telemetry,
) -> Result<(), NetError> {
    let (reader, mut writer) = conn.into_split();
    let (in_tx, mut in_rx) = unbounded_channel();
    let _reader_guard = AbortOnDrop(tokio::spawn(forward_incoming(reader, in_tx)));

    let epoch = Instant::now();
    let mut last_rx = Instant::now();
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            out = out_rx.recv() => match out {
                Some(m) => writer.send(&m).await?,
                None => return Ok(()),
            },
            inc = in_rx.recv() => {
                last_rx = Instant::now();
                match inc {
                    Some(Ok(Message::Ping(t))) => writer.send(&Message::Pong(t)).await?,
                    Some(Ok(Message::Pong(t))) => {
                        let now = epoch.elapsed().as_millis() as u64;
                        tele.set_latency(id, now.saturating_sub(t).min(u32::MAX as u64) as u32);
                    }
                    Some(Ok(msg)) => {
                        let _ = ev_tx.send(PeerEvent::Message { id: id.to_string(), session, msg });
                    }
                    Some(Err(NetError::ConnectionClosed)) | None => return Ok(()),
                    Some(Err(e)) => return Err(e),
                }
            }
            _ = ping.tick() => {
                if last_rx.elapsed() > DEAD_AFTER {
                    return Err(NetError::Io(std::io::ErrorKind::TimedOut.into()));
                }
                writer.send(&Message::Ping(epoch.elapsed().as_millis() as u64)).await?;
            }
        }
    }
}

/// Runs the controller role until the future is dropped (or a fatal local
/// error occurs): connects to every target, keeps them connected, and hands
/// the mouse and keyboard between this machine and whichever device the
/// cursor walks onto.
///
/// `targets` is keyed by device id; ids not present in `layout` are ignored.
pub async fn run_controller(
    mut layout: LayoutConfig,
    targets: HashMap<String, TargetSpec>,
    opts: SessionOptions,
    tele: Telemetry,
) -> anyhow::Result<()> {
    let cursor = LocalCursor::connect(None)?;

    // The truth about this machine's displays beats whatever the layout says.
    let local_monitors = detect_monitors(&cursor);
    if let Some(d) = layout.device_mut(&layout.local_id.clone()) {
        d.monitors = to_monitors(&local_monitors);
    }
    tele.set_local_monitors(local_monitors.clone());
    let local_info = PeerInfo {
        device_id: layout.local_id.clone(),
        monitors: local_monitors,
    };

    let (ev_tx, ev_rx) = unbounded_channel();
    let mut guards = Vec::new();
    for (id, spec) in targets {
        if layout.device(&id).is_none() || id == layout.local_id {
            tracing::warn!(target = %id, "ignoring target that is not a remote device in the layout");
            continue;
        }
        tele.set_status(&id, PeerStatus::Connecting);
        guards.push(AbortOnDrop(tokio::spawn(supervise_target(
            id,
            spec,
            local_info.clone(),
            ev_tx.clone(),
            tele.clone(),
        ))));
    }
    drop(ev_tx);
    if guards.is_empty() {
        anyhow::bail!("no remote devices to connect to");
    }

    let hotkey = opts.hotkey.as_ref().and_then(|hk| {
        let m = HotkeyMatcher::resolve(hk, &cursor);
        if m.is_none() {
            tracing::warn!("hotkey {hk} could not be resolved on this keyboard; disabled");
        }
        m
    });
    let clipboard = if opts.clipboard {
        match Clipboard::connect(None) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("clipboard sync unavailable ({e}); continuing without it");
                None
            }
        }
    } else {
        None
    };

    tele.set_running(true);
    let mut c = Controller {
        local_id: layout.local_id.clone(),
        detector: EdgeDetector::new(layout, REENTRY_MARGIN),
        cursor,
        peers: HashMap::new(),
        ev_rx,
        opts,
        tele: tele.clone(),
        hotkey,
        clipboard,
        held_keys: HashSet::new(),
        held_buttons: HashSet::new(),
        carry: (0.0, 0.0),
        edge_since: None,
        next_clipboard_poll: Instant::now(),
        next_monitor_poll: Instant::now() + MONITOR_POLL_INTERVAL,
        local_monitors: local_info.monitors,
    };
    let result = c.run().await;
    tele.set_running(false);
    tele.set_active(None);
    drop(guards);
    result
}

struct Controller {
    local_id: String,
    detector: EdgeDetector,
    cursor: LocalCursor,
    peers: HashMap<String, PeerLink>,
    ev_rx: UnboundedReceiver<PeerEvent>,
    opts: SessionOptions,
    tele: Telemetry,
    hotkey: Option<HotkeyMatcher>,
    clipboard: Option<Clipboard>,
    held_keys: HashSet<u8>,
    held_buttons: HashSet<u8>,
    carry: (f32, f32),
    edge_since: Option<Instant>,
    next_clipboard_poll: Instant,
    next_monitor_poll: Instant,
    local_monitors: Vec<MonitorInfo>,
}

impl Controller {
    async fn run(&mut self) -> anyhow::Result<()> {
        loop {
            self.drain_peer_events()?;
            match self.detector.state().clone() {
                ControlState::Local => self.tick_local()?,
                ControlState::Remote(id) => self.tick_remote(id)?,
            }
            self.tick_housekeeping();
            tokio::time::sleep(TICK).await;
        }
    }

    fn send(&self, id: &str, msg: Message) {
        if let Some(p) = self.peers.get(id) {
            let _ = p.tx.send(msg);
        }
    }

    fn drain_peer_events(&mut self) -> anyhow::Result<()> {
        loop {
            let ev = match self.ev_rx.try_recv() {
                Ok(ev) => ev,
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => return Ok(()),
            };
            match ev {
                PeerEvent::Connected {
                    id,
                    session,
                    monitors,
                    tx,
                } => {
                    if !monitors.is_empty() {
                        self.detector.update_monitors(&id, to_monitors(&monitors));
                    }
                    self.peers.insert(id, PeerLink { session, tx });
                }
                PeerEvent::Disconnected { id, session } => {
                    // A newer session may already have replaced this one.
                    if self.peers.get(&id).map(|p| p.session) != Some(session) {
                        continue;
                    }
                    self.peers.remove(&id);
                    if self.detector.state() == &ControlState::Remote(id.clone()) {
                        self.return_to_local(None, &id, true)?;
                    }
                }
                PeerEvent::Message { id, session, msg } => {
                    if self.peers.get(&id).map(|p| p.session) != Some(session) {
                        continue;
                    }
                    match msg {
                        Message::ClipboardText(text) => self.apply_clipboard(&id, text),
                        Message::MonitorsChanged(monitors) if !monitors.is_empty() => {
                            self.tele.set_peer_monitors(&id, monitors.clone());
                            self.detector.update_monitors(&id, to_monitors(&monitors));
                        }
                        other => tracing::debug!(?other, from = %id, "ignored message from target"),
                    }
                }
            }
        }
    }

    fn apply_clipboard(&mut self, from: &str, text: String) {
        if !self.opts.clipboard || text.len() > self.opts.clipboard_cap() {
            return;
        }
        if let Some(cb) = self.clipboard.as_mut() {
            let bytes = text.len();
            match cb.set_text(text) {
                Ok(()) => tracing::info!(bytes, from, "clipboard updated from target"),
                Err(e) => tracing::warn!("failed to apply clipboard update: {e}"),
            }
        }
    }

    fn tick_local(&mut self) -> anyhow::Result<()> {
        let (lx, ly) = self.cursor.query_pointer()?;
        let snapshot = self.detector.clone();
        let Some(t) = self.detector.on_local_move(lx, ly) else {
            self.edge_since = None;
            return Ok(());
        };
        let ControlState::Remote(id) = &t.new_state else {
            // Local -> Local cannot happen; ignore defensively.
            self.detector = snapshot;
            return Ok(());
        };
        // Never hand off to a device we are not connected to, and honour the
        // edge dwell delay.
        let mut allow = self.peers.contains_key(id);
        if allow && !self.opts.edge_dwell.is_zero() {
            let since = *self.edge_since.get_or_insert_with(Instant::now);
            allow = since.elapsed() >= self.opts.edge_dwell;
        }
        if !allow {
            self.detector = snapshot;
            if !self.peers.contains_key(id) {
                self.edge_since = None;
            }
            return Ok(());
        }
        self.edge_since = None;
        self.cursor.begin_capture()?;
        self.held_keys.clear();
        self.held_buttons.clear();
        self.carry = (0.0, 0.0);
        self.detector.set_hold(false);
        if let Some((x, y)) = t.remote_target {
            self.send(id, Message::MouseWarp { x, y });
        }
        self.tele.set_active(Some(id));
        tracing::info!(target = %id, "control handed off to remote");
        Ok(())
    }

    fn tick_remote(&mut self, mut peer_id: String) -> anyhow::Result<()> {
        for ev in self.cursor.poll_capture_events()? {
            match ev {
                CaptureEvent::Motion { dx, dy } => {
                    let s = self.opts.sensitivity.get(&peer_id).copied().unwrap_or(1.0);
                    let (sx, sy) = self.scale(dx, dy, s);
                    if (sx, sy) == (0, 0) {
                        continue;
                    }
                    match self.detector.on_remote_delta(sx, sy) {
                        None => self.send(&peer_id, Message::MouseMove { dx: sx, dy: sy }),
                        Some(Transition {
                            new_state: ControlState::Local,
                            local_target,
                            ..
                        }) => {
                            // This delta is the one that crossed back; the
                            // rest of the batch is relative to the stale
                            // capture centre and is dropped.
                            self.return_to_local(local_target, &peer_id, false)?;
                            return Ok(());
                        }
                        Some(Transition {
                            new_state: ControlState::Remote(next),
                            remote_target,
                            ..
                        }) => {
                            tracing::info!(from = %peer_id, to = %next, "control moved to another device");
                            self.release_all(&peer_id);
                            self.carry = (0.0, 0.0);
                            if let Some((x, y)) = remote_target {
                                self.send(&next, Message::MouseWarp { x, y });
                            }
                            self.tele.set_active(Some(&next));
                            peer_id = next;
                        }
                    }
                }
                CaptureEvent::Key { keycode, pressed } => {
                    if self
                        .hotkey
                        .as_ref()
                        .is_some_and(|h| h.triggered(keycode, pressed, &self.held_keys))
                    {
                        tracing::info!("emergency-return hotkey pressed");
                        self.tele.notice(Notice::ReturnedByHotkey);
                        self.return_to_local(None, &peer_id, false)?;
                        return Ok(());
                    }
                    if pressed {
                        self.held_keys.insert(keycode);
                    } else {
                        self.held_keys.remove(&keycode);
                    }
                    self.send(&peer_id, Message::KeyEvent { keycode, pressed });
                }
                CaptureEvent::Button { button, pressed } => {
                    if pressed {
                        self.held_buttons.insert(button);
                    } else {
                        self.held_buttons.remove(&button);
                    }
                    self.detector
                        .set_hold(self.opts.hold_guard && !self.held_buttons.is_empty());
                    self.send(&peer_id, Message::MouseButton { button, pressed });
                }
                CaptureEvent::Scroll { dx, dy } => {
                    let sign = if self.opts.natural_scroll { -1 } else { 1 };
                    self.send(
                        &peer_id,
                        Message::Scroll {
                            dx: dx * sign,
                            dy: dy * sign,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// Applies pointer speed with fractional carry so slow speeds don't
    /// lose motion to rounding.
    fn scale(&mut self, dx: i32, dy: i32, s: f32) -> (i32, i32) {
        if (s - 1.0).abs() < f32::EPSILON {
            return (dx, dy);
        }
        self.carry.0 += dx as f32 * s;
        self.carry.1 += dy as f32 * s;
        let (ix, iy) = (self.carry.0.trunc(), self.carry.1.trunc());
        self.carry.0 -= ix;
        self.carry.1 -= iy;
        (ix as i32, iy as i32)
    }

    /// Releases everything held on `id` so nothing stays stuck down there.
    fn release_all(&mut self, id: &str) {
        for k in std::mem::take(&mut self.held_keys) {
            self.send(
                id,
                Message::KeyEvent {
                    keycode: k,
                    pressed: false,
                },
            );
        }
        for b in std::mem::take(&mut self.held_buttons) {
            self.send(
                id,
                Message::MouseButton {
                    button: b,
                    pressed: false,
                },
            );
        }
        self.detector.set_hold(false);
    }

    /// Ends capture and puts the local cursor at `target` (or, when `None`,
    /// at the detector's clamped virtual position).
    fn return_to_local(
        &mut self,
        target: Option<(i32, i32)>,
        from: &str,
        peer_gone: bool,
    ) -> anyhow::Result<()> {
        // Reaching this without a crossing point means control was pulled
        // back (hotkey / lost device): land mid-screen, not on an edge,
        // or the next tick would immediately hand off again.
        self.detector.force_local();
        let centre = {
            let local = self.detector.layout().local_device();
            local.center_local()
        };
        if !peer_gone {
            self.release_all(from);
        } else {
            self.held_keys.clear();
            self.held_buttons.clear();
        }
        self.detector.set_hold(false);
        self.cursor.end_capture()?;
        let (x, y) = target.unwrap_or(centre);
        self.cursor.warp_absolute(x, y)?;
        self.tele.set_active(None);
        tracing::info!("control returned to local");
        Ok(())
    }

    fn tick_housekeeping(&mut self) {
        let now = Instant::now();
        // Clipboard: forward local changes to every connected device.
        if self.clipboard.is_some() && now >= self.next_clipboard_poll {
            self.next_clipboard_poll = now + CLIPBOARD_POLL_INTERVAL;
            let cap = self.opts.clipboard_cap();
            match self.clipboard.as_mut().map(|c| c.poll()) {
                Some(Ok(Some(text))) if text.len() <= cap => {
                    tracing::info!(bytes = text.len(), "sending local clipboard change");
                    for p in self.peers.values() {
                        let _ = p.tx.send(Message::ClipboardText(text.clone()));
                    }
                }
                Some(Ok(Some(text))) => {
                    tracing::info!(bytes = text.len(), cap, "clipboard too large to sync")
                }
                Some(Err(e)) => tracing::warn!("clipboard poll failed: {e}"),
                _ => {}
            }
        }
        // Hot-plug of this machine's own monitors (only while not captured,
        // since capture pins the pointer to the screen centre).
        if now >= self.next_monitor_poll && self.detector.state() == &ControlState::Local {
            self.next_monitor_poll = now + MONITOR_POLL_INTERVAL;
            let mons = detect_monitors(&self.cursor);
            if mons != self.local_monitors {
                tracing::info!(count = mons.len(), "local monitor arrangement changed");
                self.detector
                    .update_monitors(&self.local_id.clone(), to_monitors(&mons));
                self.tele.set_local_monitors(mons.clone());
                self.local_monitors = mons;
            }
        }
    }
}
