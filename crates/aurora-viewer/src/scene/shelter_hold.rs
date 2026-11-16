//! What a move of its own keeps out of the rain's shelter map for a while
//! (#3642). The map is drawn from what stands still, and the lying snow is
//! laid on its surfaces: an object that moves in it lays its old shape under
//! the snow, in the map's squares. What reports a motion is left out by
//! `Object::has_motion`; a rotation a script steps (llSetRot on a timer)
//! reports none, so a change of the pose the object's own data gives
//! (`Object::position` and `rotation`, local to its parent or region) holds
//! it out for a while after.
//!
//! The pose is the object's own, not the drawn one: an origin shift (a
//! region crossed), or its parent moving, changes every drawn matrix and no
//! object's own pose (the parent's hold covers its prims,
//! `Scene::moves_for_shelter`). The hold follows the steps: an object that
//! moves again soon after holds twice that interval, so one a timer steps
//! stays out between its steps instead of coming back into the map (and
//! taking the snow) halfway between them.

use glam::{Quat, Vec3};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

/// The hold after a single move: longer than the map's refresh with the
/// shadows off (renderer.rs `SHELTER_REFRESH`, 1 s).
pub(super) const HOLD_MIN: Duration = Duration::from_secs(5);
/// The longest hold: an object stepped less often than every minute comes
/// back into the map between its steps.
pub(super) const HOLD_MAX: Duration = Duration::from_secs(120);

/// Below these an update gives the same pose (0.1 mm; about 0.06°, the
/// resolution of a unit quaternion's components in f32 near identity).
const SAME_POSITION_SQ: f32 = 1e-8;
const SAME_ROTATION_SQ: f32 = 2.5e-7;

/// One object's pose for the shelter map (`ObjGpu::shelter`): reset with
/// its slot, so that a new object in it starts with no hold.
#[derive(Default, Clone, Copy, Debug)]
pub struct ShelterPose {
    /// The pose its data gave at its last sync (none before its first).
    pose: Option<(Vec3, Quat)>,
    /// Its last move, and the hold it started.
    moved: Option<Instant>,
    hold: Duration,
    /// Its hold's end is in `Holds`.
    queued: bool,
}

/// What a hold's end found (`ShelterPose::due`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Due {
    /// Back into the map: its linkset is synced once more.
    Ended,
    /// Moved again since: its new end.
    Later(Instant),
    /// No hold of its own any more (its slot was reused): nothing to do.
    Stale,
}

impl ShelterPose {
    /// Its own pose at this sync. True when it moved and its hold's end
    /// has to be queued (none queued yet).
    pub(super) fn note(&mut self, position: Vec3, rotation: Quat, now: Instant) -> bool {
        let Some((p, r)) = self.pose.replace((position, rotation)) else {
            // its first sync: where it is, not a move
            return false;
        };
        let r = if r.dot(rotation) < 0.0 { -r } else { r };
        let turned = (glam::Vec4::from(r) - glam::Vec4::from(rotation)).length_squared();
        if p.distance_squared(position) <= SAME_POSITION_SQ && turned <= SAME_ROTATION_SQ {
            return false;
        }
        // moved again within the longest hold: as long as twice that
        // interval (a timer's steps), at least the single move's
        let since = self.moved.map(|at| now.saturating_duration_since(at));
        self.hold = match since {
            Some(since) if since < HOLD_MAX => (since * 2).clamp(HOLD_MIN, HOLD_MAX),
            _ => HOLD_MIN,
        }
        .max(self.remaining(now));
        self.moved = Some(now);
        !std::mem::replace(&mut self.queued, true)
    }

    /// Still out of the map for its own move.
    pub(super) fn held(&self, now: Instant) -> bool {
        self.remaining(now) > Duration::ZERO
    }

    fn remaining(&self, now: Instant) -> Duration {
        self.end().map_or(Duration::ZERO, |end| end.saturating_duration_since(now))
    }

    /// When its hold ends.
    pub(super) fn end(&self) -> Option<Instant> {
        self.moved.map(|at| at + self.hold)
    }

    /// Its queued end came at `now`.
    pub(super) fn due(&mut self, now: Instant) -> Due {
        if !self.queued {
            return Due::Stale;
        }
        match self.end() {
            Some(end) if end > now => Due::Later(end),
            _ => {
                self.queued = false;
                Due::Ended
            }
        }
    }
}

/// The ends of the objects' holds, soonest first: what comes back into the
/// map is found without a walk over every object.
#[derive(Default)]
pub(super) struct Holds {
    ends: BinaryHeap<Reverse<(Instant, usize)>>,
}

impl Holds {
    pub(super) fn push(&mut self, end: Instant, idx: usize) {
        self.ends.push(Reverse((end, idx)));
    }

    /// The next object whose queued end has come by `now`.
    pub(super) fn pop_due(&mut self, now: Instant) -> Option<usize> {
        let Reverse((end, _)) = self.ends.peek()?;
        if *end > now {
            return None;
        }
        self.ends.pop().map(|Reverse((_, idx))| idx)
    }

