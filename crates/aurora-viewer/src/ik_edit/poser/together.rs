//! Several joints edited as one. Shift + click on a joint (its marker, its
//! row or its finger cell) adds it to the selected joint, or takes it out;
//! the joints added follow every change of the selected one: both eyes turn
//! together, both shoulders lift together, a few spine joints bend together.
//! This is not Mirror: the followers get the same change, not its reflection.
//!
//! Port of the multi-selection of FSFloaterPoser's joint lists
//! (`multi_select` in floater_fs_poser.xml, FSFloaterPoser::getUiSelectedPoserJoints)
//! and of setSelectedJointsRotation / setSelectedJointsPosition /
//! setSelectedJointsScale (indra/newview/fsfloaterposer.cpp), which apply a
//! change to every selected joint. The Legacy Viewer gives each joint the
//! same absolute value in its default mode; here each follower gets the
//! leader's change on top of its own pose, so joints that start apart keep
//! their difference (for two eyes at rest the result is the same).
use super::*;

impl Editor {
    /// The joints that follow the selected one, without the selected one.
    pub(super) fn followers(&self) -> Vec<usize> {
        self.poser.together.iter().copied().filter(|b| Some(*b) != self.selected).collect()
    }
    /// Whether `bone` is the selected joint or one of its followers.
    pub(super) fn in_selection(&self, bone: usize) -> bool {
        self.selected == Some(bone) || self.poser.together.contains(&bone)
    }
    /// A click on a joint: alone it selects that joint only; with Shift it
    /// adds the joint to the selection, or takes it out.
    pub(super) fn click_pose_bone(&mut self, bone: usize, rig: &Rig, frame: &Frame, add: bool) {
        if add && self.selected.is_some_and(|s| s != bone) {
            toggle(&mut self.poser.together, bone);
            return;
        }
        if add && self.selected == Some(bone) {
            // the selected joint taken out: the first follower leads
            if let Some(next) = self.followers().first().copied() {
                self.poser.together.retain(|b| *b != next);
                self.select_pose_bone(next, rig, frame);
            }
            return;
        }
        self.poser.together.clear();
        self.select_pose_bone(bone, rig, frame);
    }
    /// A drag begun on a joint: on a joint of the selection it leads the
    /// others, elsewhere it selects that joint only.
    pub(super) fn grab_pose_bone(&mut self, bone: usize, rig: &Rig, frame: &Frame) {
        if self.selected == Some(bone) {
            return;
        }
        if self.poser.together.contains(&bone) {
            let lead = self.selected;
            self.poser.together.retain(|b| *b != bone);
            self.poser.together.extend(lead);
        } else {
            self.poser.together.clear();
        }
        self.select_pose_bone(bone, rig, frame);
    }
    pub(super) fn clear_together(&mut self) {
        self.poser.together.clear();
    }
}

/// Add `bone` to the followers, or take it out.
fn toggle(together: &mut Vec<usize>, bone: usize) {
    if let Some(i) = together.iter().position(|b| *b == bone) {
        together.remove(i);
    } else {
        together.push(bone);
    }
}

/// What the leader went through: the followers get the same kind of change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Rotate,
    Move,
    Scale,
}

