use super::*;
use crate::world::appearance::{self, Action};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub(crate) fn fixture(kind: u8) -> (World, State, Uuid) {
    static LIBRARY: OnceLock<Arc<crate::scene::avatar::AvatarLibrary>> = OnceLock::new();
    let library = LIBRARY
        .get_or_init(|| Arc::new(crate::scene::avatar::AvatarLibrary::load()))
        .clone();
    let mut world = World::new(library);
    world.agent_id = Uuid::from_u128(42);
    world.inventory.root = Uuid::from_u128(43);
    appearance::seed_demo(&mut world.inventory, world.agent_id);
    let id = Uuid::from_u128(9000 + u128::from(kind));
    let item = InvItem {
        id,
        parent: appearance::system_folder(&world.inventory, if kind < 4 { 13 } else { 5 }).unwrap_or(world.inventory.root),
        asset_id: Uuid::from_u128(10000 + u128::from(kind)),
        owner: world.agent_id,
        creator: world.agent_id,
        owner_mask: 0x7fff_ffff,
        base_mask: 0x7fff_ffff,
        next_owner_mask: 0x7fff_ffff,
        asset_type: if kind < 4 { 13 } else { 5 },
        inv_type: 18,
        flags: u32::from(kind),
        name: format!("Wearable {kind}"),
        ..Default::default()
    };
    world.inventory.add_items(vec![item]);
    let (change, _) = appearance::plan(
        &world.inventory,
        Action::WearItem {
            item: id,
            replace: false,
            point: 0,
        },
        &HashMap::new(),
    )
    .expect("wear plan");
    appearance::demo_mutate(&mut world.inventory, world.agent_id, &change).expect("wear");
    let mut state = State::default();
    assert!(state.open_item(&mut world, id).is_some());
    let item = &world.inventory.items[&id];
    let asset = item.asset_id;
    let data = crate::world::inventory::wearable::create(kind, &item.name, world.agent_id);
    assert!(state.loaded(&mut world, id, asset, &data));
    assert!(state.draft.is_some());
    (world, state, id)
}

#[test]
fn every_remaining_wearable_loads_and_saves_its_real_kind_and_texture_slots() {
    for kind in (1..=14).chain(std::iter::once(16)) {
        let (world, mut state, id) = fixture(kind);
        let NetCommand::SaveWearable { item, data, copy_name, .. } = state.save(&world, None).expect("save") else {
            panic!("wearable save")
        };
        let saved = Wearable::parse(&data).expect("saved wearable");
        assert_eq!(saved.kind, kind);
        assert_eq!(item.id, id);
        assert!(copy_name.is_none());
        assert!(slots(kind).iter().all(|slot| saved.textures.contains_key(slot)));
    }
}

#[test]
fn texture_edits_are_dirty_preserve_other_slots_and_require_real_inventory_rights() {
    let (mut world, mut state, id) = fixture(14);
    let original = state.draft.as_ref().expect("draft").original.textures.clone();
    let asset = Uuid::from_u128(50000);
    assert!(!state.set_texture(&world, 26, asset));
    let mut texture = world.inventory.items[&id].clone();
    texture.id = Uuid::from_u128(50001);
    texture.asset_id = asset;
    texture.asset_type = 0;
    texture.inv_type = 0;
    texture.owner_mask = crate::world::inventory::actions::COPY;
    world.inventory.add_items(vec![texture.clone()]);
    assert!(
        !state.set_texture(&world, 26, asset),
        "transferable clothing needs a transferable texture"
    );
    texture.owner_mask |= crate::world::inventory::actions::TRANSFER;
    world.inventory.add_items(vec![texture]);
    assert!(state.set_texture(&world, 26, asset));
    assert!(!state.set_texture(&world, 99, asset), "unrelated slots stay untouched");
    assert!(state.draft.as_ref().expect("draft").dirty());
    let NetCommand::SaveWearable { data, .. } = state.save(&world, None).expect("save") else {
        panic!("save")
    };
    let saved = Wearable::parse(&data).expect("saved");
    assert_eq!(saved.textures[&26], asset);
    assert_eq!(saved.textures[&27], original[&27]);
    assert_eq!(saved.textures[&28], original[&28]);
}

#[test]
fn lost_modify_copy_rights_stale_assets_and_removed_layers_cannot_be_saved() {
    let (mut world, mut state, id) = fixture(1);
    world.inventory.items.get_mut(&id).expect("skin").owner_mask &= !crate::world::inventory::actions::MODIFY;
    assert!(state.save(&world, None).is_none());
    assert!(state.save(&world, Some("Unmodified copy".into())).is_some());
    state.draft.as_mut().expect("draft").pending = None;
    world.inventory.items.get_mut(&id).expect("skin").owner_mask = 0;
    assert!(state.save(&world, Some("Denied".into())).is_none());
    world.inventory.items.get_mut(&id).expect("skin").owner_mask = 0x7fff_ffff;
    world.inventory.items.get_mut(&id).expect("skin").asset_id = Uuid::new_v4();
    assert!(state.save(&world, None).is_none());
    state.update(&mut world);
    assert!(state.draft.is_none());
}

#[test]
fn texture_permissions_are_checked_again_when_saving_after_the_picker_closes() {
    let (mut world, mut state, id) = fixture(14);
    let texture_id = Uuid::from_u128(54001);
    let asset = Uuid::from_u128(54002);
    let mut texture = world.inventory.items[&id].clone();
    texture.id = texture_id;
    texture.asset_id = asset;
    texture.asset_type = 0;
    world.inventory.add_items(vec![texture]);
    assert!(state.set_texture(&world, 26, asset));
    world.inventory.items.get_mut(&texture_id).expect("texture").owner_mask = 0;
    assert!(state.save(&world, None).is_none());
    assert!(state.draft.as_ref().expect("draft").pending.is_none());
}

#[test]
fn saving_a_copy_replaces_only_the_edited_layer_and_preserves_layer_order() {
    let (mut world, state, id) = fixture(14);
    let mut second = world.inventory.items[&id].clone();
    second.id = Uuid::from_u128(60001);
    world.inventory.add_items(vec![second.clone()]);
    let (change, _) = appearance::plan(
        &world.inventory,
        Action::WearItem {
            item: second.id,
            replace: false,
            point: 0,
        },
        &HashMap::new(),
    )
    .expect("add second");
    appearance::demo_mutate(&mut world.inventory, world.agent_id, &change).expect("second worn");
    let cof = appearance::cof(&world.inventory).expect("COF");
    let before = appearance::folder_links(&world.inventory, cof);
    let mut copy = state.draft.as_ref().expect("draft").item.clone();
    copy.id = Uuid::from_u128(60002);
    world.inventory.add_items(vec![copy.clone()]);
    let (change, sync) = appearance::plan(&world.inventory, Action::ReplaceWearable { old: id, new: copy.id }, &HashMap::new())
        .expect("replace only the saved layer");
    assert!(sync);
    assert!(change.links.iter().any(|link| link.target == second.id));
    assert!(!change.links.iter().any(|link| link.target == id));
    assert_eq!(change.links.len(), before.len());
    assert_eq!(
        change.links.iter().find(|link| link.target == copy.id).expect("copy link").desc,
        before.iter().find(|link| link.target == id).expect("old link").desc
    );
}
