//! Application state, the per-frame layout and the glue between the UI and
//! the background session.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Vec2};
use egui_phosphor::regular as ph;
use mouseshare_config::{
    DeviceRecord, Language, MonitorRecord, PairingStore, Role, Settings, ThemeMode,
};
use mouseshare_core::{Notice, PairingCode, PeerStatus, TargetSpec, Telemetry};
use mouseshare_protocol::MonitorInfo;

use crate::canvas::{self, CanvasState, DeviceVisual};
use crate::dialogs::AddDialog;
use crate::i18n::{Lang, Tr};
use crate::log_buffer::LogBuffer;
use crate::onboarding::Onboarding;
use crate::scan::Scanner;
use crate::session::{self, Session};
use crate::settings_view::SettingsTab;
use crate::theme::{Theme, SP_2, SP_3, SP_4};
use crate::widgets;

pub const RAIL_WIDTH: f32 = 320.0;
const TOAST_LIFE: Duration = Duration::from_millis(4200);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub born: Instant,
}

pub struct App {
    pub settings: Settings,
    pub saved: Settings,
    pub pairing: PairingStore,
    pub tr: Tr,
    pub theme: Theme,
    pub rt: tokio::runtime::Runtime,
    pub session: Option<Session>,
    pub tele: Telemetry,
    pub selected: Option<String>,
    pub canvas: CanvasState,
    pub scanner: Scanner,
    pub add: Option<AddDialog>,
    pub settings_open: Option<SettingsTab>,
    pub onboarding: Option<Onboarding>,
    pub confirm_remove: Option<String>,
    pub toasts: Vec<Toast>,
    pub log: LogBuffer,
    pub log_open: bool,
    pub code_edits: HashMap<String, String>,
    pub hotkey_text: String,
    pub name_text: String,
    pub run_error: Option<String>,
    pub copied_at: Option<Instant>,
    pub lan_ip: Option<Ipv4Addr>,

