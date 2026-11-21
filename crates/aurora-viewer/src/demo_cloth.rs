//! AURORA_DEMO_CLOTH=1|spin|walk: our avatar wears a rigged flared dress and
//! long hair cards built here (scene/cloth). The dress is weighted as most
//! Second Life skirts are, the pelvis at the waist blending into the hips
//! down to the hem; the hair cards to the head only. Neither carries any
//! sign that it should swing: the cloth works it out. `1`: drive the avatar
//! yourself; `spin`: it turns back and forth on the spot; `walk`: it paces
//! along x, both from frame 360 on, the camera beside it.

use super::*;
use crate::scene::{Scene, avatar::AvatarLibrary};
use aurora_assets::import_mesh::{self, Face, Model, Skin, Vertex, encode};
use glam::Mat4;
use std::f32::consts::{PI, TAU};

const SKIRT: u32 = 9_480_001;
const HAIR: u32 = 9_480_002;
const SKIRT_MESH: Uuid = Uuid::from_u128(0xc107_0000_0000_0000_0000_0000_9480_0001);
const HAIR_MESH: Uuid = Uuid::from_u128(0xc107_0000_0000_0000_0000_0000_9480_0002);
/// The frame the avatar wears them (once its skeleton stands).
const WEAR: u64 = 240;
/// The frame the motion of `spin` and `walk` starts.
const MOVE: u64 = 360;

/// Where `walk` starts: the avatar's place at frame `MOVE`.
static ORIGIN: std::sync::Mutex<Option<Vec3>> = std::sync::Mutex::new(None);

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_CLOTH").ok()
}

/// A joint's place at bind pose.
fn joint(lib: &AvatarLibrary, name: &str) -> Vec3 {
    lib.rig.default_world_inv[lib.rig.names[name]].inverse().w_axis.truncate()
}

fn skin(lib: &AvatarLibrary, names: &[&str]) -> Skin {
    Skin {
        inverse_bind: names.iter().map(|n| lib.rig.default_world_inv[lib.rig.names[*n]]).collect(),
        names: names.iter().map(|n| (*n).into()).collect(),
        alternate_bind: Vec::new(),
        bind_shape: Mat4::IDENTITY,
        pelvis_offset: 0.0,
    }
}

