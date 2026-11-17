//! The Director in the app (`crate::director`, `ui/director.rs`): the view
//! it takes for a keyframe, the camera it moves to one, the playback that
//! hides the interface and the HUDs, the keys that end it, and its demo.

use super::App;
use crate::camera::track::Pose;
use crate::director::{self, Aim, Director, History, KeyAim, Mode, Target};
use crate::keybinds::Mods;
use crate::recorder::{self, Finishing, Recording, Sink, Tool};
use crate::ui;
use crate::ui::director::Action;
use crate::ui::environment::EnvAction;
use aurora_render::Renderer;
use glam::Vec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;
use winit::keyboard::{Key, KeyCode};

/// "Go to" with the avatar's camera: how far ahead of the keyframe the
/// alt-camera's focus is put, metres.
const GO_TO_FOCUS: f32 = 5.0;

/// The ffmpeg the films are recorded with (`recorder.rs`).
#[derive(Default)]
enum Ffmpeg {
    /// Not looked for yet.
    #[default]
    Unknown,
    /// Found, and being asked what it encodes.
    Asking(Receiver<Result<Tool, String>>),
    Ready(Tool),
    /// None on the PATH nor chosen, or the one there did not answer.
    Missing,
}

/// What a thread did with the tracks' files.
enum Filed {
    /// The names of the tracks saved.
    Listed(Vec<String>),
    Saved(String),
    /// A track's file, to be read.
    Loaded(String, Vec<u8>),
    Failed(String),
}

