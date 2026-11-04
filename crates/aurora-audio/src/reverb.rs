//! The reverberation of the surroundings: one shared send bus that the
//! world voices feed, so that a small room, a large hall and the open air
//! sound different. Aurora's own (Preferences › Sound, "Reverb from the
//! surroundings", off by default); the Legacy Viewer plays every world sound
//! dry, which the switch turned off keeps, bit for bit.
//!
//! The viewer estimates the surroundings a few times a second (rays from the
//! listener, `scene/sound_reverb.rs`) and sends [`ReverbParams`]: how
//! enclosed the listener is (the level sent to the reverb), the decay time,
//! the pre-delay, the damping of the highs and the mean free path of the
//! room. The world voices on the Sounds and Ambient channels add their mono
//! signal, after the occlusion filter and before the panning or the
//! head-related filters, to the send (`mixer.rs`): the late reverberation is
//! diffuse, it comes from every side, so it carries neither the source's
//! direction nor the colouring of the direct path's filters, and it sounds
//! the same with the 3D sound for headphones on or off. Interface sounds,
//! voice chat, the parcel music and media never reach it.
//!
//! The send goes through a pre-delay line, four allpass diffusers (the
//! echoes of a clap become a smooth tail at once) and a feedback delay
//! network: eight delay lines mixed by a Householder matrix (orthogonal, so
//! the loop neither gains nor loses energy by itself), each with the gain
//! that makes it lose 60 dB in the decay time and a one-pole low-pass that
//! makes the highs die sooner. The lines' lengths follow the room's mean
//! free path; their fractional part goes through a first-order allpass
//! interpolator, flat at every frequency, so that the decay time holds at
//! any length. Two orthogonal sums of the lines make the left and right
//! outputs, uncorrelated.
//!
//! No click: the parameters glide (a quarter of a second, by blocks of
//! [`BLOCK`] frames), the lengths and the pre-delay at most [`MAX_SLEW`]
//! (a 1.5 % pitch change of the tail while it glides), the on / off switch
//! fades the output over [`FADE_SECS`]. Nothing allocates or locks while
//! rendering: every line is allocated with the reverb. A tail that died out
//! clears the state and the reverb goes idle (no work, no denormals) until
//! the next sound; values below [`DENORMAL`] are zero.

use crate::mixer::ENGINE_SAMPLE_RATE;

/// The parameters of the surroundings, from the viewer's estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    /// Share of the world sounds sent to the reverb, 0 (the open air: none)
    /// to 1 (a closed room).
    pub level: f32,
    /// Decay time, 60 dB down (RT60, seconds).
    pub decay: f32,
    /// Time between a sound and its first reflections (seconds).
    pub predelay: f32,
    /// Decay time of the highs over that of the lows (0.1 .. 1; 1: no
    /// damping).
    pub damping: f32,
    /// Mean free path of the room (m): the mean length of the delay lines.
    pub path: f32,
}

impl ReverbParams {
    /// Shortest and longest decay times held.
    pub const MIN_DECAY: f32 = 0.1;
    pub const MAX_DECAY: f32 = 4.0;
    /// Longest pre-delay (s).
    pub const MAX_PREDELAY: f32 = 0.06;
    /// Shortest and longest mean free paths (m).
    pub const MIN_PATH: f32 = BASE_PATH * MIN_SCALE;
    pub const MAX_PATH: f32 = BASE_PATH * MAX_SCALE;

    /// The values in range (garbage gives a dry reverb).
    pub fn sanitized(self) -> ReverbParams {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        ReverbParams {
            level: finite(self.level, 0.0).clamp(0.0, 1.0),
            decay: finite(self.decay, Self::MIN_DECAY).clamp(Self::MIN_DECAY, Self::MAX_DECAY),
            predelay: finite(self.predelay, 0.0).clamp(0.0, Self::MAX_PREDELAY),
            damping: finite(self.damping, 1.0).clamp(0.1, 1.0),
            path: finite(self.path, BASE_PATH).clamp(Self::MIN_PATH, Self::MAX_PATH),
        }
    }
}

impl Default for ReverbParams {
    /// A small room.
    fn default() -> Self {
        ReverbParams {
            level: 1.0,
            decay: 0.5,
            predelay: 0.008,
            damping: 0.7,
            path: 3.5,
        }
    }
}

