//! The Director's camera track: keyframes of the view (where the camera is,
//! how it is turned, its lens) laid on a timeline, and the path the camera
//! flies between them when the track plays.
//!
//! Aurora's own: the Legacy Viewer has no camera path tool. A keyframe keeps
//! the pose of its flycam as it is (LLViewerJoystick::moveFlycam,
//! indra/newview/llviewerjoystick.cpp: a position, a full rotation and a
//! view angle), so a view taken from any camera (alt-camera, free camera,
//! flycam, a tilted viewfinder) comes back exactly.
//!
//! Between two keyframes the position, the rotation and the lens follow a
//! cubic Hermite curve whose tangents come from the neighbours: a
//! Catmull-Rom spline on a timeline that need not be even. The path goes
//! through every keyframe and its speed never jumps at one. Each span is
//! also a cubic Bézier curve (`Track::handles`), which is what the world
//! overlay draws.

use crate::director::shots::{Ease, KeyShot};
use crate::director::{Aim, Joint, KeyAim, Target};
use crate::director_keys::{ExtraFile, KeyExtra};
use crate::joystick::flycam::Flycam;
use glam::{Quat, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Sub};

/// Seconds between two keyframes added one after the other.
pub const DEFAULT_GAP: f32 = 3.0;
/// Two keyframes are never closer than this on the timeline.
pub const MIN_GAP: f32 = 0.1;
/// A track holds this many keyframes at most.
pub const MAX_KEYS: usize = 200;
/// The last keyframe is never later than this (seconds).
pub const MAX_TIME: f32 = 3600.0;
/// The version of the tracks' files (`TrackFile`): 2 gives each keyframe
/// what it looks at (`KeyFile::aim`); a file of 1 says only whether it is
/// locked.
pub const FILE_VERSION: u32 = 2;
/// A hand-held camera at its most: how far it turns (radians) and drifts
/// (metres) around the view it is given.
const HAND_TURN: f32 = 0.02;
const HAND_DRIFT: f32 = 0.04;
/// Straight pieces a span is cut into, to measure it.
const MEASURE_STEPS: usize = 16;
/// Under this many metres between two keyframes the camera turns or zooms
/// on the spot: the span has a time, no speed to set.
pub const MIN_TRAVEL: f32 = 0.01;

fn clamp_fov(fov: f32) -> f32 {
    let (low, high) = (*super::FOV_RANGE.start(), *super::FOV_RANGE.end());
    if fov.is_finite() {
        fov.clamp(low, high)
    } else {
        super::DEFAULT_FOV
    }
}

/// A view: where the camera is, how it is turned, its lens (render space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    /// Local +X forward, +Y left and +Z up, as the flycam's.
    pub rotation: Quat,
    /// Vertical view angle, radians.
    pub fov: f32,
}

impl Pose {
    pub fn from_view(position: Vec3, forward: Vec3, up: Vec3, fov: f32) -> Pose {
        Pose {
            position,
            rotation: Flycam::from_direction(position, forward, up, fov).rotation,
            fov: clamp_fov(fov),
        }
    }

    /// From a heading (radians, 0 along +X, counter-clockwise), a pitch
    /// (positive looking up) and a roll (as `Camera::roll`).
    pub fn from_angles(position: Vec3, yaw: f32, pitch: f32, roll: f32, fov: f32) -> Pose {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let forward = Vec3::new(cy * cp, sy * cp, sp);
        let up = Quat::from_axis_angle(forward, roll) * super::view_reference_up(forward);
        Pose::from_view(position, forward, up, fov)
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn left(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Z
    }

    /// Roll of the view around its direction, counted as `Camera::roll`.
    pub fn roll(&self) -> f32 {
        Flycam::from_direction(self.position, self.forward(), self.up(), self.fov).roll()
    }

    /// Heading, pitch and roll, radians (see `from_angles`).
    pub fn angles(&self) -> (f32, f32, f32) {
        let f = self.forward();
        (f.y.atan2(f.x), f.z.clamp(-1.0, 1.0).asin(), self.roll())
    }

    /// This view turned towards `target` by `weight` (0: as it is, 1: the
    /// target in its centre), from where it stands and with the roll it
    /// has: the Director's lock on a target.
    pub fn aimed(&self, target: Vec3, weight: f32) -> Pose {
        let direction = target - self.position;
        let weight = if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 0.0 };
        if weight <= 0.0 || !direction.is_finite() || direction.length_squared() < 1e-4 {
            return *self;
        }
        let direction = direction.normalize();
        let up = Quat::from_axis_angle(direction, self.roll()) * super::view_reference_up(direction);
        let aimed = Flycam::from_direction(self.position, direction, up, self.fov).rotation;
        Pose {
            rotation: self.rotation.slerp(aimed, weight).normalize(),
            ..*self
        }
    }
}

/// The view as a hand-held camera gives it `time` seconds into the shot: a
/// small drift of where it stands and how it is turned, by `amount` (0:
/// none, 1: the most). The same at the same time: a track played twice
/// moves alike. Sums of sines whose periods never line up within a shot.
pub fn hand_held(pose: Pose, time: f32, amount: f32) -> Pose {
    let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
    if amount <= 0.0 || !time.is_finite() {
        return pose;
    }
    let wave = |a: f32, b: f32, c: f32| (time * a).sin() * 0.5 + (time * b + 1.3).sin() * 0.3 + (time * c + 2.1).sin() * 0.2;
    let turn = HAND_TURN * amount;
    let (yaw, pitch, roll) = (
        wave(1.3, 2.9, 4.7) * turn,
        wave(1.7, 3.1, 5.3) * turn,
        wave(0.9, 2.3, 3.7) * turn * 0.5,
    );
    let drift = (pose.left() * wave(1.1, 2.7, 4.3) + pose.up() * wave(1.9, 3.3, 5.9)) * (HAND_DRIFT * amount);
    Pose {
        position: pose.position + drift,
        rotation: (pose.rotation * Quat::from_rotation_z(yaw) * Quat::from_rotation_y(-pitch) * Quat::from_rotation_x(roll)).normalize(),
        fov: pose.fov,
    }
}

/// A keyframe as a track's file keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyFile {
    pub time: f32,
    /// Global metres (the grid's): the track comes back where it was laid,
    /// whatever region the avatar is in when it is loaded.
    pub position: [f64; 3],
    pub rotation: [f32; 4],
    pub fov: f32,
    #[serde(default)]
    pub rest: bool,
    /// Written for the files of 1: the keyframe looks at a target.
    #[serde(default)]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aim: Option<AimFile>,
    /// The shots' fields (`director::shots::KeyShot`), none of them in a
    /// file written before them: the easing of the way to this keyframe,
    /// its dolly zoom, and where its target was when it was laid to travel
    /// with it (global metres).
    #[serde(default, skip_serializing_if = "Ease::is_linear")]
    pub ease: Ease,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dolly_zoom: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<[f64; 3]>,
    /// Its animation, lens and time of day (`director_keys`); absent when
    /// it sets none, and from older files.
    #[serde(default, skip_serializing_if = "ExtraFile::is_empty")]
    pub extra: ExtraFile,
}

