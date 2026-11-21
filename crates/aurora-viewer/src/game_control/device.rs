//! A controller's own options and its physical readings. Port of
//! LLGameControl::Options, AxisOptions, parseDeviceOptions,
//! stringifyDeviceOptions and the axis routing of
//! LLGameControllerManager::onAxis / onButton
//! (indra/llwindow/llgamecontrol.cpp, LL viewer 26.5, originally LGPL 2.1).
//!
//! The physical axes and buttons are those of SDL's gamepad API, which the
//! joystick worker's standardized gamepad (`joystick::Device`) gives too:
//! sticks and triggers by role, buttons by their label.

use super::{
    AXIS_LEFT_TRIGGER, AXIS_RIGHT_TRIGGER, BUTTON_DPAD_DOWN, BUTTON_DPAD_LEFT, BUTTON_DPAD_RIGHT, BUTTON_DPAD_UP, BUTTON_EAST, BUTTON_HOME,
    BUTTON_LEFT_SHOULDER, BUTTON_LEFT_STICK, BUTTON_NORTH, BUTTON_RIGHT_SHOULDER, BUTTON_RIGHT_STICK, BUTTON_SELECT, BUTTON_SOUTH,
    BUTTON_START, BUTTON_WEST, NUM_AXES, NUM_BUTTONS,
};
use crate::joystick::GamepadButton;
use std::collections::BTreeMap;

/// A freshly reset axis ignores this much (most controllers rest a little
/// off centre).
pub const DEFAULT_AXIS_DEAD_ZONE: u16 = 2048;
pub const MAX_AXIS_DEAD_ZONE: u16 = 16384;
pub const MAX_AXIS_OFFSET: i32 = 16384;

/// Axis output codes (Options::mAxisMap): 0..6 a canonical axis, then None
/// and the trigger pair (negative half to the left trigger, positive to the
/// right).
pub const AXIS_OUTPUT_NONE: u8 = NUM_AXES as u8;
pub const AXIS_OUTPUT_TRIGGER_PAIR: u8 = NUM_AXES as u8 + 1;
pub const NUM_AXIS_OUTPUTS: u8 = NUM_AXES as u8 + 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisOptions {
    /// 1, or -1 inverted.
    pub multiplier: i32,
    pub dead_zone: u16,
    pub offset: i16,
}

impl Default for AxisOptions {
    fn default() -> Self {
        Self {
            multiplier: 1,
            dead_zone: DEFAULT_AXIS_DEAD_ZONE,
            offset: 0,
        }
    }
}

impl AxisOptions {
    /// computeModifiedValue: offset, inversion, then the dead zone (no
    /// rescaling outside it).
    pub fn apply(&self, raw: i16) -> i16 {
        let v = ((i32::from(raw) + i32::from(self.offset)) * self.multiplier).clamp(-32768, 32767) as i16;
        if i32::from(v).abs() < i32::from(self.dead_zone) { 0 } else { v }
    }

    fn save(&self) -> String {
        let mut options = Vec::new();
        if self.multiplier == -1 {
            options.push("invert:1".to_owned());
        }
        if self.dead_zone != DEFAULT_AXIS_DEAD_ZONE {
            options.push(format!("dead_zone:{}", self.dead_zone));
        }
        if self.offset != 0 {
            options.push(format!("offset:{}", self.offset));
        }
        if options.is_empty() {
            String::new()
        } else {
            format!("{{{}}}", options.join(","))
        }
    }

    /// loadFromString: what reads is kept, a bad value keeps its default.
    fn load(text: &str) -> AxisOptions {
        let mut o = AxisOptions::default();
        let Some(pairs) = parse(text) else {
            log::warn!("game control: invalid axis options");
            return o;
        };
        if let Some(invert) = pairs.get("invert") {
            if invert == "1" {
                o.multiplier = -1;
            } else {
                log::warn!("game control: invalid invert value");
            }
        }
        if let Some(text) = pairs.get("dead_zone") {
            match text.parse::<u32>() {
                Ok(n) if n <= u32::from(MAX_AXIS_DEAD_ZONE) && n.to_string() == *text => o.dead_zone = n as u16,
                _ => log::warn!("game control: invalid dead_zone value"),
            }
        }
        if let Some(text) = pairs.get("offset") {
            match text.parse::<i32>() {
                Ok(n) if n.abs() <= MAX_AXIS_OFFSET && n.to_string() == *text => o.offset = n as i16,
                _ => log::warn!("game control: invalid offset value"),
            }
        }
        o
    }
}

