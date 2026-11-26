//! Bounded, cancellable playback of a local video soundtrack.

use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;

use crate::{
    AudioError, Channel, ENGINE_SAMPLE_RATE,
    decode::DecodeStream,
    mixer::{Command, PcmSource},
    resample::{Resampler, to_stereo},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocalMediaStatus {
    Opening,
    Playing,
    Unavailable,
}

/// Owns the playback of one video pass. Drop it when playback stops or loops.
pub struct LocalMediaAudio {
    cancelled: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
}

impl LocalMediaAudio {
    pub(crate) fn start(
        path: PathBuf,
        started: Instant,
        offset: f64,
        rate: f64,
        mute: Arc<AtomicBool>,
        tx: Sender<Command>,
    ) -> Option<Self> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let status = Arc::new(AtomicU8::new(0));
        let progress = status.clone();
        thread::Builder::new()
            .name("local-media-audio".into())
            .spawn(move || {
                // Video platforms can return HE-AAC, Opus or audio in
                // containers that the built-in decoder does not support.
                // Read bounded PCM from FFmpeg when available. Keep the
                // built-in path for installs without the optional tool.
                let mut output = AudioOutput {
                    stop,
                    mute,
                    tx,
                    progress,
                    producer: None,
                };
                let played = play_ffmpeg(&path, started, offset, rate, &mut output).unwrap_or(false);
                if !played && !output.stopped() {
                    if let Err(error) = play_native(&path, started, offset, rate, &mut output) {
                        log::debug!("local video audio unavailable: {error}");
                    }
                    if output.producer.is_none() {
                        output.progress.store(2, Ordering::Relaxed);
                    }
                }
            })
            .ok()?;
        Some(Self { cancelled, status })
    }

    pub fn status(&self) -> LocalMediaStatus {
        match self.status.load(Ordering::Relaxed) {
            0 => LocalMediaStatus::Opening,
            1 => LocalMediaStatus::Playing,
            _ => LocalMediaStatus::Unavailable,
        }
    }
}

impl Drop for LocalMediaAudio {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

struct AudioOutput {
    stop: Arc<AtomicBool>,
    mute: Arc<AtomicBool>,
    tx: Sender<Command>,
    progress: Arc<AtomicU8>,
    producer: Option<rtrb::Producer<f32>>,
}

impl AudioOutput {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || self.producer.as_ref().is_some_and(|producer| producer.is_abandoned())
    }

    fn write(&mut self, mut pcm: &[f32]) -> bool {
        if pcm.is_empty() {
            return !self.stopped();
        }
        if self.stopped() {
            return false;
        }
        if self.producer.is_none() {
            let (producer, consumer) = rtrb::RingBuffer::new(4_800 * 2);
            let source = PcmSource::new(Channel::Media, consumer, 480, None, Some(self.stop.clone())).with_mute(self.mute.clone());
            if self.tx.try_send(Command::AddPcm(Box::new(source))).is_err() {
                return false;
            }
            self.producer = Some(producer);
        }
        let producer = self.producer.as_mut().expect("initialized audio producer");
        while !pcm.is_empty() {
            if self.stop.load(Ordering::Relaxed) || producer.is_abandoned() {
                return false;
            }
            let (_, remaining) = producer.push_partial_slice(pcm);
            pcm = remaining;
            self.progress.store(1, Ordering::Relaxed);
            if !pcm.is_empty() {
                thread::sleep(Duration::from_millis(5));
            }
        }
        true
    }
}

struct DecoderProcess(Child);

