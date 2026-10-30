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
