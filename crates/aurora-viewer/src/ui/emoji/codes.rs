//! Emoji shortcodes in the chat fields (`:heart:`, `:smile:`, `:+1:`): the
//! gemoji names of the picker's search, turned into their emoji when the
//! closing `:` is typed, with a list of suggestions from `:` and two letters
//! like the common chat applications. The same rules expand what is sent
//! ([`expand`]).
//!
//! A code opens after the start of the text, a space or an opening bracket
//! or quote (not after a letter, a digit or another `:`: `12:30:45`,
//! `a:b:c`), never inside a link (`https://…`), and only a known code is
//! replaced.

use super::Emoji;
use crate::theme::Palette;

/// A character of a code: gemoji names are ASCII letters, digits, `_`, `+`
/// and `-` (`:+1:`, `:-1:`, `:t-rex:`).
fn code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// A code may open after this character (None: the start of the text).
fn opens_after(prev: Option<char>) -> bool {
    prev.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '«'))
}

/// The word around `at` (from the space before it) is a link.
fn in_link(chars: &[char], at: usize) -> bool {
    let start = chars[..at].iter().rposition(|c| c.is_whitespace()).map_or(0, |i| i + 1);
    let word: String = chars[start..at].iter().collect::<String>().to_lowercase();
    word.contains("://") || word.starts_with("www.")
}

/// The emoji of a code (`heart` → ❤️), None when it is not a known one.
pub fn lookup(code: &str) -> Option<&'static str> {
    emojis::get_by_shortcode(&code.to_ascii_lowercase()).map(|e| e.as_str())
}

/// The code being typed before `cursor` (a char index): the index of its
/// opening `:` and the letters typed after it (may be empty).
pub fn open_code(chars: &[char], cursor: usize) -> Option<(usize, String)> {
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    while start > 0 && code_char(chars[start - 1]) {
        start -= 1;
    }
    let colon = start.checked_sub(1)?;
    if chars[colon] != ':' || !opens_after(colon.checked_sub(1).map(|i| chars[i])) || in_link(chars, colon) {
        return None;
    }
    Some((colon, chars[start..cursor].iter().collect()))
}

/// The closing `:` of a known code just typed before `cursor`: the text with
/// the code replaced by its emoji, and the cursor after the emoji.
pub fn close_code(text: &str, cursor: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    if cursor == 0 || cursor > chars.len() || chars[cursor - 1] != ':' {
        return None;
    }
    let (colon, code) = open_code(&chars, cursor - 1)?;
    let emoji = lookup(&code).filter(|_| !code.is_empty())?;
    let mut out: String = chars[..colon].iter().collect();
    out.push_str(emoji);
    let after = colon + emoji.chars().count();
    out.extend(&chars[cursor..]);
    Some((out, after))
}

/// Every known `:code:` of `text` replaced by its emoji, by the same rules
/// as the live conversion (what is sent: chat, IMs, gestures).
pub fn expand(text: &str) -> String {
    if !text.contains(':') {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':'
            && let Some((colon, code)) = open_code(&chars, i)
            && !code.is_empty()
            && let Some(emoji) = lookup(&code)
        {
            let emoji: Vec<char> = emoji.chars().collect();
            let len = emoji.len();
            chars.splice(colon..=i, emoji);
            i = colon + len;
            continue;
        }
        i += 1;
    }
    chars.into_iter().collect()
}

/// Up to `max` emoji whose code starts with `prefix` (then contains it),
/// shortest codes first: (code, emoji). The picker's names, skin tones and
/// emoji: those drawn as one picture (not flags nor sequences yet).
pub fn suggestions(prefix: &str, max: usize) -> Vec<(&'static str, &'static str)> {
    let prefix = prefix.to_ascii_lowercase();
    let mut found: Vec<(u8, &'static str, &'static str)> = Vec::new();
    for e in emojis::iter()
        .filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
        .filter(|e| super::single_char(e.as_str()).is_some())
    {
        let best = e
            .shortcodes()
            .filter_map(|s| {
                if s.starts_with(&prefix) {
                    Some((0, s))
                } else if s.contains(&prefix) {
                    Some((1, s))
                } else {
                    None
                }
            })
            .min_by_key(|(rank, s)| (*rank, s.len()));
        if let Some((rank, code)) = best {
            found.push((rank, code, e.as_str()));
        }
    }
    found.sort_by(|a, b| (a.0, a.1.len(), a.1).cmp(&(b.0, b.1.len(), b.1)));
    found.into_iter().take(max).map(|(_, code, emoji)| (code, emoji)).collect()
}

