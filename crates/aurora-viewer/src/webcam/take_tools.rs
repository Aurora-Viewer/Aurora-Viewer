//! Offline repairs and edit suggestions for a recorded take.

use crate::{
    scene::anim::{Rig, WebcamFeet},
    webcam::FootContact,
};
use aurora_assets::anim::{Animation, JointMotion, RotKey};
use glam::{Quat, Vec3};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

pub(super) type RotationFrame = Vec<Option<Quat>>;

#[derive(Clone)]
pub(super) struct AudioPass {
    pub(super) source: AudioSource,
    pub(super) from: f32,
    pub(super) end: f32,
    pub(super) offset: f32,
    pub(super) rate: f32,
}

#[derive(Clone)]
pub(super) struct AudioSource {
    path: PathBuf,
    _retained: Option<Arc<RetainedMedia>>,
}

struct RetainedMedia(PathBuf);

impl Drop for RetainedMedia {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl AudioSource {
    pub(super) fn local(path: PathBuf) -> Self {
        Self { path, _retained: None }
    }

    pub(super) fn downloaded(path: &Path) -> Self {
        // Link the already downloaded file in its own volume. The take can
        // export after the companion's temporary cache has been discarded.
        let target = std::env::temp_dir().join(format!(
            "aurora-webcam-take-{}.{}",
            uuid::Uuid::new_v4(),
            path.extension().and_then(|ext| ext.to_str()).unwrap_or("mp4")
        ));
        if std::fs::hard_link(path, &target).is_ok() {
            Self {
                path: target.clone(),
                _retained: Some(Arc::new(RetainedMedia(target))),
            }
        } else {
            Self::local(path.to_owned())
        }
    }
}

/// Repair only short, corroborated gaps and isolated single-frame jumps.
/// An optional playhead time explicitly replaces its closest sample using
/// neighbors on both sides of a wider window.
pub(super) fn repair(timed: &[(f32, RotationFrame)], playhead: Option<f32>) -> Vec<RotationFrame> {
    repair_selected(timed, playhead.map(|center| [center - 0.065, center + 0.065]), None, 0.35)
}

pub(super) fn repair_selected(
    timed: &[(f32, RotationFrame)],
    range: Option<[f32; 2]>,
    joints: Option<&[usize]>,
    max_gap: f32,
) -> Vec<RotationFrame> {
    let size = timed.iter().map(|(_, pose)| pose.len()).max().unwrap_or(0);
    let mut out: Vec<_> = timed
        .iter()
        .map(|(_, pose)| {
            let mut frame = pose.clone();
            frame.resize(size, None);
            frame
        })
        .collect();
    for joint in 0..size {
        if joints.is_some_and(|joints| !joints.contains(&joint)) {
            continue;
        }
        let mut known: Vec<_> = timed
            .iter()
            .enumerate()
            .filter_map(|(i, (t, frame))| frame.get(joint).copied().flatten().filter(|q| q.is_finite()).map(|q| (i, *t, q)))
            .collect();
        if let Some([start, end]) = range {
            let before = known.iter().rfind(|(_, t, _)| *t <= start);
            let after = known.iter().find(|(_, t, _)| *t >= end);
            if let Some((a, b)) = before.zip(after).filter(|(a, b)| b.1 - a.1 <= max_gap) {
                for (i, (time, _)) in timed.iter().enumerate().take(b.0).skip(a.0 + 1) {
                    out[i][joint] = Some(between(a, b, *time));
                }
            }
            continue;
        }
        let mut spikes = Vec::new();
        for w in known.windows(3) {
            let [a, p, b] = [w[0], w[1], w[2]];
            if b.1 - a.1 < 0.15 && angle(a.2, b.2) < 0.25 && angle(p.2, between(&a, &b, p.1)) > 0.42 {
                spikes.push(p.0);
            }
        }
        known.retain(|point| spikes.binary_search(&point.0).is_err());
        for w in known.windows(2) {
            let [a, b] = [w[0], w[1]];
            if b.0 > a.0 + 1 && b.1 - a.1 <= 0.34 && angle(a.2, b.2) < 1.25 {
                for (i, (time, _)) in timed.iter().enumerate().take(b.0).skip(a.0 + 1) {
                    out[i][joint] = Some(between(&a, &b, *time));
                }
            }
        }
    }
    out
}

fn angle(a: Quat, b: Quat) -> f32 {
    2.0 * a.dot(b).abs().min(1.0).acos()
}

fn between(a: &(usize, f32, Quat), b: &(usize, f32, Quat), time: f32) -> Quat {
    a.2.slerp(b.2, ((time - a.1) / (b.1 - a.1)).clamp(0.0, 1.0))
}

pub(super) fn dropout_marks(timed: &[(f32, RotationFrame)], names: &[String]) -> Vec<f32> {
    let watched: Vec<_> = names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let main = ["mHead", "mChest", "mWrist", "mElbow", "mKnee", "mAnkle"]
                .iter()
                .any(|prefix| name.starts_with(prefix));
            (main && timed.iter().filter(|(_, pose)| pose.get(i).is_some_and(Option::is_some)).count() * 4 >= timed.len()).then_some(i)
        })
        .collect();
    timed
        .iter()
        .filter_map(|(time, pose)| watched.iter().any(|&i| pose.get(i).is_none_or(Option::is_none)).then_some(*time))
        .collect()
}

