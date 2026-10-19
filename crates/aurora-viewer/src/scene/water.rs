//! Water surfaces: one quad per known region at that region's own water
//! height, plus "void" water at the agent region's height around them
//! (LLVOWater / LLVOVoidWater).

use crate::world::World;
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, Renderer, Vertex};
use glam::{Mat4, Vec2, Vec3};

/// Void water: region-sized cells within this many regions of the main
/// one, then four large slabs out to `VOID_EXTENT`.
const VOID_CELLS: i32 = 4;
const VOID_EXTENT: f32 = 16384.0;

struct Tile {
    origin: Vec3,
    size: Vec2,
    record: u32,
    patches: Vec<(u32, Vec3, Vec2)>,
}

#[derive(Default)]
pub struct WaterGpu {
    quad: Option<MeshAlloc>,
    grids: [Option<MeshAlloc>; 4],
    near_grids: [Option<MeshAlloc>; 4],
    tiles: Vec<Tile>,
    records: Vec<u32>,
    key: u64,
}

impl WaterGpu {
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        if let Some(q) = self.quad.take() {
            renderer.free_mesh(q);
        }
        for grid in self.grids.iter_mut().chain(&mut self.near_grids) {
            if let Some(g) = grid.take() {
                renderer.free_mesh(g);
            }
        }
        self.tiles.clear();
        self.key = 0;
    }

    /// Rebuild the water records when regions, sizes or heights change.
    pub fn sync(&mut self, renderer: &mut Renderer, world: &World) {
        let Some(main) = world.main_region else {
            return;
        };
        // (offset, size, height) of every region with known water
        let mut regions: Vec<(Vec3, (u32, u32), f32)> = Vec::new();
        for (h, r) in &world.regions {
            let (Some(info), Some(offset)) = (r.info.as_ref(), world.region_offset(*h)) else {
                continue;
            };
            regions.push((offset, (info.size_x.max(256), info.size_y.max(256)), info.water_height));
        }
        regions.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.0.y.total_cmp(&b.0.y)));
        let main_height = world.main_water_height();
        let mut key = main ^ 0x5157_4154_4552;
        for (o, s, hgt) in &regions {
            key = key.rotate_left(7).wrapping_mul(0x100_0000_01B3)
                ^ (o.x.to_bits() as u64)
                ^ ((o.y.to_bits() as u64) << 32)
                ^ ((s.0 as u64) << 16)
                ^ (hgt.to_bits() as u64).rotate_left(40);
        }
        key ^= (main_height.to_bits() as u64) << 3;
        if key == self.key && self.quad.is_some() {
            return;
        }
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        if self.quad.is_none() {
            self.quad = mesh(renderer, 1);
            for (i, n) in [32, 48, 64, 96].into_iter().enumerate() {
                self.grids[i] = mesh(renderer, n);
            }
            for (i, n) in [64, 96, 160, 224].into_iter().enumerate() {
                self.near_grids[i] = mesh(renderer, n);
            }
        }
        if self.quad.is_none() {
            return;
        }
        self.key = key;
        self.tiles.clear();
        let mut add = |renderer: &mut Renderer, x: f32, y: f32, w: f32, h: f32, z: f32| {
            let mut allocate = |x: f32, y: f32, w: f32, h: f32| {
                let model = Mat4::from_translation(Vec3::new(x, y, z)) * Mat4::from_scale(Vec3::new(w, h, 1.0));
                let record = renderer.records.alloc(DrawRecord {
                    model: model.to_cols_array_2d(),
                    ..Default::default()
                });
                self.records.push(record);
                record
            };
            let record = allocate(x, y, w, h);
            let mut patches = Vec::new();
            if w.max(h) > 64.0 && w.max(h) <= 2048.0 {
                let nx = (w / 64.0).ceil() as u32;
                let ny = (h / 64.0).ceil() as u32;
                let size = Vec2::new(w / nx as f32, h / ny as f32);
                for j in 0..ny {
                    for i in 0..nx {
                        let origin = Vec3::new(x + i as f32 * size.x, y + j as f32 * size.y, z);
                        patches.push((allocate(origin.x, origin.y, size.x, size.y), origin, size));
                    }
                }
            }
            self.tiles.push(Tile {
                origin: Vec3::new(x, y, z),
                size: Vec2::new(w, h),
                record,
                patches,
            });
        };
        for (o, s, hgt) in &regions {
            add(renderer, o.x, o.y, s.0 as f32, s.1 as f32, *hgt);
        }
        // void water cells not covered by a region
        let covered = |x: f32, y: f32| {
            regions
                .iter()
                .any(|(o, s, _)| x >= o.x && x < o.x + s.0 as f32 && y >= o.y && y < o.y + s.1 as f32)
        };
        for j in -VOID_CELLS..=VOID_CELLS {
            for i in -VOID_CELLS..=VOID_CELLS {
                let (x, y) = (i as f32 * 256.0, j as f32 * 256.0);
                if !covered(x + 128.0, y + 128.0) {
                    add(renderer, x, y, 256.0, 256.0, main_height);
                }
            }
        }
        let lo = -VOID_CELLS as f32 * 256.0;
        let hi = (VOID_CELLS + 1) as f32 * 256.0;
        let e = VOID_EXTENT;
        add(renderer, -e, hi, 2.0 * e, e - hi, main_height); // north
        add(renderer, -e, -e, 2.0 * e, e + lo, main_height); // south
        add(renderer, -e, lo, e + lo, hi - lo, main_height); // west
        add(renderer, hi, lo, e - hi, hi - lo, main_height); // east
    }

    pub fn draws(&self, view: &super::CullView, quality: Option<u8>) -> impl Iterator<Item = DrawCmd> + '_ {
        let view = *view;
        let grid = quality.and_then(|q| self.grids[q.min(3) as usize]);
        let near_grid = quality.and_then(|q| self.near_grids[q.min(3) as usize]);
        let quad = self.quad;
        self.tiles.iter().flat_map(move |tile| {
            let closest = (view.eye.truncate() - tile.origin.truncate()).clamp(Vec2::ZERO, tile.size) + tile.origin.truncate();
            let subdivide = grid.is_some() && !tile.patches.is_empty() && closest.distance(view.eye.truncate()) < 160.0;
            std::iter::once((tile.record, tile.origin, tile.size))
                .chain(tile.patches.iter().copied())
                .enumerate()
                .filter_map(move |(i, (record, origin, size))| {
                    if (i == 0) == subdivide {
                        return None;
                    }
                    let center = origin + size.extend(0.0) * 0.5;
                    if !view.sphere_visible(center, size.length() * 0.5 + 0.7) {
                        return None;
                    }
                    let nearest = (view.eye.truncate() - origin.truncate()).clamp(Vec2::ZERO, size) + origin.truncate();
                    let dense = size.max_element() <= 64.1 && nearest.distance(view.eye.truncate()) < 160.0;
                    // Only patches touching the close 24 m need sub-metre
                    // geometry for a body furrow; retain the ordinary far grid.
                    let close = dense && nearest.distance(view.eye.truncate()) < 24.0;
                    let mesh = if close {
                        near_grid.or(grid).or(quad)
                    } else if dense {
                        grid.or(quad)
                    } else {
                        quad
                    };
                    mesh.map(|a| DrawCmd {
                        index_count: a.index_count,
                        first_index: a.index_offset,
                        base_vertex: a.vertex_offset as i32,
                        record,
                        bounds: [0.0; 4],
                    })
                })
        })
    }
}

fn mesh(renderer: &mut Renderer, n: usize) -> Option<MeshAlloc> {
    let mut vertices = Vec::with_capacity((n + 1) * (n + 1));
    for y in 0..=n {
        for x in 0..=n {
            let u = x as f32 / n as f32;
            let v = y as f32 / n as f32;
            vertices.push(Vertex::new([u, v, 0.0], [0.0, 0.0, 1.0], [u, v]));
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6);
    for y in 0..n {
        for x in 0..n {
            let i = (y * (n + 1) + x) as u16;
            let j = i + (n + 1) as u16;
            indices.extend_from_slice(&[i, i + 1, j + 1, i, j + 1, j]);
        }
    }
    renderer.upload_mesh(&vertices, &indices)
}
