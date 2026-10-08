//! Region heightmaps, terrain composition (port of the noise logic in
//! `llvlcomposition.cpp`) and chunk mesh generation.

use aurora_render::Vertex;

pub const CHUNK_CELLS: u32 = 32;

/// Classic improved Perlin noise (2D/3D), used for terrain texture blending.
mod perlin {
    const P: [u8; 256] = [
        151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190,
        6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125,
        136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92,
        41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130,
        116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
        207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155,
        167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193,
        238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
        176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156,
        180,
    ];

    #[inline]
    fn p(i: i32) -> i32 {
        P[(i & 255) as usize] as i32
    }

    #[inline]
    fn fade(t: f32) -> f32 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    #[inline]
    fn lerp(t: f32, a: f32, b: f32) -> f32 {
        a + t * (b - a)
    }

    #[inline]
    fn grad(hash: i32, x: f32, y: f32, z: f32) -> f32 {
        let h = hash & 15;
        let u = if h < 8 { x } else { y };
        let v = if h < 4 {
            y
        } else if h == 12 || h == 14 {
            x
        } else {
            z
        };
        (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
    }

    pub fn noise3(x: f32, y: f32, z: f32) -> f32 {
        let xi = x.floor() as i32;
        let yi = y.floor() as i32;
        let zi = z.floor() as i32;
        let (x, y, z) = (x - x.floor(), y - y.floor(), z - z.floor());
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let a = p(xi) + yi;
        let aa = p(a) + zi;
        let ab = p(a + 1) + zi;
        let b = p(xi + 1) + yi;
        let ba = p(b) + zi;
        let bb = p(b + 1) + zi;
        lerp(
            w,
            lerp(
                v,
                lerp(u, grad(p(aa), x, y, z), grad(p(ba), x - 1.0, y, z)),
                lerp(u, grad(p(ab), x, y - 1.0, z), grad(p(bb), x - 1.0, y - 1.0, z)),
            ),
            lerp(
                v,
                lerp(u, grad(p(aa + 1), x, y, z - 1.0), grad(p(ba + 1), x - 1.0, y, z - 1.0)),
                lerp(u, grad(p(ab + 1), x, y - 1.0, z - 1.0), grad(p(bb + 1), x - 1.0, y - 1.0, z - 1.0)),
            ),
        )
    }

    pub fn turbulence3(x: f32, y: f32, z: f32, octaves: u32) -> f32 {
        let mut t = 0.0;
        let mut f = 1.0;
        for _ in 0..octaves {
            t += noise3(x * f, y * f, z * f).abs() / f;
            f *= 2.0;
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
    /// SW, SE, NW, NE (= 00, 01, 10, 11)
    pub start_height: [f32; 4],
    pub height_range: [f32; 4],
    pub origin_x: f64,
    pub origin_y: f64,
}

fn bilinear(v: [f32; 4], x: f32, y: f32) -> f32 {
    let s = v[0] + (v[1] - v[0]) * x;
    let n = v[2] + (v[3] - v[2]) * x;
    s + (n - s) * y
}

impl Composition {
    /// Composition value in [0, 3] at region coordinates (llvlcomposition.cpp).
    pub fn value(&self, hm: &Heightmap, x: f32, y: f32, height: f32) -> f32 {
        const XY_SCALE_INV: f32 = 1.0 / 4.9215;
        const Z_SCALE_INV: f32 = 1.0 / 4.0;
        const SLOPE_SQUARED: f32 = 1.5 * 1.5;
        const NOISE_MAGNITUDE: f32 = 2.0;
        let fx = x / hm.size_x as f32;
        let fy = y / hm.size_y as f32;
        let start = bilinear(self.start_height, fx, fy);
        let range = bilinear(self.height_range, fx, fy).max(0.01);
        let vx = ((self.origin_x + x as f64) as f32) * XY_SCALE_INV;
        let vy = ((self.origin_y + y as f64) as f32) * XY_SCALE_INV;
        let vz = height * Z_SCALE_INV;
        let mut twiddle = perlin::noise3(vx * 0.222_222_22, vy * 0.222_222_22, vz * 0.222_222_22) * 6.5;
        twiddle += perlin::turbulence3(vx, vy, vz, 2) * SLOPE_SQUARED;
        twiddle *= NOISE_MAGNITUDE;
        ((height + twiddle - start) * 4.0 / range).clamp(0.0, 3.0)
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

    #[test]
    fn noise_is_bounded() {
        for i in 0..1000 {
            let v = perlin::noise3(i as f32 * 0.37, i as f32 * 0.11, 0.5);
            assert!((-1.5..=1.5).contains(&v));
        }
    }
}
