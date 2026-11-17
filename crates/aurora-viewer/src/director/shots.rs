//! The Director's shots: the camera moves a filmmaker asks for by name,
//! laid out as ordinary keyframes from the current view (an orbit around
//! a centre, a dolly zoom, a crane, a tracking shot, a push in or a pull
//! out, a fly-over), and what a keyframe adds to the curve for them: the
//! easing of the way to it, the lens that keeps the subject's size on that
//! way, and a camera that travels with its target.
//!
//! Aurora's own: the Legacy Viewer has no camera path tool.
//!
//! - **Easing** (`Ease`) remaps the time along one segment (the way to a
//!   keyframe from the one before, as its time and speed are): the path,
//!   its length, its duration and so its mean speed stay, so the pace, the
//!   even speed and the stop keyframes keep working. "Ease in" starts from
//!   rest and reaches the curve's own speed at the next keyframe, "Ease
//!   out" leaves at the curve's speed and comes to rest: the end that is
//!   not eased joins its neighbour without a jump of speed.
//! - **Dolly zoom** (`dolly_fov`): between two keyframes on a straight line
//!   towards a subject, `distance · tan(fov / 2)` is constant, so the
//!   cotangent of the half view angle is linear in the distance travelled.
//!   The lens is computed from where the camera is on the way, every frame:
//!   the effect holds between the keyframes, whatever the easing.
//! - **Travel** (`KeyShot::anchor`): a keyframe that follows its target
//!   keeps where the target was when it was laid; played, it is moved by
//!   what the target moved since (in the world's axes: a dancer who spins
//!   does not whirl the camera round), blended smoothly with its
//!   neighbours, so a world keyframe and a following one join without a
//!   jump.

use super::{Aim, Director, KeyAim, Mode};
use crate::camera::track::{self, Curve, Pose};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::ops::RangeInclusive;

/// An orbit has a keyframe every this many radians: the curve through them
/// stays within a fraction of a percent of the circle.
pub const ORBIT_STEP: f32 = PI / 12.0;
/// The arc an orbit can sweep, degrees (three turns at most).
pub const ARC_DEGREES: RangeInclusive<f32> = 15.0..=1080.0;
/// The distances of a shot (metres).
pub const DISTANCE_RANGE: RangeInclusive<f32> = 0.5..=100.0;
pub const RADIUS_RANGE: RangeInclusive<f32> = 0.5..=200.0;
pub const HEIGHT_RANGE: RangeInclusive<f32> = -50.0..=100.0;
pub const RISE_RANGE: RangeInclusive<f32> = -50.0..=50.0;
/// Seconds a shot lasts.
pub const DURATION_RANGE: RangeInclusive<f32> = 1.0..=600.0;
/// The camera of a shot never comes closer to its centre than this.
const NEAREST: f32 = 0.5;
/// Two keyframes of a shot are this many seconds apart at least.
const SPACING: f32 = 0.2;
/// A straight move that keeps its aim has a keyframe every this many
/// radians of turn, so that the view between them stays on the centre.
const AIM_STEP: f32 = PI / 9.0;
/// The most segments such a move is cut into.
const AIM_SEGMENTS: usize = 8;
/// The fly-over looks this far down over the centre, and at its end.
const OVER_PITCH: f32 = -PI / 3.0;
const AFTER_PITCH: f32 = -PI / 12.0;

/// How the time runs on the way to a keyframe from the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    /// From rest, up to the curve's speed at the keyframe.
    In,
    /// From the curve's speed down to rest at the keyframe.
    Out,
    /// From rest to rest.
    InOut,
    /// The curve's own pace. (A value a newer viewer wrote and this one
    /// does not know reads as this one: serde's `other` is the last.)
    #[default]
    #[serde(other)]
    Linear,
}

impl Ease {
    pub const ALL: [Ease; 4] = [Ease::Linear, Ease::In, Ease::Out, Ease::InOut];

    /// How far through the segment the camera is when `s` of its time has
    /// gone, 0 to 1. Each curve starts at 0, ends at 1 and never goes back;
    /// "in" ends at the slope 1 and "out" starts at it, so the end that is
    /// not eased keeps the curve's speed.
    pub fn apply(self, s: f32) -> f32 {
        let s = if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Ease::Linear => s,
            Ease::In => s * s * (2.0 - s),
            Ease::Out => s + s * s - s * s * s,
            Ease::InOut => s * s * (3.0 - 2.0 * s),
        }
    }

    pub fn is_linear(&self) -> bool {
        *self == Ease::Linear
    }

    /// This easing with a start from rest (`start`) or a stop (`end`) added.
    fn with(self, start: bool, end: bool) -> Ease {
        let start = start || matches!(self, Ease::In | Ease::InOut);
        let end = end || matches!(self, Ease::Out | Ease::InOut);
        match (start, end) {
            (false, false) => Ease::Linear,
            (true, false) => Ease::In,
            (false, true) => Ease::Out,
            (true, true) => Ease::InOut,
        }
    }
}

/// What a keyframe adds to the curve for the shots.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KeyShot {
    /// The easing of the way to this keyframe from the one before.
    pub ease: Ease,
    /// On that way, the lens keeps the subject's size (`dolly_fov`).
    pub dolly_zoom: bool,
    /// The keyframe travels with its target: where the target was when it
    /// was laid (render space). `None`: it stays where it is in the world.
    pub anchor: Option<Vec3>,
}

