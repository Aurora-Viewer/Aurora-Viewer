//! 3D sound for headphones: world voices rendered binaurally through
//! head-related impulse responses (HRTF), so that a sound in front, behind,
//! above or below is heard there, not only to the left or the right.
//! Aurora's own (Preferences › Sound, off by default: on speakers it sounds
//! wrong); the Legacy Viewer pans world sounds between the two channels.
//!
//! The responses are the MIT Media Lab's KEMAR measurements (Bill Gardner
//! and Keith Martin, 1994, diffuse-field equalized set), turned by
//! `scripts/generate-hrtf.py` into 368 minimum-phase filter pairs of
//! [`TAPS`] taps at 48 kHz with the delay of each ear
//! (`assets/hrtf/kemar.bin`, built into the executable; origin in
//! `assets/hrtf/README.md`). Azimuths 180 to 360 degrees are the mirror
//! images of 0 to 180, the ears swapped.
//!
//! A direction blends the four nearest measurements (two azimuths on the
//! two nearest rings of elevation): minimum-phase filters start together,
//! so their blend has no comb filtering, and the interaural delay is
//! blended apart and applied by a fractional delay line. Per voice and ear,
//! the mono signal (after the occlusion filter) goes through the delay line
//! then the filter, by blocks of [`BLOCK`] frames; a new direction
//! crossfades from the old filter to the new one over [`CROSSFADE`] frames
//! and the delays glide over the buffer, so that turning the camera neither
//! clicks nor zippers. Nothing allocates or locks in the audio callback:
//! the voices' states are preallocated by the mixer.

use std::sync::LazyLock;

/// Taps of each filter (1.3 ms at 48 kHz: at least 98.8 % of every
/// minimum-phase response's energy; 128 taps change the spectra by less
/// than 0.2 dB).
pub(crate) const TAPS: usize = 64;
/// Frames rendered at a time per voice.
pub(crate) const BLOCK: usize = 64;
/// Frames of the crossfade from one direction's filter to the next.
const CROSSFADE: usize = 64;
/// Longest interaural delay the delay line holds (samples; the table's
/// widest is 40.6, a source at the side).
const MAX_DELAY: f32 = 48.0;
/// Fixed lead of the delay line, so that its four-point interpolation only
/// reads samples already written.
const LEAD: usize = 2;
/// History kept in front of a block of the delay line.
const PAD: usize = MAX_DELAY as usize + LEAD + 2;
/// Fastest glide of a delay (samples per sample, a 2 % pitch change): a
/// jump of the camera moves the interaural delay over ~40 ms instead of a
/// chirp.
const MAX_SLEW: f32 = 0.02;
/// Values this small are zero: no denormals in the filters' histories.
const DENORMAL: f32 = 1.0e-20;
/// Filter taps this small are zero (their products with the histories stay
/// normal).
const TINY_TAP: f32 = 1.0e-15;

static TABLE: &[u8] = include_bytes!("../../../assets/hrtf/kemar.bin");

/// The filter pair of one direction, taps in time order, and the delay of
/// each ear in samples.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hrir {
    pub(crate) delay: [f32; 2],
    pub(crate) taps: [[f32; TAPS]; 2],
}

impl Default for Hrir {
    fn default() -> Self {
        Self {
            delay: [0.0; 2],
            taps: [[0.0; TAPS]; 2],
        }
    }
}

/// A ring of measurements at one elevation.
#[derive(Debug, Clone)]
struct Ring {
    elevation: f32,
    /// Azimuths over the full circle, evenly spaced from 0 (in front).
    count: usize,
    /// Of which the first `stored` (0 to 180 degrees) are in the table.
    stored: usize,
    /// Index of its first measurement.
    first: usize,
}

/// The measured directions.
#[derive(Debug)]
pub(crate) struct Hrtf {
    rings: Vec<Ring>,
    entries: Vec<Hrir>,
}

