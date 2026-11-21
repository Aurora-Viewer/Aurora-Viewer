//! What the controller does to the viewer this frame. Port of
//! LLGameControllerManager::computeAgentActions and getFlycamInputs
//! (indra/llwindow/llgamecontrol.cpp, LL viewer 26.5, originally LGPL 2.1).

use super::engine::Engine;
use super::mappings::Action;
use super::{NUM_AXES, NUM_BUTTONS};

/// AvatarMiscAction: one-shot commands, on the press of their button.
pub const TOGGLE_FLY: u32 = 1 << 0;
pub const TOGGLE_SIT: u32 = 1 << 1;
pub const TOGGLE_SPEAK: u32 = 1 << 2;
pub const TOGGLE_FLYCAM: u32 = 1 << 3;
pub const TOGGLE_MOUSELOOK: u32 = 1 << 4;
pub const TOGGLE_MOUSE_CURSOR: u32 = 1 << 5;
pub const ESCAPE: u32 = 1 << 6;

/// AvatarMouseButton: simulated mouse buttons, held with their button.
pub const MOUSE_LEFT: u32 = 1 << 0;
pub const MOUSE_RIGHT: u32 = 1 << 1;

/// FlycamMiscAction and FlycamModifier.
pub const FLYCAM_UNROLL: u32 = 1 << 0;
pub const FLYCAM_ESCAPE: u32 = 1 << 1;
pub const FLYCAM_ORBIT: u32 = 1 << 0;

/// AXIS_THRESHOLD: an axis past an eighth sets its control bit.
const AXIS_THRESHOLD: i32 = 32768 / 8;
/// RUN_ENGAGE_FRACTION / RUN_RELEASE_FRACTION of the movement stick.
const RUN_ENGAGE: f32 = 0.9;
const RUN_RELEASE: f32 = 0.7;

/// LLGameControl::AgentActions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AgentActions {
    pub misc: u32,
    pub mouse_buttons: u32,
    pub control_flags: u32,
    pub running: bool,
    /// A stick, not a button, moves the avatar this frame: only then does
    /// `running` apply (LL lets a D-pad walk end a run; here it keeps it).
    pub analog_movement: bool,
    /// -1..1, the turn and look rates.
    pub yaw: f32,
    pub pitch: f32,
    /// -1..1, the cursor mode's pointer.
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    /// -1..1, the field of view (+ narrows) and the camera distance (+
    /// closer).
    pub zoom: f32,
    pub scroll: f32,
}

/// The flycam's channels (FlycamChannel order: truck, dolly, pan, tilt,
/// boom, roll, zoom), each -1..1, and its one-shot and held buttons.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlycamInputs {
    pub channels: [f32; 7],
    pub misc: u32,
    pub modifiers: u32,
}

/// The run of an analog movement stick (mIsAnalogRunning), kept between
/// frames: on past 90 %, off under 70 %.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunLatch(pub bool);

fn keep(slot: &mut i32, value: i32) {
    if value.abs() > slot.abs() {
        *slot = value;
    }
}

