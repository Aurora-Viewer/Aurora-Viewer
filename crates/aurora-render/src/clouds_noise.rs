//! The volumetric clouds' noise textures as data: a CPU port of the
//! generators in shaders/clouds.wgsl (`gen_base`, `gen_detail`, `gen_curl`),
//! the file they are shipped in (`assets/clouds/cloud_noise.bin`, made by
//! `cargo run -p aurora-render --example generate_cloud_noise`) and its
//! reader, so that the clouds are drawn from the first frame instead of after
//! 16 frames of generation, and with the same bits on every GPU and backend.
//!
//! The recipe is the one the shader has always had: periodic Perlin and
//! Worley fractal noise, Perlin dilated by Worley (Schneider & Vos, SIGGRAPH
//! 2015), with the octaves and weights of Hillaire's SIGGRAPH 2016 noise
//! generator (sebh/TileableVolumeNoise, MIT, Copyright (c) 2016 Sebastien
//! Hillaire): Worley fractal noise at scales 1, 2 and 4 with weights 0.625,
//! 0.25 and 0.125. Only f32 sums, products, `floor` and `sqrt` are used, which
//! are exact and ordered the same on every CPU, so the file is the same
//! everywhere (a test pins its hash). The shader's generators stay as the
//! fallback when the file cannot be read.
//!
//! File: magic `AURCLNZ1`, then little-endian u32 words: version, base size,
//! detail size, curl size, unpacked length, packed length, CRC-32 of the
//! unpacked bytes; then the zlib stream. Unpacked: the planes one after the
//! other (base R, G, B, A of `BASE_SIZE`^3; detail R, G, B of `DETAIL_SIZE`^3;
//! curl R, G of `CURL_SIZE`^2), each as the differences of neighbouring
//! texels along x (wrapping), which are small for the smooth channels and
//! compress better.

use std::io::{Read, Write};

pub const BASE_SIZE: u32 = 128;
pub const DETAIL_SIZE: u32 = 32;
pub const CURL_SIZE: u32 = 128;
const MAGIC: &[u8; 8] = b"AURCLNZ1";
const VERSION: u32 = 1;
const HEADER: usize = 8 + 7 * 4;

/// The shipped file, built into the viewer like the stars' tables.
static SHIPPED: &[u8] = include_bytes!("../../../assets/clouds/cloud_noise.bin");

/// The three textures as the GPU takes them (`Rgba8Unorm`, x then y then z).
pub struct Noise {
    pub base: Vec<u8>,
    pub detail: Vec<u8>,
    pub curl: Vec<u8>,
}

// ---------------------------------------------------------------- generator (port of clouds.wgsl)
type V3 = [f32; 3];

fn hash_u(x: u32) -> u32 {
    let v = x.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((v >> ((v >> 28) + 4)) ^ v).wrapping_mul(277803737);
    (w >> 22) ^ w
}

fn hash3(c: [i32; 3]) -> u32 {
    hash_u(c[0] as u32 ^ hash_u(c[1] as u32 ^ hash_u((c[2] as u32).wrapping_add(0x9e37_79b9))))
}

fn wrap3(c: [i32; 3], p: i32) -> [i32; 3] {
    c.map(|v| v.rem_euclid(p))
}

