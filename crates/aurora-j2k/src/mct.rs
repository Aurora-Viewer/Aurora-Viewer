// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/mct.h`:
//   Copyright (c) 2026, Anne Skydancer (neo-j2k)
// OpenJPEG, through it:
//   Copyright (c) 2002-2014, Universite catholique de Louvain (UCL), Belgium
//   Copyright (c) 2002-2014, Professor Benoit Macq
//   Copyright (c) 2001-2003, David Janssens
//   Copyright (c) 2002-2003, Yannick Verschueren
//   Copyright (c) 2003-2007, Francois-Olivier Devaux
//   Copyright (c) 2003-2014, Antonin Descampe
//   Copyright (c) 2005, Herve Drolon, FreeImage Team
//   Copyright (c) 2006-2007, Parvatha Elangovan
//   Copyright (c) 2008, 2011-2012, Centre National d'Etudes Spatiales (CNES), FR
//   Copyright (c) 2012, CS Systemes d'Information, France
//   Copyright (c) 2017, IntoPIX SA <support@intopix.com>
//   All rights reserved.

//! The end of a tile's decode after the inverse wavelet: the inverse colour
//! transforms (RCT, ICT), the DC level shift with its rounding and clamping,
//! and the copy into the image.
//!
//! After neo-j2k's `src/mct.h`, `src/mct.cpp` and `src/output.cpp`
//! (BSD-2-Clause, Copyright (c) 2026 Anne Skydancer) and OpenJPEG 2.5.4's
//! `mct.c` (`opj_mct_decode`, `opj_mct_decode_real`) and `tcd.c`
//! (`opj_tcd_dc_level_shift_decode`; BSD-2-Clause, see NOTICE.md): the same
//! operations per sample, so the samples are OpenJPEG's. As in neo-j2k the
//! three are fused: each row of a tile goes through the colour transform,
//! the shift and the store in one pass, a block of samples at a time.
//!
//! One departure, neo-j2k's DP-2, at the edges of the 9/7 shift: a sample
//! of 2^31 or more gives the maximum, NaN and anything below -2^31 the
//! minimum, on every platform (OpenJPEG's MSVC build rounds 2^31 to
//! INT_MIN, its Linux build to 2^31).

/// The colour transform a tile's first three components still need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mct {
    #[default]
    None,
    /// Reversible (with the 5/3).
    Rct,
    /// Irreversible (with the 9/7), on floats.
    Ict,
}

/// A component's DC shift: `shift` added, the result clamped to `[lo, hi]`;
/// float samples (9/7, `!reversible`) rounded to nearest, ties to even,
/// first.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Finish {
    pub shift: i32,
    pub lo: i32,
    pub hi: i32,
    pub reversible: bool,
}

/// Samples a pass works on at once: small enough to stay in the first
/// cache with four components.
const BLOCK: usize = 256;

#[inline(always)]
fn fb(x: i32) -> f32 {
    f32::from_bits(x as u32)
}

#[inline(always)]
fn bf(x: f32) -> i32 {
    x.to_bits() as i32
}

/// `opj_mct_decode` on a row, int32 wrap-around.
#[inline(always)]
fn rct_rows(c0: &mut [i32], c1: &mut [i32], c2: &mut [i32]) {
    for ((y, u), v) in c0.iter_mut().zip(c1.iter_mut()).zip(c2.iter_mut()) {
        let g = y.wrapping_sub(u.wrapping_add(*v) >> 2);
        let r = v.wrapping_add(g);
        let b = u.wrapping_add(g);
        *y = r;
        *u = g;
        *v = b;
    }
}

/// `opj_mct_decode_real` on a row (float bits in int32).
#[inline(always)]
fn ict_rows(c0: &mut [i32], c1: &mut [i32], c2: &mut [i32]) {
    for ((c0, c1), c2) in c0.iter_mut().zip(c1.iter_mut()).zip(c2.iter_mut()) {
        let (y, u, v) = (fb(*c0), fb(*c1), fb(*c2));
        let r = y + (v * 1.402);
        let g = y - (u * 0.34413) - (v * 0.71414);
        let b = y + (u * 1.772);
        *c0 = bf(r);
        *c1 = bf(g);
        *c2 = bf(b);
    }
}

