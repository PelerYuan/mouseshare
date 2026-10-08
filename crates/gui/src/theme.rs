//! Design tokens and the egui style built from them.
//!
//! Flat surfaces, hairline borders, one accent colour, an 8-point spacing
//! scale. Light and dark are two token tables; everything else derives from
//! them so a widget never hard-codes a colour.

use eframe::egui::{
    self, epaint::Shadow, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle, Visuals,
};
use mouseshare_config::{Accent, ThemeMode};

pub const SP_1: f32 = 4.0;
pub const SP_2: f32 = 8.0;
pub const SP_3: f32 = 12.0;
pub const SP_4: f32 = 16.0;
pub const SP_6: f32 = 24.0;
pub const RADIUS: f32 = 8.0;

#[derive(Clone, Copy)]
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Side rail / top bar.
    pub surface: Color32,
    /// Cards and inputs sitting on `surface`.
    pub raised: Color32,
    /// Hover state of raised surfaces.
    pub raised_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub canvas: Color32,
    pub canvas_dot: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub danger: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: Color32::from_rgb(17, 18, 21),
            surface: Color32::from_rgb(24, 25, 29),
            raised: Color32::from_rgb(33, 35, 40),
            raised_hover: Color32::from_rgb(42, 44, 50),
            border: Color32::from_rgb(45, 47, 53),
            text: Color32::from_rgb(236, 238, 241),
            text_dim: Color32::from_rgb(160, 165, 174),
            text_faint: Color32::from_rgb(110, 115, 125),
            canvas: Color32::from_rgb(20, 21, 25),
            canvas_dot: Color32::from_rgba_premultiplied(255, 255, 255, 16),
            ok: Color32::from_rgb(76, 201, 128),
            warn: Color32::from_rgb(240, 180, 70),
            danger: Color32::from_rgb(238, 98, 98),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(244, 245, 247),
            surface: Color32::from_rgb(255, 255, 255),
            raised: Color32::from_rgb(240, 241, 244),
            raised_hover: Color32::from_rgb(231, 233, 238),
            border: Color32::from_rgb(222, 225, 231),
            text: Color32::from_rgb(24, 27, 33),
            text_dim: Color32::from_rgb(96, 102, 114),
            text_faint: Color32::from_rgb(150, 156, 168),
            canvas: Color32::from_rgb(236, 238, 242),
            canvas_dot: Color32::from_rgba_premultiplied(0, 0, 0, 22),
            ok: Color32::from_rgb(28, 160, 90),
            warn: Color32::from_rgb(200, 130, 10),
            danger: Color32::from_rgb(205, 58, 58),
        }
    }
}

pub fn accent_color(accent: Accent, dark: bool) -> Color32 {
    let (d, l) = match accent {
        Accent::Blue => ((88, 140, 255), (38, 99, 235)),
        Accent::Violet => ((158, 120, 255), (112, 66, 230)),
        Accent::Teal => ((45, 200, 190), (13, 148, 136)),
        Accent::Green => ((84, 204, 120), (22, 148, 76)),
        Accent::Orange => ((255, 152, 68), (217, 100, 14)),
        Accent::Pink => ((255, 112, 168), (219, 54, 120)),
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

/// Resolved theme: whether it is dark, its palette and the accent colour.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub p: Palette,
    pub accent: Color32,
}

impl Theme {
    pub fn resolve(mode: ThemeMode, accent: Accent, system_dark: Option<bool>) -> Self {
        let dark = match mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::Auto => system_dark.unwrap_or(true),
        };
        Self {
            dark,
            p: palette(dark),
            accent: accent_color(accent, dark),
        }
    }

    /// Text colour readable on top of the accent.
    pub fn on_accent(&self) -> Color32 {
        Color32::WHITE
    }

    /// Accent washed out for backgrounds of selected rows and chips.
    pub fn accent_soft(&self) -> Color32 {
        self.accent
            .gamma_multiply(if self.dark { 0.22 } else { 0.14 })
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let p = &self.p;
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        v.override_text_color = Some(p.text);
        v.panel_fill = p.bg;
        v.window_fill = p.surface;
        v.extreme_bg_color = p.raised;
        v.faint_bg_color = p.raised;
        v.code_bg_color = p.raised;
        v.window_stroke = Stroke::new(1.0_f32, p.border);
        v.window_shadow = Shadow {
            offset: egui::vec2(0.0, 8.0),
            blur: 24.0,
            spread: 0.0,
            color: Color32::from_black_alpha(if self.dark { 120 } else { 40 }),
        };
        v.popup_shadow = v.window_shadow;
        v.window_rounding = Rounding::same(12.0);
        v.menu_rounding = Rounding::same(RADIUS);
        v.selection.bg_fill = self.accent.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0_f32, self.accent);
        v.hyperlink_color = self.accent;
        v.slider_trailing_fill = true;

        let r = Rounding::same(RADIUS);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.surface;
        w.noninteractive.weak_bg_fill = p.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
        w.noninteractive.rounding = r;
        w.inactive.bg_fill = p.raised;
        w.inactive.weak_bg_fill = p.raised;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
        w.inactive.rounding = r;
        w.hovered.bg_fill = p.raised_hover;
        w.hovered.weak_bg_fill = p.raised_hover;
        w.hovered.bg_stroke = Stroke::new(1.0_f32, p.border);
        w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
        w.hovered.rounding = r;
        w.active.bg_fill = p.raised_hover;
        w.active.weak_bg_fill = p.raised_hover;
        w.active.bg_stroke = Stroke::new(1.0_f32, self.accent);
        w.active.fg_stroke = Stroke::new(1.0_f32, p.text);
        w.active.rounding = r;
        w.open.bg_fill = p.raised_hover;
        w.open.weak_bg_fill = p.raised_hover;
        w.open.rounding = r;
        ctx.set_visuals(v);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(SP_2, SP_2 - 2.0);
        style.spacing.button_padding = egui::vec2(SP_3, 6.0);
        style.spacing.window_margin = egui::Margin::same(SP_4);
        style.spacing.interact_size.y = 30.0;
        style.spacing.indent = SP_3;
        style.spacing.slider_width = 160.0;
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(11.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
        ctx.set_style(style);
    }
}
