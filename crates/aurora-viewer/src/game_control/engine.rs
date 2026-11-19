//! The controller state of a frame and its trip to the region. Port of
//! accumulateInternalState, computeFinalState, accumulateFinalState,
//! computeSemanticState, setExternalInput, scheduleImmediateResend,
//! updateResendPeriod and getMinSendPeriod
//! (indra/llwindow/llgamecontrol.cpp) and of sendGameControlData
//! (indra/newview/llappviewer.cpp), LL viewer 26.5, originally LGPL 2.1.
//!
//! The message leaves with no controller at all: the keyboard's movement
//! fills the canonical axes too (in the avatar's frame, as LL's viewer
//! does), and an idle state is repeated after 0.1 s, 0.4 s, 1.6 s… so that
//! an unreliable message still arrives. Never faster than the region's
//! frame rate (45 Hz at most).

use super::device::{self, Mapped, Options, Reading};
use super::mappings::{self, Action, Lookup};
use super::{Mode, NUM_AXES, NUM_BUTTONS, NUM_SEMANTIC_SLOTS, Settings};
use aurora_net::control;
use std::time::{Duration, Instant};

/// FIRST_RESEND_PERIOD, RESEND_EXPANSION_RATE.
const FIRST_RESEND: Duration = Duration::from_millis(100);
const RESEND_EXPANSION: u32 = 4;
/// DEFAULT_SERVER_FRAME_RATE (also the highest), MIN_SERVER_FRAME_RATE.
pub const DEFAULT_SERVER_FPS: f32 = 45.0;
const MIN_SERVER_FPS: f32 = 1.0;
/// MIN_RESEND_PERIOD: (U64)(1e6 / 45.f) µs.
const MIN_SEND_PERIOD: Duration = Duration::from_micros(22_222);

/// The state that goes out (LLGameControl::ServerState).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerState {
    pub axes: [i16; NUM_AXES],
    pub buttons: u32,
    pub semantic_axes: [i16; NUM_SEMANTIC_SLOTS],
    pub semantic_buttons: u32,
    pub mode: u8,
}

/// What the viewer hands the engine each frame.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub now: Instant,
    pub mode: Mode,
    /// The flycam flies under the cursor mode (setFlycamEngaged).
    pub flycam_engaged: bool,
    /// The agent's control flags before the controller adds its own.
    pub external_flags: u32,
    /// The keys bound to `game_control_button_N`.
    pub external_buttons: u32,
    /// The avatar runs (LLAgent::getRunning).
    pub running: bool,
    /// The cursor mode's pointer: pixels from the top left of the window,
    /// its size, the real mouse buttons.
    pub cursor: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub left: bool,
    pub right: bool,
}

/// One connected controller.
#[derive(Debug, Clone)]
pub struct Controller {
    pub id: String,
    pub name: String,
    pub reading: Reading,
    pub mapped: Mapped,
    pub options: Options,
    config: String,
}

/// A controller seen this session (getSessionStatsAsLLSD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDevice {
    pub id: String,
    pub name: String,
    pub used: bool,
    pub connected: bool,
}

/// GameControlData, packed (sendGameControlData): trailing zero axes left
/// out, the semantic axes limited to the mode's, an empty button mask not
/// sent.
pub use aurora_net::GameControlPacket as Packet;

#[derive(Debug)]
pub struct Engine {
    pub controllers: Vec<Controller>,
    pub session_devices: Vec<SessionDevice>,
    /// All controllers together (g_innerState): twelve half-axes and the
    /// buttons, with last frame's buttons for the press edges.
    pub inner_half: [u16; NUM_AXES * 2],
    pub inner_buttons: u32,
    pub inner_prev_buttons: u32,
    /// The keyboard's share (mExternalServerState, mExternalActionFlags).
    external_axes: [i16; NUM_AXES],
    external_buttons: u32,
    external_flags: u32,
    external_running: bool,
    /// The last computed state (g_finalState).
    pub state: ServerState,
    pub lookup: Lookup,
    pub mode: Mode,
    cursor: Cursor,
    /// Due once past (g_nextSend); `None` is LL's 0, due at once.
    next_send: Option<Instant>,
    resend_period: Duration,
    server_fps: f32,
    packet: u8,
    /// The state of the last message handed to the network.
    pub sent_state: Option<ServerState>,
    /// Messages handed to the network.
    pub sent: u64,
    pub last_sent: Option<Instant>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            controllers: Vec::new(),
            session_devices: Vec::new(),
            inner_half: [0; NUM_AXES * 2],
            inner_buttons: 0,
            inner_prev_buttons: 0,
            external_axes: [0; NUM_AXES],
            external_buttons: 0,
            external_flags: 0,
            external_running: false,
            state: ServerState::default(),
            lookup: Lookup::default(),
            mode: Mode::Avatar,
            cursor: Cursor::default(),
            next_send: None,
            resend_period: Duration::ZERO,
            server_fps: DEFAULT_SERVER_FPS,
            packet: 0,
            sent_state: None,
            sent: 0,
            last_sent: None,
        }
    }
}

