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
    pub(super) motion: Motion,
    pub active: bool,
    pub(super) running: bool,
    demo_started: bool,
}

impl App {
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
        if !self.joystick.active {
            if !self.in_world()
                || !self.settings.joystick.enabled
                || !self.settings.joystick.flycam_enabled
                || self.joystick.snapshot.selected(&self.settings.joystick).is_none()
            {
                return;
            }
            let was_mouselook = self.camera.mouselook();
            self.stop_autopilot(true);
            self.camera.change_to_default(&mut self.world.agent, &self.settings.camera);
            self.camera.end_free(&self.settings.camera);
            self.snapshots.ui.free.on = false;
            self.photo_free_camera();
            self.camera.start_flycam(&self.settings.camera);
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

    pub(super) fn poll_joystick(&mut self, dt: f32) {
        if !self.panels.settings || !(self.focused || self.joystick.demo_started) {
            self.options_ui.cancel_joystick_capture();
        }
        let capturing = self.options_ui.joystick_capturing();
        let mode = if self.settings.joystick.enabled || capturing {
            Use::On
        } else if self.panels.settings && self.options_ui.page == crate::ui::options::Page::Controls && self.options_ui.tab != 0 {
            // The joystick tab lists the devices to choose from.
            Use::List
        } else {
            Use::Off
        };
        self.joystick.backend.poll(mode, &mut self.joystick.snapshot);
        self.joystick_demo();
        if self.camera.flycam.is_none() {
            self.joystick.active = false;
        }
        let available = self.in_world()
            && self.settings.joystick.enabled
            && self.joystick.snapshot.selected(&self.settings.joystick).is_some()
            && self.joystick.snapshot.updated.is_some_and(|at| at.elapsed().as_millis() < 250);
        if !available || !self.settings.joystick.flycam_enabled {
            self.camera.clear_flycam();
            self.joystick.active = false;
        }
        let motion = self.joystick.controller.sample(
            &self.joystick.snapshot,
            &self.settings.joystick,
            (self.focused || self.joystick.demo_started) && available && !capturing,
            self.joystick.active,
            dt,
        );
        if motion.toggle_flycam {
            self.toggle_joystick_flycam();
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
        let running = self.focused
            && !self.joystick.active
            && self.settings.joystick.avatar_enabled
            && !self.world.rlva.has("temprun")
            && axes[0].hypot(axes[1]) > self.settings.joystick.run_threshold.clamp(0.1, 1.0);
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
        let m = self.joystick.motion;
        // LLViewerJoystick::agentPush/Slide send digital simulator controls;
        // angular motion and the flycam keep the continuous values.
        m.apply_avatar(i);
        i.joystick_mouselook_yaw = self.settings.joystick.mouselook_yaw;
        if m.jump_pressed && self.world.agent.flying {
            self.toggle_fly();
        } else if m.jump {
            i.up = true;
        }
    }

    pub(super) fn joystick_flycam_step(&mut self, dt: f32) -> bool {
        if !self.joystick.active {
            return false;
        }
        if (self.focused || self.joystick.demo_started)
            && !self.options_ui.joystick_capturing()
            && let Some(cam) = self.camera.flycam.as_mut()
        {
            let m = self.joystick.motion;
            if m.reset_zoom {
                cam.reset_zoom();
            }
            let mut axes = m.axes;
            axes[2] += (f32::from(u8::from(m.rise)) - f32::from(u8::from(m.fall))) * 2.0;
            cam.step(axes, &self.settings.joystick, dt);
        }
        self.input = MoveInput::default();
        self.fb_since = None;
        self.lr_since = None;
        self.up_since = None;
        self.turn_since = None;
        true
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
                "rearm" | "axis-capture" | "zoom" | "zoom-buttons" | "zoom-triggers" | "zoom-hold"
            ) {
                self.settings.joystick = Default::default();
            }
            self.settings.joystick.enabled = true;
            self.settings.joystick.device = "demo-joystick".into();
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
            self.joystick.snapshot.devices = vec![crate::joystick::Device::new(
                "demo-joystick".into(),
                "Demo joystick / NDOF".into(),
                8,
            )];
            if matches!(mode.as_str(), "preferences" | "axes") {
                self.panels.settings = true;
                self.options_ui.open(crate::ui::options::Page::Controls, 1);
                if mode == "axes" {
                    // Both catalogues use these terms in the axis hint;
                    // the search also verifies that every mapping is indexed.
                    self.options_ui.query = "gain 0–1".to_owned();
                }
            }
        }
        // The worker's real devices never replace the synthetic input.
        let mut d = crate::joystick::Device::new("demo-joystick".into(), "Demo joystick / NDOF".into(), 8);
        if self.frame_count > 650 && !matches!(mode.as_str(), "preferences" | "axes") {
            d.axes[1] = -0.6;
            d.axes[5] = -0.5;
            if mode == "roll" {
                d.axes[4] = 0.6;
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
        self.joystick.snapshot.devices = vec![d];
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
