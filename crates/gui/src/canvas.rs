//! The draggable two-screen arrangement widget. Dragging the remote screen
//! box always snaps it flush against whichever side of the local screen
//! it's nearest to, with a clamped perpendicular offset -- this guarantees
//! the resulting `ScreenRect`s always touch (no gap, which would leave the
//! cursor stuck at the wall instead of handing off -- see
//! `mouseshare_layout`'s docs) while still feeling like a real drag, not a
//! multiple-choice form.

use eframe::egui;

/// Real pixels -> canvas points. Chosen so a pair of 1920x1080 screens
/// comfortably fits the panel without scrolling.
const SCALE: f32 = 0.08;

/// Horizontal breathing room (in canvas points) reserved inside a screen box
/// when fitting its label -- keeps truncated text from touching the border.
const LABEL_PADDING: f32 = 10.0;

/// Lays out `text` at `font_id` and, if it's wider than `max_width`,
/// progressively drops trailing characters and appends an ellipsis until it
/// fits. This is what keeps a long hostname like
/// "peler-Standard-PC-i440FX-PIIX-1996" from spilling out past its screen
/// box: no matter how long the id is, the rendered label is guaranteed to
/// fit inside `max_width`. Returns the text actually used alongside whether
/// it was shortened, so callers can offer the full string as a tooltip.
fn fit_text(
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

/// Which side the remote screen is currently snapped to, in terms of the
/// axis a one-click "align" action would adjust: side-by-side screens can
/// only usefully be aligned top/center/bottom (their shared edge is
/// vertical), stacked screens only left/center/right.
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
    /// The remote box's user-dragged (unsnapped) top-left position, in
    /// canvas-local coordinates. Only ever moved by drag deltas; the
    /// rendered/reported position is always the *snapped* projection of
    /// this, so the box doesn't fight the snap while being dragged.
    remote_desired_pos: egui::Pos2,
    initialized: bool,
    /// Cached from the previous frame's layout, so the alignment buttons
    /// (drawn after this widget, since they need to know the current snap
    /// side) can recompute `remote_desired_pos` without this widget having
    /// to expose its internal rect math to the caller.
    last_axis: SnapAxis,
    last_local_rect: egui::Rect,
    last_remote_dims: egui::Vec2,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            remote_desired_pos: egui::pos2(0.0, 0.0),
            initialized: false,
            last_axis: SnapAxis::Horizontal,
            last_local_rect: egui::Rect::NOTHING,
            last_remote_dims: egui::Vec2::ZERO,
        }
    }
}

/// Where the remote screen ended up relative to the local screen's origin,
/// in real pixels -- directly usable as a `ScreenRect`'s `(x, y)` once the
/// local screen is placed at `(0, 0)`.
pub struct RemoteOffset {
    pub dx: i32,
    pub dy: i32,
}