/// A flared dress from the waist down to the ankles: joints 0 pelvis, 1 left
/// hip, 2 right hip (+y is the avatar's left).
fn skirt_model(lib: &AvatarLibrary) -> Model {
    let pelvis = joint(lib, "mPelvis");
    let (seg, rings) = (64u32, 24u32);
    let waist = pelvis.z + 0.1;
    let mut vertices = Vec::new();
    for r in 0..=rings {
        let t = r as f32 / rings as f32;
        let radius = 0.17 + 0.3 * t.powf(1.3);
        let z = waist - 0.92 * t;
        for i in 0..=seg {
            let a = TAU * i as f32 / seg as f32;
            let (s, c) = a.sin_cos();
            // into the hip of its side as it goes down
            let hip = (0.75 * t).min(1.0) * (s.abs() * 0.5 + 0.5);
            let side = if s >= 0.0 { 1 } else { 2 };
            vertices.push(Vertex {
                position: [pelvis.x + radius * c, pelvis.y + radius * s, z],
                normal: [c, s, 0.0],
                uv: [i as f32 / seg as f32, t],
                joints: [0, side, 0, 0],
                weights: [1.0 - hip, hip, 0.0, 0.0],
            });
        }
    }
    let mut indices = Vec::new();
    let row = seg + 1;
    for r in 0..rings {
        for i in 0..seg {
            let (a, b) = (r * row + i, r * row + i + 1);
            let (c, d) = (a + row, b + row);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Model {
        name: "Cloth demo dress".into(),
        submodel: 0,
        faces: vec![Face {
            vertices,
            indices,
            material: 0,
        }],
        skin: Some(skin(lib, &["mPelvis", "mHipLeft", "mHipRight"])),
        center: Vec3::ZERO,
        size: Vec3::ONE,
    }
}

/// Twelve cards of long hair from the back of the head down the back, each
/// its own piece, skinned to the head alone.
fn hair_model(lib: &AvatarLibrary) -> Model {
    let head = joint(lib, "mHead");
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let (rows, cards) = (20u32, 12u32);
    for k in 0..cards {
        let y = (k as f32 / (cards - 1) as f32 - 0.5) * 0.19;
        let base = vertices.len() as u32;
        for r in 0..=rows {
            let t = r as f32 / rows as f32;
            // from the crown round the back of the skull, then down
            let back = -0.085 - 0.06 * (t * PI).sin() * 0.5 - 0.03 * t;
            let z = head.z + 0.07 - 0.6 * t;
            for e in [-1.0f32, 1.0] {
                vertices.push(Vertex {
                    position: [head.x + back, head.y + y + e * 0.012, z],
                    normal: [-1.0, 0.0, 0.0],
                    uv: [(e + 1.0) * 0.5, t],
                    joints: [0, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                });
            }
        }
        for r in 0..rows {
            // (wound to face away from the head, -x)
            let (a, b, c, d) = (base + 2 * r, base + 2 * r + 1, base + 2 * r + 2, base + 2 * r + 3);
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    Model {
        name: "Cloth demo hair".into(),
        submodel: 0,
        faces: vec![Face {
            vertices,
            indices,
            material: 0,
        }],
        skin: Some(skin(lib, &["mHead"])),
        center: Vec3::ZERO,
        size: Vec3::ONE,
    }
}

fn install(scene: &mut Scene, id: Uuid, mut model: Model) -> Result<(), aurora_assets::AssetError> {
    model.normalize()?;
    let data = encode::asset(
        &[None, None, None, Some(model)],
        None,
        &import_mesh::process::Physics::default(),
        encode::Options {
            skin: true,
            ..Default::default()
        },
    )?;
    let header = aurora_assets::parse_mesh_header(&data)?;
    let skin = header
        .skin
        .and_then(|range| aurora_assets::mesh::section_bytes(&data, range))
        .map(aurora_assets::decode_skin)
        .transpose()?
        .map(Arc::new);
    scene.meshes.insert_local(id, Arc::new(data), header, skin);
    Ok(())
}

fn worn_mesh(local_id: u32, mesh: Uuid, color: [f32; 4]) -> ObjectUpdate {
    let mut o = worn::root();
    o.local_id = local_id;
    o.full_id = u(local_id.into());
    o.position = Vec3::ZERO;
    o.scale = Vec3::ONE;
    o.texture_entry = Some(te(color, 0, false, 0.0));
    o.extra.sculpt = Some(SculptParams {
        texture: mesh,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    });
    o.volume.sculpt = o.extra.sculpt;
    o
}

pub fn tick(world: &mut crate::world::World, scene: &mut Scene, frame: u64) {
    let Some(mode) = mode() else {
        return;
    };
    if frame == WEAR {
        let lib = world.avatar_lib.clone();
        if let Err(error) = install(scene, SKIRT_MESH, skirt_model(&lib)).and_then(|_| install(scene, HAIR_MESH, hair_model(&lib))) {
            log::warn!("demo cloth: {error}");
            return;
        }
        world.apply(NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![
                worn_mesh(SKIRT, SKIRT_MESH, [0.62, 0.12, 0.35, 1.0]),
                worn_mesh(HAIR, HAIR_MESH, [0.85, 0.55, 0.2, 1.0]),
            ],
        });
    }
    if frame < MOVE {
        return;
    }
    let t = (frame - MOVE) as f32 / 60.0;
    let origin = *ORIGIN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(world.agent.position);
    match mode.as_str() {
        // a half turn each way and back, faster and slower
        "spin" => {
            let yaw = 1.6 * (t * 1.4).sin() * (0.6 + 0.4 * (t * 0.31).sin());
            world.agent.yaw = yaw;
            if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
                avatar.rotation = Quat::from_rotation_z(yaw);
            }
        }
        // three metres along x and back, a pause at each end
        "walk" => {
            let phase = (t / 5.0).fract();
            let s = match phase {
                p if p < 0.4 => p / 0.4,
                p if p < 0.5 => 1.0,
                p if p < 0.9 => 1.0 - (p - 0.5) / 0.4,
                _ => 0.0,
            };
            let x = origin.x + 3.0 * s;
            world.agent.position.x = x;
            world.agent.yaw = if (0.0..0.5).contains(&phase) { 0.0 } else { PI };
            if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
                avatar.position.x = x;
                avatar.rotation = Quat::from_rotation_z(world.agent.yaw);
            }
        }
        _ => {}
    }
}

/// The camera beside the avatar once the motion starts (`spin`, `walk`).
pub fn view(frame: u64, avatar: Vec3) -> Option<(Vec3, Vec3)> {
    let mode = mode()?;
    if frame != MOVE || !matches!(mode.as_str(), "spin" | "walk") {
        return None;
    }
    let at = avatar + Vec3::new(if mode == "walk" { 1.5 } else { 0.0 }, 0.0, -0.2);
    Some((at + Vec3::new(-0.6, -3.6, 0.5), at))
}
