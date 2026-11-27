//! The Director's offline render: the track filmed not in real time, one
//! frame after another, each drawn whole at the film's own size, whatever
//! the window's. Aurora's own (the Legacy Viewer records no film).
//!
//! A live film (`recorder.rs`) keeps the clock: a frame the viewer drew late
//! is written twice. A render keeps the track: frame `n` shows the track at
//! `n / fps` seconds, and nothing is repeated nor dropped however long a
//! frame takes. The world's own clock follows it (`RenderJob::plan`): the
//! frame of the film is drawn at the instant `n / fps` after the render
//! began, with `1 / fps` of time stepped since the last, and the frames the
//! window draws in between (to keep the interface and the progress alive)
//! hold the world still, as the parts of a large photo do
//! (`snapshot::Hold`). What the viewer animates follows that clock (the
//! avatars' animations, the animated textures, the particles' time, the
//! water and the sky's clock, the motion of objects between the server's
//! updates); what the server moves keeps its own real time, and the sound is
//! not filmed.
//!
//! The frames go to ffmpeg as a live film's do, or to PNG images written by
//! a few threads (`PngWriter`), no ffmpeg needed.
//!
//! With the Director's motion blur on, a frame is drawn as several
//! sub-frames over the time its shutter stays open, from `n / fps` on
//! (`RenderJob::with_shutter`): each the world at its own instant, averaged
//! by the renderer (aurora-render's shutter.rs), only the last read back.

use crate::recorder::{self, Format, Recording};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Film frames asked of the renderer and not back yet: two keep the GPU
/// busy while the last one is read.
const IN_FLIGHT: usize = 2;
/// The window is drawn at least this often during a render: the interface
/// answers, the progress moves.
const WINDOW_EVERY: Duration = Duration::from_millis(250);
/// Frames waiting for the PNG writers.
const PNG_QUEUE: usize = 4;
/// The tags of a render's frames (`aurora_render::FilmRequest::tag`): the
/// live film's have this bit clear.
pub const RENDER_TAG: u64 = 1 << 63;

/// The size a render is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resolution {
    /// The window's picture, as the live film.
    Window,
    #[default]
    Hd,
    Qhd,
    Uhd,
}

impl Resolution {
    pub const ALL: [Resolution; 4] = [Resolution::Window, Resolution::Hd, Resolution::Qhd, Resolution::Uhd];

    /// The box its picture fits in, landscape.
    fn bounds(self) -> Option<(u32, u32)> {
        match self {
            Resolution::Window => None,
            Resolution::Hd => Some((1920, 1080)),
            Resolution::Qhd => Some((2560, 1440)),
            Resolution::Uhd => Some((3840, 2160)),
        }
    }

    /// The label's number (`1080p`…), the window's has none.
    pub fn lines(self) -> Option<u32> {
        self.bounds().map(|(_, h)| h)
    }
}

/// The size of a render's frames: the picture of `aspect` (the track's
/// format; `None`: the window's proportions) as large as fits in the
/// resolution's box (turned upright for an upright picture: 1080 wide for a
/// 9:16 at 1080p), its sides even for the encoders.
pub fn render_size(resolution: Resolution, window: (u32, u32), aspect: Option<f32>, format: Format) -> (u32, u32) {
    let window_aspect = window.0.max(1) as f32 / window.1.max(1) as f32;
    let [w, h, ..] = match resolution.bounds() {
        None => recorder::picture(window, aspect, format),
        Some((long, short)) => {
            let aspect = aspect.filter(|a| a.is_finite() && *a > 0.0).unwrap_or(window_aspect);
            let bounds = if aspect < 1.0 { (short, long) } else { (long, short) };
            // (even both ways: any encoder takes it, a PNG does not mind)
            recorder::picture(bounds, Some(aspect), Format::Mp4)
        }
    };
    (w, h)
}

/// The frames of a track of `duration` seconds at `fps`: frame `n` at
/// `n / fps`, the last one on the track's end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Schedule {
    pub fps: u32,
    pub duration: f32,
}

