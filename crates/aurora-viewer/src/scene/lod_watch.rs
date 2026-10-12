//! When the detail level of an object at rest is looked at again.
//!
//! The detail level of an object (`lod_for`) depends on its size, on the
//! settings and on its distance to the camera. For an object that stands
//! still, only the camera changes that distance, and by no more than it
//! travels. Each check therefore leaves the object with the distance the
//! camera may still travel before its level can change ([`lod_slack`]), and
//! the object is looked at again once the camera has travelled that far:
//! never while the camera rests, a few objects per frame while it moves
//! (those near a change of level), and all of them within
//! [`LOD_PERIOD`](super::sync_sets::LOD_PERIOD) frames after a jump or a
//! change of the settings, as the slice of every object in turn did.
//!
//! The check itself is the sync's, with the exact `lod_for`: levels change
//! at the same distances as before, without the wait for the object's turn.

use glam::Vec3;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Longest travel an object waits for (m): what never changes level (a HUD
/// attachment, an object without size) still comes back some day.
const MAX_SLACK: f32 = 1.0e5;

/// Added to the travelled distance to make every watched object due.
const EVERYTHING: f64 = 4.0 * MAX_SLACK as f64;

/// Distance the camera may come closer or move away before `lod_for` can
/// give another level than it gives at `distance` (same other arguments),
/// rounded down: never more than the true margin, 0 next to a change.
pub fn lod_slack(radius: f32, distance: f32, lod_factor: f32, fov_y: f32) -> f32 {
    // (the steps of `lod_for`, inverted)
    let near = (1.0 - lod_factor * 0.1).max(0.01);
    let ramp = lod_factor * 2.0;
    let factor = lod_factor * (60f32.to_radians() / fov_y.max(0.1));
    let radius = (radius * 100.0).round() / 100.0;
    if !distance.is_finite() || !factor.is_finite() {
        return 0.0;
    }
    if radius.is_nan() || radius <= 0.0 {
        // always the finest level
        return MAX_SLACK;
    }
    // distance whose scaled value (before it is rounded) is `d`
    let unscaled = |d: f32| {
        let d = d.max(0.0) / (std::f32::consts::PI / 3.0);
        let d = if ramp > 0.0 && d < ramp { (d * ramp).sqrt() } else { d };
        d / near
    };
    // Scaled distances at which the level changes: `tan > t` holds while
    // `factor * radius / d` rounds above t, that is up to t + 0.005; and a
    // scaled distance that rounds to 0 gives the finest level.
    let changes = [0.24f32, 0.06, 0.03].map(|t| factor * radius / (t + 0.005));
    let mut slack = MAX_SLACK;
    for d in changes.into_iter().chain([0.0]) {
        // (the scaled distance is rounded to the hundredth; the rest is
        // what the floats may lose)
        let blur = 0.0052 + d * 2e-5;
        let (lo, hi) = (unscaled(d - blur), unscaled(d + blur));
        let gap = if distance < lo {
            lo - distance
        } else if distance > hi {
            distance - hi
        } else {
            0.0
        };
        slack = slack.min(gap);
    }
    (slack * 0.999 - 1e-3).max(0.0)
}

/// The objects at rest and the camera travel each one waits for.
#[derive(Default)]
pub struct LodWatch {
    /// Distance the camera has travelled so far (m).
    travelled: f64,
    eye: Option<Vec3>,
    /// (lod factor, field of view) the slacks were computed with.
    settings: Option<(f32, f32)>,
    /// (travel at which the object is due, object, stamp), soonest first.
    /// An entry whose stamp is no longer the object's is skipped.
    due: BinaryHeap<Reverse<(u64, u32, u32)>>,
    /// Stamp of the entry of each object; 0: not watched.
    stamp: Vec<u32>,
    next_stamp: u32,
    watched: usize,
}

impl LodWatch {
    /// The camera of this frame and the settings of the detail levels. A
    /// change of the settings makes every object due. Tells whether the
    /// camera moved, or the settings changed, since the last frame.
    pub fn advance(&mut self, eye: Vec3, lod_factor: f32, fov_y: f32) -> bool {
        let mut moved = false;
        if eye.is_finite() {
            if let Some(last) = self.eye {
                self.travelled += last.distance(eye) as f64;
                moved = last != eye;
            }
            self.eye = Some(eye);
        }
        let settings = Some((lod_factor, fov_y));
        if self.settings != settings {
            self.settings = settings;
            self.travelled += EVERYTHING;
            moved = true;
        }
        moved
    }

    /// Object `idx` was checked: it is due again once the camera has
    /// travelled `slack`.
    pub fn watch(&mut self, idx: usize, slack: f32) {
        if self.stamp.len() <= idx {
            self.stamp.resize(idx + 1, 0);
        }
        self.next_stamp = self.next_stamp.checked_add(1).unwrap_or(1);
        if std::mem::replace(&mut self.stamp[idx], self.next_stamp) == 0 {
            self.watched += 1;
        }
        let slack = if slack.is_nan() { 0.0 } else { slack.clamp(0.0, MAX_SLACK) };
        // (the bits of a positive float order as the float)
        let at = (self.travelled + slack as f64).to_bits();
        self.due.push(Reverse((at, idx as u32, self.next_stamp)));
        // entries left behind by objects checked again before their turn
        if self.due.len() > 2 * self.watched + 4096 {
            let stamp = &self.stamp;
            self.due.retain(|Reverse((_, idx, s))| stamp.get(*idx as usize) == Some(s));
        }
    }

    /// The object is checked another way from now on (it moves, the GPU
    /// places it), or is gone.
    pub fn forget(&mut self, idx: usize) {
        if let Some(s) = self.stamp.get_mut(idx)
            && std::mem::replace(s, 0) != 0
        {
            self.watched -= 1;
            if self.watched == 0 {
                // nothing but skipped entries left
                self.due.clear();
            }
        }
    }

    pub fn is_watched(&self, idx: usize) -> bool {
        self.stamp.get(idx).is_some_and(|&s| s != 0)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.watched
    }

    /// The next object the camera has travelled enough for, nearest to a
    /// change first; it is no longer watched (its check watches it again).
    pub fn pop_due(&mut self) -> Option<usize> {
        let now = self.travelled.to_bits();
        while let Some(&Reverse((at, idx, stamp))) = self.due.peek() {
            let idx = idx as usize;
            let current = self.stamp.get(idx) == Some(&stamp);
            // (not yet at the very distance of its slack: an object next
            // to a change of level waits for the camera to move at all)
            if current && at >= now {
                return None;
            }
            self.due.pop();
            if current {
                self.forget(idx);
                return Some(idx);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::lod_for;
    use super::*;

    /// Small deterministic generator (the tests need no crate for it).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / (1u64 << 24) as f32
        }
    }

    #[test]
    fn the_level_holds_within_the_slack() {
        let mut rng = Rng(7);
        let mut wide = 0;
        for case in 0..40_000 {
            let lod_factor = [0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 8.0, 12.0][case % 8];
            let fov_y = [0.2, 0.6, 1.047, 1.5, 2.6][case % 5];
            let radius = match case % 3 {
                0 => rng.next() * 0.2,
                1 => rng.next() * 3.0,
                _ => rng.next() * 60.0,
            };
            let distance = match case % 4 {
                0 => rng.next() * 2.0,
                1 => rng.next() * 30.0,
                _ => rng.next() * 400.0,
            };
            let level = lod_for(radius, distance, lod_factor, fov_y);
            let slack = lod_slack(radius, distance, lod_factor, fov_y);
            assert!(slack >= 0.0);
            if slack > 0.5 {
                wide += 1;
            }
            for k in 0..=24 {
                let d = (distance + slack * (k as f32 / 12.0 - 1.0)).max(0.0);
                assert_eq!(
                    lod_for(radius, d, lod_factor, fov_y),
                    level,
                    "radius {radius} distance {distance} slack {slack} at {d}, factor {lod_factor} fov {fov_y}"
                );
            }
        }
        // (not by answering 0 everywhere)
        assert!(wide > 20_000, "{wide}");
    }

    /// Next to each change of level, the slack leaves little unused: the
    /// level does change a little beyond it.
    #[test]
    fn the_slack_reaches_the_next_change_of_level() {
        let (radius, lod_factor, fov_y) = (1.0, 2.0, 1.047);
        // the changes of level of a 1 m object, by scanning
        let mut changes = Vec::new();
        let mut last = lod_for(radius, 0.0, lod_factor, fov_y);
        for i in 1..40_000 {
            let d = i as f32 * 0.005;
            let level = lod_for(radius, d, lod_factor, fov_y);
            if level != last {
                changes.push(d);
                last = level;
            }
        }
        assert_eq!(changes.len(), 3, "{changes:?}");
        for w in changes.windows(2) {
            let mid = (w[0] + w[1]) * 0.5;
            let slack = lod_slack(radius, mid, lod_factor, fov_y);
            let half = (w[1] - w[0]) * 0.5;
            assert!(slack <= half && slack > half - 0.1, "slack {slack} for a half band of {half}");
        }
        // beyond the last change: all the way back to it
        let slack = lod_slack(radius, changes[2] + 50.0, lod_factor, fov_y);
        assert!(slack <= 50.0 && slack > 49.9, "{slack}");
        // nothing to draw finer or coarser: an object without size
        assert_eq!(lod_slack(0.0, 10.0, lod_factor, fov_y), MAX_SLACK);
    }

    /// The whole of it: objects at rest, a camera that wanders, rests and
    /// jumps. With the due objects checked at each frame, every object has
    /// at every frame the level `lod_for` gives it, as if all of them were
    /// checked at every frame.
    #[test]
    fn levels_follow_the_camera_as_a_check_of_every_object_would() {
        let mut rng = Rng(11);
        let (lod_factor, fov_y) = (2.0, 1.047);
        let objects: Vec<(Vec3, f32)> = (0..400)
            .map(|_| {
                let p = Vec3::new(rng.next() * 256.0, rng.next() * 256.0, 20.0 + rng.next() * 10.0);
                (p, 0.05 + rng.next() * rng.next() * 12.0)
            })
            .collect();
        let mut eye = Vec3::new(128.0, 128.0, 25.0);
        let mut w = LodWatch::default();
        w.advance(eye, lod_factor, fov_y);
        let mut levels: Vec<u8> = Vec::new();
        for (idx, (p, r)) in objects.iter().enumerate() {
            let d = p.distance(eye);
            levels.push(lod_for(*r, d, lod_factor, fov_y));
            w.watch(idx, lod_slack(*r, d, lod_factor, fov_y));
        }
        let (mut checks, mut changes, mut resting) = (0, 0, 0);
        for frame in 0..3000 {
            // walks, flies, rests a while, and jumps across the region once
            let step = match (frame / 300) % 4 {
                0 => 0.05,
                1 => 0.4,
                2 => 0.0,
                _ => 0.01,
            };
            let turn = frame as f32 * 0.013;
            eye += Vec3::new(turn.cos(), turn.sin(), (turn * 0.7).sin() * 0.2) * step;
            if frame == 1500 {
                eye = Vec3::new(30.0, 220.0, 24.0);
            }
            w.advance(eye, lod_factor, fov_y);
            let before = checks;
            while let Some(idx) = w.pop_due() {
                let (p, r) = objects[idx];
                let d = p.distance(eye);
                let level = lod_for(r, d, lod_factor, fov_y);
                if level != levels[idx] {
                    changes += 1;
                    levels[idx] = level;
                }
                w.watch(idx, lod_slack(r, d, lod_factor, fov_y));
                checks += 1;
            }
            if step == 0.0 && frame != 1500 {
                assert_eq!(checks, before, "a camera at rest asks for nothing");
                resting += 1;
            }
            for (idx, (p, r)) in objects.iter().enumerate() {
                assert_eq!(
                    levels[idx],
                    lod_for(*r, p.distance(eye), lod_factor, fov_y),
                    "object {idx} at frame {frame}"
                );
            }
        }
        assert!(
            resting > 500 && changes > 100,
            "{resting} frames at rest, {changes} changes of level"
        );
        // fewer checks than one of every object every 24 frames, though
        // the camera moves in three frames out of four here
        assert!(checks < 3000 * objects.len() / 24 / 3, "{checks} checks");
    }

    #[test]
    fn objects_come_due_as_the_camera_travels() {
        let mut w = LodWatch::default();
        w.advance(Vec3::ZERO, 2.0, 1.0);
        // (the first settings made everything due: nothing was watched yet)
        w.watch(3, 5.0);
        w.watch(4, 1.0);
        w.watch(5, 2.5);
        assert_eq!(w.len(), 3);
        assert_eq!(w.pop_due(), None);
        // the camera at rest, or turning: nothing
        assert!(!w.advance(Vec3::ZERO, 2.0, 1.0));
        assert_eq!(w.pop_due(), None);
        // 1.5 m: the object 1 m from a change
        assert!(w.advance(Vec3::X * 1.5, 2.0, 1.0));
        assert_eq!(w.pop_due(), Some(4));
        assert_eq!(w.pop_due(), None);
        assert!(!w.is_watched(4) && w.is_watched(3));
        // checked: watched again from where the camera is now
        w.watch(4, 0.8);
        // back and forth counts as travel (never more than the true margin)
        w.advance(Vec3::X * 0.4, 2.0, 1.0);
        assert_eq!(w.pop_due(), Some(4));
        assert_eq!(w.pop_due(), Some(5));
        assert_eq!(w.pop_due(), None);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn new_settings_make_everything_due_once() {
        let mut w = LodWatch::default();
        w.advance(Vec3::ZERO, 2.0, 1.0);
        for idx in 0..10 {
            w.watch(idx, 50.0 + idx as f32);
        }
        w.watch(10, MAX_SLACK * 3.0);
        assert!(w.advance(Vec3::ZERO, 2.0, 0.9));
        let mut due: Vec<usize> = std::iter::from_fn(|| w.pop_due()).collect();
        // soonest first; the longest wait too
        assert_eq!(due, (0..=10).collect::<Vec<_>>());
        due.clear();
        w.advance(Vec3::ZERO, 2.0, 0.9);
        assert_eq!(w.pop_due(), None);
    }

    #[test]
    fn a_check_before_its_turn_replaces_the_wait() {
        let mut w = LodWatch::default();
        w.advance(Vec3::ZERO, 2.0, 1.0);
        w.watch(1, 1.0);
        w.watch(2, 1.0);
        // updated by the simulator: checked by the sync, now far from a change
        w.watch(1, 30.0);
        // removed, or taken by the GPU
        w.forget(2);
        w.forget(2);
        w.forget(99);
        assert_eq!(w.len(), 1);
        w.advance(Vec3::Y * 2.0, 2.0, 1.0);
        assert_eq!(w.pop_due(), None);
        assert!(w.is_watched(1) && !w.is_watched(2));
        w.watch(2, 0.0);
        w.advance(Vec3::Y * 2.5, 2.0, 1.0);
        assert_eq!(w.pop_due(), Some(2));
        assert_eq!(w.pop_due(), None);
        // emptied by removals alone: nothing left to skip
        w.forget(1);
        assert!(w.due.is_empty() && w.len() == 0);
    }

    #[test]
    fn skipped_entries_do_not_pile_up() {
        let mut w = LodWatch::default();
        w.advance(Vec3::ZERO, 2.0, 1.0);
        // objects checked at each frame while they are watched (a slice
        // that still visits them)
        for _ in 0..2000 {
            for idx in 0..8 {
                w.watch(idx, 10.0);
            }
        }
        assert_eq!(w.len(), 8);
        assert!(w.due.len() <= 2 * 8 + 4096 + 1, "{}", w.due.len());
        // a camera position that is not a number is ignored
        w.advance(Vec3::splat(f32::NAN), 2.0, 1.0);
        w.advance(Vec3::X * 20.0, 2.0, 1.0);
        let due: Vec<usize> = std::iter::from_fn(|| w.pop_due()).collect();
        assert_eq!(due.len(), 8);
    }
}
