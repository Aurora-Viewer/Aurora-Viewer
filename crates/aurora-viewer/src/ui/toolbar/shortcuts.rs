//! Keyboard shortcuts of the toolbar's buttons, drawers and blocks (Aurora's
//! own, human request, 2026-11-25). A shortcut is Ctrl and / or Alt with a
//! key, or a function key alone or with Shift, saved with the toolbar
//! (`ToolbarSettings::shortcuts`): a button's under its command, a drawer's
//! or a floating block's under its id, a plain block's under
//! `block:<one of its buttons>`, which works once the block is a drawer or a
//! floating block (the key follows it through Fold, Float, Unfold and Dock).
//!
//! A shortcut does what a click does: the button drawn this frame is
//! clicked (`claim`), RLVa's locks and a dimmed button's rules included;
//! one not drawn (in a closed drawer, a hidden block, the interface hidden)
//! runs like its Aurora menu entry (`fallback`). The keys text fields, the
//! chat and the system keep, and those of Preferences › Shortcuts, are
//! refused (`refusal`); one a Preferences action has is taken over, the
//! window saying so (`conflict`).

use super::{Bar, Edit, ToolbarSettings, ToolbarUi, drawers, structure};
use crate::keybinds::{Binding, Input, KeyBindings};
use crate::theme::Palette;
use crate::ui::{menu, skin::Layout, skin::ToolButton, widgets};
use egui::{Id, Vec2};

/// The key of a plain block's shortcut.
pub const BLOCK: &str = "block:";

pub fn block_key(command: &str) -> String {
    format!("{BLOCK}{command}")
}

/// Whether a button can have a shortcut: not a gap, a separator, a bar's
/// own block (the clock, the address bar…) nor the "Floating block" tile.
pub fn takes(command: &str) -> bool {
    !command.is_empty()
        && !structure::is_separator(command)
        && !structure::is_divider(command)
        && !structure::is_block(command)
        && command != drawers::FLOATING
}

/// The key of the shortcut of the plain block holding `command` in
/// `buttons`: the one already saved under one of its buttons, else its
/// first button's.
pub fn block_of(settings: &ToolbarSettings, buttons: &[ToolButton], command: &str) -> Option<String> {
    let range = super::edit_mode::group(buttons, command)?;
    let members = &buttons[range];
    if members
        .iter()
        .any(|button| drawers::is_drawer(&button.command) || structure::is_block(&button.command))
    {
        return None;
    }
    let real = members.iter().filter(|button| takes(&button.command));
    let first = real.clone().next()?;
    Some(
        real.map(|button| block_key(&button.command))
            .find(|key| settings.shortcuts.contains_key(key))
            .unwrap_or_else(|| block_key(&first.command)),
    )
}

/// Why a key cannot be a toolbar shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A mouse button.
    Keyboard,
    /// A modifier or lock key alone.
    Modifier,
    /// A key alone or with Shift only: it types in text fields.
    NeedsModifier,
    /// Ctrl+Alt with a character key: AltGr on Windows, which types
    /// characters (€, @, ą…) on many layouts.
    AltGr,
    /// Select all, copy, cut, paste, undo, redo.
    Editing,
    /// The text fields' moves and erasing with Ctrl or Alt.
    Navigation,
    /// Enter, Tab and Escape: the chat's and the dialogs'.
    Typing,
    /// Kept by the operating system.
    System,
    /// Closes or switches windows.
    Window,
    /// The interface's zoom.
    Zoom,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Keyboard => aurora_i18n::tr!("toolbar-shortcut-refused-keyboard"),
            Refusal::Modifier => aurora_i18n::tr!("toolbar-shortcut-refused-modifier"),
            Refusal::NeedsModifier => aurora_i18n::tr!("toolbar-shortcut-refused-needs-modifier"),
            Refusal::AltGr => aurora_i18n::tr!("toolbar-shortcut-refused-altgr"),
            Refusal::Editing => aurora_i18n::tr!("toolbar-shortcut-refused-editing"),
            Refusal::Navigation => aurora_i18n::tr!("toolbar-shortcut-refused-navigation"),
            Refusal::Typing => aurora_i18n::tr!("toolbar-shortcut-refused-typing"),
            Refusal::System => aurora_i18n::tr!("toolbar-shortcut-refused-system"),
            Refusal::Window => aurora_i18n::tr!("toolbar-shortcut-refused-window"),
            Refusal::Zoom => aurora_i18n::tr!("toolbar-shortcut-refused-zoom"),
        }
    }
}