impl Schedule {
    pub fn new(duration: f32, fps: u32) -> Schedule {
        Schedule {
            fps: fps.max(1),
            duration: if duration.is_finite() { duration.max(0.0) } else { 0.0 },
        }
    }

    pub fn count(&self) -> u64 {
        (f64::from(self.duration) * f64::from(self.fps) - 1e-6).ceil().max(0.0) as u64 + 1
    }

    /// Seconds of the track frame `n` shows.
    pub fn time(&self, n: u64) -> f32 {
        ((n as f64 / f64::from(self.fps)) as f32).min(self.duration)
    }
}

/// The file of frame `n` of `count`, named after `stem`: its number padded
/// so that the files sort in order (five digits at least, as editors and
/// ffmpeg's `%05d` expect).
pub fn png_name(stem: &str, n: u64, count: u64) -> String {
    let width = count.saturating_sub(1).to_string().len().max(5);
    format!("{stem}_{n:0width$}.png")
}

/// A folder of `parent` no render has yet, named after `stamp`.
pub fn free_folder(parent: &Path, stamp: &str) -> PathBuf {
    (0u32..)
        .map(|n| {
            let suffix = if n == 0 { String::new() } else { format!("_{n}") };
            parent.join(format!("Aurora_{stamp}{suffix}"))
        })
        .find(|path| !path.exists())
        .unwrap_or_else(|| parent.join(format!("Aurora_{stamp}")))
}

/// Seconds left to render `count - done` frames, at the pace of the `done`
/// first in `elapsed`; `None` before a pace is known.
pub fn time_left(done: u64, count: u64, elapsed: Duration) -> Option<f32> {
    (done >= 2 && done <= count).then(|| elapsed.as_secs_f32() / done as f32 * (count - done) as f32)
}

/// What PNG writers did: frames written, or why they stopped.
type Outcome = Result<u64, String>;

/// A frame for the PNG writers: its number and pixels.
type Png = (u64, Vec<u8>);

fn write_png(path: &Path, size: (u32, u32), pixels: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // (a 4K frame a second or so with the default; fast keeps the writers
    // ahead of the GPU, the files a little larger)
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| format!("{}: {e}", path.display()))?;
    writer.write_image_data(pixels).map_err(|e| format!("{}: {e}", path.display()))?;
    writer.finish().map_err(|e| format!("{}: {e}", path.display()))
}

/// The PNG images of a render, written by a few threads.
pub struct PngWriter {
    frames: SyncSender<Png>,
    writers: Vec<JoinHandle<Outcome>>,
    folder: PathBuf,
    size: (u32, u32),
    written: u64,
}

impl PngWriter {
    /// Writers for frames of `size` into `folder` (made by the caller), the
    /// files named after `stem` for a render of `count` frames.
    pub fn start(folder: PathBuf, stem: String, count: u64, size: (u32, u32)) -> PngWriter {
        let (frames, queue) = sync_channel::<Png>(PNG_QUEUE);
        let queue = Arc::new(Mutex::new(queue));
        let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
        let writers = (0..threads)
            .map(|_| {
                let (queue, folder, stem) = (queue.clone(), folder.clone(), stem.clone());
                std::thread::spawn(move || png_writer(&queue, &folder, &stem, count, size))
            })
            .collect();
        PngWriter {
            frames,
            writers,
            folder,
            size,
            written: 0,
        }
    }

    /// Frame `n`: given back while the writers are busy.
    pub fn offer(&mut self, n: u64, pixels: Vec<u8>) -> Option<Vec<u8>> {
        if pixels.len() != self.size.0 as usize * self.size.1 as usize * 4 {
            log::warn!("director: a frame of the wrong size for the render, dropped");
            return None;
        }
        match self.frames.try_send((n, pixels)) {
            Ok(()) => {
                self.written += 1;
                None
            }
            Err(TrySendError::Full((_, pixels))) => Some(pixels),
            Err(TrySendError::Disconnected(_)) => None,
        }
    }

