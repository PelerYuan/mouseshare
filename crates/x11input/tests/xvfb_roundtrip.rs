//! End-to-end test against several real (headless) Xvfb servers.
//!
//! Display :97 is used to exercise the pointer side of the "capture" role
//! (grab + recenter + delta accumulation) by driving the pointer with
//! `xdotool mousemove`. Display :98 is used to exercise the "inject" role
//! (`warp_relative` / `warp_absolute`), cross-checked against `xdotool
//! getmouselocation`. Display :99 is a regression test for batched-motion
//! delta accounting. Displays :94-:96 exercise the keyboard additions:
//! capture (`poll_capture_keys`), the pointer/keyboard poll interleaving
//! guarantee, and key injection (`inject_key`, checked via the core
//! `QueryKeymap` request).
//!
//! Each Xvfb child is always killed on the way out, including on panic, via
//! the `Drop` impl on `XvfbGuard`.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mouseshare_x11input::LocalCursor;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

/// Owns a running `Xvfb` child process and guarantees it is killed (and
/// reaped) when dropped, even if the test panics partway through.
struct XvfbGuard {
    display: String,
    child: Child,
}

impl XvfbGuard {
    fn spawn(display_num: u32, width: u32, height: u32) -> Self {
        let display = format!(":{display_num}");
        let child = Command::new("Xvfb")
            .arg(&display)
            .args(["-screen", "0", &format!("{width}x{height}x24")])
            .arg("-nolisten")
            .arg("tcp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn Xvfb - is it installed?");

        let socket = format!("/tmp/.X11-unix/X{display_num}");
        wait_for_path(Path::new(&socket), Duration::from_secs(10))
            .unwrap_or_else(|| panic!("Xvfb did not create socket {socket} in time"));

        // Xvfb creating the socket file doesn't guarantee it has finished
        // accepting connections on it yet; give it a brief moment.
        std::thread::sleep(Duration::from_millis(200));

        Self { display, child }
    }
}

