//! The actions a controller input can be bound to, their defaults per mode,
//! and the lookup from a canonical input to its action. Port of
//! buildDefaultModeMappings, the avatar*/flycam*/cursor* bridge tables,
//! channelFromInputName, applyAxisMappings, applyButtonMappings and
//! rebuildActionLookup (indra/llwindow/llgamecontrol.cpp, LL viewer 26.5,
//! originally LGPL 2.1).
//!
//! The labels are LL's own: they are the keys of the saved mappings and
//! name the actions in the preferences (through `Action::label_key`).

use super::{AXIS_LEFT_TRIGGER, AXIS_NAMES, AXIS_RIGHT_TRIGGER, AXIS_TRIGGERS, BUTTON_NAMES, Mode, NUM_AXES, NUM_BUTTONS, Settings};
use aurora_net::control;
use std::collections::BTreeMap;

/// Every action label of the Game Controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // analog, avatar family
    StrafeAxis,
    AdvanceAxis,
    TurnAxis,
    LookAxis,
    FlyAxis,
    ZoomAxis,
    ScrollAxis,
    // analog, cursor
    MouseX,
    MouseY,
    // analog, flycam
    TruckAxis,
    DollyAxis,
    PanAxis,
    TiltAxis,
    BoomAxis,
    RollAxis,
    FlycamZoomAxis,
    // buttons, avatar movement (held)
    Jump,
    Crouch,
    FlyUp,
    FlyDown,
    AdvanceForward,
    AdvanceBack,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
    LookUp,
    LookDown,
    // buttons, one-shot
    ToggleFly,
    ToggleSit,
    ToggleSpeak,
    ToggleFlycam,
    ToggleMouselook,
    ToggleMouseCursor,
    Escape,
    // buttons, simulated mouse (held)
    MouseClickLeft,
    MouseClickRight,
    // buttons, camera
    ZoomIn,
    ZoomOut,
    ScrollIn,
    ScrollOut,
    // buttons, cursor
    CursorUp,
    CursorDown,
    CursorLeft,
    CursorRight,
    // buttons, flycam
    TruckLeft,
    TruckRight,
    DollyForward,
    DollyBack,
    PanLeft,
    PanRight,
    TiltUp,
    TiltDown,
    BoomUp,
    BoomDown,
    RollCcw,
    RollCw,
    FlycamZoomIn,
    FlycamZoomOut,
    Unroll,
    Orbit,
    /// "Interact": listed, does nothing yet in LL's viewer either.
    Interact,
    /// A saved label this viewer does not know: it holds its input and
    /// does nothing, as an unknown label does in LL's lookup.
    Other,
}

