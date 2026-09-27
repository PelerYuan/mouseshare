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

pub struct CanvasState {
    /// The remote box's user-dragged (unsnapped) top-left position, in
    /// canvas-local coordinates. Only ever moved by drag deltas; the
    /// rendered/reported position is always the *snapped* projection of
    /// this, so the box doesn't fight the snap while being dragged.
    remote_desired_pos: egui::Pos2,
    initialized: bool,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            remote_desired_pos: egui::pos2(0.0, 0.0),
            initialized: false,
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
        let canvas_size = egui::vec2(480.0, 260.0);
        let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 4.0, ui.visuals().extreme_bg_color);

        let local_dims = egui::vec2(local_size.0 as f32 * SCALE, local_size.1 as f32 * SCALE);
        let remote_dims = egui::vec2(remote_size.0 as f32 * SCALE, remote_size.1 as f32 * SCALE);

        // Local screen is fixed, roughly centered with room to its right
        // for the remote screen on first paint.
        let local_min = canvas_rect.center() - local_dims / 2.0 - egui::vec2(local_dims.x * 0.35, 0.0);
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

        painter.rect_filled(local_rect, 3.0, egui::Color32::from_rgb(70, 130, 180));
        painter.rect_stroke(local_rect, 3.0, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
        painter.text(
            local_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{local_id}\n(local)"),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

        let remote_color = if drag_response.dragged() {
            egui::Color32::from_rgb(230, 160, 70)
        } else {
            egui::Color32::from_rgb(150, 110, 200)
        };
        painter.rect_filled(remote_rect, 3.0, remote_color);
        painter.rect_stroke(remote_rect, 3.0, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
        painter.text(
            remote_rect.center(),
            egui::Align2::CENTER_CENTER,
            remote_id,
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

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
        let y = desired
            .min
            .y
            .clamp(local.top() - desired.height() + min_overlap, local.bottom() - min_overlap);
        egui::pos2(x, y)
    } else {
        let y = if dy >= 0.0 {
            local.bottom()
        } else {
            local.top() - desired.height()
        };
        let x = desired
            .min
            .x
            .clamp(local.left() - desired.width() + min_overlap, local.right() - min_overlap);
        egui::pos2(x, y)
    }
}
