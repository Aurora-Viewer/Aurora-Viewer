//! Native joystick / NDOF input. Behaviour follows LLViewerJoystick's
//! moveAvatar, moveFlycam and scanJoystick (indra/newview/llviewerjoystick.cpp,
//! originally LGPL 2.1). Device I/O lives on a worker; the frame only copies
//! its latest state. Axis numbers are native, including unmapped joysticks.

mod backend;
mod bindings;
pub mod capture;
pub mod flycam;
mod zoom;

pub use backend::{Backend, Use};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const AXES: usize = 8;
pub const CHANNELS: usize = 7;
pub const BUTTONS: usize = 32;
pub const INHERIT_BUTTON: i8 = -2;
pub const AUTO_LEFT_TRIGGER: i8 = -3;
pub const AUTO_RIGHT_TRIGGER: i8 = -4;
pub const AUTO_DPAD_UP: i8 = -5;
pub const AUTO_DPAD_DOWN: i8 = -6;
pub const AUTO_DPAD_LEFT: i8 = -7;
pub const AUTO_DPAD_RIGHT: i8 = -8;
pub const AUTO_SOUTH: i8 = -9;
pub const AUTO_EAST: i8 = -10;
pub const AUTO_LEFT_BUMPER: i8 = -11;
pub const AUTO_RIGHT_BUMPER: i8 = -12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadStyle {
    #[default]
    Auto,
    Xbox,
    Playstation,
    Generic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadStick {
    #[default]
    Left,
    Right,
    None,
    Custom,
}

/// A physical gamepad control in gilrs-core's standardized event vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    Select,
    Start,
    Mode,
    LeftStick,
    RightStick,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    Other,
}

impl GamepadButton {
    pub const STANDARD: [Self; 17] = [
        Self::South,
        Self::East,
        Self::North,
        Self::West,
        Self::LeftBumper,
        Self::RightBumper,
        Self::LeftTrigger,
        Self::RightTrigger,
        Self::Select,
        Self::Start,
        Self::Mode,
        Self::LeftStick,
        Self::RightStick,
        Self::DpadUp,
        Self::DpadDown,
        Self::DpadLeft,
        Self::DpadRight,
    ];
}

/// Zoom bindings also accept either direction of a native axis, encoded
/// after physical buttons. Existing button settings remain 0..BUTTONS.
pub fn button_axis(binding: i8) -> Option<(usize, bool)> {
    usize::try_from(binding)
        .ok()?
        .checked_sub(BUTTONS)
        .filter(|n| *n < AXES * 2)
        .map(|n| (n / 2, n % 2 == 0))
}

pub fn gamepad_trigger_binding(device: &Device, side: usize) -> Option<i8> {
    let button = match side {
        0 => GamepadButton::LeftTrigger,
        1 => GamepadButton::RightTrigger,
        _ => return None,
    };
    device
        .gamepad_triggers
        .get(side)
        .copied()
        .flatten()
        .filter(|axis| *axis < AXES)
        .map(|axis| (BUTTONS + axis * 2) as i8)
        .or_else(|| {
            device
                .gamepad_buttons
                .iter()
                .take(BUTTONS)
                .position(|candidate| *candidate == button)
                .map(|index| index as i8)
        })
}

pub fn gamepad_dpad_binding(device: &Device, direction: usize) -> Option<i8> {
    let (button, component, positive) = match direction {
        0 => (GamepadButton::DpadUp, 1, false),
        1 => (GamepadButton::DpadDown, 1, true),
        2 => (GamepadButton::DpadLeft, 0, false),
        3 => (GamepadButton::DpadRight, 0, true),
        _ => return None,
    };
    device
        .gamepad_buttons
        .iter()
        .take(BUTTONS)
        .position(|candidate| *candidate == button)
        .map(|index| index as i8)
        .or_else(|| {
            device.gamepad_dpad[component]
                .filter(|axis| *axis < AXES)
                .map(|axis| (BUTTONS + axis * 2 + usize::from(!positive)) as i8)
        })
}

/// Defaults identify physical controls, never the platform's native button order.
pub fn automatic_button(binding: i8) -> Option<GamepadButton> {
    match binding {
        AUTO_SOUTH => Some(GamepadButton::South),
        AUTO_EAST => Some(GamepadButton::East),
        AUTO_LEFT_BUMPER => Some(GamepadButton::LeftBumper),
        AUTO_RIGHT_BUMPER => Some(GamepadButton::RightBumper),
        _ => None,
    }
}

pub fn resolve_binding(device: &Device, binding: i8) -> i8 {
    if let Some(button) = automatic_button(binding) {
        return if device.is_gamepad {
            device
                .gamepad_buttons
                .iter()
                .take(BUTTONS)
                .position(|candidate| *candidate == button)
                .map_or(-1, |index| index as i8)
        } else {
            // Keep the established raw joystick button defaults.
            GamepadButton::STANDARD
                .iter()
                .position(|candidate| *candidate == button)
                .map_or(-1, |index| index as i8)
        };
    }
    match binding {
        AUTO_LEFT_TRIGGER => gamepad_trigger_binding(device, 0).unwrap_or(-1),
        AUTO_RIGHT_TRIGGER => gamepad_trigger_binding(device, 1).unwrap_or(-1),
        AUTO_DPAD_UP => gamepad_dpad_binding(device, 0).unwrap_or(-1),
        AUTO_DPAD_DOWN => gamepad_dpad_binding(device, 1).unwrap_or(-1),
        AUTO_DPAD_LEFT => gamepad_dpad_binding(device, 2).unwrap_or(-1),
        AUTO_DPAD_RIGHT => gamepad_dpad_binding(device, 3).unwrap_or(-1),
        binding => binding,
    }
}

pub fn axis_button_mask(device: &Device) -> u16 {
    device
        .axes
        .iter()
        .take(device.axis_count.min(AXES))
        .enumerate()
        .fold(0, |mask, (axis, &value)| {
            let value = finite(value, 0.0);
            mask | if value > 0.5 {
                1 << (axis * 2)
            } else if value < -0.5 {
                1 << (axis * 2 + 1)
            } else {
                0
            }
        })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomMode {
    #[default]
    Axis,
    Buttons,
    HoldAxis,
}

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

/// Controller inputs for the viewer's existing mouselook camera. Signed gains
/// are independent of the shared avatar/flycam joystick inversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FirstPerson {
    pub tuning: Tuning,
    pub move_stick: GamepadStick,
    pub camera_stick: GamepadStick,
    pub jump_button: i8,
    pub rise_button: i8,
    pub fall_button: i8,
    pub zoom_in_button: i8,
    pub zoom_out_button: i8,
    pub view_button: i8,
}

