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
            lens_fov: self.camera_lens(),
            roll: self.camera.roll,
            preset: self.settings.camera.preset,
            shots: &self.settings.camera_shots,
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
        let saved = crate::ui::controls::Compact {
            camera: self.settings.compact_controls,
            movement: self.settings.compact_movement_controls,
        };
        // AURORA_DEMO_CONTROLS=compact… and movement-compact: the forms of
        // the run, never saved (`DemoCompact`)
        if self.demo
            && self.demo_compact.is_none()
            && let Some(camera) = demo_compact()
        {
            self.demo_compact = Some(DemoCompact::start(saved, camera));
        }
        let before = match self.demo_compact.as_mut() {
            Some(run) => run.shown(saved),
            None => saved,
        };
        let mut compact = before;
        let out = self.controls_ui.show(
            ctx,
            p,
            &mut self.panels.camera_controls,
            &mut self.panels.movement_controls,
            &mut compact,
            self.settings.classic_controls,
            &view,
        );
        if compact != before {
            if let Some(run) = self.demo_compact.as_mut() {
                run.run = compact;
            } else {
                self.settings.compact_controls = compact.camera;
                self.settings.compact_movement_controls = compact.movement;
                self.settings.save();
            }
        }
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

    /// The toolbar's Move and View, the World menu, the shortcut
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

    pub(super) fn camera_lens(&self) -> f32 {
        let shown = self.world.rlva.camera_settings(&self.settings.camera, self.view_aspect()).fov;
        self.camera.flycam.as_ref().map_or(shown, |flight| flight.fov)
    }

    pub(super) fn change_camera_lens(&mut self, fov: f32, factor: Option<f32>) {
        // LLZoomer (indra/newview/llviewermenu.cpp): a user lens change
        // replaces a forced FOV even under @setcam; FOV bounds still apply.
        let aspect = self.view_aspect();
        let fov = crate::camera::zoom_lens(fov, factor, aspect, self.world.rlva.camera_fov_limits(aspect));
        self.settings.camera.fov = fov;
        self.world.rlva.user_chose_fov();
        if let Some(flight) = &mut self.camera.flycam {
            flight.set_fov(fov);
        }
        if !self.demo {
            self.settings.save();
        }
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
            ControlAction::Lens(fov) => self.change_camera_lens(fov, Some(1.0)),
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
            ControlAction::Position => {
                crate::ui::widgets::toggle_floater(&self.egui_ctx, crate::ui::camera_position::WINDOW, &mut self.panels.camera_position)
            }
        }
    }

    /// The camera position window (#4587): the live offsets and distance.
    /// The camera reads them each frame; RLVa keeps them under its lock, and
    /// an edit ends the offsets a force put in place, as an edit in
    /// Preferences does (`preserve_camera_settings`). Saved once the
    /// pointer is released; its save goes the Controls window's way.
    pub(super) fn camera_position_window(&mut self, ctx: &egui::Context, p: &Palette) {
        self.camera_position_demo();
        let locked = self.world.rlva.camera_preset_locked(uuid::Uuid::nil());
        let mut open = self.panels.camera_position;
        let before = self.settings.camera.clone();
        let out = self.camera_position_ui.show(ctx, p, &mut open, &mut self.settings, locked);
        self.panels.camera_position = open;
        if out.changed {
            self.world.rlva.preserve_camera_settings(&mut self.settings.camera, &before);
        }
        if out.save_settings {
            self.settings.save();
        }
        if let Some(name) = out.save {
            self.control_action(ControlAction::SaveShot(name));
        }
    }

    /// AURORA_DEMO_CONTROLS=position | classic-position (#4587): the
    /// Controls window (Aurora's or the Legacy Viewer's) and the camera
    /// position window open at 600; at 640 the camera is raised 0.5 m as
    /// five Ctrl + wheel clicks do, which the window's Z shows at once. The
    /// log gives the offsets before and after.
    fn camera_position_demo(&mut self) {
        if !self.demo
            || !matches!(
                std::env::var("AURORA_DEMO_CONTROLS").as_deref(),
                Ok("position" | "classic-position")
            )
        {
            return;
        }
        match self.frame_count {
            600 => {
                self.settings.classic_controls = std::env::var("AURORA_DEMO_CONTROLS").as_deref() == Ok("classic-position");
                self.panels.camera_controls = true;
                crate::ui::widgets::place_floater(
                    &self.egui_ctx,
                    crate::ui::camera_position::WINDOW,
                    egui::pos2(360.0, 120.0),
                    egui::vec2(crate::ui::camera_position::WIDTH, 200.0),
                );
                self.panels.camera_position = true;
                log::info!("demo camera position: camera offset {:?}", self.settings.camera.camera_offset);
            }
            640 => {
                let c = &mut self.settings.camera;
                c.camera_offset[2] += 0.5;
                c.preset = crate::camera::CameraPreset::Custom;
                log::info!("demo camera position: raised to {:?}", c.camera_offset);
            }
            _ => {}
        }
    }

    /// AURORA_DEMO_CONTROLS=camera | roll | movement | compact |
    /// compact-move | compact-close | compact-move-back | movement-compact |
    /// classic | free | free-compact: the window open (minimized for
    /// `compact`; `compact-move` also, then Move pressed 40 frames later as
    /// the toolbar does, #1815; `compact-close` the camera's toolbar button
    /// pressed at 640, which closes it, and again at 680, which opens it
    /// compact, #3904; `compact-move-back` Move pressed at 640, then the
    /// window's Camera tab at 660: compact again, logged at 680, #3904;
    /// `movement-compact` the Movement tab shown compact at 640, the run's
    /// form, its saved choice left as it is: the movement pad and walk / run
    /// / fly); `free` the
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
        if std::env::var("AURORA_DEMO_CONTROLS").as_deref() == Ok("compact-move-back") {
            let compact = || crate::ui::widgets::floater_minimized(&self.egui_ctx, crate::ui::controls::WINDOW);
            match self.frame_count {
                640 => self.request_controls(true, true),
                660 => {
                    log::info!("demo controls: Move pressed, compact {}", compact());
                    // the window's Camera tab clicked
                    self.controls_ui.tab = crate::ui::controls::Tab::Camera;
                }
                680 => log::info!(
                    "demo controls: back by the Camera tab, camera {} compact {}",
                    self.panels.camera_controls,
                    compact()
                ),
                _ => {}
            }
        }
        if self.frame_count == 640 && std::env::var("AURORA_DEMO_CONTROLS").as_deref() == Ok("movement-compact") {
            // the run's Movement form, applied to the open tab at once
            // as a Preferences switch is; nothing saved
            if let Some(run) = self.demo_compact.as_mut() {
                run.run.movement = true;
            }
            log::info!("demo controls: the Movement tab shown compact");
        }
        if std::env::var("AURORA_DEMO_CONTROLS").as_deref() == Ok("compact-close") {
            let compact = || crate::ui::widgets::floater_minimized(&self.egui_ctx, crate::ui::controls::WINDOW);
            match self.frame_count {
                640 => {
                    self.request_controls(false, true);
                    log::info!("demo controls: camera pressed on the compact window");
                }
                660 => log::info!("demo controls: open {} after the press", self.panels.camera_controls),
                680 => self.request_controls(false, true),
                700 => log::info!(
                    "demo controls: open {}, compact {} when pressed again",
                    self.panels.camera_controls,
                    compact()
                ),
                _ => {}
            }
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
            Ok("movement" | "movement-compact") => self.panels.movement_controls = true,
            // compact for this run only (`DemoCompact`, AGENTS.md section 4)
            Ok("compact" | "compact-move" | "compact-move-back" | "compact-close") => self.panels.camera_controls = true,
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

/// AURORA_DEMO_CONTROLS=compact | compact-move | compact-move-back |
/// compact-close | movement-compact: the run keeps its own compact forms
/// (`DemoCompact`), whatever the saved choices; `Some(true)` when the camera
/// starts compact, `Some(false)` for `movement-compact` (its Movement tab
/// made compact at 640).
fn demo_compact() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("AURORA_DEMO_CONTROLS").as_deref() {
        Ok("compact" | "compact-move" | "compact-move-back" | "compact-close") => Some(true),
        Ok("movement-compact") => Some(false),
        _ => None,
    })
}

