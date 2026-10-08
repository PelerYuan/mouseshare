//! The left rail: role switch, device list, the selected device's details
//! and the pinned Start/Stop footer.

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};
use egui_phosphor::regular as ph;
use mouseshare_config::{DeviceRecord, Role};
use mouseshare_core::PairingCode;

use crate::app::App;
use crate::canvas::{self, Alignment, SnapAxis};
use crate::theme::{RADIUS, SP_1, SP_2, SP_3};
use crate::widgets;

const FOOTER_H: f32 = 118.0;
const ROW_H: f32 = 54.0;

/// "2 monitors · 5120×1440", or just the size for a single monitor.
pub fn monitors_summary(d: &DeviceRecord, tr: &crate::i18n::Tr) -> String {
    if d.monitors.is_empty() {
        return tr.t("monitors_unknown").to_string();
    }
    let (_, _, w, h) = canvas::local_bounds(d);
    if d.monitors.len() == 1 {
        format!("{w}\u{00d7}{h}")
    } else {
        tr.f(
            "monitors_n",
            &[&d.monitors.len().to_string(), &format!("{w}\u{00d7}{h}")],
        )
    }
}

impl App {
    pub fn rail(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();

        // Role switch.
        ui.add_enabled_ui(!running, |ui| {
            let labels = [tr.t("role_controller"), tr.t("role_target")];
            let idx = if self.settings.role == Role::Controller {
                0
            } else {
                1
            };
            if let Some(i) = widgets::segmented(ui, &th, &labels, idx) {
                self.settings.role = if i == 0 {
                    Role::Controller
                } else {
                    Role::Target
                };
            }
        });
        ui.add_space(SP_1);
        ui.label(
            RichText::new(match self.settings.role {
                Role::Controller => tr.t("role_controller_hint"),
                Role::Target => tr.t("role_target_hint"),
            })
            .size(12.0)
            .color(th.p.text_dim),
        );
        ui.add_space(SP_3);

        let body_h = (ui.available_height() - FOOTER_H).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("rail_scroll")
            .max_height(body_h)
            .auto_shrink([false, false])
            .show(ui, |ui| match self.settings.role {
                Role::Controller => self.controller_rail(ui, ctx),
                Role::Target => self.target_rail(ui),
            });

        self.footer(ui);
    }

