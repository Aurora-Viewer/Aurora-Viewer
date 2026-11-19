//! The Director's recording: the frames of the 3D view, without the
//! interface, sent as raw RGBA to an ffmpeg the user has on their computer,
//! which encodes the film. Aurora's own: the Legacy Viewer records nothing.
//!
//! ffmpeg is a separate program: it is found on the PATH or chosen by the
//! user, never shipped nor downloaded by the viewer, and only ever run with
//! the arguments written here (`arguments`). What it can encode is asked of
//! it once (`Tool::probe`, its `-encoders` list): a format whose encoder that
//! build lacks is not offered.
//!
//! The film keeps real time: a frame is due every 1 / fps second of the
//! clock (`Pace`), taken from the view when it is, and written twice or more
//! when the viewer or the encoder fell behind, so the film lasts what the
//! shot lasted. The frames go to a writer thread through a short queue: the
//! frame loop never waits for the encoder.

use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::Instant;

/// Where ffmpeg is downloaded from, by the user.
pub const DOWNLOAD_PAGE: &str = "https://ffmpeg.org/download.html";
/// The frames a second a film can have.
pub const RATES: [u32; 6] = [24, 25, 30, 50, 60, 120];
/// Frames waiting for the writer: past this, a frame is skipped and the next
/// one written is repeated in its place.
const QUEUE: usize = 3;
/// A frame is never written more than this many seconds of film in a row:
/// past it the film is shortened rather than frozen.
const MAX_REPEAT_SECONDS: u32 = 2;
/// The first lines of what ffmpeg wrote, kept for the message of a failure:
/// it names the cause first, then what stopped because of it.
const ERROR_HEAD: usize = 600;

/// What the film is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Format {
    /// H.264 in an MP4: plays everywhere.
    #[default]
    Mp4,
    /// ProRes 422 in a MOV: for editing.
    ProRes,
    /// ProRes 4444 in a MOV, with its transparency: the isolation's empty
    /// background stays empty in the film.
    ProResAlpha,
    /// VP9 in a WebM.
    Webm,
    /// FFV1 in an MKV: lossless, large.
    Lossless,
}

impl Format {
    pub const ALL: [Format; 5] = [Format::Mp4, Format::ProRes, Format::ProResAlpha, Format::Webm, Format::Lossless];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::ProRes | Format::ProResAlpha => "mov",
            Format::Webm => "webm",
            Format::Lossless => "mkv",
        }
    }

    /// The ffmpeg encoders that write it, the preferred first.
    pub fn encoders(self) -> &'static [&'static str] {
        match self {
            Format::Mp4 => &["libx264", "h264_mf", "libopenh264", "mpeg4"],
            Format::ProRes => &["prores_ks", "prores"],
            Format::ProResAlpha => &["prores_ks"],
            Format::Webm => &["libvpx-vp9"],
            Format::Lossless => &["ffv1"],
        }
    }

    /// The film keeps the transparency of its frames.
    pub fn alpha(self) -> bool {
        self == Format::ProResAlpha
    }

    /// The ffmpeg audio encoders its sound is written with, the preferred
    /// first: AAC in an MP4, PCM beside ProRes (what editing expects of a
    /// MOV), Opus in a WebM, FLAC (lossless, like its picture) in an MKV.
    pub fn audio_encoders(self) -> &'static [&'static str] {
        match self {
            Format::Mp4 => &["aac", "libfdk_aac"],
            Format::ProRes | Format::ProResAlpha => &["pcm_s16le"],
            Format::Webm => &["libopus", "opus"],
            Format::Lossless => &["flac", "libopus"],
        }
    }

    /// The sides of its picture that must be even (width, height): pixels
    /// stored 4:2:0 halve both, 4:2:2 the width (`prores_ks` would pad an odd
    /// one, the other ProRes encoder refuses it).
    fn even(self) -> (bool, bool) {
        match self {
            Format::Mp4 | Format::Webm => (true, true),
            Format::ProRes => (true, false),
            Format::ProResAlpha | Format::Lossless => (false, false),
        }
    }
}

/// How much the encoder keeps: a lighter file, or a cleaner picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Quality {
    Light,
    #[default]
    Good,
    Best,
}

/// What the user chose for the next film (kept in the settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub format: Format,
    pub quality: Quality,
    pub fps: u32,
    /// The ffmpeg chosen by hand; `None`: the one on the PATH.
    pub ffmpeg: Option<PathBuf>,
    /// The Output section's mode: an offline render, not the live film.
    pub render: bool,
    /// The render's size (`director_render.rs`).
    pub resolution: crate::director_render::Resolution,
    /// The render as a sequence of PNG images (no ffmpeg needed).
    pub png: bool,
    /// The live film carries the viewer's own sound: the world's sounds,
    /// the music and the media, as the user hears them.
    pub sound: bool,
    /// The interface's sounds in it too.
    pub ui_sounds: bool,
    /// Voice chat in it: other people's voices, off unless asked.
    pub voice: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            format: Format::Mp4,
            quality: Quality::Good,
            fps: 30,
            ffmpeg: None,
            render: false,
            resolution: crate::director_render::Resolution::default(),
            png: false,
            sound: true,
            ui_sounds: false,
            voice: false,
        }
    }
}