fn function_key(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

/// The letter a key types on the keyboard in use (`keybinds::learn`), else
/// on a QWERTY one.
fn letter(key: &str) -> Option<char> {
    let label = Input::Key(key.into()).label();
    let mut chars = label.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// The rule of the keys a toolbar shortcut may not take, whatever uses
/// them now (`conflict` says what a free one takes over).
pub fn refusal(b: &Binding) -> Option<Refusal> {
    let Input::Key(key) = &b.input else {
        return Some(Refusal::Keyboard);
    };
    let key = key.as_str();
    let (ctrl, alt) = (b.ctrl, b.alt);
    if matches!(
        key,
        "ControlLeft"
            | "ControlRight"
            | "AltLeft"
            | "AltRight"
            | "ShiftLeft"
            | "ShiftRight"
            | "SuperLeft"
            | "SuperRight"
            | "Meta"
            | "Hyper"
            | "Fn"
            | "FnLock"
            | "CapsLock"
            | "NumLock"
            | "ScrollLock"
            | "ContextMenu"
    ) {
        return Some(Refusal::Modifier);
    }
    // Print Screen, and Alt+F4, Alt+Tab, Alt+Esc, Alt+Space, Ctrl+Esc,
    // Ctrl+Shift+Esc, Ctrl+Alt+Delete, Ctrl+Space (the input method)
    if key == "PrintScreen"
        || (alt && !ctrl && matches!(key, "F4" | "Tab" | "Escape" | "Space"))
        || (ctrl && key == "Escape")
        || (ctrl && alt && key == "Delete")
        || ((ctrl || alt) && key == "Space")
    {
        return Some(Refusal::System);
    }
    // Alt+Enter is the viewer's full screen (app.rs, fullscreen_shortcut)
    if alt && !ctrl && matches!(key, "Enter" | "NumpadEnter") {
        return Some(Refusal::System);
    }
    // Ctrl+F4, Ctrl+Tab, Ctrl+Shift+Tab; Ctrl+W and Ctrl+Shift+W close
    // windows (ui/close_key.rs)
    if ctrl && !alt && (matches!(key, "F4" | "Tab") || letter(key) == Some('W')) {
        return Some(Refusal::Window);
    }
    if matches!(key, "Enter" | "NumpadEnter" | "Tab" | "Escape") {
        return Some(Refusal::Typing);
    }
    if (ctrl || alt)
        && matches!(
            key,
            "ArrowLeft"
                | "ArrowRight"
                | "ArrowUp"
                | "ArrowDown"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
                | "Backspace"
                | "Delete"
                | "Insert"
        )
    {
        return Some(Refusal::Navigation);
    }
    if ctrl && !alt {
        // judged by the letter typed, and by the keys that type those
        // letters on QWERTY, AZERTY and QWERTZ (A/Q, Z/W, Y/Z), as
        // `keybinds::text_field_keeps` does: a layout changed later keeps
        // them refused
        let typed = letter(key);
        if typed.is_some_and(|c| "ACVXYZ".contains(c)) {
            return Some(Refusal::Editing);
        }
        // Quit, Hide and Minimize on macOS, where Command counts as Ctrl
        if typed.is_some_and(|c| "QHM".contains(c)) {
            return Some(Refusal::System);
        }
        if matches!(key, "KeyA" | "KeyQ" | "KeyC" | "KeyV" | "KeyX" | "KeyY" | "KeyZ" | "KeyW") {
            return Some(Refusal::Editing);
        }
        if matches!(key, "Equal" | "Minus" | "Digit0" | "NumpadAdd" | "NumpadSubtract" | "Numpad0") {
            return Some(Refusal::Zoom);
        }
    }
    if function_key(key) {
        return None;
    }
    if ctrl && alt {
        return Some(Refusal::AltGr);
    }
    if !ctrl && !alt {
        return Some(Refusal::NeedsModifier);
    }
    None
}

/// What a shortcut takes over: a Preferences › Shortcuts action, which the
/// toolbar's shortcut then goes before (human request, 2026-11-25: Ctrl+U
/// may be used, and says so), or another toolbar target, which loses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    Overrides(String),
    Moves(String),
}

impl Conflict {
    pub fn message(&self) -> String {
        match self {
            Conflict::Overrides(name) => aurora_i18n::tr!("toolbar-shortcut-overrides", name = name.clone()),
            Conflict::Moves(name) => aurora_i18n::tr!("toolbar-shortcut-moves", name = name.clone()),
        }
    }
}

pub fn conflict(b: &Binding, keybinds: Option<&KeyBindings>, settings: &ToolbarSettings, layout: &Layout, own: &str) -> Option<Conflict> {
    if let Some(action) = keybinds.and_then(|keybinds| keybinds.users(b).first().copied()) {
        return Some(Conflict::Overrides(crate::keybinds::label(action).to_owned()));
    }
    settings
        .shortcuts
        .iter()
        .find(|(key, bound)| *bound == b && key.as_str() != own)
        .map(|(key, _)| Conflict::Moves(name(settings, layout, key)))
}

/// The button saved under `command`, wherever it stands.
fn find_button<'a>(settings: &'a ToolbarSettings, layout: &'a Layout, command: &str) -> Option<&'a ToolButton> {
    Bar::ALL
        .into_iter()
        .flat_map(|bar| settings.bar(bar, layout).iter())
        .chain(settings.drawers.values().flat_map(|drawer| drawer.buttons.iter()))
        .find(|button| button.command == command)
}

/// The name of a shortcut's target, as its button shows it.
pub fn name(settings: &ToolbarSettings, layout: &Layout, key: &str) -> String {
    if let Some(command) = key.strip_prefix(BLOCK) {
        let inner = find_button(settings, layout, command).map_or_else(|| command.to_owned(), |button| super::label(button).to_owned());
        return aurora_i18n::tr!("toolbar-shortcut-block-name", name = inner);
    }
    if let Some(button) = find_button(settings, layout, key) {
        return super::label(button).to_owned();
    }
    if let Some(floating) = settings.drawers.get(key).and_then(|drawer| drawer.floating.as_ref()) {
        return super::label(&floating.button(key)).to_owned();
    }
    let button = ToolButton {
        command: key.into(),
        icon: String::new(),
        tooltip: String::new(),
    };
    super::label(&button).to_owned()
}

/// Whether the shortcut saved under `key` works now: its button stands in
/// a bar or a block, its drawer has its button in a bar, or its floating
/// block exists. A plain block's never does.
pub fn active(settings: &ToolbarSettings, layout: &Layout, key: &str) -> bool {
    if key.starts_with(BLOCK) || structure::is_separator(key) || structure::is_divider(key) {
        return false;
    }
    if drawers::is_drawer(key) {
        return drawers::is_floating(settings, key) || (settings.drawers.contains_key(key) && settings.placed(key, layout));
    }
    settings.placed(key, layout)
}

/// The target of a key press, if a working toolbar shortcut is exactly it.
pub fn target(settings: &ToolbarSettings, layout: &Layout, b: &Binding) -> Option<String> {
    settings
        .shortcuts
        .iter()
        .find(|(key, bound)| *bound == b && active(settings, layout, key))
        .map(|(key, _)| key.clone())
}

/// A plain block's shortcut goes to the drawer or floating block `id` it
/// becomes (Fold, Float).
pub(super) fn into_drawer(settings: &mut ToolbarSettings, members: &[ToolButton], id: &str) {
    let mut found = None;
    for member in members {
        if let Some(b) = settings.shortcuts.remove(&block_key(&member.command)) {
            found.get_or_insert(b);
        }
    }
    if let Some(b) = found {
        settings.shortcuts.entry(id.into()).or_insert(b);
    }
}

/// A drawer's or floating block's shortcut goes to the plain block it
/// becomes (Unfold, Dock as a block).
pub(super) fn out_of_drawer(settings: &mut ToolbarSettings, id: &str, members: &[ToolButton]) {
    let first = members
        .iter()
        .find(|button| !structure::is_separator(&button.command) && !structure::is_divider(&button.command));
    if let Some(b) = settings.shortcuts.remove(id)
        && let Some(first) = first
    {
        settings.shortcuts.insert(block_key(&first.command), b);
    }
}

/// The shortcuts of drawers and floating blocks gone go with them; a
/// button's stays while it is in no bar, for when it comes back.
pub(super) fn forget_removed(settings: &mut ToolbarSettings) {
    let drawers = &settings.drawers;
    settings
        .shortcuts
        .retain(|key, _| !drawers::is_drawer(key) || drawers.contains_key(key));
}

fn press_id() -> Id {
    Id::unique("toolbar_shortcut_press")
}

/// A shortcut was pressed for `key`: its button is clicked when drawn
/// (`claim`), else `fallback` runs it after the frame.
pub fn press(ctx: &egui::Context, key: String) {
    ctx.data_mut(|data| data.insert_temp(press_id(), key));
    ctx.request_repaint();
}

/// Whether the button `command`, being drawn, takes the pressed shortcut.
pub fn claim(ctx: &egui::Context, command: &str) -> bool {
    ctx.data_mut(|data| {
        if data.get_temp::<String>(press_id()).as_deref() == Some(command) {
            data.remove_temp::<String>(press_id());
            true
        } else {
            false
        }
    })
}

/// A shortcut's press the frame did not take, run like its menu entry.
#[derive(Debug, PartialEq, Eq)]
pub enum Fallback {
    None,
    /// The settings changed (a floating block shown or hidden).
    Changed,
    /// The import bar, shown by the import preference.
    ImportBar,
}

pub fn fallback(ctx: &egui::Context, settings: &mut ToolbarSettings) -> Fallback {
    let Some(key) = ctx.data_mut(|data| data.remove_temp::<String>(press_id())) else {
        return Fallback::None;
    };
    if key == drawers::IMPORT_BAR && drawers::is_floating(settings, &key) {
        return Fallback::ImportBar;
    }
    if let Some(shown) = settings
        .drawers
        .get(&key)
        .and_then(|drawer| drawer.floating.as_ref())
        .map(|floating| floating.shown)
    {
        return if drawers::show_floating(settings, &key, !shown) {
            Fallback::Changed
        } else {
            Fallback::None
        };
    }
    if drawers::is_drawer(&key) {
        let open = drawers::is_open(ctx, &key);
        drawers::set_open(ctx, &key, !open);
        return Fallback::None;
    }
    super::commands::request(ctx, &key);
    ctx.request_repaint();
    Fallback::None
}

/// The shortcut window's state: the modifiers turned on, the key chosen,
/// and whether the next key press is the key (once "Choose a key" is
/// clicked).
#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub input: Option<String>,
    pub capturing: bool,
}

