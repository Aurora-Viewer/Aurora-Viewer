//! The animated box of an animated object (animesh): the space its rigged
//! meshes fill in their current pose, which their detail level is judged
//! by. Port of LLVOAvatar::calculateSpatialExtents for a control avatar
//! (the pelvis, 0.25 m around it, then the rigged extents of every joint
//! placed by the joint's posed matrix) and of the rigging table it reads
//! (LLVOVolume::updateRiggingInfo, LLSkinningUtil::updateRiggingInfo),
//! indra/newview/llvoavatar.cpp, llvovolume.cpp, llskinningutil.cpp,
//! originally LGPL 2.1.
//!
//! LLVOVolume::calcLOD gives a volume of an animated object half the
//! diagonal of that box as its radius and the distance to the box's centre
//! (where LLVOAvatar::updateSpatialExtents puts the control avatar's
//! drawable): the size of the whole figure, not of the root prim, which is
//! often a small invisible box.

use super::animesh::JointBounds;
use super::{GeomKey, GeomState, Scene, anchors, anim};
use crate::world::World;
use glam::{Mat4, Vec3};
use std::hash::{Hash, Hasher};
use std::time::Instant;

/// Padding around the pelvis the box starts from (calculateSpatialExtents).
const PELVIS_PADDING: f32 = 0.25;

/// A box wider than this comes from a palette not posed yet, not a figure.
const MAX_DIAGONAL: f32 = 1000.0;

impl Scene {
    /// The animated box of every animated object (`anim_boxes`), from this
    /// frame's palettes. The rigged extents are gathered again only when the
    /// object's rigged meshes or their geometry change; placing them costs
    /// eight points per rigged joint.
    pub(super) fn update_anim_boxes(&mut self, world: &World, now: Instant) {
        let skeletons = &self.skeletons;
        self.anim_boxes.retain(|id, _| skeletons.contains_key(id));
        self.rig_extents.retain(|id, _| skeletons.contains_key(id));
        let pelvis = self.avatar_lib.rig.pelvis;
        for (owner, skeleton) in &self.skeletons {
            let Some(owner_idx) = world.objects.index_of_uuid(owner) else {
                continue;
            };
            let Some(o) = world.objects.get(owner_idx) else { continue };
            if o.is_avatar() || !self.posed_at.contains_key(owner) {
                continue;
            }
            let Some(&slot) = self.palette_slots.get(owner) else { continue };
            let Some((pos, rot, false)) = Self::object_transform_in(world, owner_idx, now, 0, None) else {
                continue;
            };
            // the rigged meshes drawn now, with their geometry
            let rigged = skeleton.members.iter().filter_map(|&i| match self.gpu.get(i)?.geom? {
                key @ GeomKey::Mesh { id, .. } if self.meshes.meta(&id).is_some_and(|m| m.skin.is_some()) => {
                    match &self.geoms.get(&key)?.state {
                        GeomState::Ready(geom) => Some((i, key, id, geom, self.skin_binds.base_of(&id)?)),
                        _ => None,
                    }
                }
                _ => None,
            });
            let mut signature = std::collections::hash_map::DefaultHasher::new();
            for (i, key, _, _, binds) in rigged.clone() {
                (i, key, binds).hash(&mut signature);
            }
            let extents = self.rig_extents.entry(*owner).or_default();
            if extents.begin(signature.finish()) {
                for (_, _, _, geom, binds) in rigged {
                    extents.add(&geom.joint_bounds, |j| {
                        let b = self.skin_binds.list().get(binds as usize + j as usize)?;
                        Some((b.joint[0], Mat4::from_cols_array_2d(&b.inverse_bind)))
                    });
                }
            }
            if extents.is_empty() {
                self.anim_boxes.remove(owner);
                continue;
            }
            let model = anchors::skeleton_model(world, &self.meshes, owner_idx, pos, rot, 1.0);
            let palette = |j: u32| {
                let p = self.palettes.get(slot as usize * anim::PALETTE_JOINTS + j as usize)?;
                Some(model * Mat4::from_cols_array_2d(p))
            };
            let Some(pelvis) = palette(pelvis as u32).map(|m| m.transform_point3(Vec3::ZERO)) else {
                continue;
            };
            let b = extents.posed_box(pelvis, palette);
            if b.min.is_finite() && b.max.is_finite() && (b.max - b.min).length() < MAX_DIAGONAL {
                self.anim_boxes.insert(*owner, b);
            } else {
                self.anim_boxes.remove(owner);
            }
        }
    }
}

/// The rigged extents of a skeleton's meshes per joint, in the joint's own
/// space (what its palette matrix places): the union of the joint boxes of
/// every rigged mesh of the object, as LLJointRiggingInfoTab.
#[derive(Default)]
pub struct RigExtents {
    signature: u64,
    /// (palette joint, min, max)
    joints: Vec<(u32, Vec3, Vec3)>,
}

