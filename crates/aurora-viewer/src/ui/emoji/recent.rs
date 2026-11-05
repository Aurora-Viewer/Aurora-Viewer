//! The emojis used last, a row under the chat fields. What is remembered
//! follows LLFloaterEmojiPicker::onEmojiUsed and FSCommon::updateUsedEmojis
//! (indra/newview/llfloateremojipicker.cpp, fscommon.cpp, originally LGPL
//! 2.1): the emojis of a sent message, the last one first, 20 at most, for
//! the account. A click adds one at the caret and leaves the keyboard in the
//! field (LLTextEditor::insertEmoji).
//!
//! Aurora differences: the row is a Preferences › Chat switch, on, in place
//! of a small arrow beside the field that starts closed at each session, and
//! it takes no room until there is an emoji to show.

use super::Emoji;
use crate::theme::Palette;
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};

pub const FILE: &str = "recent_emojis.json";
/// `maxCountR` of LLFloaterEmojiPicker::loadState, and the `max_visible` of
/// the Legacy row.
const MAX: usize = 20;
const CELL: f32 = 24.0;
const GAP: f32 = 2.0;
/// What the row adds to the height of its field.
pub const HEIGHT: f32 = CELL + GAP;

const VARIATION: char = '\u{FE0F}';

/// Whether `c`, followed by `next`, is an emoji worth remembering: what the
/// emoji list knows, without what only counts as one when asked to (© or an
/// arrow in plain text) and without the skin tones, which are not drawn alone.
fn is_emoji(c: char, next: Option<char>) -> bool {
    if ('\u{1F3FB}'..='\u{1F3FF}').contains(&c) {
        return false;
    }
    let text = c.to_string();
    // the list also answers for the bare form of what wants the selector;
    // from U+2600 the picker itself writes them without it (❤)
    emojis::get(&text).is_some_and(|e| e.as_str() == text || next == Some(VARIATION) || c >= '\u{2600}')
}

/// `list` with the emojis of `text` in front, the last one first; whether
/// it took any.
fn remember(list: &mut Vec<char>, text: &str) -> bool {
    let mut any = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_emoji(c, chars.peek().copied()) {
            list.retain(|known| *known != c);
            list.insert(0, c);
            any = true;
        }
    }
    list.truncate(MAX);
    any
}

/// A saved list, kept to what can show: a file edited by hand or by an
/// older version never reaches the row as it is.
fn parsed(bytes: &[u8]) -> Result<Vec<char>, serde_json::Error> {
    let saved: Vec<char> = serde_json::from_slice(bytes)?;
    let mut list = Vec::with_capacity(MAX);
    for c in saved {
        if is_emoji(c, None) && !list.contains(&c) && list.len() < MAX {
            list.push(c);
        }
    }
    Ok(list)
}

fn read(path: &Path) -> Vec<char> {
    match std::fs::read(path) {
        Ok(bytes) => parsed(&bytes).unwrap_or_else(|error| {
            log::warn!("Cannot read the recent emojis: {error}");
            Vec::new()
        }),
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Cannot read the recent emojis: {error}");
            }
            Vec::new()
        }
    }
}

fn write(path: &Path, list: &[char]) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(list)?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// The account's recent emojis.
#[derive(Default)]
pub struct Recent {
    list: Vec<char>,
    /// Preferences › Chat, given every frame by the app.
    pub enabled: bool,
    /// The writes leave the frame; none in the demo.
    writer: Option<Sender<Vec<char>>>,
}

impl Recent {
    /// The account's list, read at login: a file of a few dozen bytes.
    pub fn load(path: PathBuf) -> Recent {
        let list = read(&path);
        let (writer, writes) = crossbeam_channel::unbounded::<Vec<char>>();
        let spawned = std::thread::Builder::new().name("aurora-recent-emojis".into()).spawn(move || {
            for list in writes {
                if let Err(error) = write(&path, &list) {
                    log::warn!("Cannot save the recent emojis: {error}");
                }
            }
        });
        if let Err(error) = spawned {
            log::warn!("Recent emojis are not saved: {error}");
            return Recent { list, ..Recent::default() };
        }
        Recent {
            list,
            writer: Some(writer),
            ..Recent::default()
        }
    }

    /// `AURORA_DEMO_RECENT_EMOJIS`: `text`'s emojis, for the run.
    pub fn demo(text: &str) -> Recent {
        let mut recent = Recent::default();
        remember(&mut recent.list, text);
        recent
    }

    /// A message left: its emojis go first.
    pub fn note(&mut self, text: &str) {
        if remember(&mut self.list, text)
            && let Some(writer) = &self.writer
        {
            let _ = writer.send(self.list.clone());
        }
    }

    /// The row takes its place under the fields.
    pub fn shown(&self) -> bool {
        self.enabled && !self.list.is_empty()
    }

    /// What the fields take for the row.
    pub fn height(&self) -> f32 {
        if self.shown() { HEIGHT } else { 0.0 }
    }

