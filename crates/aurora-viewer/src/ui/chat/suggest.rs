//! Suggestions of the chat fields (#980): the residents to mention after
//! `@`, the gestures after `/` and the names of the residents around.
//!
//! - `@` at the start of a word lists the residents of the conversation (the
//!   nearby chat: those within chat range; an IM: who it is with; a group
//!   chat or a conference: its participants), like LLChatMentionHelper and
//!   LLFloaterChatMentionPicker (indra/llui/llchatmentionhelper.cpp,
//!   indra/newview/llfloaterchatmentionpicker.cpp, originally LGPL 2.1): the
//!   arrows choose, Tab or Enter take, Escape closes. The field shows
//!   `@Name`; what is sent is the mention link
//!   `secondlife:///app/agent/<id>/mention` that the Legacy Viewer's editor
//!   draws as that name ([`Shared::expand`]).
//! - `/` and a letter in the nearby chat list the active gestures whose
//!   trigger starts so, under the inline completion of `completion.rs`. The
//!   Legacy Viewer completes inline only; the list is Aurora's (#980), so
//!   that the gestures can be found: the arrows write the chosen trigger in
//!   the field (Enter sends it, as the inline completion), Tab keeps it,
//!   Escape closes the list.
//! - With the option (FSChatbarNamePrediction; on, the human's choice of
//!   2026-10-26, where the Legacy Viewer leaves it off), the nearby chat
//!   completes the user name of a resident around after three letters
//!   (FSNearbyChat::handleChatBarKeystroke, FIRE-3192,
//!   indra/newview/fsnearbychathub.cpp, originally LGPL 2.1).

use crate::gestures::Manager;
use crate::theme::Palette;
use crate::world::World;
use egui::text::{CCursor, CCursorRange};
use std::collections::HashMap;
use uuid::Uuid;

/// NearMeRange: the residents whose name the nearby chat predicts.
const NEAR_ME_RANGE: f32 = 162.0;
/// The nearby chat's participants for `@` (FSNearbyChatControl, the say
/// range).
const SAY_RANGE: f32 = crate::world::radar::CHAT_RANGE;
/// NAME_PREDICTION_MINIMUM_LENGTH.
const PREDICTION_LETTERS: usize = 3;
/// Letters after `/` before the gestures' list (the field holds two).
const GESTURE_CHARS: usize = 2;
/// The list's room.
const SHOWN: usize = 8;
/// How often the residents around are taken again while a field wants them.
const REFRESH_SECS: f64 = 0.5;
/// The `@Name` labels remembered per conversation.
const LABELS: usize = 32;

/// What the fields complete (Preferences › Chat), given every frame by the
/// app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub gestures: bool,
    pub names: bool,
    pub mentions: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            gestures: true,
            names: true,
            mentions: true,
        }
    }
}

/// A resident a field can name.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub id: Uuid,
    /// The name the viewer shows (display name, or the user name without
    /// them).
    pub display: String,
    /// LLAvatarName::getUserName: "First Last", or "First" for a Resident.
    pub user: String,
}

/// The residents of a conversation's `@`, taken only while its list is
/// open.
pub type People<'a> = &'a dyn Fn() -> Vec<Person>;

/// Which field: the nearby chat's (its gestures and the residents around)
/// or a conversation's (who `@` can name).
#[derive(Clone, Copy)]
pub enum Scope<'a> {
    Nearby(&'a Manager),
    Session(Uuid, People<'a>),
}

impl Scope<'_> {
    fn conversation(&self) -> Option<Uuid> {
        match self {
            Scope::Nearby(_) => None,
            Scope::Session(id, _) => Some(*id),
        }
    }
}

/// Shared by the chat fields (in `ChatUi`): the options, the residents
/// around, filled by the app while a nearby field wants them, and the
/// `@Name` written in each conversation.
#[derive(Default)]
pub struct Shared {
    pub options: Options,
    /// (resident, distance), nearest first.
    nearby: Vec<(Person, f32)>,
    /// A nearby field asked for them this frame.
    wanted: bool,
    /// When they were taken (egui time).
    taken: Option<f64>,
    /// Counts the takes: an open list of the nearby chat follows them.
    generation: u64,
    /// The mentions written in each conversation's field (None = nearby
    /// chat): their label and resident, the latest last.
    labels: HashMap<Option<Uuid>, Vec<(String, Uuid)>>,
}

