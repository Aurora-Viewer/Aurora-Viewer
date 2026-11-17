//! The Director's films in the app, beside `app/director.rs`: the offline
//! render (`director_render.rs`) and its world clock, the live film's frames
//! read back without waiting (aurora-render's film_readback.rs), its sound
//! (`film_sound.rs`), and the fades and title cards drawn into the picture
//! (`director_titles.rs`). Aurora's own: the Legacy Viewer has no camera
//! path tool.

use super::App;
use super::snapshot::PhotoFrame;
use crate::director::{self, Mode};
use crate::director_render::{self, Finishing, PngWriter, RENDER_TAG, RenderJob, Schedule, Step, Stopped, Writer};
use crate::director_titles::{self, Place, Tint};
use crate::film_sound::{self, SoundWriter};
use crate::recorder::{self, Pace, Recording, Sink, Tool};
use aurora_render::{FilmRequest, OverlayEffect, Renderer, SnapshotSpec};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

/// The title card of a title, as drawn for a picture of one height.
#[derive(Debug, Clone, PartialEq)]
struct CardKey {
    lines: Vec<String>,
    size: u32,
    place: Place,
    height: u32,
}

/// A title's card: drawn on a thread, then a texture of the renderer.
#[derive(Default)]
struct Card {
    key: Option<CardKey>,
    drawing: Option<Receiver<Option<director_titles::Card>>>,
    /// Its texture's slot and size.
    texture: Option<(u32, u32, u32)>,
}

/// The live film's sound, once its recording ends: what gives it to the
/// picture when the encoder has finished.
struct SoundToMux {
    sound: film_sound::Stopped,
    encoder: &'static str,
    ffmpeg: PathBuf,
    fps: u32,
}

/// The films' state beside `DirectorState`'s.
#[derive(Default)]
pub(super) struct Films {
    /// Render was pressed: it starts with the next frame (the renderer's
    /// size known).
    render_request: bool,
    job: Option<RenderJob>,
    /// The render whose files are being finished.
    finishing: Option<Finishing>,
    /// The live film's frame asked of the renderer, and when.
    live_asked: Option<(u64, Instant)>,
    next_tag: u64,
    /// The live film's sound being written, its encoder, ffmpeg, rate.
    sound: Option<(SoundWriter, &'static str, PathBuf, u32)>,
    /// The sound of a film whose picture is being finished.
    to_mux: Option<SoundToMux>,
    /// The thread that puts the picture and its sound in one file.
    muxing: Option<Receiver<film_sound::Muxed>>,
    cards: Vec<Card>,
}

/// The audio encoder a live film of `options` gets its sound with: `None`
/// for a silent film (the sound off, or none this ffmpeg can write).
pub(super) fn sound_for(options: &recorder::Options, tool: &Tool, engine: bool) -> Option<&'static str> {
    (engine && !options.sound_channels().is_empty())
        .then(|| tool.audio_encoder(options.format))
        .flatten()
}

/// Where a picture of `aspect` (`None`: the whole frame) stands in a frame
/// of `frame`: left, top, width, height, pixels (the format's bars).
fn picture_in(frame: (f32, f32), aspect: Option<f32>) -> [f32; 4] {
    let (fw, fh) = (frame.0.max(1.0), frame.1.max(1.0));
    match aspect.filter(|a| a.is_finite() && *a > 0.0) {
        Some(a) if fw / fh > a => {
            let w = fh * a;
            [(fw - w) * 0.5, 0.0, w, fh]
        }
        Some(a) => {
            let h = fw / a;
            [0.0, (fh - h) * 0.5, fw, h]
        }
        None => [0.0, 0.0, fw, fh],
    }
}

impl App {
    /// A render is under way.
    pub(super) fn director_rendering(&self) -> bool {
        self.director.films.job.is_some()
    }

    /// A film's files are being finished (the render's, the sound added).
    pub(super) fn director_films_finishing(&self) -> bool {
        self.director.films.finishing.is_some() || self.director.films.muxing.is_some() || self.director.films.to_mux.is_some()
    }

