//! A large selection deleted from a large inventory (#1580): the checks of
//! the Delete command, its menu and its confirmation stay well under a frame.

use super::*;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::{InvFolder, InvItem};
use std::time::Instant;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const ROOT: u128 = 1;
const TRASH: u128 = 2;
const BIG: u128 = 3;

fn folder(inv: &mut Inventory, me: Uuid, parent: Uuid, type_default: i32) {
    inv.folders.insert(
        me,
        Folder {
            info: InvFolder {
                id: me,
                parent,
                type_default,
                name: format!("Folder {}", me.as_u128()),
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    if let Some(p) = inv.folders.get_mut(&parent) {
        p.children.push(me);
    }
}

fn item(inv: &mut Inventory, me: Uuid, parent: Uuid) {
    inv.items.insert(
        me,
        InvItem {
            id: me,
            parent,
            name: format!("Item {}", me.as_u128()),
            asset_type: 6,
            owner_mask: 0x7fff_ffff,
            ..Default::default()
        },
    );
    if let Some(p) = inv.folders.get_mut(&parent) {
        p.items.push(me);
    }
}

fn received() -> Inventory {
    let mut inv = Inventory {
        root: id(ROOT),
        ..Default::default()
    };
    folder(&mut inv, id(ROOT), Uuid::nil(), 8);
    folder(&mut inv, id(TRASH), id(ROOT), 14);
    folder(&mut inv, id(10), id(ROOT), 50);
    folder(&mut inv, id(20), id(10), -1);
    folder(&mut inv, id(21), id(20), -1);
    item(&mut inv, id(30), id(21));
    item(&mut inv, id(31), id(10));
    inv.items.get_mut(&id(30)).expect("received item").owner_mask = 0;
    inv
}

#[test]
fn explorer_finishes_one_trash_move_and_accepts_the_next_while_its_main_or_detached_window_is_open() {
    use crate::ui::widgets::elide_tests::Frames;
    let mut frames = Frames::new();
    let theme = crate::theme::Theme::default();
    theme.apply(&frames.ctx, 1.0);
    let palette = theme.palette();
    let icons = Icons::default();
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut prefs = InventoryPreferences {
        confirm_delete: false,
        ..Default::default()
    };
    let facts = Facts::default();
    let request = |inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, item: Uuid| {
        let mut actions = Vec::new();
        delete::confirm_or_run(inv, st, prefs, &facts, &mut actions, vec![item]);
        actions
    };
    for detached in [false, true] {
        world.inventory = received();
        for item_id in [40, 41] {
            item(&mut world.inventory, id(item_id), id(ROOT));
        }
        let mut st = InventoryUi {
            selected: Some(id(30)),
            selection: HashSet::from([id(30)]),
            anchor: Some(id(30)),
            ..Default::default()
        };
        let mut open = !detached;
        if detached {
            st.open_additional_window(&frames.ctx, InventoryStyle::Explorer, id(ROOT));
        }
        let mut draw = |world: &mut World, st: &mut InventoryUi, prefs: &mut InventoryPreferences| {
            frames.run(vec![], |ui| {
                show(
                    ui.ctx(),
                    &palette,
                    &icons,
                    world,
                    st,
                    None,
                    prefs,
                    &mut open,
                    false,
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    InventoryStyle::Explorer,
                );
            });
        };
        draw(&mut world, &mut st, &mut prefs);
        let [InvAction::Delete(first)] = &mut request(&world.inventory, &mut st, &prefs, id(40))[..] else {
            panic!("first item did not generate a Trash move");
        };
        st.begin_delete(id(90), first.selection.clone());
        st.finish_delete(id(91), true, false);
        draw(&mut world, &mut st, &mut prefs);
        assert!(request(&world.inventory, &mut st, &prefs, id(41)).is_empty(), "unrelated reply");
        st.finish_delete(id(90), true, true);
        draw(&mut world, &mut st, &mut prefs);
        assert!(
            request(&world.inventory, &mut st, &prefs, id(41)).is_empty(),
            "UDP move still pending"
        );
        rules::demo_mutate(&mut world.inventory, id(99), std::mem::take(&mut first.change)).unwrap();
        draw(&mut world, &mut st, &mut prefs);
        let [InvAction::Delete(second)] = &mut request(&world.inventory, &mut st, &prefs, id(41))[..] else {
            panic!("completed first move blocked the next deletion");
        };
        st.begin_delete(id(91), second.selection.clone());
        rules::demo_mutate(&mut world.inventory, id(99), std::mem::take(&mut second.change)).unwrap();
        st.finish_delete(id(91), true, false);
        draw(&mut world, &mut st, &mut prefs);
        assert!(st.pending_delete.is_none());
        assert!(
            [40, 41]
                .into_iter()
                .all(|item_id| rules::parent(&world.inventory, id(item_id)) == Some(id(TRASH)))
        );
        assert_eq!((st.selected, st.anchor), (Some(id(30)), Some(id(30))));
        assert_eq!(st.selection, HashSet::from([id(30)]));
    }
}

#[test]
fn received_deliveries_can_be_deleted_without_unlocking_other_edits() {
    let mut inv = received();
    let prefs = InventoryPreferences::default();
    let facts = Facts::default();
    let selected = [id(20), id(30), id(31)];
    let mut st = InventoryUi {
        selection: selected.into(),
        ..Default::default()
    };
    for target in [id(20), id(21), id(30), id(31)] {
        assert!(delete::allowed(&inv, &[target], &prefs, &facts), "received {target}");
        assert!(!rules::mutable(&inv, target), "other edits retain their rules");
        let menu = context_catalog::build(&inv, target, &st, &prefs, &facts);
        assert!(menu.entries.iter().any(|entry| entry.id == "delete" && entry.enabled));
    }
    let mut actions = Vec::new();
    delete::request(&inv, &mut st, &prefs, &facts, &mut actions);
    assert!(matches!(&st.dialog, Some(EditDialog::Delete(ids, false)) if ids.len() == 3));
    assert!(actions.is_empty());
    let change = rules::move_selection(&inv, &selected, id(TRASH), &prefs.protected, &facts.worn).expect("trash received deliveries");
    assert!(change.move_only());
    assert_eq!(change.operations.len(), 2, "the selected child stays in its purchase folder");
    assert!(change.refresh.contains(&id(10)) && change.refresh.contains(&id(TRASH)));
    rules::demo_mutate(&mut inv, id(99), change).expect("apply deletion");
    assert_eq!(inv.folders[&id(20)].info.parent, id(TRASH));
    assert_eq!(inv.folders[&id(21)].info.parent, id(20));
    assert_eq!(inv.items[&id(30)].parent, id(21));
    assert_eq!(inv.items[&id(31)].parent, id(TRASH));
    assert_eq!(inv.folders[&id(10)].info.parent, id(ROOT));
}

#[test]
fn received_deletion_preserves_system_worn_library_and_locked_folder_protections() {
    let mut inv = received();
    let mut prefs = InventoryPreferences::default();
    let mut facts = Facts::default();
    for target in [id(ROOT), id(TRASH), id(10), id(999)] {
        assert!(!delete::allowed(&inv, &[target], &prefs, &facts));
        assert!(rules::move_selection(&inv, &[target], id(TRASH), &prefs.protected, &facts.worn).is_err());
    }
    facts.worn.insert(id(30));
    for target in [id(20), id(21), id(30)] {
        assert!(!delete::allowed(&inv, &[target, id(31)], &prefs, &facts));
        assert!(rules::move_selection(&inv, &[target], id(TRASH), &prefs.protected, &facts.worn).is_err());
    }
    facts.worn.clear();
    for protected in [id(20), id(21)] {
        prefs.protected = HashSet::from([protected]);
        assert!(!delete::allowed(&inv, &[protected], &prefs, &facts), "the locked folder itself");
        assert!(rules::move_selection(&inv, &[protected], id(TRASH), &prefs.protected, &facts.worn).is_err());
        assert!(delete::allowed(&inv, &[id(30)], &prefs, &facts), "its contents stay ordinary");
    }
    prefs.protected.clear();
    inv.folders.get_mut(&id(21)).expect("library folder").library = true;
    assert!(!delete::allowed(&inv, &[id(30)], &prefs, &facts));
    assert!(rules::move_selection(&inv, &[id(30)], id(TRASH), &prefs.protected, &facts.worn).is_err());
}

/// LLFolderBridge::isProtected (#1728): a locked folder is neither moved nor
/// deleted, by a drag, the menu or the Delete key; its contents are ordinary
/// entries, and its parent may go to the Trash with it, like the Legacy Viewer.
#[test]
fn a_locked_folder_holds_only_itself_like_the_legacy_viewer() {
    let mut inv = received();
    folder(&mut inv, id(39), id(ROOT), -1);
    folder(&mut inv, id(40), id(39), -1);
    folder(&mut inv, id(41), id(40), -1);
    item(&mut inv, id(50), id(40));
    item(&mut inv, id(51), id(41));
    let prefs = InventoryPreferences {
        protected: HashSet::from([id(40)]),
        ..Default::default()
    };
    let facts = Facts::default();
    let st = InventoryUi::default();
    let menu = |target| context_catalog::build(&inv, target, &st, &prefs, &facts);
    let enabled = |target, key: &str| menu(target).entries.iter().any(|entry| entry.id == key && entry.enabled);
    assert!(!delete::allowed(&inv, &[id(40)], &prefs, &facts));
    assert!(rules::move_selection(&inv, &[id(40)], id(TRASH), &prefs.protected, &facts.worn).is_err());
    assert!(rules::move_selection(&inv, &[id(40)], id(ROOT), &prefs.protected, &facts.worn).is_err());
    assert!(!enabled(id(40), "delete") && !enabled(id(40), "cut"));
    assert!(enabled(id(40), "rename"), "get_is_category_renameable keeps the rename");
    for target in [id(41), id(50), id(51)] {
        assert!(delete::allowed(&inv, &[target], &prefs, &facts), "Delete {target}");
        assert!(
            enabled(target, "delete") && enabled(target, "cut") && enabled(target, "rename"),
            "menu {target}"
        );
        for to in [id(TRASH), id(ROOT)] {
            assert!(
                rules::move_selection(&inv, &[target], to, &prefs.protected, &facts.worn).is_ok(),
                "move {target} to {to}"
            );
        }
    }
    assert!(rules::move_selection(&inv, &[id(50)], id(41), &prefs.protected, &facts.worn).is_ok());
    // get_is_category_and_children_removable only refuses system folders below
    assert!(delete::allowed(&inv, &[id(39)], &prefs, &facts));
    assert!(rules::move_selection(&inv, &[id(39)], id(TRASH), &prefs.protected, &facts.worn).is_ok());
}

/// LLInvFVBridge::isItemMovable has no inbox check: purchases are dragged,
/// cut or moved out of Received Items, which stays closed to renames.
#[test]
fn received_purchases_move_out_but_are_not_renamed() {
    let inv = received();
    let prefs = InventoryPreferences::default();
    let facts = Facts::default();
    let st = InventoryUi::default();
    for target in [id(20), id(21), id(30), id(31)] {
        assert!(rules::relocatable(&inv, target, &prefs.protected), "{target}");
        assert!(!rules::renameable(&inv, target), "{target}");
        let menu = context_catalog::build(&inv, target, &st, &prefs, &facts);
        assert!(menu.entries.iter().any(|entry| entry.id == "cut" && entry.enabled), "cut {target}");
        assert!(
            rules::paste_allowed(&inv, &[target], id(ROOT), true, false, &prefs.protected, &facts.worn),
            "paste {target}"
        );
    }
    let change = rules::move_selection(&inv, &[id(20), id(31)], id(ROOT), &prefs.protected, &facts.worn).expect("move purchases out");
    assert!(change.move_only());
    assert_eq!(change.operations.len(), 2);
    // the inbox itself stays where it is
    assert!(!rules::relocatable(&inv, id(10), &prefs.protected));
    assert!(rules::move_selection(&inv, &[id(10)], id(ROOT), &prefs.protected, &facts.worn).is_err());
}

/// "Lock" is offered on a delivery folder inside Received Items too: the
/// lock must not open it, nor anything in Received Items, to moves in.
#[test]
fn a_locked_folder_in_received_items_stays_closed_to_moves_in() {
    let mut inv = received();
    folder(&mut inv, id(40), id(ROOT), -1);
    item(&mut inv, id(50), id(40));
    let mut prefs = InventoryPreferences::default();
    let facts = Facts::default();
    for locked in [id(20), id(21)] {
        prefs.protected = HashSet::from([locked]);
        for to in [id(10), id(20), id(21)] {
            assert!(!rules::destination(&inv, to), "{locked} locked, to {to}");
            assert!(
                rules::move_selection(&inv, &[id(50)], to, &prefs.protected, &facts.worn).is_err(),
                "{locked} locked, to {to}"
            );
            for cut in [true, false] {
                assert!(
                    !rules::paste_allowed(&inv, &[id(50)], to, cut, false, &prefs.protected, &facts.worn),
                    "{locked} locked, paste into {to}"
                );
            }
        }
    }
}

/// `folders` personal folders ten wide (four or five deep), two items in
/// each, and `selected` items in one folder three levels down; 60 worn items.
fn large(folders: u128, selected: u128) -> (Inventory, Vec<Uuid>, Facts) {
    let mut inv = Inventory {
        root: id(ROOT),
        ..Default::default()
    };
    folder(&mut inv, id(ROOT), Uuid::nil(), 8);
    folder(&mut inv, id(TRASH), id(ROOT), 14);
    for (n, kind) in [(10, 46), (11, 48), (12, 50), (13, 3), (14, 0), (15, 5)] {
        folder(&mut inv, id(n), id(ROOT), kind);
    }
    for n in 0..folders {
        let parent = if n < 10 { id(ROOT) } else { id(1000 + n / 10 - 1) };
        folder(&mut inv, id(1000 + n), parent, -1);
        item(&mut inv, id(1_000_000 + 2 * n), id(1000 + n));
        item(&mut inv, id(1_000_001 + 2 * n), id(1000 + n));
    }
    folder(&mut inv, id(BIG), id(1000 + 150), -1);
    let selection: Vec<_> = (0..selected).map(|n| id(5_000_000 + n)).collect();
    for it in &selection {
        item(&mut inv, *it, id(BIG));
    }
    let facts = Facts {
        worn: (0..60).map(|n| id(1_000_000 + 2 * (folders - 1 - n))).collect(),
        ..Default::default()
    };
    (inv, selection, facts)
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "measurement: cargo test --workspace delete_tests::measure -- --ignored --nocapture"]
fn measure() {
    let prefs = InventoryPreferences::default();
    for selected in [500, 2000, 5000] {
        let (inv, ids, facts) = large(20_000, selected);
        let mut st = InventoryUi {
            selection: ids.iter().copied().collect(),
            ..Default::default()
        };
        let t = Instant::now();
        assert!(delete::allowed(&inv, &ids, &prefs, &facts));
        let allowed = ms(t);
        // Built every frame while open: the best of three, without first-call costs.
        let mut menu_ms = f64::MAX;
        let mut menu = None;
        for _ in 0..3 {
            let t = Instant::now();
            menu = Some(context_catalog::build(&inv, ids[0], &st, &prefs, &facts));
            menu_ms = menu_ms.min(ms(t));
        }
        let menu = menu.expect("menu");
        assert!(menu.entries.iter().any(|e| e.id == "delete"));
        let t = Instant::now();
        let mut actions = Vec::new();
        delete::request(&inv, &mut st, &prefs, &facts, &mut actions);
        let request = ms(t);
        assert!(matches!(&st.dialog, Some(EditDialog::Delete(d, false)) if d.len() == ids.len()));
        assert!(actions.is_empty());
        let t = Instant::now();
        let change = rules::move_selection(&inv, &ids, id(TRASH), &prefs.protected, &facts.worn).expect("plan");
        let confirm = ms(t);
        assert_eq!(change.operations.len(), ids.len());
        let (folders, items) = (inv.folders.len(), inv.items.len());
        // Each move comes back as its own update (InventoryMoved, then the
        // simulator's BulkUpdateInventory), applied within the frame's 8 ms.
        let mut inv = inv;
        let t = Instant::now();
        for it in &ids {
            let mut moved = inv.items[it].clone();
            moved.parent = id(TRASH);
            inv.server_updated(Vec::new(), vec![moved]);
        }
        let per_move = ms(t) / ids.len() as f64;
        assert_eq!(inv.folders[&id(TRASH)].items.len(), ids.len());
        println!(
            "{selected:>5} selected, {folders} folders, {items} items: allowed {allowed:.1} ms, menu {menu_ms:.1} ms, request {request:.1} ms, confirm {confirm:.1} ms, {per_move:.3} ms per move"
        );
    }
}

#[test]
fn the_backspace_key_deletes_on_a_mac_only() {
    use super::delete::deletes;
    assert!(deletes(egui::Key::Delete, false));
    assert!(deletes(egui::Key::Delete, true));
    assert!(!deletes(egui::Key::Backspace, false));
    assert!(deletes(egui::Key::Backspace, true), "the Mac key labelled Delete");
    assert!(!deletes(egui::Key::A, true));
}

/// LLFolderView::getNextUnselectedItem: the row after the last one deleted,
/// else the one before; a deleted folder's rows leave with it (#2764).
#[test]
fn the_row_after_a_deletion_is_the_next_one_that_stays() {
    let mut inv = Inventory {
        root: id(ROOT),
        ..Default::default()
    };
    folder(&mut inv, id(ROOT), Uuid::nil(), 8);
    folder(&mut inv, id(10), id(ROOT), -1);
    item(&mut inv, id(11), id(10));
    item(&mut inv, id(12), id(10));
    item(&mut inv, id(20), id(ROOT));
    item(&mut inv, id(30), id(ROOT));
    let order = [10, 11, 12, 20, 30].map(id);
    let next = |deleted: &[u128], last: u128| {
        let gone = deleted.iter().map(|n| id(*n)).collect();
        delete::next_row(&inv, &order, &gone, id(last))
    };
    assert_eq!(next(&[11], 11), Some(id(12)), "the row below");
    assert_eq!(next(&[30], 30), Some(id(20)), "the row above at the end of the list");
    assert_eq!(next(&[10], 10), Some(id(20)), "past the deleted folder's own rows");
    assert_eq!(next(&[12, 20], 20), Some(id(30)));
    assert_eq!(next(&[20, 30], 30), Some(id(12)), "above, past every deleted row");
    assert_eq!(next(&[20, 30, 10], 30), None, "nothing stays");
    assert_eq!(next(&[11], 99), None, "a row the list does not show");
    let mut st = InventoryUi {
        selection: HashSet::from([id(11), id(20)]),
        selected: Some(id(11)),
        ..Default::default()
    };
    st.plain.order = order.to_vec();
    let deletion = delete::DeleteRequest::new(&inv, &st, &[id(11), id(20)], Default::default());
    st.begin_delete(id(90), deletion.selection);
    st.finish_delete(id(90), true, false);
    assert!(delete::reconcile_selection(&inv, &mut st));
    assert_eq!(
        st.selection,
        HashSet::from([id(12)]),
        "after the current row, like the Legacy Viewer's last selected"
    );
    assert_eq!((st.selected, st.anchor), (Some(id(12)), Some(id(12))));
    st.selected = Some(id(99));
    let deletion = delete::DeleteRequest::new(&inv, &st, &[id(12), id(20)], Default::default());
    st.begin_delete(id(91), deletion.selection);
    st.finish_delete(id(91), true, false);
    assert!(delete::reconcile_selection(&inv, &mut st));
    assert_eq!(st.selected, Some(id(30)), "else after the lowest deleted row");
}

/// #3507: the trash of the Gestures window asked again at every gesture,
/// whatever "Do not show again" said.
#[test]
fn a_delete_from_another_window_follows_do_not_show_again() {
    let inv = received();
    let ids = vec![id(31)];
    // asked for: the confirmation, and nothing sent yet
    let (mut st, mut actions) = (InventoryUi::default(), Vec::new());
    let prefs = InventoryPreferences::default();
    assert!(prefs.confirm_delete);
    assert!(delete::request_from_window(
        &inv,
        &mut st,
        &prefs,
        HashSet::new(),
        &mut actions,
        ids.clone()
    ));
    assert!(matches!(&st.dialog, Some(EditDialog::Delete(d, false)) if *d == ids));
    assert!(actions.is_empty());
    // "Do not show again": straight to the Trash, the inventory stays closed
    let (mut st, mut actions) = (InventoryUi::default(), Vec::new());
    let prefs = InventoryPreferences {
        confirm_delete: false,
        ..Default::default()
    };
    assert!(!delete::request_from_window(
        &inv,
        &mut st,
        &prefs,
        HashSet::new(),
        &mut actions,
        ids.clone()
    ));
    assert!(st.dialog.is_none());
    assert!(matches!(actions.as_slice(), [InvAction::Delete(_)]));
    // a worn item cannot go without a word: the confirmation states why
    let (mut st, mut actions) = (InventoryUi::default(), Vec::new());
    assert!(delete::request_from_window(
        &inv,
        &mut st,
        &prefs,
        HashSet::from([id(31)]),
        &mut actions,
        ids.clone()
    ));
    assert!(matches!(&st.dialog, Some(EditDialog::Delete(d, false)) if *d == ids));
    assert!(actions.is_empty());
}
