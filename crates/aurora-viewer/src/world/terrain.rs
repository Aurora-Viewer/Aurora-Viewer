//! Region heightmaps, terrain composition (port of the noise logic in
//! `llvlcomposition.cpp`) and chunk mesh generation.

use aurora_net::estate::TerrainTransform;
use aurora_prim::noise;
use aurora_render::{SkinVertex, Vertex};
use glam::Vec3;
use uuid::Uuid;

#[path = "terrain_paint.rs"]
mod paint;
pub use paint::PaintMap;

/// Local surface changes never overwrite the simulator's region handshake.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceOverride {
    pub ids: [Uuid; 4],
    pub materials: bool,
    pub transforms: [TerrainTransform; 4],
}

#[derive(Default)]
pub struct SurfaceState {
    pub transforms: [TerrainTransform; 4],
    pub preview: Option<SurfaceOverride>,
    /// Keep an applied surface visible until the next region handshake.
    pub applied: Option<SurfaceOverride>,
    /// Experimental local paintmap, as LLAdvancedTerrainCreateLocalPaintMap.
    /// Never sent in estate commands or saved as the region's terrain.
    pub paintmap: Option<PaintMap>,
    pub paint_requested: bool,
    pub paint_failed: bool,
    pub(super) paint_job: Option<paint::PaintJob>,
}

impl SurfaceState {
    pub fn effective(&self) -> Option<SurfaceOverride> {
        self.preview.or(self.applied)
    }
}

pub const CHUNK_CELLS: u32 = 32;

/// Terrain heights for one region; patches are written as they arrive.
#[derive(Clone)]
pub struct Heightmap {
    pub size_x: u32,
    pub size_y: u32,
    pub heights: Vec<f32>,
    pub patch_received: Vec<bool>,
    pub patches_x: u32,
}

impl Heightmap {
    pub fn new(size_x: u32, size_y: u32) -> Heightmap {
        let patches_x = size_x / 16;
        let patches_y = size_y / 16;
        Heightmap {
            size_x,
            size_y,
            heights: vec![0.0; (size_x * size_y) as usize],
            patch_received: vec![false; (patches_x * patches_y) as usize],
            patches_x,
        }
    }

    /// Write a patch; returns the chunk indices that became dirty.
    pub fn apply_patch(&mut self, px: u32, py: u32, size: u32, data: &[f32]) -> Vec<u32> {
        let x0 = px * size;
        let y0 = py * size;
        if x0 >= self.size_x || y0 >= self.size_y || data.len() < (size * size) as usize {
            return Vec::new();
        }
        for j in 0..size {
            for i in 0..size {
                let x = x0 + i;
                let y = y0 + j;
                if x < self.size_x && y < self.size_y {
                    self.heights[(y * self.size_x + x) as usize] = data[(j * size + i) as usize];
                }
            }
        }
        let pp = size / 16;
        for j in 0..pp.max(1) {
            for i in 0..pp.max(1) {
                let idx = (py * pp + j) * self.patches_x + px * pp + i;
                if let Some(r) = self.patch_received.get_mut(idx as usize) {
                    *r = true;
                }
            }
        }
        // Chunks containing this patch, plus neighbours sharing its edges.
        let cx_n = self.size_x / CHUNK_CELLS;
        let cy_n = self.size_y / CHUNK_CELLS;
        let mut dirty = Vec::new();
        let cx0 = x0.saturating_sub(1) / CHUNK_CELLS;
        let cy0 = y0.saturating_sub(1) / CHUNK_CELLS;
        let cx1 = ((x0 + size) / CHUNK_CELLS).min(cx_n - 1);
        let cy1 = ((y0 + size) / CHUNK_CELLS).min(cy_n - 1);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                dirty.push(cy * cx_n + cx);
            }
        }
        dirty
    }

    pub fn received_fraction(&self) -> f32 {
        let n = self.patch_received.iter().filter(|r| **r).count();
        n as f32 / self.patch_received.len().max(1) as f32
    }

    #[inline]
    pub fn height(&self, x: i32, y: i32) -> f32 {
        let xi = x.clamp(0, self.size_x as i32 - 1) as u32;
        let yi = y.clamp(0, self.size_y as i32 - 1) as u32;
        self.heights[(yi * self.size_x + xi) as usize]
    }

    /// Bilinear height at region coordinates.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let x = x.clamp(0.0, self.size_x as f32 - 1.001);
        let y = y.clamp(0.0, self.size_y as f32 - 1.001);
        let xi = x.floor() as i32;
        let yi = y.floor() as i32;
        let fx = x - xi as f32;
        let fy = y - yi as f32;
        let h00 = self.height(xi, yi);
        let h10 = self.height(xi + 1, yi);
        let h01 = self.height(xi, yi + 1);
        let h11 = self.height(xi + 1, yi + 1);
        let a = h00 + (h10 - h00) * fx;
        let b = h01 + (h11 - h01) * fx;
        a + (b - a) * fy
    }

    pub fn chunk_count(&self) -> u32 {
        (self.size_x / CHUNK_CELLS) * (self.size_y / CHUNK_CELLS)
    }
}