impl Hrtf {
    /// The table built into the executable, decoded at first use (the
    /// mixer's creation, off the audio callback).
    pub(crate) fn embedded() -> &'static Hrtf {
        static EMBEDDED: LazyLock<Hrtf> = LazyLock::new(|| Hrtf::parse(TABLE).expect("the HRTF table built into the executable is valid"));
        &EMBEDDED
    }

    /// Decodes `kemar.bin` (format in scripts/generate-hrtf.py).
    fn parse(bytes: &[u8]) -> Result<Hrtf, String> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = bytes.get(at..at + n).ok_or("truncated HRTF table")?;
            at += n;
            Ok(s)
        };
        if take(8)? != b"AURHRTF1" {
            return Err("not an HRTF table".into());
        }
        let u16_at = |s: &[u8]| u16::from_le_bytes([s[0], s[1]]);
        let f32_at = |s: &[u8]| f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let head = take(8)?;
        let rate = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let taps = u16_at(&head[4..]) as usize;
        let ring_count = u16_at(&head[6..]) as usize;
        if rate != crate::mixer::ENGINE_SAMPLE_RATE || taps != TAPS || ring_count == 0 {
            return Err(format!("HRTF table at {rate} Hz with {taps} taps"));
        }
        let mut rings = Vec::with_capacity(ring_count);
        let mut first = 0;
        for _ in 0..ring_count {
            let r = take(6)?;
            let elevation = i16::from_le_bytes([r[0], r[1]]) as f32;
            let (count, stored) = (u16_at(&r[2..]) as usize, u16_at(&r[4..]) as usize);
            if count == 0 || stored != count / 2 + 1 || rings.last().is_some_and(|l: &Ring| l.elevation >= elevation) {
                return Err(format!("HRTF ring at {elevation} degrees"));
            }
            rings.push(Ring {
                elevation,
                count,
                stored,
                first,
            });
            first += stored;
        }
        let mut entries = Vec::with_capacity(first);
        for _ in 0..first {
            let mut h = Hrir::default();
            let d = take(8)?;
            h.delay = [f32_at(d), f32_at(&d[4..])];
            if h.delay.iter().any(|d| !(0.0..=MAX_DELAY).contains(d)) {
                return Err(format!("HRTF delay {:?}", h.delay));
            }
            for ear in &mut h.taps {
                for (t, s) in ear.iter_mut().zip(take(TAPS * 4)?.as_chunks::<4>().0) {
                    let v = f32::from_le_bytes(*s);
                    *t = if v.is_finite() && v.abs() >= TINY_TAP { v } else { 0.0 };
                }
            }
            entries.push(h);
        }
        if at != bytes.len() {
            return Err("trailing bytes after the HRTF table".into());
        }
        Ok(Hrtf { rings, entries })
    }

    /// The measurement `j` of a ring (over the full circle), and whether its
    /// ears are swapped (a mirror image).
    fn measured(&self, ring: &Ring, j: usize) -> (&Hrir, bool) {
        if j < ring.stored {
            (&self.entries[ring.first + j], false)
        } else {
            (&self.entries[ring.first + ring.count - j], true)
        }
    }

    /// The filters and delays of a direction in the listener's frame (x to
    /// the right, y ahead, z up; any length), blended from the four nearest
    /// measurements. Below the lowest ring (-40 degrees) the lowest one is
    /// used. No allocation.
    pub(crate) fn lookup(&self, direction: [f32; 3], out: &mut Hrir) {
        let [x, y, z] = direction;
        let len = (x * x + y * y + z * z).sqrt();
        let (x, y, z) = if len > 1.0e-6 && len.is_finite() {
            (x / len, y / len, z / len)
        } else {
            (0.0, 1.0, 0.0)
        };
        let elevation = z.clamp(-1.0, 1.0).asin().to_degrees();
        let azimuth = x.atan2(y).to_degrees().rem_euclid(360.0);
        let (lowest, highest) = (&self.rings[0], &self.rings[self.rings.len() - 1]);
        let elevation = elevation.clamp(lowest.elevation, highest.elevation);
        let below = self.rings.iter().rposition(|r| r.elevation <= elevation).unwrap_or(0);
        let above = (below + 1).min(self.rings.len() - 1);
        let t = if above == below {
            0.0
        } else {
            (elevation - self.rings[below].elevation) / (self.rings[above].elevation - self.rings[below].elevation)
        };
        *out = Hrir::default();
        for (ring, weight) in [(&self.rings[below], 1.0 - t), (&self.rings[above], t)] {
            if weight <= 0.0 {
                continue;
            }
            let a = azimuth / 360.0 * ring.count as f32;
            let base = a.floor();
            let u = a - base;
            let j0 = base as usize % ring.count;
            let j1 = (j0 + 1) % ring.count;
            for (j, w) in [(j0, weight * (1.0 - u)), (j1, weight * u)] {
                if w <= 0.0 {
                    continue;
                }
                let (h, swapped) = self.measured(ring, j);
                for ear in 0..2 {
                    let from = if swapped { 1 - ear } else { ear };
                    out.delay[ear] += w * h.delay[from];
                    for (o, s) in out.taps[ear].iter_mut().zip(&h.taps[from]) {
                        *o += w * s;
                    }
                }
            }
        }
        for t in out.taps.iter_mut().flatten() {
            if t.abs() < TINY_TAP {
                *t = 0.0;
            }
        }
    }
}

