//! System avatar: base meshes (`*.llm`), skeleton and attachment points,
//! rendered in default pose with server-side baked textures.

use aurora_assets::{LlmMesh, Skeleton};
use aurora_llsd::xml;
use aurora_render::{SkinVertex, Vertex};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
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

/// The bakes of an avatar that "Refresh textures" downloads again
/// (avatar_tex_refresh, llviewermenu.cpp): every slot holding a texture,
/// leaving out what destroy_texture leaves (IMG_DEFAULT and the default
/// textures: the default avatar texture, the invisible bake, the "use baked"
/// placeholders). Firestorm goes through the six system slots; the five of
/// bakes on mesh are taken too, they are bakes like the others.
pub fn refreshable_bakes(bake: impl Fn(usize) -> Option<Uuid>) -> Vec<Uuid> {
    let mut out = Vec::new();
    for (te, _, _) in BAKES {
        let Some(t) = bake(te) else { continue };
        let default = t.is_nil() || t == IMG_DEFAULT_AVATAR || t == IMG_INVISIBLE || bake_slot_for(&t).is_some();
        if !default && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct AttachPoint {
    /// Avatar-local (pelvis-origin) position of the point.
    pub position: Vec3,
    pub rotation: Quat,
    pub hud: bool,
    /// Native avatar_lad.xml visibility, including Bento facial points.
    pub visible_in_first_person: bool,
    /// Skeleton-space (feet at z = 0) rest position, and the rig joint the
    /// point follows when the avatar is animated.
    pub skel_position: Vec3,
    /// Attachment translation in its bone's local frame. The bone scales
    /// this offset; neither it nor linked prim scales resize worn geometry.
    pub local_position: Vec3,
    pub rest_joint: Mat4,
    pub joint: Option<usize>,
    /// Anchor of the wearer's palette holding the posed point (anchors.rs);
    /// None for a HUD point, or past the anchors a palette has.
    pub anchor: Option<u8>,
    /// avatar_lad.xml `group` and `pie_slice`: the body part submenu of
    /// "Attach" and the place in it (LLVOAvatarSelf::buildMenus); 9 is HUD.
    pub group: u8,
    pub pie_slice: u8,
}

impl AttachPoint {
    /// LLXformMatrix::update: inherit the bone's scaled child offset and
    /// world rotation, with unit scale on the attachment joint itself.
    pub(crate) fn frame(self, joint_world: Option<Mat4>) -> (Vec3, Quat) {
        let matrix = joint_world.unwrap_or(self.rest_joint);
        let (_, rotation, _) = matrix.to_scale_rotation_translation();
        (
            matrix.transform_point3(self.local_position),
            (rotation.normalize() * self.rotation).normalize(),
        )
    }
}

/// One base mesh part with its geometry in avatar-local space.
pub struct AvatarPart {
    pub name: &'static str,
    pub bake_te: usize,
    /// Skeleton space (feet at z = 0), default pose.
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub skin: Vec<SkinVertex>,
    /// Normals of the file, as they are before a vertex packs them: the
    /// morphs add to them.
    pub normals: Vec<Vec3>,
    /// Visual parameters that move this part's vertices (morph.rs).
    pub morphs: Vec<super::morph::MorphTarget>,
}

pub struct AvatarLibrary {
    #[allow(dead_code)] // parsed avatar_skeleton.xml, kept for debugging tools
    pub skeleton: Option<Skeleton>,
    pub rig: std::sync::Arc<super::anim::Rig>,
    pub pelvis: Vec3,
    pub attach_points: HashMap<u8, AttachPoint>,
    /// Attachment point of each anchor of a palette, from anchor 1 (anchor
    /// 0 is the wearer's root).
    pub anchor_points: Vec<u8>,
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
    let normals = (0..m.positions.len())
        .map(|i| m.normals.get(i).map_or(Vec3::Z, |n| Vec3::from_array(*n)))
        .collect();
    AvatarPart {
        name,
        bake_te,
        vertices,
        indices,
        skin: Vec::new(),
        normals,
        morphs: Vec::new(),
    }
}

macro_rules! character_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../assets/character/", $name)) as &[u8])),*]
    };
}

