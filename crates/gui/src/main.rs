mod canvas;
mod log_buffer;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use mouseshare_discovery::DiscoveredPeer;
use mouseshare_layout::{LayoutConfig, ScreenRect};

use canvas::CanvasState;
use log_buffer::LogBuffer;

fn main() -> eframe::Result<()> {
    let log_buffer = LogBuffer::new(400);
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_ansi(false) // the log panel is a plain egui label, not a terminal
        .with_writer(log_buffer.clone())
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 660.0]),
        ..Default::default()
    };

    eframe::run_native(
        "mouseshare",
        native_options,
        Box::new(|cc| {
            configure_style(&cc.egui_ctx);
            Ok(Box::new(App::new(log_buffer)))
        }),
    )
}

/// Nudges the stock egui dark theme toward something a bit less
/// spreadsheet-default: softer rounding, more breathing room between
/// widgets, and a touch more contrast between the side panel and the
/// canvas behind it. Purely cosmetic -- no behavior here.
fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(27, 29, 33);
    visuals.window_fill = egui::Color32::from_rgb(30, 32, 37);
    visuals.extreme_bg_color = egui::Color32::from_rgb(18, 19, 22);
    visuals.faint_bg_color = egui::Color32::from_rgb(36, 38, 43);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(33, 35, 40);
    visuals.selection.bg_fill = egui::Color32::from_rgb(90, 130, 190);
    let rounding = egui::Rounding::same(6.0);
    visuals.window_rounding = rounding;
    visuals.menu_rounding = rounding;
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.open.rounding = rounding;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.indent = 14.0;
    ctx.set_style(style);
}

/// A left accent bar + bold label, used instead of a bare `ui.heading` to
/// give each side-panel section a bit more visual weight without the
/// clutter of a full box around every group.
fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let accent = ui.visuals().selection.bg_fill;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 1.5, accent);
        ui.label(egui::RichText::new(text).strong().size(14.5));
    });
    ui.add_space(2.0);
}

/// Wraps a section's contents in a subtly-filled rounded card, so the
/// sidebar reads as a stack of grouped panels rather than a flat list of
/// fields separated only by thin lines -- the flat-list version was part of
/// what made the previous pass still feel unfinished despite the individual
/// widgets being fine.
fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::NONE)
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| add_contents(ui));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Controller,
    Target,
}

enum Status {
    Idle,
    Running,
    Stopped,
    Error(String),
}

impl Status {
    fn label(&self) -> String {
        match self {
            Status::Idle => "idle".to_string(),
            Status::Running => "running".to_string(),
            Status::Stopped => "stopped".to_string(),
            Status::Error(e) => format!("error: {e}"),
        }
    }
}

struct App {
    local_screen_id: String,
    local_width: i32,
    local_height: i32,
    remote_screen_id: String,
    remote_width: i32,
    remote_height: i32,
    canvas: CanvasState,

    role: Role,
    listen_port: u16,
    connect_addr_text: String,
    discover_timeout_secs: u64,

    discovered_peers: Arc<Mutex<Vec<DiscoveredPeer>>>,
    discovering: Arc<Mutex<bool>>,

    rt: tokio::runtime::Runtime,
    running_handle: Option<tokio::task::JoinHandle<()>>,
    status: Arc<Mutex<Status>>,
    log_buffer: LogBuffer,
    /// Set by the "启动" button in the side panel; consumed once the
    /// central panel's canvas widget (drawn later in the same frame) has
    /// reported the current snapped remote-screen offset, since that's
    /// what `start()` needs to build the layout.
    pending_start: bool,
}

/// Best-effort local hostname, used as the default local screen_id. Not
/// worth a dependency for: falls back to a generic placeholder if the
/// `hostname` command isn't available or its output isn't valid UTF-8.
fn detect_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "local".to_string())
}

