//! Device output (cpal; WASAPI on Windows) on a dedicated thread.
//!
//! The thread owns the cpal stream, rebuilds it when the device disappears,
//! the stream is invalidated or another device is selected, and exits when
//! the engine is dropped. The mixer is shared with the stream callback and
//! survives device changes (channels, sinks and playing sounds are kept).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use parking_lot::Mutex;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::devices::{self, Direction};
use crate::mixer::{ENGINE_CHANNELS, ENGINE_SAMPLE_RATE, Mixer};
use crate::processing::{EchoReference, RenderTap};
use crate::resample::Resampler;
use crate::{AudioError, VoiceStream, voice_stream_for};

/// Description of the opened output device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputInfo {
    /// Human-readable device name.
    pub device_name: String,
    /// Device sample rate (the mixer runs at 48 kHz and is resampled if needed).
    pub sample_rate: u32,
    /// Device channel count.
    pub channels: u16,
    /// `true` when a named device was requested but could not be found or
    /// opened, so the system default is used instead.
    pub fallback: bool,
}

/// Sound queued in the output device (s): four of WASAPI's usual 10 ms
/// periods, so that a callback held back by up to about 30 ms (the viewer
/// loading a region on every core) still finds sound to play instead of
/// leaving a gap, heard as a tick in a continuous sound such as the wind
/// while flying. The callbacks keep the device period; only the latency
/// grows, still under the Legacy Viewer's (FMOD's default mixing buffers,
/// 4 × 1024 frames, about 90 ms).
const OUTPUT_QUEUE_SECS: f64 = 0.04;

/// Health of the playing stream: written by the stream callback, read by the
/// output thread. A callback that comes later than the sound queued in the
/// device leaves it silent for a moment (a tick); counting those tells a
/// starved output from a defect of the sound itself.
struct StreamHealth {
    /// The device reported the stream lost.
    failed: AtomicBool,
    /// Sound queued in the device (µs; 0 when the device chose).
    queued_us: u32,
    /// Callbacks that came after the queued sound had run out.
    gaps: AtomicU32,
    /// Longest wait between two callbacks (µs).
    worst_gap_us: AtomicU32,
    /// Longest mix in a callback (µs).
    worst_mix_us: AtomicU32,
    /// Shortest callback buffer, the device period (µs).
    period_us: AtomicU32,
}

impl StreamHealth {
    fn new(queued: Duration) -> Self {
        Self {
            failed: AtomicBool::new(false),
            queued_us: micros(queued),
            gaps: AtomicU32::new(0),
            worst_gap_us: AtomicU32::new(0),
            worst_mix_us: AtomicU32::new(0),
            period_us: AtomicU32::new(u32::MAX),
        }
    }

    /// Longest wait the device survives: the sound queued, and at least two
    /// periods when the device chose its own buffer.
    fn tolerance_us(&self) -> u32 {
        self.queued_us.max(self.period_us.load(Ordering::Relaxed).saturating_mul(2))
    }

