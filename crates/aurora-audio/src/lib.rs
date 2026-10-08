//! Aurora Viewer audio output engine.
//!
//! * A mixer with Second Life's volume channels, as in Firestorm's audio
//!   volume floater: [`Channel::Master`] scales [`Channel::Ui`],
//!   [`Channel::Ambient`], [`Channel::Sfx`], [`Channel::Music`],
//!   [`Channel::Media`] and [`Channel::Voice`]. Each channel has a volume,
//!   a mute flag and an "enabled" flag; gain changes are smoothed (no clicks).
//! * Parcel music: Shoutcast/Icecast streams over HTTP(S) (MP3, AAC/ADTS,
//!   Ogg Vorbis) with ICY `StreamTitle` metadata and automatic reconnection.
//! * PCM sinks ([`VoiceSink`]) used by the voice client (and later media)
//!   to push 48 kHz audio with jitter buffering.
//! * One-shot sounds ([`SoundClip`]) for UI / sound effects.
//! * Device selection ([`output_devices`], [`input_devices`],
//!   [`AudioEngine::set_output_device`]) and microphone capture
//!   ([`MicCapture`]: 48 kHz mono, input gain, level meter) for voice.
//!
//! The device (cpal; WASAPI on Windows) is driven from a dedicated thread.
//! [`AudioEngine::new`] fails when no output device exists; the viewer should
//! then simply run without sound.
//!
//! Volume semantics derived from the Second Life / Firestorm viewer source
//! (`indra/newview/llvieweraudio.cpp`), Copyright (C) Linden Research, Inc.
//! and The Phoenix Firestorm Project, originally LGPL 2.1.

mod capture;
mod decode;
mod devices;
mod http;
mod icy;
mod mixer;
mod output;
mod resample;
mod stream;

use std::sync::Arc;

use crossbeam_channel::{Sender, bounded};
use parking_lot::Mutex;

pub use capture::{CAPTURE_SAMPLE_RATE, MAX_INPUT_GAIN, MicCapture, MicFeed, OVERDRIVEN_POWER_LEVEL};
pub use decode::SoundClip;
pub use devices::{input_devices, output_devices};
pub use mixer::{Channel, ChannelSettings, ENGINE_SAMPLE_RATE};
pub use output::OutputInfo;
pub use stream::StreamStatus;

use mixer::{ClipVoice, Command, Mixer, PcmSource, SharedParams, WorldVoice};
use stream::StreamController;

/// Errors reported by the audio engine.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// No audio output device is available.
    #[error("no audio output device")]
    NoDevice,
    /// The device exists but could not be opened / started.
    #[error("audio device error: {0}")]
    Device(String),
    /// Audio data could not be decoded.
    #[error("audio decode error: {0}")]
    Decode(String),
}

struct Inner {
    params: Arc<SharedParams>,
    cmd_tx: Sender<Command>,
    stream: Mutex<StreamController>,
    info: Arc<Mutex<OutputInfo>>,
    // Dropped last: stops the device thread.
    output: output::OutputThread,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stream.lock().stop();
    }
}

/// Handle to the audio engine. Cheap to clone; the engine (device thread,
/// music stream) shuts down when the last clone is dropped.
#[derive(Clone)]
pub struct AudioEngine {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine").field("output", &*self.inner.info.lock()).finish()
    }
}

/// Capacity of the command queue towards the audio callback.
const COMMAND_QUEUE: usize = 256;

impl AudioEngine {
    /// Opens the default output device and starts the mixer.
    ///
    /// Returns [`AudioError::NoDevice`] (or `Device`) when there is no usable
    /// output; callers should continue without sound.
    pub fn new() -> Result<Self, AudioError> {
        Self::new_with_device(None)
    }

