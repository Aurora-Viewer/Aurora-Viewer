//! Night sky: the stars the sky shader draws (`sky_stars` in env.wgsl).
//!
//! Two catalogues share one storage buffer (`StarSky` in env.wgsl):
//!
//! - the real sky: the stars of `assets/stars/stars.bin` (HYG database,
//!   CC BY-SA 4.0, magnitude 6.5 and brighter), in J2000 equatorial
//!   coordinates, with their colour from the B-V index;
//! - the Legacy Viewer's stars, a port of LLVOWLSky::initStars
//!   (indra/newview/llvowlsky.cpp, originally LGPL 2.1): 1,000 random stars
//!   on the upper half of the dome, in the region's frame.
//!
//! Each catalogue is sorted into the cells of a cube map (`FACE_CELLS` per
//! face edge). A star also goes into the neighbouring cells its point can
//! cover (`MARGIN`), so a sky pixel only reads the stars of its own cell. The
//! sky shader turns the view direction into the catalogue's frame with the
//! frame's `sky_stars` rotation (`StarFrame`), filled by the viewer.

use glam::Vec3;

/// Cells per edge of a cube face. Must match `STAR_FACE_CELLS` (env.wgsl).
pub const FACE_CELLS: u32 = 24;
/// Cells of one catalogue.
pub const CELLS: u32 = 6 * FACE_CELLS * FACE_CELLS;
/// Length of one catalogue's cell table (`CELLS + 1` offsets, padded to 16
/// bytes). Must match `STAR_CELL_TABLE` (env.wgsl).
pub const CELL_TABLE: u32 = (CELLS + 1).div_ceil(4) * 4;
/// Catalogue index of the real sky, then of the Legacy Viewer's stars.
pub const REAL: u32 = 0;
pub const LEGACY: u32 = 1;
const CATALOGUES: u32 = 2;
/// Angular radius (radians) of the widest star point the shader draws: a
/// star is also listed in every cell within it. Wide enough for a whole
/// three-sigma point even in the reflection probes' irradiance capture
/// (128 pixels for 90°); there, only the brightest stars' points and glows
/// are cut. Must match `STAR_MARGIN`.
pub const MARGIN: f32 = 0.025;

/// The star table, built into the viewer (`scripts/generate-star-catalog.py`).
static CATALOGUE: &[u8] = include_bytes!("../../../assets/stars/stars.bin");
const MAGIC: &[u8; 8] = b"AURSTAR1";
const NO_COLOUR: i16 = 32767;

/// One star as the shader reads it (`GpuStar` in env.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuStar {
    /// Unit direction in the catalogue's frame, peak intensity (linear).
    pub pos: [f32; 4],
    /// Linear colour (luminance 1), size of the point in pixels (sigma).
    pub color: [f32; 4],
}

/// A star of the table, decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatalogueStar {
    /// Right ascension and declination (radians, J2000).
    pub ra: f32,
    pub dec: f32,
    pub magnitude: f32,
    /// B-V colour index, `None` when the catalogue has none.
    pub bv: Option<f32>,
}

/// Decodes `stars.bin` (format in scripts/generate-star-catalog.py).
pub fn parse(bytes: &[u8]) -> Result<Vec<CatalogueStar>, String> {
    let header = bytes.get(..12).ok_or("star table: too short")?;
    if &header[..8] != MAGIC {
        return Err("star table: bad magic".into());
    }
    let count = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    let body = &bytes[12..];
    if body.len() != count * 8 {
        return Err(format!("star table: {} bytes for {count} stars", body.len()));
    }
    Ok(body
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            let ra = u16::from_le_bytes([c[0], c[1]]);
            let dec = i16::from_le_bytes([c[2], c[3]]);
            let mag = i16::from_le_bytes([c[4], c[5]]);
            let bv = i16::from_le_bytes([c[6], c[7]]);
            CatalogueStar {
                ra: ra as f32 / 65536.0 * std::f32::consts::TAU,
                dec: (dec as f32 / 32767.0 * 90.0).to_radians(),
                magnitude: mag as f32 / 100.0,
                bv: (bv != NO_COLOUR).then_some(bv as f32 / 1000.0),
            }
        })
        .collect())
}

