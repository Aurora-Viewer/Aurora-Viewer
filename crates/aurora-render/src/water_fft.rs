//! The wind sea of enhanced water as a spectrum (Tessendorf's method): a
//! JONSWAP (or Phillips) spectrum of thousands of waves, evolved in time
//! and brought back to heights by inverse FFTs, in two tiles of different
//! sizes ("cascades") so that its repetition does not show. It replaces
//! the few crossing wind-chop trains, whose crossing drew a lattice of
//! diamonds; the long swell (water_sea.rs) stays a single refracted train
//! on top of it.
//!
//! The FFTs run on the CPU every frame (one scoped thread per map) and their maps are uploaded
//! to the bathymetry texture: the shaders and the CPU's samplers (buoyancy,
//! contacts, boats) read the very same maps, with no readback from the GPU.
//!
//! Each map texel holds, per cascade, the horizontal displacement and the
//! height (`disp`: x, y, z, and ∂x/∂y of the displacement) and the slope
//! and the displacement's other derivatives (`slope`: ∂h/∂x, ∂h/∂y,
//! ∂Dx/∂x, ∂Dy/∂y), from which the folding of the crests (the Jacobian) is
//! worked out for the whitecaps.

use glam::{Vec2, Vec3};
use std::f32::consts::{PI, TAU};

/// Texels a side of a cascade's maps.
pub const OCEAN_N: usize = 128;
/// The cascades' tile sizes (m): long chop, and short ripples (an uneven
/// ratio, so their repetitions never line up).
pub const OCEAN_TILES: [f32; 2] = [192.0, 27.3];
/// The split between the cascades: waves longer than this (m) are the
/// first's, the others the second's.
const SPLIT: f32 = 10.0;
const G: f32 = 9.81;
/// How far the wind blows over open water for the spectrum by default (m):
/// a sea a region or two across, whose local fetch (water_shelter.rs)
/// scales it down further in bays and lakes. The Sea panel sets it.
pub const FETCH: f32 = 15_000.0;

/// The spectrum the wind sea follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Spectrum {
    /// A sea still growing over a limited fetch (the JONSWAP measurements
    /// of the North Sea): a sharp peak, the default.
    #[default]
    Jonswap,
    /// Tessendorf's fully developed open ocean.
    Phillips,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct C(f32, f32);

impl C {
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn conj(self) -> C {
        C(self.0, -self.1)
    }
    fn scale(self, s: f32) -> C {
        C(self.0 * s, self.1 * s)
    }
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    /// i·self
    fn i(self) -> C {
        C(-self.1, self.0)
    }
}

/// The maps of both cascades, cascade after cascade, each `OCEAN_N` rows of
/// `disp` then `OCEAN_N` rows of `slope` (see the module's comment).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OceanMaps {
    pub texels: Vec<[f32; 4]>,
    /// The significant wave height of the wind sea (m), for the depth's
    /// cap and the shelter's, and the scale its amplitudes were drawn at
    /// (the height is the scaled one).
    pub height: f32,
    pub scale: f32,
}

/// The ocean at a point (as `WaterOceanAt`): displacement (xyz), slope
/// (∂h/∂x, ∂h/∂y) and the displacement's derivatives (∂Dx/∂x, ∂Dx/∂y,
/// ∂Dy/∂y, the Gerstner trains' Jacobian order), summed over the cascades.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OceanAt {
    pub offset: Vec3,
    pub slope: Vec2,
    pub jacobian: Vec3,
}

impl OceanMaps {
    /// The texel (`row`, `col`) of cascade `c`'s maps (wrapping).
    fn texel(&self, c: usize, slope: bool, x: usize, y: usize) -> [f32; 4] {
        let row = c * 2 * OCEAN_N + usize::from(slope) * OCEAN_N + (y % OCEAN_N);
        self.texels.get(row * OCEAN_N + (x % OCEAN_N)).copied().unwrap_or([0.0; 4])
    }

