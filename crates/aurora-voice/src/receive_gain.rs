//! A gain on the received voice, applied by the viewer itself.
//!
//! The Legacy Viewer only asks the voice server for a resident's volume
//! (`LLVoiceWebRTCConnection::setUserVolume`, the `ug` message of
//! indra/newview/llvoicewebrtc.cpp): the server mixes every voice into the
//! one track it sends. On a private call's channel the server ignores that
//! request, so the slider changes nothing, in the Legacy Viewer too
//! (issue #2874). There the received track carries a single voice, the other
//! resident's, so the viewer can scale it itself: the application sets that
//! resident's volume here, and the stream is scaled as it is decoded.
//! Spatial voice and conferences mix several voices and keep the gain at 1.

use std::sync::atomic::{AtomicU32, Ordering};

/// Largest gain: a resident's volume of 1 (200 %), twice as loud as sent.
pub(crate) const MAX_RECEIVE_GAIN: f32 = 2.0;

/// Frames (per channel) over which a gain change is spread: 20 ms at
/// 48 kHz, like the microphone's switch, to avoid clicks.
const RAMP_FRAMES: f32 = 960.0;

/// The wanted gain, set by the API handle and read by the playout task.
pub(crate) struct ReceiveGain(AtomicU32);

impl Default for ReceiveGain {
    fn default() -> Self {
        Self(AtomicU32::new(1.0f32.to_bits()))
    }
}

impl ReceiveGain {
    /// A resident's volume (0..=1, 0.5 heard as sent, the scale of
    /// LLSpeakerVolumeStorage) as a gain: 0 silent, 1 as sent, 2 at the top.
    pub(crate) fn set_volume(&self, volume: f32) {
        let gain = if volume.is_finite() { (volume * 2.0).clamp(0.0, MAX_RECEIVE_GAIN) } else { 1.0 };
        self.0.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The gain the playout applies, moving towards the wanted one.
pub(crate) struct GainRamp {
    current: f32,
}

impl Default for GainRamp {
    fn default() -> Self {
        Self { current: 1.0 }
    }
}

impl GainRamp {
    /// Scales interleaved `samples` (`channels` per frame) towards `target`,
    /// ramping over [`RAMP_FRAMES`]; an amplified sample is kept in -1..=1.
    pub(crate) fn apply(&mut self, target: f32, samples: &mut [f32], channels: usize) {
        if self.current == target && target == 1.0 {
            return;
        }
        let step = 1.0 / RAMP_FRAMES;
        for frame in samples.chunks_mut(channels.max(1)) {
            let d = target - self.current;
            self.current = if d.abs() <= step { target } else { self.current + step.copysign(d) };
            for s in frame {
                *s = (*s * self.current).clamp(-1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_maps_to_gain() {
        let g = ReceiveGain::default();
        assert_eq!(g.get(), 1.0);
        for (volume, gain) in [(0.0, 0.0), (0.1, 0.2), (0.5, 1.0), (0.75, 1.5), (1.0, 2.0), (3.0, 2.0), (-1.0, 0.0)] {
            g.set_volume(volume);
            assert_eq!(g.get(), gain, "volume {volume}");
        }
        g.set_volume(f32::NAN);
        assert_eq!(g.get(), 1.0);
    }

    #[test]
    fn unity_leaves_the_samples_alone() {
        let mut ramp = GainRamp::default();
        let mut s = vec![0.25f32, -0.5, 1.0, -1.0];
        ramp.apply(1.0, &mut s, 2);
        assert_eq!(s, [0.25, -0.5, 1.0, -1.0]);
    }

    #[test]
    fn a_change_ramps_then_holds() {
        let mut ramp = GainRamp::default();
        // 30 ms of stereo at 0.5: the gain reaches 0 within the 20 ms ramp
        let mut s = vec![0.5f32; 1440 * 2];
        ramp.apply(0.0, &mut s, 2);
        assert!(s[0] < 0.5 && s[0] > 0.49, "first frame barely moves: {}", s[0]);
        assert_eq!(s[0], s[1], "both channels of a frame take the same gain");
        let mid = s[480 * 2];
        assert!((mid - 0.25).abs() < 0.01, "half way at 10 ms: {mid}");
        assert!(s[960 * 2..].iter().all(|&x| x == 0.0), "silent after the ramp");
        // the next packet stays silent
        let mut next = vec![0.5f32; 64];
        ramp.apply(0.0, &mut next, 2);
        assert!(next.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn amplified_samples_stay_in_range() {
        let mut ramp = GainRamp { current: 2.0 };
        let mut s = vec![0.25f32, 0.75, -0.75, -0.1];
        ramp.apply(2.0, &mut s, 2);
        assert_eq!(s, [0.5, 1.0, -1.0, -0.2]);
    }
}
