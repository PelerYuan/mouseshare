//! Font setup: the Phosphor icon font (bundled), a modern UI face if the
//! system has one, and a CJK fallback so Chinese text renders. CJK fonts
//! are far too large to bundle, so they are discovered on the system.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// (file name, TTC face index). Searched case-insensitively.
const CJK_CANDIDATES: &[(&str, u32)] = &[
    ("NotoSansCJK-Regular.ttc", 2),
    ("NotoSansSC-Regular.otf", 0),
    ("NotoSansSC-Regular.ttf", 0),
    ("NotoSansCJKsc-Regular.otf", 0),
    ("SourceHanSansSC-Regular.otf", 0),
    ("SourceHanSansCN-Regular.otf", 0),
    ("wqy-microhei.ttc", 0),
    ("wqy-zenhei.ttc", 0),
    ("DroidSansFallbackFull.ttf", 0),
    ("msyh.ttc", 0),
];

const UI_CANDIDATES: &[&str] = &[
    "Inter-Regular.ttf",
    "Inter-Regular.otf",
    "InterVariable.ttf",
    "Inter.ttf",
    "NotoSans-Regular.ttf",
    "Cantarell-Regular.otf",
    "Cantarell-VF.otf",
];

fn font_dirs() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        v.push(home.join(".local/share/fonts"));
        v.push(home.join(".fonts"));
    }
    v
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, depth - 1, out);
        } else {
            out.push(p);
        }
    }
}

fn all_font_files() -> &'static Vec<PathBuf> {
    static FILES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut out = Vec::new();
        for d in font_dirs() {
            walk(&d, 5, &mut out);
        }
        out
    })
}

fn find(name: &str) -> Option<PathBuf> {
    all_font_files()
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .cloned()
}

fn find_cjk() -> Option<(PathBuf, u32)> {
    CJK_CANDIDATES
        .iter()
        .find_map(|(n, idx)| find(n).map(|p| (p, *idx)))
}

/// Whether a CJK-capable font exists on this system.
pub fn cjk_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| find_cjk().is_some())
}

pub fn install(ctx: &egui::Context, with_cjk: bool) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    if let Some(path) = UI_CANDIDATES.iter().find_map(|n| find(n)) {
        if let Ok(bytes) = std::fs::read(&path) {
            fonts
                .font_data
                .insert("ui".into(), FontData::from_owned(bytes));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "ui".into());
        }
    }

    if with_cjk {
        if let Some((path, index)) = find_cjk() {
            if let Ok(bytes) = std::fs::read(&path) {
                let mut data = FontData::from_owned(bytes);
                data.index = index;
                fonts.font_data.insert("cjk".into(), data);
                for family in [FontFamily::Proportional, FontFamily::Monospace] {
                    fonts.families.entry(family).or_default().push("cjk".into());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
}