    /// The ocean at world `p`, bilinear and wrapping, as `water_fx_ocean`.
    pub fn sample(&self, p: Vec2) -> OceanAt {
        if self.texels.len() != 4 * OCEAN_N * OCEAN_N {
            return OceanAt::default();
        }
        let mut at = OceanAt::default();
        for (c, tile) in OCEAN_TILES.iter().enumerate() {
            // (the FFT's texel i is the sea at i·tile/N)
            let pos = (p / tile).fract_gl() * OCEAN_N as f32;
            let base = pos.floor();
            let f = pos - base;
            let (x, y) = (
                (base.x as i32).rem_euclid(OCEAN_N as i32) as usize,
                (base.y as i32).rem_euclid(OCEAN_N as i32) as usize,
            );
            let mix = |slope: bool| {
                let t = |dx: usize, dy: usize| self.texel(c, slope, x + dx, y + dy);
                let (a, b, cc, d) = (t(0, 0), t(1, 0), t(0, 1), t(1, 1));
                std::array::from_fn::<f32, 4, _>(|k| {
                    let top = a[k] + (b[k] - a[k]) * f.x;
                    let bottom = cc[k] + (d[k] - cc[k]) * f.x;
                    top + (bottom - top) * f.y
                })
            };
            let d = mix(false);
            let s = mix(true);
            at.offset += Vec3::new(d[0], d[1], d[2]);
            at.slope += Vec2::new(s[0], s[1]);
            at.jacobian += Vec3::new(s[2], d[3], s[3]);
        }
        at
    }
}

/// The spectrum's state: its random amplitudes (fixed: the sea keeps its
/// face as the wind changes), the initial spectrum for the last wind, and
/// the maps.
#[derive(Debug, Clone)]
pub struct Ocean {
    /// Per cascade and wave vector: two unit Gaussian numbers.
    noise: Vec<[f32; 2]>,
    /// Per cascade and wave vector: the initial amplitude h0(k).
    h0: Vec<C>,
    /// The wind the spectrum was laid out for (toward, speed, spectrum,
    /// fetch).
    laid: Option<(Vec2, f32, Spectrum, f32)>,
    pub maps: std::sync::Arc<OceanMaps>,
}

impl Default for Ocean {
    fn default() -> Self {
        Self::new()
    }
}

/// A deterministic pair of unit Gaussian numbers for `seed` (Box-Muller).
fn gaussian(seed: u32) -> [f32; 2] {
    let hash = |mut h: u32| {
        h ^= h >> 16;
        h = h.wrapping_mul(0x7feb_352d);
        h ^= h >> 15;
        h = h.wrapping_mul(0x846c_a68b);
        h ^= h >> 16;
        (h >> 8) as f32 / 16_777_216.0
    };
    let u1 = hash(seed.wrapping_mul(2).wrapping_add(0x9e37_79b9)).max(1e-7);
    let u2 = hash(seed.wrapping_mul(2).wrapping_add(0x85eb_ca6b));
    let r = (-2.0 * u1.ln()).sqrt();
    [r * (TAU * u2).cos(), r * (TAU * u2).sin()]
}

/// The wave vector of texel (`x`, `y`) of a tile `tile` m wide.
fn wave(x: usize, y: usize, tile: f32) -> Vec2 {
    let n = OCEAN_N as i32;
    let fx = if (x as i32) < n / 2 { x as i32 } else { x as i32 - n };
    let fy = if (y as i32) < n / 2 { y as i32 } else { y as i32 - n };
    Vec2::new(fx as f32, fy as f32) * (TAU / tile)
}

impl Ocean {
    pub fn new() -> Self {
        let count = OCEAN_TILES.len() * OCEAN_N * OCEAN_N;
        Self {
            noise: (0..count as u32).map(gaussian).collect(),
            h0: vec![C::default(); count],
            laid: None,
            maps: std::sync::Arc::new(OceanMaps::default()),
        }
    }