impl Options {
    /// A rate the film can have (a settings file may hold anything).
    pub fn rate(&self) -> u32 {
        if RATES.contains(&self.fps) { self.fps } else { 30 }
    }

    /// The mixer's channels the film's sound takes (none: a silent film).
    pub fn sound_channels(&self) -> Vec<aurora_audio::Channel> {
        use aurora_audio::Channel;
        if !self.sound {
            return Vec::new();
        }
        let mut channels = vec![Channel::Ambient, Channel::Sfx, Channel::Music, Channel::Media];
        if self.ui_sounds {
            channels.push(Channel::Ui);
        }
        if self.voice {
            channels.push(Channel::Voice);
        }
        channels
    }
}

/// The part of a frame of `size` the film keeps: the largest picture of
/// `aspect` (width over height; `None`: the whole frame) centred in it, its
/// sides even when the format needs them so. Width, height, left, top.
pub fn picture(size: (u32, u32), aspect: Option<f32>, format: Format) -> [u32; 4] {
    let (width, height) = (size.0.max(2), size.1.max(2));
    let (mut w, mut h) = match aspect {
        Some(aspect) if aspect.is_finite() && aspect > 0.0 => {
            if width as f32 / height as f32 > aspect {
                (((height as f32 * aspect).round() as u32).clamp(2, width), height)
            } else {
                (width, ((width as f32 / aspect).round() as u32).clamp(2, height))
            }
        }
        _ => (width, height),
    };
    let (even_width, even_height) = format.even();
    if even_width {
        w -= w % 2;
    }
    if even_height {
        h -= h % 2;
    }
    [w, h, (width - w) / 2, (height - h) / 2]
}

/// The arguments ffmpeg is run with: raw RGBA frames of `size` on its
/// standard input at `fps`, cut to `crop` (`picture`), encoded by `encoder`
/// into `out`, no sound.
pub fn arguments(encoder: &str, format: Format, quality: Quality, size: (u32, u32), crop: [u32; 4], fps: u32, out: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(format!("{}x{}", size.0, size.1).into());
    for arg in ["-framerate", &fps.to_string(), "-i", "pipe:0", "-an"] {
        args.push(arg.into());
    }
    if crop != [size.0, size.1, 0, 0] {
        args.push("-vf".into());
        args.push(format!("crop={}:{}:{}:{}", crop[0], crop[1], crop[2], crop[3]).into());
    }
    let level = |light: &'static str, good: &'static str, best: &'static str| match quality {
        Quality::Light => light,
        Quality::Good => good,
        Quality::Best => best,
    };
    let codec: Vec<&str> = match (encoder, format) {
        ("libx264", _) => vec![
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            level("26", "20", "14"),
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ],
        ("h264_mf" | "libopenh264", _) => vec!["-c:v", encoder, "-b:v", level("6M", "14M", "40M"), "-pix_fmt", "yuv420p"],
        ("mpeg4", _) => vec!["-c:v", "mpeg4", "-q:v", level("8", "4", "2"), "-pix_fmt", "yuv420p"],
        (_, Format::ProResAlpha) => vec!["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le"],
        (_, Format::ProRes) => vec!["-c:v", encoder, "-profile:v", level("1", "2", "3"), "-pix_fmt", "yuv422p10le"],
        (_, Format::Webm) => vec![
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-row-mt",
            "1",
            "-crf",
            level("40", "32", "24"),
            "-b:v",
            "0",
            "-pix_fmt",
            "yuv420p",
        ],
        _ => vec!["-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgr0"],
    };
    args.extend(codec.into_iter().map(OsString::from));
    args.push(out.as_os_str().to_owned());
    args
}

/// The video encoders an ffmpeg lists (`ffmpeg -encoders`): after the line
/// of dashes, each line is its flags (V for video), the encoder's name, then
/// what it is.
pub fn parse_encoders(listing: &str) -> Vec<String> {
    encoders_of(listing, 'V')
}

/// The audio encoders of the same listing (flags starting with A).
pub fn parse_audio_encoders(listing: &str) -> Vec<String> {
    encoders_of(listing, 'A')
}

fn encoders_of(listing: &str, kind: char) -> Vec<String> {
    listing
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("---"))
        .skip(1)
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let flags = words.next()?;
            flags.starts_with(kind).then(|| words.next().map(str::to_owned)).flatten()
        })
        .collect()
}

/// The arguments of the pass that gives a film its sound: the picture
/// `video` copied as it is, the sound `audio` (raw 32-bit float stereo at
/// 48 kHz, `film_sound.rs`) encoded by `encoder`, both into `out`. The two
/// last as long as each other (the sound is padded to the picture),
/// `-shortest` only trims a rounding.
pub fn mux_arguments(video: &Path, audio: &Path, encoder: &str, out: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-y", "-i"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push(video.as_os_str().to_owned());
    for arg in ["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"] {
        args.push(arg.into());
    }
    args.push(audio.as_os_str().to_owned());
    for arg in ["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", encoder] {
        args.push(arg.into());
    }
    let rate: &[&str] = match encoder {
        "aac" | "libfdk_aac" => &["-b:a", "192k"],
        "libopus" | "opus" => &["-b:a", "160k"],
        _ => &[],
    };
    args.extend(rate.iter().map(OsString::from));
    if encoder == "opus" {
        // (ffmpeg's own Opus encoder is still marked experimental)
        args.extend(["-strict", "-2"].map(OsString::from));
    }
    args.push("-shortest".into());
    args.push(out.as_os_str().to_owned());
    args
}

