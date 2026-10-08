# Architecture

[简体中文](ARCHITECTURE.zh-CN.md)

mouseshare is a Cargo workspace of small crates with one-way dependencies.
Pure logic (protocol, layout) is kept apart from I/O (net, x11input) so it can
be tested without a display or a network.

```text
                    ┌─────────┐   ┌────────────┐
   mouseshare (CLI) │  gui    │   │  tests/e2e │
                    └────┬────┘   └─────┬──────┘
                         │              │ drives real binaries
                    ┌────▼──────────────▼───┐
                    │ config  (settings, pairing codes, autostart)
                    └────┬──────────────────┘
                    ┌────▼────┐
                    │  core   │  controller / target runtime, telemetry
                    └─┬──┬──┬─┘
        ┌─────────────┘  │  └───────────────┐
   ┌────▼────┐     ┌─────▼─────┐      ┌─────▼─────┐   ┌───────────┐
   │ layout  │     │    net    │      │ x11input  │   │ discovery │
   │ (pure)  │     │ SPAKE2+AEAD      │ capture/  │   │   mDNS    │
   └─────────┘     └─────┬─────┘      │ inject/   │   └───────────┘
                         │            │ clipboard │
                    ┌────▼────┐       └───────────┘
                    │protocol │ (pure) wire messages & framing
                    └─────────┘
```

## Crates

| Crate | Role | Notes |
|---|---|---|
| `protocol` | `Message` enum, `MonitorInfo`, length-prefixed framing | Pure; bincode + serde. |
| `layout` | `Device`/`Monitor` model, `LayoutConfig` (TOML), `EdgeDetector` | Pure. Edge detection over the union of a device's monitors; entry points; hold-guard. |
| `net` | TCP transport, SPAKE2 key exchange, ChaCha20-Poly1305 record layer, `PairingCode` | tokio. Wrong code → `AuthFailed`. |
| `x11input` | RandR monitor listing, capture (grab + recentre), XTest injection, ICCCM clipboard | `x11rb` only; the single place that knows about X11. |
| `discovery` | mDNS announce / browse | `mdns-sd`. |
| `core` | The runtime: `run_controller`, `run_target`, `Telemetry`, hotkey | One implementation shared by CLI and GUI. |
| `config` | `Settings`, `PairingStore`, autostart entry | Plain TOML under `~/.config/mouseshare/`; pairing file is `0600`. |
| `gui` (`mouseshare-gui`) | egui/eframe application | Theme, i18n, onboarding, canvas, settings. |
| root (`mouseshare`) | clap CLI | Thin wrapper over `core` and `config`. |

## Controller runtime

```text
 local input ──► input loop (8 ms tick) ──► EdgeDetector ──► active device?
                    ▲        │                                  │
   supervisor ◄─────┘        └─ MouseMove/Button/Scroll/Key ────┤
   tasks (one per target)       over the active peer's channel  ▼
   - resolve address                                      remote device
   - dial + handshake
   - ping / latency
   - reconnect (1s→15s)
```

- One **supervisor task per target** owns the connection lifecycle and
  exposes it to the input loop through channels, so a slow or dead peer can
  never stall input handling for the others.
- The **input loop** polls the local pointer. While control is local it feeds
  positions to `EdgeDetector`; while remote it consumes captured deltas,
  applies the per-device pointer speed, forwards events, and tracks the virtual
  cursor to detect the way back (or a crossing into a third device).
- On peer loss or the emergency hotkey, held buttons/keys are released on the
  remote and the cursor is returned to the local device **centre**, so it does
  not instantly re-trigger a hand-off at the edge.
- `Telemetry` is a shared snapshot (peer states, latency, monitors, who has the
  cursor) plus a queue of notices. Frontends poll it; `core` never calls into a
  UI.

## Target runtime

`run_target` binds the port, announces itself over mDNS, and accepts one
authenticated controller at a time. Wrong codes are counted per source IP
(5 per minute). Received events are injected with XTest; absolute warps use
`WarpPointer`; the target reports monitors on connect and when they change.

## GUI

```text
App (state + per-frame update)
 ├─ theme      tokens, light/dark palettes, egui style
 ├─ widgets    buttons, cards, toggle, segmented control, pills
 ├─ canvas     device-group arrangement view (pan/zoom/snap/align)
 ├─ rail       role switch, device list + detail editor, Start/Stop
 ├─ target_view  pairing-code screen for controlled computers
 ├─ dialogs    modal frame, add device, confirm removal
 ├─ settings_view / onboarding
 ├─ i18n + strings   en / zh-CN tables, key-coverage tests
 ├─ fonts      system UI font, Phosphor icons, runtime CJK font
 └─ session / scan   background tokio runtime, mDNS scanning thread
```

The UI is immediate-mode: state lives in `App` and `Settings`; `Settings` is
auto-saved (debounced) whenever it changes. Network work runs on a background
tokio runtime so the UI thread never blocks.

## Design decisions

- **Why X11 only (for now).** Global capture and injection are what the whole
  product is. X11 allows both; Wayland needs compositor-specific protocols.
  All X11 code is behind `x11input`, which is where a second backend would plug
  in (see the [roadmap](ROADMAP.md)).
- **Why SPAKE2 instead of TLS.** A typed code is low-entropy; a PAKE makes it
  safe and avoids certificate management. See
  [Protocol & security design](PROTOCOL.md#security-design).
- **Why monitors are rigid groups.** Users think in computers, not monitors;
  moving a computer keeps its internal geometry correct and makes edge
  crossing predictable.
- **Why two front-ends share `core`.** Behaviour is implemented and tested
  once; the CLI and the GUI differ only in how they collect settings and show
  state.
- **Why the end-to-end tests drive real X servers.** Mocked input would not
  catch the bugs that matter (grab ordering, XTest quirks, clipboard ownership).

## Testing

| Layer | How |
|---|---|
| `protocol`, `layout`, `config`, `net` | Plain unit tests (no display). |
| `x11input` | Against throwaway `Xvfb` displays with `xdotool` / `xclip` as independent clients. |
| `gui` | i18n key coverage, plus manual visual checks. |
| End to end (`tests/e2e.rs`) | Spawns the compiled `mouseshare` as controller and target on two Xvfb displays: handoff, buttons, scroll, keys, clipboard, mDNS discovery, wrong-code rejection, reconnect, return hotkey, multi-monitor crossing. |

`cargo test --workspace` runs everything; it needs `xvfb xdotool x11-xserver-utils xclip`.
