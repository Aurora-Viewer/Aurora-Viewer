//! Color emoji from the Noto 3D emoji font (sbix PNG strikes, Apache 2.0 /
//! OFL — see NOTICE.md). The font is memory-mapped (it is far
//! too large to embed) and glyphs are decoded lazily into egui textures.
//! Provides the emoji picker and inline emoji rendering for chat text.

use crate::theme::Palette;
use std::collections::HashMap;
use std::path::PathBuf;

/// Picker tabs: (label, Phosphor icon, Unicode group).
const CATEGORIES: &[(&str, &str, emojis::Group)] = &[
    ("Smileys", "smiley", emojis::Group::SmileysAndEmotion),
    ("Personnes", "hand-waving", emojis::Group::PeopleAndBody),
    ("Animaux et nature", "drop", emojis::Group::AnimalsAndNature),
    ("Nourriture", "gift", emojis::Group::FoodAndDrink),
    ("Voyages", "airplane-tilt", emojis::Group::TravelAndPlaces),
    ("Activités", "person-simple-run", emojis::Group::Activities),
    ("Objets", "cube", emojis::Group::Objects),
    ("Symboles", "star", emojis::Group::Symbols),
];

/// The single code point an emoji is drawn with (variation selector ignored);
/// multi code point sequences (ZWJ, flags, skin tones) are not drawable yet.
fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars().filter(|c| *c != '\u{FE0F}');
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

