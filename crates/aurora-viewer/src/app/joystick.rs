//! Joystick input joins the existing movement/control pipeline; flycam
//! overrides the camera only. AgentUpdate still carries the real camera.

use super::App;
use crate::agent::MoveInput;
use crate::joystick::{Backend, Controller, Motion, Snapshot, Use};
use std::time::Instant;

#[derive(Default)]
pub struct State {
    backend: Backend,
    pub(super) snapshot: Snapshot,
    controller: Controller,
    gamepad_controller: Controller,
    pub(super) motion: Motion,
    pub active: bool,
    flycam_gamepad: bool,
    pub(super) running: bool,
    demo_started: bool,
}

impl App {
    fn joystick_input_focused(&self) -> bool {
        (if self.vr_camera.is_some() { self.vr_focused } else { self.focused }) || self.joystick.demo_started
    }

    pub(super) fn joystick_status(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.joystick.active {
            return;
        }
        let room = crate::ui::layout::work_area(ctx);
        let mut stop = false;
        let mut reset = false;
        egui::Area::new(egui::Id::unique("joystick-flycam-status"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(room.center().x, room.top() + 12.0))
            .show(ctx, |ui| {
                crate::ui::widgets::overlay(ui, p, egui::Margin::symmetric(12, 8), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("joystick-flycam"));
                        stop = crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("joystick-flycam-stop")).clicked();
                        reset = crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("keybinds-return-to-avatar")).clicked();
                    });
                });
            });
        if stop {
            self.toggle_joystick_flycam();
        }
        if reset {
            self.reset_camera_view();
        }
    }

    pub(super) fn toggle_joystick_flycam(&mut self) {
        let raw_available = self.settings.joystick.enabled
            && self.settings.joystick.flycam_enabled
            && self.joystick.snapshot.selected_joystick(&self.settings.joystick).is_some();
        let pad_available = self.settings.gamepad.enabled && self.joystick.snapshot.selected_gamepad(&self.settings.gamepad).is_some();
        self.toggle_flycam_from_input(!raw_available && pad_available);
    }

    pub(super) fn toggle_flycam_from_input(&mut self, gamepad: bool) {
        let (profile, selected) = if gamepad {
            (
                &self.settings.gamepad,
                self.joystick.snapshot.selected_gamepad(&self.settings.gamepad),
            )
        } else {
            (
                &self.settings.joystick,
                self.joystick.snapshot.selected_joystick(&self.settings.joystick),
            )
        };
        if !self.joystick.active {
            if !self.in_world() || !profile.enabled || !profile.flycam_enabled || selected.is_none() {
                return;
            }
            self.joystick.flycam_gamepad = gamepad;
            let was_mouselook = self.camera.mouselook();
            self.stop_autopilot(true);
            self.camera.change_to_default(&mut self.world.agent, &self.settings.camera);
            self.camera.end_free(&self.settings.camera);
            self.snapshots.ui.free.on = false;
            self.photo_free_camera();
            self.camera.start_flycam(&self.settings.camera, gamepad && was_mouselook);
            if was_mouselook {
                self.set_mouselook_grab(false);
            }
        } else {
            self.camera.retain_flycam_view(&mut self.world.agent, &self.settings.camera);
        }
        self.joystick.active = !self.joystick.active;
        // Off keeps the current point of view until movement or Reset camera,
        // as LLViewerJoystick::toggleFlycam does.
        self.last_input = Instant::now();
    }

    /// VR keeps the desktop mappings while explicitly selecting a level camera rig.
    pub(super) fn apply_vr_mode(&mut self, mode: crate::camera::VrView) {
        use crate::camera::VrView;
        if mode != VrView::FirstPerson {
            self.release_cursor(false);
        }
        self.vr_revision = self.vr_revision.wrapping_add(1);
        self.vr_body_follow.rebase();
        self.camera.vr_body_offset(glam::Vec3::ZERO);
        self.stop_autopilot(true);
        self.vr_rig_yaw = self
            .camera
            .enter_vr_mode(mode, &mut self.world.agent, &self.settings.camera, self.vr_rig_yaw);
        self.snapshots.ui.free.on = false;
        self.joystick.active = mode == VrView::Flycam;
        self.joystick.flycam_gamepad = self.settings.gamepad.enabled;
        self.vr_mode = Some(mode);
    }

    pub(super) fn cycle_gamepad_view(&mut self) {
        if !self.in_world() {
            return;
        }
        if self.vr_camera.is_some() {
            use crate::camera::VrView;
            let mode = match self.vr_mode.unwrap_or(self.vr_requested_mode) {
                VrView::Avatar => VrView::FirstPerson,
                VrView::FirstPerson => VrView::Flycam,
                VrView::Flycam => VrView::Avatar,
            };
            self.apply_vr_mode(mode);
            return;
        }
        self.end_mouselook_zoom();
        self.camera.reset_gamepad_mouselook_zoom();
        if self.joystick.active || self.camera.flycam.is_some() {
            self.reset_camera_view();
        } else if !self.camera.mouselook() && self.settings.camera.enable_mouselook {
            self.stop_autopilot(true);
            self.toggle_mouselook();
        } else if self.settings.gamepad.flycam_enabled {
            self.toggle_flycam_from_input(true);
        } else {
            self.reset_camera_view();
        }
        self.last_input = Instant::now();
    }

    /// Keep locomotion in the rendered gaze frame without feeding physical yaw
    /// back into the camera rig on the next frame.
    pub(super) fn vr_locomotion(&mut self, previous_body_yaw: f32, dt: f32) {
        if !self.vr_focused {
            self.input = MoveInput::default();
            self.world.agent.yaw = previous_body_yaw;
            return;
        }
        if self.joystick.active {
            self.world.agent.yaw = previous_body_yaw;
            return;
        }
        let key_yaw = self.input.keyboard_yaw();
        let yaw_rate = std::mem::take(&mut self.input.joystick_yaw) + key_yaw;
        let turn = yaw_rate * dt.clamp(0.0, 0.2);
        let avatar = self.vr_mode == Some(crate::camera::VrView::Avatar);
        self.vr_rig_yaw = self.world.agent.yaw;
        if avatar {
            // Desktop Avatar steers with the right stick while walking. Route
            // the same yaw into the VR boom, even when focus is unlocked.
            self.camera.vr_request_orbit_framing(key_yaw != 0.0);
            self.camera.joystick_orbit_yaw(yaw_rate, dt, &mut self.world.agent);
            self.input.turn_left = false;
            self.input.turn_right = false;
        } else {
            self.vr_rig_yaw += turn;
        }
        let translating = self.input.forward || self.input.back || self.input.strafe_left || self.input.strafe_right;
        self.world.agent.yaw = if translating || key_yaw != 0.0 {
            crate::camera::vr_walk_yaw(self.camera.vr_view(self.vr_rig_yaw), self.vr_head_rotation) + if avatar { turn } else { 0.0 }
        } else {
            previous_body_yaw
        };
    }

    pub(super) fn vr_rebase_after_travel(&mut self) {
        self.vr_rebase_reference(true);
    }
    pub(super) fn vr_rebase_reference(&mut self, reposition: bool) {
        self.vr_revision = self.vr_revision.wrapping_add(1);
        self.vr_body_follow.rebase();
        self.camera.vr_body_offset(glam::Vec3::ZERO);
        self.vr_head_pose = None;
        self.camera.vr_rebase(reposition);
    }

    pub(super) fn vr_update_body_follow(&mut self, typing: bool, dt: f32) {
        use crate::camera::VrView;
        let text_input = typing && !self.chat_bar_moves_avatar();
        let travelling = self.world.teleporting || !self.world.movement_complete || !self.in_world();
        let first = self.vr_camera.is_some() && self.vr_mode == Some(VrView::FirstPerson) && self.camera.mouselook();
        let seated = self.world.agent.is_sitting();
        if !first {
            self.vr_body_follow.rebase();
        } else if seated || self.world.agent.flying || travelling {
            // Crossing handshakes and locomotion takeovers suspend following;
            // explicit teleport/recenter events rebase the origin separately.
            self.vr_body_follow.pause();
        }
        let head = self.vr_head_pose.filter(|p| {
            p.matches(self.world.main_region, self.vr_revision) && matches!(p.mode, aurora_render::stereo_view::RigMode::FirstPerson)
        });
        let manual = self.input.resets_camera(self.crouch_toggle.latched())
            || self.input.up
            || self.input.down
            || self.joystick.motion.player_movement_input()
            || self.joystick.motion.yaw_input
            || self.autopilot.is_some();
        let locked = self.settings.bridge.get(&self.world.agent_id).is_some_and(|prefs| prefs.enabled);
        let allowed = first
            && !seated
            && !self.world.agent.flying
            && self.vr_focused
            && self.autopilot.is_none()
            && !text_input
            && !travelling
            && !locked;
        let facing = head.map_or(self.world.agent.yaw, |pose| {
            crate::camera::vr_walk_yaw(self.camera.vr_view(self.vr_rig_yaw), pose.rotation)
        });
        if allowed && head.is_some() {
            self.world.agent.yaw = facing;
        }
        let command = self.vr_body_follow.step_facing(
            self.camera.vr_body_root(),
            self.world.agent.velocity,
            facing,
            head.map(|pose| self.camera.vr_body_goal(pose)),
            allowed,
            manual,
            dt,
        );
        self.vr_follow_flags = None;
        if let Some(command) = command {
            self.input = MoveInput::default();
            self.vr_follow_flags = Some(if command.braking {
                aurora_net::control::STOP
            } else if command.forward {
                // Pick the nearest native walk direction while the body faces
                // the headset. Diagonal steps use the same controls as WASD.
                let angle = command.yaw - facing;
                let sector = (angle / std::f32::consts::FRAC_PI_4).round() as i32;
                let direction = sector.rem_euclid(8);
                self.input.forward = matches!(direction, 0 | 1 | 7);
                self.input.back = matches!(direction, 3..=5);
                self.input.strafe_left = matches!(direction, 1..=3);
                self.input.strafe_right = matches!(direction, 5..=7);
                use aurora_net::control;
                (u32::from(self.input.forward) * control::AT_POS)
                    | (u32::from(self.input.back) * control::AT_NEG)
                    | (u32::from(self.input.strafe_left) * control::LEFT_POS)
                    | (u32::from(self.input.strafe_right) * control::LEFT_NEG)
            } else {
                0
            });
        }
        let run = self.always_run || self.temp_run.is_some() || self.joystick.running;
        if let Some(run) = self.vr_walk_override.update(command.is_some(), run) {
            self.send(aurora_net::NetCommand::SetAlwaysRun(run));
        }
    }
    pub(super) fn vr_counter_body_motion(&mut self) {
        if self.vr_mode == Some(crate::camera::VrView::FirstPerson) {
            let offset = self
                .vr_body_follow
                .accepted_root_at(self.camera.vr_body_root(), self.world.agent.velocity, Instant::now());
            self.camera.vr_body_offset(offset);
        }
    }

    pub(super) fn poll_joystick(&mut self, dt: f32) {
        if !self.panels.settings || !(self.focused || self.joystick.demo_started) {
            self.options_ui.cancel_joystick_capture();
        }
        let capturing = self.options_ui.joystick_capturing();
        let mode = if self.settings.joystick.enabled || capturing {
            Use::On
        } else if self.panels.settings && self.options_ui.page == crate::ui::options::Page::Controls && self.options_ui.tab != 0 {
            Use::List
        } else if self.settings.gamepad.enabled || (self.panels.settings && self.options_ui.page == crate::ui::options::Page::Gamepad) {
            Use::Gamepad
        } else {
            Use::Off
        };
        self.joystick.backend.poll(mode, &mut self.joystick.snapshot);
        self.joystick_demo();
        if self.camera.flycam.is_none() {
            self.joystick.active = false;
        }
        let joystick = &self.settings.joystick;
        let gamepad = &self.settings.gamepad;
        let snapshot = &self.joystick.snapshot;
        let raw_device = snapshot.selected_joystick(joystick).filter(|_| joystick.enabled);
        let pad_device = snapshot.selected_gamepad(gamepad).filter(|_| gamepad.enabled);
        let available = self.in_world() && snapshot.updated.is_some_and(|at| at.elapsed().as_millis() < 250);
        let pad_flycam = self.joystick.flycam_gamepad;
        let flycam_available = if pad_flycam {
            gamepad.flycam_enabled && pad_device.is_some()
        } else {
            joystick.flycam_enabled && raw_device.is_some()
        };
        if self.joystick.active && (!available || !flycam_available) && self.vr_mode != Some(crate::camera::VrView::Flycam) {
            self.camera.clear_flycam();
            self.joystick.active = false;
        }
        let flycam = self.joystick.active;
        let active = available && self.joystick_input_focused() && !capturing;
        let first_person = self.camera.mouselook();
        let raw = self.joystick.controller.sample_device(
            raw_device.filter(|_| active && (!flycam || !pad_flycam)),
            joystick,
            flycam,
            first_person,
            dt,
        );
        let pad = self.joystick.gamepad_controller.sample_device(
            pad_device.filter(|_| active && (!flycam || pad_flycam)),
            gamepad,
            flycam,
            first_person,
            dt,
        );
        // Use one source per frame. Flycam keeps the source that entered it,
        // while an idle gamepad leaves ordinary movement to the joystick.
        let motion = if (flycam && pad_flycam) || pad.moving() || pad.toggle_flycam {
            pad
        } else {
            raw
        };
        let profile = if motion.gamepad { gamepad } else { joystick };
        let run_threshold = profile.run_threshold.clamp(0.1, 1.0);
        let avatar_enabled = profile.avatar_enabled;
        if motion.toggle_flycam {
            if motion.gamepad {
                self.cycle_gamepad_view();
            } else {
                self.toggle_flycam_from_input(false);
            }
        }
        self.joystick.motion = if motion.toggle_flycam { Motion::default() } else { motion };
        if motion.moving() || motion.toggle_flycam {
            self.last_input = Instant::now();
            if self.world.status.away {
                self.world.set_away(false);
                self.auto_away = false;
            }
        }
        let axes = self.joystick.motion.axes;
        let running = self.joystick_input_focused()
            && !self.joystick.active
            && avatar_enabled
            && !self.world.rlva.has("temprun")
            && axes[0].hypot(axes[1]) > run_threshold;
        if running != self.joystick.running {
            self.joystick.running = running;
            if !self.always_run && self.temp_run.is_none() {
                self.send(aurora_net::NetCommand::SetAlwaysRun(running));
            }
        }
    }

    pub(super) fn joystick_avatar(&mut self, i: &mut MoveInput) {
        if self.camera.free().is_some() || self.joystick.active {
            return;
        }
        let mut m = self.joystick.motion;
        if matches!(
            self.vr_mode,
            Some(crate::camera::VrView::FirstPerson | crate::camera::VrView::Avatar)
        ) && m.gamepad
        {
            m.keep_vr_forward();
        }
        // LLViewerJoystick::agentPush/Slide send digital simulator controls;
        // angular motion and the flycam keep the continuous values.
        m.apply_avatar(i);
        if self.vr_camera.is_some() {
            i.joystick_pitch = 0.0;
        }
        // A console gamepad orbits the idle third-person camera with the
        // right stick; while translating, that same stick steers the avatar.
        if m.gamepad
            && !self.camera.mouselook()
            && self.mouse_mode != crate::app::MouseMode::FocusOrbit
            && !(i.forward || i.back || i.turn_left || i.turn_right || i.strafe_left || i.strafe_right)
        {
            i.joystick_yaw = 0.0;
        }
        i.joystick_mouselook_yaw = if m.gamepad {
            self.settings.gamepad.mouselook_yaw
        } else {
            self.settings.joystick.mouselook_yaw
        };
        if m.jump_pressed && self.world.agent.flying {
            self.toggle_fly();
        } else if m.jump {
            i.up = true;
        }
    }

    pub(super) fn joystick_flycam_step(&mut self, keys: &MoveInput, dt: f32) {
        if self.joystick_input_focused()
            && !self.options_ui.joystick_capturing()
            && let Some(cam) = self.camera.flycam.as_mut()
        {
            let m = self.joystick.motion;
            if m.reset_zoom {
                cam.reset_zoom();
            }
            let mut axes = m.axes;
            if self.vr_camera.is_some() {
                if self.focused {
                    axes[0] += (i32::from(keys.forward) - i32::from(keys.back)) as f32;
                    axes[1] += (i32::from(keys.strafe_left) - i32::from(keys.strafe_right)) as f32;
                    axes[5] += keys.keyboard_yaw();
                }
                axes = crate::joystick::flycam::vr_axes(axes, self.vr_head_rotation);
                if self.focused {
                    axes[2] += (i32::from(keys.up) - i32::from(keys.down)) as f32;
                }
                let forward = cam.forward();
                let heading = glam::Vec3::new(forward.x, forward.y, 0.0).normalize_or(glam::Vec3::X);
                cam.rotation = crate::joystick::flycam::Flycam::from_direction(cam.position, heading, glam::Vec3::Z, cam.fov).rotation;
            }
            axes[2] += (f32::from(u8::from(m.rise)) - f32::from(u8::from(m.fall))) * 2.0;
            let profile = if self.joystick.flycam_gamepad {
                &self.settings.gamepad
            } else {
                &self.settings.joystick
            };
            if let Some(multiplier) = m.flycam_speed {
                cam.step_with_speed(axes, profile, dt, multiplier);
            } else {
                cam.step(axes, profile, dt);
            }
        }
        self.input = MoveInput::default();
        self.fb_since = None;
        self.lr_since = None;
        self.up_since = None;
        if self.vr_camera.is_none() {
            self.turn_since = None;
        }
    }

    pub(super) fn joystick_avatar_camera(&mut self, moving: bool, player_movement_input: bool, dt: f32) {
        let motion = self.joystick.motion;
        if self.vr_mode == Some(crate::camera::VrView::Avatar) {
            if player_movement_input {
                self.camera.vr_resume_avatar_follow(&mut self.world.agent, &self.settings.camera);
            }
            self.camera.vr_request_movement_framing(player_movement_input);
            self.camera.vr_request_orbit_framing(motion.yaw_input);
            self.camera.vr_request(crate::camera::vr::FramingInput {
                recenter: motion.avatar_pan != 0.0,
                zoom: motion.avatar_zoom,
            });
        }
        if self.camera.mouselook() {
            if player_movement_input {
                self.camera.reset_gamepad_mouselook_zoom();
            } else if motion.avatar_zoom != 0.0 && self.world.rlva.camera_owner().is_none() {
                self.camera
                    .gamepad_mouselook_zoom(motion.avatar_zoom, dt, &self.settings.camera, self.view_aspect());
            }
            return;
        }
        if motion.avatar_zoom == 0.0 && motion.avatar_pan == 0.0 && motion.avatar_cycle == 0 {
            return;
        }
        let was_mouselook = self.camera.mouselook();
        if !moving && motion.avatar_cycle != 0 {
            let now = Instant::now();
            // Reuse People's nearest-first ordering and its ZoomAvatar action.
            // Unloaded coarse sightings cannot be followed, just as in People.
            let nearby = crate::ui::people::nearby(&self.world);
            let available: Vec<_> = nearby
                .into_iter()
                .filter(|(id, _, _, _)| {
                    *id != self.world.agent_id && crate::camera::focus_object_geom(&self.world, id, now).is_some_and(|g| g.avatar)
                })
                .collect();
            let current = self
                .camera
                .focus_object_id()
                .and_then(|id| available.iter().position(|row| row.0 == id));
            if let Some(index) = avatar_cycle_index(current, available.len(), motion.avatar_cycle) {
                let id = available[index].0;
                if self.demo {
                    log::info!(
                        "gamepad avatar cycle: direction={} rank={} count={} subject={} distance={:.2}",
                        motion.avatar_cycle,
                        index,
                        available.len(),
                        id,
                        available[index].3
                    );
                }
                self.on_ctx_action(crate::ui::context::CtxAction::ZoomAvatar(id));
                if self.vr_mode != Some(crate::camera::VrView::Avatar)
                    && self.camera.focus_object_id() == Some(id)
                    && let Some(head) = self.scene.head_world(&self.world, &id, now).or_else(|| self.camera.focus_point())
                {
                    self.camera.gamepad_follow_head(head, &mut self.world.agent, &self.settings.camera);
                }
            } else {
                self.reset_camera_view();
                if self.demo {
                    log::info!("gamepad avatar cycle: direction={} subject=player", motion.avatar_cycle);
                }
            }
        }
        self.camera
            .gamepad_zoom(motion.avatar_zoom, dt, &mut self.world.agent, &self.settings.camera);
        if was_mouselook != self.camera.mouselook() {
            self.set_mouselook_grab(!was_mouselook);
        }
        if !self.world.rlva.camera_preset_locked(uuid::Uuid::nil()) {
            self.camera
                .gamepad_pan_vertical(motion.avatar_pan * dt.clamp(0.0, 0.2), &mut self.world.agent, &self.settings.camera);
        }
    }

    fn joystick_demo(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(scenario) = std::env::var("AURORA_DEMO_JOYSTICK") else {
            return;
        };
        let absolute = scenario.ends_with("-absolute");
        let reset_demo = scenario.starts_with("reset-");
        let mode = scenario.trim_start_matches("reset-").trim_end_matches("-absolute").to_owned();
        if self.frame_count < 600 {
            return;
        }
        if !self.joystick.demo_started {
            self.joystick.demo_started = true;
            if matches!(
                mode.as_str(),
                "rearm"
                    | "axis-capture"
                    | "zoom"
                    | "zoom-buttons"
                    | "zoom-triggers"
                    | "zoom-hold"
                    | "avatar-camera"
                    | "view-modes"
                    | "gamepad-defaults"
                    | "avatar-orbit"
            ) {
                self.settings.joystick = Default::default();
            }
            if matches!(
                mode.as_str(),
                "gamepad-preferences" | "avatar-camera" | "view-modes" | "gamepad-defaults" | "avatar-orbit"
            ) {
                self.settings.gamepad = crate::joystick::Settings::gamepad_defaults();
                self.settings.gamepad.enabled = true;
                if mode != "gamepad-preferences" {
                    self.settings.gamepad.device = "demo-joystick".into();
                }
            } else {
                self.settings.joystick.enabled = true;
                self.settings.joystick.device = "demo-joystick".into();
            }
            if mode == "roll" {
                self.settings.joystick.auto_level = false;
                self.settings.joystick.flycam.gain[3] = 1.0;
            }
            if mode == "rearm" {
                self.settings.joystick.mapping[6] = 6;
                self.settings.joystick.flycam.gain[6] = 0.5;
            }
            if mode == "zoom" {
                self.settings.camera.fov = crate::camera::DEFAULT_FOV;
                self.settings.joystick.mapping[6] = 6;
                self.settings.joystick.flycam.gain[6] = 1.0;
            }
            if matches!(mode.as_str(), "zoom-buttons" | "zoom-triggers" | "zoom-hold") {
                self.settings.camera.fov = crate::camera::DEFAULT_FOV;
                self.settings.joystick.flycam.feather = 0.0;
                if mode == "zoom-hold" {
                    self.settings.joystick.zoom_mode = crate::joystick::ZoomMode::HoldAxis;
                    self.settings.joystick.zoom_hold_button = 44;
                    self.settings.joystick.mapping[6] = 5;
                    self.settings.joystick.flycam.gain[6] = 0.6;
                } else {
                    self.settings.joystick.zoom_mode = crate::joystick::ZoomMode::Buttons;
                    self.settings.joystick.zoom_in_button = if mode == "zoom-triggers" { 44 } else { 8 };
                    self.settings.joystick.zoom_out_button = if mode == "zoom-triggers" { 46 } else { 9 };
                    self.settings.joystick.zoom_speed = 0.6;
                }
            }
            if absolute {
                self.settings.joystick.direct_zoom = true;
            }
            if mode != "gamepad-preferences" || !self.joystick.snapshot.devices.iter().any(|d| d.is_gamepad) {
                self.joystick.snapshot.devices = vec![if matches!(
                    mode.as_str(),
                    "gamepad-preferences" | "avatar-camera" | "view-modes" | "gamepad-defaults" | "avatar-orbit"
                ) {
                    crate::joystick::Device::gamepad(
                        "demo-joystick".into(),
                        "Demo gamepad".into(),
                        8,
                        [[Some(0), Some(1)], [Some(3), Some(4)]],
                        [Some(6), Some(7)],
                        crate::joystick::GamepadStyle::Generic,
                        crate::joystick::GamepadButton::STANDARD.to_vec(),
                    )
                } else {
                    crate::joystick::Device::new("demo-joystick".into(), "Demo joystick / NDOF".into(), 8)
                }];
            }
            if matches!(mode.as_str(), "preferences" | "axes" | "gamepad-preferences") {
                self.panels.settings = true;
                self.options_ui.open(
                    if mode == "gamepad-preferences" {
                        crate::ui::options::Page::Gamepad
                    } else {
                        crate::ui::options::Page::Controls
                    },
                    1,
                );
                if mode == "axes" {
                    // Both catalogues use these terms in the axis hint;
                    // the search also verifies that every mapping is indexed.
                    self.options_ui.query = "gain 0–1".to_owned();
                }
            }
        }
        // The worker's real devices never replace the synthetic input.
        let mut d = if matches!(
            mode.as_str(),
            "gamepad-preferences" | "avatar-camera" | "view-modes" | "gamepad-defaults" | "avatar-orbit"
        ) {
            crate::joystick::Device::gamepad(
                "demo-joystick".into(),
                "Demo gamepad".into(),
                8,
                [[Some(0), Some(1)], [Some(3), Some(4)]],
                [Some(6), Some(7)],
                crate::joystick::GamepadStyle::Generic,
                crate::joystick::GamepadButton::STANDARD.to_vec(),
            )
        } else {
            crate::joystick::Device::new("demo-joystick".into(), "Demo joystick / NDOF".into(), 8)
        };
        if self.frame_count > 650
            && !matches!(
                mode.as_str(),
                "preferences" | "axes" | "avatar-camera" | "view-modes" | "gamepad-defaults" | "avatar-orbit"
            )
        {
            d.axes[1] = -0.6;
            d.axes[5] = -0.5;
            if mode == "roll" {
                d.axes[4] = 0.6;
            }
        }
        if mode == "gamepad-defaults" {
            d.gamepad_style = crate::joystick::GamepadStyle::Xbox;
            // Exercise native button orders that place triggers before A/B.
            d.gamepad_buttons.swap(0, 7);
            d.gamepad_buttons.swap(1, 6);
        }
        if mode == "avatar-orbit" {
            d.axes = [0.0; crate::joystick::AXES];
            if (620..1000).contains(&self.frame_count)
                || (1060..1520).contains(&self.frame_count)
                || (1700..1850).contains(&self.frame_count)
            {
                d.axes[7] = 1.0;
            }
            if (1005..1045).contains(&self.frame_count) {
                d.buttons |= 1 << 14;
            }
            d.presses[14] = u64::from(self.frame_count >= 1005);
            if (1540..1650).contains(&self.frame_count) {
                d.axes[3] = 1.0;
            }
            if self.frame_count == 1680 {
                let delta = glam::Vec3::new(1.0, 0.5, 0.25);
                self.world.agent.position += delta;
                if let Some(index) = self.world.objects.index_of_uuid(&self.world.agent_id)
                    && let Some(avatar) = self.world.objects.get_mut(index)
                {
                    avatar.position += delta;
                }
            }
            if matches!(self.frame_count, 1000 | 1520 | 1700 | 1850) {
                log::info!(
                    "gamepad player orbit: frame={} first_person={} player={:?} focus={:?} radius={:.4}",
                    self.frame_count,
                    self.camera.mouselook(),
                    self.world.agent.position,
                    self.camera.target,
                    self.camera.position.distance(self.camera.target)
                );
            }
        }
        if mode == "view-modes" {
            d.axes = [0.0; crate::joystick::AXES];
            // Three distinct presses must traverse third person, mouselook,
            // flycam and back without replaying a held input across modes.
            d.presses[0] = [630, 900, 1100].into_iter().filter(|frame| self.frame_count >= *frame).count() as u64;
            if [630..640, 900..910, 1100..1110]
                .into_iter()
                .any(|range| range.contains(&self.frame_count))
            {
                d.buttons = 1;
            }
            if (680..760).contains(&self.frame_count) {
                d.axes[3] = 0.4;
                d.axes[4] = -0.25;
            }
            if (780..820).contains(&self.frame_count) || (896..900).contains(&self.frame_count) {
                d.axes[7] = 1.0;
                d.buttons |= 1 << 7;
            }
            if (830..860).contains(&self.frame_count) {
                d.axes[3] = -0.4;
            }
            if self.frame_count == 840 {
                self.world.agent.position.x += 0.05;
            }
            if (870..890).contains(&self.frame_count) {
                d.axes[1] = -0.5;
            }
            if (950..1000).contains(&self.frame_count) {
                d.axes[1] = -0.5;
            }
            if matches!(
                self.frame_count,
                620 | 660 | 770 | 810 | 825 | 850 | 865 | 895 | 899 | 930 | 1030 | 1140
            ) {
                log::info!(
                    "gamepad view modes: frame={} first_person={} flycam={} yaw={:.3} pitch={:.3} fov={:.3}",
                    self.frame_count,
                    self.camera.mouselook(),
                    self.joystick.active,
                    self.world.agent.yaw,
                    self.world.agent.pitch,
                    self.camera.fov_y
                );
            }
        }
        if mode == "avatar-camera" {
            d.axes = [0.0; crate::joystick::AXES];
            if (620..700).contains(&self.frame_count) {
                d.buttons |= 1 << 14;
            }
            if matches!(self.frame_count, 720 | 900 | 1080 | 1090) {
                d.buttons |= 1 << 16;
            }
            if matches!(self.frame_count, 1000 | 1040 | 1060) {
                d.buttons |= 1 << 15;
            }
            d.presses[14] = u64::from(self.frame_count >= 620);
            d.presses[16] = [720, 900, 1080, 1090]
                .into_iter()
                .filter(|frame| self.frame_count >= *frame)
                .count() as u64;
            d.presses[15] = [1000, 1040, 1060].into_iter().filter(|frame| self.frame_count >= *frame).count() as u64;
            if (1100..1160).contains(&self.frame_count) {
                d.axes[1] = -0.6;
            }
            if self.frame_count == 800
                && let Some(index) = self
                    .camera
                    .gamepad_avatar_target()
                    .and_then(|id| self.world.objects.index_of_uuid(&id))
                && let Some(avatar) = self.world.objects.get_mut(index)
            {
                avatar.position.y += 2.0;
            }
            if matches!(self.frame_count, 710 | 730 | 820 | 910 | 1010 | 1170) {
                log::info!(
                    "gamepad avatar camera: frame={} subject={:?} eye={:?} focus={:?}",
                    self.frame_count,
                    self.camera.gamepad_avatar_target(),
                    self.camera.position,
                    self.camera.target
                );
            }
        }
        if mode == "axis-capture" {
            d.axes = [0.0; crate::joystick::AXES];
            if (700..760).contains(&self.frame_count) || (820..880).contains(&self.frame_count) {
                // A small sideways motion accompanies a clear movement
                // on another native axis, as when assigning a stick's Y.
                d.axes[0] = 0.05;
                d.axes[7] = -0.8;
            }
        }
        if mode == "zoom" && !reset_demo {
            d.axes = [0.0; crate::joystick::AXES];
            if (650..710).contains(&self.frame_count) {
                d.axes[6] = 0.6;
            }
            if self.frame_count == 750 {
                self.settings.camera.fov = 0.9;
            }
            if self.frame_count == 800 {
                self.settings.joystick.direct_zoom = true;
            }
            if (820..880).contains(&self.frame_count) {
                d.axes[6] = 0.4;
            }
            if matches!(self.frame_count, 641 | 721 | 771 | 861 | 921) {
                log::info!(
                    "joystick zoom: frame={} flycam={} fov={:.4}",
                    self.frame_count,
                    self.joystick.active,
                    self.camera.fov_y
                );
            }
        }
        if matches!(mode.as_str(), "zoom-buttons" | "zoom-triggers" | "zoom-hold") {
            d.axes = [0.0; crate::joystick::AXES];
            if mode == "zoom-hold" {
                if (650..760).contains(&self.frame_count) {
                    d.axes[6] = 0.9;
                }
                if (650..690).contains(&self.frame_count) || (810..850).contains(&self.frame_count) {
                    d.axes[5] = 0.8;
                }
                if (720..790).contains(&self.frame_count) {
                    d.axes[5] = -0.8;
                }
            } else {
                let inside = (650..690).contains(&self.frame_count) || (780..810).contains(&self.frame_count);
                let outside = (720..760).contains(&self.frame_count) || (780..810).contains(&self.frame_count);
                if mode == "zoom-triggers" {
                    d.axes[6] = if inside && absolute {
                        if self.frame_count < 670 { 0.25 } else { 0.75 }
                    } else {
                        f32::from(u8::from(inside))
                    };
                    d.axes[7] = f32::from(u8::from(outside));
                } else {
                    d.buttons = (u32::from(inside) << 8) | (u32::from(outside) << 9);
                    d.presses[8] = u64::from(self.frame_count >= 650) + u64::from(self.frame_count >= 780);
                    d.presses[9] = u64::from(self.frame_count >= 720) + u64::from(self.frame_count >= 780);
                }
            }
            if matches!(self.frame_count, 641 | 701 | 771 | 791 | 831 | 861) {
                log::info!(
                    "joystick zoom mode: mode={} frame={} flycam={} fov={:.4} yaw={:.4} zoom={:.4}",
                    mode,
                    self.frame_count,
                    self.joystick.active,
                    self.camera.fov_y,
                    self.joystick.motion.axes[5],
                    self.joystick.motion.axes[6]
                );
            }
        }
        if mode == "rearm" {
            // A relative zoom axis that never centres must not prevent
            // flycam entry/exit or movement on the other axes.
            d.axes[6] = -1.0;
            d.presses[0] = u64::from(self.frame_count >= 630) + u64::from(self.frame_count >= 750);
            if matches!(self.frame_count, 630 | 750) {
                d.buttons = 1;
            }
            if (770..800).contains(&self.frame_count) {
                d.axes[1] = 0.0;
                d.axes[5] = 0.0;
            }
            if matches!(self.frame_count, 701 | 761 | 831) {
                log::info!(
                    "joystick rearm: frame={} flycam={} axes={:?}",
                    self.frame_count,
                    self.joystick.active,
                    self.joystick.motion.axes
                );
            }
        }
        if reset_demo {
            d.axes = [0.0; crate::joystick::AXES];
            d.buttons = 0;
            d.presses = [0; crate::joystick::BUTTONS];
            let first = (650..690).contains(&self.frame_count);
            let tap = (730..736).contains(&self.frame_count) || (744..770).contains(&self.frame_count);
            let resumed = (810..840).contains(&self.frame_count);
            match mode.as_str() {
                "zoom" => {
                    d.axes[6] = if first || tap {
                        0.6
                    } else if resumed {
                        -0.6
                    } else {
                        0.0
                    };
                }
                "zoom-hold" => {
                    d.axes[6] = if first || tap || resumed { 0.9 } else { 0.0 };
                    // Leave the shared stick deflected briefly after release.
                    d.axes[5] = if first || tap || (770..780).contains(&self.frame_count) {
                        0.8
                    } else if resumed {
                        -0.8
                    } else {
                        0.0
                    };
                }
                "zoom-buttons" | "zoom-triggers" => {
                    let inside = first || (744..770).contains(&self.frame_count);
                    let outside = (744..755).contains(&self.frame_count) || resumed;
                    if mode == "zoom-triggers" {
                        d.axes[6] = f32::from(u8::from(inside));
                        d.axes[7] = f32::from(u8::from(outside));
                    } else {
                        d.buttons = (u32::from(inside) << 8) | (u32::from(outside) << 9);
                        d.presses[8] = u64::from(self.frame_count >= 650) + u64::from(self.frame_count >= 744);
                        d.presses[9] = u64::from(self.frame_count >= 744) + u64::from(self.frame_count >= 810);
                    }
                }
                _ => {}
            }
            if matches!(self.frame_count, 701 | 741 | 751 | 781 | 841) {
                log::info!(
                    "joystick zoom reset: mode={} frame={} fov={:.4} yaw={:.4} zoom={:.4}",
                    mode,
                    self.frame_count,
                    self.camera.fov_y,
                    self.joystick.motion.axes[5],
                    self.joystick.motion.axes[6]
                );
            }
        }
        if mode == "gamepad-preferences" {
            let connected: Vec<_> = self
                .joystick
                .snapshot
                .devices
                .iter()
                .filter(|device| device.is_gamepad)
                .cloned()
                .collect();
            self.joystick.snapshot.devices = if connected.is_empty() { vec![d] } else { connected };
        } else {
            self.joystick.snapshot.devices = vec![d];
        }
        self.joystick.snapshot.updated = Some(Instant::now());
        if self.frame_count == 630
            && matches!(
                mode.as_str(),
                "flycam" | "roll" | "zoom" | "zoom-buttons" | "zoom-triggers" | "zoom-hold"
            )
        {
            self.toggle_joystick_flycam();
        }
    }
}

