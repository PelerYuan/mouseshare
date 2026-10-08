# Protocol & security design

[简体中文](PROTOCOL.zh-CN.md)

This document describes what mouseshare sends over the network and why it is
safe to do so. The protocol is currently **version 2**
(`mouseshare_protocol::PROTOCOL_VERSION`).

## Contents

- [Discovery](#discovery)
- [Transport and framing](#transport-and-framing)
- [Security design](#security-design)
- [Messages](#messages)
- [Session lifecycle](#session-lifecycle)
- [Coordinates and monitors](#coordinates-and-monitors)
- [Versioning](#versioning)

## Discovery

A computer that is sharing as *Controlled* advertises itself with mDNS /
DNS-SD as `_mouseshare._tcp.local.` with two TXT records:

| Key | Value |
|---|---|
| `screen_id` | the device name |
| `protocol_version` | the protocol version it speaks |

Advertisement contains **no secrets**. Discovery is optional: a controller can
always be given an explicit `host:port`.

## Transport and framing

One TCP connection per controller ↔ target pair (default port `7878`,
`TCP_NODELAY` on). Every message is a **frame**:

```text
+----------------+---------------------------+
| length: u32 BE | body (length bytes)       |
+----------------+---------------------------+
```

- Before the key exchange completes, frames carry the raw SPAKE2 messages
  (at most 256 bytes).
- After it, the body is `ChaCha20-Poly1305(bincode(Message))` including the
  16-byte tag, and `length` covers the ciphertext.
- Frames larger than `MAX_FRAME_LEN` (1 MiB) are rejected *before* any
  allocation, so a malicious length prefix cannot exhaust memory.

Messages are serialised with [bincode](https://docs.rs/bincode).

## Security design

**Goals.** Only someone who knows the pairing code can control a computer or
read its traffic, even on an untrusted network; and the code is short enough to
type, so it must not be crackable offline.

**Pairing code.** 10 characters of Crockford base32 (`0-9 A-Z` without
`I L O U`), shown as `XXXXX-XXXXX` — 50 bits from the OS random source. Input
is case-insensitive and tolerates look-alikes (`O→0`, `I/L→1`).

**Handshake** (both sides know only the code):

1. dialer → listener: SPAKE2 message A (33 bytes)
2. listener → dialer: SPAKE2 message B (33 bytes)
3. Both compute the SPAKE2 shared key `K` (Ed25519 group, identities
   `mouseshare-v2-dialer` / `mouseshare-v2-listener`) and derive two 32-byte
   keys with HKDF-SHA256, salted with `SHA-256(A ‖ B)`, one per direction
   (`"mouseshare v2 dialer->listener"`, `"mouseshare v2 listener->dialer"`).
4. The dialer sends an encrypted `Hello`; the listener answers with an
   encrypted `HelloAck`. A successful decryption in each direction **is** the
   mutual key confirmation. With a wrong code the keys differ, the first AEAD
   tag fails and the connection is dropped without revealing anything else.

**Record layer.** ChaCha20-Poly1305 with a 96-bit nonce that is a strictly
increasing per-direction counter (safe because keys are per-connection and
per-direction). Replayed, reordered, dropped or modified records fail
authentication and end the session.

**Why a PAKE.** With a short code, a plain MAC/AEAD keyed from the code would
let an eavesdropper test guesses offline at billions per second. SPAKE2 turns
every guess into one *live* connection attempt.

**Online guessing.** The target allows at most **5 failed handshakes per source
address per 60 seconds** and drops further attempts from that address. At 50
bits that makes brute force hopeless.

**What it protects against:** passive eavesdropping, active
man-in-the-middle, replay, injection of input from other hosts, and offline
cracking of captured traffic.

**What it does not protect against:** someone who already has the code or
`pairing.toml` (treat both like a password), malware on either computer, or
denial of service (an attacker on the network can still make connections fail).
Session keys come from ephemeral values, so learning the code later does not
let anyone decrypt traffic captured earlier — but from then on they can
impersonate a peer, so regenerate the code if it leaks.

The listener only ever accepts **one controller at a time**; a second
connection is refused while one is active.

## Messages

`enum Message` in `crates/protocol/src/lib.rs`. Positive `dx/dy` is right/down.

| Message | Direction | Meaning |
|---|---|---|
| `Hello { version, device_id, monitors }` | controller → target | First encrypted message: protocol version, the controller's name and monitors. |
| `HelloAck { version, device_id, monitors }` | target → controller | Reply with the target's name and monitors. |
| `MouseMove { dx, dy }` | controller → target | Relative pointer motion in target pixels (already scaled by the device's pointer speed). |
| `MouseWarp { x, y }` | controller → target | Place the pointer at an absolute position in the target's root coordinates; sent when control enters the device so the cursor appears where it crossed. |
| `MouseButton { button, pressed }` | controller → target | X11 button number (1 left, 2 middle, 3 right, 8/9 back/forward). Wheel buttons 4–7 are never sent here. |
| `Scroll { dx, dy }` | controller → target | Wheel notches. Positive `dy` scrolls down, positive `dx` right. |
| `KeyEvent { keycode, pressed }` | controller → target | Raw X11 keycode press/release. |
| `ClipboardText(String)` | both | The sender's clipboard changed to this plain text. |
| `Ping(u64)` / `Pong(u64)` | both | Latency probe and liveness check; `Pong` echoes the token. |
| `MonitorsChanged(Vec<MonitorInfo>)` | both | Monitor hot-plug or resolution change. |

`MonitorInfo { name, x, y, width, height, primary }` is in the sender's own
root-window coordinates; the top-left of the leftmost monitor is not
necessarily `(0, 0)`.

## Session lifecycle

1. The controller resolves the target's address (explicit, or via mDNS), dials
   it (5 s timeout) and runs the handshake (10 s timeout).
2. Both sides send their monitor lists in `Hello` / `HelloAck`; the controller
   places the target in the arrangement.
3. While the cursor is on the controller's own screens nothing but pings and
   clipboard text flows. When it crosses an edge the controller sends
   `MouseWarp` then streams `MouseMove`, `MouseButton`, `Scroll` and
   `KeyEvent`; input is captured locally and not delivered to local windows.
4. A `Ping` is sent every 2 s. If no traffic arrives for 10 s the link is
   declared dead: held buttons and keys are released on the target, the cursor
   returns to the controller, and the supervisor reconnects with exponential
   backoff (1 s → 15 s).
5. The emergency-return hotkey is matched locally on the controller and never
   forwarded.

## Coordinates and monitors

Each device owns a **rigid group** of monitors placed in a shared *virtual
desktop*. Edge detection runs over the *union* of a device's monitors, so the
cursor can cross wherever monitors of two devices touch, and it enters at the
corresponding point (`MouseWarp`). Different resolutions are handled in plain
pixels, the same model real multi-monitor setups use — there is no automatic
proportional scaling; use *Align* to choose where screens meet.

## Versioning

`Hello.version` carries the protocol version. A mismatch is reported as an
incompatible-version error and the connection closes. Within a version new
message variants are not added; any change to the wire format bumps the
version.
