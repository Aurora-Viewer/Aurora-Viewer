//! Preferences › Appearance › System fonts draws each family's name in its
//! own face, like the font pickers of the operating systems. An Aurora
//! addition: the Legacy Viewer lists no system fonts on Linux.
//!
//! egui draws only fonts it has loaded, and an installed system can have
//! thousands of families, so only the rows on screen and a margin around
//! them are loaded, memory-mapped, as the list scrolls. Every addition makes
//! egui rebuild its fonts (atlas and layouts), so the faces of a frame go in
//! one batch and the margin makes a batch last a few dozen rows of
//! scrolling. Once the list is no longer drawn, the interface fonts are put
//! back alone, which frees the faces and their glyphs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::mutex::Mutex;

use super::SystemFont;

/// Rows loaded beyond each end of the visible ones.
const MARGIN: usize = 64;
/// Faces kept loaded at most: past them, the interface fonts are put back
/// before the next batch, so a long scroll does not keep growing egui's
/// fonts.
const CAP: usize = 512;

const STATE_ID: &str = "aurora-font-previews";

#[derive(Default)]
struct State {
    /// Per file: its family once added to egui, `None` when it cannot draw
    /// its own name or cannot be read.
    faces: HashMap<PathBuf, Option<egui::FontFamily>>,
    /// Faces added to egui since the interface fonts were last installed.
    added: usize,
    /// Frame the list was last drawn.
    drawn: u64,
}

fn state(ctx: &egui::Context) -> Arc<Mutex<State>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<State>>>(egui::Id::unique(STATE_ID))
            .clone()
    })
}

/// The previews of one frame of the open list.
pub struct Previews {
    state: Arc<Mutex<State>>,
    /// Indices (into the list as shown) of the first and last visible rows.
    visible: Option<(usize, usize)>,
}

impl Previews {
    /// Start a frame of the open list. Faces dropped by a font change since
    /// the last frame (a new interface font, the text size) are forgotten,
    /// to be loaded again.
    pub fn begin(ctx: &egui::Context) -> Self {
        let state = state(ctx);
        {
            let mut s = state.lock();
            s.drawn = ctx.cumulative_frame_nr();
            let loaded = s.faces.values().flatten().next().cloned();
            if let Some(family) = loaded
                && !ctx.fonts(|f| f.definitions().families.contains_key(&family))
            {
                s.faces.clear();
                s.added = 0;
            }
        }
        Self { state, visible: None }
    }

    /// The family drawing `font`'s name, once loaded and able to.
    pub fn family(&self, font: &SystemFont) -> Option<egui::FontFamily> {
        self.state.lock().faces.get(&font.path).cloned().flatten()
    }

    /// Row `index` of the list as shown is on screen.
    pub fn visible(&mut self, index: usize) {
        self.visible = Some(match self.visible {
            Some((first, last)) => (first.min(index), last.max(index)),
            None => (index, index),
        });
    }

    /// Load, in one batch, the rows around the visible ones when one of
    /// those is missing. `shown` is the list as drawn this frame.
    pub fn finish(self, ctx: &egui::Context, shown: &[&SystemFont]) {
        let Some((first, last)) = self.visible else {
            return;
        };
        let mut s = self.state.lock();
        if shown[first..=last].iter().all(|f| s.faces.contains_key(&f.path)) {
            return;
        }
        let window = &shown[first.saturating_sub(MARGIN)..(last + MARGIN + 1).min(shown.len())];
        let mut wanted: Vec<&SystemFont> = window.iter().copied().filter(|f| !s.faces.contains_key(&f.path)).collect();
        if s.added + wanted.len() > CAP {
            super::reinstall(ctx);
            s.faces.clear();
            s.added = 0;
            wanted = window.to_vec();
        }
        let scale = super::scale(ctx);
        for font in wanted {
            let family = face(&font.path, &font.name, scale).map(|(name, data)| {
                let family = egui::FontFamily::Name(name.as_str().into());
                ctx.add_font(egui::FontInsert::new(
                    &name,
                    data,
                    vec![egui::InsertFontFamily {
                        family: family.clone(),
                        priority: egui::FontPriority::Highest,
                    }],
                ));
                s.added += 1;
                family
            });
            s.faces.insert(font.path.clone(), family);
        }
    }
}