    /// A writer stopped on its own: the disk.
    pub fn failed(&self) -> bool {
        self.writers.iter().any(JoinHandle::is_finished)
    }
}

fn png_writer(queue: &Mutex<Receiver<Png>>, folder: &Path, stem: &str, count: u64, size: (u32, u32)) -> Outcome {
    let mut written = 0;
    loop {
        // (the lock only while one frame is taken: the others write)
        let next = match queue.lock() {
            Ok(queue) => queue.recv(),
            Err(_) => return Err("the writers stopped".to_owned()),
        };
        let Ok((n, pixels)) = next else {
            return Ok(written);
        };
        write_png(&folder.join(png_name(stem, n, count)), size, &pixels)?;
        written += 1;
    }
}

/// Where a render's frames go.
pub enum Writer {
    Film(Recording),
    Png(PngWriter),
}

impl Writer {
    fn offer(&mut self, n: u64, pixels: Vec<u8>) -> Option<Vec<u8>> {
        match self {
            Writer::Film(film) => film.offer(pixels),
            Writer::Png(png) => png.offer(n, pixels),
        }
    }

    fn failed(&self) -> bool {
        match self {
            Writer::Film(film) => film.failed(),
            Writer::Png(png) => png.failed(),
        }
    }

    fn finish(self) -> Finishing {
        match self {
            Writer::Film(film) => Finishing::Film(film.finish()),
            Writer::Png(png) => {
                let PngWriter {
                    frames, writers, folder, ..
                } = png;
                // (the queue closed: each writer ends once it is empty)
                drop(frames);
                Finishing::Png(writers, folder)
            }
        }
    }
}

/// A render whose files are being finished.
pub enum Finishing {
    Film(recorder::Finishing),
    Png(Vec<JoinHandle<Outcome>>, PathBuf),
}

impl Finishing {
    /// The file (or folder) and the frames written, or why it failed;
    /// `None` while the writers still work.
    pub fn done(&mut self) -> Option<Result<(PathBuf, u64), String>> {
        match self {
            Finishing::Film(film) => film.done(),
            Finishing::Png(writers, folder) => {
                if writers.is_empty() || !writers.iter().all(JoinHandle::is_finished) {
                    return None;
                }
                let mut written = 0;
                let mut failure = None;
                for writer in writers.drain(..) {
                    match writer.join().unwrap_or_else(|_| Err("a writer stopped".to_owned())) {
                        Ok(n) => written += n,
                        Err(why) => failure = Some(why),
                    }
                }
                Some(match failure {
                    Some(why) => Err(why),
                    None => Ok((folder.clone(), written)),
                })
            }
        }
    }
}

/// What a frame of the app is, during a render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Frame `n` of the film, the track at `time`; `shutter`: the
    /// sub-frame of it this one is, when its shutter takes several.
    Film {
        n: u64,
        time: f32,
        shutter: Option<aurora_render::Shutter>,
    },
    /// A frame of the window: the world held at the last film frame's time.
    Window { time: f32 },
}

/// A render under way.
pub struct RenderJob {
    pub size: (u32, u32),
    pub schedule: Schedule,
    /// The world's instant and the shaders' clock when it began: frame `n`
    /// shows them `n / fps` later.
    base: (Instant, f32),
    /// The next frame to ask for.
    next: u64,
    /// Its next sub-frame, with a shutter.
    sub: u32,
    /// The shutter: seconds it stays open from each frame's instant, and
    /// the sub-frames drawn over that time (`with_shutter`).
    shutter: Option<(f32, u32)>,
    /// Frames asked and not back, in order.
    asked: VecDeque<u64>,
    /// Frames back and not taken by the writer yet, in order.
    backlog: VecDeque<Png>,
    /// Frames handed to the writer.
    written: u64,
    writer: Writer,
    started: Instant,
    last_window: Instant,
    /// This frame's step (`plan`).
    pub step: Option<Step>,
    /// The track time of the last film frame asked.
    shown: f32,
    pub out: PathBuf,
    /// The HUD attachments are drawn in it.
    pub huds: bool,
}

