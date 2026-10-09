//! Texture animation (llSetTextureAnim), CPU reference and record packing.
//!
//! Port of LLViewerTextureAnim::animateTextures
//! (indra/newview/llviewertextureanim.cpp) and of the texture matrix
//! LLVOVolume::animateTextures builds from its result (indra/newview/llvovolume.cpp),
//! originally LGPL 2.1.
//!
//! Firestorm evaluates every animation once per frame on the CPU and loads a
//! texture matrix per face (`texture_matrix0`). Aurora writes the animation
//! into the face's draw record once, when it changes, and the vertex shaders
//! evaluate it against the renderer clock (`tex_anim_*` in object.wgsl, a
//! line-by-line copy of [`clock_counter`] and [`frame_from_counter`]): no
//! per-frame CPU work, whatever the number of animated faces.
//!
//! The animation replaces parts of the face's texture transform
//! (LLVOVolume::animateTextures): ROTATE drives the rotation, SCALE the
//! repeats, the default mode the offsets (and the repeats, 1 / frame grid
//! size); the texture entry keeps the other parts. The result is applied like
//! the texture entry transform (LLFace xform: about the face center, rotate,
//! scale, offset). Legacy normal / specular maps follow the diffuse map
//! (LLFace::getGeometryVolume `tex_anim`); PBR faces apply it before each
//! map's KHR transform (textureUtilV.glsl texture_transform).

use std::time::Instant;

/// LLTextureAnim mode bits.
pub mod mode {
    pub const ON: u8 = 0x01;
    pub const LOOP: u8 = 0x02;
    pub const REVERSE: u8 = 0x04;
    pub const PING_PONG: u8 = 0x08;
    pub const SMOOTH: u8 = 0x10;
    pub const ROTATE: u8 = 0x20;
    pub const SCALE: u8 = 0x40;
}

/// Animation parameters (LLTextureAnim, without the face selection).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexAnimParams {
    pub mode: u8,
    pub size_x: u8,
    pub size_y: u8,
    pub start: f32,
    pub length: f32,
    pub rate: f32,
}

impl TexAnimParams {
    /// Non-finite values (corrupt packets) become 0 rather than NaN texture
    /// coordinates.
    pub fn sanitized(self) -> Self {
        let f = |v: f32| if v.is_finite() { v } else { 0.0 };
        TexAnimParams {
            start: f(self.start),
            length: f(self.length),
            rate: f(self.rate),
            ..self
        }
    }

    pub fn is_on(&self) -> bool {
        self.mode & mode::ON != 0
    }

    fn has(&self, bit: u8) -> bool {
        self.mode & bit != 0
    }

    /// Frames of one pass: `length`, or the whole grid.
    pub fn num_frames(&self) -> f32 {
        if self.length != 0.0 {
            self.length
        } else {
            (self.size_x as f32 * self.size_y as f32).max(1.0)
        }
    }

    /// Counter range of one cycle (ping-pong goes there and back).
    pub fn full_length(&self) -> f32 {
        let n = self.num_frames();
        if self.has(mode::PING_PONG) {
            if self.has(mode::SMOOTH) {
                2.0 * n
            } else if self.has(mode::LOOP) {
                (2.0 * n - 2.0).max(1.0)
            } else {
                (2.0 * n - 1.0).max(1.0)
            }
        } else {
            n
        }
    }
}

/// A face texture transform as LLFace's xform applies it: rotation (radians),
/// then repeats, then offsets, about the face center.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexXform {
    pub rot: f32,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

impl Default for TexXform {
    fn default() -> Self {
        TexXform {
            rot: 0.0,
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
        }
    }
}

impl TexXform {
    /// LLFace xform on GL texture coordinates (t up).
    pub fn apply(&self, st: [f32; 2]) -> [f32; 2] {
        let (s, t) = (st[0] - 0.5, st[1] - 0.5);
        let (sa, ca) = self.rot.sin_cos();
        let (s, t) = (s * ca + t * sa, -s * sa + t * ca);
        [s * self.scale[0] + self.offset[0] + 0.5, t * self.scale[1] + self.offset[1] + 0.5]
    }
}

