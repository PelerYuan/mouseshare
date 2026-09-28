# Changelog

All notable changes to this project are documented here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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

[Unreleased]: https://github.com/PelerYuan/mouseshare/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/PelerYuan/mouseshare/releases/tag/v0.1.0