fn leg(name: &str) -> bool {
    ["mHip", "mKnee", "mAnkle", "mFoot", "mToe"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn foot_phase(feet: &[(f32, [bool; 2])], time: f32) -> [bool; 2] {
    let index = feet.partition_point(|p| p.0 < time).min(feet.len().saturating_sub(1));
    feet.get(index).map_or([false; 2], |point| point.1)
}

pub(super) fn find_loop(clip: &Animation, [start, end]: [f32; 2], feet: &[(f32, [bool; 2])]) -> Option<[f32; 2]> {
    let span = end - start;
    if span < 0.7 || clip.joints.is_empty() {
        return None;
    }
    let step = (span / 180.0).max(0.10);
    let count = (span / step).ceil() as usize;
    let times: Vec<_> = (0..=count).map(|i| start + span * i as f32 / count as f32).collect();
    let weights: Vec<_> = clip
        .joints
        .iter()
        .map(|joint| {
            if leg(&joint.joint_name) {
                3.0
            } else if joint.joint_name.starts_with("mFace") || joint.joint_name.starts_with("mHand") {
                0.15
            } else {
                1.0
            }
        })
        .collect();
    let total: f32 = weights.iter().sum();
    let poses: Vec<_> = times
        .iter()
        .map(|&t| {
            clip.joints
                .iter()
                .map(|joint| {
                    let pose = joint.rotation_at(t).unwrap_or(Quat::IDENTITY);
                    let before = joint.rotation_at((t - 0.065).max(0.0)).unwrap_or(pose);
                    let after = joint.rotation_at((t + 0.065).min(clip.duration)).unwrap_or(pose);
                    (pose, before.inverse() * after)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let phase: Vec<_> = times.iter().map(|&t| foot_phase(feet, t)).collect();
    let mut best = None;
    let min_length = (span * 0.32).clamp(0.65, 3.0);
    for (i, &a) in times.iter().enumerate() {
        for (j, &b) in times.iter().enumerate().skip(i + 1) {
            if b - a < min_length || phase[i] != phase[j] {
                continue;
            }
            let distance: f32 = poses[i]
                .iter()
                .zip(&poses[j])
                .zip(&weights)
                .map(|((&(a, va), &(b, vb)), w)| w * ((1.0 - a.dot(b).abs().min(1.0)) + 0.65 * (1.0 - va.dot(vb).abs().min(1.0))))
                .sum::<f32>()
                / total;
            // Ignore poor seams; among comparable candidates keep more material.
            let cost = distance - 0.0015 * (b - a) / span;
            if distance < 0.021 && best.is_none_or(|(score, _)| cost < score) {
                best = Some((cost, [a, b]));
            }
        }
    }
    best.map(|(_, range)| range)
}

pub(super) fn blend_seam(joint: &JointMotion, time: f32, [start, end]: [f32; 2]) -> Option<Quat> {
    let q = joint.rotation_at(time)?;
    let width = (0.20 * (end - start)).min(0.18);
    if width <= 0.0 || time <= end - width {
        return Some(q);
    }
    let first = joint.rotation_at(start)?;
    let u = ((time - end + width) / width).clamp(0.0, 1.0);
    Some(q.slerp(first, u * u * (3.0 - 2.0 * u)))
}

pub(super) fn seam_keys(joint: &JointMotion, range: [f32; 2]) -> Vec<RotKey> {
    let [start, end] = range;
    let width = (0.20 * (end - start)).min(0.18);
    let bridge = end - width;
    let mut keys = Vec::new();
    if let Some(rotation) = joint.rotation_at(start) {
        keys.push(RotKey { time: 0.0, rotation });
    }
    keys.extend(
        joint
            .rot_keys
            .iter()
            .filter(|key| key.time > start && key.time < bridge)
            .map(|key| RotKey {
                time: key.time - start,
                rotation: key.rotation,
            }),
    );
    let frames = (width * 30.0).ceil().max(1.0) as usize;
    for i in 0..=frames {
        let time = if i == frames {
            end
        } else {
            bridge + width * i as f32 / frames as f32
        };
        if let Some(rotation) = blend_seam(joint, time, range) {
            keys.push(RotKey {
                time: time - start,
                rotation,
            });
        }
    }
    keys
}

/// Bake the same bridge used by preview and export. Retain the start ankle
/// position during the blend when this side is planted on both sides.
pub(super) fn bake_loop(clip: &Animation, range: [f32; 2], rig: &Rig, feet: &[(f32, [bool; 2])]) -> Animation {
    let [start, end] = range;
    let width = (0.20 * (end - start)).min(0.18);
    let tail = end - width;
    let first_phase = foot_phase(feet, start);
    let tail_phases = [foot_phase(feet, tail), foot_phase(feet, (tail + end) * 0.5), foot_phase(feet, end)];
    let support = [0, 1].map(|i| first_phase[i] && tail_phases.iter().all(|phase| phase[i]));
    let mut baked = clip.clone();
    for (track, src) in baked.joints.iter_mut().zip(&clip.joints) {
        track.rot_keys = seam_keys(src, range)
            .into_iter()
            .map(|key| RotKey {
                time: key.time + start,
                rotation: key.rotation,
            })
            .collect();
    }
    if !support.iter().any(|&on| on) {
        return baked;
    }
    let joints: Vec<_> = clip.joints.iter().map(|j| rig.names.get(&j.joint_name).copied()).collect();
    let local_at = |t: f32| {
        let mut rotations = rig.local_rot.clone();
        for (track, bone) in clip.joints.iter().zip(&joints) {
            if let Some((bone, q)) = bone.zip(blend_seam(track, t, range)) {
                rotations[bone] = q;
            }
        }
        rotations
    };
    let mut locks = WebcamFeet::default();
    let anchor = |weight| support.map(|on| on.then_some(FootContact { id: 1, weight }));
    let mut initial = local_at(start);
    let world = rig.world_with(&initial, Vec3::ZERO, &rig.local_pos, &rig.scale);
    locks.apply(rig, &mut initial, &world, anchor(0.0));
    let steps = (width * 30.0).ceil().max(1.0) as usize;
    let mut tail_rotations = Vec::with_capacity(steps.saturating_sub(1));
    for i in 1..steps {
        let time = tail + width * i as f32 / steps as f32;
        let mut local = local_at(time);
        let world = rig.world_with(&local, Vec3::ZERO, &rig.local_pos, &rig.scale);
        locks.apply(rig, &mut local, &world, anchor((i as f32 / steps as f32 * 2.0).min(1.0)));
        tail_rotations.push(local);
    }
    for (track, bone) in baked.joints.iter_mut().zip(joints) {
        if let Some(bone) = bone
            && leg(&track.joint_name)
        {
            let end = track.rot_keys.len() - 1;
            let start = end - tail_rotations.len();
            for (key, pose) in track.rot_keys[start..end].iter_mut().zip(&tail_rotations) {
                key.rotation = pose[bone];
            }
        }
    }
    baked
}

pub(super) fn segments([start, end]: [f32; 2], beats: &[f32], count: usize) -> Vec<[f32; 2]> {
    let cuts = beats.iter().copied().filter(|beat| *beat > start + 0.18 && *beat < end - 0.18);
    let mut boundaries = vec![start];
    boundaries.extend(
        cuts.skip(count.saturating_sub(1))
            .step_by(count.max(1))
            .filter(|cut| cut.is_finite()),
    );
    boundaries.push(end);
    boundaries.windows(2).map(|w| [w[0], w[1]]).collect()
}

pub(super) fn beat_grid(bpm: u32, anchor: f32, duration: f32) -> Vec<f32> {
    let step = 60.0 / bpm.clamp(40, 240) as f32;
    let start = anchor.rem_euclid(step);
    (0..).map(|i| start + i as f32 * step).take_while(|t| *t <= duration).collect()
}

/// Transient accents in 10 ms bins. A refractory period prevents a single
/// percussion hit from turning into several cut markers.
fn accents(pcm: &[u8]) -> Vec<f32> {
    let mut previous = 0.0_f32;
    let energy: Vec<_> = pcm
        .chunks_exact(320)
        .map(|block| {
            let mut sum = 0.0;
            for sample in block.chunks_exact(4) {
                let x = f32::from_le_bytes(sample.try_into().expect("fixed-size PCM"));
                let low = previous + (x - previous) * 0.12;
                sum += 0.72 * low * low + 0.28 * x * x;
                previous = low;
            }
            (sum / 80.0).sqrt()
        })
        .collect();
    let growth: Vec<_> = energy.windows(2).map(|w| (w[1] - 0.82 * w[0]).max(0.0)).collect();
    let peak = growth.iter().copied().fold(0.0_f32, f32::max);
    let mut markers: Vec<(usize, f32)> = Vec::new();
    for (i, &value) in growth.iter().enumerate() {
        let nearby = &growth[i.saturating_sub(20)..(i + 21).min(growth.len())];
        let avg = nearby.iter().sum::<f32>() / nearby.len() as f32;
        if value < (avg * 2.4).max(peak * 0.07).max(0.004) || growth.get(i + 1).is_some_and(|v| *v > value) {
            continue;
        }
        if let Some(last) = markers.last_mut().filter(|last| i - last.0 < 27) {
            if value > last.1 {
                *last = (i, value);
            }
        } else {
            markers.push((i, value));
        }
    }
    markers.into_iter().map(|(i, _)| (i + 1) as f32 * 0.01).collect()
}

pub(super) fn audio_beats(passes: &[AudioPass], duration: f32) -> Vec<f32> {
    let mut result: Vec<f32> = Vec::new();
    for pass in passes {
        let end = pass.end.min(duration);
        let length = end - pass.from;
        if length < 0.22 {
            continue;
        }
        let mut command = audio_command(pass, pass.offset, length, 8000, "f32le");
        if let Ok(out) = command.output()
            && out.status.success()
        {
            for beat in accents(&out.stdout) {
                let time = pass.from + beat;
                if time <= end && result.last().is_none_or(|last| time - last > 0.19) {
                    result.push(time);
                }
            }
        }
    }
    result
}

pub(super) fn audio_command(pass: &AudioPass, offset: f32, length: f32, rate: u32, format: &str) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe,crypto,data",
            "-ss",
            &format!("{offset:.6}"),
            "-i",
        ])
        .arg(&pass.source.path)
        .args([
            "-map",
            "0:a:0",
            "-af",
            &format!("aresample=44100,asetrate={}", (44_100.0 * pass.rate).round() as u32),
            "-t",
            &format!("{length:.6}"),
            "-vn",
            "-f",
            format,
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_repair_only_changes_the_selected_rotation_between_anchors() {
        let spike = Some(Quat::from_rotation_x(1.8));
        let stable = Some(Quat::IDENTITY);
        let timed = [(0.0, vec![stable, stable]), (0.1, vec![spike, spike]), (0.2, vec![stable, stable])];
        let repaired = repair_selected(&timed, Some([0.08, 0.12]), Some(&[0]), 0.35);
        assert_eq!(repaired, [vec![stable, stable], vec![stable, spike], vec![stable, stable]]);
    }

    #[test]
    fn short_gaps_and_spikes_are_repaired_while_long_missing_runs_stay_missing() {
        let q = |angle| Some(Quat::from_rotation_x(angle));
        let source = vec![
            (0.00, vec![q(0.0), q(0.0)]),
            (0.05, vec![None, q(1.2)]),
            (0.10, vec![q(0.2), q(0.0)]),
            (0.25, vec![None, None]),
            (0.90, vec![q(0.3), q(0.0)]),
        ];
        let repaired = repair(&source, None);
        assert!(angle(repaired[1][0].unwrap(), q(0.1).unwrap()) < 0.002);
        assert!(angle(repaired[1][1].unwrap(), Quat::IDENTITY) < 0.002);
        assert_eq!(repaired[3], vec![None, None]);
    }

    #[test]
    fn seam_tail_matches_its_start_and_keeps_earlier_keys() {
        let joint = JointMotion {
            joint_name: "mAnkleLeft".into(),
            priority: 4,
            pos_keys: Vec::new(),
            rot_keys: [(0.0, 0.0), (0.403, 0.3), (0.9, 0.4), (1.0, 0.1)]
                .map(|(time, x)| RotKey {
                    time,
                    rotation: Quat::from_rotation_x(x),
                })
                .into(),
        };
        let keys = seam_keys(&joint, [0.0, 1.0]);
        assert!(keys.iter().any(|k| k.time == 0.403));
        assert!((keys.last().unwrap().time - 1.0).abs() < 1e-6);
        assert!(angle(keys[0].rotation, keys.last().unwrap().rotation) < 0.002);
    }

    #[test]
    fn click_track_and_manual_grid_give_predictable_cuts() {
        let mut pcm = Vec::new();
        for i in 0..(8000 * 4) {
            let point = i % 4000;
            let x: f32 = if (1600..1760).contains(&point) { 0.8 } else { 0.0 };
            pcm.extend(x.to_le_bytes());
        }
        let beats = accents(&pcm);
        assert_eq!(beats.len(), 8);
        assert!((beats[0] - 0.20).abs() < 0.025);
        let grid = beat_grid(120, 0.0, 4.0);
        assert_eq!(segments([0.0, 4.0], &grid, 4), vec![[0.0, 2.0], [2.0, 4.0]]);
    }
}
