# mouseshare

A LAN mouse and keyboard sharing tool, in the spirit of
Synergy/Barrier/Input Leap: one machine owns the physical mouse/keyboard;
moving the cursor into a configured screen edge hands control to the
neighboring machine over the network.

## Status

MVP working end-to-end: two machines, mouse + keyboard, Linux X11 only,
LAN auto-discovery via mDNS (manual IP still works too), no encryption
yet. Verified against two real headless X servers (see `tests/e2e.rs`) —
edge detection, capture, network forwarding, injection, keyboard
forwarding, mDNS discovery, and handoff back to local all work together,
not just in isolation. See the requirements analysis this project
started from for the full phased roadmap (auth/encryption, multi-monitor
DPI handling, Wayland support, clipboard/file transfer) — those are
deliberately out of scope for this first pass.

## Running it manually

On the target machine (the one that will receive forwarded mouse and
keyboard input — copy `layout.example.toml`, set `local_id` to that
machine's screen id). It also starts announcing itself over mDNS so
controllers on the LAN can auto-discover it:

```bash
mouseshare --config layout.toml target --listen 0.0.0.0:7878
```

On the controller machine (the one with the physical mouse/keyboard;
same layout file, `local_id` set to its own screen id), either let it
find the target automatically over mDNS:

```bash
mouseshare --config layout.toml controller
```

or connect to a known address directly, bypassing discovery
(`--discover-timeout-secs` tunes how long the mDNS lookup waits when
`--connect` is omitted; defaults to 3s):

```bash
mouseshare --config layout.toml controller --connect <target-ip>:7878
```

Move the mouse into the edge of the controller's screen that borders the
target's screen in the layout config, and control (mouse and keyboard)
hands off.

## GUI

`mouseshare-gui` is an optional, separately-built binary for people who'd
rather not hand-edit TOML: it auto-detects your local screen resolution
and hostname, lets you pick controller/target and drag the remote screen
against any edge of the local one (always snapping flush, so the layout
crate's touching-edges invariant holds), scans the LAN for other
mouseshare instances via mDNS with click-to-fill, and shows a live log
panel. It's built on the same `mouseshare-core` runtime as the CLI, just a
different frontend.

```bash
cargo run --release -p mouseshare-gui
```

This is a deliberate exception to the project's lightweight-by-default
stance: pulling in `egui`/`eframe` costs real size (~5M release, stripped,
vs. the CLI's ~1.4M) for the sake of not hand-editing config files. The
CLI binary is completely unaffected — it's a separate crate/binary and
doesn't depend on the GUI crate at all.

## Layout

Workspace crates:

- `crates/protocol` — wire message types (`Message` enum: `Hello`,
  `HelloAck`, `MouseMove{dx,dy}`, `KeyEvent{keycode,pressed}`,
  `Heartbeat`) and length-prefixed framing (`encode_frame`/`decode_body`).
  Pure logic, no I/O.
- `crates/layout` — screen geometry (`ScreenRect`, TOML-loaded
  `LayoutConfig`) and the `EdgeDetector` state machine that tracks a
  virtual cursor position across the whole configured desktop and decides
  when control should hand off between screens. Pure logic, no I/O — the
  controller machine is the only one that ever runs this, since every
  delta forwarded to a peer originated locally and peers never need to
  report position back.
- `crates/net` — tokio TCP transport built on the protocol crate's
  framing, plus the `Hello`/`HelloAck` handshake.
- `crates/x11input` — X11 pointer/keyboard capture (grab + re-center
  technique to extract raw motion deltas without letting the real cursor
  move) and injection (relative/absolute `WarpPointer`, XTest
  `fake_input` for keys), via `x11rb`.
- `crates/discovery` — LAN auto-discovery via mDNS/DNS-SD (`mdns-sd`):
  `Announcement::start` advertises a target's `screen_id`/port, `discover`
  browses for one.
- `crates/core` — the actual controller/target runtime loops
  (`run_controller`, `run_target`, `resolve_target_addr`), shared by every
  frontend so there's exactly one implementation to get right and keep
  tested. Observability is left to `tracing` rather than a bespoke
  callback API, so each frontend can install whatever subscriber suits it.
- `crates/gui` — the optional `mouseshare-gui` binary (see "GUI" above):
  `eframe`/`egui` UI, a drag-and-snap screen-arrangement widget, and a
  `tracing_subscriber` writer that feeds a live log panel.

`KeyEvent`'s keycode is a raw, unmapped X11 keycode: fine while both ends
are X11 (the only backend that exists so far), but not real cross-platform
key translation — see that variant's doc comment in
`crates/protocol/src/lib.rs`.

`src/main.rs` is a thin CLI wrapper around `mouseshare-core` (arg parsing,
loading the layout file, picking controller/target role); `crates/gui`'s
`main.rs` is the other frontend on top of the same core.

## Config

See `layout.example.toml` — copy it, adjust `local_id`/coordinates per
machine. Screen rects live in a shared virtual-desktop coordinate space;
edges must touch exactly for handoff to feel seamless (a gap means the
cursor gets stuck at the wall instead of crossing over).

## Running tests

```bash
cargo test --workspace
```

The `x11input` crate's tests and the root `tests/e2e.rs` (which runs the
actual compiled binary as both a controller and a target against two
throwaway Xvfb instances) need `Xvfb`, `xdotool`, and `xmodmap` installed,
but not a real display. The `discovery` crate's tests and `e2e.rs`'s
mDNS-discovery test additionally need the environment to actually permit
IP multicast on some interface (including loopback) — see
`crates/discovery/tests/announce_and_discover.rs` for details.
