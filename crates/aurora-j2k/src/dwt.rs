// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/dwt.cpp`:
//   Copyright (c) 2026, Anne Skydancer (neo-j2k)
// OpenJPEG, through it:
//   Copyright (c) 2002-2014, Universite catholique de Louvain (UCL), Belgium
//   Copyright (c) 2002-2014, Professor Benoit Macq
//   Copyright (c) 2001-2003, David Janssens
//   Copyright (c) 2002-2003, Yannick Verschueren
//   Copyright (c) 2003-2007, Francois-Olivier Devaux
//   Copyright (c) 2003-2014, Antonin Descampe
//   Copyright (c) 2005, Herve Drolon, FreeImage Team
//   Copyright (c) 2007, Jonathan Ballard <dzonatas@dzonux.net>
//   Copyright (c) 2007, Callum Lerwick <seg@haxxed.com>
//   Copyright (c) 2017, IntoPIX SA <support@intopix.com>
//   All rights reserved.

//! The inverse wavelet transforms of a whole tile-component: the reversible
//! 5/3 and the irreversible 9/7.
//!
//! After neo-j2k's `src/dwt.cpp` (BSD-2-Clause, Copyright (c) 2026 Anne
//! Skydancer) and OpenJPEG 2.5.4's `dwt.c` (`opj_dwt_decode_tile`,
//! `opj_dwt_decode_tile_97`; BSD-2-Clause, see NOTICE.md): every sample
//! takes OpenJPEG's lifting steps, in OpenJPEG's order (each level its rows
//! first, then its columns) and with its expressions, so the results are
//! OpenJPEG's, lossy included. The 9/7 works on floats; Rust never contracts
//! a product and a sum into one fused operation, which is what bit identity
//! with OpenJPEG's SSE path needs.
//!
//! The structure is neo-j2k's row-wise transform, taken one step further:
//! - a row's horizontal lifting works on its low and high halves where they
//!   lie, every access contiguous, which the compiler vectorises (OpenJPEG
//!   gathers eight rows into an interleaved buffer and scatters them back);
//! - the vertical lifting works on whole rows, its steps advancing together
//!   down the rows, so a row is in the cache from its first step to its
//!   last;
//! - the rows of a level are stored where the transform leaves them
//!   ([`RowLayout`]): the low rows of a level on the rows of one parity, the
//!   high rows on the other, as the lifting scheme has them. The vertical
//!   lifting then needs no reordering of the rows, and only the columns of a
//!   finished row are interleaved, through one row of scratch. Tier-1 writes
//!   each band's rows there (`tile.rs`).
//!
//! One difference from OpenJPEG on streams no encoder writes: where the 5/3
//! sums wrap around 32 bits, OpenJPEG's result depends on the build (its
//! SIMD column code computes `(d + d + 2) >> 2` where its scalar code has
//! `(d + 1) >> 1`); this decoder has the scalar forms everywhere.

use crate::header::MAX_RESOLUTIONS;

/// A resolution's area in its own coordinates (tile-component at that
/// level).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl ResRect {
    fn w(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }

    fn h(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }
}

/// Where the rows of each resolution lie in a tile-component's buffer: row
/// `n` of resolution `k` (in its own interleaved order) is the buffer's row
/// `off[k] + n * step[k]`. The top resolution's rows are the buffer's; the
/// low rows of a resolution are the rows of the resolution below.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RowLayout {
    off: [u64; MAX_RESOLUTIONS],
    step: [u64; MAX_RESOLUTIONS],
}

impl RowLayout {
    /// The layout for resolutions `rects[0]` to the top one, `rects.last()`.
    pub fn new(rects: &[ResRect]) -> Self {
        let mut l = RowLayout {
            off: [0; MAX_RESOLUTIONS],
            step: [1; MAX_RESOLUTIONS],
        };
        let top = rects.len().clamp(1, MAX_RESOLUTIONS) - 1;
        for k in (1..=top).rev() {
            // the low rows of resolution k are its rows of parity vcas
            let vcas = (rects[k].y0 & 1) as u64;
            l.off[k - 1] = l.off[k] + vcas * l.step[k];
            l.step[k - 1] = l.step[k].saturating_mul(2);
        }
        l
    }

