//! BVH import, based on LLBVHLoader::loadBVHFile / makeTranslation /
//! serialize (Firestorm indra/llcharacter, originally LGPL 2.1), and the BVH
//! export that writes back what the import reads. Bounds, parsing and writing
//! are independent of the viewer and run on its background workers.

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

/// The most motion values a file may hold. LLBVHLoader sets no limit of its
/// own: this one only bounds memory, far above any 60 second animation.
const MAX_MOTION_VALUES: usize = 8 * 1024 * 1024;

fn number(token: &str) -> Result<f32, AssetError> {
    token
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| AssetError::invalid("BVH number"))
}

/// One frame per line, as LLBVHLoader::loadBVHFile reads them: lines past the
/// declared frames and values past a line's channels are ignored, and every
/// value of a line read must be a number. A file that does not hold its frames
/// on separate lines is read as one stream, which must then be exact.
fn motion_values(text: &str, frames: usize, channels: usize) -> Result<Vec<f32>, AssetError> {
    let lines: Vec<&str> = text
        .split(['\r', '\n'])
        .filter(|line| !line.trim().is_empty())
        .take(frames)
        .collect();
    let mut values = Vec::with_capacity(frames * channels);
    if lines.len() == frames && lines.iter().all(|line| line.split_whitespace().count() >= channels) {
        for line in lines {
            let parsed = line.split_whitespace().map(number).collect::<Result<Vec<_>, _>>()?;
            values.extend_from_slice(&parsed[..channels]);
        }
        return Ok(values);
    }
    let mut tokens = text.split_whitespace();
    for _ in 0..frames * channels {
        values.push(number(tokens.next().ok_or(AssetError::Truncated("BVH token"))?)?);
    }
    if tokens.next().is_some() {
        return Err(AssetError::invalid("extra BVH motion data"));
    }
    Ok(values)
}

struct Tokens<'a> {
    rest: &'a str,
    index: usize,
}
impl<'a> Tokens<'a> {
    fn next(&mut self) -> Result<&'a str, AssetError> {
        self.index += 1;
        let rest = self.rest.trim_start();
        if rest.is_empty() {
            return Err(AssetError::Truncated("BVH token"));
        }
        let (token, rest) = rest.split_at(rest.find(char::is_whitespace).unwrap_or(rest.len()));
        self.rest = rest;
        Ok(token)
    }
    fn require(&mut self, expected: &str) -> Result<(), AssetError> {
        if self.next()? == expected {
            Ok(())
        } else {
            Err(AssetError::invalid(format!("BVH: expected {expected} at token {}", self.index)))
        }
    }
    /// The value after a label, with or without a space between them: sscanf's
    /// "Frames: %d" takes both.
    fn labelled(&mut self, label: &str) -> Result<&'a str, AssetError> {
        let token = self.next()?;
        if token == label {
            return self.next();
        }
        token
            .strip_prefix(label)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AssetError::invalid(format!("BVH: expected {label} at token {}", self.index)))
    }
    /// LLBVHLoader::loadBVHFile never reads the values of an OFFSET line. The
    /// hierarchy keywords still stop a section that lost its closing keyword.
    fn offset(&mut self, end: &str) -> Result<(), AssetError> {
        self.require("OFFSET")?;
        loop {
            let token = self.next()?;
            if token == end {
                return Ok(());
            }
            if matches!(token, "{" | "}" | "JOINT" | "End" | "CHANNELS" | "MOTION") {
                return Err(AssetError::invalid(format!("BVH: expected {end} at token {}", self.index)));
            }
        }
    }
    /// sscanf's "%*s %79s" takes the first word of a joint's name: the rest of
    /// its line is ignored, up to a brace written on that same line.
    fn skip_name_tail(&mut self) {
        let line = &self.rest[..self.rest.find(['\r', '\n']).unwrap_or(self.rest.len())];
        self.rest = &self.rest[line.find('{').unwrap_or(line.len())..];
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
        self.skip_name_tail();
        self.require("{")?;
        self.offset("CHANNELS")?;
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
                    self.offset("}")?;
                }
                _ => return Err(AssetError::invalid("BVH hierarchy")),
            }
        }
        Ok(())
    }
}