/// The 5/3 DC shift of a row: the add without overflow, clamped.
#[inline(always)]
fn shift_53_row(p: &mut [i32], f: &Finish) {
    // (the sum saturated to 32 bits then clamped is the 64-bit sum clamped)
    for p in p {
        *p = p.saturating_add(f.shift).clamp(f.lo, f.hi);
    }
}

/// Round to nearest, ties to even, as `lrintf` in the default rounding
/// mode: adding and taking away 2^23 leaves a float below it without its
/// fraction, rounded the way the sum rounds; one at or above it has none.
#[inline(always)]
fn rint(v: f32) -> f32 {
    let m = 8_388_608f32.copysign(v);
    if v.abs() < 8_388_608.0 { (v + m) - m } else { v }
}

/// The 9/7 DC shift of one sample (float bits): at or above 2^31 the
/// maximum, below -2^31 or NaN the minimum, else `lrintf` plus the shift in
/// 64 bits, clamped.
#[inline(always)]
fn shift_97(bits: i32, f: &Finish) -> i32 {
    let v = fb(bits);
    if v >= 2_147_483_648.0 {
        return f.hi;
    }
    // below INT_MIN, or NaN
    if v.is_nan() || v < -2_147_483_648.0 {
        return f.lo;
    }
    let r = rint(v) as i64 + i64::from(f.shift);
    r.clamp(i64::from(f.lo), i64::from(f.hi)) as i32
}

/// What the fast form of the 9/7 DC shift needs of a component: its bounds
/// less its shift, as floats.
#[derive(Clone, Copy)]
struct FastShift {
    lo: f32,
    hi: f32,
    shift: i32,
}

/// The fast form's bounds, for a 9/7 component whose bounds less its shift
/// lie within 22 bits (every component of up to 22 bits).
fn fast_shift(f: &Finish) -> Option<FastShift> {
    let (lo, hi) = (i64::from(f.lo) - i64::from(f.shift), i64::from(f.hi) - i64::from(f.shift));
    (!f.reversible && lo >= -(1 << 22) && hi < (1 << 22)).then_some(FastShift {
        lo: lo as f32,
        hi: hi as f32,
        shift: f.shift,
    })
}

/// `shift_97` without a branch, a call or a float to integer conversion,
/// for the compiler to vectorise. The sample is clamped first, which gives
/// the same integer as clamping after the rounding (the bounds are
/// integers, and rounding keeps the order); the comparisons are written so
/// that NaN gives the minimum. Adding 1.5 x 2^23 then moves it where floats
/// are the integers: the sum is rounded to nearest, ties to even, as
/// `lrintf` rounds, and its mantissa is the integer, 2^22 added.
#[inline(always)]
fn shift_97_fast(v: f32, s: &FastShift) -> i32 {
    let c = if v > s.lo { v } else { s.lo };
    let c = if c < s.hi { c } else { s.hi };
    ((c + 12_582_912.0).to_bits() as i32)
        .wrapping_sub(0x4B40_0000)
        .wrapping_add(s.shift)
}

/// The 9/7 DC shift of a row.
#[inline(always)]
fn shift_97_row(p: &mut [i32], f: &Finish) {
    if let Some(s) = fast_shift(f) {
        for p in p {
            *p = shift_97_fast(fb(*p), &s);
        }
    } else {
        for p in p {
            *p = shift_97(*p, f);
        }
    }
}

#[inline(always)]
fn shift_row(p: &mut [i32], f: &Finish) {
    if f.reversible { shift_53_row(p, f) } else { shift_97_row(p, f) }
}

/// A component's region of a tile to finish and copy: `w` x `h` samples
/// from `src` (rows of `stride`).
pub(crate) struct Region<'a> {
    pub src: &'a [i32],
    pub stride: usize,
    pub f: Finish,
}