/// computeAgentActions.
pub fn agent_actions(e: &Engine, run: &mut RunLatch) -> AgentActions {
    let mut r = AgentActions::default();
    let (mut movement, mut yaw, mut pitch, mut dx, mut dy, mut zoom, mut scroll) = (0i32, 0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
    for axis in 0..NUM_AXES {
        let Some(b) = e.lookup.axes[axis] else { continue };
        let value = b.value(e.inner_half[axis * 2], e.inner_half[axis * 2 + 1]);
        match b.action {
            Action::MouseX => keep(&mut dx, value),
            Action::MouseY => keep(&mut dy, value),
            Action::ZoomAxis => keep(&mut zoom, value),
            Action::ScrollAxis => keep(&mut scroll, value),
            action => {
                let Some((pos, neg, _)) = action.avatar_axis() else { continue };
                if value > AXIS_THRESHOLD {
                    r.control_flags |= pos;
                }
                if value < -AXIS_THRESHOLD {
                    r.control_flags |= neg;
                }
                match action {
                    Action::StrafeAxis | Action::AdvanceAxis => movement = movement.max(value.abs()),
                    Action::TurnAxis => keep(&mut yaw, value),
                    Action::LookAxis => keep(&mut pitch, value),
                    _ => {}
                }
            }
        }
    }
    let fraction = movement as f32 / 32767.0;
    if fraction > RUN_ENGAGE {
        run.0 = true;
    } else if fraction < RUN_RELEASE {
        run.0 = false;
    }
    let edges = e.inner_buttons & !e.inner_prev_buttons;
    for btn in 0..NUM_BUTTONS {
        if e.inner_buttons & (1 << btn) == 0 {
            continue;
        }
        let Some(action) = e.lookup.buttons[btn] else { continue };
        if let Some((flag, _, _)) = action.avatar_button() {
            r.control_flags |= flag;
            match action {
                Action::TurnLeft => keep(&mut yaw, 32767),
                Action::TurnRight => keep(&mut yaw, -32767),
                Action::LookUp => keep(&mut pitch, 32767),
                Action::LookDown => keep(&mut pitch, -32767),
                _ => {}
            }
            continue;
        }
        match action {
            Action::MouseClickLeft => r.mouse_buttons |= MOUSE_LEFT,
            Action::MouseClickRight => r.mouse_buttons |= MOUSE_RIGHT,
            // (LL's bridge gives Cursor up +1, which its cursor moves down
            // the screen: here up goes up)
            Action::CursorUp => keep(&mut dy, -32767),
            Action::CursorDown => keep(&mut dy, 32767),
            Action::CursorLeft => keep(&mut dx, -32767),
            Action::CursorRight => keep(&mut dx, 32767),
            Action::ZoomIn => keep(&mut zoom, 32767),
            Action::ZoomOut => keep(&mut zoom, -32767),
            Action::ScrollIn => keep(&mut scroll, 32767),
            Action::ScrollOut => keep(&mut scroll, -32767),
            _ if edges & (1 << btn) != 0 => {
                r.misc |= match action {
                    Action::ToggleFly => TOGGLE_FLY,
                    Action::ToggleSit => TOGGLE_SIT,
                    Action::ToggleSpeak => TOGGLE_SPEAK,
                    Action::ToggleFlycam => TOGGLE_FLYCAM,
                    Action::ToggleMouselook => TOGGLE_MOUSELOOK,
                    Action::ToggleMouseCursor => TOGGLE_MOUSE_CURSOR,
                    Action::Escape => ESCAPE,
                    _ => 0,
                };
            }
            _ => {}
        }
    }
    let unit = |v: i32| (v as f32 / 32767.0).clamp(-1.0, 1.0);
    r.running = run.0;
    r.analog_movement = movement > AXIS_THRESHOLD;
    r.yaw = unit(yaw);
    r.pitch = unit(pitch);
    r.mouse_dx = unit(dx);
    r.mouse_dy = unit(dy);
    r.zoom = unit(zoom);
    r.scroll = unit(scroll);
    r
}

/// getFlycamInputs: nothing while the flycam mode is off.
pub fn flycam_inputs(e: &Engine, enabled: bool) -> FlycamInputs {
    let mut out = FlycamInputs::default();
    if !enabled {
        return out;
    }
    for axis in 0..NUM_AXES {
        let Some(b) = e.lookup.axes[axis] else { continue };
        let Some(channel) = b.action.flycam_axis() else { continue };
        let pos = f32::from(e.inner_half[axis * 2]) / 32767.0;
        let neg = f32::from(e.inner_half[axis * 2 + 1]) / 32767.0;
        let deflection = pos - neg;
        let mut v = if b.half == super::mappings::Half::Negative {
            -deflection
        } else {
            deflection
        };
        if b.invert {
            v = -v;
        }
        out.channels[channel] += v;
    }
    let edges = e.inner_buttons & !e.inner_prev_buttons;
    for btn in 0..NUM_BUTTONS {
        if e.inner_buttons & (1 << btn) == 0 {
            continue;
        }
        let Some(action) = e.lookup.buttons[btn] else { continue };
        if let Some((channel, sign)) = action.flycam_button() {
            out.channels[channel] += sign;
            continue;
        }
        match action {
            Action::Orbit => out.modifiers |= FLYCAM_ORBIT,
            Action::Unroll if edges & (1 << btn) != 0 => out.misc |= FLYCAM_UNROLL,
            Action::Escape if edges & (1 << btn) != 0 => out.misc |= FLYCAM_ESCAPE,
            _ => {}
        }
    }
    for v in &mut out.channels {
        *v = v.clamp(-1.0, 1.0);
    }
    out
}