const LABELS: &[(Action, &str)] = &[
    (Action::StrafeAxis, "Strafe left/right"),
    (Action::AdvanceAxis, "Advance forward/back"),
    (Action::TurnAxis, "Turn left/right"),
    (Action::LookAxis, "Look up/down"),
    (Action::FlyAxis, "Fly up/down"),
    (Action::ZoomAxis, "Zoom +/-"),
    (Action::ScrollAxis, "Scroll +/-"),
    (Action::MouseX, "Mouse left/right"),
    (Action::MouseY, "Mouse up/down"),
    (Action::TruckAxis, "Truck left/right"),
    (Action::DollyAxis, "Dolly forward/back"),
    (Action::PanAxis, "Pan left/right"),
    (Action::TiltAxis, "Tilt up/down"),
    (Action::BoomAxis, "Boom up/down"),
    (Action::RollAxis, "Roll CCW/CW"),
    (Action::FlycamZoomAxis, "Zoom in/out"),
    (Action::Jump, "Jump"),
    (Action::Crouch, "Crouch"),
    (Action::FlyUp, "Fly up"),
    (Action::FlyDown, "Fly down"),
    (Action::AdvanceForward, "Advance forward"),
    (Action::AdvanceBack, "Advance back"),
    (Action::StrafeLeft, "Strafe left"),
    (Action::StrafeRight, "Strafe right"),
    (Action::TurnLeft, "Turn left"),
    (Action::TurnRight, "Turn right"),
    (Action::LookUp, "Look up"),
    (Action::LookDown, "Look down"),
    (Action::ToggleFly, "Toggle fly"),
    (Action::ToggleSit, "Toggle sit"),
    (Action::ToggleSpeak, "Toggle speak"),
    (Action::ToggleFlycam, "Toggle flycam"),
    (Action::ToggleMouselook, "Toggle mouselook"),
    (Action::ToggleMouseCursor, "Toggle mouse cursor"),
    (Action::Escape, "ESC"),
    (Action::MouseClickLeft, "Mouse click left"),
    (Action::MouseClickRight, "Mouse click right"),
    (Action::ZoomIn, "Zoom +"),
    (Action::ZoomOut, "Zoom -"),
    (Action::ScrollIn, "Scroll +"),
    (Action::ScrollOut, "Scroll -"),
    (Action::CursorUp, "Cursor up"),
    (Action::CursorDown, "Cursor down"),
    (Action::CursorLeft, "Cursor left"),
    (Action::CursorRight, "Cursor right"),
    (Action::TruckLeft, "Truck left"),
    (Action::TruckRight, "Truck right"),
    (Action::DollyForward, "Dolly forward"),
    (Action::DollyBack, "Dolly back"),
    (Action::PanLeft, "Pan left"),
    (Action::PanRight, "Pan right"),
    (Action::TiltUp, "Tilt up"),
    (Action::TiltDown, "Tilt down"),
    (Action::BoomUp, "Boom up"),
    (Action::BoomDown, "Boom down"),
    (Action::RollCcw, "Roll CCW"),
    (Action::RollCw, "Roll CW"),
    (Action::FlycamZoomIn, "Zoom in"),
    (Action::FlycamZoomOut, "Zoom out"),
    (Action::Unroll, "Unroll"),
    (Action::Orbit, "Orbit"),
    (Action::Interact, "Interact"),
];

/// The axis actions the preferences offer for a mode, in their order
/// (the `*_analog_action_selector` lists of
/// panel_preferences_game_control.xml).
pub fn axis_choices(mode: Mode) -> &'static [Action] {
    use Action::*;
    match mode {
        Mode::Avatar | Mode::Captive => &[StrafeAxis, AdvanceAxis, TurnAxis, LookAxis, FlyAxis, ZoomAxis, ScrollAxis],
        Mode::Mouselook => &[StrafeAxis, AdvanceAxis, TurnAxis, LookAxis, FlyAxis, ZoomAxis],
        Mode::Flycam => &[TruckAxis, DollyAxis, PanAxis, TiltAxis, BoomAxis, RollAxis, FlycamZoomAxis],
        Mode::Cursor => &[MouseX, MouseY, TurnAxis, LookAxis, FlyAxis, StrafeAxis, AdvanceAxis],
        Mode::None => &[],
    }
}