impl Default for FirstPerson {
    fn default() -> Self {
        Self {
            tuning: Tuning {
                gain: [1.0, 1.0, 0.0, 0.0, 1.875, 1.875, 0.0],
                dead_zone: [0.15; CHANNELS],
                feather: 6.0,
            },
            move_stick: GamepadStick::Left,
            camera_stick: GamepadStick::Right,
            jump_button: AUTO_EAST,
            rise_button: AUTO_LEFT_BUMPER,
            fall_button: AUTO_RIGHT_BUMPER,
            zoom_in_button: AUTO_RIGHT_TRIGGER,
            zoom_out_button: AUTO_LEFT_TRIGGER,
            view_button: INHERIT_BUTTON,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Gamepads are monitored independently of the opt-in raw joystick/HID input.
    pub automatic_gamepad: bool,
    pub gamepad_style: GamepadStyle,
    pub first_person: FirstPerson,
    pub gamepad_flycam_view_button: i8,
    /// Analog sticks assigned to avatar and flycam motion separately.
    pub gamepad_move_stick: GamepadStick,
    pub gamepad_camera_stick: GamepadStick,
    pub gamepad_flycam_move_stick: GamepadStick,
    pub gamepad_flycam_camera_stick: GamepadStick,
    /// Multiplier applied to all translation through the flycam.
    pub flycam_speed: f32,
    /// Absolute travel-speed factors at full trigger pressure.
    pub gamepad_flycam_slowdown_speed: f32,
    pub gamepad_flycam_speedup_speed: f32,
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
    pub zoom_mode: ZoomMode,
    pub zoom_in_button: i8,
    pub zoom_out_button: i8,
    pub zoom_hold_button: i8,
    pub zoom_speed: f32,
    pub mouselook_yaw: bool,
    pub run_threshold: f32,
    /// Zero-based button numbers; -1 disables a button.
    pub flycam_button: i8,
    pub jump_button: i8,
    pub rise_button: i8,
    pub fall_button: i8,
    pub flycam_rise_button: i8,
    pub flycam_fall_button: i8,
    /// Semantic trigger defaults resolve to each controller's native inputs.
    pub gamepad_flycam_slowdown_button: i8,
    pub gamepad_flycam_speedup_button: i8,
    pub gamepad_avatar_zoom_in_button: i8,
    pub gamepad_avatar_zoom_out_button: i8,
    pub gamepad_avatar_pan_up_button: i8,
    pub gamepad_avatar_pan_down_button: i8,
    pub gamepad_avatar_next_button: i8,
    pub gamepad_avatar_previous_button: i8,
}

impl Default for Settings {
    fn default() -> Self {
        let mut settings = Self {
            enabled: false,
            automatic_gamepad: true,
            gamepad_style: GamepadStyle::Auto,
            first_person: FirstPerson::default(),
            gamepad_flycam_view_button: INHERIT_BUTTON,
            gamepad_move_stick: GamepadStick::Left,
            gamepad_camera_stick: GamepadStick::Right,
            gamepad_flycam_move_stick: GamepadStick::Left,
            gamepad_flycam_camera_stick: GamepadStick::Right,
            flycam_speed: 2.5,
            gamepad_flycam_slowdown_speed: 0.1,
            gamepad_flycam_speedup_speed: 20.0,
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
            zoom_mode: ZoomMode::Axis,
            zoom_in_button: -1,
            zoom_out_button: -1,
            zoom_hold_button: -1,
            zoom_speed: 1.0,
            mouselook_yaw: false,
            run_threshold: 0.7,
            flycam_button: AUTO_SOUTH,
            jump_button: AUTO_EAST,
            rise_button: -1,
            fall_button: -1,
            flycam_rise_button: INHERIT_BUTTON,
            flycam_fall_button: INHERIT_BUTTON,
            gamepad_flycam_slowdown_button: AUTO_LEFT_TRIGGER,
            gamepad_flycam_speedup_button: AUTO_RIGHT_TRIGGER,
            gamepad_avatar_zoom_in_button: AUTO_RIGHT_TRIGGER,
            gamepad_avatar_zoom_out_button: AUTO_LEFT_TRIGGER,
            gamepad_avatar_pan_up_button: AUTO_DPAD_UP,
            gamepad_avatar_pan_down_button: AUTO_DPAD_DOWN,
            gamepad_avatar_next_button: AUTO_DPAD_RIGHT,
            gamepad_avatar_previous_button: AUTO_DPAD_LEFT,
        };
        settings.gamepad();
        settings
    }
}

impl Settings {
    pub fn untouched_raw_layout(&self) -> bool {
        self.mapping == [1, 0, 2, 4, 3, 5, -1]
            && self.avatar.gain == [1.0, 1.0, 1.0, 0.0, 0.1, 0.1, 0.0]
            && self.avatar.dead_zone == [0.1, 0.1, 0.1, 1.0, 0.02, 0.01, 1.0]
            && self.avatar.feather == 6.0
            && self.flycam == Tuning::default()
    }

    pub fn spacemouse(&mut self) {
        self.gamepad_move_stick = GamepadStick::Custom;
        self.gamepad_camera_stick = GamepadStick::Custom;
        self.gamepad_flycam_move_stick = GamepadStick::Custom;
        self.gamepad_flycam_camera_stick = GamepadStick::Custom;
        self.flycam_rise_button = INHERIT_BUTTON;
        self.flycam_fall_button = INHERIT_BUTTON;
        self.mapping = [1, 0, 2, 4, 3, 5, -1];
        self.avatar = Tuning {
            gain: [1.0, 1.0, 1.0, 0.0, 0.1, 0.1, 0.0],
            dead_zone: [0.1, 0.1, 0.1, 1.0, 0.02, 0.01, 1.0],
            feather: 6.0,
        };
        self.flycam = Tuning::default();
        self.rise_button = -1;
        self.fall_button = -1;
    }

    /// Reuse signed gains in saved profiles for one common axis direction.
    /// Roll and zoom have no Avatar input, so only their Camera sign matters.
    pub fn axis_inverted(&self, channel: usize) -> bool {
        finite(self.flycam.gain[channel], 0.0).is_sign_negative()
            || (!matches!(channel, 3 | 6) && finite(self.avatar.gain[channel], 0.0).is_sign_negative())
    }

    pub fn set_axis_inverted(&mut self, channel: usize, inverted: bool) {
        let sign = if inverted { -1.0 } else { 1.0 };
        self.avatar.gain[channel] = self.avatar.gain[channel].abs().copysign(sign);
        self.flycam.gain[channel] = self.flycam.gain[channel].abs().copysign(sign);
    }

    pub fn absolute_zoom(&self) -> bool {
        self.direct_zoom
    }

    pub fn gamepad(&mut self) {
        self.gamepad_move_stick = GamepadStick::Left;
        self.gamepad_camera_stick = GamepadStick::Right;
        self.gamepad_flycam_move_stick = GamepadStick::Left;
        self.gamepad_flycam_camera_stick = GamepadStick::Right;
        self.mapping = [1, 0, -1, -1, 4, 3, -1];
        // Windows.Gaming.Input places right-stick Y after the two triggers.
        if cfg!(windows) {
            self.mapping[4] = 5;
            self.mapping[5] = 2;
        }
        self.rise_button = AUTO_RIGHT_BUMPER;
        self.fall_button = AUTO_LEFT_BUMPER;
        self.flycam_rise_button = AUTO_DPAD_UP;
        self.flycam_fall_button = AUTO_DPAD_DOWN;
        self.avatar = Tuning {
            gain: [1.0, 1.0, 0.0, 0.0, 4.0, 4.0, 0.0],
            dead_zone: [0.15; CHANNELS],
            feather: 6.0,
        };
        self.flycam = Tuning {
            gain: [2.1, 2.0, 0.0, 0.0, 4.0, 4.0, 0.0],
            dead_zone: [0.15; CHANNELS],
            feather: 5.0,
        };
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
    pub is_gamepad: bool,
    /// Native axis-array positions for the standardized left and right sticks.
    pub gamepad_sticks: [[Option<usize>; 2]; 2],
    /// Native axis-array positions for analog left and right triggers.
    pub gamepad_triggers: [Option<usize>; 2],
    /// Native hat X/Y axes where a platform exposes the D-pad as axes.
    pub gamepad_dpad: [Option<usize>; 2],
    pub gamepad_style: GamepadStyle,
    /// One standardized label per entry in `buttons` / `presses`.
    pub gamepad_buttons: Vec<GamepadButton>,
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
            is_gamepad: false,
            gamepad_sticks: [[None; 2]; 2],
            gamepad_triggers: [None; 2],
            gamepad_dpad: [None; 2],
            gamepad_style: GamepadStyle::Generic,
            gamepad_buttons: Vec::new(),
        }
    }

    pub fn gamepad(
        id: String,
        name: String,
        axis_count: usize,
        sticks: [[Option<usize>; 2]; 2],
        triggers: [Option<usize>; 2],
        style: GamepadStyle,
        buttons: Vec<GamepadButton>,
    ) -> Self {
        Self {
            is_gamepad: true,
            gamepad_sticks: sticks,
            gamepad_triggers: triggers,
            gamepad_style: style,
            gamepad_buttons: buttons,
            ..Self::new(id, name, axis_count)
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
    pub fn input_enabled(&self, settings: &Settings) -> bool {
        settings.enabled || (settings.automatic_gamepad && self.selected_gamepad(settings).is_some())
    }

    pub fn selected(&self, s: &Settings) -> Option<&Device> {
        if s.device.is_empty() {
            self.devices.iter().find(|d| d.is_gamepad).or_else(|| self.devices.first())
        } else {
            self.devices.iter().find(|d| d.id == s.device)
        }
    }

    pub fn selected_gamepad(&self, s: &Settings) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.is_gamepad && d.id == s.device)
            .or_else(|| self.devices.iter().find(|d| d.is_gamepad))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Motion {
    pub axes: [f32; CHANNELS],
    pub toggle_flycam: bool,
    pub reset_zoom: bool,
    pub jump: bool,
    pub jump_pressed: bool,
    pub rise: bool,
    pub fall: bool,
    pub flycam_speed: Option<f32>,
    pub avatar_zoom: f32,
    pub avatar_pan: f32,
    pub avatar_cycle: i8,
}

impl Motion {
    /// Intent to translate the player, excluding camera sticks and triggers.
    pub fn player_movement_input(&self) -> bool {
        self.axes[..3].iter().any(|value| value.abs() > 0.0001) || self.jump || self.rise || self.fall
    }

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
        self.axes.iter().any(|v| v.abs() > 0.0001)
            || self.jump
            || self.rise
            || self.fall
            || self.reset_zoom
            || self.avatar_zoom != 0.0
            || self.avatar_pan != 0.0
            || self.avatar_cycle != 0
    }
}

#[derive(Default)]
pub struct Controller {
    source: String,
    settings: Option<Settings>,
    flycam: bool,
    first_person: bool,
    armed: [bool; CHANNELS],
    buttons_armed: u32,
    filtered: [f32; CHANNELS],
    presses: Option<[u64; BUTTONS]>,
    zoom_adjusting: bool,
    axis_buttons_previous: Option<u16>,
    axis_buttons_armed: u16,
    zoom_pressure_armed: [bool; 2],
    flycam_speed_armed: [bool; 2],
    avatar_camera_armed: [bool; 4],
    flycam_speed_owner: Option<usize>,
    zoom_reset: zoom::ResetGesture,
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// LLViewerJoystick removes the dead zone without rescaling what remains.
#[cfg(test)]
fn mapped(d: &Device, s: &Settings, tune: &Tuning) -> [f32; CHANNELS] {
    mapped_sticks_mode(d, s, tune, s.gamepad_move_stick, s.gamepad_camera_stick, false)
}

fn mapped_sticks_mode(
    d: &Device,
    s: &Settings,
    tune: &Tuning,
    move_stick: GamepadStick,
    camera_stick: GamepadStick,
    first_person: bool,
) -> [f32; CHANNELS] {
    std::array::from_fn(|i| {
        let raw = s.axis_with_sticks(d, i, move_stick, camera_stick).map_or(0.0, |n| d.axes[n]);
        let raw = finite(raw, 0.0).clamp(-1.0, 1.0);
        let zone = finite(tune.dead_zone[i], 1.0).clamp(0.0, 1.0);
        // A negative legacy gain represents one common input inversion;
        // magnitudes and dead zones remain specific to the active mode.
        let inverted = if first_person {
            finite(tune.gain[i], 0.0).is_sign_negative()
        } else {
            s.axis_inverted(i)
        };
        let gain = finite(tune.gain[i], 0.0).abs().min(20.0) * if inverted { -1.0 } else { 1.0 };
        let sign = [-1.0, -1.0, 1.0, 1.0, -1.0, -1.0, -1.0][i];
        (raw.abs() - zone).max(0.0) * raw.signum() * gain * sign
    })
}

impl Controller {
    fn reset_motion(&mut self) {
        self.armed = [false; CHANNELS];
        self.buttons_armed = 0;
        self.filtered = [0.0; CHANNELS];
        self.zoom_adjusting = false;
        self.axis_buttons_armed = 0;
        self.zoom_pressure_armed = [false; 2];
        self.flycam_speed_armed = [false; 2];
        self.avatar_camera_armed = [false; 4];
        self.flycam_speed_owner = None;
        self.zoom_reset = Default::default();
    }

    fn reset(&mut self) {
        self.reset_motion();
        self.presses = None;
        self.axis_buttons_previous = None;
    }

    /// Re-centre after a focus change, device change, tuning change or mode
    /// switch. Never replay a held button or a stale worker sample.
    #[cfg(test)]
    pub fn sample(&mut self, snapshot: &Snapshot, s: &Settings, active: bool, flycam: bool, dt: f32) -> Motion {
        self.sample_view(snapshot, s, active, flycam, false, dt)
    }

    pub fn sample_view(&mut self, snapshot: &Snapshot, s: &Settings, active: bool, flycam: bool, first_person: bool, dt: f32) -> Motion {
        let source = if s.enabled {
            snapshot.selected(s)
        } else if s.automatic_gamepad {
            snapshot.selected_gamepad(s)
        } else {
            None
        };
        let Some(d) = source.filter(|_| active && snapshot.updated.is_some_and(|at| at.elapsed().as_millis() < 250)) else {
            self.reset();
            return Motion::default();
        };
        if self.source != d.id || self.settings.as_ref() != Some(s) {
            self.source.clone_from(&d.id);
            self.settings = Some(s.clone());
            self.reset();
        }
        let first_person = first_person && !flycam && d.is_gamepad;
        if self.flycam != flycam || self.first_person != first_person {
            self.flycam = flycam;
            self.first_person = first_person;
            // Keep button edges across a mode switch: scanJoystick checks
            // the flycam toggle independently of moveAvatar/moveFlycam.
            self.reset_motion();
        }
        let tune = if first_person {
            &s.first_person.tuning
        } else if flycam {
            &s.flycam
        } else {
            &s.avatar
        };
        let (move_stick, camera_stick) = if first_person {
            (s.first_person.move_stick, s.first_person.camera_stick)
        } else if flycam {
            (s.gamepad_flycam_move_stick, s.gamepad_flycam_camera_stick)
        } else {
            (s.gamepad_move_stick, s.gamepad_camera_stick)
        };
        let mut target = mapped_sticks_mode(d, s, tune, move_stick, camera_stick, first_person);
        if !flycam {
            // The avatar has neither roll nor lens zoom.
            target[3] = 0.0;
            target[6] = 0.0;
        }
        let mut new_buttons = 0;
        if let Some(previous) = self.presses.replace(d.presses) {
            for (n, (&now, before)) in d.presses.iter().zip(previous).enumerate() {
                if now != before {
                    new_buttons |= 1 << n;
                }
            }
        }
        let axis_buttons = axis_button_mask(d);
        let axis_edges = self
            .axis_buttons_previous
            .replace(axis_buttons)
            .map_or(0, |before| axis_buttons & !before);
        self.axis_buttons_armed |= !axis_buttons | axis_edges;
        let pressed = |button: i8| {
            if let Some((axis, positive)) = button_axis(button) {
                return axis_edges & (1 << (axis * 2 + usize::from(!positive))) != 0;
            }
            usize::try_from(button)
                .ok()
                .filter(|n| *n < BUTTONS)
                .is_some_and(|n| new_buttons & (1 << n) != 0)
        };
        let view_button = if d.is_gamepad {
            s.view_button(flycam, first_person)
        } else {
            s.flycam_button
        };
        let toggle_flycam = (d.is_gamepad || s.flycam_enabled) && pressed(resolve_binding(d, view_button));
        let jump_button = if first_person { s.first_person.jump_button } else { s.jump_button };
        let jump_pressed = !flycam && s.avatar_enabled && pressed(resolve_binding(d, jump_button));
        self.buttons_armed |= !d.buttons | new_buttons;
        let held = |button: i8| {
            if let Some((axis, positive)) = button_axis(button) {
                return axis_buttons & self.axis_buttons_armed & (1 << (axis * 2 + usize::from(!positive))) != 0;
            }
            usize::try_from(button)
                .ok()
                .filter(|n| *n < BUTTONS)
                .is_some_and(|n| d.buttons & self.buttons_armed & (1 << n) != 0)
        };
        let jump = !flycam && s.avatar_enabled && held(resolve_binding(d, jump_button));
        let axis_bit = |binding| button_axis(binding).map(|(axis, positive)| 1u16 << (axis * 2 + usize::from(!positive)));
        let zoom_held = |binding| axis_bit(binding).map_or_else(|| held(binding), |bit| axis_buttons & self.axis_buttons_armed & bit != 0);
        let zoom_pressed = |binding| axis_bit(binding).map_or_else(|| pressed(binding), |bit| axis_edges & bit != 0);
        let pressure = |binding| {
            button_axis(binding).map_or_else(
                || f32::from(u8::from(held(binding))),
                |(axis, positive)| {
                    if axis < d.axis_count {
                        (finite(d.axes[axis], 0.0) * if positive { 1.0 } else { -1.0 }).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                },
            )
        };
        let mut avatar_camera_pressure = if !flycam && d.is_gamepad {
            (if first_person {
                [s.first_person.zoom_in_button, s.first_person.zoom_out_button, -1, -1]
            } else {
                [
                    s.gamepad_avatar_zoom_in_button,
                    s.gamepad_avatar_zoom_out_button,
                    s.gamepad_avatar_pan_up_button,
                    s.gamepad_avatar_pan_down_button,
                ]
            })
            .map(|binding| pressure(resolve_binding(d, binding)))
        } else {
            [0.0; 4]
        };
        for (armed, value) in self.avatar_camera_armed.iter_mut().zip(&mut avatar_camera_pressure) {
            *armed |= *value <= 0.005;
            if !*armed {
                *value = 0.0;
            }
        }
        let mut flycam_speed_pressure = if flycam && d.is_gamepad {
            [
                pressure(resolve_binding(d, s.gamepad_flycam_slowdown_button)),
                pressure(resolve_binding(d, s.gamepad_flycam_speedup_button)),
            ]
        } else {
            [0.0; 2]
        };
        for (armed, value) in self.flycam_speed_armed.iter_mut().zip(&mut flycam_speed_pressure) {
            *armed |= *value <= 0.005;
            if !*armed {
                *value = 0.0;
            }
        }
        let mut flycam_speed = 1.0;
        if flycam && d.is_gamepad {
            let default_speed = finite(s.flycam_speed, 2.5).clamp(0.1, 5.0);
            if self.flycam_speed_owner.is_some_and(|owner| flycam_speed_pressure[owner] <= 0.005) {
                self.flycam_speed_owner = None;
            }
            if self.flycam_speed_owner.is_none() {
                self.flycam_speed_owner = if flycam_speed_pressure[0] > 0.005 {
                    Some(0)
                } else if flycam_speed_pressure[1] > 0.005 {
                    Some(1)
                } else {
                    None
                };
            }
            flycam_speed = match self.flycam_speed_owner {
                Some(0) => {
                    let minimum = finite(s.gamepad_flycam_slowdown_speed, 0.1).clamp(0.05, default_speed);
                    (default_speed + (minimum - default_speed) * flycam_speed_pressure[0]) / default_speed
                }
                Some(1) => {
                    let maximum = finite(s.gamepad_flycam_speedup_speed, 20.0).clamp(default_speed, 50.0);
                    (default_speed + (maximum - default_speed) * flycam_speed_pressure[1]) / default_speed
                }
                _ => 1.0,
            };
        } else {
            self.flycam_speed_owner = None;
        }
        let raw_pressure = [pressure(s.zoom_in_button), pressure(s.zoom_out_button)];
        let mut zoom_pressure = raw_pressure;
        for (armed, value) in self.zoom_pressure_armed.iter_mut().zip(&mut zoom_pressure) {
            *armed |= *value <= 0.1;
            if !*armed {
                *value = 0.0;
            }
        }
        let adjusting = flycam && s.zoom_mode == ZoomMode::HoldAxis && zoom_held(s.zoom_hold_button);
        if adjusting != self.zoom_adjusting {
            self.zoom_adjusting = adjusting;
            self.filtered[6] = 0.0;
            self.armed[6] = adjusting;
            // A shared stick belongs to only one camera operation. On
            // release, wait for neutral rather than replay its deflection.
            for (i, &axis) in s.mapping[..6].iter().enumerate() {
                if s.mapping[6] >= 0 && axis == s.mapping[6] {
                    self.filtered[i] = 0.0;
                    self.armed[i] = false;
                }
            }
        }
        if flycam {
            match s.zoom_mode {
                ZoomMode::Axis => {}
                ZoomMode::Buttons => {
                    let direction = if s.absolute_zoom() {
                        zoom_pressure[1] - zoom_pressure[0]
                    } else {
                        f32::from(u8::from(zoom_held(s.zoom_out_button) || zoom_pressed(s.zoom_out_button)))
                            - f32::from(u8::from(zoom_held(s.zoom_in_button) || zoom_pressed(s.zoom_in_button)))
                    };
                    target[6] = direction * finite(s.zoom_speed, 1.0).clamp(0.0, 20.0);
                }
                ZoomMode::HoldAxis => {
                    if adjusting {
                        for (i, goal) in target[..6].iter_mut().enumerate() {
                            if s.mapping[6] >= 0 && s.mapping[i] == s.mapping[6] {
                                *goal = 0.0;
                                self.filtered[i] = 0.0;
                            }
                        }
                    } else {
                        target[6] = 0.0;
                    }
                }
            }
            // A trigger assigned as a zoom button or modifier must not
            // also rotate/translate the camera through another mapping.
            let sources: &[i8] = match s.zoom_mode {
                ZoomMode::Axis => &[],
                ZoomMode::Buttons => &[s.zoom_in_button, s.zoom_out_button],
                ZoomMode::HoldAxis => &[s.zoom_hold_button],
            };
            for (i, goal) in target[..6].iter_mut().enumerate() {
                if sources
                    .iter()
                    .any(|&binding| button_axis(binding).is_some_and(|(axis, _)| s.mapping[i] == axis as i8))
                {
                    *goal = 0.0;
                    self.filtered[i] = 0.0;
                }
            }
            if s.zoom_mode != ZoomMode::Axis && target[6] == 0.0 {
                self.filtered[6] = 0.0;
            }
        }
        let mut reset_zoom = false;
        if flycam && s.flycam_enabled && !s.absolute_zoom() {
            let (value, press, chord) = match s.zoom_mode {
                ZoomMode::Axis => (
                    usize::try_from(s.mapping[6])
                        .ok()
                        .filter(|&axis| axis < d.axis_count.min(AXES))
                        .map_or(0.0, |axis| finite(d.axes[axis], 0.0).abs()),
                    false,
                    false,
                ),
                ZoomMode::Buttons => (
                    raw_pressure[0].max(raw_pressure[1]),
                    false,
                    s.zoom_in_button != s.zoom_out_button && zoom_held(s.zoom_in_button) && zoom_held(s.zoom_out_button),
                ),
                ZoomMode::HoldAxis => (
                    pressure(s.zoom_hold_button),
                    button_axis(s.zoom_hold_button).is_none() && zoom_pressed(s.zoom_hold_button),
                    false,
                ),
            };
            let (reset, suppress) = self
                .zoom_reset
                .sample(value, press, chord, s.zoom_mode != ZoomMode::Buttons, Instant::now());
            reset_zoom = reset;
            if suppress {
                target[6] = 0.0;
                self.filtered[6] = 0.0;
            }
        }
        // Arm relative channels independently; button capture and the
        // modifier already enforce release after focus/settings changes.
        for (i, goal) in target.iter_mut().enumerate() {
            self.armed[i] |= goal.abs() < 0.0001 || (flycam && i == 6 && s.absolute_zoom());
            if !self.armed[i] {
                *goal = 0.0;
            }
        }
        if (flycam && !s.flycam_enabled) || (!flycam && !s.avatar_enabled) {
            self.filtered = [0.0; CHANNELS];
            return Motion {
                toggle_flycam,
                ..Default::default()
            };
        }
        let t = finite(dt, 0.0).clamp(0.0, 0.2);
        let feather = finite(tune.feather, 5.0).clamp(0.0, 32.0);
        let blend = if feather == 0.0 { 1.0 } else { (t * feather).min(1.0) };
        for (i, (v, goal)) in self.filtered.iter_mut().zip(target).enumerate() {
            if (!flycam && i < 3) || (flycam && i == 6 && s.zoom_mode == ZoomMode::Buttons) {
                *v = goal;
            } else {
                *v += (goal - *v) * blend;
            }
            if goal == 0.0 && v.abs() < 0.0001 {
                *v = 0.0;
            }
        }
        Motion {
            axes: self.filtered,
            toggle_flycam,
            reset_zoom,
            jump,
            jump_pressed,
            rise: held(resolve_binding(
                d,
                if first_person {
                    s.first_person.rise_button
                } else if d.is_gamepad && flycam && s.flycam_rise_button != INHERIT_BUTTON {
                    s.flycam_rise_button
                } else {
                    s.rise_button
                },
            )),
            fall: held(resolve_binding(
                d,
                if first_person {
                    s.first_person.fall_button
                } else if d.is_gamepad && flycam && s.flycam_fall_button != INHERIT_BUTTON {
                    s.flycam_fall_button
                } else {
                    s.fall_button
                },
            )),
            flycam_speed: (flycam && d.is_gamepad).then_some(flycam_speed),
            avatar_zoom: avatar_camera_pressure[0] - avatar_camera_pressure[1],
            avatar_pan: avatar_camera_pressure[2] - avatar_camera_pressure[3],
            avatar_cycle: if !flycam && !first_person && d.is_gamepad {
                i8::from(pressed(resolve_binding(d, s.gamepad_avatar_next_button)))
                    - i8::from(pressed(resolve_binding(d, s.gamepad_avatar_previous_button)))
            } else {
                0
            },
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
        let mut settings = Settings {
            enabled: true,
            ..Default::default()
        };
        settings.spacemouse();
        (snapshot, settings, Controller::default())
    }

    #[test]
    fn joystick_absolute_zoom_uses_control_position_in_all_modes_and_trigger_pressure() {
        for mode in [ZoomMode::Axis, ZoomMode::Buttons, ZoomMode::HoldAxis] {
            for trigger in [false, true] {
                let (mut snap, mut s, mut controller) = fixture();
                s.zoom_mode = mode;
                s.direct_zoom = true;
                s.mapping[6] = 6;
                s.flycam.gain[6] = 0.4;
                s.flycam.dead_zone[6] = 0.0;
                s.flycam.feather = 0.0;
                s.zoom_speed = 0.4;
                s.zoom_in_button = if trigger { 44 } else { 8 };
                s.zoom_out_button = if trigger { 46 } else { 9 };
                s.zoom_hold_button = if trigger { 46 } else { 8 };
                let mut camera = flycam::Flycam::from_view(glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z, 1.0);
                controller.sample(&snap, &s, true, true, 0.1);
                snap.devices[0].axes[6] = 0.25;
                snap.devices[0].axes[7] = if mode == ZoomMode::HoldAxis { 0.9 } else { 0.0 };
                snap.devices[0].button(8, true);
                let expected = if mode == ZoomMode::Buttons && !trigger { 0.6 } else { 0.9 };
                for dt in [0.01, 0.2, 0.05] {
                    let motion = controller.sample(&snap, &s, true, true, dt);
                    assert!(!motion.reset_zoom);
                    camera.step(motion.axes, &s, dt);
                    assert!(
                        (camera.fov - expected).abs() < 1e-5,
                        "absolute zoom is position-based, not integrated"
                    );
                }
                if mode == ZoomMode::Buttons && trigger {
                    // Small pressure must be useful, but never replayed after focus loss.
                    controller.sample(&snap, &s, false, true, 0.1);
                    assert_eq!(controller.sample(&snap, &s, true, true, 0.1).axes[6], 0.0);
                }
                snap.devices[0].axes = [0.0; AXES];
                snap.devices[0].button(8, false);
                let motion = controller.sample(&snap, &s, true, true, 0.1);
                camera.step(motion.axes, &s, 0.1);
                assert!(
                    (camera.fov - 1.0).abs() < 1e-5,
                    "rest returns to the flight reference in every absolute mode"
                );
                for stage in 0..3 {
                    let active = stage != 1;
                    snap.devices[0].axes[6] = if active { 0.8 } else { 0.0 };
                    snap.devices[0].axes[7] = if (mode == ZoomMode::HoldAxis && active) || (mode == ZoomMode::Buttons && stage == 2) {
                        0.9
                    } else {
                        0.0
                    };
                    snap.devices[0].button(8, active);
                    snap.devices[0].button(9, mode == ZoomMode::Buttons && stage == 2);
                    assert!(
                        !controller.sample(&snap, &s, true, true, 0.1).reset_zoom,
                        "absolute mode ignores relative reset gestures"
                    );
                }
            }
        }
    }

    #[test]
    fn joystick_relative_zoom_reset_gestures_restore_only_the_lens_and_wait_for_release() {
        for mode in [ZoomMode::Axis, ZoomMode::Buttons, ZoomMode::HoldAxis] {
            for trigger in [false, true] {
                let (mut snap, mut s, mut controller) = fixture();
                s.zoom_mode = mode;
                s.auto_level = false;
                s.mapping[6] = 6;
                s.flycam.gain[6] = 0.7;
                s.flycam.feather = 8.0;
                s.zoom_in_button = if trigger { 44 } else { 8 };
                s.zoom_out_button = if trigger { 46 } else { 9 };
                s.zoom_hold_button = if trigger { 46 } else { 8 };
                let mut camera = flycam::Flycam::from_view(glam::Vec3::new(3.0, 4.0, 5.0), glam::Vec3::X, glam::Vec3::Z, 1.0);
                camera.step([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8], &s, 0.2);
                assert!(camera.fov > 1.1);
                let pose = (camera.position, camera.rotation);
                controller.sample(&snap, &s, true, true, 0.1);
                let mut reset = Motion::default();
                for stage in 0..3 {
                    let value = if stage == 1 { 0.0 } else { 0.8 };
                    match mode {
                        ZoomMode::Axis => snap.devices[0].axes[6] = value,
                        ZoomMode::HoldAxis if trigger => snap.devices[0].axes[7] = value,
                        ZoomMode::HoldAxis => snap.devices[0].button(8, value > 0.5),
                        ZoomMode::Buttons => {
                            if trigger {
                                snap.devices[0].axes[6] = 0.8;
                                snap.devices[0].axes[7] = if stage == 2 { 0.8 } else { 0.0 };
                            } else {
                                snap.devices[0].button(8, true);
                                snap.devices[0].button(9, stage == 2);
                            }
                        }
                    }
                    reset = controller.sample(&snap, &s, true, true, 0.1);
                    assert_eq!(reset.reset_zoom, stage == 2);
                }
                assert!(reset.moving(), "a lens reset counts as user activity");
                assert_eq!(reset.axes[6], 0.0, "reset clears filtered zoom immediately");
                camera.set_fov(0.9);
                camera.step([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8], &s, 0.2);
                camera.reset_zoom();
                camera.step(reset.axes, &s, 0.1);
                assert!((camera.fov - 0.9).abs() < 1e-5, "reset follows the edited flight reference");
                assert!(
                    camera.position.abs_diff_eq(pose.0, 1e-6) && camera.rotation.abs_diff_eq(pose.1, 1e-6),
                    "lens reset preserves the flight pose"
                );
                let held = controller.sample(&snap, &s, true, true, 0.1);
                assert!(!held.reset_zoom && held.axes[6] == 0.0);
                snap.devices[0].axes = [0.0; AXES];
                snap.devices[0].button(8, false);
                snap.devices[0].button(9, false);
                assert_eq!(controller.sample(&snap, &s, true, true, 0.1).axes[6], 0.0);
                if mode == ZoomMode::Buttons {
                    if trigger {
                        snap.devices[0].axes[6] = 0.8;
                    } else {
                        snap.devices[0].button(8, true);
                    }
                } else {
                    snap.devices[0].axes[6] = 0.8;
                    if mode == ZoomMode::HoldAxis {
                        if trigger {
                            snap.devices[0].axes[7] = 0.8;
                        } else {
                            snap.devices[0].button(8, true);
                        }
                    }
                }
                assert!(
                    controller.sample(&snap, &s, true, true, 0.1).axes[6] < 0.0,
                    "zoom resumes after release"
                );
            }
        }
    }

    #[test]
    fn joystick_zoom_reset_discards_tap_history_across_input_lifecycle_changes() {
        for barrier in 0..5 {
            let (mut snap, mut s, mut controller) = fixture();
            s.mapping[6] = 6;
            s.flycam.gain[6] = 1.0;
            controller.sample(&snap, &s, true, true, 0.1);
            snap.devices[0].axes[6] = 0.8;
            controller.sample(&snap, &s, true, true, 0.1);
            snap.devices[0].axes[6] = 0.0;
            controller.sample(&snap, &s, true, true, 0.1);
            match barrier {
                0 => {
                    controller.sample(&snap, &s, false, true, 0.1);
                }
                1 => {
                    snap.updated = Some(Instant::now() - std::time::Duration::from_secs(1));
                    controller.sample(&snap, &s, true, true, 0.1);
                    snap.updated = Some(Instant::now());
                }
                2 => snap.devices[0].id = "replacement".into(),
                3 => s.flycam.gain[6] = 0.7,
                _ => {
                    controller.sample(&snap, &s, true, false, 0.1);
                }
            }
            controller.sample(&snap, &s, true, true, 0.1);
            snap.devices[0].axes[6] = 0.8;
            assert!(
                !controller.sample(&snap, &s, true, true, 0.1).reset_zoom,
                "a tap cannot cross an input reset"
            );
        }
    }

    #[test]
    fn joystick_zoom_buttons_change_both_directions_cancel_and_stop_on_release() {
        for triggers in [false, true] {
            let (mut snap, mut s, mut controller) = fixture();
            s.zoom_mode = ZoomMode::Buttons;
            s.zoom_in_button = if triggers { 44 } else { 8 };
            s.zoom_out_button = if triggers { 46 } else { 9 };
            s.mapping[4] = 6;
            s.mapping[5] = 7;
            s.zoom_speed = 0.8;
            s.direct_zoom = false;
            let mut camera = flycam::Flycam::from_view(glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z, 1.0);
            controller.sample(&snap, &s, true, true, 0.1);
            for (inside, outside, expected) in [(true, false, 0.92), (false, false, 0.92), (false, true, 1.0), (true, true, 1.0)] {
                if triggers {
                    snap.devices[0].axes[6] = f32::from(u8::from(inside));
                    snap.devices[0].axes[7] = f32::from(u8::from(outside));
                } else {
                    snap.devices[0].button(8, inside);
                    snap.devices[0].button(9, outside);
                }
                let motion = controller.sample(&snap, &s, true, true, 0.1);
                camera.step(motion.axes, &s, 0.1);
                assert!(
                    (camera.fov - expected).abs() < 1e-5,
                    "button zoom must integrate without an axis assignment or gain"
                );
                assert_eq!(
                    camera.rotation,
                    glam::Quat::IDENTITY,
                    "a trigger zoom binding must not rotate the camera"
                );
            }
            assert_eq!(controller.sample(&snap, &s, true, false, 0.1).axes[6], 0.0);
        }
    }

    #[test]
    fn joystick_hold_zoom_consumes_only_its_axis_and_waits_for_neutral_before_camera_return() {
        let (mut snap, mut s, mut controller) = fixture();
        s.zoom_mode = ZoomMode::HoldAxis;
        s.zoom_hold_button = 10;
        s.mapping[6] = s.mapping[5];
        s.flycam.gain[6] = 1.0;
        s.flycam.feather = 0.0;
        s.direct_zoom = false;
        let mut camera = flycam::Flycam::from_view(glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z, 1.0);
        controller.sample(&snap, &s, true, true, 0.1);
        snap.devices[0].axes[5] = 0.8;
        let ordinary = controller.sample(&snap, &s, true, true, 0.1);
        assert!(ordinary.axes[5] < 0.0 && ordinary.axes[6] == 0.0);
        snap.devices[0].button(10, true);
        let zoom = controller.sample(&snap, &s, true, true, 0.1);
        assert!(zoom.axes[6] < 0.0 && zoom.axes[5] == 0.0);
        let rotation = camera.rotation;
        camera.step(zoom.axes, &s, 0.1);
        let narrowed = camera.fov;
        assert!(narrowed < 1.0 && camera.rotation == rotation);
        snap.devices[0].axes[5] = -0.8;
        snap.devices[0].axes[1] = -0.5;
        let zoom = controller.sample(&snap, &s, true, true, 0.1);
        assert!(
            zoom.axes[6] > 0.0 && zoom.axes[5] == 0.0 && zoom.axes[0] > 0.0,
            "unrelated movement stays available"
        );
        camera.step(zoom.axes, &s, 0.1);
        assert!((camera.fov - 1.0).abs() < 1e-5);
        snap.devices[0].axes[1] = 0.0;
        snap.devices[0].button(10, false);
        let released = controller.sample(&snap, &s, true, true, 0.1);
        assert!(
            released.axes[5] == 0.0 && released.axes[6] == 0.0,
            "release must stop zoom and not replay camera rotation"
        );
        camera.step(released.axes, &s, 0.1);
        assert!(
            (camera.fov - 1.0).abs() < 1e-5,
            "relative zoom must retain the adjusted lens on release"
        );
        snap.devices[0].axes[5] = 0.0;
        controller.sample(&snap, &s, true, true, 0.1);
        snap.devices[0].axes[5] = 0.8;
        assert!(controller.sample(&snap, &s, true, true, 0.1).axes[5] < 0.0);
    }

    #[test]
    fn joystick_zoom_modes_ignore_held_buttons_after_focus_loss_or_stale_input() {
        for mode in [ZoomMode::Buttons, ZoomMode::HoldAxis] {
            for binding in [8, 44] {
                let (mut snap, mut s, mut controller) = fixture();
                s.zoom_mode = mode;
                s.zoom_in_button = binding;
                s.zoom_hold_button = binding;
                s.mapping[6] = 6;
                s.flycam.gain[6] = 1.0;
                s.flycam.feather = 0.0;
                controller.sample(&snap, &s, true, true, 0.1);
                snap.devices[0].button(8, true);
                snap.devices[0].axes[6] = 0.8;
                assert!(controller.sample(&snap, &s, true, true, 0.1).axes[6] < 0.0);
                assert_eq!(controller.sample(&snap, &s, false, true, 0.1).axes, [0.0; CHANNELS]);
                assert_eq!(
                    controller.sample(&snap, &s, true, true, 0.1).axes[6],
                    0.0,
                    "a held button must not restart after focus recovery"
                );
                snap.devices[0].button(8, false);
                if binding == 44 {
                    snap.devices[0].axes[6] = 0.0;
                }
                controller.sample(&snap, &s, true, true, 0.1);
                snap.devices[0].button(8, true);
                snap.devices[0].axes[6] = 0.8;
                assert!(controller.sample(&snap, &s, true, true, 0.1).axes[6] < 0.0);
                snap.updated = Some(Instant::now() - std::time::Duration::from_secs(1));
                assert_eq!(controller.sample(&snap, &s, true, true, 0.1).axes, [0.0; CHANNELS]);
            }
        }
    }

    #[test]
    fn joystick_zoom_modes_preserve_legacy_axis_settings_and_round_trip_new_assignments() {
        let legacy: Settings = serde_json::from_str(r#"{"direct_zoom":true,"mapping":[1,0,2,4,3,5,6]}"#).expect("legacy settings");
        assert_eq!(legacy.zoom_mode, ZoomMode::Axis);
        assert!(legacy.absolute_zoom());
        assert_eq!(button_axis(31), None);
        assert_eq!(button_axis(32), Some((0, true)));
        assert_eq!(button_axis(47), Some((7, false)));
        assert_eq!(button_axis(48), None);
        for mode in [ZoomMode::Buttons, ZoomMode::HoldAxis] {
            let chosen = Settings {
                zoom_mode: mode,
                zoom_in_button: 44,
                zoom_out_button: 9,
                zoom_hold_button: 31,
                zoom_speed: 1.7,
                ..legacy.clone()
            };
            let restored: Settings = serde_json::from_str(&serde_json::to_string(&chosen).expect("saved zoom")).expect("restored zoom");
            assert_eq!(restored, chosen);
            assert!(restored.absolute_zoom());
        }
    }

    #[test]
    fn joystick_shared_axis_inversion_preserves_mode_tuning_and_legacy_signs() {
        let (mut snap, mut s, _) = fixture();
        snap.devices[0].axes[1] = -0.8;
        s.avatar.gain[0] = 1.7;
        s.flycam.gain[0] = 0.9;
        let zones = (s.avatar.dead_zone, s.flycam.dead_zone);
        let avatar = mapped(&snap.devices[0], &s, &s.avatar)[0];
        let camera = mapped(&snap.devices[0], &s, &s.flycam)[0];
        assert!(avatar > 0.0 && camera > 0.0);
        s.set_axis_inverted(0, true);
        assert_eq!((s.avatar.gain[0], s.flycam.gain[0]), (-1.7, -0.9));
        assert_eq!(mapped(&snap.devices[0], &s, &s.avatar)[0], -avatar);
        assert_eq!(mapped(&snap.devices[0], &s, &s.flycam)[0], -camera);
        assert_eq!((s.avatar.dead_zone, s.flycam.dead_zone), zones);
        s.set_axis_inverted(0, false);
        assert_eq!(mapped(&snap.devices[0], &s, &s.avatar)[0], avatar);
        assert_eq!(mapped(&snap.devices[0], &s, &s.flycam)[0], camera);
        for negative_avatar in [false, true] {
            // Either legacy mode's negative sign now reverses both modes.
            s.avatar.gain[0] = if negative_avatar { -1.7 } else { 1.7 };
            s.flycam.gain[0] = if negative_avatar { 0.9 } else { -0.9 };
            assert!(s.axis_inverted(0));
            assert_eq!(mapped(&snap.devices[0], &s, &s.avatar)[0], -avatar);
            assert_eq!(mapped(&snap.devices[0], &s, &s.flycam)[0], -camera);
        }
        for channel in [3, 6] {
            s.avatar.gain[channel] = -1.0;
            s.flycam.gain[channel] = 0.0;
            assert!(!s.axis_inverted(channel), "unused Avatar gains do not invert Camera-only axes");
            s.set_axis_inverted(channel, true);
            let json = serde_json::to_string(&s).expect("saved shared inversion");
            let restored: Settings = serde_json::from_str(&json).expect("restored shared inversion");
            assert!(restored.axis_inverted(channel), "inverted zero persists until gain is raised");
        }
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
        snap.devices[0].button(0, true);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(m.toggle_flycam);
        assert!(!m.moving());
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

    #[test]
    fn joystick_flycam_toggle_survives_held_axes_and_rapid_mode_changes() {
        let (mut snap, s, mut c) = fixture();
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].axes[1] = -0.8;
        snap.devices[0].button(0, true);
        assert!(c.sample(&snap, &s, true, false, 0.1).toggle_flycam);
        // The next complete press can arrive before the first flycam frame.
        snap.devices[0].button(0, false);
        snap.devices[0].button(0, true);
        snap.devices[0].button(0, false);
        let exit = c.sample(&snap, &s, true, true, 0.1);
        assert!(exit.toggle_flycam);
        assert!(!exit.moving(), "held movement still waits for its own neutral");
        assert!(!c.sample(&snap, &s, true, false, 0.1).toggle_flycam);
        snap.devices[0].button(0, true);
        assert!(c.sample(&snap, &s, true, false, 0.1).toggle_flycam);
        assert!(!c.sample(&snap, &s, true, true, 0.1).toggle_flycam, "a held toggle is not replayed");
        snap.devices[0].axes[1] = 0.0;
        c.sample(&snap, &s, true, true, 0.1);
        snap.devices[0].axes[1] = -0.8;
        assert!(c.sample(&snap, &s, true, true, 0.1).axes[0] > 0.0);
    }

    #[test]
    fn joystick_enabling_zoom_cannot_lock_other_axes_or_buttons() {
        let (mut snap, mut s, mut c) = fixture();
        c.sample(&snap, &s, true, true, 0.1);
        // An unsigned trigger is reported as -1 at rest by the raw backend.
        snap.devices[0].axes[6] = -1.0;
        s.mapping[6] = 6;
        s.flycam.gain[6] = 0.5;
        c.sample(&snap, &s, true, true, 0.1);
        snap.devices[0].axes[1] = -0.8;
        snap.devices[0].button(0, true);
        let m = c.sample(&snap, &s, true, true, 0.1);
        assert!(m.axes[0] > 0.0);
        assert_eq!(m.axes[6], 0.0, "relative zoom still needs its own neutral");
        assert!(m.toggle_flycam);
        snap.devices[0].axes[6] = 0.0;
        c.sample(&snap, &s, true, true, 0.1);
        snap.devices[0].axes[6] = -0.5;
        let m = c.sample(&snap, &s, true, true, 0.1);
        let mut cam = flycam::Flycam::from_view(glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z, 1.0);
        cam.step(m.axes, &s, 0.1);
        assert!(cam.fov > 1.0, "an armed zoom channel still reaches the lens");
    }

    #[test]
    fn joystick_fresh_movement_buttons_work_on_the_first_frame_after_a_mode_switch() {
        let (mut snap, mut s, mut c) = fixture();
        s.rise_button = 4;
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].button(4, true);
        assert!(c.sample(&snap, &s, true, true, 0.1).rise);
        snap.devices[0].button(1, true);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(m.jump && m.jump_pressed, "the jump press arrived after the mode switch");
        assert!(!m.rise, "the rise button was already held in flycam");
        assert!(c.sample(&snap, &s, true, false, 0.1).jump, "a fresh press can stay held");
    }

    #[test]
    fn joystick_absolute_zoom_does_not_require_a_centre_position() {
        let (mut snap, mut s, mut c) = fixture();
        s.mapping[6] = 6;
        s.flycam.gain[6] = 0.5;
        s.direct_zoom = true;
        snap.devices[0].axes[6] = 0.8;
        assert!(c.sample(&snap, &s, true, true, 0.1).axes[6] < 0.0);
        snap.devices[0].axes[1] = -0.8;
        snap.devices[0].button(0, true);
        let m = c.sample(&snap, &s, true, true, 0.1);
        assert!(m.axes[0] > 0.0 && m.axes[6] < 0.0 && m.toggle_flycam);
    }

    #[test]
    fn joystick_avatar_ignores_unused_roll_and_zoom_channels() {
        let (mut snap, mut s, mut c) = fixture();
        for n in [3, 6] {
            s.mapping[n] = 6;
            s.avatar.gain[n] = 1.0;
            s.avatar.dead_zone[n] = 0.0;
        }
        snap.devices[0].axes[6] = -1.0;
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].axes[1] = -0.8;
        snap.devices[0].button(0, true);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(m.axes[0] > 0.0 && m.toggle_flycam);
        assert_eq!((m.axes[3], m.axes[6]), (0.0, 0.0));
    }

    #[test]
    fn joystick_reacquisition_suppresses_old_buttons_without_blocking_new_input() {
        let (mut snap, mut s, mut c) = fixture();
        s.rise_button = 4;
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].axes[1] = -0.8;
        snap.devices[0].button(0, true);
        snap.devices[0].button(1, true);
        snap.devices[0].button(4, true);
        c.sample(&snap, &s, false, false, 0.1);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(!m.moving() && !m.toggle_flycam && !m.jump_pressed);
        // A fresh toggle works even while other buttons and an axis stay held.
        snap.devices[0].button(0, false);
        snap.devices[0].button(0, true);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(m.toggle_flycam && !m.moving() && !m.jump_pressed);
        snap.devices[0].axes[1] = 0.0;
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].axes[1] = -0.8;
        assert!(c.sample(&snap, &s, true, false, 0.1).axes[0] > 0.0);
        snap.devices[0].button(1, false);
        snap.devices[0].button(4, false);
        c.sample(&snap, &s, true, false, 0.1);
        snap.devices[0].button(1, true);
        snap.devices[0].button(4, true);
        let m = c.sample(&snap, &s, true, false, 0.1);
        assert!(m.jump && m.jump_pressed && m.rise);
    }
}
