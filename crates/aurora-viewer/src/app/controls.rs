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
            precision: self.settings.camera.precision,
            preset: self.settings.camera.preset,
            shots: &self.camera_shots,
            quick_state: self.camera_slots.state,
            quick_write_failed: self.camera_slots.write_failed,
            views: &self.settings.camera_views,
            flying: self.world.agent.flying,
            always_run: self.always_run,
            seated: self.world.agent.seated,
            // the answers `set_always_run`, `toggle_fly` and the send
            // boundary give, so a button never promises what they refuse
            run_locked: self.world.rlva.has("alwaysrun"),
            fly_locked: !self.world.agent.flying && !crate::autopilot::can_takeoff(&self.world),
            stand_locked: !self
                .world
                .rlva_allows_command(&NetCommand::OneShotControl(control::STAND_UP), uuid::Uuid::nil()),
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

    pub(super) fn control_action(&mut self, action: ControlAction) {
        if self.world.rlva.camera_preset_locked(uuid::Uuid::nil())
            && matches!(
                action,
                ControlAction::Preset(_)
                    | ControlAction::UseShot(_)
                    | ControlAction::QuickSave(_, _)
                    | ControlAction::QuickLoad(_)
                    | ControlAction::SaveShot(_)
                    | ControlAction::SaveView(_)
                    | ControlAction::RestoreView(_)
            )
        {
            return;
        }
        match action {
            ControlAction::QuickSetup => self.camera_slots.prompt_open = true,
            ControlAction::QuickImport => self.start_legacy_slot_import(),
            ControlAction::Precision(on) => {
                self.settings.camera.precision = on;
                self.settings.save();
            }
            ControlAction::QuickSave(slot, name) => {
                if let Err(text) = self.save_quick_camera(slot, name) {
                    self.camera_preset_message(text);
                }
            }
            ControlAction::QuickLoad(slot) => {
                if let Some(i) = self.camera_shots.iter().position(|s| s.quick_slot == Some(slot))
                    && let Err(text) = self.apply_camera_preset(i)
                {
                    self.camera_preset_message(text);
                }
            }
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
                // @alwaysrun: the Run mode is greyed (LLFloaterMove::
                // sUpdateMovementStatus), and a refused Run must not land
                // a flying avatar on its way to being refused
                if action == ControlAction::Run && self.world.rlva.has("alwaysrun") {
                    return;
                }
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
        // Quick views exercise world position, roll and lens through the production
        // Save/Load actions, in both control layouts.
        if let Ok(mode @ ("quick" | "classic-quick")) = std::env::var("AURORA_DEMO_CONTROLS").as_deref() {
            match self.frame_count {
                600 => {
                    self.settings.classic_controls = mode == "classic-quick";
                    self.panels.camera_controls = true;
                    self.panels.chat = false;
                    self.panels.people = false;
                    self.control_action(ControlAction::Precision(true));
                    self.camera.roll = 0.4;
                    self.settings.camera.fov =
                        crate::camera::zoom_lens(crate::camera::DEFAULT_FOV, Some(1.0 / 1.2), self.view_aspect(), None);
                    self.control_action(ControlAction::QuickSave(0, "Portrait".into()));
                    log::info!(
                        "demo quick view: saved position {:?}, roll {}, lens {}",
                        self.camera.position,
                        self.camera.roll,
                        self.settings.camera.fov
                    );
                }
                620 => {
                    self.camera.demo_orbit(1.4, 0.35, Some(9.0), &self.settings.camera);
                    self.camera.roll = -0.2;
                    self.settings.camera.fov = crate::camera::zoom_lens(self.settings.camera.fov, Some(1.2), self.view_aspect(), None);
                    log::info!("demo quick view: changed lens {}", self.settings.camera.fov);
                }
                660 => {
                    self.camera.start_flycam(&self.settings.camera, false);
                    self.joystick.active = true;
                    self.control_action(ControlAction::QuickLoad(0));
                    log::info!(
                        "demo quick view: flycam stopped {}",
                        !self.joystick.active && self.camera.flycam.is_none()
                    );
                }
                720 => log::info!(
                    "demo quick view: restored position {:?}, roll {}, lens {}, rendered FOV {}",
                    self.camera.position,
                    self.camera.roll,
                    self.settings.camera.fov,
                    self.camera.fov_y
                ),
                _ => {}
            }
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
