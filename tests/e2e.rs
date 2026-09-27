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

/// Writes a two-screen layout (`id_a` at x=0..800, `id_b` at x=800..1600,
/// both 800x600) with the given `local_id`, returning the config file's
/// path.
fn write_layout_with_ids(dir: &Path, filename: &str, local_id: &str, id_a: &str, id_b: &str) -> PathBuf {
    let path = dir.join(filename);
    let mut f = std::fs::File::create(&path).expect("create layout file");
    write!(
        f,
        r#"
local_id = "{local_id}"

[[screens]]
id = "{id_a}"
x = 0
y = 0
width = 800
height = 600

[[screens]]
id = "{id_b}"
x = 800
y = 0
width = 800
height = 600
"#
    )
    .unwrap();
    path
}

/// Writes a two-screen layout (A at x=0..800, B at x=800..1600, both
/// 800x600) with the given `local_id`, returning the config file's path.
fn write_layout(dir: &Path, filename: &str, local_id: &str) -> PathBuf {
    write_layout_with_ids(dir, filename, local_id, "A", "B")
}

/// Runs `xmodmap -pke` against `display` and returns the X11 keycode bound
/// to the given unshifted keysym name (e.g. `"a"`). Mirrors the identical
/// helper in `crates/x11input/tests/xvfb_roundtrip.rs`: it lets a test
/// assert on the *exact* keycode a `xdotool key` press should produce,
/// rather than hardcoding a keycode number that isn't guaranteed to hold on
/// every X server's default keymap.
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

/// Proves keyboard events are actually wired end-to-end through main.rs, not
/// just implemented in the x11input crate in isolation: a key pressed on A's
/// (grabbed) X server while control is captured by B should arrive over the
/// real TCP connection as a `Message::KeyEvent` and get injected into B's X
/// server, observable via an independent `QueryKeymap` on B.
#[test]
fn keyboard_events_are_forwarded_while_remote() {
    let display_a = XvfbGuard::spawn(197, 800, 600);
    let display_b = XvfbGuard::spawn(198, 800, 600);

    let tmp = std::env::temp_dir().join(format!("mouseshare-e2e-kbd-{}", std::process::id()));
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

    // Hand control off to B.
    xdotool_mousemove(&display_a.display, 799, 300);
    std::thread::sleep(Duration::from_millis(300));

    // Raw X11 keycodes are forwarded unmapped (see mouseshare_protocol's
    // KeyEvent doc comment), so whatever keycode "a" resolves to on A is
    // exactly the keycode that should end up down on B.
    let keycode = xmodmap_keycode_for(&display_a.display, "a");

    // Independent connection to B, purely to observe key state via
    // QueryKeymap -- not going through mouseshare-x11input, so this is a
    // genuinely external check of what actually happened on B's server.
    let (keymap_conn, _) =
        x11rb::connect(Some(&display_b.display)).expect("connect to B for QueryKeymap");
    let is_down = |code: u8| -> bool {
        use x11rb::protocol::xproto::ConnectionExt as _;
        let reply = keymap_conn
            .query_keymap()
            .expect("query_keymap request")
            .reply()
            .expect("query_keymap reply");
        let byte = reply.keys[(code / 8) as usize];
        (byte & (1 << (code % 8))) != 0
    };
    assert!(!is_down(keycode), "keycode {keycode} should start up on B");

    xdotool(&display_a.display, &["keydown", "--clearmodifiers", "a"]);
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
        "keycode {keycode} pressed on A while captured should be forwarded and injected on B"
    );

    xdotool(&display_a.display, &["keyup", "--clearmodifiers", "a"]);
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
        "keycode {keycode} released on A while captured should be forwarded and injected on B"
    );

    drop(controller);
    drop(target);
}

/// Proves `mouseshare-discovery` is actually wired into main.rs: run a
/// target with no explicit port shared out-of-band and a controller with no
/// `--connect` at all, relying purely on mDNS to find each other, then
/// confirm the ordinary mouse-handoff path still works over the
/// auto-discovered address.
#[test]
fn controller_auto_discovers_target_via_mdns() {
    let display_a = XvfbGuard::spawn(199, 800, 600);
    let display_b = XvfbGuard::spawn(200, 800, 600);

    let tmp = std::env::temp_dir().join(format!("mouseshare-e2e-mdns-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // Unique screen ids so a stray responder from another concurrently
    // running test/process on the same LAN can't be mistaken for this
    // test's own target.
    let unique = std::process::id();
    let id_a = format!("A-{unique}");
    let id_b = format!("B-{unique}");
    let layout_a = write_layout_with_ids(&tmp, "a.toml", &id_a, &id_a, &id_b);
    let layout_b = write_layout_with_ids(&tmp, "b.toml", &id_b, &id_a, &id_b);

    let port = free_port();
    let bin = env!("CARGO_BIN_EXE_mouseshare");

    let target = ChildGuard(
        Command::new(bin)
            .args([
                "--config",
                layout_b.to_str().unwrap(),
                "target",
                "--listen",
                // Must be reachable at the LAN address mDNS advertises, not
                // just loopback, since the controller dials whatever
                // address discover() hands back.
                &format!("0.0.0.0:{port}"),
            ])
            .env("DISPLAY", &display_b.display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn target"),
    );
    // mDNS probe/announce settles slower than a plain TCP listen.
    std::thread::sleep(Duration::from_millis(1200));

    let controller = ChildGuard(
        Command::new(bin)
            .args([
                "--config",
                layout_a.to_str().unwrap(),
                "controller", // deliberately no --connect
            ])
            .env("DISPLAY", &display_a.display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn controller"),
    );
    // Room for discovery (well under its 3s default timeout in practice)
    // plus the TCP handshake.
    std::thread::sleep(Duration::from_millis(4000));

    xdotool_mousemove(&display_a.display, 799, 300);
    std::thread::sleep(Duration::from_millis(300));
    let (bx0, by0) = xdotool_mouselocation(&display_b.display);

    xdotool_mousemove(&display_a.display, 450, 300);
    std::thread::sleep(Duration::from_millis(300));
    let (bx1, by1) = xdotool_mouselocation(&display_b.display);
    assert_eq!(
        (bx1 - bx0, by1 - by0),
        (50, 0),
        "B's cursor should move by the forwarded delta after an mDNS auto-discovered connect"
    );

    drop(controller);
    drop(target);
}
