//! Manual posing writes local joint channels, not persistent world-space pins.
//! Explicit limb/target constraints remain a separate solver layer.
use super::*;
use std::collections::HashSet;

#[derive(Clone, Default)]
pub(super) struct State {
    owners: HashSet<usize>,
    edits: Vec<(usize, ik::JointEdit)>,
}
impl State {
    pub fn owns(&self, bone: usize) -> bool {
        self.owners.contains(&bone)
    }
    pub fn remove(&mut self, owner: usize) {
        self.owners.remove(&owner);
        self.edits.retain(|(bone, _)| *bone != owner);
    }
    pub fn bones(&self, owner: usize) -> Vec<usize> {
        self.edits
            .iter()
            .filter(|(b, _)| *b == owner)
            .map(|(_, e)| e.bone)
            .chain(std::iter::once(owner))
            .collect()
    }
    pub fn baked(&mut self, bones: &[usize]) -> Vec<usize> {
        self.edits.retain(|(_, e)| !bones.contains(&e.bone));
        let complete: Vec<_> = self
            .owners
            .iter()
            .copied()
            .filter(|owner| bones.contains(owner) || !self.edits.iter().any(|(o, _)| o == owner))
            .collect();
        for owner in &complete {
            self.remove(*owner);
        }
        complete
    }
    pub fn request(&self, goals: &[Goal], disabled: &[usize]) -> Vec<ik::JointEdit> {
        self.edits
            .iter()
            .filter_map(|(owner, value)| {
                let goal = goals.iter().find(|g| g.bone == *owner && !disabled.contains(owner))?;
                Some(ik::JointEdit {
                    weight: goal.weight,
                    ..value.clone()
                })
            })
            .collect()
    }
    fn record(&mut self, owner: usize, before: &ik::LocalPose, after: &ik::LocalPose) {
        self.owners.insert(owner);
        for bone in 0..before.rotations.len() {
            let rotation = (before.rotations[bone].dot(after.rotations[bone]).abs() < 0.9999999).then_some(after.rotations[bone]);
            let previous = self
                .edits
                .iter()
                .filter(|(_, e)| e.bone == bone)
                .filter_map(|(_, e)| e.translation)
                .sum::<Vec3>();
            let translation = (before.positions[bone].distance_squared(after.positions[bone]) > 1e-12)
                .then_some(previous + after.positions[bone] - before.positions[bone]);
            if rotation.is_none() && translation.is_none() {
                continue;
            }
            // Each channel has one owner; later edits replace only that channel.
            for (_, old) in self.edits.iter_mut().filter(|(_, e)| e.bone == bone) {
                if rotation.is_some() {
                    old.rotation = None;
                }
                if translation.is_some() {
                    old.translation = None;
                }
            }
            self.edits.retain(|(_, old)| old.rotation.is_some() || old.translation.is_some());
            self.edits.push((
                owner,
                ik::JointEdit {
                    bone,
                    rotation,
                    translation,
                    weight: 1.0,
                },
            ));
        }
    }
}

