//! The shape handles' areas drawn on the avatar's own meshes (#5022, Aurora's
//! own): each joint of the skeleton gets the area its skin belongs to, as a
//! part class of the renderer's part pick and tint (the poser's part
//! overlay: part_pick.rs, `fs_part_mask` and `fs_part_pick` in object.wgsl).
//! The renderer weighs each pixel's skinning weights by these classes, so a
//! mesh body, a Bento head, a furry or a feral avatar lights and picks its
//! areas where its own surface follows the bones, whatever its proportions;
//! glows laid over the collision volumes missed what a creator moved away
//! from them (a quadruped's torso, a long snout).
//!
//! The system head is the exception: its face follows the head bone alone
//! (its features are morphs), so its areas keep the glows over the dots.

use std::collections::HashMap;

use glam::Vec3;

use super::handles::{self, Anchor, Layout, Part};
use crate::scene::anim::Rig;

/// The classes the shader tells apart (`poser_pick_class`: 1 to 6; 0 is
/// skin that belongs to no area, and an unrigged accessory).
const CLASSES: usize = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct MeshAreas {
    /// Four bits per palette joint (`FrameParams::part_classes`).
    pub classes: [u32; 32],
    /// The area of each class (index 0 unused).
    areas: [Option<usize>; CLASSES + 1],
    /// Each palette joint's class (the identity slot past the skeleton
    /// included), for the tint's masks.
    of_joint: Vec<u32>,
}

impl MeshAreas {
    /// The classes of the areas `shown` (those with dots), for the part of
    /// the avatar `layout` is: the body's four areas, or the face's groups
    /// under a mesh head. Skin of an area with no dot shown is no class:
    /// a click there picks nothing.
    pub fn new(rig: &Rig, layout: &Layout, shown: &[usize]) -> Self {
        let mut listed: Vec<usize> = shown.to_vec();
        listed.sort_unstable();
        listed.dedup();
        let mut areas = [None; CLASSES + 1];
        for (class, area) in listed.iter().take(CLASSES).enumerate() {
            areas[class + 1] = Some(*area);
        }
        let class_of = |area: Option<usize>| -> u32 { area.and_then(|a| areas.iter().position(|c| *c == Some(a))).map_or(0, |c| c as u32) };
        let joint_areas = match layout.part {
            Part::Body => (0..rig.len()).map(|j| Some(body_area(rig, j))).collect::<Vec<_>>(),
            Part::Head => face_areas(rig, layout),
        };
        let mut of_joint: Vec<u32> = joint_areas.into_iter().map(class_of).collect();
        // the identity slot (the system body's root vertices) is the torso's
        of_joint.push(match layout.part {
            Part::Body => class_of(Some(handles::AREA_TORSO)),
            Part::Head => 0,
        });
        let mut classes = [0u32; 32];
        for (j, class) in of_joint.iter().enumerate().take(256) {
            classes[j >> 3] |= class << ((j & 7) * 4);
        }
        Self { classes, areas, of_joint }
    }

    /// The area of a class the renderer's pick answered.
    pub fn area(&self, class: u32) -> Option<usize> {
        self.areas.get(class as usize).copied().flatten()
    }

    /// The tint's joints for `area` (`FrameParams::part_joints`): bit j for
    /// each palette joint of its skin.
    pub fn mask(&self, area: usize) -> [u32; 8] {
        let mut mask = [0u32; 8];
        let Some(class) = self.areas.iter().position(|a| *a == Some(area)) else {
            return mask;
        };
        for (j, c) in self.of_joint.iter().enumerate().take(256) {
            if *c as usize == class {
                mask[j >> 5] |= 1 << (j & 31);
            }
        }
        mask
    }
}

/// A joint's body area, as the dots' (`handles::area_of`).
fn body_area(rig: &Rig, joint: usize) -> usize {
    handles::area_of(rig, Part::Body, Anchor::on(joint, Vec3::ZERO), 0)
}

/// The face area of each joint under a mesh head: the area of the dots on
/// that joint (most of them, when several groups move it), else of the
/// nearest joint with dots, at rest; the skull itself is the Head group's.
/// Joints off the head have none.
fn face_areas(rig: &Rig, layout: &Layout) -> Vec<Option<usize>> {
    let Some(head) = rig.names.get("mHead").copied() else {
        return vec![None; rig.len()];
    };
    let mut votes: HashMap<usize, HashMap<usize, usize>> = HashMap::new();
    for handle in &layout.handles {
        for anchor in std::iter::once(handle.anchor).chain(handle.mirror) {
            *votes.entry(anchor.joint).or_default().entry(handle.area).or_default() += 1;
        }
    }
    // the eyes are the Eyes tab's whatever moves them, and the brows the
    // forehead's: Outer Eye Corner moves the outer brow bone, which lit the
    // brows as the eyes and left the lids to the nearest other dots
    let fixed = |j: usize| {
        let name = rig.canonical_names.get(j).map_or("", String::as_str);
        if name.contains("Eyebrow") {
            Some(HEAD_GROUP)
        } else if ["mEye", "mFaceEyeAlt", "mFaceEyeLid", "mFaceEyecorner"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            Some(EYES_GROUP)
        } else {
            None
        }
    };
    let voted: HashMap<usize, usize> = votes
        .into_iter()
        .filter_map(|(joint, counts)| {
            // the most dots, the lower area on a tie (the same each time)
            let area = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?.0;
            Some((joint, fixed(joint).unwrap_or(area)))
        })
        .collect();
    let rest = |j: usize| rig.default_world_inv[j].inverse().w_axis.truncate();
    let under_head = |j: usize| {
        let mut at = Some(j);
        while let Some(k) = at {
            if k == head {
                return true;
            }
            at = rig.parent[k];
        }
        false
    };
    (0..rig.len())
        .map(|j| {
            if !under_head(j) {
                return None;
            }
            if j == head {
                return Some(HEAD_GROUP);
            }
            fixed(j).or_else(|| voted.get(&j).copied()).or_else(|| {
                voted
                    .iter()
                    .min_by(|a, b| {
                        rest(*a.0)
                            .distance_squared(rest(j))
                            .total_cmp(&rest(*b.0).distance_squared(rest(j)))
                            .then(a.0.cmp(b.0))
                    })
                    .map(|(_, area)| *area)
            })
        })
        .collect()
}

/// The shape editor's Head group (`catalog::Param::group`): the skull.
const HEAD_GROUP: usize = 1;
/// Its Eyes group.
const EYES_GROUP: usize = 2;

#[cfg(test)]
#[path = "mesh_areas_tests.rs"]
mod tests;
