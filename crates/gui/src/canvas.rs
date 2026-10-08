//! The draggable screen-arrangement widget. Dragging any remote screen box
//! always snaps it flush against whichever side of the local screen it's
//! nearest to, with a clamped perpendicular offset -- this guarantees the
//! resulting `ScreenRect`s always touch (no gap, which would leave the
//! cursor stuck at the wall instead of handing off -- see
//! `mouseshare_layout`'s docs) while still feeling like a real drag, not a
//! multiple-choice form.
//!
//! Supports an arbitrary number of remote screens: each is tracked, dragged,
//! and snapped independently against the local screen. Snapping is still
//! pairwise against the local screen only (not against other remotes), so
//! two remotes can end up overlapping each other -- when that happens both
//! boxes get a red outline and the canvas shows a hint, rather than
//! resolving it silently (silently placing two screens on top of each other
//! would make edge-crossing to either one ambiguous).
//!
//! All screen positions are tracked in *model space*: real screen pixels,
//! with the local screen always centered at model-space origin. Model space
//! never changes when the user pans or zooms -- only the transform from
//! model space to canvas (screen) space does. This is what lets zoom be a
//! pure rendering-time concern instead of something that has to rewrite
//! every stored position whenever it changes.

use std::collections::{HashMap, HashSet};

use eframe::egui;

/// Model pixels -> canvas points at zoom = 1.0. Chosen so a pair of
/// 1920x1080 screens comfortably fits the panel without scrolling before the
/// user ever touches zoom.
const BASE_SCALE: f32 = 0.08;

/// Zoom multiplier applied on top of `BASE_SCALE`. Kept in a sane range so
/// "zoom" can't shrink the arrangement into a single pixel or blow it up
/// past the point the grid/labels still mean anything.
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 3.0;
const ZOOM_STEP: f32 = 0.15;

/// Horizontal breathing room (in canvas points) reserved inside a screen box
/// when fitting its label -- keeps truncated text from touching the border.
const LABEL_PADDING: f32 = 10.0;

/// Below this box width there isn't room for id text + caption to stay
/// legible (DESIGN_REVIEW standard #11's compact-mode threshold, pinned to a
/// box-size trigger rather than a device-count one now that zoom exists --
/// device count no longer tracks box size 1:1). Below it a box shows only a
/// centered monogram, nothing else.
const COMPACT_WIDTH_THRESHOLD: f32 = 48.0;

const LOCAL_COLOR: egui::Color32 = egui::Color32::from_rgb(70, 130, 180);
const DRAGGING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 160, 70);
const OVERLAP_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 90, 90);

/// Cycled through by a remote screen's position in the list, so each device
/// reads as a distinct box at a glance without the caller having to assign
/// colors itself. Only the first 4 devices get a distinct color -- past
/// that, color stops being asked to carry identity (it would just repeat
/// and imply a grouping that doesn't exist); those boxes get a neutral fill
/// instead and rely on the overflow monogram badge for identity.
const REMOTE_PALETTE: [egui::Color32; 4] = [
    egui::Color32::from_rgb(150, 110, 200), // purple
    egui::Color32::from_rgb(90, 170, 160),  // teal
    egui::Color32::from_rgb(200, 130, 90),  // amber
    egui::Color32::from_rgb(190, 100, 140), // rose
];

/// Fill used for the 5th+ remote screen, once the palette above has run out.
const OVERFLOW_FILL: egui::Color32 = egui::Color32::from_rgb(52, 55, 61);

/// Lays out `text` at `font_id` and, if it's wider than `max_width`,
/// progressively drops trailing characters and appends an ellipsis until it
/// fits. This is what keeps a long hostname like
/// "peler-Standard-PC-i440FX-PIIX-1996" from spilling out past its screen
/// box: no matter how long the id is, the rendered label is guaranteed to
/// fit inside `max_width`. Returns the text actually used alongside whether
/// it was shortened, so callers can offer the full string as a tooltip *and*
/// draw a visible truncation marker -- a tooltip alone is easy to miss
/// (DESIGN_REVIEW standard #9), so truncation must also be visible without
/// hovering.
pub(crate) fn fit_text(
    painter: &egui::Painter,
    text: &str,
    font_id: egui::FontId,
    max_width: f32,
) -> (String, bool) {
    let max_width = max_width.max(0.0);
    let full_width = painter
        .layout_no_wrap(text.to_string(), font_id.clone(), egui::Color32::WHITE)
        .size()
        .x;
    if full_width <= max_width {
        return (text.to_string(), false);
    }

    let ellipsis = "\u{2026}"; // "…"
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + ellipsis;
        let width = painter
            .layout_no_wrap(candidate, font_id.clone(), egui::Color32::WHITE)
            .size()
            .x;
        if width <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    let shortened = if lo == 0 {
        ellipsis.to_string()
    } else {
        let prefix: String = chars[..lo].iter().collect();
        format!("{prefix}{ellipsis}")
    };
    (shortened, true)
}