    pub(super) fn clear(&mut self) {
        self.ends.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.ends.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn at_rest(t0: Instant) -> ShelterPose {
        let mut s = ShelterPose::default();
        assert!(!s.note(Vec3::new(10.0, 20.0, 30.0), Quat::IDENTITY, t0));
        s
    }

    #[test]
    fn a_first_sync_and_the_same_pose_again_are_no_move() {
        let t0 = Instant::now();
        let mut s = at_rest(t0);
        assert!(!s.held(t0));
        // re-sent as it was (an update of something else, an origin shift
        // re-syncing everything: the own pose does not change), or the same
        // rotation as its opposite quaternion
        assert!(!s.note(Vec3::new(10.0, 20.0, 30.0), Quat::IDENTITY, t0 + S(1)));
        assert!(!s.note(Vec3::new(10.0, 20.0, 30.0), -Quat::IDENTITY, t0 + S(2)));
        assert!(!s.held(t0 + S(2)));
        assert_eq!(s.end(), None);
    }

    #[test]
    fn a_single_move_holds_the_shortest_while() {
        let t0 = Instant::now();
        let mut s = at_rest(t0);
        let turned = Quat::from_rotation_z(15f32.to_radians());
        assert!(s.note(Vec3::new(10.0, 20.0, 30.0), turned, t0 + S(1)), "queued");
        assert!(s.held(t0 + S(1)));
        assert_eq!(s.end(), Some(t0 + S(1) + HOLD_MIN));
        assert!(s.held(t0 + S(1) + HOLD_MIN - Duration::from_millis(1)));
        assert!(!s.held(t0 + S(1) + HOLD_MIN));
        // a move of a millimetre counts
        let mut s = at_rest(t0);
        assert!(s.note(Vec3::new(10.001, 20.0, 30.0), Quat::IDENTITY, t0 + S(1)));
    }

    #[test]
    fn a_timers_steps_keep_it_out_between_them() {
        // llSetRot every 10 s: from the second step on it is held twice that,
        // never back in the map halfway (it was 5 s: in for 5 s of every 10)
        let t0 = Instant::now();
        let mut s = at_rest(t0);
        let mut queued = 0;
        for step in 1..=6u64 {
            let at = t0 + S(10 * step);
            queued += usize::from(s.note(Vec3::new(10.0, 20.0, 30.0), Quat::from_rotation_z(step as f32 * 0.26), at));
            if step >= 2 {
                assert!(s.held(at + S(10) - Duration::from_millis(1)), "step {step}");
            }
        }
        // one end queued at a time (no due end taken off here)
        assert_eq!(queued, 1);
        // a timer slower than the longest hold comes back between its steps
        let mut s = at_rest(t0);
        s.note(Vec3::X, Quat::IDENTITY, t0 + S(1));
        s.note(Vec3::Y, Quat::IDENTITY, t0 + S(1) + HOLD_MAX);
        assert_eq!(s.end(), Some(t0 + S(1) + HOLD_MAX + HOLD_MIN));
        // a quick move never shortens the hold already running
        let mut s = at_rest(t0);
        s.note(Vec3::X, Quat::IDENTITY, t0 + S(10));
        s.note(Vec3::Y, Quat::IDENTITY, t0 + S(40));
        let end = s.end().expect("held");
        s.note(Vec3::Z, Quat::IDENTITY, t0 + S(41));
        assert!(s.end().expect("held") >= end);
    }

    #[test]
    fn a_hold_ends_once_and_moves_its_end_when_it_moved_again() {
        let t0 = Instant::now();
        let mut holds = Holds::default();
        let mut s = at_rest(t0);
        assert!(s.note(Vec3::X, Quat::IDENTITY, t0));
        holds.push(s.end().expect("held"), 7);
        assert_eq!(holds.pop_due(t0 + S(1)), None);
        // moved again meanwhile: not queued twice, its end is later
        assert!(!s.note(Vec3::Y, Quat::IDENTITY, t0 + S(3)));
        assert_eq!(holds.pop_due(t0 + HOLD_MIN), Some(7));
        let Due::Later(end) = s.due(t0 + HOLD_MIN) else {
            panic!("moved again");
        };
        holds.push(end, 7);
        assert_eq!(holds.pop_due(end), Some(7));
        assert_eq!(s.due(end), Due::Ended);
        assert!(!s.held(end));
        assert_eq!(holds.len(), 0);
        // a slot reused since: its new object has no hold of its own
        holds.push(end, 7);
        let mut fresh = ShelterPose::default();
        assert_eq!(holds.pop_due(end), Some(7));
        assert_eq!(fresh.due(end), Due::Stale);
    }

    #[test]
    fn the_soonest_end_comes_first() {
        let t0 = Instant::now();
        let mut holds = Holds::default();
        holds.push(t0 + S(9), 1);
        holds.push(t0 + S(3), 2);
        holds.push(t0 + S(6), 3);
        let due: Vec<usize> = std::iter::from_fn(|| holds.pop_due(t0 + S(7))).collect();
        assert_eq!(due, [2, 3]);
        holds.clear();
        assert_eq!(holds.pop_due(t0 + S(60)), None);
    }
}
