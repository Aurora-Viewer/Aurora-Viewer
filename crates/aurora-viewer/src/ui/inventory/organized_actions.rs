//! Native inventory operations from an Explorer selection. A local category has no SL parent UUID.
use super::super::{clipboard, context_catalog as native_menu, inventory_path, move_to::MoveTo};
use super::*;
use aurora_i18n::tr;

pub(super) struct Menu<'a> {
    pub native: &'a InventoryUi,
    pub prefs: &'a InventoryPreferences,
    pub facts: &'a Facts,
}

pub(super) enum Request {
    Native(native_menu::Command),
    NextOwner(Vec<Uuid>),
    Move(Vec<Uuid>),
    Reveal(Uuid),
    Paste(Location, bool),
    Create(Location, String),
}

pub(super) struct Destination {
    origin: Location,
    root: Uuid,
    clipboard: Vec<Uuid>,
    cut: bool,
    links: bool,
    selected: Option<Uuid>,
}

pub(super) fn ids(state: &ExplorerState) -> Option<Vec<Uuid>> {
    let mut ids: Vec<_> = state.here.selected.iter().map(|r| r.id()).collect::<Option<_>>()?;
    ids.sort_unstable();
    (!ids.is_empty()).then_some(ids)
}

// Restore changes every selected real parent. Mixed search results must not relocate
// items outside Trash, and restoring a selected folder carries its children with it.
fn restore_roots(inv: &Inventory, selected: &[Uuid]) -> Option<Vec<Uuid>> {
    (!selected.is_empty() && selected.iter().all(|id| rules::in_type(inv, *id, 14))).then(|| rules::roots(inv, selected))
}

fn target(state: &ExplorerState, row: Option<Row>) -> Location {
    match row {
        Some(Row::Place(place)) => place,
        _ => state.here.location,
    }
}

fn may_paste(menu: &Menu<'_>, inv: &Inventory, to: Uuid, links: bool) -> bool {
    !menu.native.clipboard.is_empty()
        && menu.native.pending.is_none()
        && model::personal_contents(inv, to)
        && !(menu.native.cut && menu.native.clipboard.iter().any(|id| model::client_managed(inv, *id)))
        && if links {
            !menu.native.cut
                && rules::paste_allowed(
                    inv,
                    &menu.native.clipboard,
                    to,
                    false,
                    true,
                    &menu.prefs.protected,
                    &menu.facts.worn,
                )
        } else {
            clipboard::paste_allowed(inv, menu.native, to, menu.prefs, menu.facts)
        }
}

pub(super) fn paste_enabled(menu: &Menu<'_>, inv: &Inventory, layout: &Preferences, place: Location, links: bool) -> bool {
    if menu.native.clipboard.is_empty() || matches!(place, Location::Home | Location::Library | Location::Group(Group::System)) {
        return false;
    }
    if let Some(id) = folder::fixed_parent(place, inv) {
        return may_paste(menu, inv, id, links);
    }
    folder::choices(inv, layout, place)
        .into_iter()
        .any(|to| may_paste(menu, inv, to, links))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Edit {
    Cut,
    Copy,
    Rename,
    Delete,
}

impl Edit {
    pub(super) fn allowed(self, inv: &Inventory, ids: &[Uuid], menu: &Menu<'_>, guarded: bool) -> bool {
        if ids.is_empty() || menu.native.pending.is_some() {
            return false;
        }
        match self {
            Self::Cut => {
                !guarded
                    && ids
                        .iter()
                        .all(|id| rules::relocatable(inv, *id, &menu.prefs.protected) && menu.facts.rlva.removable(inv, *id))
            }
            Self::Copy => clipboard::copy_allowed(inv, ids),
            Self::Rename => !guarded && ids.len() == 1 && rules::renameable(inv, ids[0]),
            // Permanent deletion remains an explicit Trash command.
            Self::Delete => {
                !guarded
                    && ids.iter().all(|id| !rules::in_type(inv, *id, 14))
                    && super::super::delete::allowed(inv, ids, menu.prefs, menu.facts)
            }
        }
    }

    pub(super) fn request(self, ids: Vec<Uuid>) -> Request {
        Request::Native(match self {
            Self::Cut | Self::Copy => native_menu::Command::Copy(ids, self == Self::Cut),
            Self::Rename => native_menu::Command::Rename(ids[0]),
            Self::Delete => native_menu::Command::Delete(ids, false),
        })
    }
}

// Inventory rename and Trash use a single, unmodified physical press.
fn plain_edit_key(event: &egui::Event) -> Option<Key> {
    match event {
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if modifiers.is_none() && matches!(key, Key::F2 | Key::Delete) => Some(*key),
        _ => None,
    }
}

pub(super) fn keyboard(ui: &Ui, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, menu: &Menu<'_>) {
    if state.file_request.is_some()
        || menu.native.pending.is_some()
        || menu.native.dialog.is_some()
        || menu.native.move_to.is_some()
        || menu.native.rename.is_some()
        || state.folder_draft.is_some()
        || state.paste_destination.is_some()
        || !state.list_focus.is_some_and(|id| ui.memory(|m| m.has_focus(id)))
        || !super::super::explorer_owns_keys(ui.ctx())
    {
        return;
    }
    // egui-winit delivers clipboard chords as clipboard events, not Key::C/X/V.
    let event = ui.input_mut(|i| {
        i.events
            .iter()
            .position(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)) || plain_edit_key(e).is_some())
            .map(|index| i.events.remove(index))
    });
    if state.dashboard_shortcuts(inv, layout) {
        return;
    }
    if let Some(egui::Event::Paste(text)) = &event {
        if clipboard::holds_marker(menu.native, text) {
            let selected = (state.here.selected.len() == 1)
                .then(|| state.here.selected.iter().next().copied())
                .flatten();
            let to = target(state, selected);
            if paste_enabled(menu, inv, layout, to, false) {
                state.file_request = Some(Request::Paste(to, false));
            } else {
                state.status = tr!("explorer-real-unavailable").into();
            }
        }
        return;
    }
    if event.is_none() {
        return;
    }
    let Some(selected) = ids(state) else { return };
    let guarded = selected.iter().any(|id| model::client_managed(inv, *id));
    let edit = match event {
        Some(egui::Event::Copy) => Edit::Copy,
        Some(egui::Event::Cut) => Edit::Cut,
        Some(ref event) if plain_edit_key(event) == Some(Key::F2) => Edit::Rename,
        Some(ref event) if plain_edit_key(event) == Some(Key::Delete) => Edit::Delete,
        _ => return,
    };
    if edit.allowed(inv, &selected, menu, guarded) {
        state.file_request = Some(edit.request(selected));
    } else {
        state.status = tr!("explorer-real-unavailable").into();
    }
}

