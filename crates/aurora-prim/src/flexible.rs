//! Local flexible-prim physics, port of LLVolumeImplFlexible::doFlexibleUpdate
//! and remapSections (Firestorm indra/newview/llflexibleobject.cpp, LGPL 2.1).
//! The anchor is the negative Z end; forces and inertia act in world space.

use crate::extra::FlexibleParams;
use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug)]
struct Section {
    position: Vec3,
    velocity: Vec3,
    direction: Vec3,
    rotation: Quat,
}

/// A sampled sweep frame, in the object's unscaled local space (metres).
#[derive(Clone, Copy, Debug)]
pub struct FlexiblePoint {
    pub position: Vec3,
    pub rotation: Quat,
}

pub struct Flexible {
    sections: Vec<Section>,
    length: f32,
}

impl Flexible {
    pub fn new(softness: u8, length: f32, position: Vec3, rotation: Quat) -> Self {
        let length = finite(length, 1.0).clamp(0.001, 64.0);
        let n = 1usize << softness.min(3);
        let direction = rotation * Vec3::Z;
        let sections = (0..=n)
            .map(|i| Section {
                position: position + direction * (length * (i as f32 / n as f32 - 0.5)),
                velocity: Vec3::ZERO,
                direction,
                rotation,
            })
            .collect();
        Self { sections, length }
    }

    /// Keep inertia when the render origin moves to another region.
    pub fn shift(&mut self, delta: Vec3) {
        for s in &mut self.sections {
            s.position += delta;
        }
    }

    pub fn matches(&self, softness: u8, length: f32) -> bool {
        self.sections.len() == (1usize << softness.min(3)) + 1 && (self.length - length).abs() < 1e-5
    }

    /// Long frames are bounded as in Firestorm, and split to keep the length
    /// and angle constraints stable. Velocity is stored in metres/second so
    /// inertia does not depend on the display refresh rate.
    pub fn update(&mut self, params: FlexibleParams, position: Vec3, rotation: Quat, dt: f32, wind: impl Fn(Vec3) -> Vec3) {
        let dt = finite(dt, 0.0).clamp(0.0, 0.2);
        if dt == 0.0 || !position.is_finite() || !rotation.is_finite() {
            return;
        }
        let steps = (dt * 60.0).ceil().max(1.0) as u32;
        let dt = dt / steps as f32;
        for _ in 0..steps {
            self.step(params, position, rotation, dt, &wind);
        }
    }

    fn step(&mut self, p: FlexibleParams, position: Vec3, rotation: Quat, dt: f32, wind: &impl Fn(Vec3) -> Vec3) {
        let n = self.sections.len() - 1;
        let length = self.length / n as f32;
        let direction = rotation * Vec3::Z;
        self.sections[0] = Section {
            position: position - direction * (self.length * 0.5),
            velocity: Vec3::ZERO,
            direction,
            rotation,
        };
        let tension = (finite(p.tension, 1.0).clamp(0.0, 10.0) * 0.1 * (1.0 - 0.85f32.powf(dt * 30.0))).min(0.99);
        let momentum = 10.0f32.powf(-(finite(p.air_friction, 2.0).clamp(0.0, 10.0) * 2.0 + 1.0) * dt);
        let wind_factor = finite(p.wind_sensitivity, 0.0).clamp(0.0, 10.0) * 0.1 * length * dt;
        let gravity = finite(p.gravity, 0.0).clamp(-10.0, 10.0);
        let force = if p.user_force.is_finite() { p.user_force } else { Vec3::ZERO };
        let max_angle = (length * 2.0).atan();
        let mut parent_rotation = rotation;
        for i in 1..=n {
            let parent = self.sections[i - 1];
            let previous_direction = self.sections[i.saturating_sub(2)].direction;
            let s = &mut self.sections[i];
            let old = s.position;
            s.position += (force - Vec3::Z * gravity) * (length * dt);
            if wind_factor > 0.00001 {
                let w = wind(s.position);
                if w.is_finite() {
                    s.position += w * wind_factor;
                }
            }
            s.position += (previous_direction * length - (s.position - parent.position)) * tension;
            s.position += s.velocity * (dt * momentum);
            let target = (s.position - parent.position).normalize_or(parent.direction);
            let delta = Quat::from_rotation_arc(parent.direction, target);
            let (axis, angle) = delta.to_axis_angle();
            let clamped = Quat::from_axis_angle(axis, angle.min(max_angle));
            s.direction = (clamped * parent.direction).normalize_or(parent.direction);
            s.position = parent.position + s.direction * length;
            parent_rotation = (clamped * parent_rotation).normalize();
            s.rotation = parent_rotation;
            // Firestorm limits the displacement carried to the next frame
            // to one metre (at its reference 60 Hz).
            s.velocity = ((s.position - old) / dt).clamp_length_max(60.0);
            if i > 1 {
                self.sections[i - 1].rotation = (Quat::from_axis_angle(axis, angle * 0.5) * parent.rotation).normalize();
            }
        }
    }