/// Inverse of Rig::world_with: parent scale affects offsets, never inherited
/// rotations or child scale (LLXformMatrix). The avatar model is removed first.
pub(super) fn local_pose(rig: &Rig, frame: &Frame) -> Option<(ik::LocalPose, Vec<Vec3>)> {
    let inverse = frame.model.inverse();
    let mut matrices: Vec<_> = frame
        .joints
        .iter()
        .map(|m| (inverse * *m).to_scale_rotation_translation())
        .collect();
    if matrices.len() != rig.len()
        || matrices
            .iter()
            .any(|(s, q, p)| !s.is_finite() || s.abs().min_element() < 1e-5 || !q.is_finite() || !p.is_finite())
    {
        return None;
    }
    // Shape sliders give some face and pectoral joints a negative scale on
    // one axis (LLPolySkeletalDistortion adds scale deltas without a floor).
    // A mirrored matrix does not tell which axis: glam puts the sign on X
    // and turns the rotation half a turn. Keep the reading whose local
    // rotation is nearest the rig's rest one, so that rotations set from a
    // preset, a file or the T-pose do not turn that joint over. Parents first.
    for &bone in &rig.order {
        let (scale, rotation, _) = matrices[bone];
        if scale.x >= 0.0 {
            continue;
        }
        let parent = rig.parent[bone].map_or(Quat::IDENTITY, |p| matrices[p].1);
        let rest = rig.local_rot[bone];
        let (flip, signs) = [
            (Quat::IDENTITY, Vec3::ONE),
            (Quat::from_rotation_z(std::f32::consts::PI), Vec3::new(-1.0, -1.0, 1.0)),
            (Quat::from_rotation_y(std::f32::consts::PI), Vec3::new(-1.0, 1.0, -1.0)),
            (Quat::from_rotation_x(std::f32::consts::PI), Vec3::new(1.0, -1.0, -1.0)),
        ]
        .into_iter()
        .max_by(|a, b| {
            let near = |flip: Quat| (parent.inverse() * rotation * flip).normalize().dot(rest).abs();
            near(a.0).total_cmp(&near(b.0))
        })
        .unwrap_or((Quat::IDENTITY, Vec3::ONE));
        matrices[bone].0 = scale * signs;
        matrices[bone].1 = (rotation * flip).normalize();
    }
    let mut pose = ik::LocalPose {
        rotations: vec![Quat::IDENTITY; rig.len()],
        positions: vec![Vec3::ZERO; rig.len()],
    };
    for bone in 0..rig.len() {
        let (parent_scale, parent_q, parent_p) =
            rig.parent[bone].map_or((rig.parent_scale[bone], Quat::IDENTITY, Vec3::ZERO), |parent| matrices[parent]);
        if parent_scale.abs().min_element() < 1e-5 {
            return None;
        }
        pose.rotations[bone] = (parent_q.inverse() * matrices[bone].1).normalize();
        pose.positions[bone] = (parent_q.inverse() * (matrices[bone].2 - parent_p)) / parent_scale;
    }
    Some((pose, matrices.iter().map(|m| m.0).collect()))
}

impl Editor {
    pub(super) fn pose_constrained(&self, rig: &Rig, bone: usize) -> bool {
        self.control == Control::Pole
            || self.limb_enabled(bone)
            || self.bindings.iter().any(|b| b.bone == bone)
            || self.graph.definition.bindings.iter().any(|b| rig.names.get(&b.bone) == Some(&bone))
    }
    pub(super) fn commit_manipulation(&mut self, rig: &Rig, frame: &Frame, rotate: bool) {
        if self.poser.active {
            self.commit_poser_manipulation(rig, frame, rotate);
            return;
        }
        let Some(bone) = self.selected else { return };
        if self.pose_constrained(rig, bone) || self.clip.waiting_source() {
            self.commit();
            return;
        }
        let mirror = self.symmetry.then(|| self.mirrors.counterpart(bone)).flatten();
        let pinned_mirror = mirror.filter(|other| self.pose_constrained(rig, *other));
        let Some((mut pose, scale)) = local_pose(rig, frame) else { return };
        if self.disabled.contains(&bone) {
            self.authoring.remove(bone);
        }
        self.commit();
        let bones = [Some(bone), mirror];
        for bone in bones.into_iter().flatten() {
            if Some(bone) == pinned_mirror {
                continue;
            }
            let Some(mut goal) = self.goals.iter().find(|g| g.bone == bone).cloned() else {
                continue;
            };
            if rotate && goal.mode == Mode::Ik {
                // FK rotation keeps the joint pivot and every ancestor fixed.
                goal.root = bone;
                goal.pole = None;
            }
            goal.weight = 1.0;
            let before = pose.clone();
            if ik::apply_goal(rig, &mut pose, Vec3::ZERO, &scale, &goal) {
                self.authoring.record(bone, &before, &pose);
            }
        }
        self.refresh_clip_marks();
    }
    pub(super) fn sync_authored_selection(&mut self, frame: &Frame) {
        let Some(bone) = self.selected.filter(|bone| self.authoring.owns(*bone)) else {
            return;
        };
        if self.graph.selected.is_some() || self.drag.is_some() || self.gizmo.dragging() || self.weight != 1.0 {
            return;
        }
        let local = frame.model.inverse() * frame.joints[bone];
        self.position = local.transform_point3(self.contact);
        let (z, y, x) = local.to_scale_rotation_translation().1.to_euler(EulerRot::ZYX);
        self.degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
    }
}

#[cfg(test)]
mod tests;
