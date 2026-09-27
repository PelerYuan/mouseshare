//! Full end-to-end test: two real (headless) Xvfb "computers" running the
//! actual `mouseshare` binary in `controller`/`target` roles, connected over
//! real localhost TCP, driven by `xdotool` exactly like a human would move a
//! physical mouse. This is the scenario the whole MVP exists to prove out:
//! edge detection -> capture -> network forwarding -> injection -> handoff
//! back to local, all working together, not just each piece in isolation.

use std::io::Write as _;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
        let deadline = Instant::now() + Duration::from_secs(10);
        while !Path::new(&socket).exists() {
            assert!(Instant::now() < deadline, "Xvfb did not create {socket} in time");
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(200));

        Self { display, child }
    }
}

impl Drop for XvfbGuard {
    fn drop(&mut self) {
        terminate_gracefully(&mut self.child);
    }
}

/// Kills a child process, running it (target/controller) for the duration of
/// one test.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        terminate_gracefully(&mut self.0);
    }
}

fn terminate_gracefully(child: &mut Child) {
    let pid = child.id().to_string();
    let _ = Command::new("kill")
        .args(["-TERM", &pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_millis(1000);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .unwrap()
        .port()
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
    xdotool(display, &["mousemove", "--sync", &x.to_string(), &y.to_string()]);
}

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
    (x.expect("no X= in getmouselocation output"), y.expect("no Y="))
}

/// Writes a two-screen layout (A at x=0..800, B at x=800..1600, both
/// 800x600) with the given `local_id`, returning the config file's path.
fn write_layout(dir: &Path, filename: &str, local_id: &str) -> PathBuf {
    let path = dir.join(filename);
    let mut f = std::fs::File::create(&path).expect("create layout file");
    write!(
        f,
        r#"
local_id = "{local_id}"

[[screens]]
id = "A"
x = 0
y = 0
width = 800
height = 600

[[screens]]
id = "B"
x = 800
y = 0
width = 800
height = 600
"#
    )
    .unwrap();
    path
}

#[test]
fn mouse_handoff_round_trip_between_two_real_processes() {
    let display_a = XvfbGuard::spawn(195, 800, 600);
    let display_b = XvfbGuard::spawn(196, 800, 600);

    let tmp = std::env::temp_dir().join(format!("mouseshare-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let layout_a = write_layout(&tmp, "a.toml", "A");
    let layout_b = write_layout(&tmp, "b.toml", "B");

    let port = free_port();
    let bin = env!("CARGO_BIN_EXE_mouseshare");

    let target = ChildGuard(
        Command::new(bin)
            .args([
                "--config",
                layout_b.to_str().unwrap(),
                "target",
                "--listen",
                &format!("127.0.0.1:{port}"),
            ])
            .env("DISPLAY", &display_b.display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn target"),
    );
    std::thread::sleep(Duration::from_millis(500));

    let controller = ChildGuard(
        Command::new(bin)
            .args([
                "--config",
                layout_a.to_str().unwrap(),
                "controller",
                "--connect",
                &format!("127.0.0.1:{port}"),
            ])
            .env("DISPLAY", &display_a.display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn controller"),
    );
    std::thread::sleep(Duration::from_millis(800));

    // 1. Drive A's cursor to its physical right edge (799 = width-1, the
    //    most an X server will ever actually report). This should hand
    //    control off to B.
    xdotool_mousemove(&display_a.display, 799, 300);
    std::thread::sleep(Duration::from_millis(300));

    let (bx0, by0) = xdotool_mouselocation(&display_b.display);

    // 2. While captured, A's own pointer should be pinned at the capture
    //    center (400, 300) — it must never visibly leave the local screen.
    let (ax, ay) = xdotool_mouselocation(&display_a.display);
    assert_eq!((ax, ay), (400, 300), "local cursor should be pinned at the capture center");

    // 3. Move +50,0 from the (recentered) capture point; that delta should
    //    be forwarded over the network and injected into B.
    xdotool_mousemove(&display_a.display, 450, 300);
    std::thread::sleep(Duration::from_millis(300));
    let (bx1, by1) = xdotool_mouselocation(&display_b.display);
    assert_eq!((bx1 - bx0, by1 - by0), (50, 0), "B's cursor should have moved by the forwarded delta");

    // 4. A large leftward move should cross back over the (margin-guarded)
    //    boundary and hand control back to local, snapping A's real cursor
    //    to the exact virtual crossing point: it entered B at virtual x=800
    //    (B's left edge) then moved +50 then -100, landing at virtual
    //    x=750, i.e. local coordinate (750, 300) on A.
    xdotool_mousemove(&display_a.display, 300, 300);
    std::thread::sleep(Duration::from_millis(300));
    let (ax2, ay2) = xdotool_mouselocation(&display_a.display);
    assert_eq!((ax2, ay2), (750, 300), "control should return to local at the exact crossing point");

    // 5. B should not have moved any further once control returned.
    let (bx2, by2) = xdotool_mouselocation(&display_b.display);
    assert_eq!((bx2, by2), (bx1, by1), "B's cursor should be untouched after control returns to local");

    drop(controller);
    drop(target);
}
