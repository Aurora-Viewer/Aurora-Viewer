//! Articulated posing uses only the numerical solver, never binding state.
use super::*;
use crate::scene::ik::LocalPose;

fn solve(rig: &Rig, pose: &mut LocalPose, scales: &[Vec3], bone: usize, position: Vec3, rotation: Option<Quat>) -> bool {
    ik::apply_goal(
        rig,
        pose,
        Vec3::ZERO,
        scales,
        &Goal {
            follow: None,
            weight: 1.0,
            bone,
            root: ik::default_root(rig, bone),
            position,
            contact: Vec3::ZERO,
            mode: if rig.parent[bone].is_none() { Mode::Direct } else { Mode::Ik },
            pole: None,
            rotation,
        },
    )
}
impl SubjectPose {
    pub(super) fn resolved(&self, rig: &Rig, model: Mat4) -> LocalPose {
        let mut pose = LocalPose {
            rotations: self.rotations.clone(),
            positions: self.positions.clone(),
        };
        if !self.locks.is_empty() {
            let inverse = model.inverse();
            // Deterministic, bounded relaxation of multiple pins. An unreachable
            // pin yields at the chain's reach instead of translating a child.
            for _ in 0..3 {
                for &bone in &rig.order {
                    if let Some(target) = self.locks.get(&bone) {
                        let (_, q, p) = (inverse * *target).to_scale_rotation_translation();
                        solve(rig, &mut pose, &self.scales, bone, p, Some(q));
                    }
                }
            }
        }
        pose
    }
    pub(super) fn move_joint(&mut self, rig: &Rig, bone: usize, position: Vec3) -> bool {
        let mut pose = LocalPose {
            rotations: self.rotations.clone(),
            positions: self.positions.clone(),
        };
        if !solve(rig, &mut pose, &self.scales, bone, position, None) {
            return false;
        }
        self.rotations = pose.rotations;
        self.positions = pose.positions;
        true
    }
    /// Retarget imported translations without changing the subject's shaped
    /// parent offsets. Only an unparented root may translate freely.
    pub(super) fn keep_offsets(&mut self, rig: &Rig, reference: &Self) {
        let requested = rig.world_with(&self.rotations, Vec3::ZERO, &self.positions, &self.scales);
        let translated: Vec<_> = rig
            .order
            .iter()
            .copied()
            .filter(|b| rig.parent[*b].is_some() && self.positions[*b] != reference.positions[*b])
            .collect();
        for &bone in &translated {
            self.positions[bone] = reference.positions[bone];
        }
        for bone in translated {
            self.move_joint(rig, bone, requested[bone].w_axis.truncate());
        }
    }
}