/// The button actions the preferences offer for a mode, in their order
/// (the `*_binary_action_selector` lists).
pub fn button_choices(mode: Mode) -> &'static [Action] {
    use Action::*;
    match mode {
        Mode::Avatar => &[
            Jump,
            Crouch,
            ToggleSit,
            ToggleSpeak,
            ToggleMouselook,
            ToggleFlycam,
            ToggleMouseCursor,
            AdvanceForward,
            AdvanceBack,
            StrafeLeft,
            StrafeRight,
            FlyUp,
            FlyDown,
            TurnLeft,
            TurnRight,
            ZoomIn,
            ZoomOut,
            ScrollIn,
            ScrollOut,
        ],
        Mode::Mouselook => &[
            Jump,
            Crouch,
            ToggleSit,
            ToggleSpeak,
            ToggleMouselook,
            ToggleFlycam,
            ToggleMouseCursor,
            MouseClickLeft,
            MouseClickRight,
            AdvanceForward,
            AdvanceBack,
            StrafeLeft,
            StrafeRight,
            FlyUp,
            FlyDown,
            TurnLeft,
            TurnRight,
            ZoomIn,
            ZoomOut,
            Escape,
        ],
        Mode::Flycam => &[
            FlycamZoomOut,
            PanRight,
            PanLeft,
            FlycamZoomIn,
            ToggleMouseCursor,
            ToggleFlycam,
            Unroll,
            RollCcw,
            RollCw,
            Orbit,
            DollyForward,
            DollyBack,
            TruckLeft,
            TruckRight,
            BoomUp,
            BoomDown,
            Escape,
        ],
        Mode::Captive => &[
            ToggleSit,
            ToggleSpeak,
            ToggleMouselook,
            ToggleFlycam,
            ToggleMouseCursor,
            ZoomIn,
            ZoomOut,
        ],
        Mode::Cursor => &[
            Jump,
            Crouch,
            ToggleSit,
            ToggleSpeak,
            ToggleMouselook,
            ToggleFlycam,
            ToggleMouseCursor,
            MouseClickLeft,
            MouseClickRight,
            AdvanceForward,
            AdvanceBack,
            StrafeLeft,
            StrafeRight,
            FlyUp,
            FlyDown,
            TurnLeft,
            TurnRight,
            CursorUp,
            CursorDown,
            CursorLeft,
            CursorRight,
            Escape,
        ],
        Mode::None => &[],
    }
}

impl Action {
    /// The flycam's roll actions, greyed while roll is off.
    pub fn is_roll(self) -> bool {
        matches!(self, Action::RollAxis | Action::RollCcw | Action::RollCw | Action::Unroll)
    }

    pub fn from_label(label: &str) -> Action {
        LABELS.iter().find(|(_, l)| *l == label).map_or(Action::Other, |(a, _)| *a)
    }

