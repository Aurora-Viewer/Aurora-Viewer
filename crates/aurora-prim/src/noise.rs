//! Port of the Second Life viewer's lattice noise (indra/newview/noise.h,
//! noise.cpp, originally LGPL 2.1), used by the terrain composition and the
//! bark of the Linden trees. The gradient and permutation tables come from
//! `srand(42)` + `rand()`, so the results only match the Legacy Viewer's if
//! the same pseudo-random sequence is reproduced: the Legacy Viewer for
//! Windows (the reference) is built with MSVC, whose `rand()` is a 32-bit
//! LCG returning bits 16..30. All arithmetic is in f32 in the same order as
//! the C code (built with /fp:precise, no FMA).

use std::sync::OnceLock;

const B: usize = 0x100;
const BM: i32 = 0xff;
const NF32: f32 = 4096.0;

struct Tables {
    p: [usize; B + B + 2],
    g2: [[f32; 2]; B + B + 2],
    g3: [[f32; 3]; B + B + 2],
}

/// MSVC CRT `rand()`: holdrand = holdrand * 214013 + 2531011,
/// result (holdrand >> 16) & 0x7fff.
pub struct MsvcRand(u32);

impl MsvcRand {
    pub fn new(seed: u32) -> MsvcRand {
        MsvcRand(seed)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((self.0 >> 16) & 0x7fff) as i32
    }
}

