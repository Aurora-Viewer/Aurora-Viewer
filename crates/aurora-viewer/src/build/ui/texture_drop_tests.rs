use super::*;
const AT_TEXTURE: i32 = 0;
use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
use std::sync::Arc;
use uuid::Uuid;

const TEXTURE: u128 = 78001;
const NO_COPY_TEXTURE: u128 = 78002;
const MATERIAL: u128 = 78003;
const NO_MODIFY_TEXTURE: u128 = 78004;
const NO_TRANSFER_TEXTURE: u128 = 78005;

/// The demo's action object 970, owned and modifiable by the agent, its
/// face 0 selected, and three items: a full-permission texture, a no-copy
/// texture and a material.
fn world() -> (World, Vec<Sel>) {
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
    let te = *o.te.as_ref().expect("demo faces").face(0);
    let all = contents_drop::UNRESTRICTED;
    for (n, kind, mask) in [
        (TEXTURE, AT_TEXTURE, all),
        (NO_COPY_TEXTURE, AT_TEXTURE, all & !rules::COPY),
        (MATERIAL, AT_MATERIAL, all),
        (NO_MODIFY_TEXTURE, AT_TEXTURE, all & !rules::MODIFY),
        (NO_TRANSFER_TEXTURE, AT_TEXTURE, all & !rules::TRANSFER),
    ] {
        let id = Uuid::from_u128(n);
        w.inventory.items.insert(
            id,
            InvItem {
                id,
                asset_type: kind,
                inv_type: kind,
                asset_id: Uuid::from_u128(n + 500),
                owner: agent,
                base_mask: mask,
                owner_mask: mask,
                name: format!("Item {n}"),
                ..Default::default()
            },
        );
    }
    (w, vec![Sel { idx, face: 0, te }])
}

fn item(w: &World, n: u128) -> InvItem {
    w.inventory.items[&Uuid::from_u128(n)].clone()
}

#[test]
fn a_slot_takes_a_texture_and_the_material_slot_a_material() {
    let (w, sel) = world();
    let (texture, material) = (item(&w, TEXTURE), item(&w, MATERIAL));
    for target in [
        PickTarget::Diffuse,
        PickTarget::Normal,
        PickTarget::Specular,
        PickTarget::PbrMap(0),
        PickTarget::PbrMap(3),
    ] {
        assert_eq!(allowed(&w, &sel, target, &texture), Ok(()), "{target:?}");
        assert_eq!(allowed(&w, &sel, target, &material), Err(Refusal::Cargo), "{target:?}");
    }
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMaterial, &material), Ok(()));
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMaterial, &texture), Err(Refusal::Cargo));
    for target in [PickTarget::Sculpt, PickTarget::LightTexture] {
        assert_eq!(allowed(&w, &sel, target, &texture), Ok(()), "editable object texture slot");
    }
    // a no-copy texture still goes on a prim in the world (PERM_NONE filter)
    assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &item(&w, NO_COPY_TEXTURE)), Ok(()));
}

/// The PBR map slots take a texture the agent may copy and transfer, its
/// modify right aside (setFilterPermissionMasks(PERM_COPY | PERM_TRANSFER)).
#[test]
fn a_pbr_map_takes_only_a_copy_and_transfer_texture() {
    let (mut w, sel) = world();
    let all = contents_drop::UNRESTRICTED;
    let mut texture = item(&w, TEXTURE);
    for (mask, ok) in [
        (all, true),
        (all & !rules::MODIFY, true),
        (all & !rules::COPY, false),
        (all & !rules::TRANSFER, false),
    ] {
        texture.owner_mask = mask;
        let expected = if ok { Ok(()) } else { Err(Refusal::Restricted) };
        for map in 0..4 {
            assert_eq!(allowed(&w, &sel, PickTarget::PbrMap(map), &texture), expected, "{mask:#x}");
        }
        // the Blinn-Phong slots keep their own rule
        assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &texture), Ok(()), "{mask:#x}");
    }
    // someone else's item counts with its everyone rights
    texture.owner = Uuid::from_u128(1);
    texture.everyone_mask = rules::COPY | rules::TRANSFER;
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMap(2), &texture), Ok(()));
    texture.everyone_mask = rules::COPY;
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMap(2), &texture), Err(Refusal::Restricted));
    w.objects.get_mut(sel[0].idx).expect("selected prim").update_flags = OBJECT_YOU_OWNER;
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMap(2), &item(&w, TEXTURE)), Ok(()));
}

#[test]
fn an_object_that_refuses_the_item_refuses_the_drop_except_on_a_pbr_map() {
    let (mut w, sel) = world();
    w.objects.get_mut(sel[0].idx).expect("selected prim").update_flags = OBJECT_YOU_OWNER;
    let texture = item(&w, TEXTURE);
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(allowed(&w, &sel, target, &texture), Err(Refusal::Object), "{target:?}");
    }
    assert_eq!(
        allowed(&w, &sel, PickTarget::PbrMaterial, &item(&w, MATERIAL)),
        Err(Refusal::Object)
    );
    assert_eq!(
        allowed(&w, &sel, PickTarget::PbrMap(1), &texture),
        Ok(()),
        "the PBR maps have no drag callback"
    );
    assert_eq!(
        allowed(&w, &[], PickTarget::Diffuse, &texture),
        Err(Refusal::Object),
        "nothing selected"
    );
}