impl Editor {
    fn new(key: &str, current: Option<&Binding>) -> Editor {
        let input = current.and_then(|b| match &b.input {
            Input::Key(code) => Some(code.clone()),
            Input::Mouse(_) => None,
        });
        Editor {
            key: key.into(),
            ctrl: current.is_none_or(|b| b.ctrl),
            shift: current.is_some_and(|b| b.shift),
            alt: current.is_some_and(|b| b.alt),
            // it waits once "Choose a key" is clicked, not on opening
            // (human request, 2026-11-26)
            capturing: false,
            input,
        }
    }

    /// The shortcut the window shows, once it has a key.
    pub fn binding(&self) -> Option<Binding> {
        self.input.as_ref().map(|code| Binding {
            input: Input::Key(code.clone()),
            ctrl: self.ctrl,
            shift: self.shift,
            alt: self.alt,
        })
    }
}

const WINDOW: &str = "toolbar_shortcut";

impl ToolbarUi {
    /// The "Shortcut…" entry of a right click, its keys beside it.
    pub(super) fn shortcut_item(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &ToolbarSettings, key: &str) {
        let label = if key.starts_with(BLOCK) {
            aurora_i18n::tr!("toolbar-shortcut-block")
        } else {
            aurora_i18n::tr!("toolbar-shortcut")
        };
        self.shortcut_row(ui, p, settings, key, label);
    }

