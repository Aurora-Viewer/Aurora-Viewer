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
        assert!(!rules::mutable(&inv, target, &prefs.protected), "other edits retain their rules");
        let menu = context_catalog::build(&inv, target, &st, &prefs, &facts);
        assert!(menu.entries.iter().any(|entry| entry.id == "delete" && entry.enabled));
    }
    delete::request(&inv, &mut st, &prefs, &facts);
    assert!(matches!(&st.dialog, Some(EditDialog::Delete(ids, false)) if ids.len() == 3));
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
    for protected in [id(10), id(20), id(21)] {
        prefs.protected = HashSet::from([protected]);
        assert!(!delete::allowed(&inv, &[id(30)], &prefs, &facts));
        assert!(rules::move_selection(&inv, &[id(30)], id(TRASH), &prefs.protected, &facts.worn).is_err());
    }
    prefs.protected.clear();
    inv.folders.get_mut(&id(21)).expect("library folder").library = true;
    assert!(!delete::allowed(&inv, &[id(30)], &prefs, &facts));
    assert!(rules::move_selection(&inv, &[id(30)], id(TRASH), &prefs.protected, &facts.worn).is_err());
}

/// Rule B of #1728: a locked folder's contents move but are not deleted,
/// whether dragged to the Trash or deleted from the menu or the Delete key.
#[test]
fn a_locked_folders_contents_move_but_neither_drag_nor_delete_trashes_them() {
    let mut inv = received();
    folder(&mut inv, id(40), id(ROOT), -1);
    folder(&mut inv, id(41), id(40), -1);
    item(&mut inv, id(50), id(40));
    item(&mut inv, id(51), id(41));
    let prefs = InventoryPreferences {
        protected: HashSet::from([id(40)]),
        ..Default::default()
    };
    let facts = Facts::default();
    let st = InventoryUi::default();
    for target in [id(40), id(41), id(50), id(51)] {
        assert!(!delete::allowed(&inv, &[target], &prefs, &facts), "Delete {target}");
        assert!(
            rules::move_selection(&inv, &[target], id(TRASH), &prefs.protected, &facts.worn).is_err(),
            "drag {target}"
        );
        let menu = context_catalog::build(&inv, target, &st, &prefs, &facts);
        assert!(
            menu.entries.iter().any(|entry| entry.id == "delete" && !entry.enabled),
            "menu {target}"
        );
    }
    for target in [id(41), id(50), id(51)] {
        assert!(
            rules::move_selection(&inv, &[target], id(ROOT), &prefs.protected, &facts.worn).is_ok(),
            "move {target}"
        );
    }
    assert!(rules::move_selection(&inv, &[id(40)], id(ROOT), &prefs.protected, &facts.worn).is_err());
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
            assert!(!rules::move_destination(&inv, to), "{locked} locked, to {to}");
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
        delete::request(&inv, &mut st, &prefs, &facts);
        let request = ms(t);
        assert!(matches!(&st.dialog, Some(EditDialog::Delete(d, false)) if d.len() == ids.len()));
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