/// The avatar definitions the viewer reads (`assets/character`), built into
/// the executable: read from a folder next to it, an installation that lost
/// that folder, or could not read it, started with an empty skeleton and
/// stopped at the first avatar.
const FILES: &[(&str, &[u8])] = character_files![
    "avatar_skeleton.xml",
    "avatar_lad.xml",
    "avatar_head.llm",
    "avatar_upper_body.llm",
    "avatar_lower_body.llm",
    "avatar_hair.llm",
    "avatar_eyelashes.llm",
    "avatar_skirt.llm",
    "avatar_eye.llm",
];

impl AvatarLibrary {
    /// One of the avatar definition files, by its name in `assets/character`.
    pub fn file(name: &str) -> Option<&'static [u8]> {
        FILES.iter().find(|(n, _)| *n == name).map(|(_, data)| *data)
    }

    pub fn load() -> AvatarLibrary {
        let skeleton = Self::file("avatar_skeleton.xml").and_then(|d| Skeleton::parse(d).map_err(|e| log::warn!("skeleton: {e}")).ok());
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
        let lad = Self::file("avatar_lad.xml");
        if let Some(d) = lad
            && let Ok(root) = xml::parse(d)
        {
            let mut stack = vec![&root];
            while let Some(el) = stack.pop() {
                for c in el.elements() {
                    if c.name == "attachment_point" {
                        let id = c.attr("id").and_then(|v| v.parse::<u8>().ok());
                        let joint = c.attr("joint").unwrap_or("mPelvis");
                        let pos = parse_vec3(c.attr("position").unwrap_or("0 0 0"));
                        let rot = parse_vec3(c.attr("rotation").unwrap_or("0 0 0"));
                        // LLVOAvatar::initAttachmentPoints (indra/newview/llvoavatar.cpp,
                        // originally LGPL 2.1) uses LLQuaternion::setQuat: qx * qy * qz.
                        // The skeleton's mayaQ convention composes these in reverse.
                        let rotation = Quat::from_euler(glam::EulerRot::XYZ, rot.x.to_radians(), rot.y.to_radians(), rot.z.to_radians());
                        let hud = c.attr("hud").is_some_and(|v| v == "true");
                        if let Some(id) = id {
                            attach_names.insert(id, c.attr("name").unwrap_or(aurora_i18n::tr!("common-attachment")).to_owned());
                            let jp = joint_pos(joint).unwrap_or(pelvis);
                            joint_names.push((id, joint.to_owned()));
                            attach_points.insert(
                                id,
                                AttachPoint {
                                    position: jp + pos - pelvis,
                                    rotation,
                                    hud,
                                    visible_in_first_person: c.attr("visible_in_first_person") == Some("true"),
                                    skel_position: jp + pos,
                                    local_position: pos,
                                    rest_joint: Mat4::from_translation(jp),
                                    joint: None,
                                    anchor: None,
                                    group: c.attr("group").and_then(|v| v.parse().ok()).unwrap_or(0),
                                    pie_slice: c.attr("pie_slice").and_then(|v| v.parse().ok()).unwrap_or(0),
                                },
                            );
                        }
                    } else {
                        stack.push(c);
                    }
                }
            }
        }

        let mut rig = match &skeleton {
            Some(sk) => super::anim::Rig::new(sk),
            None => super::anim::Rig::new(&Skeleton { joints: Vec::new() }),
        };
        for (id, name) in &joint_names {
            if let Some(ap) = attach_points.get_mut(id) {
                ap.joint = rig.names.get(name).copied();
                if let Some(joint) = ap.joint {
                    ap.rest_joint = rig.default_world_inv[joint].inverse();
                    ap.skel_position = ap.rest_joint.transform_point3(ap.local_position);
                    ap.position = ap.skel_position - pelvis;
                    if let Some(point) = attach_names.get(id).filter(|_| !ap.hud) {
                        let local = Mat4::from_rotation_translation(ap.rotation, ap.local_position);
                        rig.skin_points.entry(point.clone()).or_insert((joint, local));
                    }
                }
            }
        }
        let rig = std::sync::Arc::new(rig);
        // one anchor per body point, in id order, after the root's
        let mut anchor_points: Vec<u8> = attach_points.iter().filter(|(_, ap)| !ap.hud).map(|(id, _)| *id).collect();
        anchor_points.sort_unstable();
        anchor_points.truncate(super::anim::ANCHORS - 1);
        for (i, id) in anchor_points.iter().enumerate() {
            if let Some(ap) = attach_points.get_mut(id) {
                ap.anchor = Some(i as u8 + 1);
            }
        }
        let mut parts = Vec::new();
        let off = Vec3::ZERO;
        let load = |f: &str| -> Option<LlmMesh> {
            Self::file(f).and_then(|d| aurora_assets::parse_llm(d).map_err(|e| log::warn!("{f}: {e}")).ok())
        };
        let morph_params = lad.map(super::morph::mesh_params).unwrap_or_default();
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
                if let Some(params) = morph_params.get(file) {
                    part.morphs = super::morph::targets(&m, params);
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
        let shape_params = match (&skeleton, lad) {
            (Some(sk), Some(d)) => super::shape::ShapeParams::parse(d, sk, &rig),
            _ => Default::default(),
        };
        log::info!(
            "avatar library: {} parts, {} morphs, {} attachment points, skeleton {}",
            parts.len(),
            parts.iter().map(|p| p.morphs.len()).sum::<usize>(),
            attach_points.len(),
            skeleton.is_some()
        );
        AvatarLibrary {
            skeleton,
            rig,
            pelvis,
            attach_points,
            anchor_points,
            attach_names,
            shape_params,
            parts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_person_attachment_flags_include_bento_facial_points() {
        let lib = AvatarLibrary::load();
        let hidden = [2, 11, 12, 13, 14, 15, 16, 17, 47, 48, 49, 50, 51, 52];
        for (&id, point) in &lib.attach_points {
            assert_eq!(point.visible_in_first_person, !hidden.contains(&id), "point {id}");
        }
        assert!(
            lib.attach_points[&47].group != 2,
            "Bento visibility must not be inferred from the old head group"
        );
        assert!((31..=38).all(|id| lib.attach_points[&id].hud && lib.attach_points[&id].visible_in_first_person));
    }

    #[test]
    fn attachment_rotations_match_firestorms_local_axes() {
        let lib = AvatarLibrary::load();
        assert_eq!(lib.attach_points.len(), 55);
        for (&id, point) in &lib.attach_points {
            // Basis vectors from LLQuaternion::setQuat in
            // LLVOAvatar::initAttachmentPoints (llvoavatar.cpp), not mayaQ.
            let expected = match id {
                1 => [Vec3::Y, Vec3::Z, Vec3::X],   // Chest: 0, 90, 90
                9 => [Vec3::Y, -Vec3::Z, -Vec3::X], // Spine: 0, -90, 90
                2 => [Vec3::Y, -Vec3::X, Vec3::Z],  // Skull: 0, 0, 90
                _ => [Vec3::X, Vec3::Y, Vec3::Z],
            };
            for (axis, expected) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().zip(expected) {
                let actual = point.rotation * axis;
                assert!(
                    actual.abs_diff_eq(expected, 1e-6),
                    "point {id}, axis {axis:?}: {actual:?} != {expected:?}"
                );
            }
        }
    }

    /// Issue 586: a head rigged to the "Nose" and "Mouth" attachment points
    /// (LLVOAvatar::getJoint finds them) had its snout follow the pelvis,
    /// stretched to the ground.
    #[test]
    fn skin_joints_named_after_attachment_points_follow_the_point() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let point = |name: &str| {
            let id = lib
                .attach_names
                .iter()
                .find(|(_, n)| n.as_str() == name)
                .map(|(id, _)| *id)
                .unwrap();
            lib.attach_points[&id]
        };
        let nose = point("Nose");
        let head = rig.names["mHead"];
        assert_eq!(nose.joint, Some(head));
        // Bound at the point's rest frame, as an uploader does.
        let rest = nose.rest_joint * Mat4::from_rotation_translation(nose.rotation, nose.local_position);
        let (joint, inverse_bind) = rig.skin_bind("Nose", rest.inverse());
        assert_eq!(joint, head);
        let at_rest = rig.default_world_inv[head].inverse() * inverse_bind;
        assert!(at_rest.abs_diff_eq(Mat4::IDENTITY, 1e-5), "moved at rest: {at_rest:?}");
        // Posed: a vertex on the point goes where the point goes.
        let mut rot = rig.local_rot.clone();
        rot[head] = (rot[head] * Quat::from_rotation_z(0.6) * Quat::from_rotation_y(-0.3)).normalize();
        let world = rig.world_with(&rot, Vec3::new(0.0, 0.0, 0.1), &rig.local_pos, &rig.scale);
        let skinned = (world[head] * inverse_bind).transform_point3(nose.skel_position);
        let expected = nose.frame(Some(world[head])).0;
        assert!(skinned.distance(expected) < 1e-5, "{skinned:?} != {expected:?}");

        // Names stay case-sensitive: "PELVIS" is the collision volume,
        // "Pelvis" the attachment point below the pelvis.
        let volume = rig.names["PELVIS"];
        assert_eq!(rig.skin_bind("PELVIS", Mat4::IDENTITY), (volume, Mat4::IDENTITY));
        let (joint, below) = rig.skin_bind("Pelvis", Mat4::IDENTITY);
        assert_eq!(joint, rig.pelvis);
        assert!(below.w_axis.truncate().abs_diff_eq(point("Pelvis").local_position, 1e-6));
        // Bones and other names are unchanged; HUD points are not joints.
        let ib = Mat4::from_translation(Vec3::new(0.1, 0.2, 0.3));
        assert_eq!(rig.skin_bind("mHead", ib), (head, ib));
        assert_eq!(rig.skin_bind("obsolete_joint", ib), (rig.pelvis, ib));
        assert_eq!(rig.skin_bind("Center", ib), (rig.pelvis, ib));
    }

    #[test]
    fn refresh_takes_the_bakes_not_the_default_textures() {
        let head = Uuid::from_u128(1);
        let arm = Uuid::from_u128(2);
        let got = refreshable_bakes(|te| match te {
            8 => Some(head),
            // the same bake in two slots is refreshed once
            9 => Some(head),
            10 => Some(IMG_DEFAULT_AVATAR),
            11 => Some(IMG_INVISIBLE),
            19 => Some(BAKES[4].2),
            20 => Some(Uuid::nil()),
            40 => Some(arm),
            _ => None,
        });
        assert_eq!(got, vec![head, arm]);
    }

    /// Issue 262: an empty library (no skeleton, hence no joint) stopped the
    /// viewer at the first avatar. Every file `load` asks for is built in.
    #[test]
    fn the_library_is_whole_without_a_folder_next_to_the_executable() {
        let lib = AvatarLibrary::load();
        assert!(lib.skeleton.is_some());
        assert!(lib.rig.pelvis < lib.rig.local_pos.len());
        assert_eq!(lib.rig.names.get("mPelvis"), Some(&lib.rig.pelvis));
        let mut parts: Vec<_> = lib.parts.iter().map(|p| p.name).collect();
        parts.sort_unstable();
        assert_eq!(
            parts,
            ["eye_left", "eye_right", "eyelashes", "hair", "head", "lower", "skirt", "upper"]
        );
        assert!(lib.parts.iter().all(|p| !p.vertices.is_empty() && !p.skin.is_empty()));
        assert!(lib.attach_points.len() >= lib.anchor_points.len() && !lib.anchor_points.is_empty());
        assert!(AvatarLibrary::file("avatar_head_1.llm").is_none());
    }
}
