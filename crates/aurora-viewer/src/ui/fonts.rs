//! UI fonts: Inter (default), Noto Sans, Roboto and OpenDyslexic (its
//! letters weighted at the bottom, easier to read with dyslexia; #851) are
//! embedded with their semi-bold face (SIL OFL, see
//! assets/fonts/LICENSE-*.txt); installed
//! system fonts are listed too. Characters none of them has come from the
//! system's fonts (`fallback`).

use std::path::PathBuf;
use std::sync::Arc;

pub mod fallback;

pub const INTER: &str = "Inter";
pub const NOTO: &str = "Noto Sans";
pub const ROBOTO: &str = "Roboto";
pub const OPEN_DYSLEXIC: &str = "OpenDyslexic";

/// The interface font of a new profile, and of a choice that cannot be
/// loaded any more.
pub const DEFAULT: &str = INTER;

static INTER_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static INTER_SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
static NOTO_TTF: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
static NOTO_SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/NotoSans-SemiBold.ttf");
static ROBOTO_TTF: &[u8] = include_bytes!("../../assets/fonts/Roboto-Regular.ttf");
// Roboto has no static SemiBold: Medium is its emphasis weight
static ROBOTO_MEDIUM_TTF: &[u8] = include_bytes!("../../assets/fonts/Roboto-Medium.ttf");
// OpenDyslexic has no semi-bold: Bold is its emphasis weight
static OPEN_DYSLEXIC_OTF: &[u8] = include_bytes!("../../assets/fonts/OpenDyslexic-Regular.otf");
static OPEN_DYSLEXIC_BOLD_OTF: &[u8] = include_bytes!("../../assets/fonts/OpenDyslexic-Bold.otf");

/// An embedded interface font: its regular face and the face of its
/// semi-bold texts.
pub struct Builtin {
    pub name: &'static str,
    regular: &'static [u8],
    semibold: &'static [u8],
}

/// The embedded fonts, in the order of the Preferences.
pub const BUILTINS: [Builtin; 4] = [
    Builtin {
        name: INTER,
        regular: INTER_TTF,
        semibold: INTER_SEMIBOLD_TTF,
    },
    Builtin {
        name: NOTO,
        regular: NOTO_TTF,
        semibold: NOTO_SEMIBOLD_TTF,
    },
    Builtin {
        name: ROBOTO,
        regular: ROBOTO_TTF,
        semibold: ROBOTO_MEDIUM_TTF,
    },
    Builtin {
        name: OPEN_DYSLEXIC,
        regular: OPEN_DYSLEXIC_OTF,
        semibold: OPEN_DYSLEXIC_BOLD_OTF,
    },
];

/// The embedded font of a setting; "" (files older than the setting) is
/// the default one.
pub fn builtin(choice: &str) -> Option<&'static Builtin> {
    let name = if choice.is_empty() { DEFAULT } else { choice };
    BUILTINS.iter().find(|b| b.name == name)
}

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

/// Where the system keeps its fonts (their subdirectories count too).
fn font_dirs() -> Vec<PathBuf> {
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
    dirs
}

/// Regular-weight TrueType/OpenType fonts installed on the system, by family name.
pub fn system_fonts() -> Vec<SystemFont> {
    let mut out: Vec<SystemFont> = Vec::new();
    let mut stack = font_dirs();
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
/// fallbacks for symbols and emoji), then the system's fallback fonts for
/// every other script. `choice` is a built-in name or "system:<path>".
///
/// Returns false while the system fonts are still being looked for: they
/// are then missing from this call, which has to be made again once
/// `fallback::ready()` (the viewer does it at the start of a frame).
pub fn apply(ctx: &egui::Context, choice: &str) -> bool {
    let system = fallback::ready();
    install(ctx, definitions_for(choice, system.unwrap_or_default()));
    system.is_some()
}

const DEFINITIONS_ID: &str = "aurora-font-definitions";
const SEMIBOLD_ID: &str = "aurora-font-has-semibold";
const SCALE_ID: &str = "aurora-font-scale";

/// Combined skin and preference scale, also used by inline emoji pictures.
pub fn scale(ctx: &egui::Context) -> f32 {
    ctx.data(|data| data.get_temp(egui::Id::unique(SCALE_ID))).unwrap_or(1.0)
}

/// Scale the font faces, including explicit FontIds and rich text sizes.
/// In egui 0.36 these metrics drive shaping, wrapping and row height as well
/// as rasterization. Scaling text styles alone misses custom-painted text.
pub fn set_scale(ctx: &egui::Context, scale: f32) {
    if self::scale(ctx) == scale {
        return;
    }
    let definitions = ctx.data_mut(|data| {
        data.insert_temp(egui::Id::unique(SCALE_ID), scale);
        data.get_temp::<Arc<egui::FontDefinitions>>(egui::Id::unique(DEFINITIONS_ID))
    });
    install(ctx, definitions.as_deref().cloned().unwrap_or_default());
}

const CHAT_ID: &str = "aurora-chat-text-ratio";

/// Size the conversations' text on their own (Preferences › Chat › Text
/// size): `chat_scale` replaces the interface's `text_scale` for them, the
/// skin's scale kept. The faces stay scaled for the interface, so the
/// conversations ask `chat_size` for every size they draw.
pub fn set_chat_scale(ctx: &egui::Context, chat_scale: f32, text_scale: f32) {
    let ratio = chat_ratio(chat_scale, text_scale);
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(CHAT_ID), ratio));
}

fn chat_ratio(chat_scale: f32, text_scale: f32) -> f32 {
    let ratio = chat_scale / text_scale;
    if ratio.is_finite() && ratio > 0.0 { ratio } else { 1.0 }
}

/// A text size of the conversations, once the faces' scale is applied
/// the size the chat's own text size asks for.
pub fn chat_size(ctx: &egui::Context, size: f32) -> f32 {
    size * ctx.data(|data| data.get_temp(egui::Id::unique(CHAT_ID))).unwrap_or(1.0)
}

