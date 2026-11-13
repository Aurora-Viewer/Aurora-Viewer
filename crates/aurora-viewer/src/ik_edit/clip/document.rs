//! Native rotation keys and pelvis translations. Capture uses LL's rigid
//! parent rotation rule, never inherited matrix scale. Export trims/rebases
//! tracks and samples both boundaries before the standard .anim codec.
use super::*;
use aurora_assets::anim::{JointMotion, PosKey, RotKey};
use std::io::{self, Write};

pub const FPS: f32 = 30.0;
pub const MAX_DURATION: f32 = 60.0;
pub const MAX_FRAME: u32 = (MAX_DURATION as u32) * 60;
pub(super) const MAX_ANIM_KEYS: usize = 16_384;
/// The last hand pose of the editor's list (LLHandMotion::NUM_HAND_POSES is
/// 14, indra/llcharacter/llhandmotion.h).
const MAX_HAND_POSE: u32 = 13;
const ANIM_EXPORT_SAMPLE_RATE: u32 = 60;
pub const MAX_PROJECT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct Track {
    pub bone: usize,
    pub enabled: bool,
    pub authored: bool,
    pub motion: JointMotion,
    pub interpolation: Interpolation,
    pub curve: Curve,
    pub(super) base_position: Vec3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Interpolation {
    Smooth,
    #[default]
    Linear,
    Stepped,
    Curve,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Curve {
    /// First cubic Bézier control point's value (time is fixed at one third).
    pub start: f32,
    /// Second cubic Bézier control point's value (time is fixed at two thirds).
    pub end: f32,
}

impl Default for Curve {
    fn default() -> Self {
        Self { start: 0.0, end: 1.0 }
    }
}

impl Curve {
    pub(super) fn normalized(self) -> Self {
        let start = if self.start.is_finite() { self.start.clamp(0.0, 1.0) } else { 0.0 };
        let end = if self.end.is_finite() { self.end.clamp(start, 1.0) } else { 1.0 };
        Self { start, end }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectFile {
    version: u32,
    name: String,
    fps: u32,
    auto_keyframe: bool,
    #[serde(default = "default_auto_key_advance_seconds")]
    auto_key_advance_seconds: f32,
    start: u32,
    end: u32,
    base_priority: i32,
    looping: bool,
    loop_in: f32,
    loop_out: f32,
    ease_in: f32,
    ease_out: f32,
    hand_pose: u32,
    emote_name: String,
    tracks: Vec<ProjectTrack>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectTrack {
    joint_name: String,
    enabled: bool,
    #[serde(default = "authored_by_default")]
    authored: bool,
    priority: i32,
    interpolation: Interpolation,
    #[serde(default)]
    curve: Curve,
    rotations: Vec<ProjectRotation>,
    positions: Vec<ProjectPosition>,
}

fn authored_by_default() -> bool {
    true
}

fn default_auto_key_advance_seconds() -> f32 {
    0.5
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(end) = self.bytes.len().checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::new(io::ErrorKind::WriteZero, "project size overflow"));
        };
        if end > self.limit {
            self.exceeded = true;
            return Err(io::Error::new(io::ErrorKind::WriteZero, "project exceeds size limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn serialize_project<T: serde::Serialize>(project: &T, limit: usize) -> Result<Vec<u8>, &'static str> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, project);
    if writer.exceeded {
        Err("anim-edit-project-too-large")
    } else if result.is_err() {
        Err("anim-edit-invalid")
    } else {
        Ok(writer.bytes)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectRotation {
    time: f32,
    value: [f32; 4],
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectPosition {
    time: f32,
    value: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub bone: usize,
    pub rotation: Option<Quat>,
    pub position: Option<Vec3>,
}

fn eased(mode: Interpolation, curve: Curve, amount: f32) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    match mode {
        Interpolation::Smooth => amount * amount * (3.0 - 2.0 * amount),
        Interpolation::Linear => amount,
        Interpolation::Stepped => {
            if amount >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        Interpolation::Curve => {
            let curve = curve.normalized();
            let inverse = 1.0 - amount;
            3.0 * inverse * inverse * amount * curve.start + 3.0 * inverse * amount * amount * curve.end + amount * amount * amount
        }
    }
}

fn sample<K, T: Copy>(
    keys: &[K],
    time: f32,
    mode: Interpolation,
    curve: Curve,
    key: impl Fn(&K) -> (f32, T),
    mix: impl Fn(T, T, f32) -> T,
) -> Option<T> {
    let (first_time, first) = key(keys.first()?);
    let (last_time, last) = key(keys.last()?);
    if time <= first_time {
        return Some(first);
    }
    if time >= last_time {
        return Some(last);
    }
    let upper = keys.partition_point(|candidate| key(candidate).0 < time);
    let (a_time, a) = key(&keys[upper - 1]);
    let (b_time, b) = key(&keys[upper]);
    let amount = if b_time > a_time {
        (time - a_time) / (b_time - a_time)
    } else {
        1.0
    };
    Some(mix(a, b, eased(mode, curve, amount)))
}

/// ANIM playback interpolates between every stored key. Represent an editor
/// step by holding its previous value through the last representable time
/// bucket, then placing the new value in the next bucket.
fn stepped_key_times(keys: impl Iterator<Item = f32>, start: f32, end: f32, duration: f32) -> Vec<(f32, f32)> {
    let mut transitions: Vec<_> = keys.filter(|time| time.is_finite() && *time > start && *time <= end).collect();
    transitions.sort_by(f32::total_cmp);
    let mut result = Vec::with_capacity(transitions.len() * 2);
    for time in transitions {
        let normalized = ((time - start) / duration).clamp(0.0, 1.0);
        let bucket = (normalized * 65_535.0).floor() as u16;
        let output_time = |bucket: u16| {
            if bucket == 0 {
                0.0
            } else if bucket == u16::MAX {
                duration
            } else {
                (f32::from(bucket) + 0.5) * duration / 65_535.0
            }
        };
        if bucket > 0 {
            result.push((f32::from_bits(time.to_bits() - 1), output_time(bucket - 1)));
        }
        result.push((time, output_time(bucket)));
    }
    result
}

/// ANIM playback interpolates linearly, so eased editor curves need denser
/// samples than the user's timeline FPS to retain their shape in-world.
fn eased_key_times(keys: impl Iterator<Item = f32>, start: f32, end: f32) -> Vec<(f32, f32)> {
    let mut authored: Vec<f32> = keys.filter(|time| *time > start && *time < end).collect();
    authored.sort_by(f32::total_cmp);
    let duration = end - start;
    let sample_budget = MAX_ANIM_KEYS.saturating_sub(authored.len() + 2);
    let sample_rate = (sample_budget as f32 / duration).min(ANIM_EXPORT_SAMPLE_RATE as f32);
    let mut times: Vec<f32> = (1..=sample_budget)
        .map(|tick| tick as f32 / sample_rate)
        .take_while(|elapsed| *elapsed < duration)
        .map(|elapsed| start + elapsed)
        .chain(authored)
        .collect();
    times.sort_by(f32::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 0.000_001);
    times.into_iter().map(|time| (time, time - start)).collect()
}

impl Track {
    pub(super) fn rotation_at(&self, time: f32) -> Option<Quat> {
        sample(
            &self.motion.rot_keys,
            time,
            self.interpolation,
            self.curve,
            |key| (key.time, key.rotation),
            |a, b, u| a.slerp(b, u),
        )
    }

    pub(super) fn position_at(&self, time: f32) -> Option<Vec3> {
        sample(
            &self.motion.pos_keys,
            time,
            self.interpolation,
            self.curve,
            |key| (key.time, key.position),
            |a, b, u| a.lerp(b, u),
        )
    }
}
#[derive(Clone)]
pub struct Document {
    pub name: String,
    pub meta: Animation,
    pub fps: u32,
    pub auto_keyframe: bool,
    pub auto_key_advance_seconds: f32,
    pub start: u32,
    pub end: u32,
    pub tracks: Vec<Track>,
}
impl Document {
    pub fn max_frame(&self) -> u32 {
        MAX_DURATION as u32 * self.fps.clamp(1, 60)
    }
    pub fn last_authored_keyframe(&self) -> Option<u32> {
        self.tracks
            .iter()
            .filter(|track| track.enabled && track.authored)
            .flat_map(|track| {
                track
                    .motion
                    .rot_keys
                    .iter()
                    .map(|key| key.time)
                    .chain(track.motion.pos_keys.iter().map(|key| key.time))
            })
            .filter(|time| time.is_finite())
            .map(|time| (time * self.fps as f32).round() as u32)
            .filter(|frame| (*frame >= self.start) && (*frame <= self.end))
            .max()
    }
    pub fn new() -> Self {
        Self {
            name: aurora_i18n::tr!("anim-edit-new-name").to_string(),
            fps: FPS as u32,
            auto_keyframe: false,
            auto_key_advance_seconds: default_auto_key_advance_seconds(),
            start: 0,
            end: 900,
            tracks: Vec::new(),
            meta: Animation {
                base_priority: 3,
                duration: 30.0,
                emote_name: String::new(),
                loop_in: 0.0,
                loop_out: 30.0,
                looping: true,
                ends_after: None,
                ease_in: 0.3,
                ease_out: 0.3,
                hand_pose: 0,
                joints: Vec::new(),
            },
        }
    }
    pub fn from_animation(animation: Animation, rig: &Rig) -> Result<Self, &'static str> {
        let fps = FPS as u32;
        let duration = animation.duration;
        if !duration.is_finite() || duration < 0.0 || duration > MAX_DURATION {
            return Err("anim-edit-invalid");
        }
        let end = (duration * fps as f32).ceil().max(1.0) as u32;
        let mut meta = animation;
        // A valid zero-duration native pose is editable as one timeline frame.
        meta.duration = end as f32 / fps as f32;
        // The file format holds values the editor has no control for: a
        // base priority of -1, joint priorities above 6 (ADDITIVE_PRIORITY)
        // and hand pose 14. Bring them into the editor's ranges here, or the
        // clip could be neither previewed nor exported until the user found
        // and changed the value.
        meta.base_priority = meta.base_priority.clamp(0, 6);
        if meta.hand_pose > MAX_HAND_POSE {
            meta.hand_pose = 0;
        }
        let joints = std::mem::take(&mut meta.joints);
        let mut tracks = Vec::new();
        let mut unsupported = false;
        for mut motion in joints {
            let Some(&bone) = rig.names.get(&motion.joint_name) else {
                unsupported = true;
                continue;
            };
            motion.priority = motion.priority.clamp(-1, 6);
            tracks.push(Track {
                bone,
                enabled: true,
                authored: true,
                base_position: base_position(rig, bone),
                motion,
                interpolation: Interpolation::Linear,
                curve: Curve::default(),
            });
        }
        if unsupported {
            return Err("anim-edit-unsupported-bones");
        }
        if tracks.is_empty() {
            return Err("anim-edit-no-bones");
        }
        let mut document = Self {
            name: aurora_i18n::tr!("anim-edit-imported-name").to_string(),
            meta,
            fps,
            auto_keyframe: false,
            auto_key_advance_seconds: default_auto_key_advance_seconds(),
            start: 0,
            end,
            tracks,
        };
        document.timing();
        Ok(document)
    }
    pub fn project_json(&self) -> Result<Vec<u8>, &'static str> {
        let project = ProjectFile {
            version: 1,
            name: self.name.clone(),
            fps: self.fps,
            auto_keyframe: self.auto_keyframe,
            auto_key_advance_seconds: self.auto_key_advance_seconds,
            start: self.start,
            end: self.end,
            base_priority: self.meta.base_priority,
            looping: self.meta.looping,
            loop_in: self.meta.loop_in,
            loop_out: self.meta.loop_out,
            ease_in: self.meta.ease_in,
            ease_out: self.meta.ease_out,
            hand_pose: self.meta.hand_pose,
            emote_name: self.meta.emote_name.clone(),
            tracks: self
                .tracks
                .iter()
                .map(|track| ProjectTrack {
                    joint_name: track.motion.joint_name.clone(),
                    enabled: track.enabled,
                    authored: track.authored,
                    priority: track.motion.priority,
                    interpolation: track.interpolation,
                    curve: track.curve,
                    rotations: track
                        .motion
                        .rot_keys
                        .iter()
                        .map(|key| ProjectRotation {
                            time: key.time,
                            value: key.rotation.to_array(),
                        })
                        .collect(),
                    positions: track
                        .motion
                        .pos_keys
                        .iter()
                        .map(|key| ProjectPosition {
                            time: key.time,
                            value: key.position.to_array(),
                        })
                        .collect(),
                })
                .collect(),
        };
        serialize_project(&project, MAX_PROJECT_BYTES)
    }
    pub fn from_project_json(bytes: &[u8], rig: &Rig) -> Result<Self, &'static str> {
        if bytes.len() > MAX_PROJECT_BYTES {
            return Err("anim-edit-project-too-large");
        }
        let project: ProjectFile = serde_json::from_slice(bytes).map_err(|_| "anim-edit-invalid")?;
        if project.version != 1
            || !(1..=60).contains(&project.fps)
            || project.start >= project.end
            || project.end > MAX_DURATION as u32 * project.fps
            || (project.end - project.start) as f32 / project.fps as f32 > MAX_DURATION
            || !project.auto_key_advance_seconds.is_finite()
            || !(0.1..=5.0).contains(&project.auto_key_advance_seconds)
        {
            return Err("anim-edit-invalid");
        }
        if project.tracks.len() > 256 {
            return Err("anim-edit-invalid");
        }
        let mut tracks = Vec::with_capacity(project.tracks.len());
        let mut seen_bones = std::collections::HashSet::with_capacity(project.tracks.len());
        for source in project.tracks {
            let Some(&bone) = rig.names.get(&source.joint_name) else {
                return Err("anim-edit-unsupported-bones");
            };
            if !seen_bones.insert(bone) {
                return Err("anim-edit-invalid");
            }
            if source.rotations.len() > 16_384 || source.positions.len() > 16_384 || !(-1..=6).contains(&source.priority) {
                return Err("anim-edit-invalid");
            }
            let mut motion = JointMotion {
                joint_name: source.joint_name,
                priority: source.priority,
                rot_keys: Vec::with_capacity(source.rotations.len()),
                pos_keys: Vec::with_capacity(source.positions.len()),
            };
            for key in source.rotations {
                let rotation = Quat::from_array(key.value);
                if !key.time.is_finite()
                    || !(0.0..=60.0).contains(&key.time)
                    || !rotation.is_finite()
                    || rotation.length_squared() < 0.00001
                {
                    return Err("anim-edit-invalid");
                }
                motion.rot_keys.push(RotKey {
                    time: key.time,
                    rotation: rotation.normalize(),
                });
            }
            for key in source.positions {
                let position = Vec3::from_array(key.value);
                if !key.time.is_finite() || !(0.0..=60.0).contains(&key.time) || !position.is_finite() || position.abs().max_element() > 5.0
                {
                    return Err("anim-edit-invalid");
                }
                motion.pos_keys.push(PosKey { time: key.time, position });
            }
            motion.rot_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            motion.pos_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            tracks.push(Track {
                bone,
                enabled: source.enabled,
                authored: source.authored,
                base_position: base_position(rig, bone),
                motion,
                interpolation: source.interpolation,
                curve: source.curve.normalized(),
            });
        }
        let mut document = Self {
            name: project.name.chars().take(100).collect(),
            fps: project.fps,
            auto_keyframe: project.auto_keyframe,
            auto_key_advance_seconds: project.auto_key_advance_seconds,
            start: project.start,
            end: project.end,
            tracks,
            meta: Animation {
                base_priority: project.base_priority,
                duration: (project.end - project.start) as f32 / project.fps as f32,
                emote_name: project.emote_name.chars().take(100).collect(),
                loop_in: project.loop_in,
                loop_out: project.loop_out,
                looping: project.looping,
                ends_after: None,
                ease_in: project.ease_in,
                ease_out: project.ease_out,
                hand_pose: project.hand_pose,
                joints: Vec::new(),
            },
        };
        if !(0..=6).contains(&document.meta.base_priority)
            || document.meta.hand_pose > MAX_HAND_POSE
            || ![
                document.meta.loop_in,
                document.meta.loop_out,
                document.meta.ease_in,
                document.meta.ease_out,
            ]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err("anim-edit-invalid");
        }
        document.timing();
        Ok(document)
    }
    pub fn range(&mut self, start: u32, end: u32) {
        let old = self.meta.duration;
        let max_frame = self.max_frame();
        self.start = start.min(max_frame - 1);
        self.end = end.clamp(self.start + 1, max_frame);
        self.meta.duration = (self.end - self.start) as f32 / self.fps as f32;
        if (self.meta.loop_out - old).abs() < 0.0001 {
            self.meta.loop_out = self.meta.duration;
        }
        self.timing();
    }

    /// Scale every existing key's time around the clip start by the ratio of
    /// the new and old auto-advance steps. Values are quantized to the current
    /// FPS; reject collisions or durations beyond the native 60-second limit.
    /// A key before the clip start stays where it is: it is outside the clip
    /// (the hidden rest key at frame 0 of a clip that starts later), and
    /// scaled around the start it would land before frame 0.
    pub fn retime_keys_to_step(&mut self, old_step: f32, new_step: f32) -> bool {
        if !old_step.is_finite() || !new_step.is_finite() || old_step <= 0.0 || new_step <= 0.0 {
            return false;
        }
        let ratio = new_step / old_step;
        let start = self.start;
        let fps = self.fps;
        let max_frame = self.max_frame();
        let mut candidate = self.clone();
        let mut latest = candidate.end;
        for track in &mut candidate.tracks {
            let mut retime = |times: Vec<f32>| -> Option<Vec<f32>> {
                let mut previous = None;
                times
                    .into_iter()
                    .map(|time| {
                        if !time.is_finite() {
                            return None;
                        }
                        let offset = time * fps as f32 - start as f32;
                        let before_start = offset < -0.5;
                        let frame_value = if before_start {
                            (time * fps as f32).round()
                        } else {
                            (start as f32 + offset * ratio).round()
                        };
                        if !(0.0..=max_frame as f32).contains(&frame_value) {
                            return None;
                        }
                        let frame = frame_value as u32;
                        if frame > max_frame || previous.is_some_and(|prior| frame <= prior) {
                            return None;
                        }
                        previous = Some(frame);
                        if before_start {
                            return Some(time);
                        }
                        latest = latest.max(frame);
                        Some(frame as f32 / fps as f32)
                    })
                    .collect()
            };
            let Some(rotation_times) = retime(track.motion.rot_keys.iter().map(|key| key.time).collect()) else {
                return false;
            };
            let Some(position_times) = retime(track.motion.pos_keys.iter().map(|key| key.time).collect()) else {
                return false;
            };
            for (key, time) in track.motion.rot_keys.iter_mut().zip(rotation_times) {
                key.time = time;
            }
            for (key, time) in track.motion.pos_keys.iter_mut().zip(position_times) {
                key.time = time;
            }
        }
        if latest > max_frame {
            return false;
        }
        if latest > candidate.end {
            candidate.range(candidate.start, latest);
        }
        *self = candidate;
        true
    }
    pub fn timing(&mut self) {
        let d = self.meta.duration;
        let frame = 1.0 / self.fps as f32;
        self.meta.loop_in = self.meta.loop_in.clamp(0.0, (d - frame).max(0.0));
        self.meta.loop_out = self.meta.loop_out.clamp(self.meta.loop_in + frame, d);
        self.meta.ease_in = self.meta.ease_in.clamp(0.0, d);
        self.meta.ease_out = self.meta.ease_out.clamp(0.0, d);
        if !self.meta.looping && self.meta.ease_in + self.meta.ease_out > d {
            let factor = d / (self.meta.ease_in + self.meta.ease_out);
            self.meta.ease_in *= factor;
            self.meta.ease_out *= factor;
        }
    }
    pub fn set_fps(&mut self, fps: u32) {
        let fps = fps.clamp(1, 60);
        if fps == self.fps {
            return;
        }
        let scale = fps as f32 / self.fps as f32;
        let max_frame = MAX_DURATION as u32 * fps;
        self.start = ((self.start as f32 * scale).round() as u32).min(max_frame - 1);
        self.end = ((self.end as f32 * scale).round() as u32).clamp(self.start + 1, max_frame);
        self.fps = fps;
        self.meta.duration = (self.end - self.start) as f32 / fps as f32;
        self.timing();
    }
    pub fn record(&mut self, rig: &Rig, frame: &Frame, at: u32, bones: &[usize], translations: &[usize]) -> Result<(), &'static str> {
        if at > self.max_frame() {
            return Err("anim-edit-invalid");
        }
        let inverse = frame.model.inverse();
        let matrices: Vec<_> = frame.joints.iter().map(|m| inverse * *m).collect();
        let mut captured = Vec::new();
        for &bone in bones {
            let Some(name) = rig.canonical_names.get(bone).filter(|n| n.starts_with('m')) else {
                continue;
            };
            let Some(matrix) = matrices.get(bone) else {
                return Err("anim-edit-invalid");
            };
            let (_, q, p) = matrix.to_scale_rotation_translation();
            let parent = rig.parent[bone]
                .and_then(|j| matrices.get(j))
                .map_or(Quat::IDENTITY, |m| m.to_scale_rotation_translation().1);
            let q = (parent.inverse() * q).normalize();
            let local = if bone == rig.pelvis {
                p - rig.local_pos[bone]
            } else if let Some(j) = rig.parent[bone] {
                let (scale, _, parent_p) = matrices[j].to_scale_rotation_translation();
                if scale.abs().min_element() < 0.00001 {
                    return Err("anim-edit-invalid");
                }
                parent.inverse() * (p - parent_p) / scale
            } else {
                p
            };
            let previous = self.tracks.iter().find(|t| t.bone == bone);
            let position = bone == rig.pelvis
                || previous.is_some_and(|t| !t.motion.pos_keys.is_empty())
                || (translations.contains(&bone) && previous.is_none_or(|t| t.base_position.distance_squared(local) > 0.0001 * 0.0001));
            if !q.is_finite() || !local.is_finite() || (position && local.abs().max_element() > 5.0) {
                return Err("anim-edit-pelvis-bound");
            }
            captured.push((bone, name.clone(), q, local, position));
        }
        if captured.is_empty() {
            return Err("anim-edit-no-bones");
        }
        let time = at as f32 / self.fps as f32;
        for (bone, name, q, local, position) in captured {
            if !self.tracks.iter().any(|t| t.bone == bone) {
                self.tracks.push(Track {
                    bone,
                    enabled: true,
                    authored: true,
                    base_position: local,
                    interpolation: Interpolation::Linear,
                    curve: Curve::default(),
                    motion: JointMotion {
                        joint_name: name,
                        priority: -1,
                        rot_keys: Vec::new(),
                        pos_keys: Vec::new(),
                    },
                });
            }
            let Some(track) = self.tracks.iter_mut().find(|t| t.bone == bone) else {
                continue;
            };
            track.authored = true;
            upsert_key(&mut track.motion.rot_keys, at, self.fps, RotKey { time, rotation: q });
            if position {
                if track.motion.pos_keys.is_empty() && at > 0 {
                    track.motion.pos_keys.push(PosKey {
                        time: 0.0,
                        position: track.base_position,
                    });
                }
                upsert_key(&mut track.motion.pos_keys, at, self.fps, PosKey { time, position: local });
            }
        }
        if at < self.start || at > self.end {
            self.range(self.start.min(at), self.end.max(at));
        }
        Ok(())
    }
    /// The pose of the authored, enabled tracks at a frame. The baseline
    /// tracks a fresh draft seeds for every bone stay out: pasting them would
    /// make every untouched joint part of the exported animation.
    pub fn copy_keyframe(&self, at: u32) -> Vec<Keyframe> {
        let time = at as f32 / self.fps as f32;
        self.tracks
            .iter()
            .filter(|track| track.enabled && track.authored)
            .filter_map(|track| {
                let rotation = track.rotation_at(time);
                let position = track.position_at(time);
                (rotation.is_some() || position.is_some()).then_some(Keyframe {
                    bone: track.bone,
                    rotation,
                    position,
                })
            })
            .collect()
    }
    pub fn paste_keyframe(&mut self, at: u32, keys: &[Keyframe]) {
        let frame = at.min(self.max_frame());
        let time = frame_time(frame, self.fps);
        for key in keys {
            let Some(track) = self.tracks.iter_mut().find(|track| track.bone == key.bone) else {
                continue;
            };
            if let Some(rotation) = key.rotation {
                upsert_key(&mut track.motion.rot_keys, frame, self.fps, RotKey { time, rotation });
                track.authored = true;
            }
            if let Some(position) = key.position {
                upsert_key(&mut track.motion.pos_keys, frame, self.fps, PosKey { time, position });
                track.authored = true;
            }
        }
        if at < self.start || at > self.end {
            self.range(self.start.min(at), self.end.max(at));
        }
    }
    pub fn move_track_keyframe(&mut self, bone: usize, from: u32, to: u32) {
        let source = from.min(self.max_frame());
        let destination = to.min(self.max_frame());
        if source == destination {
            return;
        }
        let fps = self.fps;
        if let Some(track) = self.tracks.iter_mut().find(|track| track.bone == bone) {
            let has_source = has_key_on(&track.motion.rot_keys, source, fps) || has_key_on(&track.motion.pos_keys, source, fps);
            move_key(&mut track.motion.rot_keys, source, destination, fps);
            move_key(&mut track.motion.pos_keys, source, destination, fps);
            track.authored |= has_source;
        }
        if to < self.start || to > self.end {
            self.range(self.start.min(to), self.end.max(to));
        }
    }
    /// Delete every authored key the frame shows. The hidden baseline keys of
    /// a fresh draft's untouched tracks keep the preview at rest and stay.
    pub fn remove_keyframe(&mut self, at: u32) {
        let fps = self.fps;
        for track in self.tracks.iter_mut().filter(|track| track.authored) {
            remove_keys(&mut track.motion.rot_keys, at, fps);
            remove_keys(&mut track.motion.pos_keys, at, fps);
        }
    }
    pub fn remove_track_keyframe(&mut self, bone: usize, at: u32) {
        let fps = self.fps;
        if let Some(track) = self.tracks.iter_mut().find(|track| track.bone == bone) {
            remove_keys(&mut track.motion.rot_keys, at, fps);
            remove_keys(&mut track.motion.pos_keys, at, fps);
        }
    }
    #[cfg(test)]
    pub fn frames(&self, bone: Option<usize>) -> Vec<u32> {
        let mut frames: Vec<_> = self
            .tracks
            .iter()
            .filter(|t| bone.is_none_or(|b| b == t.bone))
            .flat_map(|t| {
                t.motion
                    .rot_keys
                    .iter()
                    .map(|k| k.time)
                    .chain(t.motion.pos_keys.iter().map(|k| k.time))
            })
            .map(|t| (t * self.fps as f32).round() as u32)
            .collect();
        frames.sort();
        frames.dedup();
        frames
    }
    pub fn animation(&self) -> Result<Animation, &'static str> {
        self.build(false)
    }
    /// Keep the editor's full timeline intact while matching file exports to
    /// the active last-authored-key loop range when the user requests it.
    pub fn animation_for_export(&self, last_key_only: bool) -> Result<Animation, &'static str> {
        if !last_key_only {
            return self.animation();
        }
        let Some(last_key) = self.last_authored_keyframe() else {
            return self.animation();
        };
        let mut export = self.clone();
        export.range(self.start, last_key.saturating_add(1).min(self.max_frame()));
        export.animation()
    }
    /// An empty preview releases every keyed channel. An empty upload remains
    /// invalid, but turning off the final track must not retain a stale sample.
    pub fn preview(&self) -> Result<Animation, &'static str> {
        self.build(true)
    }
    pub fn has_exportable_tracks(&self) -> bool {
        self.tracks
            .iter()
            .any(|track| track.enabled && track.authored && (!track.motion.rot_keys.is_empty() || !track.motion.pos_keys.is_empty()))
    }
    fn build(&self, allow_empty: bool) -> Result<Animation, &'static str> {
        if self.start >= self.end
            || self.end > self.max_frame()
            || !(0..=6).contains(&self.meta.base_priority)
            || self.meta.hand_pose > MAX_HAND_POSE
        {
            return Err("anim-edit-invalid");
        }
        if ![self.meta.loop_in, self.meta.loop_out, self.meta.ease_in, self.meta.ease_out]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err("anim-edit-invalid");
        }
        let mut animation = self.meta.clone();
        animation.duration = (self.end - self.start) as f32 / self.fps as f32;
        let start = self.start as f32 / self.fps as f32;
        let end = self.end as f32 / self.fps as f32;
        for track in self.tracks.iter().filter(|track| track.enabled && (allow_empty || track.authored)) {
            let source = &track.motion;
            if !(-1..=6).contains(&source.priority) {
                return Err("anim-edit-invalid");
            }
            let mut joint = JointMotion {
                joint_name: source.joint_name.clone(),
                priority: source.priority,
                rot_keys: Vec::new(),
                pos_keys: Vec::new(),
            };
            if let (Some(a), Some(b)) = (track.rotation_at(start), track.rotation_at(end)) {
                joint.rot_keys.push(RotKey { time: 0.0, rotation: a });
                if track.interpolation == Interpolation::Linear {
                    joint
                        .rot_keys
                        .extend(source.rot_keys.iter().filter(|k| k.time > start && k.time < end).map(|k| RotKey {
                            time: k.time - start,
                            rotation: k.rotation,
                        }));
                } else if track.interpolation == Interpolation::Stepped {
                    for (sample_time, output_time) in
                        stepped_key_times(source.rot_keys.iter().map(|key| key.time), start, end, animation.duration)
                    {
                        if let Some(rotation) = track.rotation_at(sample_time) {
                            joint.rot_keys.push(RotKey {
                                time: output_time,
                                rotation,
                            });
                        }
                    }
                } else if source.rot_keys.len() > 1 {
                    for (time, output_time) in eased_key_times(source.rot_keys.iter().map(|key| key.time), start, end) {
                        if let Some(rotation) = track.rotation_at(time) {
                            joint.rot_keys.push(RotKey {
                                time: output_time,
                                rotation,
                            });
                        }
                    }
                }
                joint.rot_keys.push(RotKey {
                    time: animation.duration,
                    rotation: b,
                });
            }
            if let (Some(a), Some(b)) = (track.position_at(start), track.position_at(end)) {
                joint.pos_keys.push(PosKey { time: 0.0, position: a });
                if track.interpolation == Interpolation::Linear {
                    joint
                        .pos_keys
                        .extend(source.pos_keys.iter().filter(|k| k.time > start && k.time < end).map(|k| PosKey {
                            time: k.time - start,
                            position: k.position,
                        }));
                } else if track.interpolation == Interpolation::Stepped {
                    for (sample_time, output_time) in
                        stepped_key_times(source.pos_keys.iter().map(|key| key.time), start, end, animation.duration)
                    {
                        if let Some(position) = track.position_at(sample_time) {
                            joint.pos_keys.push(PosKey {
                                time: output_time,
                                position,
                            });
                        }
                    }
                } else if source.pos_keys.len() > 1 {
                    for (time, output_time) in eased_key_times(source.pos_keys.iter().map(|key| key.time), start, end) {
                        if let Some(position) = track.position_at(time) {
                            joint.pos_keys.push(PosKey {
                                time: output_time,
                                position,
                            });
                        }
                    }
                }
                joint.pos_keys.push(PosKey {
                    time: animation.duration,
                    position: b,
                });
            }
            if !joint.rot_keys.is_empty() || !joint.pos_keys.is_empty() {
                if joint.rot_keys.len() > MAX_ANIM_KEYS || joint.pos_keys.len() > MAX_ANIM_KEYS {
                    return Err("anim-edit-invalid");
                }
                animation.joints.push(joint);
            }
        }
        if animation.joints.is_empty() && !allow_empty {
            return Err("anim-edit-no-bones");
        }
        Ok(animation)
    }
}