    /// LL's label, the key of the saved mappings.
    pub fn label(self) -> &'static str {
        LABELS.iter().find(|(a, _)| *a == self).map_or("", |(_, l)| l)
    }

    /// The avatar axis actions (avatarAxisBridge): the control bits of
    /// their positive and negative sides, and their semantic slot
    /// (avatarSemanticAxisSlots).
    pub fn avatar_axis(self) -> Option<(u32, u32, usize)> {
        Some(match self {
            Action::StrafeAxis => (control::LEFT_POS, control::LEFT_NEG, 0),
            Action::AdvanceAxis => (control::AT_POS, control::AT_NEG, 1),
            Action::TurnAxis => (control::YAW_POS, control::YAW_NEG, 2),
            Action::LookAxis => (control::PITCH_POS, control::PITCH_NEG, 3),
            Action::FlyAxis => (control::UP_POS, control::UP_NEG, 4),
            _ => return None,
        })
    }

    /// The held movement buttons (avatarButtonBridge): their control bit,
    /// and the semantic slot and sign they fold into
    /// (avatarSemanticButtonAxisSlots).
    pub fn avatar_button(self) -> Option<(u32, usize, i32)> {
        Some(match self {
            Action::Jump => (control::UP_POS, 4, 1),
            Action::Crouch => (control::UP_NEG, 4, -1),
            Action::FlyUp => (control::UP_POS, 4, 1),
            Action::FlyDown => (control::UP_NEG, 4, -1),
            Action::AdvanceForward => (control::AT_POS, 1, 1),
            Action::AdvanceBack => (control::AT_NEG, 1, -1),
            Action::StrafeLeft => (control::LEFT_POS, 0, 1),
            Action::StrafeRight => (control::LEFT_NEG, 0, -1),
            Action::TurnLeft => (control::YAW_POS, 2, 1),
            Action::TurnRight => (control::YAW_NEG, 2, -1),
            Action::LookUp => (control::PITCH_POS, 3, 1),
            Action::LookDown => (control::PITCH_NEG, 3, -1),
            _ => return None,
        })
    }

    /// The flycam axis actions (flycamAxisBridge): their channel.
    pub fn flycam_axis(self) -> Option<usize> {
        Some(match self {
            Action::TruckAxis => 0,
            Action::DollyAxis => 1,
            Action::PanAxis => 2,
            Action::TiltAxis => 3,
            Action::BoomAxis => 4,
            Action::RollAxis => 5,
            Action::FlycamZoomAxis => 6,
            _ => return None,
        })
    }

    /// The flycam buttons (flycamButtonBridge): their channel and sign.
    pub fn flycam_button(self) -> Option<(usize, f32)> {
        Some(match self {
            Action::TruckLeft => (0, 1.0),
            Action::TruckRight => (0, -1.0),
            Action::DollyForward => (1, 1.0),
            Action::DollyBack => (1, -1.0),
            Action::PanLeft => (2, 1.0),
            Action::PanRight => (2, -1.0),
            Action::TiltUp => (3, 1.0),
            Action::TiltDown => (3, -1.0),
            Action::BoomUp => (4, 1.0),
            Action::BoomDown => (4, -1.0),
            Action::RollCcw => (5, -1.0),
            Action::RollCw => (5, 1.0),
            Action::FlycamZoomIn => (6, 1.0),
            Action::FlycamZoomOut => (6, -1.0),
            _ => return None,
        })
    }

    /// The cursor's own actions, which take an input from the flycam when
    /// the cursor is shown over it (isMouseCursorAxisAction and
    /// isMouseCursorButtonAction).
    fn mouse_cursor(self) -> bool {
        matches!(
            self,
            Action::MouseX
                | Action::MouseY
                | Action::MouseClickLeft
                | Action::MouseClickRight
                | Action::CursorUp
                | Action::CursorDown
                | Action::CursorLeft
                | Action::CursorRight
        )
    }

    /// An axis action of the mode (isKnownAnalogAxisAction).
    fn known_axis(self, mode: Mode) -> bool {
        match mode {
            Mode::Flycam => self.flycam_axis().is_some(),
            Mode::Cursor => self.avatar_axis().is_some() || matches!(self, Action::MouseX | Action::MouseY),
            _ => self.avatar_axis().is_some() || matches!(self, Action::ZoomAxis | Action::ScrollAxis),
        }
    }
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
}

/// The built-in axis bindings of a mode (buildDefaultModeMappings).
pub fn default_axes(mode: Mode) -> BTreeMap<String, String> {
    match mode {
        Mode::Avatar | Mode::Captive => map(&[
            ("Strafe left/right", "AXIS_LEFTX"),
            ("Advance forward/back", "AXIS_LEFTY"),
            ("Turn left/right", "AXIS_RIGHTX"),
            ("Look up/down", "AXIS_RIGHTY"),
            ("Fly up/down", "AXIS_TRIGGERS"),
            ("Zoom +/-", "AXIS_NONE"),
            ("Scroll +/-", "AXIS_NONE"),
        ]),
        Mode::Mouselook => map(&[
            ("Strafe left/right", "AXIS_LEFTX"),
            ("Advance forward/back", "AXIS_LEFTY"),
            ("Turn left/right", "AXIS_RIGHTX"),
            ("Look up/down", "AXIS_RIGHTY"),
            ("Fly up/down", "AXIS_TRIGGERS"),
            ("Zoom +/-", "AXIS_NONE"),
        ]),
        Mode::Cursor => map(&[
            ("Mouse left/right", "AXIS_LEFTX"),
            ("Mouse up/down", "AXIS_LEFTY"),
            ("Turn left/right", "AXIS_RIGHTX"),
            ("Look up/down", "AXIS_RIGHTY"),
            ("Fly up/down", "AXIS_TRIGGERS"),
        ]),
        Mode::Flycam => map(&[
            ("Truck left/right", "AXIS_LEFTX"),
            ("Dolly forward/back", "AXIS_LEFTY"),
            ("Pan left/right", "AXIS_RIGHTX"),
            ("Tilt up/down", "AXIS_RIGHTY"),
            ("Boom up/down", "AXIS_TRIGGERS"),
            ("Roll CCW/CW", "AXIS_NONE"),
        ]),
        Mode::None => BTreeMap::new(),
    }
}