impl Drop for XvfbGuard {
    fn drop(&mut self) {
        // Prefer a graceful SIGTERM so Xvfb removes its own lock file and
        // /tmp/.X11-unix socket; fall back to SIGKILL if it doesn't exit
        // promptly, so we never leave an orphan process behind either way.
        let pid = self.child.id().to_string();
        let _ = Command::new("kill")
            .args(["-TERM", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let deadline = Instant::now() + Duration::from_millis(1000);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_path(path: &Path, timeout: Duration) -> Option<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return Some(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Connects to `display`, retrying briefly on failure.
///
/// `XvfbGuard::spawn` already waits for the display's Unix socket file to
/// exist before returning, plus a fixed grace sleep -- but that's still a
/// best-effort heuristic, not a guarantee that Xvfb has finished its startup
/// far enough to *accept* connections: the socket file can be created
/// slightly before the server is actually listening on it, and a connect
/// attempt in that narrow window fails with ECONNRESET/ECONNREFUSED. This
/// shows up as a genuine (if rare) flake under system load rather than any
/// bug in `LocalCursor` itself, so the fix belongs here at the test level:
/// retry the connection itself with a bounded deadline, instead of guessing
/// a longer fixed sleep that would either not be long enough under worse
/// load or waste time in the common case.
fn connect_retrying(display: &str) -> LocalCursor {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match LocalCursor::connect(Some(display)) {
            Ok(cursor) => return cursor,
            Err(err) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
                let _ = err;
            }
            Err(err) => panic!("failed to connect to {display} after retrying for 5s: {err}"),
        }
    }
}

fn xdotool(display: &str, args: &[&str]) -> String {
    let output = Command::new("xdotool")
        .args(args)
        .env("DISPLAY", display)
        .output()
        .expect("failed to run xdotool - is it installed?");
    assert!(
        output.status.success(),
        "xdotool {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn xdotool_mousemove(display: &str, x: i32, y: i32) {
    xdotool(
        display,
        &[
            "mousemove",
            "--sync",
            &x.to_string(),
            &y.to_string(),
        ],
    );
}

/// Parses `xdotool getmouselocation --shell` output (`X=1\nY=2\n...`) into
/// (x, y).
fn xdotool_mouselocation(display: &str) -> (i32, i32) {
    let out = xdotool(display, &["getmouselocation", "--shell"]);
    let mut x = None;
    let mut y = None;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("X=") {
            x = v.parse::<i32>().ok();
        } else if let Some(v) = line.strip_prefix("Y=") {
            y = v.parse::<i32>().ok();
        }
    }
    (
        x.expect("no X= in getmouselocation output"),
        y.expect("no Y= in getmouselocation output"),
    )
}

fn xdotool_keydown(display: &str, key: &str) {
    xdotool(display, &["keydown", "--clearmodifiers", key]);
}

fn xdotool_keyup(display: &str, key: &str) {
    xdotool(display, &["keyup", "--clearmodifiers", key]);
}

/// Runs `xmodmap -pke` against `display` and returns the X11 keycode bound
/// to the given unshifted keysym name (e.g. `"a"`). This is an
/// independent (non-`x11rb`, non-`LocalCursor`) way of learning which
/// keycode a key we ask `xdotool` to press should end up as, so capture
/// assertions can check the *exact* reported keycode rather than merely
/// "some nonzero value" -- without hardcoding a keycode number that isn't
/// guaranteed to hold on every X server's default keymap.
fn xmodmap_keycode_for(display: &str, keysym_name: &str) -> u8 {
    let output = Command::new("xmodmap")
        .arg("-pke")
        .env("DISPLAY", display)
        .output()
        .expect("failed to run xmodmap - is it installed?");
    assert!(
        output.status.success(),
        "xmodmap -pke failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        // Lines look like: "keycode  38 = a A a A"
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        if rhs.split_whitespace().next() == Some(keysym_name) {
            let keycode_str = lhs
                .trim()
                .strip_prefix("keycode")
                .expect("xmodmap -pke line should start with 'keycode'")
                .trim();
            return keycode_str
                .parse()
                .expect("xmodmap keycode column should be numeric");
        }
    }
    panic!("could not find a keycode bound to keysym {keysym_name:?} in `xmodmap -pke` output on {display}");
}

/// Polls `poll_capture_keys` in a short loop until at least one event has
/// arrived or `timeout` elapses.
fn wait_for_keys(cursor: &mut LocalCursor, timeout: Duration) -> Vec<(u8, bool)> {
    let start = Instant::now();
    loop {
        let batch = cursor.poll_capture_keys().expect("poll_capture_keys");
        if !batch.is_empty() || start.elapsed() > timeout {
            return batch;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls `poll_capture_delta` in a short loop, accumulating whatever deltas
/// show up, until either something has arrived and a subsequent poll comes
/// back empty (i.e. we've drained a settled burst), or `timeout` elapses.
fn accumulate_deltas(cursor: &mut LocalCursor, timeout: Duration) -> (i32, i32) {
    let start = Instant::now();
    let mut sum = (0i32, 0i32);
    let mut saw_any = false;
    loop {
        let batch = cursor.poll_capture_delta().expect("poll_capture_delta");
        if !batch.is_empty() {
            saw_any = true;
            for (dx, dy) in batch {
                sum.0 += dx;
                sum.1 += dy;
            }
        } else if saw_any {
            break;
        }
        if start.elapsed() > timeout {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    sum
}

#[test]
fn capture_role_reports_correct_deltas() {
    let guard = XvfbGuard::spawn(97, 800, 600);

    let mut cursor = connect_retrying(&guard.display);

    // Sanity check on a fresh Xvfb screen.
    assert_eq!(cursor.screen_size(), (800, 600));
    let (x0, y0) = cursor.query_pointer().expect("query_pointer");
    assert!((0..800).contains(&x0), "x0={x0} out of range");
    assert!((0..600).contains(&y0), "y0={y0} out of range");

    cursor.begin_capture().expect("begin_capture");

    // begin_capture() warped the pointer to the center of the screen.
    let (cx, cy) = cursor.query_pointer().expect("query_pointer after begin_capture");
    assert_eq!((cx, cy), (400, 300));

    // Rightward move: from center (400, 300) to (550, 300), i.e. dx=+150, dy=0.
    xdotool_mousemove(&guard.display, 550, 300);
    let sum_right = accumulate_deltas(&mut cursor, Duration::from_secs(2));
    assert!(
        (sum_right.0 - 150).abs() <= 5,
        "expected dx summing to ~150, got {sum_right:?}"
    );
    assert!(
        sum_right.1.abs() <= 5,
        "expected dy summing to ~0, got {sum_right:?}"
    );

    // The pointer should have been re-centered by poll_capture_delta.
    let (rx, ry) = cursor.query_pointer().expect("query_pointer after recenter");
    assert_eq!((rx, ry), (400, 300), "pointer was not recentered");

    // Downward move: from center (400, 300) to (400, 450), i.e. dx=0, dy=+150.
    xdotool_mousemove(&guard.display, 400, 450);
    let sum_down = accumulate_deltas(&mut cursor, Duration::from_secs(2));
    assert!(
        sum_down.0.abs() <= 5,
        "expected dx summing to ~0, got {sum_down:?}"
    );
    assert!(
        (sum_down.1 - 150).abs() <= 5,
        "expected dy summing to ~150, got {sum_down:?}"
    );

    cursor.end_capture().expect("end_capture");

    // After ungrab, xdotool should be able to move the pointer freely again
    // and it should stick (no more recentering happening).
    xdotool_mousemove(&guard.display, 10, 10);
    let (fx, fy) = cursor.query_pointer().expect("query_pointer after end_capture");
    assert_eq!((fx, fy), (10, 10));
}

/// Regression test for a bug where `poll_capture_delta` computed every
/// event in a drained batch as a delta from the same stale center point,
/// instead of from the previous event in the batch — which double-counts
/// earlier events whenever more than one `MotionNotify` lands before a
/// single poll call drains them (a realistic scenario: the poller doesn't
/// necessarily run between every individual OS-level mouse event).
#[test]
fn poll_capture_delta_does_not_double_count_a_batched_burst() {
    let guard = XvfbGuard::spawn(99, 800, 600);

    let mut cursor = connect_retrying(&guard.display);
    cursor.begin_capture().expect("begin_capture");
    assert_eq!(cursor.query_pointer().unwrap(), (400, 300));

    // Two moves back-to-back with NO poll_capture_delta call in between, so
    // (contingent on the server not coalescing them) both MotionNotify
    // events should be sitting in the connection's buffer together the
    // first time we drain it below.
    xdotool_mousemove(&guard.display, 450, 300); // +50 from center
    xdotool_mousemove(&guard.display, 500, 300); // +50 more (total +100 from center)

    // xdotool's --sync only guarantees the move itself completed, not that
    // our client has received the notification yet; give it a moment.
    std::thread::sleep(Duration::from_millis(200));

    let batch = cursor.poll_capture_delta().expect("poll_capture_delta");
    let sum: i32 = batch.iter().map(|(dx, _)| dx).sum();
    assert!(
        (sum - 100).abs() <= 5,
        "expected batched deltas to sum to ~100 (not double-counted), got {batch:?} (sum={sum})"
    );

    cursor.end_capture().expect("end_capture");
}

#[test]
fn inject_role_warps_relative_and_absolute() {
    let guard = XvfbGuard::spawn(98, 800, 600);

    let cursor = connect_retrying(&guard.display);

    assert_eq!(cursor.screen_size(), (800, 600));

    // Put the pointer at a known starting point via xdotool so this test
    // doesn't depend on Xvfb's default pointer position.
    xdotool_mousemove(&guard.display, 300, 200);
    let (before_x, before_y) = cursor.query_pointer().expect("query_pointer before warp");
    assert_eq!((before_x, before_y), (300, 200));

    // Relative warp.
    cursor.warp_relative(50, -30).expect("warp_relative");
    let (after_x, after_y) = cursor.query_pointer().expect("query_pointer after warp_relative");
    assert_eq!((after_x, after_y), (350, 170));
    // Cross-check with an independent tool.
    assert_eq!(xdotool_mouselocation(&guard.display), (350, 170));

    // Absolute warp.
    cursor.warp_absolute(600, 450).expect("warp_absolute");
    let (abs_x, abs_y) = cursor.query_pointer().expect("query_pointer after warp_absolute");
    assert_eq!((abs_x, abs_y), (600, 450));
    assert_eq!(xdotool_mouselocation(&guard.display), (600, 450));
}

/// Exercises the keyboard half of the "capture" role: `begin_capture` grabs
/// the keyboard (in addition to the pointer, which the other capture test
/// covers), and `poll_capture_keys` reports real `xdotool keydown`/`keyup`
/// presses as `(keycode, pressed)` pairs. The expected keycode is looked up
/// independently via `xmodmap`, not assumed, so this checks the *exact*
/// keycode, not just "some nonzero value".
#[test]
fn keyboard_capture_reports_press_and_release() {
    let guard = XvfbGuard::spawn(96, 800, 600);
    let expected_keycode = xmodmap_keycode_for(&guard.display, "a");

    let mut cursor = connect_retrying(&guard.display);
    cursor.begin_capture().expect("begin_capture");

    xdotool_keydown(&guard.display, "a");
    let press = wait_for_keys(&mut cursor, Duration::from_secs(2));
    assert_eq!(
        press,
        vec![(expected_keycode, true)],
        "expected a single press event for keycode {expected_keycode}"
    );

    xdotool_keyup(&guard.display, "a");
    let release = wait_for_keys(&mut cursor, Duration::from_secs(2));
    assert_eq!(
        release,
        vec![(expected_keycode, false)],
        "expected a single release event for keycode {expected_keycode}"
    );

    cursor.end_capture().expect("end_capture");

    // Symmetric with the pointer-side regression test: after ungrab, the
    // keyboard is free again. There's no easy behavioral probe for "is the
    // keyboard still grabbed" without another client, so this just confirms
    // end_capture() didn't error, which it would if the keyboard grab
    // established by begin_capture() had never actually taken hold.
}

/// Regression test for the interleaving hazard fixed by routing both
/// `poll_capture_delta` and `poll_capture_keys` through one shared drain of
/// the X connection's event queue: since `poll_for_event` is destructive,
/// two methods reading the connection independently would have one silently
/// steal (and drop) events meant for the other, depending on call order.
/// This drives both a pointer move and a key press before draining either,
/// polls the pointer side first, and confirms the key event is still there
/// afterwards.
#[test]
fn poll_capture_delta_and_poll_capture_keys_do_not_steal_each_others_events() {
    let guard = XvfbGuard::spawn(95, 800, 600);
    let expected_keycode = xmodmap_keycode_for(&guard.display, "b");

    let mut cursor = connect_retrying(&guard.display);
    cursor.begin_capture().expect("begin_capture");

    xdotool_mousemove(&guard.display, 500, 350);
    xdotool_keydown(&guard.display, "b");
    // Give the server a moment to deliver both events before draining.
    std::thread::sleep(Duration::from_millis(200));

    let deltas = cursor.poll_capture_delta().expect("poll_capture_delta");
    assert!(
        !deltas.is_empty(),
        "expected the mouse move to be reported as a delta"
    );

    let keys = wait_for_keys(&mut cursor, Duration::from_secs(2));
    assert_eq!(
        keys,
        vec![(expected_keycode, true)],
        "key event must not be dropped by the preceding poll_capture_delta call"
    );

    xdotool_keyup(&guard.display, "b");
    let release = wait_for_keys(&mut cursor, Duration::from_secs(2));
    assert_eq!(release, vec![(expected_keycode, false)]);

    cursor.end_capture().expect("end_capture");
}

/// Exercises key *injection* via `inject_key` (XTest `FakeInput`), verified
/// with the core `QueryKeymap` request on a second, independent connection
/// to the same display -- this sidesteps needing a window manager or a
/// focused text-input widget, neither of which Xvfb provides.
#[test]
fn inject_key_sets_and_clears_query_keymap_bit() {
    let guard = XvfbGuard::spawn(94, 800, 600);

    let cursor = connect_retrying(&guard.display);

    // A second connection used purely to observe server-side key state; it
    // doesn't go through LocalCursor because QueryKeymap is a test-only
    // verification tool here, not part of the crate's public API surface.
    let (keymap_conn, _) =
        x11rb::connect(Some(&guard.display)).expect("second connection for QueryKeymap");
    // The connection setup always reports a valid min_keycode (traditionally
    // 8; keycodes below that are reserved), so this is guaranteed in-range
    // for XTest's FakeInput without needing to look up any specific key.
    let keycode = keymap_conn.setup().min_keycode;

    let is_down = |code: u8| -> bool {
        let reply = keymap_conn
            .query_keymap()
            .expect("query_keymap request")
            .reply()
            .expect("query_keymap reply");
        let byte = reply.keys[(code / 8) as usize];
        (byte & (1 << (code % 8))) != 0
    };

    assert!(!is_down(keycode), "keycode {keycode} should start up");

    cursor.inject_key(keycode, true).expect("inject_key press");
    let mut seen_down = false;
    for _ in 0..50 {
        if is_down(keycode) {
            seen_down = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        seen_down,
        "keycode {keycode} should be down after inject_key(_, true)"
    );

    cursor.inject_key(keycode, false).expect("inject_key release");
    let mut seen_up = false;
    for _ in 0..50 {
        if !is_down(keycode) {
            seen_up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        seen_up,
        "keycode {keycode} should be up after inject_key(_, false)"
    );
}
