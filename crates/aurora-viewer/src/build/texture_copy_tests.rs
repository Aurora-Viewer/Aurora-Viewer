use super::*;
use crate::ui::context::flags::OBJECT_YOU_OWNER;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::InvFolder;
use std::sync::Arc;

const ASSET: Uuid = Uuid::from_u128(0x7e57_0001);
const OTHER: Uuid = Uuid::from_u128(0x7e57_0002);
const LIBRARY: Uuid = Uuid::from_u128(0x7e57_00f0);
const FULL: u32 = rules::FULL;
const COPY_ONLY: u32 = rules::COPY | rules::MODIFY;

/// The demo's action object 970, owned and modifiable by the agent.
fn world() -> (World, usize) {
    let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
        w.apply(ev);
    }
    let idx = w.objects.index_of_uuid(&crate::demo::action_id(970)).expect("demo action object");
    let agent = w.agent_id;
    let o = w.objects.get_mut(idx).expect("demo action object");
    o.parent_id = 0;
    o.owner_id = agent;
    o.update_flags = OBJECT_MODIFY | OBJECT_YOU_OWNER;
    w.inventory.items.clear();
    (w, idx)
}

fn give(w: &mut World, n: u128, asset: Uuid, mask: u32) -> Uuid {
    let id = Uuid::from_u128(n);
    let agent = w.agent_id;
    w.inventory.items.insert(
        id,
        InvItem {
            id,
            asset_type: 0,
            inv_type: 0,
            asset_id: asset,
            owner: agent,
            base_mask: FULL,
            owner_mask: mask,
            name: format!("Texture {n}"),
            ..Default::default()
        },
    );
    id
}