/// Keep the unscaled faces so live changes never compound their scale and
/// late system fallbacks or a font change inherit the current preference.
fn install(ctx: &egui::Context, definitions: egui::FontDefinitions) {
    let heavier =
        definitions.font_data.contains_key(SEMIBOLD) || definitions.font_data.get(MAIN).is_some_and(|face| has_weight_axis(face.bytes()));
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::unique(DEFINITIONS_ID), Arc::new(definitions.clone()));
        data.insert_temp(egui::Id::unique(SEMIBOLD_ID), heavier);
    });
    let scale = scale(ctx);
    let mut scaled = definitions;
    for face in scaled.font_data.values_mut() {
        let face = Arc::make_mut(face);
        face.tweak.scale *= scale;
        face.tweak.y_offset *= scale;
    }
    ctx.set_fonts(scaled);
}

/// The fonts of `choice`: an embedded font with its own semi-bold face,
/// or a system font that keeps its own design for the semi-bold texts too
/// (its `wght` axis when it is a variable font, see `semibold_format`,
/// else its regular face). A system font that cannot be read gives way to
/// the default font.
fn definitions_for(choice: &str, system: &[fallback::SystemFace]) -> egui::FontDefinitions {
    let file = match choice.strip_prefix("system:") {
        Some(path) => std::fs::read(path)
            .inspect_err(|e| log::warn!("font '{path}' unavailable ({e}), using {DEFAULT}"))
            .ok(),
        None => None,
    };
    let (regular, semibold) = match file {
        Some(bytes) => (egui::FontData::from_owned(bytes), None),
        None => {
            let b = builtin(choice).unwrap_or_else(|| {
                if !choice.starts_with("system:") {
                    log::warn!("unknown font '{choice}', using {DEFAULT}");
                }
                &BUILTINS[0]
            });
            (egui::FontData::from_static(b.regular), Some(b.semibold))
        }
    };
    let mut defs = definitions(regular, system);
    if let Some(semibold) = semibold {
        let mut face = egui::FontData::from_static(semibold);
        face.tweak.y_offset_factor = optical_center_offset(face.bytes());
        defs.font_data.insert(SEMIBOLD.into(), Arc::new(face));
        if let Some(f) = defs.families.get_mut(&semibold_family()) {
            f.insert(0, SEMIBOLD.into());
        }
    }
    defs
}

/// Name of the semi-bold family and of its face.
const SEMIBOLD: &str = "aurora-semibold";
/// Name of the interface font's regular face.
const MAIN: &str = "aurora-main";
const ICON_FALLBACK: &str = "emoji-icon-font";

fn semibold_family() -> egui::FontFamily {
    egui::FontFamily::Name(SEMIBOLD.into())
}

/// Family of the semi-bold texts (the name on an avatar's name plate):
/// the semi-bold face of the embedded font, else the interface font. A
/// context without the viewer's fonts (tests) gets the proportional
/// family, since egui panics on a family it does not know.
pub fn semibold(ctx: &egui::Context) -> egui::FontFamily {
    let known = ctx.fonts(|f| f.definitions().families.contains_key(&semibold_family()));
    if known { semibold_family() } else { egui::FontFamily::Proportional }
}

/// Format of a text in the `semibold` family. The `wght` axis makes a
/// variable system font semi-bold too; a static font ignores it.
pub fn semibold_format(family: &egui::FontFamily, size: f32, color: egui::Color32) -> egui::TextFormat {
    let mut format = egui::TextFormat::simple(egui::FontId::new(size, family.clone()), color);
    format.coords = egui::epaint::text::VariationCoords::new([(b"wght", 600.0)]);
    format
}

/// Emphasis shared by plain text, emoji runs and laid-out chat cards.
/// `RichText::strong` changes the colour only; weight needs our font face
/// and the variable-font axis, as in `semibold_format`.
#[derive(Clone, Copy, Default)]
pub(crate) struct TextEmphasis {
    pub italics: bool,
    pub strong: bool,
}

impl From<bool> for TextEmphasis {
    fn from(italics: bool) -> Self {
        Self { italics, strong: false }
    }
}

impl TextEmphasis {
    pub fn rich_text(self, ctx: &egui::Context, text: impl Into<String>, size: f32, color: egui::Color32) -> egui::RichText {
        let mut text = egui::RichText::new(text).size(size).color(color);
        if self.strong {
            text = text.family(semibold(ctx)).variation(b"wght", 600.0);
        }
        if self.italics {
            text = text.italics();
        }
        text
    }

    pub fn format(self, ctx: &egui::Context, size: f32, color: egui::Color32) -> egui::TextFormat {
        let mut format = if self.strong {
            semibold_format(&semibold(ctx), size, color)
        } else {
            egui::TextFormat::simple(egui::FontId::proportional(size), color)
        };
        format.italics = self.italics;
        format
    }
}

/// Does `semibold_format` draw heavier than the regular text? Yes with an
/// embedded font or a variable system font; a static system font draws its
/// semi-bold texts with its regular face.
pub fn has_semibold(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp(egui::Id::unique(SEMIBOLD_ID))).unwrap_or(false)
}

fn has_weight_axis(font: &[u8]) -> bool {
    let wght = ttf_parser::Tag::from_bytes(b"wght");
    ttf_parser::Face::parse(font, 0).is_ok_and(|face| face.variation_axes().into_iter().any(|axis| axis.tag == wght))
}

