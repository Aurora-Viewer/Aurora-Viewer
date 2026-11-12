//! Parcel music: internet radio streaming on a background thread.
//!
//! Behaviour mirrors the viewer's streaming audio (`LLStreamingAudioInterface`
//! in the Legacy Viewer's `indra/llaudio`): one stream at a time, a new URL replaces
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
use crate::playlist;
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
    Error(StreamError),
}

/// Stable error categories for translated UI text; server text stays in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error("web page with no radio stream")]
    WebPage,
    #[error("radio playlist the decoder cannot follow")]
    Playlist,
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    #[error("secure connection failed")]
    Tls,
    #[error("connection failed")]
    Connection,
    #[error("invalid or unsupported stream address")]
    Address,
    #[error("unsupported or invalid audio format")]
    AudioFormat,
    #[error("audio stream interrupted")]
    Interrupted,
    #[error("audio worker could not start")]
    Worker,
}

impl From<http::HttpError> for StreamError {
    fn from(error: http::HttpError) -> Self {
        log::warn!("music HTTP failure: {error}");
        match error {
            http::HttpError::Status(code) => Self::HttpStatus(code),
            http::HttpError::Tls(_) => Self::Tls,
            http::HttpError::Io(ref error) if error.get_ref().is_some_and(|cause| cause.is::<rustls::Error>()) => Self::Tls,
            http::HttpError::Url(_) | http::HttpError::Scheme(_) | http::HttpError::PlaylistOrigin => Self::Address,
            _ => Self::Connection,
        }
    }
}

impl From<playlist::PlaylistError> for StreamError {
    fn from(error: playlist::PlaylistError) -> Self {
        match error {
            playlist::PlaylistError::Http(error) => error.into(),
            other => {
                log::warn!("music playlist failure: {other}");
                match other {
                    playlist::PlaylistError::WebPage(_) => Self::WebPage,
                    playlist::PlaylistError::Read(_) => Self::Connection,
                    _ => Self::Playlist,
                }
            }
        }
    }
}

/// Ring buffer between the decoder thread and the mixer (5 s at 48 kHz).
const MUSIC_RING_FRAMES: usize = 5 * ENGINE_SAMPLE_RATE as usize;
/// Jitter buffer before playback starts (1 s).
const MUSIC_PREBUFFER_FRAMES: usize = ENGINE_SAMPLE_RATE as usize;
/// Like the Legacy Viewer, resume after a starvation only when the buffer is
/// mostly full; the shorter initial threshold keeps station changes responsive.
const MUSIC_REBUFFER_FRAMES: usize = MUSIC_RING_FRAMES * 4 / 5;
const STARVATION_LOG_INTERVAL: Duration = Duration::from_secs(10);
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
        // A retry must not erase the reason for silence. Only decoded audio,
        // a newer failure or a controller generation change clears it.
        if slot.generation == self.generation
            && slot.status != status
            && !(matches!(status, StreamStatus::Connecting) && matches!(slot.status, StreamStatus::Error(_)))
        {
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
            self.slot.lock().status = StreamStatus::Error(StreamError::Worker);
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

struct StarvationMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl StarvationMonitor {
    fn start(starved: Arc<AtomicBool>) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::Builder::new().name("aurora-audio-starvation".into()).spawn(move || {
            let mut last_log = None;
            while !sleep_or_stop(Duration::from_millis(100), &thread_stop) {
                if should_log_starvation(&starved, &mut last_log, Instant::now()) {
                    log::warn!("music stream: starved, rebuffering");
                }
            }
        });
        match thread {
            Ok(thread) => Some(Self {
                stop,
                thread: Some(thread),
            }),
            Err(error) => {
                log::warn!("cannot start music starvation monitor: {error}");
                None
            }
        }
    }
}

impl Drop for StarvationMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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