/// noise.h `init`: g1, g2 and g3 are drawn interleaved for each lattice
/// point (1 + 2 + 3 calls), then the permutation is shuffled. g1 (noise1)
/// is unused here but still consumes its draw.
fn init() -> Tables {
    let mut r = MsvcRand::new(42);
    let mut t = Tables {
        p: [0; B + B + 2],
        g2: [[0.0; 2]; B + B + 2],
        g3: [[0.0; 3]; B + B + 2],
    };
    let draw = |r: &mut MsvcRand| ((r.next() % (B + B) as i32) - B as i32) as f32 / B as f32;
    for i in 0..B {
        t.p[i] = i;
        let _g1 = draw(&mut r);
        let v = [draw(&mut r), draw(&mut r)];
        // normalize2 / normalize3 (never a zero vector with this seed, see tests)
        let s = 1.0 / (v[0] * v[0] + v[1] * v[1]).sqrt();
        t.g2[i] = [v[0] * s, v[1] * s];
        let v = [draw(&mut r), draw(&mut r), draw(&mut r)];
        let s = 1.0 / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        t.g3[i] = [v[0] * s, v[1] * s, v[2] * s];
    }
    // while (--i) { k = p[i]; p[i] = p[j = rand() % B]; p[j] = k; }
    for i in (1..B).rev() {
        let j = (r.next() % B as i32) as usize;
        t.p.swap(i, j);
    }
    for i in 0..B + 2 {
        t.p[B + i] = t.p[i];
        t.g2[B + i] = t.g2[i];
        t.g3[B + i] = t.g3[i];
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

/// noise.h `noise3`: signed 3-D gradient noise.
pub fn noise3(v: [f32; 3]) -> f32 {
    let t = tables();
    let (bx0, bx1, rx0, rx1) = fast_setup(v[0]);
    let (by0, by1, ry0, ry1) = fast_setup(v[1]);
    let (bz0, bz1, rz0, rz1) = fast_setup(v[2]);
    let i = t.p[bx0];
    let j = t.p[bx1];
    let b00 = t.p[i + by0];
    let b10 = t.p[j + by0];
    let b01 = t.p[i + by1];
    let b11 = t.p[j + by1];
    let sx = s_curve(rx0);
    let sy = s_curve(ry0);
    let sz = s_curve(rz0);
    // fast_at3
    let at3 = |rx: f32, ry: f32, rz: f32, q: [f32; 3]| rx * q[0] + ry * q[1] + rz * q[2];
    let a = lerp(sx, at3(rx0, ry0, rz0, t.g3[b00 + bz0]), at3(rx1, ry0, rz0, t.g3[b10 + bz0]));
    let b = lerp(sx, at3(rx0, ry1, rz0, t.g3[b01 + bz0]), at3(rx1, ry1, rz0, t.g3[b11 + bz0]));
    let c = lerp(sy, a, b);
    let a = lerp(sx, at3(rx0, ry0, rz1, t.g3[b00 + bz1]), at3(rx1, ry0, rz1, t.g3[b10 + bz1]));
    let b = lerp(sx, at3(rx0, ry1, rz1, t.g3[b01 + bz1]), at3(rx1, ry1, rz1, t.g3[b11 + bz1]));
    let d = lerp(sy, a, b);
    lerp(sz, c, d)
}

/// noise.h `turbulence3`: the 3-D octaves of `turbulence2`.
pub fn turbulence3(v: [f32; 3], mut freq: f32) -> f32 {
    let mut t = 0.0;
    while freq >= 1.0 {
        t += noise3([freq * v[0], freq * v[1], freq * v[2]]) / freq;
        freq *= 0.5;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from the Legacy Viewer's noise.cpp / noise.h
    /// compiled with MSVC 2022 (/O2 /fp:precise /arch:AVX2, the CRT's
    /// rand()), the same toolchain as its Windows build.
    #[test]
    fn noise_matches_the_legacy_viewer() {
        let close = |a: f32, b: f32| (a - b).abs() <= 1e-6;
        for (x, y, n, t) in [
            (0.0, 0.0, 0.0, 0.0),
            (0.5, 0.25, -0.356_576_26, -0.409_460_78),
            (1.37, 2.11, 0.197_173_73, 0.061_257_884),
            (10.3, -4.7, 0.099_117_63, -0.060_509_764),
            (123.456, 78.9, 0.125_369_07, 0.200_200_22),
            (52012.2, 52043.7, -0.132_383_69, -0.441_523_5),
        ] {
            let got = noise2(x, y);
            assert!(close(got, n), "noise2({x}, {y}) = {got}, Legacy Viewer {n}");
            let got = turbulence2(x, y, 2.0);
            assert!(close(got, t), "turbulence2({x}, {y}) = {got}, Legacy Viewer {t}");
        }
    }

    #[test]
    fn msvc_rand_sequence() {
        // srand(42); rand() x 3 with the MSVC CRT
        let mut r = MsvcRand::new(42);
        assert_eq!([r.next(), r.next(), r.next()], [175, 400, 17869]);
    }

    #[test]
    fn noise3_is_a_bounded_lattice_noise() {
        // zero on the lattice, continuous, within the gradient noise range
        assert_eq!(noise3([0.0, 0.0, 0.0]), 0.0);
        assert_eq!(noise3([3.0, -2.0, 7.0]), 0.0);
        let mut max = 0.0f32;
        let mut sum = 0.0f64;
        let n = 40;
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    let v = noise3([i as f32 * 0.37 + 0.1, j as f32 * 0.29 + 0.2, k as f32 * 0.41 + 0.3]);
                    max = max.max(v.abs());
                    sum += v as f64;
                }
            }
        }
        assert!(max < 1.1 && max > 0.3, "max {max}");
        assert!((sum / (n * n * n) as f64).abs() < 0.05);
        let a = noise3([1.25, 2.5, 3.75]);
        let b = noise3([1.2501, 2.5, 3.75]);
        assert!((a - b).abs() < 1e-3);
        // turbulence3 at 5: octaves 5, 2.5 and 1.25
        let v = [0.3f32, 0.6, 0.9];
        let at = |f: f32| noise3([f * v[0], f * v[1], f * v[2]]) / f;
        let want = at(5.0) + at(2.5) + at(1.25);
        assert_eq!(turbulence3(v, 5.0), want);
    }
}
