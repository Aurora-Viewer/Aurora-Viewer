//! Parcel music: internet radio streaming on a background thread.
//!
//! Behaviour mirrors the viewer's streaming audio (`LLStreamingAudioInterface`
//! in Firestorm's `indra/llaudio`): one stream at a time, a new URL replaces
//! the current one, an empty URL stops. In addition this implementation
//! reconnects with exponential backoff when the connection drops.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use symphonia::core::io::ReadOnlySource;

use crate::decode::{DecodeStream, hint_for_content_type, hint_for_path};
use crate::http;
use crate::icy::{IcyReader, TitleSlot};
use crate::mixer::{Channel, Command, ENGINE_SAMPLE_RATE, PcmSource};
use crate::resample::{Resampler, to_stereo};

/// State of the parcel music stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamStatus {
    /// No stream requested (or the Music channel is disabled).
    Idle,
    /// Connecting / buffering (also while waiting to reconnect).
    Connecting,
    /// Audio is flowing. `title` is the current ICY `StreamTitle` (or the
    /// Ogg comment title), when the station sends one.
    Playing { title: Option<String> },
    /// Last attempt failed; a reconnect is scheduled.
    Error(String),
}

/// Ring buffer between the decoder thread and the mixer (1.5 s).
const MUSIC_RING_FRAMES: usize = 72_000;
/// Jitter buffer before playback starts (400 ms).
const MUSIC_PREBUFFER_FRAMES: usize = 19_200;
const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A connection that played this long resets the backoff.
const BACKOFF_RESET_AFTER: Duration = Duration::from_secs(30);

struct StatusSlot {
    generation: u64,
    status: StreamStatus,
}

#[derive(Clone)]
pub(crate) struct StatusHandle {
    slot: Arc<Mutex<StatusSlot>>,
    generation: u64,
}

impl StatusHandle {
    fn set(&self, status: StreamStatus) {
        let mut slot = self.slot.lock();
        if slot.generation == self.generation && slot.status != status {
            slot.status = status;
        }
    }
}

/// Owns the current stream thread (if any).
pub(crate) struct StreamController {
    slot: Arc<Mutex<StatusSlot>>,
    cmd_tx: Sender<Command>,
    stop: Option<Arc<AtomicBool>>,
    generation: u64,
    url: String,
    enabled: bool,
}

impl StreamController {
    pub(crate) fn new(cmd_tx: Sender<Command>) -> Self {
        Self {
            slot: Arc::new(Mutex::new(StatusSlot {
                generation: 0,
                status: StreamStatus::Idle,
            })),
            cmd_tx,
            stop: None,
            generation: 0,
            url: String::new(),
            enabled: true,
        }
    }

    pub(crate) fn play(&mut self, url: &str) {
        let url = url.trim();
        if url == self.url && self.stop.is_some() {
            return;
        }
        self.halt();
        self.url = url.to_string();
        if !self.url.is_empty() && self.enabled {
            self.spawn();
        }
    }

    pub(crate) fn stop(&mut self) {
        self.halt();
        self.url.clear();
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        if !enabled {
            self.halt();
        } else if !self.url.is_empty() && self.stop.is_none() {
            self.spawn();
        }
    }

    pub(crate) fn status(&self) -> StreamStatus {
        self.slot.lock().status.clone()
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Stops the running thread (without forgetting the URL).
    fn halt(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.generation += 1;
        let mut slot = self.slot.lock();
        slot.generation = self.generation;
        slot.status = StreamStatus::Idle;
    }

    fn spawn(&mut self) {
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Some(stop.clone());
        let status = StatusHandle {
            slot: self.slot.clone(),
            generation: self.generation,
        };
        status.set(StreamStatus::Connecting);
        let url = self.url.clone();
        let cmd_tx = self.cmd_tx.clone();
        let spawned = thread::Builder::new()
            .name("aurora-audio-stream".into())
            .spawn(move || stream_thread(url, stop, status, cmd_tx));
        if let Err(e) = spawned {
            log::warn!("cannot start music stream thread: {e}");
            self.slot.lock().status = StreamStatus::Error(e.to_string());
            self.stop = None;
        }
    }
}

impl Drop for StreamController {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

/// Sleeps up to `d`, returning early (true) when `stop` is raised.
fn sleep_or_stop(d: Duration, stop: &AtomicBool) -> bool {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    stop.load(Ordering::Relaxed)
}

enum EndReason {
    /// Stream ended / stopped normally.
    Ended,
    /// The engine is gone.
    Shutdown,
}

fn stream_thread(url: String, stop: Arc<AtomicBool>, status: StatusHandle, cmd_tx: Sender<Command>) {
    log::info!("music stream: starting {url}");
    let mut backoff = BACKOFF_START;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        status.set(StreamStatus::Connecting);
        let started = Instant::now();
        let result = play_once(&url, &stop, &status, &cmd_tx);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match result {
            Ok(EndReason::Shutdown) => break,
            Ok(EndReason::Ended) => {
                log::info!("music stream ended, reconnecting: {url}");
                status.set(StreamStatus::Connecting);
            }
            Err(e) => {
                log::warn!("music stream error ({url}): {e}");
                status.set(StreamStatus::Error(e));
            }
        }
        if started.elapsed() > BACKOFF_RESET_AFTER {
            backoff = BACKOFF_START;
        }
        if sleep_or_stop(backoff, &stop) {
            break;
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
    log::info!("music stream: stopped {url}");
}

fn play_once(url: &str, stop: &Arc<AtomicBool>, status: &StatusHandle, cmd_tx: &Sender<Command>) -> Result<EndReason, String> {
    let resp = http::open(url, stop).map_err(|e| e.to_string())?;
    let hint = resp
        .content_type
        .as_deref()
        .and_then(hint_for_content_type)
        .or_else(|| url::Url::parse(url).ok().and_then(|u| hint_for_path(u.path())));
    let title: TitleSlot = Arc::new(Mutex::new(None));
    let reader: Box<dyn std::io::Read + Send + Sync> = match resp.metaint {
        Some(mi) => Box::new(IcyReader::new(resp.body, mi, title.clone())),
        None => resp.body,
    };
    let mut decoder = DecodeStream::open(Box::new(ReadOnlySource::new(reader)), hint).map_err(|e| e.to_string())?;

    let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(MUSIC_RING_FRAMES * 2);
    let source = PcmSource::new(Channel::Music, cons, MUSIC_PREBUFFER_FRAMES, None, Some(stop.clone()));
    if cmd_tx.send(Command::AddPcm(Box::new(source))).is_err() {
        return Ok(EndReason::Shutdown);
    }

    let mut stereo = Vec::new();
    let mut out = Vec::new();
    let mut resampler: Option<(u32, Resampler)> = None;
    let mut reported: Option<Option<String>> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(EndReason::Ended);
        }
        let rate = {
            let chunk = match decoder.next_chunk() {
                Ok(Some(c)) => c,
                Ok(None) => return Ok(EndReason::Ended),
                Err(e) => {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(EndReason::Ended);
                    }
                    return Err(e.to_string());
                }
            };
            stereo.clear();
            to_stereo(chunk.samples, chunk.channels, &mut stereo);
            chunk.rate
        };
        let pcm: &[f32] = if rate == ENGINE_SAMPLE_RATE {
            &stereo
        } else {
            if resampler.as_ref().is_none_or(|(r, _)| *r != rate) {
                resampler = Some((rate, Resampler::new(rate, ENGINE_SAMPLE_RATE, 2)));
            }
            out.clear();
            if let Some((_, rs)) = resampler.as_mut() {
                rs.process(&stereo, &mut out);
            }
            &out
        };