impl Shared {
    /// The residents around are to be taken again (`set_nearby`): a nearby
    /// field wanted them in the last frame.
    pub fn wants_nearby(&mut self, now: f64) -> bool {
        std::mem::take(&mut self.wanted) && self.taken.is_none_or(|t| now - t >= REFRESH_SECS || now < t)
    }

    pub fn set_nearby(&mut self, people: Vec<(Person, f32)>, now: f64) {
        self.nearby = people;
        self.taken = Some(now);
        self.generation += 1;
    }

    fn remember(&mut self, conversation: Option<Uuid>, label: String, id: Uuid) {
        let labels = self.labels.entry(conversation).or_default();
        labels.retain(|(l, _)| *l != label);
        labels.push((label, id));
        if labels.len() > LABELS {
            labels.remove(0);
        }
    }

    /// What is sent: each `@Name` chosen in this conversation's list becomes
    /// its mention link (not inside a longer word).
    pub fn expand(&self, conversation: Option<Uuid>, text: &str) -> String {
        match self.labels.get(&conversation) {
            Some(labels) if text.contains('@') => expand_labels(text, labels),
            _ => text.to_owned(),
        }
    }
}

/// The mention link of a resident.
fn mention_link(id: Uuid) -> String {
    format!("secondlife:///app/agent/{id}/mention")
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn expand_labels(text: &str, labels: &[(String, Uuid)]) -> String {
    // the longest first: "@Jane Doe" before "@Jane"; the latest label of a
    // name wins
    let mut order: Vec<&(String, Uuid)> = labels.iter().rev().collect();
    order.sort_by_key(|(l, _)| std::cmp::Reverse(l.chars().count()));
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !word_char(chars[i - 1])) {
            for (label, id) in &order {
                let n = label.chars().count();
                if chars.len() >= i + n
                    && chars[i..i + n].iter().copied().eq(label.chars())
                    && chars.get(i + n).is_none_or(|c| !word_char(*c))
                {
                    out.push_str(&mention_link(*id));
                    i += n;
                    continue 'scan;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A resident's names, None while they are not known.
pub fn person(world: &World, id: Uuid) -> Option<Person> {
    if id.is_nil() || id == world.agent_id || !world.rlva_can_name(id) {
        return None;
    }
    let names = &world.social.avatar_names;
    if let Some(n) = names.get(&id) {
        return Some(Person {
            id,
            display: n.display(&names.options),
            user: n.user_name(),
        });
    }
    // the name of its object while the name service has not answered
    let name = world.avatar_name(&id).or_else(|| world.social.names.get(&id).cloned())?;
    Some(Person {
        id,
        display: name.clone(),
        user: name.strip_suffix(" Resident").map(str::to_owned).unwrap_or(name),
    })
}

/// The residents around, nearest first, for the nearby chat (`set_nearby`).
pub fn nearby(world: &World) -> Vec<(Person, f32)> {
    crate::ui::people::nearby(world)
        .into_iter()
        .filter(|(_, _, _, d)| *d <= NEAR_ME_RANGE)
        .filter_map(|(id, _, _, d)| person(world, id).map(|p| (p, d)))
        .collect()
}

/// Who a conversation's `@` can name, by name: who an IM is with, the
/// participants of a group chat or a conference.
pub fn members(world: &World, session: Uuid) -> Vec<Person> {
    let mut out: Vec<Person> = match world.groups.sessions.get(&session) {
        Some(s) => s.participants.iter().filter_map(|id| person(world, *id)).collect(),
        None if world.is_group_session(&session) => Vec::new(),
        None => person(world, session).into_iter().collect(),
    };
    out.sort_by_cached_key(|p| p.display.to_lowercase());
    out
}

/// The mention being typed before `cursor` (a char index): the index of its
/// `@` at the start of a word and the letters after it
/// (LLChatMentionHelper::isCursorInNameMention).
fn mention_at(chars: &[char], cursor: usize) -> Option<(usize, String)> {
    if cursor == 0 || cursor > chars.len() {
        return None;
    }
    let start = chars[..cursor].iter().rposition(|c| c.is_whitespace()).map_or(0, |i| i + 1);
    (start < cursor && chars.get(start) == Some(&'@')).then(|| (start, chars[start + 1..cursor].iter().collect()))
}

/// The residents whose display or user name starts with `typed` (or one of
/// their words does, then contains it), in the order given.
fn mention_matches(people: &[Person], typed: &str, max: usize) -> Vec<Person> {
    let typed = typed.to_lowercase();
    let rank = |p: &Person| {
        let names = [p.display.to_lowercase(), p.user.to_lowercase()];
        if names.iter().any(|n| n.starts_with(&typed)) {
            Some(0)
        } else if names.iter().any(|n| n.split([' ', '.', '_']).any(|w| w.starts_with(&typed))) {
            Some(1)
        } else if names.iter().any(|n| n.contains(&typed)) {
            Some(2)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, usize, &Person)> = people.iter().enumerate().filter_map(|(i, p)| rank(p).map(|r| (r, i, p))).collect();
    found.sort_by_key(|(r, i, _)| (*r, *i));
    let mut out: Vec<Person> = Vec::new();
    for (_, _, p) in found {
        if !out.iter().any(|o| o.id == p.id) {
            out.push(p.clone());
        }
        if out.len() == max {
            break;
        }
    }
    out
}

/// FSNearbyChat::handleChatBarKeystroke's name prediction: the word before
/// `cursor` (three letters at least), or the two last words, start the user
/// name of a resident around; the rest of their first name (their last name
/// after two words) and a space follow what was typed, selected, so that the
/// next letter replaces them. The text, then the selection (char indices).
fn predict(text: &str, cursor: usize, people: &[(Person, f32)]) -> Option<(String, usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let length = chars.iter().skip_while(|c| c.is_whitespace()).count();
    if length <= PREDICTION_LETTERS || cursor == 0 || cursor > chars.len() || chars[cursor - 1] == ' ' {
        return None;
    }
    let before = &chars[..cursor];
    let last_space = before.iter().rposition(|c| *c == ' ');
    let word_start = last_space.map_or(0, |i| i + 1);
    let pattern: String = before[word_start..].iter().collect();
    if pattern.chars().count() < PREDICTION_LETTERS {
        return None;
    }
    let lower = |s: &str| s.to_lowercase();
    let starts = |p: &Person, with: &str| lower(&p.user).starts_with(with);
    // the two last words: "jane do" → "Jane Doe"
    let previous = last_space.filter(|&s| s > 0).map(|s| {
        let start = before[..s].iter().rposition(|c| *c == ' ').map_or(0, |i| i + 1);
        before[start..word_start].iter().collect::<String>()
    });
    let full = previous
        .map(|prev| lower(&(prev + &pattern)))
        .and_then(|full| people.iter().find(|(p, _)| starts(p, &full)));
    let (found, last) = match full {
        Some((p, _)) => (p, true),
        None => (&people.iter().find(|(p, _)| starts(p, &lower(&pattern)))?.0, false),
    };
    let (first_name, last_name) = found.user.split_once(' ').unwrap_or((found.user.as_str(), ""));
    let name = if last { last_name } else { first_name };
    let rest: String = name.chars().skip(pattern.chars().count()).collect();
    if rest.is_empty() {
        return None;
    }
    let mut out: String = before.iter().collect();
    let from = cursor;
    out.push_str(&rest);
    out.push(' ');
    let to = from + rest.chars().count() + 1;
    out.extend(&chars[cursor..]);
    Some((out, from, to))
}

/// The field's selection from `from` to `to` (char indices), for this frame
/// and the next.
fn select(ui: &egui::Ui, output: &mut egui::text_edit::TextEditOutput, from: usize, to: usize) {
    let range = CCursorRange::two(CCursor::new(from), CCursor::new(to));
    output.cursor_range = Some(range);
    output.state.cursor.set_char_range(Some(range));
    output.state.clone().store(ui.ctx(), output.response.id);
    output.response.mark_changed();
    ui.ctx().request_repaint();
}

/// What a list is open for.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// `@` at this char index and the letters after it.
    Mention(usize, String),
    /// The beginning of the field typed (the rest is the inline completion).
    Gesture(String),
}

#[derive(Clone, Debug)]
enum Item {
    Person(Person),
    /// A trigger and what the gesture says instead.
    Gesture(String, String),
}

/// The list of one field, kept between frames.
#[derive(Clone, Default)]
struct List {
    token: Option<Token>,
    items: Vec<Item>,
    /// The highlighted row: the first resident, or the gesture written in
    /// the field (none while it holds only the common beginning).
    selected: Option<usize>,
    /// Escape closed the list for this token.
    dismissed: Option<Token>,
    /// The residents around it was made from (`Shared::generation`).
    generation: u64,
    /// The field had the keyboard in the last frame (egui takes it away on
    /// Escape before the frame's widgets see the key).
    had_focus: bool,
}

impl List {
    fn shown(&self) -> bool {
        !self.items.is_empty() && self.dismissed != self.token
    }

    fn gestures(&self) -> bool {
        matches!(self.token, Some(Token::Gesture(_)))
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
    field.with("chat_suggest")
}

/// Before the field `field` is drawn: while its list is shown, the arrows,
/// Tab and Escape are the list's, and Enter too for a resident's list (the
/// gestures' Enter sends the trigger written in the field). Nothing is taken
/// while the list is not shown.
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
        } else if i.consume_key(none, egui::Key::Tab) || (!list.gestures() && i.consume_key(none, egui::Key::Enter)) {
            Some(Nav::Pick)
        } else if i.consume_key(none, egui::Key::Escape) {
            Some(Nav::Dismiss)
        } else {
            None
        }
    });
    if nav.is_some() {
        // the key was the list's: the field keeps the keyboard, and egui
        // does not move it on (Tab, the arrows) at the end of the frame
        ui.ctx().memory_mut(|m| {
            m.request_focus(field);
            m.move_focus(egui::FocusDirection::None);
        });
    }
    nav
}

/// A letter was typed in this frame (not a key, a paste or a recall).
fn typed(ui: &egui::Ui) -> bool {
    ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(s) if !s.is_empty())))
}

