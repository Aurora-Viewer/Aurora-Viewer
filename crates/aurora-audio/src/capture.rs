//! Microphone capture: a cpal input stream converted to 48 kHz mono f32,
//! with a software input gain, a level meter and a lock-free ring read by
//! the voice client.
//!
//! Nothing is opened until [`MicCapture::open`] is called: no thread, no
//! stream and no OS microphone indicator otherwise.
//!
//! The level meter follows Firestorm's WebRTC device module and tuning
//! meter (`LLWebRTCAudioTransport::RecordedDataIsAvailable` in
//! `indra/llwebrtc/llwebrtc.cpp`, `LLWebRTCVoiceClient::tuningGetEnergy` in
//! `indra/newview/llvoicewebrtc.cpp`): RMS over 300 ms of 10 ms blocks,
//! mapped as `0.8 - 0.01 * (-20 log10 rms)`.
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//! Licensed under the GNU Lesser General Public License, version 2.1.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use parking_lot::Mutex;

use crate::AudioError;
use crate::devices::{self, Direction};
use crate::mixer::ENGINE_SAMPLE_RATE;
use crate::resample::Resampler;

/// Sample rate of the captured audio (mono).
pub const CAPTURE_SAMPLE_RATE: u32 = ENGINE_SAMPLE_RATE;

/// `LLVoiceClient::OVERDRIVEN_POWER_LEVEL`: a VU meter is full at this
/// [`MicCapture::level`] (Firestorm draws `level / OVERDRIVEN_POWER_LEVEL`).
pub const OVERDRIVEN_POWER_LEVEL: f32 = 0.7;

/// Largest accepted software input gain (Firestorm's slider goes to 2).
pub const MAX_INPUT_GAIN: f32 = 4.0;

/// One second of captured audio is buffered for the reader.
const RING_SAMPLES: usize = CAPTURE_SAMPLE_RATE as usize;
/// Meter block: 10 ms (one WebRTC audio frame).
const METER_BLOCK: usize = 480;
/// `NUM_PACKETS_TO_FILTER`: 30 blocks = 300 ms of smoothing.
const METER_BLOCKS: usize = 30;
/// `TUNING_LEVEL_START_POINT` / `TUNING_LEVEL_SCALE`.
const TUNING_LEVEL_START_POINT: f32 = 0.8;
const TUNING_LEVEL_SCALE: f32 = 0.01;
/// Gain changes are ramped over 10 ms (no clicks).
const GAIN_RAMP_SAMPLES: f32 = 480.0;

/// State shared by the handle, the device thread and the writer.
struct MicShared {
    /// Requested gain (f32 bits).
    gain: AtomicU32,
    /// Smoothed RMS of the gained signal, 0..1 (f32 bits).
    rms: AtomicU32,
    /// Samples written so far (stall detection).
    captured: AtomicU64,
    /// Name of the device being captured ("" = none).
    device: Mutex<String>,
    fallback: AtomicBool,
}

impl MicShared {
    fn new() -> Self {
        Self {
            gain: AtomicU32::new(1.0f32.to_bits()),
            rms: AtomicU32::new(0),
            captured: AtomicU64::new(0),
            device: Mutex::new(String::new()),
            fallback: AtomicBool::new(false),
        }
    }

    fn set_rms(&self, v: f32) {
        self.rms.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// Producer side: gain, metering and the ring. Used by the device callback
/// (or by a [`MicFeed`]); never allocates.
struct MicWriter {
    prod: rtrb::Producer<f32>,
    shared: Arc<MicShared>,
    gain: f32,
    block_energy: f32,
    block_fill: usize,
    history: [f32; METER_BLOCKS],
    history_pos: usize,
}

impl MicWriter {
    fn new(prod: rtrb::Producer<f32>, shared: Arc<MicShared>) -> Self {
        Self {
            prod,
            shared,
            gain: 1.0,
            block_energy: 0.0,
            block_fill: 0,
            history: [0.0; METER_BLOCKS],
            history_pos: 0,
        }
    }

    /// Applies the gain, meters and queues 48 kHz mono samples. Returns how
    /// many were queued (the rest is dropped when the reader lags).
    fn write(&mut self, samples: &[f32]) -> usize {
        let target = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));
        let step = 1.0 / GAIN_RAMP_SAMPLES * MAX_INPUT_GAIN;
        let mut buf = [0.0f32; 256];
        let mut queued = 0;
        for chunk in samples.chunks(buf.len()) {
            for (o, &s) in buf.iter_mut().zip(chunk) {
                if self.gain != target {
                    self.gain += (target - self.gain).clamp(-step, step);
                }
                let v = s * self.gain;
                let v = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
                *o = v;
                self.meter(v);
            }
            let (done, _) = self.prod.push_partial_slice(&buf[..chunk.len()]);
            queued += done.len();
        }
        self.shared.captured.fetch_add(samples.len() as u64, Ordering::Relaxed);
        queued
    }