fn play_once(url: &str, stop: &Arc<AtomicBool>, status: &StatusHandle, cmd_tx: &Sender<Command>) -> Result<EndReason, StreamError> {
    let resp = playlist::open(url, stop)?;
    let hint = resp
        .content_type
        .as_deref()
        .and_then(hint_for_content_type)
        .or_else(|| hint_for_path(resp.url.path()));
    let title: TitleSlot = Arc::new(Mutex::new(None));
    let reader: Box<dyn std::io::Read + Send + Sync> = match resp.metaint {
        Some(mi) => Box::new(IcyReader::new(resp.body, mi, title.clone())),
        None => resp.body,
    };
    let mut decoder = DecodeStream::open(Box::new(ReadOnlySource::new(reader)), hint).map_err(|e| {
        log::warn!("music decoder failure: {e}");
        StreamError::AudioFormat
    })?;

    let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(MUSIC_RING_FRAMES * 2);
    let starved = Arc::new(AtomicBool::new(false));
    let source = PcmSource::new_with_rebuffer(
        Channel::Music,
        cons,
        MUSIC_PREBUFFER_FRAMES,
        MUSIC_REBUFFER_FRAMES,
        None,
        Some(stop.clone()),
        Some(starved.clone()),
    );
    if cmd_tx.send(Command::AddPcm(Box::new(source))).is_err() {
        return Ok(EndReason::Shutdown);
    }
    let _starvation_monitor = StarvationMonitor::start(starved.clone());

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
                    log::warn!("music decoding interrupted: {e}");
                    return Err(StreamError::Interrupted);
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

fn should_log_starvation(starved: &AtomicBool, last_log: &mut Option<Instant>, now: Instant) -> bool {
    if !starved.swap(false, Ordering::Relaxed) {
        return false;
    }
    if last_log.is_some_and(|last| now.saturating_duration_since(last) < STARVATION_LOG_INTERVAL) {
        return false;
    }
    *last_log = Some(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::tests::mp3_silence;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn parcel_music_retry_keeps_last_error_until_audio_returns() {
        let slot = Arc::new(Mutex::new(StatusSlot {
            generation: 1,
            status: StreamStatus::Error(StreamError::Connection),
        }));
        let handle = StatusHandle {
            slot: slot.clone(),
            generation: 1,
        };
        handle.set(StreamStatus::Connecting);
        assert!(matches!(slot.lock().status, StreamStatus::Error(_)));
        handle.set(StreamStatus::Playing { title: None });
        assert_eq!(slot.lock().status, StreamStatus::Playing { title: None });
    }

    /// The error a station serving `body` as `content_type` ends in, through
    /// the controller's real path (`playlist::open`, then the decoder).
    fn loopback_content_error(content_type: &str, body: &str) -> StreamError {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}");
        let server = thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut sock = std::io::BufReader::new(sock);
            let mut line = String::new();
            loop {
                use std::io::BufRead;
                line.clear();
                if sock.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            sock.get_mut().write_all(response.as_bytes()).unwrap();
            sock.get_mut().shutdown(std::net::Shutdown::Write).unwrap();
            let mut drain = Vec::new();
            let _ = sock.read_to_end(&mut drain);
        });
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ctl = StreamController::new(tx);
        ctl.play(&url);
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = loop {
            if let StreamStatus::Error(error) = ctl.status() {
                break error;
            }
            assert!(Instant::now() < deadline, "{:?}", ctl.status());
            thread::sleep(Duration::from_millis(10));
        };
        ctl.stop();
        server.join().unwrap();
        error
    }

    #[test]
    fn parcel_music_loopback_html_has_a_readable_error() {
        // A page that exports no radio playlist (not an AzuraCast player).
        let error = loopback_content_error("text/html; charset=utf-8", "<html>Not a radio stream</html>");
        assert_eq!(error, StreamError::WebPage);
        assert!(error.to_string().contains("web page"), "{error}");
    }

    #[test]
    fn parcel_music_loopback_playlists_have_a_readable_error() {
        // Playlists the radio decoder cannot follow: HLS, and one without a stream.
        for (mime, body) in [
            ("application/vnd.apple.mpegurl", "#EXTM3U\n#EXT-X-TARGETDURATION:10\nsegment.ts"),
            ("audio/x-mpegurl", "#EXTM3U\n#EXTINF:-1,nothing"),
            ("audio/x-scpls", "[playlist]\nNumberOfEntries=0"),
        ] {
            let error = loopback_content_error(mime, body);
            assert_eq!(error, StreamError::Playlist, "{mime}");
            assert!(error.to_string().contains("playlist"), "{error}");
        }
        // A playlist link to another site is refused as an address.
        assert_eq!(
            loopback_content_error("audio/x-mpegurl", "http://radio.example/stream"),
            StreamError::Address
        );
    }

    #[test]
    fn parcel_music_loopback_http_error_survives_pending_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let server = thread::spawn(move || {
            for attempt in 0..2 {
                let deadline = Instant::now() + Duration::from_secs(10);
                let (mut sock, _) = loop {
                    match listener.accept() {
                        Ok(client) => break client,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "client did not connect");
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                sock.set_nonblocking(false).unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                if attempt == 0 {
                    sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
                    sock.shutdown(std::net::Shutdown::Write).unwrap();
                    let _ = sock.read(&mut buf);
                } else {
                    ready_tx.send(()).unwrap();
                    let _ = done_rx.recv_timeout(Duration::from_secs(5));
                }
            }
        });
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ctl = StreamController::new(tx);
        ctl.play(&url);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let status = ctl.status();
        ctl.stop();
        let _ = done_tx.send(());
        server.join().unwrap();
        assert_eq!(status, StreamStatus::Error(StreamError::HttpStatus(404)));
    }

    #[test]
    fn parcel_music_classifies_tls_address_and_http_errors() {
        assert_eq!(StreamError::from(http::HttpError::Status(503)), StreamError::HttpStatus(503));
        let tls = std::io::Error::other(rustls::Error::General("handshake failed".into()));
        assert_eq!(StreamError::from(http::HttpError::Io(tls)), StreamError::Tls);
        assert_eq!(StreamError::from(http::HttpError::PlaylistOrigin), StreamError::Address);
        assert_eq!(
            StreamError::from(http::HttpError::Connect("unreachable".into())),
            StreamError::Connection
        );
    }

    #[test]
    fn starvation_log_is_rate_limited_and_consumes_each_signal() {
        let signal = AtomicBool::new(true);
        let start = Instant::now();
        let mut last = None;
        assert!(should_log_starvation(&signal, &mut last, start));
        assert!(!should_log_starvation(&signal, &mut last, start + Duration::from_secs(1)));
        signal.store(true, Ordering::Relaxed);
        assert!(!should_log_starvation(&signal, &mut last, start + Duration::from_secs(5)));
        signal.store(true, Ordering::Relaxed);
        assert!(should_log_starvation(&signal, &mut last, start + Duration::from_secs(10)));
    }

    #[test]
    fn music_buffer_keeps_two_seconds_of_stall_margin() {
        assert_eq!(MUSIC_RING_FRAMES, 5 * ENGINE_SAMPLE_RATE as usize);
        assert_eq!(MUSIC_PREBUFFER_FRAMES, ENGINE_SAMPLE_RATE as usize);
        assert_eq!(MUSIC_REBUFFER_FRAMES, MUSIC_RING_FRAMES * 4 / 5);
    }

    #[test]
    #[ignore = "contacts the public HTTPS station reported in #1628"]
    fn issue_1628_live_probe() {
        let stop = Arc::new(AtomicBool::new(false));
        let resp = http::open("https://stream.electroradio.fm/320k", &stop).unwrap();
        eprintln!("content_type={:?} metaint={:?}", resp.content_type, resp.metaint);
        let title = Arc::new(Mutex::new(None));
        let reader: Box<dyn Read + Send + Sync> = match resp.metaint {
            Some(mi) => Box::new(IcyReader::new(resp.body, mi, title.clone())),
            None => resp.body,
        };
        let mut decoder = DecodeStream::open(Box::new(ReadOnlySource::new(reader)), Some("mp3")).unwrap();
        let mut frames = 0;
        let mut peak = 0.0_f32;
        while frames < 44_100 {
            let chunk = decoder.next_chunk().unwrap().expect("audio chunk");
            frames += chunk.samples.len() / chunk.channels;
            peak = chunk.samples.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
        }
        eprintln!("decoded frames={frames} peak={peak} title={:?}", *title.lock());
        assert!(peak > 0.0);
    }

    /// Serves one ICY response on loopback: MP3 frames with in-band metadata.
    fn serve_icy_once(metaint: usize) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let req = String::from_utf8_lossy(&req).to_string();
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
            // Dropping the socket now can reset the connection and discard
            // what the client has not read yet, the response head included
            // (#1422, Windows 11 26H2). End the stream cleanly and keep the
            // socket until the client hangs up.
            let _ = sock.shutdown(std::net::Shutdown::Write);
            let _ = sock.set_read_timeout(Some(Duration::from_secs(10)));
            while matches!(sock.read(&mut buf), Ok(n) if n > 0) {}
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
