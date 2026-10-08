//! Central view while this computer is a target: the pairing code front
//! and centre, since entering it on the controller is the only thing the
//! user has to do.

use eframe::egui::{self, Align2, FontId, RichText, Sense, Vec2};
use egui_phosphor::regular as ph;

use crate::app::App;
use crate::theme::{RADIUS, SP_2, SP_4, SP_6};
use crate::widgets;

impl App {
    pub fn target_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();
        let code = self.pairing.own_code().display();
        let port = self.settings.listen_port;
        let address = self
            .lan_ip
            .map(|ip| format!("{ip}:{port}"))
            .unwrap_or_else(|| format!("<ip>:{port}"));
        let snap = self.tele.snapshot();

        ui.vertical_centered(|ui| {
            ui.set_max_width(560.0);
            ui.add_space(SP_6);
            ui.label(RichText::new(tr.t("target_title")).size(22.0).strong());
            ui.add_space(SP_2);
            ui.label(RichText::new(tr.t("target_sub")).color(th.p.text_dim));
            ui.add_space(SP_6);

            // The code card.
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 164.0), Sense::hover());
            ui.painter().rect_filled(rect, 14.0, th.p.surface);
            ui.painter()
                .rect_stroke(rect, 14.0, egui::Stroke::new(1.0_f32, th.p.border));
            ui.painter().text(
                egui::pos2(rect.center().x, rect.top() + 26.0),
                Align2::CENTER_CENTER,
                tr.t("pairing_code").to_uppercase(),
                FontId::proportional(11.0),
                th.p.text_faint,
            );
            ui.painter().text(
                egui::pos2(rect.center().x, rect.center().y - 6.0),
                Align2::CENTER_CENTER,
                &code,
                FontId::monospace(40.0),
                th.accent,
            );
            let just_copied = self
                .copied_at
                .is_some_and(|t| t.elapsed().as_millis() < 1500);
            let btn_label = if just_copied {
                format!("{}  {}", ph::CHECK, tr.t("copied"))
            } else {
                format!("{}  {}", ph::COPY, tr.t("copy"))
            };
            let btn_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x - 56.0, rect.bottom() - 28.0),
                Vec2::new(104.0, 30.0),
            );
            let regen_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x + 64.0, rect.bottom() - 28.0),
                Vec2::new(120.0, 30.0),
            );
            if ui
                .put(btn_rect, egui::Button::new(btn_label).rounding(RADIUS))
                .clicked()
            {
                self.copy(ctx, code.clone());
            }
            let regen = ui.add_enabled_ui(!running, |ui| {
                ui.put(
                    regen_rect,
                    egui::Button::new(format!("{}  {}", ph::ARROWS_CLOCKWISE, tr.t("new_code")))
                        .rounding(RADIUS),
                )
            });
            if regen
                .inner
                .on_disabled_hover_text(tr.t("stop_first"))
                .clicked()
            {
                self.pairing.regenerate_own();
            }
            ui.add_space(SP_4);

            widgets::card(ui, &th, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr.t("address")).color(th.p.text_dim));
                    widgets::right(ui, |ui| {
                        if widgets::icon_button(ui, &th, ph::COPY, tr.t("copy")).clicked() {
                            self.copy(ctx, address.clone());
                        }
                        ui.label(RichText::new(&address).monospace());
                    });
                });
                ui.add_space(SP_2);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr.t("status")).color(th.p.text_dim));
                    widgets::right(ui, |ui| {
                        let (color, text) = if !running {
                            (th.p.text_faint, tr.t("target_stopped").to_string())
                        } else if let Some(c) = &snap.controller {
                            (th.p.ok, tr.f("target_controlled_by", &[c]))
                        } else {
                            (th.p.warn, tr.t("target_waiting").to_string())
                        };
                        ui.label(RichText::new(text).color(color));
                        widgets::dot(ui, color);
                    });
                });
            });
            ui.add_space(SP_4);

            ui.label(
                RichText::new(tr.t("target_steps_title"))
                    .size(12.0)
                    .strong()
                    .color(th.p.text_faint),
            );
            ui.add_space(SP_2);
            for (i, text) in [
                tr.t("target_step1"),
                tr.t("target_step2"),
                tr.t("target_step3"),
            ]
            .into_iter()
            .enumerate()
            {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}", i + 1))
                            .strong()
                            .color(th.accent),
                    );
                    ui.label(RichText::new(text).color(th.p.text_dim));
                });
            }
        });
    }
}
