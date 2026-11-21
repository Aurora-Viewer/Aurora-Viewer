//! Turning a joint's goal on and off, and the four limbs' IK switches.
use super::*;

impl Editor {
    pub(super) fn enabled(&self, bone: usize) -> bool {
        self.goals.iter().any(|g| g.bone == bone) && !self.disabled.contains(&bone)
    }
    pub(super) fn limb_enabled(&self, bone: usize) -> bool {
        self.enabled(bone) && !self.authoring.owns(bone) && self.goals.iter().any(|g| g.bone == bone && g.mode == Mode::Ik)
    }

    pub(super) fn enable_limb(&mut self, bone: usize, enabled: bool, rig: &Rig, frame: &Frame) {
        self.enable(bone, enabled, rig, frame);
        if !enabled {
            return;
        }
        let Some(limb) = limbs(rig).into_iter().find(|l| l.end == bone) else {
            return;
        };
        let Some(goal) = self.goals.iter_mut().find(|g| g.bone == bone && g.mode == Mode::Direct) else {
            return;
        };
        goal.mode = Mode::Ik;
        goal.root = ik::extended_root(rig, bone).unwrap_or(limb.root);
        goal.pole = Some(limb.pole(frame));
        if self.selected == Some(bone) {
            self.select(bone, rig, frame);
        }
    }

    pub(super) fn enable(&mut self, bone: usize, enabled: bool, rig: &Rig, frame: &Frame) {
        if enabled && self.authoring.owns(bone) {
            self.authoring.remove(bone);
            self.goals.retain(|g| g.bone != bone);
        }
        if enabled && limbs(rig).iter().any(|l| l.end == bone) {
            self.limb_controls = true;
            self.visibility.reveal(&rig.canonical_names[bone]);
        }
        self.disabled.retain(|b| *b != bone);
        if enabled {
            if !self.goals.iter().any(|g| g.bone == bone) {
                self.select(bone, rig, frame);
                if let Some(limb) = limbs(rig).into_iter().find(|l| l.end == bone) {
                    self.root = ik::extended_root(rig, bone).unwrap_or(limb.root);
                    self.pole = Some(limb.pole(frame));
                }
                // Activation is deliberately per limb even with symmetry on.
                let symmetry = self.symmetry;
                self.symmetry = false;
                self.commit();
                self.symmetry = symmetry;
            }
            // Existing pins retain their creator-selected root. Extended roots
            // are defaults for new pins, never an activation-time migration.
            if self.selected == Some(bone) {
                // Selection of an inactive pin rebases its displayed transform.
                // Restoring the pin must also restore the inspector's target.
                self.select(bone, rig, frame);
            }
        } else {
            self.disabled.push(bone);
            self.drag = None;
            self.gizmo.cancel();
            if self.selected == Some(bone) {
                self.released_pose(frame);
                self.refresh = true;
            }
        }
    }
}