/// Give each follower the change `lead` went through from `before` to
/// `after`: a rotation turned by the same amount in the follower's own
/// frame, a move by the same offset in the avatar's frame, a scale by the
/// same ratio. A follower's world lock goes, as the leader's does.
pub(super) fn follow(rig: &Rig, before: &SubjectPose, after: &mut SubjectPose, lead: usize, followers: &[usize], change: Change) {
    let followers: Vec<_> = followers.iter().copied().filter(|f| *f != lead && *f < rig.len()).collect();
    if followers.is_empty() {
        return;
    }
    match change {
        Change::Rotate => {
            let turn = (before.rotations[lead].inverse() * after.rotations[lead]).normalize();
            for &bone in &followers {
                after.rotations[bone] = (before.rotations[bone] * turn).normalize();
            }
        }
        Change::Scale => {
            let grow = after.scales[lead] / before.scales[lead];
            if !grow.is_finite() {
                return;
            }
            for &bone in &followers {
                after.scales[bone] = data::bounded_scale(before.scales[bone] * grow);
            }
        }
        Change::Move => {
            let was = rig.world_with(&before.rotations, Vec3::ZERO, &before.positions, &before.scales);
            let now = rig.world_with(&after.rotations, Vec3::ZERO, &after.positions, &after.scales);
            let offset = now[lead].w_axis.truncate() - was[lead].w_axis.truncate();
            if !offset.is_finite() {
                return;
            }
            for &bone in &followers {
                after.move_joint(rig, bone, was[bone].w_axis.truncate() + offset);
            }
        }
    }
    for bone in followers {
        after.locks.remove(&bone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(rig: &Rig) -> SubjectPose {
        SubjectPose {
            rotations: rig.local_rot.clone(),
            positions: rig.local_pos.clone(),
            scales: rig.scale.clone(),
            locks: Default::default(),
            origin: rig.local_rot.clone(),
        }
    }

    #[test]
    fn both_eyes_turn_together_and_not_mirrored() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        let (left, right) = (rig.names["mEyeLeft"], rig.names["mEyeRight"]);
        let before = rest(rig);
        let mut after = before.clone();
        let look = Quat::from_rotation_z(0.4);
        after.rotations[left] = before.rotations[left] * look;
        follow(rig, &before, &mut after, left, &[right], Change::Rotate);
        let world = rig.world_with(&after.rotations, Vec3::ZERO, &after.positions, &after.scales);
        let (_, l, _) = world[left].to_scale_rotation_translation();
        let (_, r, _) = world[right].to_scale_rotation_translation();
        // the same gaze: both forward axes point the same way
        assert!((l * Vec3::X).dot(r * Vec3::X) > 0.9999);
        // and they did turn
        assert!(after.rotations[right].angle_between(before.rotations[right]) > 0.39);
    }

    #[test]
    fn followers_keep_their_own_offset_and_drop_their_lock() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        let (lead, other) = (rig.names["mShoulderLeft"], rig.names["mShoulderRight"]);
        let mut before = rest(rig);
        before.rotations[other] *= Quat::from_rotation_x(0.3);
        before.locks.insert(other, Mat4::IDENTITY);
        let mut after = before.clone();
        let turn = Quat::from_rotation_y(-0.5);
        after.rotations[lead] = before.rotations[lead] * turn;
        follow(rig, &before, &mut after, lead, &[other], Change::Rotate);
        assert!(after.rotations[other].abs_diff_eq(before.rotations[other] * turn, 1e-5));
        assert!(!after.locks.contains_key(&other));
    }

    #[test]
    fn scale_follows_by_ratio_and_nothing_else() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        let (lead, other) = (rig.names["mEyeLeft"], rig.names["mEyeRight"]);
        let before = rest(rig);
        let mut after = before.clone();
        after.scales[lead] = before.scales[lead] * 1.5;
        follow(rig, &before, &mut after, lead, &[other], Change::Scale);
        assert!(after.scales[other].abs_diff_eq(before.scales[other] * 1.5, 1e-5));
        assert_eq!(after.rotations[other], before.rotations[other]);
        assert_eq!(after.positions[other], before.positions[other]);
    }

    #[test]
    fn a_move_carries_the_same_offset() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        let (lead, other) = (rig.names["mWristLeft"], rig.names["mWristRight"]);
        let start = rest(rig);
        let was = rig.world_with(&start.rotations, Vec3::ZERO, &start.positions, &start.scales);
        let offset = Vec3::new(0.08, 0.0, 0.1);
        // a drag: one small step a frame, the follower after the leader
        let mut after = start.clone();
        for step in 1..=20 {
            let before = after.clone();
            let target = was[lead].w_axis.truncate() + offset * (step as f32 / 20.0);
            assert!(after.move_joint(rig, lead, target));
            follow(rig, &before, &mut after, lead, &[other], Change::Move);
        }
        let mut alone = start.clone();
        for step in 1..=20 {
            alone.move_joint(rig, lead, was[lead].w_axis.truncate() + offset * (step as f32 / 20.0));
        }
        let alone = rig.world_with(&alone.rotations, Vec3::ZERO, &alone.positions, &alone.scales);
        let now = rig.world_with(&after.rotations, Vec3::ZERO, &after.positions, &after.scales);
        // the follower does not pull the leader away
        assert!(now[lead].w_axis.distance(alone[lead].w_axis) < 0.002);
        let led = now[lead].w_axis.truncate() - was[lead].w_axis.truncate();
        let moved = now[other].w_axis.truncate() - was[other].w_axis.truncate();
        // the chain solver leaves both hands the same small sideways error
        // (mirrored, one hand on each side): forward and up match exactly
        assert!(
            led.distance(offset) < 0.03 && moved.distance(offset) < 0.03,
            "{led} / {moved} vs {offset}"
        );
        assert!(
            (moved.x - led.x).abs() < 0.005 && (moved.z - led.z).abs() < 0.005,
            "{moved} vs {led}"
        );
        assert!((moved.y + led.y).abs() < 0.005, "{moved} vs {led}");
    }

    #[test]
    fn shift_click_adds_and_removes_and_a_plain_click_resets() {
        let (world, mut editor) = super::super::tests::selected_fixture();
        let rig = &world.avatar_lib.rig;
        let frame = editor.editor_frame(&world).unwrap();
        let (head, left, right) = (rig.names["mHead"], rig.names["mEyeLeft"], rig.names["mEyeRight"]);
        editor.click_pose_bone(left, rig, &frame, false);
        editor.click_pose_bone(right, rig, &frame, true);
        assert_eq!(editor.selected, Some(left));
        assert_eq!(editor.followers(), vec![right]);
        assert!(editor.in_selection(right) && !editor.in_selection(head));
        // Shift on the leader: the follower leads alone
        editor.click_pose_bone(left, rig, &frame, true);
        assert_eq!(editor.selected, Some(right));
        assert!(editor.followers().is_empty());
        editor.click_pose_bone(left, rig, &frame, true);
        editor.click_pose_bone(left, rig, &frame, true);
        assert!(editor.followers().is_empty());
        editor.click_pose_bone(left, rig, &frame, true);
        editor.click_pose_bone(head, rig, &frame, false);
        assert_eq!(editor.selected, Some(head));
        assert!(editor.followers().is_empty());
    }
}