/// The built-in button bindings of a mode.
pub fn default_buttons(mode: Mode) -> BTreeMap<String, String> {
    match mode {
        Mode::Avatar | Mode::Captive => map(&[
            ("Jump", "BUTTON_SOUTH"),
            ("Crouch", "BUTTON_EAST"),
            ("Toggle sit", "BUTTON_WEST"),
            ("Toggle mouse cursor", "BUTTON_SELECT"),
            ("Toggle speak", "BUTTON_HOME"),
            ("Toggle mouselook", "BUTTON_START"),
            ("Toggle flycam", "BUTTON_RIGHT_STICK"),
            ("Advance forward", "BUTTON_DPAD_UP"),
            ("Advance back", "BUTTON_DPAD_DOWN"),
            ("Strafe left", "BUTTON_DPAD_LEFT"),
            ("Strafe right", "BUTTON_DPAD_RIGHT"),
            ("Scroll +", "BUTTON_NONE"),
            ("Scroll -", "BUTTON_NONE"),
        ]),
        Mode::Mouselook | Mode::Cursor => map(&[
            ("Jump", "BUTTON_SOUTH"),
            ("Crouch", "BUTTON_EAST"),
            ("Toggle sit", "BUTTON_WEST"),
            ("Toggle mouse cursor", "BUTTON_SELECT"),
            ("Toggle speak", "BUTTON_HOME"),
            ("Mouse click left", "BUTTON_LEFT_SHOULDER"),
            ("Mouse click right", "BUTTON_RIGHT_SHOULDER"),
            ("Toggle mouselook", "BUTTON_START"),
            ("Toggle flycam", "BUTTON_RIGHT_STICK"),
            ("ESC", "BUTTON_NONE"),
            ("Advance forward", "BUTTON_DPAD_UP"),
            ("Advance back", "BUTTON_DPAD_DOWN"),
            ("Strafe left", "BUTTON_DPAD_LEFT"),
            ("Strafe right", "BUTTON_DPAD_RIGHT"),
        ]),
        Mode::Flycam => map(&[
            ("Zoom out", "BUTTON_SOUTH"),
            ("Zoom in", "BUTTON_NORTH"),
            ("Toggle mouse cursor", "BUTTON_SELECT"),
            ("Toggle flycam", "BUTTON_RIGHT_STICK"),
            ("Unroll", "BUTTON_NONE"),
            ("Roll CCW", "BUTTON_NONE"),
            ("Roll CW", "BUTTON_NONE"),
            ("Orbit", "BUTTON_RIGHT_SHOULDER"),
            ("ESC", "BUTTON_NONE"),
        ]),
        Mode::None => BTreeMap::new(),
    }
}

/// The axis actions inverted by default.
pub fn default_axes_invert(mode: Mode) -> BTreeMap<String, bool> {
    let labels: &[&str] = match mode {
        Mode::Flycam => &["Truck left/right", "Dolly forward/back", "Pan left/right"],
        Mode::None => &[],
        _ => &["Strafe left/right", "Advance forward/back", "Turn left/right"],
    };
    labels.iter().map(|l| ((*l).to_owned(), true)).collect()
}

/// A canonical input named in the mappings (channelFromInputName).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Axis(usize),
    Button(usize),
    None,
}

pub fn input_from_name(name: &str) -> Input {
    if let Some(axis) = AXIS_NAMES.iter().position(|n| *n == name) {
        return Input::Axis(axis);
    }
    if let Some(button) = BUTTON_NAMES.iter().position(|n| *n == name) {
        return Input::Button(button);
    }
    // the higher buttons by number; BUTTON_0 is not read (index > 0)
    if let Some(n) = name.strip_prefix("BUTTON_") {
        let index = atoi(n);
        if index > 0 && (index as usize) < NUM_BUTTONS {
            return Input::Button(index as usize);
        }
    }
    Input::None
}

