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

/// The two prims of the backpack, root first, rezzed with these local ids
/// on `point`; their object ids follow the local ids, like a new rez.
fn linkset(root_id: u32, point: u8) -> Vec<ObjectUpdate> {
    let mut root = prim(
        root_id,
        Vec3::new(-0.24, 0.0, 0.1),
        Quat::IDENTITY,
        Vec3::new(0.16, 0.36, 0.44),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.545, 0.361, 0.965, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    root.parent_id = 9000;
    root.state = point.rotate_left(4);
    root.owner_id = DEMO_AGENT;
    // move, touch, ours, owned, copy, modify (object_flags.h)
    root.update_flags = (1 << 8) | (1 << 7) | (1 << 5) | (1 << 4) | (1 << 3) | (1 << 2);
    root.name_values = format!("AttachItemID STRING RW SV {}", item_id());
    let mut child = prim(
        root_id + (CHILD - ROOT),
        Vec3::new(0.0, 0.0, 0.3),
        Quat::from_rotation_z(0.6),
        Vec3::splat(0.16),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.369, 0.918, 0.831, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    child.parent_id = root_id;
    child.owner_id = DEMO_AGENT;
    child.update_flags = root.update_flags & !(1 << 7);
    vec![root, child]
}

pub fn root() -> ObjectUpdate {
    linkset(ROOT, POINT).swap_remove(0)
}

/// The temporary attachment of `AURORA_DEMO_WORN=temporary`: the backpack's
/// shape as an experience attaches it to the left hand, with no item of
/// ours: its AttachItemID is its own object id (LLViewerObject::isTempAttachment).
pub const TEMPORARY: u32 = 9320;

pub fn temporary() -> Vec<ObjectUpdate> {
    let mut objects = linkset(TEMPORARY, 5);
    objects[0].name_values = format!("AttachItemID STRING RW SV {}", objects[0].full_id);
    objects
}

/// The temporary attachment in the demo's AttachmentResources answer, which
/// names it, when `AURORA_DEMO_WORN=temporary`.
pub fn temporary_resources() -> Option<aurora_net::land::ScriptAttachment> {
    (std::env::var("AURORA_DEMO_WORN").as_deref() == Ok("temporary")).then(|| aurora_net::land::ScriptAttachment {
        location: "Left Hand".into(),
        objects: vec![aurora_net::land::ScriptObject {
            id: temporary()[0].full_id,
            name: aurora_i18n::tr!("demo-worn-temporary-name").into(),
            owner: DEMO_AGENT,
            group_owned: false,
            memory: 64 * 1024,
            urls: 0,
            location: None,
        }],
    })
}

pub fn events() -> Vec<NetEvent> {
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: linkset(ROOT, POINT),
    }]
}

/// The inventory item of the worn object ("Demo backpack"): the object id
/// of its first rez.
pub fn item_id() -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | ROOT as u128)
}

/// The demo server's answer to RezMultipleAttachmentsFromInv for the
/// backpack, as the simulator's: the linkset rezzed again from its asset,
/// under new local ids and object ids, its AttachItemID the item, on the
/// point asked for (0: the point it was last worn on). `None` for another item.
pub fn rez(item: &aurora_net::AttachRequest) -> Option<Vec<ObjectUpdate>> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(ROOT + 100);
    if item.item_id != item_id() {
        return None;
    }
    let point = if item.point == 0 { POINT } else { item.point };
    Some(linkset(NEXT.fetch_add(2, Ordering::Relaxed), point))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = DEMO_AGENT;
        for ev in crate::demo::events().into_iter().chain(events()) {
            world.apply(ev);
        }
        world
    }

    fn request(point: u8) -> aurora_net::AttachRequest {
        aurora_net::AttachRequest {
            item_id: item_id(),
            owner_id: DEMO_AGENT,
            point,
            add: false,
            flags: u32::from(POINT),
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        }
    }

    /// Detached (KillObject of its local id), then worn again from the
    /// inventory: the viewer takes the new rez, two prims on the point,
    /// whatever its new local and object ids, and a second detach finds it
    /// again by its AttachItemID.
    #[test]
    fn the_backpack_detached_then_worn_again_comes_back_with_its_prims() {
        let mut world = world();
        let kill = |world: &mut World| {
            let root = world.worn_attachment(&item_id()).expect("worn");
            let local_id = world.objects.get(root).expect("root").key.local_id;
            world.apply(NetEvent::ObjectsKilled {
                handle: HANDLE,
                local_ids: vec![local_id],
            });
            assert!(world.worn_attachment(&item_id()).is_none(), "detached");
            local_id
        };
        let first = kill(&mut world);
        assert_eq!(first, ROOT);
        assert!(world.objects.iter().all(|(_, o)| o.key.local_id != CHILD), "its child goes with it");
        for point in [0, 9] {
            let objects = rez(&request(point)).expect("the backpack");
            world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
            let root = world.worn_attachment(&item_id()).expect("worn again");
            let o = world.objects.get(root).expect("root");
            assert_ne!(o.key.local_id, ROOT, "a new rez");
            assert_ne!(o.full_id, item_id());
            assert_eq!(o.attachment_point(), POINT);
            assert!(o.scale.min_element() > 0.1, "its own prims, not a placeholder");
            assert_eq!(world.objects.subtree(root).len(), 2, "with its linked prim");
            kill(&mut world);
        }
        assert!(
            rez(&aurora_net::AttachRequest {
                item_id: Uuid::nil(),
                ..request(0)
            })
            .is_none()
        );
    }
}