/// The position a track's keys start from when its first position key is
/// recorded after frame 0. `Document::record` stores the pelvis as an offset
/// from its rest position and every other joint as its position under its
/// parent, so a loaded track takes the same: no offset for the pelvis (its
/// rest height here would lift the avatar by it), the rest position else.
fn base_position(rig: &Rig, bone: usize) -> Vec3 {
    if bone == rig.pelvis { Vec3::ZERO } else { rig.local_pos[bone] }
}

/// A key belongs to the frame its time rounds to, exactly as the timeline
/// draws it (`State::timeline_tracks`). Every edit matches keys this way, so a
/// key off the frame grid, from an imported `.anim`/`.bvh` or left between
/// frames by an FPS change, can still be deleted, dragged or replaced from the
/// frame that shows it.
pub(super) fn key_on_frame(time: f32, frame: u32, fps: u32) -> bool {
    time.is_finite() && (time * fps as f32).round() == frame as f32
}

fn frame_time(frame: u32, fps: u32) -> f32 {
    frame as f32 / fps as f32
}

/// Rotation and position keys share the frame rules below.
trait Timed: Clone {
    fn time(&self) -> f32;
    fn set_time(&mut self, time: f32);
}

impl Timed for RotKey {
    fn time(&self) -> f32 {
        self.time
    }
    fn set_time(&mut self, time: f32) {
        self.time = time;
    }
}

