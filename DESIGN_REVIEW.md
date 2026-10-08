# mouseshare GUI — Design Review

## 1. Overall verdict

**Partially.** This is a solid step up from a default-egui form — the card grouping, accent bar section headings, monitor-shaped drag targets, and segmented role toggle all show real design intent, and the canvas widget in particular (drop-shadow monitors, snap-to-edge, scale ruler) is more polished than most native Rust tool GUIs ever get. But it isn't yet a *system*: there are at least three unrelated button visual languages on screen at once, the card surfaces are too close in value to the panel background to read as cards, and the whole layout (one "Local screen" card + one "Remote screen" card + a canvas hard-coded to exactly two rectangles) is built for the 1-controller/1-target case specifically, not for "N devices." Before this scales to arbitrary device counts, the button/color language needs to be unified and the canvas needs an explicit scaling story.

## 2. Specific findings

**P0**
- **Three incompatible button styles coexist for the same job.** Start/Stop (solid green/red fill), Controller/Target + Scan LAN (accent-blue fill or accent-outline pill), and the Align Top/Center/Bottom buttons (plain default-gray egui buttons, `canvas.rs` — no custom fill/stroke at all) all mean "clickable action" but look like three different apps. The Align row especially reads as an unstyled leftover next to the heavily-styled canvas above it.
- **Cards barely register as cards.** `main.rs:45,90` — `panel_fill` is `rgb(27,29,33)` and `card()`'s `faint_bg_color` fill is `rgb(36,38,43)`. A 9-unit RGB delta on a dark theme is nearly invisible; in the screenshots the card boundary is only findable by squinting. The "stack of grouped panels" the code comment aims for doesn't actually read that way.
- **No multi-device story exists yet, visually or structurally.** The sidebar has literal singular sections "Local screen" / "Remote screen," and `canvas.rs` (`SCALE`, `snap_touching`, `local_rect`/`remote_rect`) is written for exactly one pair of rectangles. There is currently no placeholder for "what does a third device look like" anywhere in the UI.

**P1**
- **Truncated hostname loses information nobody can recover.** `canvas.rs:26-66` `fit_text` shortens "peler-Standard-PC-i440FX-PIIX-1996" to "peler-St…" inside the monitor box (confirmed in all three screenshots) with only a hover tooltip as fallback — on a touch/laptop trackpad workflow hover is unreliable, and there's no visible affordance (no dotted underline, no info icon) telling the user truncation happened at all.
- **Placeholder vs. real value contrast is inverted from expectation.** `main.rs:456-475`: the *label above* the connect-address field ("Connect address (blank = mDNS auto-discover)") is dimmed `weak()` gray, the *hint text inside* the field is also dim, but real typed text is forced to near-white — meaning the visual weight ordering is empty-field ≈ label < filled-field, which is a reasonable fix for one bug (per the code comment) but leaves the label and the placeholder still nearly the same gray, so at a glance you can't tell "field with a hint" from "field with no hint at all."
- **Nine distinct font sizes with no declared scale**: 20 (title), 18 (canvas heading), 14.5 (section heading), 13 (canvas id), 12 (connect label), 11.5 (log), 11 (role description), 10 (canvas caption), 9 (scale ruler), 8.5 (badge). Nothing is obviously wrong per-instance, but there's no evidence of a type ramp (e.g. a 4-step scale) driving these — they read as independently tuned.
- **Status pill is visually an afterthought.** `main.rs:566-570`: the colored dot + "idle"/"running" label is jammed directly beside the Start/Stop button inside the same card with only 4px of spacing, no separating rule or alignment — it looks bolted on rather than designed as a status component.

**P2**
- Canvas monitor boxes are small (≈72×56px at the default 900×700 test resolution) and must fit three lines of content (id, resolution, YOU/role badge) — legible today only because the test hostnames happen to abbreviate reasonably; a longer real screen_id plus a 4-digit-vs-4-digit resolution string is close to the wrapping limit.
- DragValue number fields (`900`, `700`) are visually identical to plain disabled-looking gray boxes — no affordance distinguishing "click-drag to change" from static/disabled text, discoverable only by accident.
- Icon-to-label spacing in buttons (`"🖱  Controller"`, `"🔍  Scan LAN"`, `"▶  Start"`) is done with a literal double-space character baked into the string rather than a layout gap — fragile as more icons get added, and glyph width varies by emoji so the visual gap isn't actually consistent across buttons.