/// Why a render stopped before its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    /// A frame could not be drawn (video memory, a lost device).
    Draw,
    /// The writer stopped (the encoder, the disk).
    Write,
}

impl RenderJob {
    pub fn new(writer: Writer, size: (u32, u32), schedule: Schedule, base: (Instant, f32), out: PathBuf, huds: bool) -> RenderJob {
        let now = Instant::now();
        RenderJob {
            size,
            schedule,
            base,
            next: 0,
            sub: 0,
            shutter: None,
            asked: VecDeque::new(),
            backlog: VecDeque::new(),
            written: 0,
            writer,
            started: now,
            last_window: now,
            step: None,
            shown: 0.0,
            out,
            huds,
        }
    }

    /// Each frame drawn as `count` sub-frames over the `open` seconds its
    /// shutter stays open (none when `count` is below 2 or `open` is not
    /// above 0).
    pub fn with_shutter(mut self, open: f32, count: u32) -> RenderJob {
        self.shutter = (open > 0.0 && open.is_finite() && count >= 2).then_some((open, count));
        self
    }

    /// The track's time of sub-frame `sub` of frame `n`: the middle of its
    /// share of the shutter's time.
    fn film_time(&self, n: u64, sub: u32) -> f32 {
        let base = self.schedule.time(n);
        match self.shutter {
            Some((open, count)) => base + open * (sub as f32 + 0.5) / count as f32,
            None => base,
        }
    }

    /// The sub-frame `sub` of a frame, with a shutter.
    fn sub_frame(&self, sub: u32) -> Option<aurora_render::Shutter> {
        self.shutter.map(|(_, count)| aurora_render::Shutter { index: sub, count })
    }

    /// The frame `now` begins: the film's next frame when it can be asked
    /// (few enough in flight, the writer keeping up, the window drawn
    /// lately, and never ahead of the real clock: the world's events then
    /// come in its past), else a frame of the window.
    pub fn plan(&mut self, now: Instant) -> Step {
        let time = self.film_time(self.next, self.sub);
        let at = self.base.0 + Duration::from_secs_f32(time);
        let film = self.next < self.schedule.count()
            && self.asked.len() < IN_FLIGHT
            && self.backlog.is_empty()
            && now.saturating_duration_since(self.last_window) < WINDOW_EVERY
            && at <= now;
        let step = if film {
            Step::Film {
                n: self.next,
                time,
                shutter: self.sub_frame(self.sub),
            }
        } else {
            self.last_window = self.last_window.max(now);
            Step::Window { time: self.shown }
        };
        self.step = Some(step);
        step
    }

    /// The world's instant and clock, and whether it holds still, for this
    /// frame (`snapshot::Hold`).
    pub fn hold(&self) -> crate::snapshot::Hold {
        let (time, later) = match self.step {
            Some(Step::Film { time, .. }) => (time, false),
            Some(Step::Window { time }) => (time, true),
            None => (self.shown, true),
        };
        crate::snapshot::Hold {
            at: self.base.0 + Duration::from_secs_f32(time),
            clock: self.base.1 + time,
            later,
        }
    }

    /// The time stepped by this frame: from the last film frame (or
    /// sub-frame) asked to this one, 1 / fps without a shutter (none for
    /// the first), none for the window's.
    pub fn dt(&self) -> f32 {
        match self.step {
            Some(Step::Film { time, .. }) if self.next > 0 || self.sub > 0 => (time - self.shown).max(0.0),
            _ => 0.0,
        }
    }

    /// The film frame planned for this frame is asked of the renderer: its
    /// tag. With a shutter, a sub-frame before the last is answered by
    /// nothing: the frame is waited for once its last is asked.
    pub fn asked(&mut self, n: u64) -> u64 {
        self.shown = self.film_time(n, self.sub);
        match self.sub_frame(self.sub) {
            Some(s) if !s.last() => self.sub += 1,
            _ => {
                self.asked.push_back(n);
                self.next = n + 1;
                self.sub = 0;
            }
        }
        n | RENDER_TAG
    }

