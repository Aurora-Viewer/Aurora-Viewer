//! Local flexible-prim physics, port of LLVolumeImplFlexible::doFlexibleUpdate
//! and remapSections (Firestorm indra/newview/llflexibleobject.cpp, LGPL 2.1).
//! The anchor is the negative Z end; forces and inertia act in world space.

use crate::extra::FlexibleParams;
use glam::{Quat, Vec3};

// LLVolumeImplFlexible::updateClass / SEC_PER_FLEXI_FRAME. The legacy
// displacement-based forces are calibrated for this cadence, not display FPS.
const STEP_SECONDS: f64 = 1.0 / 60.0;

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
    previous: Vec<Section>,
    length: f32,
    remainder: f64,
    frame_position: Vec3,
    frame_rotation: Quat,
}

impl Flexible {
    pub fn new(softness: u8, length: f32, position: Vec3, rotation: Quat) -> Self {
        let length = finite(length, 1.0).clamp(0.001, 64.0);
        let n = 1usize << softness.min(3);
        let direction = rotation * Vec3::Z;
        let sections: Vec<_> = (0..=n)
            .map(|i| Section {
                position: position + direction * (length * (i as f32 / n as f32 - 0.5)),
                velocity: Vec3::ZERO,
                direction,
                rotation,
            })
            .collect();
        Self {
            previous: sections.clone(),
            sections,
            length,
            remainder: 0.0,
            frame_position: position,
            frame_rotation: rotation,
        }
    }

    /// Keep inertia when the render origin moves to another region.
    pub fn shift(&mut self, delta: Vec3) {
        for s in self.sections.iter_mut().chain(&mut self.previous) {
            s.position += delta;
        }
        self.frame_position += delta;
    }

    pub fn matches(&self, softness: u8, length: f32) -> bool {
        self.sections.len() == (1usize << softness.min(3)) + 1 && (self.length - finite(length, 1.0).clamp(0.001, 64.0)).abs() < 1e-5
    }

    /// LLVolumeImplFlexible::onSetScale / remapSections: keep the bent
    /// chain and its inertia while changing size or simulation resolution.
    pub fn reconfigure(&mut self, softness: u8, length: f32, position: Vec3, rotation: Quat) {
        if self.matches(softness, length) || !position.is_finite() || !rotation.is_finite() {
            return;
        }
        let samples = self.path(Vec3::ZERO, Quat::IDENTITY);
        let old = &self.sections;
        let old_n = old.len() - 1;
        let n = 1usize << softness.min(3);
        let length = finite(length, 1.0).clamp(0.001, 64.0);
        let mut sections = Vec::with_capacity(n + 1);
        sections.push(Section {
            position: position - rotation * Vec3::Z * (length * 0.5),
            velocity: Vec3::ZERO,
            direction: rotation * Vec3::Z,
            rotation,
        });
        for i in 1..=n {
            let parent = sections[i - 1];
            let (a, b) = (samples[(i - 1) * 8 / n], samples[i * 8 / n]);
            let direction = (b.position - a.position).normalize_or(parent.direction);
            let t = i as f32 * old_n as f32 / n as f32;
            let k = (t as usize).min(old_n - 1);
            sections.push(Section {
                position: parent.position + direction * (length / n as f32),
                velocity: old[k].velocity.lerp(old[k + 1].velocity, t - k as f32),
                direction,
                rotation: b.rotation,
            });
        }
        self.sections = sections;
        self.previous.clone_from(&self.sections);
        self.length = length;
        self.frame_position = position;
        self.frame_rotation = rotation;
    }

