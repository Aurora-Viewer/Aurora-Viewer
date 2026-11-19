//! Water surfaces: one quad per known region at that region's own water
//! height, plus "void" water at the agent region's height around them
//! (LLVOWater / LLVOVoidWater).

use crate::world::World;
use aurora_render::{DrawCmd, DrawRecord, MeshAlloc, Renderer, Vertex};
use glam::{Mat4, Vec2, Vec3};

/// Void water: region-sized cells within this many regions of the main
/// one, then four large slabs out to `VOID_EXTENT`, or as far as the
/// horizon of a curved planet (`void_reach`).
const VOID_CELLS: i32 = 4;
const VOID_EXTENT: f32 = 16384.0;
/// The farthest the void water reaches on a curved planet (m): the
/// horizon from ~3 km up on the Earth. Region coordinates in single
/// precision step by ~3 cm there, beyond the horizon.
const VOID_EXTENT_MAX: f32 = 262_144.0;
/// On a curved planet the slabs are grids whose lines run on from the
/// region cells' (every 256 m) and grow apart by this factor outwards. A
/// flat cell of side s strays from the drop's parabola by s² / 8R wherever
/// it lies, so the cells at the region block, as fine as its own, leave no
/// crack along it, and the far ones, about a fifth of their distance wide,
/// stray by under a pixel's angle.
const SLAB_GROWTH: f32 = 1.2;

/// How far the void water must reach from the main region's origin: to
/// the horizon seen from `eye` (with a margin) on a curved planet, rounded
/// up to a power of two so that the water is rebuilt only when the camera
/// climbs or descends a long way; `VOID_EXTENT` when flat.
pub fn void_reach(eye: Vec3, water: f32, radius: Option<f32>) -> f32 {
    let Some(radius) = radius else {
        return VOID_EXTENT;
    };
    let height = (eye.z - water).max(2.0);
    // the horizon plus the camera's distance from the origin, and a margin
    let reach = aurora_render::curvature::horizon_distance(height, radius) * 1.25 + eye.truncate().length();
    if !reach.is_finite() {
        return VOID_EXTENT_MAX;
    }
    ((reach.max(VOID_EXTENT) as u64).next_power_of_two() as f32).min(VOID_EXTENT_MAX)
}

struct Tile {
    origin: Vec3,
    size: Vec2,
    record: u32,
    patches: Vec<(u32, Vec3, Vec2)>,
    /// A slab's own graded grid (curved planet), in region metres.
    mesh: Option<MeshAlloc>,
}

/// The lines of the slabs' grids along one axis (curved planet): the region
/// block's, every 256 m from `lo` to `hi`, then further apart by
/// SLAB_GROWTH out to ±`reach`. All four slabs take theirs from this list,
/// so that their shared edges and the block's have the same vertices.
fn slab_lines(lo: f32, hi: f32, reach: f32) -> Vec<f32> {
    let outwards = |from: f32, sign: f32| {
        let mut lines = Vec::new();
        let (mut at, mut step) = (from, 256.0);
        while (reach - at * sign) > step * 0.5 {
            at += step * sign;
            lines.push(at);
            step *= SLAB_GROWTH;
        }
        // the last line on the edge
        match lines.last_mut() {
            Some(last) => *last = reach * sign,
            None => lines.push(reach * sign),
        }
        lines
    };
    let mut lines: Vec<f32> = outwards(lo, -1.0).into_iter().rev().collect();
    let cells = ((hi - lo) / 256.0).round() as i32;
    lines.extend((0..=cells).map(|i| lo + i as f32 * 256.0));
    lines.extend(outwards(hi, 1.0));
    lines
}

#[derive(Default)]
pub struct WaterGpu {
    quad: Option<MeshAlloc>,
    /// The slabs' graded grids on a curved planet (`slab_lines`).
    slabs: Vec<MeshAlloc>,
    /// How far the void water reaches now (`void_reach`).
    reach: f32,
    grids: [Option<MeshAlloc>; 4],
    near_grids: [Option<MeshAlloc>; 4],
    tiles: Vec<Tile>,
    records: Vec<u32>,
    key: u64,
    /// The agent region's water height (LLEnvironment::getWaterHeight),
    /// the plane the blended faces are drawn before or after the water by.
    pub height: Option<f32>,
}