/// The view angle on the way from `a` to `b` with the camera at
/// `position`: `distance · tan(fov / 2)` held constant for a subject on
/// that line, so the cotangent of the half angle goes linearly from one
/// keyframe's to the other's with the distance travelled (measured along
/// the line, whatever the easing or the curve). A camera that does not
/// move has nothing to keep: `fallback`.
pub fn dolly_fov(a: &Pose, b: &Pose, position: Vec3, fallback: f32) -> f32 {
    let way = b.position - a.position;
    let length = way.length_squared();
    if length < track::MIN_TRAVEL * track::MIN_TRAVEL || !position.is_finite() {
        return fallback;
    }
    let u = ((position - a.position).dot(way) / length).clamp(0.0, 1.0);
    let cot = |fov: f32| 1.0 / (fov * 0.5).tan();
    let (from, to) = (cot(a.fov), cot(b.fov));
    let fov = 2.0 * (1.0 / (from + (to - from) * u)).atan();
    if fov.is_finite() {
        fov.clamp(a.fov.min(b.fov), a.fov.max(b.fov))
    } else {
        fallback
    }
}

/// The view `pose` the track gives, moved with the targets its keyframes
/// travel with: each following keyframe by what its target moved since it
/// was laid, blended between the two keyframes around the playhead as the
/// view's turn is (`Track::around`). A keyframe whose target is not in
/// sight, or that does not follow one, stays where it was laid.
pub fn travelled(d: &Director, pose: Pose, find: &impl Fn(Aim) -> Option<Vec3>) -> Pose {
    let Some((before, after, s)) = d.track.around(d.time) else {
        return pose;
    };
    let moved = |index: usize| {
        let anchor = d.track.keys().get(index)?.shot.anchor?;
        let now = find(d.key_aim(index)?)?;
        let moved = now - anchor;
        moved.is_finite().then_some(moved)
    };
    let (from, to) = (moved(before).unwrap_or(Vec3::ZERO), moved(after).unwrap_or(Vec3::ZERO));
    Pose {
        position: pose.position + from.lerp(to, s),
        ..pose
    }
}

/// Where a shot starts from: the current view, the centre it turns around
/// or looks at, and whether that centre is the lock's target (in sight).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Here {
    pub view: Pose,
    pub centre: Vec3,
    pub on_target: bool,
}

impl Default for Here {
    fn default() -> Self {
        Here {
            view: Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, crate::camera::DEFAULT_FOV),
            centre: Vec3::X * 5.0,
            on_target: false,
        }
    }
}

/// The shots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Round the centre, looking at it.
    Orbit,
    /// Towards the centre (or away), the lens widening (or narrowing) so
    /// that the subject keeps its size: the vertigo effect.
    DollyZoom,
    /// Straight up, looking at the centre.
    Crane,
    /// Sideways, the view turned as it is.
    Truck,
    /// Forward along the view, never past the centre.
    PushIn,
    /// Backwards along the view.
    PullOut,
    /// Over the centre and on beyond it, looking ahead and down.
    FlyOver,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Orbit,
        Kind::DollyZoom,
        Kind::Crane,
        Kind::Truck,
        Kind::PushIn,
        Kind::PullOut,
        Kind::FlyOver,
    ];
}

/// A shot as the window asks for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub kind: Kind,
    /// Seconds it lasts.
    pub duration: f32,
    /// Metres: how far a crane rises, a tracking shot or a push goes, a
    /// dolly zoom travels; how high a fly-over passes over the centre.
    pub distance: f32,
    /// An orbit's radius and height over the centre; `None`: the camera's
    /// own now, so that the orbit starts where the camera is.
    pub radius: Option<f32>,
    pub height: Option<f32>,
    /// The arc an orbit sweeps, radians, and which way (seen from above).
    pub arc: f32,
    pub clockwise: bool,
    /// Metres an orbit climbs from its start to its end (a spiral).
    pub rise: f32,
    /// A tracking shot to the left, a dolly zoom away from the centre.
    pub reverse: bool,
    /// The shot replaces the track instead of following its last keyframe.
    pub replace: bool,
    /// Its keyframes travel with the lock's target.
    pub travel: bool,
}

impl Shot {
    /// The shot with its usual distance and duration.
    pub fn new(kind: Kind) -> Shot {
        let (distance, duration) = match kind {
            Kind::Orbit => (4.0, 10.0),
            Kind::DollyZoom => (4.0, 4.0),
            Kind::Crane => (4.0, 6.0),
            Kind::Truck => (4.0, 6.0),
            Kind::PushIn | Kind::PullOut => (3.0, 4.0),
            Kind::FlyOver => (3.0, 8.0),
        };
        Shot {
            kind,
            duration,
            distance,
            radius: None,
            height: None,
            arc: 2.0 * PI,
            clockwise: false,
            rise: 0.0,
            reverse: false,
            replace: false,
            travel: true,
        }
    }
}

/// A keyframe of a shot: seconds from its start, its view and what it adds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotKey {
    pub time: f32,
    pub pose: Pose,
    pub shot: KeyShot,
}

/// The keyframes a shot lays.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    pub keys: Vec<ShotKey>,
    /// They look at the centre: at the lock's target while the lock is on,
    /// as a keyframe added then does (else where the camera is laid).
    pub aimed: bool,
    /// A curved path: the track's curve is made smooth.
    pub curved: bool,
}

