//! Voice transmission: microphone frames -> gain ramp -> level -> Opus.
//!
//! The gain / mute handling follows `LLCustomProcessor::Process` and
//! `LLWebRTCImpl::setMute` / `setMicGain` (`indra/llwebrtc/llwebrtc.cpp`):
//! the microphone gain is applied in the send path, muting ramps the gain to
//! zero (no click) and a muted track then sends silence. The own speaking
//! level follows `LLWebRTCVoiceClient::updateOwnVolume` /
//! `predUpdateOwnVolume` (`indra/newview/llvoicewebrtc.cpp`): RMS over
//! 300 ms in 16-bit units, `0.18 - 0.005 * (-20 log10 rms)`, speaking above
//! 0.30, and 0 while the microphone is muted. Opus is encoded as llwebrtc
//! sends it: 48 kHz mono, 20 ms frames, VoIP application, in-band FEC.
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//! Licensed under the GNU Lesser General Public License, version 2.1.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use aurora_audio::MicCapture;
use parking_lot::Mutex;
use rusty_opus::{Application, OpusEncoder};

/// 20 ms at 48 kHz mono.
pub(crate) const FRAME_SAMPLES: usize = 960;
pub(crate) const FRAME_DURATION: Duration = Duration::from_millis(20);
/// Opus silence frame (TOC: CELT FB 20 ms stereo, payload FF FE), sent while
/// the microphone is off, as libwebrtc does for a disabled audio track.
pub(crate) const OPUS_SILENCE: [u8; 3] = [0xFC, 0xFF, 0xFE];
/// libwebrtc's default Opus target for a mono voice stream.
const BITRATE_BPS: i32 = 32_000;
/// libwebrtc's default encoder complexity on desktop.
const COMPLEXITY: i32 = 9;
/// Expected loss used to size the in-band FEC (`useinbandfec=1`).
const PACKET_LOSS_PERC: i32 = 5;
/// Largest Opus packet we produce.
const MAX_PACKET: usize = 1500;
/// At most this many frames are encoded per poll (catch-up after a stall).
const MAX_FRAMES_PER_POLL: usize = 3;
/// Above this backlog the oldest audio is dropped down to `KEEP_BACKLOG`,
/// bounding the latency added by clock drift or a paused reader.
const MAX_BACKLOG: usize = FRAME_SAMPLES * 6;
const KEEP_BACKLOG: usize = FRAME_SAMPLES * 2;
/// Without microphone data for this long, silence frames are sent instead.
const STALL: Duration = Duration::from_millis(100);
/// 300 ms of smoothing (`NUM_PACKETS_TO_FILTER` x 10 ms) in 20 ms frames.
const LEVEL_FRAMES: usize = 15;
/// `LEVEL_START_POINT` / `LEVEL_SCALE` (llvoicewebrtc.cpp).
const LEVEL_START_POINT: f32 = 0.18;
const LEVEL_SCALE: f32 = 0.005;
/// `SPEAKING_AUDIO_LEVEL`.
pub(crate) const SPEAKING_AUDIO_LEVEL: f32 = 0.30;
/// Largest microphone gain (Firestorm's `AudioLevelMic` slider goes to 2).
pub(crate) const MAX_MIC_GAIN: f32 = 4.0;

/// Transmission settings shared between the API handle and the sender.
pub(crate) struct TxControl {
    transmit: AtomicBool,
    gain: AtomicU32,
    level: AtomicU32,
    pub mic: Mutex<Option<MicCapture>>,
}

impl TxControl {
    pub(crate) fn new() -> Self {
        Self {
            transmit: AtomicBool::new(false),
            gain: AtomicU32::new(1.0f32.to_bits()),
            level: AtomicU32::new(0),
            mic: Mutex::new(None),
        }
    }

    pub(crate) fn set_transmit(&self, on: bool) {
        self.transmit.store(on, Ordering::Relaxed);
    }

