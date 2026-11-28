//! Where the face's dots sit under a mesh head (#5022, Aurora's own): on the
//! worn head's own surface. Each dot moves along its own way out from the
//! head's centre to the outermost skin of the worn face meshes on that line
//! (a lip's front, an eyelid's curve), and is kept in its bone's frame, so
//! that it follows the bone as the mesh does. The system head's surface
//! (`handles::lift_to_face`) only fits a mesh head shaped like it: a fuller
//! face (a Bento human head's lips, a furry's muzzle) left the dots inside
//! it. Reading the surface along the dot's line, not per bone, places it
//! whichever bones the skin there follows: a Bento head blends its lips
//! across several bones, and one bone rarely holds them alone.

use std::collections::HashMap;

use aurora_assets::{MeshFace, SkinInfo};
use glam::{Mat4, Vec3};

use super::handles::Anchor;
use crate::scene::anim::Rig;

/// How far from a dot's line a vertex may be to be the surface on it (m):
/// the surface right in front of the dot, not a feature beside it.
const LINE_REACH: f32 = 0.004;

/// How far behind a dot (m) the surface may be, for a dot left out in front
/// of a slimmer face, and how far ahead: the lips lie one to three
/// centimetres in front of their bones, a muzzle further.
const BEHIND: f32 = 0.03;
const AHEAD: f32 = 0.08;

/// The share of a mesh's vertices that face bones (the joints under the
/// head bone) must lead for its surface to count: a head, its eyes, its
/// teeth; not a hair or a hat with a few strands weighted to the brows.
const FACE_SHARE: f32 = 0.25;

/// A dot's place in the lookup: its bone and its point in the bone's frame.
pub type Key = (usize, [u32; 3]);

pub fn key(anchor: &Anchor) -> Key {
    (anchor.joint, anchor.local.to_array().map(f32::to_bits))
}

/// The face bones a worn mesh is weighted to (the joints under the head
/// bone, not the skull itself): only such meshes are read.
pub fn face_joints(rig: &Rig) -> Vec<bool> {
    let Some(head) = rig.names.get("mHead").copied() else {
        return vec![false; rig.len()];
    };
    (0..rig.len())
        .map(|j| {
            let mut at = rig.parent[j];
            while let Some(k) = at {
                if k == head {
                    return true;
                }
                at = rig.parent[k];
            }
            false
        })
        .collect()
}

/// The worn face meshes' vertices skinned on `joints` (the skeleton the
/// layout's dots are placed on), for the meshes mostly led by face bones.
fn face_vertices(rig: &Rig, joints: &[Mat4], meshes: &[(&SkinInfo, &[MeshFace])]) -> Vec<Vec3> {
    let is_face = face_joints(rig);
    let mut found = Vec::new();
    for (skin, faces) in meshes {
        // each skin joint: the rig joint, and mesh space to its place
        let binds: Vec<(usize, Mat4)> = skin
            .joint_names
            .iter()
            .zip(&skin.inverse_bind)
            .filter_map(|(name, inverse)| {
                let (joint, inverse) = rig.skin_bind(name, *inverse);
                Some((joint, *joints.get(joint)? * inverse * skin.bind_shape))
            })
            .collect();
        if binds.len() != skin.joint_names.len() {
            continue;
        }
        let (mut vertices, mut led) = (Vec::new(), 0usize);
        for face in faces.iter() {
            if face.joints.len() != face.positions.len() || face.weights.len() != face.positions.len() {
                continue;
            }
            for ((position, bones), weights) in face.positions.iter().zip(&face.joints).zip(&face.weights) {
                let p = Vec3::from_array(*position);
                let (mut at, mut total, mut lead) = (Vec3::ZERO, 0.0, (0.0, None));
                for (k, weight) in bones.iter().zip(weights) {
                    let Some(&(joint, bind)) = binds.get(usize::from(*k)).filter(|_| *weight > 0.0) else {
                        continue;
                    };
                    at += bind.transform_point3(p) * *weight;
                    total += *weight;
                    if *weight > lead.0 {
                        lead = (*weight, Some(joint));
                    }
                }
                if total <= 1e-4 || !at.is_finite() {
                    continue;
                }
                led += usize::from(lead.1.is_some_and(|j| is_face.get(j).copied().unwrap_or(false)));
                vertices.push(at / total);
            }
        }
        if !vertices.is_empty() && led as f32 >= vertices.len() as f32 * FACE_SHARE {
            found.extend(vertices);
        }
    }
    found
}

/// Each dot of `anchors` moved onto the worn face's surface: its new point
/// in its bone's frame, by its place (`key`). `joints` is the skeleton at
/// rest the anchors are placed on (`handles::base_rest_joints`). A dot with
/// no surface on its line keeps its place (absent).
pub fn surface_anchors(rig: &Rig, joints: &[Mat4], meshes: &[(&SkinInfo, &[MeshFace])], anchors: &[Anchor]) -> HashMap<Key, Vec3> {
    let at = |name: &str| rig.names.get(name).and_then(|&j| joints.get(j)).map(|m| m.w_axis.truncate());
    // the head's centre: its collision volume's, else a little above the
    // head bone
    let Some(centre) = at("HEAD").or_else(|| Some(at("mHead")? + Vec3::Z * 0.08)) else {
        return HashMap::new();
    };
    let vertices = face_vertices(rig, joints, meshes);
    let mut lifted = HashMap::new();
    if vertices.is_empty() {
        return lifted;
    }
    for anchor in anchors.iter().filter(|a| a.vertex.is_none()) {
        let (Some(dot), Some(bone)) = (anchor.at(joints), joints.get(anchor.joint)) else {
            continue;
        };
        let Some(out) = (dot - centre).try_normalize() else { continue };
        // the outermost vertex on the dot's line
        let outermost = vertices
            .iter()
            .filter_map(|v| {
                let d = *v - dot;
                let t = d.dot(out);
                ((-BEHIND..=AHEAD).contains(&t) && d.length_squared() - t * t <= LINE_REACH * LINE_REACH).then_some(t)
            })
            .reduce(f32::max);
        if let Some(t) = outermost {
            let local = bone.inverse().transform_point3(dot + out * t);
            if local.is_finite() {
                lifted.insert(key(anchor), local);
            }
        }
    }
    lifted
}

#[cfg(test)]
#[path = "face_surface_tests.rs"]
mod tests;
