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
        loop_out: if loop_out <= 0.0 { duration } else { loop_out.clamp(0.0, duration) },
        looping,
        ease_in: ease_in.max(0.0),
        ease_out: ease_out.max(0.0),
        hand_pose,
        joints,
    })
}

impl Animation {
    /// Local animation time for an animation started `elapsed` seconds ago.
    pub fn local_time(&self, elapsed: f32) -> f32 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        if self.looping {
            if elapsed <= self.loop_out {
                return elapsed;
            }
            let span = (self.loop_out - self.loop_in).max(1e-3);
            self.loop_in + (elapsed - self.loop_out).rem_euclid(span)
        } else {
            elapsed.min(self.duration)
        }
    }

    /// Blend weight from ease in/out (non-looping anims fade out at the end).
    pub fn weight(&self, elapsed: f32) -> f32 {
        let mut w = 1.0f32;
        if self.ease_in > 0.0 {
            w = w.min((elapsed / self.ease_in).clamp(0.0, 1.0));
        }
        if !self.looping && self.ease_out > 0.0 {
            w = w.min(((self.duration - elapsed) / self.ease_out).clamp(0.0, 1.0));
        }
        w
    }
}

impl JointMotion {
    pub fn rotation_at(&self, t: f32) -> Option<Quat> {
        let k = &self.rot_keys;
        let first = k.first()?;
        if k.len() == 1 || t <= first.time {
            return Some(first.rotation);
        }
        for w in k.windows(2) {
            if t <= w[1].time {
                let span = (w[1].time - w[0].time).max(1e-5);
                let f = ((t - w[0].time) / span).clamp(0.0, 1.0);
                return Some(w[0].rotation.slerp(w[1].rotation, f));
            }
        }
        k.last().map(|l| l.rotation)
    }

    pub fn position_at(&self, t: f32) -> Option<Vec3> {
        let k = &self.pos_keys;
        let first = k.first()?;
        if k.len() == 1 || t <= first.time {
            return Some(first.position);
        }
        for w in k.windows(2) {
            if t <= w[1].time {
                let span = (w[1].time - w[0].time).max(1e-5);
                let f = ((t - w[0].time) / span).clamp(0.0, 1.0);
                return Some(w[0].position.lerp(w[1].position, f));
            }
        }
        k.last().map(|l| l.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Vec<u8> {
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
        v.extend_from_slice(&2i32.to_le_bytes());
        for (t, z) in [(0u16, 32767u16), (65535, 40000)] {
            v.extend_from_slice(&t.to_le_bytes());
            v.extend_from_slice(&32767u16.to_le_bytes());
            v.extend_from_slice(&32767u16.to_le_bytes());
            v.extend_from_slice(&z.to_le_bytes());
        }
        v.extend_from_slice(&0i32.to_le_bytes());
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
}
