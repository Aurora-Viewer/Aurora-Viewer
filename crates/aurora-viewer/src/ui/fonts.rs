//! UI fonts: Noto Sans (default), Inter and Roboto are embedded (SIL OFL,
//! see assets/fonts/LICENSE-*.txt); installed system fonts are listed too.
//! Characters none of them has come from the system's fonts (`fallback`).

use std::path::PathBuf;
use std::sync::Arc;

pub mod fallback;

pub const INTER: &str = "Inter";
pub const NOTO: &str = "Noto Sans";
pub const ROBOTO: &str = "Roboto";

static INTER_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static INTER_SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
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
    let system = fallback::ready();
    let mut defs = definitions(data, system.unwrap_or_default());
    // the embedded sans-serifs are close enough to share Inter's semi-bold
    // face; a system font keeps its own design (and its `wght` axis when it
    // is a variable font, see `semibold_format`)
    if matches!(choice, INTER | NOTO | ROBOTO | "") {
        let mut face = egui::FontData::from_static(INTER_SEMIBOLD_TTF);
        face.tweak.y_offset_factor = optical_center_offset(&face.font);
        defs.font_data.insert(SEMIBOLD.into(), Arc::new(face));
        if let Some(f) = defs.families.get_mut(&semibold_family()) {
            f.insert(0, SEMIBOLD.into());
        }
    }
    ctx.set_fonts(defs);
    system.is_some()
}

/// Name of the semi-bold family and of its face.
const SEMIBOLD: &str = "aurora-semibold";

fn semibold_family() -> egui::FontFamily {
    egui::FontFamily::Name(SEMIBOLD.into())
}

/// Family of the semi-bold texts (the name on an avatar's name plate):
/// Inter SemiBold with the embedded fonts, else the interface font. A
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

/// `primary` first for proportional text and after egui's own fonts for
/// monospace, then the system faces, last in both families: what the
/// embedded fonts draw keeps its look, only the squares are replaced.
fn definitions(mut primary: egui::FontData, system: &[fallback::SystemFace]) -> egui::FontDefinitions {
    primary.tweak.y_offset_factor = optical_center_offset(&primary.font);
    let shift = primary.tweak.y_offset_factor;
    let row = ttf_parser::Face::parse(&primary.font, 0)
        .ok()
        .and_then(|face| fallback::RowMetrics::of(&face));
    let mut defs = egui::FontDefinitions::default();
    defs.font_data.insert("aurora-main".into(), Arc::new(primary));
    if let Some(f) = defs.families.get_mut(&egui::FontFamily::Proportional) {
        f.insert(0, "aurora-main".into());
    }
    if let Some(f) = defs.families.get_mut(&egui::FontFamily::Monospace) {
        f.push("aurora-main".into());
    }
    // the semi-bold family starts as the proportional one: `apply` puts
    // its own face first when it has one
    let proportional = defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    defs.families.insert(semibold_family(), proportional);
    for face in system {
        defs.font_data.insert(face.key.clone(), Arc::new(face.font_data(row, shift)));
        for family in defs.families.values_mut() {
            family.push(face.key.clone());
        }
    }
    defs
}