/// The compact forms of a demo run (AGENTS.md section 4: a test never
/// changes the shared settings). The window's title button switches the
/// run's form, never the saved one; a Preferences switch (the saved choice
/// changing) still takes over its tab, as it does outside the demo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DemoCompact {
    /// The forms the window shows for the run.
    pub run: crate::ui::controls::Compact,
    /// The saved choices of the last frame, to see a Preferences switch.
    saved: crate::ui::controls::Compact,
}

impl DemoCompact {
    /// The run's start: the camera compact or not, the movement full.
    fn start(saved: crate::ui::controls::Compact, camera: bool) -> Self {
        Self {
            run: crate::ui::controls::Compact { camera, movement: false },
            saved,
        }
    }

    /// The forms to show this frame: a saved choice changed since the last
    /// frame (Preferences) replaces the run's form of its tab.
    fn shown(&mut self, saved: crate::ui::controls::Compact) -> crate::ui::controls::Compact {
        if saved.camera != self.saved.camera {
            self.run.camera = saved.camera;
        }
        if saved.movement != self.saved.movement {
            self.run.movement = saved.movement;
        }
        self.saved = saved;
        self.run
    }
}

#[cfg(test)]
mod tests {
    use super::DemoCompact;
    use crate::ui::controls::Compact;

    /// #3904: a demo run keeps its own compact forms (AGENTS.md section 4),
    /// and a Preferences switch of a saved choice takes over that tab.
    #[test]
    fn a_demo_run_keeps_its_forms_until_preferences_switch_one() {
        let saved = Compact {
            camera: false,
            movement: true,
        };
        let mut run = DemoCompact::start(saved, true);
        assert_eq!(
            run.shown(saved),
            Compact {
                camera: true,
                movement: false
            },
            "the run's start"
        );
        // the title button switched the run's camera to full
        run.run.camera = false;
        assert_eq!(
            run.shown(saved),
            Compact {
                camera: false,
                movement: false
            },
            "kept, the saved choices unchanged"
        );
        // Preferences turns compact movement off, then compact camera on
        let saved = Compact {
            camera: false,
            movement: false,
        };
        assert_eq!(
            run.shown(saved),
            Compact {
                camera: false,
                movement: false
            },
            "movement as saved"
        );
        let saved = Compact {
            camera: true,
            movement: false,
        };
        assert_eq!(
            run.shown(saved),
            Compact {
                camera: true,
                movement: false
            },
            "camera as saved"
        );
        run.run.movement = true;
        assert_eq!(
            run.shown(saved),
            Compact {
                camera: true,
                movement: true
            },
            "the title button again"
        );
    }
}
