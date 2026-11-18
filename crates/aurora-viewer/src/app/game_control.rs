//! Game Controls in the frame: the controllers read, GameControlData sent
//! to the region, and what the controller does to the avatar, the flycam and
//! the cursor. Port of the game-control steps of LLAppViewer::idle
//! (indra/newview/llappviewer.cpp), LLAgent::updateGameControlMode,
//! applyExternalActions, toggleMouseCursorMode and resetToAvatarMode
//! (indra/newview/llagent.cpp), LLAgentCamera::updateFlycam and
//! updateFlycamCursorVisibility (indra/newview/llagentcamera.cpp) and
//! find_nearby_seat (indra/newview/llviewermenu.cpp), LL viewer 26.5,
//! originally LGPL 2.1.
//!
//! The data goes to the region whatever drives the avatar. The local control
//! applies only while the Aurora gamepad profile is off: both read the same
//! gamepads and would move the avatar twice (LL keeps 3D mice and gamepads
//! apart for the same reason, LLViewerJoystick::is3DConnexionDevice).

use super::App;
use crate::agent::MoveInput;
use crate::game_control::{
    Mode, actions,
    engine::{Cursor, Frame},
    flycam,
};
use aurora_net::{NetCommand, control};
use std::f32::consts::FRAC_PI_2;
use std::time::{Duration, Instant};
use winit::event::{ElementState, MouseButton, WindowEvent};

/// The cursor mode's pointer at full deflection (agent.cpp).
const CURSOR_SPEED: f32 = 1200.0;
/// Scroll clicks a second at full deflection.
const SCROLL_CLICKS: f32 = 8.0;
/// Zoom: the view divided by 1.2 a second at full deflection.
const ZOOM_BASE: f32 = 1.2;
/// SEAT_SEARCH_RANGE_HEIGHTS: a seat within twice the avatar's height.
const SEAT_SEARCH_HEIGHTS: f32 = 2.0;
/// The avatar's height before its shape is known.
const DEFAULT_BODY_HEIGHT: f32 = 1.9;
/// updateFlycamCursorVisibility's delays.
const CURSOR_SHOWN_AFTER_MOVE: Duration = Duration::from_secs(5);
const CURSOR_SHOWN_AFTER_FLIGHT: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct State {
    pub engine: crate::game_control::Engine,
    run: actions::RunLatch,
    /// mUsingMouseCursor.
    pub cursor_mode: bool,
    /// The flycam is this controller's (not the 3D mouse's).
    pub flycam: bool,
    flycam_state: flycam::State,
    /// Last frame's flags of the keyboard and the 3D mouse.
    keyboard_flags: u32,
    /// This frame's local control, `None` when the controller drives
    /// nothing.
    actions: Option<actions::AgentActions>,
    /// The avatar runs because of the stick (sent as SetAlwaysRun).
    pub running: bool,
    mouse_bits: u32,
    last_cursor: Option<Instant>,
    cursor_rest: (f32, f32),
    /// Clicks of the controller waiting for the event loop.
    pub synthetic: Vec<(MouseButton, bool)>,
    hide: CursorHide,
    /// A binding popup of the preferences waits for an input: nothing is
    /// sent and the controller drives nothing (isWaitingForInputChannel).
    pub capturing: bool,
    /// AURORA_DEMO_GAME_CONTROL has started: its synthetic gamepad reads as
    /// focused, like the joystick demo's.
    demo_started: bool,
}

/// updateFlycamCursorVisibility: the cursor hides while the flycam flies.
#[derive(Default)]
struct CursorHide {
    wanted: bool,
    hidden: bool,
    deadline: Option<Instant>,
    last_cursor: (f32, f32),
    last_view: Option<(glam::Vec3, glam::Quat, f32)>,
}

