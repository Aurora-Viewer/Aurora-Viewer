//! A path taken by hand for the Director: while "Record my path" is on, the
//! live view (whatever camera gives it: alt-camera, free camera, flycam) is
//! kept twenty times a second with its time; once it stops, the take becomes
//! keyframes: only those the path needs to stay within a tolerance of what
//! was flown (Ramer-Douglas-Peucker, on the place, the turn and the lens
//! alike), at the times they were flown, so the track plays the hand-held
//! move back at its own pace.
//!
//! Aurora's own: the Legacy Viewer has no camera path tool.

use crate::camera::track::{self, Keyframe, Pose};
use crate::director::KeyAim;
use crate::director_keys::KeyExtra;
use glam::Vec3;

/// Views kept a second while the path is taken.
pub const SAMPLE_RATE: f32 = 20.0;
/// The "detail" slider's default: halfway between the loosest and the
/// finest tolerance.
pub const DEFAULT_DETAIL: f32 = 0.5;

/// One view of the take, `time` seconds after it started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub time: f32,
    pub pose: Pose,
}

/// The views of a take, in the order they were flown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Take {
    samples: Vec<Sample>,
}

impl Take {
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// Seconds from the first view to the last.
    pub fn duration(&self) -> f32 {
        match (self.samples.first(), self.samples.last()) {
            (Some(first), Some(last)) => last.time - first.time,
            _ => 0.0,
        }
    }

    /// The view at `time` seconds into the take: kept when a twentieth of a
    /// second has gone by since the last one. False once the take is as long
    /// as a track may be (the take is full).
    pub fn sample(&mut self, time: f32, pose: Pose) -> bool {
        if !time.is_finite() || !pose.position.is_finite() || !pose.rotation.is_finite() {
            return true;
        }
        if self.duration() >= track::MAX_TIME {
            return false;
        }
        let due = self.samples.last().is_none_or(|last| time - last.time >= 1.0 / SAMPLE_RATE - 1e-4);
        if due {
            self.samples.push(Sample { time, pose });
        }
        true
    }

    /// The region positions are relative to changed (`Camera::shift`).
    pub fn shift(&mut self, delta: Vec3) {
        for sample in &mut self.samples {
            sample.pose.position += delta;
        }
    }
}

/// How far the path played back may stray from the take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Metres.
    pub position: f32,
    /// Radians of the view's turn.
    pub angle: f32,
    /// Radians of the view angle.
    pub fov: f32,
}

impl Tolerance {
    /// The tolerance of the "detail" slider: 0, the loosest (30 cm, 6°),
    /// to 1, the finest (2 cm, half a degree); evenly in ratio between.
    pub fn from_detail(detail: f32) -> Tolerance {
        let detail = if detail.is_finite() {
            detail.clamp(0.0, 1.0)
        } else {
            DEFAULT_DETAIL
        };
        let between = |loose: f32, fine: f32| loose * (fine / loose).powf(detail);
        Tolerance {
            position: between(0.30, 0.02),
            angle: between(6f32.to_radians(), 0.5f32.to_radians()),
            fov: between(0.05, 0.005),
        }
    }

    fn scaled(self, by: f32) -> Tolerance {
        Tolerance {
            position: self.position * by,
            angle: self.angle * by,
            fov: self.fov * by,
        }
    }
}