/// A block of the three components of an ICT finished in one pass: the
/// transform (`ict_rows`) and each component's DC shift (`shift_97_fast`),
/// every value in registers from its load to its store.
#[inline(always)]
fn ict_shift_block(src: [&[i32]; 3], dst: [&mut [i32]; 3], s: &[FastShift; 3]) {
    let [s0, s1, s2] = src;
    let [d0, d1, d2] = dst;
    for (((((d0, d1), d2), &y), &u), &v) in d0.iter_mut().zip(d1.iter_mut()).zip(d2.iter_mut()).zip(s0).zip(s1).zip(s2) {
        let (y, u, v) = (fb(y), fb(u), fb(v));
        let r = y + (v * 1.402);
        let g = y - (u * 0.34413) - (v * 0.71414);
        let b = y + (u * 1.772);
        *d0 = shift_97_fast(r, &s[0]);
        *d1 = shift_97_fast(g, &s[1]);
        *d2 = shift_97_fast(b, &s[2]);
    }
}

/// A block of one 9/7 component's DC shift, read from the tile.
#[inline(always)]
fn shift_block(src: &[i32], dst: &mut [i32], s: &FastShift) {
    for (d, &v) in dst.iter_mut().zip(src) {
        *d = shift_97_fast(fb(v), s);
    }
}