/// Dot product of a filter (reversed) and a window of history, in eight
/// lanes so that it vectorises.
#[inline(always)]
fn dot(filter: &[f32; TAPS], window: &[f32]) -> f32 {
    let window = &window[..TAPS];
    let mut acc = [0.0f32; 8];
    for (f, w) in filter.as_chunks::<8>().0.iter().zip(window.as_chunks::<8>().0) {
        for k in 0..8 {
            acc[k] += f[k] * w[k];
        }
    }
    (acc[0] + acc[4]) + (acc[1] + acc[5]) + ((acc[2] + acc[6]) + (acc[3] + acc[7]))
}

/// The binaural state of one world voice: its delay line, the two ears'
/// histories, filters and delays.
pub(crate) struct Binaural {
    /// The mono input: [`PAD`] samples of history, then the block.
    input: [f32; PAD + BLOCK],
    /// Each ear's delayed signal: `TAPS - 1` samples of history, then the
    /// block.
    ear: [[f32; TAPS - 1 + BLOCK]; 2],
    /// Filters in use and the next ones, reversed (newest sample last).
    filter: [[f32; TAPS]; 2],
    next: [[f32; TAPS]; 2],
    /// Frames into the crossfade to `next`, `None` when there is none.
    fade: Option<usize>,
    delay: [f32; 2],
    delay_target: [f32; 2],
    /// Glide of each delay per sample over this buffer.
    delay_step: [f32; 2],
}

impl Binaural {
    pub(crate) fn new() -> Self {
        Self {
            input: [0.0; PAD + BLOCK],
            ear: [[0.0; TAPS - 1 + BLOCK]; 2],
            filter: [[0.0; TAPS]; 2],
            next: [[0.0; TAPS]; 2],
            fade: None,
            delay: [0.0; 2],
            delay_target: [0.0; 2],
            delay_step: [0.0; 2],
        }
    }

    fn reversed(h: &Hrir, out: &mut [[f32; TAPS]; 2]) {
        for (o, t) in out.iter_mut().zip(&h.taps) {
            for (a, b) in o.iter_mut().zip(t.iter().rev()) {
                *a = *b;
            }
        }
    }

    fn clamp_delay(d: f32) -> f32 {
        if d.is_finite() { d.clamp(0.0, MAX_DELAY) } else { 0.0 }
    }

    /// Starts afresh at a direction: silent histories, its filters and
    /// delays at once.
    pub(crate) fn reset(&mut self, h: &Hrir) {
        self.input.fill(0.0);
        for e in &mut self.ear {
            e.fill(0.0);
        }
        Self::reversed(h, &mut self.filter);
        self.next = self.filter;
        self.fade = None;
        self.delay = h.delay.map(Self::clamp_delay);
        self.delay_target = self.delay;
        self.delay_step = [0.0; 2];
    }

    /// Moves to a new direction: a crossfade to its filters from where the
    /// current one stands, and its delays as the next targets.
    pub(crate) fn steer(&mut self, h: &Hrir) {
        if let Some(pos) = self.fade.take() {
            // the blend reached so far becomes the filter faded from (the
            // same output: a blend of outputs is the blend of the filters)
            let t = pos as f32 / CROSSFADE as f32;
            for (f, n) in self.filter.iter_mut().flatten().zip(self.next.iter().flatten()) {
                *f += (n - *f) * t;
                if f.abs() < TINY_TAP {
                    *f = 0.0;
                }
            }
        }
        Self::reversed(h, &mut self.next);
        self.fade = Some(0);
        self.delay_target = h.delay.map(Self::clamp_delay);
    }