    /// The spectrum's energy density per unit wave-vector area at `k` for
    /// a wind blowing toward `toward` at `speed` m/s.
    fn density(k: Vec2, toward: Vec2, speed: f32, spectrum: Spectrum, fetch: f32) -> f32 {
        let len = k.length();
        if len < 1e-4 || speed < 0.05 {
            return 0.0;
        }
        let cos = (k / len).dot(toward);
        // spread round the wind as cos², none against it
        let spread = if cos > 0.0 { cos * cos * 2.0 / PI } else { 0.0 };
        match spectrum {
            Spectrum::Jonswap => {
                let omega = (G * len).sqrt();
                // past about 2.2·10⁴ U²/g of fetch, the sea is fully
                // developed (Pierson-Moskowitz) and grows no more
                let fetch = fetch.min(2.2e4 * speed * speed / G);
                let peak = 22.0 * (G * G / (speed * fetch)).powf(1.0 / 3.0);
                let alpha = 0.076 * (speed * speed / (fetch * G)).powf(0.22);
                let sigma = if omega <= peak { 0.07 } else { 0.09 };
                let r = (-(omega - peak).powi(2) / (2.0 * sigma * sigma * peak * peak)).exp();
                let s = alpha * G * G / omega.powi(5) * (-1.25 * (peak / omega).powi(4)).exp() * 3.3f32.powf(r);
                // S(ω) dω over a ring of k: dω/dk / k
                s * (G / (2.0 * omega)) / len * spread
            }
            Spectrum::Phillips => {
                let l = speed * speed / G;
                let small = 0.001 * l;
                3e-3 * (-1.0 / (len * l).powi(2)).exp() / len.powi(4) * (-(len * small).powi(2)).exp() * spread / TAU
            }
        }
    }

    /// Lay the initial spectrum out for a wind blowing toward `toward`
    /// (unit) at `speed` over `fetch` m; its significant height.
    fn lay(&mut self, toward: Vec2, speed: f32, spectrum: Spectrum, fetch: f32) -> f32 {
        let mut variance = 0.0;
        for (c, tile) in OCEAN_TILES.iter().enumerate() {
            let dk = TAU / tile;
            for y in 0..OCEAN_N {
                for x in 0..OCEAN_N {
                    let k = wave(x, y, *tile);
                    let len = k.length();
                    // each wave length belongs to one cascade only
                    let mine = if c == 0 { len < TAU / SPLIT } else { len >= TAU / SPLIT };
                    let i = (c * OCEAN_N + y) * OCEAN_N + x;
                    let e = if mine {
                        Self::density(k, toward, speed, spectrum, fetch)
                    } else {
                        0.0
                    };
                    variance += e * dk * dk;
                    // h(k) gathers h0(k) and conj(h0(-k)): each carries a
                    // quarter of the cell's variance, two Gaussians apiece
                    let amp = (e * dk * dk * 0.25).sqrt();
                    let [a, b] = self.noise[i];
                    self.h0[i] = C(a * amp, b * amp);
                }
            }
        }
        4.0 * variance.sqrt()
    }

    /// The maps at `time` (s) for a wind blowing toward `toward` at
    /// `speed` m/s over `fetch` m; `chop` sharpens the crests (0 smooth,
    /// about 1 sharp), `scale` multiplies the heights (the Sea panel's,
    /// the height limit's).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        time: f64,
        toward: Vec2,
        speed: f32,
        spectrum: Spectrum,
        chop: f32,
        fetch: f32,
        scale: f32,
    ) -> std::sync::Arc<OceanMaps> {
        let toward = toward.try_normalize().unwrap_or(Vec2::X);
        let speed = speed.max(0.0);
        let fetch = fetch.clamp(100.0, 100_000.0);
        let scale = if scale.is_finite() { scale.max(0.0) } else { 1.0 };
        // laid out again when the wind has changed enough to see
        let raw = match self.laid {
            Some((t, s, k, f)) if k == spectrum && f == fetch && t.dot(toward) > 0.99995 && (s - speed).abs() < 0.01 * s.max(1.0) => {
                self.maps.height / self.maps.scale.max(1e-6)
            }
            _ => {
                self.laid = Some((toward, speed, spectrum, fetch));
                self.lay(toward, speed, spectrum, fetch)
            }
        };
        let height = raw * scale;
        let n = OCEAN_N;
        let h0 = &self.h0;
        // one job per cascade and packed pair of fields: the spectrum at
        // `time`, then its inverse FFT (real fields: A + iB comes back as
        // a + ib)
        let grids: Vec<Vec<C>> = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..OCEAN_TILES.len() * 4)
                .map(|job| scope.spawn(move || field(h0, job / 4, job % 4, time, chop, scale)))
                .collect();
            jobs.into_iter()
                .map(|j| j.join().unwrap_or_else(|_| vec![C::default(); n * n]))
                .collect()
        });
        let mut texels = vec![[0.0f32; 4]; OCEAN_TILES.len() * 2 * n * n];
        for (job, grid) in grids.iter().enumerate() {
            let (c, f) = (job / 4, job % 4);
            // fields 0, 1 fill `disp` (xy, zw), 2, 3 fill `slope`
            let rows = c * 2 * n + (f / 2) * n;
            let lane = (f % 2) * 2;
            for (i, v) in grid.iter().enumerate() {
                let t = &mut texels[rows * n + i];
                t[lane] = v.0;
                t[lane + 1] = v.1;
            }
        }
        self.maps = std::sync::Arc::new(OceanMaps { texels, height, scale });
        self.maps.clone()
    }
}