pub(super) fn execute(
    ui: &Ui,
    inv: &Inventory,
    layout: &Preferences,
    state: &mut ExplorerState,
    native: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(request) = state.file_request.take() else { return };
    state.native_feedback = true;
    native.message.clear();
    match request {
        Request::Create(place, key) => {
            let files = Menu { native, prefs, facts };
            menus::create(inv, layout, state, &files, place, &key, actions);
        }
        Request::NextOwner(ids) => native.next_owner.open(inv.root, ids),
        Request::Native(native_menu::Command::Rename(id)) => {
            native.begin_rename(inv, id);
            native.reveal = None;
            native.visible.push(id);
            state.action_rename = Some(id);
            let row = if inv.folders.contains_key(&id) {
                Row::Place(Location::Folder(id))
            } else {
                Row::Item(id)
            };
            state.select_and_reveal(row);
        }
        Request::Native(native_menu::Command::DefaultFolder(selected)) => {
            if let Some(roots) = restore_roots(inv, &selected) {
                native_menu::execute(native_menu::Command::DefaultFolder(roots), ui, inv, native, prefs, facts, actions);
            } else {
                state.status = tr!("explorer-real-unavailable").into();
            }
        }
        Request::Native(native_menu::Command::Links(id)) => actions.push(InvAction::FindLinks(id)),
        Request::Native(native_menu::Command::Copy(ids, cut)) => {
            let guarded = ids.iter().any(|id| model::client_managed(inv, *id));
            let edit = if cut { Edit::Cut } else { Edit::Copy };
            if !edit.allowed(inv, &ids, &Menu { native, prefs, facts }, guarded)
                || !clipboard::offer(ui.ctx(), inv, native, ids, cut, &prefs.protected, &facts.rlva)
            {
                state.status = tr!("explorer-real-unavailable").into();
            }
        }
        Request::Native(command) => native_menu::execute(command, ui, inv, native, prefs, facts, actions),
        Request::Move(ids) => native.move_to = Some(MoveTo::explorer(inv.root, ids)),
        Request::Reveal(id) => {
            if let Some(parent) = rules::parent(inv, id) {
                let row = if inv.folders.contains_key(&id) {
                    Row::Place(Location::Folder(id))
                } else {
                    Row::Item(id)
                };
                show_parent(inv, state, row, parent);
            }
        }
        Request::Paste(place, links) => {
            let menu = Menu { native, prefs, facts };
            if !paste_enabled(&menu, inv, layout, place, links) {
                state.status = tr!("explorer-real-unavailable").into();
                return;
            }
            if let Some(to) = folder::fixed_parent(place, inv) {
                clipboard::paste(inv, to, links, native, prefs, facts, actions);
            } else {
                state.paste_destination = Some(Destination {
                    origin: place,
                    root: inv.root,
                    clipboard: native.clipboard.clone(),
                    cut: native.cut,
                    links,
                    selected: None,
                });
            }
        }
    }
}