    /// At the start of a frame, during a render: the world's instant for it
    /// (`snapshot::Hold`), the film's next frame or the window's.
    pub(super) fn director_render_hold(&mut self, now: Instant) -> Option<crate::snapshot::Hold> {
        let job = self.director.films.job.as_mut()?;
        job.plan(now);
        Some(job.hold())
    }

    /// The time this frame steps, during a render: one frame of the film,
    /// none between its frames.
    pub(super) fn director_render_dt(&self) -> Option<f32> {
        self.director.films.job.as_ref().map(RenderJob::dt)
    }

    /// A network event's instant, on the render's clock: the world shows a
    /// time behind the real one, the event arrived that much earlier for it
    /// (a motion reported starts from there).
    pub(super) fn director_render_stamp(&self, sent: Instant) -> Instant {
        let Some(job) = &self.director.films.job else {
            return sent;
        };
        let lag = Instant::now().saturating_duration_since(job.hold().at);
        sent.checked_sub(lag).unwrap_or(sent)
    }

    /// After the playback's tick: the track's view at the frame rendered.
    /// The render stops with what stops a playback.
    pub(super) fn director_render_scrub(&mut self, open: bool) {
        let Some(job) = &self.director.films.job else {
            return;
        };
        if !open || self.world.teleporting || self.director_restricted() {
            log::info!("director: the render stops (window closed, teleport or RLVa)");
            self.director_render_stop(Some(aurora_i18n::tr!("director-render-interrupted").to_owned()));
            return;
        }
        let time = match job.step {
            Some(Step::Film { time, .. } | Step::Window { time }) => time,
            None => 0.0,
        };
        self.director.d.scrub(time);
    }

    /// Before a frame is laid out, during a render: the film's frame it
    /// carries, drawn offscreen at the film's size and read back later.
    pub(super) fn director_render_photo_frame(&mut self, renderer: &mut Renderer) -> Option<PhotoFrame> {
        let job = self.director.films.job.as_mut()?;
        let Some(Step::Film { n, .. }) = job.step else {
            return None;
        };
        if renderer.snapshot_request.is_some() || renderer.film_request.is_some() {
            // (a photo asked first: this frame is the window's)
            job.not_asked();
            return None;
        }
        let tag = job.asked(n);
        let size = job.size;
        let (width, height) = size;
        renderer.film_request = Some(FilmRequest {
            tag,
            size: Some(job.size),
            huds: job.huds,
        });
        self.snapshots.capturing = Some(SnapshotSpec {
            width,
            height,
            interface: false,
            huds: job.huds,
            photo: job.size,
            cube: None,
        });
        Some(PhotoFrame {
            aspect: width as f32 / height.max(1) as f32,
            tile: glam::Mat4::IDENTITY,
            height,
            scale: crate::snapshot::viewfinder_scale(job.size, renderer.size(), self.settings.render_scale_percent),
            // the Filters tab's looks, on the whole frame
            look: self.director_render_look(size),
            context: aurora_render::PhotoContext::None,
            cube_face: false,
        })
    }

    /// Render pressed: with the next frame.
    pub(super) fn director_render_request(&mut self) {
        self.director.films.render_request = true;
    }

