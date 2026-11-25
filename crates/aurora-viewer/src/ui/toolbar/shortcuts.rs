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
//! refused at capture (`refusal`, `taken_by`).

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
/// them now (`taken_by` adds those of Preferences › Shortcuts).
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

/// What already uses `b`: an action of Preferences › Shortcuts, or another
/// toolbar shortcut than `own`'s.
pub fn taken_by(b: &Binding, keybinds: &KeyBindings, settings: &ToolbarSettings, layout: &Layout, own: &str) -> Option<String> {
    if let Some(action) = keybinds.users(b).first() {
        return Some(crate::keybinds::label(*action).to_owned());
    }
    settings
        .shortcuts
        .iter()
        .find(|(key, bound)| *bound == b && key.as_str() != own)
        .map(|(key, _)| name(settings, layout, key))
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
    if key == drawers::IMPORT_BAR {
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

/// The shortcut window's state.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub key: String,
    pub capturing: bool,
    pub message: Option<String>,
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
        let keys = settings.shortcuts.get(key).map(Binding::label).unwrap_or_default();
        if menu::item_keys(ui, p, "keyboard", label, &keys) {
            self.shortcut_editor = Some(Editor {
                key: key.into(),
                capturing: true,
                message: None,
            });
        }
    }

    /// Whether the shortcut window waits for keys: the app gives them to
    /// `shortcut_captured` instead of their actions.
    pub fn shortcut_capturing(&self) -> bool {
        self.shortcut_editor.as_ref().is_some_and(|editor| editor.capturing)
    }

    /// Keys pressed in the shortcut window: kept unless refused or taken
    /// (the reason is shown); Escape stops waiting.
    pub fn shortcut_captured(&mut self, b: Binding, escape: bool, keybinds: &KeyBindings, settings: &ToolbarSettings, layout: &Layout) {
        let Some(editor) = self.shortcut_editor.as_mut() else { return };
        if escape && !b.ctrl && !b.shift && !b.alt {
            editor.capturing = false;
            editor.message = None;
            return;
        }
        let refused = refusal(&b).map(|refusal| refusal.message().to_owned()).or_else(|| {
            taken_by(&b, keybinds, settings, layout, &editor.key)
                .map(|name| aurora_i18n::tr!("toolbar-shortcut-taken", keys = b.label(), name = name))
        });
        match refused {
            Some(message) => editor.message = Some(format!("{} — {message}", b.label())),
            None => {
                editor.capturing = false;
                editor.message = None;
                self.pending = Some(Edit::Shortcut(editor.key.clone(), Some(b)));
            }
        }
    }

    pub(super) fn show_shortcut_editor(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings, layout: &Layout) {
        let Some(mut editor) = self.shortcut_editor.clone() else { return };
        let current = settings.shortcuts.get(&editor.key).cloned();
        let mut open = true;
        let mut clear = false;
        let mut window = widgets::Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-shortcut-title"),
            ctx.content_rect().center() - Vec2::new(180.0, 110.0),
            Vec2::new(360.0, 220.0),
        );
        window.order = egui::Order::Foreground;
        window.show(ctx, p, &mut open, |ui| {
            ui.label(egui::RichText::new(name(settings, layout, &editor.key)).strong());
            ui.add_space(8.0);
            let fill = if editor.capturing { p.violet } else { p.raised };
            let caps = match &current {
                Some(b) if !editor.capturing => widgets::keycap::caps(ui, b, fill),
                _ => Vec::new(),
            };
            let (text, color) = if editor.capturing {
                (aurora_i18n::tr!("toolbar-shortcut-press"), p.on_accent)
            } else if current.is_some() {
                ("", p.ink)
            } else {
                (aurora_i18n::tr!("toolbar-shortcut-none"), p.muted)
            };
            let width = widgets::keycap::row_width(&caps);
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::Button::new(egui::RichText::new(text).color(color))
                        .fill(fill)
                        .min_size(Vec2::new((width + 16.0).max(200.0), 28.0)),
                );
                let left = egui::pos2(response.rect.center().x - width / 2.0, response.rect.center().y);
                widgets::keycap::paint_row(ui, p, &caps, left, fill);
                if response.clicked() {
                    editor.capturing = !editor.capturing;
                    editor.message = None;
                }
                if current.is_some()
                    && widgets::action_button(
                        ui,
                        p,
                        "trash-filled",
                        aurora_i18n::tr!("toolbar-shortcut-clear"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                {
                    clear = true;
                }
            });
            if let Some(message) = &editor.message {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(message).color(p.danger));
            }
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("toolbar-shortcut-help"))
                    .small()
                    .color(p.muted),
            );
            if editor.key.starts_with(BLOCK) {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("toolbar-shortcut-block-help"))
                        .small()
                        .color(p.muted),
                );
            } else if !active(settings, layout, &editor.key) {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("toolbar-shortcut-inactive"))
                        .small()
                        .color(p.muted),
                );
            }
        });
        if clear {
            self.pending = Some(Edit::Shortcut(editor.key.clone(), None));
            editor.capturing = false;
            editor.message = None;
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
        assert!(taken_by(&preferences, &keybinds, &settings, &layout, "inventory").is_some());
        let k = keys("KeyK", true, true, false);
        assert!(taken_by(&k, &keybinds, &settings, &layout, "inventory").is_none());
        assert!(set(&mut settings, "inventory", Some(k.clone())));
        assert!(!set(&mut settings, "inventory", Some(k.clone())), "no change");
        assert!(taken_by(&k, &keybinds, &settings, &layout, "inventory").is_none(), "its own");
        assert!(taken_by(&k, &keybinds, &settings, &layout, "people").is_some());
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
        press(&ctx, drawers::IMPORT_BAR.into());
        assert_eq!(fallback(&ctx, &mut settings), Fallback::ImportBar);
        let id = drawers::create_floating(&mut settings, "Tools");
        press(&ctx, id.clone());
        assert_eq!(fallback(&ctx, &mut settings), Fallback::Changed);
    }
}
