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
    if !(v >= -2_147_483_648.0) {
        return f.lo; // below INT_MIN, or NaN
    }
    let r = rint(v) as i64 + i64::from(f.shift);
    r.clamp(i64::from(f.lo), i64::from(f.hi)) as i32
}

/// The 9/7 DC shift of a row.
#[inline(always)]
fn shift_97_row(p: &mut [i32], f: &Finish) {
    // Bounds that floats hold exactly (every component of up to 24 bits):
    // the clamp is taken on the rounded float, which the compiler
    // vectorises. `max` then `min` give the minimum for NaN, as shift_97.
    let (lo, hi) = (i64::from(f.lo) - i64::from(f.shift), i64::from(f.hi) - i64::from(f.shift));
    if lo.abs() <= 1 << 24 && hi.abs() <= 1 << 24 {
        let (lo, hi) = (lo as f32, hi as f32);
        for p in p {
            *p = (rint(fb(*p)).max(lo).min(hi) as i32).wrapping_add(f.shift);
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

/// Finishes `k` (1 to 4) regions of the same size, the colour transformed
/// three first when `mct` is set, a block of a row at a time, and hands each
/// finished block to `store(row, first sample, samples)`: `samples[c]` holds
/// component `c`'s finished samples of the block.
#[inline(always)]
fn finish_regions(regions: &[Region<'_>], mct: Mct, w: usize, h: usize, mut store: impl FnMut(usize, usize, &[[i32; BLOCK]; 4], usize)) {
    let k = regions.len().min(4);
    let mut buf = [[0i32; BLOCK]; 4];
    for j in 0..h {
        let mut at = 0;
        while at < w {
            let n = BLOCK.min(w - at);
            for (b, r) in buf.iter_mut().zip(regions) {
                let from = j * r.stride + at;
                b[..n].copy_from_slice(&r.src[from..from + n]);
            }
            if k >= 3 && mct != Mct::None {
                let [c0, c1, c2, _] = &mut buf;
                match mct {
                    Mct::Rct => rct_rows(&mut c0[..n], &mut c1[..n], &mut c2[..n]),
                    _ => ict_rows(&mut c0[..n], &mut c1[..n], &mut c2[..n]),
                }
            }
            for (b, r) in buf.iter_mut().zip(regions) {
                shift_row(&mut b[..n], &r.f);
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
                for ((o, &a), &b) in out.chunks_exact_mut(2).zip(&buf[0][..n]).zip(&buf[1][..n]) {
                    o[0] = a as u8;
                    o[1] = b as u8;
                }
            }
            3 => {
                for (((o, &a), &b), &c) in out.chunks_exact_mut(3).zip(&buf[0][..n]).zip(&buf[1][..n]).zip(&buf[2][..n]) {
                    o[0] = a as u8;
                    o[1] = b as u8;
                    o[2] = c as u8;
                }
            }
            _ => {
                for ((((o, &a), &b), &c), &d) in out
                    .chunks_exact_mut(4)
                    .zip(&buf[0][..n])
                    .zip(&buf[1][..n])
                    .zip(&buf[2][..n])
                    .zip(&buf[3][..n])
                {
                    // one 32-bit store a pixel
                    o.copy_from_slice(
                        &((a as u8 as u32) | (b as u8 as u32) << 8 | (c as u8 as u32) << 16 | (d as u8 as u32) << 24).to_le_bytes(),
                    );
                }
            }
        }
    });
}

/// The regions finished into planes of int32, one a component: `dsts[c]`
/// starts at the regions' first sample in component `c`'s plane, its rows
/// `dst_stride` samples apart.
pub(crate) fn write_planes(regions: &[Region<'_>], mct: Mct, w: usize, h: usize, dsts: &mut [&mut [i32]], dst_stride: usize) {
    finish_regions(regions, mct, w, h, |j, at, buf, n| {
        for (dst, b) in dsts.iter_mut().zip(buf) {
            dst[j * dst_stride + at..j * dst_stride + at + n].copy_from_slice(&b[..n]);
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