/// Packed pair `f` of cascade `c`'s fields at `time`, brought back to the
/// tile by an inverse FFT: 0 (Dx, Dy), 1 (h, ∂Dx/∂y), 2 (∂h/∂x, ∂h/∂y),
/// 3 (∂Dx/∂x, ∂Dy/∂y).
fn field(h0: &[C], c: usize, f: usize, time: f64, chop: f32, scale: f32) -> Vec<C> {
    let n = OCEAN_N;
    let tile = OCEAN_TILES[c];
    let mut grid: Vec<C> = (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            let k = wave(x, y, tile);
            let len = k.length();
            if len < 1e-6 {
                return C::default();
            }
            // h(k, t) = h0(k) e^{iωt} + conj(h0(-k)) e^{-iωt}, the phase
            // kept in f64 (hours of time)
            let omega = (G as f64 * len as f64).sqrt();
            let phase = ((omega * time) % std::f64::consts::TAU) as f32;
            let (s, co) = phase.sin_cos();
            let mirror = ((n - x) % n, (n - y) % n);
            let a = h0[(c * n + y) * n + x];
            let b = h0[(c * n + mirror.1) * n + mirror.0];
            let h = a.mul(C(co, s)).add(b.conj().mul(C(co, -s))).scale(scale);
            // displacement toward the crests, D = i k/|k| h (a train
            // cos(kx) moves by -sin(kx), as a Gerstner train): its
            // divergence folds the crests together, as the Gerstner
            // trains' (the Jacobian falls below 1 on them)
            let ih = h.i().scale(chop / len);
            match f {
                0 => ih.scale(k.x).add(ih.scale(k.y).i()),
                1 => h.add(h.scale(-k.x * k.y * chop / len).i()),
                2 => h.i().scale(k.x).add(h.i().scale(k.y).i()),
                // ∂Dx/∂x = i kx · i kx/|k| h = -kx²/|k| h
                _ => h.scale(-k.x * k.x * chop / len).add(h.scale(-k.y * k.y * chop / len).i()),
            }
        })
        .collect();
    inverse_fft_2d(&mut grid, n);
    grid
}

/// In-place inverse FFT of a power-of-two length (no 1/n: the spectrum's
/// amplitudes are already the waves').
fn inverse_fft(data: &mut [C]) {
    let n = data.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = TAU / len as f32;
        let step = C(angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let mut w = C(1.0, 0.0);
            for k in 0..len / 2 {
                let a = data[start + k];
                let b = data[start + k + len / 2].mul(w);
                data[start + k] = a.add(b);
                data[start + k + len / 2] = C(a.0 - b.0, a.1 - b.1);
                w = w.mul(step);
            }
        }
        len <<= 1;
    }
}

