//! LLInventoryPanel::handleKeyHere and LLPanelMainInventory::onTrashButtonClick
//! (Firestorm indra/newview, originally LGPL 2.1): delete uses the same selected
//! inventory action as the context menu, with no keyboard modifiers.

use super::*;
use aurora_net::inventory::operations::Operation;

pub(super) fn allowed(inv: &Inventory, ids: &[Uuid], prefs: &InventoryPreferences, facts: &Facts) -> bool {
    if ids.is_empty() {
        return false;
    }
    let holds_worn = rules::worn_ancestors(inv, &facts.worn);
    ids.iter()
        .all(|id| rules::removable(inv, *id, &prefs.protected) && !holds_worn.contains(id))
}

/// Build the mutation for a confirmed Delete: Trash move, or permanent purge
/// when every selected entry is already in the Trash.
pub(super) fn plan(inv: &Inventory, ids: &[Uuid], purge: bool, prefs: &InventoryPreferences, facts: &Facts) -> Result<Mutation, String> {
    if purge {
        let ids = rules::roots(inv, ids);
        let refresh = ids.iter().filter_map(|id| rules::parent(inv, *id)).collect();
        let operations = ids
            .iter()
            .map(|id| Operation::Delete {
                id: *id,
                folder: inv.folders.contains_key(id),
            })
            .collect();
        let holds_worn = rules::worn_ancestors(inv, &facts.worn);
        if ids
            .iter()
            .all(|id| rules::removable(inv, *id, &prefs.protected) && rules::in_type(inv, *id, 14) && !holds_worn.contains(id))
        {
            Ok(Mutation {
                operations,
                refresh,
                removed: ids,
                ..Default::default()
            })
        } else {
            Err(aurora_i18n::tr!("ui-inventory-dialogs-the-selection-can-no-longer-be-purged").into())
        }
    } else {
        rules::system(inv, 14)
            .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
            .and_then(|to| rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn))
    }
}

/// Open the Delete confirmation, or apply a Trash move immediately when the
/// preference is off. Purge from Trash always asks (PurgeSelectedItems has no
/// ignore box). `DeleteItems` notification, indra/newview/skins/.../notifications.xml.
pub(super) fn confirm_or_run(
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    mut ids: Vec<Uuid>,
) {
    if st.pending.is_some() || st.pending_delete.is_some() || st.dialog.is_some() {
        return;
    }
    ids.sort();
    if !allowed(inv, &ids, prefs, facts) {
        return;
    }
    // A selection spanning trash and another folder first moves to trash.
    // Permanent deletion is offered only when every selected entry is there.
    let purge = ids.iter().all(|id| rules::in_type(inv, *id, 14));
    if purge || prefs.confirm_delete {
        st.dialog = Some(EditDialog::Delete(ids, purge));
        return;
    }
    match plan(inv, &ids, false, prefs, facts) {
        Ok(change) => actions.push(InvAction::Delete(Box::new(DeleteRequest::new(inv, st, &ids, change)))),
        Err(reason) => st.message = reason,
    }
}

pub(super) fn request(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts, actions: &mut Vec<InvAction>) {
    let ids: Vec<_> = st.selection.iter().copied().collect();
    confirm_or_run(inv, st, prefs, facts, actions, ids);
}

/// LLInventoryPanel::handleKeyHere: on a Mac the key labelled Delete sends
/// Backspace, so both delete there (`#if LL_DARWIN`); elsewhere Backspace does
/// nothing in the list.
pub(super) fn deletes(key: egui::Key, mac: bool) -> bool {
    key == egui::Key::Delete || (mac && key == egui::Key::Backspace)
}

pub(super) fn shortcut(
    ui: &mut egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    // Called only by the focused list's shortcut dispatcher. Text editors,
    // popups and other inventory windows retain their own Delete handling.
    let pressed = ui.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            if matches!(event, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none() && deletes(*key, cfg!(target_os = "macos")))
            {
                pressed = true;
                false
            } else {
                true
            }
        });
        pressed
    });
    if pressed {
        request(inv, st, prefs, facts, actions);
    }
}