    /// The buffer's row for row `n` of resolution `level`.
    pub fn row(&self, level: usize, n: usize) -> usize {
        (self.off[level] + n as u64 * self.step[level]) as usize
    }

    /// The buffer's row for row `y` of a band of resolution `level` (`high`:
    /// the LH and HH bands, else LL and HL), and the buffer rows between two
    /// rows of the band. `vcas`: the parity of the resolution's first row.
    pub fn band_row(&self, level: usize, high: bool, vcas: usize, y: usize) -> (usize, usize) {
        if level == 0 {
            return (self.row(0, y), self.step[0] as usize);
        }
        let parity = if high { 1 - vcas } else { vcas };
        (self.row(level, 2 * y + parity), (2 * self.step[level]) as usize)
    }
}

// From table F.4 of the standard, and OpenJPEG's BUG_WEIRD_TWO_INVK: the
// high band is scaled by 2 / invK, which tcd.c compensates with a gain of 0
// for irreversible bands
#[allow(clippy::excessive_precision)]
const ALPHA: f32 = -1.586134342;
#[allow(clippy::excessive_precision)]
const BETA: f32 = -0.052980118;
#[allow(clippy::excessive_precision)]
const GAMMA: f32 = 0.882911075;
#[allow(clippy::excessive_precision)]
const DELTA: f32 = 0.443506852;
#[allow(clippy::excessive_precision)]
const K: f32 = 1.230174105;
#[allow(clippy::excessive_precision)]
const TWO_INV_K: f32 = 1.625732422;

#[inline(always)]
fn fb(x: i32) -> f32 {
    f32::from_bits(x as u32)
}

#[inline(always)]
fn bf(x: f32) -> i32 {
    x.to_bits() as i32
}

/// The lifting steps of a wavelet. A line holds its low band then its high
/// band; step `s` lifts the low band when `s` is even, the high band when it
/// is odd, from the samples of the other band next to each target.
trait Kernel {
    const STAGES: usize;
    /// A target between two sources.
    fn mid(s: usize, t: &mut [i32], a: &[i32], b: &[i32]);
    /// The first sample of the line as target: its one source, mirrored.
    fn first(s: usize, t: &mut [i32], a: &[i32]);
    /// The last sample of the line as target: its one source, mirrored.
    fn tail(s: usize, t: &mut [i32], a: &[i32]);
    /// What a line takes before its steps: the bands' scaling.
    fn scale(low: bool, t: &mut [i32]);
    /// A line of one sample, on parity `cas`.
    fn single(cas: usize, t: &mut [i32]);
}

/// The 9/7 (`opj_v8dwt_decode`): `K` and `2 / K` on the bands, then delta
/// and beta on the low band, gamma and alpha on the high one.
struct Irreversible;

impl Kernel for Irreversible {
    const STAGES: usize = 4;

    #[inline(always)]
    fn mid(s: usize, t: &mut [i32], a: &[i32], b: &[i32]) {
        let c = [-DELTA, -GAMMA, -BETA, -ALPHA][s];
        for ((t, &a), &b) in t.iter_mut().zip(a).zip(b) {
            *t = bf(fb(*t) + ((fb(a) + fb(b)) * c));
        }
    }

    #[inline(always)]
    fn first(s: usize, t: &mut [i32], a: &[i32]) {
        Self::mid(s, t, a, a);
    }

    #[inline(always)]
    fn tail(s: usize, t: &mut [i32], a: &[i32]) {
        let c = [-DELTA, -GAMMA, -BETA, -ALPHA][s];
        let c = c + c;
        for (t, &a) in t.iter_mut().zip(a) {
            *t = bf(fb(*t) + (c * fb(a)));
        }
    }

    #[inline(always)]
    fn scale(low: bool, t: &mut [i32]) {
        let c = if low { K } else { TWO_INV_K };
        for t in t {
            *t = bf(fb(*t) * c);
        }
    }