impl App {
    /// Each frame, before the movement keys: read the controllers, send
    /// GameControlData when due, work out the controller's actions.
    pub(super) fn game_control_frame(&mut self, dt: f32) {
        let now = Instant::now();
        if self.camera.flycam.is_none() {
            self.game_control.flycam = false;
        }
        self.game_control_demo();
        let focused = self.focused || self.game_control.demo_started;
        self.game_control
            .engine
            .update_controllers(&self.joystick.snapshot.devices, &self.settings.game_control, focused);
        self.game_control_preferences();
        let gc = &mut self.game_control;
        if let Some(fps) = self.world.sim_fps {
            gc.engine.set_server_fps(fps);
        }
        let flycam_engaged = self.camera.flycam.is_some();
        let mode = Mode::of(
            gc.cursor_mode,
            flycam_engaged,
            self.camera.mouselook(),
            self.world.agent.is_sitting(),
        );
        let cursor = (mode == Mode::Cursor)
            .then(|| self.gfx.as_ref().map(|g| g.window.inner_size()))
            .flatten()
            .map(|size| {
                let (w, h) = (size.width as i32, size.height as i32);
                Cursor {
                    x: (self.cursor_pos.0 as i32).clamp(0, w),
                    y: (self.cursor_pos.1 as i32).clamp(0, h),
                    width: w,
                    height: h,
                    left: self.left_down,
                    right: self.right_drag,
                }
            });
        let frame = Frame {
            now,
            mode,
            flycam_engaged,
            external_flags: gc.keyboard_flags,
            external_buttons: 0,
            running: self.always_run || self.temp_run.is_some() || gc.running,
            cursor,
        };
        gc.keyboard_flags = 0;
        gc.actions = None;
        let due = gc.engine.compute(&self.settings.game_control, &frame);
        if gc.capturing {
            return;
        }
        let in_world = self.in_world();
        // (the deadline waits while the region does not take it: the first
        // frame where it does sends at once)
        if due && in_world && self.world.game_control_enabled() {
            let packet = self.game_control.engine.packet(now);
            self.send(NetCommand::GameControlData(packet));
        }
        let s = &self.settings.game_control;
        let gc = &mut self.game_control;
        let drives = !self.settings.gamepad.enabled && gc.engine.enabled() && in_world && mode != Mode::None && s.mode_enabled(mode);
        if !drives {
            gc.mouse_bits = 0;
            return;
        }
        let a = actions::agent_actions(&gc.engine, &mut gc.run);
        gc.actions = Some(a);
        self.game_control_misc(a.misc);
        self.game_control_cursor(mode, &a, dt, now);
        self.game_control_clicks(a.mouse_buttons);
        if self.camera.flycam.is_none() {
            self.game_control_camera(&a, dt);
            self.game_control_run(&a);
        }
    }

    /// AURORA_DEMO_GAME_CONTROL=<scenario>: from frame 600, a synthetic
    /// gamepad through the production path (the joystick worker's snapshot),
    /// the Aurora gamepad profile off and the demo region taking
    /// GameControlData. `actions`, `device-mappings`, `device-options` and
    /// `server-data` open that tab of Preferences › Game Controls while the
    /// left stick walks; `walk` walks, runs and turns with the sticks;
    /// `flycam` clicks the right stick into the flycam and flies it;
    /// `cursor` presses Select into the cursor mode and moves the pointer.
    fn game_control_demo(&mut self) {
        if !self.demo || self.frame_count < 600 {
            return;
        }
        let Ok(scenario) = std::env::var("AURORA_DEMO_GAME_CONTROL") else {
            return;
        };
        if !self.game_control.demo_started {
            self.game_control.demo_started = true;
            self.settings.gamepad.enabled = false;
            self.settings.game_control = Default::default();
            if let Some(h) = self.world.main_region
                && let Some(r) = self.world.regions.get_mut(&h)
            {
                r.game_control = true;
            }
            let tab = match scenario.as_str() {
                "actions" => Some(0),
                "device-mappings" => Some(1),
                "device-options" => Some(2),
                "server-data" => Some(3),
                _ => None,
            };
            if let Some(tab) = tab {
                self.panels.settings = true;
                self.options_ui.open(crate::ui::options::Page::GameControls, tab);
            }
        }
        // crate::joystick::GamepadButton::STANDARD: Select is 8, the right
        // stick 12; the sticks are axes 0/1 and 3/4, the triggers 6 and 7.
        let mut d = crate::joystick::Device::gamepad(
            "demo-gamepad".into(),
            "Demo gamepad".into(),
            8,
            [[Some(0), Some(1)], [Some(3), Some(4)]],
            [Some(6), Some(7)],
            crate::joystick::GamepadStyle::Generic,
            crate::joystick::GamepadButton::STANDARD.to_vec(),
        );
        let f = self.frame_count;
        match scenario.as_str() {
            "walk" => {
                if (650..1100).contains(&f) {
                    d.axes[1] = if f < 850 { -0.6 } else { -0.95 };
                }
                if (900..1000).contains(&f) {
                    d.axes[3] = 0.5;
                }
            }
            "flycam" => {
                if (630..634).contains(&f) {
                    d.buttons |= 1 << 12;
                }
                if (660..1000).contains(&f) {
                    d.axes[1] = -0.6;
                    d.axes[3] = 0.3;
                }
            }
            "cursor" => {
                if (630..634).contains(&f) {
                    d.buttons |= 1 << 8;
                }
                if (660..900).contains(&f) {
                    d.axes[0] = 0.5;
                    d.axes[1] = -0.3;
                }
            }
            _ => {
                if f >= 650 {
                    d.axes[1] = -0.6;
                    d.axes[7] = 0.5;
                    d.buttons |= 1;
                }
            }
        }
        self.joystick.snapshot.devices = vec![d];
        if f.is_multiple_of(100) {
            let gc = &self.game_control;
            log::info!(
                "game control demo: frame={f} mode={} sent={} flycam={} cursor={} position={:?}",
                gc.engine.mode.name(),
                gc.engine.sent_state.is_some(),
                gc.flycam,
                gc.cursor_mode,
                self.world.agent.position
            );
        }
    }