/// Unit vector of an equatorial position: x towards the March equinox,
/// z towards the north celestial pole.
pub fn equatorial(ra: f32, dec: f32) -> Vec3 {
    Vec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin())
}

/// Peak intensity (linear) of a star of this visual magnitude. The real
/// range (Sirius is 1,500 times brighter than a magnitude 6.5 star) is
/// compressed to about 1 to 100, so that the faintest stars just show over
/// the night sky and the constellations' bright stars stand out, as the eye
/// sees them.
pub fn intensity(magnitude: f32) -> f32 {
    (2.0 * 10f32.powf(-0.4 * 0.75 * magnitude)).min(3.0)
}

/// Size of a star's point, in pixels (sigma of its Gaussian): the brightest
/// stars are a little larger.
pub fn point_size(magnitude: f32) -> f32 {
    0.6 + 0.6 * ((3.0 - magnitude) / 4.0).clamp(0.0, 1.0)
}

/// Colour (linear, luminance 1) of a star of this B-V index. Temperature
/// from Ballesteros (2012), black body colour from Tanner Helland's fit,
/// then half desaturated: the eye sees little colour in starlight.
pub fn colour(bv: Option<f32>) -> Vec3 {
    let Some(bv) = bv else {
        return Vec3::ONE;
    };
    let bv = bv.clamp(-0.4, 2.0);
    let t = 4600.0 * (1.0 / (0.92 * bv + 1.7) + 1.0 / (0.92 * bv + 0.62));
    let t = t / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.7 * (t - 60.0).powf(-0.1332) };
    let g = if t <= 66.0 {
        99.47 * t.ln() - 161.12
    } else {
        288.12 * (t - 60.0).powf(-0.0755)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.52 * (t - 10.0).ln() - 305.04
    };
    let srgb = Vec3::new(r, g, b).clamp(Vec3::ZERO, Vec3::splat(255.0)) / 255.0;
    let linear = srgb.powf(2.2);
    let luma = linear.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-4);
    let c = linear / luma;
    (c * 0.5 + Vec3::splat(0.5)).min(Vec3::splat(2.0))
}

/// The real sky's stars, brightest first.
pub fn real_stars() -> Vec<GpuStar> {
    let stars = match parse(CATALOGUE) {
        Ok(s) => s,
        Err(e) => {
            log::error!("{e}: the night sky has no stars");
            Vec::new()
        }
    };
    stars
        .iter()
        .map(|s| {
            let d = equatorial(s.ra, s.dec);
            let c = colour(s.bv);
            GpuStar {
                pos: [d.x, d.y, d.z, intensity(s.magnitude)],
                color: [c.x, c.y, c.z, point_size(s.magnitude)],
            }
        })
        .collect()
}

