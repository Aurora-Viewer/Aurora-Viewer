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
//!
//! Aurora addition: favourite emojis, kept for the account, marked from the
//! right-click menu of an emoji. They come first in the row, with a small
//! star, and have their tab in the picker.

use super::Emoji;
use crate::theme::Palette;
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const FILE: &str = "recent_emojis.json";
pub const FAVORITES_FILE: &str = "favorite_emojis.json";
/// `maxCountR` of LLFloaterEmojiPicker::loadState, and the `max_visible` of
/// the Legacy row.
const MAX: usize = 20;
/// Favourites kept: two rows of the picker and a bit more.
const MAX_FAVORITES: usize = 27;
const CELL: f32 = 24.0;
const GAP: f32 = 2.0;
/// What the row adds to the height of its field.
pub const HEIGHT: f32 = CELL + GAP;

const VARIATION: char = '\u{FE0F}';
static WRITER: AtomicU64 = AtomicU64::new(0);

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

/// `c` as text: with its selector when it is only an emoji with one (⏱,
/// ©), so that it reads as an emoji again when sent.
fn written(c: char) -> String {
    let mut text = c.to_string();
    if c < '\u{2600}' && emojis::get(&text).is_some_and(|e| e.as_str() != text) {
        text.push(VARIATION);
    }
    text
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
fn parsed(bytes: &[u8], max: usize) -> Result<Vec<char>, serde_json::Error> {
    let saved: Vec<char> = serde_json::from_slice(bytes)?;
    let mut list = Vec::with_capacity(max);
    for c in saved {
        // kept without their selector: one was there when they were sent
        if is_emoji(c, Some(VARIATION)) && !list.contains(&c) && list.len() < max {
            list.push(c);
        }
    }
    Ok(list)
}

fn read(path: &Path, max: usize) -> Vec<char> {
    match std::fs::read(path) {
        Ok(bytes) => parsed(&bytes, max).unwrap_or_else(|error| {
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

/// `writer` names its temporary file: a logout's writer may still be
/// finishing as the next login of the account starts its own.
fn write(path: &Path, list: &[char], writer: u64) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(list)?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let temporary = path.with_extension(format!("json.{writer}.tmp"));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// The account's recent emojis.
#[derive(Default)]
pub struct Recent {
    list: Vec<char>,
    /// The favourites, newest first.
    favorites: Vec<char>,
    /// Preferences › Chat, given every frame by the app.
    pub enabled: bool,
    /// Preferences › Chat › Favorite emojis, off by default, given every
    /// frame by the app: while off, no star, menu nor tab, and the row shows
    /// the recent emojis alone (the file stays for when they come back).
    pub favorites_enabled: bool,
    /// `AURORA_DEMO_FAVORITE_EMOJIS`: on for the run, whatever the switch.
    demo_favorites: bool,
    /// The row's place, given every frame by the app: above the chat
    /// fields, nearer the lines read (the human's choice, 2026-11-09);
    /// under them with the Legacy chat style, as FSFloaterNearbyChat's
    /// `emoji_recent_layout_panel` follows its `chat_layout_panel`.
    pub above: bool,
    /// The writes leave the frame, with the file each goes to; none in the
    /// demo.
    writer: Option<Sender<(&'static str, Vec<char>)>>,
    /// The writing thread, waited for when the viewer ends (`finish`).
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Recent {
    /// The account's lists, read at login from its `folder`: files of a few
    /// dozen bytes.
    pub fn load(folder: PathBuf) -> Recent {
        let list = read(&folder.join(FILE), MAX);
        let favorites = read(&folder.join(FAVORITES_FILE), MAX_FAVORITES);
        let (writer, writes) = crossbeam_channel::unbounded::<(&'static str, Vec<char>)>();
        let id = WRITER.fetch_add(1, Ordering::Relaxed);
        let spawned = std::thread::Builder::new().name("aurora-recent-emojis".into()).spawn(move || {
            for (file, list) in writes {
                if let Err(error) = write(&folder.join(file), &list, id) {
                    log::warn!("Cannot save the recent emojis: {error}");
                }
            }
        });
        let thread = match spawned {
            Ok(thread) => thread,
            Err(error) => {
                log::warn!("Recent emojis are not saved: {error}");
                return Recent {
                    list,
                    favorites,
                    ..Recent::default()
                };
            }
        };
        Recent {
            list,
            favorites,
            writer: Some(writer),
            thread: Some(thread),
            ..Recent::default()
        }
    }

    /// Before the viewer ends: the last list asked for reaches its file,
    /// `timeout` at most (an emoji sent just before quitting is kept).
    pub fn finish(&mut self, timeout: std::time::Duration) {
        // the thread ends once the writes already sent are done
        self.writer = None;
        let Some(thread) = self.thread.take() else { return };
        let start = std::time::Instant::now();
        while !thread.is_finished() {
            if start.elapsed() >= timeout {
                log::warn!("recent emojis: the last save was not written within {timeout:?}");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = thread.join();
    }

    /// `AURORA_DEMO_RECENT_EMOJIS`: `text`'s emojis, for the run.
    pub fn demo(text: &str) -> Recent {
        let mut recent = Recent::default();
        remember(&mut recent.list, text);
        recent
    }

    /// `AURORA_DEMO_FAVORITE_EMOJIS`: `text`'s emojis as the favourites.
    pub fn demo_favorites(&mut self, text: &str) {
        remember(&mut self.favorites, text);
        self.demo_favorites = true;
    }

    /// The favourites, newest first; `None` while they are turned off.
    pub fn favorites(&self) -> Option<&[char]> {
        (self.favorites_enabled || self.demo_favorites).then_some(self.favorites.as_slice())
    }

    /// The favourites the row draws: none while they are turned off.
    fn shown_favorites(&self) -> &[char] {
        self.favorites().unwrap_or_default()
    }

    /// `c` becomes a favourite, or stops being one.
    pub fn toggle_favorite(&mut self, c: char) {
        if self.favorites.contains(&c) {
            self.favorites.retain(|known| *known != c);
        } else {
            self.favorites.insert(0, c);
            self.favorites.truncate(MAX_FAVORITES);
        }
        if let Some(writer) = &self.writer {
            let _ = writer.send((FAVORITES_FILE, self.favorites.clone()));
        }
    }

    /// A message left: its emojis go first.
    pub fn note(&mut self, text: &str) {
        if remember(&mut self.list, text)
            && let Some(writer) = &self.writer
        {
            let _ = writer.send((FILE, self.list.clone()));
        }
    }

    /// The row takes its place under the fields.
    pub fn shown(&self) -> bool {
        self.enabled && !(self.list.is_empty() && self.shown_favorites().is_empty())
    }

    /// What the fields take for the row.
    pub fn height(&self) -> f32 {
        if self.shown() { HEIGHT } else { 0.0 }
    }

    /// The favourites, then the recent emojis that are not.
    fn in_row(&self) -> impl Iterator<Item = char> + '_ {
        let favorites = self.shown_favorites();
        let recent = self.list.iter().filter(move |c| !favorites.contains(c));
        favorites.iter().chain(recent).copied()
    }

    /// The row on a strip of `width` taken from `ui`'s layout, when it is
    /// shown: a click puts its emoji in `draft` at `field`'s caret. The
    /// strip's rect.
    pub fn strip(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        emoji: &mut Emoji,
        width: f32,
        field: egui::Id,
        draft: &mut String,
    ) -> Option<egui::Rect> {
        if !self.shown() {
            return None;
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEIGHT - ui.spacing().item_spacing.y), egui::Sense::hover());
        if let Some(c) = self.row(ui, p, emoji, rect, field) {
            insert(ui.ctx(), field, draft, c);
        }
        Some(rect)
    }

    /// The row in `rect`, as many as fit; the emoji clicked. `field` is the
    /// chat field it serves.
    pub fn row(&mut self, ui: &egui::Ui, p: &Palette, emoji: &mut Emoji, rect: egui::Rect, field: egui::Id) -> Option<char> {
        let fit = (rect.width() / CELL).floor() as usize;
        let mut chosen = None;
        let mut placed = 0;
        let mut toggled = None;
        for c in self.in_row() {
            if placed == fit {
                break;
            }
            if !emoji.in_font(c) {
                continue;
            }
            let cell = egui::Rect::from_min_size(rect.min + egui::vec2(placed as f32 * CELL, 0.0), egui::Vec2::splat(CELL));
            let resp = ui.interact(cell, egui::Id::unique(("recent_emoji", field, placed)), egui::Sense::click());
            let favorite = self.favorites().map(|f| f.contains(&c));
            let (resp, toggle) = emoji.cell(ui, p, c, favorite, resp);
            if resp.clicked() {
                chosen = Some(c);
            }
            if toggle {
                toggled = Some(c);
            }
            placed += 1;
        }
        if let Some(c) = toggled {
            self.toggle_favorite(c);
        }
        chosen
    }
}

/// `text` with `emoji` in place of the characters `from..to`.
fn spliced(text: &str, from: usize, to: usize, emoji: &str) -> String {
    let mut out: String = text.chars().take(from).collect();
    out.push_str(emoji);
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
    let emoji = written(c);
    *text = spliced(text, from, to, &emoji);
    let after = egui::text::CCursor::new(from + emoji.chars().count());
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
    fn an_emoji_that_needs_its_selector_survives_the_file_and_the_row() {
        let mut list = Vec::new();
        assert!(remember(&mut list, "⏱\u{FE0F} ↔\u{FE0F} ©\u{FE0F}"));
        assert_eq!(list, ['©', '↔', '⏱']);
        let saved = serde_json::to_vec(&list).expect("a list");
        assert_eq!(parsed(&saved, MAX).expect("a list"), list);
        // picked from the row and sent again, it counts again
        let ctx = egui::Context::default();
        let mut text = String::new();
        insert(&ctx, egui::Id::unique("selector"), &mut text, '⏱');
        assert_eq!(text, "⏱\u{FE0F}");
        assert!(remember(&mut list, &text));
        assert_eq!(list, ['⏱', '©', '↔']);
        assert_eq!(written('😂'), "😂");
        assert_eq!(written('❤'), "❤");
    }

    #[test]
    fn a_saved_list_is_kept_to_what_can_show() {
        assert_eq!(
            parsed("[\"😂\",\"a\",\"😂\",\"👍\"]".as_bytes(), MAX).expect("a list"),
            ['😂', '👍']
        );
        assert!(parsed(b"{", MAX).is_err());
    }

    #[test]
    fn the_list_comes_back_from_its_file() {
        let folder = std::env::temp_dir().join(format!("aurora-recent-emojis-{}", std::process::id()));
        let path = folder.join(FILE);
        write(&path, &['😂', '👍'], 0).expect("written");
        assert_eq!(read(&path, MAX), ['😂', '👍']);
        let _ = std::fs::remove_dir_all(&folder);
        assert!(read(&path, MAX).is_empty());
    }

    /// An emoji sent and a favourite kept just before quitting are in their
    /// files once `finish` returns, written by the thread.
    #[test]
    fn the_last_list_is_written_before_the_viewer_ends() {
        let folder = std::env::temp_dir().join(format!("aurora-recent-emojis-finish-{}", std::process::id()));
        let path = folder.join(FILE);
        let mut recent = Recent::load(folder.clone());
        recent.note("bye 👋");
        recent.toggle_favorite('⭐');
        recent.finish(std::time::Duration::from_secs(5));
        assert_eq!(read(&path, MAX), ['👋']);
        assert_eq!(read(&folder.join(FAVORITES_FILE), MAX_FAVORITES), ['⭐']);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_pick_goes_at_the_caret_and_over_the_selection() {
        assert_eq!(spliced("héllo", 2, 2, "😂"), "hé😂llo");
        assert_eq!(spliced("héllo", 1, 4, "😂"), "h😂o");
        assert_eq!(spliced("", 0, 0, "😂"), "😂");
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
    fn favorites_come_first_in_the_row_without_repeating() {
        let mut recent = Recent::demo("😂🔥👍");
        recent.favorites_enabled = true;
        recent.toggle_favorite('🔥');
        recent.toggle_favorite('❤');
        assert_eq!(recent.favorites(), Some(&['❤', '🔥'][..]));
        assert_eq!(recent.in_row().collect::<Vec<_>>(), ['❤', '🔥', '👍', '😂']);
        recent.toggle_favorite('🔥');
        assert_eq!(recent.favorites(), Some(&['❤'][..]));
        assert_eq!(recent.in_row().collect::<Vec<_>>(), ['❤', '👍', '🔥', '😂']);
    }

    #[test]
    fn favorites_alone_show_the_row() {
        let mut recent = Recent::demo("");
        recent.enabled = true;
        recent.favorites_enabled = true;
        recent.toggle_favorite('❤');
        assert!(recent.shown());
        recent.enabled = false;
        assert!(!recent.shown());
    }

    /// Off by default: the row is the recent emojis alone, as before
    /// favourites existed, and the kept ones come back with the switch.
    #[test]
    fn favorites_turned_off_leave_the_recent_row_alone() {
        let mut recent = Recent::demo("😂🔥");
        recent.enabled = true;
        recent.toggle_favorite('❤');
        assert_eq!(recent.favorites(), None);
        assert_eq!(recent.in_row().collect::<Vec<_>>(), ['🔥', '😂']);
        let mut alone = Recent::demo("");
        alone.enabled = true;
        alone.toggle_favorite('❤');
        assert!(!alone.shown(), "no row for favourites turned off");
        recent.favorites_enabled = true;
        assert_eq!(recent.in_row().collect::<Vec<_>>(), ['❤', '🔥', '😂']);
    }

    #[test]
    fn favorites_are_bounded_and_come_back_from_their_file() {
        let chars: Vec<char> = emojis::iter()
            .filter_map(|e| super::super::single_char(e.as_str()))
            .filter(|c| is_emoji(*c, None))
            .take(MAX_FAVORITES + 3)
            .collect();
        let mut recent = Recent::demo("");
        for c in &chars {
            recent.toggle_favorite(*c);
        }
        assert_eq!(recent.favorites.len(), MAX_FAVORITES);
        assert_eq!(recent.favorites.first(), chars.last());
        let folder = std::env::temp_dir().join(format!("aurora-favorite-emojis-{}", std::process::id()));
        write(&folder.join(FAVORITES_FILE), &['⏱', '😂'], 0).expect("written");
        assert_eq!(Recent::load(folder.clone()).favorites, ['⏱', '😂']);
        let _ = std::fs::remove_dir_all(&folder);
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
