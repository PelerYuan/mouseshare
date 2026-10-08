//! The settings window.

use eframe::egui::{self, RichText, Sense, Stroke, Vec2};
use egui_phosphor::regular as ph;
use mouseshare_config::{Accent, ThemeMode};
use mouseshare_core::Hotkey;

use crate::app::{App, ToastKind};
use crate::dialogs::modal;
use crate::theme::{accent_color, SP_1, SP_2, SP_3};
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Input,
    Clipboard,
    Security,
    About,
}

impl App {
    pub fn settings_window(&mut self, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        let Some(mut tab) = self.settings_open else {
            return;
        };
        let mut close = false;
        let mut reopen_welcome = false;

        let esc = modal(ctx, &th, "settings", 620.0, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr.t("settings")).size(18.0).strong());
                widgets::right(ui, |ui| {
                    if widgets::icon_button(ui, &th, ph::X, tr.t("close")).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(SP_2);
            let tabs = [
                (SettingsTab::General, tr.t("tab_general")),
                (SettingsTab::Input, tr.t("tab_input")),
                (SettingsTab::Clipboard, tr.t("tab_clipboard")),
                (SettingsTab::Security, tr.t("tab_security")),
                (SettingsTab::About, tr.t("tab_about")),
            ];
            let labels: Vec<&str> = tabs.iter().map(|t| t.1).collect();
            let idx = tabs.iter().position(|t| t.0 == tab).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &th, &labels, idx) {
                tab = tabs[i].0;
            }
            ui.add_space(SP_3);
            if self.running() {
                ui.label(
                    RichText::new(format!("{}  {}", ph::INFO, tr.t("apply_on_restart")))
                        .size(12.0)
                        .color(th.p.warn),
                );
                ui.add_space(SP_2);
            }
            egui::ScrollArea::vertical()
                .max_height(
                    (ctx.screen_rect().height() / ctx.zoom_factor() - 190.0).clamp(240.0, 460.0),
                )
                .auto_shrink([false, true])
                .show(ui, |ui| match tab {
                    SettingsTab::General => self.tab_general(ui, &mut reopen_welcome),
                    SettingsTab::Input => self.tab_input(ui),
                    SettingsTab::Clipboard => self.tab_clipboard(ui),
                    SettingsTab::Security => self.tab_security(ui, ctx),
                    SettingsTab::About => self.tab_about(ui),
                });
        });

        if reopen_welcome {
            self.settings_open = None;
            self.onboarding = Some(crate::onboarding::Onboarding::default());
        } else if close || esc {
            self.settings_open = None;
        } else {
            self.settings_open = Some(tab);
        }
    }

    fn tab_general(&mut self, ui: &mut egui::Ui, reopen_welcome: &mut bool) {
        let th = self.theme;
        let tr = self.tr;

        let langs = self.language_choices();
        let cur = langs
            .iter()
            .find(|(l, _)| *l == self.settings.language)
            .map(|(_, n)| *n)
            .unwrap_or("English");
        widgets::setting_row(ui, &th, tr.t("language"), None, |ui| {
            egui::ComboBox::from_id_salt("lang")
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    for (l, name) in &langs {
                        ui.selectable_value(&mut self.settings.language, *l, *name);
                    }
                });
        });
        widgets::setting_row(ui, &th, tr.t("theme"), None, |ui| {
            let labels = [tr.t("theme_auto"), tr.t("theme_dark"), tr.t("theme_light")];
            let idx = match self.settings.theme {
                ThemeMode::Auto => 0,
                ThemeMode::Dark => 1,
                ThemeMode::Light => 2,
            };
            if let Some(i) = widgets::segmented(ui, &th, &labels, idx) {
                self.settings.theme = [ThemeMode::Auto, ThemeMode::Dark, ThemeMode::Light][i];
            }
        });
        widgets::setting_row(ui, &th, tr.t("accent"), None, |ui| {
            for a in Accent::ALL.iter().rev() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                let c = accent_color(*a, th.dark);
                ui.painter().circle_filled(rect.center(), 9.0, c);
                if *a == self.settings.accent {
                    ui.painter().circle_stroke(
                        rect.center(),
                        11.5,
                        Stroke::new(2.0_f32, th.p.text),
                    );
                }
                if resp.clicked() {
                    self.settings.accent = *a;
                }
            }
        });
        widgets::setting_row(ui, &th, tr.t("ui_scale"), None, |ui| {
            ui.add(
                egui::Slider::new(&mut self.settings.ui_scale, 0.8..=1.6)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        });
        ui.add_space(SP_2);
        widgets::divider(ui, &th);
        ui.add_space(SP_3);

        let mut autostart = self.settings.autostart;
        widgets::setting_row(
            ui,
            &th,
            tr.t("autostart"),
            Some(tr.t("autostart_hint")),
            |ui| {
                widgets::toggle(ui, &th, &mut autostart);
            },
        );
        if autostart != self.settings.autostart {
            let exe = std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "mouseshare-gui".into());
            match mouseshare_config::set_autostart(autostart, &exe) {
                Ok(()) => self.settings.autostart = autostart,
                Err(e) => self.toast(ToastKind::Error, format!("{e}")),
            }
        }
        widgets::setting_row(
            ui,
            &th,
            tr.t("start_on_launch"),
            Some(tr.t("start_on_launch_hint")),
            |ui| {
                widgets::toggle(ui, &th, &mut self.settings.start_sharing_on_launch);
            },
        );
        ui.add_space(SP_2);
        widgets::divider(ui, &th);
        ui.add_space(SP_3);
        if widgets::secondary_button(ui, &th, tr.t("show_welcome")).clicked() {
            *reopen_welcome = true;
        }
    }

    fn tab_input(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let tr = self.tr;
        widgets::setting_row(ui, &th, tr.t("hotkey"), Some(tr.t("hotkey_hint")), |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.hotkey_text)
                    .desired_width(140.0)
                    .margin(Vec2::new(8.0, 6.0)),
            );
            if r.changed() {
                if let Ok(h) = Hotkey::parse(&self.hotkey_text) {
                    self.settings.hotkey = h.to_string();
                }
            }
            if Hotkey::parse(&self.hotkey_text).is_err() {
                ui.label(RichText::new(ph::WARNING).color(th.p.danger));
            }
        });
        widgets::setting_row(
            ui,
            &th,
            tr.t("edge_dwell"),
            Some(tr.t("edge_dwell_hint")),
            |ui| {
                ui.add(
                    egui::Slider::new(&mut self.settings.edge_dwell_ms, 0..=1000)
                        .custom_formatter(|v, _| format!("{v:.0} ms")),
                );
            },
        );
        widgets::setting_row(
            ui,
            &th,
            tr.t("hold_guard"),
            Some(tr.t("hold_guard_hint")),
            |ui| {
                widgets::toggle(ui, &th, &mut self.settings.hold_guard);
            },
        );
        widgets::setting_row(
            ui,
            &th,
            tr.t("natural_scroll"),
            Some(tr.t("natural_scroll_hint")),
            |ui| {
                widgets::toggle(ui, &th, &mut self.settings.natural_scroll);
            },
        );
    }

    fn tab_clipboard(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let tr = self.tr;
        widgets::setting_row(
            ui,
            &th,
            tr.t("clip_sync"),
            Some(tr.t("clip_sync_hint")),
            |ui| {
                widgets::toggle(ui, &th, &mut self.settings.clipboard);
            },
        );
        ui.add_enabled_ui(self.settings.clipboard, |ui| {
            widgets::setting_row(
                ui,
                &th,
                tr.t("clip_max"),
                Some(tr.t("clip_max_hint")),
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.settings.clipboard_max_kb, 16..=900)
                            .logarithmic(true)
                            .custom_formatter(|v, _| format!("{v:.0} KB")),
                    );
                },
            );
        });
    }

    fn tab_security(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let th = self.theme;
        let tr = self.tr;
        widgets::card(ui, &th, |ui| {
            ui.label(
                RichText::new(format!("{}  {}", ph::SHIELD_CHECK, tr.t("sec_title"))).strong(),
            );
            ui.add_space(SP_1);
            ui.label(
                RichText::new(tr.t("sec_body"))
                    .size(12.5)
                    .color(th.p.text_dim),
            );
        });
        ui.add_space(SP_3);

        let running = self.running();
        let code = self.pairing.own_code().display();
        widgets::setting_row(
            ui,
            &th,
            tr.t("sec_own_code"),
            Some(tr.t("sec_own_code_hint")),
            |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    if widgets::icon_button(ui, &th, ph::ARROWS_CLOCKWISE, tr.t("new_code"))
                        .clicked()
                    {
                        self.pairing.regenerate_own();
                    }
                });
                if widgets::icon_button(ui, &th, ph::COPY, tr.t("copy")).clicked() {
                    self.copy(ctx, code.clone());
                }
                ui.label(RichText::new(&code).monospace());
            },
        );
        widgets::setting_row(ui, &th, tr.t("port"), Some(tr.t("port_hint")), |ui| {
            ui.add_enabled(
                !running,
                egui::DragValue::new(&mut self.settings.listen_port).range(1024..=65535),
            );
        });
        ui.add_space(SP_2);
        ui.label(
            RichText::new(format!(
                "{}  {}",
                ph::FOLDER,
                mouseshare_config::config_dir().display()
            ))
            .size(12.0)
            .color(th.p.text_faint),
        );
    }

    fn tab_about(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let tr = self.tr;
        ui.horizontal(|ui| {
            ui.label(RichText::new(ph::CURSOR_CLICK).size(34.0).color(th.accent));
            ui.vertical(|ui| {
                ui.label(RichText::new("mouseshare").size(20.0).strong());
                ui.label(
                    RichText::new(format!("{} {}", tr.t("version"), env!("CARGO_PKG_VERSION")))
                        .color(th.p.text_dim),
                );
            });
        });
        ui.add_space(SP_3);
        ui.label(RichText::new(tr.t("about_body")).color(th.p.text_dim));
        ui.add_space(SP_3);
        ui.hyperlink_to(
            format!("{}  {}", ph::GITHUB_LOGO, "github.com/PelerYuan/mouseshare"),
            "https://github.com/PelerYuan/mouseshare",
        );
        ui.add_space(SP_2);
        ui.label(
            RichText::new(tr.t("about_license"))
                .size(12.0)
                .color(th.p.text_faint),
        );
    }
}
