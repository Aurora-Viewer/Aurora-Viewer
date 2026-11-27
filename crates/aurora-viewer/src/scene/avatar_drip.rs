//! Enhanced water's wet avatars: water dripping off them as they come out
//! of it. After the drips of PyKat Resident's Radiance Viewer
//! (rdwetdrip.cpp, originally LGPL 2.1; NOTICE.md): water gathers where the body hangs
//! lowest, most at the hands, then the elbows and the chin, a little at the
//! knees and the head; drops fall from there as fast as the place is wet
//! (avatar_wet.rs keeps how wet), a soaked hand pours a short stream, and
//! the drops leave with the limb's motion, so a raised hand flings them.
//! They fall as water particles (`WaterEvent` kind 11, env.wgsl) and stop
//! at the water, the ground or the feet; where they fall into the water,
//! it rings (kind 3, sent when they get there).

use super::anim::ANCHOR_BASE;
use super::avatar_wet::AvatarWet;
use aurora_render::WaterEvent;
use aurora_render::water_effects::WaterGround;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

/// The joints water drips from, and how much each gathers.
const SOURCES: [(&str, f32); 9] = [
    ("mHandMiddle3Left", 1.0),
    ("mHandMiddle3Right", 1.0),
    ("mElbowLeft", 0.45),
    ("mElbowRight", 0.45),
    ("mFaceChin", 0.4),
    ("mKneeLeft", 0.22),
    ("mKneeRight", 0.22),
    ("mHead", 0.15),
    ("mPelvis", 0.12),
];
/// Drops a second from a fully wet source of weight 1, and a soaked hand's
/// stream (Radiance's RadianceWetDripRate and RadianceWetDripPour).
const RATE: f32 = 10.0;
const POUR: f32 = 30.0;
/// How often a source sends its drops (s): one event each time.
const PERIOD: f32 = 0.15;
/// A drop's size (m).
const SIZE: f32 = 0.0035;
/// Drops fall at most this fast (m/s; env.wgsl holds them near it).
const FALL: f32 = 7.0;
/// Only avatars this near the camera drip (m).
const RANGE: f32 = 32.0;

#[derive(Clone, Copy)]
struct Source {
    at: Vec3,
    seen: f32,
    next: f32,
    owed: f32,
}

/// The drips of the wet avatars near the camera.
#[derive(Default)]
pub struct AvatarDrip {
    sources: HashMap<(Uuid, usize), Source>,
    /// Where drops will fall into the water, and when (the ring is sent
    /// then: the water's simulation takes an impulse while it is due).
    rings: Vec<(f32, Vec3)>,
    joints: Vec<(usize, f32)>,
}

/// How long a drop takes to fall `height` m (with the drag of env.wgsl).
pub fn fall_time(height: f32) -> f32 {
    let k = 9.81 / FALL;
    let h = height.max(0.0);
    (h * k / FALL).exp().acosh() / k
}

