# Changelog

[简体中文](CHANGELOG.zh-CN.md)

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0]

A rebuilt, production-ready release: secure, multi-device, multi-monitor and
with a new GUI.

### Added

- **Any number of computers**, each with any number of monitors. The layout
  model treats each computer as a rigid monitor group; monitors are detected
  automatically via RandR and refreshed on connect and hot-plug.
- **Pairing code authentication and encryption** — SPAKE2 key exchange,
  ChaCha20-Poly1305 records, online-guess rate limiting. A random
  `XXXXX-XXXXX` code per computer, stored in `~/.config/mouseshare/pairing.toml`
  (mode `0600`).
- **Mouse buttons and scroll wheel** forwarding (previously only motion and
  keys), including extra buttons and horizontal scroll.
- The cursor now **appears where it crossed** the edge on the remote computer.
- **Reconnect** with exponential backoff, per-device **live latency** and
  status, and dead-link detection.
- **Emergency-return hotkey** (default `Ctrl+Alt+Esc`), **hold-guard**, **edge
  delay**, per-device **pointer speed**, **natural scrolling**, clipboard
  on/off and size limit.
- New **`config` crate**: settings and pairing codes persisted and shared by
  the CLI and the GUI. `mouseshare pair-code` prints / regenerates a code.
- **GUI rewrite**: dark / light / auto themes, six accents, UI scale, English
  and 简体中文 (CJK font discovered at runtime), first-run guide, settings
  window, toasts, device list with per-device editing, multi-monitor canvas
  with snap and one-click align, pairing-code screen for controlled computers,
  autostart.
- **Packaging**: `.deb`, tarball with `install.sh`, desktop entry and icon;
  release workflow on version tags.
- **Project health**: `docs/` (user guide, architecture, protocol & security
  design, troubleshooting, roadmap), Simplified Chinese translations of every
  document, security policy, issue forms, Dependabot, CODEOWNERS, expanded CI
  (format, lint, tests under Xvfb, audit, release build).

### Changed

- **Protocol version 2** (not compatible with 0.1.x): monitor lists in the
  handshake, buttons, scroll, absolute warp, ping/pong, and the encrypted
  transport.
- `layout.example.toml` uses the multi-monitor `[[devices]]` form; the old
  `[[screens]]` form still loads.
- The CLI takes `--pair-code`, repeated `--connect id=host:port`, and uses
  saved settings when no `--config` is given.
- The log subscriber writes to stderr so stdout stays clean.

### Removed

- The unauthenticated, unencrypted transport. There is deliberately no
  "insecure mode".

## [0.1.0]

Initial public release. LAN mouse + keyboard sharing between two Linux/X11
machines, with:

- Edge-based control handoff (`crates/layout`) over a configurable
  virtual-desktop layout
- X11 pointer/keyboard capture and injection (`crates/x11input`)
- A length-prefixed TCP wire protocol (`crates/protocol`, `crates/net`)
- Bidirectional plain-text clipboard sync over the same connection
- LAN auto-discovery via mDNS (`crates/discovery`), manual IP still
  supported as a fallback
- An optional `mouseshare-gui` binary (`crates/gui`, `egui`/`eframe`) for
  visual configuration and connection management, alongside the
  lightweight `mouseshare` CLI
- No authentication or encryption yet — LAN-trust-only for this release

[Unreleased]: https://github.com/PelerYuan/mouseshare/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/PelerYuan/mouseshare/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/PelerYuan/mouseshare/releases/tag/v0.1.0
