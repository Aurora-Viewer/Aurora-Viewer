//! Our own transform edits against the simulator's late reports of them
//! (issue #1193).
//!
//! Stretching streams MultipleObjectUpdate ten times a second
//! (LLManipScale::sendUpdates, indra/newview/llmanipscale.cpp, originally
//! LGPL 2.1), and the simulator answers each send with a report of the
//! object, a round trip later. Once the mouse is released, the reports of
//! the earlier sends still arrive: they put the object back at a size and a
//! place it already left, then the report of the last send brings it to its
//! final size, and the object shakes. The Legacy Viewer applies every report
//! (LLViewerObject::processUpdateMessage). Here, the values last sent for an
//! object are kept until a report agrees with them: the reports of earlier
//! sends are not shown. A simulator that refuses or limits the edit never
//! agrees: its latest report wins after `ECHO_TIMEOUT`.

use super::objects::ObjKey;
use crate::fast_hash::FastMap;
use glam::{Quat, Vec3};
use std::time::{Duration, Instant};

/// How long the values sent wait for a report that agrees with them: far
/// above the round trip of a busy region, short enough that a refused edit
/// soon shows the simulator's values.
const ECHO_TIMEOUT: Duration = Duration::from_secs(2);
/// A report this close to what was sent answers it. Positions and scales
/// travel as floats, but the simulator recomputes those of the prims of a
/// stretched linkset itself; rotations of a terse update are quantised.
const DISTANCE_EPSILON: f32 = 1e-3;
const RELATIVE_EPSILON: f32 = 1e-4;
const ROTATION_DOT: f32 = 0.99999;

/// Position, rotation and scale of an object, each when known or sent.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Transform {
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
}

struct Sent {
    values: Transform,
    at: Instant,
    /// What the reports that did not agree said last: the simulator's
    /// values when the wait ends without an answer.
    late: Transform,
}

/// The transforms sent and not answered yet, by object.
#[derive(Default)]
pub struct SentEdits {
    sent: FastMap<ObjKey, Sent>,
}

impl SentEdits {
    /// `values` were sent for `key` at `now`: they replace what was sent
    /// before (whose reports are now late).
    pub fn note(&mut self, key: ObjKey, values: Transform, now: Instant) {
        if values == Transform::default() {
            return;
        }
        let late = self.sent.remove(&key).map(|s| s.late).unwrap_or_default();
        self.sent.insert(key, Sent { values, at: now, late });
    }

    /// The object's local coordinates no longer mean what was sent (new
    /// parent, removed).
    pub fn forget(&mut self, key: &ObjKey) {
        self.sent.remove(key);
    }

    /// A report put `key` at `reported` (`scale` None in a terse update).
    /// Returns the values sent to keep instead, when the report answers an
    /// earlier send; None to apply it as it is.
    pub fn filter(&mut self, key: ObjKey, reported: Transform, now: Instant) -> Option<Transform> {
        let sent = self.sent.get_mut(&key)?;
        if now.saturating_duration_since(sent.at) >= ECHO_TIMEOUT {
            self.sent.remove(&key);
            return None;
        }
        let v = sent.values;
        let position_ok = agree(v.position, reported.position, near_vec);
        let rotation_ok = agree(v.rotation, reported.rotation, near_quat);
        // a terse update carries no scale: it cannot answer a sent scale
        let scale_ok = match (v.scale, reported.scale) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(a), Some(b)) => near_vec(a, b),
        };
        if position_ok && rotation_ok && scale_ok {
            self.sent.remove(&key);
            return None;
        }
        let late = &mut sent.late;
        late.position = reported.position.or(late.position);
        late.rotation = reported.rotation.or(late.rotation);
        late.scale = reported.scale.or(late.scale);
        Some(v)
    }

    /// End the waits that got no answer: the objects and the last values
    /// their simulator reported, to show now.
    pub fn expire(&mut self, now: Instant) -> Vec<(ObjKey, Transform)> {
        if self.sent.is_empty() {
            return Vec::new();
        }
        let mut late = Vec::new();
        self.sent.retain(|key, s| {
            if now.saturating_duration_since(s.at) < ECHO_TIMEOUT {
                return true;
            }
            if s.late != Transform::default() {
                late.push((*key, s.late));
            }
            false
        });
        late
    }
}

fn agree<T: Copy>(sent: Option<T>, reported: Option<T>, near: fn(T, T) -> bool) -> bool {
    match (sent, reported) {
        (Some(a), Some(b)) => near(a, b),
        _ => true,
    }
}

fn near_vec(a: Vec3, b: Vec3) -> bool {
    let tolerance = DISTANCE_EPSILON + RELATIVE_EPSILON * a.abs().max(b.abs()).max_element();
    (a - b).abs().max_element() <= tolerance
}