impl Drop for DecoderProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn ffmpeg_audio(path: &Path, offset: f64, rate: f64) -> Process {
    let mut command = Process::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe,crypto,data",
            "-threads",
            "1",
            "-ss",
            &format!("{offset:.6}"),
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-af",
            &format!(
                "aresample={ENGINE_SAMPLE_RATE},asetrate={}",
                (f64::from(ENGINE_SAMPLE_RATE) * rate).round() as u32
            ),
            "-ac",
            "2",
            "-ar",
            &ENGINE_SAMPLE_RATE.to_string(),
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn play_ffmpeg(path: &Path, started: Instant, offset: f64, rate: f64, output: &mut AudioOutput) -> io::Result<bool> {
    let launch_delay = started.elapsed().as_secs_f64();
    let mut process = DecoderProcess(ffmpeg_audio(path, offset + launch_delay * rate, rate).spawn()?);
    let mut reader = process.0.stdout.take().expect("piped audio");
    let mut bytes = [0; 4096];
    let mut decoded = Vec::with_capacity(bytes.len() / 4);
    let mut skip = None;
    while !output.stopped() {
        let mut count = 0;
        while count < bytes.len() && !output.stopped() {
            match reader.read(&mut bytes[count..]) {
                Ok(0) => break,
                Ok(read) => count += read,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if count < 8 {
            break;
        }
        let remaining = skip.get_or_insert_with(|| {
            let elapsed = (started.elapsed().as_secs_f64() - launch_delay - 0.030).max(0.0);
            (elapsed * f64::from(ENGINE_SAMPLE_RATE)) as usize
        });
        let skip_frames = (*remaining).min(count / 8);
        *remaining -= skip_frames;
        decoded.clear();
        decoded.extend(bytes[skip_frames * 8..count - count % 8].chunks_exact(4).map(|sample| {
            let value = f32::from_le_bytes(sample.try_into().expect("PCM sample"));
            if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 }
        }));
        if !output.write(&decoded) {
            break;
        }
    }
    Ok(output.producer.is_some())
}

fn play_native(path: &Path, started: Instant, offset: f64, rate: f64, output: &mut AudioOutput) -> Result<(), AudioError> {
    let file = File::open(path).map_err(|error| AudioError::Decode(error.to_string()))?;
    let mut decoder = DecodeStream::open(Box::new(file), path.extension().and_then(|ext| ext.to_str()))?;
    // A failed demuxer seek can invalidate the stream. Decode from the start
    // on a fresh reader in that case.
    let mut skip_source = if offset >= 0.1 {
        if let Some(remaining) = decoder.seek_before(offset) {
            remaining
        } else {
            let file = File::open(path).map_err(|error| AudioError::Decode(error.to_string()))?;
            decoder = DecodeStream::open(Box::new(file), path.extension().and_then(|ext| ext.to_str()))?;
            offset
        }
    } else {
        offset
    };
    if output.stopped() {
        return Ok(());
    }

    // Opening a file should not delay the pictures. If probing was slow,
    // start with the samples that correspond to the current video position.
    let skip = started.elapsed().saturating_sub(Duration::from_millis(30));
    skip_source += skip.as_secs_f64() * rate;
    let mut skip_frames = (skip_source / rate * f64::from(ENGINE_SAMPLE_RATE)) as usize;
    let mut stereo = Vec::new();
    let mut converted = Vec::new();
    let mut resampler = None;
    while !output.stopped() {
        let Some(chunk) = decoder.next_chunk()? else { break };
        stereo.clear();
        to_stereo(chunk.samples, chunk.channels, &mut stereo);
        let input_rate = (f64::from(chunk.rate) * rate).round() as u32;
        let pcm = if input_rate == ENGINE_SAMPLE_RATE {
            resampler = None;
            &stereo
        } else {
            let (rate, converter) = resampler.get_or_insert_with(|| (input_rate, Resampler::new(input_rate, ENGINE_SAMPLE_RATE, 2)));
            if *rate != input_rate {
                *rate = input_rate;
                *converter = Resampler::new(input_rate, ENGINE_SAMPLE_RATE, 2);
            }
            converted.clear();
            converter.process(&stereo, &mut converted);
            &converted
        };
        let skip = skip_frames.min(pcm.len() / 2);
        skip_frames -= skip;
        if !output.write(&pcm[skip * 2..]) {
            break;
        }
    }
    Ok(())
}