/// Put the interface fonts back alone once the list has not been drawn
/// for a frame (closed, or its page or window left). Call once per frame,
/// before the interface is drawn.
pub fn release_unused(ctx: &egui::Context) {
    let state = state(ctx);
    let mut s = state.lock();
    if !s.faces.is_empty() && s.drawn + 1 < ctx.cumulative_frame_nr() {
        if s.added > 0 {
            super::reinstall(ctx);
        }
        s.faces.clear();
        s.added = 0;
    }
}

/// The face of `path` for egui under its own family name, at the
/// interface's scale, when it has a glyph for every character of `name`.
fn face(path: &Path, name: &str, scale: f32) -> Option<(String, egui::FontData)> {
    let file = std::fs::File::open(path)
        .inspect_err(|e| log::debug!("font preview {}: {e}", path.display()))
        .ok()?;
    // SAFETY: the font file is opened read-only and never written by the
    // viewer. Windows refuses to truncate or delete a mapped file; elsewhere
    // fonts are replaced by renaming, which leaves this mapping whole.
    let map = unsafe { memmap2::Mmap::map(&file) }
        .inspect_err(|e| log::debug!("font preview {}: {e}", path.display()))
        .ok()?;
    let face = ttf_parser::Face::parse(&map, 0).ok()?;
    if !draws(&face, name) {
        return None;
    }
    let mut data = egui::FontData::from_blob(Arc::new(map), 0);
    data.tweak.y_offset_factor = super::optical_center_offset(data.bytes());
    data.tweak.scale *= scale;
    Some((format!("aurora-preview:{}", path.display()), data))
}

/// Does `face` draw `name` as letters? It needs a glyph for every character,
/// and symbol fonts are refused: those with a Windows Symbol `cmap`
/// (Wingdings) and those whose Latin letters lead to glyphs named for
/// something else (dingbats at `A` named `a10`, a Greek `Alpha`). Such
/// names, and fonts of another script than their name's, stay in the
/// interface font.
fn draws(face: &ttf_parser::Face<'_>, name: &str) -> bool {
    let symbol = face.tables().cmap.is_some_and(|cmap| {
        cmap.subtables
            .into_iter()
            .any(|t| t.platform_id == ttf_parser::PlatformId::Windows && t.encoding_id == 0)
    });
    !symbol
        && name.chars().all(|c| {
            face.glyph_index(c)
                .is_some_and(|g| g.0 != 0 && (!c.is_ascii_alphabetic() || face.glyph_name(g).is_none_or(|n| letter_glyph(n, c))))
        })
}

/// Is a glyph named `name` the letter `c`: `A`, `a.ss01`, or its code
/// point (`uni0041`, `u0041`)?
fn letter_glyph(name: &str, c: char) -> bool {
    let code = format!("{:04X}", c as u32);
    name.chars().next().is_some_and(|first| first.eq_ignore_ascii_case(&c)) && !name.starts_with("uni")
        || name
            .strip_prefix("uni")
            .or_else(|| name.strip_prefix('u'))
            .is_some_and(|hex| hex.starts_with(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_draws_its_name_only_with_every_glyph() {
        let face = ttf_parser::Face::parse(super::super::INTER_TTF, 0).expect("Inter parses");
        assert!(draws(&face, "Inter Display"));
        // Inter has no Han glyphs: such a name stays in the interface font
        assert!(!draws(&face, "思源黑体"));
    }

    #[test]
    fn letters_are_named_for_themselves_or_their_code_point() {
        assert!(letter_glyph("A", 'A'));
        assert!(letter_glyph("a.ss01", 'a'));
        assert!(letter_glyph("uni0048", 'H'));
        assert!(letter_glyph("u0048", 'H'));
        assert!(!letter_glyph("a10", 'D'));
        assert!(!letter_glyph("psi", 'y'));
        // `uni…` of another code point is not the letter `u`
        assert!(!letter_glyph("uni0041", 'u'));
    }

    #[test]
    fn symbol_fonts_do_not_draw_their_name() {
        // dingbats at Latin code points (the URW D050000L of Linux systems)
        let Ok(data) = std::fs::read("/usr/share/fonts/gsfonts/D050000L.otf") else {
            return;
        };
        let face = ttf_parser::Face::parse(&data, 0).expect("D050000L parses");
        assert!(!draws(&face, "D050000L"));
    }
}
