//! Native joystick / NDOF input. Behaviour follows LLViewerJoystick's
//! moveAvatar, moveFlycam and scanJoystick (indra/newview/llviewerjoystick.cpp,
//! originally LGPL 2.1). Device I/O lives on a worker; the frame only copies
//! its latest state. Axis numbers are native, including unmapped joysticks.

mod backend;
pub mod flycam;

pub use backend::{Backend, Use};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const AXES: usize = 8;
pub const CHANNELS: usize = 7;
pub const BUTTONS: usize = 32;

/// Forward, left, up, roll, pitch, yaw, zoom, as in LLViewerJoystick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    pub gain: [f32; CHANNELS],
    pub dead_zone: [f32; CHANNELS],
    pub feather: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            gain: [2.1, 2.0, 2.0, 0.0, 0.1, 0.15, 0.0],
            dead_zone: [0.01; CHANNELS],
            feather: 5.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Empty chooses the first device. An explicit choice never falls back
    /// to another controller when unplugged.
    pub device: String,
    pub avatar_enabled: bool,
    pub flycam_enabled: bool,
    /// -1 disconnects a logical axis.
    pub mapping: [i8; CHANNELS],
    pub avatar: Tuning,
    pub flycam: Tuning,
    pub auto_level: bool,
    pub direct_zoom: bool,
    pub mouselook_yaw: bool,
    pub run_threshold: f32,
    /// Zero-based button numbers; -1 disables a button.
    pub flycam_button: i8,
    pub jump_button: i8,
    pub rise_button: i8,
    pub fall_button: i8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            device: String::new(),
            avatar_enabled: true,
            flycam_enabled: true,
            mapping: [1, 0, 2, 4, 3, 5, -1],
            avatar: Tuning {
                gain: [1.0, 1.0, 1.0, 0.0, 0.1, 0.1, 0.0],
                dead_zone: [0.1, 0.1, 0.1, 1.0, 0.02, 0.01, 1.0],
                feather: 6.0,
            },
            flycam: Tuning::default(),
            auto_level: true,
            direct_zoom: false,
            mouselook_yaw: false,
            run_threshold: 0.7,
            flycam_button: 0,
            jump_button: 1,
            rise_button: -1,
            fall_button: -1,
        }
    }
}

impl Settings {
    pub fn gamepad(&mut self) {
        // Xbox One / XInput layout (gilrs / WGI on Windows): left stick
        // XY, right stick X then triggers then right stick Y. Left bumper
        // rises, right bumper falls. Live input still overrides any axis.
        self.mapping = [1, 0, -1, -1, 3, 2, -1];
        if cfg!(windows) {
            self.mapping[4] = 5;
        }
        self.rise_button = 4;
        self.fall_button = 5;
        self.jump_button = 0;
        self.flycam_button = 7;
        self.avatar = Tuning {
            gain: [1.0, 1.0, 0.0, 0.0, 1.5, 1.5, 0.0],
            dead_zone: [0.18, 0.18, 0.15, 1.0, 0.18, 0.18, 1.0],
            feather: 6.0,
        };
        // Cinematic flycam (#1469): vertical and roll enabled, softer
        // feather so pans ease in and coast to a stop.
        self.flycam = Tuning {
            gain: [1.6, 1.5, 1.8, 0.7, 1.2, 1.2, 0.9],
            dead_zone: [0.18, 0.18, 0.12, 0.15, 0.18, 0.18, 0.12],
            feather: 2.5,
        };
        self.auto_level = false;
        // Zoom from the right trigger when WGI exposes it before right Y.
        if cfg!(windows) {
            self.mapping[6] = 4;
            self.flycam.gain[6] = 0.9;
        }
    }

