//! Preferences › Appearance › System fonts draws each family's name in its
//! own face, like the font pickers of the operating systems. An Aurora
//! addition: the Legacy Viewer lists no system fonts on Linux.
//!
//! An installed system can have thousands of families, and adding a face to
//! egui makes it rebuild its fonts and shape every window's text again (a
//! frame of about 20 ms each time). So the faces never enter egui: a thread
//! of the list's own draws each name, in its face, into a small picture
//! (white coverage, tinted with the row's text colour when painted), for the
//! rows on screen and a margin around them, as the list scrolls. The frame
//! only uploads the pictures that are ready; a row whose picture is not
//! (yet) there shows its name in the interface font. Once the list is no
//! longer drawn, the pictures are dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use egui::mutex::Mutex;
use resvg::tiny_skia;

use super::SystemFont;

/// Rows drawn beyond each end of the visible ones.
const MARGIN: usize = 64;
/// Pictures kept at most: past them, those far from the visible rows are
/// dropped, so a long scroll does not keep growing the textures.
const CAP: usize = 512;

const STATE_ID: &str = "aurora-font-previews";

/// A name drawn in its own face, ready to paint.
#[derive(Clone)]
pub struct Drawn {
    texture: egui::TextureHandle,
    /// The picture's size, in points.
    pub size: egui::Vec2,
    /// How far below a row's centre the picture's centre goes, in points
    /// (the face's optical centre, as `fonts::optical_center_offset`
    /// shifts the interface fonts).
    pub y_offset: f32,
}

impl Drawn {
    pub fn texture(&self) -> egui::TextureId {
        self.texture.id()
    }
}

enum Entry {
    /// Asked of the drawing thread.
    Pending,
    /// Drawn, or `None` when the face cannot draw its own name or cannot be
    /// read.
    Done(Option<Drawn>),
}

/// A picture the drawing thread made.
struct Picture {
    /// [`State::generation`] when it was asked for.
    generation: u64,
    path: PathBuf,
    image: Option<(egui::ColorImage, f32)>,
}

/// How the names are drawn: a change draws them all again.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    /// Text size, in pixels.
    px: f32,
    /// Widest picture, in pixels (a wider name is cut, as a row truncates).
    max_w: f32,
    pixels_per_point: f32,
}

struct State {
    entries: HashMap<PathBuf, Entry>,
    look: Option<Look>,
    /// Bumped when the entries are dropped: pictures asked for before are
    /// thrown away when they arrive.
    generation: u64,
    sender: mpsc::Sender<Picture>,
    receiver: mpsc::Receiver<Picture>,
    /// Frame the list was last drawn.
    drawn: u64,
}

impl Default for State {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            look: None,
            generation: 0,
            sender,
            receiver,
            drawn: 0,
        }
    }
}

impl State {
    fn clear(&mut self) {
        self.entries.clear();
        self.generation += 1;
    }
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
    look: Look,
    /// Indices (into the list as shown) of the first and last visible rows.
    visible: Option<(usize, usize)>,
}

impl Previews {
    /// Start a frame of the open list, whose names are `size` points high
    /// (the interface's scale included) in rows that fit `max_width`
    /// points of text: takes the pictures drawn since the last frame.
    pub fn begin(ctx: &egui::Context, size: f32, max_width: f32) -> Self {
        let pixels_per_point = ctx.pixels_per_point();
        let look = Look {
            px: (size * pixels_per_point).max(1.0),
            max_w: (max_width * pixels_per_point).max(1.0),
            pixels_per_point,
        };
        let state = state(ctx);
        {
            let mut s = state.lock();
            s.drawn = ctx.cumulative_frame_nr();
            if s.look != Some(look) {
                s.clear();
                s.look = Some(look);
            }
            let ready: Vec<Picture> = s.receiver.try_iter().collect();
            for picture in ready {
                if picture.generation != s.generation {
                    continue;
                }
                let drawn = picture.image.map(|(image, y_offset)| {
                    let size = egui::vec2(image.size[0] as f32, image.size[1] as f32) / pixels_per_point;
                    let name = format!("font-preview:{}", picture.path.display());
                    Drawn {
                        texture: ctx.load_texture(name, image, egui::TextureOptions::LINEAR),
                        size,
                        y_offset,
                    }
                });
                s.entries.insert(picture.path, Entry::Done(drawn));
            }
        }
        Self {
            state,
            look,
            visible: None,
        }
    }

