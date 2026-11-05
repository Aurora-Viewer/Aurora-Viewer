use super::*;
const AT_TEXTURE: i32 = 0;
use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
use std::sync::Arc;
use uuid::Uuid;

const TEXTURE: u128 = 78001;
const NO_COPY_TEXTURE: u128 = 78002;
const MATERIAL: u128 = 78003;

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
                base_mask: all,
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
    assert_eq!(allowed(&w, &sel, PickTarget::PbrMap(0), &no_copy), Ok(()));
}
