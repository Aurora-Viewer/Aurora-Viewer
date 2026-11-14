//! Several joints edited as one in the animation editor, like the pose
//! editor's (`poser/together.rs`): Shift or Ctrl + click on a joint (its
//! handle, its bone, its dot in the picker or its row in Tracks) adds it to
//! the selected joint, or takes it out. The joints added get the selected
//! one's change on top of their own pose (the same turn in their own frame,
//! the same offset for a move) and are keyed with it.
//!
//! Port of the multi-selection of FSFloaterPoser's joint lists
//! (FSFloaterPoser::getUiSelectedPoserJoints, setSelectedJointsRotation and
//! setSelectedJointsPosition, indra/newview/fsfloaterposer.cpp), which the
//! pose editor already follows; the Legacy Viewer has no animation editor.
use super::*;

impl Editor {
    /// The joints that follow the selected one in the animation editor,
    /// without the selected one.
    pub(in crate::ik_edit) fn clip_followers(&self) -> Vec<usize> {
        self.clip.together.iter().copied().filter(|b| Some(*b) != self.selected).collect()
    }
    /// Whether `bone` follows the selected joint in the animation editor.
    pub(in crate::ik_edit) fn clip_follows(&self, bone: usize) -> bool {
        self.clip.active && Some(bone) != self.selected && self.clip.together.contains(&bone)
    }
    /// A click on a joint in the animation editor: alone it selects that
    /// joint only; with Shift or Ctrl it adds the joint to the selection, or
    /// takes it out (the selected joint taken out: the first follower leads).
    pub(in crate::ik_edit) fn clip_click_bone(&mut self, bone: usize, rig: &Rig, frame: &Frame, add: bool) {
        match self.selected {
            Some(lead) if add && lead != bone => {
                if let Some(i) = self.clip.together.iter().position(|b| *b == bone) {
                    self.clip.together.remove(i);
                } else {
                    self.clip.together.push(bone);
                }
            }
            Some(_) if add => {
                if let Some(next) = self.clip_followers().first().copied() {
                    self.clip.together.retain(|b| *b != next);
                    self.select(next, rig, frame);
                }
            }
            _ => {
                self.clip.together.clear();
                self.select(bone, rig, frame);
            }
        }
    }
    /// A drag begun on a joint: on a joint of the selection it leads the
    /// others, elsewhere it selects that joint only.
    pub(in crate::ik_edit) fn clip_grab_bone(&mut self, bone: usize, rig: &Rig, frame: &Frame) {
        if self.selected == Some(bone) {
            return;
        }
        if self.clip.together.contains(&bone) {
            let lead = self.selected;
            self.clip.together.retain(|b| *b != bone);
            self.clip.together.extend(lead);
        } else {
            self.clip.together.clear();
        }
        self.select(bone, rig, frame);
    }
    /// Give the followers the change `lead` went through from `start` to
    /// `pose`, record it as their own edit and keep a goal for each, so that
    /// the clip shows and keys them like the selected joint. `skip`: the
    /// mirrored joint, which symmetry already moved.
    pub(in crate::ik_edit) fn follow_clip_lead(
        &mut self,
        rig: &Rig,
        lead: usize,
        skip: Option<usize>,
        start: &ik::LocalPose,
        pose: &mut ik::LocalPose,
        scale: &[Vec3],
        rotate: bool,
    ) {
        let followers: Vec<_> = self
            .clip_followers()
            .into_iter()
            .filter(|f| *f < rig.len() && Some(*f) != skip && !self.pose_constrained(rig, *f))
            .collect();
        if followers.is_empty() || lead >= rig.len() {
            return;
        }
        let was = rig.world_with(&start.rotations, Vec3::ZERO, &start.positions, scale);
        let now = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, scale);
        let turn = (start.rotations[lead].inverse() * pose.rotations[lead]).normalize();
        let offset = now[lead].w_axis.truncate() - was[lead].w_axis.truncate();
        let direct = self.goals.iter().any(|g| g.bone == lead && g.mode == Mode::Direct);
        for bone in followers {
            // a joint the selected one's own change already carried (its
            // chain, or a child for a move) keeps that and nothing more
            let moved = pose.rotations[bone].dot(start.rotations[bone]).abs() < 0.9999999
                || (!rotate && now[bone].w_axis.truncate().distance_squared(was[bone].w_axis.truncate()) > 1e-12);
            if moved {
                continue;
            }
            let before = pose.clone();
            let mode = if direct || rig.parent[bone].is_none() {
                Mode::Direct
            } else {
                Mode::Ik
            };
            if rotate {
                pose.rotations[bone] = (start.rotations[bone] * turn).normalize();
            } else {
                if !offset.is_finite() || offset.length_squared() < 1e-12 {
                    continue;
                }
                let goal = Goal {
                    follow: None,
                    weight: 1.0,
                    bone,
                    root: ik::default_root(rig, bone),
                    position: was[bone].w_axis.truncate() + offset,
                    rotation: None,
                    contact: Vec3::ZERO,
                    pole: None,
                    mode,
                };
                if !ik::apply_goal(rig, pose, Vec3::ZERO, scale, &goal) {
                    *pose = before;
                    continue;
                }
            }
            if self.disabled.contains(&bone) {
                self.authoring.remove(bone);
            }
            self.authoring.record(bone, &before, pose);
            let world = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, scale);
            let (_, rotation, position) = world[bone].to_scale_rotation_translation();
            let old = self.goals.iter().find(|g| g.bone == bone).cloned();
            let goal = Goal {
                follow: None,
                weight: 1.0,
                bone,
                root: if rotate {
                    bone
                } else {
                    old.as_ref().map_or(ik::default_root(rig, bone), |g| g.root)
                },
                position,
                rotation: (rotate || old.as_ref().is_some_and(|g| g.rotation.is_some())).then_some(rotation),
                contact: Vec3::ZERO,
                pole: None,
                mode,
            };
            self.disabled.retain(|b| *b != bone);
            self.clip.mark(&goal);
            if let Some(old) = self.goals.iter_mut().find(|g| g.bone == bone) {
                *old = goal;
            } else {
                self.goals.push(goal);
            }
        }
    }
}