fn rand3(c: [i32; 3]) -> V3 {
    let h = hash3(c);
    [
        (h & 1023) as f32 / 1023.0,
        ((h >> 10) & 1023) as f32 / 1023.0,
        ((h >> 20) & 1023) as f32 / 1023.0,
    ]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

fn grad3(c: [i32; 3], f: V3) -> f32 {
    let r = rand3(c);
    let g = [r[0] * 2.0 - 1.0 + 1e-4, r[1] * 2.0 - 1.0 + 1e-4, r[2] * 2.0 - 1.0 + 1e-4];
    let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
    (g[0] / len) * f[0] + (g[1] / len) * f[1] + (g[2] / len) * f[2]
}

/// Periodic gradient noise, about [0, 1].
fn perlin3(p: V3, period: i32) -> f32 {
    let i = p.map(|v| v.floor());
    let f = [p[0] - i[0], p[1] - i[1], p[2] - i[2]];
    let i = i.map(|v| v as i32);
    let w = f.map(|f| f * f * f * (f * (f * 6.0 - 15.0) + 10.0));
    let n = |dx: i32, dy: i32, dz: i32| {
        grad3(
            wrap3([i[0] + dx, i[1] + dy, i[2] + dz], period),
            [f[0] - dx as f32, f[1] - dy as f32, f[2] - dz as f32],
        )
    };
    let x0 = mix(mix(n(0, 0, 0), n(1, 0, 0), w[0]), mix(n(0, 1, 0), n(1, 1, 0), w[0]), w[1]);
    let x1 = mix(mix(n(0, 0, 1), n(1, 0, 1), w[0]), mix(n(0, 1, 1), n(1, 1, 1), w[0]), w[1]);
    mix(x0, x1, w[2]) * 0.6 + 0.5
}

/// Periodic cellular noise: 1 at the feature points.
fn worley3(p: V3, period: i32) -> f32 {
    let i = p.map(|v| v.floor());
    let f = [p[0] - i[0], p[1] - i[1], p[2] - i[2]];
    let i = i.map(|v| v as i32);
    let mut d = 1e9_f32;
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                let r = rand3(wrap3([i[0] + x, i[1] + y, i[2] + z], period));
                let fp = [x as f32 + r[0] - f[0], y as f32 + r[1] - f[1], z as f32 + r[2] - f[2]];
                d = d.min(fp[0] * fp[0] + fp[1] * fp[1] + fp[2] * fp[2]);
            }
        }
    }
    1.0 - d.sqrt().clamp(0.0, 1.0)
}

fn scale(p: V3, s: f32) -> V3 {
    p.map(|v| v * s)
}

fn worley_fbm(p: V3, f: i32) -> f32 {
    worley3(scale(p, f as f32), f) * 0.625
        + worley3(scale(p, (2 * f) as f32), 2 * f) * 0.25
        + worley3(scale(p, (4 * f) as f32), 4 * f) * 0.125
}