/// C's atoi: leading spaces, a sign, the digits that follow; 0 otherwise.
fn atoi(text: &str) -> i64 {
    let t = text.trim_start();
    let (sign, digits) = match t.as_bytes().first() {
        Some(b'-') => (-1, &t[1..]),
        Some(b'+') => (1, &t[1..]),
        _ => (1, t),
    };
    let run: String = digits.chars().take_while(char::is_ascii_digit).take(12).collect();
    sign * run.parse::<i64>().unwrap_or(0)
}

/// Which half of an input feeds an axis action (AxisActionBinding::half):
/// both sides of a stick, or one trigger of `AXIS_TRIGGERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Full,
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisBinding {
    pub action: Action,
    pub half: Half,
    pub invert: bool,
}

impl AxisBinding {
    /// The signed value an axis gives its action (the deflection of its two
    /// halves, the trigger side, the inversion).
    pub fn value(&self, positive: u16, negative: u16) -> i32 {
        let deflection = i32::from(positive) - i32::from(negative);
        let value = if self.half == Half::Negative { -deflection } else { deflection };
        if self.invert { -value } else { value }
    }
}

/// The action of each canonical input in the current mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Lookup {
    pub axes: [Option<AxisBinding>; NUM_AXES],
    pub buttons: [Option<Action>; NUM_BUTTONS],
}

impl Default for Lookup {
    fn default() -> Self {
        Self {
            axes: [None; NUM_AXES],
            buttons: [None; NUM_BUTTONS],
        }
    }
}

impl Lookup {
    /// rebuildActionLookup: the mode's bindings inverted into per-input
    /// actions, the last one winning a collision (LLSD maps go in key
    /// order, as `BTreeMap` does). The cursor over a flying flycam keeps
    /// the flycam's bindings and lays the cursor's own actions over them.
    pub fn build(settings: &Settings, mode: Mode, flycam_engaged: bool) -> Lookup {
        let mut lookup = Lookup::default();
        if mode == Mode::Cursor && flycam_engaged {
            lookup.apply_axes(settings, Mode::Flycam, &settings.axes(Mode::Flycam));
            lookup.apply_buttons(&settings.buttons(Mode::Flycam));
            let mut axes = settings.axes(Mode::Cursor);
            axes.retain(|label, _| Action::from_label(label).mouse_cursor());
            lookup.apply_axes(settings, Mode::Cursor, &axes);
            let mut buttons = settings.buttons(Mode::Cursor);
            buttons.retain(|label, _| Action::from_label(label).mouse_cursor());
            lookup.apply_buttons(&buttons);
            return lookup;
        }
        if mode == Mode::None {
            return lookup;
        }
        lookup.apply_axes(settings, mode, &settings.axes(mode));
        lookup.apply_buttons(&settings.buttons(mode));
        lookup
    }

    /// applyAxisMappings. A label that is no axis action of the mode (an
    /// older name in the settings) hands its input to the default owner of
    /// that input, unless that action is bound too.
    fn apply_axes(&mut self, settings: &Settings, mode: Mode, axes: &BTreeMap<String, String>) {
        for (label, input) in axes {
            let mut action = Action::from_label(label);
            if !action.known_axis(mode) {
                match default_axis_action_for_input(mode, input) {
                    Some(default) if !axes.contains_key(default.as_str()) => action = Action::from_label(&default),
                    _ => continue,
                }
            }
            let invert = settings.axis_invert(mode, action.label());
            if input == AXIS_TRIGGERS {
                self.axes[AXIS_LEFT_TRIGGER] = Some(AxisBinding {
                    action,
                    half: Half::Negative,
                    invert,
                });
                self.axes[AXIS_RIGHT_TRIGGER] = Some(AxisBinding {
                    action,
                    half: Half::Positive,
                    invert,
                });
                continue;
            }
            if let Input::Axis(axis) = input_from_name(input) {
                self.axes[axis] = Some(AxisBinding {
                    action,
                    half: Half::Full,
                    invert,
                });
            }
        }
    }

