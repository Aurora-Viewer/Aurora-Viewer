//! Aurora Weather's fog: the ground it pools on, in a square around the
//! camera, for the renderer's fog volume (aurora-render `FogGround`). Fog
//! fills hollows and valleys and thins on the hills: its height is measured
//! from the ground smoothed over tens of metres, so that in a valley the
//! level it fills to stands above the valley floor, and on a summit below
//! it. The water counts as ground: fog lies on the sea. 1 km a side in 4 m
//! cells (var-regions and the neighbouring regions included, through
//! `World::ground_height`), its corner snapped to 32 m so it only moves
//! after the camera has; again every four seconds for terraforming. The far
//! fog lies on it too (common.wgsl `fog_ground_at`): seen from high up, the
//! fog follows the hills to the horizon instead of stopping where the
//! volume does. The
//! height fog takes its level under the camera, the volume the whole
//! square. Nothing while the fog is off.

use super::Scene;
use crate::world::World;
use aurora_render::{FOG_GROUND_SIZE, FogGround};
use glam::{Vec2, Vec3};
use rayon::prelude::*;
use std::time::{Duration, Instant};

const EXTENT: f32 = 1024.0;
const SNAP: f32 = 32.0;
const REFRESH: Duration = Duration::from_secs(4);
/// The smoothing's reach, cells each side of a box pass (two passes: about
/// a 40 m wide bell).
const SMOOTH: usize = 4;

#[derive(Default)]
pub struct FogGroundCache {
    ground: FogGround,
    at: Option<Instant>,
}

impl Scene {
    /// The ground the fog pools on around `eye` into the frame's lists while
    /// it is drawn (`on`); none otherwise. `water` is the region's
    /// water height.
    pub fn update_fog_ground(&mut self, world: &World, eye: Vec3, water: f32, on: bool) {
        if !on {
            self.fog_ground = FogGroundCache::default();
            self.lists.fog_ground = FogGround::default();
            return;
        }
        let origin = (eye.truncate() / SNAP).floor() * SNAP - Vec2::splat(EXTENT * 0.5);
        let cache = &mut self.fog_ground;
        let moved = cache.ground.origin != origin.to_array();
        if moved || cache.at.is_none_or(|t| t.elapsed() >= REFRESH) {
            let n = FOG_GROUND_SIZE as usize;
            let cell = EXTENT / FOG_GROUND_SIZE as f32;
            // outside every region: the water's level, as an open sea
            let raw: Vec<f32> = (0..n * n)
                .into_par_iter()
                .map(|i| {
                    let xy = origin + (Vec2::new((i % n) as f32, (i / n) as f32) + Vec2::splat(0.5)) * cell;
                    world.ground_height(xy.extend(0.0)).unwrap_or(water).max(water)
                })
                .collect();
            let smooth = smoothed(&raw, n, SMOOTH);
            cache.ground = FogGround {
                origin: origin.to_array(),
                extent: EXTENT,
                heights: raw.iter().zip(&smooth).map(|(&r, &s)| [r, s]).collect(),
            };
            cache.at = Some(Instant::now());
        }
        self.lists.fog_ground = cache.ground.clone();
    }
}

/// `h` (`n` a side, row by row) under two box passes of `r` cells each
/// side, along the rows then the columns: a bell about `4 r` cells wide.
/// The edges take the mean of the cells they have.
fn smoothed(h: &[f32], n: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], along_rows: bool| -> Vec<f32> {
        (0..n * n)
            .into_par_iter()
            .map(|i| {
                let (x, y) = (i % n, i / n);
                let at = |k: usize| if along_rows { src[y * n + k] } else { src[k * n + x] };
                let c = if along_rows { x } else { y };
                let (a, b) = (c.saturating_sub(r), (c + r).min(n - 1));
                (a..=b).map(at).sum::<f32>() / (b - a + 1) as f32
            })
            .collect()
    };
    let mut out = h.to_vec();
    for _ in 0..2 {
        out = pass(&out, true);
        out = pass(&out, false);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valley_fills_above_its_floor_and_a_summit_stands_above_its_fog() {
        // a ridge across the middle (20 m), a valley at its foot (0 m)
        let n = 32;
        let h: Vec<f32> = (0..n * n).map(|i| if (12..20).contains(&(i % n)) { 20.0 } else { 0.0 }).collect();
        let s = smoothed(&h, n, 3);
        let at = |x: usize| s[16 * n + x];
        // on the ridge the level is below the summit, at its foot above
        // the valley floor: fog pools at the foot and thins on the top
        assert!(at(16) < 20.0 && at(16) > 10.0, "{}", at(16));
        assert!(at(10) > 0.5, "{}", at(10));
        // far from it, the flat floor stays as it is
        assert!(at(0).abs() < 1e-4, "{}", at(0));
    }

    #[test]
    fn flat_ground_stays_flat() {
        let n = 16;
        let s = smoothed(&vec![7.5; n * n], n, 4);
        assert!(s.iter().all(|v| (v - 7.5).abs() < 1e-4));
    }
}