    pub(crate) fn transmit(&self) -> bool {
        self.transmit.load(Ordering::Relaxed)
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        let gain = if gain.is_finite() { gain.clamp(0.0, MAX_MIC_GAIN) } else { 1.0 };
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    pub(crate) fn set_level(&self, level: f32) {
        self.level.store(level.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

/// What to send for one 20 ms frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Payload {
    /// The muted-track silence frame ([`OPUS_SILENCE`]).
    Silence,
    /// An encoded Opus packet.
    Opus(Vec<u8>),
}

/// Encoder state of one connection.
pub(crate) struct Transmitter {
    encoder: Option<OpusEncoder>,
    frame: Vec<f32>,
    packet: Vec<u8>,
    /// Gain applied at the end of the last frame (ramps towards the target).
    gain: f32,
    energies: [f32; LEVEL_FRAMES],
    energy_pos: usize,
    last_data: Option<Instant>,
    next_silence: Option<Instant>,
}

impl Transmitter {
    pub(crate) fn new() -> Self {
        let encoder = match OpusEncoder::new(48_000, 1, Application::Voip) {
            Ok(mut e) => {
                e.bitrate_bps = BITRATE_BPS;
                e.complexity = COMPLEXITY;
                e.use_inband_fec = true;
                e.packet_loss_perc = PACKET_LOSS_PERC;
                e.use_dtx = false;
                e.use_cbr = false;
                Some(e)
            }
            Err(e) => {
                log::error!("voice: cannot create Opus encoder ({e:?}); sending silence");
                None
            }
        };
        Self {
            encoder,
            frame: vec![0.0; FRAME_SAMPLES],
            packet: vec![0; MAX_PACKET],
            gain: 0.0,
            energies: [0.0; LEVEL_FRAMES],
            energy_pos: 0,
            last_data: None,
            next_silence: None,
        }
    }

    /// Called every 10 ms: turns the microphone frames available now into
    /// payloads (appended to `out`). Without microphone data, silence frames
    /// keep the media flowing every 20 ms, like a muted libwebrtc track.
    pub(crate) fn poll(&mut self, now: Instant, mic: Option<&MicCapture>, transmit: bool, gain: f32, out: &mut Vec<Payload>) {
        let target = if transmit { gain } else { 0.0 };
        let mut produced = 0;
        if let Some(mic) = mic {
            let backlog = mic.available();
            if backlog > MAX_BACKLOG {
                mic.skip(backlog - KEEP_BACKLOG);
            }
            while produced < MAX_FRAMES_PER_POLL && mic.available() >= FRAME_SAMPLES {
                let n = mic.read(&mut self.frame);
                self.frame[n..].fill(0.0);
                self.last_data = Some(now);
                let payload = self.process_frame(target);
                out.push(payload);
                produced += 1;
            }
        }
        if produced > 0 {
            self.next_silence = Some(now + FRAME_DURATION);
            return;
        }
        let stalled = self.last_data.is_none_or(|t| now.duration_since(t) >= STALL);
        if !stalled {
            // The next microphone buffer is on its way.
            return;
        }
        let mut due = self.next_silence.unwrap_or(now);
        if now.saturating_duration_since(due) > FRAME_DURATION * 2 {
            // Far behind (after a stall): restart the cadence, no burst.
            due = now;
        }
        if now >= due {
            out.push(Payload::Silence);
            self.push_energy(0.0);
            self.gain = 0.0;
            due += FRAME_DURATION;
        }
        self.next_silence = Some(due);
    }

    /// Ramps the gain over the frame (`LLCustomProcessor::Process`), meters
    /// it and encodes it, or returns silence when muted throughout.
    fn process_frame(&mut self, target: f32) -> Payload {
        let start = self.gain;
        let step = (target - start) / FRAME_SAMPLES as f32;
        let mut g = start;
        let mut energy = 0.0f32;
        for s in &mut self.frame {
            g += step;
            let v = *s * g;
            let v = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
            *s = v;
            energy += v * v;
        }
        self.gain = target;
        self.push_energy(energy / FRAME_SAMPLES as f32);
        if start == 0.0 && target == 0.0 {
            return Payload::Silence;
        }
        let Some(encoder) = self.encoder.as_mut() else {
            return Payload::Silence;
        };
        match encoder.encode(&self.frame, FRAME_SAMPLES, &mut self.packet) {
            Ok(len) if len > 0 && len <= self.packet.len() => Payload::Opus(self.packet[..len].to_vec()),
            Ok(_) => Payload::Silence,
            Err(e) => {
                log::debug!("voice: Opus encoding failed: {e:?}");
                Payload::Silence
            }
        }
    }

    fn push_energy(&mut self, mean_square: f32) {
        self.energies[self.energy_pos] = mean_square;
        self.energy_pos = (self.energy_pos + 1) % LEVEL_FRAMES;
    }

    /// Own speaking level (0..1) as Firestorm shows it; 0 when not
    /// transmitting (`mMuteMic`).
    pub(crate) fn level(&self, transmit: bool) -> f32 {
        if !transmit {
            return 0.0;
        }
        let mean = self.energies.iter().sum::<f32>() / LEVEL_FRAMES as f32;
        // WebRTC audio buffers hold floats in 16-bit units.
        let rms = mean.sqrt() * 32_768.0;
        if rms <= 0.0 || !rms.is_finite() {
            return 0.0;
        }
        (LEVEL_START_POINT + LEVEL_SCALE * 20.0 * rms.log10()).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, amp: f32, offset: usize) -> Vec<f32> {
        (offset..offset + n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn decode_rms(packets: &[Vec<u8>]) -> f32 {
        let mut dec = opus_decoder::OpusDecoder::new(48_000, 2).unwrap();
        let mut pcm = vec![0.0f32; 5760 * 2];
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for p in packets {
            let n = dec.decode_float(p, &mut pcm, false).unwrap();
            assert_eq!(n, FRAME_SAMPLES);
            for v in &pcm[..n * 2] {
                sum += f64::from(*v) * f64::from(*v);
            }
            count += n * 2;
        }
        (sum / count.max(1) as f64).sqrt() as f32
    }

    fn opus(payloads: &[Payload]) -> Vec<Vec<u8>> {
        payloads
            .iter()
            .filter_map(|p| match p {
                Payload::Opus(v) => Some(v.clone()),
                Payload::Silence => None,
            })
            .collect()
    }

    #[test]
    fn silence_without_microphone() {
        let mut tx = Transmitter::new();
        let mut out = Vec::new();
        let t0 = Instant::now();
        for i in 0..10 {
            tx.poll(t0 + Duration::from_millis(10 * i), None, true, 1.0, &mut out);
        }
        // One frame every 20 ms.
        assert_eq!(out, vec![Payload::Silence; 5]);
        assert_eq!(tx.level(true), 0.0);
    }

    #[test]
    fn muted_microphone_sends_silence_and_drains() {
        let (mut feed, mic) = MicCapture::virtual_input();
        let mut tx = Transmitter::new();
        let mut out = Vec::new();
        let t0 = Instant::now();
        feed.push(&sine(FRAME_SAMPLES * 2, 0.5, 0));
        tx.poll(t0, Some(&mic), false, 1.0, &mut out);
        assert_eq!(out, vec![Payload::Silence; 2]);
        assert_eq!(mic.available(), 0);
        assert_eq!(tx.level(false), 0.0);
        // Waiting for the next buffer: nothing extra is sent.
        out.clear();
        tx.poll(t0 + Duration::from_millis(30), Some(&mic), false, 1.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn transmitting_encodes_decodable_speech_and_ramps() {
        let (mut feed, mic) = MicCapture::virtual_input();
        let mut tx = Transmitter::new();
        let mut out = Vec::new();
        let t0 = Instant::now();
        feed.push(&sine(FRAME_SAMPLES / 2, 0.5, 0));
        let mut offset = FRAME_SAMPLES / 2;
        for i in 0..50u64 {
            feed.push(&sine(FRAME_SAMPLES / 2, 0.5, offset));
            offset += FRAME_SAMPLES / 2;
            tx.poll(t0 + Duration::from_millis(10 * i), Some(&mic), true, 1.0, &mut out);
        }
        assert_eq!(out.len(), 25);
        let packets = opus(&out);
        assert_eq!(packets.len(), 25, "all frames encoded");
        assert!(packets.iter().all(|p| p.len() > 10 && p.len() < 400));
        // A 0.5 sine has an RMS of 0.35 (duplicated on both channels).
        let rms = decode_rms(&packets[5..]);
        assert!((0.25..0.45).contains(&rms), "{rms}");
        // Firestorm level of a -9 dBFS signal: 0.18 + 0.1 log10(0.35 * 32768).
        let level = tx.level(true);
        assert!((level - 0.586).abs() < 0.02, "{level}");
        assert!(level > SPEAKING_AUDIO_LEVEL);
        assert_eq!(tx.level(false), 0.0);

        // Push-to-talk released: one ramp-down frame, then muted silence.
        out.clear();
        for i in 50..60u64 {
            feed.push(&sine(FRAME_SAMPLES / 2, 0.5, offset));
            offset += FRAME_SAMPLES / 2;
            tx.poll(t0 + Duration::from_millis(10 * i), Some(&mic), false, 1.0, &mut out);
        }
        assert_eq!(out.len(), 5);
        assert!(matches!(out[0], Payload::Opus(_)));
        assert_eq!(&out[1..], vec![Payload::Silence; 4].as_slice());

        // Gain 0 while transmitting is silence too; non-finite input is safe.
        out.clear();
        feed.push(&[f32::NAN; FRAME_SAMPLES]);
        tx.poll(t0 + Duration::from_secs(2), Some(&mic), true, 0.0, &mut out);
        assert_eq!(out, vec![Payload::Silence]);
        out.clear();
        feed.push(&[f32::NAN; FRAME_SAMPLES * 2]);
        tx.poll(t0 + Duration::from_secs(3), Some(&mic), true, 1.0, &mut out);
        let packets = opus(&out);
        assert_eq!(packets.len(), 2);
        assert!(decode_rms(&packets) < 0.01);
    }

    #[test]
    fn backlog_is_bounded() {
        let (mut feed, mic) = MicCapture::virtual_input();
        let mut tx = Transmitter::new();
        let mut out = Vec::new();
        feed.push(&vec![0.0; FRAME_SAMPLES * 20]);
        tx.poll(Instant::now(), Some(&mic), false, 1.0, &mut out);
        // Cut to 40 ms, which is then sent.
        assert_eq!(out.len(), KEEP_BACKLOG / FRAME_SAMPLES);
        assert_eq!(mic.available(), 0);
        // A smaller backlog is caught up a few frames per poll.
        feed.push(&vec![0.0; FRAME_SAMPLES * 5]);
        out.clear();
        tx.poll(Instant::now(), Some(&mic), false, 1.0, &mut out);
        assert_eq!(out.len(), MAX_FRAMES_PER_POLL);
        assert_eq!(mic.available(), FRAME_SAMPLES * 2);
    }

    #[test]
    fn stalled_microphone_falls_back_to_silence() {
        let (mut feed, mic) = MicCapture::virtual_input();
        let mut tx = Transmitter::new();
        let mut out = Vec::new();
        let t0 = Instant::now();
        feed.push(&sine(FRAME_SAMPLES, 0.5, 0));
        tx.poll(t0, Some(&mic), true, 1.0, &mut out);
        assert!(matches!(out[0], Payload::Opus(_)));
        out.clear();
        for i in 1..20u64 {
            tx.poll(t0 + Duration::from_millis(10 * i), Some(&mic), true, 1.0, &mut out);
        }
        // Nothing for 100 ms, then silence every 20 ms.
        assert_eq!(out, vec![Payload::Silence; 5]);
    }

    #[test]
    fn control_clamps_gain() {
        let c = TxControl::new();
        assert!(!c.transmit());
        assert_eq!(c.gain(), 1.0);
        c.set_gain(9.0);
        assert_eq!(c.gain(), MAX_MIC_GAIN);
        c.set_gain(f32::INFINITY);
        assert_eq!(c.gain(), 1.0);
        c.set_gain(-1.0);
        assert_eq!(c.gain(), 0.0);
        c.set_transmit(true);
        assert!(c.transmit());
        c.set_level(0.5);
        assert_eq!(c.level(), 0.5);
    }
}
