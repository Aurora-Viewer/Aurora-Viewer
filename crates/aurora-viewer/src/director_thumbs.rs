//! The keyframes' thumbnails: a small picture of the view above each
//! keyframe of the timeline. Aurora's own.
//!
//! The picture is taken when a keyframe is laid or updated: the live view is
//! that keyframe's view then. The renderer copies it and reads it back
//! without waiting (`aurora_render::thumbnail`); it comes a few frames
//! later, tagged. The pictures are kept in memory only, never in a track's
//! file, by the keyframe's view (`PoseKey`): a keyframe moved in time keeps
//! its picture, one whose view changed waits for a new one, and an undo
//! finds the picture of the view it brings back. A keyframe of a loaded
//! track has none (a placeholder) until "Go to" puts the camera there: the
//! picture is taken once the live view is that keyframe's.

use crate::camera::track::Pose;
use std::collections::{HashMap, VecDeque};

/// Pictures kept at most (the oldest go first): a few tracks' worth.
pub const KEPT: usize = 400;
/// Frames "Go to" waits for the live view to reach the keyframe's.
const REFRESH_FRAMES: u32 = 180;
/// The live view is the keyframe's when this close (metres, radians).
const SAME_PLACE: f32 = 0.02;
const SAME_TURN: f32 = 0.01;

/// A keyframe's view, as the thumbnails find it: its position, rotation and
/// lens rounded (to a millimetre, a ten-thousandth), the rotation's sign
/// settled (`q` and `-q` are one rotation; a track may write either).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PoseKey([i64; 8]);

impl PoseKey {
    pub fn of(pose: &Pose) -> PoseKey {
        let q = pose.rotation;
        let first = [q.w, q.x, q.y, q.z].into_iter().find(|c| c.abs() > 1e-6).unwrap_or(1.0);
        let q = if first < 0.0 { -q } else { q };
        let round = |v: f32, step: f32| {
            if v.is_finite() {
                (f64::from(v) / f64::from(step)).round() as i64
            } else {
                i64::MIN
            }
        };
        PoseKey([
            round(pose.position.x, 1e-3),
            round(pose.position.y, 1e-3),
            round(pose.position.z, 1e-3),
            round(q.x, 1e-4),
            round(q.y, 1e-4),
            round(q.z, 1e-4),
            round(q.w, 1e-4),
            round(pose.fov, 1e-4),
        ])
    }
}

/// What the timeline shows above a keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot<T> {
    Picture(T),
    /// Asked for, on its way.
    Waiting,
    /// None: the placeholder.
    Missing,
}

#[derive(Debug, Clone)]
struct Refresh {
    pose: Pose,
    frames: u32,
}

/// The pictures (`T`: a texture in the app), by keyframe view.
#[derive(Debug, Clone)]
pub struct Thumbs<T> {
    pictures: HashMap<PoseKey, T>,
    order: VecDeque<PoseKey>,
    /// Tags asked of the renderer, and the view each is for.
    pending: HashMap<u64, PoseKey>,
    next_tag: u64,
    refresh: Option<Refresh>,
}

impl<T> Default for Thumbs<T> {
    fn default() -> Self {
        Thumbs {
            pictures: HashMap::new(),
            order: VecDeque::new(),
            pending: HashMap::new(),
            next_tag: 1,
            refresh: None,
        }
    }
}

impl<T: Clone> Thumbs<T> {
    /// A picture of the live view is wanted for the keyframe of `pose`: the
    /// tag to ask the renderer for.
    pub fn request(&mut self, pose: &Pose) -> u64 {
        let tag = self.next_tag;
        self.next_tag += 1;
        // (an answer that never comes is forgotten with the oldest)
        if self.pending.len() >= 16
            && let Some(oldest) = self.pending.keys().min().copied()
        {
            self.pending.remove(&oldest);
        }
        self.pending.insert(tag, PoseKey::of(pose));
        tag
    }

    /// The picture tagged `tag` came: kept for its keyframe. False for a
    /// tag that is not asked (another caller's, or forgotten).
    pub fn arrived(&mut self, tag: u64, picture: T) -> bool {
        let Some(key) = self.pending.remove(&tag) else {
            return false;
        };
        if self.pictures.insert(key, picture).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > KEPT {
            if let Some(old) = self.order.pop_front() {
                self.pictures.remove(&old);
            }
        }
        true
    }

    pub fn slot(&self, pose: &Pose) -> Slot<T> {
        let key = PoseKey::of(pose);
        match self.pictures.get(&key) {
            Some(picture) => Slot::Picture(picture.clone()),
            None if self.pending.values().any(|k| *k == key) => Slot::Waiting,
            None => Slot::Missing,
        }
    }

    /// Every picture kept, by view (for the timeline).
    pub fn pictures(&self) -> impl Iterator<Item = (&PoseKey, &T)> {
        self.pictures.iter()
    }

    /// "Go to" a keyframe: without a picture, one is taken once the live
    /// view is there.
    pub fn go_to(&mut self, pose: &Pose) {
        if matches!(self.slot(pose), Slot::Missing) {
            self.refresh = Some(Refresh { pose: *pose, frames: 0 });
        }
    }