/// `primary` first for proportional text and after egui's text fonts for
/// monospace, then the system faces before egui's icon font. That icon
/// font has fullwidth Latin glyphs with uneven feet; prefer text faces,
/// as LLFontRegistry::createFont does (indra/newview/fonts/fonts.xml).
fn definitions(mut primary: egui::FontData, system: &[fallback::SystemFace]) -> egui::FontDefinitions {
    primary.tweak.y_offset_factor = optical_center_offset(primary.bytes());
    let shift = primary.tweak.y_offset_factor;
    let row = ttf_parser::Face::parse(primary.bytes(), 0)
        .ok()
        .and_then(|face| fallback::RowMetrics::of(&face));
    let mut defs = egui::FontDefinitions::default();
    defs.font_data.insert(MAIN.into(), Arc::new(primary));
    if let Some(f) = defs.families.get_mut(&egui::FontFamily::Proportional) {
        f.insert(0, MAIN.into());
    }
    if let Some(f) = defs.families.get_mut(&egui::FontFamily::Monospace) {
        let before_icons = f.iter().position(|name| name == ICON_FALLBACK).unwrap_or(f.len());
        f.insert(before_icons, MAIN.into());
    }
    // the semi-bold family starts as the proportional one: `apply` puts
    // its own face first when it has one
    let proportional = defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    defs.families.insert(semibold_family(), proportional);
    for face in system {
        defs.font_data.insert(face.key.clone(), Arc::new(face.font_data(row, shift)));
        for family in defs.families.values_mut() {
            let before_icons = family.iter().position(|name| name == ICON_FALLBACK).unwrap_or(family.len());
            family.insert(before_icons, face.key.clone());
        }
    }
    defs
}