/// A controller's options (LLGameControl::Options).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub axes: [AxisOptions; NUM_AXES],
    /// Physical axis → output code.
    pub axis_map: [u8; NUM_AXES],
    /// Physical button → canonical button.
    pub button_map: [u8; NUM_BUTTONS],
}

impl Default for Options {
    fn default() -> Self {
        Self {
            axes: [AxisOptions::default(); NUM_AXES],
            axis_map: std::array::from_fn(|i| i as u8),
            button_map: std::array::from_fn(|i| i as u8),
        }
    }
}

impl Options {
    /// stringifyDeviceOptions: only what differs from the defaults, after
    /// the name; empty when nothing differs, unless `force`.
    pub fn save(&self, name: &str, force: bool) -> String {
        let mut options = Vec::new();
        let axes = join(self.axes.iter().enumerate().map(|(i, o)| {
            let s = o.save();
            if s.is_empty() { s } else { format!("{i}:{s}") }
        }));
        if !axes.is_empty() {
            options.push(format!("axis_options:{{{axes}}}"));
        }
        let remap = |values: &[u8]| {
            join(
                values
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| if usize::from(v) == i { String::new() } else { format!("{i}:{v}") }),
            )
        };
        let axis_map = remap(&self.axis_map);
        if !axis_map.is_empty() {
            options.push(format!("axis_map:{{{axis_map}}}"));
        }
        let button_map = remap(&self.button_map);
        if !button_map.is_empty() {
            options.push(format!("button_map:{{{button_map}}}"));
        }
        if !force && options.is_empty() {
            return String::new();
        }
        let safe: String = name.chars().filter(|c| !matches!(c, ',' | '{' | '}')).collect();
        options.insert(0, format!("name:{safe}"));
        format!("{{{}}}", join(options.into_iter()))
    }

    /// parseDeviceOptions: the whole string reads or every option keeps its
    /// default; a bad single value keeps its own default.
    pub fn load(text: &str) -> Options {
        let mut o = Options::default();
        if text.is_empty() {
            return o;
        }
        let Some(pairs) = parse(text) else {
            log::warn!("game control: invalid device options");
            return o;
        };
        let part = |key: &str| parse(pairs.get(key).map_or("", String::as_str));
        let (Some(axis_options), Some(axis_map), Some(button_map)) = (part("axis_options"), part("axis_map"), part("button_map")) else {
            log::warn!("game control: invalid device options");
            return o;
        };
        for i in 0..NUM_AXES {
            let key = i.to_string();
            if let Some(text) = axis_options.get(&key) {
                o.axes[i] = AxisOptions::load(text);
            }
            if let Some(text) = axis_map.get(&key) {
                match text.parse::<u32>() {
                    Ok(n) if n < u32::from(NUM_AXIS_OUTPUTS) && n.to_string() == *text => o.axis_map[i] = n as u8,
                    _ => log::warn!("game control: invalid axis mapping {i}"),
                }
            }
        }
        for (i, slot) in o.button_map.iter_mut().enumerate() {
            if let Some(text) = button_map.get(&i.to_string()) {
                match text.parse::<u32>() {
                    Ok(n) if (n as usize) < NUM_BUTTONS && n.to_string() == *text => *slot = n as u8,
                    _ => log::warn!("game control: invalid button mapping {i}"),
                }
            }
        }
        o
    }
}

/// The name saved in a device's options (`name:`), for a device that is not
/// connected.
pub fn saved_name(config: &str) -> Option<String> {
    parse(config)?.get("name").cloned()
}

/// CALIBRATION_SAMPLE_COUNT: frames read before the resting axes are set.
pub const CALIBRATION_FRAMES: usize = 5;

/// Auto calibrate (onAutoCalibrate, llpanelpreferencegamecontrol.cpp): an
/// axis whose raw value did not change over the samples is at rest, its
/// offset brings it to zero and its dead zone covers twice that offset
/// (2048 at least). A moving axis keeps its options.
pub fn calibrate(options: &mut Options, samples: &[[i16; NUM_AXES]]) {
    let Some(first) = samples.first() else { return };
    for axis in 0..NUM_AXES {
        if samples.iter().any(|s| s[axis] != first[axis]) {
            continue;
        }
        let offset = (-i32::from(first[axis])).clamp(-MAX_AXIS_OFFSET, MAX_AXIS_OFFSET);
        let a = &mut options.axes[axis];
        a.offset = offset as i16;
        a.dead_zone = (2 * offset.abs()).clamp(i32::from(DEFAULT_AXIS_DEAD_ZONE), i32::from(MAX_AXIS_DEAD_ZONE)) as u16;
    }
}

