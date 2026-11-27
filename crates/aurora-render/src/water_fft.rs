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
//! worked out for the whitecaps; and the foam the folds leave (`foam`: x),
//! which lingers after the crest has passed, fading over some seconds and
//! drawn out downwind into streaks, as a sea's foam does.

use glam::{Vec2, Vec3};
use std::f32::consts::{PI, TAU};

/// Texels a side of a cascade's maps.
pub const OCEAN_N: usize = 128;
/// The cascades' tile sizes (m): the open sea's long chop and short
/// ripples (an uneven ratio, so their repetitions never line up), and the
/// lakes' chop (`LAKE`).
pub const OCEAN_TILES: [f32; 3] = [192.0, 27.3, 19.7];
/// The split between the open sea's cascades: waves longer than this (m)
/// are the first's, the others the second's.
const SPLIT: f32 = 10.0;
/// The lakes' cascade: a sea grown over `LAKE_FETCH` m only, all of it in
/// one tile. A lake's wind does not raise the open sea's waves smaller (its
/// long waves cannot grow over a few hundred metres): it raises short,
/// steep chop, its peak at 2 to 4.5 m from a breeze to a gale (a JONSWAP
/// sea limited by its fetch, whatever the panel's spectrum). Where the sea
/// grid finds a short fetch (water_shelter.rs `exposure`), the open sea's
/// cascades give way to it, scaled by that fetch (`ocean_weights`).
pub const LAKE: usize = 2;
pub const LAKE_FETCH: f32 = 800.0;

/// The angular frequency (rad/s) at the peak of a sea the wind raises at
/// `speed` m/s over `fetch` m (JONSWAP, Hasselmann et al. 1973: `22 (g² /
/// (U F))^(1/3)`). Past about 2.2·10⁴ U²/g of fetch the sea is fully
/// developed (Pierson-Moskowitz) and grows no more.
pub fn jonswap_peak(speed: f32, fetch: f32) -> f32 {
    let speed = speed.max(0.05);
    let fetch = fetch.min(2.2e4 * speed * speed / G).max(1.0);
    22.0 * (G * G / (speed * fetch)).powf(1.0 / 3.0)
}
const G: f32 = 9.81;
/// How long the foam a crest leaves lingers (s, its e-folding time).
const FOAM_LINGER: f32 = 4.5;
/// How fast lingering foam spreads along the wind (m²/s): a little, over
/// its life (at 0.9, with a high chop, it drew long tails behind crests).
const FOAM_STREAK: f32 = 0.3;
/// How long the foam of a breaking crest rides on with it before it is
/// left behind (s, its e-folding time): a short cap at the crest's tip (at
/// 1 s, carried at up to 10 m/s, it trailed some 10 m behind it).
pub const FOAM_RIDE: f32 = 0.4;

/// How fast a breaking crest of cascade `c` carries its foam downwind (m/s)
/// in a wind of `speed` m/s: about its waves' phase speed (the long
/// chop's breaking waves, some 10 to 60 m, run at 4 to 10 m/s; the short
/// cascade's, under 10 m, at under 4; the GPU's finest at under 1.5).
pub fn ride_speed(c: usize, speed: f32) -> f32 {
    match c {
        0 => (0.45 * speed).min(10.0),
        1 => (0.3 * speed).min(3.5),
        _ => (0.2 * speed).min(1.5),
    }
}
/// Map blocks per cascade: `disp`, `slope`, `foam`.
pub const OCEAN_BLOCKS: usize = 3;
/// The sea's loop (s): every wave's angular frequency is a multiple of
/// 2π/LOOP (by under 0.007 rad/s), so its phase is that of the time modulo
/// LOOP, small enough for the GPU's 32-bit floats after hours online, and
/// the CPU's and the GPU's maps keep the very same phases.
pub const OCEAN_LOOP: f64 = 1024.0;
/// The GPU's finer maps (water_ocean.wgsl): texels a side, and the tile of
/// the finest cascade (m), whose waves run from where the short cascade's
/// stop (0.43 m) down to some 4 cm.
pub const FINE_N: usize = 256;
pub const FINE_TILE: f32 = 5.151;
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
/// `disp`, `OCEAN_N` rows of `slope`, then `OCEAN_N` rows of `foam` (see
/// the module's comment).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OceanMaps {
    pub texels: Vec<[f32; 4]>,
    /// The significant wave height of the wind sea (m), for the depth's
    /// cap and the shelter's, and the scale its amplitudes were drawn at
    /// (the height is the scaled one).
    pub height: f32,
    pub scale: f32,
    /// The lakes' cascade's significant height (m, scaled as `height`):
    /// the chop of a lake `LAKE_FETCH` across in this wind.
    pub lake_height: f32,
    /// What the GPU's finer maps take (water_ocean_gpu.rs): their initial
    /// spectrum, and the time (modulo `OCEAN_LOOP`), chop and wind of
    /// these maps.
    pub fine: Option<std::sync::Arc<FineSpectrum>>,
    pub fine_time: f64,
    pub chop: f32,
    pub toward: [f32; 2],
    /// The wind's speed (m/s), for the foam riding with the crests.
    pub speed: f32,
}