    /// The frame planned could not be asked (a photo went first): the
    /// frame is a window's.
    pub fn not_asked(&mut self) {
        self.step = Some(Step::Window { time: self.shown });
    }

    /// A frame back from the renderer (a render's: `RENDER_TAG`): to the
    /// writer, in order. An error when it could not be drawn.
    pub fn answer(&mut self, tag: u64, pixels: Option<(u32, u32, Vec<u8>)>) -> Result<(), Stopped> {
        let n = tag & !RENDER_TAG;
        if self.asked.front() != Some(&n) {
            // (an answer of another render, cancelled)
            return Ok(());
        }
        self.asked.pop_front();
        match pixels {
            Some((w, h, pixels)) if (w, h) == self.size => {
                self.backlog.push_back((n, pixels));
                Ok(())
            }
            _ => Err(Stopped::Draw),
        }
    }

    /// Each frame: the frames back handed to the writer while it takes
    /// them. An error when it stopped.
    pub fn feed(&mut self) -> Result<(), Stopped> {
        while let Some((n, pixels)) = self.backlog.pop_front() {
            if let Some(pixels) = self.writer.offer(n, pixels) {
                self.backlog.push_front((n, pixels));
                break;
            }
            self.written += 1;
        }
        if self.writer.failed() { Err(Stopped::Write) } else { Ok(()) }
    }

    /// Frames handed to the writer, of how many.
    pub fn progress(&self) -> (u64, u64) {
        (self.written, self.schedule.count())
    }

    /// Every frame is with the writer.
    pub fn complete(&self) -> bool {
        self.written >= self.schedule.count()
    }

    /// Seconds left, at the pace so far.
    pub fn time_left(&self, now: Instant) -> Option<f32> {
        time_left(self.written, self.schedule.count(), now.saturating_duration_since(self.started))
    }