    fn single(_cas: usize, _t: &mut [i32]) {}
}

/// The 5/3 (`opj_idwt53_h`, `opj_idwt53_v`), in wrapping 32-bit arithmetic.
struct Reversible;

impl Kernel for Reversible {
    const STAGES: usize = 2;

    #[inline(always)]
    fn mid(s: usize, t: &mut [i32], a: &[i32], b: &[i32]) {
        if s == 0 {
            for ((t, &a), &b) in t.iter_mut().zip(a).zip(b) {
                *t = t.wrapping_sub(a.wrapping_add(b).wrapping_add(2) >> 2);
            }
        } else {
            for ((t, &a), &b) in t.iter_mut().zip(a).zip(b) {
                *t = t.wrapping_add(a.wrapping_add(b) >> 1);
            }
        }
    }

    #[inline(always)]
    fn first(s: usize, t: &mut [i32], a: &[i32]) {
        Self::tail(s, t, a);
    }

    #[inline(always)]
    fn tail(s: usize, t: &mut [i32], a: &[i32]) {
        if s == 0 {
            for (t, &a) in t.iter_mut().zip(a) {
                *t = t.wrapping_sub(a.wrapping_add(1) >> 1);
            }
        } else {
            for (t, &a) in t.iter_mut().zip(a) {
                *t = t.wrapping_add(a);
            }
        }
    }

    fn scale(_low: bool, _t: &mut [i32]) {}

    #[inline(always)]
    fn single(cas: usize, t: &mut [i32]) {
        if cas == 1 {
            for t in t {
                *t /= 2;
            }
        }
    }
}

/// A band on the even positions of its line lifted from the band on the odd
/// ones: `e[i]` between `o[i - 1]` and `o[i]`.
#[inline(always)]
fn lift_even<Kn: Kernel>(s: usize, e: &mut [i32], o: &[i32]) {
    let (ne, no) = (e.len(), o.len());
    let m = ne.min(no);
    if m == 0 {
        return;
    }
    Kn::first(s, &mut e[..1], &o[..1]);
    Kn::mid(s, &mut e[1..m], &o[..m - 1], &o[1..m]);
    if no < ne {
        Kn::tail(s, &mut e[no..no + 1], &o[no - 1..no]);
    }
}

/// A band on the odd positions lifted from the band on the even ones:
/// `o[i]` between `e[i]` and `e[i + 1]`.
#[inline(always)]
fn lift_odd<Kn: Kernel>(s: usize, o: &mut [i32], e: &[i32]) {
    let (no, ne) = (o.len(), e.len());
    if ne == 0 {
        return;
    }
    let m = no.min(ne - 1);
    Kn::mid(s, &mut o[..m], &e[..m], &e[1..m + 1]);
    if m < no {
        Kn::tail(s, &mut o[m..m + 1], &e[m..m + 1]);
    }
}

/// One row's horizontal lifting: `sn` low then high samples in `row`, the
/// first of the resolution's row on parity `cas`, lifted where they lie.
/// Their interleave is left to the level's last touch of the row.
#[inline(always)]
fn lift_line<Kn: Kernel>(row: &mut [i32], sn: usize, cas: usize) {
    if row.len() < 2 {
        Kn::single(cas, row);
        return;
    }
    let (lo, hi) = row.split_at_mut(sn);
    Kn::scale(true, lo);
    Kn::scale(false, hi);
    for s in 0..Kn::STAGES {
        // the low band is on the even positions when cas is 0
        if s % 2 == 0 {
            if cas == 0 {
                lift_even::<Kn>(s, lo, hi)
            } else {
                lift_odd::<Kn>(s, lo, hi)
            }
        } else if cas == 0 {
            lift_odd::<Kn>(s, hi, lo)
        } else {
            lift_even::<Kn>(s, hi, lo)
        }
    }
}