pub(super) fn destination(
    ui: &mut Ui,
    inv: &Inventory,
    layout: &Preferences,
    state: &mut ExplorerState,
    native: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(mut picker) = state.paste_destination.take() else { return };
    if picker.root != inv.root || picker.clipboard != native.clipboard || picker.cut != native.cut {
        return;
    }
    let menu = Menu { native, prefs, facts };
    let choices: Vec<_> = folder::choices(inv, layout, picker.origin)
        .into_iter()
        .filter(|id| may_paste(&menu, inv, *id, picker.links))
        .collect();
    if picker.selected.is_some_and(|id| !choices.contains(&id)) {
        picker.selected = None;
    }
    ui.label(tr!("explorer-real-paste-in"));
    egui::ComboBox::from_id_salt("explorer-native-paste-target")
        .selected_text(
            picker
                .selected
                .map_or_else(|| tr!("explorer-create-choose-parent").to_string(), |id| inventory_path(inv, id)),
        )
        .show_ui(ui, |ui| {
            for id in choices {
                ui.selectable_value(&mut picker.selected, Some(id), inventory_path(inv, id));
            }
        });
    let mut close = false;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(picker.selected.is_some(), egui::Button::new(tr!("common-paste")))
            .clicked()
            && let Some(to) = picker.selected
        {
            clipboard::paste(inv, to, picker.links, native, prefs, facts, actions);
            close = true;
        }
        close |= ui.button(tr!("common-cancel")).clicked();
    });
    if !close {
        state.paste_destination = Some(picker);
    }
}