fn clamp(value: f32, range: &RangeInclusive<f32>, otherwise: f32) -> f32 {
    if value.is_finite() {
        value.clamp(*range.start(), *range.end())
    } else {
        otherwise
    }
}

/// The view at `position` turned to `centre`, with the lens and the roll of
/// `view`.
fn looking(view: &Pose, position: Vec3, centre: Vec3) -> Pose {
    Pose { position, ..*view }.aimed(centre, 1.0)
}

fn key(time: f32, pose: Pose) -> ShotKey {
    ShotKey {
        time,
        pose,
        shot: KeyShot::default(),
    }
}

/// The keyframes of `shot` from `here`; `None` when there is no room for
/// it (a camera already on its centre, a lens at its limit, no way to go).
pub fn make(shot: &Shot, here: &Here) -> Option<Made> {
    let view = here.view;
    let (eye, centre) = (view.position, here.centre);
    if !eye.is_finite() || !centre.is_finite() {
        return None;
    }
    let distance = clamp(shot.distance, &DISTANCE_RANGE, 1.0);
    let duration = clamp(shot.duration, &DURATION_RANGE, 1.0);
    let made = |keys: Vec<ShotKey>, aimed: bool, curved: bool| Some(Made { keys, aimed, curved });
    match shot.kind {
        Kind::Orbit => {
            let rel = eye - centre;
            let flat = Vec2::new(rel.x, rel.y);
            let start = if flat.length() > 1e-3 {
                flat.y.atan2(flat.x)
            } else {
                let back = -view.forward();
                back.y.atan2(back.x)
            };
            let radius = clamp(shot.radius.unwrap_or(flat.length()), &RADIUS_RANGE, NEAREST);
            let height = clamp(shot.height.unwrap_or(rel.z), &HEIGHT_RANGE, 0.0);
            let rise = clamp(shot.rise, &RISE_RANGE, 0.0);
            let arc = clamp(
                shot.arc,
                &(ARC_DEGREES.start().to_radians()..=ARC_DEGREES.end().to_radians()),
                2.0 * PI,
            );
            let segments = ((arc / ORBIT_STEP).ceil() as usize).max(2);
            let duration = duration.max(segments as f32 * SPACING);
            let turn = if shot.clockwise { -arc } else { arc };
            let keys = (0..=segments)
                .map(|k| {
                    let f = k as f32 / segments as f32;
                    let angle = start + turn * f;
                    let at = centre + Vec3::new(radius * angle.cos(), radius * angle.sin(), height + rise * f);
                    key(duration * f, looking(&view, at, centre))
                })
                .collect();
            made(keys, true, true)
        }
        Kind::DollyZoom => {
            let to = centre - eye;
            let near = to.length();
            if near < NEAREST * 2.0 {
                return None;
            }
            let wanted = if shot.reverse {
                near + distance
            } else {
                (near - distance).max(NEAREST)
            };
            let (low, high) = (*crate::camera::FOV_RANGE.start(), *crate::camera::FOV_RANGE.end());
            let half = (view.fov * 0.5).tan();
            let fov = (2.0 * (half * near / wanted).atan()).clamp(low, high);
            // a lens at its limit stops the move where it holds the subject
            let far = near * half / (fov * 0.5).tan();
            if !far.is_finite() || (far - near).abs() < track::MIN_TRAVEL {
                return None;
            }
            let start = looking(&view, eye, centre);
            let end = Pose {
                position: centre - to / near * far,
                fov,
                ..start
            };
            let mut keys = vec![key(0.0, start), key(duration, end)];
            keys[1].shot.dolly_zoom = true;
            made(keys, true, false)
        }
        Kind::Crane => {
            let top = eye + Vec3::Z * distance;
            Some(aimed_line(&view, top, centre, duration))
        }
        Kind::Truck => {
            let left = view.left();
            let side = Vec3::new(left.x, left.y, 0.0).try_normalize().unwrap_or(left);
            let way = if shot.reverse { side } else { -side };
            straight(&view, eye + way * distance, duration)
        }
        Kind::PushIn => {
            let ahead = (centre - eye).dot(view.forward());
            let room = if ahead > NEAREST * 2.0 { ahead - NEAREST } else { distance };
            straight(&view, eye + view.forward() * distance.min(room), duration)
        }
        Kind::PullOut => straight(&view, eye - view.forward() * distance, duration),
        Kind::FlyOver => {
            let rel = eye - centre;
            let mut flat = Vec3::new(rel.x, rel.y, 0.0);
            if flat.length() < NEAREST {
                let back = -view.forward();
                flat = Vec3::new(back.x, back.y, 0.0).try_normalize().unwrap_or(Vec3::X) * (NEAREST * 10.0);
            }
            let ahead = -flat.normalize();
            let yaw = ahead.y.atan2(ahead.x);
            let over = centre + Vec3::Z * (rel.z.max(0.0) + distance);
            let beyond = centre - flat + Vec3::Z * rel.z;
            let keys = vec![
                key(0.0, looking(&view, eye, centre)),
                key(duration * 0.5, Pose::from_angles(over, yaw, OVER_PITCH, 0.0, view.fov)),
                key(duration, Pose::from_angles(beyond, yaw, AFTER_PITCH, 0.0, view.fov)),
            ];
            made(keys, false, true)
        }
    }
}

