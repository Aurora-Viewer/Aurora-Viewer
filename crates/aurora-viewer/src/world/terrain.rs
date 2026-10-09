//! Region heightmaps, terrain composition (port of the noise logic in
//! `llvlcomposition.cpp`) and chunk mesh generation.

use aurora_render::Vertex;

pub const CHUNK_CELLS: u32 = 32;

/// Port of the Second Life viewer's lattice noise (indra/newview/noise.h,
/// noise.cpp, originally LGPL 2.1), used by the terrain composition. The
/// gradient and permutation tables come from `srand(42)` + `rand()`, so the
/// texture blend only matches Firestorm's if the same pseudo-random sequence
/// is reproduced: Firestorm for Windows (the reference) is built with MSVC,
/// whose `rand()` is a 32-bit LCG returning bits 16..30. All arithmetic is in
/// f32 in the same order as the C code (built with /fp:precise, no FMA).
mod noise {
    use std::sync::OnceLock;

    const B: usize = 0x100;
    const BM: i32 = 0xff;
    const NF32: f32 = 4096.0;

    struct Tables {
        p: [usize; B + B + 2],
        g2: [[f32; 2]; B + B + 2],
    }

    /// MSVC CRT `rand()`: holdrand = holdrand * 214013 + 2531011,
    /// result (holdrand >> 16) & 0x7fff.
    pub(super) struct MsvcRand(u32);

    impl MsvcRand {
        pub(super) fn new(seed: u32) -> MsvcRand {
            MsvcRand(seed)
        }

        pub(super) fn next(&mut self) -> i32 {
            self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
            ((self.0 >> 16) & 0x7fff) as i32
        }
    }

    /// noise.h `init`: g1, g2 and g3 are drawn interleaved for each lattice
    /// point (1 + 2 + 3 calls), then the permutation is shuffled. Only `p`
    /// and `g2` are used (noise2), but g1 / g3 still consume their draws.
    fn init() -> Tables {
        let mut r = MsvcRand::new(42);
        let mut t = Tables {
            p: [0; B + B + 2],
            g2: [[0.0; 2]; B + B + 2],
        };
        let draw = |r: &mut MsvcRand| ((r.next() % (B + B) as i32) - B as i32) as f32 / B as f32;
        for i in 0..B {
            t.p[i] = i;
            let _g1 = draw(&mut r);
            let v = [draw(&mut r), draw(&mut r)];
            // normalize2 (never a zero vector with this seed, see tests)
            let s = 1.0 / (v[0] * v[0] + v[1] * v[1]).sqrt();
            t.g2[i] = [v[0] * s, v[1] * s];
            for _ in 0..3 {
                let _g3 = draw(&mut r);
            }
        }
        // while (--i) { k = p[i]; p[i] = p[j = rand() % B]; p[j] = k; }
        for i in (1..B).rev() {
            let j = (r.next() % B as i32) as usize;
            t.p.swap(i, j);
        }
        for i in 0..B + 2 {
            t.p[B + i] = t.p[i];
            t.g2[B + i] = t.g2[i];
        }
        t
    }