    /// Called at the end of each callback that began at `start`.
    fn record(&self, start: Instant, previous: Option<Instant>, frames: usize, rate: u32) {
        let period = frames as u64 * 1_000_000 / u64::from(rate.max(1));
        self.period_us
            .fetch_min(u32::try_from(period).unwrap_or(u32::MAX).max(1), Ordering::Relaxed);
        self.worst_mix_us.fetch_max(micros(start.elapsed()), Ordering::Relaxed);
        if let Some(previous) = previous {
            let gap = micros(start.duration_since(previous));
            self.worst_gap_us.fetch_max(gap, Ordering::Relaxed);
            if gap > self.tolerance_us() {
                self.gaps.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Logs and resets what happened since the last report.
    fn report(&self, secs: u64) {
        let ms = |us: u32| us as f32 / 1000.0;
        let gaps = self.gaps.swap(0, Ordering::Relaxed);
        let gap = ms(self.worst_gap_us.swap(0, Ordering::Relaxed));
        let mix = ms(self.worst_mix_us.swap(0, Ordering::Relaxed));
        let tolerance = ms(self.tolerance_us());
        if gaps > 0 {
            log::warn!(
                "audio output: {gaps} callbacks too late in {secs} s, the sound cut (longest wait {gap:.1} ms for {tolerance:.1} ms queued, longest mix {mix:.2} ms)"
            );
        } else {
            log::debug!("audio output: longest wait {gap:.1} ms for {tolerance:.1} ms queued, longest mix {mix:.2} ms");
        }
    }
}

fn micros(d: Duration) -> u32 {
    u32::try_from(d.as_micros()).unwrap_or(u32::MAX)
}

/// The output buffer to ask for. WASAPI's shared mode takes any length
/// (cpal reports only the device period as its range there); other hosts
/// keep their own choice.
fn output_buffer(rate: u32) -> (cpal::BufferSize, Duration) {
    if cfg!(windows) && rate > 0 {
        let frames = (f64::from(rate) * OUTPUT_QUEUE_SECS).round() as u32;
        (cpal::BufferSize::Fixed(frames), Duration::from_secs_f64(OUTPUT_QUEUE_SECS))
    } else {
        (cpal::BufferSize::Default, Duration::ZERO)
    }
}

/// The output buffer on a PulseAudio server (PipeWire included). cpal asks
/// it for twice the period, so a period of half [`OUTPUT_QUEUE_SECS`] queues
/// the same 40 ms as on Windows; the server's default would be two seconds.
fn pulse_output_buffer(rate: u32) -> (cpal::BufferSize, Duration) {
    let frames = (f64::from(rate) * OUTPUT_QUEUE_SECS / 2.0).round() as u32;
    (cpal::BufferSize::Fixed(frames.max(1)), Duration::from_secs_f64(OUTPUT_QUEUE_SECS))
}

/// How often the output thread reports the stream's health.
const HEALTH_REPORT: Duration = Duration::from_secs(5);

/// Requests to the output thread.
enum OutputCmd {
    /// Switch to the named device (`None` = system default).
    SetDevice(Option<String>),
    /// Set the requested voice-only output (`None` = the main output).
    SetVoiceDevice(Option<String>),
}

pub(crate) struct OutputThread {
    ctl: Option<Sender<OutputCmd>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl OutputThread {
    /// Asks the thread to reopen the stream on another device.
    pub(crate) fn set_device(&self, name: Option<String>) {
        if let Some(tx) = &self.ctl {
            let _ = tx.send(OutputCmd::SetDevice(name));
        }
    }

    /// Updates the voice-only output request without replacing voice sinks.
    pub(crate) fn set_voice_device(&self, name: Option<String>) {
        if let Some(tx) = &self.ctl {
            let _ = tx.send(OutputCmd::SetVoiceDevice(name));
        }
    }
}

impl Drop for OutputThread {
    fn drop(&mut self) {
        // Disconnecting the command channel stops the thread.
        drop(self.ctl.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Main-mix-clock voice frames (48 kHz) waiting for the independent voice
/// device. The ring holds at most 250 ms; the voice callback plays them 60 ms
/// behind the main mix and follows the main device's clock around that
/// target, and a queue past 180 ms (a device that stopped consuming for a
/// while) is cut back to the target at once.
const VOICE_QUEUE_MAX_FRAMES: usize = 12_000;
const VOICE_QUEUE_TARGET_FRAMES: usize = 2_880;
const VOICE_QUEUE_LIMIT_FRAMES: usize = 3 * VOICE_QUEUE_TARGET_FRAMES;

/// Largest correction of the voice device's read rate (±0.5 %): enough for
/// the drift between two sound cards, inaudible as a pitch change.
const VOICE_RATE_CORRECTION: f64 = 0.005;

/// Output frames the voice adapter renders per pass. Its input buffer holds
/// one pass of main-clock frames, so it is allocated once and never grows.
const VOICE_PASS_FRAMES: usize = 1_024;

#[derive(Clone)]
struct VoicePcmRing {
    producer: Arc<Mutex<Producer<f32>>>,
    consumer: Arc<Mutex<Consumer<f32>>>,
}

impl VoicePcmRing {
    fn new() -> Self {
        let (producer, consumer) = RingBuffer::new(VOICE_QUEUE_MAX_FRAMES * ENGINE_CHANNELS);
        Self {
            producer: Arc::new(Mutex::new(producer)),
            consumer: Arc::new(Mutex::new(consumer)),
        }
    }

    /// Queues whole stereo frames; what a full ring cannot take is dropped.
    fn push(&self, samples: &[f32]) {
        let mut producer = self.producer.lock();
        let room = producer.slots() / ENGINE_CHANNELS * ENGINE_CHANNELS;
        let whole = samples.len() / ENGINE_CHANNELS * ENGINE_CHANNELS;
        let _ = producer.push_partial_slice(&samples[..whole.min(room)]);
    }

    fn clear(&self) {
        let mut consumer = self.consumer.lock();
        let all = consumer.slots();
        discard(&mut consumer, all);
    }
}

/// Drops the oldest `samples` of the ring without copying them.
fn discard(consumer: &mut Consumer<f32>, samples: usize) {
    if let Ok(chunk) = consumer.read_chunk(samples.min(consumer.slots())) {
        chunk.commit_all();
    }
}

/// Starts the output thread on `device` (`None` = default) and waits until
/// the first stream is playing. `info` is kept up to date by the thread.
pub(crate) fn spawn(
    mixer: Arc<Mutex<Mixer>>,
    device: Option<String>,
    info: Arc<Mutex<OutputInfo>>,
    reference: EchoReference,
) -> Result<OutputThread, AudioError> {
    let (ready_tx, ready_rx) = bounded::<Result<(), AudioError>>(1);
    let (ctl_tx, ctl_rx) = unbounded::<OutputCmd>();
    let voice_ring = VoicePcmRing::new();
    let voice_separate = Arc::new(AtomicBool::new(false));
    let handle = thread::Builder::new()
        .name("aurora-audio-out".into())
        .spawn(move || output_main(mixer, device, info, ready_tx, ctl_rx, reference, voice_ring, voice_separate))
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let mut out = OutputThread {
        ctl: Some(ctl_tx),
        handle: Some(handle),
    };
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(out),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Do not block on a wedged driver: detach the thread.
            out.handle = None;
            Err(AudioError::Device("audio device did not start".into()))
        }
    }
}

fn output_main(
    mixer: Arc<Mutex<Mixer>>,
    mut wanted: Option<String>,
    info: Arc<Mutex<OutputInfo>>,
    ready_tx: Sender<Result<(), AudioError>>,
    ctl: Receiver<OutputCmd>,
    reference: EchoReference,
    voice_ring: VoicePcmRing,
    voice_separate: Arc<AtomicBool>,
) {
    let mut ready = Some(ready_tx);
    let mut voice = VoiceRoute::new(voice_ring.clone(), voice_separate.clone());
    loop {
        let mut reopen_now = false;
        voice.stop();
        match open_stream(&mixer, wanted.as_deref(), &reference, &voice_ring, &voice_separate) {
            Ok((stream, new_info, health, device)) => {
                log::info!(
                    "audio output: {} ({} Hz, {} ch)",
                    new_info.device_name,
                    new_info.sample_rate,
                    new_info.channels
                );
                *info.lock() = new_info.clone();
                if let Some(tx) = ready.take() {
                    let _ = tx.send(Ok(()));
                }
                voice.configure(&new_info, Instant::now());
                let watch = if wanted.is_none() {
                    devices::DefaultWatch::new(Direction::Output, &device)
                } else {
                    None
                };
                let mut last_report = Instant::now();
                let mut last_watch = Instant::now();
                let mut last_device_list = Instant::now();
                loop {
                    if last_report.elapsed() >= HEALTH_REPORT {
                        health.report(last_report.elapsed().as_secs());
                        last_report = Instant::now();
                    }
                    if let Some(watch) = &watch
                        && last_watch.elapsed() >= devices::DefaultWatch::EVERY
                    {
                        last_watch = Instant::now();
                        if watch.changed() {
                            log::info!("audio output: the default device changed; reopening");
                            reopen_now = true;
                            break;
                        }
                    }
                    voice.check_stream(Instant::now());
                    if last_device_list.elapsed() >= devices::DefaultWatch::EVERY {
                        last_device_list = Instant::now();
                        voice.check_devices(&new_info, Instant::now());
                    }
                    match ctl.recv_timeout(Duration::from_millis(250)) {
                        Ok(OutputCmd::SetDevice(name)) => {
                            if name != wanted {
                                wanted = name;
                                voice.stop();
                                reopen_now = true;
                                break;
                            }
                        }
                        Ok(OutputCmd::SetVoiceDevice(name)) => {
                            if name != voice.wanted {
                                voice.wanted = name;
                                voice.backoff = VoiceBackoff::default();
                                voice.configure(&new_info, Instant::now());
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if health.failed.load(Ordering::Relaxed) {
                                log::warn!("audio output lost; reopening the device");
                                voice.stop();
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                // The callback stops before the new stream is built.
                voice.stop();
                drop(stream);
            }
            Err(e) => {
                if let Some(tx) = ready.take() {
                    let _ = tx.send(Err(e));
                    return;
                }
                log::debug!("audio output reopen failed: {e}");
            }
        }
        if reopen_now {
            continue;
        }
        // Retry after a pause; selecting a device retries at once.
        match ctl.recv_timeout(Duration::from_secs(1)) {
            Ok(OutputCmd::SetDevice(name)) => wanted = name,
            Ok(OutputCmd::SetVoiceDevice(name)) => {
                voice.wanted = name;
                voice.backoff = VoiceBackoff::default();
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn output_device_snapshot() -> Vec<String> {
    let mut devices = devices::output_devices();
    devices.sort();
    devices.dedup();
    devices
}

/// The separate voice stream playing on its own device.
struct VoiceOut {
    name: String,
    _stream: cpal::Stream,
    health: Arc<StreamHealth>,
    opened: Instant,
}

/// A voice device chosen but not playing: voice goes through the main output
/// meanwhile.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VoiceFallback {
    name: String,
    /// The output list when it failed.
    devices: Vec<String>,
    /// When to try the device again while it stays listed.
    retry_at: Instant,
}

impl VoiceFallback {
    /// Whether to open the device again, given the output list now: at once
    /// when the list changed (the device came back, or another one with its
    /// name), otherwise once the backoff has passed if the device is listed
    /// (a stream invalidated by a format change, a device another program
    /// held exclusively). A device still missing waits for the list to change.
    fn retry_now(&self, devices: &[String], now: Instant) -> bool {
        devices != self.devices.as_slice() || (now >= self.retry_at && devices.iter().any(|name| name == &self.name))
    }
}

/// The wait before the voice device is tried again after a failure: 2 s,
/// doubled at each failure in a row up to 30 s. A stream that played 30 s
/// before failing starts the count again.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct VoiceBackoff {
    failures: u32,
}

impl VoiceBackoff {
    const FIRST: Duration = Duration::from_secs(2);
    const MAX: Duration = Duration::from_secs(30);
    const STABLE: Duration = Duration::from_secs(30);

    /// The wait after a failure of a stream that played for `played` (zero
    /// when it did not open).
    fn after_failure(&mut self, played: Duration) -> Duration {
        if played >= Self::STABLE {
            self.failures = 0;
        }
        let delay = Self::FIRST.saturating_mul(1 << self.failures.min(4)).min(Self::MAX);
        self.failures = self.failures.saturating_add(1);
        delay
    }
}

/// The voice-only output requested by the viewer, and its stream when it
/// plays on a device other than the main output's.
struct VoiceRoute {
    /// The requested voice device (`None` = the main output).
    wanted: Option<String>,
    stream: Option<VoiceOut>,
    fallback: Option<VoiceFallback>,
    backoff: VoiceBackoff,
    ring: VoicePcmRing,
    /// Read by the main callback: the Voice bus goes to `ring`.
    separate: Arc<AtomicBool>,
}

impl VoiceRoute {
    fn new(ring: VoicePcmRing, separate: Arc<AtomicBool>) -> Self {
        Self {
            wanted: None,
            stream: None,
            fallback: None,
            backoff: VoiceBackoff::default(),
            ring,
            separate,
        }
    }

    /// Voice back to the main mix; the stream (if any) is closed.
    fn stop(&mut self) {
        self.separate.store(false, Ordering::Relaxed);
        self.stream = None;
        self.ring.clear();
    }

    /// Opens the voice stream that `wanted` asks for beside `main`, or
    /// returns voice to the main mix.
    fn configure(&mut self, main: &OutputInfo, now: Instant) {
        match voice_stream_for(&main.device_name, self.wanted.as_deref().unwrap_or_default()) {
            VoiceStream::Same => {
                self.stop();
                self.fallback = None;
            }
            VoiceStream::Separate(name) => {
                if self
                    .stream
                    .as_ref()
                    .is_some_and(|out| out.name == name && !out.health.failed.load(Ordering::Relaxed))
                {
                    return;
                }
                self.stop();
                match open_voice_stream(&name, &self.ring) {
                    Ok((stream, health)) => {
                        if self.fallback.take().is_some() {
                            log::info!("voice output: {name} is back");
                        }
                        self.stream = Some(VoiceOut {
                            name,
                            _stream: stream,
                            health,
                            opened: now,
                        });
                        self.separate.store(true, Ordering::Relaxed);
                    }
                    Err(error) => {
                        log::debug!("voice output: {name} cannot be opened ({error})");
                        self.failed(name, output_device_snapshot(), Duration::ZERO, now);
                    }
                }
            }
        }
    }

    /// Voice falls back to the main mix, and the device is tried again later.
    fn failed(&mut self, name: String, devices: Vec<String>, played: Duration, now: Instant) {
        self.stop();
        let delay = self.backoff.after_failure(played);
        if self.fallback.as_ref().is_none_or(|fallback| fallback.name != name) {
            log::warn!("voice output: {name} unavailable, using the audio output");
        }
        log::debug!(
            "voice output: next try of {name} in {} s or when the device list changes",
            delay.as_secs()
        );
        self.fallback = Some(VoiceFallback {
            name,
            devices,
            retry_at: now + delay,
        });
    }

    /// Whether the output list is read for voice: only while a separate
    /// voice stream plays or waits for its device.
    fn lists_devices(&self) -> bool {
        self.stream.is_some() || self.fallback.is_some()
    }

    /// A stream the device reported lost.
    fn check_stream(&mut self, now: Instant) {
        if let Some(out) = &self.stream
            && out.health.failed.load(Ordering::Relaxed)
        {
            let (name, played) = (out.name.clone(), now.saturating_duration_since(out.opened));
            self.failed(name, output_device_snapshot(), played, now);
        }
    }

    /// The periodic look at the output list: a device that disappeared under
    /// its stream, or one to try again. The list is read only while a
    /// separate stream plays or waits for its device (enumeration probes the
    /// ALSA devices), never while voice simply follows the main output.
    fn check_devices(&mut self, main: &OutputInfo, now: Instant) {
        if !self.lists_devices() {
            return;
        }
        let devices = output_device_snapshot();
        if let Some(out) = &self.stream {
            if !devices.iter().any(|name| name == &out.name) {
                let (name, played) = (out.name.clone(), now.saturating_duration_since(out.opened));
                self.failed(name, devices, played, now);
            }
        } else if self.fallback.as_ref().is_some_and(|fallback| fallback.retry_now(&devices, now)) {
            self.configure(main, now);
        }
    }
}

/// Opens `name` (or the default device), and returns the device opened too.
/// A named device that is missing or fails to open falls back to the default
/// one.
fn open_stream(
    mixer: &Arc<Mutex<Mixer>>,
    name: Option<&str>,
    reference: &EchoReference,
    voice_ring: &VoicePcmRing,
    voice_separate: &Arc<AtomicBool>,
) -> Result<(cpal::Stream, OutputInfo, Arc<StreamHealth>, cpal::Device), AudioError> {
    let picked = devices::pick(Direction::Output, name).ok_or(AudioError::NoDevice)?;
    match open_device(mixer, &picked.device, reference, voice_ring, voice_separate) {
        Ok((stream, mut info, health)) => {
            info.fallback = picked.fallback;
            Ok((stream, info, health, picked.device))
        }
        Err(e) if name.is_some() && !picked.fallback => {
            log::warn!("audio output device {name:?} cannot be opened ({e}); using the default");
            let picked = devices::pick(Direction::Output, None).ok_or(AudioError::NoDevice)?;
            let (stream, mut info, health) = open_device(mixer, &picked.device, reference, voice_ring, voice_separate)?;
            info.fallback = true;
            Ok((stream, info, health, picked.device))
        }
        Err(e) => Err(e),
    }
}

/// Opens only the requested voice device. It never substitutes the system
/// default: that case is handled by returning voice to the main mix.
fn open_voice_stream(name: &str, ring: &VoicePcmRing) -> Result<(cpal::Stream, Arc<StreamHealth>), AudioError> {
    let device = devices::find(Direction::Output, name).ok_or_else(|| AudioError::Device("device is missing".into()))?;
    let supported = device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
    let mut config = supported.config();
    let queued;
    (config.buffer_size, queued) = if devices::is_pulse(&device) {
        pulse_output_buffer(config.sample_rate)
    } else {
        output_buffer(config.sample_rate)
    };
    let health = Arc::new(StreamHealth::new(queued));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_voice::<f32>(&device, &config, ring, &health),
        SampleFormat::I16 => build_voice::<i16>(&device, &config, ring, &health),
        SampleFormat::U16 => build_voice::<u16>(&device, &config, ring, &health),
        SampleFormat::I32 => build_voice::<i32>(&device, &config, ring, &health),
        SampleFormat::I24 => build_voice::<cpal::I24>(&device, &config, ring, &health),
        SampleFormat::F64 => build_voice::<f64>(&device, &config, ring, &health),
        SampleFormat::I8 => build_voice::<i8>(&device, &config, ring, &health),
        SampleFormat::U8 => build_voice::<u8>(&device, &config, ring, &health),
        other => Err(AudioError::Device(format!("unsupported device sample format {other:?}"))),
    }?;
    stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
    log::info!("voice output: {name} ({} Hz, {} ch)", config.sample_rate, config.channels);
    Ok((stream, health))
}

fn open_device(
    mixer: &Arc<Mutex<Mixer>>,
    device: &cpal::Device,
    reference: &EchoReference,
    voice_ring: &VoicePcmRing,
    voice_separate: &Arc<AtomicBool>,
) -> Result<(cpal::Stream, OutputInfo, Arc<StreamHealth>), AudioError> {
    let supported = device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
    let mut config = supported.config();
    let queued;
    (config.buffer_size, queued) = if devices::is_pulse(device) {
        pulse_output_buffer(config.sample_rate)
    } else {
        output_buffer(config.sample_rate)
    };
    let health = Arc::new(StreamHealth::new(queued));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        SampleFormat::I16 => build::<i16>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        SampleFormat::U16 => build::<u16>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        SampleFormat::I32 => build::<i32>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        // The PulseAudio host reports 24-bit sinks (s24le, s24-32le) as I24.
        SampleFormat::I24 => build::<cpal::I24>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        SampleFormat::F64 => build::<f64>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        SampleFormat::I8 => build::<i8>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        SampleFormat::U8 => build::<u8>(device, &config, mixer, &health, reference, voice_ring, voice_separate),
        other => Err(AudioError::Device(format!("unsupported device sample format {other:?}"))),
    }?;
    stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
    let info = OutputInfo {
        device_name: devices::device_name(device),
        sample_rate: config.sample_rate,
        channels: config.channels,
        fallback: false,
    };
    Ok((stream, info, health))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: &Arc<Mutex<Mixer>>,
    health: &Arc<StreamHealth>,
    reference: &EchoReference,
    voice_ring: &VoicePcmRing,
    voice_separate: &Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut adapter = OutputAdapter::with_voice(
        config.sample_rate,
        usize::from(config.channels),
        voice_ring.clone(),
        voice_separate.clone(),
    );
    adapter.tap = Some(RenderTap::new(reference.clone()));
    let reference = reference.clone();
    let mixer = mixer.clone();
    let health = health.clone();
    let callback_health = health.clone();
    let (rate, channels) = (config.sample_rate, usize::from(config.channels).max(1));
    let mut previous = None;
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], info| {
                let start = Instant::now();
                let timestamp = info.timestamp();
                let delay = timestamp.playback.duration_since(timestamp.callback).as_millis().min(500) as i32;
                reference.delay(delay);
                let mut m = mixer.lock();
                adapter.fill(&mut m, data);
                drop(m);
                callback_health.record(start, previous.replace(start), data.len() / channels, rate);
            },
            move |err| match err.kind() {
                cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::HostUnavailable => {
                    health.failed.store(true, Ordering::Relaxed);
                }
                _ => log::debug!("audio stream: {err}"),
            },
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}

fn build_voice<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    ring: &VoicePcmRing,
    health: &Arc<StreamHealth>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut adapter = VoiceOutputAdapter::new(config.sample_rate, usize::from(config.channels));
    let ring = ring.clone();
    let health = health.clone();
    let callback_health = health.clone();
    let (rate, channels) = (config.sample_rate, usize::from(config.channels).max(1));
    let mut previous = None;
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], _| {
                let start = Instant::now();
                adapter.fill(&ring, data);
                callback_health.record(start, previous.replace(start), data.len() / channels, rate);
            },
            move |err| match err.kind() {
                cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::HostUnavailable => {
                    health.failed.store(true, Ordering::Relaxed);
                }
                _ => log::debug!("voice output stream: {err}"),
            },
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}

/// Reads the main clock's 48 kHz voice queue at the independent device clock.
///
/// The queue is the ring plus the frames already taken from it and not yet
/// played (`input` past `pos`): the read rate follows that whole depth, so
/// the voice stays [`VOICE_QUEUE_TARGET_FRAMES`] behind the main mix for the
/// whole call whatever the two clocks do. Each pass takes from the ring only
/// the frames it interpolates, into a buffer allocated once: the callback
/// copies a period, never the backlog, and never allocates.
struct VoiceOutputAdapter {
    channels: usize,
    base_step: f64,
    /// Read position in `input` (frames); the frame before it is kept for
    /// the interpolation.
    pos: f64,
    /// Main-clock stereo frames taken from the ring (capacity: one pass).
    input: Box<[f32]>,
    input_frames: usize,
    /// Queue depth (frames), smoothed over the main callbacks' bursts.
    depth: f64,
    /// Playing. Off until the queue first reaches its target, and again
    /// after it ran dry, so that playback starts with the full margin.
    primed: bool,
}

/// The read step that brings the queue back to its target: a proportional
/// correction (2 % of the relative error), at most ±0.5 %.
fn voice_rate_step(base: f64, queued_frames: f64) -> f64 {
    let error = (queued_frames - VOICE_QUEUE_TARGET_FRAMES as f64) / VOICE_QUEUE_TARGET_FRAMES as f64;
    base * (1.0 + (error * 0.02).clamp(-VOICE_RATE_CORRECTION, VOICE_RATE_CORRECTION))
}

impl VoiceOutputAdapter {
    fn new(device_rate: u32, channels: usize) -> Self {
        let base_step = f64::from(ENGINE_SAMPLE_RATE) / f64::from(device_rate.max(1));
        // A pass of n frames reads up to (n - 1) × step frames past `pos`
        // (below 2 after a pass), and two more for the interpolation.
        let capacity = (VOICE_PASS_FRAMES as f64 * base_step * (1.0 + VOICE_RATE_CORRECTION)).ceil() as usize + 6;
        let mut adapter = Self {
            channels: channels.max(1),
            base_step,
            pos: 1.0,
            input: vec![0.0; capacity * ENGINE_CHANNELS].into_boxed_slice(),
            input_frames: 0,
            depth: 0.0,
            primed: false,
        };
        adapter.restart();
        adapter
    }

    /// Back to waiting for the target.
    fn restart(&mut self) {
        self.primed = false;
        self.input[..ENGINE_CHANNELS].fill(0.0);
        self.input_frames = 1;
        self.pos = 1.0;
    }

    /// Frames between the main mix and what this device plays next.
    fn queued_frames(&self, ring_frames: usize) -> f64 {
        ring_frames as f64 + (self.input_frames as f64 - self.pos).max(0.0)
    }

    fn fill<T: SizedSample + FromSample<f32>>(&mut self, ring: &VoicePcmRing, data: &mut [T]) {
        let frames = data.len() / self.channels;
        let mut consumer = ring.consumer.lock();
        let mut ring_frames = consumer.slots() / ENGINE_CHANNELS;

        // A device that stopped consuming for a while comes back to the
        // target at once instead of playing the stale voice late.
        if self.queued_frames(ring_frames) > VOICE_QUEUE_LIMIT_FRAMES as f64 {
            let excess = (self.queued_frames(ring_frames) - VOICE_QUEUE_TARGET_FRAMES as f64) as usize;
            discard(&mut consumer, excess.min(ring_frames) * ENGINE_CHANNELS);
            ring_frames = consumer.slots() / ENGINE_CHANNELS;
            self.depth = self.queued_frames(ring_frames);
        }

        let queued = self.queued_frames(ring_frames);
        if !self.primed {
            if queued < VOICE_QUEUE_TARGET_FRAMES as f64 {
                drop(consumer);
                data.fill(T::from_sample(0.0));
                return;
            }
            // The frame before the first one played is a copy of it, so
            // that the voice starts without a step from silence.
            if let Ok(chunk) = consumer.read_chunk(ENGINE_CHANNELS) {
                let (first, rest) = chunk.as_slices();
                for (slot, &sample) in self.input.iter_mut().zip(first.iter().chain(rest)) {
                    *slot = sample;
                }
            }
            self.primed = true;
            self.depth = queued;
        }
        self.depth += (queued - self.depth) * 0.05;
        let step = voice_rate_step(self.base_step, self.depth);
        let capacity = self.input.len() / ENGINE_CHANNELS;

        let mut played = 0;
        while played < frames {
            let n = (frames - played).min(VOICE_PASS_FRAMES);
            // Take only the frames this pass interpolates.
            let last = (self.pos + (n - 1) as f64 * step) as usize;
            let need = (last + 3).min(capacity);
            if need > self.input_frames {
                let take = need - self.input_frames;
                let start = self.input_frames * ENGINE_CHANNELS;
                let (popped, _) = consumer.pop_partial_slice(&mut self.input[start..start + take * ENGINE_CHANNELS]);
                self.input_frames += popped.len() / ENGINE_CHANNELS;
            }
            let mut dry = false;
            for frame in data[played * self.channels..(played + n) * self.channels].chunks_exact_mut(self.channels) {
                let i = self.pos as usize;
                if i + 2 >= self.input_frames {
                    dry = true;
                    break;
                }
                let t = (self.pos - i as f64) as f32;
                let at = |frame: usize, channel: usize| self.input[frame * ENGINE_CHANNELS + channel];
                let left = hermite(at(i - 1, 0), at(i, 0), at(i + 1, 0), at(i + 2, 0), t);
                let right = hermite(at(i - 1, 1), at(i, 1), at(i + 1, 1), at(i + 2, 1), t);
                write_frame(frame, left, right);
                self.pos += step;
                played += 1;
            }
            if dry {
                // The queue ran dry: silence, then the full margin again.
                drop(consumer);
                data[played * self.channels..].fill(T::from_sample(0.0));
                self.restart();
                return;
            }
            // Keep the frame before `pos` and what follows it.
            let used = ((self.pos as usize).saturating_sub(1)).min(self.input_frames);
            if used > 0 {
                self.input
                    .copy_within(used * ENGINE_CHANNELS..self.input_frames * ENGINE_CHANNELS, 0);
                self.input_frames -= used;
                self.pos -= used as f64;
            }
        }
    }
}

/// Writes one stereo frame into a device frame of any channel count.
fn write_frame<T: SizedSample + FromSample<f32>>(frame: &mut [T], left: f32, right: f32) {
    if frame.len() == 1 {
        frame[0] = T::from_sample((left + right) * 0.5);
    } else {
        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample = T::from_sample(match channel {
                0 => left,
                1 => right,
                _ => 0.0,
            });
        }
    }
}

fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// Converts the mixer's 48 kHz stereo into the device's rate / layout.
pub(crate) struct OutputAdapter {
    channels: usize,
    resampler: Option<Resampler>,
    step: f64,
    mix_buf: Vec<f32>,
    voice_buf: Vec<f32>,
    reference_buf: Vec<f32>,
    out_buf: Vec<f32>,
    tap: Option<RenderTap>,
    voice_ring: VoicePcmRing,
    voice_separate: Arc<AtomicBool>,
}

impl OutputAdapter {
    #[cfg(test)]
    pub(crate) fn new(device_rate: u32, channels: usize) -> Self {
        Self::with_voice(device_rate, channels, VoicePcmRing::new(), Arc::new(AtomicBool::new(false)))
    }

    fn with_voice(device_rate: u32, channels: usize, voice_ring: VoicePcmRing, voice_separate: Arc<AtomicBool>) -> Self {
        let resampler = (device_rate != ENGINE_SAMPLE_RATE && device_rate > 0)
            .then(|| Resampler::new(ENGINE_SAMPLE_RATE, device_rate, ENGINE_CHANNELS));
        Self {
            channels: channels.max(1),
            resampler,
            step: f64::from(ENGINE_SAMPLE_RATE) / f64::from(device_rate.max(1)),
            mix_buf: Vec::with_capacity(8192),
            voice_buf: Vec::with_capacity(8192),
            reference_buf: Vec::with_capacity(8192),
            out_buf: Vec::with_capacity(8192),
            tap: None,
            voice_ring,
            voice_separate,
        }
    }

    pub(crate) fn fill<T: SizedSample + FromSample<f32>>(&mut self, mixer: &mut Mixer, data: &mut [T]) {
        let frames = data.len() / self.channels;
        let separate_voice = self.voice_separate.load(Ordering::Relaxed);
        let stereo: &[f32] = match self.resampler.as_mut() {
            None => {
                self.mix_buf.resize(frames * ENGINE_CHANNELS, 0.0);
                self.voice_buf.resize(frames * ENGINE_CHANNELS, 0.0);
                mixer.render_split(&mut self.mix_buf, &mut self.voice_buf, separate_voice);
                if separate_voice {
                    self.voice_ring.push(&self.voice_buf);
                }
                if let Some(tap) = &mut self.tap {
                    tap_reference(tap, &self.mix_buf, &self.voice_buf, &mut self.reference_buf, separate_voice);
                }
                &self.mix_buf
            }
            Some(rs) => {
                self.out_buf.clear();
                let mut produced = 0;
                while produced < frames {
                    produced += rs.pull(&mut self.out_buf, frames - produced);
                    if produced < frames {
                        let need = (((frames - produced) as f64) * self.step).ceil() as usize + 4;
                        self.mix_buf.resize(need.min(8192) * ENGINE_CHANNELS, 0.0);
                        self.voice_buf.resize(self.mix_buf.len(), 0.0);
                        mixer.render_split(&mut self.mix_buf, &mut self.voice_buf, separate_voice);
                        if separate_voice {
                            self.voice_ring.push(&self.voice_buf);
                        }
                        if let Some(tap) = &mut self.tap {
                            tap_reference(tap, &self.mix_buf, &self.voice_buf, &mut self.reference_buf, separate_voice);
                        }
                        rs.push(&self.mix_buf);
                    }
                }
                &self.out_buf
            }
        };
        for (f, frame) in data.chunks_exact_mut(self.channels).enumerate() {
            let l = stereo.get(2 * f).copied().unwrap_or(0.0);
            let r = stereo.get(2 * f + 1).copied().unwrap_or(0.0);
            if self.channels == 1 {
                frame[0] = T::from_sample((l + r) * 0.5);
            } else {
                for (c, s) in frame.iter_mut().enumerate() {
                    *s = T::from_sample(match c {
                        0 => l,
                        1 => r,
                        _ => 0.0,
                    });
                }
            }
        }
    }
}

fn tap_reference(tap: &mut RenderTap, main: &[f32], voice: &[f32], reference: &mut Vec<f32>, separate_voice: bool) {
    if !separate_voice {
        tap.write(main);
        return;
    }
    reference.resize(main.len(), 0.0);
    for ((out, &main), &voice) in reference.iter_mut().zip(main).zip(voice) {
        *out = (main + voice).clamp(-1.0, 1.0);
    }
    // Both parts are from the main mix clock. The reference delay remains
    // measured against the main device; the separate device's additional
    // hardware latency is not independently estimated.
    tap.write(reference);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Channel;
    use crate::mixer::{ClipVoice, Command, SharedParams};
    use cpal::Sample;

    #[test]
    fn health_counts_callbacks_later_than_the_queued_sound() {
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        // The device chose its buffer; 441 frames at 44.1 kHz: a 10 ms period.
        let h = StreamHealth::new(Duration::ZERO);
        h.record(t0, None, 441, 44_100);
        h.record(t0 + ms(10), Some(t0), 441, 44_100);
        h.record(t0 + ms(25), Some(t0 + ms(10)), 441, 44_100);
        assert_eq!(h.gaps.load(Ordering::Relaxed), 0, "jitter under two periods");
        h.record(t0 + ms(55), Some(t0 + ms(25)), 1323, 44_100);
        assert_eq!(h.gaps.load(Ordering::Relaxed), 1);
        assert_eq!(h.worst_gap_us.load(Ordering::Relaxed), 30_000);
        assert_eq!(
            h.period_us.load(Ordering::Relaxed),
            10_000,
            "a refill after a wait is not the period"
        );
        h.report(5);
        assert_eq!(h.gaps.load(Ordering::Relaxed), 0, "a report starts a new count");
        assert_eq!(h.worst_gap_us.load(Ordering::Relaxed), 0);

        // 40 ms queued: a 30 ms wait is absorbed, a 50 ms one is not.
        let h = StreamHealth::new(ms(40));
        h.record(t0, None, 441, 44_100);
        h.record(t0 + ms(30), Some(t0), 1323, 44_100);
        assert_eq!(h.gaps.load(Ordering::Relaxed), 0);
        h.record(t0 + ms(80), Some(t0 + ms(30)), 1764, 44_100);
        assert_eq!(h.gaps.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn output_buffer_queues_forty_ms_on_windows() {
        let (size, queued) = output_buffer(44_100);
        if cfg!(windows) {
            assert_eq!(size, cpal::BufferSize::Fixed(1764));
            assert_eq!(queued, Duration::from_millis(40));
        } else {
            assert_eq!(size, cpal::BufferSize::Default);
        }
    }

    #[test]
    fn pulse_output_buffer_queues_forty_ms_in_two_periods() {
        let (size, queued) = pulse_output_buffer(48_000);
        assert_eq!(size, cpal::BufferSize::Fixed(960));
        assert_eq!(queued, Duration::from_millis(40));
    }

    #[test]
    fn separate_voice_clock_correction_is_bounded_and_tracks_queue_drift() {
        let base = 48_000.0 / 44_100.0;
        let target = VOICE_QUEUE_TARGET_FRAMES as f64;
        assert!(voice_rate_step(base, target + 100.0) > base);
        assert!(voice_rate_step(base, target - 100.0) < base);
        assert!((voice_rate_step(base, f64::MAX) / base - 1.0 - VOICE_RATE_CORRECTION).abs() < 1e-10);
        assert!((voice_rate_step(base, 0.0) / base - 1.0 + VOICE_RATE_CORRECTION).abs() < 1e-10);
    }

    #[test]
    fn separate_voice_adapter_waits_for_its_target_then_plays() {
        let ring = VoicePcmRing::new();
        let mut adapter = VoiceOutputAdapter::new(48_000, 2);
        let mut out = vec![1.0f32; 480 * 2];
        ring.push(&vec![0.25f32; (VOICE_QUEUE_TARGET_FRAMES - 1) * 2]);
        adapter.fill(&ring, &mut out);
        assert!(out.iter().all(|sample| *sample == 0.0), "silent below the target");
        assert_eq!(
            ring.consumer.lock().slots(),
            (VOICE_QUEUE_TARGET_FRAMES - 1) * 2,
            "nothing taken yet"
        );

        ring.push(&[0.25f32; 2]);
        adapter.fill(&ring, &mut out);
        assert!(out.iter().all(|sample| (*sample - 0.25).abs() < 1e-3), "no step from silence");
        // Only the frames played (and the interpolation's three) left the ring.
        let taken = VOICE_QUEUE_TARGET_FRAMES - ring.consumer.lock().slots() / 2;
        assert!((480..=484).contains(&taken), "{taken} frames taken for 480 played");
    }

    #[test]
    fn separate_voice_adapter_cuts_a_stale_queue_back_to_its_target() {
        let ring = VoicePcmRing::new();
        let mut adapter = VoiceOutputAdapter::new(48_000, 1);
        ring.push(&vec![0.25f32; VOICE_QUEUE_MAX_FRAMES * 2]);
        let mut out = vec![0.0f32; 480];
        adapter.fill(&ring, &mut out);
        let left = adapter.queued_frames(ring.consumer.lock().slots() / 2);
        assert!((left - (VOICE_QUEUE_TARGET_FRAMES - 480) as f64).abs() < 8.0, "{left} frames left");
        assert!(out.iter().all(|sample| (*sample - 0.25).abs() < 1e-3), "mono takes both sides");
    }

    /// Runs the main clock (blocks of `block` frames at 48 kHz) against a
    /// voice device of `rate` Hz whose clock runs `drift` faster (a ratio),
    /// for `callbacks` callbacks of `period` frames. Returns the total queue
    /// depth at each callback once settled, and the count of output
    /// samples that were not the voice (gaps).
    fn run_voice_clocks(rate: u32, drift: f64, block: usize, period: usize, callbacks: usize) -> (Vec<f64>, usize) {
        let ring = VoicePcmRing::new();
        let mut adapter = VoiceOutputAdapter::new(rate, 2);
        let samples = vec![0.25f32; block * 2];
        let mut out = vec![0.0f32; period * 2];
        let main_period = block as f64 / 48_000.0;
        let voice_period = period as f64 / (f64::from(rate) * drift);
        let (mut next_main, mut next_voice) = (0.0, voice_period / 3.0);
        let (mut depths, mut gaps, mut started) = (Vec::new(), 0, false);
        for callback in 0..callbacks {
            while next_main <= next_voice {
                ring.push(&samples);
                next_main += main_period;
            }
            next_voice += voice_period;
            // The delay at the callback: what the main mix queued and this
            // device has not played yet. Ten seconds to settle, then every
            // callback counts.
            if callback as f64 * voice_period > 10.0 {
                depths.push(adapter.queued_frames(ring.consumer.lock().slots() / 2));
            }
            adapter.fill(&ring, &mut out);
            started |= out.iter().any(|sample| *sample != 0.0);
            if started {
                gaps += out.iter().filter(|sample| (**sample - 0.25).abs() > 1e-3).count();
            }
        }
        (depths, gaps)
    }

    #[test]
    fn separate_voice_delay_stays_at_its_target_for_a_long_call() {
        let target = VOICE_QUEUE_TARGET_FRAMES as f64;
        // (device rate, its clock against the main one, main block, device period)
        let cases = [
            (48_000, 1.0, 480, 480),
            (48_000, 1.0, 1_024, 441),
            (48_000, 1.002, 480, 480),
            (48_000, 0.998, 480, 480),
            (44_100, 1.0, 480, 441),
            (44_100, 1.001, 960, 441),
            (44_100, 0.999, 512, 1_764),
            (16_000, 1.0005, 480, 160),
        ];
        for (rate, drift, block, period) in cases {
            // Ten minutes of the voice device's callbacks.
            let callbacks = (600.0 * f64::from(rate) / period as f64) as usize;
            let (depths, gaps) = run_voice_clocks(rate, drift, block, period, callbacks);
            assert!(depths.len() > 1_000, "{rate} Hz: {} callbacks counted", depths.len());
            let (low, high) = depths.iter().fold((f64::MAX, 0.0f64), |(lo, hi), &d| (lo.min(d), hi.max(d)));
            // Within a main block and a device period of the target: the
            // total delay neither grows nor shrinks over the call.
            let margin = (block + period) as f64 * 48_000.0 / f64::from(rate) + 300.0;
            assert!(
                low > target - margin && high < target + margin,
                "{rate} Hz × {drift}, block {block}, period {period}: depth {low:.0}..{high:.0} for {target} ± {margin:.0}"
            );
            let tail = &depths[depths.len() - depths.len() / 10..];
            let end = tail.iter().sum::<f64>() / tail.len() as f64;
            assert!(
                (end - target).abs() < 400.0,
                "{rate} Hz × {drift}: {end:.0} frames queued at the end"
            );
            assert_eq!(gaps, 0, "{rate} Hz × {drift}: the voice never ran dry nor skipped");
        }
    }

    #[test]
    fn a_failed_voice_device_still_listed_is_retried_after_a_backoff() {
        let t0 = Instant::now();
        let listed = vec!["Headset".to_owned(), "Speakers".to_owned()];
        let mut backoff = VoiceBackoff::default();
        let delay = backoff.after_failure(Duration::from_secs(1));
        let fallback = VoiceFallback {
            name: "Headset".into(),
            devices: listed.clone(),
            retry_at: t0 + delay,
        };
        // A stream invalidated while its device stays listed: the list does
        // not change, the timer brings the device back.
        assert!(!fallback.retry_now(&listed, t0), "not before the backoff");
        assert!(fallback.retry_now(&listed, t0 + delay), "the timer retries an unchanged list");
        // A device still missing waits for the list to change.
        let gone = vec!["Speakers".to_owned()];
        let missing = VoiceFallback {
            devices: gone.clone(),
            ..fallback.clone()
        };
        assert!(!missing.retry_now(&gone, t0 + Duration::from_secs(600)));
        assert!(missing.retry_now(&listed, t0), "a changed list retries at once");
    }

    #[test]
    fn voice_backoff_doubles_up_to_its_limit_and_restarts_after_a_stable_stream() {
        let mut backoff = VoiceBackoff::default();
        let delays: Vec<u64> = (0..7).map(|_| backoff.after_failure(Duration::ZERO).as_secs()).collect();
        assert_eq!(delays, [2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(
            backoff.after_failure(Duration::from_secs(5)).as_secs(),
            30,
            "a short play keeps the count"
        );
        assert_eq!(
            backoff.after_failure(VoiceBackoff::STABLE).as_secs(),
            2,
            "a stream that played starts again"
        );
    }

    #[test]
    fn voice_lists_devices_only_for_a_separate_route_or_a_pending_fallback() {
        let mut route = VoiceRoute::new(VoicePcmRing::new(), Arc::new(AtomicBool::new(false)));
        assert!(!route.lists_devices(), "voice on the main output: no enumeration");
        route.wanted = Some("Speakers".into());
        let main = OutputInfo {
            device_name: "Speakers".into(),
            ..OutputInfo::default()
        };
        // The requested device is the main one: voice stays on the main mix.
        route.configure(&main, Instant::now());
        assert!(!route.lists_devices());
        route.fallback = Some(VoiceFallback {
            name: "Headset".into(),
            devices: Vec::new(),
            retry_at: Instant::now(),
        });
        assert!(route.lists_devices(), "a pending fallback watches the list");
        route.configure(&main, Instant::now());
        assert!(!route.lists_devices(), "back on the main output: the fallback is dropped");
    }

    #[test]
    fn adapter_resamples_and_maps_channels() {
        let params = Arc::new(SharedParams::new());
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut mixer = Mixer::new(params, rx);
        let data: Arc<[f32]> = vec![0.5f32; 480_000].into();
        tx.send(Command::PlayClip(ClipVoice::new(Channel::Ui, data, 1.0))).unwrap();

        // 44.1 kHz, 6 channels (5.1), i16.
        let mut adapter = OutputAdapter::new(44_100, 6);
        let mut out = vec![0i16; 441 * 6];
        for _ in 0..10 {
            adapter.fill(&mut mixer, &mut out);
        }
        let full = i16::from_sample(0.5f32);
        for frame in out.as_chunks::<6>().0 {
            assert!((i32::from(frame[0]) - i32::from(full)).abs() < 50);
            assert!((i32::from(frame[1]) - i32::from(full)).abs() < 50);
            assert!(frame[2..].iter().all(|&s| s == 0));
        }
    }

    #[test]
    fn adapter_fills_a_24_bit_device() {
        let params = Arc::new(SharedParams::new());
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut mixer = Mixer::new(params, rx);
        let data: Arc<[f32]> = vec![0.5f32; 48_000].into();
        tx.send(Command::PlayClip(ClipVoice::new(Channel::Ui, data, 1.0))).unwrap();

        // 48 kHz stereo, I24 (a 24-bit sink of the PulseAudio host).
        let mut adapter = OutputAdapter::new(48_000, 2);
        let mut out = vec![cpal::I24::EQUILIBRIUM; 480 * 2];
        for _ in 0..10 {
            adapter.fill(&mut mixer, &mut out);
        }
        let full = cpal::I24::from_sample(0.5f32).inner();
        assert!(out.iter().all(|s| (s.inner() - full).abs() < 4096));
    }
}