    /// Start the render asked: the track from its start, at the size and
    /// in the format chosen, into a new file (or folder of images) of the
    /// films' folder.
    fn director_render_start(&mut self, window: (u32, u32)) {
        let d = &self.director.d;
        if !d.can_play() || self.director_restricted() || self.director.recording.is_some() || self.director_films_finishing() {
            return;
        }
        let options = self.settings.director_record.clone();
        let fps = options.rate();
        let size = director_render::render_size(options.resolution, window, d.format.aspect(), options.format);
        let schedule = Schedule::new(d.track.duration(), fps);
        let folder = recorder::default_folder();
        if let Err(e) = std::fs::create_dir_all(&folder) {
            self.director.film = Some(Err(format!("{}: {e}", folder.display())));
            return;
        }
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let (writer, out) = if options.png {
            let out = director_render::free_folder(&folder, &stamp);
            if let Err(e) = std::fs::create_dir(&out) {
                self.director.film = Some(Err(format!("{}: {e}", out.display())));
                return;
            }
            (
                Writer::Png(PngWriter::start(out.clone(), "Aurora".to_owned(), schedule.count(), size)),
                out,
            )
        } else {
            let super::director::Ffmpeg::Ready(tool) = &self.director.ffmpeg else {
                return;
            };
            let Some(encoder) = tool.encoder(options.format) else {
                self.director.film = Some(Err(aurora_i18n::tr!("director-record-no-encoder").to_owned()));
                return;
            };
            let out = recorder::free_path(&folder, &stamp, options.format);
            let whole = [size.0, size.1, 0, 0];
            let args = recorder::arguments(encoder, options.format, options.quality, size, whole, fps, &out);
            let sink = Sink::Ffmpeg(tool.path.clone(), args);
            (Writer::Film(Recording::start_with(sink, size, Pace::new(fps), out.clone())), out)
        };
        // the HUDs as the playback shows them
        let huds = !(self.director.d.hide_huds && !self.world.rlva_has_locked_hud());
        let base = (Instant::now(), self.start.elapsed().as_secs_f32());
        log::info!(
            "director: rendering {} frames of {}x{} at {fps} a second to {}",
            schedule.count(),
            size.0,
            size.1,
            out.display()
        );
        self.director.d.stop();
        self.director.d.scrub(0.0);
        self.director.film = None;
        self.director.films.job = Some(RenderJob::new(writer, size, schedule, base, out, huds));
    }

    /// End the render: its writer finishes what it has (the frames written
    /// so far, when it is stopped before its end). `why`: it stopped early.
    fn director_render_stop(&mut self, why: Option<String>) {
        let Some(job) = self.director.films.job.take() else {
            return;
        };
        let (done, count) = job.progress();
        match &why {
            None => log::info!("director: render of {count} frames drawn, finishing its files"),
            Some(why) => log::info!(
                "director: render stopped at frame {done} of {count} ({why}), in {}",
                job.out.display()
            ),
        }
        self.director.films.finishing = Some(job.finish());
        if let Some(why) = why {
            // (what was written stays; the message says why it is short)
            self.director.film = Some(Err(why));
        }
        self.director.d.stop();
    }

    /// Escape, or Cancel: the render stops; true when there was one.
    pub(super) fn director_render_cancel(&mut self) -> bool {
        if !self.director_rendering() {
            return false;
        }
        self.director_render_stop(Some(aurora_i18n::tr!("director-render-cancelled").to_owned()));
        true
    }

    /// Before the frame is sent: Render pressed starts one.
    pub(super) fn director_render_frame(&mut self, renderer: &Renderer) {
        if std::mem::take(&mut self.director.films.render_request) && !self.director_rendering() {
            self.director_render_start(renderer.size());
        }
    }

    /// A frame of the live film is due: this frame's 3D view is asked of the
    /// renderer, read back later (`director_films_after`).
    pub(super) fn director_film_ask(&mut self, renderer: &mut Renderer, huds: bool) {
        let films = &mut self.director.films;
        let tag = films.next_tag & !RENDER_TAG;
        films.next_tag = films.next_tag.wrapping_add(1);
        renderer.film_request = Some(FilmRequest { tag, size: None, huds });
        films.live_asked = Some((tag, Instant::now()));
        self.director.record_pending = true;
    }

    /// The live film's sound starts with its picture: the engine's mix of
    /// the channels chosen, written beside the film (`film_sound.rs`).
    pub(super) fn director_sound_begin(&mut self, encoder: &'static str, ffmpeg: PathBuf, out: &std::path::Path, fps: u32) {
        let Some(engine) = &self.audio_engine else {
            return;
        };
        let channels = self.settings.director_record.sound_channels();
        let tap = engine.start_recording(&channels);
        log::info!("director: the film's sound ({channels:?}) written with {encoder}");
        let writer = SoundWriter::start(tap, film_sound::sound_path(out));
        self.director.films.sound = Some((writer, encoder, ffmpeg, fps));
    }

