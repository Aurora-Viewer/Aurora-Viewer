//! Rain prototype: the terrain's height in a square around the camera, for
//! the renderer's puddles (aurora-render `RainGround`): they gather in the
//! terrain's dips, judged from the region's heightmap rather than from the
//! screen. 64 m a side in half-metre cells, its corner snapped to 4 m so it
//! only moves (and is sampled again) after the camera has; again every two
//! seconds for terraforming. Nothing while it does not rain.

use super::Scene;
use crate::world::World;
use aurora_render::{RAIN_GROUND_SIZE, RainGround};
use glam::{Vec2, Vec3};
use rayon::prelude::*;
use std::time::{Duration, Instant};

const EXTENT: f32 = 64.0;
const SNAP: f32 = 4.0;
const REFRESH: Duration = Duration::from_secs(2);
/// A cell without terrain (outside every region).
const NO_GROUND: f32 = -1.0e4;

#[derive(Default)]
pub struct RainGroundCache {
    ground: RainGround,
    at: Option<Instant>,
}

impl Scene {
    /// The terrain heights around `eye` into the frame's lists while it
    /// rains (`raining`); none otherwise.
    pub fn update_rain_ground(&mut self, world: &World, eye: Vec3, raining: bool) {
        if !raining {
            self.rain_ground = RainGroundCache::default();
            self.lists.rain_ground = RainGround::default();
            return;
        }
        let origin = (eye.truncate() / SNAP).floor() * SNAP - Vec2::splat(EXTENT * 0.5);
        let cache = &mut self.rain_ground;
        let moved = cache.ground.origin != origin.to_array();
        if moved || cache.at.is_none_or(|t| t.elapsed() >= REFRESH) {
            let n = RAIN_GROUND_SIZE as usize;
            let cell = EXTENT / RAIN_GROUND_SIZE as f32;
            let heights: Vec<f32> = (0..n * n)
                .into_par_iter()
                .map(|i| {
                    let xy = origin + (Vec2::new((i % n) as f32, (i / n) as f32) + Vec2::splat(0.5)) * cell;
                    world.ground_height(xy.extend(0.0)).unwrap_or(NO_GROUND)
                })
                .collect();
            cache.ground = RainGround {
                origin: origin.to_array(),
                extent: EXTENT,
                heights: heights.into(),
            };
            cache.at = Some(Instant::now());
        }
        self.lists.rain_ground = cache.ground.clone();
    }
}