    /// Firestorm's 60 Hz clock, with bounded catch-up after long frames.
    /// Interpolate the parent at each physics tick: reusing the final pose
    /// for every substep would make low-FPS attachments receive one big jump.
    pub fn update(&mut self, params: FlexibleParams, position: Vec3, rotation: Quat, dt: f64, wind: impl Fn(Vec3) -> Vec3) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.2) } else { 0.0 };
        if dt == 0.0 || !position.is_finite() || !rotation.is_finite() {
            return;
        }
        self.remainder += dt;
        // Only absorb floating-point roundoff at a tick boundary, never
        // round each frame up to an extra physics step (issue #172).
        while self.remainder + 1e-12 >= STEP_SECONDS {
            self.remainder = (self.remainder - STEP_SECONDS).max(0.0);
            let t = ((dt - self.remainder) / dt).clamp(0.0, 1.0) as f32;
            let tick_position = if t == 1.0 || self.frame_position == position {
                position
            } else {
                self.frame_position.lerp(position, t)
            };
            let tick_rotation = if t == 1.0 || self.frame_rotation == rotation {
                rotation
            } else {
                self.frame_rotation.slerp(rotation, t)
            };
            self.previous.clone_from(&self.sections);
            self.step(params, tick_position, tick_rotation, STEP_SECONDS as f32, &wind);
        }
        self.frame_position = position;
        self.frame_rotation = rotation;
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
        self.sample_path(&self.sections, position, rotation)
    }

    /// Blend the two last physics states with one tick of latency. The
    /// anchor follows the current parent immediately; interpolated segment
    /// directions preserve lengths instead of shrinking a bending chain.
    /// This is a render-only path: interpolation never feeds back into physics.
    pub fn render_path(&self, position: Vec3, rotation: Quat) -> [FlexiblePoint; 9] {
        let n = self.sections.len() - 1;
        let alpha = (self.remainder / STEP_SECONDS).clamp(0.0, 1.0) as f32;
        let mut sections = [self.sections[0]; 9];
        sections[0].position = position - rotation * Vec3::Z * (self.length * 0.5);
        sections[0].direction = rotation * Vec3::Z;
        sections[0].rotation = rotation;
        for i in 1..=n {
            let a = self.previous[i];
            let b = self.sections[i];
            let direction = a.direction.lerp(b.direction, alpha).normalize_or(b.direction);
            sections[i] = Section {
                position: sections[i - 1].position + direction * (self.length / n as f32),
                direction,
                rotation: a.rotation.slerp(b.rotation, alpha),
                ..b
            };
        }
        self.sample_path(&sections[..=n], position, rotation)
    }

    fn sample_path(&self, sections: &[Section], position: Vec3, rotation: Quat) -> [FlexiblePoint; 9] {
        let n = sections.len() - 1;
        let length = self.length / n as f32;
        let derivative = |i: usize| {
            if i == 0 {
                (sections[1].position - sections[0].position) / length
            } else if i == n {
                (sections[n].position - sections[n - 1].position) / length
            } else {
                (sections[i + 1].position - sections[i - 1].position) / (2.0 * length)
            }
        };
        let inv = rotation.conjugate();
        std::array::from_fn(|i| {
            let t = i as f32 * n as f32 / 8.0;
            let k = (t as usize).min(n - 1);
            let u = t - k as f32;
            let a = sections[k];
            let b = sections[k + 1];
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

    fn pose(t: f64, moving: bool) -> (Vec3, Quat) {
        let base = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        if moving {
            (Vec3::new(0.5, 1.0, 0.2) * t as f32, Quat::from_rotation_z(0.4 * t as f32) * base)
        } else {
            (Vec3::ZERO, base)
        }
    }

    fn assert_sections_near(actual: &[Section], expected: &[Section], position_tolerance: f32) {
        for (a, b) in actual.iter().zip(expected) {
            assert!(
                a.position.distance(b.position) < position_tolerance,
                "position: {:?} != {:?}",
                a.position,
                b.position
            );
            assert!(
                a.velocity.distance(b.velocity) < position_tolerance * 100.0,
                "velocity: {:?} != {:?}",
                a.velocity,
                b.velocity
            );
            assert!(
                a.rotation.abs_diff_eq(b.rotation, position_tolerance),
                "rotation: {:?} != {:?}",
                a.rotation,
                b.rotation
            );
        }
    }

    #[test]
    fn forces_and_parent_motion_follow_the_same_physics_clock_at_30_to_500_fps() {
        // Compare the whole transient, not only its final equilibrium. At
        // 59 FPS the former ceil(dt * 60) ran 118 physics steps per second.
        for softness in 0..=3 {
            for moving in [false, true] {
                let mut p = params();
                p.softness = softness;
                p.wind_sensitivity = 5.0;
                p.user_force = Vec3::new(0.5, 1.0, 0.0);
                let wind = |sample: Vec3| Vec3::new(2.0 + sample.y * 0.1, 0.0, 0.0);
                let (pos, rot) = pose(0.0, moving);
                let mut reference = Flexible::new(softness, 4.0, pos, rot);
                let mut states = vec![reference.sections.clone()];
                for tick in 1..=120 {
                    let (pos, rot) = pose(tick as f64 * STEP_SECONDS, moving);
                    reference.update(p, pos, rot, STEP_SECONDS, wind);
                    states.push(reference.sections.clone());
                }
                for fps in [30, 59, 60, 61, 90, 120, 144, 240, 500] {
                    let (pos, rot) = pose(0.0, moving);
                    let mut actual = Flexible::new(softness, 4.0, pos, rot);
                    for frame in 1..=fps * 2 {
                        let (pos, rot) = pose(frame as f64 / fps as f64, moving);
                        actual.update(p, pos, rot, 1.0 / fps as f64, wind);
                        let tick = (frame * 60 / fps) as usize;
                        assert_sections_near(&actual.sections, &states[tick], if moving { 0.01 } else { 1e-5 });
                    }
                }
            }
        }
    }

    #[test]
    fn fluctuating_fps_keep_elapsed_time_and_interpolate_parent_substeps() {
        let (pos, rot) = pose(0.0, true);
        let mut reference = Flexible::new(2, 4.0, pos, rot);
        let mut states = vec![reference.sections.clone()];
        for tick in 1..=120 {
            let (pos, rot) = pose(tick as f64 * STEP_SECONDS, true);
            reference.update(params(), pos, rot, STEP_SECONDS, |_| Vec3::ZERO);
            states.push(reference.sections.clone());
        }
        let mut actual = Flexible::new(2, 4.0, pos, rot);
        let mut elapsed: f64 = 0.0;
        for dt in [1.0 / 500.0, 1.0 / 59.0, 1.0 / 61.0, 1.0 / 144.0, 1.0 / 30.0].into_iter().cycle() {
            let dt = f64::min(dt, 2.0 - elapsed);
            elapsed += dt;
            let (pos, rot) = pose(elapsed, true);
            actual.update(params(), pos, rot, dt, |_| Vec3::ZERO);
            let tick = ((elapsed / STEP_SECONDS + 1e-10).floor() as usize).min(120);
            assert_sections_near(&actual.sections, &states[tick], 0.002);
            if elapsed >= 2.0 {
                break;
            }
        }
        assert!(actual.remainder < 1e-10);
    }

    #[test]
    fn render_interpolation_is_continuous_and_never_changes_physics() {
        let rot = pose(0.0, false).1;
        let mut f = Flexible::new(3, 4.0, Vec3::ZERO, rot);
        let initial = f.render_path(Vec3::ZERO, rot);
        f.update(params(), Vec3::ZERO, rot, STEP_SECONDS, |_| Vec3::ZERO);
        let physics = f.sections.clone();
        let mut last = initial[8].position;
        for _ in 0..3 {
            f.update(params(), Vec3::ZERO, rot, STEP_SECONDS * 0.25, |_| Vec3::ZERO);
            let path = f.render_path(Vec3::ZERO, rot);
            assert!(path[8].position.distance(last) > 0.001, "display frames between ticks must move");
            last = path[8].position;
            assert_sections_near(&f.sections, &physics, 1e-6);
            for pair in path.windows(2) {
                assert!((pair[0].position.distance(pair[1].position) - 0.5).abs() < 1e-5);
            }
        }
        f.update(params(), Vec3::ZERO, rot, STEP_SECONDS * 0.25 - 1e-7, |_| Vec3::ZERO);
        let before_tick = f.render_path(Vec3::ZERO, rot);
        f.update(params(), Vec3::ZERO, rot, 1e-7, |_| Vec3::ZERO);
        let after_tick = f.render_path(Vec3::ZERO, rot);
        for (a, b) in before_tick.iter().zip(after_tick) {
            assert!(a.position.distance(b.position) < 1e-4, "interpolation must not jump at a tick");
            assert!(a.rotation.abs_diff_eq(b.rotation, 1e-4));
        }
    }

    #[test]
    fn interpolated_render_anchor_and_origin_shift_follow_the_current_parent() {
        let rot = pose(0.0, false).1;
        let mut f = Flexible::new(3, 4.0, Vec3::ZERO, rot);
        for _ in 0..45 {
            f.update(params(), Vec3::ZERO, rot, 1.0 / 144.0, |_| Vec3::ZERO);
        }
        let pos = Vec3::new(0.1, 0.2, 0.3);
        let rot = Quat::from_rotation_z(0.1) * rot;
        f.update(params(), pos, rot, 1.0 / 500.0, |_| Vec3::ZERO);
        let before = f.render_path(pos, rot);
        assert!(before[0].position.distance(-Vec3::Z * 2.0) < 1e-5);
        assert!(before[0].rotation.abs_diff_eq(Quat::IDENTITY, 1e-5));
        let shift = Vec3::new(-256.0, 256.0, 0.0);
        f.shift(shift);
        let after = f.render_path(pos + shift, rot);
        for (a, b) in before.iter().zip(after) {
            assert!(a.position.distance(b.position) < 1e-4);
            assert!(a.rotation.abs_diff_eq(b.rotation, 1e-5));
        }
        assert!(f.frame_position.distance(pos + shift) < 1e-5);
        f.reconfigure(1, 6.0, pos + shift, rot);
        let resized = f.render_path(pos + shift, rot);
        assert!(resized[0].position.distance(-Vec3::Z * 3.0) < 1e-4);
        assert!(resized.iter().all(|p| p.position.is_finite() && p.rotation.is_finite()));
    }

    #[test]
    fn invalid_times_do_not_advance_physics_and_long_frames_have_bounded_catch_up() {
        let rot = pose(0.0, false).1;
        let mut f = Flexible::new(2, 4.0, Vec3::ZERO, rot);
        let initial = f.sections.clone();
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            f.update(params(), Vec3::ZERO, rot, dt, |_| Vec3::ZERO);
            assert_sections_near(&f.sections, &initial, 1e-6);
        }
        f.update(params(), Vec3::ZERO, rot, 10.0, |_| Vec3::ZERO);
        let mut reference = Flexible::new(2, 4.0, Vec3::ZERO, rot);
        for _ in 0..12 {
            reference.update(params(), Vec3::ZERO, rot, STEP_SECONDS, |_| Vec3::ZERO);
        }
        assert_sections_near(&f.sections, &reference.sections, 1e-6);
        assert!(f.remainder < 1e-10);
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

    #[test]
    fn resizing_and_changing_softness_preserve_bend_and_anchor() {
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut f = Flexible::new(2, 4.0, Vec3::ZERO, rot);
        for _ in 0..240 {
            f.update(params(), Vec3::ZERO, rot, 1.0 / 60.0, |_| Vec3::ZERO);
        }
        let before = f.sections.clone();
        let anchor = before[0].position;
        // Stretch from the positive Z face; the opposite face stays fixed.
        let pos = anchor + rot * Vec3::Z * 3.0;
        f.reconfigure(2, 6.0, pos, rot);
        assert!(f.sections[0].position.abs_diff_eq(anchor, 1e-5));
        for (old, new) in before.iter().zip(&f.sections).skip(1) {
            assert!(old.direction.dot(new.direction) > 0.9999);
            assert!(old.velocity.abs_diff_eq(new.velocity, 1e-5));
        }
        assert!(f.sections[4].position.z < pos.z - 0.5, "resizing must not straighten a bent prim");
        f.reconfigure(3, 6.0, pos, rot);
        assert!(f.matches(3, 6.0));
        for pair in f.sections.windows(2) {
            assert!((pair[0].position.distance(pair[1].position) - 0.75).abs() < 1e-5);
        }
        assert!(f.sections[8].position.z < pos.z - 0.5);
        f.update(params(), pos, rot, 1.0 / 60.0, |_| Vec3::ZERO);
        assert!(f.path(pos, rot).iter().all(|p| p.position.is_finite() && p.rotation.is_finite()));
    }
}
