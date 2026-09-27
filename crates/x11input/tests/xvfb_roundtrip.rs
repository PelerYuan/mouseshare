//! End-to-end test against two real (headless) Xvfb servers.
//!
//! Display :97 is used to exercise the "capture" role (grab + recenter +
//! delta accumulation) by driving the pointer with `xdotool mousemove`.
//! Display :98 is used to exercise the "inject" role (`warp_relative` /
//! `warp_absolute`), cross-checked against `xdotool getmouselocation`.
//!
//! Both Xvfb children are always killed on the way out, including on panic,
//! via the `Drop` impl on `XvfbGuard`.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mouseshare_x11input::LocalCursor;

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

    let mut cursor =
        LocalCursor::connect(Some(&guard.display)).expect("connect to capture display");

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

#[test]
fn inject_role_warps_relative_and_absolute() {
    let guard = XvfbGuard::spawn(98, 800, 600);

    let cursor = LocalCursor::connect(Some(&guard.display)).expect("connect to inject display");

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