    fn meter(&mut self, v: f32) {
        self.block_energy += v * v;
        self.block_fill += 1;
        if self.block_fill < METER_BLOCK {
            return;
        }
        self.history[self.history_pos] = self.block_energy;
        self.history_pos = (self.history_pos + 1) % METER_BLOCKS;
        self.block_energy = 0.0;
        self.block_fill = 0;
        let total: f32 = self.history.iter().sum();
        self.shared.set_rms((total / (METER_BLOCK * METER_BLOCKS) as f32).sqrt());
    }

    fn reset_meter(&mut self) {
        self.history = [0.0; METER_BLOCKS];
        self.block_energy = 0.0;
        self.block_fill = 0;
        self.shared.set_rms(0.0);
    }
}

/// Converts device buffers (any format, rate and channel count) into 48 kHz
/// mono for a [`MicWriter`].
pub(crate) struct InputAdapter {
    channels: usize,
    resampler: Option<Resampler>,
    mono: Vec<f32>,
    out: Vec<f32>,
}

impl InputAdapter {
    pub(crate) fn new(device_rate: u32, channels: usize) -> Self {
        let resampler =
            (device_rate != CAPTURE_SAMPLE_RATE && device_rate > 0).then(|| Resampler::new(device_rate, CAPTURE_SAMPLE_RATE, 1));
        Self {
            channels: channels.max(1),
            resampler,
            mono: Vec::with_capacity(8192),
            out: Vec::with_capacity(8192),
        }
    }

    /// Downmixes (average of all channels) and resamples `data`, appending
    /// the result to `self.out`'s cleared buffer; returns it.
    pub(crate) fn convert<T>(&mut self, data: &[T]) -> &[f32]
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        self.mono.clear();
        let scale = 1.0 / self.channels as f32;
        for frame in data.chunks_exact(self.channels) {
            let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
            self.mono.push(sum * scale);
        }
        match self.resampler.as_mut() {
            None => &self.mono,
            Some(rs) => {
                self.out.clear();
                rs.process(&self.mono, &mut self.out);
                &self.out
            }
        }
    }
}

/// Requests to the capture thread.
enum MicCmd {
    SetDevice(Option<String>),
}

struct MicInner {
    shared: Arc<MicShared>,
    cons: Mutex<rtrb::Consumer<f32>>,
    /// `None` for a virtual input.
    ctl: Mutex<Option<Sender<MicCmd>>>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Drop for MicInner {
    fn drop(&mut self) {
        drop(self.ctl.lock().take());
        if let Some(h) = self.thread.lock().take() {
            let _ = h.join();
        }
    }
}

/// A microphone: 48 kHz mono f32 samples with a software input gain and a
/// level meter.
///
/// Cheap to clone (all clones share the device); the device is closed when
/// the last clone is dropped. Samples are read with [`MicCapture::read`]
/// (one reader at a time: the voice client). Up to one second is buffered;
/// newer samples are dropped while the buffer is full, so a reader should
/// drain it regularly and use [`MicCapture::skip`] to cut latency.
#[derive(Clone)]
pub struct MicCapture {
    inner: Arc<MicInner>,
}

impl std::fmt::Debug for MicCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicCapture")
            .field("device", &self.device_name())
            .field("gain", &self.gain())
            .field("level", &self.level())
            .finish()
    }
}

impl MicCapture {
    fn with_ring() -> (MicWriter, Self) {
        let shared = Arc::new(MicShared::new());
        let (prod, cons) = rtrb::RingBuffer::<f32>::new(RING_SAMPLES);
        let writer = MicWriter::new(prod, shared.clone());
        let mic = Self {
            inner: Arc::new(MicInner {
                shared,
                cons: Mutex::new(cons),
                ctl: Mutex::new(None),
                thread: Mutex::new(None),
            }),
        };
        (writer, mic)
    }