    /// The row in `rect`, as many as fit; the emoji clicked. `field` is the
    /// chat field it serves.
    pub fn row(&self, ui: &egui::Ui, p: &Palette, emoji: &mut Emoji, rect: egui::Rect, field: egui::Id) -> Option<char> {
        let fit = (rect.width() / CELL).floor() as usize;
        let mut chosen = None;
        let mut placed = 0;
        for c in &self.list {
            if placed == fit {
                break;
            }
            if !emoji.has(*c) {
                continue;
            }
            let cell = egui::Rect::from_min_size(rect.min + egui::vec2(placed as f32 * CELL, 0.0), egui::Vec2::splat(CELL));
            let resp = ui.interact(cell, egui::Id::unique(("recent_emoji", field, placed)), egui::Sense::click());
            if emoji.cell(ui, p, *c, resp).clicked() {
                chosen = Some(*c);
            }
            placed += 1;
        }
        chosen
    }
}

/// `text` with `c` in place of the characters `from..to`.
fn spliced(text: &str, from: usize, to: usize, c: char) -> String {
    let mut out: String = text.chars().take(from).collect();
    out.push(c);
    out.extend(text.chars().skip(to));
    out
}

/// `c` at the caret of the field `id`, over its selection, the caret after
/// it and the keyboard back in the field.
pub fn insert(ctx: &egui::Context, id: egui::Id, text: &mut String, c: char) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let count = text.chars().count();
    let (from, to) = state.cursor.char_range().map_or((count, count), |r| {
        let (a, b) = (r.primary.index.0.min(count), r.secondary.index.0.min(count));
        (a.min(b), a.max(b))
    });
    *text = spliced(text, from, to, c);
    let after = egui::text::CCursor::new(from + 1);
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(after)));
    egui::TextEdit::store_state(ctx, id, state);
    ctx.memory_mut(|m| m.request_focus(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_emoji_of_a_message_comes_first_and_none_repeats() {
        let mut list = vec!['🔥', '😂'];
        assert!(remember(&mut list, "ha 😂 and 👍"));
        assert_eq!(list, ['👍', '😂', '🔥']);
        assert!(remember(&mut list, "🔥🔥"));
        assert_eq!(list, ['🔥', '👍', '😂']);
    }

    #[test]
    fn plain_text_is_no_emoji() {
        let mut list = Vec::new();
        assert!(!remember(&mut list, "a 1 # * © ® ™ → é 日本語 :) "), "{list:?}");
        assert!(remember(&mut list, "⌚ ©\u{FE0F}"), "an emoji by itself, or asked to be one");
        assert_eq!(list, ['©', '⌚']);
    }

    #[test]
    fn hearts_count_with_or_without_their_selector() {
        let mut list = Vec::new();
        assert!(remember(&mut list, "❤\u{FE0F}"));
        assert_eq!(list, ['❤']);
        list.clear();
        assert!(remember(&mut list, "❤"));
        assert_eq!(list, ['❤']);
    }

    #[test]
    fn a_skin_tone_keeps_its_hand_alone() {
        let mut list = Vec::new();
        remember(&mut list, "👍🏽");
        assert_eq!(list, ['👍']);
    }

    #[test]
    fn twenty_are_kept() {
        let text: String = emojis::iter()
            .filter_map(|e| super::super::single_char(e.as_str()))
            .take(30)
            .collect();
        let mut list = Vec::new();
        remember(&mut list, &text);
        assert_eq!(list.len(), MAX);
        assert_eq!(list.first(), text.chars().last().as_ref());
    }

    #[test]
    fn a_saved_list_is_kept_to_what_can_show() {
        assert_eq!(parsed("[\"😂\",\"a\",\"😂\",\"👍\"]".as_bytes()).expect("a list"), ['😂', '👍']);
        assert!(parsed(b"{").is_err());
    }

    #[test]
    fn the_list_comes_back_from_its_file() {
        let folder = std::env::temp_dir().join(format!("aurora-recent-emojis-{}", std::process::id()));
        let path = folder.join(FILE);
        write(&path, &['😂', '👍']).expect("written");
        assert_eq!(read(&path), ['😂', '👍']);
        let _ = std::fs::remove_dir_all(&folder);
        assert!(read(&path).is_empty());
    }

    #[test]
    fn a_pick_goes_at_the_caret_and_over_the_selection() {
        assert_eq!(spliced("héllo", 2, 2, '😂'), "hé😂llo");
        assert_eq!(spliced("héllo", 1, 4, '😂'), "h😂o");
        assert_eq!(spliced("", 0, 0, '😂'), "😂");
    }

    #[test]
    fn a_pick_keeps_the_caret_after_it_and_the_keyboard_in_the_field() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("field");
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(2))));
        egui::TextEdit::store_state(&ctx, id, state);
        let mut text = "abcd".to_owned();
        insert(&ctx, id, &mut text, '😂');
        insert(&ctx, id, &mut text, '👍');
        assert_eq!(text, "ab😂👍cd");
        assert!(ctx.memory(|m| m.has_focus(id)));
        // no caret yet: at the end, a caret left past a shorter text too
        let other = egui::Id::unique("other");
        let mut text = "ab".to_owned();
        insert(&ctx, other, &mut text, '😂');
        assert_eq!(text, "ab😂");
    }

    #[test]
    fn the_row_needs_the_switch_and_an_emoji() {
        let mut recent = Recent::demo("");
        recent.enabled = true;
        assert!(!recent.shown());
        assert_eq!(recent.height(), 0.0);
        recent.note("😂");
        assert!(recent.shown());
        recent.enabled = false;
        assert!(!recent.shown());
    }
}