/// The Legacy Viewer's 1,000 stars (LLVOWLSky::initStars): a random
/// direction on the upper half of the dome, an intensity
/// `min(rand² + 0.1, 1)` and a colour between white and pale green
/// (`r, b` in 0.75..1, `g` 1). Their random generator is not
/// reproducible; this one is seeded so that the sky is the same each run.
pub fn legacy_stars() -> Vec<GpuStar> {
    let mut seed: u32 = 0x5EED_57A5;
    let mut frand = move || {
        // xorshift32
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    (0..1000)
        .map(|_| {
            let d = Vec3::new(frand() - 0.5, frand() - 0.5, frand() / 2.0).normalize_or(Vec3::Z);
            let i = (frand().powi(2) + 0.1).min(1.0);
            let c = Vec3::new(0.75 + frand() * 0.25, 1.0, 0.75 + frand() * 0.25);
            GpuStar {
                pos: [d.x, d.y, d.z, i * 1.6],
                color: [c.x, c.y, c.z, 0.75],
            }
        })
        .collect()
}

/// Cell of a direction (any length). Mirrors `star_cell` in env.wgsl.
pub fn cell_of(q: Vec3) -> u32 {
    let a = q.abs();
    let (face, u, v, m) = if a.x >= a.y && a.x >= a.z {
        (if q.x >= 0.0 { 0 } else { 1 }, q.y, q.z, a.x)
    } else if a.y >= a.z {
        (if q.y >= 0.0 { 2 } else { 3 }, q.x, q.z, a.y)
    } else {
        (if q.z >= 0.0 { 4 } else { 5 }, q.x, q.y, a.z)
    };
    let n = FACE_CELLS as f32;
    let m = m.max(1e-20);
    let cx = ((u / m * 0.5 + 0.5) * n).floor().clamp(0.0, n - 1.0) as u32;
    let cy = ((v / m * 0.5 + 0.5) * n).floor().clamp(0.0, n - 1.0) as u32;
    face * FACE_CELLS * FACE_CELLS + cy * FACE_CELLS + cx
}

/// Cells a star's point can reach: its own and those within `MARGIN`.
fn cells_around(d: Vec3, out: &mut Vec<u32>) {
    out.clear();
    out.push(cell_of(d));
    let side = if d.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let e1 = d.cross(side).normalize();
    let e2 = d.cross(e1);
    // rings out to a little past the margin, closer than a cell's corner
    // can slip between two samples
    for ring in [0.25, 0.5, 0.75, 1.0, 1.08] {
        for k in 0..32 {
            let a = k as f32 / 32.0 * std::f32::consts::TAU;
            let p = d + (e1 * a.cos() + e2 * a.sin()) * (MARGIN * ring);
            let c = cell_of(p);
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
}

/// Sorts one catalogue into its cells: the offsets (`CELLS + 1`, padded to
/// `CELL_TABLE`, relative to `base`) and the stars, repeated in each cell
/// they reach.
fn bin(stars: &[GpuStar], base: u32) -> (Vec<u32>, Vec<GpuStar>) {
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); CELLS as usize];
    let mut around = Vec::new();
    for (i, s) in stars.iter().enumerate() {
        cells_around(Vec3::new(s.pos[0], s.pos[1], s.pos[2]), &mut around);
        for &c in &around {
            lists[c as usize].push(i as u32);
        }
    }
    let mut table = Vec::with_capacity(CELL_TABLE as usize);
    let mut out = Vec::new();
    for list in &lists {
        table.push(base + out.len() as u32);
        out.extend(list.iter().map(|&i| stars[i as usize]));
    }
    table.push(base + out.len() as u32);
    table.resize(CELL_TABLE as usize, base + out.len() as u32);
    (table, out)
}

/// Contents of the star buffer: both cell tables, then the stars.
pub fn buffer_contents() -> Vec<u8> {
    let (real_table, real) = bin(&real_stars(), 0);
    let (legacy_table, legacy) = bin(&legacy_stars(), real.len() as u32);
    let mut bytes = Vec::with_capacity((CATALOGUES * CELL_TABLE) as usize * 4 + (real.len() + legacy.len()) * 32);
    bytes.extend_from_slice(bytemuck::cast_slice(&real_table));
    bytes.extend_from_slice(bytemuck::cast_slice(&legacy_table));
    bytes.extend_from_slice(bytemuck::cast_slice(&real));
    bytes.extend_from_slice(bytemuck::cast_slice(&legacy));
    bytes
}

/// What the sky shader needs to place the stars, for one frame
/// (`sky_stars` of the frame uniforms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarFrame {
    /// Which catalogue (`REAL`, `LEGACY`), `None`: no stars.
    pub catalogue: Option<u32>,
    /// Columns of the rotation from the catalogue's frame to the region's
    /// (x east, y north, z up): the region directions of the catalogue's
    /// x, y and z axes.
    pub axes: [Vec3; 3],
    /// Strength of the Milky Way (0: none).
    pub milky_way: f32,
}

impl Default for StarFrame {
    fn default() -> Self {
        StarFrame {
            catalogue: None,
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
            milky_way: 0.0,
        }
    }
}

