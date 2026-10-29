//! Offline mesh-head fixture through the ordinary asset/skin/BOM path.

use crate::{scene::avatar::AvatarLibrary, world::World};
use aurora_assets::import_mesh::{self as codec, encode};
use glam::{Mat4, Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub const ASSET: Uuid = Uuid::from_u128(0xA0E0_5A9E_0000_0000_0000_0000_0000_0001);
const ITEM: Uuid = Uuid::from_u128(0xA0E0_5A9E_0000_0000_0000_0000_0000_0002);
pub const TINT: [f32; 4] = [0.25, 0.65, 1.0, 1.0];

/// Synthetic customise motions for checking the camera's half-turn offline.
pub fn customize_animation(done: bool) -> aurora_assets::Animation {
    let rotation = |time, angle| aurora_assets::anim::RotKey {
        time,
        rotation: Quat::from_rotation_z(angle),
    };
    aurora_assets::Animation {
        base_priority: 6,
        duration: 2.0,
        emote_name: String::new(),
        loop_in: 1.35,
        loop_out: 2.0,
        looping: !done,
        ease_in: if done { 1.35 } else { 0.0 },
        ease_out: 1.35,
        hand_pose: 0,
        joints: vec![aurora_assets::JointMotion {
            joint_name: "mPelvis".into(),
            priority: -1,
            rot_keys: if done {
                vec![rotation(0.0, 0.0)]
            } else {
                vec![
                    rotation(0.0, 0.0),
                    rotation(1.35, std::f32::consts::PI),
                    rotation(2.0, std::f32::consts::PI),
                ]
            },
            pos_keys: Vec::new(),
        }],
    }
}

pub struct MeshAsset {
    pub data: Arc<Vec<u8>>,
    pub header: aurora_assets::MeshHeader,
    pub skin: Option<Arc<aurora_assets::SkinInfo>>,
}

/// Called on a worker: compression and decoding never run in a frame.
pub fn asset(library: &AvatarLibrary, rigged: bool) -> Result<MeshAsset, String> {
    build(library, rigged, false)
}
pub fn layered_asset(library: &AvatarLibrary) -> Result<MeshAsset, String> {
    build(library, true, true)
}
fn build(library: &AvatarLibrary, rigged: bool, layered: bool) -> Result<MeshAsset, String> {
    let part = library.parts.iter().find(|p| p.name == "head").ok_or("missing head fixture")?;
    let joint = *library.rig.names.get("mHead").ok_or("missing head joint")?;
    let inverse = library.rig.default_world_inv[joint];
    let mut model = codec::Model {
        name: "shape-head-fixture".into(),
        submodel: 0,
        faces: vec![codec::Face {
            vertices: part
                .vertices
                .iter()
                .map(|vertex| {
                    let position = Vec3::from_array(vertex.pos);
                    let normal = Vec3::new(vertex.normal[0] as f32, vertex.normal[1] as f32, vertex.normal[2] as f32).normalize_or(Vec3::Z);
                    codec::Vertex {
                        position: if rigged { position } else { inverse.transform_point3(position) }.to_array(),
                        normal: normal.to_array(),
                        // The system library has top-left UVs; mesh assets
                        // store SL texture coordinates (bottom-left).
                        uv: [vertex.uv[0], 1.0 - vertex.uv[1]],
                        joints: [0; 4],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    }
                })
                .collect(),
            indices: part.indices.iter().map(|index| u32::from(*index)).collect(),
            material: 0,
        }],
        skin: rigged.then(|| codec::Skin {
            names: vec!["mHead".into()],
            inverse_bind: vec![inverse],
            alternate_bind: Vec::new(),
            bind_shape: Mat4::IDENTITY,
            pelvis_offset: 0.0,
        }),
        center: Vec3::ZERO,
        size: Vec3::ONE,
    };
    if layered {
        // Many mesh heads place cosmetic/cut layers just above the skin.
        // A transparent built-in texture must not become an opaque shell.
        let mut shell = model.faces[0].clone();
        shell.material = 1;
        for vertex in &mut shell.vertices {
            vertex.position = (Vec3::from_array(vertex.position) + Vec3::from_array(vertex.normal) * 0.003).to_array();
        }
        model.faces.push(shell);
    }
    let data = encode::asset(
        &[None, None, None, Some(model)],
        None,
        &Default::default(),
        encode::Options {
            skin: rigged,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let header = aurora_assets::parse_mesh_header(&data).map_err(|error| error.to_string())?;
    let skin = header
        .skin
        .and_then(|range| aurora_assets::mesh::section_bytes(&data, range))
        .map(aurora_assets::decode_skin)
        .transpose()
        .map_err(|error| error.to_string())?
        .map(Arc::new);
    Ok(MeshAsset {
        data: Arc::new(data),
        header,
        skin,
    })
}

pub fn layered_object(world: &World) -> Option<aurora_net::objects::ObjectUpdate> {
    let mut object = object(world)?;
    let te = Arc::make_mut(object.texture_entry.as_mut()?);
    te.faces[0].color = [1.0; 4];
    te.faces[1].texture = aurora_prim::te::TRANSPARENT_TEXTURE;
    te.faces[1].color = [1.0; 4];
    Some(object)
}

pub fn object(world: &World) -> Option<aurora_net::objects::ObjectUpdate> {
    let avatar = world.objects.get(world.objects.index_of_uuid(&world.agent_id)?)?;
    let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
        item_id: ITEM,
        owner_id: world.agent_id,
        point: 2,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    object.parent_id = avatar.key.local_id;
    object.scale = Vec3::ONE;
    let point = world.avatar_lib.attach_points.get(&2)?;
    let head = world.avatar_lib.rig.names.get("mHead")?;
    let head_position = world.avatar_lib.rig.default_world_inv[*head].inverse().w_axis.truncate();
    object.position = point.rotation.inverse() * (head_position - point.skel_position);
    object.rotation = point.rotation.inverse() * Quat::IDENTITY;
    let sculpt = aurora_prim::SculptParams {
        texture: ASSET,
        sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
    };
    object.volume.sculpt = Some(sculpt);
    object.extra.sculpt = Some(sculpt);
    let mut texture = aurora_prim::TextureEntry::default();
    for face in &mut texture.faces {
        face.texture = crate::scene::avatar::BAKES[0].2;
        face.color = TINT;
    }
    object.texture_entry = Some(Arc::new(texture));
    Some(object)
}