    /// Like [`AudioEngine::new`] on the output device called `device` (see
    /// [`output_devices`]); `None`, or a device that is missing or cannot be
    /// opened, uses the system default (logged, and reported by
    /// [`OutputInfo::fallback`]).
    pub fn new_with_device(device: Option<String>) -> Result<Self, AudioError> {
        let params = Arc::new(SharedParams::new());
        let (cmd_tx, cmd_rx) = bounded(COMMAND_QUEUE);
        let mixer = Arc::new(Mutex::new(Mixer::new(params.clone(), cmd_rx)));
        let info = Arc::new(Mutex::new(OutputInfo::default()));
        let output = output::spawn(mixer, device, info.clone())?;
        Ok(Self {
            inner: Arc::new(Inner {
                params,
                stream: Mutex::new(StreamController::new(cmd_tx.clone())),
                cmd_tx,
                info,
                output,
            }),
        })
    }

    /// The output device in use (updated after a device change or when the
    /// device is lost and reopened).
    pub fn output_info(&self) -> OutputInfo {
        self.inner.info.lock().clone()
    }

    /// Switches the output to the device called `name` (`None` = system
    /// default) without interrupting the mixer: volumes, sinks, the music
    /// stream and playing sounds carry over. Returns at once; the switch
    /// happens on the device thread (a few milliseconds of silence) and is
    /// visible in [`AudioEngine::output_info`] afterwards. A missing device
    /// falls back to the default (logged).
    pub fn set_output_device(&self, name: Option<String>) {
        self.inner.output.set_device(name);
    }

    /// Sets a channel volume (linear, clamped to 0..=1).
    pub fn set_volume(&self, ch: Channel, v: f32) {
        self.inner.params.set_volume(ch, v);
    }

    /// Mutes / unmutes a channel.
    pub fn set_muted(&self, ch: Channel, m: bool) {
        self.inner.params.set_muted(ch, m);
    }

    /// Enables / disables a channel. Disabling [`Channel::Music`] also stops
    /// the network stream; re-enabling resumes the last requested URL.
    pub fn set_enabled(&self, ch: Channel, e: bool) {
        self.inner.params.set_enabled(ch, e);
        if ch == Channel::Music {
            self.inner.stream.lock().set_enabled(e);
        }
    }

    /// Current settings of a channel.
    pub fn settings(&self, ch: Channel) -> ChannelSettings {
        self.inner.params.get(ch)
    }

    /// Plays a parcel music stream on the Music channel, replacing the current
    /// one. An empty URL stops the music. Re-requesting the URL that is
    /// already playing is a no-op.
    pub fn play_stream(&self, url: &str) {
        self.inner.stream.lock().play(url);
    }

    /// Stops the parcel music stream.
    pub fn stop_stream(&self) {
        self.inner.stream.lock().stop();
    }

    /// Status of the parcel music stream.
    pub fn stream_status(&self) -> StreamStatus {
        self.inner.stream.lock().status()
    }

    /// URL of the requested parcel music stream ("" if none).
    pub fn stream_url(&self) -> String {
        self.inner.stream.lock().url().to_string()
    }

    /// Creates a sink feeding the Voice channel (see [`PcmSink`]).
    pub fn voice_sink(&self) -> VoiceSink {
        self.pcm_sink(Channel::Voice)
    }

    /// Creates a sink feeding `ch` with 48 kHz PCM. Each sink is an
    /// independent jitter-buffered source; they are summed by the mixer.
    pub fn pcm_sink(&self, ch: Channel) -> PcmSink {
        let (prebuffer, max_latency) = match ch {
            // 60 ms jitter buffer, never more than 250 ms behind.
            Channel::Voice => (2_880, 12_000),
            // 100 ms / 500 ms for media-like sources.
            _ => (4_800, 24_000),
        };
        let (prod, cons) = rtrb::RingBuffer::<f32>::new(SINK_RING_FRAMES * 2);
        let source = PcmSource::new(ch, cons, prebuffer, Some(max_latency), None);
        match self.inner.cmd_tx.try_send(Command::AddPcm(Box::new(source))) {
            Ok(()) => PcmSink {
                prod: Some(prod),
                scratch: Vec::new(),
            },
            Err(_) => PcmSink::null(),
        }
    }

