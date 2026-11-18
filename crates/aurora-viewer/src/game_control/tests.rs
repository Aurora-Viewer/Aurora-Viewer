//! The engine against the LL viewer 26.5's rules (llgamecontrol.cpp,
//! sendGameControlData).

use super::actions::{self, RunLatch};
use super::device::Reading;
use super::engine::{Cursor, Engine, Frame, Packet};
use super::*;
use aurora_net::control;
use std::time::{Duration, Instant};

fn frame(now: Instant, mode: Mode, flags: u32) -> Frame {
    Frame {
        now,
        mode,
        flycam_engaged: false,
        external_flags: flags,
        external_buttons: 0,
        running: false,
        cursor: None,
    }
}

/// A gamepad of the joystick worker with these SDL readings.
fn pad(reading: Reading) -> crate::joystick::Device {
    use crate::joystick::{GamepadButton as B, GamepadStyle};
    let labels = vec![
        B::South,
        B::East,
        B::West,
        B::North,
        B::Select,
        B::Mode,
        B::Start,
        B::LeftStick,
        B::RightStick,
        B::LeftBumper,
        B::RightBumper,
        B::DpadUp,
        B::DpadDown,
        B::DpadLeft,
        B::DpadRight,
    ];
    let mut d = crate::joystick::Device::gamepad(
        "pad".into(),
        "Test pad".into(),
        6,
        [[Some(0), Some(1)], [Some(2), Some(3)]],
        [Some(4), Some(5)],
        GamepadStyle::Xbox,
        labels,
    );
    for i in 0..4 {
        d.axes[i] = if reading.axes[i] < 0 {
            f32::from(reading.axes[i]) / 32768.0
        } else {
            f32::from(reading.axes[i]) / 32767.0
        };
    }
    d.axes[4] = f32::from(reading.axes[4]) / 32767.0;
    d.axes[5] = f32::from(reading.axes[5]) / 32767.0;
    for b in 0..15 {
        d.button(b, reading.buttons & (1 << b) != 0);
    }
    d
}

fn with_pad(e: &mut Engine, s: &Settings, axes: [i16; 6], buttons: u32) {
    e.update_controllers(&[pad(Reading { axes, buttons })], s, true);
}

#[test]
fn the_first_frame_sends_at_once_even_without_a_controller() {
    let s = Settings::default();
    let mut e = Engine::default();
    let t = Instant::now();
    assert!(e.compute(&s, &frame(t, Mode::Avatar, 0)));
    let p = e.packet(t);
    assert_eq!(
        p,
        Packet {
            packet: 0,
            action_mode: 0,
            axes: vec![],
            buttons: None,
            mode_axes: vec![],
            mode_buttons: None,
        }
    );
    // the idle state is repeated after 100 ms, 400 ms, 1.6 s…
    let mut at = t;
    for wait in [100u64, 400, 1600, 6400] {
        assert!(
            !e.compute(&s, &frame(at + Duration::from_millis(wait), Mode::Avatar, 0)),
            "not yet at {wait} ms"
        );
        at += Duration::from_millis(wait) + Duration::from_micros(1);
        assert!(e.compute(&s, &frame(at, Mode::Avatar, 0)), "due after {wait} ms");
        e.packet(at);
    }
    assert_eq!(e.packet(at).packet, 5, "the packet number counts every message");
}

#[test]
fn sending_off_never_sends() {
    let s = Settings {
        send_to_server: false,
        ..Settings::default()
    };
    let mut e = Engine::default();
    assert!(!e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0)));
}

