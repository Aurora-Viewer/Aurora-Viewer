//! WebRTC microphone processing, off the device callbacks and the UI thread.
//! Settings follow LLWebRTCImpl::setAudioConfig (indra/llwebrtc/llwebrtc.cpp):
//! AEC3, high-pass, AGC2 adaptive digital and four noise suppression strengths.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use crossbeam_channel::{Receiver, Sender, bounded};
use sonora::config::{AdaptiveDigital, EchoCanceller, GainController2, HighPassFilter, NoiseSuppression, NoiseSuppressionLevel};
use sonora::{AudioProcessing, Config, StreamConfig};

pub(crate) const BLOCK: usize = 480;

/// User-selectable effects. Manual microphone gain is applied afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicProcessing {
    pub automatic_gain: bool,
    pub echo_cancellation: bool,
    /// 0 = off, 1 = low, 2 = moderate, 3 = high, 4 = very high.
    pub noise_suppression: u8,
}

impl Default for MicProcessing {
    fn default() -> Self {
        Self {
            automatic_gain: true,
            echo_cancellation: true,
            noise_suppression: 4,
        }
    }
}

/// Bounded, nonblocking tap of the final 48 kHz stereo mixer output. Old data
/// is discarded when capture is closed; there is no unbounded playback backlog.
#[derive(Clone)]
pub struct EchoReference(Arc<ReferenceInner>);

struct ReferenceInner {
    tx: Sender<[f32; BLOCK * 2]>,
    rx: Receiver<[f32; BLOCK * 2]>,
    delay_ms: AtomicI32,
}

impl Default for EchoReference {
    fn default() -> Self {
        let (tx, rx) = bounded(20);
        Self(Arc::new(ReferenceInner {
            tx,
            rx,
            delay_ms: AtomicI32::new(0),
        }))
    }
}

impl EchoReference {
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn delay(&self, ms: i32) {
        self.0.delay_ms.store(ms.clamp(0, 500), Ordering::Relaxed);
    }

    fn push(&self, block: [f32; BLOCK * 2]) {
        if let Err(crossbeam_channel::TrySendError::Full(block)) = self.0.tx.try_send(block) {
            let _ = self.0.rx.try_recv();
            let _ = self.0.tx.try_send(block);
        }
    }
}

pub(crate) struct RenderTap {
    reference: EchoReference,
    block: [f32; BLOCK * 2],
    fill: usize,
}

impl RenderTap {
    pub(crate) fn new(reference: EchoReference) -> Self {
        Self {
            reference,
            block: [0.0; BLOCK * 2],
            fill: 0,
        }
    }

    pub(crate) fn write(&mut self, stereo: &[f32]) {
        for &s in stereo {
            self.block[self.fill] = s;
            self.fill += 1;
            if self.fill == self.block.len() {
                self.reference.push(self.block);
                self.fill = 0;
            }
        }
    }
}

pub(crate) struct Processor {
    apm: AudioProcessing,
    config: MicProcessing,
    reference: Option<EchoReference>,
    render: [[f32; BLOCK]; 2],
    render_out: [[f32; BLOCK]; 2],
    capture_delay_ms: i32,
}

impl Processor {
    pub(crate) fn new(config: MicProcessing, reference: Option<EchoReference>) -> Self {
        let apm = AudioProcessing::builder()
            .config(Self::config(config))
            .capture_config(StreamConfig::new(48_000, 1))
            .render_config(StreamConfig::new(48_000, 2))
            .build();
        Self {
            apm,
            config,
            reference,
            render: [[0.0; BLOCK]; 2],
            render_out: [[0.0; BLOCK]; 2],
            capture_delay_ms: 0,
        }
    }

