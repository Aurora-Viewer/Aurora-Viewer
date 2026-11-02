//! Linux: the volume of a web page's sound, set on the sound server.
//!
//! `set_volume` reaches `media_plugin_cef` like on the other systems, but the
//! Linux plugin is built with `dummy_volume_catcher.cpp`, whose `setVolume`
//! does nothing: the Legacy Viewer's build has its PulseAudio catcher turned
//! off (indra/media_plugins/cef/CMakeLists.txt, `include(MediaPluginBase)`
//! commented out). Master mute, the media slider, a face's mute and the
//! fade with distance never touched a page's sound.
//!
//! So the viewer does the work of `VolumeCatcherImpl`
//! (indra/media_plugins/cef/linux_volume_catcher.cpp, originally LGPL 2.1)
//! itself, with the pack as it is. That catcher lists the server's streams
//! and walks each one's process tree up to the plugin. Here each plugin is
//! started with a tag instead (`PULSE_PROP_OVERRIDE`, read by the libpulse
//! inside CEF): an exact match, for one viewer among several. The same
//! variable names the stream. CEF calls it "Chromium", and the server keeps a
//! stream's volume per application name, so a page we mute would have left
//! the user's own Chromium muted.
//!
//! A stream is only touched once its plugin has been given a volume: the web
//! windows have none, and keep the sound they always had.
//!
//! All of it runs on a thread of its own, over the server's protocol
//! (`pulseaudio`, through pipewire-pulse under PipeWire): the frame only
//! sends a request. Without a sound server nothing happens, as CEF then plays
//! through ALSA, out of reach of any catcher.

use pulseaudio::protocol;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// Property of the stream that says which plugin it belongs to.
const TAG: &CStr = c"aurora.media";
/// Name of the streams in the system's mixer, and of the saved volume.
const STREAM_NAME: &str = "Aurora Viewer media";
/// Longest wait for an answer of the server, on the worker's thread.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
/// Volumes this close (of 65,536 at full volume) are the same one.
const SAME_VOLUME: u32 = 4;
/// A server that refused us is asked again after this long, at the next
/// request.
const RETRY_AFTER: Duration = Duration::from_secs(10);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

enum Request {
    Set {
        tag: String,
        volume: f32,
    },
    Forget(String),
    /// A stream appeared or changed on the server.
    StreamChanged,
    /// The server closed the watching connection.
    ServerGone,
}

/// The sound of one plugin. Dropping it leaves the stream as it is: the
/// plugin goes with it.
#[derive(Debug)]
pub struct StreamVolume {
    tag: String,
    last: Option<f32>,
}

impl StreamVolume {
    pub fn new() -> StreamVolume {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        StreamVolume {
            // with the process id: two viewers run side by side
            tag: format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)),
            last: None,
        }
    }

    /// The variable the plugin is started with.
    pub fn env(&self) -> (String, String) {
        (
            "PULSE_PROP_OVERRIDE".to_string(),
            format!("application.name='{STREAM_NAME}' {}='{}'", TAG.to_string_lossy(), self.tag),
        )
    }

    /// Volume 0..1, kept for the streams to come. Never waits.
    pub fn set(&mut self, volume: f32) {
        if self.last != Some(volume) {
            self.last = Some(volume);
            send(Request::Set {
                tag: self.tag.clone(),
                volume,
            });
        }
    }
}

impl Drop for StreamVolume {
    fn drop(&mut self) {
        if self.last.is_some() {
            send(Request::Forget(std::mem::take(&mut self.tag)));
        }
    }
}

fn send(request: Request) {
    let _ = sender().send(request);
}

/// The worker's queue; it starts with the first plugin given a volume.
fn sender() -> &'static Sender<Request> {
    static WORKER: OnceLock<Sender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        let events = tx.clone();
        let started = std::thread::Builder::new()
            .name("aurora-media-volume".into())
            .spawn(move || run(rx, events));
        if let Err(e) = started {
            log::warn!("media volume: no thread ({e}): web pages keep their sound");
        }
        tx
    })
}

