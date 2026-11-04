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

pub mod codes;

/// Replace `:shortcode:` (GitHub / gemoji names, e.g. `:hearts:`, `:smile:`)
/// by the emoji, by the rules of the chat fields ([`codes::expand`]: not
/// inside a word, a time or a link).
pub fn expand_shortcodes(text: &str) -> String {
    codes::expand(text)
}

enum Glyph {
    Ready(egui::TextureHandle),
    Missing,
}

pub struct Emoji {
    face: Option<ttf_parser::Face<'static>>,
    /// By code point and full size (the font's bitmap) or not.
    glyphs: HashMap<(char, bool), Glyph>,
    /// Per-category list of code points that exist in the font.
    lists: Vec<Vec<char>>,
    pub tab: usize,
    /// Picker search text.
    pub query: String,
    /// The egui pass that last drew the picker: a picker not drawn in the
    /// pass before was just opened, and its search takes the keyboard.
    shown_pass: Option<u64>,
}

/// What the picker did this pass: an emoji chosen, or Escape pressed (the
/// caller closes it and gives the keyboard back to its field).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Picked {
    pub chosen: Option<char>,
    pub escape: bool,
}

/// The picker's search field.
const SEARCH_ID: &str = "emoji_picker_search";
/// Set to the pass number by every pass that draws a picker
/// ([`picker_open`]).
const OPEN_ID: &str = "emoji_picker_open";

/// Another picker of the Conversations field (the GIF picker) was drawn in
/// this pass: Escape is its own too ([`picker_open`]).
pub fn note_picker_shown(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(OPEN_ID), pass));
}

/// A picker was drawn in the last pass: Escape is its own, not the camera's
/// (the window events come before the pass that would consume the key).
pub fn picker_open(ctx: &egui::Context) -> bool {
    let shown: Option<u64> = ctx.data(|d| d.get_temp(egui::Id::unique(OPEN_ID)));
    shown.is_some_and(|p| p + 1 >= ctx.cumulative_pass_nr())
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

/// Rows of text with pictures in them (emoji, link badges), as tall as the
/// rows of a label of that `size`: a wrapped layout's rows are at least
/// `interact_size.y` tall (26 in the theme), which made a line with an
/// emoji taller than one without.
pub fn text_rows<R>(ui: &mut egui::Ui, size: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    text_rows_in(ui, size, false, add)
}

/// `text_rows` for a text that reads from right to left (`rtl`): its pieces
/// (runs of text, links, pictures) are placed from the right edge, so that
/// they come in reading order.
pub fn text_rows_in<R>(ui: &mut egui::Ui, size: f32, rtl: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let row = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(size)));
    let before = ui.spacing().interact_size.y;
    ui.spacing_mut().interact_size.y = row;
    let rows = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
        add(ui)
    };
    let out = if rtl {
        // (Ui::horizontal_wrapped's own size, the other way)
        let size = egui::vec2(ui.available_size_before_wrap().x, row);
        let layout = egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true);
        ui.allocate_ui_with_layout(size, layout, rows).inner
    } else {
        ui.horizontal_wrapped(rows).inner
    };
    ui.spacing_mut().interact_size.y = before;
    out
}

/// Side of an emoji picture in a text of `size`: a little larger than the
/// letters, as the color emoji fonts draw them.
pub fn emoji_side(size: f32) -> f32 {
    size * 1.25
}

