//! Keyframe animation assets (`animatn`), port of
//! `LLKeyframeMotion::deserialize` (indra/llcharacter/llkeyframemotion.cpp,
//! Linden Research, originally LGPL 2.1).

use crate::{AssetError, ByteReader};
use glam::{Quat, Vec3};

const MAX_ANIM_DURATION: f32 = 60.0;
const MAX_JOINTS: usize = 256;
const MAX_KEYS: usize = 16384;
const LL_MAX_PELVIS_OFFSET: f32 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RotKey {
    pub time: f32,
    pub rotation: Quat,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PosKey {
    pub time: f32,
    pub position: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JointMotion {
    pub joint_name: String,
    pub priority: i32,
    pub rot_keys: Vec<RotKey>,
    pub pos_keys: Vec<PosKey>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub base_priority: i32,
    pub duration: f32,
    pub emote_name: String,
    pub loop_in: f32,
    pub loop_out: f32,
    pub looping: bool,
    pub ease_in: f32,
    pub ease_out: f32,
    pub hand_pose: u32,
    pub joints: Vec<JointMotion>,
}

#[inline]
fn u16_to_f32(v: u16, lower: f32, upper: f32) -> f32 {
    let delta = upper - lower;
    let mut val = v as f32 / 65535.0 * delta + lower;
    if val.abs() < delta / 65535.0 {
        val = 0.0;
    }
    val
}

fn unpack_quat(v: Vec3) -> Quat {
    let t = 1.0 - v.length_squared();
    let w = if t > 0.0 { t.sqrt() } else { 0.0 };
    let q = Quat::from_xyzw(v.x, v.y, v.z, w);
    let n = q.normalize();
    if n.is_finite() { n } else { Quat::IDENTITY }
}

/// Euler angles (radians) as LL's `LLQuaternion::setQuat(roll, pitch, yaw)`.
fn ll_euler(r: Vec3) -> Quat {
    let (sr, cr) = (r.x * 0.5).sin_cos();
    let (sp, cp) = (r.y * 0.5).sin_cos();
    let (sy, cy) = (r.z * 0.5).sin_cos();
    Quat::from_xyzw(
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
        cr * cp * cy + sr * sp * sy,
    )
    .normalize()
}

fn cstring(r: &mut ByteReader) -> Result<String, AssetError> {
    let mut bytes = Vec::new();
    loop {
        let b = r.u8("string")?;
        if b == 0 {
            break;
        }
        bytes.push(b);
        if bytes.len() > 512 {
            return Err(AssetError::invalid("string too long"));
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn parse_animation(data: &[u8]) -> Result<Animation, AssetError> {
    let mut r = ByteReader::new(data);
    let version = r.u16("version")?;
    let sub_version = r.u16("sub_version")?;
    let old_version = match (version, sub_version) {
        (0, 1) => true,
        (1, 0) => false,
        _ => return Err(AssetError::Unsupported(format!("anim version {version}.{sub_version}"))),
    };
    let base_priority = r.i32("base_priority")?.clamp(-1, 6);
    let duration = r.f32("duration")?;
    if !duration.is_finite() || duration > MAX_ANIM_DURATION || duration < 0.0 {
        return Err(AssetError::invalid("bad duration"));
    }
    let emote_name = cstring(&mut r)?;
    let loop_in = r.f32("loop_in")?;
    let loop_out = r.f32("loop_out")?;
    let looping = r.i32("loop")? != 0;
    let ease_in = r.f32("ease_in")?;
    let ease_out = r.f32("ease_out")?;
    if ![loop_in, loop_out, ease_in, ease_out].iter().all(|v| v.is_finite()) {
        return Err(AssetError::invalid("bad animation timing"));
    }
    let hand_pose = r.u32("hand_pose")?;
    let num_joints = r.u32("num_joints")? as usize;
    if num_joints > MAX_JOINTS {
        return Err(AssetError::invalid("too many joints"));
    }
    let mut joints = Vec::with_capacity(num_joints);
    for _ in 0..num_joints {
        let joint_name = cstring(&mut r)?;
        let priority = r.i32("joint_priority")?;
        let nrot = r.i32("num_rot_keys")?;
        if nrot < 0 || nrot as usize > MAX_KEYS {
            return Err(AssetError::invalid("bad rot key count"));
        }
        let mut rot_keys = Vec::with_capacity(nrot as usize);
        for _ in 0..nrot {
            let (time, rotation) = if old_version {
                let t = r.f32("time")?;
                let v = Vec3::new(r.f32("rx")?, r.f32("ry")?, r.f32("rz")?);
                (t, ll_euler(v))
            } else {
                let t = u16_to_f32(r.u16("time")?, 0.0, duration);
                let x = u16_to_f32(r.u16("rx")?, -1.0, 1.0);
                let y = u16_to_f32(r.u16("ry")?, -1.0, 1.0);
                let z = u16_to_f32(r.u16("rz")?, -1.0, 1.0);
                (t, unpack_quat(Vec3::new(x, y, z)))
            };
            if time.is_finite() && rotation.is_finite() {
                rot_keys.push(RotKey { time, rotation });
            }
        }
        let npos = r.i32("num_pos_keys")?;
        if npos < 0 || npos as usize > MAX_KEYS {
            return Err(AssetError::invalid("bad pos key count"));
        }
        let mut pos_keys = Vec::with_capacity(npos as usize);
        for _ in 0..npos {
            let (time, position) = if old_version {
                let t = r.f32("time")?;
                (t, Vec3::new(r.f32("px")?, r.f32("py")?, r.f32("pz")?))
            } else {
                let t = u16_to_f32(r.u16("time")?, 0.0, duration);
                let m = LL_MAX_PELVIS_OFFSET;
                let x = u16_to_f32(r.u16("px")?, -m, m);
                let y = u16_to_f32(r.u16("py")?, -m, m);
                let z = u16_to_f32(r.u16("pz")?, -m, m);
                (t, Vec3::new(x, y, z))
            };
            if time.is_finite() && position.is_finite() {
                pos_keys.push(PosKey { time, position });
            }
        }
        rot_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        pos_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        // LLKeyframeMotion::deserialize assigns mKeys[time]: the last
        // serialized value wins when quantized timestamps coincide.
        rot_keys.dedup_by(|later, earlier| {
            if later.time == earlier.time {
                *earlier = later.clone();
                true
            } else {
                false
            }
        });
        pos_keys.dedup_by(|later, earlier| {
            if later.time == earlier.time {
                *earlier = later.clone();
                true
            } else {
                false
            }
        });
        if joint_name != "mScreen" && joint_name != "mRoot" {
            joints.push(JointMotion {
                joint_name,
                priority,
                rot_keys,
                pos_keys,
            });
        }
    }
    // Constraints follow; they are not needed for playback here.
    Ok(Animation {
        base_priority,
        duration,
        emote_name,
        loop_in: loop_in.clamp(0.0, duration),
        loop_out: loop_out.clamp(0.0, duration),
        looping,
        ease_in: ease_in.max(0.0),
        ease_out: ease_out.max(0.0),
        hand_pose,
        joints,
    })
}

impl Animation {
    /// Local animation time for an animation started `elapsed` seconds ago.
    /// Port of LLKeyframeMotion::onUpdate; keep the introduction and the
    /// first passage through loop_out. Only key sampling narrows to f32.
    pub fn local_time(&self, elapsed: f64) -> f64 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        let (a, b) = (f64::from(self.loop_in), f64::from(self.loop_out));
        if self.looping && elapsed > b {
            if b > a { a + (elapsed - b).rem_euclid(b - a) } else { b }
        } else {
            elapsed.max(0.0).min(f64::from(self.duration))
        }
    }

    /// Blend weight from ease in/out (non-looping anims fade out at the end).
    pub fn weight(&self, elapsed: f64) -> f32 {
        let mut w = 1.0;
        if self.ease_in > 0.0 {
            w = cubic_step(elapsed / f64::from(self.ease_in));
        }
        if !self.looping && self.ease_out > 0.0 {
            w = w.min(cubic_step((f64::from(self.duration) - elapsed) / f64::from(self.ease_out)));
        }
        w
    }

    /// Periodic completion of missing boundary intervals is an intentional
    /// adaptation of Aurora's sampler, not Firestorm's clamped getValue.
    pub fn loop_range(&self) -> Option<(f32, f32)> {
        (self.looping && self.loop_out > self.loop_in).then_some((self.loop_in, self.loop_out))
    }
}

/// LLMotionController::updateMotionsByType, bounded cubic transition.
pub fn cubic_step(u: f64) -> f32 {
    let u = u.clamp(0.0, 1.0);
    (u * u * (3.0 - 2.0 * u)) as f32
}

impl JointMotion {
    pub fn rotation_at(&self, t: f32) -> Option<Quat> {
        self.rotation_at_loop(t, None)
    }

    pub fn position_at(&self, t: f32) -> Option<Vec3> {
        self.position_at_loop(t, None)
    }

    pub fn rotation_at_loop(&self, t: f32, range: Option<(f32, f32)>) -> Option<Quat> {
        sample_curve(&self.rot_keys, t, range, &|k| (k.time, k.rotation), &|a, b, u| a.slerp(b, u))
    }

    pub fn position_at_loop(&self, t: f32, range: Option<(f32, f32)>) -> Option<Vec3> {
        sample_curve(&self.pos_keys, t, range, &|k| (k.time, k.position), &|a, b, u| a.lerp(b, u))
    }
}

fn sample_curve<K, T: Copy>(
    keys: &[K],
    t: f32,
    range: Option<(f32, f32)>,
    key: &impl Fn(&K) -> (f32, T),
    interpolate: &impl Fn(T, T, f32) -> T,
) -> Option<T> {
    let point = |k: &K| {
        let (time, value) = key(k);
        (f64::from(time), value)
    };
    let first = point(keys.first()?);
    let last = point(keys.last()?);
    let sample_time = f64::from(t);
    let between = |left: (f64, T), right: (f64, T)| {
        let span = right.0 - left.0;
        let u = if span > 0.0 {
            ((sample_time - left.0) / span).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        interpolate(left.1, right.1, u)
    };
    if let Some((a, b)) = range.filter(|&(a, b)| b > a && t >= a && t <= b) {
        let (start, end) = (f64::from(a), f64::from(b));
        // Never replace explicit keys at either boundary. When intro/exit
        // keys exist, sample their boundary value with the ordinary sampler.
        if sample_time < first.0 && first.0 <= end {
            let left = if last.0 <= end {
                (last.0 - (end - start), last.1)
            } else {
                (start, sample_curve(keys, b, None, key, interpolate)?)
            };
            return Some(between(left, first));
        }
        if sample_time > last.0 && last.0 >= start {
            let right = if first.0 >= start {
                (first.0 + (end - start), first.1)
            } else {
                (end, sample_curve(keys, a, None, key, interpolate)?)
            };
            return Some(between(last, right));
        }
    }
    if sample_time <= first.0 {
        return Some(first.1);
    }
    let right = keys.partition_point(|k| key(k).0 <= t);
    if right == keys.len() {
        Some(last.1)
    } else {
        Some(between(point(&keys[right - 1]), point(&keys[right])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Vec<u8> {
        build_keys(&[(0, 32767), (65535, 40000)], &[])
    }

    fn build_keys(rot: &[(u16, u16)], pos: &[(u16, u16)]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&3i32.to_le_bytes());
        v.extend_from_slice(&2.0f32.to_le_bytes());
        v.extend_from_slice(b"\0");
        v.extend_from_slice(&0.0f32.to_le_bytes());
        v.extend_from_slice(&2.0f32.to_le_bytes());
        v.extend_from_slice(&1i32.to_le_bytes());
        v.extend_from_slice(&0.5f32.to_le_bytes());
        v.extend_from_slice(&0.5f32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(b"mShoulderLeft\0");
        v.extend_from_slice(&3i32.to_le_bytes());
        v.extend_from_slice(&(rot.len() as i32).to_le_bytes());
        for (t, z) in rot {
            v.extend_from_slice(&t.to_le_bytes());
            v.extend_from_slice(&32767u16.to_le_bytes());
            v.extend_from_slice(&32767u16.to_le_bytes());
            v.extend_from_slice(&z.to_le_bytes());
        }
        v.extend_from_slice(&(pos.len() as i32).to_le_bytes());
        for (t, x) in pos {
            v.extend_from_slice(&t.to_le_bytes());
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&32767u16.to_le_bytes());
            v.extend_from_slice(&32767u16.to_le_bytes());
        }
        v.extend_from_slice(&0i32.to_le_bytes()); // constraints
        v
    }

    #[test]
    fn parses_and_samples() {
        let a = parse_animation(&build()).unwrap();
        assert_eq!(a.joints.len(), 1);
        assert_eq!(a.joints[0].rot_keys.len(), 2);
        assert!((a.duration - 2.0).abs() < 1e-6);
        let r0 = a.joints[0].rotation_at(0.0).unwrap();
        assert!(r0.dot(Quat::IDENTITY).abs() > 0.999);
        let r1 = a.joints[0].rotation_at(2.0).unwrap();
        assert!(r1.z > 0.1);
        assert!((a.local_time(3.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn garbage_safe() {
        let b = build();
        for n in 0..b.len() {
            let _ = parse_animation(&b[..n]);
        }
    }

    fn curve(keys: &[(f32, f32)]) -> JointMotion {
        JointMotion {
            joint_name: "mPelvis".into(),
            priority: 3,
            rot_keys: keys
                .iter()
                .map(|&(time, angle)| RotKey {
                    time,
                    rotation: Quat::from_rotation_z(angle),
                })
                .collect(),
            pos_keys: keys
                .iter()
                .map(|&(time, x)| PosKey {
                    time,
                    position: Vec3::new(x, 0.0, 0.0),
                })
                .collect(),
        }
    }

    fn assert_pose_near(joint: &JointMotion, a: f32, b: f32, range: Option<(f32, f32)>, tolerance: f32) {
        let pa = joint.position_at_loop(a, range).expect("position");
        let pb = joint.position_at_loop(b, range).expect("position");
        let ra = joint.rotation_at_loop(a, range).expect("rotation") * Vec3::X;
        let rb = joint.rotation_at_loop(b, range).expect("rotation") * Vec3::X;
        assert!((pa - pb).length() < tolerance, "position {a} / {b}: {pa:?} / {pb:?}");
        assert!((ra - rb).length() < tolerance, "rotation {a} / {b}: {ra:?} / {rb:?}");
    }

    #[test]
    fn loop_seams_preserve_poses_for_one_hundred_cycles() {
        let mut a = parse_animation(&build()).expect("animation");
        a.duration = 1.0;
        a.loop_out = 1.0;
        // Different first/last poses, first key after A as in ordinary SL assets.
        let joint = curve(&[(1.0 / 15.0, 1.0), (0.5, 0.3), (1.0, -1.0)]);
        let unchanged = joint.clone();
        for cycle in 1..=100 {
            let before = a.local_time(f64::from(cycle) - 1e-6) as f32;
            let after = a.local_time(f64::from(cycle) + 1e-6) as f32;
            assert_pose_near(&joint, before, after, a.loop_range(), 5e-5);
        }
        let mid = joint.position_at_loop(1.0 / 30.0, a.loop_range()).expect("position");
        assert!(mid.x.abs() < 1e-5, "missing interval must interpolate: {mid:?}");
        let direction = joint.rotation_at_loop(1.0 / 30.0, a.loop_range()).expect("rotation") * Vec3::X;
        assert!((direction - Vec3::X).length() < 1e-5);
        assert_eq!(joint, unchanged);
    }

    #[test]
    fn both_missing_intervals_and_each_curve_use_their_own_neighbors() {
        let mut joint = curve(&[(0.2, 1.0), (0.8, -1.0)]);
        joint.pos_keys = curve(&[(0.1, 1.0), (0.9, -1.0)]).pos_keys;
        let range = Some((0.0, 1.0));
        assert_pose_near(&joint, 1.0 - 1e-6, 1e-6, range, 3e-5);
        assert!(joint.position_at_loop(0.05, range).expect("position").x > 0.49);
        let angle = joint.rotation_at_loop(0.05, range).expect("rotation").to_axis_angle().1;
        assert!((angle - 0.25).abs() < 1e-5);
    }

    #[test]
    fn partial_loops_use_exit_or_intro_values_at_the_missing_boundary() {
        let range = Some((1.0, 2.5));
        let exit = curve(&[(1.2, 1.0), (2.0, 0.5), (3.0, -0.5)]);
        assert_pose_near(&exit, 2.5 - 1e-6, 1.0 + 1e-6, range, 1e-5);
        let mid = exit.position_at_loop(1.1, range).expect("position").x;
        assert!((mid - 0.5).abs() < 1e-5);
        // Outside the loop, preserve normal exit sampling and pre-loop hold.
        assert_eq!(exit.position_at_loop(2.7, range), exit.position_at(2.7));
        assert_eq!(exit.rotation_at_loop(0.8, range), exit.rotation_at(0.8));

        let intro = curve(&[(0.0, -0.5), (1.5, 1.0), (2.0, -0.5)]);
        assert_pose_near(&intro, 2.5 - 1e-6, 1.0 + 1e-6, range, 1e-5);
        let mid = intro.position_at_loop(2.25, range).expect("position").x;
        assert!(mid.abs() < 1e-5);
        assert_eq!(intro.position_at_loop(0.5, range), intro.position_at(0.5));
        assert_eq!(intro.rotation_at_loop(2.8, range), intro.rotation_at(2.8));
    }

    #[test]
    fn explicit_discontinuities_empty_and_single_key_curves_are_preserved() {
        let joint = curve(&[(0.0, 1.0), (1.0, -1.0)]);
        let range = Some((0.0, 1.0));
        assert_eq!(joint.position_at_loop(0.0, range).expect("A").x, 1.0);
        assert_eq!(joint.position_at_loop(1.0, range).expect("B").x, -1.0);
        assert_eq!(joint.rotation_at_loop(0.0, range), joint.rotation_at(0.0));
        assert_eq!(joint.rotation_at_loop(1.0, range), joint.rotation_at(1.0));
        let single = curve(&[(0.3, 0.7)]);
        for t in [0.0, 0.2, 0.8, 1.0] {
            assert_eq!(single.position_at_loop(t, range), single.position_at(t));
            assert_eq!(single.rotation_at_loop(t, range), single.rotation_at(t));
        }
        assert_eq!(curve(&[]).rotation_at_loop(0.0, range), None);
        assert_eq!(curve(&[]).position_at_loop(0.0, range), None);
    }

    #[test]
    fn phase_keeps_first_passage_degenerate_bounds_and_short_loop_precision() {
        let mut a = parse_animation(&build()).expect("animation");
        a.loop_in = 0.5;
        a.loop_out = 1.5;
        for elapsed in [0.0, 0.25, 1.0, 1.5] {
            assert_eq!(a.local_time(elapsed), elapsed);
        }
        assert!((a.local_time(2.0) - 1.0).abs() < 1e-12);
        a.loop_out = a.loop_in;
        assert_eq!(a.local_time(100.0), 0.5);
        a.loop_out = 0.0;
        assert_eq!(a.local_time(100.0), 0.0);
        a.loop_in = 0.0;
        a.loop_out = 0.000005;
        let span = f64::from(a.loop_out);
        let elapsed = span * 10_000_000_000.0 + span * 0.25;
        assert!((a.local_time(elapsed) - span * 0.25).abs() < 1e-10);
        // No artificial minimum span in interpolation either.
        let joint = curve(&[(0.000001, 1.0), (0.000005, -1.0)]);
        assert!(joint.position_at_loop(0.0000005, a.loop_range()).expect("position").x.abs() < 1e-5);
    }

    #[test]
    fn quantized_duplicate_timestamps_keep_last_serialized_key() {
        let data = build_keys(&[(65535, 40000), (0, 32767), (65535, 45000)], &[(20000, 40000), (20000, 50000)]);
        let a = parse_animation(&data).expect("duplicates");
        let expected = parse_animation(&build_keys(&[(0, 32767), (65535, 45000)], &[(20000, 50000)])).expect("unique");
        assert_eq!(a.joints, expected.joints);
    }

    #[test]
    fn transitions_are_cubic_and_asset_timing_must_be_finite() {
        let a = parse_animation(&build()).expect("animation");
        assert!((a.weight(0.125) - 0.15625).abs() < 1e-6);
        assert_eq!(a.weight(10.0), 1.0); // ease-in never wraps with phase
        for offset in [13, 17, 25, 29] {
            let mut data = build();
            data[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
            assert!(parse_animation(&data).is_err());
        }
        let mut data = build();
        data[17..21].copy_from_slice(&0.0f32.to_le_bytes());
        let zero = parse_animation(&data).expect("zero loop out");
        assert_eq!(zero.loop_out, 0.0);
        assert_eq!(zero.local_time(10.0), 0.0);
    }
}
