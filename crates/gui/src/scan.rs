//! LAN scanning (mDNS) in a background thread, plus a best-effort lookup of
//! this machine's LAN address for display.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mouseshare_discovery::DiscoveredPeer;

#[derive(Clone, Default)]
pub struct Scanner {
    peers: Arc<Mutex<Vec<DiscoveredPeer>>>,
    scanning: Arc<AtomicBool>,
}

impl Scanner {
    pub fn is_scanning(&self) -> bool {
        self.scanning.load(Ordering::Relaxed)
    }

    pub fn peers(&self) -> Vec<DiscoveredPeer> {
        self.peers.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// Starts a scan unless one is already running.
    pub fn scan(&self) {
        if self.scanning.swap(true, Ordering::Relaxed) {
            return;
        }
        let peers = self.peers.clone();
        let flag = self.scanning.clone();
        std::thread::spawn(move || {
            if let Ok(found) = mouseshare_discovery::discover(Duration::from_secs(3)) {
                if let Ok(mut p) = peers.lock() {
                    *p = found;
                }
            }
            flag.store(false, Ordering::Relaxed);
        });
    }
}

/// The IPv4 address other machines on the LAN would use to reach us. Uses a
/// UDP "connect" (which sends nothing) just to learn the routing choice.
pub fn lan_ip() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}