    fn controller_rail(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();

        ui.horizontal(|ui| {
            widgets::section_label(ui, &th, tr.t("devices"));
            widgets::right(ui, |ui| {
                if ui
                    .add_enabled(
                        !running,
                        egui::Button::new(
                            RichText::new(format!("{}  {}", ph::PLUS, tr.t("add_device")))
                                .color(th.accent)
                                .strong(),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                    )
                    .clicked()
                {
                    self.scanner.scan();
                    self.add = Some(crate::dialogs::AddDialog::default());
                }
            });
        });
        ui.add_space(SP_1);

        let ids: Vec<String> = self.settings.devices.iter().map(|d| d.id.clone()).collect();
        let local = self.settings.device_name.clone();
        let mut remote_idx = 0usize;
        for id in &ids {
            let color = if *id == local {
                th.accent
            } else {
                let c = canvas::device_color(remote_idx);
                remote_idx += 1;
                c
            };
            self.device_row(ui, id, color);
        }

        ui.add_space(SP_3);
        if let Some(sel) = self.selected.clone() {
            if self.settings.device(&sel).is_some() {
                self.detail(ui, &sel);
            }
        } else {
            ui.add_space(SP_2);
            ui.label(
                RichText::new(tr.t("select_hint"))
                    .size(12.5)
                    .color(th.p.text_faint),
            );
        }
    }

    fn device_row(&mut self, ui: &mut egui::Ui, id: &str, color: Color32) {
        let th = self.theme;
        let tr = self.tr;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
        let selected = self.selected.as_deref() == Some(id);
        if selected {
            ui.painter().rect_filled(rect, RADIUS, th.accent_soft());
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, RADIUS, th.p.raised);
        }
        let Some(rec) = self.settings.device(id) else {
            return;
        };
        let vis = self.device_visual(id);
        let is_local = id == self.settings.device_name;
        let sw = egui::Rect::from_min_size(
            rect.left_top() + Vec2::new(SP_3, (ROW_H - 22.0) / 2.0),
            Vec2::new(30.0, 22.0),
        );
        ui.painter().rect_filled(sw, 5.0, color);
        let name_font = FontId::proportional(14.0);
        let max_w = rect.width() - 30.0 - SP_3 * 2.0 - 28.0 - SP_2;
        let name = canvas::fit_text(
            ui.painter(),
            id,
            &name_font,
            max_w - if is_local { 40.0 } else { 0.0 },
        );
        let x = sw.right() + SP_2 + 2.0;
        let name_rect = ui.painter().text(
            egui::pos2(x, rect.top() + 11.0),
            Align2::LEFT_TOP,
            &name,
            name_font,
            th.p.text,
        );
        if is_local {
            ui.painter().text(
                egui::pos2(name_rect.right() + 6.0, rect.top() + 13.0),
                Align2::LEFT_TOP,
                tr.t("you"),
                FontId::proportional(11.0),
                th.accent,
            );
        }
        let sub = if self.running() && !vis.status_text.is_empty() {
            vis.status_text.clone()
        } else {
            monitors_summary(rec, &tr)
        };
        let sub = canvas::fit_text(ui.painter(), &sub, &FontId::proportional(12.0), max_w);
        ui.painter().text(
            egui::pos2(x, rect.top() + 31.0),
            Align2::LEFT_TOP,
            sub,
            FontId::proportional(12.0),
            th.p.text_dim,
        );
        ui.painter().circle_filled(
            egui::pos2(rect.right() - SP_3 - 4.0, rect.center().y),
            4.0,
            vis.status_color,
        );
        if resp.clicked() {
            self.selected = Some(id.to_string());
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui, id: &str) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();
        let is_local = id == self.settings.device_name;

        widgets::card(ui, &th, |ui| {
            if is_local {
                ui.label(RichText::new(tr.t("this_computer")).strong());
                ui.add_space(SP_2);
                ui.label(
                    RichText::new(tr.t("device_name"))
                        .size(12.0)
                        .color(th.p.text_dim),
                );
                let r = ui.add_enabled(
                    !running,
                    egui::TextEdit::singleline(&mut self.name_text)
                        .desired_width(f32::INFINITY)
                        .margin(Vec2::new(8.0, 6.0)),
                );
                if r.lost_focus() {
                    let t = self.name_text.clone();
                    self.rename_local(&t);
                    self.name_text = self.settings.device_name.clone();
                }
            } else {
                self.remote_detail(ui, id);
            }

            if let Some(d) = self.settings.device(id) {
                if d.monitors.len() > 1 {
                    ui.add_space(SP_2);
                    ui.label(
                        RichText::new(tr.t("monitors"))
                            .size(12.0)
                            .color(th.p.text_dim),
                    );
                    for (i, m) in d.monitors.iter().enumerate() {
                        let name = if m.name.is_empty() {
                            format!("{}", i + 1)
                        } else {
                            m.name.clone()
                        };
                        ui.label(
                            RichText::new(format!(
                                "{name}  \u{00b7}  {}\u{00d7}{}",
                                m.width, m.height
                            ))
                            .size(12.5),
                        );
                    }
                }
            }

            if !running {
                if let Some(axis) = self.canvas.axis(id) {
                    ui.add_space(SP_3);
                    widgets::divider(ui, &th);
                    ui.add_space(SP_2);
                    ui.label(RichText::new(tr.t("align")).size(12.0).color(th.p.text_dim));
                    let (a, b, c) = match axis {
                        SnapAxis::Horizontal => (
                            tr.t("align_top"),
                            tr.t("align_middle"),
                            tr.t("align_bottom"),
                        ),
                        SnapAxis::Vertical => (
                            tr.t("align_left"),
                            tr.t("align_center"),
                            tr.t("align_right"),
                        ),
                    };
                    ui.horizontal(|ui| {
                        for (label, al) in [
                            (a, Alignment::Start),
                            (b, Alignment::Center),
                            (c, Alignment::End),
                        ] {
                            if widgets::secondary_button(ui, &th, label).clicked() {
                                self.canvas.align(&mut self.settings.devices, id, al);
                            }
                        }
                    });
                }
            }
        });
    }

    fn remote_detail(&mut self, ui: &mut egui::Ui, id: &str) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();
        ui.horizontal(|ui| {
            ui.label(RichText::new(id).strong());
            widgets::right(ui, |ui| {
                if ui
                    .add_enabled(
                        !running,
                        egui::Button::new(RichText::new(ph::TRASH).size(16.0).color(th.p.danger))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                    )
                    .on_hover_text(tr.t("remove_device"))
                    .clicked()
                {
                    self.confirm_remove = Some(id.to_string());
                }
            });
        });
        ui.add_space(SP_2);

        // Address.
        ui.label(
            RichText::new(tr.t("address"))
                .size(12.0)
                .color(th.p.text_dim),
        );
        if let Some(rec) = self.settings.device_mut(id) {
            let mut text = rec.addr.clone().unwrap_or_default();
            let r = ui.add_enabled(
                !running,
                egui::TextEdit::singleline(&mut text)
                    .desired_width(f32::INFINITY)
                    .hint_text(RichText::new(tr.t("address_hint")).color(th.p.text_faint))
                    .margin(Vec2::new(8.0, 6.0)),
            );
            if r.changed() {
                let t = text.trim().to_string();
                rec.addr = if t.is_empty() { None } else { Some(t) };
            }
        }
        ui.add_space(SP_2);