    fn config(c: MicProcessing) -> Config {
        Config {
            high_pass_filter: Some(HighPassFilter::default()),
            echo_canceller: c.echo_cancellation.then(EchoCanceller::default),
            noise_suppression: (c.noise_suppression > 0).then(|| NoiseSuppression {
                level: match c.noise_suppression {
                    1 => NoiseSuppressionLevel::Low,
                    2 => NoiseSuppressionLevel::Moderate,
                    3 => NoiseSuppressionLevel::High,
                    _ => NoiseSuppressionLevel::VeryHigh,
                },
                ..Default::default()
            }),
            gain_controller2: c.automatic_gain.then(|| GainController2 {
                adaptive_digital: Some(AdaptiveDigital::default()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub(crate) fn update(&mut self, config: MicProcessing, reference: Option<EchoReference>) {
        let reference_changed = match (&self.reference, &reference) {
            (Some(a), Some(b)) => !a.same(b),
            (None, None) => false,
            _ => true,
        };
        if reference_changed {
            // Device/engine replacement must not retain an unrelated echo path.
            *self = Self::new(config, reference);
        } else if self.config != config {
            self.apm.apply_config(Self::config(config));
            self.config = config;
        }
    }

    pub(crate) fn process(&mut self, input: &[f32; BLOCK], output: &mut [f32; BLOCK]) {
        if let Some(reference) = &self.reference {
            while let Ok(block) = reference.0.rx.try_recv() {
                for (i, frame) in block.as_chunks::<2>().0.iter().enumerate() {
                    self.render[0][i] = frame[0];
                    self.render[1][i] = frame[1];
                }
                let [left, right] = &mut self.render_out;
                if let Err(e) = self.apm.process_render_f32(&[&self.render[0], &self.render[1]], &mut [left, right]) {
                    log::debug!("voice render processing: {e:?}");
                }
            }
            let _ = self
                .apm
                .set_stream_delay_ms((reference.0.delay_ms.load(Ordering::Relaxed) + self.capture_delay_ms).min(500));
        }
        if let Err(e) = self.apm.process_capture_f32(&[input], &mut [output]) {
            log::warn!("microphone processing: {e:?}");
            output.copy_from_slice(input);
        }
    }

    pub(crate) fn capture_delay(&mut self, ms: i32) {
        self.capture_delay_ms = ms.clamp(0, 500);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_gain_amplifies_a_quiet_input_and_can_be_disabled() {
        let mut energies = [0.0; 2];
        for (index, enabled) in [false, true].into_iter().enumerate() {
            let mut p = Processor::new(
                MicProcessing {
                    automatic_gain: enabled,
                    echo_cancellation: false,
                    noise_suppression: 0,
                },
                None,
            );
            for frame in 0..100 {
                let input = std::array::from_fn(|i| 0.01 * ((frame * BLOCK + i) as f32 * 0.0576).sin());
                let mut output = [0.0; BLOCK];
                p.process(&input, &mut output);
                if frame > 50 {
                    energies[index] += output.iter().map(|v| v * v).sum::<f32>();
                }
            }
        }
        assert!(energies[1] > energies[0] * 2.0, "AGC energies: {energies:?}");
    }

    #[test]
    fn echo_canceller_removes_delayed_playback() {
        let reference = EchoReference::default();
        reference.delay(80);
        let mut tap = RenderTap::new(reference.clone());
        let mut p = Processor::new(
            MicProcessing {
                automatic_gain: false,
                echo_cancellation: true,
                noise_suppression: 0,
            },
            Some(reference),
        );
        let mut history = [[0.0; BLOCK]; 8];
        let mut seed = 43u32;
        let mut energy = (0.0f64, 0.0f64);
        let start = std::time::Instant::now();
        for n in 0..700 {
            let render = std::array::from_fn::<_, BLOCK, _>(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                seed as i32 as f32 / i32::MAX as f32 * 0.15
            });
            let stereo = std::array::from_fn::<_, { BLOCK * 2 }, _>(|i| render[i / 2]);
            tap.write(&stereo);
            let input = history[n % 8].map(|v| v * 0.6);
            history[n % 8] = render;
            let mut output = [0.0; BLOCK];
            p.process(&input, &mut output);
            if n > 500 {
                energy.0 += input.iter().map(|v| f64::from(v * v)).sum::<f64>();
                energy.1 += output.iter().map(|v| f64::from(v * v)).sum::<f64>();
            }
        }
        println!(
            "AEC fixture: {:.1} dB attenuation, {:.3} ms/block",
            10.0 * (energy.0 / energy.1).log10(),
            start.elapsed().as_secs_f64() * 1000.0 / 700.0
        );
        assert!(energy.1 < energy.0 * 0.25, "echo energies: {energy:?}");
    }

    #[test]
    fn all_effects_can_change_live_without_invalid_samples() {
        let mut p = Processor::new(MicProcessing::default(), None);
        for strength in 0..=4 {
            for gain in [false, true] {
                for echo in [false, true] {
                    p.update(
                        MicProcessing {
                            automatic_gain: gain,
                            echo_cancellation: echo,
                            noise_suppression: strength,
                        },
                        None,
                    );
                    let input = std::array::from_fn(|i| 0.8 * (i as f32 * 0.1).sin());
                    let mut output = [0.0; BLOCK];
                    p.process(&input, &mut output);
                    assert!(output.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
                }
            }
        }
    }

    #[test]
    fn suppression_reduces_stationary_noise_at_every_strength() {
        let mut seed = 7u32;
        let noise: Vec<[f32; BLOCK]> = (0..300)
            .map(|_| {
                std::array::from_fn(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    (seed as i32 as f32 / i32::MAX as f32) * 0.03
                })
            })
            .collect();
        let mut energies = Vec::new();
        for strength in 0..=4 {
            let mut p = Processor::new(
                MicProcessing {
                    automatic_gain: false,
                    echo_cancellation: false,
                    noise_suppression: strength,
                },
                None,
            );
            let mut energy = 0.0;
            let mut out = [0.0; BLOCK];
            for (i, block) in noise.iter().enumerate() {
                p.process(block, &mut out);
                assert!(out.iter().all(|v| v.is_finite()));
                if i > 200 {
                    energy += out.iter().map(|v| v * v).sum::<f32>();
                }
            }
            energies.push(energy);
        }
        for &e in &energies[1..] {
            assert!(e < energies[0] * 0.5, "noise energies: {energies:?}");
        }
        assert!(energies[4] < energies[1], "noise energies: {energies:?}");
    }

    #[test]
    fn reference_is_bounded_and_keeps_latest_playback() {
        let r = EchoReference::default();
        let mut tap = RenderTap::new(r.clone());
        for i in 0..100 {
            tap.write(&[i as f32; BLOCK * 2]);
        }
        let blocks: Vec<_> = r.0.rx.try_iter().collect();
        assert_eq!(blocks.len(), 20);
        assert_eq!(blocks[0][0], 80.0);
        assert_eq!(blocks[19][0], 99.0);
    }
}