        let mut rest = pcm;
        while !rest.is_empty() {
            let (_, remaining) = prod.push_partial_slice(rest);
            rest = remaining;
            if !rest.is_empty() {
                if stop.load(Ordering::Relaxed) {
                    return Ok(EndReason::Ended);
                }
                if prod.is_abandoned() {
                    return Ok(EndReason::Shutdown);
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        let current = title.lock().clone().or_else(|| decoder.tag_title());
        if reported.as_ref() != Some(&current) {
            status.set(StreamStatus::Playing { title: current.clone() });
            reported = Some(current);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::tests::mp3_silence;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves one ICY response on loopback: MP3 frames with in-band metadata.
    fn serve_icy_once(metaint: usize) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut req = [0u8; 1024];
            let n = sock.read(&mut req).unwrap_or(0);
            let req = String::from_utf8_lossy(&req[..n]).to_string();
            assert!(req.contains("Icy-MetaData: 1"), "{req}");
            let head = format!("ICY 200 OK\r\nicy-name: Loopback FM\r\ncontent-type: audio/mpeg\r\nicy-metaint: {metaint}\r\n\r\n");
            let _ = sock.write_all(head.as_bytes());
            let audio = mp3_silence(200);
            let meta = b"StreamTitle='Test Artist - Test Song';";
            let mut block = meta.to_vec();
            block.resize(meta.len().div_ceil(16) * 16, 0);
            for chunk in audio.chunks(metaint) {
                if sock.write_all(chunk).is_err() {
                    return;
                }
                if chunk.len() == metaint {
                    let _ = sock.write_all(&[(block.len() / 16) as u8]);
                    let _ = sock.write_all(&block);
                }
            }
        });
        (format!("http://{addr}/stream"), handle)
    }

    #[test]
    fn loopback_icy_stream_plays_and_reports_title() {
        let (url, server) = serve_icy_once(4096);
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let mut ctl = StreamController::new(cmd_tx);
        ctl.play(&url);
        assert_eq!(ctl.url(), url);

        // Wait for decoded audio and the ICY title. The ring buffer is large
        // enough to hold the first seconds without a consumer.
        let mut sources = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                if let Command::AddPcm(s) = cmd {
                    sources.push(s);
                }
            }
            if let StreamStatus::Playing { title: Some(t) } = ctl.status() {
                assert_eq!(t, "Test Artist - Test Song");
                break;
            }
            assert!(Instant::now() < deadline, "status: {:?}", ctl.status());
            thread::sleep(Duration::from_millis(20));
        }
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let Command::AddPcm(s) = cmd {
                sources.push(s);
            }
        }
        assert_eq!(sources.len(), 1);
        assert!(sources[0].queued_samples() > 0);
        ctl.stop();
        assert_eq!(ctl.status(), StreamStatus::Idle);
        assert_eq!(ctl.url(), "");
        drop(sources);
        let _ = server.join();
    }

    #[test]
    fn disabled_music_does_not_connect() {
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let mut ctl = StreamController::new(cmd_tx);
        ctl.set_enabled(false);
        // Unroutable on purpose: nothing must be attempted while disabled.
        ctl.play("http://192.0.2.1/stream");
        assert_eq!(ctl.status(), StreamStatus::Idle);
        assert!(ctl.stop.is_none());
        ctl.play("");
        assert_eq!(ctl.status(), StreamStatus::Idle);
    }
}