    /// Eight render sections, with Firestorm's cubic Hermite remapping when
    /// softness uses fewer simulation sections. Detail never reduces the
    /// physical simulation or makes nearby flexies step at different rates.
    pub fn path(&self, position: Vec3, rotation: Quat) -> [FlexiblePoint; 9] {
        let n = self.sections.len() - 1;
        let length = self.length / n as f32;
        let derivative = |i: usize| {
            if i == 0 {
                (self.sections[1].position - self.sections[0].position) / length
            } else if i == n {
                (self.sections[n].position - self.sections[n - 1].position) / length
            } else {
                (self.sections[i + 1].position - self.sections[i - 1].position) / (2.0 * length)
            }
        };
        let inv = rotation.conjugate();
        std::array::from_fn(|i| {
            let t = i as f32 * n as f32 / 8.0;
            let k = (t as usize).min(n - 1);
            let u = t - k as f32;
            let a = self.sections[k];
            let b = self.sections[k + 1];
            let c = derivative(k) * length;
            let y = derivative(k + 1) * length - c;
            let x = b.position - a.position - c;
            let aa = y - x * 2.0;
            let bb = x - aa;
            FlexiblePoint {
                position: inv * ((u * u * (u * aa + bb) + u * c + a.position) - position),
                rotation: (inv * a.rotation.slerp(b.rotation, u)).normalize(),
            }
        })
    }
}

fn finite(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> FlexibleParams {
        FlexibleParams {
            softness: 2,
            tension: 1.0,
            air_friction: 2.0,
            gravity: 3.0,
            wind_sensitivity: 0.0,
            user_force: Vec3::ZERO,
        }
    }

    #[test]
    fn gravity_bends_but_preserves_anchor_and_segment_lengths() {
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let pos = Vec3::new(10.0, 20.0, 30.0);
        let mut f = Flexible::new(2, 4.0, pos, rot);
        for _ in 0..240 {
            f.update(params(), pos, rot, 1.0 / 60.0, |_| Vec3::ZERO);
        }
        assert!(f.sections[4].position.z < pos.z - 0.5);
        assert!((f.sections[0].position - (pos - rot * Vec3::Z * 2.0)).length() < 1e-5);
        for pair in f.sections.windows(2) {
            assert!(((pair[1].position - pair[0].position).length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn first_step_matches_firestorm_reference_and_tension_stiffens_the_chain() {
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut f = Flexible::new(0, 4.0, Vec3::ZERO, rot);
        f.update(params(), Vec3::ZERO, rot, 1.0 / 60.0, |_| Vec3::ZERO);
        // LLVolumeImplFlexible at 60 Hz: section length 4, gravity 3,
        // tension 1; initial velocity zero, angle clamp not reached.
        let expected = Vec3::new(1.995_086_8, 0.0, -0.198_195_35);
        assert!((f.sections[1].position - expected).length() < 1e-5);
        let tip = |tension: f32| {
            let mut f = Flexible::new(2, 4.0, Vec3::ZERO, rot);
            let mut p = params();
            p.tension = tension;
            for _ in 0..240 {
                f.update(p, Vec3::ZERO, rot, 1.0 / 60.0, |_| Vec3::ZERO);
            }
            f.sections[4].position.z
        };
        assert!(tip(10.0) > tip(0.0) + 0.1, "tension resists gravity");
    }

    #[test]
    fn wind_force_moving_parent_and_origin_shift() {
        let mut p = params();
        p.gravity = 0.0;
        p.wind_sensitivity = 10.0;
        let mut f = Flexible::new(2, 4.0, Vec3::ZERO, Quat::IDENTITY);
        for _ in 0..120 {
            f.update(p, Vec3::ZERO, Quat::IDENTITY, 1.0 / 60.0, |_| Vec3::X * 3.0);
        }
        assert!(f.sections[4].position.x > 0.5);
        p.wind_sensitivity = 0.0;
        p.user_force = -Vec3::Y * 5.0;
        for _ in 0..120 {
            f.update(p, Vec3::ZERO, Quat::IDENTITY, 1.0 / 60.0, |_| Vec3::ZERO);
        }
        assert!(f.sections[4].position.y < -0.5);
        let before = f.path(Vec3::ZERO, Quat::IDENTITY);
        let shift = Vec3::new(-256.0, 256.0, 0.0);
        f.shift(shift);
        let after = f.path(shift, Quat::IDENTITY);
        for (a, b) in before.iter().zip(after) {
            assert!((a.position - b.position).length() < 0.0001);
        }
        let tip = f.sections[4].position;
        f.update(p, shift + Vec3::X, Quat::IDENTITY, 1.0 / 60.0, |_| Vec3::ZERO);
        assert!(
            f.sections[4].position.distance(tip + Vec3::X) > 0.1,
            "tip keeps inertia instead of following rigidly"
        );
    }

    #[test]
    fn all_softness_levels_and_bad_external_values_stay_finite() {
        for softness in [0, 1, 2, 3, 255] {
            let mut f = Flexible::new(softness, 4.0, Vec3::ZERO, Quat::IDENTITY);
            let mut p = params();
            p.gravity = f32::NAN;
            p.user_force.x = f32::INFINITY;
            f.update(p, Vec3::ZERO, Quat::IDENTITY, 10.0, |_| Vec3::splat(f32::NAN));
            for s in f.path(Vec3::ZERO, Quat::IDENTITY) {
                assert!(s.position.is_finite() && s.rotation.is_finite());
            }
        }
    }
}