/// The names of the tracks saved in `folder`, sorted.
fn saved_tracks(folder: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
        .filter_map(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// The Director's track and what the app keeps beside it.
#[derive(Default)]
pub(super) struct DirectorState {
    pub(super) d: Director,
    /// The window's "Free camera" switch (the photo window's free camera,
    /// `App::photo_free_camera`).
    pub(super) free: bool,
    /// What the timeline strip asked for this frame: it is drawn with the
    /// bars, while the skin is borrowed, and the window's turn applies it.
    pub(super) pending: Vec<Action>,
    /// The view angle and the roll the user had when the window opened:
    /// they come back when it closes.
    before: Option<(f32, f32)>,
    /// Where what the view looks at now is this frame (`Director::current_aim`),
    /// smoothed; `None` when it looks at nothing or it is not in sight.
    aim: Option<Vec3>,
    /// Where each target the lock and the keyframes look at is this frame,
    /// smoothed (`director::follow`), each on its own; the ones in sight.
    pub(super) aims: HashMap<Aim, Vec3>,
    /// A target asked for is not in sight.
    lost: bool,
    /// The playback hid the mouse pointer.
    pointer_hidden: bool,
    ffmpeg: Ffmpeg,
    /// The file dialog that chooses an ffmpeg, while it is open.
    choosing: Option<Receiver<Option<PathBuf>>>,
    /// The film being recorded, and whether it is the track's playback
    /// (it ends with it) or the live view (it ends when asked).
    recording: Option<(Recording, bool)>,
    /// The film whose file the encoder is finishing.
    finishing: Option<Finishing>,
    /// Record was pressed: start, or stop, with the next frame.
    record_request: bool,
    /// A frame of the film was asked of the renderer this frame.
    record_pending: bool,
    /// The last film: its file, or why it could not be written.
    film: Option<Result<PathBuf, String>>,
    /// The tracks saved for this account (their names), once listed, and
    /// the name typed to save this one under.
    tracks: Option<Vec<String>>,
    track_name: String,
    /// The threads that list, save and load the tracks' files answer here.
    filing: Option<Receiver<Filed>>,
    /// What the last save or load did, or why it failed.
    filed: Option<Result<String, String>>,
    /// The changes of the track, for Ctrl+Z and Ctrl+Y.
    history: History,
    /// The Director has the keyboard (`ui::director::keys`): Delete, Ctrl+Z
    /// and Ctrl+Y are its own.
    keys: bool,
    demo_done: bool,
    /// The demo's film waits for the ffmpeg found to answer.
    demo_record: bool,
}

impl App {
    /// The track plays: the world alone, the interface out of sight.
    pub(super) fn director_playing(&self) -> bool {
        self.director.d.playing()
    }

    /// The HUDs go with the interface while the track plays, unless RLVa
    /// keeps one shown.
    pub(super) fn director_hides_huds(&self) -> bool {
        (self.director.d.playing() || self.director_filming()) && self.director.d.hide_huds && !self.world.rlva_has_locked_hud()
    }

    /// A film is being recorded, of the track or of the live view: the world
    /// is drawn alone, as while the track plays. The windows stay for the
    /// live view (the film takes the 3D view before them), but the name
    /// tags, the lock's ring, the guides and every other mark laid on the
    /// world are drawn into that view (to be hidden behind what is nearer)
    /// and would be filmed with it.
    pub(super) fn director_filming(&self) -> bool {
        self.director.recording.is_some()
    }

    /// The film being recorded keeps the transparency of the isolation's
    /// empty background (a format with alpha): the frames carry it.
    pub(super) fn director_cutout_alpha(&self) -> bool {
        self.director.recording.is_some() && self.settings.director_record.format.alpha() && self.world.photo_isolation.empty_background()
    }

    /// The ffmpeg chosen by hand, else the PATH's, asked on a thread what it
    /// encodes (it is a program to run: never on the frame's thread).
    fn director_find_ffmpeg(&mut self) {
        let chosen = self.settings.director_record.ffmpeg.clone();
        let path = chosen.or_else(|| std::env::var_os("PATH").and_then(|path| recorder::find_on_path(&path)));
        self.director.ffmpeg = match path {
            None => Ffmpeg::Missing,
            Some(path) => {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(Tool::probe(&path));
                });
                Ffmpeg::Asking(rx)
            }
        };
    }

    /// The Director's lens while its window is open with its depth of
    /// field on: its f-number, and the distance it is focused at along the
    /// view (the lock's target when one is in sight, else the distance set).
    pub(super) fn director_dof(&self) -> Option<(f32, f32)> {
        let f_number = self.director.d.dof.filter(|_| self.panels.director && self.in_world())?;
        let (eye, at) = (self.camera.position, self.camera.target - self.camera.position);
        let distance = match self.director.aim {
            Some(point) => crate::dof::distance_along(eye, at, point),
            None => self.director.d.focus,
        };
        Some((
            f_number,
            if distance.is_finite() {
                distance.max(0.1)
            } else {
                self.director.d.focus
            },
        ))
    }

    /// The folder of this account's saved tracks.
    fn director_tracks_folder(&self) -> PathBuf {
        crate::settings::account_dir(&self.world.agent_id).join("director")
    }

    /// Run `work` on a thread, its answers taken by `director_tracks_poll`
    /// (one such thread at a time: the last asked replaces the others).
    fn director_file(&mut self, work: impl FnOnce(&std::sync::mpsc::Sender<Filed>) + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || work(&tx));
        self.director.filing = Some(rx);
    }

    /// Save the track under the name typed: a JSON file of the account's
    /// `director` folder, written on a thread, then the list read again.
    fn director_save_track(&mut self) {
        let (Some(stem), Some(origin)) = (crate::camera::track::file_stem(&self.director.track_name), self.world.main_origin()) else {
            self.director.filed = Some(Err(aurora_i18n::tr!("director-track-bad-name").to_owned()));
            return;
        };
        let bytes = match serde_json::to_vec_pretty(&self.director.d.track.to_file(origin)) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.director.filed = Some(Err(e.to_string()));
                return;
            }
        };
        let folder = self.director_tracks_folder();
        self.director_file(move |tx| {
            let path = folder.join(format!("{stem}.json"));
            let written = std::fs::create_dir_all(&folder).and_then(|()| crate::atomic_file::write(&path, &bytes));
            let _ = tx.send(match written {
                Ok(()) => Filed::Saved(stem),
                Err(e) => Filed::Failed(format!("{}: {e}", path.display())),
            });
            let _ = tx.send(Filed::Listed(saved_tracks(&folder)));
        });
    }

    /// Load the saved track at `index` of the list: its file read on a
    /// thread, the track replaced when it comes back.
    fn director_load_track(&mut self, index: usize) {
        let Some(name) = self.director.tracks.as_ref().and_then(|names| names.get(index)).cloned() else {
            return;
        };
        let path = self.director_tracks_folder().join(format!("{name}.json"));
        self.director_file(move |tx| {
            let _ = tx.send(match std::fs::read(&path) {
                Ok(bytes) => Filed::Loaded(name, bytes),
                Err(e) => Filed::Failed(format!("{}: {e}", path.display())),
            });
        });
    }

    /// Each frame: the saved tracks listed once the window opens, and what
    /// the files' threads answered. A loaded file is data from outside: a
    /// track only when it reads as one (`Track::from_file`).
    fn director_tracks_poll(&mut self, open: bool) {
        if open && self.director.tracks.is_none() && self.director.filing.is_none() {
            self.director.tracks = Some(Vec::new());
            let folder = self.director_tracks_folder();
            self.director_file(move |tx| {
                let _ = tx.send(Filed::Listed(saved_tracks(&folder)));
            });
        }
        let Some(answers) = &self.director.filing else {
            return;
        };
        let answer = match answers.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.director.filing = None;
                return;
            }
        };
        match answer {
            Filed::Listed(names) => self.director.tracks = Some(names),
            Filed::Saved(name) => {
                log::info!("director: track saved as {name}");
                self.director.filed = Some(Ok(aurora_i18n::tr!("director-track-saved", name = name.as_str())));
            }
            Filed::Failed(why) => {
                log::warn!("director: track file: {why}");
                self.director.filed = Some(Err(aurora_i18n::tr!("director-track-failed", why = why.as_str())));
            }
            Filed::Loaded(name, bytes) => {
                let older = self.director.d.lock.aim();
                let track = serde_json::from_slice::<crate::camera::track::TrackFile>(&bytes)
                    .ok()
                    .zip(self.world.main_origin())
                    .and_then(|(file, origin)| crate::camera::track::Track::from_file(&file, origin, older));
                self.director.filed = Some(match track {
                    Some(track) => {
                        log::info!("director: track {name} loaded, {} keyframes", track.len());
                        self.director.d.load(track);
                        self.director.track_name = name.clone();
                        Ok(aurora_i18n::tr!("director-track-loaded", name = name.as_str()))
                    }
                    None => Err(aurora_i18n::tr!("director-track-failed", why = name.as_str())),
                });
            }
        }
    }

    /// Each frame: the answers of the threads (the ffmpeg asked, the file
    /// dialog, the encoder finishing a film).
    fn director_record_poll(&mut self, open: bool) {
        if open && matches!(self.director.ffmpeg, Ffmpeg::Unknown) {
            self.director_find_ffmpeg();
        }
        if let Ffmpeg::Asking(answer) = &self.director.ffmpeg {
            match answer.try_recv() {
                Ok(Ok(tool)) => {
                    log::info!("director: ffmpeg at {}", tool.path.display());
                    self.director.ffmpeg = Ffmpeg::Ready(tool);
                }
                Ok(Err(why)) => {
                    log::warn!("director: ffmpeg did not answer ({why})");
                    self.director.ffmpeg = Ffmpeg::Missing;
                }
                Err(TryRecvError::Disconnected) => self.director.ffmpeg = Ffmpeg::Missing,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(dialog) = &self.director.choosing {
            match dialog.try_recv() {
                Ok(picked) => {
                    self.director.choosing = None;
                    if let Some(path) = picked {
                        self.settings.director_record.ffmpeg = Some(path);
                        self.settings.save();
                        self.director.ffmpeg = Ffmpeg::Unknown;
                    }
                }
                Err(TryRecvError::Disconnected) => self.director.choosing = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(done) = self.director.finishing.as_mut().and_then(Finishing::done) {
            self.director.finishing = None;
            match &done {
                Ok((file, frames)) => log::info!("director: film of {frames} frames written to {}", file.display()),
                Err(why) => log::warn!("director: the film could not be written ({why})"),
            }
            self.director.film = Some(done.map(|(file, _)| file));
        }
    }

    /// Start a film of frames of `size`: through the ffmpeg found, in the
    /// format chosen, into a new file of the films' folder. With a track to
    /// play, the film is its playback (no start delay: nothing else films);
    /// without one, the live view until Record is pressed again.
    fn director_record_start(&mut self, size: (u32, u32)) {
        let options = self.settings.director_record.clone();
        let (format, fps) = (options.format, options.rate());
        // AURORA_DEMO_DIRECTOR_FILM: the raw frames into that file, no encoder
        let raw = self
            .demo
            .then(|| std::env::var_os("AURORA_DEMO_DIRECTOR_FILM"))
            .flatten()
            .map(PathBuf::from);
        let (sink, out) = match (raw, &self.director.ffmpeg) {
            (Some(raw), _) => (Sink::Raw(raw.clone()), raw),
            (None, Ffmpeg::Ready(tool)) => {
                let Some(encoder) = tool.encoder(format) else {
                    self.director.film = Some(Err(aurora_i18n::tr!("director-record-no-encoder").to_owned()));
                    return;
                };
                let folder = recorder::default_folder();
                if let Err(e) = std::fs::create_dir_all(&folder) {
                    self.director.film = Some(Err(format!("{}: {e}", folder.display())));
                    return;
                }
                let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
                let out = recorder::free_path(&folder, &stamp, format);
                let crop = recorder::picture(size, self.director.d.format.aspect(), format);
                let args = recorder::arguments(encoder, format, options.quality, size, crop, fps, &out);
                (Sink::Ffmpeg(tool.path.clone(), args), out)
            }
            _ => return,
        };
        let with_track = self.director.d.can_play() && !self.director_restricted();
        if with_track {
            self.director_play();
            if !self.director.d.playing() {
                return;
            }
            self.director.d.mode = Mode::Playing { wait: 0.0, hold: None };
        }
        log::info!(
            "director: recording {}x{} at {fps} frames a second to {} ({})",
            size.0,
            size.1,
            out.display(),
            if with_track { "the track" } else { "the live view" }
        );
        self.director.film = None;
        self.director.recording = Some((Recording::start(sink, size, fps, out), with_track));
    }

    /// End the film: the encoder finishes its file on its own thread.
    fn director_record_stop(&mut self) {
        if let Some((recording, _)) = self.director.recording.take() {
            if recording.skipped > 0 {
                log::info!("director: {} frames repeated, the encoder was behind", recording.skipped);
            }
            let finishing = recording.finish();
            log::info!("director: recording ended, {} frames to finish", finishing.frames);
            self.director.finishing = Some(finishing);
        }
    }

    /// Before the frame is drawn: Record pressed starts or stops the film;
    /// a film of the track ends with its playback (before the hold on the
    /// last view), any film with the window; and when a frame of the film is
    /// due, this frame's view is asked of the renderer (unless something
    /// else captures it: that frame of the film is then the next one's).
    pub(super) fn director_record_frame(&mut self, renderer: &mut Renderer) {
        if std::mem::take(&mut self.director.record_request) {
            if self.director.recording.is_some() {
                self.director_record_stop();
            } else if self.director.finishing.is_none() {
                self.director_record_start(renderer.size());
            }
            // the film's first frame is the next one: this frame was laid
            // out before the recording began (its HUDs, its transparency)
            return;
        }
        let Some((recording, with_track)) = &self.director.recording else {
            return;
        };
        let playing = matches!(self.director.d.mode, Mode::Playing { hold: None, .. });
        if recording.failed() || (*with_track && !playing) || !self.panels.director {
            self.director_record_stop();
            return;
        }
        let free = !renderer.capture_scene
            && !renderer.capture_request
            && !self.scene_capture_pending
            && self.inventory_images.capturing.is_none();
        if free && recording.wants_frame(Instant::now()) {
            renderer.capture_scene = true;
            self.director.record_pending = true;
        }
    }

    /// After the frame is drawn: the view asked for the film goes to it. A
    /// view of another size (the window was resized) ends the film: its
    /// frames all have one size.
    pub(super) fn director_recorded(&mut self, renderer: &mut Renderer) {
        if !std::mem::take(&mut self.director.record_pending) {
            return;
        }
        let Some((width, height, pixels)) = renderer.captured.take() else {
            return;
        };
        let kept = self
            .director
            .recording
            .as_mut()
            .is_none_or(|(recording, _)| recording.frame(Instant::now(), (width, height), pixels));
        if !kept {
            log::info!("director: the window changed size, the recording ends");
            self.director_record_stop();
        }
    }

    /// The key color (linear RGB) the isolation's empty background shows
    /// while the Director is open: a green screen to film against.
    pub(super) fn director_key_color(&self) -> Option<[f32; 3]> {
        self.director.d.key_color.filter(|_| self.panels.director).map(director::linear)
    }

    /// The track may not take the view: RLVa restricts the camera, or the
    /// view is a headset's.
    fn director_restricted(&self) -> bool {
        self.world.rlva.camera_restricted() || self.vr_camera.is_some()
    }

    /// The current view, as a keyframe keeps it.
    fn director_pose(&self) -> Pose {
        Pose::from_view(
            self.camera.position,
            self.camera.forward(),
            self.camera.view_up(),
            self.camera.fov_y,
        )
    }

    /// Each frame, before the camera updates: the view angle and the roll
    /// kept for the window's closing, the playback's clock, and the view the
    /// track takes or gives back.
    pub(super) fn director_frame(&mut self, dt: f32) {
        self.director_demo();
        // (what a frame without the window's turn left behind)
        let pending = std::mem::take(&mut self.director.pending);
        self.director_actions(pending);
        // what the last frame changed in the track is a step to undo, a
        // whole drag one step
        let at_rest = !self.egui_ctx.input(|i| i.pointer.any_down());
        self.director.history.observe(&self.director.d, at_rest);
        let open = self.panels.director && self.in_world();
        match (open, self.director.before) {
            (true, None) => self.director.before = Some((self.settings.camera.fov, self.camera.roll)),
            (false, Some((fov, roll))) => {
                self.director.before = None;
                self.camera.roll = roll;
                if (self.settings.camera.fov - fov).abs() > 1e-6 {
                    self.settings.camera.fov = fov;
                    self.settings.save();
                    log::info!("director: field of view back to {:.1}°", fov.to_degrees());
                }
            }
            _ => {}
        }
        if !open {
            self.director.free = false;
        }
        self.director_record_poll(open);
        self.director_tracks_poll(open);
        let was = self.director.d.mode;
        if !open || self.world.teleporting || self.director_restricted() {
            self.director.d.stop();
        }
        self.director.d.tick(dt);
        if matches!(was, Mode::Playing { .. }) && !self.director.d.playing() {
            log::info!("director: playback ended at {:.1} s", self.director.d.time);
        }
        // where the targets are, those the live camera and the keyframes
        // look at: each followed smoothly, found again at once after a loss
        let wanted = if open && !self.director_restricted() {
            self.director.d.aims()
        } else {
            Vec::new()
        };
        let smoothing = self.director.d.lock.smoothing;
        let mut found = HashMap::with_capacity(wanted.len());
        for &aim in &wanted {
            if let Some(to) = self.director_target(aim) {
                found.insert(aim, director::follow(self.director.aims.get(&aim).copied(), to, smoothing, dt));
            }
        }
        self.director.lost = found.len() < wanted.len();
        self.director.aim = self.director.d.current_aim().and_then(|aim| found.get(&aim).copied());
        let view = self.director.d.view(|aim| found.get(&aim).copied());
        self.director.aims = found;
        self.camera.set_track_view(view);
    }

    /// Where a target is now: the joint of the avatar as it was last drawn
    /// (its animation included), or the object's centre. `None` when it is
    /// not in the scene.
    fn director_target(&self, aim: Aim) -> Option<Vec3> {
        let now = Instant::now();
        match aim.target {
            Target::Own => self.scene.joint_world(&self.world, &self.world.agent_id, aim.joint.bone(), now),
            Target::Avatar(id) => self.scene.joint_world(&self.world, &id, aim.joint.bone(), now),
            Target::Object(id) => {
                let index = self.world.objects.index_of_uuid(&id)?;
                crate::scene::Scene::object_transform(&self.world, index, now, 0).map(|(position, ..)| position)
            }
        }
    }

    /// After the camera's update: the live lock turns the user's own camera
    /// to the target, wherever they moved it (`Camera::aim_at`). The track's
    /// view was aimed before it was handed over.
    pub(super) fn director_aim(&mut self) {
        if self.director.d.lock.on
            && self.director.d.mode == Mode::Editing
            && let Some(point) = self.director.aim
        {
            self.camera.aim_at(point);
        }
    }

    /// The mouse pointer goes while the track plays (and comes back with a
    /// question over the picture, or when a camera drag holds it: the hold
    /// shows it again itself when it ends).
    pub(super) fn director_pointer(&mut self, window: &winit::window::Window) {
        if self.cursor_hold.held() {
            self.director.pointer_hidden = false;
            return;
        }
        let hide = self.director.d.playing() && self.director.d.hide_pointer && !self.exit_prompt.open;
        if hide != self.director.pointer_hidden {
            window.set_cursor_visible(!hide);
            self.director.pointer_hidden = hide;
        }
    }

    /// Escape ends a playback or a scrub, or the choosing in the scene the
    /// Director's isolation started; true when it did.
    pub(super) fn director_escape(&mut self) -> bool {
        if self.director.d.mode != Mode::Editing {
            self.director.d.stop();
            return true;
        }
        if self.panels.director && !self.panels.snapshot && self.world.photo_isolation.picking {
            self.world.photo_isolation.set_picking(false);
            return true;
        }
        false
    }

    /// A key while the Director has the keyboard (the last click was on its
    /// timeline, its window or a keyframe in the world) and no field is being
    /// typed in: Delete removes the selected keyframe, Ctrl+Z undoes the
    /// track's last change, Ctrl+Y or Ctrl+Shift+Z does it again. The letters
    /// are the layout's (Z is not where a QWERTY keyboard has it everywhere).
    /// True when the key was the Director's.
    pub(super) fn director_key(&mut self, code: KeyCode, logical: &Key, pressed: bool, mods: Mods) -> bool {
        if !self.panels.director || !self.director.keys || self.director.d.playing() {
            return false;
        }
        let letter = |wanted: &str| matches!(logical, Key::Character(text) if text.eq_ignore_ascii_case(wanted));
        let undo = mods.ctrl && !mods.alt && !mods.shift && letter("z");
        let redo = mods.ctrl && !mods.alt && (letter("y") || (mods.shift && letter("z")));
        let delete = code == KeyCode::Delete && !mods.ctrl && !mods.alt && self.director.d.selected.is_some();
        if !(undo || redo || delete) {
            return false;
        }
        if pressed {
            if undo {
                self.director.history.undo(&mut self.director.d);
            } else if redo {
                self.director.history.redo(&mut self.director.d);
            } else if let Some(index) = self.director.d.selected {
                self.director.d.remove(index);
            }
        }
        // (the release is the Director's too: it must not reach a shortcut)
        true
    }

    /// The live camera and its restrictions, for the window and the strip.
    pub(super) fn director_live(&self) -> ui::director::Live {
        ui::director::Live {
            fov: self.settings.camera.fov,
            roll: self.camera.roll,
            free: self.camera.free().is_some(),
            free_speed: self.snapshots.ui.free.speed,
            lens_locked: self.world.rlva.camera_owner().is_some(),
            restricted: self.director_restricted(),
            focus: self.camera.focus_object_id().is_some(),
            target: self.director.aim,
            lost: self.director.lost,
            can_undo: self.director.history.can_undo(),
            can_redo: self.director.history.can_redo(),
            can_record: matches!(self.director.ffmpeg, Ffmpeg::Ready(_))
                || (self.demo && std::env::var_os("AURORA_DEMO_DIRECTOR_FILM").is_some()),
            recording: self.director.recording.is_some(),
        }
    }

    /// The track in the world, with the world's other marks (under the
    /// bars and the windows).
    pub(super) fn director_world(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, projector: &ui::hud::Projector) {
        if !self.panels.director {
            return;
        }
        let mut actions = Vec::new();
        let aspect = self.director.d.picture_aspect(self.camera_aspect());
        ui::director::guides(ctx, p, projector, &self.director.d, self.director.aim);
        // choosing in the scene for the isolation, as the photo mode does
        let (_, hover) = self.photo_isolate_rows();
        let mut chosen = Vec::new();
        ui::director::isolate_picking(ctx, p, &self.world.photo_isolation, hover.as_ref(), &mut chosen);
        for choice in chosen {
            self.photo_isolate_out(choice, ctx.pixels_per_point());
        }
        ui::director::world(ctx, p, projector, &mut self.director.d, aspect, &self.director.aims, &mut actions);
        self.director_actions(actions);
    }

    /// The window, drawn with Quick preferences: the choices of its
    /// environment block come back for `env_actions`. What the timeline
    /// strip asked for earlier in the frame is applied first.
    pub(super) fn director_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, lighting: &mut bool) -> Vec<EnvAction> {
        let mut env = Vec::new();
        let mut actions = std::mem::take(&mut self.director.pending);
        let live = self.director_live();
        let (rows, _) = self.photo_isolate_rows();
        let mut isolate = Vec::new();
        let (ffmpeg, formats) = match &self.director.ffmpeg {
            Ffmpeg::Ready(tool) => (
                Some(Some(tool.path.as_path())),
                recorder::Format::ALL.map(|format| tool.encoder(format).is_some()),
            ),
            Ffmpeg::Missing => (Some(None), [false; 5]),
            Ffmpeg::Unknown | Ffmpeg::Asking(_) => (None, [false; 5]),
        };
        let mut record_changed = false;
        let mut open = self.panels.director;
        ui::director::window(
            ctx,
            p,
            &mut open,
            &mut self.director.d,
            &live,
            ui::director::IsolateView {
                isolation: &self.world.photo_isolation,
                rows: &rows,
                out: &mut isolate,
            },
            ui::director::Record {
                options: &mut self.settings.director_record,
                changed: &mut record_changed,
                ffmpeg,
                formats,
                recording: self.director.recording.as_ref().map(|(film, _)| film.elapsed(Instant::now())),
                finishing: self.director.finishing.is_some(),
                last: self
                    .director
                    .film
                    .as_ref()
                    .map(|film| film.as_ref().map(PathBuf::as_path).map_err(String::as_str)),
            },
            ui::director::Tracks {
                name: &mut self.director.track_name,
                saved: self.director.tracks.as_deref().unwrap_or_default(),
                note: self
                    .director
                    .filed
                    .as_ref()
                    .map(|note| note.as_ref().map(String::as_str).map_err(String::as_str)),
            },
            ui::director::Env {
                ui: &mut self.env_ui,
                view: self.world.eep.local_view(),
                loading: self.world.eep.local_loading(),
                time_of_day: self.panels.time_of_day,
                lighting,
                out: &mut env,
            },
            &mut actions,
        );
        self.panels.director = open;
        if record_changed {
            self.settings.save();
        }
        self.director_actions(actions);
        for choice in isolate {
            self.photo_isolate_out(choice, ctx.pixels_per_point());
        }
        self.director.keys = open && ui::director::keys(ctx, self.director.keys, self.director.d.track.len());
        env
    }

    /// While the track plays: the countdown of its start delay, nothing else.
    pub(super) fn director_playing_ui(&self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let recording = self.director.recording.as_ref().map(|(film, _)| film.elapsed(Instant::now()));
        ui::director::playing(ctx, p, &self.director.d, recording);
    }

    fn director_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::AddKey => {
                    let pose = self.director_pose();
                    match self.director.d.add_key(pose) {
                        Some(index) => log::info!(
                            "director: keyframe {} at {:.1} s, {:.1?}, {:.0}°",
                            index + 1,
                            self.director.d.time,
                            pose.position,
                            pose.fov.to_degrees()
                        ),
                        None => log::info!("director: the track is full"),
                    }
                }
                Action::Update(index) => {
                    let pose = self.director_pose();
                    self.director.d.track.set_pose(index, pose);
                }
                Action::GoTo(index) => self.director_go_to(index),
                Action::Play => self.director_play(),
                Action::Lens(fov) => self.director_lens(fov),
                Action::Roll(roll) => self.camera.roll = roll,
                Action::Free(on) => self.director.free = on,
                Action::FreeSpeed(speed) => self.snapshots.ui.free.speed = speed,
                Action::LockFocus => self.director_lock_focus(),
                Action::Record => self.director.record_request = true,
                Action::SaveTrack => self.director_save_track(),
                Action::LoadTrack(index) => self.director_load_track(index),
                Action::ChooseFfmpeg => {
                    if self.director.choosing.is_none() {
                        // (a dialog blocks its thread: not the frame's)
                        let (tx, rx) = std::sync::mpsc::channel();
                        let title = aurora_i18n::tr!("director-record-choose");
                        std::thread::spawn(move || {
                            let _ = tx.send(rfd::FileDialog::new().set_title(title).pick_file());
                        });
                        self.director.choosing = Some(rx);
                    }
                }
                Action::FindFfmpeg => self.director.ffmpeg = Ffmpeg::Unknown,
                Action::FfmpegSite => ui::chat::open_web_link(&self.egui_ctx, recorder::DOWNLOAD_PAGE),
                Action::RevealFilm => {
                    if let Some(Ok(file)) = &self.director.film {
                        crate::snapshot::gallery::reveal(&recorder::default_folder(), Some(file));
                    }
                }
                Action::Undo => {
                    self.director.history.undo(&mut self.director.d);
                }
                Action::Redo => {
                    self.director.history.redo(&mut self.director.d);
                }
            }
        }
    }

    /// Lock the view on what the camera is focused on (Alt + click): the
    /// avatar it is or belongs to (a worn body is its wearer), else the
    /// object, by its root prim. The lock turns on.
    fn director_lock_focus(&mut self) {
        let Some(focus) = self.camera.focus_object_id() else {
            return;
        };
        let root = self
            .world
            .objects
            .index_of_uuid(&focus)
            .and_then(|index| crate::snapshot::isolate::root_of(&self.world, index));
        let Some((index, id)) = root else {
            return;
        };
        let avatar = self.world.objects.get(index).is_some_and(|object| object.is_avatar());
        self.director.d.lock.target = match (avatar, id == self.world.agent_id) {
            (true, true) => Target::Own,
            (true, false) => Target::Avatar(id),
            (false, _) => Target::Object(id),
        };
        self.director.d.lock.on = !self.director_restricted();
        log::info!(
            "director: view locked on {}",
            match self.director.d.lock.target {
                Target::Own => "our avatar",
                Target::Avatar(_) => "another avatar",
                Target::Object(_) => "an object",
            }
        );
    }

    /// The view angle of the camera (the setting, as its shortcuts change
    /// it), within the window's limits and RLVa's; the user's own comes back
    /// when the Director closes.
    fn director_lens(&mut self, fov: f32) {
        if self.world.rlva.camera_owner().is_some() {
            return;
        }
        let aspect = self.camera_aspect();
        let limits = self.world.rlva.camera_fov_limits(aspect);
        self.settings.camera.fov = crate::camera::zoom_lens(fov, Some(1.0), aspect, limits);
        if let Some(flight) = &mut self.camera.flycam {
            flight.set_fov(self.settings.camera.fov);
        }
    }

    /// The camera to a keyframe, to look through it or adjust it: the
    /// flycam or the free camera where they fly; else the alt-camera, put
    /// there like the `pstcampos` chat command does. Not while RLVa
    /// restricts the camera.
    fn director_go_to(&mut self, index: usize) {
        let Some(key) = self.director.d.track.keys().get(index).copied() else {
            return;
        };
        self.director.d.select(index);
        if self.director_restricted() {
            return;
        }
        let pose = key.pose;
        if self.camera.mouselook() {
            self.toggle_mouselook();
        }
        if let Some(flight) = &mut self.camera.flycam {
            flight.position = pose.position;
            flight.rotation = pose.rotation;
        } else if let Some(free) = self.camera.free_mut() {
            *free = crate::camera::FreeCamera::from_view(pose.position, pose.position + pose.forward());
        } else {
            self.camera.set_view(
                pose.position,
                pose.position + pose.forward() * GO_TO_FOCUS,
                &mut self.world.agent,
                &self.settings.camera,
            );
        }
        self.camera.roll = pose.roll();
        self.director_lens(pose.fov);
    }

    /// Play the track: mouselook ends, the HUDs are left if they go, and the
    /// interface is no longer drawn (`director_playing`).
    fn director_play(&mut self) {
        if self.director_restricted() {
            return;
        }
        if self.camera.mouselook() {
            self.toggle_mouselook();
        }
        if !self.director.d.play() {
            return;
        }
        if self.director.d.hide_huds && !self.world.rlva_has_locked_hud() {
            self.leave_huds();
        }
        log::info!(
            "director: playing {} keyframes, {:.1} s from {:.1} s, start delay {:.1} s",
            self.director.d.track.len(),
            self.director.d.track.duration(),
            self.director.d.time,
            self.director.d.start_delay
        );
    }

    /// AURORA_DEMO_DIRECTOR: once in the world, the window opened on a
    /// track of four keyframes around our avatar; `play` plays it at once,
    /// without a start delay; `lock` locks the view on our avatar's neck,
    /// live and all along the track, its second keyframe on the neck and its
    /// third on the head (targets of their own, kept when the lock goes
    /// off), the fourth's target cleared, in a 2.39:1 frame with the rule
    /// of thirds; `green` shows our avatar alone on a green screen.
    fn director_demo(&mut self) {
        if !self.demo {
            return;
        }
        if self.director.demo_done {
            self.director_demo_record();
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_DIRECTOR") else {
            self.director.demo_done = true;
            return;
        };
        if self.frame_count < 620 || !matches!(self.screen, super::Screen::World) {
            return;
        }
        self.director.demo_done = true;
        let subject = self.world.agent.position + Vec3::Z * 1.0;
        for (angle, radius, height, fov) in [
            (-2.4_f32, 9.0, 1.0, 1.05),
            (-1.2, 7.0, 2.5, 0.9),
            (0.0, 5.0, 0.6, 0.6),
            (1.3, 8.0, 3.5, 1.0),
        ] {
            let eye = subject + Vec3::new(angle.cos() * radius, angle.sin() * radius, height);
            self.director.d.add_key(Pose::from_view(eye, subject - eye, Vec3::Z, fov));
        }
        self.director.d.select(2);
        ui::widgets::open_floater(&self.egui_ctx, ui::director::WINDOW, &mut self.panels.director);
        log::info!(
            "director demo: {} keyframes over {:.1} s, {:.1} m",
            self.director.d.track.len(),
            self.director.d.track.duration(),
            self.director.d.track.length()
        );
        if mode == "play" {
            self.director.d.start_delay = 0.0;
            self.director.d.time = 0.0;
            self.director_play();
        }
        if mode == "lock" {
            let d = &mut self.director.d;
            d.lock.on = true;
            // the second keyframe on the neck, the third on the head (a
            // target of its own), the fourth's target cleared
            let own = |joint| {
                KeyAim::On(Aim {
                    target: Target::Own,
                    joint,
                })
            };
            d.track.set_aim(1, own(director::Joint::Neck));
            d.track.set_aim(2, own(director::Joint::Head));
            d.track.set_aim(3, KeyAim::Free);
            d.format = director::Format::Scope;
            d.thirds = true;
            log::info!("director demo: view locked on our avatar's neck, 2.39:1");
        }
        if mode.starts_with("record") {
            // a second of film, through the ffmpeg found or, with
            // AURORA_DEMO_DIRECTOR_FILM, as raw frames
            self.director.d.track.set_duration(1.0);
            self.director.d.time = 0.0;
            self.settings.director_record.fps = 24;
            if mode == "record-alpha" {
                let isolation = &mut self.world.photo_isolation;
                isolation.set_mode(crate::snapshot::isolate::Mode::Own);
                isolation.set_transparent(true);
                self.settings.director_record.format = recorder::Format::ProResAlpha;
            }
            self.director.demo_record = true;
            log::info!("director demo: recording a second of the track");
        }
        if mode == "green" {
            let isolation = &mut self.world.photo_isolation;
            isolation.set_mode(crate::snapshot::isolate::Mode::Own);
            isolation.set_transparent(true);
            self.director.d.key_color = Some(director::KEY_GREEN);
            log::info!("director demo: our avatar alone on a green screen");
        }
    }

    /// The demo's film starts once there is something to write it with: the
    /// raw file at once, the ffmpeg found when it has said what it encodes
    /// (it is asked on a thread when the window opens).
    fn director_demo_record(&mut self) {
        if !self.director.demo_record {
            return;
        }
        let raw = std::env::var_os("AURORA_DEMO_DIRECTOR_FILM").is_some();
        let ready = raw || matches!(self.director.ffmpeg, Ffmpeg::Ready(_));
        let missing = !raw && matches!(self.director.ffmpeg, Ffmpeg::Missing);
        if missing {
            log::warn!("director demo: no ffmpeg to record with");
        }
        if ready || missing {
            self.director.demo_record = false;
            self.director.record_request = ready;
        }
    }
}
