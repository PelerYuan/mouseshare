# Roadmap

[简体中文](ROADMAP.zh-CN.md)

What mouseshare does today, what is planned, and where help is most welcome.
Items marked 🙋 are open for contributors — comment on the linked issue (or open
one) before starting something large.

## Shipped

- Any number of computers, each with any number of monitors (RandR), arranged
  on a drag-and-snap canvas with one-click alignment.
- Pointer movement, buttons, scroll wheel and keyboard; plain-text clipboard.
- Pairing code with SPAKE2 authentication and ChaCha20-Poly1305 encryption;
  online-guess rate limiting.
- Automatic reconnect, per-device live status and latency, dead-link detection.
- Emergency-return hotkey, hold-guard, edge delay, per-device pointer speed,
  natural-scroll option, clipboard size limit.
- GUI: dark / light / auto themes, six accents, UI scale, English + 简体中文,
  first-run guide, settings window, toasts, autostart.
- CLI sharing the same settings, `.deb` and tarball releases, end-to-end tests
  on real X servers, CI on every pull request.

## Next

| Item | Notes |
|---|---|
| 🙋 System tray / minimise-to-tray | Needs StatusNotifierItem (D-Bus) or GTK AppIndicator; behaves differently across desktops. |
| 🙋 More translations | UI strings live in `crates/gui/src/strings.rs`; docs in `*.zh-CN.md` are the model for other languages. |
| 🙋 Layout import/export in the GUI | `LayoutConfig::to_toml_string` already writes the CLI format. |
| Per-profile arrangements | Home / office presets. |
| Jump-to-screen shortcut | Hotkey to send the cursor to a specific computer. |
| Image and rich-text clipboard | Needs a size-aware clipboard protocol. |
| Packages for more distros | `.rpm`, AUR, Flatpak/AppImage. |

## Later / larger projects

| Item | Notes |
|---|---|
| 🙋 **Wayland** | Needs a second `input` backend (libei / XDG RemoteDesktop portal for injection, compositor protocols for capture). The seam is the `x11input` crate; the first step is turning its concrete types into a trait. |
| 🙋 **Windows and macOS** | Same backend seam; keycodes would need a cross-platform representation (today `KeyEvent` carries raw X11 keycodes). |
| 🙋 File drag and drop between computers | Needs a per-platform file-transfer protocol. |
| Multiple controllers | Currently one controller drives many targets. |

## Deliberately out of scope

- An insecure / unauthenticated mode — a footgun on a shared network.
- Cloud relays or accounts — mouseshare is LAN-only by design.
- Remote desktop / screen streaming — use a remote-desktop tool for that.

## How to help

See [CONTRIBUTING.md](../CONTRIBUTING.md). Good first contributions: a
translation, a doc fix, a bug report with a log, or testing on a desktop
environment we have not tried. Issues labelled
[`help wanted`](https://github.com/PelerYuan/mouseshare/labels/help%20wanted)
and [`good first issue`](https://github.com/PelerYuan/mouseshare/labels/good%20first%20issue)
are the best starting points.
