//! Color emoji from Google's Noto Color Emoji font (2D, CBDT PNG bitmaps,
//! OFL — see NOTICE.md), built into the executable; glyphs are decoded
//! lazily into egui textures. Provides the emoji picker and inline emoji
//! rendering for chat text.

use crate::theme::Palette;
use std::collections::HashMap;
use std::path::Path;

/// Picker tabs: (label, Tabler icon, Unicode group).
fn categories() -> &'static Vec<(&'static str, &'static str, emojis::Group)> {
    static TABLE: aurora_i18n::Localized<Vec<(&'static str, &'static str, emojis::Group)>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            (
                aurora_i18n::tr!("ui-emoji-smileys"),
                "mood-smile-filled",
                emojis::Group::SmileysAndEmotion,
            ),
            (aurora_i18n::tr!("common-people"), "user-filled", emojis::Group::PeopleAndBody),
            (
                aurora_i18n::tr!("ui-emoji-animals-and-nature"),
                "paw-filled",
                emojis::Group::AnimalsAndNature,
            ),
            (aurora_i18n::tr!("ui-emoji-food"), "apple-filled", emojis::Group::FoodAndDrink),
            (
                aurora_i18n::tr!("ui-emoji-travel"),
                "plane-tilt-filled",
                emojis::Group::TravelAndPlaces,
            ),
            (
                aurora_i18n::tr!("ui-emoji-activities"),
                "ball-bowling-filled",
                emojis::Group::Activities,
            ),
            (aurora_i18n::tr!("common-objects"), "bulb-filled", emojis::Group::Objects),
            (aurora_i18n::tr!("ui-emoji-symbols"), "star-filled", emojis::Group::Symbols),
        ]
    })
}

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

/// The font is part of the executable (10.7 MB): no folder of the
/// installation to lose or to be refused.
static BUILT_IN: &[u8] = include_bytes!("../../../../assets/emoji/NotoColorEmoji.ttf");

/// Another font to try (`AURORA_EMOJI_FONT`), kept for the session.
fn read_font(path: &Path) -> Option<&'static [u8]> {
    std::fs::read(path)
        .map_err(|e| log::warn!("emoji font {}: {e}", path.display()))
        .ok()
        .map(|bytes| &*Box::leak(bytes.into_boxed_slice()))
}