/// Parameters for the texture composition (from RegionHandshake).
#[derive(Debug, Clone, Copy)]
pub struct Composition {
    /// TerrainStartHeight00, 01, 10, 11 = LLVLComposition corners SOUTHWEST,
    /// SOUTHEAST, NORTHWEST, NORTHEAST.
    pub start_height: [f32; 4],
    pub height_range: [f32; 4],
    pub origin_x: f64,
    pub origin_y: f64,
}

/// llvlcomposition.cpp `bilinear(v00, v01, v10, v11, x, y)`, called with
/// (SW, SE, NW, NE): despite the corner names, the second corner (SE) is
/// weighted along y and the third (NW) along x. Kept as is: the region's
/// corner heights are authored against what the viewers draw.
fn bilinear(v: [f32; 4], x: f32, y: f32) -> f32 {
    let ix = 1.0 - x;
    let iy = 1.0 - y;
    ix * iy * v[0] + x * iy * v[2] + ix * y * v[1] + x * y * v[3]
}

impl Composition {
    /// LLSurfacePatch::eval's second alpha-ramp coordinate, computed at
    /// mesh vertices and interpolated independently of the composition.
    pub(super) fn blend_noise(&self, x: f32, y: f32) -> f32 {
        let scale = (1.0 / (4.9215_f32 * 7.0)) * 0.222_222_22;
        let vx = (((self.origin_x + f64::from(x)) as f32) * scale) % 256.0;
        let vy = (((self.origin_y + f64::from(y)) as f32) * scale) % 256.0;
        (noise::noise2(vx, vy) * 0.75 + 0.5).clamp(0.0, 1.0)
    }

    /// Composition value in [0, 3] at region coordinates: port of
    /// LLVLComposition::generateHeights (llvlcomposition.cpp, originally
    /// LGPL 2.1). Only the horizontal position feeds the noise (noise2 /
    /// turbulence2 ignore the height component LL fills in).
    pub fn value(&self, hm: &Heightmap, x: f32, y: f32, height: f32) -> f32 {
        const XY_SCALE_INV: f32 = 1.0 / 4.9215;
        const SLOPE_SQUARED: f32 = 1.5 * 1.5;
        const NOISE_MAGNITUDE: f32 = 2.0;
        const ASSET_COUNT: f32 = 4.0;
        let fx = x / hm.size_x as f32;
        let fy = y / hm.size_y as f32;
        let start = bilinear(self.start_height, fx, fy);
        let range = bilinear(self.height_range, fx, fy);
        let vx = ((self.origin_x + x as f64) as f32) * XY_SCALE_INV;
        let vy = ((self.origin_y + y as f64) as f32) * XY_SCALE_INV;
        // low frequency component for large divisions
        let mut twiddle = noise::noise2(vx * 0.222_222_22, vy * 0.222_222_22) * 6.5;
        // high frequency component
        twiddle += noise::turbulence2(vx, vy, 2.0) * SLOPE_SQUARED;
        twiddle *= NOISE_MAGNITUDE;
        let v = (height + twiddle - start) * ASSET_COUNT / range;
        // LL divides by the raw range: a zero range gives +-inf (clamped to
        // 0 or 3); 0 / 0 (undefined in LL) gives the first layer
        if v.is_nan() { 0.0 } else { v.clamp(0.0, 3.0) }
    }
}

