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
pub const RATES: [u32; 5] = [24, 25, 30, 50, 60];
/// Frames waiting for the writer: past this, a frame is skipped and the next
/// one written is repeated in its place.
const QUEUE: usize = 3;
/// A frame is never written more than this many times in a row (two seconds
/// of a 60 fps film): past it the film is shortened rather than frozen.
const MAX_REPEAT: u32 = 120;
/// The last lines of what ffmpeg wrote, kept for the message of a failure.
const ERROR_TAIL: usize = 600;

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

    /// Its pixels are stored 4:2:0: the picture's sides must be even.
    fn even(self) -> bool {
        matches!(self, Format::Mp4 | Format::Webm)
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
}

impl Default for Options {
    fn default() -> Self {
        Options {
            format: Format::Mp4,
            quality: Quality::Good,
            fps: 30,
            ffmpeg: None,
        }
    }
}

impl Options {
    /// A rate the film can have (a settings file may hold anything).
    pub fn rate(&self) -> u32 {
        if RATES.contains(&self.fps) { self.fps } else { 30 }
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
    if format.even() {
        w -= w % 2;
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
    listing
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("---"))
        .skip(1)
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let flags = words.next()?;
            flags.starts_with('V').then(|| words.next().map(str::to_owned)).flatten()
        })
        .collect()
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
fn quiet(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}

/// An ffmpeg that answered, and the video encoders it has.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub path: PathBuf,
    encoders: Vec<String>,
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
        let encoders = parse_encoders(&String::from_utf8_lossy(&output.stdout));
        if encoders.is_empty() {
            return Err(format!("{}: no video encoder listed", path.display()));
        }
        Ok(Tool {
            path: path.to_owned(),
            encoders,
        })
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
}

impl Pace {
    pub fn new(fps: u32) -> Pace {
        Pace {
            fps: fps.max(1),
            written: 0,
        }
    }

    /// How many frames the film is short of, `elapsed` seconds in: 0 when
    /// none is due yet, more than 1 when the viewer fell behind.
    pub fn due(&self, elapsed: f64) -> u32 {
        let wanted = (elapsed.max(0.0) * f64::from(self.fps)).floor() as u64 + 1;
        wanted.saturating_sub(self.written).min(u64::from(MAX_REPEAT)) as u32
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

fn tail(text: &str) -> &str {
    let text = text.trim();
    let mut start = text.len().saturating_sub(ERROR_TAIL);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
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
                _ => Err(tail(&said).to_owned()),
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
        let (frames, queue) = sync_channel(QUEUE);
        Recording {
            frames,
            writer: std::thread::spawn(move || run(sink, &queue)),
            started: Instant::now(),
            pace: Pace::new(fps),
            size,
            out,
            skipped: 0,
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
        assert_eq!(picture((1683, 901), None, Format::ProRes), [1683, 901, 0, 0]);
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
        assert_eq!(pace.due(1000.0), MAX_REPEAT);
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
        assert_eq!(tail("  short  "), "short");
        assert_eq!(tail(&"é".repeat(ERROR_TAIL)).len(), ERROR_TAIL);
    }
}
