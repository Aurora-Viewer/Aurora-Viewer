//! BVH import, based on LLBVHLoader::loadBVHFile / makeTranslation /
//! serialize (Firestorm indra/llcharacter, originally LGPL 2.1). Bounds and
//! parsing are independent of the viewer and run on its background workers.

use crate::{
    Animation, AssetError, JointMotion, Skeleton,
    anim::{PosKey, RotKey},
};
use glam::{Mat3, Quat, Vec3};

#[derive(Debug, Clone, Copy)]
enum Channel {
    Position(usize),
    Rotation(usize),
}
#[derive(Debug)]
struct Joint {
    name: String,
    channels: Vec<Channel>,
    offset: usize,
}

#[derive(Debug, Clone)]
pub struct Bvh {
    pub animation: Animation,
    pub frames: usize,
    pub frame_time: f32,
}

struct Tokens<'a> {
    tokens: std::str::SplitWhitespace<'a>,
    index: usize,
}
impl<'a> Tokens<'a> {
    fn next(&mut self) -> Result<&'a str, AssetError> {
        self.index += 1;
        self.tokens.next().ok_or(AssetError::Truncated("BVH token"))
    }
    fn require(&mut self, expected: &str) -> Result<(), AssetError> {
        if self.next()? == expected {
            Ok(())
        } else {
            Err(AssetError::invalid(format!("BVH: expected {expected} at token {}", self.index)))
        }
    }
    fn float(&mut self) -> Result<f32, AssetError> {
        self.next()?
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| AssetError::invalid(format!("BVH number at token {}", self.index)))
    }
    fn count(&mut self, maximum: usize) -> Result<usize, AssetError> {
        self.next()?
            .parse::<usize>()
            .ok()
            .filter(|n| *n <= maximum)
            .ok_or_else(|| AssetError::invalid("BVH count"))
    }
    fn joint(&mut self, joints: &mut Vec<Joint>, offset: &mut usize, depth: usize) -> Result<(), AssetError> {
        if depth > 64 || joints.len() >= 256 {
            return Err(AssetError::TooLarge("BVH hierarchy"));
        }
        let name = self.next()?.to_owned();
        if name.len() > 255 {
            return Err(AssetError::invalid("BVH joint name"));
        }
        self.require("{")?;
        self.require("OFFSET")?;
        for _ in 0..3 {
            self.float()?;
        }
        self.require("CHANNELS")?;
        let count = self.count(6)?;
        if !matches!(count, 3 | 6) {
            return Err(AssetError::invalid("BVH joint needs 3 or 6 channels"));
        }
        let mut channels = Vec::with_capacity(count);
        let mut used = [false; 6];
        for _ in 0..count {
            let channel = self.next()?;
            let (axis, kind) = match channel {
                "Xposition" => (0, false),
                "Yposition" => (1, false),
                "Zposition" => (2, false),
                "Xrotation" => (0, true),
                "Yrotation" => (1, true),
                "Zrotation" => (2, true),
                _ => return Err(AssetError::invalid("unknown BVH channel")),
            };
            let index = axis + if kind { 3 } else { 0 };
            if used[index] {
                return Err(AssetError::invalid("duplicate BVH channel"));
            }
            used[index] = true;
            channels.push(if kind { Channel::Rotation(axis) } else { Channel::Position(axis) });
        }
        if !used[3..].iter().all(|b| *b) {
            return Err(AssetError::invalid("missing BVH rotation axis"));
        }
        joints.push(Joint {
            name,
            channels,
            offset: *offset,
        });
        *offset += count;
        loop {
            match self.next()? {
                "}" => break,
                "JOINT" => self.joint(joints, offset, depth + 1)?,
                "End" => {
                    self.require("Site")?;
                    self.require("{")?;
                    self.require("OFFSET")?;
                    for _ in 0..3 {
                        self.float()?;
                    }
                    self.require("}")?;
                }
                _ => return Err(AssetError::invalid("BVH hierarchy")),
            }
        }
        Ok(())
    }
}

