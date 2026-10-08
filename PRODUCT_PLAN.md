# mouseshare — product plan (v0.2 "ready to use")

Goal: turn the working MVP into something a non-expert can install, pair and
use in under two minutes, that looks modern/flat/premium, and that does not
fall over on real setups (multi-monitor, flaky Wi-Fi, untrusted LAN).

## 1. Where the MVP actually stood (audit)

| Gap | Severity | Decision |
|---|---|---|
| Mouse **buttons and wheel never forwarded** — remote cursor moves but cannot click/scroll | blocker | fix first (protocol v2) |
| No authentication/encryption; mDNS broadcasts the target to the whole LAN; anyone can inject keystrokes | blocker | pairing code + PAKE + AEAD |
| One device == one rectangle (X root size) — wrong for multi-monitor | blocker for real desks | monitor model via RandR |
| Cursor does not appear at the crossing point on the remote (relative moves only) | feels broken | absolute warp on entry |
| No reconnect; a Wi-Fi blip needs manual Stop/Start | high | backoff reconnect |
| GUI forgets everything on exit; user types resolutions by hand | high | persisted settings, auto-detected monitors |
| No way out when the cursor is stuck on a dead remote | high | emergency-return hotkey |
| Docs stale (old `--connect` syntax), no packaging | high | rewrite docs, .deb, install script, release workflow |

## 2. Feature weighing

Scoring: **V**alue to a normal user, **C**ost (effort+risk), **F**it with the
current X11-only architecture. Decision = ship / defer / reject.

| Feature | V | C | F | Decision | Why |
|---|---|---|---|---|---|
| Click + scroll forwarding | 5 | 1 | 5 | **ship** | table stakes |
| Pairing code (SPAKE2) + ChaCha20-Poly1305 channel | 5 | 3 | 5 | **ship** | a low-entropy typed code must not be offline-crackable; PAKE gives that, TLS-with-certs would force a PKI/fingerprint UX |
| Auto-generated code shown on Target, typed once on Controller, remembered | 5 | 2 | 5 | **ship** | the "easy to start" path |
| Multi-monitor (RandR) for every device, rigid per-device groups on canvas | 5 | 4 | 5 | **ship** | |
| Auto-detect monitors on connect (no manual resolution typing) | 5 | 2 | 5 | **ship** | |
| Enter remote at the crossing point (absolute warp) | 4 | 1 | 5 | **ship** | |
| Reconnect with backoff + live per-device status & latency | 5 | 2 | 5 | **ship** | |
| Emergency-return hotkey (default Ctrl+Alt+Esc) | 4 | 1 | 5 | **ship** | |
| Per-device pointer speed, natural-scroll invert | 3 | 1 | 5 | **ship** | cheap, commonly needed with mixed OS/mice |
| Edge dwell delay / "don't switch while a button is held" | 3 | 2 | 4 | **ship (hold-guard + dwell)** | prevents accidental hand-offs while dragging |
| Clipboard on/off + size cap | 3 | 1 | 5 | **ship** | privacy control |
| Persisted settings (`~/.config/mouseshare`), layout import/export | 5 | 2 | 5 | **ship** | |
| First-run onboarding wizard | 5 | 3 | 5 | **ship** | the "上手即用" part |
| Themes: Dark / Light / Auto, 6 accent colours, UI scale | 3 | 2 | 5 | **ship** | modern/premium look |
| Languages: English + 简体中文 (runtime CJK font discovery) | 4 | 2 | 4 | **ship** | primary user is Chinese-speaking; zh offered only if a CJK system font exists |
| Settings window (General / Input / Clipboard / Security / About) | 4 | 3 | 5 | **ship** | |
| Toast notifications (connected / lost / copied) | 3 | 1 | 5 | **ship** | |
| Autostart on login, start-sharing-on-launch | 4 | 1 | 5 | **ship** | `.desktop` autostart file |
| .deb + tarball + install script + .desktop + icon + release workflow | 5 | 3 | 5 | **ship** | |
| System tray | 3 | 4 | 2 | defer | needs GTK/libappindicator or DBus SNI; fragile across DEs; window minimise + autostart covers 90 % |
| Wayland / Windows / macOS | 5 | 5+ | 0 | defer | whole new input backends; seams kept (`x11input` is isolated) |
| File drag & drop between machines | 3 | 5 | 1 | defer | needs clipboard-file/DnD protocol per platform |
| Image / rich clipboard | 2 | 3 | 3 | defer | |
| Per-profile layouts | 2 | 2 | 5 | defer | layout export/import covers it |
| Mouse-wheel/keyboard shortcut to jump to a specific screen | 2 | 2 | 4 | defer | |
| Remote-to-remote paths with a *third* machine's own physical input | 2 | 5 | 2 | reject | breaks the single-controller model |
| Insecure/"no auth" mode | — | — | — | **reject** | footgun; tests use a fixed pairing code instead |