    /// Plays a sound placed in the world under `id` (replacing a sound with
    /// the same id): mono, looped or not, with (left, right) gains updated
    /// by [`AudioEngine::world_gain`] as the listener or the source moves.
    pub fn play_world(&self, id: u64, ch: Channel, clip: &SoundClip, looped: bool, gain: (f32, f32)) {
        let _ = self
            .inner
            .cmd_tx
            .try_send(Command::PlayWorld(WorldVoice::new(id, ch, clip.samples.clone(), looped, gain)));
    }

    /// New (left, right) gains of a world sound (ramped over one buffer).
    pub fn world_gain(&self, id: u64, gain: (f32, f32)) {
        let _ = self.inner.cmd_tx.try_send(Command::WorldGain(id, gain));
    }

    /// Fades a world sound out and removes it.
    pub fn stop_world(&self, id: u64) {
        let _ = self.inner.cmd_tx.try_send(Command::StopWorld(id));
    }

    /// Plays a one-shot sound on `ch` with an extra per-sound `gain` (0..=1).
    pub fn play_clip(&self, ch: Channel, clip: &SoundClip, gain: f32) {
        let _ = self
            .inner
            .cmd_tx
            .try_send(Command::PlayClip(ClipVoice::new(ch, clip.samples.clone(), gain)));
    }
}

/// One second of buffering per sink.
const SINK_RING_FRAMES: usize = ENGINE_SAMPLE_RATE as usize;

/// Producer side of a jitter-buffered PCM source at [`ENGINE_SAMPLE_RATE`].
///
/// Push interleaved f32 frames (mono or stereo; extra channels are ignored).
/// Playback starts once a small jitter buffer has filled, pauses on underrun
/// and drops old audio when too much accumulates. Dropping the sink removes
/// the source from the mixer once its queue has played out.
pub struct PcmSink {
    prod: Option<rtrb::Producer<f32>>,
    scratch: Vec<f32>,
}

/// The sink type used by the voice client.
pub type VoiceSink = PcmSink;

impl std::fmt::Debug for PcmSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcmSink").field("connected", &self.is_connected()).finish()
    }
}

impl PcmSink {
    /// A sink that discards everything (e.g. when there is no audio device).
    pub fn null() -> Self {
        Self {
            prod: None,
            scratch: Vec::new(),
        }
    }

    /// A sink connected to a [`PcmTap`] instead of the mixer (tests, recording
    /// or custom consumers). `frames` is the buffer capacity.
    pub fn loopback(frames: usize) -> (PcmSink, PcmTap) {
        let (prod, cons) = rtrb::RingBuffer::<f32>::new(frames.max(1) * 2);
        (
            PcmSink {
                prod: Some(prod),
                scratch: Vec::new(),
            },
            PcmTap { cons },
        )
    }

    /// `true` while the engine is alive and consuming this sink.
    pub fn is_connected(&self) -> bool {
        self.prod.as_ref().is_some_and(|p| !p.is_abandoned())
    }

    /// Queues interleaved 48 kHz samples with `channels` channels. Returns the
    /// number of frames accepted (excess is dropped when the buffer is full).
    pub fn push(&mut self, samples: &[f32], channels: u16) -> usize {
        let Some(prod) = self.prod.as_mut() else {
            return 0;
        };
        if channels == 0 {
            return 0;
        }
        let stereo: &[f32] = if channels == 2 {
            &samples[..samples.len() & !1]
        } else {
            self.scratch.clear();
            resample::to_stereo(samples, usize::from(channels), &mut self.scratch);
            &self.scratch
        };
        let (pushed, _) = prod.push_partial_slice(stereo);
        pushed.len() / 2
    }
}

/// Consumer end of [`PcmSink::loopback`]: interleaved stereo 48 kHz samples.
pub struct PcmTap {
    cons: rtrb::Consumer<f32>,
}

impl std::fmt::Debug for PcmTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcmTap").field("queued", &self.cons.slots()).finish()
    }
}

