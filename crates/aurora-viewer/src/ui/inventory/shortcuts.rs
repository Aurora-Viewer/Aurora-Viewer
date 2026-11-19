//! The inventory lists' keyboard shortcuts in one place: each command, the
//! keys the user gave it (Preferences › Shortcuts › Inventory,
//! `keybinds::is_inventory`), and how a tooltip or a menu entry names them.
//! The list reads them from here (`take`, `mouse`), the toolbar's tooltips
//! and the menus' labels show them (`label`, `tip`), and the viewer leaves
//! a focused list its chords (`list_keeps`).
//!
//! The default keys follow Windows Explorer and KDE Dolphin (Finder on a
//! Mac), which most people already know (Aurora difference: the Legacy
//! Viewer's inventory has none of these but its arrows, Enter, F2 and
//! Delete, LLFolderView::handleKeyHere and LLInventoryPanel::handleKeyHere).
//! Where a desktop habit and a Second Life rule meet, the rule wins: on a
//! Mac, Backspace deletes as in the Legacy Viewer (`delete::deletes`), so
//! it does not go up there by default.
//!
//! As the chat fields' (`ui/chat/shortcuts.rs`), the app publishes the
//! user's table once a frame (`publish`) and the lists read it from the
//! context (`get`), the defaults until then. The keys the list keeps fixed
//! (its arrows, Enter, F2, Delete, Ctrl+C / X / V…, `keybinds::
//! inventory_accepts`) are only named here, for the menus.

use crate::keybinds::{Action, Binding, Input, KeyBindings};
use egui::{Key, KeyboardShortcut, Modifiers};

/// What a shortcut does in an inventory list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Command {
    Back,
    Forward,
    Up,
    /// Explorer's Alt+Home: the top of the inventory.
    Root,
    /// Ask the server for the folder again (F5).
    Refresh,
    /// The breadcrumb becomes a path to type (Ctrl+L, Alt+D).
    EditPath,
    SelectAll,
    /// Dolphin's Ctrl+Shift+A.
    InvertSelection,
    /// Explorer's Ctrl+Space: the row in focus in or out of the selection.
    ToggleSelection,
    NewFolder,
    /// Explorer's Alt+Enter.
    Properties,
    /// Shift+F10: the selection's menu, where the selection is.
    ContextMenu,
    /// Shift+Delete: out of the Trash for good, after its confirmation.
    Purge,
    /// F3, beside the search field's Ctrl+F.
    Search,
    /// Dolphin's Ctrl+I: the bar that filters the folder shown.
    Filter,
    /// Explorer's Alt+P: the details pane shows what the selection holds.
    Preview,
    /// Explorer's Ctrl+Shift+1…6: the large, medium and small pictures, the
    /// Compact view, the Details (the list), the Tiles.
    View(u8),
    // Named in the menus only: the list keeps these keys fixed and handles
    // them itself (keyboard::open_shortcut, the rename, delete::shortcut,
    // egui's clipboard events).
    Open,
    Rename,
    Delete,
    Cut,
    Copy,
    Paste,
}

impl Command {
    /// The key-bind action that holds the command's keys; None for the
    /// fixed keys.
    pub(crate) fn action(self) -> Option<Action> {
        Some(match self {
            Command::Back => Action::InvBack,
            Command::Forward => Action::InvForward,
            Command::Up => Action::InvUp,
            Command::Root => Action::InvRoot,
            Command::Refresh => Action::InvRefresh,
            Command::EditPath => Action::InvEditPath,
            Command::SelectAll => Action::InvSelectAll,
            Command::InvertSelection => Action::InvInvertSelection,
            Command::ToggleSelection => Action::InvToggleSelection,
            Command::NewFolder => Action::InvNewFolder,
            Command::Properties => Action::InvProperties,
            Command::ContextMenu => Action::InvContextMenu,
            Command::Purge => Action::InvPurge,
            Command::Search => Action::InvSearch,
            Command::Filter => Action::InvFilter,
            Command::Preview => Action::InvPreview,
            Command::View(1) => Action::InvViewLarge,
            Command::View(2) => Action::InvViewMedium,
            Command::View(3) => Action::InvViewSmall,
            Command::View(4) => Action::InvViewCompact,
            Command::View(5) => Action::InvViewDetails,
            Command::View(_) => Action::InvViewTiles,
            Command::Open | Command::Rename | Command::Delete | Command::Cut | Command::Copy | Command::Paste => return None,
        })
    }