impl WaterGpu {
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        for mesh in self.quad.take().into_iter().chain(self.slabs.drain(..)) {
            renderer.free_mesh(mesh);
        }
        for grid in self.grids.iter_mut().chain(&mut self.near_grids) {
            if let Some(g) = grid.take() {
                renderer.free_mesh(g);
            }
        }
        self.tiles.clear();
        self.key = 0;
    }

    /// Rebuild the water records when regions, sizes or heights change, or
    /// the void water must reach further (`radius`: planet curvature's, m).
    pub fn sync(&mut self, renderer: &mut Renderer, world: &World, eye: Vec3, radius: Option<f32>) {
        let curved = radius.is_some();
        let mut reach = void_reach(eye, world.main_water_height(), radius);
        // kept until it is four times what is needed: a camera at the height
        // of a threshold does not rebuild the water at each crossing
        if curved && reach < self.reach && reach * 4.0 > self.reach {
            reach = self.reach;
        }
        let Some(main) = world.main_region else {
            self.height = None;
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
        self.height = Some(main_height);
        let mut key = main ^ 0x5157_4154_4552;
        for (o, s, hgt) in &regions {
            key = key.rotate_left(7).wrapping_mul(0x100_0000_01B3)
                ^ (o.x.to_bits() as u64)
                ^ ((o.y.to_bits() as u64) << 32)
                ^ ((s.0 as u64) << 16)
                ^ (hgt.to_bits() as u64).rotate_left(40);
        }
        key ^= (main_height.to_bits() as u64) << 3;
        key ^= (reach.to_bits() as u64).rotate_left(23) ^ u64::from(curved) << 61;
        if key == self.key && self.quad.is_some() {
            return;
        }
        for r in self.records.drain(..) {
            renderer.records.free(r);
        }
        for slab in self.slabs.drain(..) {
            renderer.free_mesh(slab);
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
        self.reach = reach;
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
                mesh: None,
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
        let e = reach;
        // (x, y, width, height): north, south, west, east
        let slabs = [
            (-e, hi, 2.0 * e, e - hi),
            (-e, -e, 2.0 * e, e + lo),
            (-e, lo, e + lo, hi - lo),
            (hi, lo, e - hi, hi - lo),
        ];
        if !curved {
            for (x, y, w, h) in slabs {
                add(renderer, x, y, w, h, main_height);
            }
            return;
        }
        // A curved planet bends them at their vertices: graded grids on
        // shared lines, in region metres (a record placing them at the
        // water's height only)
        let lines = slab_lines(lo, hi, e);
        for (x, y, w, h) in slabs {
            let within = |a: f32, b: f32| {
                lines
                    .iter()
                    .copied()
                    .filter(|&l| l >= a - 0.5 && l <= b + 0.5)
                    .collect::<Vec<f32>>()
            };
            let Some(slab) = grid_mesh(renderer, &within(x, x + w), &within(y, y + h)) else {
                continue;
            };
            let record = renderer.records.alloc(DrawRecord {
                model: Mat4::from_translation(Vec3::new(0.0, 0.0, main_height)).to_cols_array_2d(),
                ..Default::default()
            });
            self.records.push(record);
            self.tiles.push(Tile {
                origin: Vec3::new(x, y, main_height),
                size: Vec2::new(w, h),
                record,
                patches: Vec::new(),
                mesh: Some(slab),
            });
            self.slabs.push(slab);
        }
    }

    /// `radius`: planet curvature's (m), None when flat.
    pub fn draws(&self, view: &super::CullView, quality: Option<u8>, radius: Option<f32>) -> impl Iterator<Item = DrawCmd> + '_ {
        let view = *view;
        let curvature = aurora_render::curvature::Curvature::new(view.eye, radius);
        let grid = quality.and_then(|q| self.grids[q.min(3) as usize]);
        let near_grid = quality.and_then(|q| self.near_grids[q.min(3) as usize]);
        let quad = self.quad;
        self.tiles.iter().flat_map(move |tile| {
            let own = tile.mesh;
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
                    // drawn lower than it lies by up to the drop at its
                    // farthest corner (the frustum test is flat)
                    let far = (view.eye.truncate() - center.truncate()).abs() + size * 0.5;
                    let sag = curvature.drop_at(view.eye.truncate() + far);
                    if !view.sphere_visible(center - Vec3::Z * sag * 0.5, size.length() * 0.5 + 0.7 + sag * 0.5) {
                        return None;
                    }
                    let nearest = (view.eye.truncate() - origin.truncate()).clamp(Vec2::ZERO, size) + origin.truncate();
                    let dense = size.max_element() <= 64.1 && nearest.distance(view.eye.truncate()) < 160.0;
                    // Only patches touching the close 24 m need sub-metre
                    // geometry for a body furrow; retain the ordinary far grid.
                    let close = dense && nearest.distance(view.eye.truncate()) < 24.0;
                    let mesh = if own.is_some() {
                        own
                    } else if close {
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

/// A grid on these lines (region metres), at z 0.
fn grid_mesh(renderer: &mut Renderer, xs: &[f32], ys: &[f32]) -> Option<MeshAlloc> {
    let (nx, ny) = (xs.len(), ys.len());
    if nx < 2 || ny < 2 || nx * ny > u16::MAX as usize {
        return None;
    }
    let mut vertices = Vec::with_capacity(nx * ny);
    for &y in ys {
        for &x in xs {
            vertices.push(Vertex::new([x, y, 0.0], [0.0, 0.0, 1.0], [x / 256.0, y / 256.0]));
        }
    }
    let mut indices = Vec::with_capacity((nx - 1) * (ny - 1) * 6);
    for row in 0..ny - 1 {
        for col in 0..nx - 1 {
            let i = (row * nx + col) as u16;
            let j = i + nx as u16;
            indices.extend_from_slice(&[i, i + 1, j + 1, i, j + 1, j]);
        }
    }
    renderer.upload_mesh(&vertices, &indices)
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

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH: Option<f32> = Some(aurora_render::curvature::EARTH_RADIUS);

    #[test]
    fn the_slabs_lines_follow_the_cells_then_grow_out_to_the_edge() {
        let (lo, hi) = (-1024.0, 1280.0);
        for reach in [VOID_EXTENT, 131_072.0, VOID_EXTENT_MAX] {
            let lines = slab_lines(lo, hi, reach);
            assert_eq!((lines[0], *lines.last().unwrap()), (-reach, reach));
            assert!(lines.windows(2).all(|w| w[1] > w[0]), "{lines:?}");
            // every cell line of the block, then 256 m on both sides
            for i in 0..=9 {
                assert!(lines.contains(&(lo + i as f32 * 256.0)));
            }
            assert!(lines.contains(&(hi + 256.0)) && lines.contains(&(lo - 256.0)));
            // a cell is at most about a fifth of its distance from the main region
            // wide (the camera's region, around 128, 128), so that it strays
            // from the drop by under a pixel's angle
            for w in lines.windows(2).filter(|w| w[0] >= hi) {
                assert!(w[1] - w[0] <= (w[0] - 128.0) * 0.3, "{w:?}");
            }
            // small enough for one slab's u16 indices
            assert!(lines.len() * lines.len() / 2 < u16::MAX as usize, "{}", lines.len());
        }
    }

    #[test]
    fn flat_void_water_keeps_its_extent() {
        assert_eq!(void_reach(Vec3::new(128.0, 128.0, 5000.0), 20.0, None), VOID_EXTENT);
    }

    #[test]
    fn curved_void_water_reaches_past_the_horizon() {
        for (z, eye) in [
            (30.0, Vec3::new(128.0, 128.0, 30.0)),
            (120.0, Vec3::new(700.0, -300.0, 120.0)),
            (1020.0, Vec3::new(128.0, 128.0, 1020.0)),
        ] {
            let reach = void_reach(eye, 20.0, EARTH);
            let horizon = aurora_render::curvature::horizon_distance(z - 20.0, EARTH.unwrap());
            // every slab's far edge lies past the horizon, in every direction
            assert!(reach - eye.x.abs().max(eye.y.abs()) > horizon, "{z} m: {reach} vs {horizon}");
            assert!(reach >= VOID_EXTENT && reach.log2().fract() == 0.0, "{reach}");
        }
        // standing on the water: the regions' own extent already reaches it
        assert_eq!(void_reach(Vec3::new(128.0, 128.0, 22.0), 20.0, EARTH), VOID_EXTENT);
        // from very high up, capped
        assert_eq!(void_reach(Vec3::new(0.0, 0.0, 50_000.0), 20.0, EARTH), VOID_EXTENT_MAX);
    }
}