/// What a keyframe looks at, as a track's file keeps it: the joint by its
/// bone's name, the avatar or the object by its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AimFile {
    Lock,
    Free,
    Own { joint: String },
    Avatar { id: String, joint: String },
    Object { id: String },
}

impl AimFile {
    fn of(aim: KeyAim) -> AimFile {
        let joint = |aim: Aim| aim.joint.bone().to_owned();
        match aim {
            KeyAim::Lock => AimFile::Lock,
            KeyAim::Free => AimFile::Free,
            KeyAim::On(aim) => match aim.target {
                Target::Own => AimFile::Own { joint: joint(aim) },
                Target::Avatar(id) => AimFile::Avatar {
                    id: id.to_string(),
                    joint: joint(aim),
                },
                Target::Object(id) => AimFile::Object { id: id.to_string() },
            },
        }
    }

    /// A file is data from outside: an id that is none, and the keyframe
    /// follows the lock; a joint unknown, and it looks at the neck.
    fn read(&self) -> KeyAim {
        let joint = |name: &str| Joint::ALL.into_iter().find(|j| j.bone() == name).unwrap_or_default();
        let on = |target, joint| KeyAim::On(Aim { target, joint });
        match self {
            AimFile::Lock => KeyAim::Lock,
            AimFile::Free => KeyAim::Free,
            AimFile::Own { joint: name } => on(Target::Own, joint(name)),
            AimFile::Avatar { id, joint: name } => match uuid::Uuid::parse_str(id) {
                Ok(id) => on(Target::Avatar(id), joint(name)),
                Err(_) => KeyAim::Lock,
            },
            AimFile::Object { id } => match uuid::Uuid::parse_str(id) {
                Ok(id) => on(Target::Object(id), Joint::default()),
                Err(_) => KeyAim::Lock,
            },
        }
    }
}

/// A track as its file keeps it (JSON, in the account's `director` folder).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackFile {
    pub version: u32,
    #[serde(default)]
    pub linear: bool,
    #[serde(default)]
    pub ease_ends: bool,
    pub keys: Vec<KeyFile>,
}

/// The name a track is saved under, as a file's: letters, digits, spaces,
/// `-` and `_` kept, anything else replaced, sixty characters at most.
/// `None` for a name that leaves nothing, or that a system reserves.
pub fn file_stem(name: &str) -> Option<String> {
    let stem: String = name
        .trim()
        .chars()
        .take(60)
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim().to_owned();
    let reserved = ["con", "prn", "aux", "nul"];
    let device = |s: &str| s.len() == 4 && (s.starts_with("com") || s.starts_with("lpt")) && s.ends_with(|c: char| c.is_ascii_digit());
    let lower = stem.to_lowercase();
    (!stem.is_empty() && !stem.chars().all(|c| c == '_') && !reserved.contains(&lower.as_str()) && !device(&lower)).then_some(stem)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    /// Seconds from the start of the track (the first keyframe is at 0).
    pub time: f32,
    pub pose: Pose,
    /// The camera comes to rest on this keyframe and leaves it from rest,
    /// instead of passing through at speed (the smooth curve only).
    pub rest: bool,
    /// What the view looks at here (`Director::key_aim`): between two
    /// keyframes it turns smoothly from what one looks at to the next's.
    pub aim: KeyAim,
    /// Its easing, its dolly zoom, the target it travels with.
    pub shot: KeyShot,
    /// The animation it starts or stops, its own lens and time of day.
    pub extra: KeyExtra,
}

/// How the camera goes from a keyframe to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Curve {
    /// A curve through the keyframes, no jump of speed at any.
    #[default]
    Smooth,
    /// Straight lines, a constant speed on each.
    Linear,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// By time, the first at 0, each rotation in the hemisphere of the one
    /// before (`tidy`).
    keys: Vec<Keyframe>,
    pub curve: Curve,
    /// The camera leaves the first keyframe from rest and comes to rest on
    /// the last, instead of starting and stopping at full speed.
    pub ease_ends: bool,
}

impl Default for Track {
    fn default() -> Self {
        Track {
            keys: Vec::new(),
            curve: Curve::Smooth,
            ease_ends: true,
        }
    }
}

fn hermite<T>(a: T, va: T, b: T, vb: T, span: f32, s: f32) -> T
where
    T: Copy + Add<Output = T> + Mul<f32, Output = T>,
{
    let (s2, s3) = (s * s, s * s * s);
    a * (2.0 * s3 - 3.0 * s2 + 1.0) + va * ((s3 - 2.0 * s2 + s) * span) + b * (3.0 * s2 - 2.0 * s3) + vb * ((s3 - s2) * span)
}