    /// The preferences' capture and Auto calibrate, which last only while
    /// their tab shows (a tab change closes LL's popups too).
    fn game_control_preferences(&mut self) {
        use crate::ui::options::Page;
        let shown = |tab: usize| self.panels.settings && self.options_ui.page == Page::GameControls && self.options_ui.tab == tab;
        if !shown(0) {
            self.options_ui.game_control_capture = None;
        }
        if !shown(2) {
            self.options_ui.game_control_calibration = None;
        }
        self.game_control.capturing = self.options_ui.game_control_capture.is_some();
        let Some(cal) = self.options_ui.game_control_calibration.as_mut() else {
            return;
        };
        let Some(c) = self.game_control.engine.controllers.iter().find(|c| c.id == cal.device) else {
            // the device went away: no calibration
            self.options_ui.game_control_calibration = None;
            return;
        };
        cal.samples.push(c.reading.axes);
        if cal.samples.len() < crate::game_control::device::CALIBRATION_FRAMES {
            return;
        }
        let (id, name, samples) = (cal.device.clone(), c.name.clone(), std::mem::take(&mut cal.samples));
        self.options_ui.game_control_calibration = None;
        let config = self.settings.game_control.devices.get(&id).cloned().unwrap_or_default();
        let mut options = crate::game_control::device::Options::load(&config);
        crate::game_control::device::calibrate(&mut options, &samples);
        self.settings.game_control.devices.insert(id, options.save(&name, true));
        self.settings.save();
    }

    /// Steps 1 to 7 of applyExternalActions: the one-shot commands.
    fn game_control_misc(&mut self, misc: u32) {
        if misc & actions::TOGGLE_FLY != 0 {
            self.toggle_fly();
        }
        if misc & actions::TOGGLE_FLYCAM != 0 {
            self.game_control_toggle_flycam();
        }
        if misc & actions::TOGGLE_SIT != 0 {
            self.game_control_toggle_sit();
        }
        if misc & actions::TOGGLE_SPEAK != 0 {
            // LLVoiceClient::toggleUserPTTState
            if self.settings.audio.mic_hold {
                self.mic_button_held = !self.mic_button_held;
            } else {
                self.toggle_mic();
            }
        }
        if misc & actions::TOGGLE_MOUSELOOK != 0 {
            self.toggle_mouselook();
        }
        if misc & actions::TOGGLE_MOUSE_CURSOR != 0 {
            self.game_control.cursor_mode = !self.game_control.cursor_mode;
            self.game_control.last_cursor = None;
        }
        if misc & actions::ESCAPE != 0 {
            self.game_control_reset_to_avatar();
        }
    }

    /// resetToAvatarMode: cursor mode off, flycam off, out of mouselook.
    fn game_control_reset_to_avatar(&mut self) {
        self.game_control.cursor_mode = false;
        if self.camera.flycam.is_some() {
            self.game_control_flycam_off();
        }
        if self.camera.mouselook() {
            self.toggle_mouselook();
        }
    }

