//! What is kept of an avatar's worn objects: each root it wears in the
//! world (not the HUDs, which only their wearer sees and could not click
//! in a copy) with its prims, as they are drawn now, and only what drawing
//! them needs. The text over them, their media, sounds, particles, click
//! actions and name-values (the AttachItemID among them) are left out: a
//! copy is never taken for the object, by the outfit, RLVa or a click.

use super::file::{WornObject, WornPrim, WornSet};
use crate::world::World;
use crate::world::objects::Object;
use aurora_net::objects::{ObjectUpdate, ParticleUpdate};
use glam::Vec3;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

/// The HUD attachment points (LLViewerJointAttachment::getIsHUDAttachment).
pub fn is_hud_point(point: u8) -> bool {
    (31..=38).contains(&point)
}

/// The update that draws `object` again, without what a copy leaves out.
/// The ids, parent and flags are the copy's to set.
pub fn drawing_update(object: &Object) -> ObjectUpdate {
    ObjectUpdate {
        local_id: object.key.local_id,
        full_id: object.full_id,
        parent_id: object.parent_id,
        pcode: object.pcode,
        state: object.state,
        crc: 0,
        material: object.prim_material,
        click_action: 0,
        scale: object.scale,
        position: object.position,
        rotation: object.rotation,
        velocity: Vec3::ZERO,
        acceleration: Vec3::ZERO,
        // a spinning attachment keeps spinning (llTargetOmega)
        angular_velocity: object.angular_velocity,
        update_flags: 0,
        owner_id: object.owner_id,
        volume: object.volume,
        texture_entry: object.te.clone(),
        extra: object.extra.clone(),
        name_values: String::new(),
        text: String::new(),
        text_color: [0; 4],
        media_url: String::new(),
        tree_species: object.tree_species,
        texture_anim: object.tex_anim,
        foot_plane: None,
        particles: ParticleUpdate::Clear,
        sound: Default::default(),
    }
}

/// The worn roots of the avatar at `avatar` a copy can be made of: the
/// region's objects (not the viewer's own previews, ours included), not
/// the HUDs.
pub fn worn_roots(world: &World, avatar: usize) -> Vec<usize> {
    let Some(av) = world.objects.get(avatar) else {
        return Vec::new();
    };
    world
        .objects
        .children_of(&av.key)
        .iter()
        .copied()
        .filter(|&i| {
            world
                .objects
                .get(i)
                .is_some_and(|o| !o.is_client_preview() && o.is_attachment() && !is_hud_point(o.attachment_point()))
        })
        .collect()
}

/// The prims of the worn root at `root`, the root first (an avatar sitting
/// on it is not part of it).
fn prims_of(world: &World, root: usize) -> Vec<usize> {
    world
        .objects
        .subtree(root)
        .into_iter()
        .filter(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar() && !o.is_client_preview()))
        .collect()
}

/// What changes when what the avatar wears or how it looks changes: its
/// roots, and each prim's count of full updates and GLTF overrides.
pub fn signature(world: &World, avatar: usize) -> u64 {
    let mut roots: Vec<(Uuid, usize)> = worn_roots(world, avatar)
        .into_iter()
        .filter_map(|i| world.objects.get(i).map(|o| (o.full_id, i)))
        .collect();
    roots.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (id, root) in roots {
        id.hash(&mut h);
        for p in prims_of(world, root) {
            if let Some(o) = world.objects.get(p) {
                o.full_update_serial.hash(&mut h);
                world
                    .gltf_overrides
                    .get(&o.key)
                    .map(|sides| std::sync::Arc::as_ptr(sides) as *const u8 as usize)
                    .hash(&mut h);
            }
        }
    }
    h.finish()
}

/// The GLTF overrides of a prim, as the file keeps them: LLSD binary of an
/// array of `[face, override]`.
fn overrides_of(world: &World, object: &Object) -> Vec<u8> {
    let Some(sides) = world.gltf_overrides.get(&object.key) else {
        return Vec::new();
    };
    let list = sides
        .iter()
        .map(|(face, o)| aurora_llsd::Llsd::Array(vec![aurora_llsd::Llsd::Integer(i32::from(*face)), o.to_llsd()]))
        .collect();
    aurora_llsd::to_binary(&aurora_llsd::Llsd::Array(list))
}

/// Read back what [`overrides_of`] wrote.
pub fn read_overrides(d: &[u8]) -> Vec<(u8, aurora_assets::material::PbrOverride)> {
    if d.is_empty() {
        return Vec::new();
    }
    let Ok((list, _)) = aurora_llsd::from_binary_local(d) else {
        return Vec::new();
    };
    list.as_array()
        .iter()
        .filter_map(|side| {
            let face = u8::try_from(side.at(0).as_i32()).ok()?;
            Some((face, aurora_assets::material::PbrOverride::from_llsd(side.at(1))))
        })
        .collect()
}

/// The worn set of the avatar at `avatar`, written at `saved_at` (Unix
/// seconds).
pub fn capture(world: &World, avatar: usize, saved_at: u64) -> WornSet {
    let mut objects = Vec::new();
    let mut room = super::file::MAX_PRIMS;
    for root in worn_roots(world, avatar).into_iter().take(super::file::MAX_OBJECTS) {
        let Some(r) = world.objects.get(root) else { continue };
        let ids = prims_of(world, root);
        // (an object that does not fit whole is left out: a part of it
        // would be drawn as if it were all)
        if ids.len() > room {
            continue;
        }
        room -= ids.len();
        let prims = ids
            .into_iter()
            .filter_map(|i| world.objects.get(i))
            .map(|o| WornPrim {
                update_flags: 0,
                data: aurora_net::objects::encode_compressed(&drawing_update(o)),
                overrides: overrides_of(world, o),
            })
            .collect();
        objects.push(WornObject {
            root_id: r.full_id,
            item_id: r.attachment_item_id().unwrap_or_default(),
            point: r.attachment_point(),
            prims,
        });
    }
    WornSet { saved_at, objects }
}