## 3. Architecture changes

* `protocol` v2: `Hello{device_id, monitors}`, `MouseButton`, `Scroll`,
  `MouseWarp{x,y}`, `Ping/Pong{ts}`, `MonitorsChanged`.
* `net`: `SecureChannel` — SPAKE2(Ed25519) on the pairing code → HKDF → two
  ChaCha20-Poly1305 directions with counter nonces; first encrypted frame is
  the key confirmation. Listener rate-limits wrong codes.
* `x11input`: RandR monitor listing + change polling, XTest button
  injection, button/scroll capture folded into one ordered event stream.
* `layout`: `Device{origin, monitors[]}`; legacy `[[screens]]` TOML still
  parses (1 screen = 1 device with 1 monitor). Edge detector works on the
  *union of a device's monitors*; transitions carry the entry point.
* new `config` crate: `Settings`, device book, pairing codes (0600 file),
  layout import/export, shared by CLI and GUI.
* `core`: `SessionOptions`, event channel (status, latency, toasts),
  reconnect supervisor per target, hotkey handling, pointer scaling.
* `gui`: modules `theme`, `i18n`, `widgets`, `onboarding`, `settings_ui`,
  `canvas` (device groups), `app`.

## 4. Visual language

Flat, quiet surfaces; one accent; 4/8/12/16/24 spacing; type scale
20/14/13/11; 1 px hairlines instead of boxes; Phosphor icon font instead of
emoji; Inter typeface; rounded 8 px controls; status shown with colour **and**
text. Light and dark palettes are token tables in `theme.rs`.

## 5. Definition of done (what "ready to use" means)

1. `cargo fmt`, `clippy -D warnings`, all tests (incl. e2e with two Xvfb
   displays: move, click, scroll, key, clipboard, wrong-code rejection,
   reconnect, multi-monitor crossing) green in CI.
2. Fresh machine: install .deb → open app → pick role → pair → drag to
   arrange → Start. No config files edited by hand.
3. Settings survive restarts; layout can be exported to the CLI format.
4. README (en/zh), CHANGELOG, user guide current.

## 6. Open-source & GitHub maturity track

Audit at the start of v0.2 work: one merged PR ever, `master` unprotected,
no SECURITY policy, no CODEOWNERS, no Dependabot, no release automation, no
`docs/`, zh-CN covers README/CONTRIBUTING/CoC only, community health 85 %.

| Item | Decision |
|---|---|
| English-first docs with full zh-CN mirror (README, USER_GUIDE, ARCHITECTURE, PROTOCOL, SECURITY, TROUBLESHOOTING/FAQ, CHANGELOG, CONTRIBUTING, CoC) | ship, `*.zh-CN.md` beside each file, language switcher line at top |
| SECURITY.md (private reporting via GitHub advisories, threat model summary) | ship |
| CODEOWNERS, issue forms (bug / feature / question), PR template with checklist | ship |
| Dependabot (cargo + github-actions, weekly) | ship |
| CI: fmt, clippy, tests under Xvfb, docs build, `cargo audit`, concurrency cancel, release build; stable required checks named for branch protection | ship |
| Release workflow on `v*` tags: build, tarball + .deb + SHA256SUMS, GitHub Release from CHANGELOG | ship (workflow only; tagging is the maintainer's call) |
| Branch protection on `master`: PR required, required checks, linear history, no force-push/deletion, squash-merge only, auto-delete merged branches | apply via API once CI check names exist |
| Labels, seed issues (good first issue / help wanted roadmap: Wayland, Windows/macOS, DnD, tray) | ship |
| Contributors | cannot be invented: CONTRIBUTING + good-first-issues + "help wanted" roadmap are the on-ramp; the contributor graph fills as people contribute |