    applied: Option<(ThemeMode, mouseshare_config::Accent, bool, u32, Lang)>,
    system_lang: Lang,
    next_local_poll: Instant,
    dirty_since: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, log: LogBuffer) -> Self {
        let mut settings = Settings::load();
        let pairing = PairingStore::load();
        let system_lang = Lang::from_env();
        let lang = Lang::resolve(settings.language, system_lang);
        crate::fonts::install(&cc.egui_ctx, lang == Lang::ZhCn);

        // Multi-thread (one worker): a current_thread runtime only polls
        // spawned tasks while something block_on's it, which eframe never does.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .enable_io()
            .build()
            .expect("failed to build the background tokio runtime");

        let first_run = !settings.onboarded;
        let mut app = Self {
            hotkey_text: settings.hotkey.clone(),
            name_text: settings.device_name.clone(),
            saved: settings.clone(),
            settings: {
                ensure_local(&mut settings);
                settings
            },
            pairing,
            tr: Tr::new(lang),
            theme: Theme::resolve(ThemeMode::Auto, mouseshare_config::Accent::Blue, None),
            rt,
            session: None,
            tele: Telemetry::new(),
            selected: None,
            canvas: CanvasState::default(),
            scanner: Scanner::default(),
            add: None,
            settings_open: None,
            onboarding: first_run.then(Onboarding::default),
            confirm_remove: None,
            toasts: Vec::new(),
            log,
            log_open: false,
            code_edits: HashMap::new(),
            run_error: None,
            copied_at: None,
            lan_ip: crate::scan::lan_ip(),
            applied: None,
            system_lang,
            next_local_poll: Instant::now(),
            dirty_since: None,
        };
        app.refresh_local_monitors();
        if app.settings.start_sharing_on_launch && app.settings.onboarded {
            app.start();
        }
        app
    }

    pub fn running(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.is_finished())
    }

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            born: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn copy(&mut self, ctx: &egui::Context, text: String) {
        ctx.copy_text(text);
        self.copied_at = Some(Instant::now());
        let msg = self.tr.t("toast_copied");
        self.toast(ToastKind::Info, msg);
    }

    // ---- session control ------------------------------------------------

    pub fn start(&mut self) {
        if self.running() {
            return;
        }
        self.run_error = None;
        self.tele.clear();
        let opts = self.settings.session_options();
        match self.settings.role {
            Role::Target => {
                let code = self.pairing.own_code();
                let listen =
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.settings.listen_port);
                self.session = Some(session::start_target(
                    &self.rt,
                    self.settings.device_name.clone(),
                    listen,
                    code,
                    opts,
                    self.tele.clone(),
                ));
            }
            Role::Controller => {
                let layout = match self.settings.to_layout() {
                    Ok(l) => l,
                    Err(e) => {
                        self.fail(format!("{e}"));
                        return;
                    }
                };
                let remotes: Vec<String> = layout.remote_ids().map(str::to_string).collect();
                if remotes.is_empty() {
                    let m = self.tr.t("err_no_devices");
                    self.fail(m.to_string());
                    return;
                }
                if canvas::any_overlap(&self.settings.devices) {
                    let m = self.tr.t("err_overlap");
                    self.fail(m.to_string());
                    return;
                }
                let mut targets = HashMap::new();
                for id in remotes {
                    let Some(code) = self.pairing.peer_code(&id) else {
                        let m = self.tr.f("err_no_code", &[&id]);
                        self.selected = Some(id);
                        self.fail(m);
                        return;
                    };
                    let addr = match self
                        .settings
                        .device(&id)
                        .and_then(|d| d.addr.as_deref())
                        .map(str::trim)
                        .filter(|a| !a.is_empty())
                    {
                        Some(text) => match text.parse::<SocketAddr>() {
                            Ok(a) => Some(a),
                            Err(_) => {
                                let m = self.tr.f("err_bad_addr", &[&id, text]);
                                self.selected = Some(id);
                                self.fail(m);
                                return;
                            }
                        },
                        None => None,
                    };
                    targets.insert(id, TargetSpec { addr, code });
                }
                self.session = Some(session::start_controller(
                    &self.rt,
                    layout,
                    targets,
                    opts,
                    self.tele.clone(),
                ));
            }
        }
    }

    fn fail(&mut self, msg: String) {
        self.toast(ToastKind::Error, msg.clone());
        self.run_error = Some(msg);
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.session.take() {
            s.stop();
        }
        self.tele.clear();
    }

    // ---- data helpers -----------------------------------------------------

    fn refresh_local_monitors(&mut self) {
        let mons = match mouseshare_x11input::LocalCursor::connect(None) {
            Ok(c) => {
                let m = c.monitors().unwrap_or_default();
                if m.is_empty() {
                    let (w, h) = c.screen_size();
                    vec![MonitorRecord {
                        name: String::new(),
                        x: 0,
                        y: 0,
                        width: w,
                        height: h,
                    }]
                } else {
                    m.into_iter()
                        .map(|m| MonitorRecord {
                            name: m.name,
                            x: m.x,
                            y: m.y,
                            width: m.width,
                            height: m.height,
                        })
                        .collect()
                }
            }
            Err(_) => return,
        };
        let name = self.settings.device_name.clone();
        if let Some(d) = self.settings.device_mut(&name) {
            if d.monitors != mons {
                d.monitors = mons;
            }
        }
    }

    pub fn add_remote(&mut self, name: String, addr: Option<String>, code: PairingCode) {
        let mut rec = DeviceRecord {
            id: name.clone(),
            x: 0,
            y: 0,
            monitors: Vec::new(),
            addr,
            sensitivity: 1.0,
        };
        let (x, y) = canvas::default_origin(&self.settings.devices, &rec);
        rec.x = x;
        rec.y = y;
        self.settings.devices.retain(|d| d.id != name);
        self.settings.devices.push(rec);
        self.pairing.set_peer_code(&name, &code);
        self.selected = Some(name);
    }

    pub fn remove_remote(&mut self, id: &str) {
        self.settings.devices.retain(|d| d.id != id);
        self.pairing.forget_peer(id);
        self.code_edits.remove(id);
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
    }

    /// Renames this computer, keeping the arrangement entry in step.
    pub fn rename_local(&mut self, new_name: &str) {
        let clean = mouseshare_config::sanitize_device_name(new_name);
        if clean.is_empty() || clean == self.settings.device_name {
            return;
        }
        if self.settings.devices.iter().any(|d| d.id == clean) {
            return;
        }
        let old = self.settings.device_name.clone();
        if let Some(d) = self.settings.device_mut(&old) {
            d.id = clean.clone();
        }
        if self.selected.as_deref() == Some(old.as_str()) {
            self.selected = Some(clean.clone());
        }
        self.settings.device_name = clean.clone();
        self.name_text = clean;
    }

    pub fn device_visual(&self, id: &str) -> DeviceVisual {
        let th = &self.theme;
        let tr = &self.tr;
        let snap = self.tele.snapshot();
        let running = self.running();
        if id == self.settings.device_name {
            return DeviceVisual {
                status_color: if running { th.p.ok } else { th.p.text_faint },
                status_text: if running
                    && snap.active.is_none()
                    && self.settings.role == Role::Controller
                {
                    tr.t("cursor_here").to_string()
                } else {
                    String::new()
                },
            };
        }
        if !running {
            return DeviceVisual {
                status_color: th.p.text_faint,
                status_text: tr.t("st_not_connected").to_string(),
            };
        }
        match snap.peers.get(id) {
            None => DeviceVisual {
                status_color: th.p.warn,
                status_text: tr.t("st_connecting").to_string(),
            },
            Some(p) => match &p.status {
                PeerStatus::Connecting => DeviceVisual {
                    status_color: th.p.warn,
                    status_text: tr.t("st_connecting").to_string(),
                },
                PeerStatus::Connected => {
                    let here = snap.active.as_deref() == Some(id);
                    let mut text = match p.latency_ms {
                        Some(ms) => tr.f("st_connected_ms", &[&ms.to_string()]),
                        None => tr.t("st_connected").to_string(),
                    };
                    if here {
                        text = tr.t("cursor_here").to_string();
                    }
                    DeviceVisual {
                        status_color: th.p.ok,
                        status_text: text,
                    }
                }
                PeerStatus::AuthFailed => DeviceVisual {
                    status_color: th.p.danger,
                    status_text: tr.t("st_wrong_code").to_string(),
                },
                PeerStatus::Failed(_) => DeviceVisual {
                    status_color: th.p.danger,
                    status_text: tr.t("st_retrying").to_string(),
                },
            },
        }
    }

    // ---- per-frame bookkeeping --------------------------------------------

    fn sync(&mut self, ctx: &egui::Context) {
        // Finished session (error or otherwise).
        if self.session.as_ref().is_some_and(|s| s.is_finished()) {
            if let Some(s) = self.session.take() {
                if let Some(e) = s.take_error() {
                    self.toast(ToastKind::Error, e.clone());
                    self.run_error = Some(e);
                }
            }
        }

        // Pull the real monitors devices reported into the arrangement.
        let snap = self.tele.snapshot();
        for (id, p) in &snap.peers {
            if p.monitors.is_empty() {
                continue;
            }
            let mons = to_records(&p.monitors);
            if let Some(d) = self.settings.device_mut(id) {
                if d.monitors != mons {
                    d.monitors = mons;
                }
            }
        }
        if !snap.local_monitors.is_empty() {
            let mons = to_records(&snap.local_monitors);
            let name = self.settings.device_name.clone();
            if let Some(d) = self.settings.device_mut(&name) {
                if d.monitors != mons {
                    d.monitors = mons;
                }
            }
        } else if !self.running() && Instant::now() >= self.next_local_poll {
            self.next_local_poll = Instant::now() + Duration::from_secs(5);
            self.refresh_local_monitors();
        }

        // Notices -> toasts.
        for n in self.tele.drain_notices() {
            let tr = self.tr;
            let (kind, text) = match n {
                Notice::PeerConnected(id) => (ToastKind::Success, tr.f("toast_connected", &[&id])),
                Notice::PeerLost(id) => (ToastKind::Warn, tr.f("toast_lost", &[&id])),
                Notice::PeerAuthFailed(id) => (ToastKind::Error, tr.f("toast_auth", &[&id])),
                Notice::ControllerConnected(id) => {
                    (ToastKind::Success, tr.f("toast_ctrl_in", &[&id]))
                }
                Notice::ControllerLeft(id) => (ToastKind::Info, tr.f("toast_ctrl_out", &[&id])),
                Notice::IncomingRejected(ip) => (ToastKind::Warn, tr.f("toast_rejected", &[&ip])),
                Notice::ReturnedByHotkey => (ToastKind::Info, tr.t("toast_hotkey").to_string()),
            };
            self.toast(kind, text);
        }

        // Theme / scale / language.
        let sys_dark = ctx.system_theme().map(|t| t == egui::Theme::Dark);
        let lang = Lang::resolve(self.settings.language, self.system_lang);
        let key = (
            self.settings.theme,
            self.settings.accent,
            sys_dark.unwrap_or(true),
            (self.settings.ui_scale * 100.0) as u32,
            lang,
        );
        if self.applied != Some(key) {
            let lang_changed = self.applied.map(|a| a.4) != Some(lang);
            self.theme = Theme::resolve(self.settings.theme, self.settings.accent, sys_dark);
            self.theme.apply(ctx);
            ctx.set_zoom_factor(self.settings.ui_scale);
            self.tr = Tr::new(lang);
            if lang_changed && self.applied.is_some() {
                crate::fonts::install(ctx, lang == Lang::ZhCn);
            }
            self.applied = Some(key);
        }

        // Autosave, debounced.
        self.settings.normalize();
        if self.settings != self.saved {
            let since = *self.dirty_since.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_millis(400) {
                if let Err(e) = self.settings.save() {
                    tracing::warn!("could not save settings: {e}");
                }
                self.saved = self.settings.clone();
                self.dirty_since = None;
            }
        } else {
            self.dirty_since = None;
        }

        self.toasts.retain(|t| t.born.elapsed() < TOAST_LIFE);
    }

    pub fn language_choices(&self) -> Vec<(Language, &'static str)> {
        let mut v = vec![
            (Language::Auto, self.tr.t("lang_auto")),
            (Language::En, "English"),
        ];
        if crate::fonts::cjk_available() {
            v.push((Language::ZhCn, "简体中文"));
        }
        v
    }
}

