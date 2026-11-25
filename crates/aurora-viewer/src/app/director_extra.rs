//! The Director's path taken by hand, its keyframes' animations, lens and
//! time of day, and its keyframes dragged in the world, in the app
//! (`director_capture`, `director_anim`, `director_keys`,
//! `ui::director_drag`; the Director itself is `app/director.rs`).
//!
//! An animation plays as the inventory's "Play in world" does
//! (LLPreviewAnim::play, indra/newview/llpreviewanim.cpp): its asset asked
//! of the simulator with AgentAnimation (LLAgent::sendAnimationRequest,
//! ANIM_REQUEST_START, then ANIM_REQUEST_STOP), one message each; which
//! animation may play is the shared rule (`actions::animation_playable`).
//! The Legacy Viewer's RLVa has no restriction on playing an animation (its
//! @sendgesture is for gestures alone), and the track plays nothing while
//! RLVa restricts the camera, since it does not play then. The time of day
//! is the photo window's sun over the local environment (`set_photo`):
//! never sent to the region, refused under RLVa's @setenv, and the user's
//! own light fades back in as soon as the track stops giving one.

use super::App;
use crate::camera::track::{self, Pose};
use crate::director::{KeyAim, Mode};
use crate::director_anim::{Cue, Schedule};
use crate::director_capture::{self, Take, Tolerance};
use crate::ui::director_capture::{CaptureAction, CaptureView};
use crate::ui::director_key_rows::{self, AnimChoice, KeyRows};
use crate::world::inventory::actions as rules;
use aurora_net::NetCommand;
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

/// What the app keeps for these features beside the Director's state.
pub(super) struct DirectorExtra {
    /// The path being taken by hand: when it started, and its views.
    taking: Option<(Instant, Take)>,
    /// The capture's "detail" slider.
    detail: f32,
    /// What the last take gave, or why it gave nothing.
    note: Option<Result<String, String>>,
    /// The animations the playback started.
    schedule: Schedule,
    /// Those asked of the simulator: the item, and the asset it played (the
    /// one its stop asks for, whatever the inventory became meanwhile).
    sent: Vec<(Uuid, Uuid)>,
    /// The animations the keyframe's list offers, for an inventory's
    /// generation.
    choices: Option<(u64, Vec<AnimChoice>)>,
}

impl Default for DirectorExtra {
    fn default() -> Self {
        DirectorExtra {
            taking: None,
            detail: director_capture::DEFAULT_DETAIL,
            note: None,
            schedule: Schedule::default(),
            sent: Vec::new(),
            choices: None,
        }
    }
}

impl DirectorExtra {
    /// The region positions are relative to changed (`Camera::shift`).
    pub(super) fn shift(&mut self, delta: Vec3) {
        if let Some((_, take)) = &mut self.taking {
            take.shift(delta);
        }
    }

    /// A path is being taken.
    pub(super) fn taking(&self) -> bool {
        self.taking.is_some()
    }

    /// What the window shows of them: the section of the path taken by
    /// hand, and the rows of the selected keyframe (the animations of `inv`
    /// it offers listed by `App::director_refresh_choices`).
    pub(super) fn views<'a>(
        &'a mut self,
        inv: &'a crate::world::inventory::Inventory,
        restricted: bool,
        out: &'a mut Vec<CaptureAction>,
    ) -> (CaptureView<'a>, KeyRows<'a>) {
        let capture = CaptureView {
            taking: self
                .taking
                .as_ref()
                .map(|(started, take)| (started.elapsed().as_secs_f32(), take.samples().len())),
            detail: &mut self.detail,
            restricted,
            note: self
                .note
                .as_ref()
                .map(|note| note.as_ref().map(String::as_str).map_err(String::as_str)),
            out,
        };
        let rows = KeyRows {
            inv,
            anims: self.choices.as_ref().map_or(&[], |(_, list)| list.as_slice()),
        };
        (capture, rows)
    }
}

