//! End-to-end test of `Clipboard` against a real (headless) Xvfb server and
//! a real independent `xclip` process -- not just in-crate logic, but an
//! actual ICCCM selection exchange with another client, the same way a real
//! text editor or terminal would interact with it.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mouseshare_x11input::Clipboard;

struct XvfbGuard {
    display: String,
    child: Child,
}

impl XvfbGuard {
    fn spawn(display_num: u32) -> Self {
        let display = format!(":{display_num}");
        let child = Command::new("Xvfb")
            .arg(&display)
            .args(["-screen", "0", "800x600x24"])
            .arg("-nolisten")
            .arg("tcp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn Xvfb - is it installed?");

        let socket = format!("/tmp/.X11-unix/X{display_num}");
        wait_for_path(Path::new(&socket), Duration::from_secs(10))
            .unwrap_or_else(|| panic!("Xvfb did not create socket {socket} in time"));
        std::thread::sleep(Duration::from_millis(200));

        Self { display, child }
    }
}

impl Drop for XvfbGuard {
    fn drop(&mut self) {
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
                    std::thread::sleep(Duration::from_millis(20))
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

fn connect_retrying(display: &str) -> Clipboard {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match Clipboard::connect(Some(display)) {
            Ok(c) => return c,
            Err(err) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
                let _ = err;
            }
            Err(err) => panic!("failed to connect to {display} after retrying for 5s: {err}"),
        }
    }
}

/// Spawns `xclip -o` (non-blocking) and polls `clipboard` concurrently
/// until it exits -- unlike a plain blocking `Command::output()` call, this
/// keeps servicing xclip's `SelectionRequest` while waiting for it, the
/// same way the real app's tick loop would.
fn xclip_paste_while_polling(display: &str, clipboard: &mut Clipboard) -> String {
    let mut child = Command::new("xclip")
        .args(["-o", "-selection", "clipboard"])
        .env("DISPLAY", display)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run xclip -o - is it installed?");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        clipboard.poll().unwrap();
        if let Some(status) = child.try_wait().expect("try_wait xclip -o") {
            let output = child.wait_with_output().expect("collect xclip -o output");
            assert!(
                status.success(),
                "xclip -o failed: stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
            return String::from_utf8_lossy(&output.stdout).to_string();
        }
        assert!(Instant::now() < deadline, "xclip -o never exited");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn xclip_copy(display: &str, text: &str) {
    use std::io::Write as _;
    let mut child = Command::new("xclip")
        .args(["-i", "-selection", "clipboard"])
        .env("DISPLAY", display)
        .stdin(Stdio::piped())
        .spawn()
        .expect("failed to run xclip -i - is it installed?");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .expect("write to xclip stdin");
    let status = child.wait().expect("wait for xclip -i");
    assert!(status.success(), "xclip -i exited with {status}");
}

/// `set_text` makes our content available to a real, independent client
/// asking for it -- proves the `SelectionRequest` servicing path (TARGETS
/// and UTF8_STRING) actually works, not just that it compiles.
#[test]
fn set_text_is_readable_by_another_client() {
    let xvfb = XvfbGuard::spawn(210);
    let mut clipboard = connect_retrying(&xvfb.display);

    clipboard
        .set_text("hello from mouseshare".to_string())
        .unwrap();

    let got = xclip_paste_while_polling(&xvfb.display, &mut clipboard);
    assert_eq!(got, "hello from mouseshare");
}

/// When another real client (`xclip`) takes clipboard ownership, `poll`
/// should notice via `GetSelectionOwner` and fetch the new content via a
/// real `ConvertSelection` round trip against that client.
#[test]
fn detects_local_change_from_another_client() {
    let xvfb = XvfbGuard::spawn(211);
    let mut clipboard = connect_retrying(&xvfb.display);

    xclip_copy(&xvfb.display, "typed directly into xclip");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = None;
    while Instant::now() < deadline {
        if let Some(text) = clipboard.poll().unwrap() {
            seen = Some(text);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(seen.as_deref(), Some("typed directly into xclip"));
}

/// A second, different local change should be detected again, not
/// suppressed by the dedup logic meant only to stop echoing an update we
/// just applied ourselves.
#[test]
fn detects_successive_local_changes() {
    let xvfb = XvfbGuard::spawn(212);
    let mut clipboard = connect_retrying(&xvfb.display);

    let poll_until = |clipboard: &mut Clipboard, expected: &str| {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(text) = clipboard.poll().unwrap() {
                assert_eq!(text, expected);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("never observed clipboard change to {expected:?}");
    };

    xclip_copy(&xvfb.display, "first");
    poll_until(&mut clipboard, "first");

    xclip_copy(&xvfb.display, "second");
    poll_until(&mut clipboard, "second");
}