    /// Xbox One naming (and XInput stand-ins) get the gamepad preset, with
    /// Linux xpad's common RightY-before-triggers layout kept when not on
    /// Windows.
    pub fn apply_xbox_one(&mut self) {
        self.gamepad();
        if !cfg!(windows) {
            // Common Linux Xbox layout: LX LY RX RY LT RT.
            self.mapping = [1, 0, -1, -1, 3, 2, 5];
            self.flycam.gain[6] = 0.9;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub axes: [f32; AXES],
    pub axis_count: usize,
    pub buttons: u32,
    pub presses: [u64; BUTTONS],
}

impl Device {
    pub fn new(id: String, name: String, axis_count: usize) -> Self {
        Self {
            id,
            name,
            axis_count: axis_count.min(AXES),
            axes: [0.0; AXES],
            buttons: 0,
            presses: [0; BUTTONS],
        }
    }

    pub fn button(&mut self, index: usize, pressed: bool) {
        if index >= BUTTONS {
            return;
        }
        let mask = 1u32 << index;
        if pressed && self.buttons & mask == 0 {
            self.presses[index] = self.presses[index].wrapping_add(1);
        }
        if pressed {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub updated: Option<Instant>,
    pub error: String,
}

impl Snapshot {
    pub fn selected(&self, s: &Settings) -> Option<&Device> {
        if s.device.is_empty() {
            self.devices.first()
        } else {
            self.devices.iter().find(|d| d.id == s.device)
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Motion {
    pub axes: [f32; CHANNELS],
    pub toggle_flycam: bool,
    pub jump: bool,
    pub jump_pressed: bool,
    pub rise: bool,
    pub fall: bool,
}

impl Motion {
    pub fn apply_avatar(&self, i: &mut crate::agent::MoveInput) {
        let a = self.axes;
        i.forward |= a[0] > 0.0;
        i.back |= a[0] < 0.0;
        i.strafe_left |= a[1] > 0.0;
        i.strafe_right |= a[1] < 0.0;
        i.up |= a[2] > 0.0 || self.rise;
        i.down |= a[2] < 0.0 || self.fall;
        i.joystick_yaw = a[5];
        i.joystick_pitch = a[4];
    }

    pub fn moving(&self) -> bool {
        self.axes.iter().any(|v| v.abs() > 0.0001) || self.jump || self.rise || self.fall
    }
}

#[derive(Default)]
pub struct Controller {
    source: String,
    settings: Option<Settings>,
    flycam: bool,
    armed: bool,
    filtered: [f32; CHANNELS],
    presses: [u64; BUTTONS],
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// LLViewerJoystick removes the dead zone without rescaling what remains.
fn mapped(d: &Device, s: &Settings, tune: &Tuning) -> [f32; CHANNELS] {
    std::array::from_fn(|i| {
        let raw = usize::try_from(s.mapping[i])
            .ok()
            .filter(|n| *n < d.axis_count)
            .map_or(0.0, |n| d.axes[n]);
        let raw = finite(raw, 0.0).clamp(-1.0, 1.0);
        let zone = finite(tune.dead_zone[i], 1.0).clamp(0.0, 1.0);
        let gain = finite(tune.gain[i], 0.0).clamp(-20.0, 20.0);
        let sign = [-1.0, -1.0, 1.0, 1.0, -1.0, -1.0, -1.0][i];
        (raw.abs() - zone).max(0.0) * raw.signum() * gain * sign
    })
}

impl Controller {
    pub fn reset(&mut self) {
        self.armed = false;
        self.filtered = [0.0; CHANNELS];
    }

    /// Re-centre after a focus change, device change, tuning change or mode
    /// switch. Never replay a held button or a stale worker sample.
    pub fn sample(&mut self, snapshot: &Snapshot, s: &Settings, active: bool, flycam: bool, dt: f32) -> Motion {
        let Some(d) = snapshot
            .selected(s)
            .filter(|_| s.enabled && active && snapshot.updated.is_some_and(|at| at.elapsed().as_millis() < 250))
        else {
            self.reset();
            return Motion::default();
        };
        if self.source != d.id || self.settings.as_ref() != Some(s) || self.flycam != flycam {
            self.source.clone_from(&d.id);
            self.settings = Some(s.clone());
            self.flycam = flycam;
            self.reset();
        }
        let tune = if flycam { &s.flycam } else { &s.avatar };
        let target = mapped(d, s, tune);
        if !self.armed {
            self.presses = d.presses;
            self.armed = d.buttons == 0 && target.iter().all(|v| v.abs() < 0.0001);
            return Motion::default();
        }
        let pressed = |button: i8| {
            usize::try_from(button)
                .ok()
                .filter(|n| *n < BUTTONS)
                .is_some_and(|n| d.presses[n] != self.presses[n])
        };
        let toggle_flycam = s.flycam_enabled && pressed(s.flycam_button);
        let jump_pressed = !flycam && s.avatar_enabled && pressed(s.jump_button);
        let held = |button: i8| {
            usize::try_from(button)
                .ok()
                .filter(|n| *n < BUTTONS)
                .is_some_and(|n| d.buttons & (1 << n) != 0)
        };
        let jump = !flycam
            && s.avatar_enabled
            && usize::try_from(s.jump_button)
                .ok()
                .filter(|n| *n < BUTTONS)
                .is_some_and(|n| d.buttons & (1 << n) != 0);
        self.presses = d.presses;
        if (flycam && !s.flycam_enabled) || (!flycam && !s.avatar_enabled) {
            self.filtered = [0.0; CHANNELS];
            return Motion {
                toggle_flycam,
                ..Default::default()
            };
        }
        let t = finite(dt, 0.0).clamp(0.0, 0.2);
        let feather = finite(tune.feather, 5.0).clamp(0.0, 32.0);
        for (i, (v, goal)) in self.filtered.iter_mut().zip(target).enumerate() {
            if !flycam && i < 3 {
                *v = goal;
            } else if feather == 0.0 {
                *v = goal;
            } else {
                // Cinematic ease (#1469): slower takeoff, firmer stop so
                // pans accelerate gently and do not halt abruptly.
                let accelerating = goal.abs() > v.abs() + 1e-4;
                let rate = feather * if accelerating { 0.55 } else { 1.35 };
                *v += (goal - *v) * (t * rate).min(1.0);
            }
            if goal == 0.0 && v.abs() < 0.0001 {
                *v = 0.0;
            }
        }
        Motion {
            axes: self.filtered,
            toggle_flycam,
            jump,
            jump_pressed,
            rise: held(s.rise_button),
            fall: held(s.fall_button),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Snapshot, Settings, Controller) {
        let snapshot = Snapshot {
            devices: vec![Device::new("test".into(), "Test joystick".into(), AXES)],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let settings = Settings {
            enabled: true,
            ..Default::default()
        };
        (snapshot, settings, Controller::default())
    }

    #[test]
    fn joystick_mapping_dead_zones_inversion_and_bad_settings() {
        let (mut snap, mut s, _) = fixture();
        snap.devices[0].axes = [0.05, -0.8, 0.3, 0.5, 0.8, -0.5, 0.0, 0.0];
        let a = mapped(&snap.devices[0], &s, &s.avatar);
        assert!((a[0] - 0.7).abs() < 1e-5);
        assert_eq!(a[1], 0.0);
        s.mapping[0] = 127;
        s.mapping[1] = -1;
        s.avatar.gain[2] = f32::NAN;
        s.avatar.dead_zone[4] = f32::INFINITY;
        assert!(mapped(&snap.devices[0], &s, &s.avatar).iter().all(|v| v.is_finite()));
        assert_eq!(mapped(&snap.devices[0], &s, &s.avatar)[0..3], [0.0; 3]);
        s.avatar.gain[5] = -0.1;
        assert!(mapped(&snap.devices[0], &s, &s.avatar)[5] < 0.0);
    }

    #[test]
    fn joystick_routes_avatar_controls_and_continuous_rotation() {
        let m = Motion {
            axes: [0.4, -0.5, 0.0, 0.0, 0.3, -0.6, 0.0],
            rise: true,
            ..Default::default()
        };
        let mut i = crate::agent::MoveInput::default();
        m.apply_avatar(&mut i);
        let mut agent = crate::agent::AgentState::default();
        let before = agent.position;
        agent.update(&i, false, 0.1, 3.0);
        assert!((agent.yaw + 0.06).abs() < 1e-5);
        assert!((agent.pitch - 0.03).abs() < 1e-5);
        assert_eq!(agent.position, before, "online movement stays simulator-driven");
        let f = agent.control_flags(&i, false);
        use aurora_net::control;
        assert_eq!(
            f & (control::AT_POS | control::LEFT_NEG | control::UP_POS | control::YAW_NEG | control::PITCH_POS),
            control::AT_POS | control::LEFT_NEG | control::UP_POS | control::YAW_NEG | control::PITCH_POS
        );
        assert_eq!(f & (control::AT_NEG | control::LEFT_POS | control::YAW_POS | control::PITCH_NEG), 0);
        assert_eq!(agent.control_flags(&i, true) & (control::YAW_NEG | control::YAW_POS), 0);
        i.joystick_mouselook_yaw = true;
        assert_ne!(agent.control_flags(&i, true) & control::YAW_NEG, 0);
    }

    #[test]
    fn joystick_settings_round_trip_and_mode_changes_require_neutral() {
        let (mut snap, mut s, mut c) = fixture();
        s.gamepad();
        s.rise_button = 4;
        let restored: Settings = serde_json::from_str(&serde_json::to_string(&s).expect("settings JSON")).expect("settings");
        assert_eq!(restored, s);
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].button(4, true);
        assert!(c.sample(&snap, &s, true, false, 0.1).rise);
        assert!(!c.sample(&snap, &s, true, true, 0.1).moving());
        snap.devices[0].button(4, false);
        c.sample(&snap, &s, true, true, 0.1);
        snap.devices[0].button(4, true);
        assert!(c.sample(&snap, &s, true, true, 0.1).rise);
        s.avatar_enabled = false;
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].button(4, false);
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].button(s.flycam_button as usize, true);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(m.toggle_flycam);
        assert!(!m.moving());
    }

    #[test]
    fn xbox_gamepad_preset_enables_cinematic_flycam_axes() {
        let mut s = Settings::default();
        s.apply_xbox_one();
        assert!(s.flycam.gain[2] > 0.0, "vertical");
        assert!(s.flycam.gain[3] > 0.0, "roll");
        assert!(s.flycam.gain[6] > 0.0, "zoom");
        assert!(s.flycam.feather < 5.0, "softer cinematic feather");
        assert!(!s.auto_level, "roll stays until the operator levels");
        assert_eq!(s.rise_button, 4);
        assert_eq!(s.fall_button, 5);
    }

    #[test]
    fn joystick_focus_disconnect_reconnect_and_button_edges() {
        let (mut snap, s, mut c) = fixture();
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].axes[1] = -1.0;
        assert!(c.sample(&snap, &s, true, false, 0.1).axes[0] > 0.0);
        assert!(!c.sample(&snap, &s, false, false, 0.1).moving());
        assert!(!c.sample(&snap, &s, true, false, 0.1).moving());
        snap.devices[0].axes[1] = 0.0;
        c.sample(&snap, &s, true, false, 0.1);
        // A short press and release between frames must still be seen, once.
        snap.devices[0].button(0, true);
        snap.devices[0].button(0, false);
        assert!(c.sample(&snap, &s, true, false, 0.1).toggle_flycam);
        assert!(!c.sample(&snap, &s, true, false, 0.1).toggle_flycam);
        let chosen = Settings {
            device: "unplugged".into(),
            ..s.clone()
        };
        assert!(!c.sample(&snap, &chosen, true, false, 0.1).moving());
        snap.updated = Some(Instant::now() - std::time::Duration::from_secs(1));
        assert!(!c.sample(&snap, &s, true, false, 0.1).moving());
    }
}