/// Draws a subtle dotted underline just below `text_rect`, the visible
/// truncation affordance DESIGN_REVIEW standard #9 asks for -- a tooltip
/// alone is easy to never discover, this is visible every time you look at
/// the box, no hover required.
pub(crate) fn paint_truncation_marker(painter: &egui::Painter, text_rect: egui::Rect) {
    let y = text_rect.bottom() + 1.0;
    let dot_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 150);
    let mut x = text_rect.left();
    while x < text_rect.right() {
        painter.circle_filled(egui::pos2(x, y), 0.6, dot_color);
        x += 3.0;
    }
}

/// Which side a remote screen is currently snapped to, in terms of the axis
/// a one-click "align" action would adjust: side-by-side screens can only
/// usefully be aligned top/center/bottom (their shared edge is vertical),
/// stacked screens only left/center/right.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SnapAxis {
    Horizontal,
    Vertical,
}

/// A one-click alignment along the touching edge, for lining up two screens
/// of different resolution without needing pixel-perfect dragging -- the
/// same "align top/center/bottom" convenience real OS display-arrangement
/// dialogs offer for mismatched monitors.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
}

pub struct CanvasState {
    /// Each remote's user-dragged (unsnapped) top-left position, in *model*
    /// coordinates (real screen pixels, local screen centered at the
    /// origin) -- never touched by zoom/pan, so changing either doesn't
    /// move anything. The rendered/reported position is always the
    /// *snapped* projection of this.
    positions: HashMap<String, egui::Pos2>,
    /// Cached from the previous frame's layout, so the alignment buttons
    /// (drawn by the caller after this widget, since they need to know a
    /// remote's current snap side) can recompute a remote's desired
    /// position without this widget having to expose its internal rect math
    /// to the caller. Both in model space.
    last_local_rect: egui::Rect,
    last_axis: HashMap<String, SnapAxis>,
    last_remote_dims: HashMap<String, egui::Vec2>,
    /// User-controlled view transform. `zoom` multiplies `BASE_SCALE`; `pan`
    /// is a canvas-point offset added on top of centering the view on the
    /// local screen.
    zoom: f32,
    pan: egui::Vec2,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            positions: HashMap::new(),
            last_local_rect: egui::Rect::NOTHING,
            last_axis: HashMap::new(),
            last_remote_dims: HashMap::new(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

/// The two font sizes every screen box is drawn with, bundled together so
/// `draw_screen_box` doesn't need two separate `FontId` parameters.
struct ScreenFonts {
    id_font: egui::FontId,
    caption_font: egui::FontId,
}

/// Where a remote screen ended up relative to the local screen's origin, in
/// real pixels -- directly usable as a `ScreenRect`'s `(x, y)` once the
/// local screen is placed at `(0, 0)`.
pub struct RemoteOffset {
    pub dx: i32,
    pub dy: i32,
}

/// What happened this frame: every remote's current placement, plus which
/// one (if any) the user just clicked -- callers use the latter to move the
/// sidebar's "selected device" detail editor to match what was clicked on
/// the canvas.
pub struct CanvasOutput {
    pub offsets: HashMap<String, RemoteOffset>,
    pub clicked_remote: Option<String>,
    pub any_overlap: bool,
}

/// Converts between model space (real screen pixels, local screen centered
/// at the origin) and canvas/screen space (the actual painted pixels),
/// given the current pan/zoom. Kept as a small value type so `ui()` doesn't
/// have to thread `(canvas_rect, zoom, pan)` through every helper
/// individually.
#[derive(Clone, Copy)]
struct ViewTransform {
    origin: egui::Pos2,
    scale: f32,
}

impl ViewTransform {
    fn to_screen_pos(self, model: egui::Pos2) -> egui::Pos2 {
        self.origin + model.to_vec2() * self.scale
    }

    fn to_screen_vec(self, model: egui::Vec2) -> egui::Vec2 {
        model * self.scale
    }

    fn to_screen_rect(self, model: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_size(
            self.to_screen_pos(model.min),
            self.to_screen_vec(model.size()),
        )
    }

    fn to_model_vec(self, screen: egui::Vec2) -> egui::Vec2 {
        screen / self.scale
    }
}

impl CanvasState {
    /// `remotes` is `(screen_id, (width, height))` for every currently
    /// configured remote device, in stable order (used for default
    /// placement and palette color assignment on first appearance).
    /// `selected_id` draws a highlight ring around that remote, if any.
    /// Fills all available space in `ui` -- callers that want a floor on
    /// how small this can get should enforce it on the window itself
    /// (there's no useful way to keep 6 boxes legible below a real pixel
    /// floor, zoom can't create pixels that aren't there).
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        local_id: &str,
        local_size: (i32, i32),
        remotes: &[(String, (i32, i32))],
        selected_id: Option<&str>,
    ) -> CanvasOutput {
        let canvas_size = ui.available_size();
        let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 6.0, ui.visuals().extreme_bg_color);

        // Drop tracked state for devices that were removed, so a re-added
        // device with the same id later doesn't inherit a stale position.
        let current_ids: HashSet<&str> = remotes.iter().map(|(id, _)| id.as_str()).collect();
        self.positions
            .retain(|id, _| current_ids.contains(id.as_str()));
        self.last_axis
            .retain(|id, _| current_ids.contains(id.as_str()));
        self.last_remote_dims
            .retain(|id, _| current_ids.contains(id.as_str()));

        // Pan via plain scroll, zoom via ctrl+scroll, only while the
        // pointer is actually over the canvas -- deliberately not
        // drag-to-pan, since left-drag already means "move this screen box"
        // and overloading the same gesture for the canvas background would
        // make the two impossible to tell apart at a glance.
        if ui.rect_contains_pointer(canvas_rect) {
            let (scroll, modifiers) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers));
            if scroll != egui::Vec2::ZERO {
                if modifiers.ctrl || modifiers.command {
                    self.zoom = (self.zoom + scroll.y * 0.002).clamp(MIN_ZOOM, MAX_ZOOM);
                } else {
                    self.pan += scroll;
                }
            }
        }

        let local_dims_model = egui::vec2(local_size.0 as f32, local_size.1 as f32);
        let local_rect_model = egui::Rect::from_center_size(egui::Pos2::ZERO, local_dims_model);
        self.last_local_rect = local_rect_model;

        let view = ViewTransform {
            origin: canvas_rect.center() + self.pan,
            scale: BASE_SCALE * self.zoom,
        };

        paint_grid(&painter, canvas_rect, &view);
        paint_scale_indicator(&painter, canvas_rect, &view);

        let local_rect = view.to_screen_rect(local_rect_model);

        let fonts = ScreenFonts {
            id_font: egui::FontId::proportional(13.0),
            caption_font: egui::FontId::proportional(11.0),
        };

        draw_screen_box(
            &painter,
            local_rect,
            local_id,
            local_size,
            LOCAL_COLOR,
            &fonts,
            BadgeSpec::Identity("YOU"),
        );
        let local_hover_id = ui.id().with("local_screen_hover");
        let local_response = ui.interact(local_rect, local_hover_id, egui::Sense::hover());
        if local_response.hovered() {
            let (_, truncated) = fit_text(
                &painter,
                local_id,
                fonts.id_font.clone(),
                (local_rect.width() - LABEL_PADDING).max(0.0),
            );
            if truncated {
                local_response.on_hover_text(local_id);
            }
        }

        let mut offsets = HashMap::new();
        let mut clicked_remote = None;
        let mut placed_rects_model = vec![local_rect_model];
        let mut any_overlap = false;
        // Two passes: first compute every remote's snapped rect in model
        // space (so overlap detection sees the final layout regardless of
        // draw order), then draw them in screen space. `remote_rects` also
        // lets the second pass avoid redoing the drag/snap math.
        let mut remote_rects: Vec<(String, egui::Rect, bool)> = Vec::new(); // (id, model_rect, is_being_dragged)

        for (index, (id, size)) in remotes.iter().enumerate() {
            let dims_model = egui::vec2(size.0 as f32, size.1 as f32);
            let pos_model = *self
                .positions
                .entry(id.clone())
                .or_insert_with(|| default_position_for_index(local_rect_model, dims_model, index));

            let desired_rect_model = egui::Rect::from_min_size(pos_model, dims_model);
            let desired_rect_screen = view.to_screen_rect(desired_rect_model);
            let drag_id = ui.id().with(("remote_drag", id.as_str()));
            let response = ui.interact(desired_rect_screen, drag_id, egui::Sense::click_and_drag());
            if response.dragged() {
                if let Some(p) = self.positions.get_mut(id) {
                    *p += view.to_model_vec(response.drag_delta());
                }
            }
            if response.clicked() {
                clicked_remote = Some(id.clone());
            }
            let pos_model = self.positions[id];
            let desired_rect_model = egui::Rect::from_min_size(pos_model, dims_model);
            let snapped_min_model = snap_touching(local_rect_model, desired_rect_model);
            let remote_rect_model = egui::Rect::from_min_size(snapped_min_model, dims_model);

            let dx = desired_rect_model.center().x - local_rect_model.center().x;
            let dy = desired_rect_model.center().y - local_rect_model.center().y;
            let axis = if dx.abs() >= dy.abs() {
                SnapAxis::Horizontal
            } else {
                SnapAxis::Vertical
            };
            self.last_axis.insert(id.clone(), axis);
            self.last_remote_dims.insert(id.clone(), dims_model);

            offsets.insert(
                id.clone(),
                RemoteOffset {
                    dx: (remote_rect_model.min.x - local_rect_model.min.x).round() as i32,
                    dy: (remote_rect_model.min.y - local_rect_model.min.y).round() as i32,
                },
            );

            if response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }

            if placed_rects_model
                .iter()
                .any(|r| meaningfully_overlaps(*r, remote_rect_model))
            {
                any_overlap = true;
            }
            placed_rects_model.push(remote_rect_model);
            remote_rects.push((id.clone(), remote_rect_model, response.dragged()));

            if response.hovered() {
                let remote_rect_screen = view.to_screen_rect(remote_rect_model);
                let (_, truncated) = fit_text(
                    &painter,
                    id,
                    fonts.id_font.clone(),
                    (remote_rect_screen.width() - LABEL_PADDING).max(0.0),
                );
                if truncated {
                    response.on_hover_text(id.as_str());
                }
            }
        }

        for (id, rect_model, dragged) in &remote_rects {
            let rect = view.to_screen_rect(*rect_model);
            let index = remotes.iter().position(|(rid, _)| rid == id).unwrap_or(0);
            let size = remotes[index].1;
            let in_palette = index < REMOTE_PALETTE.len();
            let base_color = if in_palette {
                REMOTE_PALETTE[index]
            } else {
                OVERFLOW_FILL
            };
            let color = if *dragged { DRAGGING_COLOR } else { base_color };
            let badge = if in_palette {
                BadgeSpec::None
            } else {
                BadgeSpec::Overflow(monogram(id))
            };
            draw_screen_box(&painter, rect, id, size, color, &fonts, badge);
            if selected_id == Some(id.as_str()) {
                painter.rect_stroke(
                    rect.expand(2.0),
                    8.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                );
            }
            let overlaps = placed_rects_model
                .iter()
                .filter(|r| meaningfully_overlaps(**r, *rect_model))
                .count()
                > 1;
            if overlaps {
                painter.rect_stroke(rect, 6.0, egui::Stroke::new(2.0_f32, OVERLAP_COLOR));
            }
        }

        if any_overlap {
            painter.text(
                canvas_rect.center_top() + egui::vec2(0.0, 8.0),
                egui::Align2::CENTER_TOP,
                "Screens overlap \u{2014} drag to separate",
                egui::FontId::proportional(11.0),
                OVERLAP_COLOR,
            );
        }

        self.paint_zoom_controls(ui, &painter, canvas_rect, &placed_rects_model);

        CanvasOutput {
            offsets,
            clicked_remote,
            any_overlap,
        }
    }

    /// The `[-] [100%] [+] [Fit]` cluster in the canvas's bottom-right
    /// corner -- the same layout convention OS display-arrangement dialogs
    /// use for exactly this task, so there's nothing new to learn here.
    fn paint_zoom_controls(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        canvas_rect: egui::Rect,
        placed_rects_model: &[egui::Rect],
    ) {
        let margin = 12.0;
        let button_size = egui::vec2(26.0, 24.0);
        let fit_size = egui::vec2(40.0, 24.0);
        let readout_width = 48.0;
        let gap = 4.0;
        let total_width = button_size.x * 2.0 + readout_width + fit_size.x + gap * 3.0;
        let cluster_rect = egui::Rect::from_min_size(
            canvas_rect.right_bottom()
                - egui::vec2(total_width, button_size.y)
                - egui::vec2(margin, margin),
            egui::vec2(total_width, button_size.y),
        );
        painter.rect_filled(
            cluster_rect.expand(4.0),
            6.0,
            egui::Color32::from_black_alpha(120),
        );

        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(cluster_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut ui;
        if ui
            .add_sized(button_size, secondary_button(ui, "-"))
            .clicked()
        {
            self.zoom = (self.zoom - ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        ui.add_sized(
            [readout_width, button_size.y],
            egui::Label::new(format!("{:.0}%", self.zoom * 100.0)),
        );
        if ui
            .add_sized(button_size, secondary_button(ui, "+"))
            .clicked()
        {
            self.zoom = (self.zoom + ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        if ui
            .add_sized(fit_size, secondary_button(ui, "Fit"))
            .clicked()
        {
            self.fit_all(canvas_rect, placed_rects_model);
        }
    }

    /// Resets zoom/pan so every currently-placed screen (local + remotes)
    /// fits inside the canvas with a comfortable margin -- the "fit all"
    /// convention any display-arrangement tool needs once panning/zooming
    /// exist at all, since a manually-panned view has no other way back to
    /// "show me everything."
    fn fit_all(&mut self, canvas_rect: egui::Rect, placed_rects_model: &[egui::Rect]) {
        let Some(first) = placed_rects_model.first().copied() else {
            return;
        };
        let union = placed_rects_model
            .iter()
            .fold(first, |acc, r| acc.union(*r));

        let padding = 40.0;
        let avail = (canvas_rect.size() - egui::vec2(padding, padding)).max(egui::vec2(1.0, 1.0));
        let scale_x = avail.x / union.width().max(1.0);
        let scale_y = avail.y / union.height().max(1.0);
        let scale = scale_x
            .min(scale_y)
            .clamp(MIN_ZOOM * BASE_SCALE, MAX_ZOOM * BASE_SCALE);
        self.zoom = (scale / BASE_SCALE).clamp(MIN_ZOOM, MAX_ZOOM);

        // Re-derive pan so the union's center lands on the canvas center:
        // origin = canvas_center + pan, and we want
        // origin + union.center().to_vec2() * new_scale == canvas_center.
        let new_scale = BASE_SCALE * self.zoom;
        self.pan = -union.center().to_vec2() * new_scale;
    }

    /// A non-interactive rendering of just the local screen -- no drag, no
    /// pan/zoom, no remotes. Used for the Target role, which never receives
    /// the controller-side arrangement over the wire (see `run_target`) and
    /// so has no real topology to draw; a positioned second box here would
    /// be fabricating a spatial relationship this machine has no data for.
    pub fn ui_readonly(&self, ui: &mut egui::Ui, local_id: &str, local_size: (i32, i32)) {
        let side = 220.0_f32.min(ui.available_width()).max(120.0);
        let aspect = local_size.1 as f32 / local_size.0.max(1) as f32;
        let box_size = if aspect <= 1.0 {
            egui::vec2(side, side * aspect)
        } else {
            egui::vec2(side / aspect, side)
        };
        let (rect, _) = ui.allocate_exact_size(box_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let fonts = ScreenFonts {
            id_font: egui::FontId::proportional(13.0),
            caption_font: egui::FontId::proportional(11.0),
        };
        draw_screen_box(
            &painter,
            rect,
            local_id,
            local_size,
            LOCAL_COLOR,
            &fonts,
            BadgeSpec::Identity("YOU"),
        );
    }

    /// Which one-click alignment buttons currently make sense for `id`:
    /// side-by-side screens align top/center/bottom, stacked screens align
    /// left/center/right. `None` if `id` hasn't been drawn by `ui()` yet.
    pub fn axis(&self, id: &str) -> Option<SnapAxis> {
        self.last_axis.get(id).copied()
    }

    /// Snaps remote screen `id` to a precise top/center/bottom (or
    /// left/center/right, depending on `axis()`) alignment along the
    /// touching edge -- the one-click alternative to dragging by hand when
    /// two screens have different resolutions and you want a specific,
    /// exact alignment rather than whatever pixel offset a drag happened to
    /// land on.
    pub fn align(&mut self, id: &str, alignment: Alignment) {
        let Some(axis) = self.last_axis.get(id).copied() else {
            return;
        };
        let Some(dims) = self.last_remote_dims.get(id).copied() else {
            return;
        };
        let local_rect = self.last_local_rect;
        let Some(pos) = self.positions.get_mut(id) else {
            return;
        };
        match axis {
            SnapAxis::Horizontal => {
                pos.y = match alignment {
                    Alignment::Start => local_rect.top(),
                    Alignment::Center => local_rect.center().y - dims.y / 2.0,
                    Alignment::End => local_rect.bottom() - dims.y,
                };
            }
            SnapAxis::Vertical => {
                pos.x = match alignment {
                    Alignment::Start => local_rect.left(),
                    Alignment::Center => local_rect.center().x - dims.x / 2.0,
                    Alignment::End => local_rect.right() - dims.x,
                };
            }
        }
    }
}

/// The one "secondary action" button style used everywhere in this app: an
/// accent-outline pill. Duplicated from `main.rs`'s helper of the same name
/// (rather than shared) since pulling it into a common module for two call
/// sites isn't worth the indirection -- if a third spot needs it, that's the
/// point to extract it.
fn secondary_button(ui: &egui::Ui, label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    let accent = ui.visuals().selection.bg_fill;
    egui::Button::new(label.into())
        .fill(accent.linear_multiply(0.16))
        .stroke(egui::Stroke::new(1.3_f32, accent))
}

/// A reasonable first-appearance position for the `index`-th remote screen,
/// cycling through the four sides of the local screen and pushing further
/// out on each extra lap so a 5th+ device doesn't land exactly on top of the
/// 1st. Purely a starting point -- the user can drag it anywhere. In model
/// space, so this only ever needs computing once per device regardless of
/// zoom.
fn default_position_for_index(
    local_rect: egui::Rect,
    dims: egui::Vec2,
    index: usize,
) -> egui::Pos2 {
    let lap_offset = (index / 4) as f32 * dims.y.max(dims.x) * 0.3;
    match index % 4 {
        0 => local_rect.right_top() + egui::vec2(lap_offset, 0.0),
        1 => egui::pos2(local_rect.left(), local_rect.bottom() + lap_offset),
        2 => egui::pos2(local_rect.left() - dims.x - lap_offset, local_rect.top()),
        _ => egui::pos2(local_rect.left(), local_rect.top() - dims.y - lap_offset),
    }
}

/// First 1-2 characters of `id`, uppercased -- the overflow-identity
/// monogram for devices past the color palette. Plain ASCII/whatever the
/// id's own characters are, so it never risks an unverified glyph.
fn monogram(id: &str) -> String {
    id.chars().take(2).collect::<String>().to_uppercase()
}

/// Which corner badge (if any) a screen box should show, and which visual
/// treatment it uses. Kept as one type rather than two separate booleans so
/// the two meanings ("this is you" vs. "we ran out of palette colors") can
/// never accidentally render identically -- DESIGN_REVIEW standard #8 exists
/// precisely because a shared visual treatment stops meaning any one thing
/// once two different facts use it.
enum BadgeSpec<'a> {
    None,
    /// The "YOU" ownership marker: dark filled pill, top-left corner.
    Identity(&'a str),
    /// An overflow-identity monogram: outlined pill, top-right corner --
    /// visually distinct from `Identity` on purpose, so the two can never
    /// be confused for meaning the same thing.
    Overflow(String),
}

/// Draws one screen box: the monitor shape, its (possibly truncated) id, a
/// resolution caption below it when there's room, and an optional corner
/// badge (e.g. "YOU"). Shared between the local screen and every remote so
/// they're guaranteed to look and size identically. Below
/// `COMPACT_WIDTH_THRESHOLD` this skips id/caption text entirely and shows
/// only a centered monogram -- there simply isn't room for three lines of
/// content at that size, and shrinking the font indefinitely just produces
/// illegible text instead of an honest "too small to show detail" state.
fn draw_screen_box(
    painter: &egui::Painter,
    rect: egui::Rect,
    id: &str,
    size: (i32, i32),
    fill: egui::Color32,
    fonts: &ScreenFonts,
    badge: BadgeSpec,
) {
    draw_monitor(painter, rect, fill);

    if rect.width() < COMPACT_WIDTH_THRESHOLD {
        let compact_font = egui::FontId::proportional((rect.height() * 0.4).clamp(8.0, 14.0));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            monogram(id),
            compact_font,
            egui::Color32::WHITE,
        );
        return;
    }

    let max_width = (rect.width() - LABEL_PADDING).max(0.0);
    let (label, truncated) = fit_text(painter, id, fonts.id_font.clone(), max_width);
    let caption = format!("{} \u{00d7} {}", size.0, size.1);
    let show_caption = rect.height() >= 44.0;
    let id_pos = if show_caption {
        rect.center() - egui::vec2(0.0, 8.0)
    } else {
        rect.center()
    };
    let galley = painter.layout_no_wrap(label.clone(), fonts.id_font.clone(), egui::Color32::WHITE);
    painter.text(
        id_pos,
        egui::Align2::CENTER_CENTER,
        &label,
        fonts.id_font.clone(),
        egui::Color32::WHITE,
    );
    if truncated {
        let label_rect =
            egui::Rect::from_center_size(id_pos, galley.size()).translate(egui::vec2(0.0, 1.0));
        paint_truncation_marker(painter, label_rect);
    }
    if show_caption {
        painter.text(
            rect.center() + egui::vec2(0.0, 9.0),
            egui::Align2::CENTER_CENTER,
            caption,
            fonts.caption_font.clone(),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
        );
    }
    match badge {
        BadgeSpec::None => {}
        BadgeSpec::Identity(text) => paint_identity_badge(painter, rect, text),
        BadgeSpec::Overflow(text) => paint_overflow_badge(painter, rect, &text),
    }
}

/// Draws a screen as a little monitor rather than a bare rectangle: a
/// drop shadow for depth, the screen itself, and a neck+base stand below
/// it. Reads instantly as "this represents a physical display" the way a
/// real OS's display-arrangement dialog does, which matters here since two
/// boxes of different sizes are exactly what a mixed-resolution setup looks
/// like.
fn draw_monitor(painter: &egui::Painter, rect: egui::Rect, fill: egui::Color32) {
    let shadow_rect = rect.translate(egui::vec2(3.0, 4.0));
    painter.rect_filled(shadow_rect, 6.0, egui::Color32::from_black_alpha(70));

    let neck_w = (rect.width() * 0.16).clamp(5.0, 16.0);
    let neck_h = (rect.height() * 0.14).clamp(3.0, 9.0);
    let neck_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() + neck_h / 2.0),
        egui::vec2(neck_w, neck_h),
    );
    painter.rect_filled(neck_rect, 0.0, fill.gamma_multiply(0.85));

    let base_w = (rect.width() * 0.4).clamp(neck_w * 1.4, rect.width());
    let base_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, neck_rect.bottom() + 1.5),
        egui::vec2(base_w, 3.0),
    );
    painter.rect_filled(base_rect, 1.5, fill.gamma_multiply(0.85));

    painter.rect_filled(rect, 6.0, fill);
    painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
}