impl StarFrame {
    /// The three `vec4` of the frame uniforms: the axes, then in w the
    /// catalogue (-1: none), the Milky Way and 0.
    pub fn uniform(&self) -> [[f32; 4]; 3] {
        let cat = self.catalogue.map_or(-1.0, |c| c as f32);
        [
            self.axes[0].extend(cat).to_array(),
            self.axes[1].extend(self.milky_way).to_array(),
            self.axes[2].extend(0.0).to_array(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_the_naked_eye_sky_brightest_first() {
        let stars = parse(CATALOGUE).expect("built-in table");
        assert!((8000..10000).contains(&stars.len()), "{} stars", stars.len());
        // Sirius: RA 6h45m, Dec -16.7°, magnitude -1.46
        let sirius = stars[0];
        assert!((sirius.ra.to_degrees() - 101.29).abs() < 0.1, "{sirius:?}");
        assert!((sirius.dec.to_degrees() + 16.72).abs() < 0.1, "{sirius:?}");
        assert!(sirius.magnitude < -1.4);
        assert!(stars.windows(2).all(|w| w[0].magnitude <= w[1].magnitude));
        assert!(stars.iter().all(|s| s.magnitude <= 6.5));
        // Polaris sits within a degree of the pole
        assert!(stars.iter().any(|s| s.magnitude < 2.1 && s.dec.to_degrees() > 89.0));
    }

    #[test]
    fn a_damaged_table_is_refused() {
        assert!(parse(b"AURSTAR1").is_err());
        assert!(parse(b"NOTSTARS\x01\0\0\0abcdefgh").is_err());
        assert!(parse(b"AURSTAR1\x02\0\0\0abcdefgh").is_err());
        assert_eq!(parse(b"AURSTAR1\0\0\0\0").map(|s| s.len()), Ok(0));
    }

    #[test]
    fn colours_go_from_blue_to_orange() {
        let rigel = colour(Some(-0.03));
        let sun = colour(Some(0.65));
        let betelgeuse = colour(Some(1.85));
        assert!(rigel.z > rigel.x, "{rigel}");
        assert!(betelgeuse.x > betelgeuse.z, "{betelgeuse}");
        assert!(sun.x > sun.z && sun.x - sun.z < betelgeuse.x - betelgeuse.z);
        assert_eq!(colour(None), Vec3::ONE);
    }

    #[test]
    fn brighter_stars_are_brighter_and_larger() {
        assert!(intensity(-1.46) > intensity(1.0));
        assert!(intensity(1.0) > intensity(6.5));
        assert!(intensity(6.5) > 0.015);
        assert!(point_size(-1.0) > point_size(5.0));
    }

    #[test]
    fn every_star_is_found_from_any_pixel_of_its_point() {
        let stars = real_stars();
        let (table, binned) = bin(&stars, 0);
        let cell_has = |q: Vec3, s: &GpuStar| {
            let c = cell_of(q) as usize;
            binned[table[c] as usize..table[c + 1] as usize].contains(s)
        };
        for s in stars.iter().step_by(7) {
            let d = Vec3::new(s.pos[0], s.pos[1], s.pos[2]);
            let e1 = d.any_orthonormal_vector();
            let e2 = d.cross(e1);
            for k in 0..24 {
                let a = k as f32 / 24.0 * std::f32::consts::TAU;
                for r in [0.0, 0.3, 0.6, 0.95] {
                    let q = (d + (e1 * a.cos() + e2 * a.sin()) * MARGIN * r).normalize();
                    assert!(cell_has(q, s), "star {d} missed at {q}");
                }
            }
        }
    }

    #[test]
    fn the_cell_tables_cover_the_stars_in_order() {
        let bytes = buffer_contents();
        let words: &[u32] = bytemuck::cast_slice(&bytes[..(2 * CELL_TABLE * 4) as usize]);
        let stars = (bytes.len() - words.len() * 4) / 32;
        let real = &words[..CELL_TABLE as usize];
        let legacy = &words[CELL_TABLE as usize..];
        assert_eq!(real[0], 0);
        assert!(real.windows(2).all(|w| w[0] <= w[1]));
        assert!(legacy.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(legacy[0], real[CELLS as usize]);
        assert_eq!(legacy[CELLS as usize] as usize, stars);
        // the stars start on a 16-byte boundary, as WGSL lays out `StarSky`
        assert_eq!(words.len() * 4 % 16, 0);
    }

    #[test]
    fn legacy_stars_stay_on_the_upper_half_of_the_dome() {
        let stars = legacy_stars();
        assert_eq!(stars.len(), 1000);
        assert!(stars.iter().all(|s| s.pos[2] >= 0.0 && s.pos[3] > 0.0));
        assert_eq!(stars, legacy_stars());
    }
}
