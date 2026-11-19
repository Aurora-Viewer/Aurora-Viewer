//! Game Controls: a game controller bound to the avatar, the mouselook view,
//! the flycam, the seated avatar and the mouse cursor, and the controller's
//! state sent to the region's scripts in GameControlData.
//!
//! Port of LLGameControl and LLGameControllerManager
//! (indra/llwindow/llgamecontrol.cpp and llgamecontrol.h of the LL viewer
//! 26.5, originally LGPL 2.1), the frame order of LLAppViewer::idle and
//! sendGameControlData (indra/newview/llappviewer.cpp) and
//! LLAgent::updateGameControlMode (indra/newview/llagent.cpp). The Legacy
//! Viewer does not have it yet.
//!
//! - `mappings`: the actions, the per-mode bindings and their defaults
//!   (buildDefaultModeMappings), the lookup from a canonical input to its
//!   action (rebuildActionLookup).
//! - `device`: a controller's own options (axis inversion, dead zone,
//!   offset, axis and button remapping) and their saved string.
//! - `engine`: the canonical and semantic state, the keyboard's share of it,
//!   the resend schedule and the packing of GameControlData.
//! - `actions`: what the controller does to the viewer
//!   (computeAgentActions, getFlycamInputs).
//!
//! The controllers are read by the joystick worker (`joystick::Backend`),
//! which already knows each gamepad's standardized sticks, triggers, D-pad
//! and buttons: they become the canonical channels LL's viewer takes from
//! SDL's gamepad API.

pub mod actions;
pub mod device;
pub mod engine;
pub mod flycam;
pub mod mappings;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use engine::Engine;

/// Canonical axes (LLGameControl::KeyboardAxis): left stick X and Y, right
/// stick X and Y, left and right triggers. Controller frame: + is right and
/// down (towards the user), triggers 0..32767.
pub const NUM_AXES: usize = 6;
pub const AXIS_LEFTX: usize = 0;
pub const AXIS_LEFTY: usize = 1;
pub const AXIS_RIGHTX: usize = 2;
pub const AXIS_RIGHTY: usize = 3;
pub const AXIS_LEFT_TRIGGER: usize = 4;
pub const AXIS_RIGHT_TRIGGER: usize = 5;

/// Canonical buttons (LLGameControl::Button), in SDL's gamepad order.
pub const NUM_BUTTONS: usize = 32;
pub const BUTTON_SOUTH: usize = 0;
pub const BUTTON_EAST: usize = 1;
pub const BUTTON_WEST: usize = 2;
pub const BUTTON_NORTH: usize = 3;
pub const BUTTON_SELECT: usize = 4;
pub const BUTTON_HOME: usize = 5;
pub const BUTTON_START: usize = 6;
pub const BUTTON_LEFT_STICK: usize = 7;
pub const BUTTON_RIGHT_STICK: usize = 8;
pub const BUTTON_LEFT_SHOULDER: usize = 9;
pub const BUTTON_RIGHT_SHOULDER: usize = 10;
pub const BUTTON_DPAD_UP: usize = 11;
pub const BUTTON_DPAD_DOWN: usize = 12;
pub const BUTTON_DPAD_LEFT: usize = 13;
pub const BUTTON_DPAD_RIGHT: usize = 14;

/// The names of the canonical buttons in the mappings
/// (InputChannel::getRemoteName); the higher ones are `BUTTON_<n>`.
pub const BUTTON_NAMES: [&str; 21] = [
    "BUTTON_SOUTH",
    "BUTTON_EAST",
    "BUTTON_WEST",
    "BUTTON_NORTH",
    "BUTTON_SELECT",
    "BUTTON_HOME",
    "BUTTON_START",
    "BUTTON_LEFT_STICK",
    "BUTTON_RIGHT_STICK",
    "BUTTON_LEFT_SHOULDER",
    "BUTTON_RIGHT_SHOULDER",
    "BUTTON_DPAD_UP",
    "BUTTON_DPAD_DOWN",
    "BUTTON_DPAD_LEFT",
    "BUTTON_DPAD_RIGHT",
    "BUTTON_MISC1",
    "BUTTON_PADDLE1",
    "BUTTON_PADDLE2",
    "BUTTON_PADDLE3",
    "BUTTON_PADDLE4",
    "BUTTON_TOUCHPAD",
];

/// The names of the canonical axes in the mappings.
pub const AXIS_NAMES: [&str; NUM_AXES] = [
    "AXIS_LEFTX",
    "AXIS_LEFTY",
    "AXIS_RIGHTX",
    "AXIS_RIGHTY",
    "AXIS_LEFT_TRIGGER",
    "AXIS_RIGHT_TRIGGER",
];
/// The two triggers as one axis: left the negative side, right the positive.
pub const AXIS_TRIGGERS: &str = "AXIS_TRIGGERS";
pub const AXIS_NONE: &str = "AXIS_NONE";
pub const BUTTON_NONE: &str = "BUTTON_NONE";

