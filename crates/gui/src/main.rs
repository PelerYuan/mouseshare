mod canvas;
mod log_buffer;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use mouseshare_core::{ControllerIdCell, PeerStatus, PeerStatusMap};
use mouseshare_discovery::DiscoveredPeer;
use mouseshare_layout::{LayoutConfig, ScreenRect};

use canvas::CanvasState;
use log_buffer::LogBuffer;

/// Fixed rail width -- not a resizable range. A unified row (swatch + name +
/// resolution + status dot + remove button) needs more room than the old
/// 260-280px range comfortably gave it without introducing its own
/// truncation problems.
const RAIL_WIDTH: f32 = 288.0;
/// Height of the pinned footer (Start/Stop + status pill), reserved before
/// anything above it is laid out so it never scrolls out of reach regardless
/// of how long the device list or detail editor above it gets.
const FOOTER_HEIGHT: f32 = 60.0;
const LOG_DRAWER_COLLAPSED: f32 = 28.0;
const LOG_DRAWER_EXPANDED: f32 = 192.0;
const ROW_HEIGHT: f32 = 32.0;

fn main() -> eframe::Result<()> {
    let log_buffer = LogBuffer::new(400);
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_ansi(false) // the log panel is a plain egui label, not a terminal
        .with_writer(log_buffer.clone())
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 660.0])
            // Below this, the rail (288) + a usable canvas floor (360) plus
            // margins genuinely don't fit -- zoom can't create pixels that
            // aren't there, so the floor has to be a real window constraint,
            // not just a documented hope.
            .with_min_inner_size([680.0, 480.0]),
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
    // Raised from (36,38,43): a 9-unit delta against panel_fill (27,29,33)
    // was nearly invisible on a dark theme, so cards barely read as cards.
    visuals.faint_bg_color = egui::Color32::from_rgb(44, 47, 53);
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
    // On the 4/8/12/16/24 spacing scale (was 12, off-scale).
    style.spacing.window_margin = egui::Margin::same(16.0);
    style.spacing.indent = 14.0;
    ctx.set_style(style);
}

/// The one "secondary action" button style used everywhere in this app
/// (Scan LAN, the alignment buttons, Add device, Remove device): an
/// accent-outline pill. Having every non-primary button share exactly this
/// look, rather than each spot reaching for whatever egui::Button::new
/// defaults to, is what makes the UI read as one design system instead of a
/// pile of unrelated widgets.
fn secondary_button(ui: &egui::Ui, label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    let accent = ui.visuals().selection.bg_fill;
    egui::Button::new(label.into())
        .fill(accent.linear_multiply(0.16))
        .stroke(egui::Stroke::new(1.3_f32, accent))
}

/// Wraps a section's contents in a subtly-filled rounded card, so the
/// device list and the detail editor each read as one grouped surface
/// instead of a flat run of fields with no visual boundary.
fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::NONE)
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| add_contents(ui));
}

/// Renders `text` at Body scale (13pt), shortening it with the same
/// binary-search algorithm the canvas uses if it doesn't fit `max_width`,
/// and -- unlike a tooltip-only approach -- draws a small dotted underline
/// so the truncation is visible without hovering (DESIGN_REVIEW standard
/// #9). Shares the exact truncation logic with `canvas::fit_text` so the
/// rail and the canvas never disagree about what "too long" means.
fn truncating_label(ui: &mut egui::Ui, text: &str, max_width: f32, strong: bool) -> egui::Response {
    let font_id = egui::FontId::proportional(13.0);
    let (label, truncated) = canvas::fit_text(ui.painter(), text, font_id, max_width);
    let rich = if strong {
        egui::RichText::new(label).strong()
    } else {
        egui::RichText::new(label)
    };
    let response = ui.add(egui::Label::new(rich).selectable(false));
    if truncated {
        canvas::paint_truncation_marker(ui.painter(), response.rect);
        return response.on_hover_text(text);
    }
    response
}

