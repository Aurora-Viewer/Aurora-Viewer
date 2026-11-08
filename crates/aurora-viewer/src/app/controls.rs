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
            roll: self.camera.roll,
            preset: self.settings.camera.preset,
            shots: &self.settings.camera_shots,
            views: &self.settings.camera_views,
            flying: self.world.agent.flying,
            always_run: self.always_run,
            run_locked: self.world.rlva.has("alwaysrun"),
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

    /// The toolbar's Move and View, the View menu, the shortcut
    /// (`ui::controls::request`; the free camera keeps Aurora's window).
    pub(super) fn request_controls(&mut self, movement: bool, toggle: bool) {
        let classic = self.settings.classic_controls && self.camera.free().is_none();
        crate::ui::controls::request(
            &self.egui_ctx,
            &mut self.panels.camera_controls,
            &mut self.panels.movement_controls,
            classic,
            movement,
            toggle,
        );
    }

    fn control_action(&mut self, action: ControlAction) {
        if self.world.rlva.camera_preset_locked(uuid::Uuid::nil())
            && matches!(
                action,
                ControlAction::Preset(_)
                    | ControlAction::UseShot(_)
                    | ControlAction::SaveShot(_)
                    | ControlAction::SaveView(_)
                    | ControlAction::RestoreView(_)
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
            ControlAction::Preset(preset) => self.switch_camera_view(preset),
            ControlAction::ResetOrbit => self.reset_camera_view(),
            ControlAction::ResetCamera => {
                self.reset_camera_all();
                self.settings.save();
            }
            ControlAction::RollBy(delta) => {
                if self.in_world() && !self.camera.mouselook() && self.camera.free().is_none() && self.camera.flycam.is_none() {
                    self.camera.roll += delta;
                }
            }
            ControlAction::LevelRoll => self.camera.roll = 0.0,
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
                // a view's name replaces the view, as in the Legacy Viewer
                if let Some(view) = crate::camera::CameraPreset::from_preset_name(&name) {
                    self.save_camera_view(view);
                    return;
                }
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
            ControlAction::SaveView(view) => self.save_camera_view(view),
            ControlAction::RestoreView(view) => self.restore_camera_view(view),
        }
    }

    /// AURORA_DEMO_CONTROLS=camera | movement | compact | compact-move |
    /// classic | free | free-compact: the window open (minimized for
    /// `compact`; `compact-move` also, then Move pressed 40 frames later as
    /// the toolbar does, #1815); `free` the
    /// photo window's free camera on, which opens it, `free-compact` the
    /// same minimized; 40 frames later the free camera has flown 2 m to the
    /// left and 1 m up and turned toward the avatar, which stayed.
    /// AURORA_DEMO_CONTROLS=views | classic-views (issue #2755): the
    /// window open at 600 (Aurora's or the Legacy Viewer's); the camera
    /// turned away at 610 and saved as the Front view at 620, as its right
    /// click does; the Rear view at 640, the Front view again at 660. The log
    /// gives the camera when saved and at 700: the same place.
    fn controls_views_demo(&mut self) {
        if !matches!(std::env::var("AURORA_DEMO_CONTROLS").as_deref(), Ok("views" | "classic-views")) {
            return;
        }
        let preset = crate::camera::CameraPreset::Front;
        match self.frame_count {
            610 => self.camera.demo_orbit(0.9, 0.25, Some(4.0), &self.settings.camera),
            620 => {
                log::info!("demo controls: camera at {:.2?} when saved", self.camera.position);
                self.control_action(ControlAction::SaveView(preset));
            }
            640 => self.control_action(ControlAction::Preset(crate::camera::CameraPreset::Rear)),
            660 => self.control_action(ControlAction::Preset(preset)),
            700 => log::info!("demo controls: camera at {:.2?} on the saved Front view", self.camera.position),
            _ => {}
        }
    }

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
        if self.frame_count == 640 && std::env::var("AURORA_DEMO_CONTROLS").as_deref() == Ok("compact-move") {
            self.request_controls(true, true);
            log::info!("demo controls: Move pressed on the minimized window");
        }
        self.controls_views_demo();
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
            Ok("roll") => {
                self.panels.camera_controls = true;
                self.camera.roll = 15.0_f32.to_radians();
            }
            Ok("movement") => self.panels.movement_controls = true,
            Ok("compact" | "compact-move") => {
                self.panels.camera_controls = true;
                crate::ui::widgets::minimize_floater(&self.egui_ctx, crate::ui::controls::WINDOW);
            }
            Ok("classic") => {
                self.settings.classic_controls = true;
                self.panels.camera_controls = true;
                self.panels.movement_controls = true;
            }
            Ok("views") => self.panels.camera_controls = true,
            Ok("classic-views") => {
                self.settings.classic_controls = true;
                self.panels.camera_controls = true;
            }
            _ => {}
        }
    }
}