/// The GPU's finer maps' initial spectrum: per cascade (the short one at
/// `FINE_N`, the same waves as the CPU's, and the finest, `FINE_TILE`) and
/// texel, h0(k) and conj(h0(-k)). `generation` changes with each layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FineSpectrum {
    pub texels: Vec<[f32; 4]>,
    pub generation: u64,
}

/// The angular frequency of wave number `len` (deep water, with surface
/// tension: ω² = g k (1 + k² L²), L² = σ / (ρ g); Tessendorf, "Simulating
/// Ocean Water", 2004, eq. 33: centimetre ripples run some 8 % faster), on
/// the loop's lattice (`OCEAN_LOOP`). water_ocean.wgsl's `omega` is the
/// same.
pub fn omega(len: f32) -> f64 {
    let step = std::f64::consts::TAU / OCEAN_LOOP;
    let k = len as f64;
    ((G as f64 * k * (1.0 + k * k * TENSION_L2)).sqrt() / step).floor() * step
}

/// Water's capillary length squared (m²): its surface tension over its
/// density and gravity (0.0728 N/m / (1000 kg/m³ · 9.81 m/s²)).
const TENSION_L2: f64 = 7.42e-6;

/// The ocean at a point (as `WaterOceanAt`): displacement (xyz), slope
/// (∂h/∂x, ∂h/∂y) and the displacement's derivatives (∂Dx/∂x, ∂Dx/∂y,
/// ∂Dy/∂y, the Gerstner trains' Jacobian order), summed over the cascades.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OceanAt {
    pub offset: Vec3,
    pub slope: Vec2,
    pub jacobian: Vec3,
    /// The crests' folding now or lately (1 - the Jacobian's
    /// determinant, held as it fades), the lingering foam's cue.
    pub foam: f32,
}

impl OceanMaps {
    /// The texel (`x`, `y`) of map `block` (0 `disp`, 1 `slope`, 2 `foam`)
    /// of cascade `c` (wrapping).
    fn block(&self, c: usize, block: usize, x: usize, y: usize) -> [f32; 4] {
        let row = (c * OCEAN_BLOCKS + block) * OCEAN_N + (y % OCEAN_N);
        self.texels.get(row * OCEAN_N + (x % OCEAN_N)).copied().unwrap_or([0.0; 4])
    }

    /// The open sea at world `p` (its two cascades, no lake), bilinear and
    /// wrapping.
    pub fn sample(&self, p: Vec2) -> OceanAt {
        self.sample_weighted(p, [1.0, 1.0, 0.0])
    }

