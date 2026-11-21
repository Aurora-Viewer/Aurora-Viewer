//! Several keyframes selected at once: Ctrl + click adds or removes one,
//! Shift + click every one from the selected keyframe to it, on the
//! timeline and on their cameras in the world. The keyframe the window
//! shows stays `Director::selected`; the others are `Director::group`. The
//! group moves together (its cameras dragged or moved with the gizmo, as
//! the build tools move a selection of objects; its times shifted on the
//! timeline), is deleted together, and a setting changed on the shown
//! keyframe is given to every keyframe of the group (`Director::spread`).
//!
//! Aurora's own: the Legacy Viewer has no camera path tool.

use super::Director;
use crate::camera::track::{Keyframe, Pose};
use glam::{Quat, Vec3};

impl Director {
    /// Every keyframe selected, in the track's order.
    pub fn selection(&self) -> Vec<usize> {
        let len = self.track.len();
        let mut all: Vec<usize> = self
            .selected
            .into_iter()
            .chain(self.group.iter().copied())
            .filter(|index| *index < len)
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    pub fn is_selected(&self, index: usize) -> bool {
        index < self.track.len() && (self.selected == Some(index) || self.group.contains(&index))
    }

    /// Ctrl + click: keyframe `index` joins the selection (and is the one
    /// the window shows) or leaves it.
    pub fn toggle(&mut self, index: usize) {
        if index >= self.track.len() {
            return;
        }
        if self.selected == Some(index) {
            // the next one selected is shown in its place
            self.group.retain(|other| *other != index);
            let next = (!self.group.is_empty()).then(|| self.group.remove(0));
            self.selected = next;
            if let Some(next) = next {
                self.time = self.track.keys()[next].time;
            }
        } else if self.group.contains(&index) {
            self.group.retain(|other| *other != index);
        } else {
            self.group.extend(self.selected.filter(|selected| *selected < self.track.len()));
            self.selected = Some(index);
            self.time = self.track.keys()[index].time;
        }
        self.group.sort_unstable();
        self.group.dedup();
    }

    /// Shift + click: every keyframe from the one selected to `index`,
    /// `index` shown.
    pub fn extend_to(&mut self, index: usize) {
        let Some(from) = self.selected.filter(|from| *from < self.track.len()) else {
            self.select(index);
            return;
        };
        if index >= self.track.len() {
            return;
        }
        self.group.extend(from.min(index)..=from.max(index));
        self.group.retain(|other| *other != index);
        self.group.sort_unstable();
        self.group.dedup();
        self.selected = Some(index);
        self.time = self.track.keys()[index].time;
    }

    /// Keyframe `index` shown in the window, the selection kept when it is
    /// part of it (a drag that starts on one of the group); else it alone.
    pub fn focus(&mut self, index: usize) {
        if !self.is_selected(index) {
            self.select(index);
            return;
        }
        if let Some(previous) = self.selected.filter(|previous| *previous != index) {
            self.group.push(previous);
        }
        self.group.retain(|other| *other != index);
        self.group.sort_unstable();
        self.selected = Some(index);
        self.time = self.track.keys()[index].time;
    }

    /// Delete every keyframe selected.
    pub fn remove_selection(&mut self) {
        for index in self.selection().into_iter().rev() {
            self.remove(index);
        }
        self.group.clear();
    }

    /// The keyframes of the group given what changed on the one shown, from
    /// `before` to now: its lens, its roll, coming to rest, what it looks at,
    /// its easing, dolly zoom and travel, its animations, aperture, focus
    /// and time of day. Its place, heading, pitch and time stay each one's.
    pub fn spread(&mut self, before: &Keyframe) {
        let Some(now) = self.selected.and_then(|index| self.track.keys().get(index).copied()) else {
            return;
        };
        if self.group.is_empty() || now == *before {
            return;
        }
        let roll = |pose: &Pose| pose.angles().2;
        let (fov, rolled) = (now.pose.fov != before.pose.fov, (roll(&now.pose) - roll(&before.pose)).abs() > 1e-5);
        for index in self.group.clone() {
            let Some(mut key) = self.track.keys().get(index).copied() else {
                continue;
            };
            if fov {
                key.pose.fov = now.pose.fov;
            }
            if rolled {
                let (yaw, pitch, _) = key.pose.angles();
                key.pose = Pose::from_angles(key.pose.position, yaw, pitch, roll(&now.pose), key.pose.fov);
            }
            if now.rest != before.rest {
                key.rest = now.rest;
            }
            if now.aim != before.aim {
                key.aim = now.aim;
            }
            let (shot, was) = (now.shot, before.shot);
            if shot.ease != was.ease {
                key.shot.ease = shot.ease;
            }
            if shot.dolly_zoom != was.dolly_zoom {
                key.shot.dolly_zoom = shot.dolly_zoom;
            }
            if shot.anchor != was.anchor {
                key.shot.anchor = shot.anchor;
            }
            let (extra, was) = (now.extra, before.extra);
            if extra.start != was.start {
                key.extra.start = extra.start;
            }
            if extra.stop != was.stop {
                key.extra.stop = extra.stop;
            }
            if extra.aperture != was.aperture {
                key.extra.aperture = extra.aperture;
            }
            if extra.focus != was.focus {
                key.extra.focus = extra.focus;
            }
            if extra.hour != was.hour {
                key.extra.hour = extra.hour;
            }
            self.track.set_pose(index, key.pose);
            self.track.set_rest(index, key.rest);
            self.track.set_aim(index, key.aim);
            self.track.set_shot(index, key.shot);
            self.track.set_extra(index, key.extra);
        }
    }

    /// The selection's views when a move of it starts.
    pub fn selected_poses(&self) -> Vec<(usize, Pose)> {
        self.selection()
            .into_iter()
            .map(|index| (index, self.track.keys()[index].pose))
            .collect()
    }
}

/// Where the middle of `poses` is: the point a group turns about.
pub fn centre(poses: &[(usize, Pose)]) -> Option<Vec3> {
    (!poses.is_empty()).then(|| poses.iter().map(|(_, pose)| pose.position).sum::<Vec3>() / poses.len() as f32)
}

/// `starts` moved as one: turned by `turn` about `pivot`, then moved by
/// `shift`; each camera turns with it, as the objects of a selection do.
pub fn moved(starts: &[(usize, Pose)], pivot: Vec3, shift: Vec3, turn: Quat) -> Vec<(usize, Pose)> {
    starts
        .iter()
        .map(|(index, pose)| {
            let position = pivot + turn * (pose.position - pivot) + shift;
            let rotation = (turn * pose.rotation).normalize();
            (
                *index,
                Pose {
                    position,
                    rotation,
                    ..*pose
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::director::KeyAim;

    fn at(x: f32) -> Pose {
        Pose::from_view(Vec3::new(x, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0)
    }

    fn five() -> Director {
        let mut d = Director::default();
        for x in [0.0, 5.0, 10.0, 15.0, 20.0] {
            d.add_key(at(x));
        }
        d.select(0);
        d
    }

    #[test]
    fn ctrl_adds_and_removes_and_shift_takes_a_run() {
        let mut d = five();
        d.toggle(2);
        assert_eq!((d.selected, d.selection()), (Some(2), vec![0, 2]));
        d.toggle(4);
        assert_eq!((d.selected, d.selection()), (Some(4), vec![0, 2, 4]));
        // the one shown taken away: the next one selected is shown
        d.toggle(4);
        assert_eq!((d.selected, d.selection()), (Some(0), vec![0, 2]));
        d.toggle(2);
        assert_eq!(d.selection(), vec![0]);
        // a run from the one shown, which it joins
        d.select(1);
        d.extend_to(3);
        assert_eq!((d.selected, d.selection()), (Some(3), vec![1, 2, 3]));
        // a plain click selects it alone
        d.select(4);
        assert_eq!(d.selection(), vec![4]);
        // a drag on one of the group keeps the group
        d.extend_to(2);
        d.focus(3);
        assert_eq!((d.selected, d.selection()), (Some(3), vec![2, 3, 4]));
        d.focus(0);
        assert_eq!(d.selection(), vec![0]);
    }

    #[test]
    fn a_group_is_deleted_together_and_follows_a_deletion() {
        let mut d = five();
        d.toggle(2);
        d.toggle(4);
        d.remove(1);
        assert_eq!(d.selection(), vec![0, 1, 3]);
        d.remove_selection();
        assert_eq!((d.track.len(), d.selection()), (1, vec![]));
        let mut d = five();
        d.extend_to(4);
        d.add_key(at(30.0));
        assert_eq!(d.selection(), vec![5], "a keyframe laid is selected alone");
    }

    #[test]
    fn a_setting_changed_on_the_one_shown_goes_to_the_group() {
        let mut d = five();
        // 0 and 2 selected, 2 shown; 4 apart
        d.toggle(2);
        assert_eq!((d.selected, d.selection()), (Some(2), vec![0, 2]));
        let before = d.track.keys()[2];
        let mut key = before;
        key.pose.fov = 0.6;
        key.rest = true;
        key.aim = KeyAim::Free;
        key.extra.hour = Some(18.0);
        d.track.set_pose(2, key.pose);
        d.track.set_rest(2, key.rest);
        d.track.set_aim(2, key.aim);
        d.track.set_extra(2, key.extra);
        d.spread(&before);
        let first = d.track.keys()[0];
        assert!((first.pose.fov - 0.6).abs() < 1e-6 && first.rest && first.extra.hour == Some(18.0));
        assert_eq!(first.aim, KeyAim::Free);
        assert_eq!(first.pose.position, at(0.0).position, "its place is its own");
        let apart = d.track.keys()[4];
        assert!(!apart.rest && apart.extra.hour.is_none() && (apart.pose.fov - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_group_moves_and_turns_as_one() {
        let starts = vec![(0, at(0.0)), (1, at(10.0))];
        let pivot = centre(&starts).expect("centre");
        assert_eq!(pivot, Vec3::new(5.0, 0.0, 20.0));
        let shifted = moved(&starts, pivot, Vec3::new(0.0, 3.0, 1.0), Quat::IDENTITY);
        assert_eq!(shifted[1].1.position, Vec3::new(10.0, 3.0, 21.0));
        // a quarter turn about the middle: the ends swap sides, each camera
        // turned with them
        let turned = moved(&starts, pivot, Vec3::ZERO, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!(turned[0].1.position.distance(Vec3::new(5.0, -5.0, 20.0)) < 1e-4);
        assert!(turned[1].1.forward().distance(Vec3::Y) < 1e-4);
        assert_eq!(turned[1].1.fov, 1.0);
    }
}