    /// Before a buffer of `frames`: the delays glide to their targets over
    /// it, no faster than [`MAX_SLEW`].
    pub(crate) fn begin(&mut self, frames: usize) {
        for e in 0..2 {
            let step = (self.delay_target[e] - self.delay[e]) / frames.max(1) as f32;
            self.delay_step[e] = step.clamp(-MAX_SLEW, MAX_SLEW);
        }
    }

    /// Where the next block's mono samples go (`n` up to [`BLOCK`]).
    pub(crate) fn input(&mut self, n: usize) -> &mut [f32] {
        &mut self.input[PAD..PAD + n]
    }

    /// Renders the `n` samples written by [`Binaural::input`], adding them
    /// to the interleaved stereo `out` (`2 n` samples).
    pub(crate) fn render(&mut self, n: usize, out: &mut [f32]) {
        debug_assert!(n <= BLOCK && out.len() >= 2 * n);
        for s in &mut self.input[PAD..PAD + n] {
            if s.abs() < DENORMAL {
                *s = 0.0;
            }
        }
        for e in 0..2 {
            // the delay line, four-point (Catmull-Rom) interpolation
            let (target, step) = (self.delay_target[e], self.delay_step[e]);
            let mut d = self.delay[e];
            for i in 0..n {
                d = if step > 0.0 {
                    (d + step).min(target)
                } else if step < 0.0 {
                    (d + step).max(target)
                } else {
                    d
                };
                let p = (PAD + i - LEAD) as f32 - d;
                let base = p.floor();
                let fr = p - base;
                let j = base as usize;
                let (xm, x0, x1, x2) = (self.input[j - 1], self.input[j], self.input[j + 1], self.input[j + 2]);
                let v = x0 + 0.5 * fr * (x1 - xm + fr * (2.0 * xm - 5.0 * x0 + 4.0 * x1 - x2 + fr * (3.0 * (x0 - x1) + x2 - xm)));
                self.ear[e][TAPS - 1 + i] = if v.abs() < DENORMAL { 0.0 } else { v };
            }
            self.delay[e] = d;
        }
        // the filters, crossfading to the next ones
        let fade = self.fade;
        for i in 0..n {
            let t = fade.map(|pos| ((pos + i + 1) as f32 / CROSSFADE as f32).min(1.0));
            for e in 0..2 {
                let window = &self.ear[e][i..i + TAPS];
                let mut y = dot(&self.filter[e], window);
                if let Some(t) = t {
                    y += (dot(&self.next[e], window) - y) * t;
                }
                out[2 * i + e] += y;
            }
        }
        if let Some(pos) = fade {
            if pos + n >= CROSSFADE {
                self.filter = self.next;
                self.fade = None;
            } else {
                self.fade = Some(pos + n);
            }
        }
        self.input.copy_within(n..n + PAD, 0);
        for e in &mut self.ear {
            e.copy_within(n..n + TAPS - 1, 0);
        }
    }

    #[cfg(test)]
    pub(crate) fn states(&self) -> impl Iterator<Item = f32> + '_ {
        self.input.iter().chain(self.ear.iter().flatten()).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hrir(dir: [f32; 3]) -> Hrir {
        let mut h = Hrir::default();
        Hrtf::embedded().lookup(dir, &mut h);
        h
    }

    fn from_angles(azimuth: f32, elevation: f32) -> [f32; 3] {
        let (a, e) = (azimuth.to_radians(), elevation.to_radians());
        [a.sin() * e.cos(), a.cos() * e.cos(), e.sin()]
    }

    /// Magnitude (dB) of a filter at `freq`.
    fn db(taps: &[f32; TAPS], freq: f32) -> f32 {
        let w = std::f32::consts::TAU * freq / crate::mixer::ENGINE_SAMPLE_RATE as f32;
        let (re, im) = taps.iter().enumerate().fold((0.0, 0.0), |(re, im), (k, t)| {
            (re + t * (w * k as f32).cos(), im - t * (w * k as f32).sin())
        });
        10.0 * (re * re + im * im).max(1e-20).log10()
    }