/// LLStringUtil::join: the non-empty strings, separated by commas.
fn join(values: impl Iterator<Item = String>) -> String {
    values.filter(|s| !s.is_empty()).collect::<Vec<_>>().join(",")
}

/// One `key:value` pair of [`parse`]; false when malformed.
fn split_pair(result: &mut BTreeMap<String, String>, pair: &str) -> bool {
    match pair.find(':') {
        Some(pos) if pos > 0 => {
            let (key, value) = (pair[..pos].trim(), pair[pos + 1..].trim());
            if key.is_empty() || value.is_empty() {
                return false;
            }
            result.insert(key.to_owned(), value.to_owned());
            true
        }
        _ => false,
    }
}

/// The `{key:value,…}` reader of llgamecontrol.cpp (`parse`): values may be
/// nested braces; keys and values are trimmed and must not be empty; a key
/// read twice keeps its last value. `None` when malformed.
fn parse(source: &str) -> Option<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    let source = source.trim();
    if source.is_empty() {
        return Some(result);
    }
    let inner = source.strip_prefix('{')?.strip_suffix('}')?.trim();
    if inner.is_empty() {
        return Some(result);
    }
    let mut rest = inner.to_owned();
    let mut depth = 0u32;
    let mut offset = 0usize;
    loop {
        let found = rest[offset..]
            .find(|c: char| {
                if depth > 0 {
                    c == '{' || c == '}'
                } else {
                    c == ',' || c == '{' || c == '}'
                }
            })
            .map(|p| p + offset);
        let Some(pos) = found else {
            return (depth == 0 && split_pair(&mut result, &rest)).then_some(result);
        };
        match rest.as_bytes()[pos] {
            b',' => {
                if !split_pair(&mut result, &rest[..pos]) {
                    return None;
                }
                rest = rest[pos + 1..].to_owned();
                offset = 0;
            }
            b'{' => {
                depth += 1;
                offset = pos + 1;
            }
            _ if depth > 0 => {
                depth -= 1;
                offset = pos + 1;
            }
            _ => return None,
        }
    }
}

/// What a controller reads this frame, by physical (SDL) axis and button.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reading {
    /// Sticks -32768..32767 (+ right, + down), triggers 0..32767.
    pub axes: [i16; NUM_AXES],
    pub buttons: u32,
}

/// The SDL gamepad button of a standardized gilrs label (SDL_GamepadButton
/// order: south, east, west, north, back, guide, start, sticks, shoulders,
/// D-pad).
fn sdl_button(button: GamepadButton) -> Option<usize> {
    Some(match button {
        GamepadButton::South => BUTTON_SOUTH,
        GamepadButton::East => BUTTON_EAST,
        GamepadButton::West => BUTTON_WEST,
        GamepadButton::North => BUTTON_NORTH,
        GamepadButton::Select => BUTTON_SELECT,
        GamepadButton::Mode => BUTTON_HOME,
        GamepadButton::Start => BUTTON_START,
        GamepadButton::LeftStick => BUTTON_LEFT_STICK,
        GamepadButton::RightStick => BUTTON_RIGHT_STICK,
        GamepadButton::LeftBumper => BUTTON_LEFT_SHOULDER,
        GamepadButton::RightBumper => BUTTON_RIGHT_SHOULDER,
        GamepadButton::DpadUp => BUTTON_DPAD_UP,
        GamepadButton::DpadDown => BUTTON_DPAD_DOWN,
        GamepadButton::DpadLeft => BUTTON_DPAD_LEFT,
        GamepadButton::DpadRight => BUTTON_DPAD_RIGHT,
        // digital triggers are an axis in SDL's gamepad API
        GamepadButton::LeftTrigger | GamepadButton::RightTrigger | GamepadButton::Other => return None,
    })
}

fn stick(value: f32) -> i16 {
    if !value.is_finite() {
        return 0;
    }
    let v = value.clamp(-1.0, 1.0);
    if v < 0.0 {
        (v * 32768.0).round().max(-32768.0) as i16
    } else {
        (v * 32767.0).round() as i16
    }
}

fn trigger(value: f32) -> i16 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * 32767.0).round() as i16
}

