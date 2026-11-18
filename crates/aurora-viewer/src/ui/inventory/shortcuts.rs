//! The inventory's keyboard shortcuts in one place: each command, its keys,
//! and how a tooltip or a menu entry names them. The list reads them from
//! here (`take`), the toolbar's tooltips and the menus' labels show them
//! (`label`, `tip`), so a key changes in this table only.
//!
//! The keys follow Windows File Explorer and KDE Dolphin, which most people
//! already know (Aurora difference: the Legacy Viewer's inventory has none
//! of these but its arrows, Enter, F2 and Delete, LLFolderView::
//! handleKeyHere and LLInventoryPanel::handleKeyHere). Where a desktop
//! habit and a Second Life rule meet, the rule wins: on a Mac, Backspace
//! deletes as in the Legacy Viewer (`delete::deletes`), so it never goes up
//! there, and Finder's Cmd+[ / Cmd+] / Cmd+↑ are offered as well.

use egui::{Key, KeyboardShortcut, Modifiers};

/// What a shortcut does in the explorer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Command {
    Back,
    Forward,
    Up,
    /// Explorer's Alt+Home: the top of the inventory.
    Root,
    /// Ask the server for the folder again (F5).
    Refresh,
    /// The breadcrumb becomes a path to type (Ctrl+L, Alt+D, F4).
    EditPath,
    SelectAll,
    /// Dolphin's Ctrl+Shift+A.
    InvertSelection,
    NewFolder,
    /// Explorer's Alt+Enter.
    Properties,
    /// Shift+F10: the selection's menu, where the selection is.
    ContextMenu,
    /// Shift+Delete: out of the Trash for good, after its confirmation.
    Purge,
    /// F3, beside the search field's Ctrl+F.
    Search,
    // Shown in the menus only: the list handles these keys itself
    // (keyboard::open_shortcut, the rename, delete::shortcut, egui's
    // clipboard events).
    Open,
    Rename,
    Delete,
    Cut,
    Copy,
    Paste,
}

/// One key of a command. `mac` limits it to one kind of system: `Some(true)`
/// on a Mac only, `Some(false)` elsewhere only.
struct Binding {
    command: Command,
    shortcut: KeyboardShortcut,
    mac: Option<bool>,
}

const fn bind(command: Command, modifiers: Modifiers, key: Key, mac: Option<bool>) -> Binding {
    Binding {
        command,
        shortcut: KeyboardShortcut::new(modifiers, key),
        mac,
    }
}

const COMMAND_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const NONE: Modifiers = Modifiers::NONE;

/// The table. The first key of a command is the one its tooltip and its
/// menu entry show; Backspace (Up, Dolphin) depends on a preference and is
/// handled by `take`.
const BINDINGS: &[Binding] = &[
    bind(Command::Back, Modifiers::ALT, Key::ArrowLeft, Some(false)),
    bind(Command::Back, Modifiers::COMMAND, Key::OpenBracket, Some(true)),
    bind(Command::Forward, Modifiers::ALT, Key::ArrowRight, Some(false)),
    bind(Command::Forward, Modifiers::COMMAND, Key::CloseBracket, Some(true)),
    bind(Command::Up, Modifiers::ALT, Key::ArrowUp, Some(false)),
    bind(Command::Up, Modifiers::COMMAND, Key::ArrowUp, Some(true)),
    bind(Command::Root, Modifiers::ALT, Key::Home, None),
    bind(Command::Refresh, NONE, Key::F5, None),
    bind(Command::EditPath, Modifiers::COMMAND, Key::L, None),
    bind(Command::EditPath, Modifiers::ALT, Key::D, Some(false)),
    bind(Command::EditPath, NONE, Key::F4, None),
    bind(Command::InvertSelection, COMMAND_SHIFT, Key::A, None),
    bind(Command::SelectAll, Modifiers::COMMAND, Key::A, None),
    bind(Command::NewFolder, COMMAND_SHIFT, Key::N, None),
    bind(Command::Properties, Modifiers::ALT, Key::Enter, None),
    bind(Command::ContextMenu, Modifiers::SHIFT, Key::F10, None),
    bind(Command::Purge, Modifiers::SHIFT, Key::Delete, None),
    bind(Command::Search, NONE, Key::F3, None),
    bind(Command::Open, NONE, Key::Enter, None),
    bind(Command::Rename, NONE, Key::F2, None),
    bind(Command::Delete, NONE, Key::Delete, None),
    bind(Command::Cut, Modifiers::COMMAND, Key::X, None),
    bind(Command::Copy, Modifiers::COMMAND, Key::C, None),
    bind(Command::Paste, Modifiers::COMMAND, Key::V, None),
];

/// The command whose keys an inventory menu entry shows, by the entry's id
/// (`context_catalog`).
pub(crate) fn for_menu_entry(id: &str) -> Option<Command> {
    Some(match id {
        "open" => Command::Open,
        "rename" => Command::Rename,
        "delete" => Command::Delete,
        "cut" => Command::Cut,
        "copy" => Command::Copy,
        "paste" => Command::Paste,
        "properties" => Command::Properties,
        "purge" => Command::Purge,
        "new-folder" => Command::NewFolder,
        "reload" => Command::Refresh,
        "select-all" => Command::SelectAll,
        "invert-selection" => Command::InvertSelection,
        _ => return None,
    })
}

