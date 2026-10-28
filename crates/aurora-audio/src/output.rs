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

use crate::AudioError;
use crate::devices::{self, Direction};
use crate::mixer::{ENGINE_CHANNELS, ENGINE_SAMPLE_RATE, Mixer};
use crate::processing::{EchoReference, RenderTap};
use crate::resample::Resampler;

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

/// How often the output thread reports the stream's health.
const HEALTH_REPORT: Duration = Duration::from_secs(5);

/// Requests to the output thread.
enum OutputCmd {
    /// Switch to the named device (`None` = system default).
    SetDevice(Option<String>),
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
    let handle = thread::Builder::new()
        .name("aurora-audio-out".into())
        .spawn(move || output_main(mixer, device, info, ready_tx, ctl_rx, reference))
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
) {
    let mut ready = Some(ready_tx);
    loop {
        let mut reopen_now = false;
        match open_stream(&mixer, wanted.as_deref(), &reference) {
            Ok((stream, new_info, health)) => {
                log::info!(
                    "audio output: {} ({} Hz, {} ch)",
                    new_info.device_name,
                    new_info.sample_rate,
                    new_info.channels
                );
                *info.lock() = new_info;
                if let Some(tx) = ready.take() {
                    let _ = tx.send(Ok(()));
                }
                let mut last_report = Instant::now();
                loop {
                    if last_report.elapsed() >= HEALTH_REPORT {
                        health.report(last_report.elapsed().as_secs());
                        last_report = Instant::now();
                    }
                    match ctl.recv_timeout(Duration::from_millis(250)) {
                        Ok(OutputCmd::SetDevice(name)) => {
                            if name != wanted {
                                wanted = name;
                                reopen_now = true;
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if health.failed.load(Ordering::Relaxed) {
                                log::warn!("audio output lost; reopening the device");
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                // The callback stops before the new stream is built.
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
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Opens `name` (or the default device). A named device that is missing or
/// fails to open falls back to the default one.
fn open_stream(
    mixer: &Arc<Mutex<Mixer>>,
    name: Option<&str>,
    reference: &EchoReference,
) -> Result<(cpal::Stream, OutputInfo, Arc<StreamHealth>), AudioError> {
    let picked = devices::pick(Direction::Output, name).ok_or(AudioError::NoDevice)?;
    match open_device(mixer, &picked.device, reference) {
        Ok((stream, mut info, health)) => {
            info.fallback = picked.fallback;
            Ok((stream, info, health))
        }
        Err(e) if name.is_some() && !picked.fallback => {
            log::warn!("audio output device {name:?} cannot be opened ({e}); using the default");
            let picked = devices::pick(Direction::Output, None).ok_or(AudioError::NoDevice)?;
            let (stream, mut info, health) = open_device(mixer, &picked.device, reference)?;
            info.fallback = true;
            Ok((stream, info, health))
        }
        Err(e) => Err(e),
    }
}

fn open_device(
    mixer: &Arc<Mutex<Mixer>>,
    device: &cpal::Device,
    reference: &EchoReference,
) -> Result<(cpal::Stream, OutputInfo, Arc<StreamHealth>), AudioError> {
    let supported = device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
    let mut config = supported.config();
    let queued;
    (config.buffer_size, queued) = output_buffer(config.sample_rate);
    let health = Arc::new(StreamHealth::new(queued));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(device, &config, mixer, &health, reference),
        SampleFormat::I16 => build::<i16>(device, &config, mixer, &health, reference),
        SampleFormat::U16 => build::<u16>(device, &config, mixer, &health, reference),
        SampleFormat::I32 => build::<i32>(device, &config, mixer, &health, reference),
        SampleFormat::F64 => build::<f64>(device, &config, mixer, &health, reference),
        SampleFormat::I8 => build::<i8>(device, &config, mixer, &health, reference),
        SampleFormat::U8 => build::<u8>(device, &config, mixer, &health, reference),
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
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut adapter = OutputAdapter::new(config.sample_rate, usize::from(config.channels));
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

/// Converts the mixer's 48 kHz stereo into the device's rate / layout.
pub(crate) struct OutputAdapter {
    channels: usize,
    resampler: Option<Resampler>,
    step: f64,
    mix_buf: Vec<f32>,
    out_buf: Vec<f32>,
    tap: Option<RenderTap>,
}

impl OutputAdapter {
    pub(crate) fn new(device_rate: u32, channels: usize) -> Self {
        let resampler = (device_rate != ENGINE_SAMPLE_RATE && device_rate > 0)
            .then(|| Resampler::new(ENGINE_SAMPLE_RATE, device_rate, ENGINE_CHANNELS));
        Self {
            channels: channels.max(1),
            resampler,
            step: f64::from(ENGINE_SAMPLE_RATE) / f64::from(device_rate.max(1)),
            mix_buf: Vec::with_capacity(8192),
            out_buf: Vec::with_capacity(8192),
            tap: None,
        }
    }

    pub(crate) fn fill<T: SizedSample + FromSample<f32>>(&mut self, mixer: &mut Mixer, data: &mut [T]) {
        let frames = data.len() / self.channels;
        let stereo: &[f32] = match self.resampler.as_mut() {
            None => {
                self.mix_buf.resize(frames * ENGINE_CHANNELS, 0.0);
                mixer.render(&mut self.mix_buf);
                if let Some(tap) = &mut self.tap {
                    tap.write(&self.mix_buf);
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
                        mixer.render(&mut self.mix_buf);
                        if let Some(tap) = &mut self.tap {
                            tap.write(&self.mix_buf);
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
}