fn run(requests: Receiver<Request>, events: Sender<Request>) {
    let mut wanted: HashMap<String, f32> = HashMap::new();
    let mut server: Option<Server> = None;
    let mut retry_at: Option<Instant> = None;
    while let Ok(first) = requests.recv() {
        // everything waiting at once: a slider dragged asks many times
        let mut stale = false;
        for request in std::iter::once(first).chain(requests.try_iter()) {
            match request {
                Request::Set { tag, volume } => {
                    wanted.insert(tag, volume);
                }
                Request::Forget(tag) => {
                    wanted.remove(&tag);
                }
                Request::StreamChanged => {}
                Request::ServerGone => stale = true,
            }
        }
        if stale || wanted.is_empty() {
            server = None;
        }
        if wanted.is_empty() {
            continue;
        }
        if server.is_none() && retry_at.is_none_or(|at| Instant::now() >= at) {
            match Server::open(events.clone()) {
                Ok(s) => {
                    log::info!("media volume: on the sound server");
                    server = Some(s);
                    retry_at = None;
                }
                Err(e) => {
                    // once a run, then quietly at each retry
                    if retry_at.is_none() {
                        log::info!("media volume: no sound server ({e}): web pages keep their sound");
                    }
                    retry_at = Some(Instant::now() + RETRY_AFTER);
                }
            }
        }
        if let Some(s) = server.as_mut()
            && let Err(e) = s.apply(&wanted)
        {
            log::info!("media volume: sound server lost ({e})");
            server = None;
            retry_at = Some(Instant::now() + RETRY_AFTER);
        }
    }
}

/// A client connection: authentication and name done.
struct Connection {
    sock: BufReader<UnixStream>,
    version: u16,
    seq: u32,
}

impl Connection {
    fn open(name: &str) -> Result<Connection> {
        let path = pulseaudio::socket_path_from_env().ok_or("no socket")?;
        let stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(ANSWER_TIMEOUT))?;
        let mut sock = BufReader::new(stream);
        let cookie = pulseaudio::cookie_path_from_env()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default();
        let auth = protocol::AuthParams {
            version: protocol::MAX_VERSION,
            supports_shm: false,
            supports_memfd: false,
            cookie,
        };
        sock.get_mut().set_read_timeout(Some(ANSWER_TIMEOUT))?;
        protocol::write_command_message(sock.get_mut(), 0, &protocol::Command::Auth(auth), protocol::MAX_VERSION)?;
        let (_, reply) = protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION)?;
        let version = protocol::MAX_VERSION.min(reply.version);
        let mut props = protocol::Props::new();
        props.set(protocol::Prop::ApplicationName, CString::new(name)?);
        protocol::write_command_message(sock.get_mut(), 1, &protocol::Command::SetClientName(props), version)?;
        protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version)?;
        Ok(Connection { sock, version, seq: 2 })
    }

    fn send(&mut self, command: &protocol::Command) -> Result<()> {
        self.seq += 1;
        protocol::write_command_message(self.sock.get_mut(), self.seq, command, self.version)?;
        Ok(())
    }
}

/// One connection to ask and set, one that does nothing but watch the
/// streams: answers and events would otherwise share a socket.
struct Server {
    control: Connection,
    watching: UnixStream,
}

impl Server {
    fn open(events: Sender<Request>) -> Result<Server> {
        let control = Connection::open("Aurora Viewer media volume")?;
        let mut watch = Connection::open("Aurora Viewer media watch")?;
        watch.send(&protocol::Command::Subscribe(protocol::SubscriptionMask::SINK_INPUT))?;
        protocol::read_ack_message(&mut watch.sock)?;
        // events come when they come; closed by `Drop`
        watch.sock.get_mut().set_read_timeout(None)?;
        let watching = watch.sock.get_ref().try_clone()?;
        std::thread::Builder::new()
            .name("aurora-media-watch".into())
            .spawn(move || watch_streams(watch, events))?;
        Ok(Server { control, watching })
    }

