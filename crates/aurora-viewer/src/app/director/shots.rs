//! The Director's shots in the app (`crate::director::shots`,
//! `ui/director/shots.rs`): where a shot starts from, the keyframes that
//! travel with their target, and the `orbit` demo.

use super::super::App;
use crate::camera::track::Pose;
use crate::director::shots::{self, Here, KeyShot, Kind, Shot};
use crate::director::{Aim, Joint, Target};
use glam::Vec3;

/// The alt-camera's focus is the shots' centre when it is at least this far
/// from the camera; nearer (or with a camera that has no focus), the point
/// the Director's lens is focused at, ahead of the view.
const FOCUS_NEAREST: f32 = 1.0;

impl App {
    /// Where a shot starts from: the current view, turning round the lock's
    /// target while the lock is on and it is in sight, else round the
    /// camera's focus (the alt-camera's), else the point the Director's
    /// lens focuses at (`Director::focus`) ahead of the view.
    pub(super) fn director_shots_here(&self) -> Here {
        let view = self.director_pose();
        let lock = &self.director.d.lock;
        let target = lock.on.then(|| self.director.aims.get(&lock.aim()).copied()).flatten();
        let focus = self.camera.target;
        let has_focus = self.camera.flycam.is_none()
            && self.camera.free().is_none()
            && focus.is_finite()
            && focus.distance(view.position) >= FOCUS_NEAREST;
        let centre = target.unwrap_or(if has_focus {
            focus
        } else {
            view.position + view.forward() * self.director.d.focus
        });
        Here {
            view,
            centre,
            on_target: target.is_some(),
        }
    }

    /// Keyframe `index` travels with its target (`on`), from where that
    /// target is now, or stays where it is in the world. Nothing when it
    /// looks at no target or the target is not in sight.
    pub(super) fn director_key_travel(&mut self, index: usize, on: bool) {
        let Some(key) = self.director.d.track.keys().get(index).copied() else {
            return;
        };
        let anchor = if on {
            let Some(aim) = self.director.d.key_aim(index) else {
                return;
            };
            match self.director.aims.get(&aim).copied().or_else(|| self.director_target(aim)) {
                Some(point) => Some(point),
                None => {
                    log::info!("director: keyframe {} has its target out of sight, it stays in place", index + 1);
                    return;
                }
            }
        } else {
            None
        };
        self.director.d.track.set_shot(index, KeyShot { anchor, ..key.shot });
    }

    /// "Update" on a keyframe that travels with its target: laid again
    /// next to where the target is now.
    pub(super) fn director_key_laid_again(&mut self, index: usize) {
        if self.director.d.track.keys().get(index).is_some_and(|key| key.shot.anchor.is_some()) {
            self.director_key_travel(index, true);
        }
    }

    /// AURORA_DEMO_DIRECTOR=orbit: the view locked on our avatar's neck and
    /// the track replaced by an orbit round it, one turn in 10 s from 6 m
    /// away, travelling with it.
    pub(super) fn director_shots_demo(&mut self) {
        let d = &mut self.director.d;
        d.lock.on = true;
        d.lock.target = Target::Own;
        d.lock.joint = Joint::Neck;
        let aim = Aim {
            target: Target::Own,
            joint: Joint::Neck,
        };
        let Some(centre) = self.director_target(aim) else {
            log::warn!("director demo: our avatar is not in the scene, no orbit");
            return;
        };
        let eye = centre + Vec3::new(6.0, 0.0, 1.0);
        let here = Here {
            view: Pose::from_view(eye, centre - eye, Vec3::Z, 1.0),
            centre,
            on_target: true,
        };
        let shot = Shot {
            replace: true,
            ..Shot::new(Kind::Orbit)
        };
        let laid = shots::make(&shot, &here).map(|made| (made.keys.len(), shots::lay(&mut self.director.d, &shot, &made, Some(centre))));
        match laid {
            Some((count, Ok(_))) => log::info!(
                "director demo: an orbit of {count} keyframes round our avatar, {:.1} s",
                self.director.d.track.duration()
            ),
            _ => log::warn!("director demo: the orbit could not be laid"),
        }
    }
}