/// Frames between two updates of the gliding parameters.
pub(crate) const BLOCK: usize = 32;
/// Delay lines of the network.
const LINES: usize = 8;
/// Their lengths at a scale of 1 (samples at 48 kHz, mutually prime, 18 to
/// 41 ms): a mean of 28.9 ms.
const BASE: [f32; LINES] = [887.0, 1009.0, 1163.0, 1291.0, 1427.0, 1601.0, 1777.0, 1949.0];
/// The speed of sound (m/s).
const SOUND_SPEED: f32 = 343.0;
/// The mean free path at a scale of 1 (m): the mean of `BASE` travelled.
const BASE_PATH: f32 = 11_104.0 / LINES as f32 / ENGINE_SAMPLE_RATE as f32 * SOUND_SPEED;
/// The lengths' scales for the shortest and longest paths (7 to 98 ms for
/// the longest line).
const MIN_SCALE: f32 = 0.4;
const MAX_SCALE: f32 = 2.4;
/// Room in each delay line (a power of two above the longest length, its
/// interpolator included).
const LINE_ROOM: usize = 8192;
/// Room in the pre-delay line.
const PRE_ROOM: usize = 4096;
/// The diffusers: lengths (samples) and gains.
const DIFFUSERS: [(usize, f32); 4] = [(149, 0.62), (113, 0.62), (367, 0.55), (271, 0.55)];
/// Glide of the parameters (s).
const GLIDE_SECS: f32 = 0.25;
/// Fade of the output when the reverb is turned on or off (s).
pub(crate) const FADE_SECS: f32 = 0.05;
/// Fastest change of a delay's length (samples per sample: a 1.5 % pitch
/// change of what the line holds while it glides).
const MAX_SLEW: f32 = 0.015;
/// Gain of the outputs: a closed small room (0.5 s) returns about 12 dB
/// less energy than it receives, and so does a hall (2.5 s), over a longer
/// tail: the echoes of a larger room are sparser, as in a real one.
const OUTPUT_GAIN: f32 = 0.146;
/// Values this small are zero.
const DENORMAL: f32 = 1.0e-20;
/// A tail whose lines all stay below this (-160 dB) has died out.
const SILENT: f32 = 1.0e-8;

/// Signs of the send into each line, and of each line in the left and the
/// right outputs: three orthogonal vectors, none along the Householder
/// matrix's axis (all ones).
const INPUT_SIGNS: [f32; LINES] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const LEFT_SIGNS: [f32; LINES] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
const RIGHT_SIGNS: [f32; LINES] = [1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0];

#[inline(always)]
fn flush(v: f32) -> f32 {
    if v.abs() < DENORMAL { 0.0 } else { v }
}

/// A Schroeder allpass of a fixed length.
struct Diffuser {
    buf: Vec<f32>,
    at: usize,
    gain: f32,
}

impl Diffuser {
    #[inline(always)]
    fn next(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.at];
        let v = flush(x + self.gain * delayed);
        self.buf[self.at] = v;
        self.at += 1;
        if self.at == self.buf.len() {
            self.at = 0;
        }
        delayed - self.gain * v
    }
}

/// The parameters as they glide, in the units the network uses.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Current {
    level: f32,
    decay: f32,
    damping: f32,
    /// Pre-delay (samples).
    predelay: f32,
    /// Scale of the lines' lengths.
    scale: f32,
}

impl Current {
    fn of(p: ReverbParams) -> Current {
        Current {
            level: p.level,
            decay: p.decay,
            damping: p.damping,
            predelay: p.predelay * ENGINE_SAMPLE_RATE as f32,
            scale: p.path / BASE_PATH,
        }
    }
}

/// The reverb bus, owned by the mixer.
pub(crate) struct Reverb {
    /// On (the setting), and the parameters to glide to.
    on: bool,
    target: Current,
    current: Current,
    /// Output fade, 0 (off) to 1.
    fade: f32,
    pre: Vec<f32>,
    pre_at: usize,
    diffusers: [Diffuser; 4],
    lines: [Vec<f32>; LINES],
    /// Write position, shared by every line.
    at: usize,
    /// Per line: whole delay, interpolator coefficient and state, damping
    /// pole and state, feedback gain.
    whole: [usize; LINES],
    interp: [f32; LINES],
    interp_state: [f32; LINES],
    pole: [f32; LINES],
    lowpass: [f32; LINES],
    gain: [f32; LINES],
    /// Every state is zero: nothing to render until a sound comes.
    idle: bool,
    /// Frames since the send last carried a sound.
    quiet: usize,
}

