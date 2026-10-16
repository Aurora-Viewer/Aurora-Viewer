//! Offline worn object (AURORA_DEMO_WORN): a two-prim linkset on the back
//! of our avatar, with the flags and the inventory item the menus of worn
//! objects depend on. It goes through the same ObjectUpdate, selection and
//! touch paths as an attachment of the grid. Its `inventory`, `library`
//! and `sound` modes double-click inventory rows (`double_clicks`).
use super::*;
use aurora_net::inventory::InvItem;

pub const ROOT: u32 = 9300;
pub const CHILD: u32 = 9301;
/// Spine: behind the avatar, in view of the default camera.
const POINT: u8 = 9;

pub fn root() -> ObjectUpdate {
    let mut o = prim(
        ROOT,
        Vec3::new(-0.24, 0.0, 0.1),
        Quat::IDENTITY,
        Vec3::new(0.16, 0.36, 0.44),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.545, 0.361, 0.965, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    o.parent_id = 9000;
    o.state = POINT.rotate_left(4);
    o.owner_id = DEMO_AGENT;
    // move, touch, ours, owned, copy, modify (object_flags.h)
    o.update_flags = (1 << 8) | (1 << 7) | (1 << 5) | (1 << 4) | (1 << 3) | (1 << 2);
    o.name_values = format!("AttachItemID STRING RW SV {}", o.full_id);
    o
}

pub fn events() -> Vec<NetEvent> {
    let root = root();
    let mut child = prim(
        CHILD,
        Vec3::new(0.0, 0.0, 0.3),
        Quat::from_rotation_z(0.6),
        Vec3::splat(0.16),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.369, 0.918, 0.831, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    child.parent_id = ROOT;
    child.owner_id = DEMO_AGENT;
    child.update_flags = root.update_flags & !(1 << 7);
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![root, child],
    }]
}

/// The inventory item of the worn object ("Demo backpack").
pub fn item_id() -> Uuid {
    root().full_id
}

/// Library hat, shirt and shape of `AURORA_DEMO_WORN=library`.
const LIBRARY: [u128; 3] = [9_310_001, 9_310_002, 9_310_003];
/// The chime of `AURORA_DEMO_WORN=sound`.
const SOUND: u128 = 9_310_004;

/// The inventory rows a mode double-clicks: (first frame, item). Two
/// clicks a frame apart, and every pair far enough from the next one not
/// to count as a triple click.
pub fn double_clicks(mode: &str) -> Vec<(u64, Uuid)> {
    match mode {
        "inventory" => vec![(700, item_id()), (900, item_id())],
        "library" => [700, 800, 900].into_iter().zip(LIBRARY.map(Uuid::from_u128)).collect(),
        "sound" => vec![(700, Uuid::from_u128(SOUND))],
        _ => Vec::new(),
    }
}

fn demo_item(id: u128, parent: Uuid, owner: Uuid, name: &str, asset_type: i32, inv_type: i32, flags: u32) -> InvItem {
    InvItem {
        id: Uuid::from_u128(id),
        parent,
        asset_id: Uuid::from_u128(id + 1_000),
        name: name.into(),
        asset_type,
        inv_type,
        flags,
        creator: owner,
        owner,
        base_mask: 0x7fffffff,
        owner_mask: 0x7fffffff,
        next_owner_mask: 0x7fffffff,
        ..Default::default()
    }
}

/// `AURORA_DEMO_WORN=library`: a hat, a shirt and a shape in the Library,
/// to be copied into the inventory and worn by a double-click.
pub fn seed_library(world: &mut crate::world::World) {
    use crate::world::inventory::{FetchState, Folder};
    let inv = &mut world.inventory;
    if inv.lib_root.is_nil() {
        inv.lib_root = Uuid::from_u128(8003);
        inv.lib_owner = Uuid::from_u128(8004);
    }
    let root = inv.lib_root;
    inv.folders.entry(root).or_insert_with(|| Folder {
        info: aurora_net::inventory::InvFolder {
            id: root,
            name: aurora_i18n::tr!("common-library").into(),
            type_default: 8,
            version: 1,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: true,
    });
    let owner = inv.lib_owner;
    inv.add_items(vec![
        demo_item(LIBRARY[0], root, owner, aurora_i18n::tr!("demo-library-hat"), 6, 6, 2),
        demo_item(LIBRARY[1], root, owner, aurora_i18n::tr!("demo-library-shirt"), 5, 18, 4),
        demo_item(LIBRARY[2], root, owner, aurora_i18n::tr!("demo-library-shape"), 13, 18, 0),
    ]);
}

/// `AURORA_DEMO_WORN=sound`: a sound item playing `chime`, in the inventory.
pub fn seed_sound(world: &mut crate::world::World, chime: Uuid) {
    let inv = &mut world.inventory;
    let mut item = demo_item(SOUND, inv.root, world.agent_id, aurora_i18n::tr!("demo-trigger-sound"), 1, 1, 0);
    item.asset_id = chime;
    inv.add_items(vec![item]);
}

/// The inventory item the object is worn from, in the current outfit.
pub fn seed_inventory(world: &mut crate::world::World) {
    use crate::world::appearance::{self, Action};
    let inv = &mut world.inventory;
    appearance::seed_demo(inv, world.agent_id);
    let Some(mut item) = inv.items.values().find(|item| item.asset_type == 6).cloned() else {
        return;
    };
    item.id = root().full_id;
    item.name = aurora_i18n::tr!("demo-worn-item").into();
    item.flags = u32::from(POINT);
    let id = item.id;
    inv.add_items(vec![item]);
    let wear = Action::WearItems {
        items: vec![id],
        replace: false,
        point: 0,
    };
    if let Ok((change, _)) = appearance::plan(inv, wear, &Default::default()) {
        let _ = appearance::demo_mutate(inv, world.agent_id, &change);
    }
}