/// A straight move to `end` with the view turned as it is.
fn straight(view: &Pose, end: Vec3, duration: f32) -> Option<Made> {
    if view.position.distance(end) < track::MIN_TRAVEL {
        return None;
    }
    Some(Made {
        keys: vec![key(0.0, *view), key(duration, Pose { position: end, ..*view })],
        aimed: true,
        curved: false,
    })
}

/// A straight move to `end` that keeps looking at `centre`: a keyframe each
/// `AIM_STEP` of turn, so that the view stays on it even with the lock off.
fn aimed_line(view: &Pose, end: Vec3, centre: Vec3, duration: f32) -> Made {
    let eye = view.position;
    let turn = (centre - eye).angle_between(centre - end);
    let segments = if turn.is_finite() {
        ((turn / AIM_STEP).ceil() as usize).clamp(1, AIM_SEGMENTS)
    } else {
        1
    };
    let keys = (0..=segments)
        .map(|k| {
            let f = k as f32 / segments as f32;
            key(duration * f, looking(view, eye.lerp(end, f), centre))
        })
        .collect();
    Made {
        keys,
        aimed: true,
        curved: false,
    }
}

/// Why a shot could not be laid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Too many keyframes for the track.
    Full,
    /// The track would last too long.
    TooLong,
}

/// Whether the track has room for the keyframes `made`: the time the shot
/// would start at.
pub fn room(d: &Director, shot: &Shot, made: &Made) -> Result<f32, Refused> {
    let count = made.keys.len();
    if count < 2 {
        return Err(Refused::Full);
    }
    let replace = shot.replace || d.track.is_empty();
    let kept = if replace { 0 } else { d.track.len() };
    if kept + count > track::MAX_KEYS {
        return Err(Refused::Full);
    }
    let start = if replace { 0.0 } else { d.track.duration() + track::DEFAULT_GAP };
    if start + made.keys[count - 1].time > track::MAX_TIME {
        return Err(Refused::TooLong);
    }
    Ok(start)
}

