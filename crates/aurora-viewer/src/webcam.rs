//! Webcam landmarks arrive over a private child-process pipe. Only the
//! viewer's own rendered skeleton consumes them.

mod panel;
mod recording;
mod retarget;
mod soundtrack;
mod take_tools;

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};

use base64::Engine;
use serde::Deserialize;

use retarget::Channels;
pub(crate) use retarget::Pose;
use uuid::Uuid;

pub const WINDOW: &str = "webcam_tracking";
const SCRIPT: &str = include_str!("webcam_capture.py");
const LOAD_SCRIPT: &str = "import sys; _aurora_script = sys.stdin.buffer.read(int(sys.stdin.buffer.readline())); exec(compile(_aurora_script, '<aurora-webcam>', 'exec'))";
const FRAME_BYTES: u64 = 131_072;

#[derive(Clone, Copy, Deserialize)]
pub(crate) struct FootContact {
    pub(crate) id: u64,
    pub(crate) weight: f32,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Frame {
    body: [Option<[f32; 3]>; 14],
    left: Option<[[f32; 3]; 21]>,
    right: Option<[[f32; 3]; 21]>,
    face: Option<[[f32; 3]; 16]>,
    head: Option<[[f32; 3]; 3]>,
    planar: bool,
    profile: f32,
    feet: [Option<FootContact>; 2],
}

#[derive(Deserialize)]
struct Sample {
    id: Option<u64>,
    frame: Option<Frame>,
    preview: Option<PreviewFrame>,
    video_started: Option<f32>,
    video_duration: Option<f32>,
    link_media: Option<PathBuf>,
    link_error: Option<LinkError>,
    still: Option<bool>,
    ended: Option<bool>,
    error: Option<String>,
    fatal: Option<String>,
}

#[derive(Deserialize)]
struct PreviewFrame {
    jpeg: String,
    groups: Vec<Vec<[f32; 2]>>,
}

struct PreviewImage {
    image: egui::ColorImage,
    groups: Vec<Vec<[f32; 2]>>,
}

impl PreviewFrame {
    fn decode(self) -> Option<PreviewImage> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(self.jpeg).ok()?;
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(384);
        limits.max_image_height = Some(384);
        reader.limits(limits);
        let rgb = reader.decode().ok()?.to_rgb8();
        let size = [rgb.width() as usize, rgb.height() as usize];
        Some(PreviewImage {
            image: egui::ColorImage::from_rgb(size, rgb.as_raw()),
            groups: self.groups,
        })
    }
}

#[derive(Default)]
struct Latest {
    id: Option<u64>,
    frame: Option<(Instant, Frame)>,
    preview: Option<PreviewImage>,
    video_started: Option<(Instant, f32)>,
    video_duration: Option<f32>,
    link_media: Option<PathBuf>,
    link_error: Option<LinkError>,
    still: bool,
    finished: Option<bool>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
struct Camera {
    index: i32,
    backend: i32,
}

#[derive(Deserialize)]
struct Device {
    #[serde(flatten)]
    camera: Camera,
    name: String,
}

#[derive(Clone, PartialEq, Eq)]
enum Source {
    Camera(Camera),
    File(PathBuf, FileKind),
    Link(String),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LinkError {
    Unsupported,
    Dependencies,
    Unavailable,
    TooLarge,
    Download,
}

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi"];
const IMAGE_EXTENSIONS: &[&str] = &["gif", "png", "jpg", "jpeg", "bmp", "webp"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Video,
    Gif,
    Image,
}

impl FileKind {
    fn of(path: &Path) -> Option<Self> {
        let suffix = path.extension()?.to_str()?.to_ascii_lowercase();
        match suffix.as_str() {
            "gif" => Some(Self::Gif),
            ext if IMAGE_EXTENSIONS.contains(&ext) => Some(Self::Image),
            ext if VIDEO_EXTENSIONS.contains(&ext) => Some(Self::Video),
            _ => None,
        }
    }
}

impl Source {
    fn file(path: PathBuf) -> Self {
        let kind = FileKind::of(&path).unwrap_or(FileKind::Video);
        Self::File(path, kind)
    }