/// `frame_counter` after LOOP (wrap) or one-shot (stop on the last frame),
/// from the raw counter `elapsed * rate (+ accumulated)`.
pub fn wrap_counter(p: &TexAnimParams, raw: f32) -> f32 {
    let full = p.full_length();
    if p.has(mode::LOOP) {
        // C fmod (the sign follows `raw`), as WGSL `%`
        raw % full
    } else {
        raw.min(full - 1.0)
    }
}

/// The face transform for a wrapped counter: the rest of
/// LLViewerTextureAnim::animateTextures (integer frames, ping-pong, reverse,
/// start, frame grid), combined with the texture entry's transform `te` as
/// LLVOVolume::animateTextures does.
pub fn frame_from_counter(p: &TexAnimParams, counter: f32, te: &TexXform) -> TexXform {
    let n = p.num_frames();
    let full = p.full_length();
    let smooth = p.has(mode::SMOOTH);
    let mut fc = counter;
    if !smooth {
        fc = (fc + 0.01).floor();
        // account for 0.01, we shouldn't step over full length
        fc = fc.min(full - 1.0);
    }
    if p.has(mode::PING_PONG) && fc >= n {
        fc = if smooth { n - (fc - n) } else { (n - 1.99) - (fc - n) };
    }
    if p.has(mode::REVERSE) {
        fc = if smooth { n - fc } else { (n - 0.99) - fc };
    }
    fc += p.start;
    if !smooth {
        // ll_round
        fc = (fc + 0.5).floor();
    }
    let mut x = *te;
    if p.has(mode::ROTATE) {
        x.rot = fc;
    } else if p.has(mode::SCALE) {
        x.scale = [fc, fc];
    } else if p.size_x != 0 && p.size_y != 0 {
        let (sx, sy) = (p.size_x as f32, p.size_y as f32);
        let ss = 1.0 / sx;
        let st = 1.0 / sy;
        let x_frame = fc % sx;
        let y_frame = (fc / sx).trunc();
        x.scale = [ss, st];
        x.offset = [(-0.5 + 0.5 * ss) + x_frame * ss, (0.5 - 0.5 * st) - y_frame * st];
    } else {
        // no frame grid (smooth scrolling with a size of 0): the offset only,
        // the repeats stay the texture entry's
        x.offset = [fc, 0.0];
    }
    x
}

/// Firestorm's per-frame evaluation from a raw counter (None when off).
pub fn animate(p: &TexAnimParams, raw: f32, te: &TexXform) -> Option<TexXform> {
    p.is_on().then(|| frame_from_counter(p, wrap_counter(p, raw), te))
}

/// Wrapped counter from the renderer clock, as the shaders compute it.
///
/// `elapsed_ms`: whole milliseconds since the animation's time origin (signed,
/// wrap-safe); `frac_ms`: the sub-millisecond part of the current time;
/// `frozen`: the counter of a rate-0 animation. A looping animation wraps the
/// whole seconds before adding the sub-second part: `rate * elapsed` in f32
/// would lose the frame-to-frame smoothness after a few hours.
pub fn clock_counter(p: &TexAnimParams, elapsed_ms: i32, frac_ms: f32, frozen: f32) -> f32 {
    if p.rate == 0.0 {
        return wrap_counter(p, frozen);
    }
    if elapsed_ms >= 0 {
        let whole = (elapsed_ms / 1000) as f32;
        let part = ((elapsed_ms % 1000) as f32 + frac_ms) * 0.001;
        if p.has(mode::LOOP) {
            let full = p.full_length().abs();
            let a = p.rate.abs();
            let m = ((a * whole) % full + a * part) % full;
            return if p.rate < 0.0 { -m } else { m };
        }
        return wrap_counter(p, p.rate * whole + p.rate * part);
    }
    wrap_counter(p, p.rate * (elapsed_ms as f32 + frac_ms) * 0.001)
}

/// Monotonic clock shared by the scene (animation time origins) and the
/// renderer (current time in the frame uniforms): milliseconds since the
/// renderer started, wrapping (differences stay right for 24 days either way).
#[derive(Debug, Clone, Copy)]
pub struct AnimClock {
    epoch: Instant,
}

impl AnimClock {
    pub fn new(epoch: Instant) -> Self {
        AnimClock { epoch }
    }

    /// Signed milliseconds of `t` since the epoch.
    fn signed_ms(&self, t: Instant) -> i64 {
        match t.checked_duration_since(self.epoch) {
            Some(d) => d.as_millis() as i64,
            None => -(self.epoch.duration_since(t).as_millis() as i64),
        }
    }

