//! mouseshare GUI: arrange your computers, pair them, share one mouse and
//! keyboard.

mod app;
mod canvas;
mod dialogs;
mod fonts;
mod i18n;
mod log_buffer;
mod onboarding;
mod rail;
mod scan;
mod session;
mod settings_view;
mod strings;
mod target_view;
mod theme;
mod widgets;

use eframe::egui;

use crate::log_buffer::LogBuffer;

/// A simple generated window icon: an accent-blue rounded square with a
/// white pointer, so the window has an identity even before the packaged
/// icon theme entry is installed.
fn window_icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let inside_round = |x: f32, y: f32| {
        let (cx, cy) = (x - 31.5, y - 31.5);
        let (ax, ay) = (cx.abs() - 20.0, cy.abs() - 20.0);
        let d = ax.max(0.0).hypot(ay.max(0.0)) + ax.max(ay).min(0.0) - 12.0;
        d <= 0.0
    };
    // Pointer arrow polygon (even-odd via barycentric tests on two triangles).
    let tri = |p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        let s = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| {
            (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
        };
        let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    };
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32, y as f32);
            let i = (y * N + x) * 4;
            if !inside_round(fx, fy) {
                continue;
            }
            let arrow = tri((fx, fy), (21.0, 14.0), (21.0, 46.0), (44.0, 33.0))
                || tri((fx, fy), (32.0, 36.0), (38.0, 34.0), (42.0, 49.0));
            let (r, g, b) = if arrow {
                (255, 255, 255)
            } else {
                (56, 112, 235)
            };
            rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
    }
}

fn main() -> eframe::Result<()> {
    let log = LogBuffer::new(400);
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_ansi(false)
        .with_writer(log.clone())
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mouseshare")
            .with_app_id("mouseshare")
            .with_inner_size([1020.0, 700.0])
            .with_min_inner_size([860.0, 560.0])
            .with_icon(window_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "mouseshare",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, log)))),
    )
}