pub(super) fn button(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let ids: Vec<_> = st.selection.iter().copied().collect();
    let enabled = st.pending.is_none() && st.pending_delete.is_none() && st.dialog.is_none() && allowed(inv, &ids, prefs, facts);
    let button =
        ui.scope(|ui| super::super::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-delete-shortcut"), enabled));
    if cfg!(test) || demo_mode().is_some() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_delete_button", st.ui_id)), button.response.rect));
    }
    if button.inner {
        request(inv, st, prefs, facts, actions);
    }
}

fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_INVENTORY")
            .ok()
            .filter(|mode| mode.starts_with("delete-") || mode.starts_with("aurora-delete-"))
    })
    .as_deref()
}

pub(super) fn demo_enabled() -> bool {
    demo_mode().is_some()
}

/// Actual list selection, followed by a native Delete event or footer click.
pub fn demo_events(ctx: &egui::Context, frame: u64, st: &InventoryUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode() else { return Vec::new() };
    let aurora = mode.starts_with("aurora-");
    let width = if mode.ends_with("-wide") {
        1180.0
    } else if mode.ends_with("-medium") {
        800.0
    } else if aurora {
        440.0
    } else {
        680.0
    };
    let mode = mode.strip_prefix("aurora-").unwrap_or(mode);
    let mode = mode.strip_suffix("-wide").or_else(|| mode.strip_suffix("-medium")).unwrap_or(mode);
    if frame == 640 {
        super::super::widgets::place_floater(
            ctx,
            "inventory",
            egui::pos2(20.0, 56.0),
            egui::vec2(width, if aurora { 720.0 } else { 850.0 }),
        );
    }
    // `delete-enter`: the Delete key, then Enter answers the confirmation
    let key = match (mode, frame) {
        ("delete-key" | "delete-enter" | "delete-tab", 680) => Some(egui::Key::Delete),
        ("delete-tab", 710) => Some(egui::Key::Tab),
        ("delete-confirm", 680) if aurora => Some(egui::Key::Delete),
        ("delete-confirm", 740) if aurora => Some(egui::Key::Enter),
        ("delete-enter" | "delete-tab", 740) => Some(egui::Key::Enter),
        _ => None,
    };
    if let Some(key) = key {
        return vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }];
    }
    let pos = match frame {
        650 | 651 => {
            let target = if aurora { 8100 } else { 8001 };
            ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", st.ui_id, Uuid::from_u128(target)))))
        }
        680 | 681 if !aurora && matches!(mode, "delete-button" | "delete-confirm") => ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_delete_button", st.ui_id))))
            .map(|rect| rect.center()),
        // `delete-confirm`: the trash button at 680, then the confirmation's Delete button
        740 | 741 if mode == "delete-confirm" && !aurora => {
            ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("inventory_delete_confirm", st.ui_id))))
        }
        _ => None,
    };
    if mode == "delete-confirm" && matches!(frame, 720 | 780) {
        log::info!(
            "demo inventory delete-confirm: frame={frame}, dialog_open={}, selected={:?}",
            st.dialog.is_some(),
            st.selected.map(|id| id.as_u128())
        );
    }
    pos.map_or_else(Vec::new, |pos| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(frame, 650 | 680 | 740),
                modifiers: Default::default(),
            },
        ]
    })
}