    /// No more frames: the writer finishes its files.
    pub fn finish(self) -> Finishing {
        self.writer.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_is_the_format_as_large_as_its_resolution_allows() {
        let window = (1683, 900);
        // the window: its picture, as the live film
        assert_eq!(render_size(Resolution::Window, window, None, Format::Mp4), (1682, 900));
        assert_eq!(render_size(Resolution::Window, window, Some(2.39), Format::Mp4), (1682, 704));
        // 16:9 fills the box
        assert_eq!(render_size(Resolution::Hd, window, Some(16.0 / 9.0), Format::Mp4), (1920, 1080));
        assert_eq!(render_size(Resolution::Uhd, window, Some(16.0 / 9.0), Format::Mp4), (3840, 2160));
        assert_eq!(render_size(Resolution::Qhd, window, Some(16.0 / 9.0), Format::Mp4), (2560, 1440));
        // scope: the full width, cut above and below
        assert_eq!(render_size(Resolution::Hd, window, Some(2.39), Format::Mp4), (1920, 802));
        // upright: the box turned, 1080 wide
        assert_eq!(render_size(Resolution::Hd, window, Some(9.0 / 16.0), Format::Mp4), (1080, 1920));
        // square, and the window's own proportions with no format
        assert_eq!(render_size(Resolution::Hd, window, Some(1.0), Format::Mp4), (1080, 1080));
        let (w, h) = render_size(Resolution::Uhd, window, None, Format::ProRes);
        assert!(w == 3840 && h <= 2160 && h % 2 == 0, "{w}x{h}");
        for resolution in Resolution::ALL {
            let (w, h) = render_size(resolution, (0, 0), Some(f32::NAN), Format::Mp4);
            assert!(w >= 2 && h >= 2 && w % 2 == 0 && h % 2 == 0);
        }
        assert_eq!(Resolution::Uhd.lines(), Some(2160));
        assert_eq!(Resolution::default(), Resolution::Hd);
    }

    #[test]
    fn every_frame_of_the_track_is_rendered_once_the_last_on_its_end() {
        let s = Schedule::new(2.0, 30);
        assert_eq!(s.count(), 61);
        assert_eq!(s.time(0), 0.0);
        assert!((s.time(15) - 0.5).abs() < 1e-6);
        assert_eq!(s.time(60), 2.0);
        // a duration between two frames: one more, on the end
        let s = Schedule::new(1.01, 24);
        assert_eq!(s.count(), 26);
        assert_eq!(s.time(25), 1.01);
        assert!(s.time(24) < 1.01);
        // the steps between frames are all one frame long
        let s = Schedule::new(10.0, 60);
        for n in 1..s.count() {
            assert!((s.time(n) - s.time(n - 1) - 1.0 / 60.0).abs() < 1e-4);
        }
        assert_eq!(Schedule::new(0.0, 30).count(), 1);
        assert_eq!(Schedule::new(f32::NAN, 0).count(), 1);
    }

    #[test]
    fn the_images_sort_in_order_in_a_folder_of_their_own() {
        assert_eq!(png_name("Aurora", 7, 61), "Aurora_00007.png");
        assert_eq!(png_name("Aurora", 123_456, 200_000), "Aurora_123456.png");
        let root = std::env::temp_dir().join(format!("aurora-render-folder-{}", std::process::id()));
        let _ = std::fs::create_dir_all(root.join("Aurora_x"));
        assert_eq!(free_folder(&root, "x"), root.join("Aurora_x_1"));
        assert_eq!(free_folder(&root, "y"), root.join("Aurora_y"));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(time_left(0, 100, Duration::from_secs(5)), None);
        assert_eq!(time_left(10, 100, Duration::from_secs(5)), Some(45.0));
        assert_eq!(time_left(100, 100, Duration::from_secs(5)), Some(0.0));
    }

    fn png_job(root: &Path, schedule: Schedule, base: Instant) -> RenderJob {
        let _ = std::fs::create_dir_all(root);
        let writer = Writer::Png(PngWriter::start(root.to_owned(), "f".to_owned(), schedule.count(), (2, 2)));
        RenderJob::new(writer, (2, 2), schedule, (base, 10.0), root.to_owned(), false)
    }

    #[test]
    fn the_world_steps_one_frame_per_film_frame_and_holds_in_between() {
        let root = std::env::temp_dir().join(format!("aurora-render-plan-{}", std::process::id()));
        // (the render began a minute after the world's base: its frames are
        // all in the real clock's past)
        let base = Instant::now() - Duration::from_secs(60);
        let mut job = png_job(&root, Schedule::new(1.0, 10), base);
        let late = job.started;
        // the first frame of the film: the world at its start, no step
        let Step::Film { n: 0, time, .. } = job.plan(late) else {
            panic!("the first frame is the film's");
        };
        assert_eq!(time, 0.0);
        assert_eq!(job.dt(), 0.0);
        let hold = job.hold();
        assert!(!hold.later && hold.at == base && hold.clock == 10.0);
        assert_eq!(job.asked(0), RENDER_TAG);
        // the second: one step later
        assert_eq!(
            job.plan(late),
            Step::Film {
                n: 1,
                time: 0.1,
                shutter: None
            }
        );
        assert!((job.dt() - 0.1).abs() < 1e-6);
        assert!((job.hold().clock - 10.1).abs() < 1e-5);
        job.asked(1);
        // two in flight: the window's frame, the world held at the last
        let Step::Window { time } = job.plan(late) else {
            panic!("a window frame");
        };
        assert!((time - 0.1).abs() < 1e-6);
        let hold = job.hold();
        assert!(hold.later, "nothing moves");
        assert_eq!(job.dt(), 0.0);
        // the answers go to the writer in order
        assert_eq!(job.answer(RENDER_TAG, Some((2, 2, vec![1; 16]))), Ok(()));
        assert_eq!(job.answer(RENDER_TAG | 7, Some((2, 2, vec![1; 16]))), Ok(()), "not ours: ignored");
        assert_eq!(job.answer(RENDER_TAG | 1, Some((2, 2, vec![2; 16]))), Ok(()));
        assert_eq!(job.feed(), Ok(()));
        assert_eq!(job.progress(), (2, 11));
        // never ahead of the real clock: the film's frame at 0.2 s waits
        // until 0.2 s after the start
        assert!(matches!(job.plan(base + Duration::from_millis(100)), Step::Window { .. }));
        // a frame that could not be drawn stops the render
        assert!(matches!(job.plan(late), Step::Film { n: 2, .. }));
        job.asked(2);
        assert_eq!(job.answer(RENDER_TAG | 2, None), Err(Stopped::Draw));
        let mut finishing = job.finish();
        let done = loop {
            if let Some(done) = finishing.done() {
                break done;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(done, Ok((root.clone(), 2)));
        let image = image::open(root.join("f_00001.png")).expect("a PNG").to_rgba8();
        assert_eq!((image.width(), image.height()), (2, 2));
        assert!(image.pixels().all(|p| p.0 == [2, 2, 2, 2]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shutter_draws_each_frame_as_sub_frames_over_its_open_time_and_waits_for_the_last() {
        let root = std::env::temp_dir().join(format!("aurora-render-shutter-{}", std::process::id()));
        let base = Instant::now() - Duration::from_secs(60);
        // 10 frames a second, a 180° shutter (0.05 s) in 4 sub-frames
        let mut job = png_job(&root, Schedule::new(1.0, 10), base).with_shutter(0.05, 4);
        let late = job.started;
        let mut times = Vec::new();
        for sub in 0..4 {
            let Step::Film { n: 0, time, shutter } = job.plan(late) else {
                panic!("sub-frame {sub} of the first frame");
            };
            assert_eq!(shutter, Some(aurora_render::Shutter { index: sub, count: 4 }));
            if sub == 0 {
                assert_eq!(job.dt(), 0.0, "nothing before the first");
            } else {
                assert!((job.dt() - 0.0125).abs() < 1e-6, "{}", job.dt());
            }
            times.push(time);
            assert_eq!(job.asked(0), RENDER_TAG);
        }
        // the middles of the four quarters of the open time
        for (t, want) in times.iter().zip([0.00625, 0.01875, 0.03125, 0.04375]) {
            assert!((t - want).abs() < 1e-6, "{times:?}");
        }
        // one frame waited for, then the next frame from its own instant
        assert_eq!(job.asked.len(), 1);
        let Step::Film { n: 1, time, shutter } = job.plan(late) else {
            panic!("the next frame");
        };
        assert!((time - 0.10625).abs() < 1e-6);
        assert_eq!(shutter.map(|s| s.index), Some(0));
        assert!((job.dt() - (0.10625 - 0.04375)).abs() < 1e-6);
        // too few sub-frames or no open time: no shutter
        let plain = png_job(&root, Schedule::new(1.0, 10), base).with_shutter(0.05, 1);
        assert_eq!(plain.shutter, None);
        let plain = png_job(&root, Schedule::new(1.0, 10), base).with_shutter(f32::NAN, 8);
        assert_eq!(plain.shutter, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_window_is_drawn_between_film_frames_to_keep_the_interface_alive() {
        let root = std::env::temp_dir().join(format!("aurora-render-window-{}", std::process::id()));
        let base = Instant::now() - Duration::from_secs(60);
        let mut job = png_job(&root, Schedule::new(100.0, 30), base);
        let start = job.started;
        let mut window = 0;
        for i in 0..200u64 {
            let now = start + Duration::from_millis(i * 10);
            match job.plan(now) {
                Step::Film { n, .. } => {
                    let tag = job.asked(n);
                    job.answer(tag, Some((2, 2, vec![0; 16]))).expect("drawn");
                }
                Step::Window { .. } => window += 1,
            }
            let _ = job.feed();
        }
        // two seconds: at least one window frame every quarter second
        assert!(window >= 7, "{window} window frames");
        let _ = std::fs::remove_dir_all(&root);
    }
}