    /// The fixed key of a command the list keeps (for its menu entry).
    fn fixed(self) -> Option<KeyboardShortcut> {
        let (modifiers, key) = match self {
            Command::Open => (Modifiers::NONE, Key::Enter),
            Command::Rename => (Modifiers::NONE, Key::F2),
            Command::Delete => (Modifiers::NONE, Key::Delete),
            Command::Cut => (Modifiers::COMMAND, Key::X),
            Command::Copy => (Modifiers::COMMAND, Key::C),
            Command::Paste => (Modifiers::COMMAND, Key::V),
            _ => return None,
        };
        Some(KeyboardShortcut::new(modifiers, key))
    }
}

/// Every rebindable command, in the Shortcuts page's order.
pub(crate) const REBINDABLE: [Command; 22] = [
    Command::Back,
    Command::Forward,
    Command::Up,
    Command::Root,
    Command::Refresh,
    Command::EditPath,
    Command::Search,
    Command::Filter,
    Command::Preview,
    Command::View(1),
    Command::View(2),
    Command::View(3),
    Command::View(4),
    Command::View(5),
    Command::View(6),
    Command::SelectAll,
    Command::InvertSelection,
    Command::ToggleSelection,
    Command::NewFolder,
    Command::Properties,
    Command::ContextMenu,
    Command::Purge,
];

/// The inventory actions' bindings, as the user set them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InventoryKeys {
    map: Vec<(Action, [Option<Binding>; 2])>,
}

impl InventoryKeys {
    pub(crate) fn of(kb: &KeyBindings) -> InventoryKeys {
        InventoryKeys {
            map: kb.map.iter().filter(|(a, _)| crate::keybinds::is_inventory(*a)).cloned().collect(),
        }
    }

    fn bindings(&self, command: Command) -> impl Iterator<Item = &Binding> {
        let action = command.action();
        self.map
            .iter()
            .filter(move |(a, _)| Some(*a) == action)
            .flat_map(|(_, b)| b.iter().flatten())
    }

    /// The command a key press is, among the rebindable ones.
    pub(crate) fn command(&self, key: Key, physical: Option<Key>, m: Modifiers) -> Option<Command> {
        REBINDABLE
            .into_iter()
            .find(|c| self.bindings(*c).any(|b| b.matches_egui(key, physical, m)))
    }

    /// Whether a press of the physical key `code` with these modifiers is
    /// one of the list's keys (the viewer's shortcuts leave it to a
    /// focused list).
    pub(crate) fn keeps(&self, input: &Input, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.map
            .iter()
            .flat_map(|(_, b)| b.iter().flatten())
            .any(|b| &b.input == input && b.ctrl == ctrl && b.shift == shift && b.alt == alt)
    }

    /// The first binding's keys, for a tooltip or a menu; None when unbound.
    pub(crate) fn label(&self, command: Command) -> Option<String> {
        self.bindings(command).next().map(Binding::label)
    }
}

fn id() -> egui::Id {
    egui::Id::unique("inventory_shortcuts")
}

/// The user's table, for this frame's inventory lists.
pub(crate) fn publish(ctx: &egui::Context, kb: &KeyBindings) {
    let keys = InventoryKeys::of(kb);
    ctx.data_mut(|d| {
        if d.get_temp::<InventoryKeys>(id()).as_ref() != Some(&keys) {
            d.insert_temp(id(), keys);
        }
    });
}

/// The table published, else the defaults.
pub(crate) fn get(ctx: &egui::Context) -> InventoryKeys {
    ctx.data(|d| d.get_temp::<InventoryKeys>(id()))
        .unwrap_or_else(|| InventoryKeys::of(&KeyBindings::default()))
}