    /// "Block shortcut…", in a block's menu: a drawer's and a floating
    /// block's own shortcut included.
    pub(super) fn block_shortcut_item(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &ToolbarSettings, key: &str) {
        self.shortcut_row(ui, p, settings, key, aurora_i18n::tr!("toolbar-shortcut-block"));
    }

    fn shortcut_row(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &ToolbarSettings, key: &str, label: &str) {
        let current = settings.shortcuts.get(key);
        let keys = current.map(Binding::label).unwrap_or_default();
        if menu::item_keys(ui, p, "keyboard", label, &keys) {
            self.shortcut_editor = Some(Editor::new(key, current));
        }
    }

    /// Opens the shortcut window of `key` (a block's "Edit" button).
    pub(super) fn edit_shortcut(&mut self, key: &str, current: Option<&Binding>) {
        self.shortcut_editor = Some(Editor::new(key, current));
    }

    /// Whether the shortcut window is open: the app then gives it the
    /// Preferences shortcuts (`keybinds`), for its warnings.
    pub fn shortcut_editing(&self) -> bool {
        self.shortcut_editor.is_some()
    }

    /// Whether the shortcut window waits for a key: the app gives the next
    /// press to `shortcut_captured` instead of its action.
    pub fn shortcut_capturing(&self) -> bool {
        self.shortcut_editor.as_ref().is_some_and(|editor| editor.capturing)
    }