    /// The wind sea at world `p`, each cascade taken by its `weights` (the
    /// open sea's two, the lakes': `ocean_weights`), bilinear and
    /// wrapping, as `water_fx_ocean`.
    pub fn sample_weighted(&self, p: Vec2, weights: [f32; 3]) -> OceanAt {
        if self.texels.len() != OCEAN_TILES.len() * OCEAN_BLOCKS * OCEAN_N * OCEAN_N {
            return OceanAt::default();
        }
        let mut at = OceanAt::default();
        for (c, tile) in OCEAN_TILES.iter().enumerate() {
            let w = weights[c];
            if w <= 0.0 {
                continue;
            }
            // (the FFT's texel i is the sea at i·tile/N)
            let pos = (p / tile).fract_gl() * OCEAN_N as f32;
            let base = pos.floor();
            let f = pos - base;
            let (x, y) = (
                (base.x as i32).rem_euclid(OCEAN_N as i32) as usize,
                (base.y as i32).rem_euclid(OCEAN_N as i32) as usize,
            );
            let mix = |block: usize| {
                let t = |dx: usize, dy: usize| self.block(c, block, x + dx, y + dy);
                let (a, b, cc, d) = (t(0, 0), t(1, 0), t(0, 1), t(1, 1));
                std::array::from_fn::<f32, 4, _>(|k| {
                    let top = a[k] + (b[k] - a[k]) * f.x;
                    let bottom = cc[k] + (d[k] - cc[k]) * f.x;
                    top + (bottom - top) * f.y
                })
            };
            let d = mix(0);
            let s = mix(1);
            at.offset += Vec3::new(d[0], d[1], d[2]) * w;
            at.slope += Vec2::new(s[0], s[1]) * w;
            at.jacobian += Vec3::new(s[2], d[3], s[3]) * w;
            at.foam = at.foam.max(mix(2)[0] * w);
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
    /// Per cascade and texel: the lingering foam, and when it was last
    /// worked out.
    foam: Vec<f32>,
    /// The same, for the foam riding on with the crests (`FOAM_RIDE`).
    ride: Vec<f32>,
    foam_time: Option<f64>,
    fine: std::sync::Arc<FineSpectrum>,
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
    wave_n(x, y, tile, OCEAN_N)
}

/// The same on a grid `n` texels a side.
fn wave_n(x: usize, y: usize, tile: f32, n: usize) -> Vec2 {
    let n = n as i32;
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
            foam: vec![0.0; count],
            ride: vec![0.0; count],
            foam_time: None,
            fine: std::sync::Arc::default(),
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
                let fetch = fetch.min(2.2e4 * speed * speed / G);
                let peak = jonswap_peak(speed, fetch);
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
    /// (unit) at `speed` over `fetch` m; the open sea's significant height
    /// and the lakes' (`LAKE`).
    fn lay(&mut self, toward: Vec2, speed: f32, spectrum: Spectrum, fetch: f32) -> (f32, f32) {
        let (mut variance, mut lake) = (0.0, 0.0);
        for (c, tile) in OCEAN_TILES.iter().enumerate() {
            let dk = TAU / tile;
            for y in 0..OCEAN_N {
                for x in 0..OCEAN_N {
                    let k = wave(x, y, *tile);
                    let len = k.length();
                    // each wave length of the open sea belongs to one of its
                    // cascades only; the lakes' takes them all
                    let mine = match c {
                        0 => len < TAU / SPLIT,
                        LAKE => true,
                        _ => len >= TAU / SPLIT,
                    };
                    let i = (c * OCEAN_N + y) * OCEAN_N + x;
                    let e = match (mine, c) {
                        (false, _) => 0.0,
                        (true, LAKE) => Self::density(k, toward, speed, Spectrum::Jonswap, LAKE_FETCH),
                        (true, _) => Self::density(k, toward, speed, spectrum, fetch),
                    };
                    if c == LAKE {
                        lake += e * dk * dk;
                    } else {
                        variance += e * dk * dk;
                    }
                    // h(k) gathers h0(k) and conj(h0(-k)): each carries a
                    // quarter of the cell's variance, two Gaussians apiece
                    let amp = (e * dk * dk * 0.25).sqrt();
                    let [a, b] = self.noise[i];
                    // (none on the Nyquist row and column: their -k is
                    // themselves, which the GPU's finer grid does not share)
                    let nyquist = x == OCEAN_N / 2 || y == OCEAN_N / 2;
                    self.h0[i] = if nyquist { C::default() } else { C(a * amp, b * amp) };
                }
            }
        }
        self.lay_fine(toward, speed, spectrum, fetch);
        (4.0 * variance.sqrt(), 4.0 * lake.sqrt())
    }

    /// The GPU's finer spectrum (`FineSpectrum`): the short cascade's own
    /// waves on a grid `FINE_N` a side (zero past them, so its surface is
    /// the CPU's, sampled finer), and the finest cascade's waves, those
    /// past the short cascade's band.
    fn lay_fine(&mut self, toward: Vec2, speed: f32, spectrum: Spectrum, fetch: f32) {
        let (n, fine) = (FINE_N as i32, FINE_N);
        let half = OCEAN_N as i32 / 2;
        let mut h0 = vec![C::default(); 2 * fine * fine];
        for y in 0..fine {
            for x in 0..fine {
                let fx = if (x as i32) < n / 2 { x as i32 } else { x as i32 - n };
                let fy = if (y as i32) < n / 2 { y as i32 } else { y as i32 - n };
                // the short cascade's, from its CPU grid
                if fx.abs() < half && fy.abs() < half {
                    let (cx, cy) = (fx.rem_euclid(OCEAN_N as i32) as usize, fy.rem_euclid(OCEAN_N as i32) as usize);
                    h0[y * fine + x] = self.h0[(OCEAN_N + cy) * OCEAN_N + cx];
                }
                // the finest: what lies past the short cascade's band
                let k = wave_n(x, y, FINE_TILE, fine);
                let band = half as f32 * TAU / OCEAN_TILES[1];
                if (fx.abs() < n / 2 && fy.abs() < n / 2) && k.x.abs().max(k.y.abs()) >= band {
                    let dk = TAU / FINE_TILE;
                    let e = Self::density(k, toward, speed, spectrum, fetch);
                    let amp = (e * dk * dk * 0.25).sqrt();
                    let [a, b] = gaussian(0x4000_0000 + (y * fine + x) as u32);
                    h0[fine * fine + y * fine + x] = C(a * amp, b * amp);
                }
            }
        }
        let mirror = |c: usize, x: usize, y: usize| h0[c * fine * fine + ((fine - y) % fine) * fine + (fine - x) % fine];
        let texels = (0..2 * fine * fine)
            .map(|i| {
                let (c, x, y) = (i / (fine * fine), i % fine, (i / fine) % fine);
                let (a, b) = (h0[i], mirror(c, x, y).conj());
                [a.0, a.1, b.0, b.1]
            })
            .collect();
        self.fine = std::sync::Arc::new(FineSpectrum {
            texels,
            generation: self.fine.generation.wrapping_add(1),
        });
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
        let (raw, lake_raw) = match self.laid {
            Some((t, s, k, f)) if k == spectrum && f == fetch && t.dot(toward) > 0.99995 && (s - speed).abs() < 0.01 * s.max(1.0) => (
                self.maps.height / self.maps.scale.max(1e-6),
                self.maps.lake_height / self.maps.scale.max(1e-6),
            ),
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
        let mut texels = vec![[0.0f32; 4]; OCEAN_TILES.len() * OCEAN_BLOCKS * n * n];
        for (job, grid) in grids.iter().enumerate() {
            let (c, f) = (job / 4, job % 4);
            // fields 0, 1 fill `disp` (xy, zw), 2, 3 fill `slope`
            let rows = c * OCEAN_BLOCKS * n + (f / 2) * n;
            let lane = (f % 2) * 2;
            for (i, v) in grid.iter().enumerate() {
                let t = &mut texels[rows * n + i];
                t[lane] = v.0;
                t[lane + 1] = v.1;
            }
        }
        self.linger(&mut texels, time, toward, speed);
        self.maps = std::sync::Arc::new(OceanMaps {
            texels,
            height,
            scale,
            lake_height: lake_raw * scale,
            fine: Some(self.fine.clone()),
            fine_time: time.rem_euclid(OCEAN_LOOP),
            chop,
            toward: toward.to_array(),
            speed,
        });
        self.maps.clone()
    }
}

impl Ocean {
    /// The foam the crests leave: where they fold now (1 - the Jacobian's
    /// determinant, from `texels`' `disp` and `slope`), or the foam left
    /// there before, fading over `FOAM_LINGER` and spread along the wind
    /// (`toward`), into `texels`' `foam` maps. The tiles are fixed in the
    /// world, so foam left on a tile's texel stays where the crest broke
    /// while the crest moves on; and a share of it rides on with the crest
    /// for a moment (carried downwind at the crest's speed, `ride_speed`
    /// in a wind of `speed`, fading over `FOAM_RIDE`): a breaking crest
    /// keeps a cap of foam at its tip that trails off behind it.
    fn linger(&mut self, texels: &mut [[f32; 4]], time: f64, toward: Vec2, speed: f32) {
        let n = OCEAN_N;
        let dt = match self.foam_time {
            // (a jump in time: the sea starts afresh)
            Some(before) if time >= before && time - before < 1.0 => (time - before) as f32,
            _ => {
                self.foam.iter_mut().for_each(|f| *f = 0.0);
                self.ride.iter_mut().for_each(|f| *f = 0.0);
                0.0
            }
        };
        self.foam_time = Some(time);
        let fade = (-dt / FOAM_LINGER).exp();
        let ride_fade = (-dt / FOAM_RIDE).exp();
        let mut spread = vec![0.0f32; n * n];
        let mut carried = vec![0.0f32; n * n];
        for (c, tile) in OCEAN_TILES.iter().enumerate() {
            let texel = tile / n as f32;
            // the streaking, explicit diffusion along the wind (kept under
            // its stability limit)
            let k = (FOAM_STREAK * dt / (texel * texel)).min(0.45);
            let foam = &mut self.foam[c * n * n..(c + 1) * n * n];
            let ride = &mut self.ride[c * n * n..(c + 1) * n * n];
            // (a lake's chop, 2 to 4.5 m long, runs at under 2.7 m/s)
            let rides = if c == LAKE { (0.15 * speed).min(2.7) } else { ride_speed(c, speed) };
            let shift = toward * rides * dt / texel;
            let at = |foam: &[f32], x: f32, y: f32| {
                let (bx, by) = (x.floor(), y.floor());
                let (fx, fy) = (x - bx, y - by);
                let i = |dx: i32, dy: i32| {
                    let xi = (bx as i32 + dx).rem_euclid(n as i32) as usize;
                    let yi = (by as i32 + dy).rem_euclid(n as i32) as usize;
                    foam[yi * n + xi]
                };
                let top = i(0, 0) + (i(1, 0) - i(0, 0)) * fx;
                let bottom = i(0, 1) + (i(1, 1) - i(0, 1)) * fx;
                top + (bottom - top) * fy
            };
            for y in 0..n {
                for x in 0..n {
                    let (xf, yf) = (x as f32, y as f32);
                    let here = foam[y * n + x];
                    let ahead = at(foam, xf + toward.x, yf + toward.y);
                    let behind = at(foam, xf - toward.x, yf - toward.y);
                    spread[y * n + x] = here + k * (ahead + behind - 2.0 * here);
                    carried[y * n + x] = at(ride, xf - shift.x, yf - shift.y);
                }
            }
            let base = c * OCEAN_BLOCKS * n * n;
            for (i, f) in foam.iter_mut().enumerate() {
                let d = texels[base + i];
                let s = texels[base + n * n + i];
                let fold = 1.0 - ((1.0 + s[2]) * (1.0 + s[3]) - d[3] * d[3]);
                *f = fold.max(spread[i] * fade).max(0.0);
                ride[i] = fold.max(carried[i] * ride_fade).max(0.0);
                texels[base + 2 * n * n + i] = [f.max(ride[i]), 0.0, 0.0, 0.0];
            }
        }
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
            // h(k, t) = h0(k) e^{-iωt} + conj(h0(-k)) e^{iωt}: with the
            // field brought back as Σ h(k) e^{ik·x}, a wave of vector k then
            // travels along k, downwind (with Tessendorf's e^{+iωt} it went
            // upwind, against everything the wind drives), the phase
            // kept in f64 (hours of time)
            let phase = ((omega(len) * time.rem_euclid(OCEAN_LOOP)) % std::f64::consts::TAU) as f32;
            let (s, co) = phase.sin_cos();
            let mirror = ((n - x) % n, (n - y) % n);
            let a = h0[(c * n + y) * n + x];
            let b = h0[(c * n + mirror.1) * n + mirror.0];
            let h = a.mul(C(co, -s)).add(b.conj().mul(C(co, s))).scale(scale);
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

    /// The lakes' cascade is a fetch-limited sea: in a gale, decimetres of
    /// short, steep chop (a lake 800 m across: Hs some 0.29 m at 20 m/s by
    /// the fetch-limited growth, its waves 2 to 4.5 m), shorter than the
    /// open sea's short cascade; and the open sea's sampler leaves it out.
    #[test]
    fn a_lake_s_chop_is_short_and_steep_and_apart_from_the_open_sea() {
        let mut ocean = Ocean::new();
        let gale = ocean.update(10.0, Vec2::X, 20.0, Spectrum::Jonswap, 1.0, FETCH, 1.0);
        assert!(gale.lake_height > 0.15 && gale.lake_height < 0.6, "{}", gale.lake_height);
        assert!(gale.lake_height < 0.2 * gale.height, "{} vs {}", gale.lake_height, gale.height);
        // a wave's length as its heights' RMS over its slopes' (2π / k)
        let length = |weights: [f32; 3]| {
            let (mut h, mut s) = (0.0, 0.0);
            for i in 0..2000 {
                let at = gale.sample_weighted(Vec2::new(i as f32 * 0.37, i as f32 * 0.91), weights);
                h += at.offset.z * at.offset.z;
                s += at.slope.length_squared();
            }
            TAU * (h / s.max(1e-9)).sqrt()
        };
        let lake = length([0.0, 0.0, 1.0]);
        let short = length([0.0, 1.0, 0.0]);
        assert!(lake < 6.0 && lake < short, "lake {lake} m, the open sea's short cascade {short} m");
        // the open sea's sampler is the two cascades alone
        let p = Vec2::new(12.3, -4.5);
        assert_eq!(gale.sample(p), gale.sample_weighted(p, [1.0, 1.0, 0.0]));
        // a lighter wind, lower chop
        let breeze = ocean.update(10.0, Vec2::X, 8.0, Spectrum::Jonswap, 1.0, FETCH, 1.0);
        assert!(breeze.lake_height < 0.5 * gale.lake_height, "{}", breeze.lake_height);
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
    fn the_foam_of_a_fold_lingers_then_fades_and_streaks_downwind() {
        let mut ocean = Ocean::new();
        let toward = Vec2::X;
        ocean.update(10.0, toward, 16.0, Spectrum::Jonswap, 1.0, FETCH, 1.0);
        let n = OCEAN_N;
        let mut texels = vec![[0.0f32; 4]; OCEAN_TILES.len() * OCEAN_BLOCKS * n * n];
        // a single fold on the long cascade: ∂Dx/∂x = -0.5
        texels[n * n + 40 * n + 40][2] = -0.5;
        ocean.foam_time = None;
        ocean.linger(&mut texels, 0.0, toward, 16.0);
        let foam = |t: &[[f32; 4]], x: usize, y: usize| t[2 * n * n + y * n + x][0];
        assert!((foam(&texels, 40, 40) - 0.5).abs() < 1e-5);
        // the crest moves on: the foam stays, fades, and spreads along x
        texels[n * n + 40 * n + 40][2] = 0.0;
        let mut t = 0.0;
        for _ in 0..60 {
            t += 1.0 / 30.0;
            ocean.linger(&mut texels, t, toward, 16.0);
        }
        // (a fold one texel wide spreads fast: its peak falls by both)
        let left = foam(&texels, 40, 40);
        assert!(left > 0.05 && left < 0.5 * (-2.0f32 / FOAM_LINGER).exp(), "{left}");
        assert!(foam(&texels, 41, 40) > 0.02, "downwind");
        assert!(foam(&texels, 40, 41) < 1e-4, "not across the wind");
        // and is gone some seconds later
        for _ in 0..900 {
            t += 1.0 / 30.0;
            ocean.linger(&mut texels, t, toward, 16.0);
        }
        assert!(foam(&texels, 40, 40) < 0.01);
    }

    #[test]
    fn a_breaking_crest_carries_its_foam_on_for_a_moment() {
        let mut ocean = Ocean::new();
        ocean.update(10.0, Vec2::X, 16.0, Spectrum::Jonswap, 1.0, FETCH, 1.0);
        let n = OCEAN_N;
        let mut texels = vec![[0.0f32; 4]; OCEAN_TILES.len() * OCEAN_BLOCKS * n * n];
        texels[n * n + 40 * n + 40][2] = -0.5;
        ocean.foam_time = None;
        ocean.linger(&mut texels, 0.0, Vec2::X, 16.0);
        texels[n * n + 40 * n + 40][2] = 0.0;
        let mut t = 0.0;
        for _ in 0..8 {
            t += 1.0 / 30.0;
            ocean.linger(&mut texels, t, Vec2::X, 16.0);
        }
        // a quarter second on, at 7.2 m/s over 1.5 m texels: some 1.3
        // texels downwind there is more foam than as far upwind
        let foam = |x: usize| texels[2 * n * n + 40 * n + x][0];
        assert!(foam(41) > foam(39) * 1.5 && foam(41) > 0.05, "{} {}", foam(41), foam(39));
    }

    #[test]
    fn the_waves_travel_downwind() {
        // a quarter of a second on, the heights along the wind line up best
        // shifted downwind (some 1 m at 12 m/s: the waves' phase speed)
        for toward in [Vec2::X, Vec2::new(-0.6, 0.8)] {
            let mut ocean = Ocean::new();
            let side = Vec2::new(-toward.y, toward.x) * 7.0;
            let line = |m: &OceanMaps| {
                (0..384)
                    .map(|i| m.sample(side + toward * (i as f32 * 0.5)).offset.z)
                    .collect::<Vec<f32>>()
            };
            let a = line(&ocean.update(50.0, toward, 12.0, Spectrum::Jonswap, 0.0, FETCH, 1.0));
            let b = line(&ocean.update(50.25, toward, 12.0, Spectrum::Jonswap, 0.0, FETCH, 1.0));
            let fit = |s: i32| (40..344).map(|i| a[i as usize] * b[(i + s) as usize]).sum::<f32>();
            let best = (-20i32..=20).max_by(|&x, &y| fit(x).total_cmp(&fit(y))).unwrap_or(0);
            assert!(best > 0, "the waves went {} m against the wind", -best as f32 * 0.5);
        }
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
        one.texels[OCEAN_BLOCKS * OCEAN_N * OCEAN_N..]
            .iter_mut()
            .for_each(|t| *t = [0.0; 4]);
        let far = one.sample(p + Vec2::new(OCEAN_TILES[0], -2.0 * OCEAN_TILES[0]));
        let near = one.sample(p);
        assert!((near.offset - far.offset).length() < 1e-4, "{near:?} {far:?}");
        let b = ocean.update(5.5, Vec2::X, 10.0, Spectrum::Phillips, 0.8, FETCH, 1.0);
        assert_ne!(a.sample(p).offset, b.sample(p).offset);
        assert!(b.height > 0.0);
    }
}
