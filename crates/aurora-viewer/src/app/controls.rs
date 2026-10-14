//! The Controls window in the app (`ui/controls.rs`): what its buttons and
//! joysticks do, as the keys and the menus do (LLFloaterCamera,
//! LLFloaterMove, LLFloaterSaveCameraPreset).

use super::App;
use crate::theme::Palette;
use crate::ui::controls::{ControlAction, View};
use aurora_net::{NetCommand, control};

impl App {
    pub(super) fn controls_window(&mut self, ctx: &egui::Context, p: &Palette) {
        self.controls_demo();
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
        // the object view ends with the window (LLFloaterCamera::onClose)
        if !self.panels.camera_controls && !self.panels.movement_controls {
            self.object_view = false;
        }
        for action in out.actions {
            self.control_action(action);
        }
    }

    fn control_action(&mut self, action: ControlAction) {
        match action {
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
                if let Some(shot) = self.settings.camera_shots.get(i).cloned() {
                    self.settings.camera.apply_shot(&shot);
                    self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
                    self.settings.save();
                }
            }
            ControlAction::SaveShot(name) => {
                // LLFloaterSaveCameraPreset::onBtnSave: where the camera is,
                // which becomes the view
                let (camera, focus) = self.camera.current_offsets(&self.world.agent);
                let shot = crate::camera::CameraShot {
                    name,
                    camera_offset: camera.to_array(),
                    focus_offset: focus.to_array(),
                };
                log::info!("camera position saved: {}", shot.name);
                self.settings.camera.apply_shot(&shot);
                match self.settings.camera_shots.iter_mut().find(|s| s.name == shot.name) {
                    Some(old) => *old = shot,
                    None => self.settings.camera_shots.push(shot),
                }
                self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
                self.settings.save();
            }
            ControlAction::DeleteShot(i) => {
                if i < self.settings.camera_shots.len() {
                    self.settings.camera_shots.remove(i);
                    self.settings.save();
                }
            }
        }
    }

    /// AURORA_DEMO_CONTROLS=camera | movement | classic: the window open.
    fn controls_demo(&mut self) {
        if !self.demo || self.frame_count != 600 {
            return;
        }
        match std::env::var("AURORA_DEMO_CONTROLS").as_deref() {
            Ok("camera") => self.panels.camera_controls = true,
            Ok("movement") => self.panels.movement_controls = true,
            Ok("classic") => {
                self.settings.classic_controls = true;
                self.panels.camera_controls = true;
                self.panels.movement_controls = true;
            }
            _ => {}
        }
    }
}