## 3. Design standard going forward

1. **Spacing scale**: standardize on a 4/8/12/16/24 px scale (already close: `item_spacing` 8×6, `button_padding` 10×5, `window_margin` 12 — round these onto the scale exactly, e.g. margin 16 not 12).
2. **Card contrast**: raise card fill to at least `rgb(42,45,51)` or add a 1px stroke (`rgba(255,255,255,20)`) so a card is identifiable without hover/focus.
3. **One accent color, one meaning**: the blue accent (`rgb(90,130,190)`) means "selected/primary interactive." Don't introduce a second unrelated blue for something else.
4. **One button system, three states**: primary (solid accent or semantic green/red fill, used for Start/Stop/segmented-selected), secondary (accent-outline pill, used for Scan/Align/any non-destructive action), tertiary (text-only). Migrate the Align buttons to secondary style immediately — no more default-gray buttons anywhere in this app.
5. **Type scale**: collapse to 4 sizes — Title (20), Section heading (14.5), Body (13), Caption/meta (11). Retire the 18/12/10/9/8.5 one-offs; the canvas heading becomes 20 or 14.5, not its own 18.
6. **Iconography rule**: only use glyphs confirmed to render in egui's default font — ASCII, common punctuation/bullets, geometric shapes (▶ ■ ● ▲), and full pictographic emoji (🖱 🖥 🔍 🔗 ⚙). Never use Unicode arrows (→ ⇄) or dingbat/symbol-block characters outside that set — verify any new icon in a running build before merging, since tofu boxes are silent failures.
7. **Icon+label layout as a real component**: a shared `icon_button(icon, label)` helper with a fixed layout gap (e.g. `ui.spacing()`-driven, not literal spaces in the string).
8. **Badges** (like "YOU"): reserve for identity/ownership markers only, one visual treatment (dark pill, white text) — don't let other metadata adopt the same shape or it stops meaning "this is you."
9. **Truncation must be visibly a truncation**: add a subtle underline-dot or a small ⓘ, not tooltip-only, wherever `fit_text` shortens a label.
10. **Status pill as its own mini-component**: own row or right-aligned in the card with ≥12px separation from the Start/Stop button, not inline-adjacent.
11. **Multi-device scaling — this needs explicit design work before the N-device feature lands**:
    - The sidebar's singular "Local screen"/"Remote screen" cards become a **scrollable device list** (one compact row per device: id, resolution, role badge, remove button), with a card only for the *currently selected* device's detail editor.
    - The canvas needs **pan + zoom** once fixed `SCALE=0.08` stops fitting everything — add scroll-to-zoom and drag-to-pan on empty canvas space, plus a "fit all" reset button, before adding a third box.
    - Snapping logic (`snap_touching`) is pairwise (local vs. one remote); extend it to snap against *any* existing edge, and show a live conflict/overlap indicator when a new device's box would overlap another.
    - At >4 devices, id+resolution text inside each ~70px monitor box won't fit — plan a "compact mode" (color + short id only, full detail on hover/click) that kicks in past a device-count threshold, rather than shrinking font indefinitely.
    - Decide now what the empty/loading states are (no devices yet, one device pending connection, a device that dropped) — none of these exist today even for two devices.

## 4. Immediate recommended fixes (top priority)

1. Restyle the Align Top/Center/Bottom buttons to match the accent-outline pill language already used for Scan LAN — the single most visible inconsistency in the whole UI.
2. Raise `card()`'s fill contrast against `panel_fill` (or add a stroke) so cards are visible without close inspection.
3. Collapse the font-size list to the 4-step scale in item 5 above and apply it uniformly.
4. Add a visible (non-hover-only) truncation indicator in `canvas.rs`'s `fit_text` path.
5. Before writing any multi-device code, prototype the canvas at 4-5 boxes to find where the current fixed-scale, no-pan/zoom approach breaks, and design pan/zoom + compact mode first.