/// Is `c` drawn by the fonts shipped with the viewer, whatever the system?
/// One of the embedded interface fonts, or egui's own fallbacks.
#[cfg(test)]
fn embedded_has(c: char) -> bool {
    let has = |font: &[u8]| ttf_parser::Face::parse(font, 0).is_ok_and(|face| face.glyph_index(c).is_some());
    let defs = egui::FontDefinitions::default();
    let fallbacks = defs.families.get(&egui::FontFamily::Proportional).into_iter().flatten();
    BUILTINS.iter().any(|b| has(b.regular) || has(b.semibold))
        || fallbacks.filter_map(|name| defs.font_data.get(name)).any(|d| has(d.bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(ctx: &egui::Context, family: egui::FontFamily, size: f32, width: f32) -> (egui::Vec2, usize) {
        let mut measured = Default::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout(
                    "Preferences and conversations with readable text".into(),
                    egui::FontId::new(size, family.clone()),
                    egui::Color32::PLACEHOLDER,
                    width,
                )
            });
            measured = (galley.size(), galley.rows.len());
        });
        output.textures_delta.clear();
        measured
    }

    #[test]
    fn text_scale_covers_explicit_fonts_and_wrapping() {
        for builtin in &BUILTINS {
            let ctx = egui::Context::default();
            install(&ctx, definitions_for(builtin.name, &[]));
            for family in [egui::FontFamily::Proportional, semibold_family(), egui::FontFamily::Monospace] {
                set_scale(&ctx, 1.0);
                let (base, _) = measure(&ctx, family.clone(), 13.0, f32::INFINITY);
                let widths = [100.0, 130.0, 160.0, 200.0];
                let base_rows = widths.map(|width| measure(&ctx, family.clone(), 13.0, width).1);
                for scale in [0.8, 1.4] {
                    set_scale(&ctx, scale);
                    let (scaled, _) = measure(&ctx, family.clone(), 13.0, f32::INFINITY);
                    assert!(
                        (scaled.x / base.x - scale).abs() < 0.03,
                        "{} {family}: {base:?} -> {scaled:?}",
                        builtin.name
                    );
                    assert!(
                        (scaled.y / base.y - scale).abs() < 0.06,
                        "{} {family}: {base:?} -> {scaled:?}",
                        builtin.name
                    );
                }
                let scaled_rows = widths.map(|width| measure(&ctx, family.clone(), 13.0, width).1);
                assert!(
                    scaled_rows.iter().zip(base_rows).all(|(scaled, base)| *scaled >= base)
                        && scaled_rows.iter().zip(base_rows).any(|(scaled, base)| *scaled > base),
                    "{} {family}: larger text must wrap sooner: {base_rows:?} -> {scaled_rows:?}",
                    builtin.name
                );
            }
        }
    }

    #[test]
    fn only_a_font_with_a_heavier_face_skips_the_faux_bold() {
        let ctx = egui::Context::default();
        assert!(!has_semibold(&ctx), "a context without the viewer's fonts");
        for builtin in &BUILTINS {
            install(&ctx, definitions_for(builtin.name, &[]));
            assert!(has_semibold(&ctx), "{}", builtin.name);
            set_scale(&ctx, 1.3);
            assert!(has_semibold(&ctx), "{} rescaled", builtin.name);
            set_scale(&ctx, 1.0);
        }
        // a static system font: its regular face only, no weight axis
        install(&ctx, definitions(egui::FontData::from_static(INTER_TTF), &[]));
        assert!(!has_semibold(&ctx));
    }

    #[test]
    fn text_scale_is_live_idempotent_and_survives_font_installation() {
        let ctx = egui::Context::default();
        set_scale(&ctx, 1.4); // theme is loaded before the chosen font
        install(&ctx, definitions_for(INTER, &[]));
        let family = egui::FontFamily::Proportional;
        let (large, _) = measure(&ctx, family.clone(), 13.0, f32::INFINITY);
        set_scale(&ctx, 1.4);
        let (again, _) = measure(&ctx, family.clone(), 13.0, f32::INFINITY);
        assert_eq!(large, again);
        set_scale(&ctx, 0.8);
        let (small, _) = measure(&ctx, family.clone(), 13.0, f32::INFINITY);
        assert!(small.x < large.x);
        set_scale(&ctx, 1.0);
        let (reset, _) = measure(&ctx, family.clone(), 13.0, f32::INFINITY);
        install(&ctx, definitions_for(INTER, &[])); // late fallbacks or font switch
        let (fresh, _) = measure(&ctx, family, 13.0, f32::INFINITY);
        assert_eq!(reset, fresh);
        assert!((large.x / reset.x - 1.4).abs() < 0.03);
    }

    #[test]
    fn text_scale_combines_theme_and_preference_once_for_all_text() {
        let ctx = egui::Context::default();
        let mut theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        install(&ctx, definitions_for(INTER, &[]));
        let size = egui::TextStyle::Body.resolve(&ctx.style_of(ctx.theme())).size;
        let family = egui::FontFamily::Proportional;
        let (base, _) = measure(&ctx, family.clone(), size, f32::INFINITY);
        theme.font_scale = 1.2;
        theme.apply(&ctx, 1.1);
        assert_eq!(egui::TextStyle::Body.resolve(&ctx.style_of(ctx.theme())).size, size);
        let (scaled, _) = measure(&ctx, family, size, f32::INFINITY);
        assert!((scaled.x / base.x - 1.32).abs() < 0.03);
    }

    #[test]
    fn chat_text_follows_its_own_size_whatever_the_interface_text_size() {
        let ctx = egui::Context::default();
        let mut theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        install(&ctx, definitions_for(INTER, &[]));
        let family = egui::FontFamily::Proportional;
        // no chat size set yet: the conversations draw as the interface
        assert_eq!(chat_size(&ctx, 13.5), 13.5);
        let (base, _) = measure(&ctx, family.clone(), 13.5, f32::INFINITY);
        // the same chat text at 1.5×, the interface's text small or large
        for text in [0.8, 1.0, 2.0] {
            theme.apply(&ctx, text);
            set_chat_scale(&ctx, 1.5, text);
            let (chat, _) = measure(&ctx, family.clone(), chat_size(&ctx, 13.5), f32::INFINITY);
            assert!((chat.x / base.x - 1.5).abs() < 0.03, "text {text}: {}", chat.x / base.x);
        }
        // the skin's own scale stays on the chat too
        theme.font_scale = 1.2;
        theme.apply(&ctx, 2.0);
        set_chat_scale(&ctx, 1.0, 2.0);
        let (chat, _) = measure(&ctx, family, chat_size(&ctx, 13.5), f32::INFINITY);
        assert!((chat.x / base.x - 1.2).abs() < 0.03);
    }

    #[test]
    fn chat_ratio_never_breaks_the_text() {
        assert_eq!(chat_ratio(1.5, 1.0), 1.5);
        assert_eq!(chat_ratio(1.0, 2.0), 0.5);
        assert_eq!(chat_ratio(1.0, 0.0), 1.0);
        assert_eq!(chat_ratio(f32::NAN, 1.0), 1.0);
    }

    #[test]
    fn text_scale_preserves_fallback_corrections() {
        let ctx = egui::Context::default();
        let original = definitions_for(INTER, fallback::wait());
        install(&ctx, original.clone());
        set_scale(&ctx, 1.4);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.fonts(|fonts| {
                for (name, face) in &original.font_data {
                    let scaled = &fonts.definitions().font_data[name];
                    assert_eq!(scaled.tweak.scale, face.tweak.scale * 1.4, "{name}");
                    assert_eq!(scaled.tweak.y_offset_factor, face.tweak.y_offset_factor, "{name}");
                }
            });
        });
        output.textures_delta.clear();
    }

    /// Noto Sans with these system faces: the default font when the
    /// baseline and right-to-left tests below were written.
    fn noto(system: &[fallback::SystemFace]) -> egui::FontDefinitions {
        definitions(egui::FontData::from_static(NOTO_TTF), system)
    }

    /// Text the interface font must draw without a square, script by
    /// script: what residents write besides the demo scenario's names.
    const SCRIPTS: &[(&str, &str)] = &[
        (
            "Latin with accents",
            "Ça va ? Été, forêt, ÀÉÎÕÜ çñß Łőű Žluťoučký kůň Ĳ ſ «guillemets» ‘’ “” – — … € £ ¥ © ® ™ ° ± × ÷ ½ №",
        ),
        ("Vietnamese", "Tiếng Việt: Xin chào thế giới, ước mơ, Đường phố ỹ ỵ ở ẫ ặ ằ"),
        ("Greek", "Ελληνικά: Γειά σου κόσμε! ΆΈΉ ώ"),
        ("Cyrillic", "Русский: Привет, мир! ЁЇЄҐ ё"),
        (
            "CJK",
            "日本語 こんにちは カタカナ · 한국어 안녕하세요 · 中文 你好，世界 · 繁體中文 · ｆｕｌｌｗｉｄｔｈ",
        ),
        ("Arabic", "العربية: مرحبا بالعالم ۞ ۩"),
        ("Hebrew", "עברית: שלום עולם"),
        ("Thai", "ภาษาไทย สวัสดี ๑"),
        ("Devanagari", "हिन्दी नमस्ते ॐ"),
        ("symbols", "░▒▓█ ╔═╗ → ⇒ ↑↓ ≤ ≥ ★ ♛ ♪ ✿ ❀ ∞ ①② ⅓ 𝓐𝐀 ღ"),
        ("emoji", "😀 🎉 ❤ 👍 🫠 🐺 🦊 ✨ 🌙"),
    ];

    /// The characters of `text` egui draws with no font of the family.
    fn squares(defs: egui::FontDefinitions, family: egui::FontFamily, text: &str) -> String {
        drawable(defs, family, text)
            .into_iter()
            .filter(|(c, ok)| !ok && !c.is_whitespace())
            .map(|(c, _)| c)
            .collect()
    }

    /// For each character of `text`, the name of the first face of the
    /// family that draws it.
    fn faces_drawing(defs: egui::FontDefinitions, family: egui::FontFamily, text: &str) -> Vec<(char, Option<String>)> {
        let mut fonts = egui::epaint::Fonts::new(Default::default(), defs);
        let mut fonts = fonts.with_pixels_per_point(1.0);
        let known = fonts.characters(&family);
        text.chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| (c, known.get(&c).and_then(|faces| faces.first().cloned())))
            .collect()
    }

    /// Which characters egui draws with these definitions, in a font of the
    /// family: its own faces and character maps, as in the UI. (Not
    /// `has_glyph`, which answers no for every character of the face that
    /// holds egui's replacement square: the emoji.)
    fn drawable(defs: egui::FontDefinitions, family: egui::FontFamily, text: &str) -> Vec<(char, bool)> {
        let mut fonts = egui::epaint::Fonts::new(Default::default(), defs);
        let mut fonts = fonts.with_pixels_per_point(1.0);
        let known = fonts.characters(&family);
        text.chars().map(|c| (c, known.contains_key(&c))).collect()
    }

    fn demo_text() -> Vec<&'static str> {
        let samples = crate::demo::glyphs::SAMPLES.iter().flat_map(|s| [s.title, s.display_name, s.chat]);
        samples.chain([crate::demo::glyphs::HOVER_TEXT]).collect()
    }

    #[test]
    fn embedded_fonts_cover_latin_greek_cyrillic_only() {
        for c in "Aé ΩЯ €ℓ →".chars() {
            assert!(embedded_has(c), "{c:?}");
        }
        // the reported square (#127: a group title wrapped in U+10E6), and
        // the other usual ones of display names
        for c in "ღ𝓐𝐀中あ한بא░".chars() {
            assert!(
                !embedded_has(c),
                "{c:?} is embedded: the system fallback is no longer needed for it"
            );
        }
    }

    #[test]
    fn system_text_faces_precede_the_icon_font_in_every_family() {
        let system = fallback::wait();
        let (without, with) = (noto(&[]), noto(system));
        for (family, fonts) in &with.families {
            let embedded = &without.families[family];
            let text_len = embedded.len() - 1;
            assert_eq!(embedded.last().map(String::as_str), Some(ICON_FALLBACK));
            assert_eq!(&fonts[..text_len], &embedded[..text_len], "{family}");
            assert!(
                fonts[text_len..fonts.len() - 1].iter().all(|f| f.starts_with("system:")),
                "{family}: {fonts:?}"
            );
            assert_eq!(fonts.last().map(String::as_str), Some(ICON_FALLBACK));
            assert_eq!(fonts.len() - embedded.len(), system.len(), "{family}");
        }
        assert_eq!(with.families[&egui::FontFamily::Proportional][0], "aurora-main");
        // egui panics on a family that names an unknown font
        for name in with.families.values().flatten() {
            assert!(with.font_data.contains_key(name), "{name}");
        }
    }

    #[test]
    fn fullwidth_latin_uses_text_faces_with_even_feet() {
        // These flat-footed glyphs sat at different heights in the icon
        // font, even though the layout gave them one baseline.
        const LETTERS: &str = "ＡＫａｎｈ";
        let system = fallback::wait();
        if !LETTERS.chars().all(|c| system.iter().any(|face| face.has(c))) {
            eprintln!("no system face for fullwidth Latin: not checked");
            return;
        }
        for builtin in &BUILTINS {
            for family in [egui::FontFamily::Proportional, semibold_family(), egui::FontFamily::Monospace] {
                let defs = definitions_for(builtin.name, system);
                for (c, face) in faces_drawing(defs.clone(), family.clone(), LETTERS) {
                    assert!(
                        face.as_deref().is_some_and(|name| name.starts_with("system:")),
                        "{} {family} {c}: {face:?}",
                        builtin.name
                    );
                }
                let mut fonts = egui::epaint::Fonts::new(Default::default(), defs);
                let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
                    LETTERS.into(),
                    egui::FontId::new(64.0, family.clone()),
                    egui::Color32::WHITE,
                );
                let mut low = f32::INFINITY;
                let mut high = f32::NEG_INFINITY;
                for glyph in &galley.rows[0].glyphs {
                    let foot = glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y;
                    low = low.min(foot);
                    high = high.max(foot);
                }
                assert!(high - low <= 64.0 * 0.04, "{} {family}: feet {low}..{high}", builtin.name);
            }
        }
    }

    /// Every character of the demo scenario (display names, group titles,
    /// chat, hover text) is drawn once the system fonts are there. On a
    /// system without one of the fonts (not a complete Windows 10 / 11), the
    /// missing characters are listed and the test passes: the viewer shows
    /// squares for them, as before.
    #[test]
    fn demo_characters_are_drawn_with_the_system_fonts() {
        let system = fallback::wait();
        let complete = fallback::CANDIDATES.iter().all(|c| system.iter().any(|f| f.has(c.probe)));
        let text = demo_text().join(" ");
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let missing = squares(definitions_for(DEFAULT, system), family.clone(), &text);
            if complete {
                assert!(missing.is_empty(), "{family}: no font for {missing:?}");
            } else if !missing.is_empty() {
                eprintln!("{family}: system fonts incomplete, squares for {missing:?}");
            }
        }
    }

    /// The text of the demo scenario and of every script, with each
    /// embedded interface font as the primary one, in its regular and
    /// semi-bold families: the system faces must not depend on the font
    /// chosen in Preferences, nor on the weight.
    #[test]
    fn every_script_is_drawn_whatever_the_builtin_font_and_weight() {
        let system = fallback::wait();
        if !fallback::CANDIDATES.iter().all(|c| system.iter().any(|f| f.has(c.probe))) {
            eprintln!("system fonts incomplete: not checked");
            return;
        }
        let text = demo_text()
            .into_iter()
            .chain(SCRIPTS.iter().map(|(_, t)| *t))
            .collect::<Vec<_>>()
            .join(" ");
        for b in &BUILTINS {
            for family in [egui::FontFamily::Proportional, semibold_family()] {
                let missing = squares(definitions_for(b.name, system), family.clone(), &text);
                assert!(missing.is_empty(), "{} ({family}): no font for {missing:?}", b.name);
            }
        }
    }

    /// The scripts the embedded fonts are chosen for (Latin with its
    /// accents, Vietnamese, Greek, Cyrillic) are drawn by every embedded
    /// face itself, regular and semi-bold: never by a system face of
    /// another design in the middle of a word.
    #[test]
    fn every_embedded_face_draws_latin_vietnamese_greek_and_cyrillic() {
        for b in &BUILTINS {
            for (weight, ttf) in [("regular", b.regular), ("semi-bold", b.semibold)] {
                let face = ttf_parser::Face::parse(ttf, 0).expect("embedded font");
                for (script, text) in &SCRIPTS[..4] {
                    let lacking: String = text
                        .chars()
                        .filter(|c| !c.is_whitespace() && face.glyph_index(*c).is_none())
                        .collect();
                    assert!(lacking.is_empty(), "{} {weight}, {script}: {lacking:?}", b.name);
                }
            }
        }
    }

    /// What Inter lacks is drawn by the faces that drew it when Noto Sans
    /// was the default: the system face of its script, or egui's emoji.
    /// Inter draws more of it itself (arrows, ≤ ≥, ★ ♪ ①); the few
    /// characters only Noto Sans had (small capitals ᴏ ᴜ, ℙ) come from a
    /// system face, never a square. Without the complete system fonts, only
    /// the order of the faces is compared.
    #[test]
    fn what_inter_lacks_comes_from_the_same_faces_as_with_noto() {
        let system = fallback::wait();
        let text = demo_text()
            .into_iter()
            .chain(SCRIPTS.iter().map(|(_, t)| *t))
            .collect::<Vec<_>>()
            .join(" ");
        for family in [egui::FontFamily::Proportional, semibold_family()] {
            let inter = faces_drawing(definitions_for(INTER, system), family.clone(), &text);
            let noto = faces_drawing(definitions_for(NOTO, system), family.clone(), &text);
            let own = |face: &Option<String>| matches!(face.as_deref(), Some("aurora-main" | SEMIBOLD));
            let (mut gained, mut lost) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
            for ((c, with_inter), (_, with_noto)) in inter.iter().zip(&noto) {
                match (own(with_inter), own(with_noto)) {
                    (false, false) => assert_eq!(with_inter, with_noto, "{family}: {c:?}"),
                    (false, true) => {
                        assert!(with_inter.is_some(), "{family}: Noto Sans draws {c:?}, Inter a square");
                        lost.insert(*c);
                    }
                    (true, false) => {
                        gained.insert(*c);
                    }
                    (true, true) => {}
                }
            }
            eprintln!("{family}: Inter draws {gained:?} itself (fallback faces with Noto Sans), {lost:?} come from the fallbacks");
        }
    }

    #[test]
    fn each_builtin_font_has_its_own_semibold_face() {
        let face = |defs: &egui::FontDefinitions, name: &str| defs.font_data.get(name).map(|d| d.bytes().to_vec());
        for choice in [
            "",
            INTER,
            NOTO,
            ROBOTO,
            OPEN_DYSLEXIC,
            "system:Z:\\no such font.ttf",
            "Retired Font",
        ] {
            let defs = definitions_for(choice, &[]);
            let b = builtin(choice).unwrap_or(&BUILTINS[0]);
            assert_eq!(face(&defs, "aurora-main").as_deref(), Some(b.regular), "{choice}");
            assert_eq!(face(&defs, SEMIBOLD).as_deref(), Some(b.semibold), "{choice}");
            assert_eq!(defs.families[&semibold_family()][0], SEMIBOLD, "{choice}");
        }
        assert_eq!(builtin("").map(|b| b.name), Some(INTER));
        // the semi-bold faces are a heavier design of the same family
        // (OpenDyslexic's Bold, declared 800: it has no lighter emphasis
        // weight)
        for b in &BUILTINS {
            let weight = |ttf| ttf_parser::Face::parse(ttf, 0).expect("embedded font").weight().to_number();
            let family = |ttf| font_family_name(ttf).expect("family name");
            assert_eq!(weight(b.regular), 400, "{}", b.name);
            let heaviest = if b.name == OPEN_DYSLEXIC { 800 } else { 600 };
            assert!((500..=heaviest).contains(&weight(b.semibold)), "{}", b.name);
            assert_eq!(family(b.regular), b.name);
            assert_eq!(family(b.semibold), b.name);
        }
    }

    /// A system font keeps its own design for the semi-bold texts: its
    /// regular face first in their family, no embedded semi-bold face.
    #[test]
    fn a_system_font_has_no_embedded_semibold_face() {
        let Some(font) = system_fonts().into_iter().next() else {
            eprintln!("no system font: not checked");
            return;
        };
        let defs = definitions_for(&format!("system:{}", font.path.display()), &[]);
        assert!(!defs.font_data.contains_key(SEMIBOLD));
        assert_eq!(defs.families[&semibold_family()][0], "aurora-main");
        assert_eq!(defs.font_data["aurora-main"].bytes(), std::fs::read(&font.path).expect("font file"));
    }

    /// Typographic family name of an embedded face (name ID 16, else 1).
    fn font_family_name(ttf: &[u8]) -> Option<String> {
        let face = ttf_parser::Face::parse(ttf, 0).ok()?;
        let name = |id| {
            face.names()
                .into_iter()
                .find(|n| n.name_id == id && n.is_unicode())
                .and_then(|n| n.to_string())
        };
        name(16).or_else(|| name(1))
    }

    #[test]
    fn without_system_fonts_the_reported_character_is_a_square() {
        let before = drawable(noto(&[]), egui::FontFamily::Proportional, "ღKemono Friendღ");
        assert_eq!(before.iter().filter(|(_, ok)| !ok).map(|(c, _)| *c).collect::<String>(), "ღღ");
        let system = fallback::wait();
        if system.iter().any(|f| f.has('ღ')) {
            let after = drawable(noto(system), egui::FontFamily::Proportional, "ღKemono Friendღ");
            assert!(after.iter().all(|(_, ok)| *ok), "{after:?}");
        } else {
            eprintln!("no installed font with Georgian: not checked");
        }
    }

    /// Bottom of the first glyph of `text`, in em below the top of its row.
    fn glyph_bottom(fonts: &mut egui::epaint::Fonts, text: &str) -> f32 {
        const SIZE: f32 = 64.0;
        let galley =
            fonts
                .with_pixels_per_point(1.0)
                .layout_no_wrap(text.to_owned(), egui::FontId::proportional(SIZE), egui::Color32::WHITE);
        let glyph = galley.rows[0].glyphs[0];
        (glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y) / SIZE
    }

    /// Letters of the system faces stand on the same line as the primary
    /// font's: without the baseline shift, Yu Gothic drew Japanese a quarter
    /// of the font size too high (and its neighbours a little too low).
    #[test]
    fn system_faces_share_the_primary_baseline() {
        let system = fallback::wait();
        let mut fonts = egui::epaint::Fonts::new(Default::default(), noto(system));
        let line = glyph_bottom(&mut fonts, "H");
        // flat-footed characters of other faces: bold H (symbols), Hebrew
        // bet, Georgian, Hangul, a kanji and a hanzi (drawn in the em box,
        // their foot a little under the baseline)
        for (c, slack) in [
            ('𝐇', 0.03),
            ('ב', 0.03),
            ('Ⴌ', 0.03),
            ('ᗩ', 0.04),
            ('급', 0.12),
            ('日', 0.12),
            ('们', 0.14),
        ] {
            if !system.iter().any(|f| f.has(c)) {
                eprintln!("no installed font with {c}: not checked");
                continue;
            }
            let bottom = glyph_bottom(&mut fonts, &c.to_string());
            assert!(
                (bottom - line).abs() <= slack,
                "{c}: bottom at {bottom:.3} em, Latin baseline at {line:.3} em"
            );
        }
    }

    #[test]
    fn baseline_shift_cancels_egui_s_row_centering() {
        use fallback::{RowMetrics, baseline_shift};
        let primary = RowMetrics {
            ascent: 1.069,
            height: 1.362,
        };
        // same proportions: nothing to correct
        assert_eq!(baseline_shift(primary, primary), 0.0);
        // a tall row (Yu Gothic: 0.88 + 0.12 + 0.5 of line gap) is centered
        // 0.26 em too high by egui; the shift brings its baseline back
        let tall = RowMetrics { ascent: 0.88, height: 1.5 };
        let centered = tall.ascent + 0.5 * (primary.height - tall.height);
        assert!((centered + baseline_shift(primary, tall) - primary.ascent).abs() < 1e-6);
        assert!((baseline_shift(primary, tall) - 0.258).abs() < 1e-3);
        // absurd metrics never throw the glyphs out of the row
        let broken = RowMetrics { ascent: 9.0, height: 0.1 };
        assert_eq!(baseline_shift(primary, broken), -0.5);
    }

    /// Selectable labels and text fields with these characters, clicked,
    /// dragged over and edited: egui shapes the right-to-left words of the
    /// system faces with reversed clusters, which its cursor code did not
    /// meet while they were squares.
    #[test]
    fn fallback_text_survives_selection_and_editing() {
        let system = fallback::wait();
        for text in demo_text() {
            let defs = noto(system);
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| select_and_edit(defs, &[text]))).is_err() {
                let faces: Vec<&str> = system.iter().map(|f| f.key.as_str()).collect();
                panic!("egui's cursor code panicked on {text:?} with the system faces {faces:?}");
            }
        }
    }

    /// Known defect of egui 0.36.2 and 0.37.0 (emilk/egui#8576), whatever the fonts: a
    /// run that starts with a right-to-left letter and holds n characters
    /// comes back from shaping as 2n - 1 glyphs (its cluster offsets
    /// decrease, and `layout_shaped_run` then pads the first cluster with
    /// the whole run), while egui's cursor code counts one glyph per
    /// character. The end of the row is past the end of the text, and
    /// `ccursor_previous_word` subtracts below zero from there.
    ///
    /// This test fails once egui is fixed: remove it, and the `#[ignore]` of
    /// the two tests below, put Ctrl+Left and Ctrl+Backspace back in
    /// `select_and_edit`, and update the line of TASKS.md.
    #[test]
    fn egui_still_shapes_surplus_glyphs_in_right_to_left_runs() {
        let mut fonts = egui::epaint::Fonts::new(Default::default(), noto(&[]));
        let mut glyphs = |text: &str| {
            let font = egui::FontId::proportional(14.0);
            let galley = fonts
                .with_pixels_per_point(1.0)
                .layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE);
            galley.rows[0].glyphs.len()
        };
        assert_eq!(glyphs("ب"), 1);
        assert_eq!(glyphs("dب"), 2, "a run that starts left to right");
        assert_eq!(glyphs("بب"), 3, "egui#8576 is fixed: see this test's comment");
        // egui 0.37.0 draws this one right (7 glyphs; 9 with 0.36.2), not the Arabic above
        assert_eq!(glyphs("a אבג b"), 7);
    }

    /// What the cursor code needs: one glyph per character in every row.
    #[test]
    #[ignore = "egui 0.36.2 shapes surplus glyphs in right-to-left runs (emilk/egui#8576)"]
    fn right_to_left_runs_hold_one_glyph_per_character() {
        let system = fallback::wait();
        for defs in [egui::FontDefinitions::default(), noto(&[]), noto(system)] {
            let mut fonts = egui::epaint::Fonts::new(Default::default(), defs);
            for text in ["Aurora بღd", "بب", "ببب", "אב", "a אבג b", "عدد 123 مرحبا", "ღب"] {
                let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
                    text.to_owned(),
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );
                assert_eq!(galley.rows[0].glyphs.len(), text.chars().count(), "{text:?}");
                assert_eq!(galley.end().index.0, text.chars().count(), "{text:?}");
            }
        }
    }

    /// Previous word (Ctrl+Left) and its deletion (Ctrl+Backspace) from the
    /// end of a field holding right-to-left letters. With egui 0.36.2 the
    /// caret is past the text there (11 for 10 characters): the profiles
    /// with overflow checks (`ci`, `debugging`) panic in
    /// `ccursor_previous_word`; the others send the caret to the start of
    /// the text, and Ctrl+Backspace empties the field.
    #[test]
    #[ignore = "egui 0.36.2 shapes surplus glyphs in right-to-left runs (emilk/egui#8576)"]
    fn previous_word_from_the_end_of_right_to_left_text() {
        use egui::{Event, Key, Modifiers};
        let ctx = egui::Context::default();
        let id = egui::Id::unique("edit");
        let mut text = "Aurora بღd".to_owned();
        let frame = |events: Vec<Event>, text: &mut String| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 200.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| ui.add(egui::TextEdit::multiline(text).id(id)).request_focus());
            out.textures_delta.clear();
        };
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let caret = || {
            let range = egui::TextEdit::load_state(&ctx, id).and_then(|state| state.cursor.char_range());
            range.map(|range| (range.secondary.index.0, range.primary.index.0))
        };
        let command = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        frame(vec![], &mut text);
        frame(vec![], &mut text);
        frame(vec![key(Key::End, Modifiers::NONE)], &mut text);
        assert_eq!(caret(), Some((10, 10)), "End");
        frame(vec![key(Key::ArrowLeft, command)], &mut text);
        assert_eq!(caret(), Some((7, 7)), "Ctrl+Left");
        frame(vec![key(Key::End, Modifiers::NONE)], &mut text);
        frame(vec![key(Key::Backspace, command)], &mut text);
        assert_eq!(text, "Aurora ", "Ctrl+Backspace");
        assert_eq!(caret(), Some((7, 7)), "Ctrl+Backspace");
    }

    fn select_and_edit(defs: egui::FontDefinitions, labels: &[&str]) {
        use egui::{Event, Key, Modifiers, PointerButton, Rect, pos2, vec2};
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut edited: Vec<String> = labels.iter().map(|s| (*s).to_owned()).collect();
        // fixed places, whatever the text becomes
        let label_rect = |i: usize| Rect::from_min_size(pos2(10.0, 10.0 + i as f32 * 100.0), vec2(700.0, 40.0));
        let edit_rect = |i: usize| label_rect(i).translate(vec2(0.0, 50.0));
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>, edited: &mut Vec<String>| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(720.0, 100.0 * labels.len() as f32 + 20.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                for (i, (label, text)) in labels.iter().zip(edited.iter_mut()).enumerate() {
                    ui.put(label_rect(i), egui::Label::new(*label).wrap().selectable(true));
                    ui.put(edit_rect(i), egui::TextEdit::multiline(text).id(egui::Id::unique(("edit", i))));
                }
            });
            out.textures_delta.clear();
            let _ = ctx.tessellate(out.shapes, 1.0);
        };
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let command = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        frame(vec![], &mut edited);
        for i in 0..labels.len() {
            let (label, edit) = (label_rect(i), edit_rect(i));
            // label: drags and double clicks (word selection) from every
            // eighth of its width, copied each time
            for step in 0..=8 {
                let from = pos2(label.left() + label.width() * step as f32 / 8.0, label.center().y);
                let to = pos2(label.right() - (from.x - label.left()), from.y);
                frame(vec![Event::PointerMoved(from), button(from, true)], &mut edited);
                frame(vec![Event::PointerMoved(to)], &mut edited);
                frame(vec![button(to, false), Event::Copy], &mut edited);
                frame(vec![button(from, true), button(from, false)], &mut edited);
                frame(vec![button(from, true), button(from, false), Event::Copy], &mut edited);
            }
            // field: caret at each eighth, then moves, selections and edits
            for step in 0..=8 {
                let at = pos2(edit.left() + edit.width() * step as f32 / 8.0, edit.top() + 12.0);
                frame(vec![Event::PointerMoved(at), button(at, true)], &mut edited);
                frame(vec![button(at, false)], &mut edited);
                // No move or deletion to the previous word (Ctrl+Left,
                // Ctrl+Backspace) until egui is fixed: its caret is past the
                // text once the field holds a right-to-left run, and the
                // profiles with overflow checks panic there (see
                // egui_still_shapes_surplus_glyphs_in_right_to_left_runs).
                for (k, m) in [
                    (Key::ArrowLeft, Modifiers::NONE),
                    (Key::ArrowRight, Modifiers::SHIFT),
                    (Key::ArrowRight, command | Modifiers::SHIFT),
                    (Key::Home, Modifiers::SHIFT),
                    (Key::End, Modifiers::NONE),
                    (Key::ArrowLeft, Modifiers::NONE),
                    (Key::Backspace, Modifiers::NONE),
                    (Key::ArrowLeft, Modifiers::NONE),
                    (Key::Delete, Modifiers::NONE),
                ] {
                    frame(vec![key(k, m)], &mut edited);
                }
                frame(vec![Event::Text("ب".into()), Event::Text("ღ".into())], &mut edited);
            }
            // everything selected and replaced: nothing of the text is left
            // out of reach of the cursor
            frame(vec![key(Key::A, command)], &mut edited);
            frame(vec![Event::Text("ღب".into())], &mut edited);
            assert_eq!(edited[i], "ღب", "field of {:?}", labels[i]);
        }
    }
}