/// Most emoji a message may hold alone and still be drawn large.
const JUMBO_MAX: usize = 27;
/// Text size of a message of emoji alone, against the other messages':
/// 13.5 gives pictures of 40 points.
const JUMBO_SCALE: f32 = 2.4;
/// Side the pictures are decoded at for the text; larger ones (a message of
/// emoji alone, a large text size, a dense screen) take the font's own
/// bitmap (136 × 128) instead of an enlarged blur.
const DECODED: u32 = 64;

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
            shown_pass: None,
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
        self.texture_sized(ctx, c, false)
    }

    /// Texture of an emoji, the font's own bitmap when `full`.
    fn texture_sized(&mut self, ctx: &egui::Context, c: char, full: bool) -> Option<egui::TextureHandle> {
        if let Some(g) = self.glyphs.get(&(c, full)) {
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
                let img = if full {
                    img.to_rgba8()
                } else {
                    img.resize(DECODED, DECODED, image::imageops::FilterType::Triangle).to_rgba8()
                };
                let (w, h) = img.dimensions();
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw())
            });
        let name = format!("emoji-{:x}{}", c as u32, if full { "-full" } else { "" });
        let g = match decoded {
            Some(ci) => Glyph::Ready(ctx.load_texture(name, ci, egui::TextureOptions::LINEAR)),
            None => Glyph::Missing,
        };
        let out = match &g {
            Glyph::Ready(t) => Some(t.clone()),
            Glyph::Missing => None,
        };
        self.glyphs.insert((c, full), g);
        out
    }

    /// A message of color emoji alone (spaces aside), at most `JUMBO_MAX`
    /// pictures: its emoji are drawn large. Counted as drawn: a sequence
    /// (joiner, skin tone) is still drawn as its pictures.
    pub fn only_emoji(&self, text: &str) -> bool {
        let mut pictures = 0;
        for c in text.chars() {
            if c.is_whitespace() || c == '\u{FE0F}' || c == '\u{200D}' {
                continue;
            }
            if !self.has(c) {
                return false;
            }
            pictures += 1;
        }
        (1..=JUMBO_MAX).contains(&pictures)
    }

    /// The text size of a message of the conversation (issue #483): larger
    /// for emoji alone. The Aurora, Compact and Bubbles styles only: the
    /// Legacy lines, the History window and the chat lines at the bottom of
    /// the screen keep one size, as in the Legacy Viewer.
    pub fn message_size(&self, text: &str, size: f32) -> f32 {
        if self.only_emoji(text) { size * JUMBO_SCALE } else { size }
    }

    /// Text with color emoji inline (wraps like a label). The text stays
    /// selectable for copying, like Firestorm's read-only chat editors.
    pub fn rich_text(
        &mut self,
        ui: &mut egui::Ui,
        text: &str,
        size: f32,
        color: egui::Color32,
        emphasis: impl Into<super::fonts::TextEmphasis>,
    ) {
        let emphasis = emphasis.into();
        if !self.available() || !text.chars().any(|c| self.has(c)) {
            let rt = emphasis.rich_text(ui.ctx(), text, size, color);
            // (in a conversation, its selection is the conversation's; a
            // text read from the right starts at the right edge)
            if super::bidi::rtl_paragraph(text) {
                text_rows_in(ui, size, true, |ui| super::chat::select::label(ui, rt));
            } else {
                super::chat::select::label(ui, rt);
            }
            return;
        }
        let rtl = super::bidi::rtl_paragraph(text);
        text_rows_in(ui, size, rtl, |ui| self.inline(ui, text, size, color, emphasis));
    }

    /// Text with color emoji added to the current (wrapping) layout.
    pub fn inline(
        &mut self,
        ui: &mut egui::Ui,
        text: &str,
        size: f32,
        color: egui::Color32,
        emphasis: impl Into<super::fonts::TextEmphasis>,
    ) {
        self.inline_marked(ui, text, size, color, emphasis, None);
    }

    /// `inline`, on a background when one is given (a search match).
    pub fn inline_marked(
        &mut self,
        ui: &mut egui::Ui,
        text: &str,
        size: f32,
        color: egui::Color32,
        emphasis: impl Into<super::fonts::TextEmphasis>,
        background: Option<egui::Color32>,
    ) {
        let emphasis = emphasis.into();
        let mut run = String::new();
        let flush = |ui: &mut egui::Ui, run: &mut String| {
            if !run.is_empty() {
                let mut rt = emphasis.rich_text(ui.ctx(), std::mem::take(run), size, color);
                if let Some(bg) = background {
                    rt = rt.background_color(bg);
                }
                super::chat::select::label(ui, rt);
            }
        };
        for c in text.chars() {
            // variation selector / zero-width joiner: part of the previous emoji
            if c == '\u{FE0F}' || c == '\u{200D}' {
                continue;
            }
            // a little larger than the letters, the picture keeps the height
            // of a row of text: centered on it, it overflows the row instead
            // of making a line with an emoji taller
            let s = emoji_side(size) * super::fonts::scale(ui.ctx());
            let full = s * ui.ctx().pixels_per_point() > DECODED as f32;
            if self.has(c)
                && let Some(t) = self.texture_sized(ui.ctx(), c, full)
            {
                flush(ui, &mut run);
                let row = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(size)));
                let behind = super::chat::select::atom_begin(ui);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(s, row), egui::Sense::hover());
                egui::Image::new(&t).paint_at(ui, egui::Rect::from_center_size(rect.center(), egui::vec2(s, s)));
                super::chat::select::atom_end(ui, behind, c.encode_utf8(&mut [0; 4]), rect);
                continue;
            }
            run.push(c);
        }
        flush(ui, &mut run);
    }

    /// A wrapping text field (the conversation input) whose emoji show in
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
            let edit = egui::TextEdit::multiline(text).desired_rows(1).return_key(None);
            return build(edit).show(ui);
        }
        // what TextEdit uses when given no font nor colour
        let font = egui::FontSelection::default().resolve(ui.style());
        let color = ui
            .visuals()
            .override_text_color
            .unwrap_or_else(|| ui.visuals().widgets.inactive.text_color());
        let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        // the size `inline` gives the emoji pictures
        let side = emoji_side(font.size);
        let picture_side = side * super::fonts::scale(ui.ctx());
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
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = text.as_str();
            let mut job = egui::text::LayoutJob {
                keep_trailing_whitespace: true,
                ..Default::default()
            };
            job.wrap.max_width = wrap_width;
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
        let edit = egui::TextEdit::multiline(text).desired_rows(1).return_key(None);
        let out = build(edit.layouter(&mut layouter)).show(ui);
        let painter = ui.painter().with_clip_rect(out.text_clip_rect);
        for placed in &out.galley.rows {
            let mid_y = out.galley_pos.y + placed.pos.y + placed.row.size.y / 2.0;
            if mid_y + picture_side < out.text_clip_rect.top() || mid_y - picture_side > out.text_clip_rect.bottom() {
                continue;
            }
            for g in &placed.row.glyphs {
                if !self.has(g.chr) {
                    continue;
                }
                if let Some(t) = self.texture(ui.ctx(), g.chr) {
                    let mid_x = out.galley_pos.x + placed.pos.x + g.pos.x + g.advance_width / 2.0;
                    let size = t.size_vec2() * (picture_side / t.size_vec2().max_elem());
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

    /// Emoji picker grid. Opened, its search takes the keyboard (typing
    /// filters at once) and keeps it while nothing else has it; Escape is
    /// taken here, before the search field or anything after the picker sees
    /// it, and returned for the caller to close the picker.
    pub fn picker(&mut self, ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons) -> Picked {
        let pass = ui.ctx().cumulative_pass_nr();
        let opened = self.shown_pass.is_none_or(|last| last + 1 < pass);
        self.shown_pass = Some(pass);
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(OPEN_ID), pass));
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if !self.available() {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-emoji-noto-emoji-font-not-found"))
                    .size(12.0)
                    .color(p.muted),
            );
            return Picked { chosen: None, escape };
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
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(egui::Id::unique(SEARCH_ID))
                .hint_text(aurora_i18n::tr!("ui-emoji-search-emoji-name-or-code"))
                .desired_width(f32::INFINITY),
        );
        // (a click on a tab or a picture leaves the keyboard nowhere: back
        // to the search, so typing still filters)
        if opened || ui.ctx().memory(|m| m.focused().is_none()) {
            search.request_focus();
        }
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
        Picked { chosen, escape }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;

    fn escape() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Emoji alone, spaces and variation selectors aside, up to the limit:
    /// large; any letter, digit or punctuation, or too many: the text's size.
    #[test]
    fn only_emoji_messages_are_large() {
        let emoji = Emoji::load();
        for text in ["😀", " 😀 ", "😀😂👍", "❤️", "❤\u{FE0F} 🔥", "👍🏽", &"🎉".repeat(JUMBO_MAX)] {
            assert!(emoji.only_emoji(text), "{text:?} is emoji alone");
            assert_eq!(emoji.message_size(text, 13.5), 13.5 * JUMBO_SCALE);
        }
        for text in ["", "   ", "hi 😀", "😀!", "1", ":)", "😀 a", &"🎉".repeat(JUMBO_MAX + 1)] {
            assert!(!emoji.only_emoji(text), "{text:?} is not emoji alone");
            assert_eq!(emoji.message_size(text, 13.5), 13.5);
        }
    }

    /// Opened, the picker's search has the keyboard and what is typed
    /// filters; Escape is the picker's alone (nothing drawn after it sees
    /// the key) and the search keeps its text; opened again, the search takes
    /// the keyboard again.
    #[test]
    fn the_search_takes_the_keyboard_and_escape_is_the_pickers() {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = Emoji::load();
        let field = egui::Id::unique("field");
        let mut text = String::new();
        let mut frame = |frames: &mut Frames, emoji: &mut Emoji, open: bool, events: Vec<egui::Event>| {
            let mut picked = Picked::default();
            let mut seen_after = false;
            frames.run(events, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(field));
                if open {
                    picked = emoji.picker(ui, &p, &icons);
                }
                seen_after = ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
            (picked, seen_after)
        };
        frames.ctx.memory_mut(|m| m.request_focus(field));
        frame(&mut frames, &mut emoji, false, vec![]);
        // opened: the search has the keyboard and typing filters
        frame(&mut frames, &mut emoji, true, vec![]);
        frame(&mut frames, &mut emoji, true, vec![]);
        assert!(frames.ctx.memory(|m| m.has_focus(egui::Id::unique(SEARCH_ID))));
        assert!(picker_open(&frames.ctx));
        frame(&mut frames, &mut emoji, true, vec![egui::Event::Text("heart".into())]);
        assert_eq!(emoji.query, "heart");
        // Escape: returned, consumed, the search unchanged
        let (picked, seen_after) = frame(&mut frames, &mut emoji, true, vec![escape()]);
        assert!(picked.escape && picked.chosen.is_none());
        assert!(!seen_after, "Escape reached what is drawn after the picker");
        assert_eq!(emoji.query, "heart");
        // closed, then opened again: the keyboard goes to the search again
        frames.ctx.memory_mut(|m| m.request_focus(field));
        frame(&mut frames, &mut emoji, false, vec![]);
        frame(&mut frames, &mut emoji, false, vec![]);
        assert!(!picker_open(&frames.ctx));
        assert!(frames.ctx.memory(|m| m.has_focus(field)));
        frame(&mut frames, &mut emoji, true, vec![]);
        frame(&mut frames, &mut emoji, true, vec![]);
        assert!(frames.ctx.memory(|m| m.has_focus(egui::Id::unique(SEARCH_ID))));
    }
}
