//! Right-click menu of every text field, like Firestorm's text editors
//! (LLLineEditor / LLTextEditor::showContextMenu and menu_text_editor.xml,
//! indra/llui, originally LGPL 2.1): Cut, Copy, Paste, Delete and Select
//! All, greyed out by the rules of canCut / canCopy / canPaste /
//! canDoDelete; above them, on a misspelled word of a spell-checked field
//! (`ui/spelling.rs`), up to five suggestions, Add to Dictionary and Add to
//! Ignore (LLSpellCheckMenuHandler, the "SpellCheck.*" entries of
//! llviewermenu.cpp).
//!
//! egui's `TextEdit` has no menu of its own, and the viewer has well over a
//! hundred of them: one hook around the interface pass covers them all
//! without touching their call sites. `before` runs on the raw input of the
//! frame, `after` at the end of the pass:
//! - the field is the focused one with a `TextEditState` under the right
//!   click (egui focuses a field on any press);
//! - whether it is editable or a password comes from the IME request egui
//!   makes for a focused, editable field (none for a read-only one);
//! - a chosen entry is played into the field at the next frame as the
//!   keyboard would (egui's Cut / Copy events, the clipboard's text as a
//!   Paste, Delete, Ctrl+A), so each field keeps its own rules (password,
//!   single line, character limit); a suggestion selects its word and is
//!   pasted over it (LLLineEditor::replaceWithSuggestion);
//! - a field with a right-click menu of its own (script editor, notecards)
//!   keeps it.

use crate::theme::Palette;
use egui::text::CCursorRange;
use egui::text_edit::TextEditState;
use egui::{Context, Event, IMEPurpose, Id, Key, Modifiers, PointerButton, Pos2, RawInput};

/// The entries, in Firestorm's order (menu_text_editor.xml).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

const ENTRIES: [(Entry, &str); 5] = [
    (Entry::Cut, "cut"),
    (Entry::Copy, "copy-filled"),
    (Entry::Paste, "clipboard-text-filled"),
    (Entry::Delete, "trash-filled"),
    (Entry::SelectAll, "select-all"),
];

fn label(entry: Entry) -> &'static str {
    match entry {
        Entry::Cut => aurora_i18n::tr!("common-cut"),
        Entry::Copy => aurora_i18n::tr!("common-copy-2"),
        Entry::Paste => aurora_i18n::tr!("common-paste"),
        Entry::Delete => aurora_i18n::tr!("common-delete"),
        Entry::SelectAll => aurora_i18n::tr!("common-select-all"),
    }
}

/// What the right-clicked field allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Field {
    editable: bool,
    password: bool,
    selection: bool,
}

/// LLLineEditor::canCut, canCopy, canPaste (text in the clipboard),
/// canDoDelete (Delete without a selection removes the next character) and
/// canSelectAll.
fn enabled(entry: Entry, field: Field, clipboard: bool) -> bool {
    match entry {
        Entry::Cut => field.editable && !field.password && field.selection,
        Entry::Copy => !field.password && field.selection,
        Entry::Paste => field.editable && clipboard,
        Entry::Delete => field.editable,
        Entry::SelectAll => true,
    }
}

/// egui collapses the selection on any press; Firestorm keeps the one
/// right-clicked inside, its cursor at its end, and otherwise leaves the
/// cursor where the click put it (LLLineEditor::showContextMenu).
fn after_right_press(before: CCursorRange, now: CCursorRange) -> CCursorRange {
    if !before.is_empty() && now.is_empty() && before.contains(now) {
        let [start, end] = before.sorted_cursors();
        CCursorRange::two(start, end)
    } else {
        now
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// The events that run `entry` in the focused field.
fn events(entry: Entry, clipboard: Option<String>) -> Vec<Event> {
    match entry {
        Entry::Cut => vec![Event::Cut],
        Entry::Copy => vec![Event::Copy],
        Entry::Paste => clipboard.filter(|text| !text.is_empty()).map(Event::Paste).into_iter().collect(),
        Entry::Delete => vec![key(Key::Delete, Modifiers::NONE)],
        Entry::SelectAll => vec![key(Key::A, Modifiers::COMMAND)],
    }
}

/// The misspelled word under the right click: its characters and itself.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Misspelled {
    range: std::ops::Range<usize>,
    word: String,
}

/// What runs in the field at the next frame.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Entry(Entry),
    /// The word replaced by a suggestion.
    Replace(Misspelled, String),
}

struct Open {
    field: Id,
    pos: Pos2,
    can: Field,
    clipboard: bool,
    spell: Option<Misspelled>,
    /// One menu id per opening: a right click on another field while a
    /// menu is open opens the new one instead of closing both.
    serial: u64,
}

/// `AURORA_DEMO_TEXT_MENU`: the chat bar gets a line, selected, then the
/// menu over it.
fn demo_wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var_os("AURORA_DEMO_TEXT_MENU").is_some())
}