#[test]
fn a_worn_object_takes_only_a_copy_and_transfer_texture() {
    let (mut w, sel) = world();
    let avatar = w.objects.index_of_uuid(&w.agent_id).expect("the agent's avatar");
    let avatar_id = w.objects.get(avatar).expect("the agent's avatar").key.local_id;
    let o = w.objects.get_mut(sel[0].idx).expect("selected prim");
    o.parent_id = avatar_id;
    o.state = 1;
    assert_eq!(contents_drop::linkset(&w, sel[0].idx).map(|(_, worn)| worn), Some(true));
    let no_copy = item(&w, NO_COPY_TEXTURE);
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(allowed(&w, &sel, target, &item(&w, TEXTURE)), Ok(()), "{target:?}");
        assert_eq!(allowed(&w, &sel, target, &no_copy), Err(Refusal::Restricted), "{target:?}");
    }
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMap(0), &no_copy), Err(Refusal::Restricted));
}

fn wear(w: &mut World, idx: usize) {
    let avatar = w.objects.index_of_uuid(&w.agent_id).expect("the agent's avatar");
    let avatar_id = w.objects.get(avatar).expect("the agent's avatar").key.local_id;
    let o = w.objects.get_mut(idx).expect("selected prim");
    o.parent_id = avatar_id;
    o.state = 1;
}

/// The texture's own modify right does not matter on a worn object: only
/// the object's, and copy and transfer on the texture.
#[test]
fn a_worn_object_takes_a_no_modify_texture_with_copy_and_transfer() {
    let (mut w, sel) = world();
    wear(&mut w, sel[0].idx);
    let no_modify = item(&w, NO_MODIFY_TEXTURE);
    assert_eq!(no_modify.base_mask & rules::MODIFY, 0);
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(allowed(&w, &sel, target, &no_modify), Ok(()), "{target:?}");
        for refused in [NO_COPY_TEXTURE, NO_TRANSFER_TEXTURE] {
            assert_eq!(
                allowed(&w, &sel, target, &item(&w, refused)),
                Err(Refusal::Restricted),
                "{target:?}"
            );
        }
    }
    let idx = sel[0].idx;
    assert!(!contents_drop::worn_image_refused(&w, idx, &no_modify));
    assert!(contents_drop::worn_image_refused(&w, idx, &item(&w, NO_COPY_TEXTURE)));
    assert!(contents_drop::worn_image_refused(&w, idx, &item(&w, NO_TRANSFER_TEXTURE)));
    // the base rule keeps guarding what goes into a worn object's contents
    assert_eq!(contents_drop::accepts(&w, idx, &no_modify), Err("rez-contents-denied"));
}

#[test]
fn a_worn_object_you_cannot_modify_refuses_every_texture() {
    let (mut w, sel) = world();
    wear(&mut w, sel[0].idx);
    // FLAGS_ALLOW_INVENTORY_DROP must not let a no-modify worn object through
    w.objects.get_mut(sel[0].idx).expect("selected prim").update_flags = OBJECT_YOU_OWNER | (1 << 16);
    assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &item(&w, TEXTURE)), Err(Refusal::Object));
    assert_eq!(
        allowed(&w, &sel, PickTarget::Diffuse, &item(&w, NO_MODIFY_TEXTURE)),
        Err(Refusal::Object)
    );
    assert!(!contents_drop::worn_image_refused(&w, sel[0].idx, &item(&w, TEXTURE)));
}

#[test]
fn only_a_worn_selection_narrows_the_picker() {
    let (mut w, sel) = world();
    assert_eq!(picker_permissions(&w, &sel, PickTarget::Diffuse), 0);
    wear(&mut w, sel[0].idx);
    let both = rules::COPY | rules::TRANSFER;
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(picker_permissions(&w, &sel, target), both, "{target:?}");
    }
    assert_eq!(picker_permissions(&w, &sel, PickTarget::PbrMaterial), 0);
    assert_eq!(picker_permissions(&w, &sel, PickTarget::PbrMap(0)), 0);
}

#[test]
fn every_refusal_has_a_reason() {
    for (refusal, target) in [
        (Refusal::Cargo, PickTarget::Diffuse),
        (Refusal::Cargo, PickTarget::PbrMaterial),
        (Refusal::Restricted, PickTarget::Diffuse),
        (Refusal::Object, PickTarget::Diffuse),
        (Refusal::Disabled, PickTarget::Diffuse),
    ] {
        assert!(!refusal.text(target).is_empty(), "{refusal:?}");
    }
}
