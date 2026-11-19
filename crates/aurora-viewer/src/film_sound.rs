//! The sound of the Director's live film: the viewer's own mix (the audio
//! engine's record tap, aurora-audio's record.rs), written beside the
//! picture and put in the film's file once both are finished. Aurora's own
//! (the Legacy Viewer records no film).
//!
//! The mix comes in blocks from the audio callback; a thread writes them as
//! raw 32-bit float stereo at 48 kHz into a file beside the film, with
//! silence of the same length where blocks were lost, so the sound keeps
//! the clock. The picture keeps it too (`recorder::Pace::uncapped`: a late
//! frame is written again, never skipped for good), so both last what the
//! shot lasted; the sound is then cut or padded to the picture's exact
//! length (`fit`) and ffmpeg puts the two in one file
//! (`recorder::mux_arguments`), the picture copied as it is.
//!
//! The offline render has no sound: its world does not run in real time,
//! and the engine plays at the real clock.

use crate::recorder;
use aurora_audio::{ENGINE_SAMPLE_RATE, RecordBlock, RecordTap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// Bytes of a stereo frame of the file (two 32-bit floats).
const FRAME_BYTES: u64 = 8;
/// How often the writer looks for new blocks.
const POLL: Duration = Duration::from_millis(10);

/// The sound frames as long as `video_frames` of a film at `fps`.
pub fn frames_for(video_frames: u64, fps: u32) -> u64 {
    let fps = u64::from(fps.max(1));
    (video_frames * u64::from(ENGINE_SAMPLE_RATE) + fps / 2) / fps
}

/// The raw sound file beside a film's: `film.mp4` → `film.mp4.sound.f32`.
pub fn sound_path(film: &Path) -> PathBuf {
    let mut name = film.as_os_str().to_owned();
    name.push(".sound.f32");
    PathBuf::from(name)
}

/// The picture alone, before its sound is added: `film.mp4` →
/// `film.picture.mp4` (the container's extension kept: ffmpeg chooses it
/// from the name).
pub fn picture_path(film: &Path) -> PathBuf {
    let stem = film.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = film.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    film.with_file_name(format!("{stem}.picture.{ext}"))
}

/// Where the blocks come from: the engine's tap (a test's own blocks).
trait Blocks {
    fn next(&self) -> Option<RecordBlock>;
    fn recycle(&self, block: RecordBlock);
}

impl Blocks for RecordTap {
    fn next(&self) -> Option<RecordBlock> {
        RecordTap::next(self)
    }

    fn recycle(&self, block: RecordBlock) {
        RecordTap::recycle(self, block.frames);
    }
}

/// Write the blocks of `tap` into `out` until `stop`, then what is left:
/// the stereo frames written, silences included.
fn write_sound(tap: &impl Blocks, stop: &AtomicBool, out: impl Write) -> Result<u64, String> {
    let mut out = std::io::BufWriter::new(out);
    let mut frames = 0u64;
    let silence = [0u8; 4096];
    loop {
        // (read the flag first: the blocks sent before it are all taken)
        let stopping = stop.load(Ordering::Acquire);
        while let Some(block) = tap.next() {
            let mut lost = block.lost_before * FRAME_BYTES;
            while lost > 0 {
                let n = lost.min(silence.len() as u64) as usize;
                out.write_all(&silence[..n]).map_err(|e| e.to_string())?;
                lost -= n as u64;
            }
            for sample in block.frames.iter() {
                out.write_all(&sample.to_le_bytes()).map_err(|e| e.to_string())?;
            }
            frames += block.lost_before + (block.frames.len() / 2) as u64;
            tap.recycle(block);
        }
        if stopping {
            break;
        }
        std::thread::sleep(POLL);
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(frames)
}

/// The film's sound being written.
pub struct SoundWriter {
    stop: Arc<AtomicBool>,
    writer: JoinHandle<Result<u64, String>>,
    pub path: PathBuf,
}

impl SoundWriter {
    /// Write the mix of `tap` into `path`, from now on.
    pub fn start(tap: RecordTap, path: PathBuf) -> SoundWriter {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, file) = (stop.clone(), path.clone());
        let writer = std::thread::spawn(move || {
            let out = std::fs::File::create(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            write_sound(&tap, &flag, out)
        });
        SoundWriter { stop, writer, path }
    }

    /// No more sound: the writer takes what is queued and ends. Its result
    /// with `Stopped::join` (off the frame thread).
    pub fn stop(self) -> Stopped {
        self.stop.store(true, Ordering::Release);
        Stopped {
            writer: self.writer,
            path: self.path,
        }
    }
}

/// A sound whose writer is ending.
pub struct Stopped {
    writer: JoinHandle<Result<u64, String>>,
    pub path: PathBuf,
}

impl Stopped {
    pub fn join(self) -> Result<(PathBuf, u64), String> {
        let frames = self
            .writer
            .join()
            .unwrap_or_else(|_| Err("the sound's writer stopped".to_owned()))?;
        Ok((self.path, frames))
    }
}

/// Make the sound file `frames` stereo frames long: cut, or padded with
/// silence (the picture is the film's length).
pub fn fit(path: &Path, frames: u64) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // (set_len pads with zeros: silence in 32-bit floats)
    file.set_len(frames * FRAME_BYTES).map_err(|e| format!("{}: {e}", path.display()))
}

/// What became of a film with sound: its file, and whether the sound is in
/// it (why not).
pub struct Muxed {
    pub file: PathBuf,
    pub sound: Result<(), String>,
}

/// Give the picture its sound (on a thread: ffmpeg runs). `picture` is the
/// finished picture of `video_frames` at `fps`; the result goes to `out`.
/// The sound's and the picture's own files are removed once in it; when the
/// sound could not be added, the picture alone becomes the film.
pub fn mux(ffmpeg: &Path, encoder: &str, sound: Stopped, picture: &Path, video_frames: u64, fps: u32, out: &Path) -> Muxed {
    let added = sound.join().and_then(|(path, written)| {
        let wanted = frames_for(video_frames, fps);
        log::info!("director: sound of {written} frames, fitted to the picture's {wanted}");
        fit(&path, wanted)?;
        let result = recorder::run_to_end(ffmpeg, &recorder::mux_arguments(picture, &path, encoder, out));
        let _ = std::fs::remove_file(&path);
        result
    });
    match added {
        Ok(()) => {
            let _ = std::fs::remove_file(picture);
            Muxed {
                file: out.to_owned(),
                sound: Ok(()),
            }
        }
        Err(why) => {
            let _ = std::fs::remove_file(out);
            let file = match std::fs::rename(picture, out) {
                Ok(()) => out.to_owned(),
                Err(_) => picture.to_owned(),
            };
            Muxed { file, sound: Err(why) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::Recording;

    /// Every format the ffmpeg on the PATH writes, given a second of sound:
    /// the film has its audio stream, encoded as its container expects.
    /// `cargo test --workspace the_ffmpeg_found -- --ignored --nocapture`.
    #[test]
    #[ignore = "runs the ffmpeg found on the PATH"]
    fn the_ffmpeg_found_gives_each_format_its_sound() {
        let path = std::env::var_os("PATH")
            .and_then(|path| recorder::find_on_path(&path))
            .expect("an ffmpeg on the PATH");
        let tool = recorder::Tool::probe(&path).expect("ffmpeg answers");
        let root = std::env::temp_dir().join(format!("aurora-film-sound-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("folder");
        let (size, fps, frames) = ((64, 36), 24, 24u64);
        let mut failures = Vec::new();
        for format in recorder::Format::ALL {
            let (Some(video), Some(audio)) = (tool.encoder(format), tool.audio_encoder(format)) else {
                eprintln!("skip  {format:?}: no encoder");
                continue;
            };
            let out = root.join(format!("{format:?}.{}", format.extension()));
            let picture = picture_path(&out);
            let args = recorder::arguments(video, format, recorder::Quality::Good, size, [size.0, size.1, 0, 0], fps, &picture);
            let mut film = Recording::start(recorder::Sink::Ffmpeg(tool.path.clone(), args), size, fps, out.clone());
            for n in 0..frames {
                let mut pixels = vec![n as u8 * 8; (size.0 * size.1 * 4) as usize];
                while let Some(back) = film.offer(pixels) {
                    pixels = back;
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            let mut finishing = film.finish();
            let written = loop {
                if let Some(done) = finishing.done() {
                    break done;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let Ok((_, written)) = written else {
                failures.push(format!("{format:?}: the picture failed"));
                continue;
            };
            // a second of a 440 Hz tone, a little short: padded to the picture
            let sound = sound_path(&out);
            let tone: Vec<u8> = (0..47_000)
                .flat_map(|i| {
                    let s = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.25;
                    [s, s]
                })
                .flat_map(f32::to_le_bytes)
                .collect();
            std::fs::write(&sound, tone).expect("sound");
            let stopped = Stopped {
                writer: std::thread::spawn(|| Ok(47_000)),
                path: sound.clone(),
            };
            let muxed = mux(&tool.path, audio, stopped, &picture, written, fps, &out);
            let probe = std::process::Command::new(&tool.path)
                .args(["-hide_banner", "-i"])
                .arg(&muxed.file)
                .output()
                .expect("ffmpeg runs");
            let said = String::from_utf8_lossy(&probe.stderr);
            match &muxed.sound {
                Ok(()) if said.contains("Audio:") && !picture.exists() && !sound.exists() => {
                    eprintln!("ok    {format:?}: {video} + {audio}");
                }
                Ok(()) => failures.push(format!("{format:?}: no audio stream in the film ({said})")),
                Err(why) => failures.push(format!("{format:?}: {why}")),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn the_sound_lasts_what_the_picture_lasts() {
        assert_eq!(frames_for(30, 30), 48_000);
        assert_eq!(frames_for(1, 24), 2_000);
        assert_eq!(frames_for(0, 60), 0);
        // 29.97 frames of 60 do not exist; a rate that does not divide 48 kHz
        // rounds to the nearest frame
        assert_eq!(frames_for(1, 7), 6_857);
        assert_eq!(frames_for(10, 0), 480_000, "a broken rate is one a second");
        assert_eq!(sound_path(Path::new("a/film.mp4")), Path::new("a/film.mp4.sound.f32"));
        assert_eq!(picture_path(Path::new("a/film.mp4")), Path::new("a/film.picture.mp4"));
    }

    #[test]
    fn a_sound_is_cut_or_padded_to_its_length() {
        let path = std::env::temp_dir().join(format!("aurora-fit-{}.f32", std::process::id()));
        std::fs::write(&path, [1u8; 80]).expect("written");
        fit(&path, 4).expect("cut");
        assert_eq!(std::fs::read(&path).expect("read"), [1u8; 32]);
        fit(&path, 6).expect("padded");
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(bytes.len(), 48);
        assert!(bytes[32..].iter().all(|&b| b == 0), "silence");
        let _ = std::fs::remove_file(&path);
        assert!(fit(Path::new("no/such/file.f32"), 4).is_err());
    }

    struct Fake(std::cell::RefCell<Vec<RecordBlock>>, std::cell::Cell<usize>);

    impl Blocks for Fake {
        fn next(&self) -> Option<RecordBlock> {
            let mut blocks = self.0.borrow_mut();
            (!blocks.is_empty()).then(|| blocks.remove(0))
        }

        fn recycle(&self, _: RecordBlock) {
            self.1.set(self.1.get() + 1);
        }
    }

    #[test]
    fn the_mix_is_written_with_silence_where_it_was_lost() {
        let block = |value: f32, lost_before: u64| RecordBlock {
            frames: Box::new([value; aurora_audio::RECORD_BLOCK * 2]),
            lost_before,
        };
        let tap = Fake(
            std::cell::RefCell::new(vec![block(0.5, 0), block(-0.25, 3)]),
            std::cell::Cell::new(0),
        );
        let stop = AtomicBool::new(true);
        let mut out = Vec::new();
        let frames = write_sound(&tap, &stop, &mut out).expect("written");
        let n = aurora_audio::RECORD_BLOCK as u64;
        assert_eq!(frames, 2 * n + 3);
        assert_eq!(out.len() as u64, frames * FRAME_BYTES);
        let sample = |i: usize| f32::from_le_bytes(out[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        assert_eq!(sample(0), 0.5);
        // the three frames lost before the second block: silence, in place
        let lost = 2 * n as usize;
        assert!((lost..lost + 6).all(|i| sample(i) == 0.0));
        assert_eq!(sample(lost + 6), -0.25);
        assert_eq!(tap.1.get(), 2, "the blocks are given back");
    }
}