/// Minimum box size a corner badge can legibly render inside. The height
/// (74, not just enough to fit the badge itself) is the load-bearing part:
/// `draw_screen_box` centers the id label vertically at
/// `rect.center() - (0, 8)` regardless of whether a badge is showing, and a
/// top-left badge only clears that centered label once the box is tall
/// enough for the two to not occupy the same vertical band -- below this,
/// the corner badge and the id text visibly overlapped into an unreadable
/// smear on any modest-resolution screen box at 100% zoom.
const BADGE_MIN_SIZE: egui::Vec2 = egui::vec2(64.0, 74.0);

/// A small filled pill badge in a box's top-left corner: the "YOU" ownership
/// marker. Dark fill, on purpose -- this is the one and only shape/position
/// combination that means "this is your machine."
fn paint_identity_badge(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    if rect.width() < BADGE_MIN_SIZE.x || rect.height() < BADGE_MIN_SIZE.y {
        return;
    }
    let font = egui::FontId::proportional(11.0);
    let padding = egui::vec2(5.0, 2.0);
    let galley = painter.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE);
    let badge_size = galley.size() + padding * 2.0;
    let badge_rect = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), badge_size);
    painter.rect_filled(
        badge_rect,
        3.0,
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 130),
    );
    painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235),
    );
}