        // Pairing code.
        ui.label(
            RichText::new(tr.t("pairing_code"))
                .size(12.0)
                .color(th.p.text_dim),
        );
        let stored = self.pairing.peer_code(id).map(|c| c.display());
        let entry = self
            .code_edits
            .entry(id.to_string())
            .or_insert_with(|| stored.clone().unwrap_or_default());
        let r = ui.add_enabled(
            !running,
            egui::TextEdit::singleline(entry)
                .desired_width(f32::INFINITY)
                .font(FontId::monospace(14.0))
                .hint_text(RichText::new("XXXXX-XXXXX").color(th.p.text_faint))
                .margin(Vec2::new(8.0, 6.0)),
        );
        let parsed = PairingCode::parse(entry);
        if r.changed() {
            if let Ok(code) = &parsed {
                self.pairing.set_peer_code(id, code);
            }
        }
        match (&parsed, entry.trim().is_empty()) {
            (Ok(_), _) => {
                ui.label(
                    RichText::new(format!("{} {}", ph::CHECK_CIRCLE, tr.t("code_ok")))
                        .size(12.0)
                        .color(th.p.ok),
                );
            }
            (Err(_), true) => {
                ui.label(
                    RichText::new(format!("{} {}", ph::WARNING, tr.t("code_missing")))
                        .size(12.0)
                        .color(th.p.warn),
                );
            }
            (Err(_), false) => {
                ui.label(
                    RichText::new(format!("{} {}", ph::WARNING, tr.t("code_invalid")))
                        .size(12.0)
                        .color(th.p.danger),
                );
            }
        }
        ui.add_space(SP_2);

        // Pointer speed.
        ui.label(
            RichText::new(tr.t("pointer_speed"))
                .size(12.0)
                .color(th.p.text_dim),
        );
        if let Some(rec) = self.settings.device_mut(id) {
            ui.add(
                egui::Slider::new(&mut rec.sensitivity, 0.25..=4.0)
                    .logarithmic(true)
                    .custom_formatter(|v, _| format!("{v:.2}\u{00d7}"))
                    .text(""),
            );
        }
    }

    fn target_rail(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();
        widgets::section_label(ui, &th, tr.t("this_computer"));
        ui.add_space(SP_1);
        widgets::card(ui, &th, |ui| {
            ui.label(
                RichText::new(tr.t("device_name"))
                    .size(12.0)
                    .color(th.p.text_dim),
            );
            let r = ui.add_enabled(
                !running,
                egui::TextEdit::singleline(&mut self.name_text)
                    .desired_width(f32::INFINITY)
                    .margin(Vec2::new(8.0, 6.0)),
            );
            if r.lost_focus() {
                let t = self.name_text.clone();
                self.rename_local(&t);
                self.name_text = self.settings.device_name.clone();
            }
            ui.add_space(SP_2);
            ui.label(RichText::new(tr.t("port")).size(12.0).color(th.p.text_dim));
            ui.add_enabled(
                !running,
                egui::DragValue::new(&mut self.settings.listen_port).range(1024..=65535),
            );
            if let Some(d) = self.settings.device(&self.settings.device_name) {
                ui.add_space(SP_2);
                ui.label(
                    RichText::new(tr.t("monitors"))
                        .size(12.0)
                        .color(th.p.text_dim),
                );
                ui.label(monitors_summary(d, &tr));
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let tr = self.tr;
        let running = self.running();
        ui.add_space(SP_2);
        widgets::divider(ui, &th);
        ui.add_space(SP_3);
        let w = ui.available_width();
        if running {
            let b = egui::Button::new(
                RichText::new(format!("{}  {}", ph::STOP, tr.t("stop")))
                    .color(Color32::WHITE)
                    .strong()
                    .size(15.0),
            )
            .fill(th.p.danger)
            .stroke(Stroke::NONE);
            if ui.add_sized([w, 40.0], b).clicked() {
                self.stop();
            }
        } else {
            let b = egui::Button::new(
                RichText::new(format!("{}  {}", ph::PLAY, tr.t("start")))
                    .color(th.on_accent())
                    .strong()
                    .size(15.0),
            )
            .fill(th.accent)
            .stroke(Stroke::NONE);
            if ui.add_sized([w, 40.0], b).clicked() {
                self.start();
            }
        }
        ui.add_space(SP_2);

        let snap = self.tele.snapshot();
        let (text, color) = if let Some(e) = &self.run_error {
            (e.clone(), th.p.danger)
        } else if !running {
            (
                match self.settings.role {
                    Role::Controller => tr.t("hint_idle_controller").to_string(),
                    Role::Target => tr.t("hint_idle_target").to_string(),
                },
                th.p.text_dim,
            )
        } else {
            match self.settings.role {
                Role::Controller => {
                    let total = snap
                        .peers
                        .len()
                        .max(self.settings.devices.len().saturating_sub(1));
                    let up = snap
                        .peers
                        .values()
                        .filter(|p| p.status == mouseshare_core::PeerStatus::Connected)
                        .count();
                    (
                        tr.f(
                            "hint_running_controller",
                            &[
                                &up.to_string(),
                                &total.to_string(),
                                &self.settings.hotkey().to_string(),
                            ],
                        ),
                        th.p.text_dim,
                    )
                }
                Role::Target => (
                    match &snap.controller {
                        Some(c) => tr.f("target_controlled_by", &[c]),
                        None => tr.t("target_waiting").to_string(),
                    },
                    th.p.text_dim,
                ),
            }
        };
        ui.label(RichText::new(text).size(12.0).color(color));
    }
}