impl Reverb {
    pub(crate) fn new() -> Reverb {
        let start = Current::of(ReverbParams::default());
        let mut r = Reverb {
            on: false,
            target: start,
            current: start,
            fade: 0.0,
            pre: vec![0.0; PRE_ROOM],
            pre_at: 0,
            diffusers: DIFFUSERS.map(|(len, gain)| Diffuser {
                buf: vec![0.0; len],
                at: 0,
                gain,
            }),
            lines: std::array::from_fn(|_| vec![0.0; LINE_ROOM]),
            at: 0,
            whole: [0; LINES],
            interp: [0.0; LINES],
            interp_state: [0.0; LINES],
            pole: [0.0; LINES],
            lowpass: [0.0; LINES],
            gain: [0.0; LINES],
            idle: true,
            quiet: 0,
        };
        r.update_lines();
        r
    }

    /// On with these parameters, or off (`None`: the output fades out, then
    /// the reverb stops).
    pub(crate) fn set(&mut self, params: Option<ReverbParams>) {
        match params {
            Some(p) => {
                self.on = true;
                self.target = Current::of(p.sanitized());
                if self.idle {
                    // nothing rings: the new room at once
                    self.current = self.target;
                    self.update_lines();
                }
            }
            None => self.on = false,
        }
    }