pub fn parse_bvh(bytes: &[u8], skeleton: &Skeleton) -> Result<Bvh, AssetError> {
    parse_bvh_channels(bytes, skeleton, false, false)
}

/// Animation editing must not discard channels for bones the current rig
/// cannot represent. The general BVH importer keeps its legacy skip behavior.
pub fn parse_bvh_for_edit(bytes: &[u8], skeleton: &Skeleton) -> Result<Bvh, AssetError> {
    parse_bvh_channels(bytes, skeleton, false, true)
}

/// A pose needs every static channel, including an entirely neutral T-pose.
/// Ordinary motion imports retain their existing reference-frame reduction.
pub fn parse_bvh_pose(bytes: &[u8], skeleton: &Skeleton) -> Result<Bvh, AssetError> {
    parse_bvh_channels(bytes, skeleton, true, false)
}
fn parse_bvh_channels(bytes: &[u8], skeleton: &Skeleton, keep_static: bool, reject_unknown_joints: bool) -> Result<Bvh, AssetError> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(AssetError::TooLarge("BVH file"));
    }
    // LLBVHLoader reads plain bytes and looks for its keywords inside lines:
    // neither a byte order mark nor a name in another encoding stops it.
    let text = String::from_utf8_lossy(bytes);
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(&text)
        .replace('{', " { ")
        .replace('}', " } ");
    let mut tokens = Tokens { rest: &text, index: 0 };
    tokens.require("HIERARCHY")?;
    tokens.require("ROOT")?;
    let mut joints = Vec::new();
    let mut channels = 0;
    tokens.joint(&mut joints, &mut channels, 0)?;
    if skeleton.find(&joints[0].name).is_none_or(|i| skeleton.joints[i].name != "mPelvis") {
        return Err(AssetError::invalid("BVH root must be pelvis or an alias"));
    }
    tokens.require("MOTION")?;
    let frames = tokens
        .labelled("Frames:")?
        .parse::<usize>()
        .ok()
        .filter(|n| *n <= MAX_MOTION_VALUES)
        .ok_or_else(|| AssetError::invalid("BVH count"))?;
    if frames == 0 {
        return Err(AssetError::invalid("empty BVH"));
    }
    tokens.require("Frame")?;
    let frame_time = number(tokens.labelled("Time:")?)?;
    let duration = frames.saturating_sub(2).max(1) as f32 * frame_time;
    // A frame time of zero is a pose of no duration, which LLBVHLoader allows.
    if frame_time < 0.0 || duration > 60.0 {
        return Err(AssetError::invalid("BVH duration exceeds 60 seconds or frame time is invalid"));
    }
    if frames.checked_mul(channels).is_none_or(|n| n > MAX_MOTION_VALUES) {
        return Err(AssetError::TooLarge("BVH motion"));
    }
    let motion = motion_values(tokens.rest, frames, channels)?;
    // BVH Y-up inches -> SL Z-up meters, from makeTranslation's frame matrix.
    let basis = Quat::from_mat3(&Mat3::from_cols(Vec3::Y, Vec3::Z, Vec3::X));
    let mut tracks = Vec::new();
    for joint in joints {
        let index = match skeleton.find(&joint.name) {
            Some(index) => index,
            None if reject_unknown_joints => return Err(AssetError::invalid("BVH contains joints unknown to the avatar rig")),
            None => continue,
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
            ends_after: None,
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

/// Write an animation as a BVH file that `parse_bvh_for_edit` reads back to
/// the same motion, and that Blender and the other BVH tools open on the
/// avatar's rest skeleton. The inverse of the import above:
///
/// - the hierarchy is every bone of `skeleton` (no collision volume), its
///   OFFSETs the rest positions in inches, Y up;
/// - frame 0 is the rest pose, the reference frame the import measures the
///   pelvis from; the motion starts at frame 1, one frame per `1 / fps`,
///   retimed so that the last frame falls on the animation's duration;
/// - the pelvis carries position channels, and so does any bone the
///   animation moves; its positions are written over its rest offset,
///   as the other BVH exporters do, so the hips keep their height in
///   Blender;
/// - rotations are written in Zrotation Xrotation Yrotation order, the
///   one the Legacy Viewer's sample BVH files use.
///
/// What a BVH file cannot hold (priorities, loop points, ease, hand pose,
/// expression) is left out: the format has no place for it.
pub fn encode_bvh(animation: &Animation, skeleton: &Skeleton, fps: u32) -> Result<String, AssetError> {
    use std::fmt::Write as _;
    if !animation.duration.is_finite() || !(0.0..=60.0).contains(&animation.duration) || fps == 0 || fps > 240 {
        return Err(AssetError::invalid("BVH export parameters"));
    }
    let pelvis = skeleton
        .joints
        .iter()
        .position(|joint| joint.name == "mPelvis" && !joint.is_collision_volume)
        .ok_or_else(|| AssetError::invalid("skeleton has no pelvis"))?;
    let bones: Vec<usize> = (0..skeleton.joints.len())
        .filter(|&index| !skeleton.joints[index].is_collision_volume && is_under(skeleton, index, pelvis))
        .collect();
    let mut motions: Vec<Option<&JointMotion>> = vec![None; skeleton.joints.len()];
    for motion in &animation.joints {
        // a track the rig does not know cannot be placed in the hierarchy
        let index = skeleton
            .find(&motion.joint_name)
            .filter(|index| bones.contains(index))
            .ok_or_else(|| AssetError::invalid("animation joint unknown to the avatar skeleton"))?;
        motions[index] = Some(motion);
    }
    let spans = ((animation.duration * fps as f32).round() as usize).max(1);
    let frame_time = if animation.duration > 0.0 {
        animation.duration / spans as f32
    } else {
        1.0 / fps as f32
    };
    // SL Z-up metres -> BVH Y-up inches, the inverse of the import's basis
    let basis = Quat::from_mat3(&Mat3::from_cols(Vec3::Y, Vec3::Z, Vec3::X));
    let to_bvh = basis.inverse();
    let inches = |metres: Vec3| to_bvh * metres / 0.0254;
    let moves = |index: usize| index == pelvis || motions[index].is_some_and(|motion| !motion.pos_keys.is_empty());
    let mut text = String::from("HIERARCHY\n");
    write_bvh_joint(&mut text, skeleton, &bones, pelvis, 0, &inches, &moves);
    let _ = write!(text, "MOTION\nFrames: {}\nFrame Time: {frame_time:.6}\n", spans + 2);
    let mut previous: Vec<Vec3> = vec![Vec3::ZERO; skeleton.joints.len()];
    for frame in 0..spans + 2 {
        // frame 0 is the rest pose; frame n is the animation at n - 1
        let time = frame.checked_sub(1).map(|step| (step as f32 * frame_time).min(animation.duration));
        let mut values = Vec::new();
        for &index in &bones {
            let joint = &skeleton.joints[index];
            let motion = time.and(motions[index]);
            if moves(index) {
                let rest = inches(joint.pos);
                let position = motion.and_then(|motion| motion.position_at(time.unwrap_or(0.0)));
                let position = match position {
                    Some(offset) if index == pelvis => rest + inches(offset),
                    Some(position) => inches(position),
                    None => rest,
                };
                values.extend([position.x, position.y, position.z]);
            }
            let rotation = motion
                .and_then(|motion| motion.rotation_at(time.unwrap_or(0.0)))
                .unwrap_or(Quat::IDENTITY);
            let (y, x, z) = (to_bvh * rotation * basis).normalize().to_euler(glam::EulerRot::YXZ);
            let angles = Vec3::new(z, x, y).map(f32::to_degrees);
            let angles = if frame > 1 { euler_filter(angles, previous[index]) } else { angles };
            previous[index] = angles;
            values.extend([angles.x, angles.y, angles.z]);
        }
        let line: Vec<String> = values.iter().map(|value| format!("{value:.4}")).collect();
        text.push_str(&line.join(" "));
        text.push('\n');
    }
    Ok(text)
}

/// The angles (Z, X, Y, in degrees, applied Y X Z) of the same rotation
/// closest to the frame before: each angle within half a turn of the one
/// before, and the other set of angles of that rotation taken when it is
/// closer. A tool that interpolates between frames then never spins a bone
/// the long way round (Blender's Euler filter does the same).
fn euler_filter(angles: Vec3, previous: Vec3) -> Vec3 {
    let near = |angles: Vec3| angles + ((previous - angles) / 360.0).round() * 360.0;
    let direct = near(angles);
    let flipped = near(Vec3::new(angles.x + 180.0, 180.0 - angles.y, angles.z + 180.0));
    let distance = |a: Vec3| (a - previous).abs().element_sum();
    if distance(flipped) < distance(direct) { flipped } else { direct }
}

fn is_under(skeleton: &Skeleton, mut index: usize, ancestor: usize) -> bool {
    loop {
        if index == ancestor {
            return true;
        }
        match skeleton.joints[index].parent {
            Some(parent) => index = parent,
            None => return false,
        }
    }
}

fn write_bvh_joint(
    text: &mut String,
    skeleton: &Skeleton,
    bones: &[usize],
    index: usize,
    depth: usize,
    inches: &impl Fn(Vec3) -> Vec3,
    moves: &impl Fn(usize) -> bool,
) {
    use std::fmt::Write as _;
    let joint = &skeleton.joints[index];
    let indent = "\t".repeat(depth);
    let offset = inches(joint.pos);
    let _ = writeln!(text, "{indent}{} {}", if depth == 0 { "ROOT" } else { "JOINT" }, joint.name);
    let _ = writeln!(text, "{indent}{{");
    let _ = writeln!(text, "{indent}\tOFFSET {:.4} {:.4} {:.4}", offset.x, offset.y, offset.z);
    if moves(index) {
        let _ = writeln!(
            text,
            "{indent}\tCHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation"
        );
    } else {
        let _ = writeln!(text, "{indent}\tCHANNELS 3 Zrotation Xrotation Yrotation");
    }
    let children: Vec<usize> = bones
        .iter()
        .copied()
        .filter(|&child| skeleton.joints[child].parent == Some(index))
        .collect();
    if children.is_empty() {
        let end = inches(joint.end);
        let _ = writeln!(text, "{indent}\tEnd Site\n{indent}\t{{");
        let _ = writeln!(text, "{indent}\t\tOFFSET {:.4} {:.4} {:.4}", end.x, end.y, end.z);
        let _ = writeln!(text, "{indent}\t}}");
    }
    for child in children {
        write_bvh_joint(text, skeleton, bones, child, depth + 1, inches, moves);
    }
    let _ = writeln!(text, "{indent}}}");
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

    const LINES: &str = "HIERARCHY\r\nROOT hip\r\n{\r\n\tOFFSET 0 0 0\r\n\tCHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation\r\n\tEnd Site\r\n\t{\r\n\t\tOFFSET 0 1 0\r\n\t}\r\n}\r\nMOTION\r\nFrames: 4\r\nFrame Time: 0.1\r\n0 0 0 0 0 0\r\n0 0 0 0 0 0\r\n10 0 45 0 0 0\r\n20 0 90 0 0 0\r\n";

    /// An extra joint the skeleton does not know, its name in Latin-1.
    fn foreign_joint() -> Vec<u8> {
        let mut bytes = LINES
            .replace(
                "\tEnd Site",
                "\tJOINT caf#\r\n\t{\r\n\t\tOFFSET 0 0 0\r\n\t\tCHANNELS 3 Zrotation Xrotation Yrotation\r\n\t}\r\n\tEnd Site",
            )
            .replace("0 0 0 0 0 0\r\n", "0 0 0 0 0 0 0 0 0\r\n")
            .replace("10 0 45 0 0 0\r\n", "10 0 45 0 0 0 0 0 0\r\n")
            .replace("20 0 90 0 0 0\r\n", "20 0 90 0 0 0 0 0 0\r\n")
            .into_bytes();
        let marker = bytes.iter().position(|b| *b == b'#').expect("marker");
        bytes[marker] = 0xE9;
        bytes
    }

    #[test]
    fn files_the_legacy_loader_reads_give_the_same_animation() {
        let expected = parse_bvh(LINES.as_bytes(), &skeleton()).expect("BVH").animation;
        let cases: [(&str, Vec<u8>); 8] = [
            ("byte order mark", [b"\xEF\xBB\xBF".as_slice(), LINES.as_bytes()].concat()),
            ("name in another encoding", foreign_joint()),
            ("no space after Frames:", LINES.replace("Frames: 4", "Frames:4").into_bytes()),
            (
                "no space after Frame Time:",
                LINES.replace("Frame Time: 0.1", "Frame Time:0.1").into_bytes(),
            ),
            (
                "lines past the frames",
                format!("{LINES}1 2 3 4 5 6\r\nnot a frame\r\n").into_bytes(),
            ),
            (
                "values past the channels",
                LINES.replace("10 0 45 0 0 0", "10 0 45 0 0 0 7 7").into_bytes(),
            ),
            (
                "OFFSET that is not a number",
                LINES.replace("OFFSET 0 0 0", "OFFSET 0 -1.#IND 0").into_bytes(),
            ),
            ("joint name with a space", LINES.replace("ROOT hip", "ROOT hip copy").into_bytes()),
        ];
        for (name, bytes) in cases {
            let animation = parse_bvh(&bytes, &skeleton()).unwrap_or_else(|e| panic!("{name}: {e}")).animation;
            assert_eq!(animation, expected, "{name}");
        }
    }

    #[test]
    fn editor_bvh_import_rejects_unmapped_joints_without_changing_legacy_import() {
        let bytes = foreign_joint();
        let expected = parse_bvh(LINES.as_bytes(), &skeleton()).expect("mapped animation").animation;
        assert_eq!(parse_bvh(&bytes, &skeleton()).expect("legacy skip behavior").animation, expected);
        assert!(parse_bvh_for_edit(&bytes, &skeleton()).is_err());
    }

    #[test]
    fn a_frame_time_of_zero_is_a_pose_of_no_duration() {
        let bytes = LINES.replace("Frame Time: 0.1", "Frame Time: 0").into_bytes();
        let animation = parse_bvh(&bytes, &skeleton()).expect("BVH").animation;
        assert_eq!(animation.duration, 0.0);
        let decoded = crate::parse_animation(&encode(&animation).expect("encode")).expect("decode");
        assert_eq!(decoded.joints[0].pos_keys.len(), 1);
    }

    #[test]
    fn broken_files_the_legacy_loader_refuses_stay_refused() {
        let cases = [
            LINES.replace("Frame Time: 0.1", "Frame Time: -0.1"),
            LINES.replace("20 0 90 0 0 0\r\n", "20 0 90 0 0\r\n"),
            LINES.replace("20 0 90 0 0 0\r\n", "20 0 90 0 0 x\r\n"),
            LINES.replace("10 0 45 0 0 0", "10 0 45 0 0 0 x"),
            LINES.replace("\tCHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation\r\n", ""),
        ];
        for case in cases {
            assert!(parse_bvh(case.as_bytes(), &skeleton()).is_err(), "{case}");
        }
    }
    fn avatar_skeleton() -> Skeleton {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../aurora-viewer/assets/character/avatar_skeleton.xml");
        Skeleton::parse(&std::fs::read(path).expect("avatar skeleton")).expect("skeleton")
    }

    fn wave() -> Animation {
        let turn = |degrees: f32| Quat::from_rotation_z(degrees.to_radians()) * Quat::from_rotation_x((degrees / 3.0).to_radians());
        Animation {
            base_priority: 3,
            duration: 1.0,
            emote_name: String::new(),
            loop_in: 0.0,
            loop_out: 1.0,
            looping: true,
            ends_after: None,
            ease_in: 0.3,
            ease_out: 0.3,
            hand_pose: 0,
            joints: vec![
                JointMotion {
                    joint_name: "mPelvis".into(),
                    priority: 3,
                    rot_keys: vec![
                        RotKey {
                            time: 0.0,
                            rotation: Quat::IDENTITY,
                        },
                        RotKey {
                            time: 1.0,
                            rotation: Quat::from_rotation_z(0.5),
                        },
                    ],
                    pos_keys: vec![
                        PosKey {
                            time: 0.0,
                            position: Vec3::ZERO,
                        },
                        PosKey {
                            time: 0.5,
                            position: Vec3::new(0.1, -0.05, 0.2),
                        },
                        PosKey {
                            time: 1.0,
                            position: Vec3::ZERO,
                        },
                    ],
                },
                JointMotion {
                    joint_name: "mShoulderRight".into(),
                    priority: 3,
                    rot_keys: vec![
                        RotKey {
                            time: 0.0,
                            rotation: turn(0.0),
                        },
                        RotKey {
                            time: 0.5,
                            rotation: turn(80.0),
                        },
                        RotKey {
                            time: 1.0,
                            rotation: turn(-40.0),
                        },
                    ],
                    pos_keys: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn an_exported_bvh_imports_back_to_the_same_motion() {
        let skeleton = avatar_skeleton();
        let source = wave();
        let text = encode_bvh(&source, &skeleton, 30).expect("export");
        let back = parse_bvh_for_edit(text.as_bytes(), &skeleton).expect("import").animation;
        assert!((back.duration - source.duration).abs() < 1e-4);
        for joint in &source.joints {
            let read = back.joints.iter().find(|j| j.joint_name == joint.joint_name).expect("track kept");
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                let (a, b) = (joint.rotation_at(t).expect("source"), read.rotation_at(t).expect("read"));
                assert!(a.angle_between(b) < 0.02, "{} rotation at {t}: {a} vs {b}", joint.joint_name);
                if let Some(a) = joint.position_at(t) {
                    let b = read.position_at(t).expect("position read");
                    assert!((a - b).length() < 0.002, "{} position at {t}: {a} vs {b}", joint.joint_name);
                }
            }
        }
        // bones the animation leaves alone stay out of the imported tracks
        assert_eq!(back.joints.len(), source.joints.len());
    }

    #[test]
    fn an_exported_bvh_has_the_rest_skeleton_and_a_rest_reference_frame() {
        let skeleton = avatar_skeleton();
        let text = encode_bvh(&wave(), &skeleton, 30).expect("export");
        assert!(text.starts_with("HIERARCHY\nROOT mPelvis\n"));
        let joints = text.lines().filter(|line| line.trim_start().starts_with("JOINT ")).count();
        assert_eq!(joints + 1, skeleton.bone_count());
        assert!(
            !skeleton
                .joints
                .iter()
                .filter(|j| j.is_collision_volume)
                .any(|j| text.contains(&format!("JOINT {}\n", j.name)))
        );
        assert!(text.contains("Frames: 32\nFrame Time: 0.033333\n"));
        let motion = text.split("Frame Time: 0.033333\n").nth(1).expect("motion");
        let rest: Vec<f32> = motion
            .lines()
            .next()
            .expect("frame 0")
            .split(' ')
            .map(|v| v.parse().expect("number"))
            .collect();
        // the pelvis at its rest offset, every rotation at zero
        let pelvis = skeleton.joints[skeleton.find("mPelvis").expect("pelvis")].pos;
        assert!((rest[1] - pelvis.z / 0.0254).abs() < 0.01);
        assert!(rest[3..].iter().all(|v| v.abs() < 1e-3));
        assert_eq!(motion.lines().count(), 32);
    }

    #[test]
    fn a_track_the_skeleton_does_not_know_is_refused() {
        let mut animation = wave();
        animation.joints[1].joint_name = "mTail9".into();
        assert!(encode_bvh(&animation, &avatar_skeleton(), 30).is_err());
        assert!(encode_bvh(&wave(), &avatar_skeleton(), 0).is_err());
    }

    #[test]
    fn exported_angles_do_not_jump_a_full_turn_between_frames() {
        let mut animation = wave();
        animation.joints[1].rot_keys = (0..=10)
            .map(|step| RotKey {
                time: step as f32 / 10.0,
                rotation: Quat::from_rotation_y((step as f32 * 40.0).to_radians()),
            })
            .collect();
        let text = encode_bvh(&animation, &avatar_skeleton(), 30).expect("export");
        let motion: Vec<Vec<f32>> = text
            .split("Frame Time: ")
            .nth(1)
            .expect("motion")
            .lines()
            .skip(1)
            .map(|line| line.split(' ').map(|v| v.parse().expect("number")).collect())
            .collect();
        for pair in motion.windows(2) {
            for (a, b) in pair[0].iter().zip(&pair[1]) {
                assert!((a - b).abs() < 180.0, "{a} -> {b}");
            }
        }
    }
}