/// The list's room: how many suggestions it shows.
const SHOWN: usize = 8;
/// Letters after `:` before the list opens.
const MIN_LETTERS: usize = 2;

/// The list of one field, kept between frames.
#[derive(Clone, Default)]
struct List {
    /// The opening `:` and the letters it was made for.
    token: Option<(usize, String)>,
    items: Vec<(&'static str, &'static str)>,
    selected: usize,
    /// Escape closed the list for this token: it opens again when the
    /// letters change.
    dismissed: Option<(usize, String)>,
    /// The field had the keyboard in the last frame (egui takes it away on
    /// Escape before the frame's widgets see the key).
    had_focus: bool,
}

impl List {
    fn shown(&self) -> bool {
        !self.items.is_empty() && self.dismissed != self.token
    }
}

/// A key of the open list, taken before the field sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Up,
    Down,
    Pick,
    Dismiss,
}

fn list_id(field: egui::Id) -> egui::Id {
    field.with("emoji_codes")
}

/// Before the field `field` is drawn: while its list is shown, the arrows,
/// Tab, Enter and Escape are the list's (Enter does not send, Tab does not
/// move the keyboard). Nothing is taken while the list is not shown.
pub fn keys(ui: &egui::Ui, field: egui::Id) -> Option<Nav> {
    let list: List = ui.ctx().data(|d| d.get_temp(list_id(field)))?;
    if !list.shown() || !list.had_focus {
        return None;
    }
    let none = egui::Modifiers::NONE;
    let nav = ui.input_mut(|i| {
        if i.consume_key(none, egui::Key::ArrowUp) {
            Some(Nav::Up)
        } else if i.consume_key(none, egui::Key::ArrowDown) {
            Some(Nav::Down)
        } else if i.consume_key(none, egui::Key::Tab) || i.consume_key(none, egui::Key::Enter) {
            Some(Nav::Pick)
        } else if i.consume_key(none, egui::Key::Escape) {
            Some(Nav::Dismiss)
        } else {
            None
        }
    });
    if nav.is_some() {
        // the key was the list's: the field keeps the keyboard
        ui.ctx().memory_mut(|m| m.request_focus(field));
    }
    nav
}

/// After the field: a closing `:` of a known code becomes its emoji; the
/// list of the code being typed is updated, the key taken by [`keys`]
/// applied, and the list drawn above the field. `text` is the field's text;
/// without `emoji` (no colour font) the list shows the characters.
pub fn after(
    ui: &egui::Ui,
    p: &Palette,
    emoji: Option<&mut Emoji>,
    output: &mut egui::text_edit::TextEditOutput,
    text: &mut String,
    nav: Option<Nav>,
) {
    let field = output.response.id;
    let typed = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(s) if !s.is_empty())));
    let cursor = output.cursor_range.filter(|r| r.is_empty()).map(|r| r.primary.index.0);
    if typed
        && output.response.changed()
        && let Some(at) = cursor
        && let Some((replaced, after)) = close_code(text, at)
    {
        *text = replaced;
        set_cursor(ui, output, after);
    }
    let mut list: List = ui.ctx().data(|d| d.get_temp(list_id(field))).unwrap_or_default();
    let cursor = output.cursor_range.filter(|r| r.is_empty()).map(|r| r.primary.index.0);
    let chars: Vec<char> = text.chars().collect();
    let token = cursor
        .filter(|_| output.response.has_focus())
        .and_then(|at| open_code(&chars, at))
        .filter(|(_, letters)| letters.chars().count() >= MIN_LETTERS);
    if token != list.token {
        list.items = token.as_ref().map(|(_, letters)| suggestions(letters, SHOWN)).unwrap_or_default();
        list.selected = 0;
        list.token = token;
    }
    if list.token.is_none() {
        list.dismissed = None;
    }
    let mut pick = None;
    match nav {
        Some(Nav::Up) if !list.items.is_empty() => list.selected = (list.selected + list.items.len() - 1) % list.items.len(),
        Some(Nav::Down) if !list.items.is_empty() => list.selected = (list.selected + 1) % list.items.len(),
        Some(Nav::Pick) => pick = list.items.get(list.selected).map(|(_, e)| *e),
        Some(Nav::Dismiss) => list.dismissed = list.token.clone(),
        _ => {}
    }
    if list.shown() && pick.is_none() {
        pick = draw(ui, p, emoji, output.response.rect, &list);
    }
    if let (Some(e), Some((colon, _)), Some(at)) = (pick, list.token.clone(), cursor) {
        let mut out: String = chars[..colon].iter().collect();
        out.push_str(e);
        out.extend(&chars[at.min(chars.len())..]);
        *text = out;
        set_cursor(ui, output, colon + e.chars().count());
        output.response.request_focus();
        list = List::default();
    }
    list.had_focus = output.response.has_focus() || nav.is_some();
    ui.ctx().data_mut(|d| d.insert_temp(list_id(field), list));
}