impl App {
    /// Each frame, after the playback's clock: the path being taken gets
    /// the live view, and the keyframes the playback passed start and stop
    /// their animations.
    pub(super) fn director_extra_frame(&mut self, open: bool) {
        if self.director.extra.taking() {
            if !open || self.world.teleporting || self.director_restricted() {
                log::info!("director: the path taken by hand is dropped (window closed, teleport or RLVa)");
                self.director.extra.taking = None;
                self.director.extra.note = Some(Err(aurora_i18n::tr!("director-capture-dropped").to_owned()));
            } else {
                // the live view only: a scrub would take the track's
                if self.director.d.mode != Mode::Editing {
                    self.director.d.stop();
                }
                let pose = self.director_pose();
                let full = self
                    .director
                    .extra
                    .taking
                    .as_mut()
                    .is_some_and(|(started, take)| !take.sample(started.elapsed().as_secs_f32(), pose));
                if full {
                    self.director_capture_stop();
                }
            }
        }
        let cues = self.director.extra.schedule.update(&self.director.d);
        for cue in cues {
            self.director_cue(cue);
        }
    }

    /// An animation the track starts or stops on our avatar.
    fn director_cue(&mut self, cue: Cue) {
        match cue {
            Cue::Start(item) => {
                let Some(asset) = rules::animation_playable(&self.world.inventory, item).map(|it| it.asset_id) else {
                    log::info!("director: the animation of item {item} is not in the inventory, skipped");
                    return;
                };
                log::info!("director: animation of item {item} started");
                self.director_animation(asset, true);
                self.director.extra.sent.push((item, asset));
            }
            Cue::Stop(item) => {
                if let Some(at) = self.director.extra.sent.iter().position(|(sent, _)| *sent == item) {
                    let (_, asset) = self.director.extra.sent.remove(at);
                    log::info!("director: animation of item {item} stopped");
                    self.director_animation(asset, false);
                }
            }
        }
    }

    /// LLPreviewAnim::play "Inworld": AgentAnimation, start or stop (in the
    /// demo, played on our avatar here too, as the inventory's preview does).
    fn director_animation(&mut self, asset: Uuid, start: bool) {
        if self.demo {
            self.world.preview_animation(asset, start);
        }
        self.send(NetCommand::AgentAnimation { anim: asset, start });
    }

    /// Escape drops the path being taken; true when there was one.
    pub(super) fn director_extra_escape(&mut self) -> bool {
        if self.director.extra.taking.take().is_some() {
            log::info!("director: the path taken by hand is cancelled");
            self.director.extra.note = None;
            return true;
        }
        false
    }

    fn director_capture_start(&mut self) {
        if self.director_restricted() || self.director.d.track.is_full() || self.director.extra.taking() {
            return;
        }
        if self.camera.mouselook() {
            self.toggle_mouselook();
        }
        self.director.d.stop();
        self.director.extra.note = None;
        let mut take = Take::default();
        take.sample(0.0, self.director_pose());
        self.director.extra.taking = Some((Instant::now(), take));
        log::info!("director: taking a path by hand");
    }

    /// The take becomes keyframes laid after the end of the track (the
    /// track when it is empty): one change, one step to undo. Looking where
    /// the take looked: at the lock's target when it was on, else exactly
    /// where the camera was turned.
    fn director_capture_stop(&mut self) {
        let Some((_, take)) = self.director.extra.taking.take() else {
            return;
        };
        let d = &mut self.director.d;
        let start = if d.track.is_empty() {
            0.0
        } else {
            d.track.duration() + track::DEFAULT_GAP
        };
        let aim = if d.lock.on { KeyAim::On(d.lock.aim()) } else { KeyAim::Free };
        let room = track::MAX_KEYS - d.track.len();
        let keys = director_capture::keyframes(&take, Tolerance::from_detail(self.director.extra.detail), room, start, aim);
        if keys.is_empty() {
            let why = if take.duration() < track::MIN_GAP {
                aurora_i18n::tr!("director-capture-short")
            } else {
                aurora_i18n::tr!("director-capture-full")
            };
            log::info!("director: the path taken by hand gave no keyframe ({} views)", take.samples().len());
            self.director.extra.note = Some(Err(why.to_owned()));
            return;
        }
        let first = d.track.len();
        let laid = d.track.push_keys(keys);
        if laid > 0 {
            d.select(first + laid - 1);
        }
        log::info!(
            "director: a path of {:.1} s taken by hand, {} views, became {laid} keyframes",
            take.duration(),
            take.samples().len()
        );
        let time = crate::ui::director::clock(take.duration(), true);
        self.director.extra.note = Some(Ok(aurora_i18n::tr!("director-capture-done", count = laid, time = time.as_str())));
    }