fn to_records(m: &[MonitorInfo]) -> Vec<MonitorRecord> {
    m.iter()
        .map(|m| MonitorRecord {
            name: m.name.clone(),
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
        })
        .collect()
}

/// Makes sure the arrangement contains this computer.
pub fn ensure_local(s: &mut Settings) {
    if s.device(&s.device_name.clone()).is_none() {
        s.devices.insert(
            0,
            DeviceRecord {
                id: s.device_name.clone(),
                x: 0,
                y: 0,
                monitors: Vec::new(),
                addr: None,
                sensitivity: 1.0,
            },
        );
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        self.sync(ctx);
        let th = self.theme;

        egui::TopBottomPanel::top("topbar")
            .exact_height(52.0)
            .frame(
                egui::Frame::none()
                    .fill(th.p.surface)
                    .inner_margin(egui::Margin::symmetric(SP_4, SP_2)),
            )
            .show(ctx, |ui| self.topbar(ui));

        if self.log_open {
            egui::TopBottomPanel::bottom("log")
                .exact_height(190.0)
                .frame(
                    egui::Frame::none()
                        .fill(th.p.surface)
                        .inner_margin(egui::Margin::same(SP_3)),
                )
                .show(ctx, |ui| self.log_view(ui));
        }

        egui::SidePanel::left("rail")
            .exact_width(RAIL_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(th.p.surface)
                    .inner_margin(egui::Margin::same(SP_4)),
            )
            .show(ctx, |ui| self.rail(ui, ctx));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(th.p.bg)
                    .inner_margin(egui::Margin::same(SP_4)),
            )
            .show(ctx, |ui| match self.settings.role {
                Role::Controller => self.canvas_view(ui),
                Role::Target => self.target_view(ui, ctx),
            });

        self.dialogs(ctx);
        self.toasts_ui(ctx);
    }
}