/// The field's cursor at `at` (a char index), for this frame and the next.
fn set_cursor(ui: &egui::Ui, output: &mut egui::text_edit::TextEditOutput, at: usize) {
    let range = egui::text::CCursorRange::one(egui::text::CCursor::new(at));
    output.cursor_range = Some(range);
    output.state.cursor.set_char_range(Some(range));
    output.state.clone().store(ui.ctx(), output.response.id);
    output.response.mark_changed();
    ui.ctx().request_repaint();
}

/// The list above the field `rect`, its rows like a menu's; the emoji
/// clicked, if any.
fn draw(ui: &egui::Ui, p: &Palette, mut emoji: Option<&mut Emoji>, rect: egui::Rect, list: &List) -> Option<&'static str> {
    let mut picked = None;
    egui::Area::new(egui::Id::unique("emoji_codes_list"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(rect.left_top() - egui::vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(220.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, (code, e)) in list.items.iter().enumerate() {
                    let picture = super::single_char(e)
                        .zip(emoji.as_deref_mut())
                        .and_then(|(c, emoji)| emoji.texture(ui.ctx(), c));
                    let label = format!(":{code}:");
                    if crate::ui::menu::picture_row(ui, p, picture.as_ref(), e, &label, "", i == list.selected).clicked() {
                        picked = Some(*e);
                    }
                }
            });
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn a_known_code_is_replaced_when_it_closes() {
        let text = "I :heart:";
        assert_eq!(close_code(text, 9), Some(("I ❤️".to_owned(), 2 + "❤️".chars().count())));
        let (text, at) = close_code(":+1: ok", 4).expect("thumbs up");
        assert_eq!(text, "👍 ok");
        assert_eq!(at, 1);
        // in the middle of a line, the rest kept after the emoji
        let (text, at) = close_code("a :smile: b", 9).expect("smile");
        assert_eq!(text, "a 😄 b");
        assert_eq!(at, 3);
    }

    #[test]
    fn unknown_codes_times_words_and_links_stay_as_typed() {
        assert_eq!(close_code(":notacode:", 10), None);
        assert_eq!(close_code("at 12:30:", 9), None);
        assert_eq!(close_code("word:smile:", 11), None);
        assert_eq!(close_code("https://example.com/:smile:", 27), None);
        assert_eq!(
            close_code("www.example.com/x :smile:", 25).map(|r| r.0),
            Some("www.example.com/x 😄".to_owned())
        );
        assert_eq!(close_code("::", 2), None);
        // the cursor not after a ':'
        assert_eq!(close_code(":smile:", 6), None);
    }

    #[test]
    fn the_code_being_typed_is_found_after_a_boundary() {
        assert_eq!(open_code(&chars("hi :he"), 6), Some((3, "he".to_owned())));
        assert_eq!(open_code(&chars("(:he"), 4), Some((1, "he".to_owned())));
        assert_eq!(open_code(&chars(":"), 1), Some((0, String::new())));
        assert_eq!(open_code(&chars("12:30"), 5), None);
        assert_eq!(open_code(&chars("http://a :he"), 12), Some((9, "he".to_owned())));
        assert_eq!(open_code(&chars("http://a:he"), 11), None);
    }

    #[test]
    fn what_is_sent_is_expanded_by_the_same_rules() {
        assert_eq!(expand("hello :wave: :smile:!"), "hello 👋 😄!");
        assert_eq!(expand("at 12:30:45"), "at 12:30:45");
        assert_eq!(expand("see https://x.org/:smile:"), "see https://x.org/:smile:");
        assert_eq!(expand(":nope: :joy:"), ":nope: 😂");
        assert_eq!(expand("no colons"), "no colons");
    }

    struct Field {
        frames: crate::ui::widgets::elide_tests::Frames,
        text: String,
    }

    impl Field {
        const ID: &'static str = "codes_field";

        fn new() -> Field {
            let mut f = Field {
                frames: crate::ui::widgets::elide_tests::Frames::new(),
                text: String::new(),
            };
            f.frames.ctx.memory_mut(|m| m.request_focus(egui::Id::unique(Self::ID)));
            f.frame(vec![]);
            f.frame(vec![]);
            f
        }

        /// One frame; whether Enter reached what is drawn after the field.
        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            let p = crate::theme::Theme::default().palette();
            let mut enter_after = false;
            let text = &mut self.text;
            self.frames.run(events, |ui| {
                let id = egui::Id::unique(Self::ID);
                let nav = keys(ui, id);
                let mut output = egui::TextEdit::singleline(text).id(id).show(ui);
                after(ui, &p, None, &mut output, text, nav);
                enter_after = ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            enter_after
        }

        fn key(&mut self, key: egui::Key) -> bool {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }])
        }

        fn type_text(&mut self, s: &str) {
            for c in s.chars() {
                self.frame(vec![egui::Event::Text(c.to_string())]);
            }
        }

        fn list(&self) -> List {
            self.frames
                .ctx
                .data(|d| d.get_temp(list_id(egui::Id::unique(Self::ID))))
                .unwrap_or_default()
        }
    }

    #[test]
    fn the_list_opens_after_two_letters_and_the_keys_pick_from_it() {
        let mut f = Field::new();
        f.type_text("hi :h");
        assert!(!f.list().shown(), "one letter: no list");
        f.type_text("ea");
        let list = f.list();
        assert!(list.shown());
        assert_eq!(list.items[0].0, "heart");
        // the arrows move the choice, Enter takes it and is not sent
        f.key(egui::Key::ArrowDown);
        assert_eq!(f.list().selected, 1);
        f.key(egui::Key::ArrowUp);
        assert_eq!(f.list().selected, 0);
        assert!(!f.key(egui::Key::Enter), "Enter picked from the list must not reach the send");
        assert_eq!(f.text, "hi ❤️");
        assert!(!f.list().shown());
        // with no list, Enter is the field's as before
        assert!(f.key(egui::Key::Enter));
    }

    #[test]
    fn escape_closes_the_list_until_the_letters_change_and_the_colon_converts() {
        let mut f = Field::new();
        f.type_text(":smi");
        assert!(f.list().shown());
        f.key(egui::Key::Escape);
        assert!(!f.list().shown());
        assert_eq!(f.text, ":smi");
        f.type_text("l");
        assert!(f.list().shown(), "a new letter opens the list again");
        f.type_text("e:");
        assert_eq!(f.text, "😄");
        assert!(!f.list().shown());
        // a time or an unknown code stays as typed
        let mut f = Field::new();
        f.type_text("at 12:30: and :nope:");
        assert_eq!(f.text, "at 12:30: and :nope:");
    }

    #[test]
    fn suggestions_start_with_the_letters_shortest_first() {
        let list = suggestions("hea", 8);
        assert!(!list.is_empty() && list.len() <= 8);
        assert_eq!(list[0], ("heart", "❤️"));
        assert!(list.iter().take_while(|(c, _)| c.starts_with("hea")).count() >= 2);
        assert!(suggestions("zzzz", 8).is_empty());
        assert!(suggestions("thumb", 8).iter().any(|(_, e)| *e == "👍"));
        // flags are sequences the fields cannot draw yet
        assert!(!suggestions("thu", 8).iter().any(|(code, _)| *code == "lithuania"));
    }
}
