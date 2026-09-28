//! LAN auto-discovery for mouseshare targets, via mDNS / DNS-SD
//! ([RFC 6762](https://www.rfc-editor.org/rfc/rfc6762)/[RFC 6763](https://www.rfc-editor.org/rfc/rfc6763)),
//! built on the [`mdns-sd`](https://docs.rs/mdns-sd) crate.
//!
//! Two responsibilities:
//!
//! - [`Announcement::start`] runs on the **target** machine (the one being
//!   connected to) and registers an mDNS service so controllers on the LAN
//!   can find it, advertising the target's `screen_id` and protocol version
//!   in a TXT record.
//! - [`discover`] runs on the **controller** machine and browses the LAN for
//!   a bounded amount of time, returning whatever mouseshare targets
//!   answered.
//!
//! Both functions are blocking (`mdns-sd` has no async runtime dependency
//! and runs its own background OS thread internally). See the doc comments
//! on each for guidance about calling them from an async context.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use mouseshare_protocol::PROTOCOL_VERSION;

/// The mDNS/DNS-SD service type mouseshare targets are announced under.
pub const SERVICE_TYPE: &str = "_mouseshare._tcp.local.";

/// TXT record key carrying the announcing screen's id (see
/// `mouseshare_layout::LayoutConfig::local_id`).
pub const TXT_KEY_SCREEN_ID: &str = "screen_id";

/// TXT record key carrying the announcing peer's
/// `mouseshare_protocol::PROTOCOL_VERSION`, as a base-10 string. Not
/// currently enforced by this crate (a mismatch doesn't stop a peer from
/// being discovered) — it's here so a controller can filter out or warn
/// about incompatible targets before ever dialing them, and so future
/// versions have a compatibility signal to check without a wire round-trip.
pub const TXT_KEY_PROTOCOL_VERSION: &str = "protocol_version";

/// Errors that can occur announcing or discovering mouseshare services over
/// mDNS.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DiscoveryError {
    /// An error surfaced by the underlying `mdns-sd` daemon: a malformed
    /// service (bad hostname/service-name, invalid TXT record), the
    /// daemon's internal command queue was full (`Error::Again`), the
    /// daemon thread had already exited (`Error::DaemonShutdown`), or an IP
    /// address failed to parse. See [`mdns_sd::Error`] for the full set.
    #[error("mdns-sd daemon error: {0}")]
    Mdns(#[from] mdns_sd::Error),

    /// [`Announcement::start`] was called with an empty `screen_id`. A
    /// real screen id (see `mouseshare_layout::LayoutConfig::local_id`) is
    /// required both for the TXT record to be meaningful and because
    /// mdns-sd derives the announced hostname from it.
    #[error("screen_id must not be empty")]
    EmptyScreenId,
}

/// Handle for an active mDNS announcement of this machine as a mouseshare
/// target.
///
/// Keeps the `mdns-sd` daemon thread and the service registration alive.
/// Dropping it sends an mDNS "goodbye" (unregister) packet on a best-effort
/// basis and shuts down the daemon thread. There is nothing to call to tear
/// it down explicitly — just let it go out of scope (or `drop()` it) when
/// the target should stop being discoverable.
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Announcement {
    /// Starts announcing this machine as a mouseshare target reachable at
    /// `port`, with `screen_id` (and the crate's `PROTOCOL_VERSION`) in the
    /// TXT record.
    ///
    /// The mDNS instance name is `screen_id` itself, so it should be
    /// human-meaningful and (ideally) unique on the LAN; see the crate-level
    /// docs / integrator notes about what happens when it isn't.
    ///
    /// Registration with the mDNS daemon is asynchronous internally (a
    /// probe for name conflicts happens before the service is actually
    /// announced on the wire), but this function itself returns as soon as
    /// the registration request has been *enqueued*, typically within
    /// microseconds — it does not block for the probe/announce to finish.
    /// It is safe to call directly from an async context (e.g. inside a
    /// `tokio::main`) without `spawn_blocking`.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryError::EmptyScreenId`] if `screen_id` is empty.
    /// Returns [`DiscoveryError::Mdns`] if the daemon fails to start, or if
    /// `screen_id`/`port` somehow produce a malformed service (shouldn't
    /// happen for any non-empty `screen_id`, since the derived hostname
    /// only needs to end in `.local.`, which we control).
    pub fn start(screen_id: &str, port: u16) -> Result<Self, DiscoveryError> {
        if screen_id.is_empty() {
            return Err(DiscoveryError::EmptyScreenId);
        }

        let daemon = ServiceDaemon::new()?;

        // Any string ending in ".local." is a valid mdns-sd hostname; it
        // doesn't need to match the machine's real OS hostname; it's just
        // the DNS name the A/AAAA (address) records get filled in under.
        // Deriving it from screen_id keeps things human-readable and
        // avoids a dependency on a hostname-lookup crate.
        let host_name = format!("{screen_id}.local.");
        let protocol_version = PROTOCOL_VERSION.to_string();
        let properties = [
            (TXT_KEY_SCREEN_ID, screen_id),
            (TXT_KEY_PROTOCOL_VERSION, protocol_version.as_str()),
        ];

        // Empty ip + enable_addr_auto(): let mdns-sd fill in addresses from
        // the host's own interfaces (and keep them updated if interfaces
        // change), rather than us trying to enumerate/pick one ourselves.
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            screen_id,
            &host_name,
            "",
            port,
            &properties[..],
        )?
        .enable_addr_auto();

        let fullname = service_info.get_fullname().to_string();
        daemon.register(service_info)?;

        Ok(Self { daemon, fullname })
    }
}

