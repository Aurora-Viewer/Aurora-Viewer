//! System avatar: base meshes (`*.llm`), skeleton and attachment points,
//! rendered in default pose with server-side baked textures.

use aurora_assets::{LlmMesh, Skeleton};
use aurora_llsd::xml;
use aurora_render::{SkinVertex, Vertex};
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const IMG_DEFAULT_AVATAR: Uuid = Uuid::from_u128(0xc228d1cf_4b5d_4ba8_84f4_899a0796aa97);
pub const IMG_INVISIBLE: Uuid = Uuid::from_u128(0x3a367d1c_bef1_6d43_7595_e88c1e3aadb3);

/// Bake slots: (texture-entry index, appearance-service bake name, "use baked" UUID).
pub const BAKES: [(usize, &str, Uuid); 11] = [
    (8, "head", Uuid::from_u128(0x5a9f4a74_30f2_821c_b88d_70499d3e7183)),
    (9, "upper", Uuid::from_u128(0xae2de45c_d252_50b8_5c6e_19f39ce79317)),
    (10, "lower", Uuid::from_u128(0x24daea5f_0539_cfcf_047f_fbc40b2786ba)),
    (11, "eyes", Uuid::from_u128(0x52cc6bb6_2ee5_e632_d3ad_50197b1dcb8a)),
    (19, "skirt", Uuid::from_u128(0x43529ce8_7faa_ad92_165a_bc4078371687)),
    (20, "hair", Uuid::from_u128(0x09aac1fb_6bce_0bee_7d44_caac6dbb6c63)),
    (40, "leftarm", Uuid::from_u128(0xff62763f_d60a_9855_890b_0c96f8f8cd98)),
    (41, "leftleg", Uuid::from_u128(0x8e915e25_31d1_cc95_ae08_d58a47488251)),
    (42, "aux1", Uuid::from_u128(0x9742065b_19b5_297c_858a_29711d539043)),
    (43, "aux2", Uuid::from_u128(0x03642e83_2bd1_4eb9_34b4_4c47ed586d2d)),
    (44, "aux3", Uuid::from_u128(0xedd51b77_fc10_ce7a_4b3d_011dfc349e4f)),
];

/// Map a "use baked texture" UUID to its texture-entry slot.
pub fn bake_slot_for(id: &Uuid) -> Option<(usize, &'static str)> {
    BAKES.iter().find(|(_, _, u)| u == id).map(|(i, n, _)| (*i, *n))
}

pub fn bake_name(te_index: usize) -> Option<&'static str> {
    BAKES.iter().find(|(i, _, _)| *i == te_index).map(|(_, n, _)| *n)
}

#[derive(Debug, Clone, Copy)]
pub struct AttachPoint {
    /// Avatar-local (pelvis-origin) position of the point.
    pub position: Vec3,
    pub rotation: Quat,
    pub hud: bool,
    /// Skeleton-space (feet at z = 0) rest position, and the rig joint the
    /// point follows when the avatar is animated.
    pub skel_position: Vec3,
    pub joint: Option<usize>,
}

/// One base mesh part with its geometry in avatar-local space.
pub struct AvatarPart {
    pub name: &'static str,
    pub bake_te: usize,
    /// Skeleton space (feet at z = 0), default pose.
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub skin: Vec<SkinVertex>,
}

pub struct AvatarLibrary {
    #[allow(dead_code)] // parsed avatar_skeleton.xml, kept for debugging tools
    pub skeleton: Option<Skeleton>,
    pub rig: std::sync::Arc<super::anim::Rig>,
    pub pelvis: Vec3,
    pub attach_points: HashMap<u8, AttachPoint>,
    pub attach_names: HashMap<u8, String>,
    /// Visual parameters (avatar shape) of avatar_lad.xml.
    pub shape_params: super::shape::ShapeParams,
    pub parts: Vec<AvatarPart>,
}