    /// `font`'s name drawn in its own face, once ready and able to.
    pub fn drawn(&self, font: &SystemFont) -> Option<Drawn> {
        match self.state.lock().entries.get(&font.path) {
            Some(Entry::Done(drawn)) => drawn.clone(),
            _ => None,
        }
    }

    /// Row `index` of the list as shown is on screen.
    pub fn visible(&mut self, index: usize) {
        self.visible = Some(match self.visible {
            Some((first, last)) => (first.min(index), last.max(index)),
            None => (index, index),
        });
    }

    /// Ask the drawing thread for the rows around the visible ones that
    /// are neither drawn nor asked for. `shown` is the list as drawn this
    /// frame.
    pub fn finish(self, ctx: &egui::Context, shown: &[&SystemFont]) {
        let Some((first, last)) = self.visible else {
            return;
        };
        let mut s = self.state.lock();
        if shown[first..=last].iter().all(|f| s.entries.contains_key(&f.path)) {
            return;
        }
        let window = &shown[first.saturating_sub(MARGIN)..(last + MARGIN + 1).min(shown.len())];
        let wanted: Vec<(PathBuf, String)> = window
            .iter()
            .filter(|f| !s.entries.contains_key(&f.path))
            .map(|f| (f.path.clone(), f.name.clone()))
            .collect();
        if s.entries.len() + wanted.len() > CAP {
            // the pictures far from the rows on screen go; those still
            // being drawn stay, their answer is on its way
            let near: std::collections::HashSet<&Path> = window.iter().map(|f| f.path.as_path()).collect();
            s.entries
                .retain(|path, entry| matches!(entry, Entry::Pending) || near.contains(path.as_path()));
        }
        for (path, _) in &wanted {
            s.entries.insert(path.clone(), Entry::Pending);
        }
        let (look, generation, sender, ctx) = (self.look, s.generation, s.sender.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("aurora-font-previews".into()).spawn(move || {
            for (path, name) in wanted {
                let image = picture(&path, &name, look);
                if sender.send(Picture { generation, path, image }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("font previews: no drawing thread ({e})");
        }
    }
}

/// Drop the pictures once the list has not been drawn for a frame (closed,
/// or its page or window left). Call once per frame, before the interface
/// is drawn.
pub fn release_unused(ctx: &egui::Context) {
    let state = state(ctx);
    let mut s = state.lock();
    if !s.entries.is_empty() && s.drawn + 1 < ctx.cumulative_frame_nr() {
        s.clear();
        // the pictures already sent are dropped with the queue
        while s.receiver.try_recv().is_ok() {}
    }
}

/// `name` drawn in the face of `path`, and its offset below a row's centre
/// in points, when the face has a glyph for every character of it.
fn picture(path: &Path, name: &str, look: Look) -> Option<(egui::ColorImage, f32)> {
    let file = std::fs::File::open(path)
        .inspect_err(|e| log::debug!("font preview {}: {e}", path.display()))
        .ok()?;
    // SAFETY: the font file is opened read-only and never written by the
    // viewer, and mapped only while its name is drawn. Windows refuses to
    // truncate or delete a mapped file; elsewhere fonts are replaced by
    // renaming, which leaves this mapping whole. A file truncated in place
    // during the drawing would fault on a page read, as with the fallback
    // fonts.
    let map = unsafe { memmap2::Mmap::map(&file) }
        .inspect_err(|e| log::debug!("font preview {}: {e}", path.display()))
        .ok()?;
    let face = ttf_parser::Face::parse(&map, 0).ok()?;
    if !draws(&face, name) {
        return None;
    }
    let image = rasterize(&face, name, look.px, look.max_w)?;
    let y_offset = super::optical_center_offset(&map) * look.px / look.pixels_per_point;
    Some((image, y_offset))
}

/// `name` at `px` pixels per em, as egui sizes a font (its ascender on the
/// first row of pixels, a row as high as ascender, descender and line gap),
/// cut before a glyph that would pass `max_w` pixels: white, its coverage in
/// alpha, premultiplied. `None` when nothing has ink (spaces only).
fn rasterize(face: &ttf_parser::Face<'_>, name: &str, px: f32, max_w: f32) -> Option<egui::ColorImage> {
    let units = f32::from(face.units_per_em());
    if units <= 0.0 {
        return None;
    }
    let k = px / units;
    let ascent = f32::from(face.ascender()) * k;
    let descent = f32::from(face.descender()) * k;
    let gap = f32::from(face.line_gap()).max(0.0) * k;
    // a broken face's metrics cannot make a huge picture
    let height = (ascent - descent + gap).clamp(1.0, px * 4.0).ceil() as u32;
    let kern = face
        .tables()
        .kern
        .and_then(|kern| kern.subtables.into_iter().find(|s| s.horizontal && !s.variable));
    let mut pen = Pen {
        path: tiny_skia::PathBuilder::new(),
        k,
        x: 0.0,
        baseline: ascent.round(),
    };
    let mut previous = None;
    for c in name.chars() {
        let glyph = face.glyph_index(c)?;
        if let (Some(previous), Some(kern)) = (previous, kern.as_ref()) {
            pen.x += f32::from(kern.glyphs_kerning(previous, glyph).unwrap_or(0)) * k;
        }
        let advance = f32::from(face.glyph_hor_advance(glyph).unwrap_or(0)) * k;
        if pen.x + advance > max_w {
            break;
        }
        face.outline_glyph(glyph, &mut pen);
        pen.x += advance;
        previous = Some(glyph);
    }
    let width = (pen.x.ceil() as u32).clamp(1, max_w.ceil() as u32);
    let path = pen.path.finish()?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    Some(egui::ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}

/// A glyph's outline, from font units (y up) to the picture's pixels.
struct Pen {
    path: tiny_skia::PathBuilder,
    /// Pixels per font unit.
    k: f32,
    /// The glyph's origin.
    x: f32,
    baseline: f32,
}

impl Pen {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.k, self.baseline - y * self.k)
    }
}

impl ttf_parser::OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.path.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.path.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let ((x1, y1), (x, y)) = (self.at(x1, y1), self.at(x, y));
        self.path.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let ((x1, y1), (x2, y2), (x, y)) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        self.path.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.path.close();
    }
}

/// Does `face` draw `name` as letters? It needs a glyph for every character,
/// and symbol fonts are refused: those with a Windows Symbol `cmap`
/// (Wingdings) and those whose Latin letters lead to glyphs named for
/// something else (dingbats at `A` named `a10`, a Greek `psi` at `y`). Such
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

/// Is a glyph named `name` the letter `c`: `A`, `a.ss01`, its code point
/// (`uni0041`, `u0041`), or a name that says nothing (`glyph36`, `gid36`)?
/// Only the first letter is compared: `alpha` passes for `a`, which the
/// other characters of a name and the Symbol `cmap` test make harmless.
fn letter_glyph(name: &str, c: char) -> bool {
    let code = format!("{:04X}", c as u32);
    let numbered = |prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    if numbered("glyph") || numbered("gid") {
        return true;
    }
    name.chars().next().is_some_and(|first| first.eq_ignore_ascii_case(&c)) && !name.starts_with("uni")
        || name
            .strip_prefix("uni")
            .or_else(|| name.strip_prefix('u'))
            .is_some_and(|hex| hex.starts_with(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Embedded fonts written as files, as system fonts are read, in a
    /// folder of the test's own: the tests run in parallel, and a file
    /// another test has mapped cannot be rewritten (Windows refuses it,
    /// Linux may kill the mapping test with SIGBUS).
    fn font_files(test: &str) -> Vec<SystemFont> {
        let dir = std::env::temp_dir().join(format!("aurora-font-previews-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temporary folder");
        [
            ("Inter", super::super::INTER_TTF),
            ("Noto Sans", super::super::NOTO_TTF),
            ("Roboto", super::super::ROBOTO_TTF),
        ]
        .into_iter()
        .map(|(name, data)| {
            let path = dir.join(format!("{name}.ttf"));
            std::fs::write(&path, data).expect("font file");
            SystemFont {
                name: name.to_owned(),
                path,
            }
        })
        .collect()
    }

    /// One frame; the textures it uploads are dropped (the tests have no
    /// painter), as egui's debug assertions ask.
    fn run(ctx: &egui::Context, ui: impl FnMut(&mut egui::Ui)) {
        ctx.run_ui(egui::RawInput::default(), ui).textures_delta.clear();
    }

    /// The list closed: its textures are freed within the frames, before
    /// the context goes (egui asserts that no texture change is left).
    fn close(ctx: &egui::Context) {
        for _ in 0..2 {
            run(ctx, |ui| release_unused(ui.ctx()));
        }
        assert!(state(ctx).lock().entries.is_empty());
    }

    /// One frame of the open list showing `shown`, all visible.
    fn frame(ctx: &egui::Context, shown: &[&SystemFont]) {
        run(ctx, |ui| {
            let mut previews = Previews::begin(ui.ctx(), 13.0, 300.0);
            for i in 0..shown.len() {
                previews.visible(i);
            }
            previews.finish(ui.ctx(), shown);
        });
    }

    fn drawn(ctx: &egui::Context, font: &SystemFont) -> bool {
        matches!(state(ctx).lock().entries.get(&font.path), Some(Entry::Done(Some(_))))
    }

    /// Frames until every font of `shown` is drawn (the thread's pictures
    /// arrive between them), at most a few seconds.
    fn until_drawn(ctx: &egui::Context, shown: &[&SystemFont]) -> bool {
        for _ in 0..500 {
            frame(ctx, shown);
            if shown.iter().all(|f| drawn(ctx, f)) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn the_names_are_drawn_without_adding_a_face_to_egui() {
        let fonts = font_files("drawn");
        let shown: Vec<&SystemFont> = fonts.iter().collect();
        let ctx = egui::Context::default();
        super::super::install(&ctx, super::super::definitions_for(super::super::DEFAULT, &[]));
        let families = |ctx: &egui::Context| ctx.fonts(|f| f.definitions().families.len());
        // egui has fonts once a frame ran
        run(&ctx, |_| {});
        let before = families(&ctx);
        assert!(until_drawn(&ctx, &shown), "the three names were drawn");
        // egui's fonts are those of the interface alone: no rebuild, no
        // window's text shaped again
        assert_eq!(families(&ctx), before);
        close(&ctx);
    }

    #[test]
    fn the_pictures_go_once_the_list_is_no_longer_drawn() {
        let fonts = font_files("release");
        let shown: Vec<&SystemFont> = fonts.iter().collect();
        let ctx = egui::Context::default();
        assert!(until_drawn(&ctx, &shown));
        close(&ctx);
    }

    #[test]
    fn a_name_is_drawn_at_the_size_egui_gives_its_face() {
        let face = ttf_parser::Face::parse(super::super::INTER_TTF, 0).expect("Inter parses");
        let image = rasterize(&face, "Inter", 20.0, 1000.0).expect("Inter draws its name");
        let k = 20.0 / f32::from(face.units_per_em());
        let height = (f32::from(face.ascender()) - f32::from(face.descender()) + f32::from(face.line_gap())) * k;
        assert_eq!(image.size[1], height.ceil() as usize);
        assert!(image.size[0] > 20 && image.size[0] < 100, "{:?}", image.size);
        assert!(image.pixels.iter().any(|p| p.a() > 200), "the letters have ink");
        // a narrow row cuts the name before the glyph that would pass it
        let cut = rasterize(&face, "Inter", 20.0, 25.0).expect("its first letters");
        assert!(cut.size[0] <= 25);
        // nothing to draw: spaces only
        assert!(rasterize(&face, "   ", 20.0, 1000.0).is_none());
    }

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
        assert!(letter_glyph("glyph36", 'H'));
        assert!(letter_glyph("gid36", 'H'));
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
