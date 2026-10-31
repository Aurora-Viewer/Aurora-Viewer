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
fn local_wearable_textures_preview_without_becoming_server_assets() {
    for kind in [1, 4, 13, 14, 16] {
        let (mut world, mut state, _) = fixture(kind);
        let local = Uuid::new_v4();
        let material = Uuid::new_v4();
        world.local_assets.insert(local, ("Local tattoo.png".into(), false));
        world.local_assets.insert(material, ("Local material.gltf".into(), true));
        let original = state.draft.as_ref().expect("draft").original.textures.clone();
        for &slot in slots(kind) {
            assert!(!state.set_texture(&world, slot, material));
            assert!(state.set_texture(&world, slot, local));
        }
        assert!(state.draft.as_ref().expect("draft").dirty());
        assert!(state.save(&world, None).is_none());
        assert!(state.save(&world, Some("Local copy".into())).is_none());
        assert_eq!(state.message, aurora_i18n::tr!("wearable-local-preview"));
        assert!(state.draft.as_ref().expect("draft").pending.is_none());
        if kind == 13 {
            let slot = slots(kind)[0];
            state.alpha_previous.insert(slot, local);
            state
                .draft
                .as_mut()
                .expect("draft")
                .edited
                .textures
                .insert(slot, crate::scene::avatar::IMG_INVISIBLE);
            state.update(&mut world);
        }
        world.local_assets.remove(&local);
        state.update(&mut world);
        if kind == 13 {
            let slot = slots(kind)[0];
            let previous = state.alpha_previous.remove(&slot).expect("restored alpha map");
            assert_eq!(previous, original[&slot]);
            state.draft.as_mut().expect("draft").edited.textures.insert(slot, previous);
        }
        assert_eq!(state.draft.as_ref().expect("draft").edited.textures, original);
        assert!(state.save(&world, None).is_some(), "removing the local file restores savable slots");
    }
}

#[test]
fn removing_a_local_texture_preserves_originally_absent_slots() {
    for kind in [4, 14] {
        let (mut world, mut state, _) = fixture(kind);
        let slot = slots(kind)[0];
        let draft = state.draft.as_mut().expect("draft");
        draft.original.textures.remove(&slot);
        draft.edited.textures.remove(&slot);
        let local = Uuid::new_v4();
        world.local_assets.insert(local, ("local.png".into(), false));
        assert!(state.set_texture(&world, slot, local));
        world.local_assets.remove(&local);
        state.update(&mut world);
        let draft = state.draft.as_ref().expect("draft");
        assert_eq!(draft.edited.textures, draft.original.textures);
        assert!(!draft.dirty());
        assert!(state.save(&world, None).is_some());
    }
}

#[test]
fn picker_rollback_preserves_prior_edits_and_ok_can_save_only_confirmed_inventory_textures() {
    let (mut world, mut state, id) = fixture(14);
    let local = Uuid::new_v4();
    world.local_assets.insert(local, ("local.png".into(), false));
    let mut image = world.inventory.items[&id].clone();
    image.id = Uuid::new_v4();
    image.asset_id = Uuid::new_v4();
    image.asset_type = 0;
    let inventory = image.asset_id;
    world.inventory.add_items(vec![image]);
    for before in [None, Some(Uuid::nil()), Some(inventory)] {
        let draft = state.draft.as_mut().expect("draft");
        if let Some(before) = before {
            draft.edited.textures.insert(26, before);
        } else {
            draft.edited.textures.remove(&26);
        }
        state.open_picker(&world, 26);
        assert!(state.preview_picker(&world, local));
        assert!(state.save(&world, None).is_none());
        assert!(state.save(&world, Some("Unconfirmed".into())).is_none());
        state.draft.as_mut().expect("draft").edited.params.insert(1071, 0.25);
        assert!(state.finish_picker(&world, None));
        assert_eq!(state.draft.as_ref().expect("draft").edited.textures.get(&26).copied(), before);
        assert_eq!(state.draft.as_ref().expect("draft").edited.params[&1071], 0.25);
    }
    state.open_picker(&world, 26);
    assert!(state.preview_picker(&world, inventory));
    assert!(state.save(&world, None).is_none());
    assert!(
        !state.finish_picker(&world, Some(inventory)),
        "OK keeps the already rendered choice without another bake"
    );
    let NetCommand::SaveWearable { data, .. } = state.save(&world, None).expect("confirmed choice saves") else {
        panic!("save")
    };
    assert_eq!(Wearable::parse(&data).expect("asset").textures[&26], inventory);
}

#[test]
fn switching_picker_slots_keeps_applied_choices_and_cancel_handles_removed_local_files() {
    let (mut world, mut state, id) = fixture(14);
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    world.local_assets.insert(first, ("first.png".into(), false));
    world.local_assets.insert(second, ("second.png".into(), false));
    assert!(state.set_texture(&world, 26, first));
    state.open_picker(&world, 26);
    assert!(state.preview_picker(&world, second));
    state.open_picker(&world, 27);
    assert_eq!(state.picker_slot, Some(27));
    assert_eq!(state.draft.as_ref().expect("draft").edited.textures[&26], second);
    state.finish_picker(&world, None);
    assert!(state.set_texture(&world, 26, first));
    state.open_picker(&world, 26);
    assert!(state.preview_picker(&world, second));
    world.local_assets.remove(&first);
    state.update(&mut world);
    state.finish_picker(&world, None);
    let draft = state.draft.as_ref().expect("draft");
    assert_eq!(draft.edited.textures.get(&26), draft.original.textures.get(&26));
    state.open_picker(&world, 26);
    assert!(state.preview_picker(&world, second));
    world.inventory.items.get_mut(&id).expect("tattoo").owner_mask &= !crate::world::inventory::actions::MODIFY;
    assert!(!state.preview_picker(&world, Uuid::nil()));
    state.finish_picker(&world, None);
    let draft = state.draft.as_ref().expect("draft");
    assert_eq!(
        draft.edited.textures.get(&26),
        draft.original.textures.get(&26),
        "cancellation still works after Modify is lost"
    );
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
