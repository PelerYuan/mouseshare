//! First-run wizard: name this computer, pick a role, learn the next step.

use eframe::egui::{self, Align2, FontId, RichText, Sense, Stroke, Vec2};
use egui_phosphor::regular as ph;
use mouseshare_config::Role;

use crate::app::App;
use crate::dialogs::{modal, AddDialog};
use crate::theme::{SP_2, SP_3, SP_4, SP_6};
use crate::widgets;

#[derive(Default)]
pub struct Onboarding {
    pub step: usize,
}

const STEPS: usize = 3;

impl App {
    pub fn onboarding_window(&mut self, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let Some(ob) = self.onboarding.as_ref() else {
            return;
        };
        let step = ob.step;
        let mut next = false;
        let mut back = false;
        let mut finish = false;
        let mut skip = false;

        modal(ctx, &th, "onboarding", 560.0, |ui| {
            // Progress dots.
            ui.horizontal(|ui| {
                for i in 0..STEPS {
                    let w = if i == step { 22.0 } else { 8.0 };
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 6.0), Sense::hover());
                    ui.painter().rect_filled(
                        r,
                        3.0,
                        if i <= step { th.accent } else { th.p.border },
                    );
                }
                widgets::right(ui, |ui| {
                    if step + 1 < STEPS && ui.link(tr.t("skip")).clicked() {
                        skip = true;
                    }
                });
            });
            ui.add_space(SP_6);

            match step {
                0 => {
                    ui.label(RichText::new(ph::CURSOR_CLICK).size(40.0).color(th.accent));
                    ui.add_space(SP_2);
                    ui.label(RichText::new(tr.t("ob_welcome")).size(26.0).strong());
                    ui.add_space(SP_2);
                    ui.label(RichText::new(tr.t("ob_welcome_sub")).color(th.p.text_dim));
                    ui.add_space(SP_6);
                    ui.label(RichText::new(tr.t("ob_name_q")).strong());
                    ui.add_space(SP_2);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.name_text)
                            .desired_width(f32::INFINITY)
                            .margin(Vec2::new(10.0, 8.0)),
                    );
                    if r.lost_focus() {
                        let t = self.name_text.clone();
                        self.rename_local(&t);
                        self.name_text = self.settings.device_name.clone();
                    }
                    ui.add_space(SP_2);
                    ui.label(
                        RichText::new(tr.t("ob_name_hint"))
                            .size(12.0)
                            .color(th.p.text_faint),
                    );
                }
                1 => {
                    ui.label(RichText::new(tr.t("ob_role_title")).size(24.0).strong());
                    ui.add_space(SP_2);
                    ui.label(RichText::new(tr.t("ob_role_sub")).color(th.p.text_dim));
                    ui.add_space(SP_4);
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - SP_3) / 2.0;
                        for (role, icon, title, body) in [
                            (
                                Role::Controller,
                                ph::MOUSE_SIMPLE,
                                tr.t("ob_role_ctrl"),
                                tr.t("ob_role_ctrl_body"),
                            ),
                            (
                                Role::Target,
                                ph::MONITOR,
                                tr.t("ob_role_target"),
                                tr.t("ob_role_target_body"),
                            ),
                        ] {
                            let on = self.settings.role == role;
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(w, 170.0), Sense::click());
                            ui.painter().rect_filled(
                                rect,
                                12.0,
                                if on { th.accent_soft() } else { th.p.raised },
                            );
                            ui.painter().rect_stroke(
                                rect,
                                12.0,
                                Stroke::new(
                                    if on { 2.0_f32 } else { 1.0 },
                                    if on { th.accent } else { th.p.border },
                                ),
                            );
                            ui.painter().text(
                                rect.left_top() + Vec2::new(SP_4, SP_4),
                                Align2::LEFT_TOP,
                                icon,
                                FontId::proportional(28.0),
                                if on { th.accent } else { th.p.text_dim },
                            );
                            ui.painter().text(
                                rect.left_top() + Vec2::new(SP_4, 58.0),
                                Align2::LEFT_TOP,
                                title,
                                FontId::proportional(16.0),
                                th.p.text,
                            );
                            let galley = ui.painter().layout(
                                body.to_string(),
                                FontId::proportional(12.5),
                                th.p.text_dim,
                                w - 2.0 * SP_4,
                            );
                            ui.painter().galley(
                                rect.left_top() + Vec2::new(SP_4, 84.0),
                                galley,
                                th.p.text_dim,
                            );
                            if resp.clicked() {
                                self.settings.role = role;
                            }
                        }
                    });
                }
                _ => {
                    ui.label(RichText::new(ph::CHECK_CIRCLE).size(40.0).color(th.p.ok));
                    ui.add_space(SP_2);
                    ui.label(RichText::new(tr.t("ob_ready")).size(26.0).strong());
                    ui.add_space(SP_2);
                    let sub = match self.settings.role {
                        Role::Controller => tr.t("ob_ready_ctrl"),
                        Role::Target => tr.t("ob_ready_target"),
                    };
                    ui.label(RichText::new(sub).color(th.p.text_dim));
                    ui.add_space(SP_4);
                    widgets::card(ui, &th, |ui| {
                        ui.label(RichText::new(
                            tr.f("ob_tip_hotkey", &[&self.settings.hotkey().to_string()]),
                        ));
                        ui.add_space(SP_2);
                        ui.label(RichText::new(tr.t("ob_tip_settings")).color(th.p.text_dim));
                    });
                }
            }

            ui.add_space(SP_6);
            ui.horizontal(|ui| {
                widgets::right(ui, |ui| {
                    if step + 1 < STEPS {
                        if widgets::primary_button(ui, &th, tr.t("next")).clicked() {
                            next = true;
                        }
                    } else {
                        let label = match self.settings.role {
                            Role::Controller => tr.t("ob_finish_ctrl"),
                            Role::Target => tr.t("ob_finish_target"),
                        };
                        if widgets::primary_button(ui, &th, label).clicked() {
                            finish = true;
                        }
                    }
                    if step > 0 && widgets::secondary_button(ui, &th, tr.t("back")).clicked() {
                        back = true;
                    }
                });
            });
        });

        if let Some(ob) = self.onboarding.as_mut() {
            if next {
                ob.step += 1;
            }
            if back {
                ob.step = ob.step.saturating_sub(1);
            }
        }
        if finish || skip {
            self.onboarding = None;
            self.settings.onboarded = true;
            let _ = self.settings.save();
            if finish && self.settings.role == Role::Controller {
                self.scanner.scan();
                self.add = Some(AddDialog::default());
            }
        }
    }
}