fn applies(binding: &Binding, mac: bool) -> bool {
    binding.mac.is_none_or(|only| only == mac)
}

/// Whether this build runs on a Mac (the keys differ there).
pub(crate) const fn mac() -> bool {
    cfg!(target_os = "macos")
}

/// The command a key pressed with `modifiers` gives, if any. `backspace_up`:
/// the explorer's preference for Dolphin's Backspace (never on a Mac).
pub(crate) fn command(key: Key, modifiers: Modifiers, mac: bool, backspace_up: bool) -> Option<Command> {
    if key == Key::Backspace && modifiers.is_none() && backspace_up && !mac {
        return Some(Command::Up);
    }
    BINDINGS
        .iter()
        .filter(|binding| applies(binding, mac))
        .find(|binding| binding.shortcut.logical_key == key && modifiers.matches_exact(binding.shortcut.modifiers))
        .map(|binding| binding.command)
}

/// Takes this frame's key presses that are explorer commands out of the
/// input, in order, so that no other widget acts on them too.
pub(crate) fn take(ui: &egui::Ui, backspace_up: bool, wanted: impl Fn(Command) -> bool) -> Vec<Command> {
    let mac = mac();
    let mut taken = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            match command(*key, *modifiers, mac, backspace_up).filter(|c| wanted(*c)) {
                Some(found) => {
                    taken.push(found);
                    false
                }
                None => true,
            }
        });
    });
    taken
}

/// The first key of `command` on this system, as egui writes it ("Alt+Left",
/// "⌘L").
pub(crate) fn label(ctx: &egui::Context, command: Command) -> Option<String> {
    let mac = mac();
    BINDINGS
        .iter()
        .find(|binding| binding.command == command && applies(binding, mac))
        .map(|binding| ctx.format_shortcut(&binding.shortcut))
}

/// A tooltip with its shortcut: "Back (Alt+Left)".
pub(crate) fn tip(ctx: &egui::Context, text: &str, command: Command) -> String {
    match label(ctx, command) {
        Some(key) => format!("{text} ({key})"),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Modifiers = Modifiers::COMMAND.plus(Modifiers::CTRL);

    #[test]
    fn each_key_gives_one_command_and_mac_keys_stay_on_a_mac() {
        assert_eq!(command(Key::ArrowLeft, Modifiers::ALT, false, true), Some(Command::Back));
        assert_eq!(command(Key::ArrowLeft, Modifiers::ALT, true, true), None);
        assert_eq!(
            command(Key::OpenBracket, Modifiers::MAC_CMD.plus(Modifiers::COMMAND), true, true),
            Some(Command::Back)
        );
        assert_eq!(command(Key::ArrowUp, Modifiers::ALT, false, true), Some(Command::Up));
        assert_eq!(command(Key::A, CTRL, false, true), Some(Command::SelectAll));
        assert_eq!(
            command(Key::A, CTRL.plus(Modifiers::SHIFT), false, true),
            Some(Command::InvertSelection)
        );
        assert_eq!(command(Key::N, CTRL.plus(Modifiers::SHIFT), false, true), Some(Command::NewFolder));
        assert_eq!(command(Key::N, CTRL, false, true), None);
        assert_eq!(command(Key::Delete, Modifiers::SHIFT, false, true), Some(Command::Purge));
        assert_eq!(command(Key::Delete, NONE, false, true), Some(Command::Delete));
        assert_eq!(command(Key::Enter, Modifiers::ALT, false, true), Some(Command::Properties));
        // shown in menus, handled by the list itself
        assert_eq!(command(Key::Enter, NONE, false, true), Some(Command::Open));
    }

    #[test]
    fn backspace_goes_up_by_preference_and_never_on_a_mac() {
        assert_eq!(command(Key::Backspace, NONE, false, true), Some(Command::Up));
        assert_eq!(command(Key::Backspace, NONE, false, false), None);
        // the Legacy Viewer deletes with it there
        assert_eq!(command(Key::Backspace, NONE, true, true), None);
        assert_eq!(command(Key::Backspace, Modifiers::SHIFT, false, true), None);
    }

    #[test]
    fn every_command_has_a_key_on_every_system() {
        let all = [
            Command::Back,
            Command::Forward,
            Command::Up,
            Command::Root,
            Command::Refresh,
            Command::EditPath,
            Command::SelectAll,
            Command::InvertSelection,
            Command::NewFolder,
            Command::Properties,
            Command::ContextMenu,
            Command::Purge,
            Command::Search,
            Command::Open,
            Command::Rename,
            Command::Delete,
            Command::Cut,
            Command::Copy,
            Command::Paste,
        ];
        for mac in [false, true] {
            for command in all {
                assert!(
                    BINDINGS.iter().any(|b| b.command == command && applies(b, mac)),
                    "{command:?} on mac={mac}"
                );
            }
        }
    }

    #[test]
    fn a_tip_names_its_key() {
        let ctx = egui::Context::default();
        let tip = tip(&ctx, "Back", Command::Back);
        assert!(tip.starts_with("Back (") && tip.ends_with(')'), "{tip}");
    }
}