/// Lay the keyframes `made` in the track, as one change (one undo step):
/// after its last keyframe (`track::DEFAULT_GAP` later) or in its place.
/// They look as `made` says, at the lock's target while the lock is on (as
/// "Add keyframe" does); with `anchor` (where the lock's target is now),
/// they travel with it. A track replaced starts and stops softly through
/// the easing of its first and last segments, which keeps the path exact
/// (the track's "smooth start and end" goes off: it bends the curve's
/// ends); a shot added after the track stops softly when that option is
/// off. The first keyframe of the shot is selected, the playhead on it:
/// Play plays the shot.
pub fn lay(d: &mut Director, shot: &Shot, made: &Made, anchor: Option<Vec3>) -> Result<usize, Refused> {
    let start = room(d, shot, made)?;
    let count = made.keys.len();
    let replace = shot.replace || d.track.is_empty();
    if replace {
        d.track.clear();
        d.track.ease_ends = false;
    }
    if made.curved {
        d.track.curve = Curve::Smooth;
    }
    let aim = match (made.aimed, d.lock.on) {
        (false, _) => KeyAim::Free,
        (true, true) => KeyAim::On(d.lock.aim()),
        (true, false) => KeyAim::Lock,
    };
    let anchor = anchor.filter(|point| shot.travel && point.is_finite() && matches!(aim, KeyAim::On(_)));
    let soft_end = replace || !d.track.ease_ends;
    let mut first = None;
    for (k, laid) in made.keys.iter().enumerate() {
        let Some(index) = d.track.insert(start + laid.time, laid.pose) else {
            return Err(Refused::Full);
        };
        let ease = laid.shot.ease.with(replace && k == 1, soft_end && k == count - 1 && k > 0);
        d.track.set_aim(index, aim);
        d.track.set_shot(
            index,
            KeyShot {
                ease: if k == 0 { laid.shot.ease } else { ease },
                anchor,
                ..laid.shot
            },
        );
        first.get_or_insert(index);
    }
    let first = first.unwrap_or(0);
    d.mode = Mode::Editing;
    d.select(first);
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::{Track, TrackFile};
    use crate::director::{History, Joint, Target};

    fn view(position: Vec3, towards: Vec3, fov: f32) -> Pose {
        Pose::from_view(position, towards - position, Vec3::Z, fov)
    }

    fn here(position: Vec3, centre: Vec3) -> Here {
        Here {
            view: view(position, centre, 1.0),
            centre,
            on_target: false,
        }
    }

    fn looks_at(pose: &Pose, point: Vec3) -> f32 {
        pose.forward().angle_between(point - pose.position).to_degrees()
    }

    #[test]
    fn each_easing_goes_from_zero_to_one_without_going_back() {
        for ease in Ease::ALL {
            assert_eq!(ease.apply(0.0), 0.0, "{ease:?}");
            assert!((ease.apply(1.0) - 1.0).abs() < 1e-6, "{ease:?}");
            let mut last = 0.0;
            for step in 1..=100 {
                let now = ease.apply(step as f32 / 100.0);
                assert!(now >= last, "{ease:?}");
                last = now;
            }
            assert_eq!(ease.apply(f32::NAN), 0.0);
            assert_eq!(ease.apply(-1.0), 0.0);
        }
        let slope = |ease: Ease, s: f32| (ease.apply(s + 1e-3) - ease.apply(s - 1e-3)) / 2e-3;
        // from rest, and the curve's own speed at the end that is not eased
        assert!(slope(Ease::In, 1e-3) < 0.01 && (slope(Ease::In, 1.0 - 1e-3) - 1.0).abs() < 0.01);
        assert!((slope(Ease::Out, 1e-3) - 1.0).abs() < 0.01 && slope(Ease::Out, 1.0 - 1e-3) < 0.01);
        assert!(slope(Ease::InOut, 1e-3) < 0.01 && slope(Ease::InOut, 1.0 - 1e-3) < 0.01);
        assert_eq!(Ease::Linear.apply(0.3), 0.3);
        assert_eq!(Ease::In.with(false, true), Ease::InOut);
        assert_eq!(Ease::Linear.with(false, false), Ease::Linear);
        assert_eq!(Ease::Out.with(true, false), Ease::InOut);
    }

    fn line() -> Track {
        let mut track = Track::default();
        track.curve = Curve::Linear;
        track.ease_ends = false;
        for x in [0.0, 10.0, 20.0] {
            track.append(view(Vec3::X * x, Vec3::new(x + 1.0, 0.0, 0.0), 1.0));
        }
        track
    }

    #[test]
    fn an_eased_segment_keeps_its_path_its_time_and_its_mean_speed() {
        let mut track = line();
        let plain = track.clone();
        track.set_shot(
            1,
            KeyShot {
                ease: Ease::In,
                ..Default::default()
            },
        );
        // the keyframes stay where and when they are, the length too
        for key in track.keys() {
            assert!(track.sample(key.time).expect("pose").position.distance(key.pose.position) < 1e-4);
        }
        assert!((track.length() - plain.length()).abs() < 1e-3);
        assert_eq!(track.span_speed(1), plain.span_speed(1));
        // it starts slowly, then goes on at the line's speed
        let x = |track: &Track, t: f32| track.sample(t).expect("pose").position.x;
        assert!(x(&track, 0.3) < x(&plain, 0.3) * 0.25);
        let speed = |t: f32| (x(&track, t + 0.01) - x(&track, t - 0.01)) / 0.02;
        assert!((speed(2.99) - speed(3.01)).abs() < 0.1, "{} {}", speed(2.99), speed(3.01));
        // the other segment is not touched
        assert!((x(&track, 4.5) - x(&plain, 4.5)).abs() < 1e-4);
        // the view turns with the camera: eased along with it
        let (_, _, turned) = track.around(0.3).expect("span");
        let (_, _, plain_turned) = plain.around(0.3).expect("span");
        assert!(turned < plain_turned);
    }

    #[test]
    fn the_easing_composes_with_the_pace_the_even_speed_and_the_stops() {
        let mut track = line();
        track.curve = Curve::Smooth;
        track.set_rest(1, true);
        track.set_shot(
            2,
            KeyShot {
                ease: Ease::Out,
                ..Default::default()
            },
        );
        // the stop still stops, the segments still go through their keyframes
        let at = track.keys()[1].pose.position;
        assert!(track.sample(3.0).expect("pose").position.distance(at) < 1e-4);
        assert!(track.sample(3.05).expect("pose").position.distance(at) < 0.05);
        // a span's speed is still set by its time
        let speed = track.set_span_speed(2, 5.0).expect("speed");
        assert!((speed - 5.0).abs() < 0.1, "{speed}");
        assert!((track.span_time(2).expect("time") - 2.0).abs() < 0.1);
        // even speed: the same times as without the easing
        let mut plain = track.clone();
        plain.set_shot(2, KeyShot::default());
        track.even_speed();
        plain.even_speed();
        assert_eq!(
            track.keys().iter().map(|k| k.time).collect::<Vec<_>>(),
            plain.keys().iter().map(|k| k.time).collect::<Vec<_>>()
        );
        // and it comes to rest on the last keyframe
        let end = track.duration();
        let last = track.keys()[2].pose.position;
        assert!(track.sample(end - 0.05).expect("pose").position.distance(last) < 0.02);
    }

    #[test]
    fn a_dolly_zoom_keeps_the_subject_the_same_size_all_the_way() {
        let subject = Vec3::new(0.0, 30.0, 22.0);
        let start = Vec3::new(0.0, 0.0, 22.0);
        let made = make(
            &Shot {
                distance: 15.0,
                ..Shot::new(Kind::DollyZoom)
            },
            &Here {
                view: view(start, subject, 0.5),
                centre: subject,
                on_target: false,
            },
        )
        .expect("shot");
        assert_eq!(made.keys.len(), 2);
        assert!(made.keys[1].shot.dolly_zoom);
        let size = |pose: &Pose| pose.position.distance(subject) * (pose.fov * 0.5).tan();
        let k = size(&made.keys[0].pose);
        assert!((size(&made.keys[1].pose) - k).abs() < 1e-3 * k);
        assert!(made.keys[1].pose.fov > 0.5, "closer, wider");
        let mut d = Director::default();
        lay(&mut d, &Shot::new(Kind::DollyZoom), &made, None).expect("laid");
        // between the keyframes too, whatever the easing
        for ease in Ease::ALL {
            let mut shot = d.track.keys()[1].shot;
            shot.ease = ease;
            d.track.set_shot(1, shot);
            for step in 0..=40 {
                let pose = d.track.sample(d.track.duration() * step as f32 / 40.0).expect("pose");
                assert!((size(&pose) - k).abs() < 2e-3 * k, "{ease:?} at {step}: {} {k}", size(&pose));
            }
        }
        // without the flag, the lens goes its own way between them
        d.track.set_shot(1, KeyShot::default());
        let pose = d.track.sample(d.track.duration() * 0.5).expect("pose");
        assert!((size(&pose) - k).abs() > 1e-2 * k);
    }

    #[test]
    fn a_dolly_zoom_stops_where_the_lens_reaches_its_limit() {
        let subject = Vec3::new(0.0, 10.0, 0.0);
        let (low, high) = (*crate::camera::FOV_RANGE.start(), *crate::camera::FOV_RANGE.end());
        // far away, a narrow lens: it can only narrow down to its limit
        let away = Shot {
            distance: 100.0,
            reverse: true,
            ..Shot::new(Kind::DollyZoom)
        };
        let from = Here {
            view: view(Vec3::ZERO, subject, 0.3),
            centre: subject,
            on_target: false,
        };
        let made = make(&away, &from).expect("shot");
        let end = made.keys[1].pose;
        assert!((end.fov - low).abs() < 1e-5);
        let size = |pose: &Pose| pose.position.distance(subject) * (pose.fov * 0.5).tan();
        assert!((size(&end) - size(&made.keys[0].pose)).abs() < 1e-3);
        // a lens already at its widest cannot come any closer
        let wide = Here {
            view: view(Vec3::ZERO, subject, high),
            ..from
        };
        assert_eq!(make(&Shot::new(Kind::DollyZoom), &wide), None);
        // nor a camera on its subject
        assert_eq!(make(&Shot::new(Kind::DollyZoom), &here(subject, subject)), None);
        // the lens of a still camera is left as it is
        let a = view(Vec3::ZERO, subject, 0.4);
        assert_eq!(dolly_fov(&a, &Pose { fov: 0.9, ..a }, Vec3::ZERO, 0.7), 0.7);
    }

    #[test]
    fn an_orbit_is_a_circle_round_its_centre_at_one_speed() {
        let centre = Vec3::new(50.0, 50.0, 21.0);
        let eye = Vec3::new(58.0, 50.0, 23.0);
        let mut d = Director::default();
        let shot = Shot::new(Kind::Orbit);
        let made = make(&shot, &here(eye, centre)).expect("shot");
        assert_eq!(made.keys.len(), 25);
        lay(&mut d, &shot, &made, None).expect("laid");
        // it starts where the camera is
        assert!(d.track.keys()[0].pose.position.distance(eye) < 1e-3);
        assert!(d.track.keys()[24].pose.position.distance(eye) < 1e-3, "a whole turn");
        let samples = 400;
        let mut speeds = Vec::new();
        let mut last = d.track.sample(0.0).expect("pose").position;
        for step in 1..=samples {
            let t = d.track.duration() * step as f32 / samples as f32;
            let pose = d.track.sample(t).expect("pose");
            let flat = Vec2::new(pose.position.x - centre.x, pose.position.y - centre.y).length();
            // within a few millimetres of the circle; the end segments, whose
            // ends have only one neighbour to take their direction from, a
            // few centimetres
            let span = d.track.duration() / 24.0;
            let end = t < span || t > d.track.duration() - span;
            let within = if end { 0.01 } else { 0.001 };
            assert!((flat - 8.0).abs() < 8.0 * within, "radius {flat} at {t}");
            assert!((pose.position.z - 23.0).abs() < 1e-3);
            assert!(looks_at(&pose, centre) < 1.0, "it looks at the centre");
            speeds.push(pose.position.distance(last));
            last = pose.position;
        }
        // one speed, but for the soft start and stop of its first and last
        // segments
        let middle = &speeds[samples / 8..samples * 7 / 8];
        let (low, high) = middle.iter().fold((f32::MAX, 0.0_f32), |(a, b), s| (a.min(*s), b.max(*s)));
        assert!(high < low * 1.03, "{low} {high}");
        assert!(speeds[0] < low * 0.2);
        assert_eq!(d.track.keys()[1].shot.ease, Ease::In);
        assert_eq!(d.track.keys()[24].shot.ease, Ease::Out);
        assert!(!d.track.ease_ends && d.track.curve == Curve::Smooth);
        // clockwise seen from above: the other way round
        let made = make(
            &Shot {
                clockwise: true,
                arc: PI / 2.0,
                ..shot
            },
            &here(eye, centre),
        )
        .expect("shot");
        assert_eq!(made.keys.len(), 7);
        assert!(made.keys[6].pose.position.distance(Vec3::new(50.0, 42.0, 23.0)) < 1e-3);
        // a radius and a height of its own, and a climb
        let made = make(
            &Shot {
                radius: Some(3.0),
                height: Some(0.0),
                rise: 6.0,
                ..shot
            },
            &here(eye, centre),
        )
        .expect("shot");
        assert!(made.keys[0].pose.position.distance(Vec3::new(53.0, 50.0, 21.0)) < 1e-3);
        assert!((made.keys[24].pose.position.z - 27.0).abs() < 1e-3);
    }

    #[test]
    fn a_shot_follows_the_track_or_replaces_it_and_is_one_undo_step() {
        let mut d = Director::default();
        for x in [0.0, 10.0] {
            d.add_key(view(Vec3::new(x, 0.0, 20.0), Vec3::new(x, 10.0, 20.0), 1.0));
        }
        let mut history = History::default();
        history.observe(&d, true);
        let before = d.track.clone();
        let from = here(Vec3::new(0.0, 0.0, 20.0), Vec3::new(0.0, 6.0, 20.0));
        let crane = Shot::new(Kind::Crane);
        let made = make(&crane, &from).expect("shot");
        let first = lay(&mut d, &crane, &made, None).expect("laid");
        assert_eq!(first, 2);
        assert_eq!((d.selected, d.time), (Some(2), 3.0 + track::DEFAULT_GAP));
        assert_eq!(d.track.len(), 2 + made.keys.len());
        // laid with the lock off, its keyframes follow the lock, like any
        assert!(d.track.keys()[2..].iter().all(|k| k.aim == KeyAim::Lock));
        // the track's smooth end is still on: no easing of its own
        assert!(d.track.ease_ends && d.track.keys().iter().all(|k| k.shot.ease.is_linear()));
        history.observe(&d, true);
        assert!(history.undo(&mut d));
        assert_eq!(d.track, before, "one step");
        // in place of the track
        let replace = Shot { replace: true, ..crane };
        lay(&mut d, &replace, &made, None).expect("laid");
        assert_eq!(d.track.len(), made.keys.len());
        assert_eq!((d.selected, d.time), (Some(0), 0.0));
        // too many keyframes for the track: nothing changes
        let full = Made {
            keys: vec![made.keys[0]; track::MAX_KEYS],
            ..made.clone()
        };
        let kept = d.track.clone();
        assert_eq!(lay(&mut d, &crane, &full, None), Err(Refused::Full));
        let long = Made {
            keys: vec![
                made.keys[0],
                ShotKey {
                    time: track::MAX_TIME,
                    ..made.keys[1]
                },
            ],
            ..made.clone()
        };
        assert_eq!(lay(&mut d, &crane, &long, None), Err(Refused::TooLong));
        assert_eq!(d.track, kept);
    }

    #[test]
    fn the_straight_shots_go_where_they_say() {
        let centre = Vec3::new(0.0, 10.0, 20.0);
        let eye = Vec3::new(0.0, 0.0, 20.0);
        let from = here(eye, centre);
        let end = |kind: Kind, change: fn(Shot) -> Shot| {
            let made = make(&change(Shot::new(kind)), &from).expect("shot");
            *made.keys.last().expect("key")
        };
        // a crane rises and keeps looking at the centre, all the way
        let made = make(&Shot::new(Kind::Crane), &from).expect("shot");
        assert!(made.keys.len() > 2);
        let mut d = Director::default();
        lay(&mut d, &Shot::new(Kind::Crane), &made, None).expect("laid");
        for step in 0..=30 {
            let pose = d.track.sample(d.track.duration() * step as f32 / 30.0).expect("pose");
            assert!(looks_at(&pose, centre) < 1.5, "{step}: {}", looks_at(&pose, centre));
        }
        assert!(end(Kind::Crane, |s| s).pose.position.distance(eye + Vec3::Z * 4.0) < 1e-4);
        // a tracking shot goes to the right, or to the left, turned as it was
        let right = end(Kind::Truck, |s| s).pose;
        assert!(right.position.distance(Vec3::new(4.0, 0.0, 20.0)) < 1e-4, "{}", right.position);
        assert!(right.rotation.angle_between(from.view.rotation) < 1e-4);
        let left = end(Kind::Truck, |s| Shot { reverse: true, ..s }).pose;
        assert!(left.position.distance(Vec3::new(-4.0, 0.0, 20.0)) < 1e-4);
        // in and out along the view, never past the centre
        assert!(end(Kind::PushIn, |s| s).pose.position.distance(Vec3::new(0.0, 3.0, 20.0)) < 1e-4);
        let far = end(Kind::PushIn, |s| Shot { distance: 50.0, ..s }).pose;
        assert!(far.position.distance(Vec3::new(0.0, 9.5, 20.0)) < 1e-4);
        assert!(end(Kind::PullOut, |s| s).pose.position.distance(Vec3::new(0.0, -3.0, 20.0)) < 1e-4);
        // a fly-over passes over the centre and on to its other side, free
        let made = make(&Shot::new(Kind::FlyOver), &from).expect("shot");
        assert!(!made.aimed && made.curved);
        assert!(made.keys[1].pose.position.distance(Vec3::new(0.0, 10.0, 23.0)) < 1e-4);
        assert!(made.keys[2].pose.position.distance(Vec3::new(0.0, 20.0, 20.0)) < 1e-4);
        assert!(made.keys[1].pose.forward().z < -0.8, "looking down over it");
        let mut d = Director::default();
        d.lock.on = true;
        lay(&mut d, &Shot::new(Kind::FlyOver), &made, None).expect("laid");
        assert!(d.track.keys().iter().all(|k| k.aim == KeyAim::Free));
        // nowhere to go: no shot
        assert_eq!(make(&Shot::new(Kind::Orbit), &here(Vec3::splat(f32::NAN), centre)), None);
    }

    fn walking() -> (Director, Aim) {
        let mut d = Director::default();
        d.lock.on = true;
        d.lock.target = Target::Avatar(uuid::Uuid::from_u128(5));
        d.lock.joint = Joint::Head;
        let aim = d.lock.aim();
        (d, aim)
    }

    #[test]
    fn an_orbit_round_a_walking_avatar_travels_with_it() {
        let (mut d, aim) = walking();
        let target = Vec3::new(10.0, 10.0, 21.0);
        let from = Here {
            on_target: true,
            ..here(Vec3::new(16.0, 10.0, 22.0), target)
        };
        let shot = Shot::new(Kind::Orbit);
        let made = make(&shot, &from).expect("shot");
        lay(&mut d, &shot, &made, Some(target)).expect("laid");
        assert!(
            d.track
                .keys()
                .iter()
                .all(|k| k.aim == KeyAim::On(aim) && k.shot.anchor == Some(target))
        );
        // the avatar walked 20 m east: the orbit goes round where it is now
        let now = target + Vec3::X * 20.0;
        let find = |found: Aim| (found == aim).then_some(now);
        for step in 0..=24 {
            d.scrub(d.track.duration() * step as f32 / 24.0);
            let pose = d.view(find).expect("view");
            let flat = Vec2::new(pose.position.x - now.x, pose.position.y - now.y).length();
            assert!((flat - 6.0).abs() < 0.05, "{flat} at {step}");
            assert!(looks_at(&pose, now) < 0.1);
        }
        // the avatar out of sight: where it was laid
        d.scrub(0.0);
        assert!(d.view(|_| None).expect("view").position.distance(Vec3::new(16.0, 10.0, 22.0)) < 1e-3);
        // without the travel, the keyframes stay where they were laid
        let mut d2 = walking().0;
        lay(&mut d2, &Shot { travel: false, ..shot }, &made, Some(target)).expect("laid");
        assert!(d2.track.keys().iter().all(|k| k.shot.anchor.is_none()));
    }

    #[test]
    fn a_world_keyframe_and_a_following_one_join_smoothly() {
        let (mut d, aim) = walking();
        let target = Vec3::new(0.0, 10.0, 20.0);
        d.add_key(view(Vec3::new(0.0, 0.0, 20.0), target, 1.0));
        d.add_key(view(Vec3::new(10.0, 0.0, 20.0), target, 1.0));
        d.track.set_shot(
            1,
            KeyShot {
                anchor: Some(target),
                ..Default::default()
            },
        );
        let now = target + Vec3::Y * 4.0;
        let find = |found: Aim| (found == aim).then_some(now);
        let at = |d: &mut Director, t: f32| {
            d.scrub(t);
            d.view(find).expect("view").position
        };
        // the world one where it was laid, the following one moved with its
        // target, and no jump anywhere in between
        assert!(at(&mut d, 0.0).distance(Vec3::new(0.0, 0.0, 20.0)) < 1e-4);
        assert!(at(&mut d, 3.0).distance(Vec3::new(10.0, 4.0, 20.0)) < 1e-4);
        let mut last = at(&mut d, 0.0);
        for step in 1..=300 {
            let now = at(&mut d, 3.0 * step as f32 / 300.0);
            assert!(now.distance(last) < 0.1, "{step}");
            last = now;
        }
    }

    #[test]
    fn the_shots_fields_come_back_from_the_file_and_older_files_still_load() {
        let mut track = line();
        let anchor = Vec3::new(5.0, 6.0, 7.0);
        track.set_shot(
            1,
            KeyShot {
                ease: Ease::InOut,
                dolly_zoom: true,
                anchor: Some(anchor),
            },
        );
        track.set_shot(
            2,
            KeyShot {
                ease: Ease::Out,
                ..Default::default()
            },
        );
        let origin = (256_000, 512_000);
        let neck = Aim {
            target: Target::Own,
            joint: Joint::Neck,
        };
        let file = track.to_file(origin);
        assert_eq!(file.version, track::FILE_VERSION, "no new version");
        let text = serde_json::to_string(&file).expect("json");
        assert!(text.contains("\"ease\":\"in_out\"") && text.contains("\"dolly_zoom\":true"));
        let back = Track::from_file(&serde_json::from_str(&text).expect("json"), origin, neck).expect("track");
        let shots: Vec<KeyShot> = back.keys().iter().map(|k| k.shot).collect();
        assert_eq!(shots[0], KeyShot::default());
        assert_eq!((shots[1].ease, shots[1].dolly_zoom), (Ease::InOut, true));
        assert!(shots[1].anchor.expect("anchor").distance(anchor) < 1e-4);
        assert_eq!(shots[2].ease, Ease::Out);
        // the plain keyframes write none of it
        assert_eq!(text.matches("\"ease\"").count(), 2);
        // moved with the region, like the keyframes
        let east = Track::from_file(&file, (origin.0 + 256, origin.1), neck).expect("track");
        assert!(east.keys()[1].shot.anchor.expect("anchor").distance(anchor - Vec3::X * 256.0) < 1e-3);
        let mut shifted = back.clone();
        shifted.shift(Vec3::Y * 10.0);
        assert!(shifted.keys()[1].shot.anchor.expect("anchor").distance(anchor + Vec3::Y * 10.0) < 1e-4);
        // a file of before: none of them; an easing a newer viewer wrote: linear
        let old = r#"{"version":2,"keys":[{"time":0.0,"position":[1.0,2.0,3.0],"rotation":[0.0,0.0,0.0,1.0],"fov":1.0},{"time":3.0,"position":[4.0,2.0,3.0],"rotation":[0.0,0.0,0.0,1.0],"fov":1.0,"ease":"bounce"}]}"#;
        let old: TrackFile = serde_json::from_str(old).expect("json");
        let old = Track::from_file(&old, (0, 0), neck).expect("track");
        assert!(old.keys().iter().all(|k| k.shot == KeyShot::default()));
    }
}