/// Is `c` drawn by the fonts shipped with the viewer, whatever the system?
/// The default proportional family: Noto Sans, then egui's own fallbacks.
#[cfg(test)]
fn embedded_has(c: char) -> bool {
    let has = |font: &[u8]| ttf_parser::Face::parse(font, 0).is_ok_and(|face| face.glyph_index(c).is_some());
    let defs = egui::FontDefinitions::default();
    let fallbacks = defs.families.get(&egui::FontFamily::Proportional).into_iter().flatten();
    has(NOTO_TTF) || fallbacks.filter_map(|name| defs.font_data.get(name)).any(|d| has(&d.font))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noto(system: &[fallback::SystemFace]) -> egui::FontDefinitions {
        definitions(egui::FontData::from_static(NOTO_TTF), system)
    }

    /// Which characters egui draws with these definitions, in a font of the
    /// family: its own faces and character maps, as in the UI. (Not
    /// `has_glyph`, which answers no for every character of the face that
    /// holds egui's replacement square: the emoji.)
    fn drawable(defs: egui::FontDefinitions, family: egui::FontFamily, text: &str) -> Vec<(char, bool)> {
        let mut fonts = egui::epaint::Fonts::new(Default::default(), defs);
        let mut font = fonts.fonts.font(&family);
        let known = font.characters();
        text.chars().map(|c| (c, known.contains_key(&c))).collect()
    }

    fn demo_text() -> Vec<&'static str> {
        let samples = crate::demo::glyphs::SAMPLES.iter().flat_map(|s| [s.title, s.display_name, s.chat]);
        samples.chain([crate::demo::glyphs::HOVER_TEXT]).collect()
    }

    #[test]
    fn embedded_fonts_cover_latin_greek_cyrillic_only() {
        for c in "Aé ΩЯ €ℓ".chars() {
            assert!(embedded_has(c), "{c:?}");
        }
        // the reported square (#127: a group title wrapped in U+10E6), and
        // the other usual ones of display names
        for c in "ღ𝓐𝐀中あ한بא░→".chars() {
            assert!(
                !embedded_has(c),
                "{c:?} is embedded: the system fallback is no longer needed for it"
            );
        }
    }

    #[test]
    fn system_faces_come_after_the_embedded_fonts_in_both_families() {
        let system = fallback::wait();
        let (without, with) = (noto(&[]), noto(system));
        for (family, fonts) in &with.families {
            // the embedded part of the family is what it was without them
            let embedded = &without.families[family];
            assert_eq!(&fonts[..embedded.len()], &embedded[..], "{family}");
            assert!(
                fonts[embedded.len()..].iter().all(|f| f.starts_with("system:")),
                "{family}: {fonts:?}"
            );
            assert_eq!(fonts.len() - embedded.len(), system.len(), "{family}");
        }
        assert_eq!(with.families[&egui::FontFamily::Proportional][0], "aurora-main");
        // egui panics on a family that names an unknown font
        for name in with.families.values().flatten() {
            assert!(with.font_data.contains_key(name), "{name}");
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
            let missing: String = drawable(noto(system), family.clone(), &text)
                .into_iter()
                .filter(|(c, ok)| !ok && !c.is_whitespace())
                .map(|(c, _)| c)
                .collect();
            if complete {
                assert!(missing.is_empty(), "{family}: no font for {missing:?}");
            } else if !missing.is_empty() {
                eprintln!("{family}: system fonts incomplete, squares for {missing:?}");
            }
        }
    }

    /// The same with each built-in interface font as the primary one: the
    /// system faces must not depend on the font chosen in Preferences.
    #[test]
    fn demo_characters_are_drawn_whatever_the_builtin_font() {
        let system = fallback::wait();
        if !fallback::CANDIDATES.iter().all(|c| system.iter().any(|f| f.has(c.probe))) {
            eprintln!("system fonts incomplete: not checked");
            return;
        }
        let text = demo_text().join(" ");
        for (name, ttf) in [(INTER, INTER_TTF), (NOTO, NOTO_TTF), (ROBOTO, ROBOTO_TTF)] {
            let defs = definitions(egui::FontData::from_static(ttf), system);
            let missing: String = drawable(defs, egui::FontFamily::Proportional, &text)
                .into_iter()
                .filter(|(c, ok)| !ok && !c.is_whitespace())
                .map(|(c, _)| c)
                .collect();
            assert!(missing.is_empty(), "{name}: no font for {missing:?}");
        }
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
            if std::panic::catch_unwind(|| select_and_edit(defs, &[text])).is_err() {
                let faces: Vec<&str> = system.iter().map(|f| f.key.as_str()).collect();
                panic!("egui's cursor code panicked on {text:?} with the system faces {faces:?}");
            }
        }
    }

    /// Known defect of egui 0.36.2 (emilk/egui#8576), whatever the fonts: a
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
        assert_eq!(glyphs("a אבג b"), 9, "egui#8576 is fixed: see this test's comment");
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
        let id = egui::Id::new("edit");
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
                    ui.put(edit_rect(i), egui::TextEdit::multiline(text).id(egui::Id::new(("edit", i))));
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