impl Engine {
    /// A controller is connected (isEnabled).
    pub fn enabled(&self) -> bool {
        !self.controllers.is_empty()
    }

    /// The gamepads of the joystick worker's latest snapshot, read through
    /// their options. Unfocused, they read nothing (processEvents drops
    /// the events and clears the states).
    pub fn update_controllers(&mut self, devices: &[crate::joystick::Device], settings: &Settings, focused: bool) {
        let pads: Vec<&crate::joystick::Device> = devices.iter().filter(|d| d.is_gamepad).collect();
        self.controllers.retain(|c| pads.iter().any(|d| d.id == c.id));
        for d in &pads {
            let config = settings.devices.get(&d.id).cloned().unwrap_or_default();
            let index = match self.controllers.iter().position(|c| c.id == d.id) {
                Some(i) => i,
                None => {
                    self.controllers.push(Controller {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        reading: Reading::default(),
                        mapped: Mapped::default(),
                        options: Options::load(&config),
                        config: config.clone(),
                    });
                    self.controllers.len() - 1
                }
            };
            let c = &mut self.controllers[index];
            if c.config != config {
                c.options = Options::load(&config);
                c.config = config;
            }
            c.reading = if focused { device::reading(d) } else { Reading::default() };
            c.mapped = device::map_reading(&c.reading, &c.options);
        }
        for s in &mut self.session_devices {
            s.connected = self.controllers.iter().any(|c| c.id == s.id);
        }
        for c in &self.controllers {
            let used = c.reading.buttons != 0 || c.mapped.fixed.iter().any(|v| *v != 0);
            match self.session_devices.iter_mut().find(|s| s.id == c.id) {
                Some(s) => s.used |= used,
                None => self.session_devices.push(SessionDevice {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    used,
                    connected: true,
                }),
            }
        }
    }

    /// setServerFrameRate: from each SimStats' frame rate.
    pub fn set_server_fps(&mut self, fps: f32) {
        self.server_fps = if fps > 0.0 && fps.is_finite() {
            fps.clamp(MIN_SERVER_FPS, DEFAULT_SERVER_FPS)
        } else {
            DEFAULT_SERVER_FPS
        };
    }

    /// getMinSendPeriod.
    fn min_period(&self) -> Duration {
        Duration::from_micros((1_000_000.0 / self.server_fps) as u64).max(MIN_SEND_PERIOD)
    }

    /// scheduleImmediateResend: restart the back-off and bring the deadline
    /// to one period from now at the latest, never later.
    fn schedule_immediate(&mut self, now: Instant) {
        self.resend_period = Duration::ZERO;
        // LL's deadline starts at 0, before any candidate: it stays due
        let candidate = now + self.min_period();
        self.next_send = self.next_send.map(|t| t.min(candidate));
    }

    /// The frame's state, and whether a message is due
    /// (setExternalInput, then computeFinalStateAndCheckForChanges).
    pub fn compute(&mut self, settings: &Settings, f: &Frame) -> bool {
        self.mode = f.mode;
        self.lookup = Lookup::build(settings, f.mode, f.flycam_engaged);
        if let Some(c) = f.cursor {
            self.cursor = c;
        }
        self.set_external(f.external_flags, f.external_buttons, f.running);
        self.accumulate_inner();
        self.final_state(f.now);
        settings.send_to_server && self.next_send.is_none_or(|t| t < f.now)
    }

    /// setExternalInput: the movement flags as canonical axes (half
    /// deflection walking, full running), and as the buttons bound to
    /// movement actions in this mode.
    fn set_external(&mut self, flags: u32, buttons: u32, running: bool) {
        const AXIS_BITS: [(u32, u32, usize); 5] = [
            (control::LEFT_POS, control::LEFT_NEG, super::AXIS_LEFTX),
            (control::AT_POS, control::AT_NEG, super::AXIS_LEFTY),
            (control::YAW_POS, control::YAW_NEG, super::AXIS_RIGHTX),
            (control::PITCH_POS, control::PITCH_NEG, super::AXIS_RIGHTY),
            (control::UP_POS, control::UP_NEG, super::AXIS_LEFT_TRIGGER),
        ];
        self.external_flags = flags;
        self.external_running = running;
        let magnitude = key_magnitude(running);
        self.external_axes = [0; NUM_AXES];
        for (pos, neg, axis) in AXIS_BITS {
            let mut v = 0;
            if flags & pos != 0 {
                v += magnitude;
            }
            if flags & neg != 0 {
                v -= magnitude;
            }
            self.external_axes[axis] = v.clamp(-32768, 32767) as i16;
        }
        self.external_buttons = 0;
        for (button, action) in self.lookup.buttons.iter().enumerate() {
            if let Some((flag, _, _)) = action.and_then(Action::avatar_button)
                && flags & flag != 0
            {
                self.external_buttons |= 1 << button;
            }
        }
        self.external_buttons |= buttons;
    }

