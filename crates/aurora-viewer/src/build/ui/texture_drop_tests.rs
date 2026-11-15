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
        if !ok {
            assert_eq!(allowed(&w, &sel, PickTarget::Sculpt, &texture), expected, "sculpt {mask:#x}");
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

/// willObjectAcceptInventory: a worn object takes an image whose base
/// rights are unrestricted, on top of the immediate copy and transfer
/// filter. A no-modify texture passes the filter and is refused by the
/// object, without the filter's reason.
#[test]
fn a_worn_object_refuses_a_texture_without_unrestricted_base_rights() {
    let (mut w, sel) = world();
    wear(&mut w, sel[0].idx);
    let no_modify = item(&w, NO_MODIFY_TEXTURE);
    assert_eq!(no_modify.base_mask & rules::MODIFY, 0);
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(allowed(&w, &sel, target, &item(&w, TEXTURE)), Ok(()), "{target:?}");
        assert_eq!(allowed(&w, &sel, target, &no_modify), Err(Refusal::Object), "{target:?}");
        for refused in [NO_COPY_TEXTURE, NO_TRANSFER_TEXTURE] {
            assert_eq!(
                allowed(&w, &sel, target, &item(&w, refused)),
                Err(Refusal::Restricted),
                "{target:?}"
            );
        }
    }
    let idx = sel[0].idx;
    assert!(!contents_drop::worn_refused(&w, idx, &item(&w, TEXTURE)));
    for refused in [NO_MODIFY_TEXTURE, NO_COPY_TEXTURE, NO_TRANSFER_TEXTURE] {
        assert!(contents_drop::worn_refused(&w, idx, &item(&w, refused)), "{refused}");
    }
    // the base rights count even when the agent's own rights are complete
    let mut base_limited = item(&w, TEXTURE);
    base_limited.base_mask &= !rules::MODIFY;
    assert!(contents_drop::worn_image_filter(&base_limited, w.agent_id));
    assert!(contents_drop::worn_refused(&w, idx, &base_limited));
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
    assert!(!contents_drop::worn_refused(&w, sel[0].idx, &item(&w, TEXTURE)));
}

/// The material slot on a worn object: unrestricted base rights, like an
/// image, whatever the object's contents hold.
#[test]
fn a_worn_object_takes_only_a_material_with_unrestricted_base_rights() {
    let (mut w, sel) = world();
    wear(&mut w, sel[0].idx);
    let mut material = item(&w, MATERIAL);
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMaterial, &material), Ok(()));
    assert!(!contents_drop::worn_refused(&w, sel[0].idx, &material));
    for missing in [rules::MODIFY, rules::COPY, rules::TRANSFER] {
        material.base_mask = contents_drop::UNRESTRICTED & !missing;
        assert_eq!(
            allowed(&w, &sel, PickTarget::PbrMaterial, &material),
            Err(Refusal::Object),
            "{missing:#x}"
        );
        assert!(contents_drop::worn_refused(&w, sel[0].idx, &material), "{missing:#x}");
    }
}