fn unorm(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// One layer of the base noise (`BASE_SIZE`^2 texels of 4 bytes: Perlin-Worley
/// and the Worley fractal noises of 4, 8 and 16 cells).
pub fn base_layer(z: u32) -> Vec<u8> {
    let n = BASE_SIZE as f32;
    let mut out = Vec::with_capacity((BASE_SIZE * BASE_SIZE * 4) as usize);
    for y in 0..BASE_SIZE {
        for x in 0..BASE_SIZE {
            let p = [(x as f32 + 0.5) / n, (y as f32 + 0.5) / n, (z as f32 + 0.5) / n];
            let (mut pf, mut amp, mut tot, mut fr) = (0.0_f32, 1.0_f32, 0.0_f32, 4_i32);
            for _ in 0..4 {
                pf += amp * perlin3(scale(p, fr as f32), fr);
                tot += amp;
                amp *= 0.5;
                fr *= 2;
            }
            let pf = ((pf / tot - 0.5) * 1.8 + 0.5).clamp(0.0, 1.0);
            let (w0, w1, w2) = (worley_fbm(p, 4), worley_fbm(p, 8), worley_fbm(p, 16));
            // Perlin dilated by the Worley cells, billowy and connected
            let pw = ((pf - (w0 - 1.0)) / (1.0 - (w0 - 1.0))).clamp(0.0, 1.0);
            out.extend([unorm(pw), unorm(w0), unorm(w1), unorm(w2)]);
        }
    }
    out
}

/// The detail noise (`DETAIL_SIZE`^3 texels of 4 bytes: Worley fractal noises
/// of 2, 4 and 8 cells, alpha 1).
pub fn detail() -> Vec<u8> {
    let n = DETAIL_SIZE as f32;
    let mut out = Vec::with_capacity((DETAIL_SIZE.pow(3) * 4) as usize);
    for z in 0..DETAIL_SIZE {
        for y in 0..DETAIL_SIZE {
            for x in 0..DETAIL_SIZE {
                let p = [(x as f32 + 0.5) / n, (y as f32 + 0.5) / n, (z as f32 + 0.5) / n];
                out.extend([unorm(worley_fbm(p, 2)), unorm(worley_fbm(p, 4)), unorm(worley_fbm(p, 8)), 255]);
            }
        }
    }
    out
}

/// The curl noise (`CURL_SIZE`^2 texels of 4 bytes: the curl of a Perlin
/// field in R and G, B 0, alpha 1).
pub fn curl() -> Vec<u8> {
    let n = CURL_SIZE as f32;
    let e = 0.05_f32;
    let mut out = Vec::with_capacity((CURL_SIZE * CURL_SIZE * 4) as usize);
    for y in 0..CURL_SIZE {
        for x in 0..CURL_SIZE {
            let (px, py) = ((x as f32 + 0.5) / n * 8.0, (y as f32 + 0.5) / n * 8.0);
            let at = |dx: f32, dy: f32| perlin3([px + dx, py + dy, 0.5], 8);
            let dx = at(e, 0.0) - at(-e, 0.0);
            let dy = at(0.0, e) - at(0.0, -e);
            let c = [dy / (2.0 * e), -dx / (2.0 * e)];
            out.extend([unorm(c[0] * 0.5 + 0.5), unorm(c[1] * 0.5 + 0.5), 0, 255]);
        }
    }
    out
}

// ---------------------------------------------------------------- file
fn word(h: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([h[8 + i * 4], h[9 + i * 4], h[10 + i * 4], h[11 + i * 4]])
}

fn planes(texels: &[u8], channels: usize, raw: &mut Vec<u8>) {
    for c in 0..channels {
        let mut last = 0u8;
        raw.extend(texels.as_chunks::<4>().0.iter().map(|t| {
            let d = t[c].wrapping_sub(last);
            last = t[c];
            d
        }));
    }
}

/// The file's bytes for the three textures (what the generator writes).
pub fn pack(noise: &Noise) -> Vec<u8> {
    let mut raw = Vec::new();
    planes(&noise.base, 4, &mut raw);
    planes(&noise.detail, 3, &mut raw);
    planes(&noise.curl, 2, &mut raw);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    // (writing to a Vec cannot fail)
    let _ = z.write_all(&raw);
    let packed = z.finish().unwrap_or_default();
    let mut crc = flate2::Crc::new();
    crc.update(&raw);
    let mut out = Vec::with_capacity(HEADER + packed.len());
    out.extend(MAGIC);
    for w in [
        VERSION,
        BASE_SIZE,
        DETAIL_SIZE,
        CURL_SIZE,
        raw.len() as u32,
        packed.len() as u32,
        crc.sum(),
    ] {
        out.extend(w.to_le_bytes());
    }
    out.extend(packed);
    out
}

/// Reads a file back, refusing a wrong magic, version or size, a truncated
/// or damaged stream and a failed checksum.
pub fn parse(bytes: &[u8]) -> Result<Noise, String> {
    let h = bytes.get(..HEADER).ok_or("cloud noise: too short")?;
    if &h[..8] != MAGIC {
        return Err("cloud noise: bad magic".into());
    }
    if word(h, 0) != VERSION {
        return Err(format!("cloud noise: version {}, not {VERSION}", word(h, 0)));
    }
    if (word(h, 1), word(h, 2), word(h, 3)) != (BASE_SIZE, DETAIL_SIZE, CURL_SIZE) {
        return Err("cloud noise: not the shader's sizes".into());
    }
    let (b, d, c) = (BASE_SIZE as usize, DETAIL_SIZE as usize, CURL_SIZE as usize);
    let (nb, nd, nc) = (b * b * b, d * d * d, c * c);
    let expected = nb * 4 + nd * 3 + nc * 2;
    if word(h, 4) as usize != expected {
        return Err("cloud noise: unexpected length".into());
    }
    let packed = bytes.get(HEADER..HEADER + word(h, 5) as usize).ok_or("cloud noise: truncated")?;
    let mut raw = Vec::with_capacity(expected);
    // (one byte more than expected is read, so that a longer stream is seen)
    flate2::read::ZlibDecoder::new(packed)
        .take(expected as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("cloud noise: {e}"))?;
    if raw.len() != expected {
        return Err(format!("cloud noise: {} bytes unpacked", raw.len()));
    }
    let mut crc = flate2::Crc::new();
    crc.update(&raw);
    if crc.sum() != word(h, 6) {
        return Err("cloud noise: checksum failed".into());
    }
    // texels of 4 bytes from `channels` delta planes, the rest `fill`
    let unplane = |raw: &[u8], n: usize, channels: usize, fill: &[u8]| {
        let mut out = vec![0u8; n * 4];
        for (ch, plane) in raw.chunks_exact(n).take(channels).enumerate() {
            let mut last = 0u8;
            for (i, d) in plane.iter().enumerate() {
                last = last.wrapping_add(*d);
                out[i * 4 + ch] = last;
            }
        }
        for t in out.as_chunks_mut::<4>().0 {
            t[channels..].copy_from_slice(fill);
        }
        out
    };
    let (rb, rest) = raw.split_at(nb * 4);
    let (rd, rc) = rest.split_at(nd * 3);
    Ok(Noise {
        base: unplane(rb, nb, 4, &[]),
        detail: unplane(rd, nd, 3, &[255]),
        curl: unplane(rc, nc, 2, &[0, 255]),
    })
}

/// The noise to upload: the shipped file, or the file `AURORA_CLOUD_NOISE`
/// names (a test of the fallback: a missing or damaged file gives an error
/// and the shader generates the noise instead).
pub fn load() -> Result<Noise, String> {
    match std::env::var_os("AURORA_CLOUD_NOISE") {
        Some(path) => parse(&std::fs::read(&path).map_err(|e| format!("cloud noise {}: {e}", path.to_string_lossy()))?),
        None => parse(SHIPPED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayon::prelude::*;

    fn generated() -> Noise {
        Noise {
            base: (0..BASE_SIZE).into_par_iter().flat_map_iter(base_layer).collect(),
            detail: detail(),
            curl: curl(),
        }
    }

    fn err(bytes: &[u8]) -> String {
        parse(bytes).err().expect("a refused file")
    }

    fn fnv(bytes: &[u8]) -> u64 {
        bytes
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
    }

    /// Patterned texels of the shader's sizes, in the layout `parse` returns.
    fn patterned() -> Noise {
        let fill = |n: usize, fill: &[u8]| -> Vec<u8> {
            let colours = 4 - fill.len();
            (0..n)
                .flat_map(|i| {
                    let mut t = [(i * 7) as u8, (i / 3) as u8, (i % 251) as u8, (i / 97) as u8];
                    t[colours..].copy_from_slice(fill);
                    t
                })
                .collect()
        };
        Noise {
            base: fill((BASE_SIZE as usize).pow(3), &[]),
            detail: fill((DETAIL_SIZE as usize).pow(3), &[255]),
            curl: fill((CURL_SIZE as usize).pow(2), &[0, 255]),
        }
    }

    #[test]
    fn a_file_reads_back_as_written() {
        let n = patterned();
        let back = parse(&pack(&n)).expect("round trip");
        assert!(back.base == n.base && back.detail == n.detail && back.curl == n.curl);
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let good = pack(&patterned());
        assert!(parse(&good[..10]).is_err(), "too short");
        assert!(parse(&good[..good.len() - 40]).is_err(), "truncated");
        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(err(&bad).contains("magic"));
        let mut bad = good.clone();
        bad[8] = 9;
        assert!(err(&bad).contains("version"));
        let mut bad = good.clone();
        bad[12] = 64;
        assert!(err(&bad).contains("sizes"));
        let mut bad = good.clone();
        bad[HEADER + 100] ^= 0x55;
        assert!(parse(&bad).is_err(), "a flipped byte in the stream");
        let mut bad = good.clone();
        bad[8 + 6 * 4] ^= 1;
        assert!(err(&bad).contains("checksum"));
    }

    #[test]
    fn a_missing_file_gives_an_error_for_the_fallback() {
        // SAFETY: no other test reads or writes the variable.
        unsafe { std::env::set_var("AURORA_CLOUD_NOISE", "does-not-exist.bin") };
        let r = load();
        unsafe { std::env::remove_var("AURORA_CLOUD_NOISE") };
        assert!(r.is_err());
    }

    #[test]
    fn the_generator_is_deterministic_and_matches_the_shipped_file() {
        let n = generated();
        assert_eq!(fnv(&n.detail), fnv(&detail()));
        assert_eq!(fnv(&n.curl), fnv(&curl()));
        assert_eq!(fnv(&n.base[..(BASE_SIZE * BASE_SIZE * 4) as usize]), fnv(&base_layer(0)));
        let shipped = parse(SHIPPED).expect("the shipped file");
        assert!(
            shipped.base == n.base && shipped.detail == n.detail && shipped.curl == n.curl,
            "assets/clouds/cloud_noise.bin is out of date: run the generator (docs/TESTING.md)"
        );
        assert_eq!(
            (fnv(&n.base), fnv(&n.detail), fnv(&n.curl)),
            PINNED,
            "the generator's output changed: run it again and update PINNED"
        );
    }

    /// FNV-1a of the base, detail and curl textures.
    const PINNED: (u64, u64, u64) = (3_124_285_001_996_590_873, 4_472_950_822_113_885_471, 9_906_133_218_407_151_246);
}