/// Build the mesh of one chunk (positions in region space).
pub fn build_chunk(hm: &Heightmap, comp: &Composition, chunk: u32) -> (Vec<Vertex>, Vec<SkinVertex>, Vec<u16>) {
    let cx_n = hm.size_x / CHUNK_CELLS;
    let cx = chunk % cx_n;
    let cy = chunk / cx_n;
    let x0 = (cx * CHUNK_CELLS) as i32;
    let y0 = (cy * CHUNK_CELLS) as i32;
    let n = CHUNK_CELLS as i32 + 1;
    let mut verts = Vec::with_capacity((n * n) as usize);
    let mut normals = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let x = x0 + i;
            let y = y0 + j;
            let h = hm.height(x, y);
            let normal = terrain_normal(hm, x, y);
            let c = comp.value(hm, x as f32, y as f32, h);
            // LL stores composition in a float texcoord, independent of the
            // tangent. Region-space material UVs come from position.xy.
            let v = Vertex::new(
                [x as f32, y as f32, h],
                normal.to_array(),
                [c, comp.blend_noise(x as f32, y as f32)],
            );
            verts.push(v);
            normals.push(normal);
        }
    }
    let mut idx = Vec::with_capacity((CHUNK_CELLS * CHUNK_CELLS * 6) as usize);
    for j in 0..CHUNK_CELLS as i32 {
        for i in 0..CHUNK_CELLS as i32 {
            let a = (j * n + i) as u16;
            let b = (j * n + i + 1) as u16;
            let c = ((j + 1) * n + i) as u16;
            let d = ((j + 1) * n + i + 1) as u16;
            idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    let frames = terrain_frames(&mut verts, &normals, &idx, hm.size_x as f32);
    (verts, frames, idx)
}

/// LLSurfacePatch::calcNormal<false>(..., stride=2), used by default for
/// PBR too (RenderTerrainPBRNormalsEnabled defaults to false). Its diagonal
/// four-metre stencil is different from an axial one-metre central gradient.
fn terrain_normal(hm: &Heightmap, x: i32, y: i32) -> Vec3 {
    let sw = hm.height(x - 2, y - 2);
    let se = hm.height(x + 2, y - 2);
    let nw = hm.height(x - 2, y + 2);
    let ne = hm.height(x + 2, y + 2);
    Vec3::new(sw + nw - se - ne, sw + se - nw - ne, 8.0).normalize_or(Vec3::Z)
}

/// gen_terrain_tangents / LLCalculateTangentArray (llvosurfacepatch.cpp,
/// llvolume.cpp, LGPL 2.1). Accumulate the mesh's dP/dU and dP/dV, then
/// orthogonalize against its normal; a tangent cannot be inferred from N.
fn terrain_frames(vertices: &mut [Vertex], normals: &[Vec3], indices: &[u16], region_width: f32) -> Vec<SkinVertex> {
    let mut tan1 = vec![Vec3::ZERO; vertices.len()];
    let mut tan2 = vec![Vec3::ZERO; vertices.len()];
    for tri in indices.as_chunks::<3>().0 {
        let [a, b, c] = tri.map(usize::from);
        let p = Vec3::from_array(vertices[a].pos);
        let d1 = Vec3::from_array(vertices[b].pos) - p;
        let d2 = Vec3::from_array(vertices[c].pos) - p;
        let uv1 = d1.truncate() / region_width;
        let uv2 = d2.truncate() / region_width;
        let rd = uv1.x * uv2.y - uv2.x * uv1.y;
        let r = if rd * rd > f32::EPSILON {
            1.0 / rd
        } else if rd > 0.0 {
            1024.0
        } else {
            -1024.0
        };
        let sdir = (d1 * uv2.y - d2 * uv1.y) * r;
        let tdir = (d2 * uv1.x - d1 * uv2.x) * r;
        for i in [a, b, c] {
            tan1[i] += sdir;
            tan2[i] += tdir;
        }
    }
    vertices
        .iter_mut()
        .enumerate()
        .map(|(i, v)| {
            let n = normals[i];
            let tp = tan1[i] - n * n.dot(tan1[i]);
            let (t, hand) = if tp.length_squared() > 1e-5 {
                (tp.normalize(), if n.cross(tan1[i]).dot(tan2[i]) < 0.0 { -1.0 } else { 1.0 })
            } else {
                (Vec3::Z, 1.0)
            };
            v.normal[3] = aurora_render::pack_snorm(hand);
            aurora_render::pack_terrain_frame(n, t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_builds() {
        let mut hm = Heightmap::new(256, 256);
        let dirty = hm.apply_patch(1, 2, 16, &[21.0; 256]);
        assert!(!dirty.is_empty());
        assert_eq!(hm.sample(20.0, 40.0), 21.0);
        let comp = Composition {
            start_height: [10.0; 4],
            height_range: [60.0; 4],
            origin_x: 256000.0,
            origin_y: 256000.0,
        };
        let (v, frames, i) = build_chunk(&hm, &comp, 0);
        assert_eq!(frames.len(), v.len());
        assert_eq!(v.len(), 33 * 33);
        assert_eq!(i.len(), 32 * 32 * 6);
        assert!(i.iter().all(|&x| (x as usize) < v.len()));
    }

    #[test]
    fn curved_terrain_keeps_the_legacy_diagonal_normal_and_mesh_tangent() {
        let mut hm = Heightmap::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                hm.heights[(y * 32 + x) as usize] = 20.0 + x as f32 * 0.2 + y as f32 * 0.3;
            }
        }
        hm.heights[16 * 32 + 17] += 3.0;
        let comp = Composition {
            start_height: [5.0; 4],
            height_range: [100.0; 4],
            origin_x: 0.0,
            origin_y: 0.0,
        };
        let (vertices, frames, _) = build_chunk(&hm, &comp, 0);
        let i = 16 * 33 + 16;
        let (n, t) = aurora_render::unpack_terrain_frame(frames[i]);
        let expected_n = Vec3::new(-0.2, -0.3, 1.0).normalize();
        // The neighbouring spike contributes to the six indexed triangles'
        // dP/dU, but not to LL's diagonal +/-2 normal stencil.
        let sdir = Vec3::new(1.0, 0.0, 1.2);
        let expected_t = (sdir - expected_n * expected_n.dot(sdir)).normalize();
        assert!(n.distance(expected_n) < 0.0002, "normal {n}, expected {expected_n}");
        assert!(t.distance(expected_t) < 0.0002, "tangent {t}, expected {expected_t}");
        let inferred = Vec3::new(n.z, 0.0, -n.x).normalize();
        assert!(t.distance(inferred) > 0.1, "a terrain tangent cannot be reconstructed from N");
        assert_eq!(vertices[i].uv[0], comp.value(&hm, 16.0, 16.0, hm.height(16, 16)));
        assert_eq!(vertices[i].normal[3], 127);
        assert_eq!(std::mem::size_of::<SkinVertex>(), 8);
    }

    #[test]
    fn composition_matches_firestorm() {
        let mut hm = Heightmap::new(256, 256);
        for (x, y, h) in [(0, 0, 20.0), (10, 20, 21.5), (128, 64, 35.0), (200, 250, 60.0), (33, 177, 24.25)] {
            hm.heights[y * 256 + x] = h;
        }
        let comp = Composition {
            start_height: [10.0; 4],
            height_range: [60.0; 4],
            origin_x: 256000.0,
            origin_y: 256000.0,
        };
        for (x, y, h, want) in [
            (0, 0, 20.0, 0.828_389_2),
            (10, 20, 21.5, 0.958_008_5),
            (128, 64, 35.0, 1.700_175_2),
            (200, 250, 60.0, 3.0),
            (33, 177, 24.25, 0.699_741),
        ] {
            let got = comp.value(&hm, x as f32, y as f32, h);
            assert!((got - want).abs() <= 1e-5, "comp({x}, {y}) = {got}, Firestorm {want}");
        }
    }

    #[test]
    fn corner_mapping_follows_firestorm() {
        // v = [SW, SE, NW, NE]: NW is reached along x, SE along y
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(bilinear(v, 0.0, 0.0), 1.0);
        assert_eq!(bilinear(v, 1.0, 0.0), 3.0);
        assert_eq!(bilinear(v, 0.0, 1.0), 2.0);
        assert_eq!(bilinear(v, 1.0, 1.0), 4.0);
        assert!((bilinear(v, 0.5, 0.5) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn turbulence_has_no_bias() {
        // the old |noise| turbulence always raised the blend height
        let mut sum = 0.0f64;
        let n = 200;
        for j in 0..n {
            for i in 0..n {
                sum += noise::turbulence2(i as f32 * 0.37 + 1000.0, j as f32 * 0.41 + 2000.0, 2.0) as f64;
            }
        }
        let mean = sum / (n * n) as f64;
        assert!(mean.abs() < 0.05, "mean {mean}");
    }
}