fn parse_vec3(s: &str) -> Vec3 {
    let mut it = s.split_whitespace().map(|v| v.parse::<f32>().unwrap_or(0.0));
    Vec3::new(it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
}

/// LLAvatarJointMesh::setupJoint render list: palette indices referenced by
/// the integer part of each LLM vertex weight.
fn render_list(rig: &super::anim::Rig, sk: &Skeleton, joint_names: &[String]) -> Vec<u8> {
    let skin: std::collections::HashSet<usize> = joint_names.iter().filter_map(|n| rig.names.get(n).copied()).collect();
    let root_slot = rig.len() as u8; // identity slot stands in for LL's mRoot
    let base_ancestor = |j: usize| -> Option<usize> {
        let mut p = rig.parent[j];
        while let Some(pi) = p {
            if sk.joints.get(pi).is_some_and(|x| x.support == "base") {
                return Some(pi);
            }
            p = rig.parent[pi];
        }
        None
    };
    let mut list: Vec<u8> = Vec::new();
    for &j in &rig.order {
        if !skin.contains(&j) || sk.joints.get(j).is_some_and(|x| x.is_collision_volume) {
            continue;
        }
        let anc = base_ancestor(j).map(|a| a as u8).unwrap_or(root_slot);
        if list.last() == Some(&anc) {
            list.push(j as u8);
        } else {
            list.push(anc);
            list.push(j as u8);
        }
    }
    list
}

fn llm_skin(m: &LlmMesh, list: &[u8], fixed_joint: Option<u8>) -> Vec<SkinVertex> {
    (0..m.positions.len())
        .map(|i| {
            if let Some(j) = fixed_joint {
                return SkinVertex {
                    joints: [j, 0, 0, 0],
                    weights: [255, 0, 0, 0],
                };
            }
            let w = m.weights.get(i).copied().unwrap_or(0.0).max(0.0);
            let idx = w.floor() as usize;
            let frac = (w - idx as f32).clamp(0.0, 1.0);
            let last = list.len().saturating_sub(1);
            let j0 = list.get(idx.min(last)).copied().unwrap_or(0);
            let j1 = list.get((idx + 1).min(last)).copied().unwrap_or(j0);
            SkinVertex {
                joints: [j0, j1, 0, 0],
                weights: [((1.0 - frac) * 255.0).round() as u8, (frac * 255.0).round() as u8, 0, 0],
            }
        })
        .collect()
}

fn llm_to_part(name: &'static str, bake_te: usize, m: &LlmMesh, offset: Vec3, flip_v: bool) -> AvatarPart {
    let vertices = m
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pos = Vec3::from_array(*p) + offset;
            let n = m.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
            let mut uv = m.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            if flip_v {
                uv[1] = 1.0 - uv[1];
            }
            Vertex::new(pos.to_array(), n, uv)
        })
        .collect();
    let indices = m.faces.iter().flat_map(|f| f.iter().copied()).collect();
    AvatarPart {
        name,
        bake_te,
        vertices,
        indices,
        skin: Vec::new(),
    }
}

