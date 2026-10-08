//! Modal dialogs: add a device, confirm removal. Also hosts the shared
//! modal frame used by the settings window and the onboarding wizard.

use eframe::egui::{self, Align2, Color32, Id, Order, RichText, Sense, Stroke, Vec2};
use egui_phosphor::regular as ph;
use mouseshare_core::PairingCode;

use crate::app::App;
use crate::theme::{Theme, SP_1, SP_2, SP_3, SP_4};
use crate::widgets;

#[derive(Default)]
pub struct AddDialog {
    pub name: String,
    pub addr: String,
    pub code: String,
}

/// Dims everything behind and shows `add` in a centred window. Returns true
/// when the user asked to close (Escape).
pub fn modal(
    ctx: &egui::Context,
    th: &Theme,
    id: &'static str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> bool {
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new((id, "scrim")))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(
                rect,
                0.0,
                Color32::from_black_alpha(if th.dark { 150 } else { 90 }),
            );
        });
    egui::Window::new(id)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(width)
        .frame(
            egui::Frame::none()
                .fill(th.p.surface)
                .stroke(Stroke::new(1.0_f32, th.p.border))
                .rounding(14.0)
                .inner_margin(egui::Margin::same(SP_4 + SP_1))
                .shadow(egui::epaint::Shadow {
                    offset: Vec2::new(0.0, 12.0),
                    blur: 32.0,
                    spread: 0.0,
                    color: Color32::from_black_alpha(110),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(width - 2.0 * (SP_4 + SP_1));
            add(ui);
        });
    ctx.input(|i| i.key_pressed(egui::Key::Escape))
}

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        if self.onboarding.is_some() {
            self.onboarding_window(ctx);
            return;
        }
        if self.settings_open.is_some() {
            self.settings_window(ctx);
        } else if self.add.is_some() {
            self.add_dialog(ctx);
        } else if self.confirm_remove.is_some() {
            self.confirm_dialog(ctx);
        }
    }

    fn add_dialog(&mut self, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let Some(mut dlg) = self.add.take() else {
            return;
        };
        if self.scanner.is_scanning() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let found: Vec<_> = self
            .scanner
            .peers()
            .into_iter()
            .filter(|p| !self.settings.devices.iter().any(|d| d.id == p.screen_id))
            .collect();

        let name = mouseshare_config::sanitize_device_name(&dlg.name);
        let parsed = PairingCode::parse(&dlg.code);
        let addr_trim = dlg.addr.trim().to_string();
        let addr_ok = addr_trim.is_empty() || addr_trim.parse::<std::net::SocketAddr>().is_ok();
        let dup = self.settings.devices.iter().any(|d| d.id == name);
        let valid = !name.is_empty() && !dup && parsed.is_ok() && addr_ok;

        let mut close = false;
        let mut confirm = false;
        let esc = modal(ctx, &th, "add_device", 460.0, |ui| {
            ui.label(RichText::new(tr.t("add_device")).size(18.0).strong());
            ui.add_space(SP_1);
            ui.label(RichText::new(tr.t("add_sub")).color(th.p.text_dim));
            ui.add_space(SP_3);

            ui.horizontal(|ui| {
                widgets::section_label(ui, &th, tr.t("found_on_lan"));
                widgets::right(ui, |ui| {
                    if self.scanner.is_scanning() {
                        ui.add(egui::Spinner::new().size(14.0));
                    } else if widgets::icon_button(ui, &th, ph::ARROWS_CLOCKWISE, tr.t("rescan"))
                        .clicked()
                    {
                        self.scanner.scan();
                    }
                });
            });
            if found.is_empty() {
                ui.label(
                    RichText::new(if self.scanner.is_scanning() {
                        tr.t("scanning")
                    } else {
                        tr.t("none_found")
                    })
                    .size(12.5)
                    .color(th.p.text_faint),
                );
            }
            for p in &found {
                let label = format!("{}   {}", p.screen_id, p.addr);
                let r = ui.add(
                    egui::Button::new(RichText::new(label))
                        .fill(th.p.raised)
                        .stroke(Stroke::NONE)
                        .min_size(Vec2::new(ui.available_width(), 32.0)),
                );
                if r.clicked() {
                    dlg.name = p.screen_id.clone();
                    dlg.addr = p.addr.to_string();
                }
            }
            ui.add_space(SP_3);

            ui.label(
                RichText::new(tr.t("device_name"))
                    .size(12.0)
                    .color(th.p.text_dim),
            );
            ui.add(
                egui::TextEdit::singleline(&mut dlg.name)
                    .desired_width(f32::INFINITY)
                    .margin(Vec2::new(8.0, 6.0)),
            );
            if dup {
                ui.label(
                    RichText::new(tr.t("err_dup_name"))
                        .size(12.0)
                        .color(th.p.danger),
                );
            }
            ui.add_space(SP_2);
            ui.label(
                RichText::new(tr.t("address_optional"))
                    .size(12.0)
                    .color(th.p.text_dim),
            );
            ui.add(
                egui::TextEdit::singleline(&mut dlg.addr)
                    .desired_width(f32::INFINITY)
                    .hint_text(RichText::new(tr.t("address_hint")).color(th.p.text_faint))
                    .margin(Vec2::new(8.0, 6.0)),
            );
            if !addr_ok {
                ui.label(
                    RichText::new(tr.t("err_addr_format"))
                        .size(12.0)
                        .color(th.p.danger),
                );
            }
            ui.add_space(SP_2);
            ui.label(
                RichText::new(tr.t("pairing_code"))
                    .size(12.0)
                    .color(th.p.text_dim),
            );
            ui.add(
                egui::TextEdit::singleline(&mut dlg.code)
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::monospace(15.0))
                    .hint_text(RichText::new("XXXXX-XXXXX").color(th.p.text_faint))
                    .margin(Vec2::new(8.0, 6.0)),
            );
            if !dlg.code.trim().is_empty() && parsed.is_err() {
                ui.label(
                    RichText::new(tr.t("code_invalid"))
                        .size(12.0)
                        .color(th.p.danger),
                );
            } else {
                ui.label(
                    RichText::new(tr.t("code_where"))
                        .size(12.0)
                        .color(th.p.text_faint),
                );
            }

            ui.add_space(SP_4);
            ui.horizontal(|ui| {
                widgets::right(ui, |ui| {
                    ui.add_enabled_ui(valid, |ui| {
                        if widgets::primary_button(ui, &th, tr.t("add")).clicked() {
                            confirm = true;
                        }
                    });
                    if widgets::secondary_button(ui, &th, tr.t("cancel")).clicked() {
                        close = true;
                    }
                });
            });
        });

        if confirm && valid {
            if let Ok(code) = parsed {
                let addr = if addr_trim.is_empty() {
                    None
                } else {
                    Some(addr_trim)
                };
                self.add_remote(name, addr, code);
            }
            return;
        }
        if close || esc {
            return;
        }
        self.add = Some(dlg);
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let Some(id) = self.confirm_remove.clone() else {
            return;
        };
        let mut yes = false;
        let mut no = false;
        let esc = modal(ctx, &th, "confirm_remove", 400.0, |ui| {
            ui.label(
                RichText::new(tr.f("remove_title", &[&id]))
                    .size(17.0)
                    .strong(),
            );
            ui.add_space(SP_2);
            ui.label(RichText::new(tr.t("remove_body")).color(th.p.text_dim));
            ui.add_space(SP_4);
            ui.horizontal(|ui| {
                widgets::right(ui, |ui| {
                    if widgets::danger_button(ui, &th, tr.t("remove")).clicked() {
                        yes = true;
                    }
                    if widgets::secondary_button(ui, &th, tr.t("cancel")).clicked() {
                        no = true;
                    }
                });
            });
        });
        if yes {
            self.remove_remote(&id);
            self.confirm_remove = None;
        } else if no || esc {
            self.confirm_remove = None;
        }
    }
}
