//! Enhanced water's wet look on avatars: what went into the water stays
//! wet a while, by the body part, not by the height (a dipped arm, a
//! sitting avatar wet up to where the water reached on its pose).
//!
//! After Radiance's wet look (rdwetlook.cpp, rdWetLookF.glsl, Radiance
//! Viewer contributors, originally LGPL 2.1). Each joint of a skeleton,
//! and each attachment point, keeps eight cells of wetness along the up
//! the water had in the joint's frame when it got wet, either way from
//! the joint (0.5 m; 1 m for the points, whose prims reach further); they
//! turn with the joint after. A cell under the water is soaked; out of it,
//! its fresh film drains to half wet in 2 s, it stays damp a while (the
//! lower the longer: it dries from the head down) and dries in a minute.
//!
//! The memory goes to the renderer in a block of `AVATAR_WET_BLOCKS`, the
//! water line around the avatar and its block in its weather entry (with
//! the snow: `avatar_snow::palette_entry`); the object shader reads both
//! (object.wgsl `avatar_wet_memory`, `avatar_wet_on`), so what it wears
//! takes it too. Region water only, as the rest of enhanced water's
//! effects; nothing is sent anywhere.

use super::anim::{ANCHOR_BASE, ANCHORS};
use aurora_render::{AVATAR_WET_BLOCKS, AVATAR_WET_JOINTS};
use glam::{Mat3, Mat4, Vec2, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

/// Cells per joint (object.wgsl `avatar_wet_joint`).
pub const CELLS: usize = 8;
/// How far the cells reach either way from a joint, and from an
/// attachment point (m).
const REACH_JOINT: f32 = 0.5;
const REACH_POINT: f32 = 1.0;
/// Seconds for a fresh film to drain to half wet, for the lowest cells to
/// stay damp beyond the highest, and to dry from half wet.
const DRAIN: f32 = 2.0;
const DRY_FRONT: f32 = 30.0;
const DRY: f32 = 60.0;
/// Seconds a newly wet cell takes to show (hides the cells' steps as the
/// water rises).
const FADE_IN: f32 = 1.0;
/// The fully wet band above the water line (m), and how far beside the
/// avatar the waves are read to tilt it (m).
const BAND: f32 = 0.05;
const WAVE_STEP: f32 = 0.5;
/// Only avatars this near the camera get wet (m).
const RANGE: f32 = 64.0;
/// A joint this far from its skeleton's root is not one in use (m).
const STRAY: f32 = 4.0;

const _: () = assert!(AVATAR_WET_JOINTS == ANCHOR_BASE + ANCHORS);

/// One step of a cell's wetness `w` over `dt` s: soaked under the water
/// (`hold` set to `hold_full`), out of it draining to half, held, then
/// drying (rdwetlook.cpp `step_joint_wetness`).
pub fn step(w: &mut f32, hold: &mut f32, mut dt: f32, under: bool, hold_full: f32) {
    if under {
        // at once: a short dip leaves no half-wet steps
        *w = 1.0;
        *hold = hold_full;
        return;
    }
    if *w > 0.5 {
        let rate = 0.5 / DRAIN;
        let need = (*w - 0.5) / rate;
        if dt <= need {
            *w -= dt * rate;
            return;
        }
        *w = 0.5;
        dt -= need;
    }
    if *hold > 0.0 {
        if dt <= *hold {
            *hold -= dt;
            return;
        }
        dt -= *hold;
        *hold = 0.0;
    }
    *w = (*w - dt * 0.5 / DRY).max(0.0);
}

#[derive(Clone, Copy, Default)]
struct Joint {
    wet: [f32; CELLS],
    hold: [f32; CELLS],
    shown: [f32; CELLS],
    /// The water's up in the joint's frame when it got wet.
    axis: Vec3,
    active: bool,
}

struct Wet {
    joints: Vec<Joint>,
    block: Option<usize>,
    /// The water line around it: z = x + y·px + z·py, w the band (0: none).
    line: [f32; 4],
}

impl Wet {
    fn new() -> Self {
        Self {
            joints: vec![Joint::default(); AVATAR_WET_JOINTS],
            block: None,
            line: [0.0; 4],
        }
    }
    fn wet(&self) -> bool {
        self.joints.iter().any(|j| j.active)
    }
}

/// The wet memory of the avatars near the camera.
#[derive(Default)]
pub struct AvatarWet {
    /// Set by the app each frame: enhanced water's settings (None: not
    /// in world), its clock (s) and the camera.
    pub settings: Option<aurora_render::WaterEffects>,
    pub clock: f32,
    pub eye: Vec3,
    avatars: HashMap<Uuid, Wet>,
    blocks: [Option<Uuid>; AVATAR_WET_BLOCKS],
    last: Option<f32>,
}

/// A value of 0..1 as an unorm8, and of -1..1 as an snorm16.
fn unorm8(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u32
}
fn snorm16(v: f32) -> u32 {
    u32::from((v.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16)
}

/// A joint's entry in its block (object.wgsl `avatar_wet_joint`): its
/// cells as unorm8 (x, y), its axis and reach as snorm16 (z, w).
fn pack(j: &Joint, reach: f32) -> [u32; 4] {
    if !j.active {
        return [0; 4];
    }
    let c = j.shown.map(unorm8);
    [
        c[0] | c[1] << 8 | c[2] << 16 | c[3] << 24,
        c[4] | c[5] << 8 | c[6] << 16 | c[7] << 24,
        snorm16(j.axis.x) | snorm16(j.axis.y) << 16,
        snorm16(j.axis.z) | snorm16(reach * 0.5) << 16,
    ]
}

impl AvatarWet {
    /// One frame at `time` (s). `palettes`: the scene's joint palettes,
    /// `stride` apart, with each avatar's slot from `slot` (only those in
    /// `avatars`, which are avatars, get wet); `water`: the water's surface
    /// at a point and its still level there (None: no water known there);
    /// `crest`: the highest the sea stands above its level (m). Returns
    /// the blocks to upload.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        on: bool,
        time: f32,
        palettes: &[[[f32; 4]; 4]],
        stride: usize,
        slot: impl Fn(&Uuid) -> Option<u32>,
        avatars: &[Uuid],
        eye: Vec3,
        crest: f32,
        water: impl Fn(Vec2) -> Option<(f32, f32)>,
    ) -> Vec<(usize, Vec<[u32; 4]>)> {
        let dt = match self.last {
            Some(last) if time >= last => (time - last).min(0.25),
            _ => 0.0,
        };
        self.last = Some(time);
        if !on {
            self.avatars.clear();
            self.blocks = [None; AVATAR_WET_BLOCKS];
            return Vec::new();
        }
        let mut uploads = Vec::new();
        let mut seen = Vec::new();
        for id in avatars {
            let Some(slot) = slot(id) else {
                continue;
            };
            let base = slot as usize * stride;
            let Some(block) = palettes.get(base..base + AVATAR_WET_JOINTS) else {
                continue;
            };
            let root = Mat4::from_cols_array_2d(&block[ANCHOR_BASE]);
            let at = root.w_axis.truncate();
            if !at.is_finite() || at.distance(eye) > RANGE {
                continue;
            }
            // the joints in use, where they are in the world
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            let joints: Vec<Option<Mat4>> = (0..AVATAR_WET_JOINTS)
                .map(|k| {
                    if k == ANCHOR_BASE {
                        return None;
                    }
                    let m = Mat4::from_cols_array_2d(&block[k]);
                    let w = if k < ANCHOR_BASE { root * m } else { m };
                    let o = w.w_axis.truncate();
                    if !o.is_finite() || o.distance(at) > STRAY || w.determinant().abs() < 1e-9 {
                        return None;
                    }
                    if k < ANCHOR_BASE {
                        low = low.min(o.z);
                        high = high.max(o.z);
                    }
                    Some(w)
                })
                .collect();
            if !low.is_finite() {
                continue;
            }
            // nothing to do far above the water with nothing wet
            let still = water(at.truncate());
            let dry = self.avatars.get(id).is_none_or(|w| !w.wet());
            if dry && still.is_none_or(|(_, level)| low - REACH_POINT > level + crest + 0.5) {
                self.avatars.remove(id);
                continue;
            }
            seen.push(*id);
            let wet = self.avatars.entry(*id).or_insert_with(Wet::new);
            // the water line around it, tilted by the waves beside it
            let c = at.truncate();
            let line = still.map(|(h, _)| {
                let x = Vec2::X * WAVE_STEP;
                let y = Vec2::Y * WAVE_STEP;
                let z = |q: Vec2| water(q).map_or(h, |w| w.0);
                let sx = (z(c + x) - z(c - x)) / (2.0 * WAVE_STEP);
                let sy = (z(c + y) - z(c - y)) / (2.0 * WAVE_STEP);
                (h, sx, sy)
            });
            wet.line = match line {
                Some((h, sx, sy)) if low - REACH_POINT < h + BAND => [h - sx * c.x - sy * c.y, sx, sy, BAND],
                _ => [0.0; 4],
            };
            let height = (high - low).max(0.3);
            let mut changed = false;
            for (k, w) in joints.iter().enumerate() {
                let joint = &mut wet.joints[k];
                let Some(w) = w else {
                    if joint.active {
                        *joint = Joint::default();
                        changed = true;
                    }
                    continue;
                };
                let reach = if k < ANCHOR_BASE { REACH_JOINT } else { REACH_POINT };
                let linear = Mat3::from_mat4(*w);
                let o = w.w_axis.truncate();
                if !joint.active {
                    // the water's up in the joint's frame while dry; then
                    // it turns with the joint, keeping the memory on the body
                    joint.axis = (linear.inverse() * Vec3::Z).normalize_or(Vec3::Z);
                }
                let up = (linear * joint.axis).normalize_or(Vec3::Z);
                let half = reach / CELLS as f32 * up.z.abs();
                let mut active = false;
                for cell in 0..CELLS {
                    let along = ((cell as f32 + 0.5) / CELLS as f32 * 2.0 - 1.0) * reach;
                    let p = o + up * along;
                    let under = line.is_some_and(|(h, sx, sy)| p.z - half < h + sx * (p.x - c.x) + sy * (p.y - c.y));
                    if !under && joint.wet[cell] <= 0.0 {
                        if joint.shown[cell] != 0.0 {
                            joint.shown[cell] = 0.0;
                            changed = true;
                        }
                        continue;
                    }
                    let below_top = ((high - p.z) / height).clamp(0.0, 1.0);
                    step(&mut joint.wet[cell], &mut joint.hold[cell], dt, under, DRY_FRONT * below_top);
                    active |= joint.wet[cell] > 0.0;
                    // under the water, short of full: the line's band shows it
                    let target = if under { joint.wet[cell].min(0.9) } else { joint.wet[cell] };
                    let shown = (joint.shown[cell] + dt / FADE_IN).min(target);
                    if (shown - joint.shown[cell]).abs() > 1.0 / 512.0 || (shown == 0.0) != (joint.shown[cell] == 0.0) {
                        joint.shown[cell] = shown;
                        changed = true;
                    }
                }
                if joint.active != active {
                    changed = true;
                }
                joint.active = active;
            }
            // a block while wet, given back once dry
            if wet.wet() && wet.block.is_none() {
                wet.block = self.blocks.iter().position(Option::is_none);
                if let Some(b) = wet.block {
                    self.blocks[b] = Some(*id);
                    changed = true;
                }
            } else if !wet.wet()
                && let Some(b) = wet.block.take()
            {
                self.blocks[b] = None;
            }
            if changed && let Some(b) = wet.block {
                let cells = wet
                    .joints
                    .iter()
                    .enumerate()
                    .map(|(k, j)| pack(j, if k < ANCHOR_BASE { REACH_JOINT } else { REACH_POINT }))
                    .collect();
                uploads.push((b, cells));
            }
        }
        // the avatars gone, out of range or dry above the water
        self.avatars.retain(|id, _| seen.contains(id));
        for b in &mut self.blocks {
            if b.is_some_and(|id| !self.avatars.contains_key(&id)) {
                *b = None;
            }
        }
        uploads
    }

    /// The water line around `id` (z = x + y·px + z·py, w the band; all
    /// zero: no water near) and its block plus one (0: none), for its
    /// weather entry.
    pub fn entry(&self, id: &Uuid) -> ([f32; 4], f32) {
        self.avatars
            .get(id)
            .map_or(([0.0; 4], 0.0), |w| (w.line, w.block.map_or(0.0, |b| b as f32 + 1.0)))
    }

    /// How wet joint `joint` of `id` (a palette entry: a joint, or an
    /// attachment point from `ANCHOR_BASE`) keeps point `at`, the joint
    /// placed in the world by `world`: its cells' wetness there (0..1, the
    /// film draining above 0.5). For the drips (avatar_drip.rs).
    pub fn wetness(&self, id: &Uuid, joint: usize, world: Mat4, at: Vec3) -> f32 {
        let Some(j) = self.avatars.get(id).and_then(|w| w.joints.get(joint)).filter(|j| j.active) else {
            return 0.0;
        };
        let reach = if joint < ANCHOR_BASE { REACH_JOINT } else { REACH_POINT };
        let up = (Mat3::from_mat4(world) * j.axis).normalize_or(Vec3::Z);
        let t = ((at - world.w_axis.truncate()).dot(up) / (2.0 * reach) + 0.5).clamp(0.0, 1.0) * CELLS as f32 - 0.5;
        let i = (t.floor().max(0.0) as usize).min(CELLS - 1);
        let next = (i + 1).min(CELLS - 1);
        let f = (t - i as f32).clamp(0.0, 1.0);
        j.wet[i] + (j.wet[next] - j.wet[i]) * f
    }

    /// No avatar is wet or near the water.
    pub fn is_clear(&self) -> bool {
        self.avatars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_drains_holds_and_dries() {
        let (mut w, mut hold) = (0.0, 0.0);
        step(&mut w, &mut hold, 0.1, true, 10.0);
        assert_eq!((w, hold), (1.0, 10.0));
        // out of the water: the film drains to half in 2 s
        step(&mut w, &mut hold, 1.0, false, 10.0);
        assert!((w - 0.75).abs() < 1e-5, "{w}");
        step(&mut w, &mut hold, 1.5, false, 10.0);
        assert_eq!(w, 0.5);
        assert!((hold - 9.5).abs() < 1e-5, "the rest of the step is held: {hold}");
        // held damp, then dry in a minute
        step(&mut w, &mut hold, 9.5, false, 10.0);
        assert_eq!((w, hold), (0.5, 0.0));
        step(&mut w, &mut hold, 30.0, false, 10.0);
        assert!((w - 0.25).abs() < 1e-5, "{w}");
        step(&mut w, &mut hold, 31.0, false, 10.0);
        assert_eq!(w, 0.0);
    }

    #[test]
    fn packing_keeps_the_cells_and_the_axis() {
        let j = Joint {
            shown: [0.0, 1.0, 0.5, 0.25, 0.0, 0.0, 0.0, 1.0],
            axis: Vec3::new(0.0, -1.0, 0.0),
            active: true,
            ..Default::default()
        };
        let p = pack(&j, 0.5);
        assert_eq!(p[0] & 0xff, 0);
        assert_eq!(p[0] >> 8 & 0xff, 255);
        assert_eq!(p[1] >> 24, 255);
        assert_eq!(p[2] >> 16, 0x8001, "-1 as snorm16");
        assert_eq!(p[3] >> 16, snorm16(0.25));
        assert_eq!(pack(&Joint::default(), 0.5), [0; 4], "dry: nothing");
    }

    /// A skeleton standing on the bed with its root `root_z` high, one
    /// joint at the root and one (an "arm") `arm` away.
    fn skeleton(root_z: f32, arm: Vec3) -> (Vec<[[f32; 4]; 4]>, HashMap<Uuid, u32>, Uuid) {
        let mut palettes = vec![[[0.0; 4]; 4]; aurora_render::PALETTE_STRIDE as usize];
        palettes[ANCHOR_BASE] = Mat4::from_translation(Vec3::new(10.0, 10.0, root_z)).to_cols_array_2d();
        palettes[0] = Mat4::IDENTITY.to_cols_array_2d();
        palettes[1] = Mat4::from_translation(arm).to_cols_array_2d();
        let id = Uuid::from_u128(5);
        (palettes, HashMap::from([(id, 0)]), id)
    }

    #[test]
    fn a_dipped_joint_gets_wet_and_another_stays_dry() {
        let mut wet = AvatarWet::default();
        // water at 20 m; the root 1 m above it, the "arm" reaching 1.5 m down
        let (palettes, slots, id) = skeleton(21.0, Vec3::new(0.3, 0.0, -1.5));
        let water = |_: Vec2| Some((20.0, 20.0));
        let stride = aurora_render::PALETTE_STRIDE as usize;
        let eye = Vec3::new(10.0, 10.0, 22.0);
        let mut uploads = Vec::new();
        for i in 0..30 {
            uploads = wet.update(
                true,
                i as f32 * 0.1,
                &palettes,
                stride,
                |i| slots.get(i).copied(),
                &[id],
                eye,
                0.2,
                water,
            );
        }
        let (line, block) = wet.entry(&id);
        assert_eq!(block, 1.0, "a block while wet");
        assert!((line[0] - 20.0).abs() < 1e-4 && line[3] == BAND, "{line:?}");
        let joints = &wet.avatars[&id].joints;
        assert!(joints[1].active && joints[1].wet.iter().all(|&w| w == 1.0), "the arm, under");
        assert!(!joints[0].active, "the root, above");
        // its memory went up, and it stays wet out of the water
        assert!(uploads.is_empty() || uploads[0].0 == 0);
        let dry_water = |_: Vec2| Some((10.0, 10.0));
        wet.update(true, 3.1, &palettes, stride, |i| slots.get(i).copied(), &[id], eye, 0.2, dry_water);
        wet.update(true, 3.3, &palettes, stride, |i| slots.get(i).copied(), &[id], eye, 0.2, dry_water);
        assert!(wet.avatars[&id].joints[1].active, "still wet");
        assert_eq!(wet.entry(&id).0, [0.0; 4], "no water line far above it");
        // turned off: all gone
        wet.update(false, 3.4, &palettes, stride, |i| slots.get(i).copied(), &[id], eye, 0.2, dry_water);
        assert!(wet.is_clear() && wet.entry(&id).1 == 0.0);
    }

    #[test]
    fn a_dry_avatar_far_above_the_water_costs_nothing() {
        let mut wet = AvatarWet::default();
        let (palettes, slots, id) = skeleton(40.0, Vec3::new(0.3, 0.0, -1.0));
        let stride = aurora_render::PALETTE_STRIDE as usize;
        let up = wet.update(
            true,
            0.0,
            &palettes,
            stride,
            |i| slots.get(i).copied(),
            &[id],
            Vec3::new(10.0, 10.0, 40.0),
            0.2,
            |_| Some((20.0, 20.0)),
        );
        assert!(up.is_empty() && wet.is_clear());
    }
}