/// A small outlined pill badge in a box's top-*right* corner: the
/// overflow-identity monogram for devices past the color palette.
/// Deliberately a different fill (transparent, stroked) and a different
/// corner than `paint_identity_badge`'s "YOU" marker -- an identical
/// treatment would let a second, unrelated meaning ("we ran out of colors")
/// borrow the one visual token that's supposed to mean "this is you"
/// (DESIGN_REVIEW standard #8).
fn paint_overflow_badge(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    if rect.width() < BADGE_MIN_SIZE.x || rect.height() < BADGE_MIN_SIZE.y {
        return;
    }
    let font = egui::FontId::proportional(11.0);
    let padding = egui::vec2(5.0, 2.0);
    let galley = painter.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE);
    let badge_size = galley.size() + padding * 2.0;
    let badge_rect = egui::Rect::from_min_size(
        rect.right_top() + egui::vec2(-4.0 - badge_size.x, 4.0),
        badge_size,
    );
    painter.rect_filled(
        badge_rect,
        3.0,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
    );
    painter.rect_stroke(
        badge_rect,
        3.0,
        egui::Stroke::new(
            1.0_f32,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 140),
        ),
    );
    painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235),
    );
}

/// Spacing (canvas points, at zoom = 1) between dots in the background grid.
const GRID_SPACING: f32 = 20.0;

