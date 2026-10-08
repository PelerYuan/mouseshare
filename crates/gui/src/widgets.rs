//! The small widget set the whole UI is built from, so every screen shares
//! one visual language instead of hand-styling buttons ad hoc.

use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Response, RichText, Sense, Stroke, Ui, Vec2,
};

use crate::theme::{Theme, RADIUS, SP_1, SP_2, SP_3};

/// Filled accent button: the one primary action of a view.
pub fn primary_button(ui: &mut Ui, th: &Theme, label: impl Into<String>) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label.into()).color(th.on_accent()).strong())
            .fill(th.accent)
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(0.0, 34.0)),
    )
}

/// Destructive/stop variant of the primary button.
pub fn danger_button(ui: &mut Ui, th: &Theme, label: impl Into<String>) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label.into()).color(Color32::WHITE).strong())
            .fill(th.p.danger)
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(0.0, 34.0)),
    )
}

/// Quiet neutral button for secondary actions.
pub fn secondary_button(ui: &mut Ui, th: &Theme, label: impl Into<String>) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label.into()).color(th.p.text))
            .fill(th.p.raised)
            .stroke(Stroke::new(1.0_f32, th.p.border))
            .min_size(Vec2::new(0.0, 30.0)),
    )
}

/// Icon-only button without chrome until hovered.
pub fn icon_button(ui: &mut Ui, th: &Theme, icon: &str, tip: &str) -> Response {
    let r = ui.add(
        egui::Button::new(RichText::new(icon).size(17.0).color(th.p.text_dim))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .min_size(Vec2::splat(30.0)),
    );
    r.on_hover_text(tip)
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// A rounded surface that groups related controls.
pub fn card<R>(ui: &mut Ui, th: &Theme, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(th.p.raised)
        .rounding(RADIUS)
        .inner_margin(egui::Margin::same(SP_3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Small caps-style heading above a group of controls.
pub fn section_label(ui: &mut Ui, th: &Theme, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .strong()
            .color(th.p.text_faint),
    );
}

/// A coloured status dot, 8px.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Dot + text pill used for connection state.
pub fn status_pill(ui: &mut Ui, th: &Theme, color: Color32, text: &str) {
    egui::Frame::none()
        .fill(color.gamma_multiply(if th.dark { 0.18 } else { 0.14 }))
        .rounding(99.0)
        .inner_margin(egui::Margin::symmetric(SP_2, 3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                dot(ui, color);
                ui.label(RichText::new(text).size(12.0).color(color).strong());
            });
        });
}

/// iOS-style on/off switch. Returns true when toggled this frame.
pub fn toggle(ui: &mut Ui, th: &Theme, on: &mut bool) -> Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let off = th.p.border.gamma_multiply(1.4);
    let bg = lerp_color(off, th.accent, t);
    ui.painter().rect_filled(rect, 99.0, bg);
    let r = size.y / 2.0 - 3.0;
    let cx = egui::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), r, Color32::WHITE);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// A settings row: label (+ optional hint) on the left, control on the right.
pub fn setting_row(
    ui: &mut Ui,
    th: &Theme,
    label: &str,
    hint: Option<&str>,
    control: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 190.0).max(120.0));
            ui.label(RichText::new(label).strong());
            if let Some(h) = hint {
                ui.label(RichText::new(h).size(12.0).color(th.p.text_dim));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(SP_1);
}

/// Horizontal segmented control; returns the newly chosen index, if any.
pub fn segmented(ui: &mut Ui, th: &Theme, labels: &[&str], selected: usize) -> Option<usize> {
    let mut chosen = None;
    egui::Frame::none()
        .fill(th.p.raised)
        .rounding(RADIUS)
        .inner_margin(egui::Margin::same(3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, l) in labels.iter().enumerate() {
                    let on = i == selected;
                    let b = egui::Button::new(
                        RichText::new(*l)
                            .color(if on { th.on_accent() } else { th.p.text_dim })
                            .strong(),
                    )
                    .fill(if on { th.accent } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .rounding(RADIUS - 2.0)
                    .min_size(Vec2::new(0.0, 28.0));
                    if ui.add(b).clicked() && !on {
                        chosen = Some(i);
                    }
                }
            });
        });
    chosen
}

/// Right-aligned wrapper helper.
pub fn right<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add)
        .inner
}

/// Hairline divider.
pub fn divider(ui: &mut Ui, th: &Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, th.p.border);
}