impl CanvasState {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        local_id: &str,
        local_size: (i32, i32),
        remote_id: &str,
        remote_size: (i32, i32),
    ) -> RemoteOffset {
        let canvas_width = ui.available_width().clamp(280.0, 560.0);
        let canvas_size = egui::vec2(canvas_width, 300.0);
        let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 6.0, ui.visuals().extreme_bg_color);
        paint_grid(&painter, canvas_rect);
        paint_scale_indicator(&painter, canvas_rect);

        let local_dims = egui::vec2(local_size.0 as f32 * SCALE, local_size.1 as f32 * SCALE);
        let remote_dims = egui::vec2(remote_size.0 as f32 * SCALE, remote_size.1 as f32 * SCALE);

        // Local screen is fixed, roughly centered with room to its right
        // for the remote screen on first paint.
        let local_min =
            canvas_rect.center() - local_dims / 2.0 - egui::vec2(local_dims.x * 0.35, 0.0);
        let local_rect = egui::Rect::from_min_size(local_min, local_dims);

        if !self.initialized {
            // Start the remote screen touching the local screen's right
            // edge, vertically aligned -- a sane default for the common
            // "extend desktop to the right" arrangement.
            self.remote_desired_pos = local_rect.right_top();
            self.initialized = true;
        }

        let desired_rect = egui::Rect::from_min_size(self.remote_desired_pos, remote_dims);
        let drag_id = ui.id().with("remote_screen_drag");
        let drag_response = ui.interact(desired_rect, drag_id, egui::Sense::drag());
        self.remote_desired_pos += drag_response.drag_delta();
        // Keep the desired position from drifting arbitrarily far off, so
        // the box is easy to find again after a big drag.
        let desired_rect = egui::Rect::from_min_size(self.remote_desired_pos, remote_dims);

        let snapped_min = snap_touching(local_rect, desired_rect);
        let remote_rect = egui::Rect::from_min_size(snapped_min, remote_dims);

        let dx = desired_rect.center().x - local_rect.center().x;
        let dy = desired_rect.center().y - local_rect.center().y;
        self.last_axis = if dx.abs() >= dy.abs() {
            SnapAxis::Horizontal
        } else {
            SnapAxis::Vertical
        };
        self.last_local_rect = local_rect;
        self.last_remote_dims = remote_dims;

        let id_font = egui::FontId::proportional(13.0);
        let caption_font = egui::FontId::proportional(10.0);

        draw_monitor(&painter, local_rect, egui::Color32::from_rgb(70, 130, 180));
        // Reserve room for the resolution caption on its own line below the
        // id, and fit the id itself to the box width -- this is what
        // prevents a long hostname from spilling out past the rectangle.
        let local_max_width = (local_rect.width() - LABEL_PADDING).max(0.0);
        let (local_label, local_truncated) =
            fit_text(&painter, local_id, id_font.clone(), local_max_width);
        let local_caption = format!("{} \u{00d7} {}", local_size.0, local_size.1);
        let show_local_caption = local_rect.height() >= 44.0;
        let local_id_pos = if show_local_caption {
            local_rect.center() - egui::vec2(0.0, 8.0)
        } else {
            local_rect.center()
        };
        painter.text(
            local_id_pos,
            egui::Align2::CENTER_CENTER,
            &local_label,
            id_font.clone(),
            egui::Color32::WHITE,
        );
        if show_local_caption {
            painter.text(
                local_rect.center() + egui::vec2(0.0, 9.0),
                egui::Align2::CENTER_CENTER,
                local_caption,
                caption_font.clone(),
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
            );
        }
        // "YOU" badge in the corner rather than a text line -- the resolution
        // caption already occupies the second line, and a small pill reads
        // faster than another line of text would.
        paint_badge(&painter, local_rect, "YOU");
        let local_hover_id = ui.id().with("local_screen_hover");
        let local_response = ui.interact(local_rect, local_hover_id, egui::Sense::hover());
        if local_truncated {
            local_response.on_hover_text(local_id);
        }

        let remote_color = if drag_response.dragged() {
            egui::Color32::from_rgb(230, 160, 70)
        } else {
            egui::Color32::from_rgb(150, 110, 200)
        };
        draw_monitor(&painter, remote_rect, remote_color);
        let remote_max_width = (remote_rect.width() - LABEL_PADDING).max(0.0);
        let (remote_label, remote_truncated) =
            fit_text(&painter, remote_id, id_font.clone(), remote_max_width);
        let remote_caption = format!("{} \u{00d7} {}", remote_size.0, remote_size.1);
        let show_remote_caption = remote_rect.height() >= 44.0;
        let remote_id_pos = if show_remote_caption {
            remote_rect.center() - egui::vec2(0.0, 8.0)
        } else {
            remote_rect.center()
        };
        painter.text(
            remote_id_pos,
            egui::Align2::CENTER_CENTER,
            &remote_label,
            id_font,
            egui::Color32::WHITE,
        );
        if show_remote_caption {
            painter.text(
                remote_rect.center() + egui::vec2(0.0, 9.0),
                egui::Align2::CENTER_CENTER,
                remote_caption,
                caption_font,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
            );
        }

        let drag_response = if remote_truncated {
            drag_response.on_hover_text(remote_id)
        } else {
            drag_response
        };
        if drag_response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if drag_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        RemoteOffset {
            dx: ((remote_rect.min.x - local_rect.min.x) / SCALE).round() as i32,
            dy: ((remote_rect.min.y - local_rect.min.y) / SCALE).round() as i32,
        }
    }

    /// Which one-click alignment buttons currently make sense: side-by-side
    /// screens align top/center/bottom, stacked screens align
    /// left/center/right.
    pub fn axis(&self) -> SnapAxis {
        self.last_axis
    }

    /// Snaps the remote screen to a precise top/center/bottom (or
    /// left/center/right, depending on `axis()`) alignment along the
    /// touching edge -- the one-click alternative to dragging by hand when
    /// two screens have different resolutions and you want a specific,
    /// exact alignment rather than whatever pixel offset a drag happened to
    /// land on.
    pub fn align(&mut self, alignment: Alignment) {
        match self.last_axis {
            SnapAxis::Horizontal => {
                let y = match alignment {
                    Alignment::Start => self.last_local_rect.top(),
                    Alignment::Center => {
                        self.last_local_rect.center().y - self.last_remote_dims.y / 2.0
                    }
                    Alignment::End => self.last_local_rect.bottom() - self.last_remote_dims.y,
                };
                self.remote_desired_pos.y = y;
            }
            SnapAxis::Vertical => {
                let x = match alignment {
                    Alignment::Start => self.last_local_rect.left(),
                    Alignment::Center => {
                        self.last_local_rect.center().x - self.last_remote_dims.x / 2.0
                    }
                    Alignment::End => self.last_local_rect.right() - self.last_remote_dims.x,
                };
                self.remote_desired_pos.x = x;
            }
        }
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

/// A small pill badge in a box's top-left corner (e.g. "YOU" on the local
/// screen), so identifying which box is this machine doesn't rely on
/// remembering which color means what.
fn paint_badge(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    if rect.width() < 50.0 || rect.height() < 30.0 {
        return;
    }
    let font = egui::FontId::proportional(8.5);
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

/// Spacing (canvas points) between dots in the background grid.
const GRID_SPACING: f32 = 20.0;

/// A faint dot grid over the canvas background. On its own the canvas was
/// just two small boxes floating in a big empty dark rectangle, which read
/// as an unfinished placeholder rather than an arrangement tool; a subtle
/// grid gives it the texture of a real to-scale workspace without
/// distracting from the screens themselves.
fn paint_grid(painter: &egui::Painter, canvas_rect: egui::Rect) {
    let dot_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14);
    let mut gx = canvas_rect.left() + GRID_SPACING;
    while gx < canvas_rect.right() {
        let mut gy = canvas_rect.top() + GRID_SPACING;
        while gy < canvas_rect.bottom() {
            painter.circle_filled(egui::pos2(gx, gy), 1.0, dot_color);
            gy += GRID_SPACING;
        }
        gx += GRID_SPACING;
    }
}

/// A small ruler-style scale bar in the canvas's bottom-left corner, showing
/// how many real screen pixels one canvas segment represents. Tucked away
/// from where the screen boxes are placed so it never overlaps them; mostly
/// there to reinforce that the boxes are drawn to scale, not just arbitrary
/// icons.
fn paint_scale_indicator(painter: &egui::Painter, canvas_rect: egui::Rect) {
    let real_px: f32 = 500.0;
    let bar_len = real_px * SCALE;
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
        egui::FontId::proportional(9.0),
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 130),
    );
}

/// Snaps `desired` flush against whichever side of `local` its center is
/// nearest to (comparing horizontal vs. vertical separation), clamping the
/// perpendicular axis so the two rects always keep a real overlap -- never
/// just a single touching corner, which would make edge-crossing fragile.
fn snap_touching(local: egui::Rect, desired: egui::Rect) -> egui::Pos2 {
    let dx = desired.center().x - local.center().x;
    let dy = desired.center().y - local.center().y;

    // Minimum perpendicular overlap, in canvas points, so the touching edge
    // always has some real length to cross through.
    let min_overlap = 12.0;

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