    /// applyButtonMappings.
    fn apply_buttons(&mut self, buttons: &BTreeMap<String, String>) {
        for (label, input) in buttons {
            if let Input::Button(button) = input_from_name(input) {
                self.buttons[button] = Some(Action::from_label(label));
            }
        }
    }
}

/// The action that holds an input in the defaults (defaultAxisActionForInput).
fn default_axis_action_for_input(mode: Mode, input: &str) -> Option<String> {
    default_axes(mode).into_iter().find(|(_, i)| i == input).map(|(label, _)| label)
}

/// The ModeButtons bit of each action of a mode: the canonical button its
/// default binding uses (getDefaultButtonSemanticSlots).
pub fn default_button_slot(mode: Mode, action: Action) -> Option<usize> {
    let label = action.label();
    if label.is_empty() {
        return None;
    }
    match input_from_name(default_buttons(mode).get(label)?) {
        Input::Button(b) => Some(b),
        _ => None,
    }
}

/// The semantic axis names of a mode (semanticAxisName), for the Server
/// Data tab.
pub fn semantic_axis_name(mode: Mode, slot: usize) -> &'static str {
    const AVATAR: [&str; 5] = ["AXIS_STRAFE", "AXIS_ADVANCE", "AXIS_TURN", "AXIS_LOOK", "AXIS_RISE"];
    const CURSOR: [&str; 6] = ["CURSOR_DX", "CURSOR_DY", "CURSOR_PX", "CURSOR_PY", "CURSOR_NX", "CURSOR_NY"];
    const FLYCAM: [&str; 7] = [
        "FLYCAM_AXIS_TRUCK",
        "FLYCAM_AXIS_DOLLY",
        "FLYCAM_AXIS_PAN",
        "FLYCAM_AXIS_TILT",
        "FLYCAM_AXIS_BOOM",
        "FLYCAM_AXIS_ROLL",
        "FLYCAM_AXIS_ZOOM",
    ];
    let names: &[&str] = match mode {
        Mode::Avatar | Mode::Mouselook | Mode::Captive => &AVATAR,
        Mode::Cursor => &CURSOR,
        Mode::Flycam => &FLYCAM,
        Mode::None => &[],
    };
    names.get(slot).copied().unwrap_or("")
}