fn library_folder(w: &mut World) {
    w.inventory.lib_root = LIBRARY;
    w.inventory.folders.insert(
        LIBRARY,
        Folder {
            info: InvFolder {
                id: LIBRARY,
                name: "Library".into(),
                type_default: -1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: true,
        },
    );
}

#[test]
fn a_texture_copies_when_built_in_or_held_with_copy() {
    let (mut w, _) = world();
    let agent = w.agent_id;
    for id in PREDEFINED {
        assert!(can_copy(&w.inventory, agent, id), "{id}");
    }
    assert!(can_copy(&w.inventory, agent, Uuid::nil()));
    assert!(!can_copy(&w.inventory, agent, ASSET), "not in the inventory");
    give(&mut w, 1, ASSET, FULL & !rules::COPY);
    assert!(!can_copy(&w.inventory, agent, ASSET), "no copy");
    give(&mut w, 2, ASSET, rules::COPY);
    assert!(can_copy(&w.inventory, agent, ASSET), "copy alone is enough");
    // someone else's item counts with its everyone rights
    let id = give(&mut w, 3, OTHER, FULL);
    let it = w.inventory.items.get_mut(&id).expect("item");
    it.owner = Uuid::from_u128(9);
    it.everyone_mask = 0;
    assert!(!can_copy(&w.inventory, agent, OTHER));
    w.inventory.items.get_mut(&id).expect("item").everyone_mask = rules::COPY;
    assert!(can_copy(&w.inventory, agent, OTHER));
}

#[test]
fn the_item_lookup_prefers_the_transfer_right_asked_for() {
    let (mut w, _) = world();
    let agent = w.agent_id;
    let transfer = give(&mut w, 1, ASSET, FULL);
    let no_transfer = give(&mut w, 2, ASSET, COPY_ONLY);
    give(&mut w, 3, ASSET, FULL & !rules::COPY);
    assert_eq!(copy_item(&w.inventory, agent, ASSET, false), Some(transfer));
    assert_eq!(copy_item(&w.inventory, agent, ASSET, true), Some(no_transfer));
    w.inventory.items.remove(&transfer);
    assert_eq!(copy_item(&w.inventory, agent, ASSET, false), Some(no_transfer), "any copyable one");
}

#[test]
fn availability_tells_free_ids_from_limited_ones() {
    let (mut w, _) = world();
    let agent = w.agent_id;
    let inv = |w: &World, creator| availability(&w.inventory, agent, ASSET, creator, None);
    // not held: limited, nothing to paste through
    let none = inv(&w, false);
    assert_eq!(none, Availability::default());
    assert!(!none.usable(&w.inventory));
    assert!(inv(&w, true).full, "the owner created the object");
    assert!(availability(&w.inventory, agent, PREDEFINED[0], false, None).full);
    assert!(availability(&w.inventory, agent, aurora_assets::material::OVERRIDE_NULL_UUID, false, None).full);
    // a full-rights item: free, through that item
    let full = give(&mut w, 1, ASSET, FULL);
    assert_eq!(
        inv(&w, false),
        Availability {
            full: true,
            item: Some(full),
            library: false
        }
    );
    // a copy-only item: limited, usable while the item stays
    w.inventory.items.get_mut(&full).expect("item").owner_mask = COPY_ONLY;
    let limited = inv(&w, false);
    assert_eq!((limited.full, limited.item), (false, Some(full)));
    assert!(limited.usable(&w.inventory));
    w.inventory.items.remove(&full);
    assert!(!limited.usable(&w.inventory), "the item left the inventory since the copy");
    // a Library item is free
    library_folder(&mut w);
    let lib = give(&mut w, 2, ASSET, COPY_ONLY);
    w.inventory.items.get_mut(&lib).expect("item").parent = LIBRARY;
    let from_library = inv(&w, false);
    assert!(from_library.full && from_library.library);
}

#[test]
fn only_a_limited_item_goes_into_the_target_contents() {
    let (mut w, idx) = world();
    let agent = w.agent_id;
    let full = give(&mut w, 1, ASSET, FULL);
    let no_transfer = give(&mut w, 2, OTHER, COPY_ONLY);
    let inv = &w.inventory;
    assert!(!requires_contents(inv, &inv.items[&full], agent));
    assert!(requires_contents(inv, &inv.items[&no_transfer], agent));
    let mut tool = BuildTool::default();
    assert_eq!(tool.retain_for_paste(&w, idx, full), Retain::Done);
    assert!(tool.out.is_empty(), "nothing to send for a free item");
    // the contents are asked for first
    assert_eq!(tool.retain_for_paste(&w, idx, no_transfer), Retain::Wait);
    assert!(!tool.contents_ready(&w, idx));
    let object = w.objects.get(idx).expect("object").full_id;
    tool.on_task_inventory(&w, object, Some(1), Vec::new());
    tool.out.clear();
    assert_eq!(tool.retain_for_paste(&w, idx, no_transfer), Retain::Done);
    assert!(
        tool.out.iter().any(|c| matches!(
            c,
            aurora_net::NetCommand::Build(BuildCmd::UpdateTaskInventory { item, .. }) if item.item_id == no_transfer
        )),
        "the item goes into the contents"
    );
    // once there, a second face does not send it again
    tool.out
        .retain(|c| !matches!(c, aurora_net::NetCommand::Build(BuildCmd::UpdateTaskInventory { .. })));
    tool.contents.objects.get_mut(&object).expect("listing").loading = false;
    assert_eq!(tool.retain_for_paste(&w, idx, no_transfer), Retain::Done);
    assert!(
        !tool
            .out
            .iter()
            .any(|c| matches!(c, aurora_net::NetCommand::Build(BuildCmd::UpdateTaskInventory { .. })))
    );
    // an object that takes no inventory keeps the id off its face
    let other = give(&mut w, 3, Uuid::from_u128(0x7e57_0003), COPY_ONLY);
    w.objects.get_mut(idx).expect("object").update_flags = OBJECT_YOU_OWNER;
    assert_eq!(tool.retain_for_paste(&w, idx, other), Retain::Refused);
}

#[test]
fn a_material_copies_from_an_object_with_every_right() {
    let (mut w, idx) = world();
    let mut tool = BuildTool::default();
    assert!(material_copy_allowed(&tool, &w, idx, &[0]), "no material on the face");
    w.objects.get_mut(idx).expect("object").extra.render_materials = vec![(0, ASSET)];
    assert!(!material_copy_allowed(&tool, &w, idx, &[0]), "modify alone");
    let all = OBJECT_COPY | OBJECT_MODIFY | OBJECT_TRANSFER | OBJECT_YOU_OWNER;
    w.objects.get_mut(idx).expect("object").update_flags = all;
    assert!(material_copy_allowed(&tool, &w, idx, &[0]));
    assert!(material_copy_allowed(&tool, &w, idx, &[1]), "another face, without material");
    // the contents' own item of that material needs every right too
    let object = w.objects.get(idx).expect("object").full_id;
    let held = aurora_net::build::TaskItem {
        asset_id: ASSET,
        owner_mask: COPY_ONLY,
        ..Default::default()
    };
    tool.on_task_inventory(&w, object, Some(1), vec![held]);
    assert!(!material_copy_allowed(&tool, &w, idx, &[0]));
    tool.contents.objects.get_mut(&object).expect("listing").items[0].owner_mask = FULL;
    assert!(material_copy_allowed(&tool, &w, idx, &[0]));
    tool.contents.objects.get_mut(&object).expect("listing").loading = true;
    assert!(!material_copy_allowed(&tool, &w, idx, &[0]), "contents on their way");
}

#[test]
fn an_object_counts_as_free_when_its_owner_created_it() {
    let (mut w, idx) = world();
    let mut tool = BuildTool::default();
    let object = w.objects.get(idx).expect("object").full_id;
    let agent = w.agent_id;
    tool.props.insert(
        object,
        aurora_net::build::ObjectProps {
            object_id: object,
            full: true,
            creator_id: agent,
            owner_id: agent,
            ..Default::default()
        },
    );
    assert!(!creator_owned(&tool, &w, idx), "not every right");
    w.objects.get_mut(idx).expect("object").update_flags = OBJECT_COPY | OBJECT_MODIFY | OBJECT_TRANSFER;
    assert!(creator_owned(&tool, &w, idx));
    tool.props.get_mut(&object).expect("props").creator_id = Uuid::from_u128(9);
    assert!(!creator_owned(&tool, &w, idx), "bought from its creator");
}

#[test]
fn a_pbr_map_slot_takes_copy_and_transfer() {
    let agent = Uuid::from_u128(1);
    let item = |owner_mask| InvItem {
        owner: agent,
        owner_mask,
        ..Default::default()
    };
    assert!(map_slot_takes(&item(rules::COPY | rules::TRANSFER), agent));
    assert!(!map_slot_takes(&item(COPY_ONLY), agent));
    assert!(!map_slot_takes(&item(rules::TRANSFER | rules::MODIFY), agent));
}

/// A second object of the agent next to `world()`'s, for the selections of
/// several objects.
fn second_object(w: &mut World) -> usize {
    let idx = w.objects.index_of_uuid(&crate::demo::action_id(971)).expect("demo action object");
    let agent = w.agent_id;
    let o = w.objects.get_mut(idx).expect("demo action object");
    o.parent_id = 0;
    o.owner_id = agent;
    o.update_flags = OBJECT_MODIFY | OBJECT_YOU_OWNER;
    idx
}

fn list(tool: &mut BuildTool, w: &World, idx: usize) {
    let object = w.objects.get(idx).expect("object").full_id;
    tool.contents_ready(w, idx);
    tool.on_task_inventory(w, object, Some(1), Vec::new());
}

fn sent_into_contents(tool: &BuildTool, item: Uuid) -> usize {
    tool.out
        .iter()
        .filter(|c| matches!(c, aurora_net::NetCommand::Build(BuildCmd::UpdateTaskInventory { item: i, .. }) if i.item_id == item))
        .count()
}

#[test]
fn a_no_copy_item_goes_on_one_object_only() {
    let (mut w, a) = world();
    let b = second_object(&mut w);
    let no_copy = give(&mut w, 1, ASSET, FULL & !rules::COPY);
    let mut tool = BuildTool::default();
    assert_eq!(
        tool.admit_item(&mut w, no_copy, &[a, b]),
        Err(aurora_i18n::tr!("texture-library-no-copy-multiple"))
    );
    // two faces of one object count once
    list(&mut tool, &w, a);
    assert_eq!(tool.admit_item(&mut w, no_copy, &[a, a]), Ok(()));
    assert_eq!(sent_into_contents(&tool, no_copy), 1, "moved into the object");
    assert!(!w.inventory.items.contains_key(&no_copy), "a no-copy item leaves the inventory");
}

#[test]
fn a_no_transfer_item_is_copied_into_each_object() {
    let (mut w, a) = world();
    let b = second_object(&mut w);
    let no_transfer = give(&mut w, 1, ASSET, COPY_ONLY);
    let mut tool = BuildTool::default();
    // the contents are asked for first
    assert_eq!(
        tool.admit_item(&mut w, no_transfer, &[a, b]),
        Err(aurora_i18n::tr!("build-ui-texture-paste-wait-contents"))
    );
    assert_eq!(sent_into_contents(&tool, no_transfer), 0);
    list(&mut tool, &w, a);
    list(&mut tool, &w, b);
    assert_eq!(tool.admit_item(&mut w, no_transfer, &[a, b]), Ok(()));
    assert_eq!(sent_into_contents(&tool, no_transfer), 2);
    assert!(w.inventory.items.contains_key(&no_transfer), "a copy stays in the inventory");
    // an object that takes no inventory refuses the id
    let other = give(&mut w, 2, OTHER, COPY_ONLY);
    w.objects.get_mut(b).expect("object").update_flags = OBJECT_YOU_OWNER;
    list(&mut tool, &w, a);
    list(&mut tool, &w, b);
    assert_eq!(
        tool.admit_item(&mut w, other, &[a, b]),
        Err(aurora_i18n::tr!("texture-library-locked"))
    );
}

#[test]
fn a_full_item_or_a_typed_id_needs_nothing_more() {
    let (mut w, a) = world();
    let b = second_object(&mut w);
    let full = give(&mut w, 1, ASSET, FULL);
    let mut tool = BuildTool::default();
    assert_eq!(tool.admit_item(&mut w, full, &[a, b]), Ok(()));
    assert_eq!(tool.admit_item(&mut w, Uuid::nil(), &[a, b]), Ok(()));
    assert!(tool.out.is_empty());
}

/// TextureApplyFunctor (llselectmgr.cpp): an attachment takes only an item
/// whose base rights are unrestricted, on top of the picker's copy and
/// transfer filter.
#[test]
fn a_worn_object_takes_an_image_with_copy_transfer_and_unrestricted_base_rights() {
    let (mut w, a) = world();
    let avatar = w.objects.index_of_uuid(&w.agent_id).expect("the agent's avatar");
    let avatar_id = w.objects.get(avatar).expect("the agent's avatar").key.local_id;
    let o = w.objects.get_mut(a).expect("object");
    o.parent_id = avatar_id;
    o.state = 1;
    let no_transfer = give(&mut w, 1, ASSET, COPY_ONLY);
    let full = give(&mut w, 2, OTHER, FULL);
    let mut tool = BuildTool::default();
    assert_eq!(
        tool.admit_item(&mut w, no_transfer, &[a]),
        Err(aurora_i18n::tr!("build-ui-texture-drop-restricted"))
    );
    assert_eq!(tool.admit_item(&mut w, full, &[a]), Ok(()));
    let no_modify_base = give(&mut w, 3, ASSET, rules::COPY | rules::TRANSFER);
    w.inventory.items.get_mut(&no_modify_base).expect("item").base_mask = rules::COPY | rules::TRANSFER;
    assert_eq!(
        tool.admit_item(&mut w, no_modify_base, &[a]),
        Err(aurora_i18n::tr!("texture-library-locked"))
    );
    assert!(tool.out.is_empty(), "nothing goes into a worn object's contents");
}

/// dad3dApplyToObject asks willObjectAcceptInventory before
/// handleDropMaterialProtections looks at the contents: a material without
/// unrestricted base rights stays off a worn object even when its contents
/// already hold it.
#[test]
fn a_restricted_material_already_in_a_worn_objects_contents_is_refused() {
    let (mut w, a) = world();
    let avatar = w.objects.index_of_uuid(&w.agent_id).expect("the agent's avatar");
    let avatar_id = w.objects.get(avatar).expect("the agent's avatar").key.local_id;
    let o = w.objects.get_mut(a).expect("object");
    o.parent_id = avatar_id;
    o.state = 1;
    let object = o.full_id;
    let id = give(&mut w, 1, ASSET, rules::COPY | rules::TRANSFER);
    let item = w.inventory.items.get_mut(&id).expect("item");
    item.asset_type = AT_MATERIAL;
    item.inv_type = AT_MATERIAL;
    item.base_mask = rules::COPY | rules::TRANSFER;
    let material = item.clone();
    let mut tool = BuildTool::default();
    tool.contents_ready(&w, a);
    tool.on_task_inventory(&w, object, Some(1), vec![task_inventory::from_inventory(object, &material)]);
    assert!(tool.holds(&w, a, &material), "the contents hold it");
    assert!(contents_drop::worn_refused(&w, a, &material));
    assert_eq!(tool.admit_item(&mut w, id, &[a]), Err(aurora_i18n::tr!("texture-library-locked")));
    // the same material with every base right goes on
    w.inventory.items.get_mut(&id).expect("item").base_mask = FULL;
    assert!(!contents_drop::worn_refused(&w, a, &w.inventory.items[&id]));
    assert_eq!(tool.admit_item(&mut w, id, &[a]), Ok(()));
}

/// A Library item on a worn object: the agent's rights (its everyone mask)
/// and its base rights, with no Library exception (TextureApplyFunctor,
/// LLInventoryFilter::checkAgainstPermissions through getPermissionMask).
#[test]
fn a_library_item_on_a_worn_object_follows_the_same_rule() {
    let (mut w, a) = world();
    let avatar = w.objects.index_of_uuid(&w.agent_id).expect("the agent's avatar");
    let avatar_id = w.objects.get(avatar).expect("the agent's avatar").key.local_id;
    let o = w.objects.get_mut(a).expect("object");
    o.parent_id = avatar_id;
    o.state = 1;
    library_folder(&mut w);
    let lib = give(&mut w, 1, ASSET, FULL);
    {
        let item = w.inventory.items.get_mut(&lib).expect("item");
        item.parent = LIBRARY;
        item.owner = Uuid::from_u128(0x11b0);
        item.everyone_mask = rules::COPY;
    }
    assert!(library(&w.inventory, lib));
    let mut tool = BuildTool::default();
    assert_eq!(
        tool.admit_item(&mut w, lib, &[a]),
        Err(aurora_i18n::tr!("build-ui-texture-drop-restricted"))
    );
    w.inventory.items.get_mut(&lib).expect("item").everyone_mask = rules::COPY | rules::TRANSFER;
    assert_eq!(tool.admit_item(&mut w, lib, &[a]), Ok(()));
    w.inventory.items.get_mut(&lib).expect("item").base_mask = rules::COPY | rules::TRANSFER;
    assert_eq!(tool.admit_item(&mut w, lib, &[a]), Err(aurora_i18n::tr!("texture-library-locked")));
    assert!(tool.out.is_empty());
}

/// The picker's apply onto an attachment RLVa locks: refused, like any
/// drop on it (willObjectAcceptInventory).
#[test]
fn the_picker_apply_refuses_a_locked_attachment() {
    let (mut w, a) = world();
    let avatar = w.objects.index_of_uuid(&w.agent_id).expect("the agent's avatar");
    let avatar_id = w.objects.get(avatar).expect("the agent's avatar").key.local_id;
    let o = w.objects.get_mut(a).expect("object");
    o.parent_id = avatar_id;
    o.state = 1;
    let object = o.full_id;
    let full = give(&mut w, 1, ASSET, FULL);
    let mut tool = BuildTool::default();
    w.rlva_configure(true);
    assert_eq!(tool.admit_item(&mut w, full, &[a]), Ok(()));
    let agent = w.agent_id;
    assert!(w.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Controller".to_owned(),
        source_id: object,
        owner_id: agent,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: "@detach=n".to_owned(),
    }));
    assert_eq!(tool.admit_item(&mut w, full, &[a]), Err(aurora_i18n::tr!("texture-library-locked")));
}