/// A row's columns put in their interleaved order: its `sn` low samples on
/// the positions of parity `cas`, the others between them.
#[inline(always)]
fn interleave(row: &mut [i32], sn: usize, cas: usize, tmp: &mut [i32]) {
    let n = row.len();
    if n < 2 {
        return;
    }
    let tmp = &mut tmp[..n];
    tmp.copy_from_slice(row);
    let (lo, hi) = tmp.split_at(sn);
    let (even, odd) = if cas == 0 { (lo, hi) } else { (hi, lo) };
    let pairs = even.len().min(odd.len());
    for ((pair, &e), &o) in row.as_chunks_mut::<2>().0.iter_mut().zip(even).zip(odd) {
        pair[0] = e;
        pair[1] = o;
    }
    if even.len() > pairs {
        row[n - 1] = even[pairs];
    }
}

/// One level: the resolution's size, its low band's and where its rows lie.
struct Level {
    rw: usize,
    rh: usize,
    hsn: usize,
    vsn: usize,
    hcas: usize,
    vcas: usize,
    /// The buffer's row of the resolution's row 0, and the rows between two.
    off: usize,
    step: usize,
}

/// Row `t` of the buffer for writing and rows `a` and `b` for reading (both
/// other than `t`), `rw` samples each.
#[inline(always)]
fn rows3(plane: &mut [i32], stride: usize, rw: usize, t: usize, a: usize, b: usize) -> (&mut [i32], &[i32], &[i32]) {
    let (lo, rest) = plane.split_at_mut(t * stride);
    let (trow, hi) = rest.split_at_mut(rw);
    // a row after t starts stride - rw samples into `hi`
    let pick = |r: usize| -> &[i32] {
        if r < t {
            &lo[r * stride..r * stride + rw]
        } else {
            let at = (r - t) * stride - rw;
            &hi[at..at + rw]
        }
    };
    (trow, pick(a), pick(b))
}

impl Level {
    /// The buffer's row of the resolution's row `n`.
    fn row(&self, n: usize) -> usize {
        self.off + n * self.step
    }