    /// The mixer must feed and render it: on, or fading out.
    pub(crate) fn running(&self) -> bool {
        self.on || self.fade > 0.0
    }

    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.idle
    }

    #[cfg(test)]
    fn states(&self) -> impl Iterator<Item = f32> + '_ {
        self.lines
            .iter()
            .flatten()
            .chain(&self.pre)
            .chain(self.diffusers.iter().flat_map(|d| d.buf.iter()))
            .chain(&self.interp_state)
            .chain(&self.lowpass)
            .copied()
    }

    /// The lengths, interpolators, gains and damping of the lines for the
    /// current parameters.
    fn update_lines(&mut self) {
        let rate = ENGINE_SAMPLE_RATE as f32;
        let c = self.current;
        for (i, base) in BASE.iter().enumerate() {
            let len = base * c.scale;
            // (the allpass interpolator is best with a fraction in 0.5..1.5)
            let whole = (len - 0.5).floor().max(1.0);
            let frac = len - whole;
            self.whole[i] = whole as usize;
            self.interp[i] = (1.0 - frac) / (1.0 + frac);
            let secs = len / rate;
            let g = 10f32.powf(-3.0 * secs / c.decay);
            // the highs' gain at Nyquist, through the pole
            let r = 10f32.powf(-3.0 * secs / c.decay * (1.0 / c.damping - 1.0));
            self.gain[i] = g;
            self.pole[i] = (1.0 - r) / (1.0 + r);
        }
    }

    /// Clear every state.
    fn clear(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.pre.fill(0.0);
        for d in &mut self.diffusers {
            d.buf.fill(0.0);
        }
        self.interp_state = [0.0; LINES];
        self.lowpass = [0.0; LINES];
        self.idle = true;
        self.quiet = 0;
    }

    /// Glide the parameters by one block.
    fn glide(&mut self) {
        let k = 1.0 - (-(BLOCK as f32) / (GLIDE_SECS * ENGINE_SAMPLE_RATE as f32)).exp();
        let (c, t) = (&mut self.current, self.target);
        let ease = |from: f32, to: f32| {
            let d = to - from;
            if d.abs() < 1.0e-5 * to.abs().max(1.0e-3) {
                to
            } else {
                from + d * k
            }
        };
        c.level = ease(c.level, t.level);
        c.decay = ease(c.decay, t.decay);
        c.damping = ease(c.damping, t.damping);
        // the lengths glide no faster than the slew (the longest line's)
        let max_scale = MAX_SLEW * BLOCK as f32 / BASE[LINES - 1];
        c.scale += (ease(c.scale, t.scale) - c.scale).clamp(-max_scale, max_scale);
        let max_pre = MAX_SLEW * BLOCK as f32;
        c.predelay += (ease(c.predelay, t.predelay) - c.predelay).clamp(-max_pre, max_pre);
    }

    /// Adds the reverberation of the mono `send` (one value per frame) to
    /// the interleaved stereo `out`.
    pub(crate) fn render(&mut self, send: &[f32], out: &mut [f32]) {
        let frames = send.len().min(out.len() / 2);
        let fade_step = 1.0 / (FADE_SECS * ENGINE_SAMPLE_RATE as f32);
        let mut f = 0;
        while f < frames {
            let n = (frames - f).min(BLOCK);
            let input = &send[f..f + n];
            let output = &mut out[2 * f..2 * (f + n)];
            f += n;
            let heard = input.iter().any(|x| x.abs() >= DENORMAL);
            if !self.on && self.fade <= 0.0 {
                if !self.idle {
                    self.clear();
                }
                continue;
            }
            if self.idle && !heard {
                // (nothing rings: nothing to add, the fade keeps going)
                self.fade = if self.on { 1.0 } else { 0.0 };
                continue;
            }
            self.idle = false;
            self.quiet = if heard { 0 } else { self.quiet + n };
            let from = self.current.level;
            if self.current != self.target {
                self.glide();
                self.update_lines();
            }
            let dlevel = (self.current.level - from) / n as f32;
            let mut level = from;
            // (the shortest line delays the first echo too)
            let pre_read = (self.current.predelay - BASE[0] * self.current.scale).max(0.0);
            let mut loudest = 0.0f32;
            for (x, o) in input.iter().zip(output.as_chunks_mut::<2>().0) {
                level += dlevel;
                // the pre-delay, linearly interpolated
                self.pre[self.pre_at] = flush(x * level);
                let back = pre_read.clamp(0.0, (PRE_ROOM - 2) as f32);
                let whole = back as usize;
                let frac = back - whole as f32;
                let a = self.pre[(self.pre_at + PRE_ROOM - whole) % PRE_ROOM];
                let b = self.pre[(self.pre_at + PRE_ROOM - whole - 1) % PRE_ROOM];
                self.pre_at = (self.pre_at + 1) % PRE_ROOM;
                let mut d = a + (b - a) * frac;
                for diffuser in &mut self.diffusers {
                    d = diffuser.next(d);
                }
                // the network
                let mut taps = [0.0f32; LINES];
                let mut sum = 0.0;
                for i in 0..LINES {
                    let line = &self.lines[i];
                    let near = line[(self.at + LINE_ROOM - self.whole[i]) & (LINE_ROOM - 1)];
                    let far = line[(self.at + LINE_ROOM - self.whole[i] - 1) & (LINE_ROOM - 1)];
                    // first-order allpass: y = a x[n] + x[n-1] - a y[n-1]
                    let y = flush(self.interp[i] * (near - self.interp_state[i]) + far);
                    self.interp_state[i] = y;
                    let lp = flush(y + self.pole[i] * (self.lowpass[i] - y));
                    self.lowpass[i] = lp;
                    let v = lp * self.gain[i];
                    taps[i] = v;
                    sum += v;
                    loudest = loudest.max(v.abs());
                }
                // Householder: v - 2/N sum(v), the send spread with signs
                let reflect = sum * (2.0 / LINES as f32);
                let inject = d * (1.0 / (LINES as f32).sqrt());
                let (mut l, mut r) = (0.0, 0.0);
                for i in 0..LINES {
                    self.lines[i][self.at] = flush(taps[i] - reflect + inject * INPUT_SIGNS[i]);
                    l += taps[i] * LEFT_SIGNS[i];
                    r += taps[i] * RIGHT_SIGNS[i];
                }
                self.at = (self.at + 1) & (LINE_ROOM - 1);
                self.fade = if self.on {
                    (self.fade + fade_step).min(1.0)
                } else {
                    (self.fade - fade_step).max(0.0)
                };
                let g = OUTPUT_GAIN * self.fade;
                o[0] += l * g;
                o[1] += r * g;
            }
            // died out: the send long silent (past the pre-delay and the
            // diffusers) and every line below -160 dB
            if self.quiet > PRE_ROOM + 1024 && loudest < SILENT {
                self.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(decay: f32, damping: f32, path: f32) -> ReverbParams {
        ReverbParams {
            level: 1.0,
            decay,
            predelay: 0.01,
            damping,
            path,
        }
    }

    /// The stereo response to a unit impulse, `secs` long.
    fn impulse_response(p: ReverbParams, secs: f32) -> Vec<f32> {
        let mut r = Reverb::new();
        r.set(Some(p));
        let frames = (secs * ENGINE_SAMPLE_RATE as f32) as usize;
        let mut send = vec![0.0; frames];
        send[0] = 1.0;
        // (on from the start: the fade already open)
        r.fade = 1.0;
        let mut out = vec![0.0; frames * 2];
        for (s, o) in send.chunks(480).zip(out.chunks_mut(960)) {
            r.render(s, o);
        }
        out
    }

    /// The decay time measured on the energy decay curve (Schroeder's
    /// backward integration), fitted between -5 and -35 dB.
    fn measured_decay(out: &[f32]) -> f32 {
        let energy: Vec<f64> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f64::from(f[0] * f[0] + f[1] * f[1]))
            .collect();
        let mut tail = vec![0.0f64; energy.len() + 1];
        for i in (0..energy.len()).rev() {
            tail[i] = tail[i + 1] + energy[i];
        }
        let db = |i: usize| 10.0 * (tail[i] / tail[0]).log10();
        let at = |level: f64| (0..energy.len()).find(|&i| db(i) <= level).expect("decays that far");
        let (a, b) = (at(-5.0), at(-35.0));
        (60.0 * (b - a) as f64 / 30.0 / f64::from(ENGINE_SAMPLE_RATE)) as f32
    }

    #[test]
    fn the_decay_matches_the_requested_time() {
        for (decay, path) in [(0.3, 3.0), (0.8, 6.0), (1.5, 12.0), (3.0, 20.0)] {
            let out = impulse_response(room(decay, 1.0, path), decay * 1.6 + 0.3);
            let got = measured_decay(&out);
            assert!((got / decay - 1.0).abs() < 0.1, "asked {decay} s at {path} m, measured {got} s");
        }
    }

    #[test]
    fn the_highs_die_sooner_when_damped() {
        let out = impulse_response(room(1.5, 0.4, 8.0), 2.5);
        // the highs: the second difference of successive samples
        let highs: Vec<f32> = out
            .as_chunks::<2>()
            .0
            .windows(3)
            .flat_map(|w| [w[2][0] - 2.0 * w[1][0] + w[0][0], w[2][1] - 2.0 * w[1][1] + w[0][1]])
            .collect();
        let (all, high) = (measured_decay(&out), measured_decay(&highs));
        assert!(high < all * 0.7, "highs {high} s, all {all} s");
    }

    #[test]
    fn energy_returned_for_a_room_and_a_hall() {
        let ratio = |p: ReverbParams| {
            let out = impulse_response(p, p.decay * 2.0 + 0.2);
            10.0 * out.iter().map(|v| v * v).sum::<f32>().log10()
        };
        let small = ratio(room(0.5, 0.7, 3.5));
        let hall = ratio(room(2.5, 0.5, 18.0));
        assert!((-14.0..-10.0).contains(&small), "small room {small} dB");
        assert!((-14.0..-9.0).contains(&hall), "hall {hall} dB");
    }

    #[test]
    fn stays_stable_at_the_longest_decay_and_with_garbage() {
        let mut r = Reverb::new();
        r.set(Some(ReverbParams {
            level: 1.0,
            decay: 1.0e9,
            predelay: f32::NAN,
            damping: 7.0,
            path: 1.0e6,
        }));
        let held = r.target_params();
        assert!(
            (held.decay - ReverbParams::MAX_DECAY).abs() < 1e-4 && held.damping == 1.0 && held.predelay == 0.0,
            "{held:?}"
        );
        let mut n = 0x1234_5678u32;
        let mut send = vec![0.0; 480];
        let mut out = vec![0.0; 960];
        let mut loudest = 0.0f32;
        // twenty seconds of full-scale noise, then ten of silence
        for k in 0..3000 {
            for s in &mut send {
                n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *s = if k < 2000 {
                    (n >> 8) as f32 / (1u32 << 23) as f32 - 1.0
                } else {
                    0.0
                };
            }
            out.fill(0.0);
            r.render(&send, &mut out);
            loudest = out.iter().fold(loudest, |m, v| m.max(v.abs()));
            assert!(out.iter().all(|v| v.is_finite()));
        }
        assert!(loudest < 8.0, "loudest {loudest}");
        let last = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(last < loudest * 0.01, "decays: {last}");
    }

    impl Reverb {
        fn target_params(&self) -> ReverbParams {
            ReverbParams {
                level: self.target.level,
                decay: self.target.decay,
                predelay: self.target.predelay / ENGINE_SAMPLE_RATE as f32,
                damping: self.target.damping,
                path: self.target.scale * BASE_PATH,
            }
        }
    }

    #[test]
    fn a_long_silence_leaves_no_denormals_and_goes_idle() {
        let mut r = Reverb::new();
        r.set(Some(room(4.0, 0.9, 20.0)));
        let mut send = vec![0.0; 480];
        let mut out = vec![0.0; 960];
        send[0] = 1.0;
        r.render(&send, &mut out);
        send[0] = 0.0;
        assert!(!r.is_idle());
        // a minute of silence
        for _ in 0..6000 {
            r.render(&send, &mut out);
            assert!(out.iter().all(|v| *v == 0.0 || v.is_normal()));
        }
        assert!(r.is_idle(), "died out");
        assert!(r.states().all(|v| v == 0.0), "cleared");
    }

    /// The largest step between successive frames from frame `from`.
    fn largest_step(out: &[f32], from: usize) -> f32 {
        out.as_chunks::<2>().0[from..]
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn parameter_changes_do_not_click() {
        let sine = |i: usize| 0.5 * (i as f32 * 150.0 * std::f32::consts::TAU / ENGINE_SAMPLE_RATE as f32).sin();
        let run = |change: bool| {
            let mut r = Reverb::new();
            r.set(Some(room(1.0, 0.6, 8.0)));
            let mut out = Vec::new();
            let mut i = 0;
            for k in 0..400 {
                if change && k % 5 == 0 && k > 50 {
                    // a new room every 50 ms, from a closet to a hall
                    let t = (k as f32 * 0.37).sin() * 0.5 + 0.5;
                    r.set(Some(ReverbParams {
                        level: 0.3 + 0.7 * t,
                        decay: 0.2 + 3.0 * t,
                        predelay: 0.002 + 0.05 * (1.0 - t),
                        damping: 0.3 + 0.6 * t,
                        path: 2.0 + 20.0 * t,
                    }));
                }
                let send: Vec<f32> = (0..480)
                    .map(|_| {
                        i += 1;
                        sine(i)
                    })
                    .collect();
                let mut buf = vec![0.0; 960];
                r.render(&send, &mut buf);
                out.extend_from_slice(&buf);
            }
            out
        };
        let still = largest_step(&run(false), 48_000);
        let changing = largest_step(&run(true), 48_000);
        assert!(changing < still * 1.5, "changing {changing}, still {still}");
    }

    #[test]
    fn off_fades_out_then_stops() {
        let mut r = Reverb::new();
        r.set(Some(room(2.0, 0.7, 10.0)));
        let send = vec![0.3f32; 480];
        let mut out = vec![0.0; 960];
        for _ in 0..50 {
            r.render(&send, &mut out);
        }
        r.set(None);
        let mut tail = Vec::new();
        for _ in 0..10 {
            out.fill(0.0);
            r.render(&send, &mut out);
            tail.extend_from_slice(&out);
        }
        let fade = (FADE_SECS * ENGINE_SAMPLE_RATE as f32) as usize;
        assert!(largest_step(&tail, 0) < 0.05, "fades without a click");
        assert!(tail[2 * fade + 2..].iter().all(|v| *v == 0.0), "silent after the fade");
        assert!(!r.running() && r.is_idle());
        assert!(r.states().all(|v| v == 0.0));
    }

    #[test]
    fn the_lengths_glide_within_the_slew() {
        let mut r = Reverb::new();
        r.set(Some(room(1.0, 0.7, 3.0)));
        let send = vec![0.1f32; BLOCK];
        let mut out = vec![0.0; 2 * BLOCK];
        r.render(&send, &mut out);
        r.set(Some(room(1.0, 0.7, 20.0)));
        let first = r.whole[LINES - 1] as f32;
        let mut last = first;
        for _ in 0..200 {
            r.render(&send, &mut out);
            let now = r.whole[LINES - 1] as f32;
            assert!((now - last).abs() <= MAX_SLEW * BLOCK as f32 + 1.0, "{last} -> {now}");
            last = now;
        }
        assert!(last > first + 50.0, "moving towards the hall: {first} -> {last}");
    }
}
