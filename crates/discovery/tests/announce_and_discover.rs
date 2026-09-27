//! Integration test exercising real mDNS traffic: this process announces
//! itself as a mouseshare target and then browses for it, over actual UDP
//! multicast (224.0.0.251:5353) — not mocked in any way.
//!
//! Whether this passes depends on the environment actually permitting IP
//! multicast on some interface (including loopback). See the crate's
//! report / commit message for what was verified in the sandboxed dev VM
//! this crate was originally built in.

use std::net::TcpListener;
use std::time::Duration;

use mouseshare_discovery::{discover, Announcement, DiscoveryError};

/// Starts an announcement for a fake screen_id, browses for it, and checks
/// it comes back with the right screen_id and port.
///
/// Uses a screen_id unique to this test run (via the process id) so that
/// running this test binary concurrently with itself, or alongside other
/// mDNS chatter on the same LAN/host, can't cause a stale or foreign
/// responder to be mistaken for this test's own announcement.
#[test]
fn announced_peer_is_discovered() {
    let _ = tracing_subscriber::fmt::try_init();

    let screen_id = format!("discovery-test-{}", std::process::id());
    let port = 17_878;

    let _announcement =
        Announcement::start(&screen_id, port).expect("Announcement::start should succeed");

    let peers = discover(Duration::from_secs(5)).expect("discover() should succeed");

    let found = peers.iter().find(|p| p.screen_id == screen_id);
    assert!(
        found.is_some(),
        "expected to discover screen_id={screen_id:?} among peers, got: {peers:?}"
    );
    assert_eq!(found.unwrap().addr.port(), port);
}

/// The address `discover()` hands back must be directly dialable, not just
/// well-formed. Regression test for a real bug: mDNS resolves an instance's
/// addresses incrementally, and an early `ServiceResolved` event can carry
/// only an IPv6 link-local address, which mdns-sd's `to_ip_addr()` returns
/// without its zone/scope id -- connecting to that produces a bare EINVAL
/// from the kernel. `discover()` must wait for (or otherwise only report) a
/// real, connectable address.
///
/// Binds an actual listener on the announced port and connects to whatever
/// `discover()` returns, so this fails the exact way a real caller's
/// `mouseshare_net::connect()` would if the bug were still present.
#[test]
fn discovered_peer_address_is_actually_dialable() {
    let _ = tracing_subscriber::fmt::try_init();

    let screen_id = format!("discovery-test-dial-{}", std::process::id());
    let listener = TcpListener::bind("0.0.0.0:0").expect("bind ephemeral listener");
    let port = listener.local_addr().unwrap().port();

    let _announcement = Announcement::start(&screen_id, port).expect("start");
    let peers = discover(Duration::from_secs(5)).expect("discover() should succeed");
    let found = peers
        .iter()
        .find(|p| p.screen_id == screen_id)
        .unwrap_or_else(|| panic!("expected to discover screen_id={screen_id:?}, got: {peers:?}"));

    std::net::TcpStream::connect(found.addr).unwrap_or_else(|e| {
        panic!("discovered address {} should be directly dialable, but connect failed: {e}", found.addr)
    });
}

/// Addresses for one announced instance resolve incrementally (see the
/// dialable-address test above), which previously caused `discover()` to
/// accumulate a separate `DiscoveredPeer` per partial resolution rather than
/// converging on one. A caller that just takes the first match for a
/// screen_id (as `mouseshare`'s controller does) needs that first match to
/// always be a good one, not whichever partial resolution happened to
/// arrive first.
#[test]
fn discover_returns_at_most_one_entry_per_screen_id() {
    let _ = tracing_subscriber::fmt::try_init();

    let screen_id = format!("discovery-test-dedupe-{}", std::process::id());
    let _announcement = Announcement::start(&screen_id, 17_880).expect("start");

    let peers = discover(Duration::from_secs(5)).expect("discover() should succeed");
    let count = peers.iter().filter(|p| p.screen_id == screen_id).count();
    assert_eq!(
        count, 1,
        "discover() should dedupe to a single entry per instance, got: {peers:?}"
    );
}

/// `Announcement::start` should reject an empty screen_id up front, without
/// touching the network at all. Doesn't depend on multicast working.
#[test]
fn start_rejects_empty_screen_id() {
    let result = Announcement::start("", 12345);
    assert!(matches!(result, Err(DiscoveryError::EmptyScreenId)));
}

/// `discover` with nothing announced should return promptly with an empty
/// (or at least, not-containing-nonsense) result rather than erroring or
/// hanging past its timeout. This doesn't prove multicast works (silence is
/// silence whether or not multicast is functional) but it does prove the
/// timeout/shutdown bookkeeping doesn't hang or panic.
#[test]
fn discover_respects_timeout_when_nothing_is_announced() {
    let screen_id = format!("nonexistent-{}", std::process::id());
    let start = std::time::Instant::now();
    let peers = discover(Duration::from_millis(500)).expect("discover() should succeed");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "discover() should not run substantially longer than its timeout"
    );
    assert!(!peers.iter().any(|p| p.screen_id == screen_id));
}