impl Drop for Announcement {
    fn drop(&mut self) {
        // Best-effort unregister (sends an mDNS goodbye packet) and daemon
        // shutdown. We don't propagate errors from Drop; if the daemon
        // already shut itself down there's nothing more to do.
        if let Ok(recv) = self.daemon.unregister(&self.fullname) {
            // Give the goodbye packet a brief window to actually go out
            // before we shut the daemon thread down. Not a long wait: this
            // is best-effort cleanup, not something callers should block
            // noticeably on when dropping.
            let _ = recv.recv_timeout(Duration::from_millis(500));
        }
        let _ = self.daemon.shutdown();
    }
}

/// One mouseshare target discovered on the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPeer {
    /// The `screen_id` the peer announced in its TXT record.
    pub screen_id: String,
    /// The address (IP + port) the peer is reachable at.
    pub addr: SocketAddr,
}

/// Browses the LAN for `_mouseshare._tcp.local.` services for up to
/// `timeout`, then returns whatever fully resolved (address + TXT record)
/// peers answered in that window.
///
/// This is a one-shot "collect what answers within `timeout`" call, not a
/// long-lived stream: it starts its own `ServiceDaemon`, browses, waits out
/// the full `timeout` (there's no way to know a LAN has finished answering
/// early — silence isn't a signal in mDNS), and shuts the daemon down again
/// before returning. That's a deliberate simplification for this MVP: a
/// GUI/long-running controller that wants live updates as targets come and
/// go would instead want to keep a `ServiceDaemon` and its `browse()`
/// receiver alive and consume `ServiceEvent`s as a stream — `mdns-sd`
/// supports that shape too — but nothing in this codebase needs that yet.
///
/// A service that's found (`ServiceEvent::ServiceFound`) but never fully
/// resolves (`ServiceEvent::ServiceResolved`) within `timeout`, or that
/// resolves but is missing the `screen_id` TXT property or has no usable
/// address, is silently skipped (logged at `warn` via `tracing`) rather
/// than failing the whole call — one malformed or foreign responder on the
/// LAN shouldn't prevent discovering everything else.
///
/// This function **blocks the calling thread for up to `timeout`**. Do not
/// call it directly from an async task on a tokio runtime — it will stall
/// that worker thread for the whole duration. Wrap it in
/// `tokio::task::spawn_blocking` from an async context.
///
/// # Errors
///
/// Returns [`DiscoveryError::Mdns`] if the daemon fails to start or the
/// browse request can't be enqueued. A `timeout` with no responders is not
/// an error — it returns `Ok(vec![])`.
pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredPeer>, DiscoveryError> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE_TYPE)?;

    let deadline = Instant::now() + timeout;
    // Keyed by the service's mDNS fullname (not screen_id, though in
    // practice they're the same string): addresses for one instance
    // resolve incrementally over several `ServiceResolved` events (e.g.
    // IPv6 link-local first, then loopback, then the real LAN IPv4
    // address, each as its own event with its own address snapshot), so a
    // later event for the same instance should replace an earlier one
    // rather than accumulate as a separate, possibly-stale duplicate.
    let mut peers: std::collections::HashMap<String, DiscoveredPeer> =
        std::collections::HashMap::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => match peer_from_resolved(&info) {
                Some(peer) => {
                    peers.insert(info.fullname.clone(), peer);
                }
                None => {
                    // Normal and common, not a problem: this instance just
                    // hasn't resolved a usable address yet on this event
                    // (e.g. only its IPv6 link-local address has arrived so
                    // far). A later event for the same fullname will
                    // typically supersede it before `timeout` elapses.
                    tracing::debug!(
                        fullname = %info.fullname,
                        "resolved mouseshare service has no usable address yet; waiting for a further update"
                    );
                }
            },
            Ok(_) => continue,
            // Either the timeout elapsed or the channel disconnected
            // (daemon died) — either way, we're done waiting.
            Err(_) => break,
        }
    }

    // Best-effort: stop browsing and shut the per-call daemon thread down
    // rather than leaking it now that we have our answer.
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();

    Ok(peers.into_values().collect())
}

fn peer_from_resolved(info: &ResolvedService) -> Option<DiscoveredPeer> {
    let screen_id = info
        .txt_properties
        .get_property_val_str(TXT_KEY_SCREEN_ID)?
        .to_string();

    // IPv4 only: mdns-sd's resolved address set loses the zone/scope id an
    // IPv6 link-local address needs to be dialable (`to_ip_addr()` below
    // returns a bare `Ipv6Addr`), so handing one back as a plain
    // `SocketAddr` produces an address `TcpStream::connect` rejects with
    // EINVAL. This app has no need for IPv6 in the first place -- LAN mouse
    // sharing -- so the simplest correct fix is to only ever report an
    // IPv4 address, and treat "no IPv4 resolved for this instance yet" the
    // same as "not resolved yet" rather than handing back a broken one.
    let ip = info
        .addresses
        .iter()
        .find(|addr| addr.is_ipv4())?
        .to_ip_addr();

    Some(DiscoveredPeer {
        screen_id,
        addr: SocketAddr::new(ip, info.port),
    })
}
