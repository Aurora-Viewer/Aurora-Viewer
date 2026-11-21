//! Motion of an object between two reports of its simulator.
//!
//! Port of LLViewerObject::interpolateLinearMotion and applyAngularVelocity
//! (indra/newview/llviewerobject.cpp) and of the damping of
//! LLDrawable::updateXform (indra/newview/lldrawable.cpp), originally LGPL
//! 2.1. Extrapolation integrates elapsed simulator time; drawable damping
//! follows the reference's per-frame steps. Our own avatar
//! has its own copy of the same rules in `agent.rs`.
//!
//! The simulator sends nothing while an object follows the path its last
//! report announced (velocity, acceleration, spin): the viewer extrapolates
//! for as long as the circuit lives. Time is that of the simulator: each
//! region has a clock (`ObjectStore::motion_time`) that runs at the region's
//! time dilation, slows down once its circuit has been silent for 2 s and
//! stops at 3 s, where Firestorm stops its objects; an object then stays
//! where it was and goes on when the circuit comes back.
//!
//! When a report puts an object elsewhere than where it was drawn, the gap
//! fades with the Legacy Viewer's 0.06 s half-life. Volumes retain a drawable
//! transform and interpolate position, normalized rotation and scale together
//! each frame, with the reference's joint, camera-relative stopping test.
//! A new correction requires zero reported velocity; an existing one survives
//! subsequent velocity reports (LLViewerObject::updateDrawable's ON_MOVE_LIST).
//! An already moving object without a correction snaps (#2328). HUD children
//! follow these rules; their root shows reports at once (#3369). Avatars retain
//! their separate residual correction. Receipt time anchors extrapolation,
//! never the duration of the visible transition.
//!
//! The root an avatar sits on goes further ([`damp`]): its drawn pose
//! follows its reported motion with that half-life every frame, moving or
//! not, as LLVOAvatar::idleUpdate forces its drawable into damped motion
//! (LLPipeline::updateMoveDampedAsync). A vehicle's collisions are smoothed
//! so, and the camera that follows its rider with them.

use glam::{Quat, Vec3};
use std::time::Instant;

/// PHYSICS_TIMESTEP: the reported velocity is an average over the
/// simulator's physics step.
const PHYSICS_TIMESTEP: f32 = 1.0 / 45.0;
/// Seconds of silence of a circuit after which its objects slow down, and
/// at which they stop (the 2 s / 3 s of `agent.rs`).
const PHASE_OUT_TIME: f32 = 2.0;
const MAX_INTERPOLATION_TIME: f32 = 3.0;
/// OBJECT_DAMPING_TIME_CONSTANT of lldrawable.cpp, a half-life.
const DAMPING_HALF_LIFE: f32 = 0.06;
/// MAX_INTERPOLATE_DISTANCE_SQUARED of lldrawable.cpp is 10 m squared:
/// farther than that the object was moved, not corrected.
const SNAP_DISTANCE: f32 = 10.0;
/// A correction this old has faded below a thousandth.
const CORRECTION_LIFE: f32 = DAMPING_HALF_LIFE * 10.0;
/// Gaps smaller than these are not worth a correction.
const MIN_OFFSET: f32 = 1e-4;
const MIN_TURN_W: f32 = 1.0 - 1e-7;
/// MIN_INTERPOLATE_DISTANCE_SQUARED of lldrawable.cpp is 1 mm squared per
/// metre of distance to the camera; at a fixed millimetre a damped seat
/// only settles a little later, unseen.
const SETTLED: f32 = 1e-3;