pub(super) fn show_parent(inv: &Inventory, state: &mut ExplorerState, row: Row, parent: Uuid) {
    state.visit(
        if parent == inv.root {
            Location::FullTree
        } else {
            Location::Folder(parent)
        },
        true,
    );
    state.clear_filter();
    state.select_and_reveal(row);
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::operations::Operation;

    #[test]
    fn explorer_cut_revalidates_folder_locks_before_touching_clipboard() {
        let inv = super::super::tests::fixture();
        let item = id(20);
        let mut state = ExplorerState::at(Location::Folder(id(3)));
        let mut native = InventoryUi {
            clipboard: vec![id(21)],
            ..Default::default()
        };
        let mut prefs = InventoryPreferences::default();
        let mut facts = Facts::default();
        assert!(Edit::Cut.allowed(
            &inv,
            &[item],
            &Menu {
                native: &native,
                prefs: &prefs,
                facts: &facts
            },
            false
        ));
        state.file_request = Some(Edit::Cut.request(vec![item]));
        facts.rlva = std::sync::Arc::new(crate::world::rlva::WearLocks::holding_folders([inv.items[&item].parent]));
        assert!(!Edit::Cut.allowed(
            &inv,
            &[item],
            &Menu {
                native: &native,
                prefs: &prefs,
                facts: &facts
            },
            false
        ));
        let mut actions = vec![];
        let mut frame = egui::Context::default().run_ui(Default::default(), |ui| {
            execute(
                ui,
                &inv,
                &Preferences::default(),
                &mut state,
                &mut native,
                &mut prefs,
                &facts,
                &mut actions,
            );
        });
        frame.textures_delta.clear();
        assert_eq!(native.clipboard, vec![id(21)]);
        assert!(!native.cut && actions.is_empty());
        facts.rlva = std::sync::Arc::default();
        state.file_request = Some(Edit::Cut.request(vec![item]));
        let mut frame = egui::Context::default().run_ui(Default::default(), |ui| {
            execute(
                ui,
                &inv,
                &Preferences::default(),
                &mut state,
                &mut native,
                &mut prefs,
                &facts,
                &mut actions,
            );
        });
        frame.textures_delta.clear();
        assert_eq!(native.clipboard, vec![item]);
        assert!(native.cut);
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn explorer_rename_and_trash_require_an_exact_unrepeated_press() {
        use egui::Modifiers;
        for key in [Key::F2, Key::Delete] {
            let event = |pressed, repeat, modifiers| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat,
                modifiers,
            };
            assert_eq!(plain_edit_key(&event(true, false, Modifiers::NONE)), Some(key));
            for modifiers in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::CTRL, Modifiers::COMMAND] {
                assert_eq!(plain_edit_key(&event(true, false, modifiers)), None);
            }
            assert_eq!(plain_edit_key(&event(true, true, Modifiers::NONE)), None);
            assert_eq!(plain_edit_key(&event(false, false, Modifiers::NONE)), None);
        }
    }

    #[test]
    fn explorer_restore_requires_all_entries_in_trash_and_keeps_children_with_their_selected_parent() {
        let mut inv = super::super::tests::fixture();
        inv.folders.get_mut(&id(4)).unwrap().info.type_default = 14;
        // Workshop is in Trash, and Bench notes is inside Workshop. Settings is outside Trash.
        inv.folders.get_mut(&id(3)).unwrap().info.parent = id(4);
        assert_eq!(restore_roots(&inv, &[id(20), id(3)]), Some(vec![id(3)]));
        assert_eq!(restore_roots(&inv, &[id(20)]), Some(vec![id(20)]));
        assert_eq!(restore_roots(&inv, &[id(5), id(20)]), None);
        assert_eq!(restore_roots(&inv, &[]), None);
    }

    #[test]
    fn explorer_clipboard_uses_permissions_real_parents_and_account_identity() {
        let mut inv = super::super::tests::fixture();
        let mut native = InventoryUi::default();
        let mut prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let layout = Preferences::default();
        let mut state = ExplorerState::at(Location::Folder(id(4)));
        let to = Location::Folder(id(4));
        let mut actions = vec![];
        native.enter_clipboard_account(inv.root);
        native.clipboard = vec![id(20)]; // A no-copy note in Workshop.
        let menu = Menu {
            native: &native,
            prefs: &prefs,
            facts: &facts,
        };
        assert!(!paste_enabled(&menu, &inv, &layout, to, false));
        assert!(paste_enabled(&menu, &inv, &layout, to, true));
        assert!(!paste_enabled(&menu, &inv, &layout, Location::Library, true));
        assert!(!paste_enabled(&menu, &inv, &layout, Location::Home, true));
        assert!(!paste_enabled(&menu, &inv, &layout, Location::Group(Group::System), true));

        let mut frame = egui::Context::default().run_ui(Default::default(), |ui| {
            state.file_request = Some(Request::Paste(to, true));
            execute(ui, &inv, &layout, &mut state, &mut native, &mut prefs, &facts, &mut actions);
        });
        frame.textures_delta.clear();
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if matches!(&change.operations[..],
            [Operation::Links{parent,links}] if *parent==id(4) && links[0].target==id(20))));
        assert_eq!(inv.items[&id(20)].parent, id(3));

        native.cut = true;
        let menu = Menu {
            native: &native,
            prefs: &prefs,
            facts: &facts,
        };
        assert!(paste_enabled(&menu, &inv, &layout, to, false));
        assert!(!paste_enabled(&menu, &inv, &layout, to, true));
        inv.folders.get_mut(&id(3)).unwrap().info.name = "#RLV".into();
        assert!(!may_paste(&menu, &inv, id(4), false));
        native.enter_clipboard_account(id(999));
        assert!(native.clipboard.is_empty() && !native.cut);
    }

    #[test]
    fn explorer_group_paste_waits_for_an_explicit_real_folder_and_cancels_stale_clipboard() {
        let inv = super::super::tests::fixture();
        let mut native = InventoryUi {
            clipboard: vec![id(20)],
            ..Default::default()
        };
        let mut prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let layout = Preferences::default();
        let group = Location::Group(Group::Appearance);
        let mut state = ExplorerState::at(group);
        let mut actions = vec![];
        let mut frame = egui::Context::default().run_ui(Default::default(), |ui| {
            state.file_request = Some(Request::Paste(group, true));
            execute(ui, &inv, &layout, &mut state, &mut native, &mut prefs, &facts, &mut actions);
            let draft = state.paste_destination.as_ref().unwrap();
            assert_eq!(draft.root, inv.root);
            assert!(draft.selected.is_none());
            native.clipboard.clear();
            destination(ui, &inv, &layout, &mut state, &mut native, &prefs, &facts, &mut actions);
        });
        frame.textures_delta.clear();
        assert!(actions.is_empty() && state.paste_destination.is_none());
    }

    #[test]
    fn explorer_real_actions_keep_virtual_rows_out_of_bulk_and_reveal_exact_record() {
        let inv = super::super::tests::fixture();
        let mut state = ExplorerState::at(Location::Home);
        state
            .here
            .selected
            .extend([Row::Item(id(20)), Row::Place(Location::Group(Group::Appearance))]);
        assert!(ids(&state).is_none());
        show_parent(&inv, &mut state, Row::Item(id(20)), id(3));
        assert!(state.here.raw);
        assert_eq!(state.here.location, Location::Folder(id(3)));
        assert_eq!(ids(&state), Some(vec![id(20)]));
        assert_eq!(state.reveal_row, Some(Row::Item(id(20))));
    }
}