impl Emoji {
    pub fn load() -> Emoji {
        // The font asked for (to compare another one), else the built-in one.
        let asked = std::env::var_os("AURORA_EMOJI_FONT");
        let parse = |bytes: &'static [u8], name: &str| match ttf_parser::Face::parse(bytes, 0) {
            Ok(f) => {
                log::info!("emoji font: {name}");
                Some(f)
            }
            Err(e) => {
                log::warn!("emoji font {name}: {e}");
                None
            }
        };
        let face = asked
            .map(std::path::PathBuf::from)
            .and_then(|path| read_font(&path).and_then(|bytes| parse(bytes, &path.display().to_string())))
            .or_else(|| parse(BUILT_IN, "Noto Color Emoji, built into the executable"));
        if face.is_none() {
            log::warn!("no emoji font: emoji use the monochrome fallback");
        }
        let lists = categories()
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
        self.inline_marked(ui, text, size, color, italics, None);
    }

    /// `inline`, on a background when one is given (a search match).
    pub fn inline_marked(
        &mut self,
        ui: &mut egui::Ui,
        text: &str,
        size: f32,
        color: egui::Color32,
        italics: bool,
        background: Option<egui::Color32>,
    ) {
        let mut run = String::new();
        let flush = |ui: &mut egui::Ui, run: &mut String| {
            if !run.is_empty() {
                let mut rt = egui::RichText::new(std::mem::take(run)).size(size).color(color);
                if italics {
                    rt = rt.italics();
                }
                if let Some(bg) = background {
                    rt = rt.background_color(bg);
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

    /// A one-line text field (the conversation input) whose emoji show in
    /// colour, as in the history. egui's text only has one-colour glyphs, so
    /// the emoji are laid out blank (same size as in `inline`) and their
    /// images drawn over them: editing, cursor and selection stay egui's.
    /// `build` sets the field up (id, hint, width).
    /// Return the editing state so callers can select a completed trigger.
    pub fn text_field(
        &mut self,
        ui: &mut egui::Ui,
        text: &mut String,
        build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
    ) -> egui::text_edit::TextEditOutput {
        // without emoji, egui's own layout
        if !text.chars().any(|c| self.has(c)) {
            return build(egui::TextEdit::singleline(text)).show(ui);
        }
        // what TextEdit uses when given no font nor colour
        let font = egui::FontSelection::default().resolve(ui.style());
        let color = ui
            .visuals()
            .override_text_color
            .unwrap_or_else(|| ui.visuals().widgets.inactive.text_color());
        let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        // the size `inline` gives the emoji pictures
        let side = font.size * 1.25;
        let format = |blank: bool| {
            let mut f = if blank {
                egui::TextFormat::simple(egui::FontId::new(side, font.family.clone()), egui::Color32::TRANSPARENT)
            } else {
                egui::TextFormat::simple(font.clone(), color)
            };
            f.line_height = Some(line_height);
            f.valign = egui::Align::Center;
            f
        };
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
            let text = text.as_str();
            let mut job = egui::text::LayoutJob {
                keep_trailing_whitespace: true,
                ..Default::default()
            };
            // runs of text and runs of emoji (with their variation selectors
            // and joiners, which `inline` does not draw either)
            let mut start = 0;
            let mut blank_run = false;
            for (i, c) in text.char_indices() {
                let blank = self.has(c) || c == '\u{FE0F}' || c == '\u{200D}';
                if blank != blank_run && i > start {
                    job.append(&text[start..i], 0.0, format(blank_run));
                    start = i;
                }
                blank_run = blank;
            }
            if start < text.len() {
                job.append(&text[start..], 0.0, format(blank_run));
            }
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let out = build(egui::TextEdit::singleline(text).layouter(&mut layouter)).show(ui);
        let painter = ui.painter().with_clip_rect(out.text_clip_rect);
        for placed in &out.galley.rows {
            let mid_y = out.galley_pos.y + placed.pos.y + placed.row.size.y / 2.0;
            for g in &placed.row.glyphs {
                if !self.has(g.chr) {
                    continue;
                }
                if let Some(t) = self.texture(ui.ctx(), g.chr) {
                    let mid_x = out.galley_pos.x + placed.pos.x + g.pos.x + g.advance_width / 2.0;
                    let size = t.size_vec2() * (side / t.size_vec2().max_elem());
                    painter.image(
                        t.id(),
                        egui::Rect::from_center_size(egui::pos2(mid_x, mid_y), size),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        crate::ui::widgets::PICTURE_TINT,
                    );
                }
            }
        }
        out
    }

    /// Emoji picker grid; returns the chosen emoji.
    pub fn picker(&mut self, ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons) -> Option<char> {
        if !self.available() {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-emoji-noto-emoji-font-not-found"))
                    .size(12.0)
                    .color(p.muted),
            );
            return None;
        }
        let mut chosen = None;
        ui.horizontal(|ui| {
            for (i, (label, icon, _)) in categories().iter().enumerate() {
                let sel = self.tab == i;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
                if sel {
                    ui.painter()
                        .rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.3));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
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
                .hint_text(aurora_i18n::tr!("ui-emoji-search-emoji-name-or-code"))
                .desired_width(f32::INFINITY),
        );
        crate::ui::widgets::divider(ui, p);
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
                                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
                            }
                            if let Some(t) = self.texture(ui.ctx(), *c) {
                                // keep the image's proportions: the 2D font's
                                // bitmaps are wider than tall (136 × 128)
                                let slot = rect.shrink(3.0);
                                let size = t.size_vec2();
                                let ir =
                                    egui::Rect::from_center_size(slot.center(), size * (slot.width() / size.x).min(slot.height() / size.y));
                                ui.painter().image(
                                    t.id(),
                                    ir,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    crate::ui::widgets::PICTURE_TINT,
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
