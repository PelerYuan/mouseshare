//! The target role: accepts one authenticated controller at a time and
//! replays its input on the local X server.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use mouseshare_discovery::Announcement;
use mouseshare_net::{Connection, NetError, PairingCode, PeerInfo};
use mouseshare_protocol::Message;
use mouseshare_x11input::{Clipboard, LocalCursor};
use tokio::sync::mpsc::unbounded_channel;

use crate::telemetry::{Notice, Telemetry};
use crate::{
    detect_monitors, forward_incoming, AbortOnDrop, SessionOptions, CLIPBOARD_POLL_INTERVAL,
    DEAD_AFTER, HANDSHAKE_TIMEOUT, MONITOR_POLL_INTERVAL,
};

/// Failed handshakes tolerated per source address per window. A 50-bit
/// pairing code makes online guessing hopeless at this rate.
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
struct FailureLimiter {
    fails: HashMap<IpAddr, VecDeque<Instant>>,
}

impl FailureLimiter {
    fn prune(&mut self, ip: IpAddr) -> usize {
        let q = self.fails.entry(ip).or_default();
        while q.front().is_some_and(|t| t.elapsed() > FAILURE_WINDOW) {
            q.pop_front();
        }
        q.len()
    }

    fn blocked(&mut self, ip: IpAddr) -> bool {
        self.prune(ip) >= MAX_FAILURES
    }

    fn record_failure(&mut self, ip: IpAddr) {
        self.prune(ip);
        self.fails.entry(ip).or_default().push_back(Instant::now());
    }

    fn clear(&mut self, ip: IpAddr) {
        self.fails.remove(&ip);
    }
}