/// Seconds of simulator time per second of ours, for a region whose time
/// dilation is `time_dilation` and whose circuit received its last packet
/// `silent` seconds ago.
pub fn clock_rate(time_dilation: f32, silent: f32) -> f32 {
    let dilation = if time_dilation.is_finite() {
        time_dilation.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let phase = if silent > PHASE_OUT_TIME {
        ((MAX_INTERPOLATION_TIME - silent) / (MAX_INTERPOLATION_TIME - PHASE_OUT_TIME)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    dilation * phase
}

/// Base pose and reported velocities of an object's motion. The pose can
/// include a client-side turn retained across earlier reports.
#[derive(Debug, Clone, Copy)]
pub struct Reported {
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub acceleration: Vec3,
    pub angular_velocity: Vec3,
}

/// Position and rotation `t` simulator seconds after the report. The sum of
/// Firestorm's per-frame steps, `pos += (vel + 0.5 (dt - step) accel) dt`
/// then `vel += accel dt`, whatever the size of the frames.
pub fn extrapolate(m: &Reported, t: f64) -> (Vec3, Quat) {
    let t = t.max(0.0);
    let tf = t as f32;
    let position = m.position + m.velocity * tf + m.acceleration * (0.5 * tf * (tf - PHYSICS_TIMESTEP));
    let speed = m.angular_velocity.length();
    let rotation = if speed * speed > 1e-8 {
        // an object can spin for days (llTargetOmega): the angle is reduced
        // before it loses its precision
        let angle = (f64::from(speed) * t).rem_euclid(std::f64::consts::TAU) as f32;
        (Quat::from_axis_angle(m.angular_velocity / speed, angle) * m.rotation).normalize()
    } else {
        m.rotation
    };
    if position.is_finite() {
        (position, rotation)
    } else {
        (m.position, rotation)
    }
}

/// One frame of the damping of LLDrawable::updateXform: the `drawn` pose
/// goes `dt` seconds of the half-life towards `target`, where the reports
/// send the object. None when it is to be drawn at `target`: settled, or
/// farther than 10 m (moved, not damped).
pub fn damp(drawn: (Vec3, Quat), target: (Vec3, Quat), dt: f32) -> Option<(Vec3, Quat)> {
    let k = (1.0 - 0.5_f32.powf(dt.max(0.0) / DAMPING_HALF_LIFE)).clamp(0.0, 1.0);
    let position = drawn.0.lerp(target.0, k);
    // (glam's lerp of rotations takes the short way and normalizes)
    let rotation = drawn.1.lerp(target.1, k);
    // the Legacy Viewer's measure of what is left: a turn weighs 10 m² per unit of
    // 1 - cos(half angle)
    let gap = position.distance_squared(target.0) + (1.0 - rotation.dot(target.1).abs()) * 10.0;
    let damping = (SETTLED * SETTLED..=SNAP_DISTANCE * SNAP_DISTANCE).contains(&gap);
    (damping && position.is_finite() && rotation.is_finite()).then_some((position, rotation))
}

/// The gap between where an object was drawn and where a report puts it,
/// fading away.
#[derive(Debug, Clone, Copy)]
pub struct Correction {
    offset: Vec3,
    turn: Quat,
    at: Instant,
    /// The report it leads to: a position or rotation written since by
    /// anything else (a drag in the build tools) cancels the correction.
    position: Vec3,
    rotation: Quat,
    /// Volumes keep their drawable transform across reports, as ON_MOVE_LIST
    /// does. Only the frame scheduler advances it; avatars retain residual damping.
    pub drawable: Option<(Vec3, Quat)>,
    pub fresh: bool,
}

impl Correction {
    /// From where the object was `drawn` to where the report puts it now.
    /// None when there is nothing to fade, or too much.
    #[cfg(test)]
    pub fn between(drawn: (Vec3, Quat), reported: (Vec3, Quat), now: Instant) -> Option<Correction> {
        Self::toward(drawn, reported, reported, now)
    }

    /// From where the object was `drawn` to `pose`, where its last report
    /// (`reported`) sends it at `now`: an object leaving the damping of a
    /// seat while it moves.
    pub fn toward(drawn: (Vec3, Quat), pose: (Vec3, Quat), reported: (Vec3, Quat), now: Instant) -> Option<Correction> {
        let offset = drawn.0 - pose.0;
        if !offset.is_finite() || offset.length_squared() > SNAP_DISTANCE * SNAP_DISTANCE {
            return None;
        }
        let mut turn = (drawn.1 * pose.1.inverse()).normalize();
        if turn.w < 0.0 {
            turn = -turn;
        }
        if offset.length_squared() < MIN_OFFSET * MIN_OFFSET && (turn.w > MIN_TURN_W || !turn.is_finite()) {
            return None;
        }
        Some(Correction {
            offset,
            turn: if turn.is_finite() { turn } else { Quat::IDENTITY },
            at: now,
            position: reported.0,
            rotation: reported.1,
            drawable: None,
            fresh: false,
        })
    }

    pub fn drawable(drawn: (Vec3, Quat), reported: (Vec3, Quat), now: Instant, fresh: bool) -> Self {
        Self {
            offset: Vec3::ZERO,
            turn: Quat::IDENTITY,
            at: now,
            position: reported.0,
            rotation: reported.1,
            drawable: Some(drawn),
            fresh,
        }
    }

    /// Whether it still shows at `now`.
    pub fn active(&self, now: Instant) -> bool {
        self.drawable.is_some() || now.saturating_duration_since(self.at).as_secs_f32() < CORRECTION_LIFE
    }

    /// Whether the object still holds the report this correction leads to.
    pub fn leads_to(&self, position: Vec3, rotation: Quat) -> bool {
        self.position == position && self.rotation == rotation
    }

    /// What is left of the gap at `now`, added to an extrapolated pose.
    pub fn apply(&self, pose: (Vec3, Quat), now: Instant) -> (Vec3, Quat) {
        if let Some(drawn) = self.drawable {
            return drawn;
        }
        let age = now.saturating_duration_since(self.at).as_secs_f32();
        if age >= CORRECTION_LIFE {
            return pose;
        }
        let left = 0.5_f32.powf(age / DAMPING_HALF_LIFE);
        (
            pose.0 + self.offset * left,
            (Quat::IDENTITY.slerp(self.turn, left) * pose.1).normalize(),
        )
    }
}

/// Scale follows the same 0.06 s half-life and visual start time as position.
/// LLDrawable::updateXform interpolates both together: otherwise a scripted
/// stretch moves its anchored end because the centre lags behind its size.
#[derive(Debug, Clone, Copy)]
pub struct ScaleCorrection {
    offset: Vec3,
    target: Vec3,
    at: Instant,
    pub drawn: Vec3,
    frame_driven: bool,
}

pub fn within_snap_distance(position_offset: Vec3, scale_offset: Vec3) -> bool {
    position_offset.is_finite()
        && scale_offset.is_finite()
        && position_offset.length_squared() + scale_offset.length_squared() <= SNAP_DISTANCE * SNAP_DISTANCE
}

impl ScaleCorrection {
    pub fn between(drawn: Vec3, target: Vec3, now: Instant) -> Option<Self> {
        let offset = drawn - target;
        (drawn.is_finite()
            && target.is_finite()
            && drawn.min_element() > 0.0
            && target.min_element() > 0.0
            && within_snap_distance(Vec3::ZERO, offset)
            && offset.length_squared() >= MIN_OFFSET * MIN_OFFSET)
            .then_some(Self {
                offset,
                target,
                at: now,
                drawn,
                frame_driven: false,
            })
    }

    pub fn drawable(drawn: Vec3, target: Vec3, now: Instant) -> Self {
        Self {
            offset: drawn - target,
            target,
            at: now,
            drawn,
            frame_driven: true,
        }
    }

    pub fn matches(&self, scale: Vec3) -> bool {
        self.target == scale
    }

    /// A ridden root hands its pose to the seat's forced damping.
    pub fn leave_drawable(&mut self, now: Instant) {
        if self.frame_driven {
            self.offset = self.drawn - self.target;
            self.at = now;
            self.frame_driven = false;
        }
    }

    pub fn sample(&self, scale: Vec3, now: Instant) -> Vec3 {
        if !self.matches(scale) {
            return scale;
        }
        if self.frame_driven {
            return self.drawn;
        }
        let age = now.saturating_duration_since(self.at).as_secs_f32();
        if age >= CORRECTION_LIFE {
            return scale;
        }
        scale + self.offset * 0.5_f32.powf(age / DAMPING_HALF_LIFE)
    }

    pub fn advance(&mut self, scale: Vec3, now: Instant) -> bool {
        self.drawn = self.sample(scale, now);
        self.matches(scale) && (self.frame_driven || now.saturating_duration_since(self.at).as_secs_f32() < CORRECTION_LIFE)
    }
}

#[derive(Clone, Copy)]
pub struct DampingView {
    pub distance_squared: f32,
    pub visible: bool,
}

/// LLSmoothInterpolation::getInterpolant, shared by the frame's drawables.
pub fn drawable_interpolant(dt: f32) -> f32 {
    (1.0 - 2.0_f32.powf(-dt.max(0.0) / DAMPING_HALF_LIFE)).clamp(0.0, 1.0)
}

/// Port of LLDrawable::updateXform and LLQuaternion::nlerp/slerp
/// (indra/newview/lldrawable.cpp, indra/llmath/llquaternion.cpp, LGPL 2.1).
/// The stopping test is joint, after interpolation, and relative to camera
/// distance. A moving target is followed every frame until this test settles.
pub fn drawable_step(
    drawn: (Vec3, Quat, Vec3),
    target: (Vec3, Quat, Vec3),
    k: f32,
    view: DampingView,
    spinning: bool,
    child: bool,
) -> ((Vec3, Quat, Vec3), bool) {
    let mut result = target;
    let mut gap = 0.0;
    if view.visible {
        let position = drawn.0.lerp(target.0, k);
        let rotation = drawable_nlerp(drawn.1, target.1, k);
        let scale = drawn.2.lerp(target.2, k);
        gap = position.distance_squared(target.0) + (1.0 - rotation.dot(target.1)) * 10.0 + scale.distance_squared(target.2);
        if (SETTLED * SETTLED * view.distance_squared..=SNAP_DISTANCE * SNAP_DISTANCE).contains(&gap) {
            result = (position, rotation, scale);
        } else if !spinning {
            gap = 0.0;
        }
    }
    // LLDrawable keeps animated children on the move list, including their
    // target omega, until their local motion finishes.
    if drawn.2.distance_squared(result.2) <= SETTLED * SETTLED && child && (spinning || gap > 0.0) {
        gap = 1.0;
    }
    (result, gap != 0.0)
}

fn drawable_nlerp(a: Quat, b: Quat, k: f32) -> Quat {
    let dot = a.dot(b);
    if dot >= 0.0 {
        return (a * (1.0 - k) + b * k).normalize();
    }
    // The Legacy Viewer flips a, not b, and returns in b's hemisphere.
    // That sign matters to updateXform's subsequent 1 - dot error measure.
    let cosine = -dot;
    let (beta, alpha) = if 1.0 - cosine < 0.00001 {
        (1.0 - k, k)
    } else {
        let theta = cosine.acos();
        let sine = theta.sin();
        ((theta - k * theta).sin() / sine, (k * theta).sin() / sine)
    };
    a * -beta + b * alpha
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn near(a: Vec3, b: Vec3) {
        assert!(a.distance(b) < 1e-4, "{a:?} != {b:?}");
    }

    fn moving(velocity: Vec3, acceleration: Vec3) -> Reported {
        Reported {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity,
            acceleration,
            angular_velocity: Vec3::ZERO,
        }
    }

    #[test]
    fn a_steady_course_goes_on_for_as_long_as_the_clock_runs() {
        let walker = moving(Vec3::X * 3.2, Vec3::ZERO);
        // (the previous rule stopped it 0.6 s after its last report)
        for t in [0.2, 0.6, 5.0, 60.0] {
            near(extrapolate(&walker, t).0, Vec3::X * 3.2 * t as f32);
        }
        // never before its report
        near(extrapolate(&walker, -1.0).0, Vec3::ZERO);
    }

    #[test]
    fn acceleration_matches_the_sum_of_firestorms_steps_at_any_frame_rate() {
        let falling = moving(Vec3::X * 2.0, Vec3::new(0.0, 0.0, -9.8));
        for frames in [10, 45, 600] {
            let dt = 1.0 / frames as f32;
            let (mut pos, mut vel) = (Vec3::ZERO, falling.velocity);
            for _ in 0..frames {
                pos += (vel + 0.5 * (dt - PHYSICS_TIMESTEP) * falling.acceleration) * dt;
                vel += falling.acceleration * dt;
            }
            assert!(extrapolate(&falling, 1.0).0.distance(pos) < 2e-3, "{frames} frames a second");
        }
    }

    #[test]
    fn a_spin_keeps_turning_and_keeps_its_precision_for_days() {
        let spinner = Reported {
            angular_velocity: Vec3::Z * 0.5,
            ..moving(Vec3::ZERO, Vec3::ZERO)
        };
        let after = |t: f64| extrapolate(&spinner, t).1 * Vec3::X;
        near(after(1.0), Vec3::new(0.5_f32.cos(), 0.5_f32.sin(), 0.0));
        // a whole number of turns later, the same direction
        let turn = std::f64::consts::TAU / 0.5;
        near(after(1.0 + turn * 40_000.0), after(1.0));
        // and still moving a frame later, three days in
        let late = 3.0 * 86_400.0;
        assert!(after(late).distance(after(late + 0.016)) > 0.005);
    }

    #[test]
    fn the_clock_follows_the_dilation_and_stops_with_a_silent_circuit() {
        assert_eq!(clock_rate(1.0, 0.0), 1.0);
        assert_eq!(clock_rate(0.5, 1.9), 0.5);
        assert!((clock_rate(1.0, 2.5) - 0.5).abs() < 1e-6);
        assert_eq!(clock_rate(1.0, 3.0), 0.0);
        assert_eq!(clock_rate(1.0, 60.0), 0.0);
        // a circuit never heard, or a bad value, does not freeze the region
        assert_eq!(clock_rate(f32::NAN, 0.0), 1.0);
        assert_eq!(clock_rate(1.0, f32::INFINITY), 0.0);
    }

    #[test]
    fn a_correction_fades_with_firestorms_half_life() {
        let now = Instant::now();
        let at = |ms: u64| now + Duration::from_millis(ms);
        let reported = (Vec3::X * 2.0, Quat::IDENTITY);
        let c = Correction::between((Vec3::ZERO, Quat::IDENTITY), reported, now).expect("a gap of 2 m");
        // drawn where it was, half-way one half-life later, then at the report
        near(c.apply(reported, now).0, Vec3::ZERO);
        near(c.apply(reported, at(60)).0, Vec3::X);
        near(c.apply(reported, at(120)).0, Vec3::X * 1.5);
        near(c.apply(reported, at(700)).0, Vec3::X * 2.0);
        assert!(c.active(at(500)) && !c.active(at(700)));
        // it rides on the extrapolation: no lag behind an object that moves on
        near(
            c.apply((Vec3::X * 2.0 + Vec3::Y * 5.0, Quat::IDENTITY), at(60)).0,
            Vec3::new(1.0, 5.0, 0.0),
        );
    }

    #[test]
    fn a_correction_turns_the_short_way_and_ignores_what_is_too_small_or_too_far() {
        let now = Instant::now();
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let c = Correction::between((Vec3::ZERO, Quat::IDENTITY), (Vec3::ZERO, quarter), now).expect("a quarter turn");
        let half_way = c.apply((Vec3::ZERO, quarter), now + Duration::from_millis(60)).1;
        near(half_way * Vec3::X, Quat::from_rotation_z(std::f32::consts::FRAC_PI_4) * Vec3::X);
        // the same pose: nothing to fade
        assert!(Correction::between((Vec3::ONE, quarter), (Vec3::ONE, quarter), now).is_none());
        // a teleport across the region: shown at once
        assert!(Correction::between((Vec3::ZERO, quarter), (Vec3::X * 10.5, quarter), now).is_none());
        assert!(Correction::between((Vec3::splat(f32::NAN), quarter), (Vec3::X, quarter), now).is_none());
        // a pose written since by something else cancels it
        assert!(c.leads_to(Vec3::ZERO, quarter));
        assert!(!c.leads_to(Vec3::X * 0.01, quarter));
    }

    #[test]
    fn a_damped_seat_follows_with_the_half_life_at_any_frame_rate() {
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let target = (Vec3::X * 2.0, quarter);
        for frames in [30, 60, 240] {
            let dt = 0.06 / frames as f32 * 10.0;
            let mut drawn = (Vec3::ZERO, Quat::IDENTITY);
            for _ in 0..frames / 10 {
                drawn = damp(drawn, target, dt).expect("still damping");
            }
            // one half-life: half-way, whatever the frames
            near(drawn.0, Vec3::X);
            // (steps of a linear blend of rotations, the Legacy Viewer's nlerp: about
            // half-way, a degree off for a quarter turn)
            let angle = drawn.1.angle_between(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4));
            assert!(angle < 0.05, "{frames} frames a second: {angle} rad off");
        }
        // no time: where it was drawn
        let drawn = damp((Vec3::ZERO, quarter), target, 0.0).expect("damping");
        near(drawn.0, Vec3::ZERO);
        // the other sign of the same rotation is no gap
        assert!(damp((target.0, -quarter), target, 0.016).is_none());
        // settled within a millimetre, or moved more than 10 m: drawn there
        assert!(damp((Vec3::X * 2.0005, quarter), target, 0.016).is_none());
        assert!(damp((Vec3::X * 20.0, quarter), target, 0.016).is_none());
        assert!(damp((Vec3::splat(f32::NAN), quarter), target, 0.016).is_none());
    }

    #[test]
    fn a_seat_left_while_moving_fades_from_where_it_was_drawn() {
        let now = Instant::now();
        let report = (Vec3::ZERO, Quat::IDENTITY);
        // drawn 1 m behind where its motion has taken it since the report
        let pose = (Vec3::X * 5.0, Quat::IDENTITY);
        let c = Correction::toward((Vec3::X * 4.0, Quat::IDENTITY), pose, report, now).expect("a gap");
        near(c.apply(pose, now).0, Vec3::X * 4.0);
        near(
            c.apply((Vec3::X * 6.0, Quat::IDENTITY), now + Duration::from_millis(60)).0,
            Vec3::X * 5.5,
        );
        assert!(c.leads_to(report.0, report.1));
    }
}