/// Finishes `k` (1 to 4) regions of the same size, the colour transformed
/// three first when `mct` is set, a block of a row at a time, and hands each
/// finished block to `store(row, first sample, samples)`: `samples[c]` holds
/// component `c`'s finished samples of the block.
#[inline(always)]
fn finish_regions(regions: &[Region<'_>], mct: Mct, w: usize, h: usize, mut store: impl FnMut(usize, usize, &[[i32; BLOCK]; 4], usize)) {
    let k = regions.len().min(4);
    let mct = if k >= 3 { mct } else { Mct::None };
    // the lossy texture: an ICT whose three components take the fast shift
    let fast: [Option<FastShift>; 4] = std::array::from_fn(|c| regions.get(c).and_then(|r| fast_shift(&r.f)));
    let ict = match (mct, fast) {
        (Mct::Ict, [Some(a), Some(b), Some(c), _]) => Some([a, b, c]),
        _ => None,
    };
    let mut buf = [[0i32; BLOCK]; 4];
    for j in 0..h {
        let mut at = 0;
        while at < w {
            let n = BLOCK.min(w - at);
            let row = |c: usize| -> &[i32] {
                let from = j * regions[c].stride + at;
                &regions[c].src[from..from + n]
            };
            let first = if let Some(s) = &ict {
                let [b0, b1, b2, _] = &mut buf;
                ict_shift_block([row(0), row(1), row(2)], [&mut b0[..n], &mut b1[..n], &mut b2[..n]], s);
                3
            } else if mct != Mct::None {
                for (c, b) in buf.iter_mut().enumerate().take(3) {
                    b[..n].copy_from_slice(row(c));
                }
                let [c0, c1, c2, _] = &mut buf;
                match mct {
                    Mct::Rct => rct_rows(&mut c0[..n], &mut c1[..n], &mut c2[..n]),
                    _ => ict_rows(&mut c0[..n], &mut c1[..n], &mut c2[..n]),
                }
                for (b, r) in buf.iter_mut().zip(regions).take(3) {
                    shift_row(&mut b[..n], &r.f);
                }
                3
            } else {
                0
            };
            for c in first..k {
                match &fast[c] {
                    Some(s) => shift_block(row(c), &mut buf[c][..n], s),
                    None => {
                        buf[c][..n].copy_from_slice(row(c));
                        shift_row(&mut buf[c][..n], &regions[c].f);
                    }
                }
            }
            store(j, at, &buf, n);
            at += n;
        }
    }
}

/// The regions finished into `k`-byte pixels: the low byte of each sample,
/// component `c` in byte `c`. `dst` starts at the regions' first pixel in
/// the image, its rows `dst_stride` bytes apart.
pub(crate) fn write_packed_u8(regions: &[Region<'_>], mct: Mct, w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    let k = regions.len().min(4);
    finish_regions(regions, mct, w, h, |j, at, buf, n| {
        let out = &mut dst[j * dst_stride + at * k..j * dst_stride + (at + n) * k];
        match k {
            1 => {
                for (o, &a) in out.iter_mut().zip(&buf[0][..n]) {
                    *o = a as u8;
                }
            }
            2 => {
                for ((o, &a), &b) in out.as_chunks_mut::<2>().0.iter_mut().zip(&buf[0][..n]).zip(&buf[1][..n]) {
                    o[0] = a as u8;
                    o[1] = b as u8;
                }
            }
            3 => {
                for (((o, &a), &b), &c) in out
                    .as_chunks_mut::<3>()
                    .0
                    .iter_mut()
                    .zip(&buf[0][..n])
                    .zip(&buf[1][..n])
                    .zip(&buf[2][..n])
                {
                    let px = (a as u8 as u32) | (b as u8 as u32) << 8 | (c as u8 as u32) << 16;
                    let [r, g, b, _] = px.to_le_bytes();
                    *o = [r, g, b];
                }
            }
            _ => {
                for ((((o, &a), &b), &c), &d) in out
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(&buf[0][..n])
                    .zip(&buf[1][..n])
                    .zip(&buf[2][..n])
                    .zip(&buf[3][..n])
                {
                    // one 32-bit store a pixel
                    *o = ((a as u8 as u32) | (b as u8 as u32) << 8 | (c as u8 as u32) << 16 | (d as u8 as u32) << 24).to_le_bytes();
                }
            }
        }
    });
}

/// The regions finished into planes of int32, one a component: `dsts[c]`
/// starts at the regions' first sample in component `c`'s plane, with that
/// plane's row stride (each plane's own: a sub-sampled component's plane is
/// narrower than the others, even where its tile region is the same).
pub(crate) fn write_planes(regions: &[Region<'_>], mct: Mct, w: usize, h: usize, dsts: &mut [(&mut [i32], usize)]) {
    finish_regions(regions, mct, w, h, |j, at, buf, n| {
        for ((dst, stride), b) in dsts.iter_mut().zip(buf) {
            let from = j * *stride + at;
            dst[from..from + n].copy_from_slice(&b[..n]);
        }
    });
}

/// In place, as OpenJPEG does it on the tile's buffers: the colour
/// transform over every sample of the three buffers (`n` samples each:
/// `opj_tcd_mct_decode`'s `l_samples` of the first).
pub(crate) fn mct_in_place(c0: &mut [i32], c1: &mut [i32], c2: &mut [i32], mct: Mct) {
    match mct {
        Mct::None => {}
        Mct::Rct => rct_rows(c0, c1, c2),
        Mct::Ict => ict_rows(c0, c1, c2),
    }
}

/// In place: a component's DC shift over its decoded region (`w` x `h`, top
/// left, rows of `stride`).
pub(crate) fn shift_in_place(plane: &mut [i32], stride: usize, w: usize, h: usize, f: &Finish) {
    for j in 0..h {
        shift_row(&mut plane[j * stride..j * stride + w], f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U8: Finish = Finish {
        shift: 128,
        lo: 0,
        hi: 255,
        reversible: false,
    };

    #[test]
    fn rint_rounds_ties_to_even() {
        for (v, r) in [
            (0.5, 0.0),
            (1.5, 2.0),
            (2.5, 2.0),
            (-0.5, 0.0),
            (-1.5, -2.0),
            (0.49999997, 0.0),
            (126.5, 126.0),
            (127.5, 128.0),
            (8388607.5, 8388608.0),
            (1e9, 1e9),
        ] {
            assert_eq!(rint(v), r, "{v}");
        }
    }

    #[test]
    fn the_97_shift_at_its_edges() {
        // the row form and the sample form agree, the edges included
        let wide = Finish {
            shift: 1 << 30,
            lo: 0,
            hi: i32::MAX,
            reversible: false,
        };
        for v in [
            0.0f32,
            -0.5,
            0.5,
            126.5,
            127.49,
            127.5,
            -128.5,
            -129.0,
            1e6,
            -1e6,
            2_147_483_648.0,
            -2_147_483_648.0,
            -3e9,
            3e9,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            for f in [U8, wide] {
                let mut row = [bf(v)];
                shift_97_row(&mut row, &f);
                assert_eq!(row[0], shift_97(bf(v), &f), "{v} with {f:?}");
            }
        }
        assert_eq!(shift_97(bf(2_147_483_648.0), &wide), i32::MAX);
        assert_eq!(shift_97(bf(f32::NAN), &U8), 0);
        assert_eq!(shift_97(bf(-0.5), &U8), 128);
        assert_eq!(shift_97(bf(0.5), &U8), 128);
        assert_eq!(shift_97(bf(1.5), &U8), 130);
        assert_eq!(shift_97(bf(300.0), &U8), 255);
    }

    #[test]
    fn the_fast_97_shift_is_the_sample_form() {
        let signed = Finish {
            shift: 0,
            lo: -2048,
            hi: 2047,
            reversible: false,
        };
        let wide = Finish {
            shift: 1 << 21,
            lo: 0,
            hi: (1 << 22) - 1,
            reversible: false,
        };
        let mut state = 0x0123_4567_89AB_CDEFu64;
        let mut values = vec![
            0.0f32,
            -0.0,
            0.5,
            -0.5,
            1.5,
            2.5,
            -1.5,
            -2.5,
            127.5,
            -128.5,
            254.5,
            255.5,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            3e9,
            -3e9,
            4_194_303.5,
        ];
        for i in 0..200_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let bits = (state >> 32) as u32;
            // every kind of float, then values around the range of a texture
            values.push(if i % 2 == 0 {
                f32::from_bits(bits)
            } else {
                (bits as i32 >> 12) as f32 / 512.0
            });
        }
        for f in [U8, signed, wide] {
            let s = fast_shift(&f).expect("within 22 bits");
            for &v in &values {
                assert_eq!(shift_97_fast(v, &s), shift_97(bf(v), &f), "{v} ({:#x}) with {f:?}", v.to_bits());
            }
        }
        assert!(
            fast_shift(&Finish {
                shift: 1 << 30,
                lo: 0,
                hi: i32::MAX,
                reversible: false
            })
            .is_none()
        );
        assert!(fast_shift(&Finish { reversible: true, ..U8 }).is_none());
    }

    #[test]
    fn the_fused_ict_is_the_transform_then_the_shift() {
        let mut state = 99u64;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            bf(((state >> 40) as i32 - (1 << 23)) as f32 / 30000.0)
        };
        let planes: Vec<Vec<i32>> = (0..4).map(|_| (0..700).map(|_| next()).collect()).collect();
        let regions: Vec<Region<'_>> = planes
            .iter()
            .map(|p| Region {
                src: p,
                stride: 350,
                f: U8,
            })
            .collect();
        let mut packed = vec![0u8; 700 * 4];
        write_packed_u8(&regions, Mct::Ict, 350, 2, &mut packed, 350 * 4);
        // the unfused way
        let mut c = planes.clone();
        let (a, rest) = c.split_at_mut(1);
        let (b, rest) = rest.split_at_mut(1);
        ict_rows(&mut a[0], &mut b[0], &mut rest[0]);
        for (ci, plane) in c.iter().enumerate() {
            for (i, &v) in plane.iter().enumerate() {
                assert_eq!(i32::from(packed[i * 4 + ci]), shift_97(v, &U8), "component {ci}, sample {i}");
            }
        }
    }

    #[test]
    fn rct_undoes_the_forward_transform() {
        // forward RCT of (r, g, b) = (200, 100, 50)
        let (r, g, b) = (200, 100, 50);
        let (mut y, mut u, mut v) = ([(r + 2 * g + b) >> 2], [b - g], [r - g]);
        rct_rows(&mut y, &mut u, &mut v);
        assert_eq!((y[0], u[0], v[0]), (r, g, b));
    }

    #[test]
    fn packs_four_components() {
        let planes: Vec<Vec<i32>> = (0..4).map(|c| (0..6).map(|i| c * 10 + i - 128).collect()).collect();
        let f = Finish {
            shift: 128,
            lo: 0,
            hi: 255,
            reversible: true,
        };
        let regions: Vec<Region<'_>> = planes.iter().map(|p| Region { src: p, stride: 3, f }).collect();
        let mut out = [0u8; 2 * 3 * 4];
        write_packed_u8(&regions, Mct::None, 3, 2, &mut out, 12);
        assert_eq!(&out[..8], &[0, 10, 20, 30, 1, 11, 21, 31]);
        assert_eq!(&out[20..], &[5, 15, 25, 35]);
    }
}