/// Whether a key pressed while an inventory list has the keyboard is one
/// of the list's (`app::shortcut_claims_focus` and the viewer's dispatch
/// leave it to the list then).
pub(crate) fn list_keeps(ctx: &egui::Context, input: &Input, ctrl: bool, shift: bool, alt: bool) -> bool {
    get(ctx).keeps(input, ctrl, shift, alt)
}

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

/// Takes this frame's key presses that are the list's commands out of the
/// input, in order, so that no other widget acts on them too.
pub(crate) fn take(ui: &egui::Ui, wanted: impl Fn(Command) -> bool) -> Vec<Command> {
    let keys = get(ui.ctx());
    let mut taken = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat,
                modifiers,
            } = event
            else {
                return true;
            };
            match keys.command(*key, *physical_key, *modifiers).filter(|c| wanted(*c)) {
                // a held key acts once, as Explorer's (Back held does not
                // run through the whole history); its repeats are spent
                Some(_) if *repeat => false,
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

/// The egui button of a bound mouse button (the middle, back and forward
/// ones; the left and right ones are never bound).
fn pointer_button(name: &str) -> Option<egui::PointerButton> {
    match name {
        "Middle" => Some(egui::PointerButton::Middle),
        "Back" => Some(egui::PointerButton::Extra1),
        "Forward" => Some(egui::PointerButton::Extra2),
        _ => None,
    }
}

/// The commands this frame's mouse-button presses give, among `wanted`
/// (the explorer's Back and Forward on its side buttons). The presses stay
/// in the input.
pub(crate) fn mouse(ui: &egui::Ui, wanted: impl Fn(Command) -> bool) -> Vec<Command> {
    let keys = get(ui.ctx());
    ui.input(|i| {
        REBINDABLE
            .into_iter()
            .filter(|c| wanted(*c))
            .filter(|c| {
                keys.bindings(*c).any(|b| match &b.input {
                    Input::Mouse(name) => pointer_button(name).is_some_and(|button| {
                        i.pointer.button_pressed(button)
                            && b.ctrl == crate::keybinds::control_down(i.modifiers.ctrl, i.modifiers.mac_cmd, true)
                            && b.shift == i.modifiers.shift
                            && b.alt == i.modifiers.alt
                    }),
                    Input::Key(_) => false,
                })
            })
            .collect()
    })
}

/// The keys of `command` (its first binding, or its fixed key), as the
/// key-binds table writes them ("Alt+Left", "⌘L").
pub(crate) fn label(ctx: &egui::Context, command: Command) -> Option<String> {
    match command.fixed() {
        Some(fixed) => Some(ctx.format_shortcut(&fixed)),
        None => get(ctx).label(command),
    }
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
    use winit::keyboard::KeyCode;

    const CTRL: Modifiers = Modifiers::COMMAND.plus(Modifiers::CTRL);

    fn defaults() -> InventoryKeys {
        InventoryKeys::of(&KeyBindings::default())
    }

    #[test]
    fn each_default_key_gives_its_command() {
        let keys = defaults();
        let hit = |key: Key, m: Modifiers| keys.command(key, Some(key), m);
        if !cfg!(target_os = "macos") {
            assert_eq!(hit(Key::ArrowLeft, Modifiers::ALT), Some(Command::Back));
            assert_eq!(hit(Key::ArrowUp, Modifiers::ALT), Some(Command::Up));
            // Dolphin's Backspace (a Mac's deletes)
            assert_eq!(hit(Key::Backspace, Modifiers::NONE), Some(Command::Up));
            assert_eq!(hit(Key::D, Modifiers::ALT), Some(Command::EditPath));
        }
        assert_eq!(hit(Key::A, CTRL), Some(Command::SelectAll));
        assert_eq!(hit(Key::A, CTRL.plus(Modifiers::SHIFT)), Some(Command::InvertSelection));
        assert_eq!(hit(Key::N, CTRL.plus(Modifiers::SHIFT)), Some(Command::NewFolder));
        assert_eq!(hit(Key::N, CTRL), None);
        assert_eq!(hit(Key::F10, Modifiers::SHIFT), Some(Command::ContextMenu));
        if cfg!(target_os = "macos") {
            // Finder's keys
            assert_eq!(hit(Key::I, CTRL), Some(Command::Properties));
            assert_eq!(hit(Key::Space, CTRL), None, "Spotlight's");
        } else {
            assert_eq!(hit(Key::Space, CTRL), Some(Command::ToggleSelection));
            assert_eq!(hit(Key::Delete, Modifiers::SHIFT), Some(Command::Purge));
            assert_eq!(hit(Key::Enter, Modifiers::ALT), Some(Command::Properties));
        }
        // the list's fixed keys are no command here
        assert_eq!(hit(Key::Delete, Modifiers::NONE), None);
        assert_eq!(hit(Key::Enter, Modifiers::NONE), None);
    }

    #[test]
    fn a_rebound_key_moves_with_the_users_table() {
        let mut kb = KeyBindings::default();
        let f6 = Binding::captured(Input::key(KeyCode::F6), Default::default());
        kb.set(Action::InvRefresh, 0, Some(f6));
        kb.set(Action::InvInvertSelection, 0, None);
        let keys = InventoryKeys::of(&kb);
        assert_eq!(keys.command(Key::F6, Some(Key::F6), Modifiers::NONE), Some(Command::Refresh));
        assert_eq!(keys.command(Key::F5, Some(Key::F5), Modifiers::NONE), None);
        assert_eq!(keys.command(Key::A, Some(Key::A), CTRL.plus(Modifiers::SHIFT)), None);
        assert!(keys.label(Command::InvertSelection).is_none());
        assert!(keys.label(Command::Refresh).is_some_and(|l| l.contains("F6")));
    }

    #[test]
    fn the_focused_list_keeps_its_bound_chords_from_the_viewer_shortcuts() {
        let keys = defaults();
        let key = |code: KeyCode| Input::key(code);
        // People, the sunset, the Texture Library, OOC chat elsewhere
        assert!(keys.keeps(&key(KeyCode::KeyA), true, true, false));
        assert!(keys.keeps(&key(KeyCode::KeyN), true, true, false));
        if !cfg!(target_os = "macos") {
            assert!(keys.keeps(&key(KeyCode::Space), true, false, false));
            assert!(keys.keeps(&key(KeyCode::Enter), false, false, true));
        }
        // the viewer's own stay with it
        assert!(!keys.keeps(&key(KeyCode::KeyT), true, false, false));
        assert!(!keys.keeps(&key(KeyCode::KeyI), true, true, false));
        // a chord the user unbinds goes back to the viewer
        let mut kb = KeyBindings::default();
        kb.set(Action::InvInvertSelection, 0, None);
        assert!(!InventoryKeys::of(&kb).keeps(&key(KeyCode::KeyA), true, true, false));
    }

    #[test]
    fn every_command_has_a_default_key() {
        let keys = defaults();
        // a Mac leaves Toggle selection (Spotlight's Cmd+Space) and Search
        // (Mission Control's F3) unbound
        let mac_free = |c: Command| cfg!(target_os = "macos") && matches!(c, Command::ToggleSelection | Command::Search);
        for command in REBINDABLE.into_iter().filter(|c| !mac_free(*c)) {
            assert!(keys.label(command).is_some(), "{command:?}");
        }
        assert!(REBINDABLE.iter().all(|c| c.action().is_some_and(crate::keybinds::is_inventory)));
    }

    #[test]
    fn a_tip_names_its_key() {
        let ctx = egui::Context::default();
        let tip = tip(&ctx, "Select all", Command::SelectAll);
        assert!(tip.starts_with("Select all (") && tip.ends_with(')'), "{tip}");
        assert!(label(&ctx, Command::Rename).is_some_and(|l| l.contains("F2")));
    }
}