/// Runs the target role until the future is dropped (or binding fails).
///
/// `device_id` is the name this device announces and answers to. Only a
/// controller that proves knowledge of `code` is served.
pub async fn run_target(
    device_id: String,
    listen_addr: SocketAddr,
    code: PairingCode,
    opts: SessionOptions,
    tele: Telemetry,
) -> anyhow::Result<()> {
    let cursor = LocalCursor::connect(None)?;
    let listener = mouseshare_net::listen(listen_addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!(addr = %bound_addr, "target listening");

    // Kept alive for the lifetime of this function: dropping it would send
    // an mDNS goodbye and stop controllers from finding us.
    let _announcement = match Announcement::start(&device_id, bound_addr.port()) {
        Ok(a) => Some(a),
        Err(e) => {
            tracing::warn!(
                "mDNS announcement failed ({e}); target is still reachable by address, \
                 just not auto-discoverable"
            );
            None
        }
    };

    tele.set_running(true);
    let mut limiter = FailureLimiter::default();
    let result = async {
        loop {
            let mut conn = listener.accept().await?;
            let ip = conn.peer_addr().map(|a| a.ip()).ok();
            if let Some(ip) = ip {
                if limiter.blocked(ip) {
                    tracing::warn!(%ip, "too many failed pairing attempts; refusing for a while");
                    tele.notice(Notice::IncomingRejected(ip.to_string()));
                    continue;
                }
            }
            let info = PeerInfo {
                device_id: device_id.clone(),
                monitors: detect_monitors(&cursor),
            };
            let handshake =
                tokio::time::timeout(HANDSHAKE_TIMEOUT, conn.handshake_as_listener(&code, &info))
                    .await;
            let peer = match handshake {
                Ok(Ok(peer)) => {
                    if let Some(ip) = ip {
                        limiter.clear(ip);
                    }
                    peer
                }
                other => {
                    let why = match other {
                        Ok(Err(e)) => e.to_string(),
                        _ => "handshake timed out".to_string(),
                    };
                    tracing::warn!("incoming connection rejected: {why}");
                    if let Some(ip) = ip {
                        limiter.record_failure(ip);
                        tele.notice(Notice::IncomingRejected(ip.to_string()));
                    }
                    continue;
                }
            };
            tracing::info!(controller = %peer.device_id, "controller connected");

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

            tele.set_controller(Some(&peer.device_id));
            tele.notice(Notice::ControllerConnected(peer.device_id.clone()));
            let mut held = Held::default();
            let res = serve(conn, &cursor, clipboard, &opts, &mut held).await;
            held.release_all(&cursor);
            tele.set_controller(None);
            tele.notice(Notice::ControllerLeft(peer.device_id.clone()));
            if let Err(e) = res {
                tracing::warn!("controller connection ended: {e}");
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    }
    .await;
    tele.set_running(false);
    result
}

/// Keys/buttons we are holding down on the controller's behalf, so a lost
/// connection can never leave something stuck.
#[derive(Default)]
struct Held {
    keys: HashSet<u8>,
    buttons: HashSet<u8>,
}

impl Held {
    fn release_all(&mut self, cursor: &LocalCursor) {
        for k in self.keys.drain() {
            let _ = cursor.inject_key(k, false);
        }
        for b in self.buttons.drain() {
            let _ = cursor.inject_button(b, false);
        }
    }
}

async fn serve(
    conn: Connection,
    cursor: &LocalCursor,
    mut clipboard: Option<Clipboard>,
    opts: &SessionOptions,
    held: &mut Held,
) -> Result<(), NetError> {
    let (reader, mut writer) = conn.into_split();
    let (tx, mut rx) = unbounded_channel();
    let _reader_guard = AbortOnDrop(tokio::spawn(forward_incoming(reader, tx)));

    let mut clipboard_tick = tokio::time::interval(CLIPBOARD_POLL_INTERVAL);
    clipboard_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut watch_tick = tokio::time::interval(MONITOR_POLL_INTERVAL);
    watch_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_rx = Instant::now();
    let mut monitors = detect_monitors(cursor);
    let cap = opts.clipboard_cap();

    loop {
        tokio::select! {
            msg = rx.recv() => {
                last_rx = Instant::now();
                match msg {
                    Some(Ok(Message::MouseMove { dx, dy })) => {
                        if let Err(e) = cursor.warp_relative(dx, dy) {
                            tracing::warn!("warp_relative failed: {e}");
                        }
                    }
                    Some(Ok(Message::MouseWarp { x, y })) => {
                        if let Err(e) = cursor.warp_absolute(x, y) {
                            tracing::warn!("warp_absolute failed: {e}");
                        }
                    }
                    Some(Ok(Message::MouseButton { button, pressed })) => {
                        if pressed { held.buttons.insert(button); } else { held.buttons.remove(&button); }
                        if let Err(e) = cursor.inject_button(button, pressed) {
                            tracing::warn!("inject_button failed: {e}");
                        }
                    }
                    Some(Ok(Message::Scroll { dx, dy })) => {
                        if let Err(e) = cursor.inject_scroll(dx, dy) {
                            tracing::warn!("inject_scroll failed: {e}");
                        }
                    }
                    Some(Ok(Message::KeyEvent { keycode, pressed })) => {
                        if pressed { held.keys.insert(keycode); } else { held.keys.remove(&keycode); }
                        if let Err(e) = cursor.inject_key(keycode, pressed) {
                            tracing::warn!("inject_key failed: {e}");
                        }
                    }
                    Some(Ok(Message::ClipboardText(text))) => {
                        if let Some(cb) = clipboard.as_mut() {
                            // Logged by length, not content: clipboard text
                            // can be sensitive.
                            let bytes = text.len();
                            if bytes > cap {
                                tracing::info!(bytes, cap, "ignoring oversized clipboard update");
                            } else {
                                match cb.set_text(text) {
                                    Ok(()) => tracing::info!(bytes, "clipboard updated from controller"),
                                    Err(e) => tracing::warn!("failed to apply clipboard update: {e}"),
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(t))) => writer.send(&Message::Pong(t)).await?,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(other)) => tracing::debug!(?other, "ignored message from controller"),
                    Some(Err(NetError::ConnectionClosed)) => return Ok(()),
                    Some(Err(e)) => return Err(e),
                    // The reader task always sends a terminal Err before
                    // ending, so a closed channel means it was aborted.
                    None => return Ok(()),
                }
            }
            _ = clipboard_tick.tick() => {
                if let Some(cb) = clipboard.as_mut() {
                    match cb.poll() {
                        Ok(Some(text)) if text.len() <= cap => {
                            tracing::info!(bytes = text.len(), "sending local clipboard change to controller");
                            writer.send(&Message::ClipboardText(text)).await?;
                        }
                        Ok(Some(text)) => tracing::info!(bytes = text.len(), cap, "clipboard too large to sync"),
                        Ok(None) => {}
                        Err(e) => tracing::warn!("clipboard poll failed: {e}"),
                    }
                }
            }
            _ = watch_tick.tick() => {
                if last_rx.elapsed() > DEAD_AFTER {
                    return Err(NetError::Io(std::io::ErrorKind::TimedOut.into()));
                }
                let now = detect_monitors(cursor);
                if now != monitors {
                    tracing::info!(count = now.len(), "monitor arrangement changed");
                    writer.send(&Message::MonitorsChanged(now.clone())).await?;
                    monitors = now;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_blocks_after_repeated_failures_per_ip() {
        let mut l = FailureLimiter::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for _ in 0..MAX_FAILURES {
            assert!(!l.blocked(a));
            l.record_failure(a);
        }
        assert!(l.blocked(a));
        assert!(!l.blocked(b));
        l.clear(a);
        assert!(!l.blocked(a));
    }
}