impl App {
    fn topbar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(ph::CURSOR_CLICK).size(22.0).color(th.accent));
            ui.label(RichText::new("mouseshare").size(17.0).strong());
            widgets::right(ui, |ui| {
                if widgets::icon_button(ui, &th, ph::GEAR_SIX, self.tr.t("settings")).clicked() {
                    self.settings_open = Some(SettingsTab::General);
                }
                let log_tip = self.tr.t("log");
                if widgets::icon_button(ui, &th, ph::TERMINAL_WINDOW, log_tip).clicked() {
                    self.log_open = !self.log_open;
                }
                ui.add_space(SP_2);
                let (color, text) = if self.running() {
                    (th.p.ok, self.tr.t("pill_active"))
                } else if self.run_error.is_some() {
                    (th.p.danger, self.tr.t("pill_error"))
                } else {
                    (th.p.text_faint, self.tr.t("pill_idle"))
                };
                widgets::status_pill(ui, &th, color, text);
            });
        });
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        ui.horizontal(|ui| {
            widgets::section_label(ui, &th, self.tr.t("log"));
            widgets::right(ui, |ui| {
                if widgets::icon_button(ui, &th, ph::X, self.tr.t("close")).clicked() {
                    self.log_open = false;
                }
            });
        });
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in self.log.snapshot() {
                    ui.label(
                        RichText::new(line)
                            .monospace()
                            .size(11.5)
                            .color(th.p.text_dim),
                    );
                }
            });
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        let th = self.theme;
        let screen = ctx.screen_rect();
        let mut y = screen.bottom() - 24.0;
        for (i, t) in self.toasts.iter().enumerate().rev() {
            let age = t.born.elapsed().as_secs_f32();
            let life = TOAST_LIFE.as_secs_f32();
            let alpha = ((life - age) / 0.4)
                .clamp(0.0, 1.0)
                .min((age / 0.15).clamp(0.0, 1.0));
            let color = match t.kind {
                ToastKind::Info => th.accent,
                ToastKind::Success => th.p.ok,
                ToastKind::Warn => th.p.warn,
                ToastKind::Error => th.p.danger,
            };
            let area = egui::Area::new(egui::Id::new(("toast", i)))
                .order(egui::Order::Tooltip)
                .anchor(
                    egui::Align2::CENTER_BOTTOM,
                    Vec2::new(0.0, y - screen.bottom()),
                )
                .interactable(false);
            let resp = area.show(ctx, |ui| {
                ui.set_opacity(alpha);
                egui::Frame::none()
                    .fill(th.p.surface)
                    .stroke(egui::Stroke::new(1.0_f32, th.p.border))
                    .rounding(10.0)
                    .shadow(egui::epaint::Shadow {
                        offset: Vec2::new(0.0, 6.0),
                        blur: 18.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(70),
                    })
                    .inner_margin(egui::Margin::symmetric(SP_3, SP_2))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            widgets::dot(ui, color);
                            ui.label(RichText::new(&t.text));
                        });
                    });
            });
            y -= resp.response.rect.height() + 8.0;
        }
    }

    fn canvas_view(&mut self, ui: &mut egui::Ui) {
        let th = self.theme;
        let tr = self.tr;
        let snap = self.tele.snapshot();
        let editable = !self.running();
        let local = self.settings.device_name.clone();
        let selected = self.selected.clone();
        let active = snap.active.clone();
        let running = self.running();
        let vis: HashMap<String, DeviceVisual> = self
            .settings
            .devices
            .iter()
            .map(|d| (d.id.clone(), self.device_visual(&d.id)))
            .collect();
        let visual = |id: &str| {
            vis.get(id).map_or(
                DeviceVisual {
                    status_color: Color32::GRAY,
                    status_text: String::new(),
                },
                |v| DeviceVisual {
                    status_color: v.status_color,
                    status_text: v.status_text.clone(),
                },
            )
        };
        let input = canvas::CanvasInput {
            th: &th,
            tr: &tr,
            local_id: &local,
            selected: selected.as_deref(),
            active: active.as_deref(),
            running,
            editable,
            visual: &visual,
        };
        let out = self.canvas.ui(ui, &mut self.settings.devices, &input);
        if let Some(id) = out.clicked {
            self.selected = Some(id);
        }
    }
}