    #[test]
    fn the_embedded_table_holds_the_kemar_sphere() {
        let h = Hrtf::embedded();
        assert_eq!(h.rings.len(), 14);
        assert_eq!(h.rings.iter().map(|r| r.count).sum::<usize>(), 710, "MIT KEMAR's 710 directions");
        assert_eq!(h.entries.len(), 368);
        assert_eq!((h.rings[0].elevation, h.rings[13].elevation), (-40.0, 90.0));
        assert!(Hrtf::parse(&TABLE[..TABLE.len() - 1]).is_err());
        assert!(Hrtf::parse(b"AURHRTF2").is_err());
        // a source on the right: the right ear first and louder
        let right = hrir([1.0, 0.0, 0.0]);
        assert_eq!(right.delay[1], 0.0);
        assert!(right.delay[0] > 30.0, "interaural delay {:?}", right.delay);
        let energy = |t: &[f32; TAPS]| t.iter().map(|v| v * v).sum::<f32>();
        assert!(energy(&right.taps[1]) > 4.0 * energy(&right.taps[0]));
        // ahead, the level of the panned path's centre (1 per ear), roughly
        let ahead = hrir([0.0, 1.0, 0.0]);
        assert!((0.6..1.6).contains(&energy(&ahead.taps[0])), "{}", energy(&ahead.taps[0]));
    }

    #[test]
    fn lookup_is_continuous_over_the_sphere() {
        let max_change = |a: &Hrir, b: &Hrir| {
            let taps = a.taps.iter().flatten().zip(b.taps.iter().flatten()).map(|(x, y)| (x - y).abs());
            let delays = a.delay.iter().zip(&b.delay).map(|(x, y)| (x - y).abs() / 10.0);
            taps.chain(delays).fold(0.0f32, f32::max)
        };
        let peak = Hrtf::embedded()
            .entries
            .iter()
            .flat_map(|h| h.taps.iter().flatten())
            .fold(0.0f32, |m, v| m.max(v.abs()));
        // around the head at every ring and between rings, across 360 -> 0,
        // and over the head from below
        let mut paths: Vec<Vec<[f32; 3]>> = (-50..=90)
            .step_by(5)
            .map(|e| (0..=3600).map(|a| from_angles(a as f32 * 0.1, e as f32)).collect())
            .collect();
        for a in [0.0, 37.0, 90.0, 180.0, 300.0] {
            paths.push((-500..=900).map(|e| from_angles(a, e as f32 * 0.1)).collect());
        }
        for path in paths {
            let mut prev = hrir(path[0]);
            for d in &path[1..] {
                let h = hrir(*d);
                let change = max_change(&prev, &h);
                assert!(change < peak * 0.08, "jump {change} at {d:?}");
                prev = h;
            }
        }
        // the same direction on both sides of the seam
        assert!(max_change(&hrir(from_angles(359.999, 3.0)), &hrir(from_angles(0.0, 3.0))) < peak * 1e-3);
        // straight up and straight down have one answer whatever the azimuth
        assert!(max_change(&hrir(from_angles(10.0, 90.0)), &hrir(from_angles(250.0, 90.0))) < 1e-4);
        // below the lowest ring, the lowest ring
        assert!(max_change(&hrir(from_angles(10.0, -70.0)), &hrir(from_angles(10.0, -40.0))) < 1e-4);
        // no direction (a source at the head) is ahead
        assert_eq!(hrir([0.0; 3]), hrir([0.0, 1.0, 0.0]));
        assert_eq!(hrir([f32::NAN, 0.0, 1.0]), hrir([0.0, 1.0, 0.0]));
    }

    #[test]
    fn left_and_right_are_mirror_images() {
        for (a, e) in [
            (0.0, 0.0),
            (30.0, 0.0),
            (90.0, 0.0),
            (135.0, 20.0),
            (61.0, -33.0),
            (170.0, 55.0),
            (12.0, 84.0),
        ] {
            let r = hrir(from_angles(a, e));
            let l = hrir(from_angles(360.0 - a, e));
            for ear in 0..2 {
                assert!((r.delay[ear] - l.delay[1 - ear]).abs() < 1e-3, "delay at {a} {e}");
                for (x, y) in r.taps[ear].iter().zip(&l.taps[1 - ear]) {
                    assert!((x - y).abs() < 1e-5, "taps at {a} {e}");
                }
            }
        }
    }