pub fn parse_bvh(bytes: &[u8], skeleton: &Skeleton) -> Result<Bvh, AssetError> {
    parse_bvh_channels(bytes, skeleton, false)
}

/// A pose needs every static channel, including an entirely neutral T-pose.
/// Ordinary motion imports retain their existing reference-frame reduction.
pub fn parse_bvh_pose(bytes: &[u8], skeleton: &Skeleton) -> Result<Bvh, AssetError> {
    parse_bvh_channels(bytes, skeleton, true)
}
fn parse_bvh_channels(bytes: &[u8], skeleton: &Skeleton, keep_static: bool) -> Result<Bvh, AssetError> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(AssetError::TooLarge("BVH file"));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| AssetError::invalid("BVH UTF-8"))?
        .replace('{', " { ")
        .replace('}', " } ");
    let mut tokens = Tokens {
        tokens: text.split_whitespace(),
        index: 0,
    };
    tokens.require("HIERARCHY")?;
    tokens.require("ROOT")?;
    let mut joints = Vec::new();
    let mut channels = 0;
    tokens.joint(&mut joints, &mut channels, 0)?;
    if skeleton.find(&joints[0].name).is_none_or(|i| skeleton.joints[i].name != "mPelvis") {
        return Err(AssetError::invalid("BVH root must be pelvis or an alias"));
    }
    tokens.require("MOTION")?;
    tokens.require("Frames:")?;
    let frames = tokens.count(16384)?;
    if frames == 0 {
        return Err(AssetError::invalid("empty BVH"));
    }
    tokens.require("Frame")?;
    tokens.require("Time:")?;
    let frame_time = tokens.float()?;
    let duration = frames.saturating_sub(2).max(1) as f32 * frame_time;
    if frame_time <= 0.0 || duration > 60.0 {
        return Err(AssetError::invalid("BVH duration exceeds 60 seconds or frame time is invalid"));
    }
    if frames.checked_mul(channels).is_none_or(|n| n > 8 * 1024 * 1024) {
        return Err(AssetError::TooLarge("BVH motion"));
    }
    let mut motion = Vec::with_capacity(frames * channels);
    for _ in 0..frames * channels {
        motion.push(tokens.float()?);
    }
    if tokens.tokens.next().is_some() {
        return Err(AssetError::invalid("extra BVH motion data"));
    }
    // BVH Y-up inches -> SL Z-up meters, from makeTranslation's frame matrix.
    let basis = Quat::from_mat3(&Mat3::from_cols(Vec3::Y, Vec3::Z, Vec3::X));
    let mut tracks = Vec::new();
    for joint in joints {
        let Some(index) = skeleton.find(&joint.name) else {
            continue;
        };
        let name = &skeleton.joints[index].name;
        let pose = |frame: usize| {
            let values = &motion[frame * channels + joint.offset..];
            let mut position = Vec3::ZERO;
            let mut rotation = Quat::IDENTITY;
            // mayaQ applies channels in their declared order; Hamilton product
            // is reversed relative to LLQuaternion's product.
            for (channel, value) in joint.channels.iter().zip(values) {
                match *channel {
                    Channel::Position(axis) => position[axis] = *value,
                    Channel::Rotation(axis) => {
                        rotation = Quat::from_axis_angle([Vec3::X, Vec3::Y, Vec3::Z][axis], value.to_radians()) * rotation
                    }
                }
            }
            (position, rotation.normalize())
        };
        let (reference_position, reference_rotation) = pose(0);
        let relative = name == "mPelvis";
        let first = if frames > 1 { 1 } else { 0 };
        let mut rotations = Vec::new();
        let mut positions = Vec::new();
        let mut rotation_changed = frames == 1 || keep_static;
        let mut position_changed = frames == 1 || keep_static;
        for frame in first..frames {
            let (position, rotation) = pose(frame);
            rotation_changed |= ((rotation * Vec3::X - reference_rotation * Vec3::X).length()
                + (rotation * Vec3::Y - reference_rotation * Vec3::Y).length())
                > 0.001;
            position_changed |= (position - reference_position).length_squared() > 0.000001;
            let time = frame.saturating_sub(1) as f32 * frame_time;
            let rotation = basis
                * if relative {
                    reference_rotation.inverse() * rotation
                } else {
                    rotation
                }
                * basis.inverse();
            rotations.push(RotKey {
                time: time.min(duration),
                rotation,
            });
            if joint.channels.iter().any(|c| matches!(c, Channel::Position(_))) {
                let position = if relative {
                    basis * (reference_rotation.inverse() * (position - reference_position))
                } else {
                    basis * position
                } * 0.0254;
                positions.push(PosKey {
                    time: time.min(duration),
                    position: position.clamp(Vec3::splat(-5.0), Vec3::splat(5.0)),
                });
            }
        }
        if !rotation_changed {
            rotations.clear();
        }
        if !position_changed {
            positions.clear();
        }
        // Recursive maximum-error reduction keeps endpoints and bounds the
        // interpolation error, without serializing every source frame.
        let rotations = reduce(rotations, |a, b, c| {
            let t = if c.time > a.time {
                (b.time - a.time) / (c.time - a.time)
            } else {
                1.0
            };
            let q = a.rotation.slerp(c.rotation, t);
            ((q * Vec3::X - b.rotation * Vec3::X).length() + (q * Vec3::Y - b.rotation * Vec3::Y).length()) / 0.01
        });
        let positions = reduce(positions, |a, b, c| {
            let t = if c.time > a.time {
                (b.time - a.time) / (c.time - a.time)
            } else {
                1.0
            };
            (a.position.lerp(c.position, t) - b.position).length() / (0.03 * 0.0254)
        });
        if !rotations.is_empty() || !positions.is_empty() {
            tracks.push(JointMotion {
                joint_name: name.clone(),
                priority: 2,
                rot_keys: rotations,
                pos_keys: positions,
            });
        }
    }
    if tracks.is_empty() {
        return Err(AssetError::invalid("BVH contains no moving recognized joints"));
    }
    Ok(Bvh {
        frames,
        frame_time,
        animation: Animation {
            base_priority: 2,
            duration,
            emote_name: String::new(),
            loop_in: 0.0,
            loop_out: duration,
            looping: false,
            ease_in: 0.3,
            ease_out: 0.3,
            hand_pose: 1,
            joints: tracks,
        },
    })
}

