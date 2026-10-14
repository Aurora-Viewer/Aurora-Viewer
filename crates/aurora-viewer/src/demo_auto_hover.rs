//! Offline grounding fixtures: rigid foot linksets and a rigged shoe mesh
//! attached to the spine, with a much lower triangle NOT weighted to feet.
use super::*;
use crate::scene::{Scene, avatar::AvatarLibrary};
use aurora_assets::import_mesh::{self, Face, Model, Skin, Vertex, encode};
use glam::Mat4;

pub const LEFT: u32 = 9350;
pub const RIGHT: u32 = 9351;
pub const HEEL: u32 = 9352;
pub const MESH: Uuid = Uuid::from_u128(0xd0d0_aaaa_0000_0000_0000_0000_0000_9350);

pub fn rigid_objects() -> Vec<ObjectUpdate> {
    let shoe = |id, point: u8| {
        let mut o = worn::root();
        o.local_id = id;
        o.full_id = u(id.into());
        o.state = point.rotate_left(4);
        o.position = Vec3::new(0.03, 0.0, -0.07);
        o.rotation = Quat::IDENTITY;
        o.scale = Vec3::new(0.28, 0.14, 0.14);
        o.texture_entry = Some(te([0.54, 0.36, 0.96, 1.0], 0, false, 0.0));
        o
    };
    let left = shoe(LEFT, 7);
    let right = shoe(RIGHT, 8);
    let mut heel = shoe(HEEL, 0);
    heel.parent_id = LEFT;
    heel.position = Vec3::new(-0.07, 0.0, -0.10);
    heel.scale = Vec3::new(0.08, 0.08, 0.10);
    vec![left, right, heel]
}

/// Raw foot-space model, before normalization and asset quantization.
pub fn rigged_model(lib: &AvatarLibrary) -> Model {
    let names: Vec<String> = ["mFootLeft", "mFootRight", "mPelvis"].iter().map(|s| (*s).into()).collect();
    let mut faces = Vec::new();
    for (joint, name) in names.iter().take(2).enumerate() {
        let center = lib.rig.default_world_inv[lib.rig.names[name]].inverse().w_axis.truncate() + Vec3::new(0.02, 0.0, -0.09);
        let vertices = (0..8)
            .map(|i| {
                let p = center
                    + Vec3::new(
                        if i & 1 == 0 { -0.14 } else { 0.14 },
                        if i & 2 == 0 { -0.07 } else { 0.07 },
                        if i & 4 == 0 { -0.09 } else { 0.09 },
                    );
                Vertex {
                    position: p.to_array(),
                    normal: Vec3::Z.to_array(),
                    uv: [0.0; 2],
                    joints: [joint as u8, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                }
            })
            .collect();
        faces.push(Face {
            vertices,
            indices: vec![
                0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5,
            ],
            material: joint,
        });
    }
    // The zero foot influences on this low geometry must not raise the
    // avatar by four metres merely because feet appear in its joint table.
    faces.push(Face {
        vertices: (0..3)
            .map(|i| Vertex {
                position: [i as f32 * 0.05, 0.0, -4.0],
                normal: Vec3::Z.to_array(),
                uv: [0.0; 2],
                joints: [2, 0, 1, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            })
            .collect(),
        indices: vec![0, 1, 2],
        material: 2,
    });
    Model {
        name: "Grounding shoes".into(),
        submodel: 0,
        faces,
        skin: Some(Skin {
            inverse_bind: names.iter().map(|name| lib.rig.default_world_inv[lib.rig.names[name]]).collect(),
            names,
            alternate_bind: Vec::new(),
            bind_shape: Mat4::IDENTITY,
            pelvis_offset: 0.0,
        }),
        center: Vec3::ZERO,
        size: Vec3::ONE,
    }
}

pub fn install_mesh(lib: &AvatarLibrary, scene: &mut Scene) -> Result<(), aurora_assets::AssetError> {
    let mut model = rigged_model(lib);
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
    scene.meshes.insert_local(MESH, Arc::new(data), header, skin);
    Ok(())
}

pub fn rigged_object() -> ObjectUpdate {
    let mut o = worn::root();
    o.local_id = LEFT;
    o.full_id = u(LEFT.into());
    o.position = Vec3::ZERO;
    o.scale = Vec3::ONE;
    o.extra.sculpt = Some(SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    });
    o.volume.sculpt = o.extra.sculpt;
    o
}

pub fn tick(world: &mut crate::world::World, scene: &mut Scene, frame: u64, mode: &str) {
    if !mode.starts_with("auto-") {
        return;
    }
    if frame == 600
        && let Some(avatar) = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|index| world.objects.get_mut(index))
    {
        // Match AgentState::simulate_locally's 0.84 m centre clearance.
        // The generic avatar fixture reports 1.9 m despite this physics.
        avatar.scale.z = if mode == "auto-contact" { 1.48 } else { 1.68 };
    }
    if frame >= 600 && mode == "auto-contact" {
        // A 10 cm collision margin: physical half-height is 0.74 m while
        // the simulated centre actually stands 0.84 m above the ground.
        // The demo stands on its plaza platform, above the terrain below.
        // Use the same floor as its local movement simulator.
        let position = world.agent.position;
        let plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, super::floor_at(position.x, position.y)));
        if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
            avatar.foot_plane = plane;
            avatar.position = world.agent.position;
        }
    }
    if frame == 620 {
        let objects = if matches!(mode, "auto-rigged" | "auto-contact") {
            if let Err(error) = install_mesh(&world.avatar_lib, scene) {
                log::warn!("demo auto hover: {error}");
                return;
            }
            vec![rigged_object()]
        } else {
            rigid_objects()
        };
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
    }
    if frame == 940 && mode == "auto-cycle" {
        world.apply(NetEvent::ObjectsKilled {
            handle: HANDLE,
            local_ids: vec![LEFT, RIGHT],
        });
    }
}
