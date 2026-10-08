//! UI fonts: Noto Sans (default), Inter and Roboto are embedded (SIL OFL,
//! see assets/fonts/LICENSE-*.txt); installed system fonts are listed too.

use std::path::PathBuf;
use std::sync::Arc;

pub const INTER: &str = "Inter";
pub const NOTO: &str = "Noto Sans";
pub const ROBOTO: &str = "Roboto";

static INTER_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static NOTO_TTF: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
static ROBOTO_TTF: &[u8] = include_bytes!("../../assets/fonts/Roboto-Regular.ttf");

#[derive(Debug, Clone)]
pub struct SystemFont {
    pub name: String,
    pub path: PathBuf,
}

/// Family name and "is regular" flag read from the font's name table.
fn font_family(path: &std::path::Path) -> Option<(String, bool)> {
    let data = std::fs::read(path).ok()?;
    let face = ttf_parser::Face::parse(&data, 0).ok()?;
    let mut family = None;
    let mut sub = None;
    for name in face.names() {
        let is_en = name.language_id == 0x0409 || !name.is_unicode() || name.platform_id == ttf_parser::PlatformId::Unicode;
        match name.name_id {
            // typographic family/subfamily first, legacy ones as fallback
            16 if is_en => family = name.to_string().or(family),
            1 if family.is_none() && is_en => family = name.to_string(),
            17 if is_en => sub = name.to_string().or(sub),
            2 if sub.is_none() && is_en => sub = name.to_string(),
            _ => {}
        }
    }
    let regular = !face.is_bold() && !face.is_italic() && face.weight().to_number() >= 350 && face.weight().to_number() <= 450;
    let sub_ok = sub.as_deref().is_none_or(|s| {
        let s = s.to_lowercase();
        s == "regular" || s == "normal" || s == "book" || s == "roman"
    });
    Some((family?.trim().to_owned(), regular && sub_ok))
}

/// Regular-weight TrueType/OpenType fonts installed on the system, by family name.
pub fn system_fonts() -> Vec<SystemFont> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        dirs.push(windir.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/fonts"));
        }
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
    }
    let mut out: Vec<SystemFont> = Vec::new();
    let mut stack = dirs;
    let mut visited = 0;
    while let Some(d) = stack.pop() {
        visited += 1;
        if visited > 64 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("ttf") | Some("otf")) {
                continue;
            }
            let Some((name, regular)) = font_family(&path) else {
                continue;
            };
            if !regular || name.is_empty() || out.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
                continue;
            }
            // symbol-only fonts are useless for UI text
            let lower = name.to_lowercase();
            if ["wingdings", "webdings", "symbol", "marlett", "mdl2", "fluent icons", "emoji"]
                .iter()
                .any(|s| lower.contains(s))
            {
                continue;
            }
            out.push(SystemFont { name, path });
        }
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

/// Vertical glyph shift (fraction of the font size) that puts the optical
/// center of the text (between baseline, x-height and cap height) on the
/// center of egui's text row (ascender..descender), so labels line up with
/// icons and boxes whatever the font metrics.
fn optical_center_offset(font: &[u8]) -> f32 {
    let Ok(face) = ttf_parser::Face::parse(font, 0) else {
        return 0.0;
    };
    let upm = face.units_per_em() as f32;
    if upm <= 0.0 {
        return 0.0;
    }
    let ascent = face.ascender() as f32;
    let descent = face.descender() as f32; // negative
    let cap = face.capital_height().map(|v| v as f32).unwrap_or(ascent * 0.7);
    let x = face.x_height().map(|v| v as f32).unwrap_or(cap * 0.72);
    let row_center = (ascent + descent) * 0.5; // above the baseline
    let optical = (cap + x) * 0.25; // above the baseline
    // text sitting low (row center above the optical center) moves up
    (-(row_center - optical) / upm).clamp(-0.2, 0.2)
}

/// Install the chosen font as the primary UI font (egui's defaults stay as
/// fallbacks for symbols and emoji). `choice` is a built-in name or
/// "system:<path>".
pub fn apply(ctx: &egui::Context, choice: &str) {
    let data: Option<egui::FontData> = match choice {
        INTER => Some(egui::FontData::from_static(INTER_TTF)),
        NOTO | "" => Some(egui::FontData::from_static(NOTO_TTF)),
        ROBOTO => Some(egui::FontData::from_static(ROBOTO_TTF)),
        other => other
            .strip_prefix("system:")
            .and_then(|p| std::fs::read(p).ok())
            .map(egui::FontData::from_owned),
    };
    let data = data.unwrap_or_else(|| {
        log::warn!("font '{choice}' unavailable, using Noto Sans");
        egui::FontData::from_static(NOTO_TTF)
    });
    let mut data = data;
    data.tweak.y_offset_factor = optical_center_offset(&data.font);
    let mut defs = egui::FontDefinitions::default();
    defs.font_data.insert("aurora-main".into(), Arc::new(data));
    if let Some(f) = defs.families.get_mut(&egui::FontFamily::Proportional) {
        f.insert(0, "aurora-main".into());
    }
    if let Some(f) = defs.families.get_mut(&egui::FontFamily::Monospace) {
        f.push("aurora-main".into());
    }
    ctx.set_fonts(defs);
}
