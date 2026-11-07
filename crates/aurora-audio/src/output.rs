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

/// Main-mix-clock voice blocks waiting for the independent voice device.
/// 250 ms is the hard latency ceiling; the voice callback adaptively follows
/// the main device clock around a 60 ms target.
const VOICE_QUEUE_MAX_FRAMES: usize = 12_000;
const VOICE_QUEUE_TARGET_FRAMES: usize = 2_880;

#[derive(Clone)]
struct VoicePcmRing {
    producer: Arc<Mutex<Producer<f32>>>,
    consumer: Arc<Mutex<Consumer<f32>>>,
}

impl VoicePcmRing {
    fn new() -> Self {
        let (producer, consumer) = RingBuffer::new(VOICE_QUEUE_MAX_FRAMES * 2);
        Self {
            producer: Arc::new(Mutex::new(producer)),
            consumer: Arc::new(Mutex::new(consumer)),
        }
    }

    fn push(&self, samples: &[f32]) {
        let _ = self.producer.lock().push_partial_slice(samples);
    }

    fn clear(&self) {
        let mut consumer = self.consumer.lock();
        let mut discard = [0.0; 512];
        while consumer.slots() > 0 {
            let n = consumer.slots().min(discard.len());
            let (popped, _) = consumer.pop_partial_slice(&mut discard[..n]);
            if popped.is_empty() {
                break;
            }
        }
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
    let mut voice_wanted: Option<String> = None;
    let mut voice_unavailable: Option<String> = None;
    let mut retry_devices: Option<Vec<String>> = None;
    loop {
        let mut reopen_now = false;
        voice_separate.store(false, Ordering::Relaxed);
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
                let mut voice_stream: Option<(String, cpal::Stream, Arc<StreamHealth>)> = None;
                configure_voice_stream(
                    &new_info,
                    voice_wanted.as_deref(),
                    &voice_ring,
                    &voice_separate,
                    &mut voice_stream,
                    &mut voice_unavailable,
                    &mut retry_devices,
                );
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
                    if last_device_list.elapsed() >= devices::DefaultWatch::EVERY {
                        last_device_list = Instant::now();
                        if voice_wanted.is_some() {
                            let names = output_device_snapshot();
                            if let Some((selected, _, _)) = &voice_stream {
                                if !names.iter().any(|name| name == selected) {
                                    let selected = selected.clone();
                                    voice_separate.store(false, Ordering::Relaxed);
                                    drop(voice_stream.take());
                                    voice_ring.clear();
                                    note_voice_unavailable(&selected, &mut voice_unavailable, &mut retry_devices, names);
                                }
                            } else if voice_unavailable.is_some() && retry_devices.as_ref() != Some(&names) {
                                configure_voice_stream(
                                    &new_info,
                                    voice_wanted.as_deref(),
                                    &voice_ring,
                                    &voice_separate,
                                    &mut voice_stream,
                                    &mut voice_unavailable,
                                    &mut retry_devices,
                                );
                            }
                        }
                    }
                    if let Some((selected, _, voice_health)) = &voice_stream
                        && voice_health.failed.load(Ordering::Relaxed)
                    {
                        voice_separate.store(false, Ordering::Relaxed);
                        let selected = selected.clone();
                        drop(voice_stream.take());
                        voice_ring.clear();
                        note_voice_unavailable(&selected, &mut voice_unavailable, &mut retry_devices, output_device_snapshot());
                    }
                    match ctl.recv_timeout(Duration::from_millis(250)) {
                        Ok(OutputCmd::SetDevice(name)) => {
                            if name != wanted {
                                wanted = name;
                                voice_separate.store(false, Ordering::Relaxed);
                                drop(voice_stream.take());
                                voice_ring.clear();
                                reopen_now = true;
                                break;
                            }
                        }
                        Ok(OutputCmd::SetVoiceDevice(name)) => {
                            if name != voice_wanted {
                                voice_wanted = name;
                                configure_voice_stream(
                                    &new_info,
                                    voice_wanted.as_deref(),
                                    &voice_ring,
                                    &voice_separate,
                                    &mut voice_stream,
                                    &mut voice_unavailable,
                                    &mut retry_devices,
                                );
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if health.failed.load(Ordering::Relaxed) {
                                log::warn!("audio output lost; reopening the device");
                                voice_separate.store(false, Ordering::Relaxed);
                                drop(voice_stream.take());
                                voice_ring.clear();
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                // The callback stops before the new stream is built.
                voice_separate.store(false, Ordering::Relaxed);
                drop(voice_stream);
                voice_ring.clear();
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
            Ok(OutputCmd::SetVoiceDevice(name)) => voice_wanted = name,
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

fn note_voice_unavailable(name: &str, unavailable: &mut Option<String>, retry_devices: &mut Option<Vec<String>>, devices: Vec<String>) {
    if unavailable.as_deref() != Some(name) {
        log::warn!("voice output: {name} unavailable, using the audio output");
        *unavailable = Some(name.to_owned());
    }
    *retry_devices = Some(devices);
}

fn configure_voice_stream(
    main: &OutputInfo,
    wanted: Option<&str>,
    ring: &VoicePcmRing,
    separate: &AtomicBool,
    stream: &mut Option<(String, cpal::Stream, Arc<StreamHealth>)>,
    unavailable: &mut Option<String>,
    retry_devices: &mut Option<Vec<String>>,
) {
    match voice_stream_for(&main.device_name, wanted.unwrap_or_default()) {
        VoiceStream::Same => {
            separate.store(false, Ordering::Relaxed);
            drop(stream.take());
            ring.clear();
            *unavailable = None;
            *retry_devices = None;
        }
        VoiceStream::Separate(name) => {
            if stream
                .as_ref()
                .is_some_and(|(current, _, health)| current == &name && !health.failed.load(Ordering::Relaxed))
            {
                return;
            }
            separate.store(false, Ordering::Relaxed);
            drop(stream.take());
            ring.clear();
            match open_voice_stream(&name, ring) {
                Ok((opened, health)) => {
                    *stream = Some((name, opened, health));
                    *unavailable = None;
                    *retry_devices = None;
                    separate.store(true, Ordering::Relaxed);
                }
                Err(_error) => {
                    note_voice_unavailable(&name, unavailable, retry_devices, output_device_snapshot());
                }
            }
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
/// A small bounded rate correction keeps queue depth near its 60 ms target.
struct VoiceOutputAdapter {
    channels: usize,
    base_step: f64,
    pos: f64,
    input: Vec<f32>,
    read: Vec<f32>,
    output: Vec<f32>,
}

fn voice_rate_step(base: f64, queued_frames: usize) -> f64 {
    let error = (queued_frames as f64 - VOICE_QUEUE_TARGET_FRAMES as f64) / VOICE_QUEUE_TARGET_FRAMES as f64;
    base * (1.0 + (error * 0.02).clamp(-0.005, 0.005))
}

impl VoiceOutputAdapter {
    fn new(device_rate: u32, channels: usize) -> Self {
        Self {
            channels: channels.max(1),
            base_step: f64::from(ENGINE_SAMPLE_RATE) / f64::from(device_rate.max(1)),
            pos: 1.0,
            input: Vec::with_capacity(8192),
            read: Vec::with_capacity(8192),
            output: Vec::with_capacity(8192),
        }
    }

    fn fill<T: SizedSample + FromSample<f32>>(&mut self, ring: &VoicePcmRing, data: &mut [T]) {
        let frames = data.len() / self.channels;
        self.output.clear();
        let mut consumer = ring.consumer.lock();
        let mut queued_frames = consumer.slots() / ENGINE_CHANNELS;

        // Bound stale audio if a device stopped consuming for longer than the
        // ring's 250 ms capacity. Keep the normal jitter target after repair.
        if queued_frames >= VOICE_QUEUE_MAX_FRAMES {
            let discard_frames = queued_frames.saturating_sub(VOICE_QUEUE_TARGET_FRAMES);
            let mut discard = [0.0; 512];
            let mut left = discard_frames * ENGINE_CHANNELS;
            while left > 0 {
                let n = left.min(discard.len());
                let (popped, _) = consumer.pop_partial_slice(&mut discard[..n]);
                if popped.is_empty() {
                    break;
                }
                left -= popped.len();
            }
            queued_frames = consumer.slots() / ENGINE_CHANNELS;
        }

        let step = voice_rate_step(self.base_step, queued_frames);
        self.read.resize((frames * step.ceil() as usize + 8) * ENGINE_CHANNELS, 0.0);
        let read = {
            let (popped, _) = consumer.pop_partial_slice(&mut self.read);
            popped.len()
        };
        self.input.extend_from_slice(&self.read[..read]);

        let input_frames = self.input.len() / ENGINE_CHANNELS;
        while self.output.len() / ENGINE_CHANNELS < frames {
            let i = self.pos as usize;
            if i + 2 >= input_frames {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            for channel in 0..ENGINE_CHANNELS {
                let xm1 = self.input[(i - 1) * ENGINE_CHANNELS + channel];
                let x0 = self.input[i * ENGINE_CHANNELS + channel];
                let x1 = self.input[(i + 1) * ENGINE_CHANNELS + channel];
                let x2 = self.input[(i + 2) * ENGINE_CHANNELS + channel];
                self.output.push(hermite(xm1, x0, x1, x2, t));
            }
            self.pos += step;
        }

        let drop_frames = ((self.pos as usize).saturating_sub(1)).min(input_frames.saturating_sub(1));
        if drop_frames > 0 {
            self.input.drain(..drop_frames * ENGINE_CHANNELS);
            self.pos -= drop_frames as f64;
        }
        drop(consumer);

        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0);
        }
        for (index, frame) in data.chunks_exact_mut(self.channels).enumerate() {
            let left = self.output.get(index * 2).copied().unwrap_or(0.0);
            let right = self.output.get(index * 2 + 1).copied().unwrap_or(0.0);
            if self.channels == 1 {
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
        assert!(voice_rate_step(base, VOICE_QUEUE_TARGET_FRAMES + 100) > base);
        assert!(voice_rate_step(base, VOICE_QUEUE_TARGET_FRAMES - 100) < base);
        assert!((voice_rate_step(base, usize::MAX) / base - 1.0 - 0.005).abs() < 1e-10);
        assert!((voice_rate_step(base, 0) / base - 1.0 + 0.005).abs() < 1e-10);
    }

    #[test]
    fn separate_voice_adapter_reads_bounded_main_clock_pcm() {
        let ring = VoicePcmRing::new();
        ring.push(&vec![0.25f32; 4_000]);
        let mut adapter = VoiceOutputAdapter::new(48_000, 2);
        let mut out = vec![0.0f32; 480 * 2];
        adapter.fill(&ring, &mut out);
        assert!(out[40..].iter().all(|sample| (*sample - 0.25).abs() < 1e-3));
        assert!(ring.consumer.lock().slots() <= VOICE_QUEUE_MAX_FRAMES * 2);
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