/// A faint dot grid over the canvas background, panned/zoomed along with
/// everything else so it reads as a real to-scale workspace rather than a
/// static decoration behind the screens.
fn paint_grid(painter: &egui::Painter, canvas_rect: egui::Rect, view: &ViewTransform) {
    let dot_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14);
    let spacing = (GRID_SPACING * view.scale / BASE_SCALE).max(6.0);
    // Phase the grid so it appears to pan with the content: find the
    // on-screen position of model-space origin modulo `spacing`.
    let origin_screen = view.origin;
    let start_x = canvas_rect.left() + (origin_screen.x - canvas_rect.left()).rem_euclid(spacing);
    let start_y = canvas_rect.top() + (origin_screen.y - canvas_rect.top()).rem_euclid(spacing);
    let mut gx = start_x;
    while gx < canvas_rect.right() {
        let mut gy = start_y;
        while gy < canvas_rect.bottom() {
            painter.circle_filled(egui::pos2(gx, gy), 1.0, dot_color);
            gy += spacing;
        }
        gx += spacing;
    }
}

/// A small ruler-style scale bar in the canvas's bottom-left corner, showing
/// how many real screen pixels one canvas segment represents at the current
/// zoom. Tucked away from where the screen boxes are placed so it never
/// overlaps them; mostly there to reinforce that the boxes are drawn to
/// scale, not just arbitrary icons.
fn paint_scale_indicator(painter: &egui::Painter, canvas_rect: egui::Rect, view: &ViewTransform) {
    let real_px: f32 = 500.0;
    let bar_len = real_px * view.scale;
    let origin = egui::pos2(canvas_rect.left() + 14.0, canvas_rect.bottom() - 14.0);
    let end = origin + egui::vec2(bar_len, 0.0);
    let stroke = egui::Stroke::new(
        1.0_f32,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 90),
    );

    painter.line_segment([origin, end], stroke);
    painter.line_segment(
        [
            origin + egui::vec2(0.0, -3.0),
            origin + egui::vec2(0.0, 3.0),
        ],
        stroke,
    );
    painter.line_segment(
        [end + egui::vec2(0.0, -3.0), end + egui::vec2(0.0, 3.0)],
        stroke,
    );
    painter.text(
        egui::pos2((origin.x + end.x) / 2.0, origin.y - 5.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{real_px:.0} px"),
        egui::FontId::proportional(11.0),
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 130),
    );
}