    /// LLAgentCamera::toggleFlycam from the controller: no condition, the
    /// flycam starts at the current view and the camera comes back to the
    /// avatar when it ends.
    fn game_control_toggle_flycam(&mut self) {
        if self.camera.flycam.is_some() {
            self.game_control_flycam_off();
            return;
        }
        if !self.in_world() || self.vr_mode.is_some() {
            return;
        }
        let was_mouselook = self.camera.mouselook();
        self.stop_autopilot(true);
        self.camera.end_free(&self.settings.camera);
        self.snapshots.ui.free.on = false;
        self.photo_free_camera();
        self.camera.start_flycam(&self.settings.camera, was_mouselook);
        if was_mouselook {
            self.set_mouselook_grab(false);
        }
        self.game_control.flycam = true;
        self.game_control.flycam_state = flycam::State::default();
        self.last_input = Instant::now();
    }

    fn game_control_flycam_off(&mut self) {
        self.camera.clear_flycam();
        self.joystick.active = false;
        self.game_control.flycam = false;
        self.game_control.flycam_state = flycam::State::default();
        if self.camera.mouselook() {
            self.set_mouselook_grab(true);
        }
    }

    /// Toggle sit (applyExternalActions): stand up, or sit on the nearest
    /// seat within reach (find_nearby_seat), or on the ground.
    fn game_control_toggle_sit(&mut self) {
        if self.world.agent.is_sitting() {
            self.send(NetCommand::OneShotControl(control::STAND_UP));
            return;
        }
        let now = Instant::now();
        let height = self
            .scene
            .neutral_body_height(&self.world.agent_id)
            .unwrap_or(DEFAULT_BODY_HEIGHT)
            .max(0.5);
        let reach = SEAT_SEARCH_HEIGHTS * height;
        let me = self.world.agent.position;
        let mut best: Option<(f32, uuid::Uuid, aurora_net::RegionHandle)> = None;
        for (idx, o) in self.world.objects.iter() {
            if o.pcode != aurora_prim::params::LL_PCODE_VOLUME
                || o.is_attachment()
                || o.click_action != crate::build::edits::CLICK_ACTION_SIT
            {
                continue;
            }
            let Some((pos, _, hud)) = crate::scene::Scene::object_transform(&self.world, idx, now, 0) else {
                continue;
            };
            let d2 = (pos - me).length_squared();
            // (ties: the later one, like the list's walk)
            if !hud && d2 <= reach * reach && best.is_none_or(|(b, _, _)| d2 <= b) {
                best = Some((d2, o.full_id, o.key.region));
            }
        }
        match best {
            Some((_, target, handle)) => self.send(NetCommand::RequestSit {
                handle,
                target,
                offset: glam::Vec3::ZERO,
            }),
            None => self.send(NetCommand::OneShotControl(control::SIT_ON_GROUND)),
        }
    }

