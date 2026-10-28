//! Ambient wind noise: pink noise through a resonant low-pass filter whose
//! centre frequency, gain and stereo pan follow the wind felt by the
//! listener.
//!
//! Port of LLWindGen (indra/llaudio/llwindgen.h) and of
//! LLAudioEngine::mapWindVecToGain / mapWindVecToPitch / mapWindVecToPan
//! with the target computation of LLAudioEngine_FMODSTUDIO::updateWind
//! (indra/llaudio/llaudioengine.cpp, llaudioengine_fmodstudio.cpp),
//! Copyright (C) Linden Research, Inc., originally LGPL 2.1. The Legacy
//! Viewer refreshes the targets every 0.1 s (LL_WIND_UPDATE_INTERVAL); here
//! they can change every frame, the generator smoothing them per sample
//! either way.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU32, Ordering};

/// What the generator moves towards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindTarget {
    /// Linear gain before the Ambient channel's volume.
    pub gain: f32,
    /// Centre frequency of the resonant filter (Hz).
    pub freq: f32,
    /// Share of the signal sent to the right speaker (0 = all left).
    pub pan_right: f32,
}

impl WindTarget {
    /// LLWindGen's initial targets.
    pub const SILENT: Self = Self {
        gain: 0.0,
        freq: 100.0,
        pan_right: 0.5,
    };

    /// LLAudioEngine_FMODSTUDIO::updateWind: `wind` is the wind blowing on
    /// the listener in its own frame, in Second Life axes (+X forward,
    /// +Y left, +Z up), in m/s; `max_gain` (0..=1) fades the whole sound in
    /// and out (mMaxWindGain, without the volume channels that the mixer
    /// applies).
    pub fn from_wind(wind: [f32; 3], max_gain: f32) -> Self {
        // To the DS3D / OpenAL orientation: +X right, +Y up, +Z backwards.
        let v = [-wind[1], wind[2], -wind[0]];
        let gain = map_gain(v);
        let pitch = 1.0 + map_pitch(v);
        Self {
            gain: (gain * f64::from(max_gain.clamp(0.0, 1.0))) as f32,
            freq: (80.0 * pitch.powf(2.5 * (gain + 1.0))) as f32,
            pan_right: map_pan(v) as f32,
        }
    }
}

/// LLAudioEngine::mapWindVecToGain: full gain from 20 m/s.
fn map_gain(v: [f32; 3]) -> f64 {
    let speed = f64::from(v[0]).hypot(f64::from(v[1])).hypot(f64::from(v[2]));
    if speed.is_finite() { speed.min(20.0) / 20.0 } else { 0.0 }
}

/// Angle between the wind and the listener's right axis, on 0..=1
/// (LLVector3::normVec leaves a null vector null: a right angle).
fn right_angle(v: [f32; 3]) -> f64 {
    let len = f64::from(v[0]).hypot(f64::from(v[1])).hypot(f64::from(v[2]));
    let cos = if len > 1e-6 && len.is_finite() {
        f64::from(v[0]) / len
    } else {
        0.0
    };
    cos.clamp(-1.0, 1.0).acos() / std::f64::consts::PI
}

/// LLAudioEngine::mapWindVecToPitch: highest (0.5) for a wind from the
/// front or the back, 0 from either side.
fn map_pitch(v: [f32; 3]) -> f64 {
    let theta = right_angle(v);
    if theta > 0.5 { 1.0 - theta } else { theta }
}

/// LLAudioEngine::mapWindVecToPan.
fn map_pan(v: [f32; 3]) -> f64 {
    right_angle(v)
}

/// Targets written by the viewer, read by the audio callback.
pub(crate) struct SharedWind {
    gain: AtomicU32,
    freq: AtomicU32,
    pan_right: AtomicU32,
}

impl SharedWind {
    pub(crate) fn new() -> Self {
        let t = WindTarget::SILENT;
        Self {
            gain: AtomicU32::new(t.gain.to_bits()),
            freq: AtomicU32::new(t.freq.to_bits()),
            pan_right: AtomicU32::new(t.pan_right.to_bits()),
        }
    }

