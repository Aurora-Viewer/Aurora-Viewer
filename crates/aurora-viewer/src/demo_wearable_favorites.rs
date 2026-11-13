//! AURORA_DEMO_WEARABLE_FAVORITES: the Favorite wearables window with a
//! synthetic wardrobe, open from frame 600.
//!
//! `1`: favorites in `#Aurora/Wearable Favorites` and a subfolder, beside a
//! Legacy folder. `folder`: the same, showing the subfolder. `sync`: only the
//! Legacy folder at first; Sync runs by itself at frame 720. `legacy-none`:
//! no favorites and no Legacy folder (no Sync button). `wear`: as `1`, with
//! the shirt and the hair worn from the list at frame 660. `wear-refused`: the
//! same without a current outfit folder, so the window says why it refused.
//! `delete-real`: as `folder`, with a real item in the subfolder; its delete
//! is asked at frame 720 and refused.

use crate::world::inventory::{FetchState, Folder, Inventory};
use aurora_net::inventory::{InvFolder, InvItem};
use uuid::Uuid;

const WARDROBE: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0001);
const AURORA: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0002);
const ROOT: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0003);
const EVENING: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0004);
const FIRESTORM: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0005);
const LEGACY: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0006);
const LEGACY_EVENING: Uuid = Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_0007);

/// The sample items: name, asset type, inventory type, wearable type.
const ITEMS: [(&str, i32, i32, u32); 7] = [
    ("Linen shirt", 5, 18, 4),
    ("Dark jeans", 5, 18, 5),
    ("Leather boots", 5, 18, 6),
    ("Silk evening dress", 5, 18, 12),
    ("Long braided hair", 6, 17, 0),
    ("Silver necklace", 6, 17, 0),
    ("Default shape", 13, 18, 0),
];

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_WEARABLE_FAVORITES").ok()
}

/// What `wear` puts on: the shirt, the hair.
pub fn worn_items() -> [Uuid; 2] {
    [item(0), item(4)]
}

fn item(index: usize) -> Uuid {
    Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_1000 + index as u128)
}

fn folder(inv: &mut Inventory, id: Uuid, parent: Uuid, name: &str) {
    inv.folders.entry(id).or_insert_with(|| Folder {
        info: InvFolder {
            id,
            parent,
            name: name.into(),
            type_default: -1,
            version: 1,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    });
    if let Some(p) = inv.folders.get_mut(&parent)
        && !p.children.contains(&id)
    {
        p.children.push(id);
    }
}

fn links(inv: &mut Inventory, owner: Uuid, parent: Uuid, seed: u128, targets: &[usize]) {
    for (n, target) in targets.iter().enumerate() {
        let (name, _, inv_type, _) = ITEMS[*target];
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(0xFA00_0000_0000_0000_0000_0000_0000_2000 + seed + n as u128),
            parent,
            name: name.into(),
            asset_type: 24,
            inv_type,
            asset_id: item(*target),
            owner,
            created_at: 1_790_000_000 + n as i64,
            ..Default::default()
        }]);
    }
}

/// The subfolder `delete-real` tries to delete.
pub fn evening() -> Uuid {
    EVENING
}

/// The folders of `mode`; the subfolder to show, for `folder`.
pub fn seed(inv: &mut Inventory, owner: Uuid, mode: &str) -> Option<Uuid> {
    // the base demo has no current outfit: nothing could be worn
    if mode != "wear-refused" {
        crate::world::appearance::seed_demo(inv, owner);
    }
    let root = inv.root;
    folder(inv, WARDROBE, root, "Wardrobe");
    for (index, (name, asset_type, inv_type, wearable)) in ITEMS.iter().enumerate() {
        inv.add_items(vec![InvItem {
            id: item(index),
            parent: WARDROBE,
            name: (*name).into(),
            asset_type: *asset_type,
            inv_type: *inv_type,
            flags: *wearable,
            owner,
            owner_mask: 0x7fff_ffff,
            created_at: 1_780_000_000,
            ..Default::default()
        }]);
    }
    if mode == "legacy-none" {
        inv.generation += 1;
        return None;
    }
    folder(inv, FIRESTORM, root, "#Firestorm");
    folder(inv, LEGACY, FIRESTORM, "#Wearable Favorites");
    folder(inv, LEGACY_EVENING, LEGACY, "Evening");
    links(inv, owner, LEGACY, 0x000, &[0, 2, 4]);
    links(inv, owner, LEGACY_EVENING, 0x100, &[3, 5]);
    if mode != "sync" {
        folder(inv, AURORA, root, "#Aurora");
        folder(inv, ROOT, AURORA, "Wearable Favorites");
        folder(inv, EVENING, ROOT, "Evening");
        links(inv, owner, ROOT, 0x200, &[0, 1, 4, 6]);
        links(inv, owner, EVENING, 0x300, &[3, 4]);
    }
    if mode == "delete-real"
        && let Some(mut necklace) = inv.items.get(&item(5)).cloned()
    {
        necklace.parent = EVENING;
        inv.add_items(vec![necklace]);
    }
    inv.generation += 1;
    matches!(mode, "folder" | "delete-real").then_some(EVENING)
}