fn near_quat(a: Quat, b: Quat) -> bool {
    a.dot(b).abs() >= ROTATION_DOT
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ObjKey = ObjKey { region: 1, local_id: 7 };

    fn stretch(size: f32) -> Transform {
        Transform {
            position: Some(Vec3::new(10.0, 10.0, 20.0 + size * 0.5)),
            rotation: None,
            scale: Some(Vec3::splat(size)),
        }
    }

    fn report(size: f32) -> Transform {
        Transform {
            rotation: Some(Quat::IDENTITY),
            ..stretch(size)
        }
    }

    #[test]
    fn reports_of_earlier_sends_keep_the_last_values_until_one_answers_them() {
        let mut edits = SentEdits::default();
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        // streamed while stretching, then the last values on release
        edits.note(KEY, stretch(1.5), t0);
        edits.note(KEY, stretch(2.0), ms(100));
        edits.note(KEY, stretch(3.0), ms(150));
        assert_eq!(edits.filter(KEY, report(1.5), ms(200)), Some(stretch(3.0)));
        assert_eq!(edits.filter(KEY, report(2.0), ms(260)), Some(stretch(3.0)));
        // the answer to the last send, within the float noise of the trip
        assert_eq!(edits.filter(KEY, report(3.0 + 2e-4), ms(320)), None);
        // nothing waits any more: whatever comes next is shown
        assert_eq!(edits.filter(KEY, report(1.0), ms(400)), None);
        assert!(edits.expire(ms(5000)).is_empty());
    }

    #[test]
    fn a_terse_update_cannot_answer_a_scale() {
        let mut edits = SentEdits::default();
        let t0 = Instant::now();
        edits.note(KEY, stretch(3.0), t0);
        let terse = Transform {
            scale: None,
            ..report(3.0)
        };
        assert_eq!(edits.filter(KEY, terse, t0), Some(stretch(3.0)));
        assert_eq!(edits.filter(KEY, report(3.0), t0), None);
        // a move alone is answered by the position of a terse update
        edits.note(
            KEY,
            Transform {
                position: Some(Vec3::ONE),
                ..Default::default()
            },
            t0,
        );
        let moved = Transform {
            position: Some(Vec3::ONE),
            rotation: Some(Quat::from_rotation_z(0.3)),
            scale: None,
        };
        assert_eq!(edits.filter(KEY, moved, t0), None);
    }

    #[test]
    fn a_refused_edit_shows_the_simulators_values_once_the_wait_ends() {
        let mut edits = SentEdits::default();
        let t0 = Instant::now();
        edits.note(KEY, stretch(3.0), t0);
        assert!(edits.filter(KEY, report(1.0), t0 + Duration::from_millis(100)).is_some());
        assert!(edits.expire(t0 + Duration::from_millis(1900)).is_empty());
        assert_eq!(edits.expire(t0 + ECHO_TIMEOUT), vec![(KEY, report(1.0))]);
        // a report after the wait is applied as it is
        edits.note(KEY, stretch(3.0), t0);
        assert_eq!(edits.filter(KEY, report(1.0), t0 + ECHO_TIMEOUT), None);
        // nothing reported: nothing to show at the end
        edits.note(KEY, stretch(3.0), t0);
        assert!(edits.expire(t0 + ECHO_TIMEOUT).is_empty());
    }

    #[test]
    fn rotations_agree_whatever_their_sign_and_quantisation() {
        let mut edits = SentEdits::default();
        let t0 = Instant::now();
        let r = Quat::from_rotation_z(1.0);
        let sent = Transform {
            position: Some(Vec3::ONE),
            rotation: Some(r),
            scale: None,
        };
        edits.note(KEY, sent, t0);
        let off = Transform {
            rotation: Some(Quat::from_rotation_z(1.05)),
            ..sent
        };
        assert_eq!(edits.filter(KEY, off, t0), Some(sent));
        let quantised = Quat::from_xyzw(-r.x, -r.y, -r.z + 1.0 / 32767.0, -r.w).normalize();
        let back = Transform {
            rotation: Some(quantised),
            ..sent
        };
        assert_eq!(edits.filter(KEY, back, t0), None);
    }

    #[test]
    fn forgetting_or_sending_nothing_waits_for_nothing() {
        let mut edits = SentEdits::default();
        let t0 = Instant::now();
        edits.note(KEY, Transform::default(), t0);
        assert_eq!(edits.filter(KEY, report(1.0), t0), None);
        edits.note(KEY, stretch(3.0), t0);
        edits.forget(&KEY);
        assert_eq!(edits.filter(KEY, report(1.0), t0), None);
    }
}