    /// Opens the input device called `device` (`None` = system default; a
    /// missing device falls back to the default, which is logged) and starts
    /// capturing on a dedicated thread.
    ///
    /// Fails with [`AudioError::NoDevice`] when there is no input device at
    /// all. Once open, a lost device is reopened in the background.
    pub fn open(device: Option<String>) -> Result<Self, AudioError> {
        let (writer, mic) = Self::with_ring();
        let writer = Arc::new(Mutex::new(writer));
        let (ready_tx, ready_rx) = bounded::<Result<(), AudioError>>(1);
        let (ctl_tx, ctl_rx) = unbounded::<MicCmd>();
        let shared = mic.inner.shared.clone();
        let handle = thread::Builder::new()
            .name("aurora-audio-in".into())
            .spawn(move || capture_main(writer, shared, device, ready_tx, ctl_rx))
            .map_err(|e| AudioError::Device(e.to_string()))?;
        *mic.inner.ctl.lock() = Some(ctl_tx);
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {
                *mic.inner.thread.lock() = Some(handle);
                Ok(mic)
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            // Do not block on a wedged driver: the thread is detached.
            Err(_) => Err(AudioError::Device("audio input did not start".into())),
        }
    }

    /// A microphone without a device, fed by the returned [`MicFeed`] (tests,
    /// or injecting a sound file into voice). Gain and metering apply.
    pub fn virtual_input() -> (MicFeed, Self) {
        let (writer, mic) = Self::with_ring();
        *mic.inner.shared.device.lock() = "virtual".to_string();
        (MicFeed { writer }, mic)
    }

    /// Switches to another input device (`None` = system default), keeping
    /// the gain. No effect on a virtual input.
    pub fn set_device(&self, name: Option<String>) {
        if let Some(tx) = self.inner.ctl.lock().as_ref() {
            let _ = tx.send(MicCmd::SetDevice(name));
        }
    }

    /// Name of the device being captured; `None` while no device is open
    /// (unplugged, reopening).
    pub fn device_name(&self) -> Option<String> {
        let name = self.inner.shared.device.lock();
        (!name.is_empty()).then(|| name.clone())
    }

    /// `true` when a named device was requested but the default is used.
    pub fn is_fallback(&self) -> bool {
        self.inner.shared.fallback.load(Ordering::Relaxed)
    }

    /// Sets the software input gain (linear, 0..=[`MAX_INPUT_GAIN`]; 1 =
    /// unchanged), ramped over 10 ms. It applies to the captured samples and
    /// to [`MicCapture::level`].
    pub fn set_gain(&self, gain: f32) {
        let gain = if gain.is_finite() { gain.clamp(0.0, MAX_INPUT_GAIN) } else { 1.0 };
        self.inner.shared.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// The software input gain.
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.inner.shared.gain.load(Ordering::Relaxed))
    }

    /// Input level for a VU meter, 0..1, smoothed over 300 ms (Firestorm's
    /// tuning energy: 0.8 at full scale, 0.4 at -40 dBFS, 0 at -80 dBFS).
    /// Divide by [`OVERDRIVEN_POWER_LEVEL`] to fill a bar as Firestorm does.
    pub fn level(&self) -> f32 {
        let rms = f32::from_bits(self.inner.shared.rms.load(Ordering::Relaxed));
        tuning_level(rms)
    }

    /// Samples waiting to be read.
    pub fn available(&self) -> usize {
        self.inner.cons.lock().slots()
    }

    /// Reads up to `out.len()` samples (48 kHz mono); returns how many.
    pub fn read(&self, out: &mut [f32]) -> usize {
        let (got, _) = self.inner.cons.lock().pop_partial_slice(out);
        got.len()
    }

    /// Discards up to `n` of the oldest samples; returns how many.
    pub fn skip(&self, n: usize) -> usize {
        let mut cons = self.inner.cons.lock();
        let n = n.min(cons.slots());
        match cons.read_chunk(n) {
            Ok(chunk) => {
                chunk.commit_all();
                n
            }
            Err(_) => 0,
        }
    }

    /// Total samples captured since opening (including dropped ones); a
    /// value that stops growing means the device delivers nothing.
    pub fn samples_captured(&self) -> u64 {
        self.inner.shared.captured.load(Ordering::Relaxed)
    }
}

/// `TUNING_LEVEL_START_POINT - TUNING_LEVEL_SCALE * -20 log10(rms)`, clamped.
fn tuning_level(rms: f32) -> f32 {
    if rms <= 0.0 || !rms.is_finite() {
        return 0.0;
    }
    (TUNING_LEVEL_START_POINT + TUNING_LEVEL_SCALE * 20.0 * rms.log10()).clamp(0.0, 1.0)
}

