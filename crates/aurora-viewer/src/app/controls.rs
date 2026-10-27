//! The Controls window in the app (`ui/controls.rs`): what its buttons and
//! joysticks do, as the keys and the menus do (LLFloaterCamera,
//! LLFloaterMove, LLFloaterSaveCameraPreset).

use super::App;
use crate::theme::Palette;
use crate::ui::controls::{ControlAction, FreeView, View};
use aurora_net::{NetCommand, control};

impl App {
    pub(super) fn controls_window(&mut self, ctx: &egui::Context, p: &Palette) {
        self.joystick_status(ctx, p);
        self.controls_demo();
        let flight = self.snapshots.ui.free;
        let view = View {
            zoom: self.camera.zoom_fraction(),
            preset: self.settings.camera.preset,
            shots: &self.settings.camera_shots,
            flying: self.world.agent.flying,
            always_run: self.always_run,
            seated: self.world.agent.seated,
            mouselook: self.camera.mouselook(),
            object_view: self.object_view,
            keys: &self.settings.keybinds,
            free: self.camera.free().map(|_| FreeView {
                speed: flight.speed,
                no_collisions: flight.no_collisions,
            }),
        };
        let out = self.controls_ui.show(
            ctx,
            p,
            &mut self.panels.camera_controls,
            &mut self.panels.movement_controls,
            self.settings.classic_controls,
            &view,
        );
        self.controls_held = out.held;
        self.controls_free = (out.free_move, out.free_look);
        // the object view ends with the window (LLFloaterCamera::onClose)
        if !self.panels.camera_controls && !self.panels.movement_controls {
            self.object_view = false;
        }
        for action in out.actions {
            self.control_action(action);
        }
    }

    fn control_action(&mut self, action: ControlAction) {
        if self.world.rlva.camera_preset_locked(uuid::Uuid::nil())
            && matches!(
                action,
                ControlAction::Preset(_) | ControlAction::UseShot(_) | ControlAction::SaveShot(_)
            )
        {
            return;
        }
        match action {
            ControlAction::FreeSpeed(speed) => self.snapshots.ui.free.speed = speed,
            ControlAction::FreeCollisions(on) => self.snapshots.ui.free.no_collisions = on,
            ControlAction::FreeExit => {
                self.snapshots.ui.free.on = false;
                self.photo_free_camera();
            }
            ControlAction::Preset(preset) => {
                // LLFloaterCamera::switchToPreset: the view, from behind
                self.settings.camera.apply_preset(preset);
                self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
                self.settings.save();
            }
            ControlAction::ResetOrbit => self.reset_camera_view(),
            ControlAction::ResetCamera => {
                self.reset_camera_all();
                self.settings.save();
            }
            ControlAction::Zoom(fraction) => self.camera.set_zoom_fraction(fraction),
            ControlAction::Mouselook => self.toggle_mouselook(),
            ControlAction::ObjectView => self.object_view = !self.object_view,
            ControlAction::Walk | ControlAction::Run => {
                if self.world.agent.flying {
                    self.toggle_fly();
                }
                self.set_always_run(action == ControlAction::Run);
            }
            ControlAction::Fly => self.toggle_fly(),
            ControlAction::StandUp => self.send(NetCommand::OneShotControl(control::STAND_UP)),
            ControlAction::UseShot(i) => {
                if let Err(text) = self.apply_camera_preset(i) {
                    self.camera_preset_message(text);
                }
            }
            ControlAction::SaveShot(name) => {
                // LLFloaterSaveCameraPreset::onBtnSave: where the camera is,
                // in the avatar's frame, which becomes the view (the
                // viewfinder's presets keep more)
                let include = crate::camera::presets::Include {
                    anchor: Some(crate::camera::presets::AnchorKind::Avatar),
                    lens: false,
                    looks: false,
                    light: false,
                };
                match self.save_camera_preset(name, include, None) {
                    Ok(()) => self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera),
                    Err(text) => self.camera_preset_message(text),
                }
            }
            ControlAction::DeleteShot(i) => self.delete_camera_preset(i),
        }
    }

    /// AURORA_DEMO_CONTROLS=camera | movement | compact | classic | free |
    /// free-compact: the window open (minimized for `compact`); `free` the
    /// photo window's free camera on, which opens it, `free-compact` the
    /// same minimized; 40 frames later the free camera has flown 2 m to the
    /// left and 1 m up and turned toward the avatar, which stayed.
    fn controls_demo(&mut self) {
        if !self.demo {
            return;
        }
        if self.frame_count == 640
            && std::env::var("AURORA_DEMO_CONTROLS").is_ok_and(|m| m.starts_with("free"))
            && let Some(cam) = self.camera.free_mut()
        {
            let left = glam::Vec3::new(-cam.yaw.sin(), cam.yaw.cos(), 0.0);
            cam.position += left * 2.0 + glam::Vec3::Z;
            cam.turn(-0.35, -0.15);
            log::info!("demo free camera: flown to {:.1?}", cam.position);
        }
        if self.frame_count != 600 {
            return;
        }
        match std::env::var("AURORA_DEMO_CONTROLS").as_deref() {
            Ok(mode @ ("free" | "free-compact")) => {
                self.panels.snapshot = true;
                self.panels.chat = false;
                self.panels.people = false;
                self.snapshots.ui.free.on = true;
                if mode == "free-compact" {
                    crate::ui::widgets::minimize_floater(&self.egui_ctx, crate::ui::controls::WINDOW);
                }
            }
            Ok("camera") => self.panels.camera_controls = true,
            Ok("movement") => self.panels.movement_controls = true,
            Ok("compact") => {
                self.panels.camera_controls = true;
                crate::ui::widgets::minimize_floater(&self.egui_ctx, crate::ui::controls::WINDOW);
            }
            Ok("classic") => {
                self.settings.classic_controls = true;
                self.panels.camera_controls = true;
                self.panels.movement_controls = true;
            }
            _ => {}
        }
    }
}