    fn line<'a>(&self, plane: &'a mut [i32], stride: usize, n: usize) -> &'a mut [i32] {
        let at = self.row(n) * stride;
        &mut plane[at..at + self.rw]
    }

    /// A row's first touch: its horizontal lifting, and what the vertical
    /// lifting takes of it first (the band's scaling).
    #[inline(always)]
    fn enter<Kn: Kernel>(&self, plane: &mut [i32], stride: usize, n: usize, vertical: bool) {
        let line = self.line(plane, stride, n);
        lift_line::<Kn>(line, self.hsn, self.hcas);
        if vertical {
            Kn::scale(n % 2 == self.vcas, line);
        }
    }

    /// Step `s` of the vertical lifting on its target of index `idx` (the
    /// row `2 * idx` of the resolution when `even`, else `2 * idx + 1`).
    #[inline(always)]
    fn vertical<Kn: Kernel>(&self, plane: &mut [i32], stride: usize, s: usize, even: bool, idx: usize) {
        let (ne, no) = (self.rh.div_ceil(2), self.rh / 2);
        let r = |n: usize| self.row(n);
        if even {
            if idx >= ne {
                return;
            }
            if idx == 0 {
                let (t, a, _) = rows3(plane, stride, self.rw, r(0), r(1), r(1));
                Kn::first(s, t, a);
            } else if idx < no {
                let (t, a, b) = rows3(plane, stride, self.rw, r(2 * idx), r(2 * idx - 1), r(2 * idx + 1));
                Kn::mid(s, t, a, b);
            } else {
                let (t, a, _) = rows3(plane, stride, self.rw, r(2 * idx), r(2 * idx - 1), r(2 * idx - 1));
                Kn::tail(s, t, a);
            }
        } else {
            if idx >= no {
                return;
            }
            if idx + 1 < ne {
                let (t, a, b) = rows3(plane, stride, self.rw, r(2 * idx + 1), r(2 * idx), r(2 * idx + 2));
                Kn::mid(s, t, a, b);
            } else {
                let (t, a, _) = rows3(plane, stride, self.rw, r(2 * idx + 1), r(2 * idx), r(2 * idx));
                Kn::tail(s, t, a);
            }
        }
    }

    /// The level's inverse transform, in place: each row's horizontal
    /// lifting as the row is first needed, the vertical steps advancing
    /// together down the rows, and a finished row's columns interleaved.
    fn decode<Kn: Kernel>(&self, plane: &mut [i32], stride: usize, tmp: &mut [i32]) {
        let (rw, rh) = (self.rw, self.rh);
        if rw == 0 || rh == 0 {
            return;
        }
        if rh < 2 {
            // one row: no vertical lifting
            self.enter::<Kn>(plane, stride, 0, false);
            let line = self.line(plane, stride, 0);
            Kn::single(self.vcas, line);
            interleave(line, self.hsn, self.hcas, tmp);
            return;
        }
        let stages = Kn::STAGES;
        let (ne, no) = (rh.div_ceil(2), rh / 2);
        // Iteration t brings in the rows 2t and 2t + 1. With the low rows on
        // the even ones (vcas 0), step s then lifts its target of index
        // t - (s + 1) / 2: delta on low row t, gamma on high row t - 1, beta
        // on low row t - 1, alpha on high row t - 2. With the low rows on
        // the odd ones, step s lifts index t - 1 - s / 2. A row is finished
        // once the last step that reads it has run.
        let (even_lag, odd_lag) = if self.vcas == 0 {
            (stages / 2, stages / 2)
        } else {
            (stages / 2, stages / 2 + 1)
        };
        for t in 0..ne + stages / 2 + 2 {
            if t < ne {
                self.enter::<Kn>(plane, stride, 2 * t, true);
            }
            if t < no {
                self.enter::<Kn>(plane, stride, 2 * t + 1, true);
            }
            for s in 0..stages {
                let low = s % 2 == 0;
                let (lag, even) = if self.vcas == 0 { (s.div_ceil(2), low) } else { (1 + s / 2, !low) };
                if let Some(idx) = t.checked_sub(lag) {
                    self.vertical::<Kn>(plane, stride, s, even, idx);
                }
            }
            if let Some(i) = t.checked_sub(even_lag).filter(|&i| i < ne) {
                interleave(self.line(plane, stride, 2 * i), self.hsn, self.hcas, tmp);
            }
            if let Some(i) = t.checked_sub(odd_lag).filter(|&i| i < no) {
                interleave(self.line(plane, stride, 2 * i + 1), self.hsn, self.hcas, tmp);
            }
        }
    }
}

/// Scratch of the transform, kept between decodes: one row.
#[derive(Debug, Default)]
pub(crate) struct DwtScratch {
    tmp: Vec<i32>,
}