    /// The key pressed for the shortcut window, with the modifiers held, if
    /// any, in place of those turned on; Escape alone stops waiting.
    pub fn shortcut_captured(&mut self, b: Binding, escape: bool) {
        let Some(editor) = self.shortcut_editor.as_mut() else { return };
        editor.capturing = false;
        if escape && !b.ctrl && !b.shift && !b.alt {
            return;
        }
        if let Input::Key(code) = b.input {
            editor.input = Some(code);
        }
        if b.ctrl || b.shift || b.alt {
            (editor.ctrl, editor.shift, editor.alt) = (b.ctrl, b.shift, b.alt);
        }
    }

    pub(super) fn show_shortcut_editor(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings, layout: &Layout) {
        let Some(mut editor) = self.shortcut_editor.clone() else { return };
        let current = settings.shortcuts.get(&editor.key).cloned();
        let draft = editor.binding();
        let refused = draft.as_ref().and_then(refusal);
        let conflict = draft
            .as_ref()
            .filter(|_| refused.is_none())
            .and_then(|b| conflict(b, self.keybinds.as_ref(), settings, layout, &editor.key));
        let mut open = true;
        let mut save = false;
        let mut clear = false;
        let mut window = widgets::Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-shortcut-title"),
            ctx.content_rect().center() - Vec2::new(200.0, 130.0),
            Vec2::new(420.0, 300.0),
        );
        window.order = egui::Order::Foreground;
        window.show(ctx, p, &mut open, |ui| {
            ui.label(egui::RichText::new(name(settings, layout, &editor.key)).strong());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // the modifiers, turned on and off by a click
                // the modifiers, keys as Preferences draws them (human
                // request, 2026-11-26), on in the accent
                for (on, code) in [
                    (&mut editor.ctrl, "ControlLeft"),
                    (&mut editor.alt, "AltLeft"),
                    (&mut editor.shift, "ShiftLeft"),
                ] {
                    let fill = if *on { p.violet } else { p.raised };
                    let cap = Binding {
                        input: Input::Key(code.into()),
                        ctrl: false,
                        shift: false,
                        alt: false,
                    };
                    let caps = widgets::keycap::caps_sized(ui, &cap, fill, widgets::keycap::LARGE);
                    let width = widgets::keycap::row_width(&caps);
                    let response = ui.add(egui::Button::new("").fill(fill).min_size(Vec2::new((width + 16.0).max(56.0), 28.0)));
                    let left = egui::pos2(response.rect.center().x - width / 2.0, response.rect.center().y);
                    widgets::keycap::paint_row(ui, p, &caps, left, fill);
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(Id::unique(("shortcut_modifier", code)), response.rect));
                    if response.clicked() {
                        *on = !*on;
                    }
                }
                ui.label("+");
                // the key: a click waits for the next one
                let fill = if editor.capturing { p.violet } else { p.raised };
                let key = editor.input.clone().map(|code| Binding {
                    input: Input::Key(code),
                    ctrl: false,
                    shift: false,
                    alt: false,
                });
                let caps = match &key {
                    Some(b) if !editor.capturing => widgets::keycap::caps_sized(ui, b, fill, widgets::keycap::LARGE),
                    _ => Vec::new(),
                };
                let (text, color) = if editor.capturing {
                    (aurora_i18n::tr!("toolbar-shortcut-press-key"), p.on_accent)
                } else if key.is_some() {
                    ("", p.ink)
                } else {
                    (aurora_i18n::tr!("toolbar-shortcut-choose-key"), p.muted)
                };
                let width = widgets::keycap::row_width(&caps);
                let response = ui.add(
                    egui::Button::new(egui::RichText::new(text).color(color))
                        .fill(fill)
                        .min_size(Vec2::new((width + 16.0).max(150.0), 28.0)),
                );
                let left = egui::pos2(response.rect.center().x - width / 2.0, response.rect.center().y);
                widgets::keycap::paint_row(ui, p, &caps, left, fill);
                if response.clicked() {
                    editor.capturing = !editor.capturing;
                }
            });
            ui.add_space(6.0);
            // while it waits for the key (human request, 2026-11-26)
            if editor.capturing {
                ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-shortcut-esc")).color(p.muted));
            }
            if let Some(refusal) = refused {
                ui.label(egui::RichText::new(refusal.message()).color(p.danger));
            } else if let Some(conflict) = &conflict {
                ui.label(egui::RichText::new(conflict.message()).color(p.amber));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ready = draft.is_some() && refused.is_none() && draft != current;
                save = widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy",
                    aurora_i18n::tr!("toolbar-shortcut-save"),
                    widgets::ActionKind::Primary,
                    Vec2::new(0.0, 28.0),
                    ready,
                )
                .clicked();
                clear = current.is_some()
                    && widgets::action_button(
                        ui,
                        p,
                        "trash-filled",
                        aurora_i18n::tr!("toolbar-shortcut-clear"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked();
            });
            ui.add_space(8.0);
            let note = |ui: &mut egui::Ui, text: &str| {
                ui.label(egui::RichText::new(text).color(p.muted));
            };
            note(ui, aurora_i18n::tr!("toolbar-shortcut-help"));
            if editor.key.starts_with(BLOCK) {
                note(ui, aurora_i18n::tr!("toolbar-shortcut-block-help"));
            } else if !active(settings, layout, &editor.key) {
                note(ui, aurora_i18n::tr!("toolbar-shortcut-inactive"));
            }
        });
        if save && let Some(b) = draft {
            self.pending = Some(Edit::Shortcut(editor.key.clone(), Some(b)));
        } else if clear {
            self.pending = Some(Edit::Shortcut(editor.key.clone(), None));
            editor.input = None;
        }
        if save || clear {
            editor.capturing = false;
        }
        self.shortcut_editor = open.then_some(editor);
    }
}