impl AvatarLibrary {
    pub fn data_dir() -> PathBuf {
        // Next to the executable first, then the source tree (dev builds).
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        let candidates = [
            exe_dir.join("assets").join("character"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("character"),
        ];
        for c in &candidates {
            if c.join("avatar_skeleton.xml").exists() {
                return c.clone();
            }
        }
        candidates[1].clone()
    }

    pub fn load() -> AvatarLibrary {
        let dir = Self::data_dir();
        let skeleton = std::fs::read(dir.join("avatar_skeleton.xml"))
            .ok()
            .and_then(|d| Skeleton::parse(&d).map_err(|e| log::warn!("skeleton: {e}")).ok());
        let joint_pos = |name: &str| -> Option<Vec3> {
            let sk = skeleton.as_ref()?;
            let i = sk.find(name)?;
            Some(sk.joints[i].world_position())
        };
        let pelvis = joint_pos("mPelvis").unwrap_or(Vec3::new(0.0, 0.0, 1.067));

        // Attachment points from avatar_lad.xml
        let mut attach_points = HashMap::new();
        let mut attach_names = HashMap::new();
        let mut joint_names: Vec<(u8, String)> = Vec::new();
        if let Ok(d) = std::fs::read(dir.join("avatar_lad.xml"))
            && let Ok(root) = xml::parse(&d)
        {
            let mut stack = vec![&root];
            while let Some(el) = stack.pop() {
                for c in el.elements() {
                    if c.name == "attachment_point" {
                        let id = c.attr("id").and_then(|v| v.parse::<u8>().ok());
                        let joint = c.attr("joint").unwrap_or("mPelvis");
                        let pos = parse_vec3(c.attr("position").unwrap_or("0 0 0"));
                        let rot = parse_vec3(c.attr("rotation").unwrap_or("0 0 0"));
                        let hud = c.attr("hud").is_some_and(|v| v == "true");
                        if let Some(id) = id {
                            attach_names.insert(id, c.attr("name").unwrap_or("Attachment").to_owned());
                            let jp = joint_pos(joint).unwrap_or(pelvis);
                            joint_names.push((id, joint.to_owned()));
                            attach_points.insert(
                                id,
                                AttachPoint {
                                    position: jp + pos - pelvis,
                                    rotation: aurora_assets::skeleton::maya_q_xyz(rot),
                                    hud,
                                    skel_position: jp + pos,
                                    joint: None,
                                },
                            );
                        }
                    } else {
                        stack.push(c);
                    }
                }
            }
        }

        let rig = std::sync::Arc::new(match &skeleton {
            Some(sk) => super::anim::Rig::new(sk),
            None => super::anim::Rig::new(&Skeleton { joints: Vec::new() }),
        });
        for (id, name) in &joint_names {
            if let Some(ap) = attach_points.get_mut(id) {
                ap.joint = rig.names.get(name).copied();
            }
        }
        let mut parts = Vec::new();
        let off = Vec3::ZERO;
        let load = |f: &str| -> Option<LlmMesh> {
            std::fs::read(dir.join(f))
                .ok()
                .and_then(|d| aurora_assets::parse_llm(&d).map_err(|e| log::warn!("{f}: {e}")).ok())
        };
        // UVs stay in GL convention: the object shader flips V.
        for (file, name, te) in [
            ("avatar_head.llm", "head", 8usize),
            ("avatar_upper_body.llm", "upper", 9),
            ("avatar_lower_body.llm", "lower", 10),
            ("avatar_hair.llm", "hair", 20),
            ("avatar_eyelashes.llm", "eyelashes", 8),
            ("avatar_skirt.llm", "skirt", 19),
        ] {
            if let Some(m) = load(file) {
                let mut part = llm_to_part(name, te, &m, off, false);
                if let Some(sk) = &skeleton {
                    let list = render_list(&rig, sk, &m.joint_names);
                    part.skin = llm_skin(&m, &list, None);
                }
                parts.push(part);
            }
        }
        if let Some(eye) = load("avatar_eye.llm") {
            for (j, name) in [("mEyeLeft", "eye_left"), ("mEyeRight", "eye_right")] {
                if let Some(p) = joint_pos(j) {
                    let mut part = llm_to_part(name, 11, &eye, p, false);
                    let ji = rig.names.get(j).map(|&i| i as u8);
                    part.skin = llm_skin(&eye, &[], ji);
                    parts.push(part);
                }
            }
        }
        let shape_params = match (&skeleton, std::fs::read(dir.join("avatar_lad.xml"))) {
            (Some(sk), Ok(d)) => super::shape::ShapeParams::parse(&d, sk, &rig),
            _ => Default::default(),
        };
        log::info!(
            "avatar library: {} parts, {} attachment points, skeleton {}",
            parts.len(),
            attach_points.len(),
            skeleton.is_some()
        );
        AvatarLibrary {
            skeleton,
            rig,
            pelvis,
            attach_points,
            attach_names,
            shape_params,
            parts,
        }
    }
}