/// Run ffmpeg with `args` to its end (off the frame thread): what it said
/// of a failure.
pub fn run_to_end(program: &Path, args: &[OsString]) -> Result<(), String> {
    let output = quiet(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(head(&String::from_utf8_lossy(&output.stderr)).to_owned())
    }
}

/// The ffmpeg program's file name on this system.
fn program() -> &'static str {
    if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }
}

/// The ffmpeg found in the folders of a PATH (`std::env::var_os("PATH")`).
pub fn find_on_path(path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|folder| folder.join(program()))
        .find(|file| file.is_file())
}

/// A command that opens no console window of its own.
#[cfg(windows)]
fn quiet(program: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}

/// A command as it is: only Windows opens a console window for one.
#[cfg(not(windows))]
fn quiet(program: &Path) -> Command {
    Command::new(program)
}

/// An ffmpeg that answered, and the video encoders it has.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub path: PathBuf,
    encoders: Vec<String>,
    /// Its audio encoders, for the film's sound.
    audio: Vec<String>,
}

impl Tool {
    /// Ask the program at `path` for its encoders. Runs it: off the frame
    /// thread. The error is what could not be done, for the log.
    pub fn probe(path: &Path) -> Result<Tool, String> {
        let output = quiet(path)
            .args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !output.status.success() {
            return Err(format!("{}: {}", path.display(), output.status));
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        let encoders = parse_encoders(&listing);
        if encoders.is_empty() {
            return Err(format!("{}: no video encoder listed", path.display()));
        }
        Ok(Tool {
            path: path.to_owned(),
            encoders,
            audio: parse_audio_encoders(&listing),
        })
    }

    /// The encoder this ffmpeg writes the sound of a `format` film with;
    /// `None` when it has none (the film is then silent).
    pub fn audio_encoder(&self, format: Format) -> Option<&'static str> {
        format
            .audio_encoders()
            .iter()
            .copied()
            .find(|name| self.audio.iter().any(|have| have == name))
    }

    /// The encoder this ffmpeg writes `format` with; `None` when it has none.
    pub fn encoder(&self, format: Format) -> Option<&'static str> {
        format
            .encoders()
            .iter()
            .copied()
            .find(|name| self.encoders.iter().any(|have| have == name))
    }

    #[cfg(test)]
    pub fn with(encoders: &[&str]) -> Tool {
        Tool {
            path: PathBuf::from(program()),
            encoders: encoders.iter().map(|name| (*name).to_owned()).collect(),
            audio: vec!["aac".to_owned(), "pcm_s16le".to_owned(), "flac".to_owned()],
        }
    }
}

/// When the frames of the film are due: one every 1 / fps second since the
/// recording began.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    fps: u32,
    /// Frames written so far, repeats included.
    written: u64,
    /// A long stall shortens the film (`MAX_REPEAT_SECONDS`); not for a film with
    /// sound, whose picture must last what its sound lasts.
    capped: bool,
}

impl Pace {
    pub fn new(fps: u32) -> Pace {
        Pace {
            fps: fps.max(1),
            written: 0,
            capped: true,
        }
    }

    /// A pace that owes every frame, however long the stall (a film with
    /// sound: its picture and its sound keep one clock).
    pub fn uncapped(fps: u32) -> Pace {
        Pace {
            capped: false,
            ..Pace::new(fps)
        }
    }

    /// How many frames the film is short of, `elapsed` seconds in: 0 when
    /// none is due yet, more than 1 when the viewer fell behind.
    pub fn due(&self, elapsed: f64) -> u32 {
        let wanted = (elapsed.max(0.0) * f64::from(self.fps)).floor() as u64 + 1;
        let owed = wanted.saturating_sub(self.written);
        if self.capped {
            owed.min(u64::from(MAX_REPEAT_SECONDS * self.fps)) as u32
        } else {
            owed.min(u64::from(u32::MAX)) as u32
        }
    }

    pub fn wrote(&mut self, frames: u32) {
        self.written += u64::from(frames);
    }

    pub fn written(&self) -> u64 {
        self.written
    }
}

/// Where the frames go.
#[derive(Debug, Clone, PartialEq)]
pub enum Sink {
    /// An ffmpeg run with these arguments, the frames on its standard input.
    Ffmpeg(PathBuf, Vec<OsString>),
    /// A file of the raw frames one after the other: the demo's, to check
    /// what is captured without an encoder.
    Raw(PathBuf),
}

struct Frame {
    pixels: Vec<u8>,
    repeat: u32,
}

/// What the writer did: the frames written (repeats included), or why the
/// film could not be written.
type Outcome = Result<u64, String>;