/// The player is the extra stop at each end of People's nearest-first list.
fn avatar_cycle_index(current: Option<usize>, count: usize, direction: i8) -> Option<usize> {
    if count == 0 || direction == 0 {
        return None;
    }
    match current {
        Some(index) if direction > 0 => index.checked_add(1).filter(|next| *next < count),
        Some(index) => index.checked_sub(1),
        None if direction > 0 => Some(0),
        None => Some(count - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::avatar_cycle_index;
    #[test]
    fn joystick_avatar_cycle_includes_the_player_at_both_ends() {
        assert_eq!(avatar_cycle_index(None, 3, 1), Some(0));
        assert_eq!(avatar_cycle_index(Some(0), 3, -1), None);
        assert_eq!(avatar_cycle_index(None, 3, -1), Some(2));
        assert_eq!(avatar_cycle_index(Some(2), 3, 1), None);
        assert_eq!(avatar_cycle_index(Some(1), 3, 1), Some(2));
        assert_eq!(avatar_cycle_index(Some(1), 3, -1), Some(0));
        assert_eq!(avatar_cycle_index(Some(0), 1, 1), None);
        assert_eq!(avatar_cycle_index(Some(0), 1, -1), None);
        assert_eq!(avatar_cycle_index(None, 0, 1), None);
    }
}