impl Track {
    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.keys.len() >= MAX_KEYS
    }

    /// Seconds from the first keyframe to the last.
    pub fn duration(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.time)
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    /// The keyframes back in order after a change: sorted, the first at 0,
    /// and each rotation written in the hemisphere of the one before (`q`
    /// and `-q` are the same rotation, but the curve between two keyframes
    /// must not go the long way round).
    fn tidy(&mut self) {
        self.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        let Some(first) = self.keys.first().map(|k| k.time) else {
            return;
        };
        for key in &mut self.keys {
            key.time -= first;
        }
        for i in 1..self.keys.len() {
            let before = self.keys[i - 1].pose.rotation;
            let rotation = &mut self.keys[i].pose.rotation;
            if before.dot(*rotation) < 0.0 {
                *rotation = -*rotation;
            }
        }
    }

    /// The track as its file keeps it; `origin`: the main region's corner
    /// in global metres (`World::main_origin`).
    pub fn to_file(&self, origin: (u32, u32)) -> TrackFile {
        TrackFile {
            version: FILE_VERSION,
            linear: self.curve == Curve::Linear,
            ease_ends: self.ease_ends,
            keys: self
                .keys
                .iter()
                .map(|key| KeyFile {
                    time: key.time,
                    position: super::presets::to_global(origin, key.pose.position),
                    rotation: key.pose.rotation.to_array(),
                    fov: key.pose.fov,
                    rest: key.rest,
                    locked: matches!(key.aim, KeyAim::On(_)),
                    aim: Some(AimFile::of(key.aim)),
                    ease: key.shot.ease,
                    dolly_zoom: key.shot.dolly_zoom,
                    follow: key.shot.anchor.map(|anchor| super::presets::to_global(origin, anchor)),
                    extra: ExtraFile::of(&key.extra),
                })
                .collect(),
        }
    }

    /// The track a file holds, for a main region at `origin`. A file is
    /// data from outside: `None` when it holds no keyframe, too many, or a
    /// value that is no number; times out of order or too close are put
    /// right, a view angle out of range brought in. A locked keyframe of a
    /// file of 1, which named no target, looks at `older` (the lock's now,
    /// as it did then).
    pub fn from_file(file: &TrackFile, origin: (u32, u32), older: Aim) -> Option<Track> {
        if file.keys.is_empty() || file.keys.len() > MAX_KEYS {
            return None;
        }
        let mut keys = Vec::with_capacity(file.keys.len());
        for key in &file.keys {
            let rotation = Quat::from_array(key.rotation);
            let sound = key.time.is_finite()
                && key.position.iter().all(|v| v.is_finite())
                && rotation.is_finite()
                && rotation.length_squared() > 1e-6
                && key.fov.is_finite();
            if !sound {
                return None;
            }
            keys.push(Keyframe {
                time: key.time.clamp(0.0, MAX_TIME),
                pose: Pose {
                    position: super::presets::from_global(origin, key.position),
                    rotation: rotation.normalize(),
                    fov: clamp_fov(key.fov),
                },
                rest: key.rest,
                aim: match &key.aim {
                    Some(aim) => aim.read(),
                    None if key.locked => KeyAim::On(older),
                    None => KeyAim::Lock,
                },
                shot: KeyShot {
                    ease: key.ease,
                    dolly_zoom: key.dolly_zoom,
                    anchor: key
                        .follow
                        .filter(|anchor| anchor.iter().all(|v| v.is_finite()))
                        .map(|anchor| super::presets::from_global(origin, anchor)),
                },
                extra: key.extra.read(),
            });
        }
        let mut track = Track {
            keys,
            curve: if file.linear { Curve::Linear } else { Curve::Smooth },
            ease_ends: file.ease_ends,
        };
        track.tidy();
        for i in 1..track.keys.len() {
            track.keys[i].time = track.keys[i].time.max(track.keys[i - 1].time + MIN_GAP);
        }
        Some(track)
    }

    /// A keyframe at `time`: a new one, or the view of the one already there
    /// (closer than `MIN_GAP`). Its index, or `None` when the track is full.
    pub fn insert(&mut self, time: f32, pose: Pose) -> Option<usize> {
        let time = if time.is_finite() { time.clamp(0.0, MAX_TIME) } else { 0.0 };
        if let Some(i) = self.nearest(time, MIN_GAP) {
            self.keys[i].pose = pose;
            self.tidy();
            return Some(i);
        }
        if self.is_full() {
            return None;
        }
        // (no keyframe is this close to 0, where the first one is: the
        // times stay as they are, except in an empty track)
        let first = self.keys.is_empty();
        self.keys.push(Keyframe {
            time,
            pose,
            rest: false,
            aim: KeyAim::Lock,
            shot: KeyShot::default(),
            extra: KeyExtra::default(),
        });
        self.tidy();
        if first {
            Some(0)
        } else {
            self.keys.iter().position(|k| k.time == time)
        }
    }

    /// A keyframe after the last one, `DEFAULT_GAP` later.
    pub fn append(&mut self, pose: Pose) -> Option<usize> {
        if self.is_full() || self.duration() + DEFAULT_GAP > MAX_TIME {
            return None;
        }
        let time = if self.keys.is_empty() { 0.0 } else { self.duration() + DEFAULT_GAP };
        self.keys.push(Keyframe {
            time,
            pose,
            rest: false,
            aim: KeyAim::Lock,
            shot: KeyShot::default(),
            extra: KeyExtra::default(),
        });
        self.tidy();
        Some(self.keys.len() - 1)
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.keys.len() {
            self.keys.remove(index);
            self.tidy();
        }
    }

    pub fn set_pose(&mut self, index: usize, pose: Pose) {
        if let Some(key) = self.keys.get_mut(index) {
            key.pose = Pose {
                fov: clamp_fov(pose.fov),
                ..pose
            };
            self.tidy();
        }
    }

    /// The camera comes to rest on keyframe `index`, or passes through it.
    pub fn set_rest(&mut self, index: usize, rest: bool) {
        if let Some(key) = self.keys.get_mut(index) {
            key.rest = rest;
        }
    }

    /// What the view looks at, at keyframe `index`.
    pub fn set_aim(&mut self, index: usize, aim: KeyAim) {
        if let Some(key) = self.keys.get_mut(index) {
            key.aim = aim;
        }
    }

    /// What keyframe `index` adds for the shots: its easing, its dolly zoom,
    /// the target it travels with.
    pub fn set_shot(&mut self, index: usize, shot: KeyShot) {
        if let Some(key) = self.keys.get_mut(index) {
            key.shot = shot;
        }
    }

    /// The animation, lens and time of day of keyframe `index`.
    pub fn set_extra(&mut self, index: usize, extra: KeyExtra) {
        if let Some(key) = self.keys.get_mut(index) {
            key.extra = extra;
        }
    }

    /// Keyframes laid after the last one at the times they give (a path
    /// taken by hand, `director_capture`): each at least `MIN_GAP` after the
    /// one before and no later than `MAX_TIME`, as many as the track still
    /// takes. How many were laid.
    pub fn push_keys(&mut self, keys: impl IntoIterator<Item = Keyframe>) -> usize {
        let mut laid = 0;
        for mut key in keys {
            let after = self.keys.last().map(|last| last.time + MIN_GAP);
            key.time = match after {
                Some(after) => key.time.max(after),
                None => 0.0,
            };
            if self.is_full() || !key.time.is_finite() || key.time > MAX_TIME {
                break;
            }
            self.keys.push(key);
            laid += 1;
        }
        self.tidy();
        laid
    }

    /// The keyframes around `time` and how far the view has turned from what
    /// the first looks at to what the second does, from 0 to 1: smoothly,
    /// little near either keyframe. A track of one keyframe is that one;
    /// `None` for an empty track.
    pub fn around(&self, time: f32) -> Option<(usize, usize, f32)> {
        match self.keys.len() {
            0 => None,
            1 => Some((0, 0, 0.0)),
            _ => {
                let (i, s) = self.span(if time.is_finite() { time } else { 0.0 });
                Some((i, i + 1, s * s * (3.0 - 2.0 * s)))
            }
        }
    }

    /// Move a keyframe on the timeline, between its neighbours (it never
    /// passes one). The first stays at 0: it is where the track starts.
    /// The time it took.
    pub fn set_time(&mut self, index: usize, time: f32) -> f32 {
        let Some(key) = self.keys.get(index) else {
            return 0.0;
        };
        if index == 0 || !time.is_finite() {
            return key.time;
        }
        let low = self.keys[index - 1].time + MIN_GAP;
        let high = self.keys.get(index + 1).map_or(MAX_TIME, |next| next.time - MIN_GAP);
        let time = time.clamp(low, high.max(low));
        self.keys[index].time = time;
        time
    }

    /// Keyframes moved in time together, each from its time in `starts`
    /// (the drag's start) by `by` seconds, as far as the keyframes that do
    /// not move let them (`MIN_GAP` from them, the first keyframe staying at
    /// 0, the last no later than `MAX_TIME`): the seconds they moved.
    pub fn move_times(&mut self, starts: &[(usize, f32)], by: f32) -> f32 {
        let moving = |index: usize| index > 0 && starts.iter().any(|(i, _)| *i == index);
        let (mut low, mut high) = (f32::NEG_INFINITY, f32::INFINITY);
        for &(index, start) in starts {
            if !moving(index) || index >= self.keys.len() {
                continue;
            }
            if !moving(index - 1) {
                low = low.max(self.keys[index - 1].time + MIN_GAP - start);
            }
            match self.keys.get(index + 1) {
                Some(next) if !moving(index + 1) => high = high.min(next.time - MIN_GAP - start),
                None => high = high.min(MAX_TIME - start),
                _ => {}
            }
        }
        if !by.is_finite() || low > high {
            return 0.0;
        }
        let by = by.clamp(low, high);
        for &(index, start) in starts {
            if moving(index) && index < self.keys.len() {
                self.keys[index].time = start + by;
            }
        }
        by
    }

    /// The keyframe nearest to `time`, when it is within `within` seconds.
    pub fn nearest(&self, time: f32, within: f32) -> Option<usize> {
        let (index, key) = self
            .keys
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.time - time).abs().total_cmp(&(b.1.time - time).abs()))?;
        ((key.time - time).abs() < within).then_some(index)
    }

    /// The region positions are relative to changed (`Camera::shift`).
    pub fn shift(&mut self, delta: Vec3) {
        for key in &mut self.keys {
            key.pose.position += delta;
            if let Some(anchor) = &mut key.shot.anchor {
                *anchor += delta;
            }
        }
    }

    /// The rate of change of a value at keyframe `i`, per second: from its
    /// two neighbours (Catmull-Rom); at an end, towards the only neighbour,
    /// or none when the ends are eased; none at a keyframe the camera
    /// rests on.
    fn rate<T>(&self, i: usize, value: impl Fn(&Pose) -> T) -> T
    where
        T: Copy + Sub<Output = T> + Mul<f32, Output = T>,
    {
        let last = self.keys.len() - 1;
        let (from, to) = (i.saturating_sub(1), (i + 1).min(last));
        let change = value(&self.keys[to].pose) - value(&self.keys[from].pose);
        let seconds = self.keys[to].time - self.keys[from].time;
        if seconds <= 0.0 || self.keys[i].rest || (self.ease_ends && (i == 0 || i == last)) {
            return change * 0.0;
        }
        change * (1.0 / seconds)
    }

    /// The span `time` falls in (index of its first keyframe) and how far
    /// through it, 0 to 1, with the easing of the way to its second
    /// keyframe (`Ease::apply`). Needs two keyframes.
    fn span(&self, time: f32) -> (usize, f32) {
        let last = self.keys.len() - 1;
        let time = time.clamp(0.0, self.keys[last].time);
        let i = self.keys.partition_point(|k| k.time <= time).saturating_sub(1).min(last - 1);
        let seconds = self.keys[i + 1].time - self.keys[i].time;
        let s = if seconds > 0.0 { (time - self.keys[i].time) / seconds } else { 0.0 };
        (i, self.keys[i + 1].shot.ease.apply(s.clamp(0.0, 1.0)))
    }

    /// The view `s` of the way through the span after keyframe `i`; on the
    /// way to a keyframe with a dolly zoom, the lens keeps the subject's
    /// size (`shots::dolly_fov`).
    fn span_pose(&self, i: usize, s: f32) -> Pose {
        let pose = self.span_curve(i, s);
        if !self.keys[i + 1].shot.dolly_zoom {
            return pose;
        }
        let (a, b) = (&self.keys[i].pose, &self.keys[i + 1].pose);
        Pose {
            fov: crate::director::shots::dolly_fov(a, b, pose.position, pose.fov),
            ..pose
        }
    }

    fn span_curve(&self, i: usize, s: f32) -> Pose {
        let (a, b) = (self.keys[i].pose, self.keys[i + 1].pose);
        if self.curve == Curve::Linear {
            return Pose {
                position: a.position.lerp(b.position, s),
                rotation: a.rotation.slerp(b.rotation, s),
                fov: a.fov + (b.fov - a.fov) * s,
            };
        }
        let seconds = self.keys[i + 1].time - self.keys[i].time;
        let position = hermite(
            a.position,
            self.rate(i, |p| p.position),
            b.position,
            self.rate(i + 1, |p| p.position),
            seconds,
            s,
        );
        let turn = |p: &Pose| Vec4::from(p.rotation);
        let rotation = hermite(turn(&a), self.rate(i, turn), turn(&b), self.rate(i + 1, turn), seconds, s);
        let rotation = if rotation.length_squared() > 1e-8 {
            Quat::from_vec4(rotation).normalize()
        } else {
            a.rotation.slerp(b.rotation, s)
        };
        // the lens never opens or closes past the two keyframes it goes between
        let fov = hermite(a.fov, self.rate(i, |p| p.fov), b.fov, self.rate(i + 1, |p| p.fov), seconds, s);
        Pose {
            position,
            rotation,
            fov: fov.clamp(a.fov.min(b.fov), a.fov.max(b.fov)),
        }
    }

    /// The view at `time` seconds: the first keyframe before the start, the
    /// last one after the end. `None` for an empty track.
    pub fn sample(&self, time: f32) -> Option<Pose> {
        match self.keys.len() {
            0 => None,
            1 => Some(self.keys[0].pose),
            _ => {
                let (i, s) = self.span(if time.is_finite() { time } else { 0.0 });
                Some(self.span_pose(i, s))
            }
        }
    }

    /// The two Bézier handles of the span after keyframe `i`: with the two
    /// keyframes' positions, the cubic Bézier curve the camera follows.
    pub fn handles(&self, i: usize) -> Option<(Vec3, Vec3)> {
        let (a, b) = (self.keys.get(i)?, self.keys.get(i + 1)?);
        if self.curve == Curve::Linear {
            let third = (b.pose.position - a.pose.position) / 3.0;
            return Some((a.pose.position + third, b.pose.position - third));
        }
        let third = (b.time - a.time) / 3.0;
        Some((
            a.pose.position + self.rate(i, |p| p.position) * third,
            b.pose.position - self.rate(i + 1, |p| p.position) * third,
        ))
    }

    /// The path as points, `steps` to a span, the keyframes among them
    /// (every `steps`-th point).
    pub fn path(&self, steps: usize) -> Vec<Vec3> {
        let steps = steps.max(1);
        match self.keys.len() {
            0 => Vec::new(),
            1 => vec![self.keys[0].pose.position],
            n => {
                let mut points = Vec::with_capacity((n - 1) * steps + 1);
                for i in 0..n - 1 {
                    for step in 0..steps {
                        points.push(self.span_pose(i, step as f32 / steps as f32).position);
                    }
                }
                points.push(self.keys[n - 1].pose.position);
                points
            }
        }
    }

    /// Seconds the camera takes to reach keyframe `index` from the one
    /// before; `None` for the first keyframe.
    pub fn span_time(&self, index: usize) -> Option<f32> {
        let before = self.keys.get(index.checked_sub(1)?)?;
        Some(self.keys.get(index)?.time - before.time)
    }

    /// The camera takes `seconds` to reach keyframe `index` from the one
    /// before: the keyframe and all those after it move together, so every
    /// other span keeps its own time. The time it takes now.
    pub fn set_span(&mut self, index: usize, seconds: f32) -> Option<f32> {
        let now = self.span_time(index)?;
        if !seconds.is_finite() {
            return Some(now);
        }
        // the last keyframe goes no later than the track may last
        let room = (MAX_TIME - self.duration()).max(0.0);
        let wanted = seconds.max(MIN_GAP).min(now + room);
        for key in &mut self.keys[index..] {
            key.time += wanted - now;
        }
        Some(wanted)
    }

    /// The camera's mean speed on its way to keyframe `index` from the one
    /// before, metres a second.
    pub fn span_speed(&self, index: usize) -> Option<f32> {
        let seconds = self.span_time(index)?;
        (seconds > 0.0).then(|| self.span_length(index - 1) / seconds)
    }

    /// Give that span the mean speed `speed` (metres a second) by changing
    /// the time it takes, as `set_span` does. The speed it has now; `None`
    /// when the camera does not move there.
    pub fn set_span_speed(&mut self, index: usize, speed: f32) -> Option<f32> {
        if !speed.is_finite() || speed <= 0.0 || self.span_length(index.checked_sub(1)?) < MIN_TRAVEL {
            return None;
        }
        // the curve's shape follows the timing: a few rounds settle it
        for _ in 0..6 {
            self.set_span(index, self.span_length(index - 1) / speed)?;
        }
        self.span_speed(index)
    }

    /// The whole track lasts `seconds`: every span stretched alike, the
    /// camera faster or slower everywhere.
    pub fn set_duration(&mut self, seconds: f32) {
        let (n, now) = (self.keys.len(), self.duration());
        if n < 2 || now <= 0.0 || !seconds.is_finite() {
            return;
        }
        let scale = seconds.clamp(MIN_GAP * (n - 1) as f32, MAX_TIME) / now;
        for key in &mut self.keys {
            key.time *= scale;
        }
        for i in 1..n {
            self.keys[i].time = self.keys[i].time.max(self.keys[i - 1].time + MIN_GAP);
        }
    }

    /// Metres the camera travels in the span after keyframe `i`.
    pub fn span_length(&self, i: usize) -> f32 {
        if i + 1 >= self.keys.len() {
            return 0.0;
        }
        (0..MEASURE_STEPS)
            .map(|step| {
                let from = self.span_pose(i, step as f32 / MEASURE_STEPS as f32).position;
                let to = self.span_pose(i, (step + 1) as f32 / MEASURE_STEPS as f32).position;
                from.distance(to)
            })
            .sum()
    }

    /// Metres from the first keyframe to the last, along the path.
    pub fn length(&self) -> f32 {
        (0..self.keys.len().saturating_sub(1)).map(|i| self.span_length(i)).sum()
    }

    /// The keyframes between the first and the last moved on the timeline so
    /// that the camera keeps one speed along the whole path: each span lasts
    /// in proportion to its length. The duration stays.
    pub fn even_speed(&mut self) {
        let n = self.keys.len();
        let duration = self.duration();
        if n < 3 || duration < MIN_GAP * (n - 1) as f32 * 2.0 {
            return;
        }
        // the curve's shape follows the timing: a few rounds settle it
        for _ in 0..8 {
            let lengths: Vec<f32> = (0..n - 1).map(|i| self.span_length(i)).collect();
            let total: f32 = lengths.iter().sum();
            if total < 1e-3 {
                return;
            }
            let mut time = 0.0;
            for i in 1..n - 1 {
                time += duration * lengths[i - 1] / total;
                let low = self.keys[i - 1].time + MIN_GAP;
                let high = duration - MIN_GAP * (n - 1 - i) as f32;
                self.keys[i].time = time.clamp(low, high.max(low));
            }
        }
    }
}