impl App {
    fn new(log_buffer: LogBuffer) -> Self {
        let (local_width, local_height) = match mouseshare_x11input::LocalCursor::connect(None) {
            Ok(cursor) => cursor.screen_size(),
            Err(e) => {
                tracing::warn!("could not query local X display for screen size ({e}); using a 1920x1080 placeholder -- edit it below");
                (1920, 1080)
            }
        };

        // Must be multi-thread (with a real worker thread), not
        // current_thread: a current_thread runtime only polls spawned tasks
        // while something calls `block_on` on it, and the eframe event loop
        // never does that -- spawned tasks would just sit queued forever.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .enable_io()
            .build()
            .expect("failed to build the background tokio runtime");

        Self {
            local_screen_id: detect_hostname(),
            local_width,
            local_height,
            remote_screen_id: "remote".to_string(),
            remote_width: local_width,
            remote_height: local_height,
            canvas: CanvasState::default(),

            role: Role::Controller,
            listen_port: 7878,
            connect_addr_text: String::new(),
            discover_timeout_secs: mouseshare_core::DEFAULT_DISCOVER_TIMEOUT.as_secs(),

            discovered_peers: Arc::new(Mutex::new(Vec::new())),
            discovering: Arc::new(Mutex::new(false)),

            rt,
            running_handle: None,
            status: Arc::new(Mutex::new(Status::Idle)),
            log_buffer,
            pending_start: false,
        }
    }

    fn is_running(&self) -> bool {
        matches!(*self.status.lock().unwrap(), Status::Running)
    }

    fn build_layout(&self, remote_offset: &canvas::RemoteOffset) -> LayoutConfig {
        let local = ScreenRect {
            id: self.local_screen_id.clone(),
            x: 0,
            y: 0,
            width: self.local_width,
            height: self.local_height,
        };
        let remote = ScreenRect {
            id: self.remote_screen_id.clone(),
            x: remote_offset.dx,
            y: remote_offset.dy,
            width: self.remote_width,
            height: self.remote_height,
        };
        LayoutConfig {
            local_id: self.local_screen_id.clone(),
            screens: vec![local, remote],
        }
    }

    fn start(&mut self, remote_offset: &canvas::RemoteOffset) {
        let layout = self.build_layout(remote_offset);
        let status = self.status.clone();

        match self.role {
            Role::Target => {
                let listen_addr =
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.listen_port);
                let handle = self.rt.spawn(async move {
                    let result = mouseshare_core::run_target(layout, listen_addr).await;
                    *status.lock().unwrap() = match result {
                        Ok(()) => Status::Stopped,
                        Err(e) => Status::Error(e.to_string()),
                    };
                });
                self.running_handle = Some(handle);
            }
            Role::Controller => {
                let connect = if self.connect_addr_text.trim().is_empty() {
                    None
                } else {
                    match self.connect_addr_text.trim().parse::<SocketAddr>() {
                        Ok(addr) => Some(addr),
                        Err(e) => {
                            *status.lock().unwrap() = Status::Error(format!(
                                "bad target address ({e}); expected e.g. 192.168.1.20:7878"
                            ));
                            return;
                        }
                    }
                };
                let timeout = Duration::from_secs(self.discover_timeout_secs);
                let handle = self.rt.spawn(async move {
                    let result = async {
                        let target_addr =
                            mouseshare_core::resolve_target_addr(&layout, connect, timeout).await?;
                        mouseshare_core::run_controller(layout, target_addr).await
                    }
                    .await;
                    *status.lock().unwrap() = match result {
                        Ok(()) => Status::Stopped,
                        Err(e) => Status::Error(e.to_string()),
                    };
                });
                self.running_handle = Some(handle);
            }
        }

