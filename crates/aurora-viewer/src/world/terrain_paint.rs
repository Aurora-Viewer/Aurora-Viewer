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

fn bake(hm: &Heightmap, comp: Composition, resolution: u32) -> Vec<u8> {
    let stride = hm.size_x as usize + 1;
    // Exactly the float composition supplied by build_chunk. Interpolate on
    // the same SW-NE triangle diagonal.
    let grid: Vec<[f32; 2]> = (0..stride * (hm.size_y as usize + 1))
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % stride) as f32, (i / stride) as f32);
            [comp.value(hm, x, y, hm.height(x as i32, y as i32)), comp.blend_noise(x, y)]
        })
        .collect();
    let mut pixels = vec![0; (resolution * resolution * 4) as usize];
    pixels.par_chunks_exact_mut(4).enumerate().for_each(|(i, pixel)| {
        let x = ((i % resolution as usize) as f32 + 0.5) * hm.size_x as f32 / resolution as f32;
        let y = ((i / resolution as usize) as f32 + 0.5) * hm.size_y as f32 / resolution as f32;
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x.fract(), y.fract());
        let at = iy * stride + ix;
        let blend = std::array::from_fn::<_, 2, _>(|axis| {
            let sw = grid[at][axis];
            let ne = grid[at + stride + 1][axis];
            if fx >= fy {
                sw * (1.0 - fx) + grid[at + 1][axis] * (fx - fy) + ne * fy
            } else {
                sw * (1.0 - fy) + grid[at + stride][axis] * (fy - fx) + ne * fx
            }
        });
        let w = aurora_render::textures::terrain_blend_weights(blend[0], blend[1]);
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
    fn height_noise_bake_uses_the_reference_two_dimensional_transition() {
        for (noise, first) in [(0.1, 0.6261045), (0.5, 0.260_794), (0.9, 0.6997064)] {
            let w = aurora_render::textures::terrain_blend_weights(0.5, noise);
            assert!((w[0] - first).abs() < 0.00001);
            assert!((w[1] - (1.0 - first)).abs() < 0.00001);
            assert_eq!(&w[2..], &[0.0, 0.0]);
        }
    }

    #[test]
    fn local_paintmap_encodes_linear_remainder_and_cancel_discards_pending_work() {
        assert_eq!(aurora_render::textures::terrain_blend_weights(0.0, 0.5), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(aurora_render::textures::terrain_blend_weights(3.0, 0.5), [0.0, 0.0, 0.0, 1.0]);
        let w = aurora_render::textures::terrain_blend_weights(1.5, 0.5);
        assert!(w[1] > 0.1 && w[2] > 0.1);
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
