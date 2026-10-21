use super::*;
use crate::scene::avatar::AvatarLibrary;
use aurora_net::{AvatarAppearance, NetEvent};

pub(crate) fn fixture() -> (World, State) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(42);
    world.inventory.root = Uuid::from_u128(43);
    crate::world::appearance::seed_demo(&mut world.inventory, world.agent_id);
    let item = worn_shape(&world).expect("worn shape");
    let mut state = State::default();
    state.open_item(&mut world, item).expect("load");
    let data = crate::world::inventory::wearable::create(0, "Shape", world.agent_id);
    let asset = world.inventory.items[&item].asset_id;
    assert!(state.loaded(&mut world, item, asset, &data));
    (world, state)
}
fn height() -> &'static catalog::Param {
    catalog::PARAMS.iter().find(|p| p.id == 33).expect("height")
}

#[test]
fn edit_then_cancel_restores_the_latest_authoritative_appearance() {
    let (mut world, mut state) = fixture();
    state.draft.as_mut().expect("draft").set(height(), 2.0);
    state.changed(&mut world);
    let latest = world.avatar_lib.shape_params.values_with(&[(33, -1.0), (80, 1.0)]);
    world.apply(NetEvent::Appearance(AvatarAppearance {
        avatar_id: world.agent_id,
        visual_params: latest.clone(),
        texture_entry: None,
        cof_version: 4,
        appearance_version: 1,
        hover_height: 0.0,
        attachments: 0,
    }));
    assert_ne!(world.visual_params(&world.agent_id).expect("preview"), latest);
    state.close(&mut world);
    assert_eq!(world.visual_params(&world.agent_id).expect("server"), latest);
}

#[test]
fn bounds_and_nonfinite_input_do_not_corrupt_the_shape() {
    let (_, mut state) = fixture();
    let draft = state.draft.as_mut().expect("draft");
    draft.set(height(), f32::INFINITY);
    let original = draft.value(height());
    draft.set(height(), f32::NAN);
    assert_eq!(draft.value(height()), original);
    draft.set(height(), 100.0);
    assert_eq!(draft.value(height()), height().max);
    draft.set(height(), -100.0);
    assert_eq!(draft.value(height()), height().min);
}

#[test]
fn no_copy_shape_saves_in_place_but_cannot_be_copied() {
    let (mut world, mut state) = fixture();
    let id = worn_shape(&world).expect("shape");
    world.inventory.items.get_mut(&id).expect("item").owner_mask = 0x4000;
    state.draft.as_mut().expect("draft").set(height(), 2.0);
    assert!(state.save(&world, Some("Copy".into())).is_none());
    let NetCommand::SaveWearable { item, copy_name, .. } = state.save(&world, None).expect("save") else {
        panic!("shape save")
    };
    assert_eq!(item.owner_mask, 0x4000);
    assert!(copy_name.is_none());
}

#[test]
fn read_only_shape_can_only_be_copied_unchanged_and_revoked_modify_is_respected() {
    let (mut world, mut state) = fixture();
    let id = worn_shape(&world).expect("shape");
    world.inventory.items.get_mut(&id).expect("item").owner_mask = 0x8000;
    assert!(state.save(&world, None).is_none());
    assert!(state.save(&world, Some("Copy".into())).is_some());
    state.draft.as_mut().expect("draft").pending = None;
    state.draft.as_mut().expect("draft").set(height(), 2.0);
    assert!(state.save(&world, Some("Changed".into())).is_none());
}

#[test]
fn stale_download_and_save_replies_do_not_replace_another_session() {
    let (mut world, mut state) = fixture();
    let id = worn_shape(&world).expect("shape");
    state.close(&mut world);
    state.open_item(&mut world, id).expect("load");
    assert!(!state.loaded(&mut world, id, Uuid::new_v4(), &Ok(Vec::new())));
    state.close(&mut world);
    let asset = world.inventory.items[&id].asset_id;
    assert!(!state.loaded(&mut world, id, asset, &Ok(Vec::new())));
    let (mut world, mut state) = fixture();
    let NetCommand::SaveWearable { request, .. } = state.save(&world, None).expect("save") else {
        panic!("save")
    };
    let item = state.draft.as_ref().expect("draft").item.clone();
    assert!(state.saved(&mut world, Uuid::new_v4(), Ok(item)).is_none());
    assert_eq!(state.draft.as_ref().expect("draft").pending, Some(request));
}

#[test]
fn failed_save_keeps_the_draft_retryable_and_cannot_be_reported_as_saved() {
    let (mut world, mut state) = fixture();
    state.draft.as_mut().expect("draft").set(height(), 2.0);
    let NetCommand::SaveWearable { request, data, .. } = state.save(&world, None).expect("save") else {
        panic!("save")
    };
    assert_eq!(Wearable::parse(&data).expect("asset").params[&33], 2.0);
    assert!(state.saved(&mut world, request, Err("Rejected".into())).is_none());
    assert!(state.draft.as_ref().expect("draft").dirty());
    assert!(state.draft.as_ref().expect("draft").pending.is_none());
    assert_eq!(state.message, "Rejected");
    assert!(state.save(&world, None).is_some());
}

#[test]
fn concurrent_asset_change_prevents_overwriting_the_new_shape() {
    let (mut world, mut state) = fixture();
    let id = worn_shape(&world).expect("shape");
    state.draft.as_mut().expect("draft").set(height(), 2.0);
    world.inventory.items.get_mut(&id).expect("item").asset_id = Uuid::new_v4();
    assert!(state.save(&world, None).is_none());
    assert!(state.draft.as_ref().expect("draft").pending.is_none());
}
