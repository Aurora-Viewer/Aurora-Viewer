//! AURORA_DEMO_BOM_REZZED: a box on the ground whose faces carry the
//! bake-on-mesh placeholders, as a BOM body part rezzed out.

use super::{HANDLE, floor_at, prim, shape};
use crate::scene::avatar;
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::TextureEntry;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const LOCAL_ID: u32 = 81_140;
/// Head, upper body and lower body, round the faces of the box.
const SLOTS: [usize; 3] = [8, 9, 10];

fn active() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE
        .get_or_init(|| std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_BOM_REZZED").is_ok_and(|v| v.trim() == "1"))
}

/// AURORA_DEMO_BOM_REZZED=1: the box, ahead of the start view. Not worn,
/// it shows the placeholders themselves (LLViewerObject::getBakedTextureForMagicId),
/// never white.
pub fn events() -> Vec<NetEvent> {
    if !active() {
        return Vec::new();
    }
    let mut t = TextureEntry::default();
    for (i, f) in t.faces.iter_mut().enumerate() {
        f.texture = avatar::BAKES
            .iter()
            .find(|(te, _, _)| *te == SLOTS[i % SLOTS.len()])
            .map_or(Uuid::nil(), |(_, _, id)| *id);
        f.color = [1.0; 4];
    }
    let (x, y) = (135.0, 129.5);
    let volume = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let object = prim(
        LOCAL_ID,
        Vec3::new(x, y, floor_at(x, y) + 1.0),
        Quat::from_rotation_z(0.6),
        Vec3::splat(1.8),
        volume,
        Arc::new(t),
        ExtraParams::default(),
        "",
    );
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    }]
}

/// Stand-ins for the grid's placeholder textures: diagonal stripes, one
/// colour per bake slot (head orange, upper body teal, lower body violet).
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if !active() {
        return None;
    }
    let (slot, _) = avatar::bake_slot_for(id)?;
    let color: [u8; 3] = match slot {
        8 => [235, 140, 50],
        9 => [40, 190, 170],
        _ => [150, 110, 235],
    };
    const N: u32 = 64;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let stripe = ((x + y) / 8) % 2 == 0;
            let c = if stripe { color } else { color.map(|v| v / 3) };
            px.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Some((px, N, N))
}