    fn file_kind(&self) -> Option<FileKind> {
        match self {
            Self::Camera(_) => None,
            Self::File(_, kind) => Some(*kind),
            Self::Link(_) => Some(FileKind::Video),
        }
    }
}

impl Default for Source {
    fn default() -> Self {
        Self::Camera(Camera::default())
    }
}

struct Capture {
    child: Option<Child>,
    input: ChildStdin,
    serial: u64,
    latest: Arc<Mutex<Latest>>,
}

fn companion() -> Command {
    let python = std::env::var_os("AURORA_WEBCAM_PYTHON").unwrap_or_else(|| "python".into());
    let mut command = Command::new(python);
    // Keep the script out of the Windows command line (32,767 characters).
    command.args(["-I", "-u", "-c", LOAD_SCRIPT]).stdin(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn send_script(input: &mut ChildStdin) -> std::io::Result<()> {
    writeln!(input, "{}", SCRIPT.len())?;
    input.write_all(SCRIPT.as_bytes())
}

fn enumerate_cameras() -> std::io::Result<Vec<Device>> {
    let mut child = companion().arg("--list-cameras").stdout(Stdio::piped()).spawn()?;
    send_script(&mut child.stdin.take().expect("piped webcam stdin"))?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(serde_json::from_slice(&output.stdout)?)
    } else {
        Ok(Vec::new())
    }
}

impl Capture {
    fn launch() -> std::io::Result<Self> {
        let mut command = companion();
        command.stdout(Stdio::piped());
        let mut child = command.spawn()?;
        let mut input = child.stdin.take().expect("piped webcam stdin");
        if let Err(error) = send_script(&mut input) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let stdout = child.stdout.take().expect("piped webcam stdout");
        let capture = Self {
            child: Some(child),
            input,
            serial: 0,
            latest: Arc::default(),
        };
        let latest = capture.latest.clone();
        std::thread::Builder::new().name("webcam-landmarks".into()).spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.by_ref().take(FRAME_BYTES).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.last() != Some(&b'\n') => break,
                    Ok(_) => {}
                }
                if let Ok(sample) = serde_json::from_slice::<Sample>(&line) {
                    let preview = sample.preview.and_then(PreviewFrame::decode);
                    if let Ok(mut slot) = latest.lock() {
                        if let Some(error) = sample.fatal {
                            log::warn!("webcam models unavailable: {error}");
                            slot.finished = Some(true);
                            continue;
                        }
                        if sample.id.is_none() || slot.id != sample.id {
                            continue;
                        }
                        if let Some(error) = sample.link_error {
                            slot.link_error = Some(error);
                            slot.finished = Some(true);
                        }
                        if sample.link_media.is_some() {
                            slot.link_media = sample.link_media;
                        }
                        if let Some(error) = sample.error {
                            log::warn!("webcam input failed: {error}");
                            slot.finished = Some(true);
                        } else if sample.ended == Some(true) {
                            slot.finished = Some(false);
                        }
                        if let Some(frame) = sample.frame {
                            slot.frame = Some((Instant::now(), frame));
                        }
                        if let Some(offset) = sample.video_started {
                            slot.video_started = Some((Instant::now(), offset));
                        }
                        if let Some(duration) = sample.video_duration.filter(|duration| duration.is_finite() && *duration > 0.0) {
                            slot.video_duration = Some(duration);
                        }
                        slot.still |= sample.still == Some(true);
                        if preview.is_some() {
                            slot.preview = preview;
                        }
                    }
                }
            }
        })?;
        Ok(capture)
    }

    fn send(&mut self, message: serde_json::Value) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        self.input.write_all(&line)
    }

    fn start(&mut self, source: &Source, loop_video: bool, transport: &Transport) -> std::io::Result<()> {
        self.serial += 1;
        let id = self.serial;
        *self.latest.lock().unwrap() = Latest {
            id: Some(id),
            ..Latest::default()
        };
        let mut message = match source {
            Source::Camera(camera) => serde_json::json!({"id": id, "camera": camera.index, "backend": camera.backend}),
            Source::File(path, FileKind::Video) => {
                serde_json::json!({"id": id, "video": path.to_string_lossy(), "loop_video": loop_video})
            }
            Source::File(path, _) => serde_json::json!({"id": id, "image": path.to_string_lossy(), "loop_video": loop_video}),
            Source::Link(url) => serde_json::json!({"id": id, "url": url, "loop_video": loop_video}),
        };
        if matches!(source, Source::Link(_) | Source::File(_, FileKind::Video)) {
            message["offset"] = transport.position.into();
            message["paused"] = transport.paused.into();
            message["rate"] = transport.rate.into();
            if let Some([start, end]) = transport.selection {
                message["loop_in"] = start.into();
                message["out"] = end.into();
            }
        }
        self.send(message)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.as_mut().expect("active webcam process").try_wait()
    }

    fn stop(&mut self) {
        *self.latest.lock().unwrap() = Latest::default();
        let _ = self.send(serde_json::json!({"stop": true}));
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Give the worker time to cancel a download and remove its files.
            let _ = self.send(serde_json::json!({"shutdown": true}));
            std::thread::spawn(move || {
                for _ in 0..30 {
                    if !matches!(child.try_wait(), Ok(None)) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

#[derive(Clone, Copy)]
struct MediaClock {
    began: Instant,
    offset: f32,
    rate: f32,
}

impl MediaClock {
    fn position(self, now: Instant) -> f32 {
        self.offset + now.saturating_duration_since(self.began).as_secs_f32() * self.rate
    }
}

struct Transport {
    duration: f32,
    position: f32,
    paused: bool,
    rate: f32,
    selection: Option<[f32; 2]>,
    clock: Option<MediaClock>,
    scrub: Option<f32>,
}

pub(crate) struct GestureAssets {
    pub(crate) animation: Uuid,
    pub(crate) sounds: Vec<(f32, Uuid)>,
}

struct GestureUpload {
    animation: Uuid,
    sounds: Vec<(f32, Uuid)>,
    received: std::collections::HashMap<Uuid, Uuid>,
}

impl GestureUpload {
    fn assets(&self) -> Option<GestureAssets> {
        Some(GestureAssets {
            animation: *self.received.get(&self.animation)?,
            sounds: self
                .sounds
                .iter()
                .map(|(at, entry)| Some((*at, *self.received.get(entry)?)))
                .collect::<Option<_>>()?,
        })
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            duration: 0.0,
            position: 0.0,
            paused: false,
            rate: 1.0,
            selection: None,
            clock: None,
            scrub: None,
        }
    }
}

impl Transport {
    fn range(&self) -> [f32; 2] {
        self.selection.unwrap_or([0.0, self.duration])
    }

    fn position(&self, now: Instant) -> f32 {
        self.clock
            .map_or(self.position, |clock| clock.position(now))
            .clamp(0.0, self.range()[1])
    }
}

#[derive(Default)]
pub(crate) struct Webcam {
    pub open: bool,
    capture: Option<Capture>,
    running: bool,
    still: bool,
    warm_attempted: bool,
    warming_before_open: bool,
    panel: panel::Panel,
    mirror: bool,
    error: bool,
    link_error: Option<LinkError>,
    link_path: Option<PathBuf>,
    latest: Option<(Instant, Frame)>,
    rendered: Option<(Instant, Frame)>,
    neutral: Option<Frame>,
    preview: Option<PreviewImage>,
    clip_audio: Option<aurora_audio::LocalMediaAudio>,
    soundtrack_source: Option<take_tools::AudioSource>,
    transport: Transport,
    video_muted: Arc<AtomicBool>,
    fps: f32,
    front_view_requested: bool,
    recorder: recording::Recorder,
    gesture_upload: Option<GestureUpload>,
    gesture_requested: bool,
}

impl Webcam {
    pub fn take_front_view_request(&mut self) -> bool {
        std::mem::take(&mut self.front_view_requested)
    }

    pub fn stop(&mut self) {
        if self.running
            && let Some(capture) = &mut self.capture
        {
            capture.stop();
        }
        self.clear_input();
    }

    fn clear_input(&mut self) {
        self.recorder.finish();
        self.running = false;
        self.still = false;
        self.latest = None;
        self.rendered = None;
        self.neutral = None;
        self.preview = None;
        self.clip_audio = None;
        self.soundtrack_source = None;
        self.transport = Transport::default();
        self.link_path = None;
        self.panel.clear_preview();
        self.fps = 0.0;
    }

    fn warm(&mut self) -> std::io::Result<&mut Capture> {
        if let Some(capture) = &mut self.capture
            && !matches!(capture.try_wait(), Ok(None))
        {
            self.capture = None;
        }
        if self.capture.is_none() {
            self.capture = Some(Capture::launch()?);
        }
        Ok(self.capture.as_mut().unwrap())
    }

    fn prewarm(&mut self) {
        if !self.warm_attempted {
            self.warm_attempted = true;
            if let Err(error) = self.warm() {
                log::warn!("cannot prewarm webcam tracking: {error}");
            }
        }
    }

    pub fn prewarm_demo(&mut self) {
        self.prewarm();
        self.warming_before_open = self.capture.is_some();
    }

    fn start(&mut self) {
        self.clear_input();
        self.recorder.hide_preview();
        self.link_error = None;
        let source = self.panel.source.clone();
        if let Source::File(path, FileKind::Video) = &source {
            self.soundtrack_source = Some(take_tools::AudioSource::local(path.clone()));
        }
        let loop_video = self.panel.loop_video;
        match self
            .warm()
            .and_then(|capture| capture.start(&source, loop_video, &Transport::default()))
        {
            Ok(()) => {
                self.running = true;
                self.error = false;
            }
            Err(error) => {
                log::warn!("cannot launch webcam tracking: {error}");
                self.capture = None;
                self.error = true;
            }
        }
    }

    fn restart_video(&mut self, position: f32, paused: bool) {
        self.recorder.pause(Instant::now());
        self.recorder.hide_preview();
        let [start, end] = self.transport.range();
        self.transport.position = position.clamp(start, if paused { end } else { (end - 0.05).max(start) });
        self.transport.paused = paused;
        self.transport.clock = None;
        self.clip_audio = None;
        self.latest = None;
        self.rendered = None;
        self.fps = 0.0;
        if let Some(capture) = &mut self.capture {
            let result = capture.start(&self.panel.source, self.panel.loop_video, &self.transport);
            self.running = result.is_ok();
            self.error = result.is_err();
        }
    }

    fn pause_video(&mut self, position: f32) {
        self.recorder.pause(Instant::now());
        self.transport.position = position;
        self.transport.paused = true;
        self.transport.clock = None;
        self.clip_audio = None;
        if let Some(capture) = &mut self.capture {
            capture.stop();
        }
    }

    pub(crate) fn take_recorded_upload(&mut self) -> Option<crate::ik_edit::PreparedAnimation> {
        self.recorder.take_upload()
    }

    pub(crate) fn take_media_upload(&mut self) -> Option<recording::PreparedMedia> {
        self.recorder.take_media()
    }

    pub(crate) fn track_gesture_upload(&mut self, animation: Uuid, sounds: Vec<(f32, Uuid)>) {
        self.gesture_upload = Some(GestureUpload {
            animation,
            sounds,
            received: Default::default(),
        });
        self.gesture_requested = false;
    }

    pub(crate) fn uploaded(&mut self, entry: Uuid, item: Uuid) {
        if let Some(upload) = &mut self.gesture_upload
            && (entry == upload.animation || upload.sounds.iter().any(|(_, sound)| *sound == entry))
        {
            upload.received.insert(entry, item);
            self.gesture_requested = upload.assets().is_some();
        }
    }

    pub(crate) fn take_gesture_request(&mut self) -> Option<GestureAssets> {
        if !std::mem::take(&mut self.gesture_requested) {
            return None;
        }
        self.gesture_upload.as_ref()?.assets()
    }

    pub(crate) fn clear_gesture_upload(&mut self) {
        self.gesture_upload = None;
    }

    pub fn update(&mut self, audio: Option<&aurora_audio::AudioEngine>, rig: &crate::scene::anim::Rig) -> Option<Arc<Pose>> {
        let now = Instant::now();
        if self.open
            && let Some(preview) = self.recorder.preview(rig, now)
        {
            return Some(preview);
        }
        let pose = self.update_live(audio);
        if self.running && !self.still {
            let fresh = self
                .latest
                .as_ref()
                .is_some_and(|(time, _)| now.saturating_duration_since(*time).as_secs_f32() < 0.25);
            let awaiting_video = self.current_file_kind() == Some(FileKind::Video) && self.transport.clock.is_none();
            if !awaiting_video {
                let recording = self.recorder.is_active();
                self.recorder.tick(
                    rig,
                    now,
                    pose.as_deref().filter(|_| fresh),
                    self.soundtrack_source.as_ref().zip(self.transport.clock),
                );
                if self.transport.duration > 0.0 && recording && !self.recorder.is_active() {
                    self.pause_video(self.transport.position(now));
                }
            }
        }
        pose
    }

    fn update_live(&mut self, audio: Option<&aurora_audio::AudioEngine>) -> Option<Arc<Pose>> {
        if !self.open {
            if !self.warming_before_open {
                if self.capture.take().is_some() {
                    self.clear_input();
                }
                self.warm_attempted = false;
            }
            return None;
        }
        let capture = self.capture.as_mut()?;
        let exit = capture.try_wait();
        if !matches!(exit, Ok(None)) {
            self.capture = None;
            self.clear_input();
            self.error = !matches!(exit, Ok(Some(status)) if status.success());
            return None;
        }
        if !self.running {
            return None;
        }
        let pending = capture.latest.try_lock().ok().map(|mut slot| {
            (
                slot.finished.take(),
                slot.preview.take(),
                slot.frame.take(),
                slot.video_started.take(),
                slot.video_duration.take(),
                slot.link_media.take(),
                slot.link_error.take(),
                slot.still,
            )
        });
        if let Some((finished, preview, sample, video_started, video_duration, link_media, link_error, still)) = pending {
            if let Some(duration) = video_duration {
                self.transport.duration = duration;
            }
            if let Some(preview) = preview {
                self.preview = Some(preview);
            }
            if let Some((when, frame)) = sample {
                self.update_target(when, frame);
            }
            if let Some(error) = finished {
                if !error && self.transport.duration > 0.0 {
                    self.pause_video(self.transport.range()[1]);
                } else {
                    self.stop();
                    self.error = error;
                    self.link_error = link_error;
                    return None;
                }
            }
            if let Some(path) = link_media {
                if self.link_path.as_ref() != Some(&path) && FileKind::of(&path) == Some(FileKind::Video) {
                    self.soundtrack_source = Some(take_tools::AudioSource::downloaded(&path));
                }
                self.link_path = Some(path);
            }
            self.still = still;
            if let Some((began, offset)) = video_started.filter(|_| finished.is_none() && !self.transport.paused) {
                let rate = self.transport.rate;
                self.transport.clock = Some(MediaClock { began, offset, rate });
                self.clip_audio = None;
                let path = match &self.panel.source {
                    Source::File(path, FileKind::Video) => Some(path),
                    Source::Link(_) => self.link_path.as_ref(),
                    _ => None,
                };
                if let Some((engine, path)) = audio.zip(path) {
                    self.clip_audio = engine.play_local_media(path.clone(), began, offset.into(), rate.into(), self.video_muted.clone());
                }
            }
        }
        let (received, target) = self.latest.as_ref()?;
        let now = Instant::now();
        let age = now.saturating_duration_since(*received).as_secs_f32();
        let fade_after = (2.0 / self.fps.max(3.0)).clamp(0.18, 0.45);
        let strength = if self.still || self.transport.paused {
            1.0
        } else {
            (1.0 - (age - fade_after) / 0.35).clamp(0.0, 1.0)
        };
        if strength == 0.0 || !target.detected() {
            self.rendered = None;
            return None;
        }
        // Apply new landmarks across render frames so sparse estimates do not
        // move the avatar in single-frame jumps.
        let mut frame = target.clone();
        if let Some((at, old)) = &self.rendered {
            let elapsed = now.saturating_duration_since(*at).as_secs_f32();
            let time_constant = (0.7 / self.fps.max(3.0)).clamp(0.028, 0.09);
            frame.smooth_from(old, 1.0 - (-elapsed / time_constant).exp());
        }
        self.rendered = Some((now, frame.clone()));
        let channels = self.panel.channels;
        let pose = if self.still {
            if self.mirror {
                frame = frame.mirrored();
            }
            Pose::still(&frame, channels)
        } else if self.mirror {
            Pose::new(
                &frame.mirrored(),
                self.neutral.as_ref().map(Frame::mirrored).as_ref(),
                strength,
                channels,
            )
        } else {
            Pose::new(&frame, self.neutral.as_ref(), strength, channels)
        };
        Some(Arc::new(pose))
    }

    fn update_target(&mut self, when: Instant, frame: Frame) {
        if self.latest.as_ref().is_some_and(|(_, previous)| previous.planar != frame.planar) {
            self.neutral = None;
            self.rendered = None;
        }
        let neutral = self.neutral.get_or_insert_with(|| frame.clone());
        if neutral.face.is_none() {
            neutral.face = frame.face;
        }
        if neutral.head.is_none() {
            neutral.head = frame.head;
        }
        if neutral.body[0].is_none() || neutral.body[3].is_none() {
            neutral.body = frame.body;
            neutral.profile = frame.profile;
        }
        if let Some((previous_at, _)) = &self.latest {
            let elapsed = when.saturating_duration_since(*previous_at).as_secs_f32();
            if elapsed > 0.001 {
                let rate = (1.0 / elapsed).min(120.0);
                self.fps = if self.fps == 0.0 { rate } else { self.fps * 0.88 + rate * 0.12 };
            }
        }
        self.latest = Some((when, frame));
    }
}

impl Frame {
    fn detected(&self) -> bool {
        self.face.is_some() || self.head.is_some() || self.left.is_some() || self.right.is_some() || self.body.iter().any(Option::is_some)
    }

    fn legs_detected(&self) -> bool {
        (0..2).any(|side| self.body[6 + side].is_some() && self.body[8 + side].is_some() && self.body[10 + side].is_some())
    }

    fn mirrored(&self) -> Self {
        let flip = |[x, y, z]: [f32; 3]| [x, -y, z];
        let body = [3, 4, 5, 0, 1, 2, 7, 6, 9, 8, 11, 10, 13, 12].map(|i| self.body[i].map(flip));
        let face = self
            .face
            .map(|p| [1, 0, 2, 3, 4, 5, 7, 6, 12, 13, 14, 15, 8, 9, 10, 11].map(|i| flip(p[i])));
        Self {
            body,
            face,
            head: self.head.map(|p| [flip(p[0]), flip(p[2]), flip(p[1])]),
            planar: self.planar,
            profile: self.profile,
            feet: [self.feet[1], self.feet[0]],
            left: self.right.map(|p| p.map(flip)),
            right: self.left.map(|p| p.map(flip)),
        }
    }

    fn smooth_from(&mut self, previous: &Self, amount: f32) {
        fn smooth(point: &mut [f32; 3], previous: &[f32; 3], amount: f32) {
            for (current, old) in point.iter_mut().zip(previous) {
                *current = old + (*current - old) * amount;
            }
        }

        fn smooth_group<const N: usize>(group: &mut Option<[[f32; 3]; N]>, previous: &Option<[[f32; 3]; N]>, amount: f32) {
            if let Some((current, old)) = group.as_mut().zip(previous.as_ref()) {
                for (point, old) in current.iter_mut().zip(old) {
                    smooth(point, old, amount);
                }
            }
        }

        for (point, old) in self.body.iter_mut().zip(&previous.body) {
            if let Some((point, old)) = point.as_mut().zip(old.as_ref()) {
                smooth(point, old, amount);
            }
        }
        smooth_group(&mut self.left, &previous.left, amount);
        smooth_group(&mut self.right, &previous.right, amount);
        smooth_group(&mut self.face, &previous.face, amount);
        smooth_group(&mut self.head, &previous.head, amount);
        self.profile = previous.profile + (self.profile - previous.profile) * amount;
    }
}
