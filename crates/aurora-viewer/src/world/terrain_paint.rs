//! Local terrain paintmaps, following LLTerrainPaintMap::bakeHeightNoiseIntoPBRPaintMapRGB
//! and LLAdvancedTerrainCreateLocalPaintMap (LGPL 2.1). The region protocol still
//! uses height/noise composition. A local bake freezes its four blend weights;
//! changing heights later does not repaint it. Workers build and stage the map.

use super::{Composition, Heightmap, SurfaceState};
use aurora_render::{StagedTexture, StagingPool};
use rayon::prelude::*;
use std::sync::{Arc, Mutex, mpsc};
use uuid::Uuid;

pub struct PaintMap {
    pub id: Uuid,
    pub staged: Option<StagedTexture>,
    // Keep the frozen weights for scene recreation, but release staging
    // chunks after upload so terrain experiments cannot starve streaming.
    mips: Arc<Vec<(u32, u32, Vec<u8>)>>,
    restage: Option<Mutex<mpsc::Receiver<Option<StagedTexture>>>>,
}

pub(super) type PaintJob = Mutex<mpsc::Receiver<Option<PaintMap>>>;

impl PaintMap {
    pub fn prepare(&mut self, pool: Arc<StagingPool>) -> Option<&StagedTexture> {
        if let Some(job) = &mut self.restage {
            match job.get_mut().unwrap_or_else(|e| e.into_inner()).try_recv() {
                Ok(staged) => {
                    self.staged = staged;
                    self.restage = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.restage = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.staged.is_none() && self.restage.is_none() {
            let mips = self.mips.clone();
            let (tx, rx) = mpsc::channel();
            self.restage = Some(Mutex::new(rx));
            rayon::spawn(move || {
                let levels: Vec<_> = mips.iter().map(|(w, h, data)| (*w, *h, data.as_slice())).collect();
                let _ = tx.send(StagedTexture::new(&pool, &levels));
            });
        }
        self.staged.as_ref().filter(|s| s.data.ready())
    }
}

fn resolution(max_dimension: u32, max_upload: u64) -> u32 {
    let mut size = 2048.min(max_dimension);
    // Include mip levels and their padded rows. Small staging pools use a
    // 1024 px map rather than a large synchronous frame-thread copy.
    while size > 16 && u64::from(size).pow(2) * 6 > max_upload {
        size /= 2;
    }
    size
}

impl SurfaceState {
    pub fn paint_busy(&self) -> bool {
        self.paint_requested || self.paint_job.is_some()
    }

    pub fn request_paint(&mut self) {
        if !self.paint_busy() {
            self.paint_requested = true;
            self.paint_failed = false;
        }
    }

    pub fn clear_paint(&mut self) {
        self.paintmap = None;
        self.paint_requested = false;
        self.paint_failed = false;
        self.paint_job = None;
    }

    pub fn poll_paint(&mut self, hm: &Heightmap, comp: Composition, pool: Arc<StagingPool>, max_dimension: u32) {
        if std::mem::take(&mut self.paint_requested) {
            let hm = hm.clone();
            let (tx, rx) = mpsc::channel();
            self.paint_job = Some(Mutex::new(rx));
            // TerrainPaintResolution defaults to 2048. Keep the work and its
            // mapped staging writes off the frame thread; no GPU readback.
            let resolution = resolution(max_dimension, pool.max_region_bytes());
            rayon::spawn(move || {
                let pixels = bake(&hm, comp, resolution);
                let mips = Arc::new(aurora_render::build_mips(resolution, resolution, pixels, u32::MAX));
                let levels: Vec<_> = mips.iter().map(|(w, h, data)| (*w, *h, data.as_slice())).collect();
                let staged = StagedTexture::new(&pool, &levels);
                let _ = tx.send(staged.map(|staged| PaintMap {
                    id: Uuid::new_v4(),
                    staged: Some(staged),
                    mips,
                    restage: None,
                }));
            });
        }
        if let Some(job) = &mut self.paint_job {
            match job.get_mut().unwrap_or_else(|e| e.into_inner()).try_recv() {
                Ok(Some(map)) => {
                    self.paintmap = Some(map);
                    self.paint_job = None;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.paint_failed = true;
                    self.paint_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}

// IMG_ALPHA_GRAD, same 256 linear-filtered texels as env.wgsl. The GPU
// regressions exercise both the bake weights and the production GPU mixer.
const RAMP: [u32; 64] = [
    0xfdfdfeff, 0xfbfbfbfc, 0xf9f9fafa, 0xf6f6f7f7, 0xf3f4f4f5, 0xf0f1f1f2, 0xedeeeeef, 0xeaeaebec, 0xe6e7e7e8, 0xe3e3e4e5, 0xdfe0e1e2,
    0xdbdcdddf, 0xd7d9d9da, 0xd3d5d5d7, 0xcfd0d2d3, 0xcbcccdcf, 0xc7c8c9ca, 0xc2c3c5c6, 0xbebfc0c1, 0xb9babbbd, 0xb5b6b7b9, 0xb0b1b2b4,
    0xabacaeaf, 0xa7a8a9ab, 0xa2a4a4a6, 0x9d9ea0a1, 0x989a9b9c, 0x94959697, 0x8f8f9193, 0x8a8a8c8d, 0x84868788, 0x80818283, 0x7b7b7d7e,
    0x7676787a, 0x70727374, 0x6c6c6f6f, 0x6768696a, 0x62636466, 0x5d5e5f61, 0x585a5b5c, 0x53545657, 0x4f505152, 0x4a4c4d4e, 0x45474849,
    0x41424344, 0x3d3d3f3f, 0x38393a3b, 0x34353537, 0x2f303232, 0x2c2c2e2f, 0x2728292b, 0x23242527, 0x20212223, 0x1d1e1e1f, 0x19191b1c,
    0x15161718, 0x12131415, 0x0f101111, 0x0c0d0d0f, 0x090a0a0b, 0x06070909, 0x04050506, 0x02020303, 0x00010101,
];

fn weights(c: f32) -> [f32; 4] {
    let ramp = |u: f32| {
        let x = (u * 256.0 - 0.5).clamp(0.0, 255.0);
        let i = x as usize;
        let texel = |i: usize| ((RAMP[i >> 2] >> ((i & 3) * 8)) & 255) as f32 / 255.0;
        texel(i) + (texel((i + 1).min(255)) - texel(i)) * (x - i as f32)
    };
    let (a, b, f) = (ramp(c), ramp(c - 2.0), ramp(c - 1.0));
    let mut w = [f * a, f * (1.0 - a), (1.0 - f) * b, (1.0 - f) * (1.0 - b)];
    for v in &mut w {
        *v = (*v - 0.1).max(0.0);
    }
    let sum: f32 = w.iter().sum();
    w.map(|v| v / sum)
}

fn bake(hm: &Heightmap, comp: Composition, resolution: u32) -> Vec<u8> {
    let stride = hm.size_x as usize + 1;
    // Exactly the composition supplied by build_chunk, including its snorm8
    // quantization. Interpolate on the same SW-NE triangle diagonal.
    let grid: Vec<f32> = (0..stride * (hm.size_y as usize + 1))
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % stride) as f32, (i / stride) as f32);
            let c = comp.value(hm, x, y, hm.height(x as i32, y as i32));
            (aurora_render::pack_snorm(c / 3.0 * 2.0 - 1.0) as f32 / 127.0 * 0.5 + 0.5) * 3.0
        })
        .collect();
    let mut pixels = vec![0; (resolution * resolution * 4) as usize];
    pixels.par_chunks_exact_mut(4).enumerate().for_each(|(i, pixel)| {
        let x = ((i % resolution as usize) as f32 + 0.5) * hm.size_x as f32 / resolution as f32;
        let y = ((i / resolution as usize) as f32 + 0.5) * hm.size_y as f32 / resolution as f32;
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x.fract(), y.fract());
        let at = iy * stride + ix;
        let sw = grid[at];
        let ne = grid[at + stride + 1];
        let c = if fx >= fy {
            sw * (1.0 - fx) + grid[at + 1] * (fx - fy) + ne * fy
        } else {
            sw * (1.0 - fy) + grid[at + stride] * (fy - fx) + ne * fx
        };
        let w = weights(c);
        pixel.copy_from_slice(&[
            (w[1] * 255.0).round() as u8,
            (w[2] * 255.0).round() as u8,
            (w[3] * 255.0).round() as u8,
            255,
        ]);
    });
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paintmap_encodes_linear_remainder_and_cancel_discards_pending_work() {
        assert_eq!(weights(0.0), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(weights(3.0), [0.0, 0.0, 0.0, 1.0]);
        let w = weights(1.5);
        assert!(w[1] > 0.4 && w[2] > 0.4);
        let hm = Heightmap::new(32, 32);
        let comp = Composition {
            start_height: [100.0; 4],
            height_range: [100.0; 4],
            origin_x: 0.0,
            origin_y: 0.0,
        };
        assert!(bake(&hm, comp, 16).as_chunks::<4>().0.iter().all(|p| *p == [0, 0, 0, 255]));
        assert_eq!(resolution(8192, 32 << 20), 2048);
        assert_eq!(resolution(8192, 16 << 20), 1024);
        let mut state = SurfaceState::default();
        state.request_paint();
        assert!(state.paint_busy());
        let (tx, rx) = mpsc::channel();
        state.paint_job = Some(Mutex::new(rx));
        state.clear_paint();
        assert!(!state.paint_busy());
        assert!(tx.send(None).is_err(), "a cancelled or stale bake cannot install its map");
    }
}