        *self.status.lock().unwrap() = Status::Running;
    }

    fn stop(&mut self) {
        if let Some(handle) = self.running_handle.take() {
            handle.abort();
        }
        *self.status.lock().unwrap() = Status::Stopped;
    }

    fn refresh_discovery(&mut self) {
        if *self.discovering.lock().unwrap() {
            return;
        }
        *self.discovering.lock().unwrap() = true;
        let discovering = self.discovering.clone();
        let discovered_peers = self.discovered_peers.clone();
        let timeout = Duration::from_secs(self.discover_timeout_secs.max(1));
        std::thread::spawn(move || {
            let result = mouseshare_discovery::discover(timeout);
            if let Ok(peers) = result {
                *discovered_peers.lock().unwrap() = peers;
            }
            *discovering.lock().unwrap() = false;
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The background runtime and discovery thread only update shared
        // state; repaint periodically so their progress actually shows up
        // without needing user input to nudge a frame.
        ctx.request_repaint_after(Duration::from_millis(200));

        egui::SidePanel::left("config")
            .min_width(340.0)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("mouseshare").size(20.0).strong());
                });
                ui.add_space(4.0);

                section_heading(ui, "Local screen");
                card(ui, |ui| {
                    egui::Grid::new("local_grid")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("screen_id");
                            ui.add_enabled(
                                !self.is_running(),
                                egui::TextEdit::singleline(&mut self.local_screen_id)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();

                            ui.label("resolution");
                            ui.horizontal(|ui| {
                                ui.add_enabled(
                                    !self.is_running(),
                                    egui::DragValue::new(&mut self.local_width).range(1..=16384),
                                );
                                ui.label("x");
                                ui.add_enabled(
                                    !self.is_running(),
                                    egui::DragValue::new(&mut self.local_height).range(1..=16384),
                                );
                            });
                            ui.end_row();
                        });
                });

                ui.add_space(10.0);
                section_heading(ui, "Role");
                card(ui, |ui| {
                    ui.add_enabled_ui(!self.is_running(), |ui| {
                        let accent = ui.visuals().selection.bg_fill;
                        // A two-way segmented toggle instead of stacked radio
                        // buttons: it's the more familiar "mode switch" pattern
                        // in modern apps, and reads at a glance without having
                        // to parse two full sentences of radio-button label.
                        ui.columns(2, |cols| {
                            let seg = |ui: &mut egui::Ui, selected: bool, label: &str| {
                                let button = egui::Button::new(
                                    egui::RichText::new(label).strong().color(if selected {
                                        egui::Color32::WHITE
                                    } else {
                                        ui.visuals().text_color()
                                    }),
                                )
                                .fill(if selected {
                                    accent
                                } else {
                                    ui.visuals().extreme_bg_color
                                })
                                .stroke(egui::Stroke::new(
                                    1.0_f32,
                                    if selected {
                                        accent
                                    } else {
                                        ui.visuals().widgets.inactive.bg_stroke.color
                                    },
                                ));
                                ui.add_sized([ui.available_width(), 32.0], button)
                            };
                            if seg(&mut cols[0], self.role == Role::Controller, "🖱  Controller")
                                .clicked()
                            {
                                self.role = Role::Controller;
                            }
                            if seg(&mut cols[1], self.role == Role::Target, "🖥  Target").clicked()
                            {
                                self.role = Role::Target;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(match self.role {
                            Role::Controller => "This machine has the mouse/keyboard.",
                            Role::Target => "This machine receives control.",
                        })
                        .size(11.0)
                        .weak(),
                    );
                });

                ui.add_space(10.0);
                match self.role {
                    Role::Target => {
                        section_heading(ui, "Listen port");
                        card(ui, |ui| {
                            ui.add_enabled(
                                !self.is_running(),
                                egui::DragValue::new(&mut self.listen_port).range(1..=65535),
                            );
                        });
                    }
                    Role::Controller => {
                        section_heading(ui, "Remote screen");
                        card(ui, |ui| {
                            egui::Grid::new("remote_grid")
                                .num_columns(2)
                                .spacing([8.0, 6.0])
                                .show(ui, |ui| {
                                    ui.label("screen_id");
                                    ui.add_enabled(
                                        !self.is_running(),
                                        egui::TextEdit::singleline(&mut self.remote_screen_id)
                                            .desired_width(f32::INFINITY),
                                    );
                                    ui.end_row();

                                    ui.label("resolution");
                                    ui.horizontal(|ui| {
                                        ui.add_enabled(
                                            !self.is_running(),
                                            egui::DragValue::new(&mut self.remote_width)
                                                .range(1..=16384),
                                        );
                                        ui.label("x");
                                        ui.add_enabled(
                                            !self.is_running(),
                                            egui::DragValue::new(&mut self.remote_height)
                                                .range(1..=16384),
                                        );
                                    });
                                    ui.end_row();
                                });

                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("Connect address (blank = mDNS auto-discover)")
                                    .size(12.0)
                                    .weak(),
                            );
                            // The hint text already renders in the theme's dim
                            // "weak" gray; without an explicit brighter color here,
                            // real typed text used the default *inactive widget*
                            // gray, which read as almost the same dimness -- you
                            // couldn't tell a real address from the placeholder at a
                            // glance. Force entered text to a near-white color so
                            // there's real contrast between "empty, showing a hint"
                            // and "has a value".
                            ui.add_enabled(
                                !self.is_running(),
                                egui::TextEdit::singleline(&mut self.connect_addr_text)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("192.168.1.20:7878")
                                    .text_color(egui::Color32::from_rgb(235, 237, 240)),
                            );

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let discovering = *self.discovering.lock().unwrap();
                                // Match the Start/Stop buttons' rounded-pill accent
                                // language instead of a flat default-gray rectangle:
                                // an outlined accent pill reads as "secondary action
                                // in the same design system" rather than a leftover
                                // stock widget.
                                let accent = ui.visuals().selection.bg_fill;
                                let scan_button =
                                    egui::Button::new(egui::RichText::new("🔍  Scan LAN").strong())
                                        .fill(accent.linear_multiply(0.16))
                                        .stroke(egui::Stroke::new(1.3_f32, accent))
                                        .min_size(egui::vec2(88.0, 28.0));
                                if ui.add_enabled(!discovering, scan_button).clicked() {
                                    self.refresh_discovery();
                                }
                                if discovering {
                                    ui.spinner();
                                    ui.label(egui::RichText::new("scanning...").weak());
                                }
                            });
                            let peers = self.discovered_peers.lock().unwrap().clone();
                            if !peers.is_empty() {
                                ui.add_space(4.0);
                                egui::Frame::group(ui.style())
                                    .fill(ui.visuals().extreme_bg_color)
                                    .rounding(6.0)
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        for peer in &peers {
                                            let label =
                                                format!("{}  @ {}", peer.screen_id, peer.addr);
                                            if ui
                                                .add_enabled(
                                                    !self.is_running(),
                                                    egui::Button::new(label),
                                                )
                                                .clicked()
                                            {
                                                self.remote_screen_id = peer.screen_id.clone();
                                                self.connect_addr_text = peer.addr.to_string();
                                            }
                                        }
                                    });
                            }
                        });
                    }
                }

                ui.add_space(10.0);
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        if !self.is_running() {
                            let start_button =
                                egui::Button::new(egui::RichText::new("▶  Start").strong())
                                    .fill(egui::Color32::from_rgb(60, 130, 80))
                                    .min_size(egui::vec2(88.0, 28.0));
                            if ui.add(start_button).clicked() {
                                // Placeholder offset; the real one comes from the
                                // canvas widget drawn in the central panel below,
                                // which runs after this closure on the same frame.
                                // We stash the intent and apply it once the canvas
                                // has reported its current snapped offset.
                                self.pending_start = true;
                            }
                        } else {
                            let stop_button =
                                egui::Button::new(egui::RichText::new("■  Stop").strong())
                                    .fill(egui::Color32::from_rgb(150, 60, 60))
                                    .min_size(egui::vec2(88.0, 28.0));
                            if ui.add(stop_button).clicked() {
                                self.stop();
                            }
                        }

                        let status = self.status.lock().unwrap();
                        let (dot_color, text_color) = match &*status {
                            Status::Idle => (egui::Color32::GRAY, ui.visuals().text_color()),
                            Status::Running => (
                                egui::Color32::from_rgb(90, 200, 110),
                                egui::Color32::from_rgb(140, 220, 150),
                            ),
                            Status::Stopped => (egui::Color32::GRAY, ui.visuals().text_color()),
                            Status::Error(_) => (
                                egui::Color32::from_rgb(220, 90, 90),
                                egui::Color32::from_rgb(240, 130, 130),
                            ),
                        };
                        ui.add_space(4.0);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, dot_color);
                        ui.label(egui::RichText::new(status.label()).color(text_color));
                    });
                });

                ui.add_space(10.0);
                section_heading(ui, "Log");
                // The Controller role's sidebar has more fields above this point
                // than Target's (remote screen grid, connect address, scan
                // button, discovered-peer list), so a fixed scroll height here
                // used to push the frame's bottom edge past the panel's actual
                // remaining space -- the rounded frame got silently clipped with
                // no visible border. Size the scroll area from whatever room is
                // actually left instead, so the whole frame (border included)
                // always lands fully inside the panel regardless of role.
                let log_inner_margin = 6.0;
                let bottom_breathing_room = 6.0;
                // Deliberately no lower-bound floor here: a floor that forces a
                // minimum size regardless of what's actually left is exactly
                // what caused the clipped frame in the first place (Controller's
                // taller sidebar left less room than a fixed/floored height
                // assumed). Only cap the *maximum* so a very roomy panel (e.g.
                // Target's shorter sidebar) doesn't stretch the log absurdly
                // tall; let it shrink as small as it truly must to always stay
                // fully inside the panel.
                let log_scroll_height =
                    (ui.available_height() - log_inner_margin * 2.0 - bottom_breathing_room)
                        .clamp(0.0, 220.0);
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().extreme_bg_color)
                    .rounding(6.0)
                    .inner_margin(log_inner_margin)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(log_scroll_height)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for line in self.log_buffer.snapshot() {
                                    ui.monospace(egui::RichText::new(line).size(11.5));
                                }
                            });
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Screen arrangement")
                        .size(18.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new("Drag the remote screen against any edge of the local one")
                        .weak(),
                );
            });
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                let offset = self.canvas.ui(
                    ui,
                    &self.local_screen_id,
                    (self.local_width, self.local_height),
                    &self.remote_screen_id,
                    (self.remote_width, self.remote_height),
                );

                if self.pending_start {
                    self.pending_start = false;
                    self.start(&offset);
                }

                ui.add_space(10.0);
                // One-click alignment along the touching edge: dragging by
                // hand is fine when both screens are the same size, but two
                // different resolutions/DPIs (e.g. a 4K screen next to a
                // 1080p one) usually call for a specific, exact alignment
                // rather than whatever pixel offset a drag happens to land
                // on -- the same convenience real OS display-arrangement
                // dialogs offer for mismatched monitors.
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Align:").weak());
                    let (start_label, center_label, end_label) = match self.canvas.axis() {
                        canvas::SnapAxis::Horizontal => ("Top", "Center", "Bottom"),
                        canvas::SnapAxis::Vertical => ("Left", "Center", "Right"),
                    };
                    if ui.button(start_label).clicked() {
                        self.canvas.align(canvas::Alignment::Start);
                    }
                    if ui.button(center_label).clicked() {
                        self.canvas.align(canvas::Alignment::Center);
                    }
                    if ui.button(end_label).clicked() {
                        self.canvas.align(canvas::Alignment::End);
                    }
                });
            });
        });
    }
}