/// Applies `Edit::Shortcut`: locked bars included, the shortcut is not
/// their layout.
pub(super) fn set(settings: &mut ToolbarSettings, key: &str, binding: Option<Binding>) -> bool {
    match binding {
        Some(b) => {
            if refusal(&b).is_some() || settings.shortcuts.get(key) == Some(&b) {
                return false;
            }
            settings.shortcuts.retain(|other, bound| other == key || *bound != b);
            settings.shortcuts.insert(key.into(), b);
            true
        }
        None => settings.shortcuts.remove(key).is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(code: &str, ctrl: bool, shift: bool, alt: bool) -> Binding {
        Binding {
            input: Input::Key(code.into()),
            ctrl,
            shift,
            alt,
        }
    }

    /// The keys refused whatever the keyboard (human request, 2026-11-25:
    /// "Ctrl+A, Ctrl+C, Ctrl+V, Ctrl+Shift+V, Ctrl+X…"), each with its
    /// reason, and some that stay free.
    #[test]
    fn the_keys_text_fields_the_chat_and_the_system_keep_are_refused() {
        use Refusal::*;
        // the letter keys' reason depends on the keyboard (AZERTY's
        // physical A types Q, Quit on macOS): refused either way
        for code in ["KeyA", "KeyQ", "KeyC", "KeyV", "KeyX", "KeyZ", "KeyW", "KeyY", "KeyH"] {
            for shift in [false, true] {
                let b = keys(code, true, shift, false);
                assert!(refusal(&b).is_some(), "{b:?}");
            }
        }
        let refused = [
            (keys("F4", false, false, true), System),
            (keys("Tab", false, false, true), System),
            (keys("Escape", true, true, false), System),
            (keys("Delete", true, false, true), System),
            (keys("Space", true, false, false), System),
            (keys("Enter", false, false, true), System),
            (keys("PrintScreen", false, false, false), System),
            (keys("F4", true, false, false), Window),
            (keys("Tab", true, true, false), Window),
            (keys("Enter", true, false, false), Typing),
            (keys("Escape", false, false, false), Typing),
            (keys("ArrowLeft", true, true, false), Navigation),
            (keys("Backspace", true, false, false), Navigation),
            (keys("Home", false, false, true), Navigation),
            (keys("Insert", true, false, false), Navigation),
            (keys("Equal", true, false, false), Zoom),
            (keys("Digit0", true, false, false), Zoom),
            (keys("KeyK", true, false, true), AltGr),
            (keys("Digit2", true, true, true), AltGr),
            (keys("KeyK", false, false, false), NeedsModifier),
            (keys("KeyK", false, true, false), NeedsModifier),
            (keys("Digit5", false, false, false), NeedsModifier),
            (keys("ControlLeft", false, false, false), Modifier),
            (keys("CapsLock", false, false, false), Modifier),
            (
                Binding {
                    input: Input::Mouse("Middle".into()),
                    ctrl: true,
                    shift: false,
                    alt: false,
                },
                Keyboard,
            ),
        ];
        for (b, why) in refused {
            assert_eq!(refusal(&b), Some(why), "{b:?}");
        }
        for b in [
            keys("KeyK", true, false, false),
            keys("KeyK", true, true, false),
            keys("KeyK", false, false, true),
            keys("Digit5", true, false, false),
            keys("F5", false, false, false),
            keys("F5", false, true, false),
            keys("F12", true, false, true),
            keys("F24", false, false, false),
        ] {
            assert_eq!(refusal(&b), None, "{b:?}");
        }
    }

    /// A key of Preferences › Shortcuts or of another toolbar target is
    /// taken; `set` gives a key to one target only.
    #[test]
    fn a_shortcut_belongs_to_one_target() {
        let layout = Layout::default();
        let keybinds = KeyBindings::default();
        let preferences = keybinds.get(crate::keybinds::Action::Preferences)[0]
            .clone()
            .expect("Preferences has a key");
        let mut settings = ToolbarSettings::default();
        assert!(matches!(
            conflict(&preferences, Some(&keybinds), &settings, &layout, "inventory"),
            Some(Conflict::Overrides(_))
        ));
        let k = keys("KeyK", true, true, false);
        assert!(conflict(&k, Some(&keybinds), &settings, &layout, "inventory").is_none());
        assert!(set(&mut settings, "inventory", Some(k.clone())));
        assert!(!set(&mut settings, "inventory", Some(k.clone())), "no change");
        assert!(conflict(&k, Some(&keybinds), &settings, &layout, "inventory").is_none(), "its own");
        assert!(matches!(
            conflict(&k, Some(&keybinds), &settings, &layout, "people"),
            Some(Conflict::Moves(_))
        ));
        assert!(set(&mut settings, "people", Some(k.clone())));
        assert_eq!(settings.shortcuts.keys().collect::<Vec<_>>(), ["people"], "moved, not doubled");
        assert!(!set(&mut settings, "people", Some(keys("KeyC", true, false, false))), "refused");
        assert!(set(&mut settings, "people", None));
        assert!(settings.shortcuts.is_empty());
    }

    /// A shortcut works only for a button in a bar or a block, a drawer
    /// whose button is placed, or a floating block.
    #[test]
    fn only_placed_targets_answer_their_keys() {
        let layout = Layout::default();
        let mut settings = ToolbarSettings {
            composed: true,
            buttons: Some(vec![super::super::tests::button("inventory")]),
            ..Default::default()
        };
        let k = keys("KeyK", true, false, false);
        let f = keys("F6", false, false, false);
        let b = keys("KeyJ", false, false, true);
        settings.shortcuts.insert("inventory".into(), k.clone());
        settings.shortcuts.insert("people".into(), f.clone());
        settings.shortcuts.insert(block_key("inventory"), b.clone());
        assert_eq!(target(&settings, &layout, &k).as_deref(), Some("inventory"));
        assert_eq!(target(&settings, &layout, &f), None, "in no bar");
        assert_eq!(target(&settings, &layout, &b), None, "a plain block's waits");
        let id = drawers::create_floating(&mut settings, "Tools");
        settings.shortcuts.insert(id.clone(), f.clone());
        settings.shortcuts.remove("people");
        assert_eq!(target(&settings, &layout, &f).as_deref(), Some(id.as_str()));
        settings.drawers.remove(&id);
        forget_removed(&mut settings);
        assert!(!settings.shortcuts.contains_key(&id));
        assert!(settings.shortcuts.contains_key(&block_key("inventory")), "kept for its block");
    }

    /// The window's key: the next press, Escape alone giving up; modifiers
    /// held replace those turned on.
    #[test]
    fn the_window_takes_a_key_and_its_modifiers() {
        let mut ui = ToolbarUi {
            shortcut_editor: Some(Editor::new("inventory", None)),
            ..Default::default()
        };
        assert!(!ui.shortcut_capturing(), "it waits only once asked");
        if let Some(editor) = ui.shortcut_editor.as_mut() {
            editor.capturing = true;
        }
        ui.shortcut_captured(keys("Escape", false, false, false), true);
        assert!(!ui.shortcut_capturing());
        assert_eq!(ui.shortcut_editor.as_ref().and_then(Editor::binding), None);
        ui.shortcut_captured(keys("KeyU", false, false, false), false);
        assert_eq!(
            ui.shortcut_editor.as_ref().and_then(Editor::binding),
            Some(keys("KeyU", true, false, false)),
            "Ctrl is on by default"
        );
        ui.shortcut_captured(keys("KeyJ", false, true, true), false);
        assert_eq!(
            ui.shortcut_editor.as_ref().and_then(Editor::binding),
            Some(keys("KeyJ", false, true, true))
        );
        let editor = Editor::new("people", Some(&keys("F6", false, true, false)));
        assert!(!editor.capturing && !editor.ctrl && editor.shift);
    }

    /// A press is claimed once, by its button; unclaimed, a drawer opens and
    /// a button runs like its menu entry.
    #[test]
    fn an_unclaimed_press_runs_like_the_menu() {
        let ctx = egui::Context::default();
        let mut settings = ToolbarSettings::default();
        press(&ctx, "inventory".into());
        assert!(!claim(&ctx, "people"));
        assert!(claim(&ctx, "inventory"));
        assert!(!claim(&ctx, "inventory"), "once");
        assert_eq!(fallback(&ctx, &mut settings), Fallback::None);
        assert_eq!(super::super::commands::take_request(&ctx), None);
        press(&ctx, "inventory".into());
        assert_eq!(fallback(&ctx, &mut settings), Fallback::None);
        assert_eq!(super::super::commands::take_request(&ctx).as_deref(), Some("inventory"));
        press(&ctx, "drawer:shelf".into());
        let _ = fallback(&ctx, &mut settings);
        assert!(drawers::is_open(&ctx, "drawer:shelf"));
        // the import block's display is the import windows' while it floats
        assert!(settings.ensure_import_bar(&Layout::default()));
        press(&ctx, drawers::IMPORT_BAR.into());
        assert_eq!(fallback(&ctx, &mut settings), Fallback::ImportBar);
        let id = drawers::create_floating(&mut settings, "Tools");
        press(&ctx, id.clone());
        assert_eq!(fallback(&ctx, &mut settings), Fallback::Changed);
    }
}