/// The number of ModeButtons bits a mode uses (numSemanticButtonsForMode):
/// its highest default button, plus one.
pub fn semantic_buttons(mode: Mode) -> usize {
    default_buttons(mode)
        .values()
        .filter_map(|input| match input_from_name(input) {
            Input::Button(b) => Some(b + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// The label of a ModeButtons bit (semanticButtonName): the action whose
/// default binding is that button, or `MODE_BUTTON_<n>`.
pub fn semantic_button_name(mode: Mode, slot: usize) -> String {
    default_buttons(mode)
        .into_iter()
        .find(|(_, input)| input_from_name(input) == Input::Button(slot))
        .map_or_else(|| format!("MODE_BUTTON_{slot}"), |(label, _)| label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_reads_back() {
        for (action, label) in LABELS {
            assert_eq!(Action::from_label(label), *action);
            assert_eq!(action.label(), *label);
        }
        assert_eq!(Action::from_label("Rise up"), Action::Other);
    }

    #[test]
    fn input_names_follow_channel_from_input_name() {
        assert_eq!(input_from_name("AXIS_RIGHTY"), Input::Axis(3));
        assert_eq!(input_from_name("BUTTON_TOUCHPAD"), Input::Button(20));
        assert_eq!(input_from_name("BUTTON_21"), Input::Button(21));
        assert_eq!(input_from_name("BUTTON_31"), Input::Button(31));
        // the numeric fallback starts at 1 and stops below 32
        assert_eq!(input_from_name("BUTTON_0"), Input::None);
        assert_eq!(input_from_name("BUTTON_32"), Input::None);
        assert_eq!(input_from_name("BUTTON_NONE"), Input::None);
        assert_eq!(input_from_name("AXIS_TRIGGERS"), Input::None);
    }

    #[test]
    fn default_slots_match_the_message_template() {
        assert_eq!(default_button_slot(Mode::Avatar, Action::Jump), Some(0));
        assert_eq!(default_button_slot(Mode::Avatar, Action::ToggleFlycam), Some(8));
        assert_eq!(default_button_slot(Mode::Avatar, Action::StrafeRight), Some(14));
        assert_eq!(default_button_slot(Mode::Avatar, Action::FlyUp), None);
        assert_eq!(default_button_slot(Mode::Mouselook, Action::MouseClickRight), Some(10));
        assert_eq!(default_button_slot(Mode::Flycam, Action::Orbit), Some(10));
        assert_eq!(default_button_slot(Mode::Flycam, Action::FlycamZoomOut), Some(0));
        assert_eq!(default_button_slot(Mode::Flycam, Action::Unroll), None);
        assert_eq!(semantic_buttons(Mode::Avatar), 15);
        assert_eq!(semantic_buttons(Mode::Flycam), 11);
        assert_eq!(semantic_button_name(Mode::Avatar, 3), "MODE_BUTTON_3");
        assert_eq!(semantic_button_name(Mode::Avatar, 2), "Toggle sit");
    }

    #[test]
    fn the_default_lookup_binds_the_triggers_as_one_axis() {
        let lookup = Lookup::build(&Settings::default(), Mode::Avatar, false);
        let lt = lookup.axes[AXIS_LEFT_TRIGGER].expect("left trigger bound");
        let rt = lookup.axes[AXIS_RIGHT_TRIGGER].expect("right trigger bound");
        assert_eq!((lt.action, lt.half), (Action::FlyAxis, Half::Negative));
        assert_eq!((rt.action, rt.half), (Action::FlyAxis, Half::Positive));
        assert!(lookup.axes[0].expect("left x").invert, "Strafe is inverted by default");
        assert!(!lookup.axes[3].expect("right y").invert, "Look is not");
        assert_eq!(lookup.buttons[0], Some(Action::Jump));
        assert_eq!(lookup.buttons[3], None, "North is free in Avatar mode");
    }

    #[test]
    fn a_stale_axis_label_goes_to_the_default_owner_of_its_input() {
        let mut s = Settings::default();
        let mut axes = default_axes(Mode::Avatar);
        axes.remove("Fly up/down");
        axes.insert("Rise up".into(), "AXIS_TRIGGERS".into());
        s.avatar.axes = Some(axes.clone());
        let lookup = Lookup::build(&s, Mode::Avatar, false);
        assert_eq!(lookup.axes[AXIS_RIGHT_TRIGGER].map(|b| b.action), Some(Action::FlyAxis));
        // the default owner is bound already: the stale label is dropped
        axes.insert("Fly up/down".into(), "AXIS_NONE".into());
        s.avatar.axes = Some(axes);
        let lookup = Lookup::build(&s, Mode::Avatar, false);
        assert_eq!(lookup.axes[AXIS_RIGHT_TRIGGER], None);
    }

    #[test]
    fn the_cursor_over_the_flycam_takes_only_its_mouse_actions() {
        let lookup = Lookup::build(&Settings::default(), Mode::Cursor, true);
        assert_eq!(lookup.axes[0].map(|b| b.action), Some(Action::MouseX));
        assert_eq!(
            lookup.axes[2].map(|b| b.action),
            Some(Action::PanAxis),
            "the flycam keeps the right stick"
        );
        assert_eq!(lookup.buttons[0], Some(Action::FlycamZoomOut));
        assert_eq!(lookup.buttons[9], Some(Action::MouseClickLeft));
        assert_eq!(lookup.buttons[10], Some(Action::MouseClickRight), "over the flycam's Orbit");
    }
}
