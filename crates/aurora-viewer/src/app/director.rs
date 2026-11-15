//! The Director in the app (`crate::director`, `ui/director.rs`): the view
//! it takes for a keyframe, the camera it moves to one, the playback that
//! hides the interface and the HUDs, the keys that end it, and its demo.

use super::App;
use crate::camera::track::Pose;
use crate::director::{self, Director, Mode, Target};
use crate::ui;
use crate::ui::director::Action;
use crate::ui::environment::EnvAction;
use glam::Vec3;
use std::time::Instant;

/// "Go to" with the avatar's camera: how far ahead of the keyframe the
/// alt-camera's focus is put, metres.
const GO_TO_FOCUS: f32 = 5.0;

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
    /// Where the lock's target is this frame, smoothed (`director::follow`);
    /// `None` when nothing asks for it or it is not in sight.
    aim: Option<Vec3>,
    /// The playback hid the mouse pointer.
    pointer_hidden: bool,
    demo_done: bool,
}

impl App {
    /// The track plays: the world alone, the interface out of sight.
    pub(super) fn director_playing(&self) -> bool {
        self.director.d.playing()
    }

    /// The HUDs go with the interface while the track plays, unless RLVa
    /// keeps one shown.
    pub(super) fn director_hides_huds(&self) -> bool {
        self.director.d.playing() && self.director.d.hide_huds && !self.world.rlva_has_locked_hud()
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
        let was = self.director.d.mode;
        if !open || self.world.teleporting || self.director_restricted() {
            self.director.d.stop();
        }
        self.director.d.tick(dt);
        if matches!(was, Mode::Playing { .. }) && !self.director.d.playing() {
            log::info!("director: playback ended at {:.1} s", self.director.d.time);
        }
        // where the lock's target is, when the live camera or a keyframe
        // asks for it: followed smoothly, found again at once after a loss
        let lock = self.director.d.lock;
        let wanted = open && !self.director_restricted() && (lock.on || self.director.d.track.keys().iter().any(|key| key.locked));
        self.director.aim = wanted
            .then(|| self.director_target())
            .flatten()
            .map(|to| director::follow(self.director.aim, to, lock.smoothing, dt));
        self.camera.set_track_view(self.director.d.view(self.director.aim));
    }

    /// Where the lock's target is now: the joint of the avatar as it was
    /// last drawn (its animation included), or the object's centre. `None`
    /// when it is not in the scene.
    fn director_target(&self) -> Option<Vec3> {
        let lock = self.director.d.lock;
        let now = Instant::now();
        match lock.target {
            Target::Own => self.scene.joint_world(&self.world, &self.world.agent_id, lock.joint.bone(), now),
            Target::Avatar(id) => self.scene.joint_world(&self.world, &id, lock.joint.bone(), now),
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

    /// Escape ends a playback or a scrub; true when it did.
    pub(super) fn director_escape(&mut self) -> bool {
        if self.director.d.mode == Mode::Editing {
            return false;
        }
        self.director.d.stop();
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
        let target = self.director.aim;
        ui::director::guides(ctx, p, projector, &self.director.d, target);
        ui::director::world(ctx, p, projector, &mut self.director.d, aspect, target, &mut actions);
        self.director_actions(actions);
    }

    /// The window, drawn with Quick preferences: the choices of its
    /// environment block come back for `env_actions`. What the timeline
    /// strip asked for earlier in the frame is applied first.
    pub(super) fn director_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, lighting: &mut bool) -> Vec<EnvAction> {
        let mut env = Vec::new();
        let mut actions = std::mem::take(&mut self.director.pending);
        let live = self.director_live();
        let mut open = self.panels.director;
        ui::director::window(
            ctx,
            p,
            &mut open,
            &mut self.director.d,
            &live,
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
        self.director_actions(actions);
        env
    }

    /// While the track plays: the countdown of its start delay, nothing else.
    pub(super) fn director_playing_ui(&self, ctx: &egui::Context, p: &crate::theme::Palette) {
        ui::director::playing(ctx, p, &self.director.d);
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
    /// live and at the two middle keyframes, in a 2.39:1 frame with the
    /// rule of thirds.
    fn director_demo(&mut self) {
        if !self.demo || self.director.demo_done {
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
            d.track.set_locked(1, true);
            d.track.set_locked(2, true);
            d.format = director::Format::Scope;
            d.thirds = true;
            log::info!("director demo: view locked on our avatar's neck, 2.39:1");
        }
    }
}
