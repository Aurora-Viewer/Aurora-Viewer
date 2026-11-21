//! Local render-only ride of floating objects on the wind's sea: a heave
//! and a tilt (pitch and roll) per linkset root, eased like our avatar's
//! surface float (water_float.rs). The simulator's positions stay flat, as
//! its water is; only the drawing rides the waves, and the linkset and who
//! sits on it follow their root; an avatar standing on its deck is drawn
//! carried by it. A ride eases in and out (off the water, selected in
//! Edit) instead of snapping. Which objects float, and the sea under their
//! hull, are the scene's (scene/water_ride.rs).

use glam::{Quat, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

/// The most a floating object is lifted or lowered (m), and tilted (rad).
const MAX_HEAVE: f32 = 6.0;
const MAX_TILT: f32 = 0.4;
/// How fast the drawing follows the sea (s): a hull damps the quickest
/// ripples, and a change of state (a wave appearing) never jumps.
const HEAVE_EASE: f32 = 0.18;
const TILT_EASE: f32 = 0.3;

#[derive(Debug, Clone, Copy)]
struct Ride {
    dz: f32,
    tilt: Quat,
    seen: f32,
}

/// The rides of the floating objects, by full id.
#[derive(Debug, Default)]
pub struct Rides {
    rides: HashMap<Uuid, Ride>,
    /// The roots that may float (object indices) with their hull (the
    /// linkset's box in the root's frame: centre and half size), and when
    /// they were last looked for.
    pub candidates: Vec<(usize, Vec3, Vec3)>,
    pub scan_at: f32,
    /// Avatars standing on a riding deck: how far it lifts their feet (m),
    /// eased, by full id.
    standing: HashMap<Uuid, (f32, f32)>,
}

/// A ride this close to flat has stopped (m, rad).
const SETTLED: f32 = 0.003;
/// How fast an avatar's feet follow the deck it stands on (s).
const STAND_EASE: f32 = 0.12;

impl Rides {
    /// Heave and tilt (world frame) to draw `id`'s root with, if it rides.
    pub fn pose(&self, id: &Uuid) -> Option<(f32, Quat)> {
        self.rides.get(id).map(|r| (r.dz, r.tilt))
    }

    /// Move `id`'s ride toward the sea's `heave` (m) and `tilt` at `time`;
    /// returns whether it just started riding.
    pub fn follow(&mut self, id: Uuid, time: f32, heave: f32, tilt: Quat) -> bool {
        let heave = if heave.is_finite() {
            heave.clamp(-MAX_HEAVE, MAX_HEAVE)
        } else {
            0.0
        };
        let tilt = clamp_tilt(if tilt.is_finite() { tilt } else { Quat::IDENTITY });
        match self.rides.get_mut(&id) {
            Some(r) => {
                let dt = (time - r.seen).clamp(0.0, 0.25);
                r.seen = time;
                r.dz += (heave - r.dz) * (1.0 - (-dt / HEAVE_EASE).exp());
                r.tilt = r.tilt.slerp(tilt, 1.0 - (-dt / TILT_EASE).exp()).normalize();
                false
            }
            None => {
                // it starts flat and eases into the sea
                self.rides.insert(
                    id,
                    Ride {
                        dz: 0.0,
                        tilt: Quat::IDENTITY,
                        seen: time,
                    },
                );
                true
            }
        }
    }

    /// Ease the rides not in `keep` back to flat at `time` (an object out
    /// of the water, sunk, or selected in Edit); returns the ids that have
    /// settled and stopped riding.
    pub fn release(&mut self, keep: &[Uuid], time: f32) -> Vec<Uuid> {
        let leaving: Vec<Uuid> = self.rides.keys().filter(|id| !keep.contains(id)).copied().collect();
        let mut gone = Vec::new();
        for id in leaving {
            self.follow(id, time, 0.0, Quat::IDENTITY);
            if let Some(r) = self.rides.get(&id)
                && r.dz.abs() < SETTLED
                && r.tilt.angle_between(Quat::IDENTITY) < SETTLED
            {
                self.rides.remove(&id);
                gone.push(id);
            }
        }
        gone
    }

    /// Stop every ride at once (the waves turned off, the world left).
    pub fn clear(&mut self) -> Vec<Uuid> {
        self.candidates.clear();
        self.standing.clear();
        self.rides.drain().map(|(id, _)| id).collect()
    }

    /// How far the deck an avatar stands on lifts it now (0: none).
    pub fn standing(&self, id: &Uuid) -> f32 {
        self.standing.get(id).map_or(0.0, |s| s.0)
    }

    /// The avatars standing on a riding deck at `time` with how far it
    /// lifts their feet; the others step off it eased.
    pub fn stand(&mut self, on: &[(Uuid, f32)], time: f32) {
        for &(id, dz) in on {
            let dz = if dz.is_finite() {
                dz.clamp(-MAX_HEAVE * 2.0, MAX_HEAVE * 2.0)
            } else {
                0.0
            };
            let s = self.standing.entry(id).or_insert((0.0, time));
            let dt = (time - s.1).clamp(0.0, 0.25);
            s.0 += (dz - s.0) * (1.0 - (-dt / STAND_EASE).exp());
            s.1 = time;
        }
        self.standing.retain(|id, s| {
            if on.iter().any(|(o, _)| o == id) {
                return true;
            }
            let dt = (time - s.1).clamp(0.0, 0.25);
            s.0 -= s.0 * (1.0 - (-dt / STAND_EASE).exp());
            s.1 = time;
            s.0.abs() >= SETTLED
        });
    }
}

/// The tilt of a hull whose bow (`forward`, horizontal unit, `length` m from
/// stern to bow) sits `pitch_rise` higher than its stern, and whose port
/// side (`side`, `beam` m across) sits `roll_rise` higher than starboard.
pub fn hull_tilt(forward: Vec3, side: Vec3, length: f32, beam: f32, pitch_rise: f32, roll_rise: f32) -> Quat {
    let pitch = pitch_rise.atan2(length.max(0.1));
    let roll = roll_rise.atan2(beam.max(0.1));
    // turning about the side axis by −pitch lifts the bow; about the
    // forward axis by +roll lifts the port side
    Quat::from_axis_angle(side, -pitch) * Quat::from_axis_angle(forward, roll)
}

fn clamp_tilt(q: Quat) -> Quat {
    let (axis, angle) = q.to_axis_angle();
    if angle.abs() <= MAX_TILT || !axis.is_finite() {
        return q;
    }
    Quat::from_axis_angle(axis, angle.signum() * MAX_TILT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_step_on_and_off_a_deck_eased() {
        let mut rides = Rides::default();
        let id = Uuid::from_u128(9);
        for i in 0..60 {
            rides.stand(&[(id, 0.7)], i as f32 / 60.0);
        }
        assert!((rides.standing(&id) - 0.7).abs() < 0.01);
        rides.stand(&[], 1.0 + 1.0 / 60.0);
        assert!(rides.standing(&id) > 0.4, "no snap off the deck");
        for i in 2..120 {
            rides.stand(&[], 1.0 + i as f32 / 60.0);
        }
        assert_eq!(rides.standing(&id), 0.0);
    }

    #[test]
    fn a_bow_on_a_crest_points_up_and_a_port_side_rolls_up() {
        let tilt = hull_tilt(Vec3::X, Vec3::Y, 8.0, 3.0, 1.0, 0.0);
        assert!((tilt * Vec3::X).z > 0.1, "bow up: {:?}", tilt * Vec3::X);
        let tilt = hull_tilt(Vec3::X, Vec3::Y, 8.0, 3.0, 0.0, 0.5);
        assert!((tilt * Vec3::Y).z > 0.1, "port up: {:?}", tilt * Vec3::Y);
        // a boat heading north: its bow is +y
        let tilt = hull_tilt(Vec3::Y, -Vec3::X, 8.0, 3.0, 1.0, 0.0);
        assert!((tilt * Vec3::Y).z > 0.1, "north bow up: {:?}", tilt * Vec3::Y);
    }

    #[test]
    fn rides_start_flat_ease_in_and_stay_bounded() {
        let mut rides = Rides::default();
        let id = Uuid::from_u128(7);
        assert!(rides.follow(id, 0.0, 0.8, Quat::from_rotation_x(0.2)));
        assert_eq!(rides.pose(&id), Some((0.0, Quat::IDENTITY)));
        for i in 1..120 {
            assert!(!rides.follow(id, i as f32 / 60.0, 0.8, Quat::from_rotation_x(0.2)));
        }
        let (dz, tilt) = rides.pose(&id).expect("riding");
        assert!((dz - 0.8).abs() < 0.01, "{dz}");
        assert!((tilt.to_axis_angle().1 - 0.2).abs() < 0.01);
        rides.follow(id, 2.1, 9.0, Quat::from_rotation_x(1.5));
        for i in 0..240 {
            rides.follow(id, 2.1 + i as f32 / 60.0, 9.0, Quat::from_rotation_x(1.5));
        }
        let (dz, tilt) = rides.pose(&id).expect("riding");
        assert!(dz <= MAX_HEAVE + 1e-4 && tilt.to_axis_angle().1 <= MAX_TILT + 1e-3);
        // leaving the water: it eases back to flat, then stops
        let mut t = 2.1 + 240.0 / 60.0;
        assert!(rides.release(&[], t).is_empty(), "no snap");
        let (dz, _) = rides.pose(&id).expect("still easing");
        assert!(dz > 1.0, "{dz}");
        let mut gone = Vec::new();
        for _ in 0..240 {
            t += 1.0 / 60.0;
            gone = rides.release(&[], t);
            if !gone.is_empty() {
                break;
            }
        }
        assert_eq!(gone, vec![id]);
        assert!(rides.pose(&id).is_none());
    }
}
