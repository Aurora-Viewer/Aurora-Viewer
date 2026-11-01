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
    *ACTIVE.get_or_init(|| {
        std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_BOM_REZZED").is_ok_and(|v| matches!(v.trim(), "1" | "pbr"))
    })
}

pub fn pbr() -> bool {
    active() && std::env::var("AURORA_DEMO_BOM_REZZED").as_deref() == Ok("pbr")
}

pub(super) const MATERIAL: Uuid = Uuid::from_u128(0xde40_1793_0000_0000_0000_0000_0000_0001);

pub fn material() -> aurora_assets::PbrMaterial {
    aurora_assets::PbrMaterial {
        base_color_texture: Some(avatar::BAKES[0].2),
        normal_texture: Some(avatar::BAKES[1].2),
        metallic_roughness_texture: Some(avatar::BAKES[2].2),
        emissive_texture: Some(avatar::BAKES[8].2),
        metallic_factor: 0.2,
        roughness_factor: 0.7,
        emissive_factor: [0.1; 3],
        ..Default::default()
    }
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
    let mut object = prim(
        LOCAL_ID,
        Vec3::new(x, y, floor_at(x, y) + 1.0),
        Quat::from_rotation_z(0.6),
        Vec3::splat(1.8),
        volume,
        Arc::new(t),
        ExtraParams::default(),
        "",
    );
    let mut objects = Vec::new();
    let mut events = Vec::new();
    if pbr() {
        object.extra.render_materials = (0..6).map(|face| (face, MATERIAL)).collect();
        let mut worn = object.clone();
        worn.local_id = LOCAL_ID + 1;
        worn.full_id = Uuid::from_u128(0xde40_1793_0000_0000_0000_0000_0000_0002);
        worn.parent_id = 9000;
        worn.state = 0x10; // chest attachment
        worn.position = Vec3::new(0.0, -0.8, 0.0);
        worn.scale = Vec3::splat(0.6);
        objects.push(worn);
        let mut bakes = TextureEntry::default();
        for (slot, _, _) in avatar::BAKES {
            bakes.faces[slot].texture = avatar::IMG_DEFAULT_AVATAR;
        }
        for (slot, texture) in [
            (8, super::TEX_BAKE_SKIN),
            (9, super::TEX_BUMPS),
            (10, super::TEX_ORM_DISC),
            (42, super::TEX_TILES),
        ] {
            bakes.faces[slot].texture = texture;
        }
        for avatar_id in [super::DEMO_AGENT, super::DEMO_LOUP] {
            events.push(NetEvent::Appearance(aurora_net::AvatarAppearance {
                avatar_id,
                texture_entry: Some(Arc::new(bakes.clone())),
                visual_params: Vec::new(),
                cof_version: 1,
                appearance_version: 1,
                hover_height: 0.0,
                attachments: if avatar_id == super::DEMO_LOUP { 2 } else { 1 },
            }));
        }
    }
    objects.push(object);
    events.insert(0, NetEvent::ObjectUpdates { handle: HANDLE, objects });
    events
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
    const N: u32 = 256;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let stripe = ((x + y) / 8) % 2 == 0;
            let c = if stripe { color } else { color.map(|v| v / 3) };
            px.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let name = avatar::bake_name(slot).unwrap_or("head").to_ascii_uppercase();
    let label = format!("BAKED_{name}");
    let left = (N as usize - label.len() * 12) / 2;
    for top in [40, 120, 200] {
        for (column, ch) in label.bytes().enumerate() {
            let rows = match ch {
                b'A' => [14, 17, 17, 31, 17, 17, 17],
                b'B' => [30, 17, 17, 30, 17, 17, 30],
                b'D' => [30, 17, 17, 17, 17, 17, 30],
                b'E' => [31, 16, 16, 30, 16, 16, 31],
                b'F' => [31, 16, 16, 30, 16, 16, 16],
                b'G' => [14, 17, 16, 23, 17, 17, 14],
                b'H' => [17, 17, 17, 31, 17, 17, 17],
                b'I' => [14, 4, 4, 4, 4, 4, 14],
                b'K' => [17, 18, 20, 24, 20, 18, 17],
                b'L' => [16, 16, 16, 16, 16, 16, 31],
                b'M' => [17, 27, 21, 21, 17, 17, 17],
                b'O' => [14, 17, 17, 17, 17, 17, 14],
                b'P' => [30, 17, 17, 30, 16, 16, 16],
                b'R' => [30, 17, 17, 30, 20, 18, 17],
                b'S' => [15, 16, 16, 14, 1, 1, 30],
                b'T' => [31, 4, 4, 4, 4, 4, 4],
                b'U' => [17, 17, 17, 17, 17, 17, 14],
                b'W' => [17, 17, 17, 21, 21, 21, 10],
                b'X' => [17, 17, 10, 4, 10, 17, 17],
                b'Y' => [17, 17, 10, 4, 4, 4, 4],
                b'1' => [4, 12, 4, 4, 4, 4, 14],
                b'2' => [14, 17, 1, 2, 4, 8, 31],
                b'3' => [30, 1, 1, 14, 1, 1, 30],
                b'_' => [0, 0, 0, 0, 0, 0, 31],
                _ => [0; 7],
            };
            for (row, bits) in rows.into_iter().enumerate() {
                for x in 0..5 {
                    if bits & (1 << (4 - x)) != 0 {
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let pixel = ((top + row * 2 + dy) * N as usize + left + column * 12 + x * 2 + dx) * 4;
                                px[pixel..pixel + 3].fill(12);
                            }
                        }
                    }
                }
            }
        }
    }
    Some((px, N, N))
}