    /// Step 8: the cursor mode's pointer, moved across the whole window.
    fn game_control_cursor(&mut self, mode: Mode, a: &actions::AgentActions, _dt: f32, now: Instant) {
        if mode != Mode::Cursor {
            self.game_control.last_cursor = None;
            self.game_control.cursor_rest = (0.0, 0.0);
            return;
        }
        let dt = self.game_control.last_cursor.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.game_control.last_cursor = Some(now);
        if a.mouse_dx == 0.0 && a.mouse_dy == 0.0 {
            return;
        }
        let Some(g) = &self.gfx else { return };
        let size = g.window.inner_size();
        // round(DX · 1200 · dt); +DY is up on the stick, down on the screen
        let dx = (a.mouse_dx * CURSOR_SPEED * dt).round();
        let dy = (a.mouse_dy * CURSOR_SPEED * dt).round();
        let x = (self.cursor_pos.0 + dx).clamp(0.0, size.width as f32);
        let y = (self.cursor_pos.1 + dy).clamp(0.0, size.height as f32);
        if (x, y) != self.cursor_pos {
            let _ = g
                .window
                .set_cursor_position(winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y)));
            self.cursor_pos = (x, y);
            self.last_input = now;
        }
    }

    /// Step 9: the simulated mouse buttons, by their edges, through the
    /// window's own path (`game_control_dispatch`).
    fn game_control_clicks(&mut self, bits: u32) {
        let gc = &mut self.game_control;
        let changed = bits ^ gc.mouse_bits;
        for (bit, button) in [(actions::MOUSE_LEFT, MouseButton::Left), (actions::MOUSE_RIGHT, MouseButton::Right)] {
            if changed & bit != 0 {
                gc.synthetic.push((button, bits & bit != 0));
            }
        }
        gc.mouse_bits = bits;
    }

    /// The clicks of the controller, handed to the window as real ones.
    pub(super) fn game_control_dispatch(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.game_control.synthetic.is_empty() {
            return;
        }
        let Some(id) = self.gfx.as_ref().map(|g| g.window.id()) else {
            self.game_control.synthetic.clear();
            return;
        };
        for (button, pressed) in std::mem::take(&mut self.game_control.synthetic) {
            let event = WindowEvent::MouseInput {
                device_id: winit::event::DeviceId::dummy(),
                state: if pressed { ElementState::Pressed } else { ElementState::Released },
                button,
            };
            winit::application::ApplicationHandler::window_event(self, event_loop, id, event);
        }
    }

    /// Steps 18 and 19: the zoom of the view and the camera's distance.
    fn game_control_camera(&mut self, a: &actions::AgentActions, dt: f32) {
        let dt = dt.clamp(0.0, 0.2);
        if a.zoom != 0.0 {
            let range = crate::camera::FOV_RANGE;
            let fov = self.settings.camera.fov * ZOOM_BASE.powf(-a.zoom * dt);
            self.settings.camera.fov = fov.clamp(*range.start(), *range.end());
        }
        if a.scroll != 0.0 {
            let clicks = a.scroll * SCROLL_CLICKS * dt;
            self.camera
                .scroll(clicks, false, false, &mut self.world.agent, &mut self.settings.camera);
        }
    }

    /// Step 10: the stick's run, sent like a run toggle.
    fn game_control_run(&mut self, a: &actions::AgentActions) {
        if !a.analog_movement {
            return;
        }
        let running = a.running && !self.world.rlva.has("temprun");
        if running != self.game_control.running {
            self.game_control.running = running;
            if !self.always_run && self.temp_run.is_none() && !self.joystick.running {
                self.send(NetCommand::SetAlwaysRun(running));
            }
        }
    }

    /// The keyboard's flags of this frame, for the next one's GameControlData
    /// (setExternalInput reads them before the controller adds its own).
    pub(super) fn game_control_keyboard(&mut self, i: &MoveInput) {
        self.game_control.keyboard_flags = self.world.agent.control_flags(i, self.camera.mouselook());
    }

    /// Steps 13 to 17: the controller walks, flies and turns the avatar.
    pub(super) fn game_control_avatar(&mut self, i: &mut MoveInput) {
        let Some(a) = self.game_control.actions else { return };
        if self.camera.flycam.is_some() || self.camera.free().is_some() {
            return;
        }
        let f = a.control_flags;
        i.forward |= f & control::AT_POS != 0;
        i.back |= f & control::AT_NEG != 0;
        i.strafe_left |= f & control::LEFT_POS != 0;
        i.strafe_right |= f & control::LEFT_NEG != 0;
        i.up |= f & control::UP_POS != 0;
        i.down |= f & control::UP_NEG != 0;
        let mode = self.game_control.engine.mode;
        let s = &self.settings.game_control;
        let (turn, look) = match mode {
            Mode::Mouselook => (s.speed_factor(Mode::Mouselook), s.speed_factor(Mode::Mouselook)),
            Mode::Avatar => (s.speed_factor(Mode::Avatar), 1.0),
            _ => (1.0, 1.0),
        };
        // LLAgent::propagate: 90°/s a unit of yawKey / pitchKey
        let sign = |pos: u32, neg: u32| {
            if f & pos != 0 {
                1.0
            } else if f & neg != 0 {
                -1.0
            } else {
                0.0
            }
        };
        let yaw = sign(control::YAW_POS, control::YAW_NEG) * a.yaw.abs() * turn * FRAC_PI_2;
        let pitch = sign(control::PITCH_POS, control::PITCH_NEG) * a.pitch.abs() * look * FRAC_PI_2;
        if yaw != 0.0 {
            i.joystick_yaw += yaw;
            i.joystick_mouselook_yaw = true;
        }
        i.joystick_pitch += pitch;
    }

    /// LLAgentCamera::updateFlycam with this controller's flycam: the
    /// controller and the movement keys fly it.
    pub(super) fn game_control_flycam_step(&mut self, keys: &MoveInput, dt: f32) {
        let s = &self.settings.game_control;
        let mut inputs = actions::flycam_inputs(&self.game_control.engine, s.mode_enabled(Mode::Flycam));
        if self.game_control.capturing || self.settings.gamepad.enabled {
            inputs = actions::FlycamInputs::default();
        }
        if inputs.misc & actions::FLYCAM_ESCAPE != 0 {
            self.game_control_reset_to_avatar();
            return;
        }
        let key = |pos: bool, neg: bool| f32::from(u8::from(pos)) - f32::from(u8::from(neg));
        let ch = &mut inputs.channels;
        ch[flycam::DOLLY] += key(keys.forward, keys.back);
        ch[flycam::TRUCK] += key(keys.strafe_left, keys.strafe_right);
        ch[flycam::BOOM] += key(keys.up, keys.down);
        ch[flycam::PAN] += key(keys.turn_left, keys.turn_right);
        for v in ch.iter_mut() {
            *v = v.clamp(-1.0, 1.0);
        }
        let orbit = (inputs.modifiers & actions::FLYCAM_ORBIT != 0).then(|| self.game_control_orbit_distance());
        let input = flycam::Input {
            channels: inputs.channels,
            unroll: inputs.misc & actions::FLYCAM_UNROLL != 0,
            orbit,
            speed_factor: s.speed_factor(Mode::Flycam),
            allow_roll: s.flycam_allow_roll,
        };
        if let Some(cam) = self.camera.flycam.as_mut() {
            self.game_control.flycam_state.step(cam, input, dt);
        }
        self.input = MoveInput::default();
        self.fb_since = None;
        self.lr_since = None;
        self.up_since = None;
        self.turn_since = None;
    }

    /// The orbit's focal distance: the first surface ahead, 20 m when none.
    fn game_control_orbit_distance(&self) -> f32 {
        if self.game_control.flycam_state.orbiting() {
            return flycam::ORBIT_DEFAULT_DISTANCE;
        }
        let Some(cam) = self.camera.flycam.as_ref() else {
            return flycam::ORBIT_DEFAULT_DISTANCE;
        };
        let ray = (cam.position, cam.forward());
        self.scene
            .interaction_point(&self.world, Some(ray), None, false, flycam::ORBIT_RAY_LENGTH)
            .map(|p| (p - cam.position).length())
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(flycam::ORBIT_DEFAULT_DISTANCE)
    }

    /// updateFlycamCursorVisibility: hide the cursor while the flycam flies,
    /// show it when the mouse moves, hide it again 5 s later, or 1 s once the
    /// flycam moves.
    pub(super) fn game_control_cursor_visibility(&mut self) {
        let now = Instant::now();
        let wanted = self.game_control.engine.mode == Mode::Flycam
            && self.settings.game_control.flycam_hide_cursor
            && !self.camera.mouselook()
            && self.camera.flycam.is_some();
        let view = self.camera.flycam.as_ref().map(|c| (c.position, c.rotation, c.fov));
        let cursor = self.cursor_pos;
        let h = &mut self.game_control.hide;
        let mut hidden = h.hidden;
        if wanted && !h.wanted {
            hidden = true;
            h.deadline = None;
        } else if wanted {
            if cursor != h.last_cursor {
                hidden = false;
                h.deadline = Some(now + CURSOR_SHOWN_AFTER_MOVE);
            } else if !hidden && view != h.last_view {
                let soon = now + CURSOR_SHOWN_AFTER_FLIGHT;
                h.deadline = Some(h.deadline.map_or(soon, |d| d.min(soon)));
            }
            if !hidden && h.deadline.is_some_and(|d| now >= d) {
                hidden = true;
                h.deadline = None;
            }
        } else {
            hidden = false;
        }
        h.wanted = wanted;
        h.last_cursor = cursor;
        h.last_view = view;
        if hidden != h.hidden {
            h.hidden = hidden;
            if let Some(g) = &self.gfx {
                g.window.set_cursor_visible(!hidden);
            }
        }
    }
}