impl Timed for PosKey {
    fn time(&self) -> f32 {
        self.time
    }
    fn set_time(&mut self, time: f32) {
        self.time = time;
    }
}

fn has_key_on<K: Timed>(keys: &[K], frame: u32, fps: u32) -> bool {
    keys.iter().any(|key| key_on_frame(key.time(), frame, fps))
}

fn remove_keys<K: Timed>(keys: &mut Vec<K>, frame: u32, fps: u32) {
    keys.retain(|key| !key_on_frame(key.time(), frame, fps));
}

/// One key per frame: the new key, at the frame's exact time, replaces every
/// key the frame shows (several when a denser import rounds onto it).
fn upsert_key<K: Timed>(keys: &mut Vec<K>, frame: u32, fps: u32, mut key: K) {
    key.set_time(frame_time(frame, fps));
    remove_keys(keys, frame, fps);
    keys.push(key);
    keys.sort_by(|a, b| a.time().total_cmp(&b.time()));
}

/// Move the frame's last key onto another frame, snapped to that frame's exact
/// time. The destination's own keys are replaced, in either drag direction.
fn move_key<K: Timed>(keys: &mut Vec<K>, from: u32, to: u32, fps: u32) {
    let Some(mut moved) = keys.iter().rfind(|key| key_on_frame(key.time(), from, fps)).cloned() else {
        return;
    };
    moved.set_time(frame_time(to, fps));
    keys.retain(|key| !key_on_frame(key.time(), from, fps) && !key_on_frame(key.time(), to, fps));
    keys.push(moved);
    keys.sort_by(|a, b| a.time().total_cmp(&b.time()));
}