/// A number in 0..1 from `a` and `b`, the same each time.
fn hash(a: u64, b: u32) -> f32 {
    let mut x = a ^ (u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

impl AvatarDrip {
    /// One frame at `time` (s): the drops of the avatars in `avatars`
    /// (their palettes `stride` apart, slots from `slot`, joints named by
    /// `names`) as `wet` keeps them, near `eye`, into `events`; `ground`
    /// the terrain and the still water.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        wet: &AvatarWet,
        palettes: &[[[f32; 4]; 4]],
        stride: usize,
        slot: impl Fn(&Uuid) -> Option<u32>,
        names: &HashMap<String, usize>,
        avatars: &[Uuid],
        ground: &WaterGround,
        eye: Vec3,
        time: f32,
        events: &mut Vec<WaterEvent>,
    ) {
        if self.joints.is_empty() {
            self.joints = SOURCES
                .iter()
                .filter_map(|&(name, weight)| names.get(name).filter(|&&k| k < ANCHOR_BASE).map(|&k| (k, weight)))
                .collect();
        }
        // the rings due now
        self.rings.retain(|&(at, p)| {
            if at > time {
                return true;
            }
            if time - at < 0.25 {
                events.push(WaterEvent {
                    position: p.extend(at).to_array(),
                    motion: [0.0, 0.0, -FALL, 0.035],
                    params: [0.05, 3.0, p.z, 0.0],
                });
            }
            false
        });
        let feet = names.get("mFootLeft").copied();
        let mut seen = Vec::new();
        for id in avatars {
            let (line, block) = wet.entry(id);
            if block <= 0.0 {
                continue;
            }
            let Some(s) = slot(id) else {
                continue;
            };
            let base = s as usize * stride;
            let Some(palette) = palettes.get(base..base + ANCHOR_BASE + 1) else {
                continue;
            };
            let root = Mat4::from_cols_array_2d(&palette[ANCHOR_BASE]);
            if root.w_axis.truncate().distance(eye) > RANGE {
                continue;
            }
            let foot_z = feet
                .and_then(|k| palette.get(k))
                .map(|m| (root * Mat4::from_cols_array_2d(m)).w_axis.z);
            for &(k, weight) in &self.joints {
                let Some(m) = palette.get(k) else {
                    continue;
                };
                let world = root * Mat4::from_cols_array_2d(m);
                let at = world.w_axis.truncate();
                if !at.is_finite() {
                    continue;
                }
                seen.push((*id, k));
                let source = self.sources.entry((*id, k)).or_insert(Source {
                    at,
                    seen: time,
                    next: time + PERIOD * hash(id.as_u128() as u64, k as u32),
                    owed: 0.0,
                });
                let dt = (time - source.seen).clamp(0.0, 0.25);
                let velocity = if dt > 0.0 {
                    ((at - source.at) / dt).clamp_length_max(6.0)
                } else {
                    Vec3::ZERO
                };
                source.at = at;
                source.seen = time;
                // (under the water, or in its band: no drop)
                let water = line[0] + line[1] * at.x + line[2] * at.y;
                let wetness = wet.wetness(id, k, world, at - Vec3::Z * 0.03);
                if line[3] > 0.0 && at.z < water + 0.05 || wetness <= 0.02 {
                    source.owed = 0.0;
                    continue;
                }
                // drips while wet, pours from a soaked hand
                let pour = if weight >= 0.8 {
                    ((wetness - 0.8) / 0.2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                source.owed += (RATE * weight * wetness + POUR * pour) * dt;
                if time < source.next {
                    continue;
                }
                source.next = time + PERIOD;
                let drops = source.owed.floor().min(12.0);
                source.owed -= drops;
                if drops < 1.0 {
                    continue;
                }
                // they stop at the water, the ground or the feet
                let [bed, level] = ground.sample(at.truncate()).unwrap_or([f32::NEG_INFINITY, f32::NEG_INFINITY]);
                let still = if line[3] > 0.0 { water.max(level) } else { level };
                let stop = still.max(bed).max(foot_z.map_or(f32::NEG_INFINITY, |z| z - 0.05));
                if !stop.is_finite() || stop >= at.z {
                    continue;
                }
                events.push(WaterEvent {
                    position: at.extend(time).to_array(),
                    motion: velocity.extend(drops).to_array(),
                    params: [SIZE, 11.0, stop, PERIOD / drops],
                });
                // into the water: a small ring where they land
                if still >= bed && still >= stop - 1e-3 {
                    let t = fall_time(at.z - stop);
                    let ease = (1.0 - (-t * 1.5).exp()) / 1.5;
                    let p = Vec3::new(at.x + velocity.x * ease, at.y + velocity.y * ease, stop);
                    if self.rings.len() < 64 {
                        self.rings.push((time + t, p));
                    }
                }
            }
        }
        self.sources.retain(|key, _| seen.contains(key));
    }

    /// Stop all drips (the effect turned off, the world left).
    pub fn clear(&mut self) {
        self.sources.clear();
        self.rings.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_falls_slower_than_freely_but_not_much_over_a_metre() {
        let free = (2.0f32 / 9.81).sqrt();
        let t = fall_time(1.0);
        assert!(t > free && t < free * 1.2, "{t} vs {free}");
        // long falls end near the terminal speed
        let t10 = fall_time(10.0);
        assert!((10.0 / t10) > 5.0 && (10.0 / t10) < FALL, "{t10}");
        assert_eq!(fall_time(0.0), 0.0);
    }

    #[test]
    fn the_hash_spreads_over_the_unit() {
        let values: Vec<f32> = (0..200).map(|i| hash(42, i)).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.08, "{mean}");
    }
}
