//! The arrangement canvas: every device is drawn as a rigid group of its
//! monitors, to scale, and can be dragged around the shared virtual desktop.
//! Dropping a device snaps it flush against the nearest other device so the
//! cursor can always cross between them (a gap would leave it stuck at the
//! wall; see `mouseshare_layout`).
//!
//! Model space = virtual-desktop pixels. Pan/zoom only change the model ->
//! screen transform, never the stored positions.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, Ui,
    Vec2,
};
use mouseshare_config::{DeviceRecord, MonitorRecord};

use crate::i18n::Tr;
use crate::theme::Theme;

const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 1.0;

/// Identity colours for remote devices (the local one uses the accent).
const DEVICE_HUES: [(u8, u8, u8); 6] = [
    (150, 110, 230),
    (45, 190, 175),
    (240, 150, 70),
    (235, 100, 150),
    (90, 160, 240),
    (170, 190, 70),
];

pub fn device_color(index: usize) -> Color32 {
    let (r, g, b) = DEVICE_HUES[index % DEVICE_HUES.len()];
    Color32::from_rgb(r, g, b)
}

/// 1920x1080 stand-in until a device reports its real monitors.
fn placeholder() -> Vec<MonitorRecord> {
    vec![MonitorRecord {
        name: String::new(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }]
}

pub fn monitors_of(d: &DeviceRecord) -> Vec<MonitorRecord> {
    if d.monitors.is_empty() {
        placeholder()
    } else {
        d.monitors.clone()
    }
}

/// Bounding box of a device's monitors in device-local pixels: (x, y, w, h).
pub fn local_bounds(d: &DeviceRecord) -> (i32, i32, i32, i32) {
    let ms = monitors_of(d);
    let x0 = ms.iter().map(|m| m.x).min().unwrap_or(0);
    let y0 = ms.iter().map(|m| m.y).min().unwrap_or(0);
    let x1 = ms.iter().map(|m| m.x + m.width).max().unwrap_or(0);
    let y1 = ms.iter().map(|m| m.y + m.height).max().unwrap_or(0);
    (x0, y0, x1 - x0, y1 - y0)
}

/// Device bounding box in virtual-desktop pixels.
fn bbox(d: &DeviceRecord) -> Rect {
    let (x, y, w, h) = local_bounds(d);
    Rect::from_min_size(
        Pos2::new((d.x + x) as f32, (d.y + y) as f32),
        Vec2::new(w as f32, h as f32),
    )
}

fn monitor_rects(d: &DeviceRecord) -> Vec<Rect> {
    monitors_of(d)
        .iter()
        .map(|m| {
            Rect::from_min_size(
                Pos2::new((d.x + m.x) as f32, (d.y + m.y) as f32),
                Vec2::new(m.width as f32, m.height as f32),
            )
        })
        .collect()
}

fn interior_overlap(a: Rect, b: Rect) -> bool {
    let i = a.intersect(b);
    i.width() > 1.0 && i.height() > 1.0
}

/// Whether any monitor of one device overlaps any monitor of another.
pub fn any_overlap(devices: &[DeviceRecord]) -> bool {
    devices.iter().enumerate().any(|(i, a)| {
        devices.iter().skip(i + 1).any(|b| {
            let (ra, rb) = (monitor_rects(a), monitor_rects(b));
            ra.iter()
                .any(|x| rb.iter().any(|y| interior_overlap(*x, *y)))
        })
    })
}

/// Default origin for a newly added device: flush right of everything.
pub fn default_origin(devices: &[DeviceRecord], new: &DeviceRecord) -> (i32, i32) {
    let right = devices
        .iter()
        .map(|d| bbox(d).right())
        .fold(f32::MIN, f32::max);
    let top = devices.first().map(|d| bbox(d).top()).unwrap_or(0.0);
    let (lx, ly, _, _) = local_bounds(new);
    if right == f32::MIN {
        (0, 0)
    } else {
        (right as i32 - lx, top as i32 - ly)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SnapAxis {
    /// Side by side: align top / centre / bottom.
    Horizontal,
    /// Stacked: align left / centre / right.
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
}

/// Snaps `desired` flush against whichever side of `anchor` it is nearest,
/// keeping a real overlap along the shared edge (never just a corner).
fn snap_touching(anchor: Rect, desired: Rect) -> (Pos2, SnapAxis) {
    let dx = desired.center().x - anchor.center().x;
    let dy = desired.center().y - anchor.center().y;
    let min_overlap = 150.0_f32.min(desired.width().min(desired.height()) * 0.3);
    if dx.abs() >= dy.abs() {
        let x = if dx >= 0.0 {
            anchor.right()
        } else {
            anchor.left() - desired.width()
        };
        let y = desired.min.y.clamp(
            anchor.top() - desired.height() + min_overlap,
            anchor.bottom() - min_overlap,
        );
        (Pos2::new(x, y), SnapAxis::Horizontal)
    } else {
        let y = if dy >= 0.0 {
            anchor.bottom()
        } else {
            anchor.top() - desired.height()
        };
        let x = desired.min.x.clamp(
            anchor.left() - desired.width() + min_overlap,
            anchor.right() - min_overlap,
        );
        (Pos2::new(x, y), SnapAxis::Vertical)
    }
}

#[derive(Clone, Copy)]
struct View {
    scale: f32,
    /// Model point shown at the canvas centre.
    center: Vec2,
}

impl View {
    fn to_screen(self, canvas: Rect, p: Pos2) -> Pos2 {
        canvas.center() + (p.to_vec2() - self.center) * self.scale
    }

    fn rect_to_screen(self, canvas: Rect, r: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(canvas, r.min), self.to_screen(canvas, r.max))
    }
}

/// Per-device live info the canvas needs but doesn't own.
pub struct DeviceVisual {
    pub status_color: Color32,
    pub status_text: String,
}

pub struct CanvasInput<'a> {
    pub th: &'a Theme,
    pub tr: &'a Tr,
    pub local_id: &'a str,
    pub selected: Option<&'a str>,
    /// Device currently holding the cursor (`None` = this computer).
    pub active: Option<&'a str>,
    pub running: bool,
    /// Dragging allowed (not while a session is running).
    pub editable: bool,
    pub visual: &'a dyn Fn(&str) -> DeviceVisual,
}

pub struct CanvasOutput {
    pub clicked: Option<String>,
}

struct Drag {
    id: String,
    /// Unsnapped top-left of the device bounding box, model space.
    pos: Pos2,
}

#[derive(Default)]
pub struct CanvasState {
    view: Option<View>,
    signature: u64,
    drag: Option<Drag>,
    anchors: HashMap<String, (Rect, SnapAxis)>,
}

impl CanvasState {
    pub fn axis(&self, id: &str) -> Option<SnapAxis> {
        self.anchors.get(id).map(|a| a.1)
    }

    /// One-click alignment of `id` along the edge it touches.
    pub fn align(&mut self, devices: &mut [DeviceRecord], id: &str, a: Alignment) {
        let Some((anchor, axis)) = self.anchors.get(id).copied() else {
            return;
        };
        let Some(d) = devices.iter_mut().find(|d| d.id == id) else {
            return;
        };
        let b = bbox(d);
        let (lx, ly, _, _) = local_bounds(d);
        match axis {
            SnapAxis::Horizontal => {
                let y = match a {
                    Alignment::Start => anchor.top(),
                    Alignment::Center => anchor.center().y - b.height() / 2.0,
                    Alignment::End => anchor.bottom() - b.height(),
                };
                d.y = y.round() as i32 - ly;
            }
            SnapAxis::Vertical => {
                let x = match a {
                    Alignment::Start => anchor.left(),
                    Alignment::Center => anchor.center().x - b.width() / 2.0,
                    Alignment::End => anchor.right() - b.width(),
                };
                d.x = x.round() as i32 - lx;
            }
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        devices: &mut [DeviceRecord],
        inp: &CanvasInput,
    ) -> CanvasOutput {
        let th = inp.th;
        let size = ui.available_size();
        let (canvas, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 12.0, th.p.canvas);
        paint_dots(&painter, canvas, th);

        // Re-fit when the set of devices/monitors changes, not on drags.
        let mut h = DefaultHasher::new();
        for d in devices.iter() {
            d.id.hash(&mut h);
            for m in monitors_of(d) {
                (m.x, m.y, m.width, m.height).hash(&mut h);
            }
        }
        let sig = h.finish();
        if sig != self.signature {
            self.signature = sig;
            self.view = None;
        }
        let mut view = *self.view.get_or_insert_with(|| fit_view(canvas, devices));

        // Pan with scroll, zoom around the pointer with ctrl+scroll.
        if ui.rect_contains_pointer(canvas) {
            let (scroll, mods, pointer) =
                ui.input(|i| (i.smooth_scroll_delta, i.modifiers, i.pointer.hover_pos()));
            if scroll != Vec2::ZERO {
                if mods.ctrl || mods.command {
                    let factor = (1.0 + scroll.y * 0.003).clamp(0.5, 2.0);
                    let new_scale = (view.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
                    if let Some(p) = pointer {
                        let model = view.center + (p - canvas.center()) / view.scale;
                        view.center = model - (p - canvas.center()) / new_scale;
                    }
                    view.scale = new_scale;
                } else {
                    view.center -= scroll / view.scale;
                }
            }
        }

        // ---- interaction -------------------------------------------------
        let mut clicked = None;
        let ids: Vec<String> = devices.iter().map(|d| d.id.clone()).collect();
        let mut drag_started: Option<(String, Pos2)> = None;
        let mut drag_delta: Option<Vec2> = None;
        let mut drag_stopped = false;
        let mut hovering: Option<String> = None;

        // Topmost-last: iterate in order, later devices win hit tests.
        for (i, id) in ids.iter().enumerate() {
            let model_box = match &self.drag {
                Some(dr) if dr.id == *id => Rect::from_min_size(dr.pos, bbox(&devices[i]).size()),
                _ => bbox(&devices[i]),
            };
            let rect = view.rect_to_screen(canvas, model_box);
            let resp: Response = ui.interact(
                rect,
                ui.id().with(("device", id.as_str())),
                if inp.editable {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                },
            );
            if resp.clicked() {
                clicked = Some(id.clone());
            }
            if resp.hovered() {
                hovering = Some(id.clone());
            }
            if resp.drag_started() {
                drag_started = Some((id.clone(), bbox(&devices[i]).min));
                clicked = Some(id.clone());
            }
            if resp.dragged() {
                drag_delta = Some(resp.drag_delta() / view.scale);
            }
            if resp.drag_stopped() {
                drag_stopped = true;
            }
        }
        if let Some((id, pos)) = drag_started {
            self.drag = Some(Drag { id, pos });
        }
        if let (Some(dr), Some(delta)) = (self.drag.as_mut(), drag_delta) {
            dr.pos += delta;
        }
        // Snap the dragged device (continuously, so the user sees where it
        // will land) and write the result back to the record.
        let mut snapped_overlap_id: Option<String> = None;
        if let Some(dr) = &self.drag {
            if let Some(idx) = devices.iter().position(|d| d.id == dr.id) {
                let desired = Rect::from_min_size(dr.pos, bbox(&devices[idx]).size());
                let mut best: Option<(f32, Pos2, SnapAxis, Rect)> = None;
                for (j, other) in devices.iter().enumerate() {
                    if j == idx {
                        continue;
                    }
                    let ob = bbox(other);
                    let (p, axis) = snap_touching(ob, desired);
                    let dist = (p - desired.min).length();
                    if best.as_ref().is_none_or(|b| dist < b.0) {
                        best = Some((dist, p, axis, ob));
                    }
                }
                if let Some((_, p, axis, anchor)) = best {
                    let (lx, ly, _, _) = local_bounds(&devices[idx]);
                    devices[idx].x = p.x.round() as i32 - lx;
                    devices[idx].y = p.y.round() as i32 - ly;
                    self.anchors.insert(dr.id.clone(), (anchor, axis));
                    snapped_overlap_id = Some(dr.id.clone());
                }
            }
        }
        if drag_stopped {
            self.drag = None;
        }
        let _ = snapped_overlap_id;
        if hovering.is_some() && inp.editable {
            ui.ctx().set_cursor_icon(if self.drag.is_some() {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
        }

        // Keep anchors fresh for non-dragged devices too (for Align buttons).
        if self.drag.is_none() {
            self.refresh_anchors(devices);
        }

        // ---- painting ----------------------------------------------------
        let overlap = any_overlap(devices);
        let mut remote_index = 0usize;
        for d in devices.iter() {
            let is_local = d.id == inp.local_id;
            let color = if is_local {
                th.accent
            } else {
                let c = device_color(remote_index);
                remote_index += 1;
                c
            };
            let selected = inp.selected == Some(d.id.as_str());
            let active = match inp.active {
                Some(a) => a == d.id,
                None => is_local && inp.running,
            };
            let vis = (inp.visual)(&d.id);
            paint_device(
                &painter, &view, canvas, d, color, is_local, selected, active, &vis, inp,
            );
        }

        if overlap {
            let msg = inp.tr.t("canvas_overlap");
            let galley =
                painter.layout_no_wrap(msg.to_string(), FontId::proportional(12.5), th.p.danger);
            let pill = Rect::from_center_size(
                canvas.center_top() + Vec2::new(0.0, 22.0),
                galley.size() + Vec2::new(20.0, 10.0),
            );
            painter.rect_filled(pill, 99.0, th.p.danger.gamma_multiply(0.18));
            painter.galley(pill.center() - galley.size() / 2.0, galley, th.p.danger);
        }

        self.zoom_controls(ui, canvas, devices, inp, &mut view);
        self.view = Some(view);
        CanvasOutput { clicked }
    }

    /// Recomputes which neighbour each device is touching, for Align.
    fn refresh_anchors(&mut self, devices: &[DeviceRecord]) {
        self.anchors
            .retain(|id, _| devices.iter().any(|d| &d.id == id));
        for d in devices {
            let b = bbox(d);
            let mut best: Option<(f32, Rect, SnapAxis)> = None;
            for o in devices.iter().filter(|o| o.id != d.id) {
                let ob = bbox(o);
                let (p, axis) = snap_touching(ob, b);
                let dist = (p - b.min).length();
                if best.as_ref().is_none_or(|x| dist < x.0) {
                    best = Some((dist, ob, axis));
                }
            }
            if let Some((_, anchor, axis)) = best {
                self.anchors.insert(d.id.clone(), (anchor, axis));
            }
        }
    }

    fn zoom_controls(
        &mut self,
        ui: &mut Ui,
        canvas: Rect,
        devices: &[DeviceRecord],
        inp: &CanvasInput,
        view: &mut View,
    ) {
        let th = inp.th;
        let w = 124.0;
        let rect = Rect::from_min_size(
            canvas.right_bottom() - Vec2::new(w + 12.0, 36.0 + 12.0),
            Vec2::new(w, 36.0),
        );
        ui.painter().rect_filled(rect, 10.0, th.p.surface);
        ui.painter()
            .rect_stroke(rect, 10.0, Stroke::new(1.0_f32, th.p.border));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(4.0, 3.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 2.0;
        use egui_phosphor::regular as ph;
        if crate::widgets::icon_button(ui, th, ph::MINUS, inp.tr.t("zoom_out")).clicked() {
            view.scale = (view.scale / 1.25).clamp(MIN_SCALE, MAX_SCALE);
        }
        if crate::widgets::icon_button(ui, th, ph::PLUS, inp.tr.t("zoom_in")).clicked() {
            view.scale = (view.scale * 1.25).clamp(MIN_SCALE, MAX_SCALE);
        }
        if crate::widgets::icon_button(ui, th, ph::CORNERS_OUT, inp.tr.t("zoom_fit")).clicked() {
            *view = fit_view(canvas, devices);
        }
    }
}

fn fit_view(canvas: Rect, devices: &[DeviceRecord]) -> View {
    let mut union: Option<Rect> = None;
    for d in devices {
        let b = bbox(d);
        union = Some(union.map_or(b, |u| u.union(b)));
    }
    let union = union.unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0)));
    let avail = Vec2::new(
        (canvas.width() - 120.0).max(60.0),
        (canvas.height() - 150.0).max(60.0),
    );
    let scale = (avail.x / union.width().max(1.0))
        .min(avail.y / union.height().max(1.0))
        .clamp(MIN_SCALE, 0.25);
    View {
        scale,
        center: union.center().to_vec2(),
    }
}

fn paint_dots(painter: &Painter, canvas: Rect, th: &Theme) {
    let step = 24.0;
    let mut x = canvas.left() + step / 2.0;
    while x < canvas.right() {
        let mut y = canvas.top() + step / 2.0;
        while y < canvas.bottom() {
            painter.circle_filled(Pos2::new(x, y), 1.0, th.p.canvas_dot);
            y += step;
        }
        x += step;
    }
}

/// Shortens `text` with an ellipsis to fit `max_width`.
pub fn fit_text(painter: &Painter, text: &str, font: &FontId, max_width: f32) -> String {
    let w = |s: &str| {
        painter
            .layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if w(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let cand: String = chars[..mid].iter().collect::<String>() + "\u{2026}";
        if w(&cand) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let prefix: String = chars[..lo].iter().collect();
    format!("{prefix}\u{2026}")
}

#[allow(clippy::too_many_arguments)]
fn paint_device(
    painter: &Painter,
    view: &View,
    canvas: Rect,
    d: &DeviceRecord,
    color: Color32,
    is_local: bool,
    selected: bool,
    active: bool,
    vis: &DeviceVisual,
    inp: &CanvasInput,
) {
    let th = inp.th;
    let ms = monitors_of(d);
    // While dragging, `d` already carries the snapped position.
    let rects: Vec<Rect> = monitor_rects(d)
        .into_iter()
        .map(|r| view.rect_to_screen(canvas, r))
        .collect();
    let group = rects.iter().skip(1).fold(rects[0], |acc, r| acc.union(*r));

    if active {
        painter.rect_stroke(
            group.expand(6.0),
            12.0,
            Stroke::new(2.0_f32, th.accent.gamma_multiply(0.9)),
        );
    } else if selected {
        painter.rect_stroke(
            group.expand(5.0),
            11.0,
            Stroke::new(1.5_f32, th.p.text_dim.gamma_multiply(0.8)),
        );
    }

    let fill = color.gamma_multiply(if th.dark { 0.88 } else { 0.95 });
    for (i, r) in rects.iter().enumerate() {
        painter.rect_filled(*r, 6.0, fill);
        painter.rect_stroke(*r, 6.0, Stroke::new(1.0_f32, Color32::from_white_alpha(70)));
        if r.width() > 60.0 && r.height() > 40.0 {
            let m = &ms[i];
            if ms.len() > 1 {
                painter.text(
                    r.left_top() + Vec2::new(8.0, 6.0),
                    Align2::LEFT_TOP,
                    format!("{}", i + 1),
                    FontId::proportional(13.0),
                    Color32::from_white_alpha(200),
                );
            }
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("{}\u{00d7}{}", m.width, m.height),
                FontId::proportional((r.height() * 0.14).clamp(10.0, 15.0)),
                Color32::from_white_alpha(215),
            );
        }
    }

    // Name chip under the group: dot + name (+ YOU).
    let name_font = FontId::proportional(13.0);
    let max_w = (group.width() + 80.0).max(120.0);
    let name = fit_text(painter, &d.id, &name_font, max_w - 60.0);
    let mut label = name;
    if is_local {
        label = format!("{label}  \u{00b7}  {}", inp.tr.t("you"));
    }
    let galley = painter.layout_no_wrap(label, name_font, th.p.text);
    let pad = Vec2::new(10.0, 5.0);
    let chip_size = galley.size() + pad * 2.0 + Vec2::new(14.0, 0.0);
    let chip = Rect::from_center_size(
        Pos2::new(group.center().x, group.bottom() + 20.0 + chip_size.y / 2.0),
        chip_size,
    );
    painter.rect_filled(chip, 99.0, th.p.surface);
    painter.rect_stroke(chip, 99.0, Stroke::new(1.0_f32, th.p.border));
    painter.circle_filled(
        Pos2::new(chip.left() + pad.x + 3.0, chip.center().y),
        4.0,
        vis.status_color,
    );
    painter.galley(
        Pos2::new(
            chip.left() + pad.x + 14.0,
            chip.center().y - galley.size().y / 2.0,
        ),
        galley,
        th.p.text,
    );
    if !vis.status_text.is_empty() {
        painter.text(
            Pos2::new(group.center().x, chip.bottom() + 4.0),
            Align2::CENTER_TOP,
            &vis.status_text,
            FontId::proportional(11.5),
            th.p.text_dim,
        );
    }
}
