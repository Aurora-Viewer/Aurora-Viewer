//! Offline grounding fixtures: rigid foot linksets and a rigged shoe mesh
//! attached to the spine, with a much lower triangle NOT weighted to feet.
use super::*;
use crate::scene::{Scene, avatar::AvatarLibrary};
use aurora_assets::import_mesh::{self, Face, Model, Skin, Vertex, encode};
use glam::Mat4;

pub const LEFT: u32 = 9350;
pub const RIGHT: u32 = 9351;
pub const HEEL: u32 = 9352;
pub const STILT: u32 = 9353;
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

pub fn stilt(length: f32) -> ObjectUpdate {
    let mut object = rigid_objects().remove(0);
    object.local_id = STILT;
    object.full_id = u(STILT.into());
    object.position = Vec3::new(0.0, 0.0, -0.5 * length);
    object.scale = Vec3::new(0.1, 0.1, length);
    object
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
                    joints: [joint as u16, 0, 0, 0],
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

/// Move the rest origin without changing the mesh or its inverse binds.
pub fn shift_pelvis(lib: &AvatarLibrary, scene: &mut Scene, shift: Vec3) -> Option<()> {
    let mesh = scene.meshes.view(&MESH)?;
    let mut skin = mesh.skin.as_deref()?.clone();
    skin.lock_scale_if_joint_position = false;
    skin.alt_inverse_bind = skin
        .joint_names
        .iter()
        .map(|name| {
            Mat4::from_translation(if name == "mPelvis" {
                shift
            } else {
                lib.rig.local_pos[lib.rig.names[name]]
            })
        })
        .collect();
    scene
        .meshes
        .insert_local(MESH, Arc::new(mesh.bytes.first()?.to_vec()), mesh.header, Some(Arc::new(skin)));
    Some(())
}

/// Absolute pelvis tracks replace mesh root offsets during normal animation.
pub fn hold_pelvis(world: &mut crate::world::World, scene: &mut Scene) {
    const ANIM: Uuid = Uuid::from_u128(0x0585_aaaa);
    let mut animation = super::idle_animation();
    animation.base_priority = 4;
    animation.ease_in = 0.0;
    animation.joints = vec![aurora_assets::anim::JointMotion {
        joint_name: "mPelvis".into(),
        priority: 4,
        rot_keys: Vec::new(),
        pos_keys: vec![aurora_assets::anim::PosKey {
            time: 0.0,
            position: Vec3::ZERO,
        }],
    }];
    scene.anims.insert(
        ANIM,
        Arc::new(crate::scene::anim::BoundAnim::bind(animation, &world.avatar_lib.rig)),
    );
    world.apply(NetEvent::AvatarAnimations {
        avatar: world.agent_id,
        sources: Vec::new(),
        anims: vec![(ANIM, 1)],
    });
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
    if frame >= 600
        && let Some(height) = scene.automatic_hover_box_height(world)
    {
        // This fixture's local physics stands the SAME shape-derived box on
        // the plaza. The general demo simulator's fixed 0.84 m centre height
        // otherwise gives every hover capture an unrelated ~10 cm sink.
        let position = world.agent.position;
        world.agent.position.z = super::floor_at(position.x, position.y) + 0.5 * height;
        if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
            avatar.position = world.agent.position;
        }
    }
    if frame == 600
        && let Some(avatar) = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|index| world.objects.get_mut(index))
    {
        // Deliberately differ from the shape-derived physical box: auto hover
        // must not use this streamed object's size as its standing reference.
        avatar.scale.z = 1.68;
    }
    if frame >= 600 && matches!(mode, "auto-contact" | "auto-offset") {
        // Deliberately delayed and inconsistent contacts must never change
        // the avatar-only calibration or suppress automatic offset edits.
        let position = world.agent.position;
        let floor = super::floor_at(position.x, position.y);
        let plane = match frame {
            600..=719 => None,
            720..=839 => Some(glam::Vec4::new(0.0, 0.0, 1.0, floor + 0.4)),
            840..=959 => Some(glam::Vec4::ZERO),
            _ => Some(glam::Vec4::new(0.0, 0.0, 1.0, floor - 0.3)),
        };
        if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
            avatar.foot_plane = plane;
            avatar.position = world.agent.position;
        }
    }
    if mode == "auto-state" {
        match frame {
            720 => {
                world.agent.flying = true;
                if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
                    avatar.scale.z = 12.0;
                }
            }
            840 => world.agent.flying = false,
            960 => {
                if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
                    avatar.scale.z = 0.5;
                }
            }
            _ => {}
        }
    }
    if frame == 620 {
        let objects = if matches!(
            mode,
            "auto-rigged"
                | "auto-contact"
                | "auto-offset"
                | "auto-state"
                | "auto-pelvis"
                | "auto-stilt"
                | "auto-ik"
                | "auto-ik-moving"
                | "auto-ik-others"
        ) {
            if let Err(error) = install_mesh(&world.avatar_lib, scene) {
                log::warn!("demo auto hover: {error}");
                return;
            }
            vec![rigged_object()]
        } else {
            rigid_objects()
        };
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
        if mode == "auto-ik-others" {
            // An absolute pelvis animation replaces this mesh rest offset.
            // Placement must preserve the visible posture across its switch.
            let lib = world.avatar_lib.clone();
            shift_pelvis(&lib, scene, Vec3::Z * 0.28);
            hold_pelvis(world, scene);
            let mut resident = super::action_avatar(false);
            resident.local_id = 9001;
            resident.full_id = super::DEMO_LOUP;
            resident.name_values.clear();
            let position = world.agent.position + Vec3::Y * 1.2;
            let height = scene.automatic_hover_box_height(world).unwrap_or(1.9);
            resident.position = position.with_z(super::floor_at(position.x, position.y) + height * 0.5);
            resident.rotation = Quat::IDENTITY;
            resident.velocity = Vec3::ZERO;
            resident.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, super::floor_at(position.x, position.y) + 0.05));
            let mut shoes = rigged_object();
            shoes.local_id = LEFT + 20;
            shoes.full_id = u(u128::from(LEFT + 20));
            shoes.parent_id = resident.local_id;
            world.apply(NetEvent::ObjectUpdates {
                handle: HANDLE,
                objects: vec![resident, shoes],
            });
            world.apply(NetEvent::AvatarAnimations {
                avatar: super::DEMO_LOUP,
                sources: Vec::new(),
                anims: world.own_signals().iter().map(|signal| (signal.id, signal.sequence)).collect(),
            });
        }
        if matches!(mode, "auto-pelvis" | "auto-stilt") {
            hold_pelvis(world, scene);
        }
    }
    if mode == "auto-ik-others"
        && frame >= 680
        && let (Some(height), Some(hover)) = (scene.automatic_hover_box_height(world), scene.automatic_hover_base(world))
        && let Some(resident) = world
            .objects
            .index_of_uuid(&super::DEMO_LOUP)
            .and_then(|index| world.objects.get_mut(index))
    {
        // This synthetic resident uses the same shape and shoes as ours.
        // Its simulator already supplies the correct height before IK.
        resident.position.z = super::floor_at(resident.position.x, resident.position.y) + height * 0.5 + hover;
    }
    if mode == "auto-stilt" && matches!(frame, 720 | 840) {
        let length = if frame == 720 { 0.6 } else { 1.2 };
        world.apply(NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![stilt(length)],
        });
    }
    if mode == "auto-stilt" && frame == 960 {
        world.apply(NetEvent::ObjectsKilled {
            handle: HANDLE,
            local_ids: vec![STILT],
        });
    }
    if mode == "auto-pelvis" && matches!(frame, 720 | 840 | 960) {
        let z = match frame {
            720 => 10.53,
            840 => -8.0,
            _ => 0.0,
        };
        if shift_pelvis(&world.avatar_lib, scene, Vec3::Z * z).is_none() {
            log::warn!("demo auto hover: pelvis fixture unavailable");
        }
    }
    if frame == 940 && matches!(mode, "auto-cycle" | "auto-offset") {
        world.apply(NetEvent::ObjectsKilled {
            handle: HANDLE,
            local_ids: vec![LEFT, RIGHT],
        });
    }
}