/// What the preferences show of the controllers, copied each frame.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The mode the controller is in now.
    pub mode: Mode,
    pub devices: Vec<DeviceView>,
    /// The state of the last GameControlData sent (the Server Data tab
    /// changes only when one goes out).
    pub sent: Option<engine::ServerState>,
    /// The input held now, for a binding being captured.
    pub active: Option<engine::ActiveInput>,
}

/// One connected controller, for the preferences.
#[derive(Debug, Clone, Default)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    /// The physical axes as read, and after offset, invert and dead zone.
    pub raw: [i16; NUM_AXES],
    pub fixed: [i16; NUM_AXES],
    pub buttons: u32,
}

impl View {
    pub fn of(engine: &engine::Engine) -> View {
        View {
            mode: engine.mode,
            devices: engine
                .controllers
                .iter()
                .map(|c| DeviceView {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    raw: c.reading.axes,
                    fixed: c.mapped.fixed,
                    buttons: c.reading.buttons,
                })
                .collect(),
            sent: engine.sent_state,
            active: engine.active_input(),
        }
    }
}

/// Semantic slots of the largest mode (the flycam's seven channels).
pub const NUM_SEMANTIC_SLOTS: usize = 7;

/// The name of a canonical button in the mappings.
pub fn button_name(index: usize) -> String {
    match BUTTON_NAMES.get(index) {
        Some(name) => (*name).to_owned(),
        None => format!("BUTTON_{index}"),
    }
}

/// What the controller drives (LLGameControl::AgentControlMode); its number
/// is GameControlData's ActionMode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Mode {
    #[default]
    Avatar = 0,
    Mouselook = 1,
    Flycam = 2,
    /// Seated.
    Captive = 3,
    /// The left stick drives the mouse cursor.
    Cursor = 4,
    None = 5,
}

impl Mode {
    /// The five modes with mappings, in the order of the preferences.
    pub const ALL: [Mode; 5] = [Mode::Avatar, Mode::Mouselook, Mode::Flycam, Mode::Captive, Mode::Cursor];

    /// Its name in the settings (getModeName).
    pub fn name(self) -> &'static str {
        match self {
            Mode::Avatar => "Avatar",
            Mode::Mouselook => "Mouselook",
            Mode::Flycam => "FlyCam",
            Mode::Captive => "Captive",
            Mode::Cursor => "Cursor",
            Mode::None => "",
        }
    }

    /// ModeAxes values ever packed for the mode (numSemanticAxesForMode).
    pub fn semantic_axes(self) -> usize {
        match self {
            Mode::Avatar | Mode::Mouselook | Mode::Captive => 5,
            Mode::Cursor => 6,
            Mode::Flycam => 7,
            Mode::None => 0,
        }
    }

    /// The mode of this frame (LLAgent::updateGameControlMode): the mouse
    /// cursor first, then the flycam, mouselook, seated, the avatar.
    pub fn of(cursor: bool, flycam: bool, mouselook: bool, sitting: bool) -> Mode {
        if cursor {
            Mode::Cursor
        } else if flycam {
            Mode::Flycam
        } else if mouselook {
            Mode::Mouselook
        } else if sitting {
            Mode::Captive
        } else {
            Mode::Avatar
        }
    }
}

/// One mode's bindings (the `ModeMappings/<Mode>` map of LL's GameControl
/// setting). `None`: the built-in defaults, as when LL's map lacks the kind
/// (getModeMapping, getAxisInvert).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeMapping {
    /// Action label → canonical input name (`AXIS_LEFTX`, `AXIS_TRIGGERS`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axes: Option<BTreeMap<String, String>>,
    /// Action label → canonical button name (`BUTTON_SOUTH`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buttons: Option<BTreeMap<String, String>>,
    /// Axis action label → inverted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axes_invert: Option<BTreeMap<String, bool>>,
    /// Off: the controller does not drive this mode (its data still reaches
    /// the region).
    pub enabled: bool,
    /// The "Slow".."Fast" slider (0.5..2.0): the flycam's speed, mouselook's
    /// turn and look, the avatar's turn.
    pub speed_factor: f32,
}

impl Default for ModeMapping {
    fn default() -> Self {
        Self {
            axes: None,
            buttons: None,
            axes_invert: None,
            enabled: true,
            speed_factor: 1.0,
        }
    }
}

pub const SPEED_FACTOR_MIN: f32 = 0.5;
pub const SPEED_FACTOR_MAX: f32 = 2.0;