/// How far sample `k` is from the path straight from `i` to `j` at its own
/// time, in tolerances (over 1: too far).
fn error(samples: &[Sample], i: usize, j: usize, k: usize, tol: Tolerance) -> f32 {
    let (a, b, at) = (samples[i], samples[j], samples[k]);
    let span = b.time - a.time;
    let s = if span > 0.0 {
        ((at.time - a.time) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let position = at.pose.position.distance(a.pose.position.lerp(b.pose.position, s));
    let angle = at.pose.rotation.angle_between(a.pose.rotation.slerp(b.pose.rotation, s));
    let fov = (at.pose.fov - (a.pose.fov + (b.pose.fov - a.pose.fov) * s)).abs();
    (position / tol.position).max(angle / tol.angle).max(fov / tol.fov)
}

/// The samples the path keeps (their indices, the first and the last
/// included): the fewest that leave every other within `tol` of the path
/// between its neighbours, at its own time, so the pace stays the one
/// flown. Each at least `MIN_GAP` after the one before: a span is split at
/// its farthest view that leaves both halves that long, so a span shorter
/// than two gaps is kept as it is, its views however far.
pub fn simplify(samples: &[Sample], tol: Tolerance) -> Vec<usize> {
    let n = samples.len();
    if n < 2 {
        return (0..n).collect();
    }
    let gap = track::MIN_GAP * 1.01;
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    // (a stack, not a recursion: an hour's take has 72 000 views)
    let mut spans = vec![(0, n - 1)];
    while let Some((i, j)) = spans.pop() {
        if j <= i + 1 {
            continue;
        }
        let (mut far, mut split) = (0.0_f32, None::<(usize, f32)>);
        for k in i + 1..j {
            let e = error(samples, i, j, k, tol);
            far = far.max(e);
            let room = samples[k].time - samples[i].time >= gap && samples[j].time - samples[k].time >= gap;
            if room && split.is_none_or(|(_, worst)| e > worst) {
                split = Some((k, e));
            }
        }
        if far > 1.0
            && let Some((k, _)) = split
        {
            keep[k] = true;
            spans.push((i, k));
            spans.push((k, j));
        }
    }
    let kept: Vec<usize> = (0..n).filter(|&k| keep[k]).collect();
    spaced(samples, kept)
}

/// The kept samples at least `MIN_GAP` apart: one too close to the one
/// before goes (the last one stays: it is where the take ends, and the one
/// before it goes instead).
fn spaced(samples: &[Sample], kept: Vec<usize>) -> Vec<usize> {
    let gap = track::MIN_GAP * 1.01;
    let mut out: Vec<usize> = Vec::with_capacity(kept.len());
    let last = kept.last().copied();
    for k in kept {
        let close = out.last().is_some_and(|&before| samples[k].time - samples[before].time < gap);
        if !close {
            out.push(k);
        } else if Some(k) == last {
            if out.len() > 1 {
                out.pop();
            }
            if out.last().is_none_or(|&before| samples[k].time - samples[before].time >= gap) {
                out.push(k);
            }
        }
    }
    out
}

/// The take as keyframes, the first at `start` seconds of the track: at
/// most `room` of them (the tolerance loosened until they fit), looking
/// where the take looked (`aim`). Empty for a take too short to play (under
/// two keyframes).
pub fn keyframes(take: &Take, tol: Tolerance, room: usize, start: f32, aim: KeyAim) -> Vec<Keyframe> {
    let samples = take.samples();
    if room < 2 || take.duration() < track::MIN_GAP {
        return Vec::new();
    }
    let mut tol = tol;
    let mut kept = simplify(samples, tol);
    // (each round doubles it: a few rounds fit any take)
    for _ in 0..24 {
        if kept.len() <= room {
            break;
        }
        tol = tol.scaled(2.0);
        kept = simplify(samples, tol);
    }
    if kept.len() > room || kept.len() < 2 {
        return Vec::new();
    }
    let first = samples[kept[0]].time;
    kept.into_iter()
        .map(|k| Keyframe {
            time: start + samples[k].time - first,
            pose: samples[k].pose,
            rest: false,
            aim,
            shot: Default::default(),
            extra: KeyExtra::default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::{Curve, Track};

    /// A hand-held move: a slow then fast walk along a curve, the view
    /// turning and the lens zooming a little, with a small shake.
    fn walk(seconds: f32) -> Take {
        let mut take = Take::default();
        let mut time = 0.0;
        while time <= seconds {
            let pace = time * 0.5 + time * time * 0.08;
            let shake = (time * 17.0).sin() * 0.004;
            let position = Vec3::new(pace * 2.0, (pace * 0.6).sin() * 3.0 + shake, 22.0 + (time * 0.7).sin() * 0.5);
            let pose = Pose::from_angles(position, time * 0.2, (time * 0.3).sin() * 0.1, 0.0, 1.0 - time * 0.01);
            assert!(take.sample(time, pose));
            time += 1.0 / 60.0;
        }
        take
    }

    fn played(keys: Vec<Keyframe>, curve: Curve) -> Track {
        let mut track = Track::default();
        track.curve = curve;
        track.ease_ends = false;
        track.push_keys(keys);
        track
    }

    #[test]
    fn a_take_keeps_twenty_views_a_second() {
        let take = walk(2.0);
        let count = take.samples().len();
        assert!((40..=42).contains(&count), "{count}");
        assert!(take.samples().windows(2).all(|w| w[1].time - w[0].time >= 0.049));
        // a view that is no number is skipped, not kept
        let mut take = Take::default();
        let pose = Pose::from_view(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X, Vec3::Z, 1.0);
        assert!(take.sample(0.0, pose));
        assert!(take.samples().is_empty());
        // a region crossing moves the views with the track
        let mut take = walk(1.0);
        let before = take.samples()[3].pose.position;
        take.shift(Vec3::new(-256.0, 0.0, 0.0));
        assert_eq!(take.samples()[3].pose.position, before + Vec3::new(-256.0, 0.0, 0.0));
    }

    #[test]
    fn the_path_kept_stays_within_its_tolerance_at_the_times_flown() {
        let take = walk(12.0);
        for detail in [0.0, 0.5, 1.0] {
            let tol = Tolerance::from_detail(detail);
            let kept = simplify(take.samples(), tol);
            assert_eq!(kept.first(), Some(&0));
            assert_eq!(kept.last(), Some(&(take.samples().len() - 1)));
            assert!(kept.len() < take.samples().len() / 3, "{detail}: {} keys", kept.len());
            // every view of the take, on the straight path between the kept
            // ones at its own time: within the tolerance (where the span is
            // long enough to be split, two gaps and a view)
            for pair in kept.windows(2) {
                let span = take.samples()[pair[1]].time - take.samples()[pair[0]].time;
                if span < 2.0 * track::MIN_GAP * 1.01 + 1.0 / SAMPLE_RATE {
                    continue;
                }
                for k in pair[0]..=pair[1] {
                    assert!(error(take.samples(), pair[0], pair[1], k, tol) <= 1.0 + 1e-4);
                }
            }
            // and as the track plays it, on its smooth curve: close to it too
            let keys = keyframes(&take, tol, track::MAX_KEYS, 0.0, KeyAim::Free);
            let track = played(keys, Curve::Smooth);
            for sample in take.samples() {
                let pose = track.sample(sample.time).expect("pose");
                assert!(
                    pose.position.distance(sample.pose.position) <= tol.position * 2.5,
                    "{detail}: {} m at {} s",
                    pose.position.distance(sample.pose.position),
                    sample.time
                );
                assert!(pose.rotation.angle_between(sample.pose.rotation) <= tol.angle * 2.5);
            }
        }
        // more detail, more keyframes
        let count = |detail| simplify(take.samples(), Tolerance::from_detail(detail)).len();
        assert!(count(1.0) > count(0.5) && count(0.5) > count(0.0));
    }

    #[test]
    fn the_keyframes_keep_the_take_timing() {
        let take = walk(8.0);
        let keys = keyframes(&take, Tolerance::from_detail(0.5), track::MAX_KEYS, 0.0, KeyAim::Free);
        let duration = take.duration();
        assert_eq!(keys.first().map(|k| k.time), Some(0.0));
        assert!((keys.last().expect("key").time - duration).abs() < 1e-4);
        assert!(keys.windows(2).all(|w| w[1].time - w[0].time >= track::MIN_GAP));
        assert!(keys.iter().all(|k| k.aim == KeyAim::Free && !k.rest));
        // after an existing track: from `start`, its pace the same
        let after = keyframes(&take, Tolerance::from_detail(0.5), track::MAX_KEYS, 9.0, KeyAim::Lock);
        assert_eq!(after.len(), keys.len());
        for (a, b) in after.iter().zip(&keys) {
            assert!((a.time - b.time - 9.0).abs() < 1e-4);
        }
        // the camera is faster at the end of the walk: the keyframes say so
        let track = played(keys, Curve::Linear);
        let speed = |t: f32| {
            track
                .sample(t + 0.1)
                .expect("pose")
                .position
                .distance(track.sample(t).expect("pose").position)
        };
        assert!(speed(7.5) > speed(0.5) * 2.0);
    }

    #[test]
    fn a_take_fits_the_room_the_track_has() {
        let take = walk(30.0);
        let fine = Tolerance::from_detail(1.0);
        let many = keyframes(&take, fine, track::MAX_KEYS, 0.0, KeyAim::Free);
        let few = keyframes(&take, fine, 12, 0.0, KeyAim::Free);
        assert!(many.len() > 12, "{}", many.len());
        assert!((2..=12).contains(&few.len()));
        // its ends stay where the take started and ended
        assert_eq!(few[0].pose, take.samples()[0].pose);
        assert_eq!(few.last().expect("key").pose, take.samples().last().expect("view").pose);
        // no room, or a take too short to play: nothing
        assert!(keyframes(&take, fine, 1, 0.0, KeyAim::Free).is_empty());
        let mut short = Take::default();
        short.sample(0.0, take.samples()[0].pose);
        short.sample(0.06, take.samples()[1].pose);
        assert!(keyframes(&short, fine, track::MAX_KEYS, 0.0, KeyAim::Free).is_empty());
        assert!(keyframes(&Take::default(), fine, track::MAX_KEYS, 0.0, KeyAim::Free).is_empty());
    }

    #[test]
    fn a_still_camera_is_two_keyframes() {
        let mut take = Take::default();
        let pose = Pose::from_view(Vec3::new(5.0, 5.0, 22.0), Vec3::X, Vec3::Z, 1.0);
        for step in 0..100 {
            take.sample(step as f32 * 0.05, pose);
        }
        let keys = keyframes(&take, Tolerance::from_detail(1.0), track::MAX_KEYS, 0.0, KeyAim::Free);
        assert_eq!(keys.len(), 2);
        assert!((keys[1].time - 4.95).abs() < 1e-3);
    }

    #[test]
    fn kept_views_too_close_together_are_spaced() {
        let pose = Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, 1.0);
        let samples: Vec<Sample> = [0.0, 0.05, 0.11, 0.3, 0.35].iter().map(|&time| Sample { time, pose }).collect();
        let out = spaced(&samples, vec![0, 1, 2, 3, 4]);
        assert_eq!(out, [0, 2, 4]);
        for pair in out.windows(2) {
            assert!(samples[pair[1]].time - samples[pair[0]].time >= track::MIN_GAP);
        }
        // never fewer than the two ends of a take long enough
        assert_eq!(spaced(&samples, vec![0, 4]), [0, 4]);
    }

    #[test]
    fn a_take_is_full_at_the_longest_a_track_lasts() {
        let mut take = Take::default();
        let pose = Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, 1.0);
        assert!(take.sample(0.0, pose));
        assert!(take.sample(track::MAX_TIME, pose));
        assert!(!take.sample(track::MAX_TIME + 1.0, pose));
        assert_eq!(take.samples().len(), 2);
    }
}