/// Replace `:shortcode:` (GitHub / gemoji names, e.g. `:hearts:`, `:smile:`) by the emoji.
pub fn expand_shortcodes(text: &str) -> String {
    if !text.contains(':') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(':') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find(':') {
            Some(end)
                if end > 0
                    && after[..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-') =>
            {
                match emojis::get_by_shortcode(&after[..end]) {
                    Some(e) => {
                        out.push_str(e.as_str());
                        rest = &after[end + 1..];
                    }
                    None => {
                        out.push(':');
                        rest = after;
                    }
                }
            }
            _ => {
                out.push(':');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

enum Glyph {
    Ready(egui::TextureHandle),
    Missing,
}

pub struct Emoji {
    face: Option<ttf_parser::Face<'static>>,
    glyphs: HashMap<char, Glyph>,
    /// Per-category list of code points that exist in the font.
    lists: Vec<Vec<char>>,
    pub tab: usize,
    /// Picker search text.
    pub query: String,
}

fn font_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("AURORA_EMOJI_FONT") {
        v.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        v.push(dir.join("assets").join("emoji").join("Noto-3D-128.ttf"));
        v.push(dir.join("assets").join("emoji").join("Noto-3D-watch.ttf"));
    }
    // development tree: <repo>/assets/emoji (scripts/fetch-assets.ps1)
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/emoji");
    v.push(dev.join("Noto-3D-128.ttf"));
    v.push(dev.join("Noto-3D-watch.ttf"));
    v
}

impl Emoji {
    pub fn load() -> Emoji {
        let mut face = None;
        for path in font_candidates() {
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            // SAFETY: the font file is opened read-only and never modified
            // while mapped; the mapping is leaked so the face can be 'static.
            let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
                continue;
            };
            let map: &'static memmap2::Mmap = Box::leak(Box::new(map));
            let bytes: &'static [u8] = &map[..];
            match ttf_parser::Face::parse(bytes, 0) {
                Ok(f) => {
                    log::info!("emoji font: {}", path.display());
                    face = Some(f);
                    break;
                }
                Err(e) => log::warn!("emoji font {}: {e}", path.display()),
            }
        }
        if face.is_none() {
            log::warn!("no emoji font found (Noto-3D); emoji use the monochrome fallback");
        }
        let lists = CATEGORIES
            .iter()
            .map(|(_, _, group)| {
                group
                    .emojis()
                    .filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
                    .filter_map(|e| single_char(e.as_str()))
                    .filter(|c| face.as_ref().is_some_and(|f| f.glyph_index(*c).is_some()))
                    .collect()
            })
            .collect();
        Emoji {
            face,
            glyphs: HashMap::new(),
            lists,
            tab: 0,
            query: String::new(),
        }
    }

    pub fn available(&self) -> bool {
        self.face.is_some()
    }

    /// Is this character drawn as a color emoji?
    pub fn has(&self, c: char) -> bool {
        (c as u32) >= 0x2190 && self.face.as_ref().is_some_and(|f| f.glyph_index(c).is_some())
    }

    /// Texture of an emoji (decoded on first use).
    pub fn texture(&mut self, ctx: &egui::Context, c: char) -> Option<egui::TextureHandle> {
        if let Some(g) = self.glyphs.get(&c) {
            return match g {
                Glyph::Ready(t) => Some(t.clone()),
                Glyph::Missing => None,
            };
        }
        let face = self.face.as_ref()?;
        let decoded = face
            .glyph_index(c)
            .and_then(|gid| face.glyph_raster_image(gid, 128))
            .filter(|img| img.format == ttf_parser::RasterImageFormat::PNG)
            .and_then(|img| image::load_from_memory(img.data).ok())
            .map(|img| {
                let img = img.resize(64, 64, image::imageops::FilterType::Triangle).to_rgba8();
                let (w, h) = img.dimensions();
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw())
            });
        let g = match decoded {
            Some(ci) => Glyph::Ready(ctx.load_texture(format!("emoji-{:x}", c as u32), ci, egui::TextureOptions::LINEAR)),
            None => Glyph::Missing,
        };
        let out = match &g {
            Glyph::Ready(t) => Some(t.clone()),
            Glyph::Missing => None,
        };
        self.glyphs.insert(c, g);
        out
    }

    /// Text with color emoji inline (wraps like a label). The text stays
    /// selectable for copying, like Firestorm's read-only chat editors.
    pub fn rich_text(&mut self, ui: &mut egui::Ui, text: &str, size: f32, color: egui::Color32, italics: bool) {
        if !self.available() || !text.chars().any(|c| self.has(c)) {
            let mut rt = egui::RichText::new(text).size(size).color(color);
            if italics {
                rt = rt.italics();
            }
            ui.add(egui::Label::new(rt).wrap().selectable(true));
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
            self.inline(ui, text, size, color, italics);
        });
    }

    /// Text with color emoji added to the current (wrapping) layout.
    pub fn inline(&mut self, ui: &mut egui::Ui, text: &str, size: f32, color: egui::Color32, italics: bool) {
        let mut run = String::new();
        let flush = |ui: &mut egui::Ui, run: &mut String| {
            if !run.is_empty() {
                let mut rt = egui::RichText::new(std::mem::take(run)).size(size).color(color);
                if italics {
                    rt = rt.italics();
                }
                ui.add(egui::Label::new(rt).wrap().selectable(true));
            }
        };
        for c in text.chars() {
            // variation selector / zero-width joiner: part of the previous emoji
            if c == '\u{FE0F}' || c == '\u{200D}' {
                continue;
            }
            if self.has(c)
                && let Some(t) = self.texture(ui.ctx(), c)
            {
                flush(ui, &mut run);
                let s = size * 1.25;
                ui.add(egui::Image::new(&t).fit_to_exact_size(egui::vec2(s, s)));
                continue;
            }
            run.push(c);
        }
        flush(ui, &mut run);
    }

    /// Emoji picker grid; returns the chosen emoji.
    pub fn picker(&mut self, ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons) -> Option<char> {
        if !self.available() {
            ui.label(egui::RichText::new("Police d'emoji Noto introuvable").size(12.0).color(p.muted));
            return None;
        }
        let mut chosen = None;
        ui.horizontal(|ui| {
            for (i, (label, icon, _)) in CATEGORIES.iter().enumerate() {
                let sel = self.tab == i;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
                if sel {
                    ui.painter().rect_filled(rect, 2.0, p.violet.gamma_multiply(0.3));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, p.raised);
                }
                if let Some(t) = icons.get(icon) {
                    let r = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
                    ui.painter().image(
                        t.id(),
                        r,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        if sel { p.ink } else { p.muted },
                    );
                }
                if resp.on_hover_text(*label).clicked() {
                    self.tab = i;
                }
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Rechercher un emoji (nom ou :code:)…")
                .desired_width(f32::INFINITY),
        );
        ui.separator();
        let q = self.query.trim().trim_matches(':').to_lowercase();
        let list: Vec<char> = if q.is_empty() {
            self.lists.get(self.tab).cloned().unwrap_or_default()
        } else {
            // search every category: English Unicode name and gemoji shortcodes
            let face = self.face.as_ref();
            emojis::iter()
                .filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
                .filter(|e| e.name().to_lowercase().contains(&q) || e.shortcodes().any(|s| s.contains(&q)))
                .filter_map(|e| single_char(e.as_str()))
                .filter(|c| face.is_some_and(|f| f.glyph_index(*c).is_some()))
                .take(400)
                .collect()
        };
        let cols = 9usize;
        let cell = 30.0;
        let rows = list.len().div_ceil(cols);
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .auto_shrink([false, true])
            .show_rows(ui, cell, rows, |ui, range| {
                for r in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for c in list.iter().skip(r * cols).take(cols) {
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell, cell), egui::Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 3.0, p.raised);
                            }
                            if let Some(t) = self.texture(ui.ctx(), *c) {
                                let ir = rect.shrink(3.0);
                                ui.painter().image(
                                    t.id(),
                                    ir,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );
                            }
                            // tooltip: shortcode to type it in chat
                            let s = c.to_string();
                            let info = emojis::get(&s).or_else(|| emojis::get(&format!("{s}\u{FE0F}")));
                            let resp = match info {
                                Some(e) => match e.shortcode() {
                                    Some(code) => resp.on_hover_text(format!(":{code}:  {}", e.name())),
                                    None => resp.on_hover_text(e.name()),
                                },
                                None => resp,
                            };
                            if resp.clicked() {
                                chosen = Some(*c);
                            }
                        }
                    });
                }
            });
        chosen
    }
}