/// Transforms `plane` in place from resolution 0 up to `numres - 1`.
/// `plane` holds the samples of the top resolution `rects.last()`, `stride`
/// per row (int32 for 5/3, float bits for 9/7), each band's rows where
/// [`RowLayout::new`]`(rects)` has them. When `numres` stops below the top
/// resolution, the samples of resolution `numres - 1` end in the buffer's
/// top left corner, where OpenJPEG has them.
pub(crate) fn decode(plane: &mut [i32], stride: usize, rects: &[ResRect], numres: usize, reversible: bool, scratch: &mut DwtScratch) {
    let Some(top) = rects.len().checked_sub(1) else {
        return;
    };
    let numres = numres.clamp(1, rects.len());
    if stride == 0 || rects[top].h() == 0 || plane.len() < stride * (rects[top].h() - 1) + rects[top].w() {
        return;
    }
    let layout = RowLayout::new(rects);
    if scratch.tmp.len() < stride {
        scratch.tmp.resize(stride, 0);
    }
    for k in 1..numres {
        let (prev, cur) = (&rects[k - 1], &rects[k]);
        let level = Level {
            rw: cur.w(),
            rh: cur.h(),
            hsn: prev.w().min(cur.w()),
            vsn: prev.h().min(cur.h()),
            hcas: (cur.x0 & 1) as usize,
            vcas: (cur.y0 & 1) as usize,
            off: layout.row(k, 0),
            step: layout.step[k] as usize,
        };
        // (the low band of a resolution is the resolution below: one sample
        // in two, rounded by the parity; anything else is not a wavelet level)
        let fits = |n: usize, sn: usize, cas: usize| sn == if cas == 0 { n.div_ceil(2) } else { n / 2 };
        if !fits(level.rw, level.hsn, level.hcas) || !fits(level.rh, level.vsn, level.vcas) {
            return;
        }
        if reversible {
            level.decode::<Reversible>(plane, stride, &mut scratch.tmp);
        } else {
            level.decode::<Irreversible>(plane, stride, &mut scratch.tmp);
        }
    }
    let last = numres - 1;
    if last < top {
        // the resolution decoded, moved to the top left corner: its row n
        // lies at or after the buffer's row n
        let (rw, rh) = (rects[last].w(), rects[last].h());
        for n in 0..rh {
            let from = layout.row(last, n);
            if from != n {
                plane.copy_within(from * stride..from * stride + rw, n * stride);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- OpenJPEG's own structure, as the reference: a line gathered in
    // interleaved order, lifted, written back (opj_idwt53_h, opj_v8dwt_decode
    // with one lane), rows then columns, the bands in OpenJPEG's quadrants.

    fn ref53(line: &mut [i32], sn: usize, cas: usize) {
        let len = line.len();
        if len == 0 {
            return;
        }
        let dn = len - sn;
        let add = i32::wrapping_add;
        let sub = i32::wrapping_sub;
        let mut tmp = vec![0i32; len];
        if cas == 0 {
            if len <= 1 {
                return;
            }
            let (even, odd) = line.split_at(sn);
            let mut s1n = even[0];
            let mut d1n = odd[0];
            let mut s0n = sub(s1n, add(d1n, 1) >> 1);
            let mut i = 0;
            let mut j = 1;
            while i + 3 < len {
                let d1c = d1n;
                let s0c = s0n;
                s1n = even[j];
                d1n = odd[j];
                s0n = sub(s1n, add(add(d1c, d1n), 2) >> 2);
                tmp[i] = s0c;
                tmp[i + 1] = add(d1c, add(s0c, s0n) >> 1);
                i += 2;
                j += 1;
            }
            tmp[i] = s0n;
            if len & 1 != 0 {
                tmp[len - 1] = sub(even[(len - 1) / 2], add(d1n, 1) >> 1);
                tmp[len - 2] = add(d1n, add(s0n, tmp[len - 1]) >> 1);
            } else {
                tmp[len - 1] = add(d1n, s0n);
            }
            let _ = (s1n, dn);
        } else {
            if len == 1 {
                line[0] /= 2;
                return;
            }
            let (odd, even) = line.split_at(sn);
            if len == 2 {
                tmp[1] = sub(odd[0], add(even[0], 1) >> 1);
                tmp[0] = add(even[0], tmp[1]);
            } else {
                let mut s1 = even[1];
                let mut dc = sub(odd[0], add(add(even[0], s1), 2) >> 2);
                tmp[0] = add(even[0], dc);
                let mut i = 1;
                let mut j = 1;
                let end = len - 2 - usize::from(len & 1 == 0);
                while i < end {
                    let s2 = even[j + 1];
                    let dnv = sub(odd[j], add(add(s1, s2), 2) >> 2);
                    tmp[i] = dc;
                    tmp[i + 1] = add(s1, add(dnv, dc) >> 1);
                    dc = dnv;
                    s1 = s2;
                    i += 2;
                    j += 1;
                }
                tmp[i] = dc;
                if len & 1 == 0 {
                    let dnv = sub(odd[len / 2 - 1], add(s1, 1) >> 1);
                    tmp[len - 2] = add(s1, add(dnv, dc) >> 1);
                    tmp[len - 1] = dnv;
                } else {
                    tmp[len - 1] = add(s1, dc);
                }
            }
        }
        line.copy_from_slice(&tmp);
    }

    fn ref97(line: &mut [i32], sn: usize, cas: usize) {
        let len = line.len();
        let dn = len - sn;
        let (a, b) = if cas == 0 {
            if !(dn > 0 || sn > 1) {
                return;
            }
            (0usize, 1usize)
        } else {
            if !(sn > 0 || dn > 1) {
                return;
            }
            (1, 0)
        };
        // interleaved, with room after the line as OpenJPEG's buffer has
        let mut w = vec![0f32; len + 8];
        for i in 0..sn {
            w[cas + 2 * i] = fb(line[i]);
        }
        for i in 0..dn {
            w[1 - cas + 2 * i] = fb(line[sn + i]);
        }
        for i in 0..sn {
            w[a + 2 * i] *= K;
        }
        for i in 0..dn {
            w[b + 2 * i] *= TWO_INV_K;
        }
        // opj_v8dwt_decode_step2
        let step2 = |w: &mut [f32], l: usize, fw0: usize, end: usize, m: usize, c: f32| {
            let imax = end.min(m);
            let mut fw = fw0;
            let mut fl = l;
            for _ in 0..imax {
                w[fw - 1] += (w[fl] + w[fw]) * c;
                fl = fw;
                fw += 2;
            }
            if m < end {
                let c = c + c;
                w[fw - 1] += w[fl] * c;
            }
        };
        let ml = sn.min(dn.wrapping_sub(a));
        let mh = dn.min(sn.wrapping_sub(b));
        step2(&mut w, b, a + 1, sn, ml, -DELTA);
        step2(&mut w, a, b + 1, dn, mh, -GAMMA);
        step2(&mut w, b, a + 1, sn, ml, -BETA);
        step2(&mut w, a, b + 1, dn, mh, -ALPHA);
        for (x, v) in line.iter_mut().zip(&w) {
            *x = bf(*v);
        }
    }

    /// `opj_dwt_decode_tile` on OpenJPEG's quadrant layout.
    fn reference(plane: &mut [i32], stride: usize, rects: &[ResRect], reversible: bool) {
        let line = |l: &mut [i32], sn: usize, cas: usize| if reversible { ref53(l, sn, cas) } else { ref97(l, sn, cas) };
        for k in 1..rects.len() {
            let (rw, rh) = (rects[k].w(), rects[k].h());
            let (hsn, vsn) = (rects[k - 1].w(), rects[k - 1].h());
            for j in 0..rh {
                line(&mut plane[j * stride..j * stride + rw], hsn, (rects[k].x0 & 1) as usize);
            }
            for i in 0..rw {
                let mut col: Vec<i32> = (0..rh).map(|j| plane[j * stride + i]).collect();
                line(&mut col, vsn, (rects[k].y0 & 1) as usize);
                for (j, v) in col.into_iter().enumerate() {
                    plane[j * stride + i] = v;
                }
            }
        }
    }

    fn rects_of(x0: i32, y0: i32, x1: i32, y1: i32, levels: u32) -> Vec<ResRect> {
        let up = |a: i32, l: u32| ((i64::from(a) + (1i64 << l) - 1) >> l) as i32;
        (0..=levels)
            .map(|r| levels - r)
            .map(|l| ResRect {
                x0: up(x0, l),
                y0: up(y0, l),
                x1: up(x1, l),
                y1: up(y1, l),
            })
            .collect()
    }

    /// A small generator: the same numbers on every platform.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
    }

    /// The same coefficients in OpenJPEG's quadrants and in the row layout.
    fn planes(rects: &[ResRect], stride: usize, reversible: bool, rng: &mut Rng) -> (Vec<i32>, Vec<i32>) {
        let top = rects.len() - 1;
        let (w, h) = (rects[top].w(), rects[top].h());
        let mut quad = vec![0i32; stride * h.max(1)];
        let mut rows = quad.clone();
        let layout = RowLayout::new(rects);
        let sample = |rng: &mut Rng| {
            let v = (rng.next() % 2001) as i32 - 1000;
            if reversible { v } else { bf(v as f32 * 0.37) }
        };
        let _ = w;
        for k in 0..=top {
            let (rw, rh) = (rects[k].w(), rects[k].h());
            let (pw, ph) = if k == 0 { (0, 0) } else { (rects[k - 1].w(), rects[k - 1].h()) };
            let vcas = (rects[k].y0 & 1) as usize;
            for y in 0..rh {
                for x in 0..rw {
                    if k > 0 && x < pw && y < ph {
                        continue; // the low band is the resolution below
                    }
                    let v = sample(rng);
                    quad[y * stride + x] = v;
                    let row = if k == 0 {
                        layout.band_row(0, false, 0, y).0
                    } else if y < ph {
                        layout.band_row(k, false, vcas, y).0
                    } else {
                        layout.band_row(k, true, vcas, y - ph).0
                    };
                    rows[row * stride + x] = v;
                }
            }
        }
        (quad, rows)
    }

    fn check(x0: i32, y0: i32, x1: i32, y1: i32, levels: u32, reversible: bool, seed: u64) {
        let rects = rects_of(x0, y0, x1, y1, levels);
        let top = rects.len() - 1;
        let (w, h) = (rects[top].w(), rects[top].h());
        let stride = w + 3;
        let mut rng = Rng(seed);
        let (mut quad, mut rows) = planes(&rects, stride, reversible, &mut rng);
        reference(&mut quad, stride, &rects, reversible);
        decode(&mut rows, stride, &rects, rects.len(), reversible, &mut DwtScratch::default());
        for y in 0..h {
            assert_eq!(
                &quad[y * stride..y * stride + w],
                &rows[y * stride..y * stride + w],
                "row {y} of ({x0},{y0})-({x1},{y1}), {levels} levels, reversible {reversible}"
            );
        }
    }

    #[test]
    fn matches_openjpeg_structure_on_every_small_geometry() {
        let mut seed = 1;
        for reversible in [true, false] {
            for levels in 0..=4 {
                for (x0, y0) in [(0, 0), (1, 0), (0, 1), (1, 1), (5, 3), (7, 12)] {
                    for w in 1..=11 {
                        for h in [1, 2, 3, 4, 5, 8, 13] {
                            seed += 1;
                            check(x0, y0, x0 + w, y0 + h, levels, reversible, seed);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn matches_openjpeg_structure_on_larger_tiles() {
        for reversible in [true, false] {
            check(0, 0, 64, 64, 5, reversible, 7);
            check(0, 0, 100, 37, 3, reversible, 8);
            check(3, 5, 131, 70, 5, reversible, 9);
            check(1, 1, 258, 129, 6, reversible, 10);
        }
    }

    #[test]
    fn a_partial_decode_ends_in_the_top_left_corner() {
        for reversible in [true, false] {
            let rects = rects_of(1, 3, 40, 29, 3);
            let stride = rects[3].w();
            let mut rng = Rng(99);
            let (mut quad, mut rows) = planes(&rects, stride, reversible, &mut rng);
            reference(&mut quad, stride, &rects[..3], reversible);
            decode(&mut rows, stride, &rects, 3, reversible, &mut DwtScratch::default());
            for y in 0..rects[2].h() {
                assert_eq!(
                    &quad[y * stride..y * stride + rects[2].w()],
                    &rows[y * stride..y * stride + rects[2].w()],
                    "row {y}"
                );
            }
        }
    }

    #[test]
    fn row_layout_nests_the_low_rows() {
        // an even origin: low rows on the even rows at every level
        let l = RowLayout::new(&rects_of(0, 0, 16, 16, 2));
        assert_eq!((l.row(2, 5), l.row(1, 3), l.row(0, 1)), (5, 6, 4));
        assert_eq!(l.band_row(2, true, 0, 2), (5, 2));
        assert_eq!(l.band_row(1, false, 0, 1), (4, 4));
        // an odd origin at the top level: its low rows are the odd rows
        let l = RowLayout::new(&rects_of(0, 1, 16, 17, 1));
        assert_eq!(l.band_row(1, false, 1, 0), (1, 2));
        assert_eq!(l.band_row(1, true, 1, 0), (0, 2));
        assert_eq!(l.row(0, 3), 7);
    }
}
