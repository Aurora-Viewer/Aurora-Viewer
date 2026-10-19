//! AURORA_DEMO_RIGGED_PROP: a chain made as a rigged mesh, uploaded the
//! usual way (its vertices normalized, the bind shape putting them back on
//! the skeleton), rezzed on a cube in front of the start position, and the
//! same mesh worn by our avatar. The rezzed one is a static mesh filling its
//! object box (LLVolumeGeometryManager::rebuildGeom draws a skinned mesh
//! rigged only when worn or a playing animated object); the worn one hangs
//! down our back.

use super::*;
use crate::scene::avatar::AvatarLibrary;
use aurora_assets::import_mesh::{self, Face, Model, Skin, Vertex, encode};
use glam::Mat4;

pub const MESH: Uuid = Uuid::from_u128(0xD0D0_C4A1_0000_0000_0000_0000_0000_0001);
pub const CUBE: u32 = 9760;
pub const CHAIN: u32 = 9761;
pub const WORN: u32 = 9762;
/// Links of the chain, and the segments around a link and its tube.
const LINKS: usize = 5;
const AROUND: usize = 16;
const TUBE: usize = 6;

pub fn enabled() -> bool {
    std::env::var_os("AURORA_DEMO_RIGGED_PROP").is_some()
}

/// The chain where the uploader had it: hanging down the back of the
/// default skeleton (seen from the start camera), bound to the chest, each
/// link turned a quarter from the last.
fn model(lib: &AvatarLibrary) -> Option<Model> {
    let chest = *lib.rig.names.get("mChest")?;
    let top = lib.rig.default_world_inv.get(chest)?.inverse().w_axis.truncate() + Vec3::new(-0.16, 0.0, 0.1);
    let (long, wide, tube) = (0.07, 0.04, 0.009);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for link in 0..LINKS {
        let center = top - Vec3::Z * (link as f32 * long * 1.5);
        let turn = Quat::from_rotation_z(if link % 2 == 0 { 0.0 } else { std::f32::consts::FRAC_PI_2 });
        let base = vertices.len() as u32;
        for a in 0..AROUND {
            let theta = a as f32 / AROUND as f32 * std::f32::consts::TAU;
            // an oval in the XZ plane, its tube around it
            let ring = Vec3::new(theta.cos() * wide, 0.0, theta.sin() * long);
            let out = Vec3::new(theta.cos() * long, 0.0, theta.sin() * wide).normalize();
            for t in 0..TUBE {
                let phi = t as f32 / TUBE as f32 * std::f32::consts::TAU;
                let normal = out * phi.cos() + Vec3::Y * phi.sin();
                vertices.push(Vertex {
                    position: (center + turn * (ring + normal * tube)).to_array(),
                    normal: (turn * normal).to_array(),
                    uv: [a as f32 / AROUND as f32, t as f32 / TUBE as f32],
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                });
            }
        }
        let at = |a: usize, t: usize| base + ((a % AROUND) * TUBE + t % TUBE) as u32;
        for a in 0..AROUND {
            for t in 0..TUBE {
                indices.extend([at(a, t), at(a + 1, t), at(a + 1, t + 1), at(a, t), at(a + 1, t + 1), at(a, t + 1)]);
            }
        }
    }
    Some(Model {
        name: "Demo chain".into(),
        submodel: 0,
        faces: vec![Face {
            vertices,
            indices,
            material: 0,
        }],
        skin: Some(Skin {
            names: vec!["mChest".into()],
            inverse_bind: vec![lib.rig.default_world_inv[chest]],
            alternate_bind: Vec::new(),
            bind_shape: Mat4::IDENTITY,
            pelvis_offset: 0.0,
        }),
        center: Vec3::ZERO,
        size: Vec3::ONE,
    })
}

/// The mesh asset in the scene's mesh cache; the size the upload gives the
/// object.
fn install(lib: &AvatarLibrary, scene: &mut crate::scene::Scene) -> Result<Vec3, aurora_assets::AssetError> {
    let mut model = model(lib).ok_or_else(|| aurora_assets::AssetError::invalid("no mChest joint"))?;
    model.normalize()?;
    let size = model.size;
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
    scene.meshes.insert_local(MESH, Arc::new(data), header, skin);
    Ok(size)
}

fn chain(local: u32, pos: Vec3, scale: Vec3) -> ObjectUpdate {
    let sculpt = SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    prim(
        local,
        pos,
        Quat::IDENTITY,
        scale,
        aurora_prim::VolumeParams {
            sculpt: Some(sculpt),
            ..Default::default()
        },
        te([0.85, 0.86, 0.9, 1.0], 2, false, 0.0),
        ExtraParams {
            sculpt: Some(sculpt),
            ..Default::default()
        },
        "",
    )
}

/// The cube 3 m ahead of the start position and 1 m to the left (clear of
/// our avatar in the start view), the chain standing on it, and the worn
/// copy.
fn objects(size: Vec3, wearer: Option<u32>) -> Vec<ObjectUpdate> {
    let (x, y) = (132.0 + 0.894 * 3.0 - 0.447, 126.0 + 0.447 * 3.0 + 0.894);
    let ground = floor_at(x, y);
    let mut objects = vec![
        prim(
            CUBE,
            Vec3::new(x, y, ground + 0.25),
            Quat::IDENTITY,
            Vec3::splat(0.5),
            aurora_prim::VolumeParams::default(),
            te([0.2, 0.2, 0.2, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            "",
        ),
        chain(CHAIN, Vec3::new(x, y, ground + 0.5 + size.z * 0.5), size),
    ];
    if let Some(wearer) = wearer {
        let mut worn = chain(WORN, Vec3::ZERO, size);
        worn.parent_id = wearer;
        worn.state = 0x10; // chest
        objects.push(worn);
    }
    objects
}

/// The mesh, then the objects, a little after the arrival.
pub fn tick(world: &mut crate::world::World, scene: &mut crate::scene::Scene, frame: u64) {
    if frame != 120 || !enabled() {
        return;
    }
    let size = match install(&world.avatar_lib, scene) {
        Ok(size) => size,
        Err(e) => {
            log::warn!("demo rigged prop: {e}");
            return;
        }
    };
    let wearer = world
        .objects
        .index_of_uuid(&world.agent_id)
        .and_then(|i| world.objects.get(i))
        .map(|o| o.key.local_id);
    log::info!("demo rigged prop: chain {size:.3?}, worn by {wearer:?}");
    world.apply(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: objects(size, wearer),
    });
}