fn write_frames(frames: &Receiver<Frame>, mut out: impl Write) -> Outcome {
    let mut written = 0;
    for frame in frames {
        for _ in 0..frame.repeat {
            out.write_all(&frame.pixels).map_err(|e| e.to_string())?;
            written += 1;
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(written)
}

fn head(text: &str) -> &str {
    let text = text.trim();
    let mut end = text.len().min(ERROR_HEAD);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end()
}

/// The writer thread's work: the frames into the sink until the recording
/// ends, then the encoder's own end.
fn run(sink: Sink, frames: &Receiver<Frame>) -> Outcome {
    match sink {
        Sink::Raw(path) => {
            let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            write_frames(frames, std::io::BufWriter::new(file))
        }
        Sink::Ffmpeg(program, args) => {
            let mut child = quiet(&program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("{}: {e}", program.display()))?;
            let written = match child.stdin.take() {
                Some(input) => write_frames(frames, input),
                None => Err("no input".to_owned()),
            };
            // (its input closed above: it finishes the file and exits)
            let output = child.wait_with_output().map_err(|e| e.to_string())?;
            let said = String::from_utf8_lossy(&output.stderr);
            match (written, output.status.success()) {
                (Ok(written), true) => Ok(written),
                // what ffmpeg says of a failure tells more than the broken pipe
                (Err(pipe), _) if said.trim().is_empty() => Err(pipe),
                _ => Err(head(&said).to_owned()),
            }
        }
    }
}

/// A film being recorded.
pub struct Recording {
    frames: SyncSender<Frame>,
    writer: JoinHandle<Outcome>,
    started: Instant,
    pace: Pace,
    size: (u32, u32),
    pub out: PathBuf,
    /// Frames skipped because the writer's queue was full.
    pub skipped: u64,
}

impl Recording {
    /// Start the writer for frames of `size` at `fps`; `out` is the file the
    /// sink writes, to show afterwards.
    pub fn start(sink: Sink, size: (u32, u32), fps: u32, out: PathBuf) -> Recording {
        Recording::start_with(sink, size, Pace::new(fps), out)
    }

    /// The same with its own pace (`Pace::uncapped` for a film with sound;
    /// an offline render's frames are given with `offer`, the pace unused).
    pub fn start_with(sink: Sink, size: (u32, u32), pace: Pace, out: PathBuf) -> Recording {
        let (frames, queue) = sync_channel(QUEUE);
        Recording {
            frames,
            writer: std::thread::spawn(move || run(sink, &queue)),
            started: Instant::now(),
            pace,
            size,
            out,
            skipped: 0,
        }
    }

    /// The next frame of an offline render (written once, not paced):
    /// given back when the writer is behind, to be offered again later (the
    /// render waits for it; it is not real time). A frame of another size
    /// is dropped: `None`, and the writer `failed` soon after it ends.
    pub fn offer(&mut self, pixels: Vec<u8>) -> Option<Vec<u8>> {
        if pixels.len() != self.size.0 as usize * self.size.1 as usize * 4 {
            log::warn!("director: a frame of the wrong size for the render, dropped");
            return None;
        }
        match self.frames.try_send(Frame { pixels, repeat: 1 }) {
            Ok(()) => {
                self.pace.wrote(1);
                None
            }
            Err(TrySendError::Full(frame)) => Some(frame.pixels),
            Err(TrySendError::Disconnected(_)) => None,
        }
    }

    /// Seconds since the recording began.
    pub fn elapsed(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.started).as_secs_f64()
    }

    /// A frame of the film is due: the view is to be captured now.
    pub fn wants_frame(&self, now: Instant) -> bool {
        self.pace.due(self.elapsed(now)) > 0
    }

    /// The writer ended on its own: the encoder failed, or the disk.
    pub fn failed(&self) -> bool {
        self.writer.is_finished()
    }

    /// The view captured for a frame that was due. False when its size is
    /// not the film's (the window was resized): the recording must end.
    pub fn frame(&mut self, now: Instant, size: (u32, u32), pixels: Vec<u8>) -> bool {
        if size != self.size || pixels.len() != size.0 as usize * size.1 as usize * 4 {
            return false;
        }
        let repeat = self.pace.due(self.elapsed(now));
        if repeat == 0 {
            return true;
        }
        match self.frames.try_send(Frame { pixels, repeat }) {
            Ok(()) => self.pace.wrote(repeat),
            // the writer is behind: this frame's time is given to the next
            Err(TrySendError::Full(_)) => self.skipped += 1,
            // the writer ended: `failed` says so
            Err(TrySendError::Disconnected(_)) => {}
        }
        true
    }

    /// No more frames: the writer finishes the file. Poll the result with
    /// `Finishing::done`.
    pub fn finish(self) -> Finishing {
        Finishing {
            writer: Some(self.writer),
            out: self.out,
            frames: self.pace.written(),
        }
    }
}

/// A film whose file is being finished.
pub struct Finishing {
    writer: Option<JoinHandle<Outcome>>,
    pub out: PathBuf,
    /// Frames handed to the writer.
    pub frames: u64,
}

impl Finishing {
    /// The film is written (its file and its frames), or could not be (why);
    /// `None` while the encoder still works.
    pub fn done(&mut self) -> Option<Result<(PathBuf, u64), String>> {
        if !self.writer.as_ref()?.is_finished() {
            return None;
        }
        let outcome = self.writer.take()?.join().unwrap_or_else(|_| Err("the writer stopped".to_owned()));
        Some(outcome.map(|frames| (self.out.clone(), frames)))
    }
}

/// The folder the films go to: `<Videos>/Aurora Viewer`, or the photos'
/// folder when the system has no videos folder.
pub fn default_folder() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.video_dir().map(Path::to_path_buf))
        .map_or_else(crate::snapshot::default_folder, |videos| videos.join("Aurora Viewer"))
}

