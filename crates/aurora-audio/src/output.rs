//! Device output (cpal; WASAPI on Windows) on a dedicated thread.
//!
//! The thread owns the cpal stream, rebuilds it when the device disappears,
//! the stream is invalidated or another device is selected, and exits when
//! the engine is dropped. The mixer is shared with the stream callback and
//! survives device changes (channels, sinks and playing sounds are kept).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use parking_lot::Mutex;

use crate::AudioError;
use crate::devices::{self, Direction};
use crate::mixer::{ENGINE_CHANNELS, ENGINE_SAMPLE_RATE, Mixer};
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
pub(crate) fn spawn(mixer: Arc<Mutex<Mixer>>, device: Option<String>, info: Arc<Mutex<OutputInfo>>) -> Result<OutputThread, AudioError> {
    let (ready_tx, ready_rx) = bounded::<Result<(), AudioError>>(1);
    let (ctl_tx, ctl_rx) = unbounded::<OutputCmd>();
    let handle = thread::Builder::new()
        .name("aurora-audio-out".into())
        .spawn(move || output_main(mixer, device, info, ready_tx, ctl_rx))
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
) {
    let mut ready = Some(ready_tx);
    loop {
        let mut reopen_now = false;
        match open_stream(&mixer, wanted.as_deref()) {
            Ok((stream, new_info, failed)) => {
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
                loop {
                    match ctl.recv_timeout(Duration::from_millis(250)) {
                        Ok(OutputCmd::SetDevice(name)) => {
                            if name != wanted {
                                wanted = name;
                                reopen_now = true;
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if failed.load(Ordering::Relaxed) {
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
fn open_stream(mixer: &Arc<Mutex<Mixer>>, name: Option<&str>) -> Result<(cpal::Stream, OutputInfo, Arc<AtomicBool>), AudioError> {
    let picked = devices::pick(Direction::Output, name).ok_or(AudioError::NoDevice)?;
    match open_device(mixer, &picked.device) {
        Ok((stream, mut info, failed)) => {
            info.fallback = picked.fallback;
            Ok((stream, info, failed))
        }
        Err(e) if name.is_some() && !picked.fallback => {
            log::warn!("audio output device {name:?} cannot be opened ({e}); using the default");
            let picked = devices::pick(Direction::Output, None).ok_or(AudioError::NoDevice)?;
            let (stream, mut info, failed) = open_device(mixer, &picked.device)?;
            info.fallback = true;
            Ok((stream, info, failed))
        }
        Err(e) => Err(e),
    }
}

fn open_device(mixer: &Arc<Mutex<Mixer>>, device: &cpal::Device) -> Result<(cpal::Stream, OutputInfo, Arc<AtomicBool>), AudioError> {
    let supported = device.default_output_config().map_err(|e| AudioError::Device(e.to_string()))?;
    let config = supported.config();
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(device, &config, mixer, &failed),
        SampleFormat::I16 => build::<i16>(device, &config, mixer, &failed),
        SampleFormat::U16 => build::<u16>(device, &config, mixer, &failed),
        SampleFormat::I32 => build::<i32>(device, &config, mixer, &failed),
        SampleFormat::F64 => build::<f64>(device, &config, mixer, &failed),
        SampleFormat::I8 => build::<i8>(device, &config, mixer, &failed),
        SampleFormat::U8 => build::<u8>(device, &config, mixer, &failed),
        other => Err(AudioError::Device(format!("unsupported device sample format {other:?}"))),
    }?;
    stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
    let info = OutputInfo {
        device_name: devices::device_name(device),
        sample_rate: config.sample_rate,
        channels: config.channels,
        fallback: false,
    };
    Ok((stream, info, failed))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: &Arc<Mutex<Mixer>>,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut adapter = OutputAdapter::new(config.sample_rate, usize::from(config.channels));
    let mixer = mixer.clone();
    let failed = failed.clone();
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], _info| {
                let mut m = mixer.lock();
                adapter.fill(&mut m, data);
            },
            move |err| match err.kind() {
                cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::HostUnavailable => {
                    failed.store(true, Ordering::Relaxed);
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
        }
    }

    pub(crate) fn fill<T: SizedSample + FromSample<f32>>(&mut self, mixer: &mut Mixer, data: &mut [T]) {
        let frames = data.len() / self.channels;
        let stereo: &[f32] = match self.resampler.as_mut() {
            None => {
                self.mix_buf.resize(frames * ENGINE_CHANNELS, 0.0);
                mixer.render(&mut self.mix_buf);
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