/// Producer end of [`MicCapture::virtual_input`].
pub struct MicFeed {
    writer: MicWriter,
}

impl std::fmt::Debug for MicFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicFeed").finish_non_exhaustive()
    }
}

impl MicFeed {
    /// Queues 48 kHz mono samples (as if captured). Returns how many were
    /// queued; the rest is dropped when the buffer is full.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        self.writer.write(samples)
    }

    /// `true` while a [`MicCapture`] clone is alive.
    pub fn is_connected(&self) -> bool {
        !self.writer.prod.is_abandoned()
    }
}

fn capture_main(
    writer: Arc<Mutex<MicWriter>>,
    shared: Arc<MicShared>,
    mut wanted: Option<String>,
    ready_tx: Sender<Result<(), AudioError>>,
    ctl: Receiver<MicCmd>,
) {
    let mut ready = Some(ready_tx);
    let mut warned = false;
    loop {
        let mut reopen_now = false;
        match open_input(&writer, wanted.as_deref()) {
            Ok((stream, name, fallback, failed)) => {
                log::info!("audio input: {name}");
                warned = false;
                *shared.device.lock() = name;
                shared.fallback.store(fallback, Ordering::Relaxed);
                if let Some(tx) = ready.take() {
                    let _ = tx.send(Ok(()));
                }
                let mut last = shared.captured.load(Ordering::Relaxed);
                loop {
                    match ctl.recv_timeout(Duration::from_millis(250)) {
                        Ok(MicCmd::SetDevice(name)) => {
                            if name != wanted {
                                wanted = name;
                                reopen_now = true;
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if failed.load(Ordering::Relaxed) {
                                log::warn!("audio input lost; reopening the device");
                                break;
                            }
                            // No data for 250 ms: the meter must not freeze.
                            let now = shared.captured.load(Ordering::Relaxed);
                            if now == last {
                                shared.set_rms(0.0);
                            }
                            last = now;
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                drop(stream);
                shared.device.lock().clear();
                writer.lock().reset_meter();
            }
            Err(e) => {
                if let Some(tx) = ready.take() {
                    let _ = tx.send(Err(e));
                    return;
                }
                if warned {
                    log::debug!("audio input reopen failed: {e}");
                } else {
                    log::warn!("audio input reopen failed: {e}");
                    warned = true;
                }
            }
        }
        if reopen_now {
            continue;
        }
        match ctl.recv_timeout(Duration::from_secs(1)) {
            Ok(MicCmd::SetDevice(name)) => wanted = name,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

type Opened = (cpal::Stream, String, bool, Arc<AtomicBool>);

/// Opens `name` (or the default input). A named device that is missing or
/// fails to open falls back to the default one.
fn open_input(writer: &Arc<Mutex<MicWriter>>, name: Option<&str>) -> Result<Opened, AudioError> {
    let picked = devices::pick(Direction::Input, name).ok_or(AudioError::NoDevice)?;
    match open_input_device(writer, &picked.device) {
        Ok((stream, failed)) => Ok((stream, devices::device_name(&picked.device), picked.fallback, failed)),
        Err(e) if name.is_some() && !picked.fallback => {
            log::warn!("audio input device {name:?} cannot be opened ({e}); using the default");
            let picked = devices::pick(Direction::Input, None).ok_or(AudioError::NoDevice)?;
            let (stream, failed) = open_input_device(writer, &picked.device)?;
            Ok((stream, devices::device_name(&picked.device), true, failed))
        }
        Err(e) => Err(e),
    }
}

fn open_input_device(writer: &Arc<Mutex<MicWriter>>, device: &cpal::Device) -> Result<(cpal::Stream, Arc<AtomicBool>), AudioError> {
    let supported = device.default_input_config().map_err(|e| AudioError::Device(e.to_string()))?;
    let config = supported.config();
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_input::<f32>(device, &config, writer, &failed),
        SampleFormat::I16 => build_input::<i16>(device, &config, writer, &failed),
        SampleFormat::U16 => build_input::<u16>(device, &config, writer, &failed),
        SampleFormat::I32 => build_input::<i32>(device, &config, writer, &failed),
        SampleFormat::F64 => build_input::<f64>(device, &config, writer, &failed),
        SampleFormat::I8 => build_input::<i8>(device, &config, writer, &failed),
        SampleFormat::U8 => build_input::<u8>(device, &config, writer, &failed),
        other => Err(AudioError::Device(format!("unsupported input sample format {other:?}"))),
    }?;
    stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
    Ok((stream, failed))
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    writer: &Arc<Mutex<MicWriter>>,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut adapter = InputAdapter::new(config.sample_rate, usize::from(config.channels));
    let writer = writer.clone();
    let failed = failed.clone();
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _info| {
                // Only one stream exists at a time: never contended.
                if let Some(mut w) = writer.try_lock() {
                    let mono = adapter.convert(data);
                    w.write(mono);
                }
            },
            move |err| match err.kind() {
                cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::HostUnavailable => {
                    failed.store(true, Ordering::Relaxed);
                }
                _ => log::debug!("audio input stream: {err}"),
            },
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn virtual_input_delivers_gained_samples() {
        let (mut feed, mic) = MicCapture::virtual_input();
        assert!(feed.is_connected());
        assert_eq!(mic.device_name().as_deref(), Some("virtual"));
        assert_eq!(feed.push(&[0.25; 100]), 100);
        assert_eq!(mic.available(), 100);
        let mut out = [0.0; 100];
        assert_eq!(mic.read(&mut out), 100);
        assert!(out.iter().all(|&v| (v - 0.25).abs() < 1e-6));

        // Gain is ramped, then applied and clipped.
        mic.set_gain(8.0);
        assert_eq!(mic.gain(), MAX_INPUT_GAIN);
        feed.push(&[0.1; 2000]);
        let mut out = vec![0.0; 2000];
        mic.read(&mut out);
        assert!(out[0] < 0.2, "ramped: {}", out[0]);
        assert!((out[1999] - 0.4).abs() < 1e-6);
        mic.set_gain(f32::NAN);
        assert_eq!(mic.gain(), 1.0);
        mic.set_gain(10.0);
        feed.push(&[0.5; 1000]);
        let mut out = vec![0.0; 1000];
        mic.read(&mut out);
        assert_eq!(out[999], 1.0);
        drop(mic);
        assert!(!feed.is_connected());
    }

    #[test]
    fn level_follows_firestorm_tuning_scale() {
        assert_eq!(tuning_level(0.0), 0.0);
        assert!((tuning_level(1.0) - 0.8).abs() < 1e-6);
        assert!((tuning_level(0.01) - 0.4).abs() < 1e-6);
        assert_eq!(tuning_level(1e-5), 0.0);

        let (mut feed, mic) = MicCapture::virtual_input();
        assert_eq!(mic.level(), 0.0);
        // 300 ms of a 0.5 sine: rms 0.354 -> 0.8 + 0.2 log10(0.354) = 0.71.
        feed.push(&sine(14_400, 0.5));
        assert!((mic.level() - 0.71).abs() < 0.01, "{}", mic.level());
        // Silence brings it back down once the window has passed.
        feed.push(&[0.0; 14_400]);
        assert_eq!(mic.level(), 0.0);
    }

    #[test]
    fn ring_overflow_and_skip() {
        let (mut feed, mic) = MicCapture::virtual_input();
        assert_eq!(feed.push(&vec![0.1; RING_SAMPLES + 500]), RING_SAMPLES);
        assert_eq!(mic.samples_captured(), (RING_SAMPLES + 500) as u64);
        assert_eq!(mic.skip(RING_SAMPLES - 10), RING_SAMPLES - 10);
        assert_eq!(mic.available(), 10);
        assert_eq!(mic.skip(100), 10);
        assert_eq!(mic.available(), 0);
        // Clones share the ring.
        let other = mic.clone();
        feed.push(&[0.3; 4]);
        assert_eq!(other.available(), 4);
        mic.set_device(Some("ignored".into()));
    }

    #[test]
    fn adapter_downmixes_and_resamples() {
        // 44.1 kHz stereo i16, left = right = 0.5 full scale.
        let mut a = InputAdapter::new(44_100, 2);
        let half = i16::from_sample(0.5f32);
        let data = vec![half; 4410 * 2];
        let mut total = 0;
        for _ in 0..10 {
            let out = a.convert(&data);
            total += out.len();
            assert!(out.iter().skip(4).all(|&v| (v - 0.5).abs() < 1e-3));
        }
        assert!((47_990..=48_000).contains(&total), "{total}");

        // 48 kHz 4 channels f32: averaged, no resampling.
        let mut a = InputAdapter::new(48_000, 4);
        assert_eq!(a.convert(&[1.0f32, 0.0, 0.0, 1.0, 0.2, 0.2, 0.2, 0.2]), [0.5, 0.2]);
    }
}