/// A standardized gamepad's reading in SDL's terms. gilrs-core already
/// gives sticks with + right and + down on every system, as SDL does.
pub fn reading(device: &crate::joystick::Device) -> Reading {
    let mut r = Reading::default();
    if !device.is_gamepad {
        return r;
    }
    let axis = |index: Option<usize>| index.and_then(|i| device.axes.get(i).copied());
    for (s, stick_axes) in device.gamepad_sticks.iter().enumerate() {
        for (c, index) in stick_axes.iter().enumerate() {
            if let Some(v) = axis(*index) {
                r.axes[s * 2 + c] = stick(v);
            }
        }
    }
    for (side, index) in device.gamepad_triggers.iter().enumerate() {
        if let Some(v) = axis(*index) {
            r.axes[AXIS_LEFT_TRIGGER + side] = trigger(v);
        }
    }
    for (index, label) in device.gamepad_buttons.iter().enumerate().take(crate::joystick::BUTTONS) {
        if device.buttons & (1 << index) == 0 {
            continue;
        }
        match label {
            GamepadButton::LeftTrigger => r.axes[AXIS_LEFT_TRIGGER] = 32767,
            GamepadButton::RightTrigger => r.axes[AXIS_RIGHT_TRIGGER] = 32767,
            other => {
                if let Some(b) = sdl_button(*other) {
                    r.buttons |= 1 << b;
                }
            }
        }
    }
    // a D-pad reported as a hat (+ right, + down after the worker)
    if let [Some(x), Some(y)] = device.gamepad_dpad {
        let (x, y) = (
            device.axes.get(x).copied().unwrap_or(0.0),
            device.axes.get(y).copied().unwrap_or(0.0),
        );
        for (on, bit) in [
            (y < -0.5, BUTTON_DPAD_UP),
            (y > 0.5, BUTTON_DPAD_DOWN),
            (x < -0.5, BUTTON_DPAD_LEFT),
            (x > 0.5, BUTTON_DPAD_RIGHT),
        ] {
            if on {
                r.buttons |= 1 << bit;
            }
        }
    }
    r
}

/// One controller's share of the canonical state after its options: twelve
/// half-axes (positive at 2a, negative at 2a+1) and the mapped buttons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mapped {
    pub half: [u16; NUM_AXES * 2],
    pub buttons: u32,
    /// The values after inversion, offset and dead zone, by physical axis
    /// (the Device Options tab).
    pub fixed: [i16; NUM_AXES],
}