/// After the field (and its emoji codes): the inline completions of the
/// nearby chat (gesture, name), then the list of the token before the cursor
/// updated, the key taken by [`keys`] applied, and the list drawn above the
/// field.
pub fn after(
    ui: &egui::Ui,
    p: &Palette,
    output: &mut egui::text_edit::TextEditOutput,
    text: &mut String,
    nav: Option<Nav>,
    shared: &mut Shared,
    scope: Scope<'_>,
) {
    let field = output.response.id;
    let options = shared.options;
    let focused = output.response.has_focus();
    if let Scope::Nearby(gestures) = scope {
        let length = text.len();
        if options.gestures {
            super::completion::complete(ui, output, text, gestures);
        }
        if options.names && focused {
            shared.wanted = true;
            // not over a gesture's completion (it only adds letters)
            if text.len() == length
                && typed(ui)
                && output.response.changed()
                && let Some(at) = output.cursor_range.filter(|r| r.is_empty()).map(|r| r.primary.index.0)
                && let Some((completed, from, to)) = predict(text, at, &shared.nearby)
            {
                *text = completed;
                select(ui, output, from, to);
            }
        }
    }
    let mut list: List = ui.ctx().data(|d| d.get_temp(list_id(field))).unwrap_or_default();
    let chars: Vec<char> = text.chars().collect();
    let range = output.cursor_range.map(|r| {
        let (a, b) = (r.primary.index.0, r.secondary.index.0);
        (a.min(b), a.max(b))
    });
    let mut token = None;
    if focused && let Some((from, to)) = range {
        if options.mentions
            && from == to
            && let Some((at, letters)) = mention_at(&chars, to)
        {
            token = Some(Token::Mention(at, letters));
        } else if options.gestures
            && matches!(scope, Scope::Nearby(_))
            && to == chars.len()
            && chars.first() == Some(&'/')
            && from >= GESTURE_CHARS
        {
            token = Some(Token::Gesture(chars[..from].iter().collect()));
        }
    }
    if matches!(token, Some(Token::Mention(..))) && matches!(scope, Scope::Nearby(_)) {
        shared.wanted = true;
    }
    // the residents around arrive a frame after the nearby field asked, then
    // follow them; the highlighted one stays while they are there
    let refreshed = matches!((&token, scope), (Some(Token::Mention(..)), Scope::Nearby(_))) && list.generation != shared.generation;
    if token != list.token || refreshed {
        let kept = match list.selected.and_then(|i| list.items.get(i)) {
            Some(Item::Person(p)) if token == list.token => Some(p.id),
            _ => None,
        };
        list.generation = shared.generation;
        list.items = match (&token, scope) {
            (Some(Token::Mention(_, letters)), Scope::Nearby(_)) => {
                let around: Vec<Person> = shared
                    .nearby
                    .iter()
                    .filter(|(_, d)| *d <= SAY_RANGE)
                    .map(|(p, _)| p.clone())
                    .collect();
                mention_matches(&around, letters, SHOWN).into_iter().map(Item::Person).collect()
            }
            (Some(Token::Mention(_, letters)), Scope::Session(_, people)) => {
                mention_matches(&people(), letters, SHOWN).into_iter().map(Item::Person).collect()
            }
            (Some(Token::Gesture(typed)), Scope::Nearby(gestures)) => gestures
                .matching(typed)
                .into_iter()
                .take(SHOWN)
                .map(|(t, says)| Item::Gesture(t, says))
                .collect(),
            _ => Vec::new(),
        };
        list.selected = kept.and_then(|id| list.items.iter().position(|i| matches!(i, Item::Person(p) if p.id == id)));
        list.token = token;
    }
    if list.token.is_none() {
        list.dismissed = None;
    }
    // the gesture written in the field is the highlighted one
    list.selected = match &list.token {
        Some(Token::Gesture(_)) => list
            .items
            .iter()
            .position(|i| matches!(i, Item::Gesture(t, _) if t.to_lowercase() == text.to_lowercase())),
        Some(Token::Mention(..)) if !list.items.is_empty() => Some(list.selected.unwrap_or(0).min(list.items.len() - 1)),
        _ => None,
    };
    let n = list.items.len();
    let mut pick = None;
    let mut choose = None;
    match nav {
        Some(Nav::Up) if n > 0 => choose = Some(list.selected.map_or(n - 1, |s| (s + n - 1) % n)),
        Some(Nav::Down) if n > 0 => choose = Some(list.selected.map_or(0, |s| (s + 1) % n)),
        Some(Nav::Pick) => pick = list.selected.or((n > 0).then_some(0)),
        Some(Nav::Dismiss) => list.dismissed = list.token.clone(),
        _ => {}
    }
    match (choose, list.token.clone()) {
        // the arrows: a resident is highlighted, a gesture written
        (Some(i), Some(Token::Mention(..))) => list.selected = Some(i),
        (Some(i), Some(Token::Gesture(typed))) => {
            if let Some(Item::Gesture(trigger, _)) = list.items.get(i) {
                let kept = typed.chars().count();
                *text = typed.chars().chain(trigger.chars().skip(kept)).collect();
                select(ui, output, kept, text.chars().count());
                list.selected = Some(i);
            }
        }
        _ => {}
    }
    if list.shown() && pick.is_none() {
        pick = draw(ui, p, output.response.rect, &list);
    }
    if let (Some(i), Some(token)) = (pick, list.token.clone()) {
        match (list.items.get(i).cloned(), token) {
            (Some(Item::Person(person)), Token::Mention(at, _)) => {
                let cursor = range.map_or(chars.len(), |(_, to)| to);
                let label = format!("@{}", person.display);
                let mut out: String = chars[..at].iter().collect();
                out.push_str(&label);
                let spaced = chars.get(cursor).is_some_and(|c| c.is_whitespace());
                if !spaced {
                    out.push(' ');
                }
                let after = at + label.chars().count() + 1;
                out.extend(&chars[cursor.min(chars.len())..]);
                *text = out;
                select(ui, output, after, after);
                shared.remember(scope.conversation(), label, person.id);
                list = List::default();
            }
            (Some(Item::Gesture(trigger, _)), Token::Gesture(typed)) => {
                let kept = typed.chars().count();
                *text = typed.chars().chain(trigger.chars().skip(kept)).collect();
                let end = text.chars().count();
                select(ui, output, end, end);
                // closed until the field changes
                list.token = Some(Token::Gesture(text.clone()));
                list.dismissed = list.token.clone();
            }
            _ => {}
        }
        output.response.request_focus();
    }
    list.had_focus = output.response.has_focus() || nav.is_some();
    ui.ctx().data_mut(|d| d.insert_temp(list_id(field), list));
}

/// A gesture's text in its row: its first line, shortened.
fn says(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 32 {
        line.chars().take(31).chain(std::iter::once('…')).collect()
    } else {
        line.to_owned()
    }
}

/// The list above the field `rect`, its rows like a menu's; the row
/// clicked, if any.
fn draw(ui: &egui::Ui, p: &Palette, rect: egui::Rect, list: &List) -> Option<usize> {
    let mut picked = None;
    egui::Area::new(egui::Id::unique("chat_suggest_list"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(rect.left_top() - egui::vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(240.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, item) in list.items.iter().enumerate() {
                    let selected = list.selected == Some(i);
                    let clicked = match item {
                        Item::Person(person) => {
                            // the user name when it says more than the display name
                            let detail = if person.user.eq_ignore_ascii_case(&person.display) {
                                ""
                            } else {
                                &person.user
                            };
                            crate::ui::menu::suggestion_row(ui, p, "user-filled", &person.display, detail, selected)
                        }
                        Item::Gesture(trigger, text) => crate::ui::menu::suggestion_row(ui, p, "hand-move", trigger, &says(text), selected),
                    };
                    if clicked.clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
    picked
}

#[cfg(test)]
#[path = "suggest_tests.rs"]
mod tests;
