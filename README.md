# mouseshare

A LAN mouse (and eventually keyboard) sharing tool, in the spirit of
Synergy/Barrier/Input Leap: one machine owns the physical mouse; moving the
cursor into a configured screen edge hands control to the neighboring
machine over the network.

## Status

MVP working end-to-end: two machines, mouse-only, Linux X11 only, manual
IP config (no mDNS discovery, no encryption yet). Verified against two
real headless X servers (see `tests/e2e.rs`) — edge detection, capture,
network forwarding, injection, and handoff back to local all work
together, not just in isolation. See the requirements analysis this
project started from for the full phased roadmap (keyboard sharing, mDNS
discovery, auth/encryption, multi-monitor DPI handling, Wayland support,
clipboard/file transfer) — those are deliberately out of scope for this
first pass.

## Running it manually

On the target machine (the one that will receive forwarded mouse
movement — copy `layout.example.toml`, set `local_id` to that machine's
screen id):

```bash
mouseshare --config layout.toml target --listen 0.0.0.0:7878
```

On the controller machine (the one with the physical mouse; same layout
file, `local_id` set to its own screen id):

```bash
mouseshare --config layout.toml controller --connect <target-ip>:7878
```

Move the mouse into the edge of the controller's screen that borders the
target's screen in the layout config, and control hands off.

## Layout

Workspace crates:

- `crates/protocol` — wire message types (`Message` enum: `Hello`,
  `HelloAck`, `MouseMove{dx,dy}`, `Heartbeat`) and length-prefixed framing
  (`encode_frame`/`decode_body`). Pure logic, no I/O.
- `crates/layout` — screen geometry (`ScreenRect`, TOML-loaded
  `LayoutConfig`) and the `EdgeDetector` state machine that tracks a
  virtual cursor position across the whole configured desktop and decides
  when control should hand off between screens. Pure logic, no I/O — the
  controller machine is the only one that ever runs this, since every
  delta forwarded to a peer originated locally and peers never need to
  report position back.
- `crates/net` — tokio TCP transport built on the protocol crate's
  framing, plus the `Hello`/`HelloAck` handshake.
- `crates/x11input` — X11 pointer capture (grab + re-center technique to
  extract raw motion deltas without letting the real cursor move) and
  injection (relative/absolute `WarpPointer`), via `x11rb`.

`src/main.rs` ties these together into the actual binary (controller role
vs. target role, selected on the command line).

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
throwaway Xvfb instances) need `Xvfb` and `xdotool` installed, but not a
real display.
