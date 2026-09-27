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
        viewport: egui::ViewportBuilder::default().with_inner_size([820.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "mouseshare",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new(log_buffer)))),
    )
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
                let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.listen_port);
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

        egui::SidePanel::left("config").min_width(320.0).show(ctx, |ui| {
            ui.heading("Local screen");
            ui.horizontal(|ui| {
                ui.label("screen_id:");
                ui.add_enabled(
                    !self.is_running(),
                    egui::TextEdit::singleline(&mut self.local_screen_id),
                );
            });
            ui.horizontal(|ui| {
                ui.label("resolution:");
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

            ui.separator();
            ui.heading("Role");
            ui.add_enabled_ui(!self.is_running(), |ui| {
                ui.radio_value(&mut self.role, Role::Controller, "Controller (this machine has the mouse/keyboard)");
                ui.radio_value(&mut self.role, Role::Target, "Target (receives control)");
            });

            ui.separator();
            match self.role {
                Role::Target => {
                    ui.heading("Listen port");
                    ui.add_enabled(
                        !self.is_running(),
                        egui::DragValue::new(&mut self.listen_port).range(1..=65535),
                    );
                }
                Role::Controller => {
                    ui.heading("Remote screen");
                    ui.horizontal(|ui| {
                        ui.label("screen_id:");
                        ui.add_enabled(
                            !self.is_running(),
                            egui::TextEdit::singleline(&mut self.remote_screen_id),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("resolution:");
                        ui.add_enabled(
                            !self.is_running(),
                            egui::DragValue::new(&mut self.remote_width).range(1..=16384),
                        );
                        ui.label("x");
                        ui.add_enabled(
                            !self.is_running(),
                            egui::DragValue::new(&mut self.remote_height).range(1..=16384),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Connect address (blank = mDNS auto-discover):");
                    });
                    ui.add_enabled(
                        !self.is_running(),
                        egui::TextEdit::singleline(&mut self.connect_addr_text)
                            .hint_text("192.168.1.20:7878"),
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let discovering = *self.discovering.lock().unwrap();
                        if ui
                            .add_enabled(!discovering, egui::Button::new("Scan LAN"))
                            .clicked()
                        {
                            self.refresh_discovery();
                        }
                        if discovering {
                            ui.spinner();
                        }
                    });
                    let peers = self.discovered_peers.lock().unwrap().clone();
                    for peer in &peers {
                        let label = format!("{}  @ {}", peer.screen_id, peer.addr);
                        if ui
                            .add_enabled(!self.is_running(), egui::Button::new(label))
                            .clicked()
                        {
                            self.remote_screen_id = peer.screen_id.clone();
                            self.connect_addr_text = peer.addr.to_string();
                        }
                    }
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                if !self.is_running() {
                    if ui.button("Start").clicked() {
                        // Placeholder offset; the real one comes from the
                        // canvas widget drawn in the central panel below,
                        // which runs after this closure on the same frame.
                        // We stash the intent and apply it once the canvas
                        // has reported its current snapped offset.
                        self.pending_start = true;
                    }
                } else if ui.button("Stop").clicked() {
                    self.stop();
                }
                ui.label(self.status.lock().unwrap().label());
            });

            ui.separator();
            ui.heading("Log");
            egui::ScrollArea::vertical().max_height(180.0).stick_to_bottom(true).show(ui, |ui| {
                for line in self.log_buffer.snapshot() {
                    ui.monospace(line);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Screen arrangement (drag the remote screen against any edge of the local one)");
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
        });
    }
}
