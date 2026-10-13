//! Offline worn object (AURORA_DEMO_WORN): a two-prim linkset on the back
//! of our avatar, with the flags and the inventory item the menus of worn
//! objects depend on. It goes through the same ObjectUpdate, selection and
//! touch paths as an attachment of the grid.
use super::*;

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