impl PcmTap {
    /// Copies queued samples into `out`; returns how many were written.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let (got, _) = self.cons.pop_partial_slice(out);
        got.len()
    }

    /// Number of samples waiting.
    pub fn available(&self) -> usize {
        self.cons.slots()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_sink_delivers_stereo() {
        let (mut sink, mut tap) = PcmSink::loopback(16);
        assert_eq!(sink.push(&[0.5, -0.5], 2), 1);
        assert_eq!(tap.available(), 2);
        let mut out = [0.0; 4];
        assert_eq!(tap.read(&mut out), 2);
        assert_eq!(&out[..2], &[0.5, -0.5]);
    }

    #[test]
    fn null_sink_discards() {
        let mut s = PcmSink::null();
        assert!(!s.is_connected());
        assert_eq!(s.push(&[0.0; 10], 1), 0);
    }

    #[test]
    fn sink_converts_mono_and_drops_overflow() {
        let (prod, mut cons) = rtrb::RingBuffer::<f32>::new(8);
        let mut s = PcmSink {
            prod: Some(prod),
            scratch: Vec::new(),
        };
        assert!(s.is_connected());
        assert_eq!(s.push(&[0.1, 0.2], 1), 2);
        assert_eq!(cons.pop(), Ok(0.1));
        assert_eq!(cons.pop(), Ok(0.1));
        assert_eq!(cons.pop(), Ok(0.2));
        assert_eq!(cons.pop(), Ok(0.2));
        // 8 slots: only 4 stereo frames fit.
        assert_eq!(s.push(&[0.0; 12], 2), 4);
        drop(cons);
        assert!(!s.is_connected());
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;

    /// Opens the real default output device (plays silence briefly).
    /// Not run by default: needs audio hardware.
    #[test]
    #[ignore]
    fn opens_default_device() {
        match AudioEngine::new() {
            Ok(engine) => {
                let info = engine.output_info();
                assert!(info.sample_rate > 0 && info.channels > 0, "{info:?}");
                engine.set_volume(Channel::Master, 0.0);
                engine.play_clip(Channel::Ui, &SoundClip::from_pcm(&[0.0; 4800], 1, 48_000), 1.0);
                std::thread::sleep(std::time::Duration::from_millis(300));
                assert_eq!(engine.stream_status(), StreamStatus::Idle);
            }
            Err(AudioError::NoDevice) => {}
            Err(e) => panic!("{e}"),
        }
    }

    /// Lists devices and switches the output live (needs audio hardware).
    #[test]
    #[ignore]
    fn switches_output_device() {
        let outputs = output_devices();
        println!("outputs: {outputs:?}\ninputs: {:?}", input_devices());
        let Ok(engine) = AudioEngine::new_with_device(Some("no such device".into())) else {
            return;
        };
        engine.set_volume(Channel::Master, 0.0);
        assert!(engine.output_info().fallback);
        let wait = || std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(last) = outputs.last() {
            engine.set_output_device(Some(last.clone()));
            wait();
            let info = engine.output_info();
            assert_eq!(&info.device_name, last, "{info:?}");
            assert!(!info.fallback);
        }
        engine.set_output_device(Some("still missing".into()));
        wait();
        assert!(engine.output_info().fallback);
        engine.set_output_device(None);
        wait();
        assert!(!engine.output_info().fallback);
        assert_eq!(engine.settings(Channel::Master).volume, 0.0);
    }

    /// Opens the default microphone for a second (needs audio hardware).
    #[test]
    #[ignore]
    fn captures_default_microphone() {
        let mic = match MicCapture::open(None) {
            Ok(m) => m,
            Err(AudioError::NoDevice) => return,
            Err(e) => panic!("{e}"),
        };
        println!("input: {:?}", mic.device_name());
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(mic.samples_captured() > 24_000, "{}", mic.samples_captured());
        assert!(mic.available() > 24_000);
        println!("level: {}", mic.level());
        mic.set_device(Some("no such microphone".into()));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(mic.is_fallback());
    }
}