/// onAxis / routeAxisValue and onButton, for every physical input in turn.
pub fn map_reading(reading: &Reading, options: &Options) -> Mapped {
    let mut m = Mapped::default();
    for phys in 0..NUM_AXES {
        let value = options.axes[phys].apply(reading.axes[phys]);
        m.fixed[phys] = value;
        let out = options.axis_map[phys];
        let magnitude = |v: i16| i32::from(v).unsigned_abs().min(u32::from(u16::MAX)) as u16;
        if out == AXIS_OUTPUT_NONE || out >= NUM_AXIS_OUTPUTS {
            continue;
        }
        if out == AXIS_OUTPUT_TRIGGER_PAIR {
            m.half[AXIS_RIGHT_TRIGGER * 2] = if value > 0 { value as u16 } else { 0 };
            m.half[AXIS_RIGHT_TRIGGER * 2 + 1] = 0;
            m.half[AXIS_LEFT_TRIGGER * 2] = if value < 0 { magnitude(value) } else { 0 };
            m.half[AXIS_LEFT_TRIGGER * 2 + 1] = 0;
            continue;
        }
        let out = usize::from(out);
        if phys >= AXIS_LEFT_TRIGGER && out < AXIS_LEFT_TRIGGER {
            // a trigger feeds one half of a stick axis: left the negative
            let slot = if phys == AXIS_LEFT_TRIGGER { out * 2 + 1 } else { out * 2 };
            m.half[slot] = magnitude(value);
            continue;
        }
        m.half[out * 2] = if value > 0 { value as u16 } else { 0 };
        m.half[out * 2 + 1] = if value > 0 { 0 } else { magnitude(value) };
    }
    for (phys, &to) in options.button_map.iter().enumerate() {
        if reading.buttons & (1 << phys) != 0 {
            let to = usize::from(to);
            if to < NUM_BUTTONS {
                m.buttons |= 1 << to;
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_zeroes_the_axes_at_rest() {
        let mut o = Options::default();
        let samples = [[300, 0, 5, -9000, 0, 0], [300, 0, 6, -9000, 0, 0], [300, 0, 5, -9000, 0, 0]];
        calibrate(&mut o, &samples);
        assert_eq!(
            (o.axes[0].offset, o.axes[0].dead_zone),
            (-300, 2048),
            "small rest: the default dead zone"
        );
        assert_eq!(
            (o.axes[3].offset, o.axes[3].dead_zone),
            (9000, 16384),
            "far rest: the largest dead zone"
        );
        assert_eq!(o.axes[2], AxisOptions::default(), "a moving axis keeps its options");
        assert_eq!(o.axes[1], AxisOptions::default(), "zero at rest stays the default");
    }

    #[test]
    fn a_saved_device_keeps_its_name() {
        let mut o = Options::default();
        o.axis_map[5] = AXIS_OUTPUT_TRIGGER_PAIR;
        assert_eq!(saved_name(&o.save("Pad, X", true)).as_deref(), Some("Pad X"));
        assert_eq!(saved_name("garbage"), None);
    }

    #[test]
    fn the_dead_zone_and_the_offset_follow_compute_modified_value() {
        let o = AxisOptions::default();
        assert_eq!(o.apply(2047), 0);
        assert_eq!(o.apply(2048), 2048, "no rescaling outside the dead zone");
        assert_eq!(o.apply(-32768), -32768);
        let inverted = AxisOptions {
            multiplier: -1,
            dead_zone: 0,
            offset: 100,
        };
        assert_eq!(inverted.apply(-32768), 32668);
        assert_eq!(inverted.apply(32767), -32768, "clamped after the offset");
    }

    #[test]
    fn options_round_trip_and_default_options_save_nothing() {
        let mut o = Options::default();
        assert_eq!(o.save("Pad", false), "");
        assert_eq!(o.save("Pad", true), "{name:Pad}");
        o.axes[1].multiplier = -1;
        o.axes[1].dead_zone = 4000;
        o.axes[4].offset = -12;
        o.axis_map[5] = AXIS_OUTPUT_TRIGGER_PAIR;
        o.button_map[0] = 1;
        o.button_map[1] = 0;
        let s = o.save("Pad, {X}", false);
        assert_eq!(
            s,
            "{name:Pad X,axis_options:{1:{invert:1,dead_zone:4000},4:{offset:-12}},axis_map:{5:7},button_map:{0:1,1:0}}"
        );
        assert_eq!(Options::load(&s), o);
    }

    #[test]
    fn a_malformed_string_keeps_every_default() {
        assert_eq!(Options::load("{name:Pad,axis_map:{0:3}"), Options::default());
        assert_eq!(Options::load("name:Pad"), Options::default());
        // one bad value keeps its own default only
        let o = Options::load("{name:Pad,axis_map:{0:8,1:2},button_map:{2:032}}");
        assert_eq!(o.axis_map[0], 0, "8 is out of range");
        assert_eq!(o.axis_map[1], 2);
        assert_eq!(o.button_map[2], 2, "not written back the same");
        let o = Options::load("{name:Pad,axis_options:{0:{dead_zone:16385,offset:-16384}}}");
        assert_eq!(o.axes[0].dead_zone, DEFAULT_AXIS_DEAD_ZONE);
        assert_eq!(o.axes[0].offset, -16384);
    }

    #[test]
    fn a_trigger_on_a_stick_axis_fills_one_half() {
        let mut o = Options::default();
        for a in &mut o.axes {
            a.dead_zone = 0;
        }
        o.axis_map[AXIS_LEFT_TRIGGER] = 2;
        o.axis_map[AXIS_RIGHT_TRIGGER] = 2;
        o.axis_map[2] = AXIS_OUTPUT_NONE;
        let r = Reading {
            axes: [0, 0, 9000, 0, 1000, 3000],
            buttons: 0b11,
        };
        let m = map_reading(&r, &o);
        assert_eq!((m.half[4], m.half[5]), (3000, 1000));
        assert_eq!(m.buttons, 0b11);
        o.axis_map[0] = AXIS_OUTPUT_TRIGGER_PAIR;
        let r = Reading {
            axes: [-5000, 0, 0, 0, 0, 0],
            buttons: 0,
        };
        let m = map_reading(&r, &o);
        assert_eq!((m.half[8], m.half[10]), (5000, 0));
    }

    #[test]
    fn sticks_and_triggers_scale_to_sdl_ranges() {
        assert_eq!(stick(-1.0), -32768);
        assert_eq!(stick(1.0), 32767);
        assert_eq!(stick(f32::NAN), 0);
        assert_eq!(trigger(0.5), 16384);
        assert_eq!(trigger(-0.2), 0);
    }
}
