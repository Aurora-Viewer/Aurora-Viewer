//! Streaming sample-rate converter (4-point, 3rd-order Hermite interpolation).
//!
//! Good enough for internet radio and UI sounds, cheap, and with no latency
//! beyond two frames.

/// Interleaved multi-channel streaming resampler.
pub(crate) struct Resampler {
    channels: usize,
    step: f64,
    /// Read position, in frames, into `buf`. Always >= 1 so `pos - 1` exists.
    pos: f64,
    buf: Vec<f32>,
}

impl Resampler {
    pub(crate) fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            step: f64::from(in_rate.max(1)) / f64::from(out_rate.max(1)),
            pos: 1.0,
            // One frame of silent history so interpolation can start at once.
            buf: vec![0.0; channels],
        }
    }

    /// Queues interleaved input frames.
    pub(crate) fn push(&mut self, input: &[f32]) {
        let whole = input.len() - input.len() % self.channels;
        self.buf.extend_from_slice(&input[..whole]);
    }

    /// Appends up to `max_frames` output frames to `out`; returns the number
    /// of frames produced (fewer when more input is needed).
    pub(crate) fn pull(&mut self, out: &mut Vec<f32>, max_frames: usize) -> usize {
        let ch = self.channels;
        let frames = self.buf.len() / ch;
        let mut produced = 0;
        while produced < max_frames {
            let i = self.pos as usize;
            if i + 2 >= frames {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            for c in 0..ch {
                let xm1 = self.buf[(i - 1) * ch + c];
                let x0 = self.buf[i * ch + c];
                let x1 = self.buf[(i + 1) * ch + c];
                let x2 = self.buf[(i + 2) * ch + c];
                out.push(hermite(xm1, x0, x1, x2, t));
            }
            self.pos += self.step;
            produced += 1;
        }
        let i = self.pos as usize;
        if i > 1 {
            let drop = (i - 1).min(frames.saturating_sub(1));
            self.buf.drain(..drop * ch);
            self.pos -= drop as f64;
        }
        produced
    }

    /// Convenience: push everything and pull everything available.
    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.push(input);
        self.pull(out, usize::MAX);
    }
}

#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c0 = x0;
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + c0
}

/// Converts interleaved audio with `channels` channels to interleaved stereo,
/// appending to `out`. Mono is duplicated; for more than two channels the
/// front left/right pair is kept.
pub(crate) fn to_stereo(input: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => {
            out.reserve(input.len() * 2);
            for &s in input {
                out.push(s);
                out.push(s);
            }
        }
        2 => out.extend_from_slice(&input[..input.len() & !1]),
        n => {
            for frame in input.chunks_exact(n) {
                out.push(frame[0]);
                out.push(frame[1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_length_matches_ratio() {
        let mut r = Resampler::new(44_100, 48_000, 2);
        let input = vec![0.0f32; 44_100 * 2];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let frames = out.len() / 2;
        assert!((47_990..=48_000).contains(&frames), "{frames}");
    }

    #[test]
    fn chunked_equals_whole() {
        let input: Vec<f32> = (0..4000).map(|i| ((i as f32) * 0.01).sin()).collect();
        let mut whole = Vec::new();
        Resampler::new(22_050, 48_000, 1).process(&input, &mut whole);
        let mut chunked = Vec::new();
        let mut r = Resampler::new(22_050, 48_000, 1);
        for c in input.chunks(37) {
            r.process(c, &mut chunked);
        }
        assert_eq!(whole.len(), chunked.len());
        for (a, b) in whole.iter().zip(&chunked) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn preserves_dc_and_sine_frequency() {
        let mut r = Resampler::new(48_000, 48_000, 1);
        let mut out = Vec::new();
        r.process(&[0.25; 100], &mut out);
        assert!(out[10..].iter().all(|&v| (v - 0.25).abs() < 1e-6));

        // 1 kHz sine at 44.1 kHz -> 48 kHz: count zero crossings over 1 s.
        let input: Vec<f32> = (0..44_100)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin())
            .collect();
        let mut out = Vec::new();
        Resampler::new(44_100, 48_000, 1).process(&input, &mut out);
        let crossings = out.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((1995..=2002).contains(&crossings), "{crossings}");
    }

    #[test]
    fn stereo_conversion() {
        let mut out = Vec::new();
        to_stereo(&[1.0, 2.0], 1, &mut out);
        assert_eq!(out, [1.0, 1.0, 2.0, 2.0]);
        out.clear();
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut out);
        assert_eq!(out, [1.0, 2.0, 4.0, 5.0]);
    }
}
