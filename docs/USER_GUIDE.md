# User Guide

[简体中文](USER_GUIDE.zh-CN.md)

This guide walks through setting up mouseshare, using it day to day, every
setting in the app, and the command-line tool.

## Contents

1. [How it works](#how-it-works)
2. [Install](#install)
3. [First run](#first-run)
4. [Pairing two computers](#pairing-two-computers)
5. [Arranging screens](#arranging-screens)
6. [Daily use](#daily-use)
7. [Settings reference](#settings-reference)
8. [Command line](#command-line)
9. [Files and configuration](#files-and-configuration)
10. [Uninstall](#uninstall)

## How it works

Every computer plays one of two roles:

- **Controller** — has the physical mouse and keyboard. There is exactly one
  controller in a setup.
- **Controlled** (target) — is driven by the controller. There can be any
  number of them.

When the cursor leaves the controller's screen through an edge that touches
another computer's screen in your arrangement, the mouse, keyboard and
scroll wheel are forwarded to that computer until the cursor comes back. The
clipboard text follows automatically in both directions.

## Install

See the [README](../README.md#install). After installing, *mouseshare* appears
in your application menu. The `mouseshare-gui` and `mouseshare` commands are on
your `PATH` (for the tarball installer, add `~/.local/bin` if it is not
already).

Requirements: Linux with an **X11** session on every computer, and TCP
connectivity between them (default port `7878`).

## First run

The first time you open the app a short guide asks for:

1. a **name** for this computer (other computers see it), and
2. its **role** — *Control other computers* or *Be controlled*.

You can change both later and replay the guide from
**Settings → General → Show the welcome guide again**.

## Pairing two computers

Pairing is a one-time step per computer.

1. On the **controlled** computer, pick **Controlled** in the left rail and
   press **Start sharing**. The main area shows a large **pairing code**
   (for example `7F3KD-9X2MQ`) and the computer's address on your network.
2. On the **controller**, press **Add device**. Computers that are sharing
   and reachable by mDNS appear in the list — click one to fill in its name
   and address. If nothing appears, type the name and the address
   (`192.168.1.20:7878`) yourself.
3. Type the pairing code and press **Add**.

The code is remembered, so next time you only press **Start sharing**. Codes
are case-insensitive and punctuation is ignored. If you ever need a new one,
stop sharing on the controlled computer and press **New code**; then enter the
new code on the controller (select the device in the rail and edit its code).

> **Why a code?** The code authenticates both sides and creates the
> encryption keys. Someone who does not know it cannot control your computer
> or read your traffic, even on a shared network. See
> [Protocol & security design](PROTOCOL.md#security-design).

## Arranging screens

On the controller, the canvas shows every computer as a labelled group of
monitors. **Drag a computer** to where it sits relative to yours:

- Groups **snap flush** against the nearest edge so the cursor can cross.
- Use **Align** in the rail (Top / Middle / Bottom or Left / Center / Right)
  to line up two screens of different sizes with one click.
- Computers with several monitors move as one rigid group; each monitor keeps
  its real position.
- The scroll wheel zooms, dragging empty space pans, and the buttons at the
  bottom right zoom or fit the view.
- Overlapping screens are flagged in red; drag them apart.

Monitor sizes are detected automatically each time a computer connects, so
you never type resolutions.

## Daily use

1. Press **Start sharing** on every computer (or enable **Start sharing on
   launch** in Settings).
2. Push the cursor through a screen edge to move to the neighbouring computer;
   push back through the opposite edge to return.
3. Type, click, scroll and use the clipboard as usual.

The rail shows each computer's state — *Connecting…*, *Connected · 4 ms*,
*Wrong pairing code* or *Unreachable, retrying…* — and toasts announce
connections and losses. If a computer disappears (sleep, Wi-Fi drop) mouseshare
returns the cursor to you and reconnects automatically.

**Emergency return:** press **Ctrl+Alt+Esc** (configurable) at any time to
bring the cursor and keyboard back to the controller.

## Settings reference

Open **Settings** (gear icon, top right). Changes are saved automatically.
Options marked ⟳ take effect the next time you press **Start sharing**.

### General

| Setting | Meaning |
|---|---|
| Language | Automatic, English or 简体中文. Chinese needs a CJK font on the system (`fonts-noto-cjk`). |
| Theme | Auto (follows the desktop), Dark or Light. |
| Accent colour | Blue, violet, teal, green, orange or pink. |
| Interface size | 80 %–160 %. |
| Open at login | Adds an autostart entry (`~/.config/autostart/mouseshare.desktop`). |
| Start sharing on launch | Starts sharing as soon as the app opens. |

### Input

| Setting | Meaning |
|---|---|
| Return hotkey ⟳ | Pulls the cursor back to the controller. Default `Ctrl+Alt+Esc`. Format: modifiers `Ctrl`, `Alt`, `Shift`, `Super` plus one key, e.g. `Ctrl+Shift+F12`. |
| Edge delay ⟳ | How long (0–1000 ms) the cursor must press against an edge before switching. Prevents accidental hand-offs. |
| Don't switch while dragging ⟳ | Ignore edges while a mouse button is held. |
| Natural scrolling ⟳ | Reverse the wheel direction on controlled computers. |

Per device (select it in the rail): **Pointer speed** (0.25×–4×) for
computers whose mouse feels too fast or slow, the device's **address** and its
**pairing code**.

### Clipboard

| Setting | Meaning |
|---|---|
| Share clipboard text ⟳ | Plain-text clipboard sync in both directions. |
| Size limit ⟳ | Larger text is not sent (16 KB – 900 KB). |

### Security

Shows the encryption summary, this computer's own pairing code (copy or
regenerate while stopped), the listening **port** ⟳ (default `7878`) and where
settings are stored.

## Command line

`mouseshare` shares its settings with the GUI, so a setup made in the GUI can
run headless.

```text
mouseshare [--config <layout.toml>] <COMMAND>

Commands:
  controller   Run on the computer that owns the mouse and keyboard
  target       Run on a computer that should be controlled
  pair-code    Print this computer's pairing code (--new makes a new one)
```

### `mouseshare target`

```bash
mouseshare target [--listen 0.0.0.0:7878] [--name desk] [--pair-code CODE] [--no-clipboard]
```

Prints the pairing code on start. Without `--pair-code` it uses the code saved
for this computer (generated on first use).

### `mouseshare controller`

```bash
mouseshare controller \
  --connect studio=192.168.1.20:7878 \
  --pair-code studio=7F3KD-9X2MQ \
  [--hotkey "Ctrl+Alt+Esc"] [--no-clipboard]
```

- `--connect id=host:port` — address of a target (repeatable). Without it the
  target is found via mDNS.
- `--pair-code CODE` for all targets, or `--pair-code id=CODE` for one
  (repeatable). Also read from `$MOUSESHARE_PAIR_CODE`, then from the codes
  saved by the GUI.
- Without `--config`, the arrangement saved by the GUI is used. With
  `--config layout.toml` you supply the arrangement yourself — see
  [`layout.example.toml`](../layout.example.toml).

### Scripting a headless setup

```bash
# on each target
mouseshare target --name studio --pair-code 7F3KD-9X2MQ &

# on the controller
mouseshare --config ~/layout.toml controller --pair-code studio=7F3KD-9X2MQ
```

Logs go to stderr.

## Files and configuration

| File | Purpose |
|---|---|
| `~/.config/mouseshare/settings.toml` | Preferences, the device list and the arrangement. Safe to share in bug reports. |
| `~/.config/mouseshare/pairing.toml` | **Secret.** Pairing codes (file mode `0600`). Never share it. |
| `~/.config/autostart/mouseshare.desktop` | Created by *Open at login*. |

`$XDG_CONFIG_HOME` is honoured. Deleting `~/.config/mouseshare` resets
everything (including the first-run guide).

## Uninstall

```bash
sudo apt remove mouseshare          # .deb
./install.sh --uninstall            # tarball (add --system if installed with it)
rm -rf ~/.config/mouseshare ~/.config/autostart/mouseshare.desktop   # optional: settings
```
