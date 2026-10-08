# Troubleshooting & FAQ

[简体中文](TROUBLESHOOTING.zh-CN.md)

## Contents

- [Nothing is discovered when I press Add device](#nothing-is-discovered-when-i-press-add-device)
- [It says "Wrong pairing code"](#it-says-wrong-pairing-code)
- [It says "Unreachable, retrying…"](#it-says-unreachable-retrying)
- [The cursor is stuck on another computer](#the-cursor-is-stuck-on-another-computer)
- [The cursor does not cross the edge](#the-cursor-does-not-cross-the-edge)
- [Keys type the wrong characters](#keys-type-the-wrong-characters)
- [The clipboard does not sync](#the-clipboard-does-not-sync)
- [The window is blank, or the GUI will not start](#the-window-is-blank-or-the-gui-will-not-start)
- [Chinese text shows as boxes](#chinese-text-shows-as-boxes)
- [FAQ](#faq)
- [Collecting information for a bug report](#collecting-information-for-a-bug-report)

## Nothing is discovered when I press Add device

Automatic discovery uses mDNS (multicast DNS). It needs:

- the other computer to be **sharing** (*Start sharing* pressed, role
  *Controlled*);
- both computers on the same network segment — guest Wi-Fi, VLANs and some
  routers block multicast between devices ("client isolation");
- no firewall blocking UDP port `5353`.

If it still finds nothing, **type the address**: on the controlled computer the
address is shown in the main area (for example `192.168.1.20:7878`). Enter it
in the *Address* field of **Add device**. Discovery is only a convenience;
connections never depend on it.

## It says "Wrong pairing code"

The code typed on the controller does not match the one shown on the controlled
computer. Check for look-alike characters (`0`/`O`, `1`/`I`/`L` are treated as
the same, so that is fine) and that you copied the *current* code — pressing
**New code** on the controlled computer invalidates the old one. Select the
device in the controller's rail and correct its code.

After 5 wrong attempts within a minute from the same address, the controlled
computer ignores further attempts from it for a while; wait a minute and retry.

## It says "Unreachable, retrying…"

The controller cannot open a TCP connection. Check, in order:

1. The controlled computer is running and **Start sharing** is active.
2. The address and port are right (default port `7878`).
3. A firewall allows incoming TCP on that port:
   ```bash
   sudo ufw allow 7878/tcp          # ufw
   sudo firewall-cmd --add-port=7878/tcp --permanent && sudo firewall-cmd --reload   # firewalld
   ```
4. From the controller: `nc -vz <address> 7878` should say *succeeded*.

mouseshare keeps retrying with growing delays (1 s up to 15 s), so it recovers
by itself once the problem is fixed.

## The cursor is stuck on another computer

Press **Ctrl+Alt+Esc** (or your configured return hotkey) — it is handled on the
controller and always brings you back. If the other computer went to sleep or
lost the network, mouseshare notices within about 10 seconds and returns the
cursor automatically.

## The cursor does not cross the edge

- Make sure the two screens **touch** on the canvas. They snap together when
  you drag; the red "Screens overlap" notice means they intersect.
- The status next to the device must read *Connected*.
- The **Edge delay** setting (Input) requires the cursor to rest against the
  edge for that long.
- **Don't switch while dragging** ignores the edge while a mouse button is
  held — release it first.
- With different-sized screens, only the part of the edge that actually touches
  the other screen crosses. Use **Align** to line them up.

## Keys type the wrong characters

mouseshare forwards keys by X11 keycode, so both computers should use the same
keyboard layout. If they differ (for example US on one and German on the
other), the *position* of the key is preserved but the character follows the
layout of the computer being typed on. Set the same layout on both
(`setxkbmap us`) or accept that the characters follow the target's layout.

## The clipboard does not sync

- Check **Settings → Clipboard → Share clipboard text** is on, and restart
  sharing after changing it.
- Only **plain text** up to the size limit (default 256 KB) is synced; images
  and files are not.
- Some applications (certain terminals, remote-desktop windows) only publish
  the *primary selection* (middle-click paste) rather than the clipboard.

## The window is blank, or the GUI will not start

The GUI needs OpenGL (`libgl1`) and an X11 or XWayland display. In a virtual
machine or a minimal install try:

```bash
sudo apt install libgl1 libxkbcommon-x11-0 mesa-utils
LIBGL_ALWAYS_SOFTWARE=1 mouseshare-gui      # force software rendering
```

On Wayland, run `mouseshare-gui` under XWayland (the default on most desktops),
and note that mouse sharing itself needs a real X11 session.

## Chinese text shows as boxes

Install a CJK font and restart the app:

```bash
sudo apt install fonts-noto-cjk        # or fonts-wqy-microhei
```

The *简体中文* language option is only offered when such a font is found.

## FAQ

**Does it work on Wayland?** Not yet. Capturing and injecting input globally is
restricted on Wayland and needs a different backend. See the
[roadmap](ROADMAP.md). At the login screen most distributions let you choose an
"Xorg" / "X11" session instead.

**Windows or macOS?** Not yet; contributions welcome (see the roadmap).

**Is it safe on public Wi-Fi?** Every connection is authenticated with the
pairing code and encrypted, so others on the network cannot read or inject
input. Still, treat the pairing code like a password and prefer a trusted
network. Details: [Protocol & security design](PROTOCOL.md#security-design).

**Can two computers both be controllers?** A computer is either the controller
or controlled. Switch the role in the rail to swap them.

**Can I control a computer that has several monitors?** Yes — monitors are
detected automatically and appear as one group on the canvas.

**Does it use the cloud?** No. Everything happens directly between your
computers on the local network; there is no account, server or telemetry.

**Where are my settings?** `~/.config/mouseshare/` — see the
[User Guide](USER_GUIDE.md#files-and-configuration).

**Can I run it without the GUI?** Yes: the `mouseshare` command shares the same
settings; see [Command line](USER_GUIDE.md#command-line).

## Collecting information for a bug report

Please include:

- the output of `mouseshare --version` and your distribution / desktop,
- the log (open the GUI **Log** panel, or run the CLI and copy stderr),
- `~/.config/mouseshare/settings.toml` (safe to share — **never**
  `pairing.toml`),
- what you did, what you expected, and what happened.

Then open an issue using the
[bug report form](https://github.com/PelerYuan/mouseshare/issues/new/choose).