/// The playhead `dt` seconds later on a track of `duration` seconds, and
/// whether the track came to its end (never when it loops).
pub fn advance(time: f32, dt: f32, duration: f32, looping: bool) -> (f32, bool) {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let time = time + dt;
    if duration <= 0.0 {
        return (0.0, !looping);
    }
    if time < duration {
        (time, false)
    } else if looping {
        (time.rem_euclid(duration), false)
    } else {
        (duration, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn looking(position: Vec3, yaw: f32) -> Pose {
        Pose::from_angles(position, yaw, 0.0, 0.0, 1.0)
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-3
    }

    fn three() -> Track {
        let mut track = Track::default();
        track.append(looking(Vec3::new(0.0, 0.0, 20.0), 0.0));
        track.append(looking(Vec3::new(10.0, 0.0, 20.0), 0.5));
        track.append(looking(Vec3::new(10.0, 10.0, 25.0), 1.0));
        track
    }

    #[test]
    fn keyframes_move_in_time_together_between_the_others() {
        let mut track = three();
        track.append(looking(Vec3::new(0.0, 10.0, 25.0), 1.0));
        let times: Vec<f32> = track.keys().iter().map(|k| k.time).collect();
        let starts = [(1, times[1]), (2, times[2])];
        // later: as far as the fourth keyframe lets the third go
        let moved = track.move_times(&starts, 100.0);
        assert!((moved - (times[3] - MIN_GAP - times[2])).abs() < 1e-4, "{moved}");
        assert!((track.keys()[2].time - track.keys()[1].time - (times[2] - times[1])).abs() < 1e-4);
        // earlier, from the same start: as far as the first lets the second go
        let moved = track.move_times(&starts, -100.0);
        assert!((track.keys()[1].time - MIN_GAP).abs() < 1e-4 && moved < 0.0);
        // the first keyframe stays at 0, the others selected still move
        let moved = track.move_times(&[(0, 0.0), (3, times[3])], 1.0);
        assert!((moved - 1.0).abs() < 1e-4 && track.keys()[0].time == 0.0);
        assert!((track.keys()[3].time - times[3] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_pose_keeps_the_view_it_was_taken_from() {
        let forward = Vec3::new(1.0, 2.0, -0.5).normalize();
        let up = Quat::from_axis_angle(forward, 0.3) * Vec3::Z;
        let pose = Pose::from_view(Vec3::new(1.0, 2.0, 3.0), forward, up, 0.8);
        assert!(close(pose.forward(), forward));
        assert!((pose.roll() - 0.3).abs() < 1e-4);
        let (yaw, pitch, roll) = pose.angles();
        let again = Pose::from_angles(pose.position, yaw, pitch, roll, pose.fov);
        assert!(again.rotation.angle_between(pose.rotation) < 1e-3);
        // a lens out of the camera's range is brought in, a broken one replaced
        assert_eq!(
            Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, 100.0).fov,
            *crate::camera::FOV_RANGE.end()
        );
        assert_eq!(
            Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, f32::NAN).fov,
            crate::camera::DEFAULT_FOV
        );
    }

    #[test]
    fn keyframes_added_in_a_row_are_three_seconds_apart() {
        let track = three();
        let times: Vec<f32> = track.keys().iter().map(|k| k.time).collect();
        assert_eq!(times, [0.0, 3.0, 6.0]);
        assert_eq!(track.duration(), 6.0);
    }

    #[test]
    fn the_path_goes_through_every_keyframe() {
        for curve in [Curve::Smooth, Curve::Linear] {
            let mut track = three();
            track.curve = curve;
            for key in track.keys().to_vec() {
                let pose = track.sample(key.time).expect("pose");
                assert!(close(pose.position, key.pose.position), "{curve:?}");
                assert!(pose.rotation.angle_between(key.pose.rotation) < 1e-3);
            }
            // held before the start and after the end
            assert!(close(track.sample(-5.0).expect("pose").position, track.keys()[0].pose.position));
            assert!(close(track.sample(99.0).expect("pose").position, track.keys()[2].pose.position));
        }
        assert_eq!(Track::default().sample(1.0), None);
    }

    #[test]
    fn the_speed_does_not_jump_at_a_keyframe() {
        let mut track = three();
        track.ease_ends = false;
        let speed = |t: f32| (track.sample(t + 0.001).expect("pose").position - track.sample(t - 0.001).expect("pose").position) / 0.002;
        let (before, after) = (speed(2.99), speed(3.01));
        assert!(before.distance(after) < 0.1, "{before} {after}");
        // straight lines do turn sharply there
        track.curve = Curve::Linear;
        let speed = |t: f32| (track.sample(t + 0.001).expect("pose").position - track.sample(t - 0.001).expect("pose").position) / 0.002;
        assert!(speed(2.99).distance(speed(3.01)) > 1.0);
    }

    #[test]
    fn eased_ends_start_and_stop_at_rest() {
        let mut track = three();
        let moved = |track: &Track| track.sample(0.05).expect("pose").position.distance(track.keys()[0].pose.position);
        let eased = moved(&track);
        track.ease_ends = false;
        assert!(eased < moved(&track) * 0.2);
    }

    #[test]
    fn a_turn_takes_the_short_way() {
        // from 170° to -170°: 20° through the back, not 340° through the front
        let mut track = Track::default();
        track.append(looking(Vec3::ZERO, 170f32.to_radians()));
        track.append(looking(Vec3::X, -170f32.to_radians()));
        for curve in [Curve::Smooth, Curve::Linear] {
            track.curve = curve;
            let (yaw, ..) = track.sample(1.5).expect("pose").angles();
            assert!((yaw.abs() - PI).abs() < 0.01, "{curve:?}: {yaw}");
        }
    }

    #[test]
    fn the_lens_stays_between_its_two_keyframes() {
        let mut track = Track::default();
        for fov in [1.0, 1.0, 0.4, 0.4] {
            track.append(Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, fov));
        }
        for step in 0..=90 {
            let fov = track.sample(step as f32 * 0.1).expect("pose").fov;
            assert!((0.4..=1.0).contains(&fov), "{fov}");
        }
        assert!((track.sample(4.5).expect("pose").fov - 0.7).abs() < 0.05);
    }

    #[test]
    fn a_keyframe_at_a_time_already_taken_replaces_it() {
        let mut track = three();
        let pose = looking(Vec3::new(5.0, 5.0, 5.0), FRAC_PI_2);
        assert_eq!(track.insert(3.04, pose), Some(1));
        assert_eq!(track.len(), 3);
        assert!(close(track.keys()[1].pose.position, pose.position));
        // elsewhere it goes in order
        assert_eq!(track.insert(4.5, pose), Some(2));
        assert_eq!(track.keys().iter().map(|k| k.time).collect::<Vec<_>>(), [0.0, 3.0, 4.5, 6.0]);
    }

    #[test]
    fn a_keyframe_moves_between_its_neighbours_only() {
        let mut track = three();
        assert_eq!(track.set_time(1, 4.0), 4.0);
        assert_eq!(track.set_time(1, 99.0), 6.0 - MIN_GAP);
        assert_eq!(track.set_time(1, -3.0), MIN_GAP);
        // the last one has no limit but the track's, the first stays at 0
        assert_eq!(track.set_time(2, 20.0), 20.0);
        assert_eq!(track.set_time(0, 2.0), 0.0);
        assert_eq!(track.set_time(9, 2.0), 0.0);
    }

    #[test]
    fn the_track_always_starts_at_zero() {
        let mut track = three();
        track.remove(0);
        assert_eq!(track.keys().iter().map(|k| k.time).collect::<Vec<_>>(), [0.0, 3.0]);
        track.remove(7);
        assert_eq!(track.len(), 2);
    }

    #[test]
    fn a_full_track_takes_no_more() {
        let mut track = Track::default();
        for i in 0..MAX_KEYS {
            assert!(track.insert(i as f32, looking(Vec3::X * i as f32, 0.0)).is_some());
        }
        assert!(track.is_full());
        assert_eq!(track.append(looking(Vec3::ZERO, 0.0)), None);
        assert_eq!(track.insert(0.5, looking(Vec3::ZERO, 0.0)), None);
        // the view of an existing keyframe can still change
        assert_eq!(track.insert(3.0, looking(Vec3::Z, 0.0)), Some(3));
    }

    #[test]
    fn even_speed_gives_each_span_its_share_of_the_time() {
        // 2 m then 8 m in a straight line, 3 s each
        let mut track = Track {
            curve: Curve::Linear,
            ..Default::default()
        };
        for x in [0.0, 2.0, 10.0] {
            track.append(looking(Vec3::X * x, 0.0));
        }
        track.even_speed();
        assert!((track.keys()[1].time - 1.2).abs() < 1e-3);
        assert_eq!(track.duration(), 6.0);
        assert!((track.length() - 10.0).abs() < 1e-3);
        // a curved path too: the spans' speeds end up close
        let mut track = three();
        track.ease_ends = false;
        track.even_speed();
        let speeds: Vec<f32> = (0..2)
            .map(|i| track.span_length(i) / (track.keys()[i + 1].time - track.keys()[i].time))
            .collect();
        assert!((speeds[0] - speeds[1]).abs() < speeds[0] * 0.1, "{speeds:?}");
    }

    #[test]
    fn the_camera_stops_on_a_keyframe_it_rests_on() {
        let mut track = three();
        track.ease_ends = false;
        let moved = |track: &Track| {
            let at = track.keys()[1].pose.position;
            track.sample(3.05).expect("pose").position.distance(at) + track.sample(2.95).expect("pose").position.distance(at)
        };
        let passing = moved(&track);
        track.set_rest(1, true);
        assert!(moved(&track) < passing * 0.2, "it slows to a stop there");
        // it still goes through the keyframe, and on to the next
        assert!(close(track.sample(3.0).expect("pose").position, track.keys()[1].pose.position));
        assert!(close(track.sample(6.0).expect("pose").position, track.keys()[2].pose.position));
        track.set_rest(9, true);
    }

    #[test]
    fn a_view_turns_to_its_target_and_keeps_its_place_and_roll() {
        let pose = Pose::from_angles(Vec3::new(0.0, 0.0, 20.0), 0.0, 0.0, 0.3, 0.9);
        let target = Vec3::new(0.0, 10.0, 25.0);
        let aimed = pose.aimed(target, 1.0);
        assert!(close(aimed.forward(), (target - pose.position).normalize()));
        assert_eq!((aimed.position, aimed.fov), (pose.position, pose.fov));
        assert!((aimed.roll() - 0.3).abs() < 1e-3);
        // half way: between where it looked and the target
        let half = pose.aimed(target, 0.5);
        assert!(half.forward().dot(pose.forward()) > 0.5 && half.forward().dot(aimed.forward()) > 0.5);
        assert!(half.rotation.angle_between(pose.rotation) > 0.2 && half.rotation.angle_between(aimed.rotation) > 0.2);
        // no weight, a target on the camera or a broken one: the view as it is
        assert_eq!(pose.aimed(target, 0.0), pose);
        assert_eq!(pose.aimed(pose.position, 1.0), pose);
        assert_eq!(pose.aimed(Vec3::splat(f32::NAN), 1.0), pose);
    }

    #[test]
    fn the_view_turns_smoothly_between_two_keyframes() {
        let track = three();
        assert_eq!(track.around(0.0), Some((0, 1, 0.0)));
        assert_eq!(track.around(3.0), Some((1, 2, 0.0)));
        assert_eq!(track.around(6.0), Some((1, 2, 1.0)));
        assert_eq!(track.around(99.0), Some((1, 2, 1.0)));
        let (_, _, half) = track.around(1.5).expect("span");
        assert!((half - 0.5).abs() < 1e-4);
        // smoothly: little change near a keyframe
        let ((_, _, near), (_, _, far)) = (track.around(0.3).expect("span"), track.around(2.7).expect("span"));
        assert!(near < 0.05 && far > 0.95);
        assert_eq!(track.around(f32::NAN), Some((0, 1, 0.0)));
        let mut one = Track::default();
        assert_eq!(one.around(0.0), None);
        one.append(looking(Vec3::ZERO, 0.0));
        assert_eq!(one.around(5.0), Some((0, 0, 0.0)));
    }

    #[test]
    fn a_hand_held_camera_stays_near_its_view_and_repeats_itself() {
        let pose = looking(Vec3::new(5.0, 5.0, 25.0), 0.4);
        assert_eq!(hand_held(pose, 3.0, 0.0), pose);
        assert_eq!(hand_held(pose, f32::NAN, 1.0), pose);
        let mut moved = false;
        for step in 0..200 {
            let time = step as f32 * 0.05;
            let held = hand_held(pose, time, 1.0);
            assert!(held.position.distance(pose.position) <= HAND_DRIFT * 1.5);
            assert!(held.rotation.angle_between(pose.rotation) <= HAND_TURN * 3.0);
            assert_eq!(held.fov, pose.fov);
            assert_eq!(held, hand_held(pose, time, 1.0), "the same at the same time");
            moved |= held.position.distance(pose.position) > 1e-3;
            // no jump from a frame to the next
            let next = hand_held(pose, time + 0.016, 1.0);
            assert!(next.position.distance(held.position) < 0.01);
        }
        assert!(moved);
        // half the amount, half the drift
        let (full, half) = (hand_held(pose, 2.0, 1.0), hand_held(pose, 2.0, 0.5));
        assert!((half.position.distance(pose.position) * 2.0 - full.position.distance(pose.position)).abs() < 1e-4);
    }

    #[test]
    fn a_track_comes_back_from_its_file_where_it_was_laid() {
        let mut track = three();
        track.curve = Curve::Linear;
        track.set_rest(1, true);
        let avatar = uuid::Uuid::from_u128(7);
        let on = |target, joint| KeyAim::On(Aim { target, joint });
        track.set_aim(1, on(Target::Avatar(avatar), Joint::Head));
        track.set_aim(2, KeyAim::Free);
        let origin = (256_000, 512_000);
        let neck = Aim {
            target: Target::Own,
            joint: Joint::Neck,
        };
        let file = track.to_file(origin);
        assert_eq!(file.version, FILE_VERSION);
        assert!((file.keys[1].position[0] - 256_010.0).abs() < 1e-6);
        let text = serde_json::to_string(&file).expect("json");
        let read: TrackFile = serde_json::from_str(&text).expect("json");
        let back = Track::from_file(&read, origin, neck).expect("track");
        assert_eq!(
            (back.curve, back.ease_ends, back.len()),
            (track.curve, track.ease_ends, track.len())
        );
        for (a, b) in back.keys().iter().zip(track.keys()) {
            assert_eq!((a.time, a.rest, a.aim, a.pose.fov), (b.time, b.rest, b.aim, b.pose.fov));
            assert!(close(a.pose.position, b.pose.position));
            assert!(a.pose.rotation.angle_between(b.pose.rotation) < 1e-4);
        }
        // loaded from the next region east: 256 m to the west of that corner
        let east = Track::from_file(&read, (origin.0 + 256, origin.1), neck).expect("track");
        assert!(close(east.keys()[0].pose.position, Vec3::new(-256.0, 0.0, 20.0)));
        // a file is data from outside
        let mut bad = file.clone();
        bad.keys[0].position[1] = f64::NAN;
        assert_eq!(Track::from_file(&bad, origin, neck), None);
        let mut bad = file.clone();
        bad.keys[2].rotation = [0.0; 4];
        assert_eq!(Track::from_file(&bad, origin, neck), None);
        let empty = TrackFile {
            keys: Vec::new(),
            ..file.clone()
        };
        assert_eq!(Track::from_file(&empty, origin, neck), None);
        // times out of order or equal are put right, a lens brought in range
        let mut odd = file.clone();
        odd.keys[0].time = 6.0;
        odd.keys[1].time = 6.0;
        odd.keys[2].time = 2.0;
        odd.keys[2].fov = 99.0;
        let fixed = Track::from_file(&odd, origin, neck).expect("track");
        let times: Vec<f32> = fixed.keys().iter().map(|k| k.time).collect();
        assert_eq!(times[0], 0.0);
        assert!(times[1] >= 4.0 && times[2] >= times[1] + MIN_GAP - 1e-6);
        assert_eq!(fixed.keys()[0].pose.fov, *crate::camera::FOV_RANGE.end());
        // a target that reads as none: the keyframe follows the lock; a
        // joint unknown: the neck
        let mut odd = file.clone();
        odd.keys[1].aim = Some(AimFile::Avatar {
            id: "not an id".into(),
            joint: "mTail".into(),
        });
        odd.keys[2].aim = Some(AimFile::Own { joint: "mTail".into() });
        let read = Track::from_file(&odd, origin, neck).expect("track");
        assert_eq!(read.keys()[1].aim, KeyAim::Lock);
        assert_eq!(read.keys()[2].aim, KeyAim::On(neck));
        // a viewer that reads files of 1 still sees the locked keyframes
        assert_eq!(file.keys.iter().map(|k| k.locked).collect::<Vec<_>>(), [false, true, false]);
        // an older file without the newer fields still loads; its locked
        // keyframe looks at the lock's target
        let old = r#"{"version":1,"keys":[{"time":0.0,"position":[1.0,2.0,3.0],"rotation":[0.0,0.0,0.0,1.0],"fov":1.0},{"time":3.0,"position":[1.0,2.0,3.0],"rotation":[0.0,0.0,0.0,1.0],"fov":1.0,"locked":true}]}"#;
        let old = Track::from_file(&serde_json::from_str(old).expect("json"), (0, 0), neck).expect("track");
        assert_eq!((old.keys()[0].aim, old.keys()[1].aim), (KeyAim::Lock, KeyAim::On(neck)));
    }

    #[test]
    fn a_track_is_saved_under_a_name_a_file_can_have() {
        assert_eq!(file_stem("  Dance shot 2  ").as_deref(), Some("Dance shot 2"));
        assert_eq!(file_stem("a/b\\c:d").as_deref(), Some("a_b_c_d"));
        assert_eq!(file_stem("../../etc").as_deref(), Some("______etc"));
        assert_eq!(file_stem("Été à Paris").as_deref(), Some("Été à Paris"));
        assert_eq!(file_stem(""), None);
        assert_eq!(file_stem("   "), None);
        assert_eq!(file_stem("..."), None);
        assert_eq!(file_stem("NUL"), None);
        assert_eq!(file_stem("com1"), None);
        assert_eq!(file_stem(&"x".repeat(200)).map(|s| s.len()), Some(60));
    }

    #[test]
    fn a_span_takes_the_time_it_is_given_and_the_rest_follows() {
        let mut track = three();
        assert_eq!(track.span_time(0), None);
        assert_eq!(track.span_time(1), Some(3.0));
        // the first span twice as long: the last keyframe goes 3 s later,
        // its own span still 3 s
        assert_eq!(track.set_span(1, 6.0), Some(6.0));
        assert_eq!(track.keys().iter().map(|k| k.time).collect::<Vec<_>>(), [0.0, 6.0, 9.0]);
        // never under the smallest gap, nor past what a track may last
        assert_eq!(track.set_span(2, 0.0), Some(MIN_GAP));
        let longest = track.set_span(2, f32::MAX).expect("time");
        assert!((longest - (MAX_TIME - 6.0)).abs() < 0.01, "{longest}");
        assert!((track.duration() - MAX_TIME).abs() < 0.01);
        assert_eq!(track.set_span(0, 2.0), None);
        assert_eq!(track.set_span(3, 2.0), None);
    }

    #[test]
    fn a_span_takes_the_speed_it_is_given() {
        // 10 m in a straight line then 10 m more, 3 s each
        let mut track = Track {
            curve: Curve::Linear,
            ..Default::default()
        };
        for x in [0.0, 10.0, 20.0] {
            track.append(looking(Vec3::X * x, 0.0));
        }
        assert!((track.span_speed(1).expect("speed") - 10.0 / 3.0).abs() < 1e-3);
        // twice as fast on the second span only
        let speed = track.set_span_speed(2, 20.0 / 3.0).expect("speed");
        assert!((speed - 20.0 / 3.0).abs() < 1e-3);
        assert!((track.span_time(2).expect("time") - 1.5).abs() < 1e-3);
        assert_eq!(track.span_time(1), Some(3.0));
        // on a curve too, whose shape follows the timing
        let mut track = three();
        let speed = track.set_span_speed(1, 1.0).expect("speed");
        assert!((speed - 1.0).abs() < 0.05, "{speed}");
        // a camera that turns on the spot has no speed to set
        let mut still = Track::default();
        still.append(looking(Vec3::ZERO, 0.0));
        still.append(looking(Vec3::ZERO, 1.0));
        assert_eq!(still.set_span_speed(1, 2.0), None);
        assert_eq!(still.span_speed(1), Some(0.0));
        assert_eq!(track.set_span_speed(1, 0.0), None);
        assert_eq!(track.set_span_speed(0, 1.0), None);
    }

    #[test]
    fn the_whole_track_stretches_alike() {
        let mut track = three();
        track.set_span(1, 1.0);
        track.set_duration(8.0);
        assert_eq!(track.keys().iter().map(|k| k.time).collect::<Vec<_>>(), [0.0, 2.0, 8.0]);
        // as short as its keyframes allow, no shorter
        track.set_duration(0.0);
        assert!((track.duration() - 2.0 * MIN_GAP).abs() < 1e-4);
        assert!(track.span_time(1).expect("time") >= MIN_GAP);
        let mut one = Track::default();
        one.append(looking(Vec3::ZERO, 0.0));
        one.set_duration(5.0);
        assert_eq!(one.duration(), 0.0);
    }

    #[test]
    fn the_handles_draw_the_same_curve() {
        let track = three();
        let (a, b) = (track.keys()[0].pose.position, track.keys()[1].pose.position);
        let (h1, h2) = track.handles(0).expect("handles");
        for step in 0..=10 {
            let s = step as f32 / 10.0;
            let r = 1.0 - s;
            let bezier = a * (r * r * r) + h1 * (3.0 * r * r * s) + h2 * (3.0 * r * s * s) + b * (s * s * s);
            assert!(close(bezier, track.sample(3.0 * s).expect("pose").position));
        }
        assert_eq!(track.handles(2), None);
        // the path's points: the keyframes every `steps`
        let path = track.path(8);
        assert_eq!(path.len(), 17);
        assert!(close(path[8], b));
    }

    #[test]
    fn a_region_change_moves_the_whole_track() {
        let mut track = three();
        track.shift(Vec3::new(-256.0, 0.0, 0.0));
        assert!(close(track.keys()[1].pose.position, Vec3::new(-246.0, 0.0, 20.0)));
    }

    #[test]
    fn the_playhead_stops_at_the_end_or_loops() {
        assert_eq!(advance(1.0, 0.5, 6.0, false), (1.5, false));
        assert_eq!(advance(5.9, 0.5, 6.0, false), (6.0, true));
        let (time, done) = advance(5.9, 0.5, 6.0, true);
        assert!((time - 0.4).abs() < 1e-4 && !done);
        // a single keyframe has nothing to play; a broken frame time is no time
        assert_eq!(advance(0.0, 0.1, 0.0, false), (0.0, true));
        assert_eq!(advance(2.0, f32::NAN, 6.0, false), (2.0, false));
    }
}