    pub(crate) fn set(&self, t: WindTarget) {
        let clean = |v: f32, lo: f32, hi: f32, default: f32| if v.is_finite() { v.clamp(lo, hi) } else { default };
        let s = WindTarget::SILENT;
        self.gain.store(clean(t.gain, 0.0, 1.0, 0.0).to_bits(), Ordering::Relaxed);
        self.freq.store(clean(t.freq, 20.0, 2_000.0, s.freq).to_bits(), Ordering::Relaxed);
        self.pan_right
            .store(clean(t.pan_right, 0.0, 1.0, s.pan_right).to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> WindTarget {
        WindTarget {
            gain: f32::from_bits(self.gain.load(Ordering::Relaxed)),
            freq: f32::from_bits(self.freq.load(Ordering::Relaxed)),
            pan_right: f32::from_bits(self.pan_right.load(Ordering::Relaxed)),
        }
    }
}

/// The noise is synthesised at half the output rate and interpolated
/// (mSubSamples).
const SUB_SAMPLES: usize = 2;
/// Bandwidth of the resonant filter (mFilterBandWidth, Hz).
const FILTER_BANDWIDTH: f32 = 50.0;

/// LLWindGen<F32>: the generator, owned by the mixer.
pub(crate) struct WindGen {
    sample_period: f32,
    b2: f32,
    buf: [f32; 3],
    y0: f32,
    y1: f32,
    gain: f32,
    freq: f32,
    pan_right: f32,
    last: f32,
    /// Step towards the last synthesised sample, and the output frames of
    /// that step still to write: a buffer that ends inside the step carries
    /// it into the next one. LLWindGen restarts at every buffer, which FMOD
    /// sizes in whole steps; ours have any length (the 44.1 kHz resampler
    /// asks odd ones).
    delta: f32,
    pending: usize,
    rng: u32,
}

impl WindGen {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let sample_period = SUB_SAMPLES as f32 / sample_rate.max(1) as f32;
        let s = WindTarget::SILENT;
        Self {
            sample_period,
            b2: (-TAU * FILTER_BANDWIDTH * sample_period).exp(),
            buf: [0.0; 3],
            y0: 0.0,
            y1: 0.0,
            gain: s.gain,
            freq: s.freq,
            pan_right: s.pan_right,
            last: 0.0,
            delta: 0.0,
            pending: 0,
            rng: 0x9e37_79b9,
        }
    }

    /// `ll_frand() - 0.5`: white noise on -0.5..0.5 (xorshift32).
    fn white(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 * (1.0 / (1u32 << 24) as f32) - 0.5
    }

    /// Resonant filter coefficients (a0, b1) for the current frequency.
    fn coefficients(&self) -> (f32, f32) {
        let b1 = (-4.0 * self.b2) / (1.0 + self.b2) * (TAU * self.freq * self.sample_period).cos();
        let a0 = (1.0 - self.b2) * (1.0 - (b1 * b1) / (4.0 * self.b2)).max(0.0).sqrt();
        (a0, b1)
    }

    /// LLWindGen::windGenerate, added to the interleaved stereo `out`.
    /// Nothing is computed while the sound is and stays silent.
    pub(crate) fn mix_into(&mut self, target: WindTarget, out: &mut [f32]) {
        if self.gain < 1e-5 && target.gain <= 0.0 {
            self.gain = 0.0;
            self.last = 0.0;
            self.pending = 0;
            return;
        }
        // If the frequency is not changing much, no need to interpolate it
        // in the inner loop.
        let interp_freq = (target.freq - self.freq).abs() >= self.freq * 0.112;
        let (mut a0, mut b1) = (0.0, 0.0);
        if !interp_freq {
            self.freq = target.freq;
            (a0, b1) = self.coefficients();
        }
        let mut frames = out.as_chunks_mut::<2>().0.iter_mut();
        loop {
            // Finish the step in progress before synthesising the next
            // sample; synthesise only for frames this buffer still has, as
            // windGenerate's `while (numsamples)`: a sample computed past the
            // end would be lost, the filter a step ahead of the output, a
            // seam heard as a crackle at every buffer.
            while self.pending > 0 {
                let Some(frame) = frames.next() else {
                    return;
                };
                self.pending -= 1;
                self.last += self.delta;
                let right = self.last * self.pan_right;
                frame[0] += self.last - right;
                frame[1] += right;
            }
            if frames.len() == 0 {
                return;
            }
            let white = self.white();
            // Pinking filter (Paul Kellet's economy method, the PKE
            // coefficients of llwindgen.h).
            self.buf[0] = self.buf[0] * 0.99765 + white * 0.099_046;
            self.buf[1] = self.buf[1] * 0.963 + white * 0.296_516_4;
            self.buf[2] = self.buf[2] * 0.57 + white * 1.052_691_3;
            let pink = self.buf[0] + self.buf[1] + self.buf[2] + white * 0.1848;

            if interp_freq {
                self.freq = 0.999 * self.freq + 0.001 * target.freq;
                (a0, b1) = self.coefficients();
            }
            // Resonant low-pass filter on the pink noise.
            let filtered = a0 * pink - b1 * self.y0 - self.b2 * self.y1;
            self.y1 = self.y0;
            self.y0 = filtered;

            self.gain = 0.999 * self.gain + 0.001 * target.gain;
            self.pan_right = 0.999 * self.pan_right + 0.001 * target.pan_right;
            let sample = filtered * self.gain;

            // Interpolate between the synthesised samples.
            self.delta = (sample - self.last) / SUB_SAMPLES as f32;
            self.pending = SUB_SAMPLES;
        }
    }

    #[cfg(test)]
    fn gain(&self) -> f32 {
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn rms(buf: &[f32], channel: usize) -> f32 {
        let samples: Vec<f32> = buf.as_chunks::<2>().0.iter().map(|f| f[channel]).collect();
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn targets_follow_the_legacy_mapping() {
        // Still air: silent, pan centred, the lowest frequency.
        let t = WindTarget::from_wind([0.0; 3], 1.0);
        assert_eq!(t.gain, 0.0);
        assert!((t.pan_right - 0.5).abs() < 1e-6);
        // 20 m/s or more is full gain; the fade scales it.
        assert!((WindTarget::from_wind([-30.0, 0.0, 0.0], 1.0).gain - 1.0).abs() < 1e-6);
        assert!((WindTarget::from_wind([-10.0, 0.0, 0.0], 0.5).gain - 0.25).abs() < 1e-6);
        // Head wind: pan centred, pitch at its highest: 80 * 1.5^(2.5 * 1.5).
        let head = WindTarget::from_wind([-10.0, 0.0, 0.0], 1.0);
        assert!((head.pan_right - 0.5).abs() < 1e-6);
        assert!((head.freq - 80.0 * 1.5f32.powf(3.75)).abs() < 0.01, "{}", head.freq);
        // Wind from the left side (blowing towards -Y): heard on the left,
        // with no pitch rise.
        let side = WindTarget::from_wind([0.0, -10.0, 0.0], 1.0);
        assert!(side.pan_right < 1e-6, "{side:?}");
        assert!((side.freq - 80.0).abs() < 1e-3);
        let right = WindTarget::from_wind([0.0, 10.0, 0.0], 1.0);
        assert!((right.pan_right - 1.0).abs() < 1e-6, "{right:?}");
        // Garbage in, silence out.
        assert_eq!(WindTarget::from_wind([f32::NAN, 0.0, 0.0], 1.0).gain, 0.0);
    }

    #[test]
    fn silent_wind_costs_nothing_and_adds_nothing() {
        let mut g = WindGen::new(RATE);
        let mut out = vec![0.25; 960];
        g.mix_into(WindTarget::SILENT, &mut out);
        assert!(out.iter().all(|&s| s == 0.25));
        assert_eq!(g.rng, 0x9e37_79b9, "no noise drawn while silent");
    }

    #[test]
    fn wind_fades_in_smoothly_to_a_sane_level() {
        let mut g = WindGen::new(RATE);
        let target = WindTarget::from_wind([-20.0, 0.0, 0.0], 1.0);
        let mut out = vec![0.0; RATE as usize * 2];
        g.mix_into(target, &mut out);
        // No click at the start: the gain ramps from 0.
        assert!(out[..64].iter().all(|s| s.abs() < 0.01), "{:?}", &out[..8]);
        assert!((g.gain() - 1.0).abs() < 1e-3);
        let tail = &out[out.len() / 2..];
        let (l, r) = (rms(tail, 0), rms(tail, 1));
        assert!((0.01..0.5).contains(&l), "rms {l}");
        assert!((l - r).abs() < l * 0.01, "centred: {l} {r}");
        assert!(tail.iter().all(|s| s.abs() < 1.0), "no clipping at full gain");
    }

    #[test]
    fn pan_moves_the_noise_to_one_side() {
        let mut g = WindGen::new(RATE);
        let target = WindTarget::from_wind([0.0, 10.0, 0.0], 1.0);
        let mut out = vec![0.0; RATE as usize * 2];
        g.mix_into(target, &mut out);
        let tail = &out[out.len() / 2..];
        assert!(rms(tail, 0) < rms(tail, 1) * 0.01, "{} {}", rms(tail, 0), rms(tail, 1));
    }

    #[test]
    fn a_frequency_jump_glides() {
        let mut g = WindGen::new(RATE);
        let mut out = vec![0.0; 9_600];
        g.mix_into(WindTarget::from_wind([-5.0, 0.0, 0.0], 1.0), &mut out);
        let start = g.freq;
        g.mix_into(
            WindTarget {
                freq: 600.0,
                ..WindTarget::from_wind([-5.0, 0.0, 0.0], 1.0)
            },
            &mut out[..20],
        );
        assert!(g.freq > start && g.freq < 600.0 * 0.9, "{start} -> {}", g.freq);
    }

    #[test]
    fn buffer_boundaries_leave_no_seam() {
        // The same sound whatever the buffers: in one piece, in the even
        // buffers of a 48 kHz device, and in the odd and tiny ones the
        // 44.1 kHz resampler asks. (Starting at the target frequency: a
        // glide is decided per buffer, as in LLWindGen.)
        let target = WindTarget::from_wind([-6.0, 1.0, 0.0], 1.0);
        let render = |sizes: &[usize]| {
            let mut g = WindGen::new(RATE);
            g.freq = target.freq;
            let mut out = vec![0.0; RATE as usize * 2];
            let mut pos = 0;
            for &frames in sizes.iter().cycle() {
                if pos == out.len() {
                    break;
                }
                let end = (pos + frames * 2).min(out.len());
                g.mix_into(target, &mut out[pos..end]);
                pos = end;
            }
            out
        };
        let whole = render(&[RATE as usize]);
        for sizes in [&[480][..], &[485], &[484, 485, 1, 486]] {
            let pieces = render(sizes);
            let worst = whole.iter().zip(&pieces).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(worst < 1e-6, "{sizes:?}: {worst}");
        }
    }

    #[test]
    fn shared_targets_are_sanitised() {
        let s = SharedWind::new();
        s.set(WindTarget {
            gain: 4.0,
            freq: f32::NAN,
            pan_right: -1.0,
        });
        let t = s.get();
        assert_eq!(t.gain, 1.0);
        assert_eq!(t.freq, WindTarget::SILENT.freq);
        assert_eq!(t.pan_right, 0.0);
    }

    #[test]
    fn right_angle_handles_null_wind() {
        assert!((right_angle([0.0; 3]) - 0.5).abs() < 1e-9);
        assert!(right_angle([1.0, 0.0, 0.0]).abs() < 1e-9);
        assert!((right_angle([-1.0, 0.0, 0.0]) - 1.0).abs() < 1e-9);
    }
}