    pub(super) fn director_capture_actions(&mut self, actions: Vec<CaptureAction>) {
        for action in actions {
            match action {
                CaptureAction::Start => self.director_capture_start(),
                CaptureAction::Stop => self.director_capture_stop(),
                CaptureAction::Cancel => {
                    self.director_extra_escape();
                }
            }
        }
    }

    /// A keyframe's camera dragged in the world: not while RLVa restricts
    /// the camera, as "Go to" does not move it then.
    pub(super) fn director_move_key(&mut self, index: usize, pose: Pose) {
        if !self.director_restricted() {
            self.director.d.track.set_pose(index, pose);
        }
    }

    /// The animations the keyframe's list offers, listed again when the
    /// inventory changed.
    pub(super) fn director_refresh_choices(&mut self) {
        let inv = &self.world.inventory;
        if self
            .director
            .extra
            .choices
            .as_ref()
            .is_none_or(|(generation, _)| *generation != inv.generation)
        {
            self.director.extra.choices = Some((inv.generation, director_key_rows::anim_choices(inv)));
        }
    }

    /// The sun the track gives the local environment while it plays or its
    /// playhead is held (`director_keys::sun`); `None`: the user's own.
    pub(super) fn director_sun(&self) -> Option<Quat> {
        if !self.panels.director || !self.in_world() || self.director_restricted() {
            return None;
        }
        crate::director_keys::sun(&self.director.d, self.world.eep.unturned_sun())
    }

    /// AURORA_DEMO_DIRECTOR=capture: a path flown by hand around our avatar
    /// (a slow circle that speeds up, the view on the avatar, a small shake),
    /// taken twenty times a second and made keyframes as Stop does; with
    /// `anim` the first keyframe also starts the first animation of the
    /// inventory, which the third stops, and the second keyframe is given its
    /// own lens and the evening.
    pub(super) fn director_extra_demo(&mut self, mode: &str) {
        if mode == "capture" {
            self.director.d.clear();
            let subject = self.world.agent.position + Vec3::Z;
            let mut take = Take::default();
            for step in 0..=160 {
                let time = step as f32 / director_capture::SAMPLE_RATE;
                let angle = time * 0.3 + time * time * 0.02;
                let shake = Vec3::new((time * 13.0).sin(), (time * 11.0).cos(), (time * 17.0).sin()) * 0.01;
                let eye = subject + Vec3::new(angle.cos() * 6.0, angle.sin() * 6.0, 1.5) + shake;
                take.sample(time, Pose::from_view(eye, subject - eye, Vec3::Z, 1.0));
            }
            self.director.extra.taking = Some((Instant::now(), take));
            self.director_capture_stop();
            log::info!(
                "director demo: a path taken by hand, {} keyframes over {:.1} s",
                self.director.d.track.len(),
                self.director.d.track.duration()
            );
        }
        if mode == "anim" && self.director.d.track.len() >= 3 {
            self.director_refresh_choices();
            let first = self.director.extra.choices.as_ref().and_then(|(_, list)| list.first().cloned());
            let d = &mut self.director.d;
            if let Some(choice) = first {
                let dance = crate::director_keys::AnimRef::new(choice.item, &choice.name);
                let mut start = d.track.keys()[0].extra;
                start.start = Some(dance);
                d.track.set_extra(0, start);
                crate::director_anim::set_stop(&mut d.track, 0, Some(2));
                log::info!("director demo: the first keyframe starts {}, the third stops it", choice.name);
            } else {
                log::warn!("director demo: no animation in the inventory");
            }
            let mut lens = d.track.keys()[1].extra;
            lens.aperture = Some(2.0);
            lens.focus = Some(3.0);
            lens.hour = Some(18.5);
            d.track.set_extra(1, lens);
            d.dof = Some(crate::director::DEFAULT_APERTURE);
            d.select(1);
        }
    }
}