#[test]
fn pbr_controls_edit_with_modify_on_the_object_and_its_material_item() {
    let (mut w, idx) = world();
    let mut tool = BuildTool::default();
    assert!(tool.material_edit_allowed(&w, &[(idx, 0)]), "no material on the face");
    w.objects.get_mut(idx).expect("object").extra.render_materials = vec![(0, ASSET)];
    assert!(!tool.material_edit_allowed(&w, &[(idx, 0)]), "contents not listed yet");
    list(&mut tool, &w, idx);
    assert!(tool.material_edit_allowed(&w, &[(idx, 0)]), "no item of that material");
    let object = w.objects.get(idx).expect("object").full_id;
    let mut held = InvItem {
        id: Uuid::from_u128(5),
        asset_id: ASSET,
        asset_type: AT_MATERIAL,
        owner_mask: rules::COPY | rules::TRANSFER,
        ..Default::default()
    };
    tool.on_task_inventory(&w, object, Some(2), vec![task_inventory::from_inventory(object, &held)]);
    assert!(!tool.material_edit_allowed(&w, &[(idx, 0)]), "its item cannot be modified");
    held.owner_mask = FULL;
    tool.on_task_inventory(&w, object, Some(3), vec![task_inventory::from_inventory(object, &held)]);
    assert!(tool.material_edit_allowed(&w, &[(idx, 0)]));
    w.objects.get_mut(idx).expect("object").update_flags = OBJECT_YOU_OWNER;
    assert!(!tool.material_edit_allowed(&w, &[(idx, 0)]), "the object cannot be modified");
}

