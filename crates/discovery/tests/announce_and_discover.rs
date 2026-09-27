//! Integration test exercising real mDNS traffic: this process announces
//! itself as a mouseshare target and then browses for it, over actual UDP
//! multicast (224.0.0.251:5353) — not mocked in any way.
//!
//! Whether this passes depends on the environment actually permitting IP
//! multicast on some interface (including loopback). See the crate's
//! report / commit message for what was verified in the sandboxed dev VM
//! this crate was originally built in.

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