    /// VolumeCatcherImpl::update_all_volumes for the streams that have a
    /// volume wanted and do not have it yet. Called for every change of a
    /// stream as well as for a new one: PipeWire tells a new stream's volume
    /// as 0, and its real one, or the one saved for its name, only in the
    /// changes that follow. So the viewer's volume also wins over the
    /// system's mixer, as the slider is the place to set it.
    fn apply(&mut self, wanted: &HashMap<String, f32>) -> Result<()> {
        self.control.send(&protocol::Command::GetSinkInputInfoList)?;
        let (_, streams) = protocol::read_reply_message::<protocol::SinkInputInfoList>(&mut self.control.sock, self.control.version)?;
        for stream in streams {
            let Some(&volume) = stream_tag(&stream.props).and_then(|tag| wanted.get(tag)) else {
                continue;
            };
            let Some(volume) = stream.volume_writable.then(|| target(&stream.cvolume, volume)).flatten() else {
                continue;
            };
            self.control
                .send(&protocol::Command::SetSinkInputVolume(protocol::SetStreamVolumeParams {
                    index: stream.index,
                    volume,
                }))?;
            protocol::read_ack_message(&mut self.control.sock)?;
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.watching.shutdown(std::net::Shutdown::Both);
    }
}

fn watch_streams(mut watch: Connection, events: Sender<Request>) {
    while let Ok((_, command)) = protocol::read_command_message(&mut watch.sock, watch.version) {
        if let protocol::Command::SubscribeEvent(event) = command
            && matches!(
                event.event_type,
                protocol::SubscriptionEventType::New | protocol::SubscriptionEventType::Changed
            )
            && events.send(Request::StreamChanged).is_err()
        {
            return;
        }
    }
    let _ = events.send(Request::ServerGone);
}

/// The plugin a stream belongs to (the server's values end with their NUL).
fn stream_tag(props: &protocol::Props) -> Option<&str> {
    let value = props.get_bytes(TAG)?;
    std::str::from_utf8(value.strip_suffix(&[0]).unwrap_or(value)).ok()
}

/// The channels at `volume`, or `None` if they already are: on the sound
/// server's cubic scale, as `pa_sw_volume_from_linear`. What the server
/// answers is its own rounding of what it was told, and setting it again
/// would be a change of its own, for ever: hence [`SAME_VOLUME`].
fn target(current: &protocol::ChannelVolume, volume: f32) -> Option<protocol::ChannelVolume> {
    let wanted = protocol::Volume::from_linear(volume.clamp(0.0, 1.0));
    if current
        .channels()
        .iter()
        .all(|v| v.as_u32().abs_diff(wanted.as_u32()) <= SAME_VOLUME)
    {
        return None;
    }
    let mut channels = protocol::ChannelVolume::empty();
    for _ in current.channels() {
        channels.push(wanted);
    }
    Some(channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_is_started_with_its_tag_and_our_name() {
        let volume = StreamVolume::new();
        let (name, value) = volume.env();
        assert_eq!(name, "PULSE_PROP_OVERRIDE");
        assert_eq!(
            value,
            format!("application.name='Aurora Viewer media' aurora.media='{}'", volume.tag)
        );
    }

    #[test]
    fn two_plugins_never_share_a_tag() {
        assert_ne!(StreamVolume::new().tag, StreamVolume::new().tag);
    }

    #[test]
    fn a_stream_says_which_plugin_it_belongs_to() {
        let mut props = protocol::Props::new();
        assert_eq!(stream_tag(&props), None);
        props.set_bytes(TAG, c"123-4".to_bytes_with_nul());
        assert_eq!(stream_tag(&props), Some("123-4"));
        props.set_bytes(TAG, b"123-5");
        assert_eq!(stream_tag(&props), Some("123-5"));
    }

    #[test]
    fn every_channel_takes_the_volume() {
        let stereo = protocol::ChannelVolume::norm(2);
        let muted = target(&stereo, 0.0).expect("a change");
        assert_eq!(muted.channels(), protocol::ChannelVolume::muted(2).channels());
        // the five channels of a surround stream stay five
        let half = target(&protocol::ChannelVolume::norm(5), 0.5).expect("a change");
        assert_eq!(half.channels().len(), 5);
        assert!(half.channels().iter().all(|v| (v.to_linear() - 0.5).abs() < 0.01));
    }

    #[test]
    fn a_stream_already_at_its_volume_is_left_alone() {
        assert!(target(&protocol::ChannelVolume::norm(2), 1.0).is_none());
        assert!(target(&protocol::ChannelVolume::muted(2), 0.0).is_none());
        // past the slider's range: clamped, not louder than full
        assert!(target(&protocol::ChannelVolume::norm(2), 1.5).is_none());
    }

    #[test]
    fn the_servers_rounding_is_not_a_change() {
        let wanted = protocol::Volume::from_linear(0.5);
        let rounded = protocol::Volume::from_u32_clamped(wanted.as_u32() + SAME_VOLUME);
        let mut stream = protocol::ChannelVolume::empty();
        stream.push(rounded);
        stream.push(rounded);
        assert!(target(&stream, 0.5).is_none());
        // the smallest step of a slider, at its top, still moves the sound
        assert!(target(&protocol::ChannelVolume::norm(2), 0.999).is_some());
    }
}