/// The first displayed row after `last` that does not go with the deletion,
/// else the nearest one before it. Unlike getNextOpenNode, which skips only
/// the selected rows going down, a row inside any deleted folder is skipped
/// both ways: it leaves with its folder.
pub(super) fn next_row(inv: &Inventory, order: &[Uuid], gone: &std::collections::HashSet<Uuid>, last: Uuid) -> Option<Uuid> {
    let index = order.iter().position(|row| *row == last)?;
    let stays = |row: &&Uuid| !rules::ancestors(inv, **row).any(|id| gone.contains(&id));
    order[index + 1..]
        .iter()
        .find(stays)
        .or_else(|| order[..index].iter().rev().find(stays))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn skipping_confirm_moves_to_trash_without_a_dialog() {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.folders.insert(
            id(1),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: id(1),
                    parent: Uuid::nil(),
                    type_default: 8,
                    name: "Root".into(),
                    ..Default::default()
                },
                children: vec![id(14)],
                items: vec![id(10)],
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.insert(
            id(14),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: id(14),
                    parent: id(1),
                    type_default: 14,
                    name: "Trash".into(),
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        inv.items.insert(
            id(10),
            aurora_net::inventory::InvItem {
                id: id(10),
                parent: id(1),
                name: "Shirt".into(),
                asset_type: 6,
                owner_mask: 0x7fff_ffff,
                ..Default::default()
            },
        );
        let prefs = InventoryPreferences {
            confirm_delete: false,
            ..Default::default()
        };
        let facts = Facts::default();
        let mut st = InventoryUi {
            selection: HashSet::from([id(10)]),
            selected: Some(id(10)),
            ..Default::default()
        };
        let mut actions = Vec::new();
        request(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(st.dialog.is_none(), "no confirmation when the preference is off");
        assert!(matches!(actions.as_slice(), [InvAction::Delete(request)] if request.change.move_only()));
    }
}

/// A Delete keeps its originating window and selection through any additional
/// Marketplace confirmation. No speculative selection is applied on submission.
#[derive(Debug, Clone)]
pub struct DeleteRequest {
    pub change: Mutation,
    pub(crate) selection: DeleteSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteSelection {
    window: Uuid,
    root: Uuid,
    selected: Option<Uuid>,
    selection: HashSet<Uuid>,
    anchor: Option<Uuid>,
    view: DeleteView,
    moves: Vec<(Uuid, Uuid)>,
    deleted: HashSet<Uuid>,
    candidates: Vec<Uuid>,
    next: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeleteView {
    list: Option<ListKey>,
    order: keyboard::ShownOrder,
    search: String,
    filters: view::Filters,
    browse: Uuid,
    scope: Uuid,
    sort: Option<view::Sort>,
}

impl DeleteView {
    fn of(st: &InventoryUi) -> Self {
        Self {
            list: st.last_list.map(|(_, key)| key),
            order: st.shown_order,
            search: st.search.clone(),
            filters: st.filters.clone(),
            browse: st.browse,
            scope: st.scope,
            sort: st.view.selection_sort(),
        }
    }

    fn matches(&self, st: &InventoryUi) -> bool {
        self.list == st.last_list.map(|(_, key)| key)
            && self.order == st.shown_order
            && self.search == st.search
            && self.filters == st.filters
            && self.browse == st.browse
            && self.scope == st.scope
            && self.sort == st.view.selection_sort()
    }
}

/// Bound before sending: demo mode may deliver the result synchronously.
pub(super) struct PendingDelete {
    request: Uuid,
    selection: DeleteSelection,
    accepted: bool,
    observe_moves: bool,
    generation: Option<u64>,
}

impl DeleteRequest {
    pub(crate) fn new(inv: &Inventory, st: &InventoryUi, ids: &[Uuid], change: Mutation) -> Self {
        let gone: HashSet<_> = ids.iter().copied().collect();
        let order = keyboard::shown_order(st);
        let last = st
            .selected
            .filter(|id| gone.contains(id))
            .or_else(|| order.iter().rev().find(|id| gone.contains(id)).copied());
        let next = last.and_then(|last| next_row(inv, order, &gone, last));
        // Watch only actual moves, to their requested destination. A mixed
        // selection can include roots already in Trash, which need no update.
        let moves = change
            .operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Patch { id, body, .. } if body.has("parent_id") => {
                    let to = body["parent_id"].as_uuid();
                    (rules::parent(inv, *id) != Some(to)).then_some((*id, to))
                }
                _ => None,
            })
            .collect();
        // LLFolderView::getNextUnselectedItem (indra/llui/llfolderview.cpp):
        // below the last selected row, then above it. Skip whole deleted
        // subtrees too; their descendants leave the list with their parent.
        let candidates = last
            .and_then(|last| order.iter().position(|id| *id == last))
            .map_or_else(Vec::new, |index| {
                order[index + 1..].iter().chain(order[..index].iter().rev()).copied().collect()
            });
        Self {
            change,
            selection: DeleteSelection {
                window: st.ui_id,
                root: inv.root,
                selected: st.selected,
                selection: st.selection.clone(),
                anchor: st.anchor,
                view: DeleteView::of(st),
                moves,
                deleted: gone,
                candidates,
                next,
            },
        }
    }
}

impl InventoryUi {
    /// Called only for the mutation that carries this Delete's context.
    pub(crate) fn begin_delete(&mut self, request: Uuid, selection: DeleteSelection) {
        let window = selection.window;
        let target = if self.ui_id == window {
            Some(self)
        } else {
            self.windows.iter_mut().find(|w| w.id == window && w.open).map(|w| &mut *w.state)
        };
        if let Some(target) = target {
            target.pending_delete = Some(Box::new(PendingDelete {
                request,
                selection,
                accepted: false,
                observe_moves: false,
                generation: None,
            }));
        }
    }

    pub(crate) fn finish_delete(&mut self, request: Uuid, success: bool, moves_queued: bool) {
        fn finish(st: &mut InventoryUi, request: Uuid, success: bool, moves_queued: bool) {
            let Some(pending) = st.pending_delete.as_mut().filter(|pending| pending.request == request) else {
                return;
            };
            if success {
                pending.accepted = true;
                pending.observe_moves = moves_queued;
            } else {
                st.pending_delete = None;
            }
        }
        finish(self, request, success, moves_queued);
        for window in &mut self.windows {
            finish(&mut window.state, request, success && window.open, moves_queued);
        }
    }
}

/// Run after rebuilding the actual displayed order, before laying out rows.
/// Returning true uses the existing arrow-key scrolling path to show the row.
pub(super) fn reconcile_selection(inv: &Inventory, st: &mut InventoryUi) -> bool {
    let Some(pending) = st.pending_delete.as_ref() else { return false };
    let saved = &pending.selection;
    if saved.root != inv.root
        || saved.selected != st.selected
        || saved.selection != st.selection
        || saved.anchor != st.anchor
        || !saved.view.matches(st)
    {
        st.pending_delete = None;
        return false;
    }
    if !pending.accepted || pending.generation == Some(inv.generation) {
        return false;
    }
    let pending = st.pending_delete.as_mut().expect("checked pending deletion");
    pending.generation = Some(inv.generation);
    // Reliable UDP moves are queued, not acknowledged deletions. Wait until
    // the inventory model observes every requested move (or a root is gone).
    if pending.observe_moves
        && pending
            .selection
            .moves
            .iter()
            .any(|(id, to)| rules::parent(inv, *id).is_some_and(|parent| parent != *to))
    {
        return false;
    }
    let pending = st.pending_delete.take().expect("checked pending deletion");
    let shown: HashSet<_> = keyboard::shown_order(st).iter().copied().collect();
    let next = pending
        .selection
        .next
        .into_iter()
        .chain(pending.selection.candidates.iter().copied())
        .find(|id| shown.contains(id) && !rules::ancestors(inv, *id).any(|ancestor| pending.selection.deleted.contains(&ancestor)));
    st.selection.clear();
    st.selection.extend(next);
    st.selected = next;
    st.anchor = next;
    st.keyboard_selection = true;
    true
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn trash_move(ids: &[Uuid]) -> Mutation {
        Mutation {
            operations: ids
                .iter()
                .map(|entry| Operation::Patch {
                    id: *entry,
                    folder: true,
                    body: aurora_llsd::llsd_map! { "parent_id" => id(14) },
                })
                .collect(),
            ..Default::default()
        }
    }

    fn move_folder(inv: &mut Inventory, entry: Uuid, to: Uuid) {
        let mut folder = inv.folders[&entry].info.clone();
        folder.parent = to;
        inv.server_updated(vec![folder], Vec::new());
    }

    fn fixture() -> (Inventory, InventoryUi, DeleteRequest) {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.created_folders(
            [(1, 0), (10, 1), (11, 10), (20, 1), (30, 1), (14, 1)]
                .map(|(n, parent)| aurora_net::inventory::InvFolder {
                    id: id(n),
                    parent: id(parent),
                    type_default: -1,
                    ..Default::default()
                })
                .to_vec(),
        );
        let mut st = InventoryUi {
            selected: Some(id(10)),
            selection: HashSet::from([id(10)]),
            anchor: Some(id(10)),
            ..Default::default()
        };
        st.plain.order = vec![id(10), id(11), id(20), id(30)];
        st.view.prepare(&inv, view::Sort::default());
        let deletion = DeleteRequest::new(&inv, &st, &[id(10)], trash_move(&[id(10)]));
        (inv, st, deletion)
    }

    #[test]
    fn deletion_keeps_selection_until_the_matching_success_and_skips_descendants() {
        let (inv, mut st, deletion) = fixture();
        st.begin_delete(id(90), deletion.selection);
        assert!(!reconcile_selection(&inv, &mut st));
        st.finish_delete(id(91), true, false);
        assert!(!reconcile_selection(&inv, &mut st));
        assert_eq!(st.selection, HashSet::from([id(10)]));
        st.finish_delete(id(90), true, false);
        st.plain.order = vec![id(20), id(30)];
        assert!(reconcile_selection(&inv, &mut st));
        assert_eq!(st.selection, HashSet::from([id(20)]));
        assert_eq!(st.anchor, Some(id(20)));
        assert_eq!(st.selected, Some(id(20)));
        assert!(st.keyboard_selection);
    }

    #[test]
    fn failed_delete_retains_the_original_selection() {
        let (inv, mut st, deletion) = fixture();
        st.begin_delete(id(90), deletion.selection);
        st.finish_delete(id(90), false, false);
        assert!(!reconcile_selection(&inv, &mut st));
        assert_eq!(st.selected, Some(id(10)));
        assert_eq!(st.selection, HashSet::from([id(10)]));
        assert!(st.pending_delete.is_none());
    }

    #[test]
    fn queued_moves_wait_for_the_inventory_model() {
        let (mut inv, mut st, deletion) = fixture();
        st.begin_delete(id(90), deletion.selection);
        st.finish_delete(id(90), true, true);
        assert!(!reconcile_selection(&inv, &mut st));
        inv.generation += 1;
        assert!(!reconcile_selection(&inv, &mut st), "an unrelated update is not the move");
        move_folder(&mut inv, id(10), id(14));
        st.plain.order = vec![id(20), id(30)];
        assert!(reconcile_selection(&inv, &mut st));
        assert_eq!(st.selected, Some(id(20)));
    }

    #[test]
    fn late_success_preserves_a_new_selection_or_view() {
        for change in ["selection", "search", "folder", "session", "sort"] {
            let (mut inv, mut st, deletion) = fixture();
            st.begin_delete(id(90), deletion.selection);
            match change {
                "selection" => {
                    st.selected = Some(id(30));
                    st.selection = HashSet::from([id(30)]);
                }
                "search" => st.search = "another search".into(),
                "folder" => st.browse = id(30),
                "session" => inv.root = id(2),
                _ => st.view.prepare(
                    &inv,
                    view::Sort {
                        by_date: false,
                        ..Default::default()
                    },
                ),
            }
            let before = (st.selected, st.selection.clone());
            st.finish_delete(id(90), true, false);
            assert!(!reconcile_selection(&inv, &mut st), "{change}");
            assert_eq!((st.selected, st.selection.clone()), before, "{change}");
            assert!(st.pending_delete.is_none(), "{change}");
        }
    }

    #[test]
    fn concurrent_removal_uses_the_next_surviving_candidate_or_clears() {
        for survivor in [Some(id(30)), None] {
            let (inv, mut st, deletion) = fixture();
            st.begin_delete(id(90), deletion.selection);
            st.finish_delete(id(90), true, false);
            st.plain.order = survivor.into_iter().collect();
            assert!(reconcile_selection(&inv, &mut st));
            assert_eq!(st.selected, survivor);
            assert_eq!(st.selection, survivor.into_iter().collect::<HashSet<_>>());
            assert_eq!(st.anchor, survivor);
        }
    }

    #[test]
    fn a_delete_result_belongs_to_its_window_and_a_closed_window_drops_it() {
        for open in [true, false] {
            let (inv, mut child, _) = fixture();
            child.ui_id = id(55);
            let deletion = DeleteRequest::new(&inv, &child, &[id(10)], Mutation::default());
            let mut main = InventoryUi {
                selected: Some(id(30)),
                selection: HashSet::from([id(30)]),
                ..Default::default()
            };
            main.windows.push(InventoryWindow {
                id: id(55),
                root: inv.root,
                additional: false,
                open: true,
                state: Box::new(child),
            });
            main.begin_delete(id(90), deletion.selection);
            main.windows[0].open = open;
            main.finish_delete(id(90), true, false);
            assert!(!reconcile_selection(&inv, &mut main));
            assert_eq!(main.selected, Some(id(30)));
            let child = &mut main.windows[0].state;
            child.plain.order = vec![id(20), id(30)];
            assert_eq!(reconcile_selection(&inv, child), open);
            assert_eq!(child.selected, Some(id(if open { 20 } else { 10 })));
            assert!(child.pending_delete.is_none());
        }
    }

    #[test]
    fn a_queued_batch_waits_for_every_selected_root() {
        let (mut inv, mut st, _) = fixture();
        st.selected = Some(id(20));
        st.selection.insert(id(20));
        let deletion = DeleteRequest::new(&inv, &st, &[id(10), id(20)], trash_move(&[id(10), id(20)]));
        st.begin_delete(id(90), deletion.selection);
        st.finish_delete(id(90), true, true);
        move_folder(&mut inv, id(10), id(14));
        assert!(!reconcile_selection(&inv, &mut st), "a partially observed batch must wait");
        assert_eq!(st.selection, HashSet::from([id(10), id(20)]));
        move_folder(&mut inv, id(20), id(14));
        st.plain.order = vec![id(30)];
        assert!(reconcile_selection(&inv, &mut st));
        assert_eq!(st.selection, HashSet::from([id(30)]));
    }

    #[test]
    fn a_queued_mixed_selection_does_not_wait_for_a_root_already_in_trash() {
        let (mut inv, mut st, _) = fixture();
        move_folder(&mut inv, id(10), id(14));
        st.selected = Some(id(20));
        st.selection.insert(id(20));
        let deletion = DeleteRequest::new(&inv, &st, &[id(10), id(20)], trash_move(&[id(10), id(20)]));
        assert_eq!(deletion.selection.moves, vec![(id(20), id(14))]);
        st.begin_delete(id(90), deletion.selection);
        st.finish_delete(id(90), true, true);
        assert!(!reconcile_selection(&inv, &mut st));
        move_folder(&mut inv, id(20), id(14));
        st.plain.order = vec![id(30)];
        assert!(reconcile_selection(&inv, &mut st));
        assert_eq!(st.selected, Some(id(30)));
    }

    #[test]
    fn a_queued_delete_waits_for_its_requested_destination() {
        let (mut inv, mut st, deletion) = fixture();
        st.begin_delete(id(90), deletion.selection);
        st.finish_delete(id(90), true, true);
        move_folder(&mut inv, id(10), id(30));
        assert!(
            !reconcile_selection(&inv, &mut st),
            "an unrelated move must not complete the queued deletion"
        );
        move_folder(&mut inv, id(10), id(14));
        st.plain.order = vec![id(20), id(30)];
        assert!(reconcile_selection(&inv, &mut st));
        assert_eq!(st.selected, Some(id(20)));
    }
}