/// One row's connection-status dot color, reusing the exact same RGB
/// triples the footer status pill uses -- one "attention" red, one
/// "healthy" green, one "nothing to report" gray, everywhere in the app,
/// rather than a fourth mechanism inventing its own palette.
fn peer_status_color(status: Option<&PeerStatus>) -> egui::Color32 {
    match status {
        None | Some(PeerStatus::Connecting) => egui::Color32::GRAY,
        Some(PeerStatus::Connected) => egui::Color32::from_rgb(90, 200, 110),
        Some(PeerStatus::Failed(_)) => egui::Color32::from_rgb(220, 90, 90),
    }
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

/// Which row in the unified device list is open in the detail editor below
/// it. The local screen isn't structurally special -- it's just pinned
/// first and non-removable -- so it gets the same selection/editor
/// treatment as any remote instead of a separate always-open section.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Local,
    Remote(usize),
}

/// One remote (target) screen the controller can hand off to. The sidebar
/// shows a compact list of these plus a detail editor for whichever one is
/// currently selected -- replacing the old single fixed "Remote screen"
/// section, which had no way to represent a second or third device.
struct RemoteDevice {
    id: String,
    width: i32,
    height: i32,
    connect_addr_text: String,
}

impl RemoteDevice {
    fn new(id: impl Into<String>, width: i32, height: i32) -> Self {
        Self {
            id: id.into(),
            width,
            height,
            connect_addr_text: String::new(),
        }
    }
}

struct App {
    local_screen_id: String,
    local_width: i32,
    local_height: i32,
    remotes: Vec<RemoteDevice>,
    selection: Selection,
    canvas: CanvasState,

    role: Role,
    listen_port: u16,
    discover_timeout_secs: u64,

    discovered_peers: Arc<Mutex<Vec<DiscoveredPeer>>>,
    discovering: Arc<Mutex<bool>>,

    rt: tokio::runtime::Runtime,
    running_handle: Option<tokio::task::JoinHandle<()>>,
    status: Arc<Mutex<Status>>,
    /// Per-target connection state while running, keyed by screen id --
    /// real data from `run_controller`'s connect/disconnect handling, not a
    /// decorative color. Replaced with a fresh empty map every time Start is
    /// pressed so a previous run's entries never leak into a new one.
    peer_status: PeerStatusMap,
    /// While running as Target: which controller (if any) is currently
    /// connected, straight from `run_target`'s accept loop.
    controller_status: ControllerIdCell,
    log_buffer: LogBuffer,
    log_open: bool,
    /// Set by the Start button in the side panel; consumed once the central
    /// panel's canvas widget (drawn later in the same frame) has reported
    /// the current snapped remote-screen offsets, since that's what
    /// `start()` needs to build the layout.
    pending_start: bool,
}