#[test]
fn a_restricted_texture_leaves_a_pbr_face_alone() {
    let (mut w, idx) = world();
    assert!(BuildTool::surface_takes_texture(&w, idx, &[0], false, false), "a plain face");
    w.objects.get_mut(idx).expect("object").extra.render_materials = vec![(0, ASSET)];
    assert!(!BuildTool::surface_takes_texture(&w, idx, &[0], false, false));
    assert!(BuildTool::surface_takes_texture(&w, idx, &[0], true, false), "copy and transfer");
    assert!(
        BuildTool::surface_takes_texture(&w, idx, &[0], false, true),
        "the material is removed"
    );
    assert!(BuildTool::surface_takes_texture(&w, idx, &[0, 1], false, false), "one plain face");
}

#[test]
fn a_no_copy_item_is_refused_on_several_objects() {
    let agent = Uuid::from_u128(1);
    let item = |owner_mask| InvItem {
        owner: agent,
        owner_mask,
        ..Default::default()
    };
    assert!(selection_copy_denied(&item(FULL & !rules::COPY), agent, 2));
    assert!(!selection_copy_denied(&item(FULL & !rules::COPY), agent, 1));
    assert!(!selection_copy_denied(&item(COPY_ONLY), agent, 2));
    assert!(restricted(&item(COPY_ONLY), agent));
    assert!(!restricted(&item(FULL), agent));
    let material = InvItem {
        asset_type: AT_MATERIAL,
        ..item(rules::COPY | rules::TRANSFER)
    };
    assert!(restricted(&material, agent), "a material needs modify too");
}