    /// The live film ends: so does its sound, kept until its picture is
    /// finished.
    pub(super) fn director_sound_end(&mut self) {
        let Some((writer, encoder, ffmpeg, fps)) = self.director.films.sound.take() else {
            return;
        };
        if let Some(engine) = &self.audio_engine {
            engine.stop_recording();
        }
        self.director.films.to_mux = Some(SoundToMux {
            sound: writer.stop(),
            encoder,
            ffmpeg,
            fps,
        });
    }

    /// The live film's picture is finished: its sound is added (on a
    /// thread), or it is the film.
    pub(super) fn director_film_done(&mut self, done: Result<(PathBuf, u64), String>) {
        let Some(sound) = self.director.films.to_mux.take() else {
            self.director.film = Some(done.map(|(file, _)| file));
            return;
        };
        match done {
            Ok((out, frames)) => {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let picture = film_sound::picture_path(&out);
                    let muxed = film_sound::mux(&sound.ffmpeg, sound.encoder, sound.sound, &picture, frames, sound.fps, &out);
                    let _ = tx.send(muxed);
                });
                self.director.films.muxing = Some(rx);
            }
            Err(why) => {
                // (the sound of a film that could not be written goes too)
                std::thread::spawn(move || {
                    if let Ok((path, _)) = sound.sound.join() {
                        let _ = std::fs::remove_file(path);
                    }
                });
                self.director.film = Some(Err(why));
            }
        }
    }

    /// After the frame is sent: the film frames read back go to the live
    /// film or the render, the render goes on or ends, the files being
    /// finished are polled, and the title cards are drawn.
    pub(super) fn director_films_after(&mut self, renderer: &mut Renderer) {
        let films = std::mem::take(&mut renderer.films);
        for film in films {
            if film.tag & RENDER_TAG != 0 {
                let answered = match self.director.films.job.as_mut() {
                    Some(job) => job.answer(film.tag, film.pixels),
                    None => Ok(()),
                };
                if answered.is_err() {
                    log::warn!("director: a frame of the render could not be drawn");
                    self.director_render_stop(Some(aurora_i18n::tr!("director-render-draw-failed").to_owned()));
                }
                continue;
            }
            let Some((tag, asked)) = self.director.films.live_asked else {
                continue;
            };
            if tag != film.tag {
                continue;
            }
            self.director.films.live_asked = None;
            self.director.record_pending = false;
            let Some((width, height, pixels)) = film.pixels else {
                continue;
            };
            let kept = self
                .director
                .recording
                .as_mut()
                .is_none_or(|(recording, _)| recording.frame(asked, (width, height), pixels));
            if !kept {
                log::info!("director: the window changed size, the recording ends");
                self.director_record_stop();
            }
        }
        if self.director.recording.is_none() {
            self.director.films.live_asked = None;
            self.director.record_pending = false;
        }
        if let Some(job) = self.director.films.job.as_mut() {
            let fed = job.feed();
            let complete = job.complete();
            if renderer.failed() || renderer.device_lost() {
                self.director_render_stop(Some(aurora_i18n::tr!("director-render-draw-failed").to_owned()));
            } else if let Err(stopped) = fed {
                let why = match stopped {
                    Stopped::Draw => aurora_i18n::tr!("director-render-draw-failed"),
                    Stopped::Write => aurora_i18n::tr!("director-render-write-failed"),
                };
                self.director_render_stop(Some(why.to_owned()));
            } else if complete {
                self.director_render_stop(None);
            }
        }
        if let Some(done) = self.director.films.finishing.as_mut().and_then(Finishing::done) {
            self.director.films.finishing = None;
            match &done {
                Ok((file, frames)) => log::info!("director: render of {frames} frames written to {}", file.display()),
                Err(why) => log::warn!("director: the render could not be written ({why})"),
            }
            // (a render stopped early keeps the message of why)
            if !matches!(self.director.film, Some(Err(_))) || done.is_err() {
                self.director.film = Some(done.map(|(file, _)| file));
            }
        }
        if let Some(muxing) = &self.director.films.muxing {
            match muxing.try_recv() {
                Ok(muxed) => {
                    match &muxed.sound {
                        Ok(()) => log::info!("director: film with its sound written to {}", muxed.file.display()),
                        Err(why) => log::warn!("director: the film's sound could not be added ({why})"),
                    }
                    self.director.films.muxing = None;
                    self.director.film = Some(Ok(muxed.file));
                }
                Err(TryRecvError::Disconnected) => self.director.films.muxing = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        self.director_cards(renderer);
    }

    /// The height a title card is drawn for: the render's picture, or the
    /// window's.
    fn director_card_height(&self, window: (u32, u32)) -> u32 {
        match &self.director.films.job {
            Some(job) => job.size.1,
            None => picture_in((window.0 as f32, window.1 as f32), self.director.d.format.aspect())[3].round() as u32,
        }
    }

    /// The title cards kept drawn while the Director is open: a card drawn
    /// again (on a thread) when its title or the picture's height changes,
    /// its texture freed when it goes.
    fn director_cards(&mut self, renderer: &mut Renderer) {
        let open = self.panels.director && self.in_world();
        let wanted = if open { self.director.d.finish.titles.len() } else { 0 };
        let height = self.director_card_height(renderer.size()).max(2);
        let cards = &mut self.director.films.cards;
        while cards.len() > wanted {
            if let Some((slot, ..)) = cards.pop().and_then(|card| card.texture) {
                renderer.free_texture(slot);
            }
        }
        cards.resize_with(wanted, Card::default);
        for (card, title) in cards.iter_mut().zip(&self.director.d.finish.titles) {
            let key = CardKey {
                lines: title.lines(),
                size: title.size().to_bits(),
                place: title.place,
                height,
            };
            if card.key.as_ref() != Some(&key) {
                card.key = Some(key);
                let (tx, rx) = std::sync::mpsc::channel();
                let title = title.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(director_titles::rasterize(&title, height));
                });
                // (the old card shows until the new one is drawn)
                card.drawing = Some(rx);
            }
            let Some(drawing) = &card.drawing else {
                continue;
            };
            let drawn = match drawing.try_recv() {
                Ok(drawn) => drawn,
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => None,
            };
            card.drawing = None;
            if let Some((slot, ..)) = card.texture.take() {
                renderer.free_texture(slot);
            }
            if let Some((w, h, rgba)) = drawn {
                let mip = aurora_render::textures::MipLevel {
                    width: w,
                    height: h,
                    data: &rgba,
                };
                card.texture = renderer.create_texture(&[mip]).map(|slot| (slot, w, h));
            }
        }
    }

    /// The fades and titles of this frame, drawn into the picture by the
    /// renderer (after the world's labels, before the HUDs): while the
    /// track plays, is scrubbed, filmed or rendered, not while it is edited.
    pub(super) fn director_finish_overlays(&self, window: (u32, u32)) -> Vec<OverlayEffect> {
        let d = &self.director.d;
        let filming = self.director.recording.is_some() || self.director_rendering();
        if !self.panels.director || !self.in_world() || d.finish.is_empty() || (d.mode == Mode::Editing && !filming) {
            return Vec::new();
        }
        // a film frame of the render is the picture alone; else the
        // window's frame, the picture between the format's bars
        let film = self
            .director
            .films
            .job
            .as_ref()
            .filter(|job| matches!(job.step, Some(Step::Film { .. })) && self.snapshots.capturing.is_some());
        let (frame, picture) = match film {
            Some(job) => {
                let frame = (job.size.0 as f32, job.size.1 as f32);
                (frame, [0.0, 0.0, frame.0, frame.1])
            }
            None => {
                let frame = (window.0 as f32, window.1 as f32);
                (frame, picture_in(frame, d.format.aspect()))
            }
        };
        let (time, duration) = (d.time, d.track.duration());
        let mut effects: Vec<OverlayEffect> = Tint::black(d.finish.black(time, duration)).effect().into_iter().collect();
        for (title, card) in d.finish.titles.iter().zip(&self.director.films.cards) {
            let opacity = title.opacity(time, duration);
            let (Some((slot, w, h)), Some(key)) = (card.texture, &card.key) else {
                continue;
            };
            if opacity <= 1e-3 {
                continue;
            }
            let (min, max) = director_titles::card_rect(title.place, (w, h), key.height, picture, frame);
            let [x, y, z] = aurora_render::overlay_rect(min, max);
            let [r, g, b] = director::linear(title.color);
            effects.push(OverlayEffect {
                color: [r, g, b, opacity],
                texture: [slot, x, y, z],
            });
        }
        effects
    }

    /// What the Output section shows of a render.
    pub(super) fn director_render_state(&self) -> crate::ui::director_output::RenderState {
        let options = &self.settings.director_record;
        let window = self.gfx.as_ref().map_or((1920, 1080), |g| g.renderer.size());
        let job = self.director.films.job.as_ref();
        let sound = match &self.director.ffmpeg {
            super::director::Ffmpeg::Ready(tool) => tool.audio_encoder(options.format),
            _ => None,
        };
        crate::ui::director_output::RenderState {
            progress: job.map(|job| {
                let (done, count) = job.progress();
                (done, count, job.time_left(Instant::now()))
            }),
            size: job.map_or_else(
                || director_render::render_size(options.resolution, window, self.director.d.format.aspect(), options.format),
                |job| job.size,
            ),
            can_render: self.director.d.can_play() && !self.director_restricted(),
            sound,
        }
    }

    /// `AURORA_DEMO_DIRECTOR=titles`: fades and two titles on the demo's
    /// track, played; `render`: that track rendered as PNG images at
    /// 1080p, a second of it.
    pub(super) fn director_render_demo(&mut self, mode: &str) {
        if mode != "titles" && mode != "render" {
            return;
        }
        let finish = &mut self.director.d.finish;
        finish.fade_in = 1.0;
        finish.fade_out = 1.0;
        finish.titles = vec![
            director_titles::Title {
                text: "Aurora Viewer\nDirector".to_owned(),
                ..Default::default()
            },
            director_titles::Title {
                text: "A lower third".to_owned(),
                when: director_titles::When::Range,
                from: 1.0,
                to: 4.0,
                place: Place::LowerThird,
                size: 0.045,
                color: [250, 210, 90],
                ..Default::default()
            },
        ];
        log::info!("director demo: fades of 1 s and two titles");
        if mode == "titles" {
            self.director.d.start_delay = 0.0;
            self.director.d.time = 0.0;
            self.director.pending.push(crate::ui::director::Action::Play);
        } else {
            self.director.d.track.set_duration(1.0);
            let options = &mut self.settings.director_record;
            options.render = true;
            options.png = true;
            options.resolution = director_render::Resolution::Hd;
            options.fps = 24;
            self.director_render_request();
            log::info!("director demo: rendering a second of the track as PNG images");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_stands_between_the_format_s_bars() {
        assert_eq!(picture_in((1920.0, 1080.0), None), [0.0, 0.0, 1920.0, 1080.0]);
        let [x, y, w, h] = picture_in((1920.0, 1080.0), Some(2.0));
        assert_eq!((x, w), (0.0, 1920.0));
        assert!((y - 60.0).abs() < 1e-3 && (h - 960.0).abs() < 1e-3);
        let [x, y, w, h] = picture_in((1920.0, 1080.0), Some(1.0));
        assert_eq!((x, y, w, h), (420.0, 0.0, 1080.0, 1080.0));
        assert_eq!(picture_in((0.0, 0.0), Some(f32::NAN)), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_film_has_sound_only_with_an_encoder_and_a_channel() {
        let tool = Tool::with(&["libx264"]);
        let mut options = recorder::Options::default();
        assert_eq!(sound_for(&options, &tool, true), Some("aac"));
        assert_eq!(sound_for(&options, &tool, false), None, "no audio engine");
        options.format = recorder::Format::Webm;
        assert_eq!(sound_for(&options, &tool, true), None, "no Opus in that ffmpeg");
        options.format = recorder::Format::Mp4;
        options.sound = false;
        assert_eq!(sound_for(&options, &tool, true), None);
        assert!(options.sound_channels().is_empty());
        options.sound = true;
        let channels = options.sound_channels();
        assert!(!channels.contains(&aurora_audio::Channel::Voice), "voice only when asked");
        assert!(!channels.contains(&aurora_audio::Channel::Ui));
        options.voice = true;
        assert!(options.sound_channels().contains(&aurora_audio::Channel::Voice));
    }
}