/// `AURORA_DEMO_SPELLING`: the app fills the chat bar and gives it the
/// keyboard (frame 700); then the menu opens on its last word, or keywords
/// are typed for the auto-replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpellingDemo {
    Words,
    Menu,
    Replace,
}

pub(crate) fn spelling_demo() -> Option<SpellingDemo> {
    static MODE: std::sync::OnceLock<Option<SpellingDemo>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_SPELLING").ok().map(|mode| match mode.trim() {
            "menu" => SpellingDemo::Menu,
            "replace" => SpellingDemo::Replace,
            _ => SpellingDemo::Words,
        })
    })
}

const DEMO_FOCUS: u64 = 700;
const DEMO_TYPE: u64 = 703;
const DEMO_TYPE_MORE: u64 = 705;
const DEMO_OPEN: u64 = 706;

#[derive(Default)]
pub(crate) struct TextMenu {
    /// The focused field's selection when the right button went down.
    pressed: Option<(Id, CCursorRange)>,
    /// Text was in the clipboard at the last right click.
    clipboard: bool,
    open: Option<Open>,
    serial: u64,
    /// The entry chosen last frame, played into its field this frame.
    pending: Option<(Id, Pending)>,
    demo_open: bool,
}

impl TextMenu {
    /// On the raw input, before the pass. `clipboard` reads the system
    /// clipboard (only on a right click or for Paste); `demo_frame` is the
    /// app's frame in demo mode.
    pub fn before(&mut self, ctx: &Context, raw: &mut RawInput, mut clipboard: impl FnMut() -> Option<String>, demo_frame: Option<u64>) {
        if let Some((field, pending)) = self.pending.take()
            && ctx.memory(|m| m.has_focus(field))
        {
            match pending {
                Pending::Entry(entry) => {
                    let text = if entry == Entry::Paste { clipboard() } else { None };
                    raw.events.extend(events(entry, text));
                }
                Pending::Replace(word, with) => replace(ctx, raw, field, &word, with),
            }
        }
        let right = |pressed: bool| {
            raw.events
                .iter()
                .any(|e| matches!(e, Event::PointerButton { button: PointerButton::Secondary, pressed: p, .. } if *p == pressed))
        };
        self.pressed = if right(true) {
            ctx.memory(|m| m.focused()).and_then(|field| {
                let range = TextEditState::load(ctx, field)?.cursor.char_range()?;
                (!range.is_empty()).then_some((field, range))
            })
        } else {
            None
        };
        if right(false) {
            self.clipboard = clipboard().is_some_and(|text| !text.is_empty());
        }
        if let Some(frame) = demo_frame {
            match (spelling_demo(), frame) {
                (Some(SpellingDemo::Menu), DEMO_OPEN) => {
                    self.clipboard = clipboard().is_some_and(|text| !text.is_empty());
                    self.demo_open = true;
                }
                (Some(SpellingDemo::Replace), DEMO_TYPE) => raw.events.push(Event::Text("brb ".into())),
                (Some(SpellingDemo::Replace), DEMO_TYPE_MORE) => raw.events.push(Event::Text("ty ".into())),
                _ => {}
            }
        }
        if let Some(frame) = demo_frame.filter(|_| demo_wanted()) {
            let chat = Id::unique("nearby_chat_input");
            match frame {
                DEMO_FOCUS => ctx.memory_mut(|m| m.request_focus(chat)),
                DEMO_TYPE => {
                    raw.events.push(Event::Text("Hello from the demo".into()));
                    raw.events.push(key(Key::A, Modifiers::COMMAND));
                }
                DEMO_OPEN => {
                    self.clipboard = clipboard().is_some_and(|text| !text.is_empty());
                    self.demo_open = true;
                }
                _ => {}
            }
        }
    }