/// The Game Controls preferences (LL's single `GameControl` setting).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "Send GameControl Data to server" (SendDataToServer).
    pub send_to_server: bool,
    pub avatar: ModeMapping,
    pub mouselook: ModeMapping,
    pub flycam: ModeMapping,
    pub captive: ModeMapping,
    pub cursor: ModeMapping,
    /// The flycam may roll (FlyCam/AllowRoll).
    pub flycam_allow_roll: bool,
    /// A crosshair in the middle of the view while the flycam flies
    /// (FlyCam/ShowCrosshair).
    pub flycam_crosshair: bool,
    /// The mouse cursor hides while the flycam flies and the mouse rests
    /// (FlyCam/HideCursor).
    pub flycam_hide_cursor: bool,
    /// Each controller's options (`Devices/<guid>/Config`), by device id.
    pub devices: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            send_to_server: true,
            avatar: ModeMapping::default(),
            mouselook: ModeMapping::default(),
            flycam: ModeMapping::default(),
            captive: ModeMapping::default(),
            cursor: ModeMapping::default(),
            flycam_allow_roll: false,
            flycam_crosshair: false,
            flycam_hide_cursor: true,
            devices: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn mode(&self, mode: Mode) -> Option<&ModeMapping> {
        match mode {
            Mode::Avatar => Some(&self.avatar),
            Mode::Mouselook => Some(&self.mouselook),
            Mode::Flycam => Some(&self.flycam),
            Mode::Captive => Some(&self.captive),
            Mode::Cursor => Some(&self.cursor),
            Mode::None => None,
        }
    }

    pub fn mode_mut(&mut self, mode: Mode) -> Option<&mut ModeMapping> {
        match mode {
            Mode::Avatar => Some(&mut self.avatar),
            Mode::Mouselook => Some(&mut self.mouselook),
            Mode::Flycam => Some(&mut self.flycam),
            Mode::Captive => Some(&mut self.captive),
            Mode::Cursor => Some(&mut self.cursor),
            Mode::None => None,
        }
    }

    /// The mode's axis bindings, its own or the defaults.
    pub fn axes(&self, mode: Mode) -> BTreeMap<String, String> {
        self.mode(mode)
            .and_then(|m| m.axes.clone())
            .unwrap_or_else(|| mappings::default_axes(mode))
    }

    pub fn buttons(&self, mode: Mode) -> BTreeMap<String, String> {
        self.mode(mode)
            .and_then(|m| m.buttons.clone())
            .unwrap_or_else(|| mappings::default_buttons(mode))
    }

    /// Whether an axis action is inverted (getAxisInvert): false when the
    /// mode's map does not name it.
    pub fn axis_invert(&self, mode: Mode, action: &str) -> bool {
        match self.mode(mode).and_then(|m| m.axes_invert.as_ref()) {
            Some(map) => map.get(action).copied().unwrap_or(false),
            None => mappings::default_axes_invert(mode).get(action).copied().unwrap_or(false),
        }
    }

    pub fn set_axis_invert(&mut self, mode: Mode, action: &str, invert: bool) {
        let current = self.mode(mode).and_then(|m| m.axes_invert.clone());
        let mut map = current.unwrap_or_else(|| mappings::default_axes_invert(mode));
        map.insert(action.to_owned(), invert);
        if let Some(m) = self.mode_mut(mode) {
            m.axes_invert = Some(map);
        }
    }

    /// Binds an action to an input (updateModeMapping): the axes when
    /// `axis`, the buttons otherwise.
    pub fn set_binding(&mut self, mode: Mode, axis: bool, action: &str, input: &str) {
        let mut map = if axis { self.axes(mode) } else { self.buttons(mode) };
        map.insert(action.to_owned(), input.to_owned());
        if let Some(m) = self.mode_mut(mode) {
            if axis {
                m.axes = Some(map);
            } else {
                m.buttons = Some(map);
            }
        }
    }

    /// Binds an action and frees the input from the mode's other actions of
    /// the same kind: an input drives one action (the preferences' commit,
    /// llpanelpreferencegamecontrol.cpp). `None` names clear the action.
    pub fn bind_exclusive(&mut self, mode: Mode, axis: bool, action: &str, input: &str) {
        let none = if axis { AXIS_NONE } else { BUTTON_NONE };
        if input != none && !input.is_empty() {
            let map = if axis { self.axes(mode) } else { self.buttons(mode) };
            for (other, bound) in map {
                if other != action && bound == input {
                    self.set_binding(mode, axis, &other, none);
                }
            }
        }
        self.set_binding(mode, axis, action, input);
    }

    /// Restore Defaults of the action mappings: the mode's bindings and
    /// inverts, its speed, and the flycam's options; not whether it is on.
    pub fn restore_mode_defaults(&mut self, mode: Mode) {
        if let Some(m) = self.mode_mut(mode) {
            m.axes = None;
            m.buttons = None;
            m.axes_invert = None;
            m.speed_factor = 1.0;
        }
        if mode == Mode::Flycam {
            self.flycam_allow_roll = false;
            self.flycam_crosshair = false;
            self.flycam_hide_cursor = true;
        }
    }

    /// The mode is driven by the controller (isModeEnabled).
    pub fn mode_enabled(&self, mode: Mode) -> bool {
        self.mode(mode).is_some_and(|m| m.enabled)
    }

    /// The mode's speed factor (getSpeedFactor).
    pub fn speed_factor(&self, mode: Mode) -> f32 {
        self.mode(mode).map_or(1.0, |m| m.speed_factor)
    }

    /// Values a settings file may hold out of range, brought back.
    pub fn sanitize(&mut self) {
        for mode in Mode::ALL {
            if let Some(m) = self.mode_mut(mode) {
                m.speed_factor = if m.speed_factor.is_finite() {
                    m.speed_factor.clamp(SPEED_FACTOR_MIN, SPEED_FACTOR_MAX)
                } else {
                    1.0
                };
            }
        }
    }
}