    /// Each frame, with the live view: the keyframe view to take a picture
    /// of now, once the camera that went there arrived; nothing after
    /// `REFRESH_FRAMES` (the camera was moved elsewhere, or cannot go).
    pub fn refresh_due(&mut self, live: &Pose) -> Option<Pose> {
        let refresh = self.refresh.as_mut()?;
        refresh.frames += 1;
        // (a frame or two for the camera to be drawn where it was put)
        let there = refresh.pose.position.distance(live.position) < SAME_PLACE
            && refresh.pose.rotation.angle_between(live.rotation) < SAME_TURN
            && (refresh.pose.fov - live.fov).abs() < SAME_TURN;
        if there && refresh.frames >= 3 {
            let pose = refresh.pose;
            self.refresh = None;
            return Some(pose);
        }
        if refresh.frames > REFRESH_FRAMES {
            self.refresh = None;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::Track;
    use glam::Vec3;

    impl<T: Clone> Thumbs<T> {
        fn get(&self, pose: &Pose) -> Option<&T> {
            self.pictures.get(&PoseKey::of(pose))
        }

        /// What each keyframe of `track` shows, in order.
        fn slots(&self, track: &Track) -> Vec<Slot<T>> {
            track.keys().iter().map(|key| self.slot(&key.pose)).collect()
        }
    }

    fn at(x: f32) -> Pose {
        Pose::from_view(Vec3::new(x, 0.0, 20.0), Vec3::new(1.0, 0.3, 0.0), Vec3::Z, 1.0)
    }

    fn three() -> Track {
        let mut track = Track::default();
        for x in [0.0, 5.0, 10.0] {
            track.append(at(x));
        }
        track
    }

    #[test]
    fn a_keyframe_shows_its_picture_once_it_came() {
        let mut thumbs = Thumbs::<u32>::default();
        let track = three();
        assert_eq!(thumbs.slots(&track), [Slot::Missing; 3], "placeholders at first");
        let tag = thumbs.request(&track.keys()[1].pose);
        assert_eq!(thumbs.slots(&track)[1], Slot::Waiting);
        assert!(!thumbs.arrived(tag + 50, 7), "not asked");
        assert!(thumbs.arrived(tag, 7));
        assert!(!thumbs.arrived(tag, 8), "once");
        assert_eq!(thumbs.slots(&track), [Slot::Missing, Slot::Picture(7), Slot::Missing]);
        // moved in time: the same view, the same picture
        let mut moved = track.clone();
        moved.set_time(1, 4.0);
        assert_eq!(thumbs.slots(&moved)[1], Slot::Picture(7));
        // its view changed (Update): a new picture is wanted
        moved.set_pose(1, at(6.0));
        assert_eq!(thumbs.slots(&moved)[1], Slot::Missing);
    }

    #[test]
    fn a_view_is_found_whatever_sign_its_rotation_was_written_with() {
        let pose = at(3.0);
        let flipped = Pose {
            rotation: -pose.rotation,
            ..pose
        };
        assert_eq!(PoseKey::of(&pose), PoseKey::of(&flipped));
        // a track saved and loaded back (rotations normalised again)
        let track = three();
        let file = track.to_file((256_000, 256_000));
        let aim = crate::director::Aim {
            target: crate::director::Target::Own,
            joint: crate::director::Joint::Neck,
        };
        let back = Track::from_file(&file, (256_000, 256_000), aim).expect("track");
        for (a, b) in track.keys().iter().zip(back.keys()) {
            assert_eq!(PoseKey::of(&a.pose), PoseKey::of(&b.pose));
        }
        assert_ne!(PoseKey::of(&at(3.0)), PoseKey::of(&at(3.01)));
    }

    #[test]
    fn the_oldest_pictures_go_first() {
        let mut thumbs = Thumbs::<usize>::default();
        for i in 0..KEPT + 5 {
            let tag = thumbs.request(&at(i as f32));
            thumbs.arrived(tag, i);
        }
        assert_eq!(thumbs.pictures().count(), KEPT);
        assert!(thumbs.get(&at(0.0)).is_none());
        assert_eq!(thumbs.get(&at((KEPT + 4) as f32)), Some(&(KEPT + 4)));
    }

    #[test]
    fn go_to_takes_the_picture_once_the_camera_is_there() {
        let mut thumbs = Thumbs::<u8>::default();
        let target = at(5.0);
        thumbs.go_to(&target);
        // the camera on its way, then there
        assert_eq!(thumbs.refresh_due(&at(0.0)), None);
        assert_eq!(thumbs.refresh_due(&target), None, "a frame for it to be drawn");
        assert_eq!(thumbs.refresh_due(&target), Some(target));
        assert_eq!(thumbs.refresh_due(&target), None, "once");
        // a keyframe that has its picture takes none
        let tag = thumbs.request(&target);
        thumbs.arrived(tag, 1);
        thumbs.go_to(&target);
        assert_eq!((0..5).find_map(|_| thumbs.refresh_due(&target)), None);
        // the camera never arrives: given up
        let far = at(50.0);
        thumbs.go_to(&far);
        for _ in 0..REFRESH_FRAMES + 2 {
            assert_eq!(thumbs.refresh_due(&target), None);
        }
        assert_eq!(thumbs.refresh_due(&far), None, "given up");
    }
}