/// Kicks off a background LAN scan if one isn't already running. A free
/// function (not an `App` method) so it can be called from inside a closure
/// that already holds a mutable borrow of one of `self.remotes` -- calling
/// back into any `self.*` method there would conflict with that borrow even
/// though the fields touched are disjoint, since method calls borrow `self`
/// as a whole.
fn trigger_discovery(
    discovering: &Arc<Mutex<bool>>,
    discovered_peers: &Arc<Mutex<Vec<DiscoveredPeer>>>,
    timeout_secs: u64,
) {
    if *discovering.lock().unwrap() {
        return;
    }
    *discovering.lock().unwrap() = true;
    let discovering = discovering.clone();
    let discovered_peers = discovered_peers.clone();
    let timeout = Duration::from_secs(timeout_secs.max(1));
    std::thread::spawn(move || {
        let result = mouseshare_discovery::discover(timeout);
        if let Ok(peers) = result {
            *discovered_peers.lock().unwrap() = peers;
        }
        *discovering.lock().unwrap() = false;
    });
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
            remotes: vec![RemoteDevice::new("remote", local_width, local_height)],
            selection: Selection::Remote(0),
            canvas: CanvasState::default(),

            role: Role::Controller,
            listen_port: 7878,
            discover_timeout_secs: mouseshare_core::DEFAULT_DISCOVER_TIMEOUT.as_secs(),

            discovered_peers: Arc::new(Mutex::new(Vec::new())),
            discovering: Arc::new(Mutex::new(false)),

            rt,
            running_handle: None,
            status: Arc::new(Mutex::new(Status::Idle)),
            peer_status: Arc::new(Mutex::new(HashMap::new())),
            controller_status: Arc::new(Mutex::new(None)),
            log_buffer,
            log_open: false,
            pending_start: false,
        }
    }

    fn is_running(&self) -> bool {
        matches!(*self.status.lock().unwrap(), Status::Running)
    }

    fn build_layout(&self, offsets: &HashMap<String, canvas::RemoteOffset>) -> LayoutConfig {
        let local = ScreenRect {
            id: self.local_screen_id.clone(),
            x: 0,
            y: 0,
            width: self.local_width,
            height: self.local_height,
        };
        let mut screens = vec![local];
        for remote in &self.remotes {
            let offset = offsets.get(&remote.id);
            screens.push(ScreenRect {
                id: remote.id.clone(),
                x: offset.map_or(0, |o| o.dx),
                y: offset.map_or(0, |o| o.dy),
                width: remote.width,
                height: remote.height,
            });
        }
        LayoutConfig {
            local_id: self.local_screen_id.clone(),
            screens,
        }
    }

    fn start(&mut self, offsets: &HashMap<String, canvas::RemoteOffset>) {
        let layout = self.build_layout(offsets);
        let status = self.status.clone();

        match self.role {
            Role::Target => {
                let listen_addr =
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.listen_port);
                self.controller_status = Arc::new(Mutex::new(None));
                let controller_status = self.controller_status.clone();
                let handle = self.rt.spawn(async move {
                    let result =
                        mouseshare_core::run_target(layout, listen_addr, controller_status).await;
                    *status.lock().unwrap() = match result {
                        Ok(()) => Status::Stopped,
                        Err(e) => Status::Error(e.to_string()),
                    };
                });
                self.running_handle = Some(handle);
            }
            Role::Controller => {
                if self.remotes.is_empty() {
                    *status.lock().unwrap() =
                        Status::Error("add at least one remote device first".to_string());
                    return;
                }
                let mut overrides = HashMap::new();
                for remote in &self.remotes {
                    let text = remote.connect_addr_text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    match text.parse::<SocketAddr>() {
                        Ok(addr) => {
                            overrides.insert(remote.id.clone(), addr);
                        }
                        Err(e) => {
                            *status.lock().unwrap() = Status::Error(format!(
                                "bad address for {:?} ({e}); expected e.g. 192.168.1.20:7878",
                                remote.id
                            ));
                            return;
                        }
                    }
                }
                let timeout = Duration::from_secs(self.discover_timeout_secs);
                self.peer_status = Arc::new(Mutex::new(HashMap::new()));
                let peer_status = self.peer_status.clone();
                let handle = self.rt.spawn(async move {
                    let result = async {
                        let targets =
                            mouseshare_core::resolve_target_addrs(&layout, &overrides, timeout)
                                .await?;
                        mouseshare_core::run_controller(layout, targets, peer_status).await
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

    /// Renders the unified device list (local pinned first, then every
    /// remote) as uniform fixed-height rows, updating `self.selection` when
    /// a row is clicked and returning the index of a remote whose remove
    /// button was clicked, if any.
    fn device_list(&mut self, ui: &mut egui::Ui, is_running: bool) {
        card(ui, |ui| {
            let list_height = (ui.available_height() * 0.45).clamp(ROW_HEIGHT * 2.0, 220.0);
            egui::ScrollArea::vertical()
                .max_height(list_height)
                .show(ui, |ui| {
                    self.device_row_local(ui);
                    let remotes: Vec<(String, i32, i32)> = self
                        .remotes
                        .iter()
                        .map(|r| (r.id.clone(), r.width, r.height))
                        .collect();
                    let mut remove_index = None;
                    for (index, (id, width, height)) in remotes.into_iter().enumerate() {
                        if self.device_row_remote(ui, index, &id, width, height, is_running) {
                            remove_index = Some(index);
                        }
                    }
                    if let Some(index) = remove_index {
                        self.remotes.remove(index);
                        self.selection = match self.selection {
                            Selection::Remote(sel) if sel == index => {
                                if self.remotes.is_empty() {
                                    Selection::Local
                                } else {
                                    Selection::Remote(sel.min(self.remotes.len() - 1))
                                }
                            }
                            Selection::Remote(sel) if sel > index => Selection::Remote(sel - 1),
                            other => other,
                        };
                    }
                });
            ui.add_space(4.0);
            if ui
                .add_enabled(!is_running, secondary_button(ui, "\u{2795}  Add device"))
                .clicked()
            {
                let next_index = self.remotes.len() + 1;
                self.remotes.push(RemoteDevice::new(
                    format!("remote-{next_index}"),
                    self.local_width,
                    self.local_height,
                ));
                self.selection = Selection::Remote(self.remotes.len() - 1);
            }
        });
    }

    fn device_row_local(&mut self, ui: &mut egui::Ui) {
        let is_selected = self.selection == Selection::Local;
        let width = ui.available_width();
        let row_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, ROW_HEIGHT));
        if is_selected {
            ui.painter().rect_filled(
                row_rect,
                4.0,
                ui.visuals().selection.bg_fill.linear_multiply(0.25),
            );
        }
        let label = format!(
            "{}  {}\u{00d7}{}",
            self.local_screen_id, self.local_width, self.local_height
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(8.0);
                let (swatch_rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(swatch_rect, 2.0, egui::Color32::from_rgb(70, 130, 180));
                ui.add_space(6.0);
                let text_width = (ui.available_width() - 8.0).max(20.0);
                truncating_label(ui, &label, text_width, is_selected);
            },
        );
        let click = ui.interact(row_rect, ui.id().with("row_local"), egui::Sense::click());
        if click.clicked() {
            self.selection = Selection::Local;
        }
    }

    /// Returns `true` if this row's remove button was clicked.
    fn device_row_remote(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        id: &str,
        width: i32,
        height: i32,
        is_running: bool,
    ) -> bool {
        let is_selected = self.selection == Selection::Remote(index);
        let row_width = ui.available_width();
        let row_rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(row_width, ROW_HEIGHT));
        if is_selected {
            ui.painter().rect_filled(
                row_rect,
                4.0,
                ui.visuals().selection.bg_fill.linear_multiply(0.25),
            );
        }
        let swatch = if index < 4 {
            [
                egui::Color32::from_rgb(150, 110, 200),
                egui::Color32::from_rgb(90, 170, 160),
                egui::Color32::from_rgb(200, 130, 90),
                egui::Color32::from_rgb(190, 100, 140),
            ][index]
        } else {
            egui::Color32::from_rgb(52, 55, 61)
        };
        let dot_color = if self.is_running() {
            peer_status_color(self.peer_status.lock().unwrap().get(id))
        } else {
            egui::Color32::GRAY
        };
        let label = format!("{id}  {width}\u{00d7}{height}");
        let mut removed = false;
        ui.allocate_ui_with_layout(
            egui::vec2(row_width, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(8.0);
                let (swatch_rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().rect_filled(swatch_rect, 2.0, swatch);
                ui.add_space(6.0);
                let (dot_rect, _) =
                    ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot_rect.center(), 3.5, dot_color);
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if ui
                        .add_enabled(!is_running, egui::Button::new("\u{2715}").small())
                        .on_hover_text("Remove this device")
                        .clicked()
                    {
                        removed = true;
                    }
                    let text_width = (ui.available_width() - 4.0).max(20.0);
                    truncating_label(ui, &label, text_width, is_selected);
                });
            },
        );
        let click = ui.interact(
            row_rect,
            ui.id().with(("row_remote", index)),
            egui::Sense::click(),
        );
        if click.clicked() && !removed {
            self.selection = Selection::Remote(index);
        }
        removed
    }

    /// The detail editor for whichever row is currently selected -- one
    /// component, a conditional field set: id/resolution for either kind,
    /// plus connect-address/Scan LAN/discovered-peers/align-controls only
    /// for a selected remote.
    fn detail_editor(&mut self, ui: &mut egui::Ui, is_running: bool) {
        match self.selection {
            Selection::Local => {
                card(ui, |ui| {
                    egui::Grid::new("local_grid")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("screen_id");
                            ui.add_enabled(
                                !is_running,
                                egui::TextEdit::singleline(&mut self.local_screen_id)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();

                            ui.label("resolution");
                            ui.horizontal(|ui| {
                                ui.add_enabled(
                                    !is_running,
                                    egui::DragValue::new(&mut self.local_width).range(1..=16384),
                                );
                                ui.label("x");
                                ui.add_enabled(
                                    !is_running,
                                    egui::DragValue::new(&mut self.local_height).range(1..=16384),
                                );
                            });
                            ui.end_row();
                        });
                });
            }
            Selection::Remote(index) => {
                let discovering_handle = self.discovering.clone();
                let discovered_peers_handle = self.discovered_peers.clone();
                let discover_timeout_secs = self.discover_timeout_secs;
                let axis = self
                    .remotes
                    .get(index)
                    .and_then(|r| self.canvas.axis(&r.id));
                let Some(remote) = self.remotes.get_mut(index) else {
                    return;
                };

                card(ui, |ui| {
                    egui::Grid::new("remote_grid")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("screen_id");
                            ui.add_enabled(
                                !is_running,
                                egui::TextEdit::singleline(&mut remote.id)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();

                            ui.label("resolution");
                            ui.horizontal(|ui| {
                                ui.add_enabled(
                                    !is_running,
                                    egui::DragValue::new(&mut remote.width).range(1..=16384),
                                );
                                ui.label("x");
                                ui.add_enabled(
                                    !is_running,
                                    egui::DragValue::new(&mut remote.height).range(1..=16384),
                                );
                            });
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Connect address (blank = mDNS auto-discover)")
                            .size(11.0)
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
                    let field_response = ui.add_enabled(
                        !is_running,
                        egui::TextEdit::singleline(&mut remote.connect_addr_text)
                            .desired_width(f32::INFINITY)
                            .hint_text("192.168.1.20:7878")
                            .text_color(egui::Color32::from_rgb(235, 237, 240)),
                    );

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let discovering = *discovering_handle.lock().unwrap();
                        let scan_button = secondary_button(ui, "\u{1F50D}  Scan LAN")
                            .min_size(egui::vec2(88.0, 28.0));
                        if ui.add_enabled(!discovering, scan_button).clicked() {
                            trigger_discovery(
                                &discovering_handle,
                                &discovered_peers_handle,
                                discover_timeout_secs,
                            );
                        }
                        if discovering {
                            ui.spinner();
                            ui.label(egui::RichText::new("scanning...").weak());
                        }
                    });

                    // A non-flow popup anchored to the connect-address
                    // field, not an always-reserved space in the document
                    // flow -- an inline box that appears/disappears as
                    // discovery runs would shove the align controls below
                    // it up and down, exactly the kind of shifting layout
                    // this whole pass exists to remove.
                    let popup_id = ui.id().with("discovered_peers_popup");
                    let peers = discovered_peers_handle.lock().unwrap().clone();
                    if !peers.is_empty() {
                        ui.memory_mut(|mem| mem.open_popup(popup_id));
                    }
                    egui::popup_below_widget(
                        ui,
                        popup_id,
                        &field_response,
                        egui::PopupCloseBehavior::CloseOnClickOutside,
                        |ui| {
                            ui.set_min_width(field_response.rect.width());
                            for peer in &peers {
                                let label = format!("{}  @ {}", peer.screen_id, peer.addr);
                                if ui
                                    .add_enabled(!is_running, egui::Button::new(label))
                                    .clicked()
                                {
                                    remote.id = peer.screen_id.clone();
                                    remote.connect_addr_text = peer.addr.to_string();
                                    ui.memory_mut(|mem| mem.close_popup());
                                }
                            }
                        },
                    );

                    // One-click alignment along the touching edge, for
                    // lining up two screens of different resolution without
                    // needing pixel-perfect dragging -- only shown once this
                    // remote has actually been drawn on the canvas at least
                    // once (`axis` is `None` until then) and never for the
                    // local screen, which has no position of its own to
                    // align.
                    if let Some(axis) = axis {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Align:").weak());
                            let (start_label, center_label, end_label) = match axis {
                                canvas::SnapAxis::Horizontal => ("Top", "Center", "Bottom"),
                                canvas::SnapAxis::Vertical => ("Left", "Center", "Right"),
                            };
                            let id = remote.id.clone();
                            if ui.add(secondary_button(ui, start_label)).clicked() {
                                self.canvas.align(&id, canvas::Alignment::Start);
                            }
                            if ui.add(secondary_button(ui, center_label)).clicked() {
                                self.canvas.align(&id, canvas::Alignment::Center);
                            }
                            if ui.add(secondary_button(ui, end_label)).clicked() {
                                self.canvas.align(&id, canvas::Alignment::End);
                            }
                        });
                    }
                });
            }
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if !self.is_running() {
                let start_button =
                    egui::Button::new(egui::RichText::new("\u{25B6}  Start").strong())
                        .fill(egui::Color32::from_rgb(60, 130, 80))
                        .min_size(egui::vec2(88.0, 28.0));
                if ui.add(start_button).clicked() {
                    // Placeholder offset; the real one comes from the
                    // canvas widget drawn in the central panel below, which
                    // runs after this closure on the same frame. We stash
                    // the intent and apply it once the canvas has reported
                    // its current snapped offset.
                    self.pending_start = true;
                }
            } else {
                let stop_button = egui::Button::new(egui::RichText::new("\u{25A0}  Stop").strong())
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
            // Its own component reporting run state, not an extension of
            // the Start/Stop button beside it.
            ui.add_space(12.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot_color);
            ui.label(egui::RichText::new(status.label()).color(text_color));
        });
    }

    /// The bottom log drawer: a thin always-visible header (chevron +
    /// "Log" + an error badge dot if the last status is an error) and, when
    /// expanded, the scrolling log content. Closed by default and never
    /// auto-opens on error -- toggling open/closed mid-session would resize
    /// the canvas above it while the user might be mid-drag arranging
    /// boxes, which is worse than a missed error noticed a beat later from
    /// the badge.
    fn log_drawer(&mut self, ui: &mut egui::Ui) {
        let has_error = matches!(*self.status.lock().unwrap(), Status::Error(_));
        ui.horizontal(|ui| {
            let chevron = if self.log_open {
                "\u{25BE}"
            } else {
                "\u{25B8}"
            };
            if ui.button(format!("{chevron} Log")).clicked() {
                self.log_open = !self.log_open;
            }
            if has_error && !self.log_open {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    rect.center(),
                    4.0,
                    egui::Color32::from_rgb(220, 90, 90),
                );
            }
        });
        if self.log_open {
            ui.add_space(4.0);
            egui::Frame::group(ui.style())
                .fill(ui.visuals().extreme_bg_color)
                .rounding(6.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(LOG_DRAWER_EXPANDED - 40.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in self.log_buffer.snapshot() {
                                ui.monospace(egui::RichText::new(line).size(11.0));
                            }
                        });
                });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The background runtime and discovery thread only update shared
        // state; repaint periodically so their progress actually shows up
        // without needing user input to nudge a frame.
        ctx.request_repaint_after(Duration::from_millis(200));

        // Registered before the rail and the canvas: egui lays out
        // top/bottom panels first (full window width), then side panels
        // fill whatever height is left, then the central panel takes the
        // remainder. That ordering is load-bearing, not incidental -- it's
        // what makes this a genuine full-width third band under both the
        // rail and the canvas, with the rail's own footer staying pinned to
        // the bottom of its now-shorter column, instead of the drawer and
        // the footer fighting over the same space. Reordering these three
        // `.show()` calls changes what "full width" means; if you're
        // touching this, re-read the doc comment before moving anything.
        egui::TopBottomPanel::bottom("log_drawer")
            .exact_height(if self.log_open {
                LOG_DRAWER_EXPANDED
            } else {
                LOG_DRAWER_COLLAPSED
            })
            .show(ctx, |ui| {
                self.log_drawer(ui);
            });

        egui::SidePanel::left("rail")
            .exact_width(RAIL_WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                let total_height = ui.available_height();
                let is_running = self.is_running();
                ui.allocate_ui_with_layout(
                    egui::vec2(
                        ui.available_width(),
                        (total_height - FOOTER_HEIGHT).max(0.0),
                    ),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.add_space(2.0);
                        ui.heading(egui::RichText::new("mouseshare").size(20.0).strong());
                        ui.add_space(8.0);

                        ui.add_enabled_ui(!is_running, |ui| {
                            let accent = ui.visuals().selection.bg_fill;
                            // A two-way segmented toggle instead of stacked
                            // radio buttons: it's the more familiar "mode
                            // switch" pattern in modern apps, and reads at a
                            // glance without having to parse two full
                            // sentences of radio-button label.
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
                                if seg(
                                    &mut cols[0],
                                    self.role == Role::Controller,
                                    "\u{1F5B1}  Controller",
                                )
                                .clicked()
                                {
                                    self.role = Role::Controller;
                                }
                                if seg(&mut cols[1], self.role == Role::Target, "\u{1F5A5}  Target")
                                    .clicked()
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
                        ui.add_space(12.0);

                        match self.role {
                            Role::Target => {
                                ui.horizontal(|ui| {
                                    ui.label("Listen port");
                                    ui.add_enabled(
                                        !is_running,
                                        egui::DragValue::new(&mut self.listen_port)
                                            .range(1..=65535),
                                    );
                                });
                                if is_running {
                                    ui.add_space(8.0);
                                    let connected = self.controller_status.lock().unwrap().clone();
                                    let text = match connected {
                                        Some(id) => format!("Connected to {id}"),
                                        None => "Waiting for a controller".to_string(),
                                    };
                                    ui.label(egui::RichText::new(text).weak());
                                }
                            }
                            Role::Controller => {
                                self.device_list(ui, is_running);
                                ui.add_space(12.0);
                                self.detail_editor(ui, is_running);
                            }
                        }
                    },
                );

                self.footer(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.role {
                Role::Controller => {
                    let n = self.remotes.len();
                    let header = format!("{n} device{}", if n == 1 { "" } else { "s" });
                    ui.label(egui::RichText::new(header).size(13.0).weak());
                    ui.add_space(4.0);

                    let remotes: Vec<(String, (i32, i32))> = self
                        .remotes
                        .iter()
                        .map(|r| (r.id.clone(), (r.width, r.height)))
                        .collect();
                    let selected_id: Option<String> = match self.selection {
                        Selection::Remote(i) => self.remotes.get(i).map(|r| r.id.clone()),
                        Selection::Local => None,
                    };
                    let output = self.canvas.ui(
                        ui,
                        &self.local_screen_id,
                        (self.local_width, self.local_height),
                        &remotes,
                        selected_id.as_deref(),
                    );

                    if let Some(clicked_id) = &output.clicked_remote {
                        if let Some(index) = self.remotes.iter().position(|r| &r.id == clicked_id)
                        {
                            self.selection = Selection::Remote(index);
                        }
                    }

                    if self.pending_start {
                        self.pending_start = false;
                        if output.any_overlap {
                            // Two remotes snapped to overlapping positions
                            // would make edge-crossing between them
                            // ambiguous (which one does the cursor land
                            // on?) -- refuse to start rather than let the
                            // user discover this live.
                            *self.status.lock().unwrap() = Status::Error(
                                "two remote screens overlap in the arrangement -- drag them apart first"
                                    .to_string(),
                            );
                        } else {
                            self.start(&output.offsets);
                        }
                    }
                }
                Role::Target => {
                    // Deliberately not the full interactive arrangement
                    // canvas: a Target doesn't own the topology (the
                    // Controller-side offsets are never transmitted to it
                    // over the wire -- see `run_target`), so a positioned
                    // second box would be fabricating a spatial
                    // relationship this machine has no data for. Instead: a
                    // read-only rendering of just this machine's own
                    // screen, which at least answers "is my resolution
                    // right" without needing to walk over to the
                    // controller's machine.
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        let connected = self.controller_status.lock().unwrap().clone();
                        let text = if self.is_running() {
                            match connected {
                                Some(id) => format!("Connected to {id}"),
                                None => "Waiting for a controller".to_string(),
                            }
                        } else {
                            "Not running".to_string()
                        };
                        ui.label(egui::RichText::new(text).size(13.0).weak());
                    });
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        self.canvas.ui_readonly(
                            ui,
                            &self.local_screen_id,
                            (self.local_width, self.local_height),
                        );
                    });

                    if self.pending_start {
                        self.pending_start = false;
                        self.start(&HashMap::new());
                    }
                }
            }
        });
    }
}
