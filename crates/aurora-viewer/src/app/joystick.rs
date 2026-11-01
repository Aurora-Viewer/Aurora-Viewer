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
            let up = glam::Quat::from_axis_angle(self.camera.forward(), self.camera.roll) * glam::Vec3::Z;
            self.camera.flycam = Some(crate::joystick::flycam::Flycam::from_view(
                self.camera.position,
                self.camera.target,
                up,
                self.camera.fov_y,
            ));
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
        let Ok(mode) = std::env::var("AURORA_DEMO_JOYSTICK") else {
            return;
        };
        if self.frame_count < 600 {
            return;
        }
        if !self.joystick.demo_started {
            self.joystick.demo_started = true;
            if mode == "rearm" {
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
        self.joystick.snapshot.devices = vec![d];
        self.joystick.snapshot.updated = Some(Instant::now());
        if self.frame_count == 630 && matches!(mode.as_str(), "flycam" | "roll") {
            self.toggle_joystick_flycam();
        }
    }
}