    /// At the end of the pass: keeps the selection, opens the menu of a
    /// right-clicked field and shows it.
    pub fn after(&mut self, ctx: &Context, p: &Palette) {
        let focused = ctx.memory(|m| m.focused());
        if let Some((field, before)) = self.pressed.take()
            && focused == Some(field)
            && let Some(mut state) = TextEditState::load(ctx, field)
            && let Some(now) = state.cursor.char_range()
        {
            let kept = after_right_press(before, now);
            if kept != now {
                state.cursor.set_char_range(Some(kept));
                state.store(ctx, field);
            }
        }
        let clicked = ctx.input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten());
        let demo = std::mem::take(&mut self.demo_open)
            .then(|| {
                let field = Id::unique("nearby_chat_input");
                ctx.read_response(field).map(|r| (field, r.rect.center()))
            })
            .flatten();
        let target = match (clicked, focused) {
            (Some(pos), Some(field)) => Some((field, pos)),
            _ => demo,
        };
        if let Some((field, pos)) = target
            && let Some(can) = field_at(ctx, field, pos)
        {
            self.serial += 1;
            self.open = Some(Open {
                field,
                pos,
                can,
                clipboard: self.clipboard,
                spell: can.editable.then(|| misspelled_at_cursor(ctx, field)).flatten(),
                serial: self.serial,
            });
        }
        self.show(ctx, p);
    }

    fn show(&mut self, ctx: &Context, p: &Palette) {
        let Some(open) = &self.open else {
            return;
        };
        // the field went with its window
        if ctx.read_response(open.field).is_none() {
            self.open = None;
            return;
        }
        let (field, can, clipboard) = (open.field, open.can, open.clipboard);
        let spell = open.spell.clone();
        // the spelling thread may still be looking: the entries come as
        // soon as it has them
        let suggestions = spell.as_ref().map(|s| crate::spelling::suggestions(&s.word));
        if matches!(suggestions, Some(None)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let mut keep = true;
        let mut chosen = None;
        let mut done = false;
        super::menu::popup_at(ctx, Id::unique("text_field_menu").with(open.serial), open.pos, &mut keep, p, |ui| {
            if let Some(word) = &spell {
                match &suggestions {
                    Some(Some(list)) => {
                        for with in list {
                            if super::menu::item(ui, p, "replace-filled", with) {
                                chosen = Some(Pending::Replace(word.clone(), with.clone()));
                            }
                        }
                        // "Suggestion Separator", only with suggestions
                        if !list.is_empty() {
                            super::menu::separator(ui, p);
                        }
                    }
                    _ => {
                        super::menu::item_if(ui, p, "replace-filled", aurora_i18n::tr!("spelling-looking"), false);
                        super::menu::separator(ui, p);
                    }
                }
                if super::menu::item(ui, p, "book-filled", aurora_i18n::tr!("spelling-add-to-dictionary")) {
                    crate::spelling::add_to_dictionary(&word.word);
                    done = true;
                }
                if super::menu::item(ui, p, "eye-off", aurora_i18n::tr!("spelling-ignore")) {
                    crate::spelling::ignore(&word.word);
                    done = true;
                }
                super::menu::separator(ui, p);
            }
            for (entry, icon) in ENTRIES {
                if super::menu::item_if(ui, p, icon, label(entry), enabled(entry, can, clipboard)) {
                    chosen = Some(Pending::Entry(entry));
                }
            }
        });
        if let Some(pending) = chosen {
            // the click on the menu took the focus from the field: it gets
            // it back, and the entry runs there at the next frame
            ctx.memory_mut(|m| m.request_focus(field));
            self.pending = Some((field, pending));
            keep = false;
        } else if done {
            ctx.memory_mut(|m| m.request_focus(field));
            keep = false;
        }
        if !keep {
            self.open = None;
        }
    }
}

/// The misspelled word at the cursor of `field` (where the right click put
/// it, or the end of the selection it kept: LLLineEditor::showContextMenu).
fn misspelled_at_cursor(ctx: &Context, field: Id) -> Option<Misspelled> {
    let marked = super::spelling::marked(ctx, field)?;
    let range = TextEditState::load(ctx, field)?.cursor.char_range()?;
    let at = range.sorted_cursors()[1].index.0;
    crate::spelling::words::at(&marked, at).map(|(range, word)| Misspelled {
        range: range.clone(),
        word: word.clone(),
    })
}

/// A suggestion pasted over its word, selected first, if the field still
/// holds that misspelled word there (LLLineEditor::replaceWithSuggestion).
fn replace(ctx: &Context, raw: &mut RawInput, field: Id, word: &Misspelled, with: String) {
    let still = super::spelling::marked(ctx, field).is_some_and(|marked| marked.iter().any(|(r, w)| *r == word.range && *w == word.word));
    if !still || with.is_empty() {
        return;
    }
    let Some(mut state) = TextEditState::load(ctx, field) else {
        return;
    };
    state.cursor.set_char_range(Some(CCursorRange::two(
        egui::text::CCursor::new(word.range.start),
        egui::text::CCursor::new(word.range.end),
    )));
    state.store(ctx, field);
    raw.events.push(Event::Paste(with));
}

/// The text field `field` under `pos`, if the menu may open there.
fn field_at(ctx: &Context, field: Id, pos: Pos2) -> Option<Field> {
    let state = TextEditState::load(ctx, field)?;
    let response = ctx.read_response(field)?;
    if !response.interact_rect.contains(pos) || ctx.layer_id_at(pos) != Some(response.layer_id) {
        return None;
    }
    // the field's own right-click menu (`menu::context_menu`)
    if egui::Popup::is_id_open(ctx, egui::Popup::default_response_id(&response)) {
        return None;
    }
    // egui asks for the IME (with its purpose) only for a focused, editable field
    let purpose = ctx.output(|o| o.ime.map(|ime| ime.purpose));
    Some(Field {
        editable: purpose.is_some(),
        password: purpose == Some(IMEPurpose::Password),
        selection: state.cursor.char_range().is_some_and(|range| !range.is_empty()),
    })
}

#[cfg(test)]
mod tests;
