<div align="center">

<img src="packaging/mouseshare.svg" width="96" alt="mouseshare logo">

# mouseshare

**One mouse, one keyboard, one clipboard — across all your computers.**

[![CI](https://github.com/PelerYuan/mouseshare/actions/workflows/ci.yml/badge.svg)](https://github.com/PelerYuan/mouseshare/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/PelerYuan/mouseshare?include_prereleases&sort=semver)](https://github.com/PelerYuan/mouseshare/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform: Linux/X11](https://img.shields.io/badge/platform-Linux%20%7C%20X11-lightgrey.svg)](docs/ROADMAP.md)

**English** · [简体中文](README.zh-CN.md)

</div>

mouseshare lets the mouse and keyboard of one computer control several others
on the same network. Move the cursor off the edge of your screen and it
appears on the next computer; copy text on one machine and paste it on
another. It is a small, fast, encrypted alternative to Synergy / Barrier /
Input Leap, with a modern GUI that gets you from install to working setup in
about two minutes.

## Highlights

- **Any number of computers**, each with any number of monitors. Drag the
  screens into the same arrangement as on your desk.
- **Pair with a short code.** The computer being controlled shows a
  `XXXXX-XXXXX` code; type it once on the controller. Connections are
  authenticated (SPAKE2) and end-to-end encrypted (ChaCha20-Poly1305) — no
  certificates to manage, and the code cannot be brute-forced offline.
- **Everything you expect:** mouse movement, buttons, scroll wheel, keyboard,
  and plain-text clipboard sync.
- **Robust.** Automatic reconnect with backoff, live latency per device, an
  emergency-return hotkey (`Ctrl+Alt+Esc`) if the cursor ever gets stuck, and
  a hold-guard so edges are ignored while you drag.
- **Modern GUI** with dark / light / automatic themes, six accent colours,
  adjustable UI scale, English and 简体中文, a first-run guide and toast
  notifications. A small CLI (`mouseshare`) shares the same settings for
  headless use.
- **Lightweight.** Pure-Rust, no daemon, no background service; the CLI
  binary is about a megabyte and a half.

## Install

Download the latest `.deb` or tarball from the
[Releases page](https://github.com/PelerYuan/mouseshare/releases).

```bash
# Debian / Ubuntu
sudo apt install ./mouseshare_<version>_amd64.deb

# any Linux: user-level install into ~/.local
tar xzf mouseshare-<version>-x86_64-linux.tar.gz
cd mouseshare-<version>-x86_64-linux && ./install.sh
```

Or build from source (Rust 1.88+):

```bash
git clone https://github.com/PelerYuan/mouseshare.git
cd mouseshare
cargo build --release          # target/release/mouseshare and mouseshare-gui
```

## Quick start

1. **On every computer**, open *mouseshare*.
2. **On the computer to be controlled**, choose **Controlled** and press
   **Start sharing**. A pairing code appears.
3. **On the computer with the mouse and keyboard**, choose **Control**, click
   **Add device**, pick the other computer (found automatically) and type the
   code.
4. Drag the screens so they match how they sit on your desk, then press
   **Start sharing**.
5. Push the cursor through the edge of the screen. 🎉

If the cursor gets stuck on another computer, press **Ctrl+Alt+Esc**.

Prefer the terminal?

```bash
mouseshare target                      # prints the pairing code
mouseshare controller --connect desk=192.168.1.20:7878 --pair-code desk=7F3KD-9X2MQ
```

See the [User Guide](docs/USER_GUIDE.md) for the full walkthrough, every
setting, and the command-line reference.

## Documentation

| | |
|---|---|
| [User Guide](docs/USER_GUIDE.md) | Setup, daily use, settings, CLI |
| [Troubleshooting & FAQ](docs/TROUBLESHOOTING.md) | When something does not work |
| [Architecture](docs/ARCHITECTURE.md) | How the code is organised |
| [Protocol & security design](docs/PROTOCOL.md) | Wire format, handshake, cryptography |
| [Roadmap](docs/ROADMAP.md) | What is done, what is next, how to help |
| [Security policy](SECURITY.md) | Reporting a vulnerability |
| [Changelog](CHANGELOG.md) | Release history |
| [Contributing](CONTRIBUTING.md) | Build, test, open a pull request |

Every document has a Simplified Chinese counterpart (`*.zh-CN.md`).

## Requirements

- Linux with an **X11** session on every computer (Wayland is on the
  [roadmap](docs/ROADMAP.md); under Wayland you can usually pick an
  "X11/Xorg" session at the login screen).
- The computers must reach each other over TCP (default port `7878`) and
  ideally allow mDNS (multicast DNS) for automatic discovery — typing an
  address works too.

## Status

mouseshare is young but complete for its scope: the end-to-end test suite
drives real X servers through every feature (movement, clicks, scroll, keys,
clipboard, wrong-code rejection, reconnect, multi-monitor crossing). Linux/X11
only for now. See the [roadmap](docs/ROADMAP.md) for what is planned.

## Contributing

Contributions of every size are welcome — bug reports, translations, docs,
and code. Start with [CONTRIBUTING.md](CONTRIBUTING.md) and the issues labelled
[`good first issue`](https://github.com/PelerYuan/mouseshare/labels/good%20first%20issue)
or [`help wanted`](https://github.com/PelerYuan/mouseshare/labels/help%20wanted).
Please read the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE)