    /// accumulateInternalState: every controller together, the strongest
    /// half-axis of each and the union of the buttons.
    fn accumulate_inner(&mut self) {
        self.inner_prev_buttons = self.inner_buttons;
        self.inner_half = [0; NUM_AXES * 2];
        self.inner_buttons = 0;
        for c in &self.controllers {
            self.inner_buttons |= c.mapped.buttons;
            for (i, v) in c.mapped.half.iter().enumerate() {
                self.inner_half[i] = self.inner_half[i].max(*v);
            }
        }
    }

    /// computeFinalState and accumulateFinalState: controller and keyboard
    /// combined, axes summed, buttons together; any change asks for a send.
    fn final_state(&mut self, now: Instant) {
        let buttons = self.inner_buttons | self.external_buttons;
        if buttons != self.state.buttons {
            self.state.buttons = buttons;
            self.schedule_immediate(now);
        }
        for j in 0..NUM_AXES {
            let controller = (i32::from(self.inner_half[2 * j]) - i32::from(self.inner_half[2 * j + 1])).clamp(-32768, 32767);
            let axis = (controller + i32::from(self.external_axes[j])).clamp(-32768, 32767) as i16;
            if self.state.axes[j] != axis {
                self.state.axes[j] = axis;
                self.schedule_immediate(now);
            }
        }
        self.semantic_state(now);
    }

    fn half(&self, axis: usize) -> (u16, u16) {
        (self.inner_half[axis * 2], self.inner_half[axis * 2 + 1])
    }

    /// computeSemanticState.
    fn semantic_state(&mut self, now: Instant) {
        let mode = self.mode;
        let new_mode = mode as u8;
        if self.state.mode != new_mode {
            self.state.mode = new_mode;
            self.schedule_immediate(now);
        }
        let mut values = [0i32; NUM_SEMANTIC_SLOTS];
        let keep = |values: &mut [i32; NUM_SEMANTIC_SLOTS], slot: usize, value: i32| {
            if value.abs() > values[slot].abs() {
                values[slot] = value;
            }
        };
        match mode {
            Mode::Avatar | Mode::Mouselook | Mode::Captive | Mode::Cursor => {
                let cursor = mode == Mode::Cursor;
                for axis in 0..NUM_AXES {
                    let Some(b) = self.lookup.axes[axis] else { continue };
                    let slot = if cursor {
                        match b.action {
                            Action::MouseX => 0,
                            Action::MouseY => 1,
                            _ => continue,
                        }
                    } else {
                        match b.action.avatar_axis() {
                            Some((_, _, slot)) => slot,
                            None => continue,
                        }
                    };
                    let (p, n) = self.half(axis);
                    keep(&mut values, slot, b.value(p, n));
                }
                if !cursor {
                    for btn in 0..NUM_BUTTONS {
                        if self.inner_buttons & (1 << btn) == 0 {
                            continue;
                        }
                        if let Some((_, slot, sign)) = self.lookup.buttons[btn].and_then(Action::avatar_button) {
                            keep(&mut values, slot, sign * 32767);
                        }
                    }
                    const KEY_AXIS_BITS: [(u32, u32, usize); 5] = [
                        (control::YAW_POS, control::YAW_NEG, 2),
                        (control::PITCH_POS, control::PITCH_NEG, 3),
                        (control::UP_POS, control::UP_NEG, 4),
                        (control::LEFT_POS, control::LEFT_NEG, 0),
                        (control::AT_POS, control::AT_NEG, 1),
                    ];
                    let magnitude = key_magnitude(self.external_running);
                    for (pos, neg, slot) in KEY_AXIS_BITS {
                        let mut v = 0;
                        if self.external_flags & pos != 0 {
                            v += magnitude;
                        }
                        if self.external_flags & neg != 0 {
                            v -= magnitude;
                        }
                        values[slot] += v;
                    }
                } else {
                    let c = self.cursor;
                    let px = c.x.clamp(0, c.width.max(0));
                    let py = c.y.clamp(0, c.height.max(0));
                    values[2] = px;
                    values[3] = py;
                    let normal = |p: i32, size: i32| {
                        if size > 0 {
                            ((i64::from(p) * 32767 + i64::from(size / 2)) / i64::from(size)).clamp(0, 32767) as i32
                        } else {
                            0
                        }
                    };
                    values[4] = normal(px, c.width);
                    values[5] = normal(py, c.height);
                }
            }
            Mode::Flycam => {
                for axis in 0..NUM_AXES {
                    let Some(b) = self.lookup.axes[axis] else { continue };
                    let Some(slot) = b.action.flycam_axis() else { continue };
                    let (p, n) = self.half(axis);
                    keep(&mut values, slot, b.value(p, n));
                }
                for btn in 0..NUM_BUTTONS {
                    if self.inner_buttons & (1 << btn) == 0 {
                        continue;
                    }
                    if let Some((slot, sign)) = self.lookup.buttons[btn].and_then(Action::flycam_button) {
                        keep(&mut values, slot, (sign * 32767.0) as i32);
                    }
                }
            }
            Mode::None => {}
        }
        let combined = self.inner_buttons | self.external_buttons;
        let mut bits = 0u32;
        for (btn, action) in self.lookup.buttons.iter().enumerate() {
            if combined & (1 << btn) == 0 {
                continue;
            }
            if let Some(slot) = action.and_then(|a| mappings::default_button_slot(mode, a)) {
                bits |= 1 << slot;
            }
        }
        if mode == Mode::Cursor {
            for (down, action) in [
                (self.cursor.left, Action::MouseClickLeft),
                (self.cursor.right, Action::MouseClickRight),
            ] {
                if down && let Some(slot) = mappings::default_button_slot(mode, action) {
                    bits |= 1 << slot;
                }
            }
        }
        if self.state.semantic_buttons != bits {
            self.state.semantic_buttons = bits;
            self.schedule_immediate(now);
        }
        for (slot, v) in values.iter().enumerate() {
            let v = (*v).clamp(-32768, 32767) as i16;
            if self.state.semantic_axes[slot] != v {
                self.state.semantic_axes[slot] = v;
                self.schedule_immediate(now);
            }
        }
    }