/// A file of `folder` no film has yet, named after `stamp` (the date and
/// time, as the photos').
pub fn free_path(folder: &Path, stamp: &str, format: Format) -> PathBuf {
    (0u32..)
        .map(|n| {
            let suffix = if n == 0 { String::new() } else { format!("_{n}") };
            folder.join(format!("Aurora_{stamp}{suffix}.{}", format.extension()))
        })
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(format!("Aurora_{stamp}.{}", format.extension())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const LISTING: &str = "Encoders:
 V..... = Video
 A..... = Audio
 ------
 V....D ffv1                 FFmpeg video codec #1
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V....D h264_mf              H264 via MediaFoundation (codec h264)
 V.S..D prores_ks            Apple ProRes (iCodec Pro) (codec prores)
 A....D aac                  AAC (Advanced Audio Coding)
 S..... srt                  SubRip subtitle (codec subrip)
";

    #[test]
    fn the_encoders_of_an_ffmpeg_are_read_from_its_listing() {
        assert_eq!(parse_encoders(LISTING), ["ffv1", "libx264", "h264_mf", "prores_ks"]);
        assert!(parse_encoders("not a listing").is_empty());
        assert!(parse_encoders("").is_empty());
    }

    #[test]
    fn a_format_is_written_by_the_best_encoder_the_tool_has() {
        let tool = Tool::with(&["ffv1", "h264_mf", "libx264", "prores_ks"]);
        assert_eq!(tool.encoder(Format::Mp4), Some("libx264"));
        assert_eq!(tool.encoder(Format::ProResAlpha), Some("prores_ks"));
        assert_eq!(tool.encoder(Format::Lossless), Some("ffv1"));
        // a build without it does not offer the format
        assert_eq!(tool.encoder(Format::Webm), None);
        // a build without libx264 still writes an MP4 through the system's
        assert_eq!(Tool::with(&["h264_mf"]).encoder(Format::Mp4), Some("h264_mf"));
        for format in Format::ALL {
            assert!(!format.encoders().is_empty() && !format.extension().is_empty());
        }
        assert!(Format::ProResAlpha.alpha() && !Format::Mp4.alpha());
    }

    #[test]
    fn the_picture_is_the_format_centred_in_the_frame() {
        // the whole frame, its sides made even for an MP4
        assert_eq!(picture((1683, 900), None, Format::Mp4), [1682, 900, 0, 0]);
        // ProRes 422 keeps an odd height, not an odd width; 4444 keeps both
        assert_eq!(picture((1683, 901), None, Format::ProRes), [1682, 901, 0, 0]);
        assert_eq!(picture((1683, 901), None, Format::ProResAlpha), [1683, 901, 0, 0]);
        assert_eq!(picture((1683, 901), None, Format::Lossless), [1683, 901, 0, 0]);
        // scope in a 16:9 frame: cut above and below
        assert_eq!(picture((1920, 1080), Some(2.39), Format::Mp4), [1920, 802, 0, 139]);
        // vertical: cut on the sides
        assert_eq!(picture((1920, 1080), Some(9.0 / 16.0), Format::Mp4), [608, 1080, 656, 0]);
        // nothing breaks on a tiny or a wrong value
        assert_eq!(picture((0, 0), Some(f32::NAN), Format::Mp4), [2, 2, 0, 0]);
        let [w, h, x, y] = picture((300, 200), Some(1000.0), Format::Webm);
        assert!(w == 300 && h >= 2 && x == 0 && y + h <= 200);
    }

    #[test]
    fn ffmpeg_is_asked_for_raw_frames_in_and_the_format_out() {
        let text = |args: Vec<OsString>| args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        let out = Path::new("film.mp4");
        let mp4 = text(arguments(
            "libx264",
            Format::Mp4,
            Quality::Good,
            (1920, 1080),
            [1920, 1080, 0, 0],
            30,
            out,
        ));
        assert!(mp4.starts_with(
            "-hide_banner -loglevel error -y -f rawvideo -pixel_format rgba -video_size 1920x1080 -framerate 30 -i pipe:0 -an"
        ));
        assert!(mp4.contains("-c:v libx264") && mp4.contains("-crf 20") && mp4.contains("-pix_fmt yuv420p"));
        assert!(!mp4.contains("-vf"), "the whole frame: nothing to cut");
        assert!(mp4.ends_with("film.mp4"));
        // a format's picture is cut out of the frame
        let scope = text(arguments(
            "libx264",
            Format::Mp4,
            Quality::Best,
            (1920, 1080),
            [1920, 802, 0, 139],
            60,
            out,
        ));
        assert!(scope.contains("-framerate 60") && scope.contains("-vf crop=1920:802:0:139") && scope.contains("-crf 14"));
        // the transparency is kept by ProRes 4444
        let alpha = text(arguments(
            "prores_ks",
            Format::ProResAlpha,
            Quality::Good,
            (800, 600),
            [800, 600, 0, 0],
            25,
            Path::new("a.mov"),
        ));
        assert!(alpha.contains("-c:v prores_ks -profile:v 4444 -pix_fmt yuva444p10le"));
        let system = text(arguments(
            "h264_mf",
            Format::Mp4,
            Quality::Light,
            (800, 600),
            [800, 600, 0, 0],
            24,
            out,
        ));
        assert!(system.contains("-c:v h264_mf -b:v 6M"));
        let lossless = text(arguments(
            "ffv1",
            Format::Lossless,
            Quality::Good,
            (800, 600),
            [800, 600, 0, 0],
            24,
            Path::new("a.mkv"),
        ));
        assert!(lossless.contains("-c:v ffv1"));
        let webm = text(arguments(
            "libvpx-vp9",
            Format::Webm,
            Quality::Good,
            (800, 600),
            [800, 600, 0, 0],
            24,
            Path::new("a.webm"),
        ));
        assert!(webm.contains("-c:v libvpx-vp9") && webm.contains("-crf 32"));
        let prores = text(arguments(
            "prores",
            Format::ProRes,
            Quality::Best,
            (800, 600),
            [800, 600, 0, 0],
            24,
            Path::new("a.mov"),
        ));
        assert!(prores.contains("-c:v prores -profile:v 3"));
    }

    #[test]
    fn a_frame_is_due_every_period_and_late_ones_are_repeated() {
        let mut pace = Pace::new(30);
        assert_eq!(pace.due(0.0), 1, "the first frame at once");
        pace.wrote(1);
        assert_eq!(pace.due(0.01), 0);
        assert_eq!(pace.due(1.0 / 30.0 + 1e-6), 1);
        pace.wrote(1);
        // a hitch of a third of a second: ten frames are owed
        assert_eq!(pace.due(0.4), 11);
        pace.wrote(11);
        assert_eq!(pace.written(), 13);
        assert_eq!(pace.due(0.4), 0);
        // a long stall is not a frozen film without end
        assert_eq!(pace.due(1000.0), MAX_REPEAT_SECONDS * 30);
        assert_eq!(Pace::new(0).due(-5.0), 1);
    }

    #[test]
    fn the_ffmpeg_of_the_path_is_found() {
        let root = std::env::temp_dir().join(format!("aurora-recorder-{}", std::process::id()));
        let (empty, with) = (root.join("empty"), root.join("with"));
        std::fs::create_dir_all(&empty).expect("folder");
        std::fs::create_dir_all(&with).expect("folder");
        let path = std::env::join_paths([&empty, &with]).expect("path");
        assert_eq!(find_on_path(&path), None);
        std::fs::write(with.join(program()), b"").expect("file");
        assert_eq!(find_on_path(&path), Some(with.join(program())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_frames_reach_the_file_each_as_many_times_as_asked() {
        let root = std::env::temp_dir().join(format!("aurora-recording-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("folder");
        let out = root.join("film.rgba");
        let mut recording = Recording::start(Sink::Raw(out.clone()), (2, 1), 30, out.clone());
        let start = Instant::now();
        assert!(recording.wants_frame(start));
        assert!(recording.frame(start, (2, 1), vec![1; 8]));
        // a frame of another size ends the recording
        assert!(!recording.frame(start, (3, 1), vec![0; 12]));
        assert!(!recording.frame(start, (2, 1), vec![0; 7]));
        let mut finishing = recording.finish();
        let done = loop {
            if let Some(done) = finishing.done() {
                break done;
            }
            std::thread::yield_now();
        };
        let (file, frames) = done.expect("written");
        assert_eq!(file, out);
        assert!(frames >= 1);
        let bytes = std::fs::read(&out).expect("file");
        assert_eq!(bytes.len() as u64, frames * 8);
        assert!(bytes.iter().all(|b| *b == 1));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_program_that_is_not_there_fails_with_its_name() {
        let missing = std::env::temp_dir().join("aurora-no-such-ffmpeg").join(program());
        assert!(Tool::probe(&missing).expect_err("missing").contains("aurora-no-such-ffmpeg"));
        let mut finishing = Recording::start(Sink::Ffmpeg(missing.clone(), Vec::new()), (2, 2), 30, PathBuf::from("x.mp4")).finish();
        let done = loop {
            if let Some(done) = finishing.done() {
                break done;
            }
            std::thread::yield_now();
        };
        assert!(done.expect_err("missing").contains("aurora-no-such-ffmpeg"));
    }

    #[test]
    fn a_film_gets_a_name_no_file_has() {
        let root = std::env::temp_dir().join(format!("aurora-films-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("folder");
        let first = free_path(&root, "2026-11-15_16-00-00", Format::Mp4);
        assert_eq!(first, root.join("Aurora_2026-11-15_16-00-00.mp4"));
        std::fs::write(&first, b"").expect("file");
        assert_eq!(
            free_path(&root, "2026-11-15_16-00-00", Format::Mp4),
            root.join("Aurora_2026-11-15_16-00-00_1.mp4")
        );
        assert_eq!(
            free_path(&root, "2026-11-15_16-00-00", Format::ProRes),
            root.join("Aurora_2026-11-15_16-00-00.mov")
        );
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(Options::default().rate(), 30);
        let odd = Options {
            fps: 7,
            ..Default::default()
        };
        assert_eq!(odd.rate(), 30);
        assert_eq!(head("  short  "), "short");
        // (never cut inside a character)
        assert_eq!(head(&"é".repeat(ERROR_HEAD)).len(), ERROR_HEAD);
        assert_eq!(head(&format!("a{}", "é".repeat(ERROR_HEAD))).len(), ERROR_HEAD - 1);
        assert!(head(&format!("the cause\n{}", "x".repeat(2 * ERROR_HEAD))).starts_with("the cause\n"));
    }

    /// The frames of a test film.
    const TEST_FRAMES: u32 = 12;

    /// The color the left half of a test film's frame has.
    fn test_color(index: u32) -> [u8; 3] {
        [(40 + 15 * index) as u8, 90, (220 - 12 * index) as u8]
    }

    /// A frame of a test film: its left half one color, opaque, that changes
    /// with the frame; its right half empty, as the isolation's background.
    fn test_frame(size: (u32, u32), index: u32) -> Vec<u8> {
        let [r, g, b] = test_color(index);
        let (filmed, empty) = ([r, g, b, 255], [0; 4]);
        let mut pixels = Vec::with_capacity(size.0 as usize * size.1 as usize * 4);
        for _ in 0..size.1 {
            for x in 0..size.0 {
                pixels.extend_from_slice(if x < size.0 / 2 { &filmed } else { &empty });
            }
        }
        pixels
    }

    /// A film read back by the ffmpeg that wrote it: its frames, as RGBA.
    fn read_back(tool: &Tool, film: &Path) -> Result<Vec<u8>, String> {
        let output = quiet(&tool.path)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(film)
            .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(head(&String::from_utf8_lossy(&output.stderr)).to_owned())
        }
    }

    /// Record a test film as the Director does, then read it back: what the
    /// file holds, or what went wrong.
    fn test_film(
        tool: &Tool,
        out: &Path,
        encoder: &str,
        format: Format,
        quality: Quality,
        size: (u32, u32),
        aspect: Option<f32>,
    ) -> Result<String, String> {
        let crop = picture(size, aspect, format);
        let args = arguments(encoder, format, quality, size, crop, 24, out);
        let mut recording = Recording::start(Sink::Ffmpeg(tool.path.clone(), args), size, 24, out.to_owned());
        let started = recording.started;
        for index in 0..TEST_FRAMES {
            // the middle of that frame's period, on the film's clock
            let at = started + Duration::from_secs_f64((f64::from(index) + 0.5) / 24.0);
            let pixels = test_frame(size, index);
            // (a full queue skips the frame: it is offered again)
            loop {
                let skipped = recording.skipped;
                if !recording.frame(at, size, pixels.clone()) {
                    return Err("a frame was refused".to_owned());
                }
                if recording.skipped == skipped {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let mut finishing = recording.finish();
        let (file, frames) = loop {
            if let Some(done) = finishing.done() {
                break done?;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        if frames != u64::from(TEST_FRAMES) {
            return Err(format!("{frames} frames written"));
        }
        let bytes = std::fs::metadata(&file).map_err(|e| e.to_string())?.len();
        let back = read_back(tool, &file)?;
        let (w, h) = (crop[0] as usize, crop[1] as usize);
        let frame = w * h * 4;
        if back.len() != frame * TEST_FRAMES as usize {
            return Err(format!("{} bytes read back: not {TEST_FRAMES} frames of {w}x{h}", back.len()));
        }
        // lossless keeps the colors as they are, the others near them
        let slack = if format == Format::Lossless { 0 } else { 14 };
        for index in [0, TEST_FRAMES - 1] {
            let pixel = |x: usize| {
                let at = index as usize * frame + ((h / 2) * w + x) * 4;
                [back[at], back[at + 1], back[at + 2], back[at + 3]]
            };
            let (left, right) = (pixel(w / 4), pixel(w * 3 / 4));
            let color = test_color(index);
            let near = (0..3).all(|c| left[c].abs_diff(color[c]) <= slack);
            if !near || left[3] != 255 {
                return Err(format!("frame {index}: {left:?} where {color:?} was filmed"));
            }
            let empty = if format.alpha() { 0 } else { 255 };
            if right[3] != empty {
                return Err(format!("frame {index}: alpha {} on the empty side, not {empty}", right[3]));
            }
        }
        Ok(format!("{w}x{h}, {bytes} bytes"))
    }

    /// Every format, through each encoder the ffmpeg of this computer has
    /// for it, at every quality, read back by that ffmpeg: the frames are
    /// all there, of the picture's size, with their colors and, for the
    /// format that keeps it, their transparency. Run by hand:
    /// `cargo test --workspace the_ffmpeg_found -- --ignored --nocapture`.
    #[test]
    #[ignore = "runs the ffmpeg found on the PATH"]
    fn the_ffmpeg_found_writes_every_format_it_has() {
        let path = std::env::var_os("PATH")
            .and_then(|path| find_on_path(&path))
            .expect("an ffmpeg on the PATH");
        let tool = Tool::probe(&path).expect("ffmpeg answers");
        let root = std::env::temp_dir().join(format!("aurora-ffmpeg-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("folder");
        // odd sides, as a window has: each format cuts what it cannot keep
        let all = [Quality::Light, Quality::Good, Quality::Best];
        let shots: [((u32, u32), Option<f32>, &[Quality]); 3] = [
            ((323, 181), None, &all),
            ((323, 181), Some(2.39), &[Quality::Good]),
            ((1683, 901), None, &[Quality::Good]),
        ];
        let (mut films, mut failures) = (0, Vec::new());
        for (size, aspect, qualities) in shots {
            for format in Format::ALL {
                let have = format
                    .encoders()
                    .iter()
                    .copied()
                    .filter(|name| tool.encoders.iter().any(|e| e == name));
                for encoder in have {
                    for quality in qualities.iter().copied() {
                        let cut = aspect.map_or_else(|| "frame".to_owned(), |aspect| aspect.to_string());
                        let name = format!("{format:?}-{encoder}-{quality:?}-{}x{}-{cut}", size.0, size.1);
                        let out = root.join(format!("{name}.{}", format.extension()));
                        films += 1;
                        match test_film(&tool, &out, encoder, format, quality, size, aspect) {
                            Ok(what) => eprintln!("ok    {name}: {what}"),
                            Err(why) => {
                                eprintln!("FAIL  {name}: {why}");
                                failures.push(format!("{name}: {why}"));
                            }
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(films > 0, "this ffmpeg has none of the encoders");
        assert!(
            failures.is_empty(),
            "{} of {films} films failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
    #[test]
    fn the_sound_is_written_by_the_container_s_encoder_and_muxed_beside_the_picture() {
        let listing = format!("{LISTING} A....D libopus              libopus Opus\n A....D flac                 FLAC\n");
        assert_eq!(parse_audio_encoders(&listing), ["aac", "libopus", "flac"]);
        let tool = Tool {
            path: PathBuf::from("ffmpeg"),
            encoders: parse_encoders(&listing),
            audio: parse_audio_encoders(&listing),
        };
        assert_eq!(tool.audio_encoder(Format::Mp4), Some("aac"));
        assert_eq!(tool.audio_encoder(Format::Webm), Some("libopus"));
        assert_eq!(tool.audio_encoder(Format::Lossless), Some("flac"));
        // no PCM encoder listed: a ProRes film stays silent
        assert_eq!(tool.audio_encoder(Format::ProRes), None);
        // FLAC missing: Opus in the MKV
        let opus = Tool {
            audio: vec!["libopus".to_owned()],
            ..tool.clone()
        };
        assert_eq!(opus.audio_encoder(Format::Lossless), Some("libopus"));
        let text = |args: Vec<OsString>| args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        let (video, audio) = (Path::new("v.mp4"), Path::new("a.f32"));
        let aac = text(mux_arguments(video, audio, "aac", Path::new("film.mp4")));
        assert_eq!(
            aac,
            "-hide_banner -loglevel error -y -i v.mp4 -f f32le -ar 48000 -ac 2 -i a.f32 -map 0:v:0 -map 1:a:0 -c:v copy -c:a aac -b:a 192k -shortest film.mp4"
        );
        let pcm = text(mux_arguments(video, audio, "pcm_s16le", Path::new("film.mov")));
        assert!(pcm.contains("-c:a pcm_s16le -shortest"), "no bit rate for PCM");
        let native = text(mux_arguments(video, audio, "opus", Path::new("film.webm")));
        assert!(native.contains("-b:a 160k -strict -2"), "the native Opus is experimental");
        assert!(!text(mux_arguments(video, audio, "libopus", Path::new("f.webm"))).contains("-strict"));
    }

    #[test]
    fn a_film_with_sound_owes_every_frame_and_an_offline_render_writes_each_once() {
        // a stall of five seconds: the film with sound repeats past the cap
        let mut capped = Pace::new(60);
        let mut uncapped = Pace::uncapped(60);
        capped.wrote(1);
        uncapped.wrote(1);
        assert_eq!(capped.due(5.0), MAX_REPEAT_SECONDS * 60);
        assert_eq!(uncapped.due(5.0), 300);
        uncapped.wrote(300);
        assert_eq!(uncapped.written(), 301);
        assert_eq!(uncapped.due(5.0), 0, "in step again");
        // the render: frames offered one by one, each written once
        let out = std::env::temp_dir().join(format!("aurora-offer-{}.raw", std::process::id()));
        let mut film = Recording::start_with(Sink::Raw(out.clone()), (2, 2), Pace::new(30), out.clone());
        let mut offered = 0;
        while offered < 8 {
            match film.offer(vec![offered as u8; 16]) {
                None => offered += 1,
                Some(back) => {
                    assert_eq!(back.len(), 16, "given back whole");
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        assert_eq!(film.offer(vec![0; 3]), None, "the wrong size is dropped");
        let mut finishing = film.finish();
        let done = loop {
            if let Some(done) = finishing.done() {
                break done;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(done.expect("written").1, 8);
        let bytes = std::fs::read(&out).expect("the frames");
        let _ = std::fs::remove_file(&out);
        assert_eq!(bytes.len(), 8 * 16);
        assert!(
            bytes.chunks(16).enumerate().all(|(i, frame)| frame.iter().all(|&b| b == i as u8)),
            "in order, none twice"
        );
    }
}