/// Whether `a` and `b` share real interior area, not just a touching edge or
/// corner. Two screens snapped flush against each other by `snap_touching`
/// always share a zero-width edge, and `egui::Rect::intersects` treats that
/// as an intersection (its bounds checks use `<=`) -- using it directly here
/// would flag *every* freshly-placed remote as overlapping the local screen
/// it's deliberately touching, before the user ever drags anything. A small
/// epsilon absorbs the sub-pixel slop `snap_touching`'s float math can leave
/// on an edge that's meant to be exactly flush.
fn meaningfully_overlaps(a: egui::Rect, b: egui::Rect) -> bool {
    let i = a.intersect(b);
    i.width() > 1.0 && i.height() > 1.0
}

/// Snaps `desired` flush against whichever side of `local` its center is
/// nearest to (comparing horizontal vs. vertical separation), clamping the
/// perpendicular axis so the two rects always keep a real overlap -- never
/// just a single touching corner, which would make edge-crossing fragile.
/// Operates entirely in model space (real pixels), so it's zoom-independent.
///
/// Only snaps against the local screen, not against other remotes -- with N
/// remotes that can produce overlapping boxes, which `ui()` detects and
/// flags visually rather than silently resolving.
fn snap_touching(local: egui::Rect, desired: egui::Rect) -> egui::Pos2 {
    let dx = desired.center().x - local.center().x;
    let dy = desired.center().y - local.center().y;

    // Minimum perpendicular overlap, in model pixels, so the touching edge
    // always has some real length to cross through.
    let min_overlap = (150.0_f32).min(desired.width().min(desired.height()) * 0.3);

    if dx.abs() >= dy.abs() {
        let x = if dx >= 0.0 {
            local.right()
        } else {
            local.left() - desired.width()
        };
        let y = desired.min.y.clamp(
            local.top() - desired.height() + min_overlap,
            local.bottom() - min_overlap,
        );
        egui::pos2(x, y)
    } else {
        let y = if dy >= 0.0 {
            local.bottom()
        } else {
            local.top() - desired.height()
        };
        let x = desired.min.x.clamp(
            local.left() - desired.width() + min_overlap,
            local.right() - min_overlap,
        );
        egui::pos2(x, y)
    }
}