fn reduce<T>(keys: Vec<T>, error: impl Fn(&T, &T, &T) -> f32) -> Vec<T> {
    if keys.len() < 3 {
        return keys;
    }
    let mut keep = vec![false; keys.len()];
    keep[0] = true;
    keep[keys.len() - 1] = true;
    let mut stack = vec![(0, keys.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        let worst = (first + 1..last)
            .map(|i| (i, error(&keys[first], &keys[i], &keys[last])))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, value)) = worst
            && value > 1.0
        {
            keep[i] = true;
            stack.push((first, i));
            stack.push((i, last));
        }
    }
    keys.into_iter().zip(keep).filter_map(|(key, keep)| keep.then_some(key)).collect()
}

pub fn encode(animation: &Animation) -> Result<Vec<u8>, AssetError> {
    if !animation.duration.is_finite()
        || !(0.0..=60.0).contains(&animation.duration)
        || animation.joints.len() > 256
        || ![animation.loop_in, animation.loop_out, animation.ease_in, animation.ease_out]
            .iter()
            .all(|v| v.is_finite())
        || animation.emote_name.len() > 255
        || animation.emote_name.contains('\0')
    {
        return Err(AssetError::invalid("animation upload parameters"));
    }
    let mut data = Vec::new();
    data.extend(1u16.to_le_bytes());
    data.extend(0u16.to_le_bytes());
    data.extend(animation.base_priority.to_le_bytes());
    data.extend(animation.duration.to_le_bytes());
    data.extend(animation.emote_name.bytes());
    data.push(0);
    data.extend(animation.loop_in.to_le_bytes());
    data.extend(animation.loop_out.to_le_bytes());
    data.extend(i32::from(animation.looping).to_le_bytes());
    let factor = if !animation.looping && animation.ease_in + animation.ease_out > animation.duration && animation.duration > 0.0 {
        animation.duration / (animation.ease_in + animation.ease_out)
    } else {
        1.0
    };
    data.extend((animation.ease_in * factor).to_le_bytes());
    data.extend((animation.ease_out * factor).to_le_bytes());
    data.extend(animation.hand_pose.to_le_bytes());
    data.extend((animation.joints.len() as u32).to_le_bytes());
    let quantize = |value: f32, lower: f32, upper: f32| -> u16 {
        if upper == lower {
            0
        } else {
            (((value - lower) / (upper - lower)).clamp(0.0, 1.0) * 65535.0).floor() as u16
        }
    };
    for joint in &animation.joints {
        if joint.joint_name.len() > 255 || joint.joint_name.contains('\0') || joint.rot_keys.len() > 16384 || joint.pos_keys.len() > 16384 {
            return Err(AssetError::invalid("animation joint keys"));
        }
        data.extend(joint.joint_name.bytes());
        data.push(0);
        data.extend(joint.priority.to_le_bytes());
        data.extend((joint.rot_keys.len() as i32).to_le_bytes());
        for key in &joint.rot_keys {
            if !key.time.is_finite() || !key.rotation.is_finite() || key.rotation.length_squared() < 0.000001 {
                return Err(AssetError::invalid("animation rotation key"));
            }
            data.extend(quantize(key.time, 0.0, animation.duration).to_le_bytes());
            let q = key.rotation.normalize();
            let q = if q.w < 0.0 { -q } else { q };
            for value in [q.x, q.y, q.z] {
                data.extend(quantize(value, -1.0, 1.0).to_le_bytes());
            }
        }
        data.extend((joint.pos_keys.len() as i32).to_le_bytes());
        for key in &joint.pos_keys {
            if !key.time.is_finite() || !key.position.is_finite() {
                return Err(AssetError::invalid("animation position key"));
            }
            data.extend(quantize(key.time, 0.0, animation.duration).to_le_bytes());
            for value in key.position.to_array() {
                data.extend(quantize(value, -5.0, 5.0).to_le_bytes());
            }
        }
    }
    data.extend(0i32.to_le_bytes()); // No BVH constraints in the default anim.ini.
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn skeleton() -> Skeleton {
        Skeleton::parse(br#"<linden_skeleton><bone name="mPelvis" aliases="hip" pos="0 0 0" pivot="0 0 0" rot="0 0 0" scale="1 1 1"/></linden_skeleton>"#).expect("skeleton")
    }
    const BVH: &[u8] = b"HIERARCHY ROOT hip { OFFSET 0 0 0 CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation End Site { OFFSET 0 1 0 } } MOTION Frames: 4 Frame Time: 0.1 0 0 0 0 0 0 0 0 0 0 0 0 0 10 0 45 0 0 0 20 0 90 0 0";
    #[test]
    fn reference_frame_duration_coordinates_and_rotation_order_round_trip() {
        let bvh = parse_bvh(BVH, &skeleton()).expect("BVH");
        assert_eq!(bvh.frames, 4);
        assert_eq!(bvh.animation.duration, 0.2);
        let decoded = crate::parse_animation(&encode(&bvh.animation).expect("encode")).expect("decode");
        let joint = &decoded.joints[0];
        assert_eq!(joint.joint_name, "mPelvis");
        assert!((joint.pos_keys.last().expect("key").position.z - 0.508).abs() < 0.0002);
        assert!((joint.rot_keys.last().expect("key").rotation * Vec3::Z - Vec3::NEG_Y).length() < 0.001);
    }
    #[test]
    fn malformed_motion_and_unknown_root_fail_without_panics() {
        assert!(parse_bvh(&BVH[..BVH.len() - 8], &skeleton()).is_err());
        assert!(
            parse_bvh(
                &String::from_utf8_lossy(BVH).replace("ROOT hip", "ROOT foot").into_bytes(),
                &skeleton()
            )
            .is_err()
        );
        assert!(parse_bvh(&String::from_utf8_lossy(BVH).replace("0.1", "NaN").into_bytes(), &skeleton()).is_err());
    }
}