/// On a worn object the rights that count are the agent's: the everyone
/// mask of a group-owned item and of someone else's, never their owner
/// mask; the base rights stay unrestricted for both.
#[test]
fn a_worn_object_reads_the_agents_rights_on_a_group_owned_or_someone_elses_item() {
    let (mut w, sel) = world();
    wear(&mut w, sel[0].idx);
    let idx = sel[0].idx;
    let all = contents_drop::UNRESTRICTED;
    let both = rules::COPY | rules::TRANSFER;
    let group = InvItem {
        owner: Uuid::from_u128(0x6a),
        group_owned: true,
        owner_mask: all,
        everyone_mask: rules::COPY,
        ..item(&w, TEXTURE)
    };
    let others = InvItem {
        owner: Uuid::from_u128(0xb7),
        group_owned: false,
        ..group.clone()
    };
    for (what, mut texture) in [("group-owned", group), ("someone else's", others)] {
        assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &texture), Err(Refusal::Restricted), "{what}");
        assert!(contents_drop::worn_refused(&w, idx, &texture), "{what}");
        texture.everyone_mask = both;
        assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &texture), Ok(()), "{what}");
        assert!(!contents_drop::worn_refused(&w, idx, &texture), "{what}");
        texture.base_mask = both;
        assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &texture), Err(Refusal::Object), "{what}");
        assert!(contents_drop::worn_refused(&w, idx, &texture), "{what}");
        // a material: unrestricted base rights and a transfer right
        let mut material = InvItem {
            asset_type: AT_MATERIAL,
            inv_type: AT_MATERIAL,
            base_mask: all,
            everyone_mask: rules::COPY,
            ..texture
        };
        assert!(contents_drop::worn_refused(&w, idx, &material), "{what} material without transfer");
        material.everyone_mask = rules::TRANSFER;
        assert!(!contents_drop::worn_refused(&w, idx, &material), "{what} material");
    }
    // the agent's own material needs no everyone right
    let own = InvItem {
        everyone_mask: 0,
        ..item(&w, MATERIAL)
    };
    assert!(!contents_drop::worn_refused(&w, idx, &own));
}

/// A restriction from the worn object itself, as its script would send it.
fn rlva_restrict(w: &mut World, source: Uuid, message: &str) {
    assert!(w.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Controller".to_owned(),
        source_id: source,
        owner_id: w.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: message.to_owned(),
    }));
}

/// willObjectAcceptInventory: RLVa's locked attachment refuses every drop,
/// a full-permission texture or material included, and its contents too.
#[test]
fn a_locked_attachment_refuses_every_drop() {
    let (mut w, sel) = world();
    wear(&mut w, sel[0].idx);
    let idx = sel[0].idx;
    let object = w.objects.get(idx).expect("selected prim").full_id;
    w.rlva_configure(true);
    let (texture, material) = (item(&w, TEXTURE), item(&w, MATERIAL));
    assert!(!contents_drop::attachment_locked(&w, idx), "no lock yet");
    assert_eq!(allowed(&w, &sel, PickTarget::Diffuse, &texture), Ok(()));
    rlva_restrict(&mut w, object, "@detach=n");
    assert!(contents_drop::attachment_locked(&w, idx));
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(allowed(&w, &sel, target, &texture), Err(Refusal::Object), "{target:?}");
    }
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMaterial, &material), Err(Refusal::Object));
    assert!(contents_drop::worn_refused(&w, idx, &texture));
    assert!(contents_drop::worn_refused(&w, idx, &material));
    assert_eq!(contents_drop::accepts(&w, idx, &texture), Err("rez-contents-denied"));
    // the same object in the world is not an attachment
    w.objects.get_mut(idx).expect("selected prim").parent_id = 0;
    assert!(!contents_drop::attachment_locked(&w, idx));
    assert!(!contents_drop::worn_refused(&w, idx, &texture));
}

#[test]
fn only_a_worn_selection_narrows_the_picker() {
    let (mut w, sel) = world();
    let both = rules::COPY | rules::TRANSFER;
    assert_eq!(picker_permissions(&w, &sel, PickTarget::Diffuse), 0);
    // the map and sculpt slots list copy and transfer textures anywhere
    for target in [PickTarget::PbrMap(0), PickTarget::Sculpt] {
        assert_eq!(picker_permissions(&w, &sel, target), both, "{target:?}");
    }
    wear(&mut w, sel[0].idx);
    for target in [PickTarget::Diffuse, PickTarget::Normal, PickTarget::Specular] {
        assert_eq!(picker_permissions(&w, &sel, target), both, "{target:?}");
    }
    assert_eq!(picker_permissions(&w, &sel, PickTarget::PbrMaterial), 0);
    assert_eq!(picker_permissions(&w, &sel, PickTarget::PbrMap(0)), both);
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