impl RigExtents {
    /// Starts again from nothing when `signature` (the members, their
    /// geometry and their skin) changed; false when the extents are still
    /// those of `signature`.
    pub fn begin(&mut self, signature: u64) -> bool {
        if self.signature == signature && !self.joints.is_empty() {
            return false;
        }
        self.signature = signature;
        self.joints.clear();
        true
    }

    /// Adds the joint boxes of one rigged mesh (bind space, per skin joint);
    /// `bind` gives the palette joint and inverse bind matrix of a skin
    /// joint.
    pub fn add(&mut self, bounds: &[JointBounds], bind: impl Fn(u8) -> Option<(u32, Mat4)>) {
        for b in bounds {
            let Some((joint, inverse_bind)) = bind(b.joint) else { continue };
            let Some((min, max)) = box_corners(b.min, b.max)
                .map(|p| inverse_bind.transform_point3(p))
                .try_fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(min, max), p| {
                    p.is_finite().then(|| (min.min(p), max.max(p)))
                })
            else {
                continue;
            };
            match self.joints.iter_mut().find(|j| j.0 == joint) {
                Some(j) => {
                    j.1 = j.1.min(min);
                    j.2 = j.2.max(max);
                }
                None => self.joints.push((joint, min, max)),
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    /// The box in the current pose: from `pelvis` (posed, world), 0.25 m
    /// around it, stretched by every joint's extents placed by `matrix`
    /// (joint space to world).
    pub fn posed_box(&self, pelvis: Vec3, matrix: impl Fn(u32) -> Option<Mat4>) -> AnimBox {
        let mut min = pelvis - Vec3::splat(PELVIS_PADDING);
        let mut max = pelvis + Vec3::splat(PELVIS_PADDING);
        for &(joint, lo, hi) in &self.joints {
            let Some(m) = matrix(joint) else { continue };
            for p in box_corners(lo, hi).map(|p| m.transform_point3(p)) {
                if p.is_finite() {
                    min = min.min(p);
                    max = max.max(p);
                }
            }
        }
        AnimBox { min, max }
    }
}

/// An animated object's box in the world, this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl AnimBox {
    /// (radius, distance) LLVOVolume::calcLOD gives the volumes of an
    /// animated object: half the box's diagonal, and the distance from
    /// `eye` to the box's centre.
    pub fn lod_metrics(&self, eye: Vec3) -> (f32, f32) {
        ((self.max - self.min).length() * 0.5, ((self.min + self.max) * 0.5).distance(eye))
    }
}

fn box_corners(min: Vec3, max: Vec3) -> impl Iterator<Item = Vec3> {
    (0..8).map(move |i| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(joint: u8, min: [f32; 3], max: [f32; 3]) -> JointBounds {
        JointBounds {
            joint,
            min: Vec3::from(min),
            max: Vec3::from(max),
        }
    }

    /// A tall figure on a small root: the box spans the posed meshes, the
    /// pelvis padding included, whatever the root prim's size.
    #[test]
    fn the_box_spans_every_rigged_mesh_in_its_pose() {
        let mut rig = RigExtents::default();
        assert!(rig.begin(1));
        // two meshes on the same joint (5) and one on another (9)
        rig.add(&[bounds(0, [-0.5, -0.5, 0.0], [0.5, 0.5, 2.0])], |j| {
            (j == 0).then_some((5, Mat4::IDENTITY))
        });
        rig.add(&[bounds(3, [0.0, 0.0, 0.0], [0.2, 0.2, 0.2])], |_| {
            Some((5, Mat4::from_translation(Vec3::Z)))
        });
        rig.add(&[bounds(1, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0])], |j| {
            (j == 1).then_some((9, Mat4::IDENTITY))
        });
        assert!(!rig.begin(1));
        let lift = Vec3::new(0.0, 0.0, 1.0);
        let b = rig.posed_box(Vec3::ZERO, |j| match j {
            5 => Some(Mat4::from_translation(lift)),
            9 => Some(Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0))),
            _ => None,
        });
        assert_eq!(b.min, Vec3::new(-0.5, -0.5, -0.25));
        assert_eq!(b.max, Vec3::new(3.0, 1.0, 3.0));
        let (radius, distance) = b.lod_metrics(Vec3::new(1.25, 0.25, 1.375));
        assert!((radius - (b.max - b.min).length() * 0.5).abs() < 1e-6);
        assert!(distance < 1e-6);
        // the inverse bind brings a mesh into its joint's space
        assert!(rig.begin(2));
        rig.add(&[bounds(0, [0.0; 3], [1.0; 3])], |_| Some((5, Mat4::from_scale(Vec3::splat(2.0)))));
        let b = rig.posed_box(Vec3::ZERO, |_| Some(Mat4::IDENTITY));
        assert_eq!(b.max, Vec3::splat(2.0));
    }

    /// Without rigged extents the box is the padded pelvis, as LL's.
    #[test]
    fn an_empty_rig_is_the_padded_pelvis() {
        let rig = RigExtents::default();
        assert!(rig.is_empty());
        let b = rig.posed_box(Vec3::new(1.0, 2.0, 3.0), |_| None);
        assert_eq!(b.max - b.min, Vec3::splat(0.5));
    }
}