    /// The message of a due frame (sendGameControlData): the next packet
    /// number, the packing, then updateResendPeriod.
    pub fn packet(&mut self, now: Instant) -> Packet {
        let s = self.state;
        let packet = self.packet;
        self.packet = self.packet.wrapping_add(1);
        let mut n = NUM_AXES;
        while n > 0 && s.axes[n - 1] == 0 {
            n -= 1;
        }
        let mode = mode_of(s.mode);
        let mut m = mode.semantic_axes();
        while m > 0 && s.semantic_axes[m - 1] == 0 {
            m -= 1;
        }
        self.update_resend_period(now);
        self.sent_state = Some(s);
        self.sent += 1;
        self.last_sent = Some(now);
        Packet {
            packet,
            action_mode: s.mode,
            axes: s.axes[..n].to_vec(),
            buttons: (s.buttons != 0).then_some(s.buttons),
            mode_axes: s.semantic_axes[..m].to_vec(),
            mode_buttons: (s.semantic_buttons != 0).then_some(s.semantic_buttons),
        }
    }

    /// updateResendPeriod: 100 ms (or the region's frame) after a change,
    /// then four times longer each time, with no upper bound.
    fn update_resend_period(&mut self, now: Instant) {
        let floor = self.min_period();
        self.resend_period = if self.resend_period.is_zero() {
            FIRST_RESEND.max(floor)
        } else {
            self.resend_period.saturating_mul(RESEND_EXPANSION)
        };
        // LL's U64 grows without bound; ten years is as good as never, and
        // keeps the Instant in range
        let wait = self.resend_period.max(floor).min(Duration::from_secs(10 * 365 * 86_400));
        self.next_send = Some(now.checked_add(wait).unwrap_or(now));
    }

    /// The input pressed or pushed past half way (getActiveInputChannel),
    /// for the binding capture of the preferences: a button first, else
    /// the first axis with its sign.
    pub fn active_input(&self) -> Option<ActiveInput> {
        let s = &self.state;
        if s.buttons > 0 {
            return Some(ActiveInput::Button(s.buttons.trailing_zeros() as usize));
        }
        let threshold = i32::from(i16::MAX / 2);
        (0..NUM_AXES)
            .find(|&i| i32::from(s.axes[i]).abs() > threshold)
            .map(|i| ActiveInput::Axis(i, if s.axes[i] > 0 { 1 } else { -1 }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveInput {
    Axis(usize, i32),
    Button(usize),
}

/// A key's deflection: half walking, full running.
fn key_magnitude(running: bool) -> i32 {
    if running { i32::from(i16::MAX) } else { i32::from(i16::MAX / 2) }
}

fn mode_of(n: u8) -> Mode {
    match n {
        0 => Mode::Avatar,
        1 => Mode::Mouselook,
        2 => Mode::Flycam,
        3 => Mode::Captive,
        4 => Mode::Cursor,
        _ => Mode::None,
    }
}