    /// Clock value of `t` (whole milliseconds, wrapping).
    pub fn ms(&self, t: Instant) -> u32 {
        self.signed_ms(t) as u32
    }

    /// Current time for the shaders: whole milliseconds and the
    /// sub-millisecond fraction (f32 bits).
    pub fn now(&self, t: Instant) -> [u32; 2] {
        let d = t.saturating_duration_since(self.epoch);
        let ms = d.as_millis();
        let frac = (d.as_nanos() - ms * 1_000_000) as f32 / 1.0e6;
        [ms as u32, frac.to_bits()]
    }
}

/// The animation fields of a draw record (see `DrawRecord::anim`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordAnim {
    /// `DrawRecord::flags[3]`: mode | size_x << 8 | size_y << 16.
    pub packed: u32,
    /// time origin (clock ms; f32 bits of the frozen counter at rate 0),
    /// start, length, rate (f32 bits).
    pub anim: [u32; 4],
    /// The texture entry's parts the animation leaves alone, by mode:
    /// ROTATE: repeats s, t, offsets s, t; SCALE: offsets s, t, rotation;
    /// otherwise: rotation and repeats s, t (1 / grid size with a grid).
    pub constants: [f32; 4],
}

/// Record fields of an animation whose counter is `phase + (t - start) * rate`
/// (`start` already on the [`AnimClock`]). None when the animation is off.
pub fn record_anim(p: &TexAnimParams, start_ms: u32, phase: f32, te: &TexXform) -> Option<RecordAnim> {
    let p = p.sanitized();
    if !p.is_on() {
        return None;
    }
    let origin = if p.rate == 0.0 {
        phase.to_bits()
    } else {
        // fold the accumulated phase into the time origin: counter = (t - origin) * rate
        let shift = (phase as f64 / p.rate as f64 * 1000.0).round().clamp(-1.0e9, 1.0e9) as i64;
        (start_ms as i64 - shift) as u32
    };
    let constants = if p.has(mode::ROTATE) {
        [te.scale[0], te.scale[1], te.offset[0], te.offset[1]]
    } else if p.has(mode::SCALE) {
        [te.offset[0], te.offset[1], te.rot, 0.0]
    } else if p.size_x != 0 && p.size_y != 0 {
        [te.rot, 1.0 / p.size_x as f32, 1.0 / p.size_y as f32, 0.0]
    } else {
        [te.rot, te.scale[0], te.scale[1], 0.0]
    };
    Some(RecordAnim {
        packed: p.mode as u32 | (p.size_x as u32) << 8 | (p.size_y as u32) << 16,
        anim: [origin, p.start.to_bits(), p.length.to_bits(), p.rate.to_bits()],
        constants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn params(mode: u8, size_x: u8, size_y: u8, start: f32, length: f32, rate: f32) -> TexAnimParams {
        TexAnimParams {
            mode,
            size_x,
            size_y,
            start,
            length,
            rate,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn assert_xform(x: TexXform, rot: f32, scale: [f32; 2], offset: [f32; 2]) {
        assert!(
            close(x.rot, rot) && close(x.scale[0], scale[0]) && close(x.scale[1], scale[1]),
            "{x:?} vs rot {rot} scale {scale:?}"
        );
        assert!(
            close(x.offset[0], offset[0]) && close(x.offset[1], offset[1]),
            "{x:?} vs offset {offset:?}"
        );
    }

    /// Grid frame -> (offset s, offset t) for a 4 × 4 grid, by hand from
    /// animateTextures: s = -0.375 + 0.25 x, t = 0.375 - 0.25 y.
    fn grid_offset(frame: u32) -> [f32; 2] {
        let (x, y) = ((frame % 4) as f32, (frame / 4) as f32);
        [-0.375 + 0.25 * x, 0.375 - 0.25 * y]
    }

    const ON: u8 = mode::ON;
    const LOOP: u8 = mode::LOOP;

    #[test]
    fn frame_grid_loop() {
        let te = TexXform {
            rot: 0.3,
            ..Default::default()
        };
        let p = params(ON | LOOP, 4, 4, 0.0, 0.0, 2.0);
        // (time s, expected frame): raw = 2 t, floor(raw + 0.01), wrapped at 16
        for (t, frame) in [(0.0, 0), (2.6, 5), (7.99, 15), (9.0, 2), (0.496, 1)] {
            let x = animate(&p, t * p.rate, &te).expect("on");
            // the grid replaces the repeats and offsets, the rotation stays
            assert_xform(x, 0.3, [0.25, 0.25], grid_offset(frame));
        }
    }

    #[test]
    fn one_shot_stops_on_last_frame() {
        let p = params(ON, 4, 4, 0.0, 0.0, 4.0);
        let x = animate(&p, 40.0, &TexXform::default()).expect("on");
        assert_xform(x, 0.0, [0.25, 0.25], grid_offset(15));
    }

    #[test]
    fn start_and_length_select_a_range() {
        // frames 4..8 of the grid: start 4, length 4
        let p = params(ON | LOOP, 4, 4, 4.0, 4.0, 1.0);
        for (t, frame) in [(0.5, 4), (3.2, 7), (4.1, 4), (6.0, 6)] {
            let x = animate(&p, t, &TexXform::default()).expect("on");
            assert_xform(x, 0.0, [0.25, 0.25], grid_offset(frame));
        }
    }

    #[test]
    fn ping_pong_loop() {
        // 4 frames: full length 2 * 4 - 2 = 6, sequence 0 1 2 3 2 1 | 0 ...
        let p = params(ON | LOOP | mode::PING_PONG, 4, 1, 0.0, 0.0, 1.0);
        for (t, frame) in [(0.2, 0), (1.5, 1), (3.5, 3), (4.2, 2), (5.5, 1), (6.3, 0)] {
            let x = animate(&p, t, &TexXform::default()).expect("on");
            let s = -0.375 + 0.25 * frame as f32;
            assert_xform(x, 0.0, [0.25, 1.0], [s, 0.0]);
        }
    }

    #[test]
    fn ping_pong_one_shot_ends_on_first_frame() {
        // full length 2 * 4 - 1 = 7: counter 6 -> (4 - 1.99) - 2 = 0.01 -> 0
        let p = params(ON | mode::PING_PONG, 4, 1, 0.0, 0.0, 1.0);
        let x = animate(&p, 100.0, &TexXform::default()).expect("on");
        assert_xform(x, 0.0, [0.25, 1.0], [-0.375, 0.0]);
    }

    #[test]
    fn reverse_frames() {
        // (4 - 0.99) - frame, rounded
        let p = params(ON | LOOP | mode::REVERSE, 4, 1, 0.0, 0.0, 1.0);
        for (t, frame) in [(0.0, 3), (1.5, 2), (3.9, 0), (4.0, 3)] {
            let x = animate(&p, t, &TexXform::default()).expect("on");
            assert_xform(x, 0.0, [0.25, 1.0], [-0.375 + 0.25 * frame as f32, 0.0]);
        }
    }

    #[test]
    fn smooth_scroll() {
        // the usual scrolling script: ANIM_ON | SMOOTH | LOOP, 1, 1, start 1, length 1, rate 1
        let p = params(ON | LOOP | mode::SMOOTH, 1, 1, 1.0, 1.0, 1.0);
        let x = animate(&p, 0.25, &TexXform::default()).expect("on");
        // fc = 0.25 + 1: x frame 0.25, y frame 1 (a whole repeat, invisible)
        assert_xform(x, 0.0, [1.0, 1.0], [0.25, -1.0]);
        let x = animate(&p, 3.75, &TexXform::default()).expect("on");
        assert_xform(x, 0.0, [1.0, 1.0], [0.75, -1.0]);
    }

    #[test]
    fn smooth_scroll_without_grid_keeps_repeats() {
        let te = TexXform {
            rot: 0.0,
            scale: [3.0, 2.0],
            offset: [0.1, 0.2],
        };
        let p = params(ON | LOOP | mode::SMOOTH, 0, 0, 0.0, 0.0, 0.5);
        // num frames max(1, 0) = 1: fc = fmod(0.5 * 1.5, 1) = 0.75
        let x = animate(&p, 0.75, &te).expect("on");
        assert_xform(x, 0.0, [3.0, 2.0], [0.75, 0.0]);
    }

    #[test]
    fn negative_rate_scrolls_back() {
        let p = params(ON | LOOP | mode::SMOOTH, 1, 1, 0.0, 1.0, -0.5);
        // raw -0.25: C fmod keeps the sign, x frame fmod(-0.25, 1), y frame trunc(-0.25) = 0
        let x = animate(&p, -0.25, &TexXform::default()).expect("on");
        assert_xform(x, 0.0, [1.0, 1.0], [-0.25, 0.0]);
    }

    #[test]
    fn smooth_ping_pong_and_reverse() {
        let p = params(ON | LOOP | mode::SMOOTH | mode::PING_PONG, 1, 1, 0.0, 1.0, 1.0);
        // full length 2: 1.5 -> 1 - 0.5
        let x = animate(&p, 1.5, &TexXform::default()).expect("on");
        assert_xform(x, 0.0, [1.0, 1.0], [0.5, 0.0]);
        let p = params(ON | LOOP | mode::SMOOTH | mode::REVERSE, 1, 1, 0.0, 1.0, 1.0);
        let x = animate(&p, 0.25, &TexXform::default()).expect("on");
        assert_xform(x, 0.0, [1.0, 1.0], [0.75, 0.0]);
    }

    #[test]
    fn rotate_keeps_repeats_and_offsets() {
        let te = TexXform {
            rot: 0.5,
            scale: [2.0, 3.0],
            offset: [0.25, -0.25],
        };
        let p = params(ON | LOOP | mode::SMOOTH | mode::ROTATE, 1, 1, 0.0, std::f32::consts::TAU, 1.0);
        let x = animate(&p, 1.0, &te).expect("on");
        assert_xform(x, 1.0, [2.0, 3.0], [0.25, -0.25]);
        let x = animate(&p, 7.0, &te).expect("on");
        assert_xform(x, 7.0 - std::f32::consts::TAU, [2.0, 3.0], [0.25, -0.25]);
    }

    #[test]
    fn scale_one_shot() {
        let te = TexXform {
            rot: 0.5,
            scale: [2.0, 3.0],
            offset: [0.25, -0.25],
        };
        // start 1, length 2, rate 0.5: counter min(1, raw), plus 1
        let p = params(ON | mode::SMOOTH | mode::SCALE, 1, 1, 1.0, 2.0, 0.5);
        let x = animate(&p, 0.5, &te).expect("on");
        assert_xform(x, 0.5, [1.5, 1.5], [0.25, -0.25]);
        let x = animate(&p, 5.0, &te).expect("on");
        assert_xform(x, 0.5, [2.0, 2.0], [0.25, -0.25]);
    }

    #[test]
    fn rotate_wins_over_scale_and_off_is_none() {
        let p = params(ON | mode::SMOOTH | mode::ROTATE | mode::SCALE, 1, 1, 0.0, 10.0, 1.0);
        let x = animate(&p, 2.0, &TexXform::default()).expect("on");
        assert_xform(x, 2.0, [1.0, 1.0], [0.0, 0.0]);
        let off = params(LOOP | mode::SMOOTH, 1, 1, 0.0, 1.0, 1.0);
        assert!(animate(&off, 2.0, &TexXform::default()).is_none());
    }

    #[test]
    fn xform_matches_llface() {
        // rotation by 90 degrees about the center, then scale and offset
        let x = TexXform {
            rot: std::f32::consts::FRAC_PI_2,
            scale: [2.0, 1.0],
            offset: [0.1, 0.0],
        };
        // s - 0.5 = 0.5, t - 0.5 = 0 -> rotated (0, -0.5) -> scaled (0, -0.5) -> + 0.6, 0.5
        let r = x.apply([1.0, 0.5]);
        assert!(close(r[0], 0.6) && close(r[1], 0.0), "{r:?}");
    }

    #[test]
    fn clock_counter_matches_raw_counter() {
        let cases = [
            params(ON | LOOP, 4, 4, 0.0, 0.0, 2.0),
            params(ON | LOOP | mode::SMOOTH, 1, 1, 0.0, 1.0, -0.37),
            params(ON | mode::SMOOTH, 1, 1, 0.0, 3.0, 0.7),
            params(ON | LOOP | mode::PING_PONG, 4, 1, 0.0, 0.0, 3.0),
        ];
        for p in cases {
            for (ms, frac) in [(0, 0.0f32), (1234, 0.5), (59_999, 0.25), (-1500, 0.75)] {
                let expected = wrap_counter(&p, p.rate * (ms as f64 * 0.001 + frac as f64 * 0.001) as f32);
                let got = clock_counter(&p, ms, frac, 0.0);
                assert!((got - expected).abs() < 1e-4, "{p:?} at {ms} ms: {got} vs {expected}");
            }
        }
        // rate 0: the frozen counter
        let p = params(ON | LOOP | mode::SMOOTH, 1, 1, 0.0, 1.0, 0.0);
        assert!(close(clock_counter(&p, 5000, 0.0, 2.25), 0.25));
    }

    #[test]
    fn clock_counter_stays_smooth_after_hours() {
        // 10 hours in, a fast smooth scroll: frame-to-frame steps keep the
        // exact rate (naive f32 `rate * elapsed` steps by 4 ms multiples)
        let p = params(ON | LOOP | mode::SMOOTH, 1, 1, 0.0, 1.0, 2.0);
        let base = 36_000_000i32 + 123;
        let a = clock_counter(&p, base, 0.0, 0.0);
        let b = clock_counter(&p, base + 16, 0.6667, 0.0);
        let step = (b - a).rem_euclid(1.0);
        assert!((step - 2.0 * 0.0166667).abs() < 1e-4, "step {step}");
        // and the phase itself is right
        let exact = ((base as f64 * 0.001 * 2.0) % 1.0) as f32;
        assert!((a - exact).abs() < 1e-3, "{a} vs {exact}");
    }

    #[test]
    fn record_folds_the_phase_into_the_origin() {
        let te = TexXform::default();
        let p = params(ON | LOOP | mode::SMOOTH, 1, 1, 0.0, 1.0, 0.5);
        // counter 0.25 at start: origin 0.5 s earlier
        let r = record_anim(&p, 10_000, 0.25, &te).expect("on");
        assert_eq!(r.anim[0], 9_500);
        assert_eq!(r.packed, (ON | LOOP | mode::SMOOTH) as u32 | 1 << 8 | 1 << 16);
        assert_eq!(f32::from_bits(r.anim[3]), 0.5);
        // translate with a grid: TE rotation, 1 / grid size
        assert_eq!(r.constants, [0.0, 1.0, 1.0, 0.0]);
        // rate 0: the frozen counter
        let p0 = TexAnimParams { rate: 0.0, ..p };
        let r = record_anim(&p0, 10_000, 0.75, &te).expect("on");
        assert_eq!(f32::from_bits(r.anim[0]), 0.75);
        // origin before the clock epoch wraps
        let r = record_anim(&p, 100, 0.25, &te).expect("on");
        assert_eq!(r.anim[0], (-400i32) as u32);
        assert!(record_anim(&TexAnimParams { mode: LOOP, ..p }, 0, 0.0, &te).is_none());
    }

    #[test]
    fn record_constants_by_mode() {
        let te = TexXform {
            rot: 0.5,
            scale: [2.0, 3.0],
            offset: [0.25, -0.25],
        };
        let r = |mode, sx, sy| {
            record_anim(&params(mode, sx, sy, 0.0, 1.0, 1.0), 0, 0.0, &te)
                .expect("on")
                .constants
        };
        assert_eq!(r(ON | mode::ROTATE, 1, 1), [2.0, 3.0, 0.25, -0.25]);
        assert_eq!(r(ON | mode::SCALE, 1, 1), [0.25, -0.25, 0.5, 0.0]);
        assert_eq!(r(ON, 4, 2), [0.5, 0.25, 0.5, 0.0]);
        assert_eq!(r(ON | mode::SMOOTH, 0, 0), [0.5, 2.0, 3.0, 0.0]);
    }

    #[test]
    fn clock_signed_milliseconds() {
        let epoch = Instant::now();
        let c = AnimClock::new(epoch);
        assert_eq!(c.ms(epoch + Duration::from_millis(1500)), 1500);
        if let Some(before) = epoch.checked_sub(Duration::from_millis(250)) {
            assert_eq!(c.ms(before), (-250i32) as u32);
        }
        let [ms, frac] = c.now(epoch + Duration::from_micros(2_500_250));
        assert_eq!(ms, 2500);
        assert!((f32::from_bits(frac) - 0.25).abs() < 1e-6);
    }
}