/// The inverse FFT of an `n`-square grid, rows then columns.
fn inverse_fft_2d(grid: &mut [C], n: usize) {
    grid.chunks_mut(n).for_each(inverse_fft);
    let mut columns: Vec<C> = vec![C::default(); n * n];
    for y in 0..n {
        for x in 0..n {
            columns[x * n + y] = grid[y * n + x];
        }
    }
    columns.chunks_mut(n).for_each(inverse_fft);
    for x in 0..n {
        for y in 0..n {
            grid[y * n + x] = columns[x * n + y];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inverse_fft_brings_a_single_wave_back() {
        let mut data = vec![C::default(); 16];
        data[3] = C(1.0, 0.0);
        inverse_fft(&mut data);
        for (x, v) in data.iter().enumerate() {
            let phase = TAU * 3.0 * x as f32 / 16.0;
            assert!((v.0 - phase.cos()).abs() < 1e-5 && (v.1 - phase.sin()).abs() < 1e-5, "{x}: {v:?}");
        }
    }

    #[test]
    fn the_sea_is_as_high_as_its_spectrum_says_and_grows_with_the_wind() {
        let mut ocean = Ocean::new();
        let calm = ocean.update(10.0, Vec2::X, 2.0, Spectrum::Jonswap, 0.8, FETCH, 1.0);
        let gale = ocean.update(10.0, Vec2::X, 18.0, Spectrum::Jonswap, 0.8, FETCH, 1.0);
        assert!(calm.height < 0.15 && gale.height > 0.8, "{} {}", calm.height, gale.height);
        // the heights in the maps match the spectrum's (Hs = 4σ), the
        // first cascade holding most of it
        let heights: Vec<f32> = gale.texels[..OCEAN_N * OCEAN_N].iter().map(|t| t[2]).collect();
        let mean = heights.iter().sum::<f32>() / heights.len() as f32;
        let sigma = (heights.iter().map(|h| (h - mean).powi(2)).sum::<f32>() / heights.len() as f32).sqrt();
        assert!(mean.abs() < 0.05, "{mean}");
        assert!(
            sigma * 4.0 > gale.height * 0.7 && sigma * 4.0 < gale.height * 1.15,
            "{} vs {}",
            sigma * 4.0,
            gale.height
        );
    }

    #[test]
    fn its_slope_is_its_height_s_gradient_and_its_crests_fold() {
        let mut ocean = Ocean::new();
        let maps = ocean.update(3.0, Vec2::new(0.8, 0.6), 14.0, Spectrum::Jonswap, 1.0, FETCH, 1.0);
        let h = 0.05;
        for p in [Vec2::new(3.3, 7.1), Vec2::new(-40.0, 12.5), Vec2::new(100.2, -66.0)] {
            let at = maps.sample(p);
            let dx = (maps.sample(p + Vec2::X * h).offset.z - maps.sample(p - Vec2::X * h).offset.z) / (2.0 * h);
            let dy = (maps.sample(p + Vec2::Y * h).offset.z - maps.sample(p - Vec2::Y * h).offset.z) / (2.0 * h);
            // (bilinear between texels: close, not exact)
            assert!(
                (at.slope.x - dx).abs() < 0.08 + 0.3 * dx.abs() && (at.slope.y - dy).abs() < 0.08 + 0.3 * dy.abs(),
                "{at:?} vs {dx} {dy}"
            );
        }
        // somewhere the crests fold together (the whitecaps' cue)
        let folds = (0..400)
            .map(|i| maps.sample(Vec2::new(i as f32 * 0.37, i as f32 * 0.53)))
            .filter(|a| (1.0 + a.jacobian.x) * (1.0 + a.jacobian.z) - a.jacobian.y * a.jacobian.y < 0.8)
            .count();
        assert!(folds > 0, "a gale's crests fold");
    }

    #[test]
    fn it_tiles_and_moves_on_in_time() {
        let mut ocean = Ocean::new();
        let a = ocean.update(5.0, Vec2::X, 10.0, Spectrum::Phillips, 0.8, FETCH, 1.0);
        let p = Vec2::new(12.0, 5.0);
        // the first cascade alone repeats every tile: the second's period
        // is 27.3 m, so 7 of the first's (1344 m) are 49.2 of the second's;
        // compare the cascades' maps directly instead
        let mut one = (*a).clone();
        one.texels[2 * OCEAN_N * OCEAN_N..].iter_mut().for_each(|t| *t = [0.0; 4]);
        let far = one.sample(p + Vec2::new(OCEAN_TILES[0], -2.0 * OCEAN_TILES[0]));
        let near = one.sample(p);
        assert!((near.offset - far.offset).length() < 1e-4, "{near:?} {far:?}");
        let b = ocean.update(5.5, Vec2::X, 10.0, Spectrum::Phillips, 0.8, FETCH, 1.0);
        assert_ne!(a.sample(p).offset, b.sample(p).offset);
        assert!(b.height > 0.0);
    }
}