    #[test]
    fn front_back_and_above_sound_different() {
        let ahead = hrir(from_angles(0.0, 0.0));
        let behind = hrir(from_angles(180.0, 0.0));
        let above = hrir(from_angles(0.0, 60.0));
        // same interaural delay ahead and behind: the spectrum tells them
        // apart (the pinna), most between 5 and 12 kHz
        assert!((ahead.delay[0] - ahead.delay[1]).abs() < 0.5 && (behind.delay[0] - behind.delay[1]).abs() < 0.5);
        let spread = |a: &Hrir, b: &Hrir| {
            (5_000..=12_000)
                .step_by(250)
                .map(|f| (db(&a.taps[0], f as f32) - db(&b.taps[0], f as f32)).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(spread(&ahead, &behind) > 6.0, "front / back {}", spread(&ahead, &behind));
        assert!(spread(&ahead, &above) > 4.0, "front / above {}", spread(&ahead, &above));
        // the low notes pass alike
        assert!((db(&ahead.taps[0], 150.0) - db(&behind.taps[0], 150.0)).abs() < 3.0);
    }

    /// Renders `signal` through a state steered by `dirs` (one per block),
    /// interleaved stereo.
    fn render(signal: &[f32], dirs: impl Fn(usize) -> [f32; 3], block: usize) -> Vec<f32> {
        let mut b = Binaural::new();
        let mut h = Hrir::default();
        Hrtf::embedded().lookup(dirs(0), &mut h);
        b.reset(&h);
        let mut out = vec![0.0; signal.len() * 2];
        for (k, (chunk, o)) in signal.chunks(block).zip(out.chunks_mut(block * 2)).enumerate() {
            Hrtf::embedded().lookup(dirs(k), &mut h);
            if k > 0 {
                b.steer(&h);
            }
            b.begin(chunk.len());
            for (s, o) in chunk.chunks(BLOCK).zip(o.chunks_mut(BLOCK * 2)) {
                b.input(s.len()).copy_from_slice(s);
                b.render(s.len(), o);
            }
        }
        out
    }

    fn max_step(out: &[f32], from: usize) -> f32 {
        let frames = out.as_chunks::<2>().0;
        frames[from..]
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn turning_crossfades_without_clicks() {
        let sine: Vec<f32> = (0..48_000)
            .map(|i| 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect();
        // the steepest a still source makes, whatever its direction
        let still = (0..8)
            .map(|k| max_step(&render(&sine, |_| from_angles(k as f32 * 45.0, 0.0), 480), 200))
            .fold(0.0f32, f32::max);
        // the direction jumping a quarter turn every 10 ms, and spinning
        // fast in buffers of odd sizes
        let jumping = render(&sine, |k| from_angles((k % 4) as f32 * 90.0, (k % 3) as f32 * 30.0), 480);
        let spinning = render(&sine, |k| from_angles(k as f32 * 7.0, 10.0), 333);
        for out in [&jumping, &spinning] {
            let step = max_step(out, 200);
            assert!(step < still * 1.25, "max step {step}, a still source {still}");
        }
    }

    #[test]
    fn silence_leaves_no_denormals() {
        let mut signal = vec![0.0f32; 96_000];
        signal[0] = 1.0;
        signal[1] = 1.0e-19;
        let out = render(&signal, |k| from_angles(k as f32 * 13.0, 0.0), 480);
        assert!(out.iter().all(|s| *s == 0.0 || s.is_normal()));
        let mut b = Binaural::new();
        b.reset(&hrir([0.0, 1.0, 0.0]));
        b.begin(BLOCK);
        b.input(BLOCK).fill(1.0e-30);
        let mut o = [0.0; 2 * BLOCK];
        b.render(BLOCK, &mut o);
        assert!(b.states().all(|s| s == 0.0 || s.is_normal()));
        assert!(o.iter().all(|s| *s == 0.0));
    }
}