    fn tables() -> &'static Tables {
        static T: OnceLock<Tables> = OnceLock::new();
        T.get_or_init(init)
    }

    /// noise.h `fast_setup`: lattice cell (wrapped to 8 bits) and offsets.
    #[inline]
    fn fast_setup(v: f32) -> (usize, usize, f32, f32) {
        let r1 = v + NF32;
        let t = r1 as i32;
        let b0 = (t & BM) as u8;
        let b1 = b0.wrapping_add(1);
        let r0 = r1 - t as f32;
        (b0 as usize, b1 as usize, r0, r0 - 1.0)
    }

    #[inline]
    fn s_curve(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }

    #[inline]
    fn lerp(t: f32, a: f32, b: f32) -> f32 {
        a + t * (b - a)
    }

    /// noise.cpp `noise2`: signed 2-D gradient noise, about [-0.7, 0.7].
    pub fn noise2(x: f32, y: f32) -> f32 {
        let t = tables();
        let (bx0, bx1, rx0, rx1) = fast_setup(x);
        let (by0, by1, ry0, ry1) = fast_setup(y);
        let i = t.p[bx0];
        let j = t.p[bx1];
        let b00 = t.p[i + by0];
        let b10 = t.p[j + by0];
        let b01 = t.p[i + by1];
        let b11 = t.p[j + by1];
        let sx = s_curve(rx0);
        let sy = s_curve(ry0);
        let at2 = |rx: f32, ry: f32, q: [f32; 2]| rx * q[0] + ry * q[1];
        let a = lerp(sx, at2(rx0, ry0, t.g2[b00]), at2(rx1, ry0, t.g2[b10]));
        let b = lerp(sx, at2(rx0, ry1, t.g2[b01]), at2(rx1, ry1, t.g2[b11]));
        lerp(sy, a, b)
    }

    /// noise.h `turbulence2`: signed octaves from `freq` down to 1, each
    /// weighted by 1/freq (zero mean, unlike a classic |noise| turbulence).
    pub fn turbulence2(x: f32, y: f32, mut freq: f32) -> f32 {
        let mut t = 0.0;
        while freq >= 1.0 {
            t += noise2(freq * x, freq * y) / freq;
            freq *= 0.5;
        }
        t
    }
}

/// Terrain heights for one region; patches are written as they arrive.
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
pub fn build_chunk(hm: &Heightmap, comp: &Composition, chunk: u32) -> (Vec<Vertex>, Vec<u16>) {
    let cx_n = hm.size_x / CHUNK_CELLS;
    let cx = chunk % cx_n;
    let cy = chunk / cx_n;
    let x0 = (cx * CHUNK_CELLS) as i32;
    let y0 = (cy * CHUNK_CELLS) as i32;
    let n = CHUNK_CELLS as i32 + 1;
    let mut verts = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let x = x0 + i;
            let y = y0 + j;
            let h = hm.height(x, y);
            let hl = hm.height(x - 1, y);
            let hr = hm.height(x + 1, y);
            let hd = hm.height(x, y - 1);
            let hu = hm.height(x, y + 1);
            let normal = glam::Vec3::new(hl - hr, hd - hu, 2.0).normalize_or(glam::Vec3::Z);
            let c = comp.value(hm, x as f32, y as f32, h);
            let mut v = Vertex::new([x as f32, y as f32, h], normal.to_array(), [x as f32, y as f32]);
            v.normal[3] = aurora_render::pack_snorm(c / 3.0 * 2.0 - 1.0);
            verts.push(v);
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
    (verts, idx)
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
        let (v, i) = build_chunk(&hm, &comp, 0);
        assert_eq!(v.len(), 33 * 33);
        assert_eq!(i.len(), 32 * 32 * 6);
        assert!(i.iter().all(|&x| (x as usize) < v.len()));
    }

    /// Reference values from Firestorm's noise.cpp / noise.h compiled with
    /// MSVC 2022 (/O2 /fp:precise /arch:AVX2, the CRT's rand()), the same
    /// toolchain as Firestorm for Windows.
    #[test]
    fn noise_matches_firestorm() {
        let close = |a: f32, b: f32| (a - b).abs() <= 1e-6;
        for (x, y, n, t) in [
            (0.0, 0.0, 0.0, 0.0),
            (0.5, 0.25, -0.356_576_26, -0.409_460_78),
            (1.37, 2.11, 0.197_173_73, 0.061_257_884),
            (10.3, -4.7, 0.099_117_63, -0.060_509_764),
            (123.456, 78.9, 0.125_369_07, 0.200_200_22),
            (52012.2, 52043.7, -0.132_383_69, -0.441_523_5),
        ] {
            let got = noise::noise2(x, y);
            assert!(close(got, n), "noise2({x}, {y}) = {got}, Firestorm {n}");
            let got = noise::turbulence2(x, y, 2.0);
            assert!(close(got, t), "turbulence2({x}, {y}) = {got}, Firestorm {t}");
        }
    }

    #[test]
    fn msvc_rand_sequence() {
        // srand(42); rand() x 3 with the MSVC CRT
        let mut r = noise::MsvcRand::new(42);
        assert_eq!([r.next(), r.next(), r.next()], [175, 400, 17869]);
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