#[test]
fn a_change_goes_out_within_one_region_frame_and_restarts_the_back_off() {
    let s = Settings::default();
    let mut e = Engine::default();
    let t = Instant::now();
    e.compute(&s, &frame(t, Mode::Avatar, 0));
    e.packet(t);
    // a key 10 ms later: due 22.222 ms after it, not before
    let t1 = t + Duration::from_millis(10);
    assert!(!e.compute(&s, &frame(t1, Mode::Avatar, control::AT_POS)));
    assert!(!e.compute(&s, &frame(t1 + Duration::from_millis(22), Mode::Avatar, control::AT_POS)));
    let t2 = t1 + Duration::from_micros(22_223);
    assert!(e.compute(&s, &frame(t2, Mode::Avatar, control::AT_POS)));
    e.packet(t2);
    // the back-off restarted: 100 ms again
    assert!(e.compute(&s, &frame(t2 + Duration::from_micros(100_001), Mode::Avatar, control::AT_POS)));
}

#[test]
fn a_slow_region_slows_the_messages_down() {
    let s = Settings::default();
    let mut e = Engine::default();
    e.set_server_fps(5.0);
    let t = Instant::now();
    e.compute(&s, &frame(t, Mode::Avatar, 0));
    e.packet(t);
    // first resend max(100 ms, 200 ms)
    assert!(!e.compute(&s, &frame(t + Duration::from_millis(150), Mode::Avatar, 0)));
    assert!(e.compute(&s, &frame(t + Duration::from_millis(201), Mode::Avatar, 0)));
    e.set_server_fps(0.0);
    e.set_server_fps(500.0);
}

#[test]
fn keyboard_movement_uses_the_avatar_frame_and_the_walk_or_run_deflection() {
    let s = Settings::default();
    let mut e = Engine::default();
    let t = Instant::now();
    e.compute(&s, &frame(t, Mode::Avatar, control::AT_POS | control::LEFT_NEG | control::UP_NEG));
    assert_eq!(e.state.axes, [-16383, 16383, 0, 0, -16383, 0]);
    // the movement buttons bound in this mode light too
    assert_eq!(
        e.state.buttons,
        (1 << BUTTON_DPAD_UP) | (1 << BUTTON_DPAD_RIGHT) | (1 << BUTTON_EAST)
    );
    assert_eq!(e.state.semantic_axes[..5], [-16383, 16383, 0, 0, -16383]);
    assert_eq!(
        e.state.semantic_buttons,
        (1 << BUTTON_DPAD_UP) | (1 << BUTTON_DPAD_RIGHT) | (1 << BUTTON_EAST)
    );
    let mut f = frame(t, Mode::Avatar, control::AT_POS);
    f.running = true;
    e.compute(&s, &f);
    assert_eq!(e.state.axes[1], 32767);
    // nudge bits are not translated: a tap sends nothing
    e.compute(&s, &frame(t, Mode::Avatar, control::NUDGE_AT_POS));
    assert_eq!(e.state.axes, [0; 6]);
}

#[test]
fn the_stick_and_the_keyboard_add_up_on_a_canonical_axis() {
    let s = Settings::default();
    let mut e = Engine::default();
    with_pad(&mut e, &s, [0, -20000, 0, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, control::AT_POS));
    // stick forward is negative, the W key positive
    assert_eq!(e.state.axes[1], -20000 + 16383);
    // the semantic advance: the inverted stick (+20000), plus the key
    assert_eq!(e.state.semantic_axes[1], 20000 + 16383);
}

#[test]
fn the_semantic_axes_follow_the_default_mapping() {
    let s = Settings::default();
    let mut e = Engine::default();
    with_pad(&mut e, &s, [-10000, -20000, 8000, 6000, 3000, 9000], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    // strafe = -LEFTX, advance = -LEFTY, turn = -RIGHTX, look = RIGHTY,
    // rise = the stronger trigger, right +
    assert_eq!(e.state.semantic_axes[..5], [10000, 20000, -8000, 6000, 9000]);
    assert_eq!(e.state.axes, [-10000, -20000, 8000, 6000, 3000, 9000]);
    let p = e.packet(Instant::now());
    assert_eq!(p.mode_axes.len(), 5);
    // D-pad and face buttons fold into the axes at full deflection
    with_pad(&mut e, &s, [0; 6], (1 << BUTTON_DPAD_LEFT) | (1 << BUTTON_SOUTH));
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(e.state.semantic_axes[..5], [32767, 0, 0, 0, 32767]);
    assert_eq!(e.state.semantic_buttons, (1 << BUTTON_DPAD_LEFT) | (1 << BUTTON_SOUTH));
}

#[test]
fn a_remapped_action_keeps_its_default_bit() {
    let mut s = Settings::default();
    s.set_binding(Mode::Avatar, false, "Jump", "BUTTON_NORTH");
    let mut e = Engine::default();
    with_pad(&mut e, &s, [0; 6], 1 << BUTTON_NORTH);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(e.state.buttons, 1 << BUTTON_NORTH, "the canonical button is the pressed one");
    assert_eq!(e.state.semantic_buttons, 1 << BUTTON_SOUTH, "the semantic bit is Jump's");
    // an action with no default button sets no bit
    s.set_binding(Mode::Avatar, false, "Fly up", "BUTTON_WEST");
    with_pad(&mut e, &s, [0; 6], 1 << BUTTON_WEST);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(e.state.semantic_buttons, 0);
    assert_eq!(e.state.semantic_axes[4], 32767);
}

#[test]
fn the_flycam_takes_no_keyboard_and_packs_seven_axes() {
    let s = Settings::default();
    let mut e = Engine::default();
    with_pad(&mut e, &s, [0, 0, 0, 0, 0, 0], 1 << BUTTON_NORTH);
    let t = Instant::now();
    e.compute(&s, &frame(t, Mode::Flycam, control::AT_POS));
    assert_eq!(e.state.semantic_axes, [0, 0, 0, 0, 0, 0, 32767], "zoom in");
    assert_eq!(e.state.semantic_buttons, 1 << BUTTON_NORTH);
    let p = e.packet(t);
    assert_eq!(p.action_mode, 2);
    assert_eq!(p.mode_axes.len(), 7);
    assert_eq!(p.axes, vec![0, 16383], "the canonical axes keep the keyboard");
}

#[test]
fn the_cursor_mode_carries_the_pointer() {
    let s = Settings::default();
    let mut e = Engine::default();
    let mut f = frame(Instant::now(), Mode::Cursor, control::AT_POS);
    f.cursor = Some(Cursor {
        x: 400,
        y: 300,
        width: 800,
        height: 600,
        left: true,
        right: false,
    });
    with_pad(&mut e, &s, [5000, 0, 0, 0, 0, 0], 0);
    e.compute(&s, &f);
    assert_eq!(e.state.semantic_axes[..6], [5000, 0, 400, 300, 16384, 16384]);
    assert_eq!(
        e.state.semantic_buttons & (1 << BUTTON_LEFT_SHOULDER),
        1 << BUTTON_LEFT_SHOULDER,
        "the real click"
    );
    assert_ne!(e.state.semantic_buttons & (1 << BUTTON_DPAD_UP), 0, "the W key's movement button");
}

#[test]
fn a_mode_change_is_sent_at_once() {
    let s = Settings::default();
    let mut e = Engine::default();
    let t = Instant::now();
    e.compute(&s, &frame(t, Mode::Avatar, 0));
    e.packet(t);
    let t1 = t + Duration::from_millis(30);
    assert!(e.compute(&s, &frame(t1, Mode::Mouselook, 0)));
}

#[test]
fn the_stick_moves_the_avatar_past_an_eighth_and_runs_past_ninety_percent() {
    let s = Settings::default();
    let mut e = Engine::default();
    let mut run = RunLatch::default();
    with_pad(&mut e, &s, [0, -4000, 0, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(actions::agent_actions(&e, &mut run).control_flags, 0);
    with_pad(&mut e, &s, [0, -31000, 12000, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    let a = actions::agent_actions(&e, &mut run);
    assert_eq!(a.control_flags, control::AT_POS | control::YAW_NEG);
    assert!(a.running);
    assert!((a.yaw + 12000.0 / 32767.0).abs() < 1e-6);
    // the run holds down to 70 %
    with_pad(&mut e, &s, [0, -24000, 0, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert!(actions::agent_actions(&e, &mut run).running);
    with_pad(&mut e, &s, [0, -20000, 0, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert!(!actions::agent_actions(&e, &mut run).running);
}

#[test]
fn a_toggle_fires_once_per_press() {
    let s = Settings::default();
    let mut e = Engine::default();
    let mut run = RunLatch::default();
    let t = Instant::now();
    with_pad(&mut e, &s, [0; 6], 1 << BUTTON_START);
    e.compute(&s, &frame(t, Mode::Avatar, 0));
    assert_eq!(actions::agent_actions(&e, &mut run).misc, actions::TOGGLE_MOUSELOOK);
    e.compute(&s, &frame(t, Mode::Avatar, 0));
    assert_eq!(actions::agent_actions(&e, &mut run).misc, 0, "held, not pressed again");
}

#[test]
fn flycam_inputs_sum_axes_and_buttons() {
    let s = Settings::default();
    let mut e = Engine::default();
    with_pad(
        &mut e,
        &s,
        [-16384, 0, 0, 0, 0, 32767],
        (1 << BUTTON_SOUTH) | (1 << BUTTON_RIGHT_SHOULDER),
    );
    e.compute(&s, &frame(Instant::now(), Mode::Flycam, 0));
    let f = actions::flycam_inputs(&e, true);
    assert!((f.channels[0] - 16384.0 / 32767.0).abs() < 1e-4, "truck: inverted left x");
    assert!((f.channels[4] - 1.0).abs() < 1e-6, "boom up: right trigger");
    assert!((f.channels[6] + 1.0).abs() < 1e-6, "zoom out");
    assert_eq!(f.modifiers, actions::FLYCAM_ORBIT);
    assert_eq!(actions::flycam_inputs(&e, false), actions::FlycamInputs::default());
}

#[test]
fn an_unfocused_viewer_reads_no_controller() {
    let s = Settings::default();
    let mut e = Engine::default();
    e.update_controllers(
        &[pad(Reading {
            axes: [0, -30000, 0, 0, 0, 0],
            buttons: 1,
        })],
        &s,
        false,
    );
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(e.state.axes, [0; 6]);
    assert_eq!(e.state.buttons, 0);
    assert!(e.enabled(), "still connected");
}

#[test]
fn the_active_input_is_a_button_first_then_an_axis_past_half() {
    let s = Settings::default();
    let mut e = Engine::default();
    with_pad(&mut e, &s, [0, 20000, 0, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(e.active_input(), Some(engine::ActiveInput::Axis(1, 1)));
    with_pad(&mut e, &s, [0, 20000, 0, 0, 0, 0], 1 << 3);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert_eq!(e.active_input(), Some(engine::ActiveInput::Button(3)));
}

#[test]
fn settings_round_trip_and_stay_small_by_default() {
    let s = Settings::default();
    let json = serde_json::to_string(&s).expect("serialize");
    assert!(!json.contains("axes"), "the defaults are not written out: {json}");
    let mut changed = s.clone();
    changed.set_axis_invert(Mode::Flycam, "Tilt up/down", true);
    changed.flycam.speed_factor = 7.0;
    changed.sanitize();
    assert_eq!(changed.flycam.speed_factor, SPEED_FACTOR_MAX);
    let back: Settings = serde_json::from_str(&serde_json::to_string(&changed).expect("serialize")).expect("parse");
    assert_eq!(back, changed);
    assert!(back.axis_invert(Mode::Flycam, "Tilt up/down"));
    assert!(back.axis_invert(Mode::Flycam, "Pan left/right"), "the other defaults stay");
}

#[test]
fn binding_an_input_frees_it_from_the_other_actions_of_the_mode() {
    let mut s = Settings::default();
    s.bind_exclusive(Mode::Avatar, false, "Crouch", "BUTTON_SOUTH");
    let b = s.buttons(Mode::Avatar);
    assert_eq!(b["Crouch"], "BUTTON_SOUTH");
    assert_eq!(b["Jump"], BUTTON_NONE, "South drives one action");
    assert_eq!(s.buttons(Mode::Mouselook)["Jump"], "BUTTON_SOUTH", "the other modes keep theirs");
    // clearing an action frees nothing else
    s.bind_exclusive(Mode::Avatar, true, "Turn left/right", AXIS_NONE);
    let a = s.axes(Mode::Avatar);
    assert_eq!(a["Turn left/right"], AXIS_NONE);
    assert_eq!(a["Zoom +/-"], AXIS_NONE);
    assert_eq!(a["Strafe left/right"], "AXIS_LEFTX");
}

#[test]
fn restore_defaults_resets_the_mode_but_not_whether_it_is_on() {
    let mut s = Settings::default();
    s.bind_exclusive(Mode::Flycam, false, "Orbit", "BUTTON_SOUTH");
    s.bind_exclusive(Mode::Avatar, false, "Jump", "BUTTON_NORTH");
    s.flycam.speed_factor = 2.0;
    s.flycam.enabled = false;
    s.flycam_allow_roll = true;
    s.flycam_crosshair = true;
    s.flycam_hide_cursor = false;
    s.restore_mode_defaults(Mode::Flycam);
    assert_eq!(
        s.flycam,
        ModeMapping {
            enabled: false,
            ..Default::default()
        }
    );
    assert!(!s.flycam_allow_roll && !s.flycam_crosshair && s.flycam_hide_cursor);
    assert_eq!(s.buttons(Mode::Avatar)["Jump"], "BUTTON_NORTH", "the other modes keep theirs");
}

#[test]
fn every_default_binding_has_its_row_in_the_preferences() {
    for mode in Mode::ALL {
        let axes: Vec<&str> = mappings::axis_choices(mode).iter().map(|a| a.label()).collect();
        for (label, input) in mappings::default_axes(mode) {
            assert!(input == AXIS_NONE || axes.contains(&label.as_str()), "{mode:?} axis {label}");
        }
        let buttons: Vec<&str> = mappings::button_choices(mode).iter().map(|a| a.label()).collect();
        for (label, input) in mappings::default_buttons(mode) {
            assert!(input == BUTTON_NONE || buttons.contains(&label.as_str()), "{mode:?} button {label}");
        }
    }
}

#[test]
fn cursor_up_moves_the_pointer_up_the_screen() {
    let mut s = Settings::default();
    s.bind_exclusive(Mode::Cursor, false, "Cursor up", "BUTTON_DPAD_UP");
    s.bind_exclusive(Mode::Cursor, false, "Cursor right", "BUTTON_DPAD_RIGHT");
    let mut e = Engine::default();
    let mut run = RunLatch::default();
    with_pad(&mut e, &s, [0; 6], (1 << BUTTON_DPAD_UP) | (1 << BUTTON_DPAD_RIGHT));
    e.compute(&s, &frame(Instant::now(), Mode::Cursor, 0));
    let a = actions::agent_actions(&e, &mut run);
    assert_eq!((a.mouse_dx, a.mouse_dy), (1.0, -1.0), "screen y grows downwards");
    assert_eq!(a.control_flags, 0, "the D-pad no longer walks");
}

#[test]
fn only_a_stick_updates_the_run() {
    let s = Settings::default();
    let mut e = Engine::default();
    let mut run = RunLatch::default();
    with_pad(&mut e, &s, [0; 6], 1 << BUTTON_DPAD_UP);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert!(!actions::agent_actions(&e, &mut run).analog_movement, "a D-pad walk");
    with_pad(&mut e, &s, [0, -20000, 0, 0, 0, 0], 0);
    e.compute(&s, &frame(Instant::now(), Mode::Avatar, 0));
    assert!(actions::agent_actions(&e, &mut run).analog_movement);
}
