//! Preferences › Game Controls: what a game controller's inputs do in each
//! mode, how a controller's own sticks and buttons map to the standard
//! ones, each axis's tuning, and the data the region receives. Port of
//! LLPanelPreferenceGameControl (indra/newview/llpanelpreferencegamecontrol.cpp
//! and panel_preferences_game_control.xml), LL viewer 26.5, originally
//! LGPL 2.1.
//!
//! Every edit applies at once, as in LL's panel. The inputs are named in
//! text: the interface draws Tabler icons only, not LL's controller glyph
//! font. A binding is chosen from its list, or captured: the record button
//! waits for the next button pressed (or axis pushed past half), any of the
//! 32 buttons (LL's capture cannot take Misc1 nor buttons 21 to 31).

use super::super::cx::{Custom, Cx, F, Nothing, field};
use crate::game_control::{
    self as gc, AXIS_NAMES, AXIS_NONE, AXIS_TRIGGERS, BUTTON_NONE, DeviceView, Mode, NUM_AXES, NUM_BUTTONS,
    device::{self, AXIS_OUTPUT_NONE, AXIS_OUTPUT_TRIGGER_PAIR, MAX_AXIS_DEAD_ZONE, MAX_AXIS_OFFSET, Options},
    engine::ActiveInput,
    mappings::{self, Action},
};
use crate::settings::Settings;
use crate::ui::widgets;
use aurora_i18n::tr;

/// A binding waiting for the controller's next input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub mode: Mode,
    pub axis: bool,
    pub action: Action,
}

/// The device options being calibrated (Auto calibrate): its id and the
/// axes read so far, sampled by the app each frame.
#[derive(Debug, Clone, Default)]
pub struct Calibration {
    pub device: String,
    pub samples: Vec<[i16; NUM_AXES]>,
}

pub fn tabs() -> Vec<&'static str> {
    vec![
        tr!("game-control-tab-actions"),
        tr!("game-control-tab-device-mappings"),
        tr!("game-control-tab-device-options"),
        tr!("game-control-tab-server-data"),
    ]
}

pub fn page(cx: &mut Cx, tab: usize) {
    match tab {
        0 => actions_tab(cx),
        1 => device_mappings_tab(cx),
        2 => device_options_tab(cx),
        _ => server_tab(cx),
    }
}

fn mode_title(mode: Mode) -> &'static str {
    match mode {
        Mode::Avatar => tr!("game-control-mode-avatar"),
        Mode::Mouselook => tr!("game-control-mode-mouselook"),
        Mode::Flycam => tr!("game-control-mode-flycam"),
        Mode::Captive => tr!("game-control-mode-captive"),
        Mode::Cursor => tr!("game-control-mode-cursor"),
        Mode::None => tr!("common-none"),
    }
}

/// The active mode's name (the panel's current_mode_indicator).
fn active_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Avatar => tr!("game-control-active-avatar"),
        Mode::Mouselook => tr!("common-mouselook"),
        Mode::Flycam => tr!("game-control-active-flycam"),
        Mode::Captive => tr!("game-control-active-captive"),
        Mode::Cursor => tr!("game-control-active-cursor"),
        Mode::None => tr!("common-none"),
    }
}

/// An action as the preferences name it in a mode.
fn action_name(mode: Mode, action: Action) -> &'static str {
    use Action::*;
    match action {
        StrafeAxis => tr!("game-control-action-strafe-axis"),
        AdvanceAxis => tr!("game-control-action-advance-axis"),
        TurnAxis => tr!("game-control-action-turn-axis"),
        LookAxis => tr!("game-control-action-look-axis"),
        FlyAxis => tr!("game-control-action-fly-axis"),
        ZoomAxis | FlycamZoomAxis => tr!("game-control-action-zoom-axis"),
        ScrollAxis => tr!("game-control-action-scroll-axis"),
        MouseX => tr!("game-control-action-mouse-x"),
        MouseY => tr!("game-control-action-mouse-y"),
        TruckAxis => tr!("game-control-action-truck-axis"),
        DollyAxis => tr!("game-control-action-dolly-axis"),
        PanAxis => tr!("game-control-action-pan-axis"),
        TiltAxis => tr!("game-control-action-tilt-axis"),
        BoomAxis => tr!("game-control-action-boom-axis"),
        RollAxis => tr!("game-control-action-roll-axis"),
        Jump => tr!("game-control-action-jump"),
        Crouch => tr!("game-control-action-crouch"),
        ToggleSit if mode == Mode::Captive => tr!("game-control-action-stand"),
        ToggleSit => tr!("game-control-action-sit"),
        ToggleSpeak => tr!("game-control-action-toggle-speak"),
        ToggleMouselook => tr!("game-control-action-toggle-mouselook"),
        ToggleFlycam if mode == Mode::Flycam => tr!("game-control-action-leave-flycam"),
        ToggleFlycam => tr!("game-control-action-flycam"),
        ToggleMouseCursor if mode == Mode::Cursor => tr!("game-control-action-toggle-mouse-cursor"),
        ToggleMouseCursor => tr!("game-control-action-mouse-cursor"),
        MouseClickLeft => tr!("game-control-action-mouse-left"),
        MouseClickRight => tr!("game-control-action-mouse-right"),
        AdvanceForward => tr!("game-control-action-advance-forward"),
        AdvanceBack => tr!("game-control-action-retreat"),
        StrafeLeft => tr!("game-control-action-strafe-left"),
        StrafeRight => tr!("game-control-action-strafe-right"),
        FlyUp => tr!("game-control-action-fly-up"),
        FlyDown => tr!("game-control-action-fly-down"),
        TurnLeft => tr!("game-control-action-turn-left"),
        TurnRight => tr!("game-control-action-turn-right"),
        ZoomIn | FlycamZoomIn => tr!("game-control-action-zoom-in"),
        ZoomOut | FlycamZoomOut => tr!("game-control-action-zoom-out"),
        ScrollIn => tr!("game-control-action-scroll-in"),
        ScrollOut => tr!("game-control-action-scroll-out"),
        CursorUp => tr!("game-control-action-cursor-up"),
        CursorDown => tr!("game-control-action-cursor-down"),
        CursorLeft => tr!("game-control-action-cursor-left"),
        CursorRight => tr!("game-control-action-cursor-right"),
        PanLeft => tr!("game-control-action-pan-left"),
        PanRight => tr!("game-control-action-pan-right"),
        Unroll => tr!("game-control-action-unroll"),
        RollCcw => tr!("game-control-action-roll-ccw"),
        RollCw => tr!("game-control-action-roll-cw"),
        Orbit => tr!("game-control-action-orbit"),
        DollyForward => tr!("game-control-action-dolly-forward"),
        DollyBack => tr!("game-control-action-dolly-back"),
        TruckLeft => tr!("game-control-action-truck-left"),
        TruckRight => tr!("game-control-action-truck-right"),
        BoomUp => tr!("game-control-action-boom-up"),
        BoomDown => tr!("game-control-action-boom-down"),
        Escape => tr!("game-control-action-escape"),
        ToggleFly | LookUp | LookDown | TiltUp | TiltDown | Interact | Other => action.label(),
    }
}

/// An axis input, by its index in the outputs list (axis_output_selector):
/// the six axes, then both triggers, then none.
fn axis_input_name(index: usize) -> &'static str {
    match index {
        0 => tr!("game-control-input-left-x"),
        1 => tr!("game-control-input-left-y"),
        2 => tr!("game-control-input-right-x"),
        3 => tr!("game-control-input-right-y"),
        4 => tr!("game-control-input-left-trigger"),
        5 => tr!("game-control-input-right-trigger"),
        6 => tr!("game-control-input-triggers"),
        _ => tr!("common-none"),
    }
}

/// A canonical button (button_input_selector).
fn button_input_name(index: usize) -> String {
    let name = match index {
        0 => tr!("game-control-button-south"),
        1 => tr!("game-control-button-east"),
        2 => tr!("game-control-button-west"),
        3 => tr!("game-control-button-north"),
        4 => tr!("game-control-button-select"),
        5 => tr!("game-control-button-home"),
        6 => tr!("game-control-button-start"),
        7 => tr!("game-control-button-left-stick"),
        8 => tr!("game-control-button-right-stick"),
        9 => tr!("game-control-button-left-shoulder"),
        10 => tr!("game-control-button-right-shoulder"),
        11 => tr!("game-control-button-dpad-up"),
        12 => tr!("game-control-button-dpad-down"),
        13 => tr!("game-control-button-dpad-left"),
        14 => tr!("game-control-button-dpad-right"),
        15 => tr!("game-control-button-misc1"),
        16 => tr!("game-control-button-paddle1"),
        17 => tr!("game-control-button-paddle2"),
        18 => tr!("game-control-button-paddle3"),
        19 => tr!("game-control-button-paddle4"),
        20 => tr!("game-control-button-touchpad"),
        _ => return format!("BUTTON_{index}"),
    };
    name.to_owned()
}

/// The width of a live value in the Device Options rows.
const VALUE_W: f32 = 80.0;

/// The pictogram of a canonical input (promptFontGlyph,
/// llpanelpreferencegamecontrol.cpp): PromptFont's codepoints, its generic
/// gamepad, D-pad and stick sets first, its Xbox set for the shoulders and
/// triggers, which have no generic one. `None` for the inputs without one
/// (buttons 21 to 31, none): they keep their name alone.
fn glyph(input: &str) -> Option<&'static str> {
    Some(match input {
        "BUTTON_SOUTH" => "\u{21A7}",
        "BUTTON_EAST" => "\u{21A6}",
        "BUTTON_WEST" => "\u{21A4}",
        "BUTTON_NORTH" => "\u{21A5}",
        "BUTTON_DPAD_UP" => "\u{219F}",
        "BUTTON_DPAD_DOWN" => "\u{21A1}",
        "BUTTON_DPAD_LEFT" => "\u{219E}",
        "BUTTON_DPAD_RIGHT" => "\u{21A0}",
        "BUTTON_LEFT_SHOULDER" => "\u{2198}",
        "BUTTON_RIGHT_SHOULDER" => "\u{2199}",
        "BUTTON_LEFT_STICK" => "\u{21BA}",
        "BUTTON_RIGHT_STICK" => "\u{21BB}",
        "BUTTON_SELECT" => "\u{21F7}",
        "BUTTON_HOME" => "\u{21F9}",
        "BUTTON_START" => "\u{21F8}",
        "BUTTON_MISC1" => "\u{2212}",
        "BUTTON_PADDLE1" => "\u{2277}",
        "BUTTON_PADDLE2" => "\u{2276}",
        "BUTTON_PADDLE3" => "\u{2279}",
        "BUTTON_PADDLE4" => "\u{2278}",
        "BUTTON_TOUCHPAD" => "\u{21E7}",
        "AXIS_LEFTX" => "\u{21C4}",
        "AXIS_LEFTY" => "\u{21C5}",
        "AXIS_RIGHTX" => "\u{21C6}",
        "AXIS_RIGHTY" => "\u{21F5}",
        "AXIS_LEFT_TRIGGER" => "\u{2196}",
        "AXIS_RIGHT_TRIGGER" => "\u{2197}",
        // both triggers side by side
        AXIS_TRIGGERS => "\u{2196}\u{2197}",
        _ => return None,
    })
}

/// The pictograms are drawn larger than the text, as the LL viewer's
/// "Huge" cells, so that they read clearly.
const GLYPH_SIZE: f32 = 18.0;

/// An input's pictogram then its name, in the text colour of the widget
/// that shows it (a combo, an item, a label), or in `color`.
fn with_glyph(ui: &egui::Ui, input: &str, name: &str, color: Option<egui::Color32>) -> egui::WidgetText {
    let mut job = egui::text::LayoutJob::default();
    let paint = |text: egui::RichText| match color {
        Some(c) => text.color(c),
        None => text,
    };
    let append = |job: &mut egui::text::LayoutJob, text: egui::RichText| {
        text.append_to(job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
    };
    if let (Some(g), Some(family)) = (glyph(input), crate::ui::fonts::game_control(ui.ctx())) {
        append(&mut job, paint(egui::RichText::new(g).family(family).size(GLYPH_SIZE)));
        append(&mut job, egui::RichText::new(" "));
    }
    append(&mut job, paint(egui::RichText::new(name)));
    job.into()
}

/// The saved name of an axis output of the Device Mappings.
fn output_input(code: u8) -> &'static str {
    match code {
        AXIS_OUTPUT_TRIGGER_PAIR => AXIS_TRIGGERS,
        AXIS_OUTPUT_NONE => AXIS_NONE,
        n => AXIS_NAMES.get(usize::from(n)).copied().unwrap_or(AXIS_NONE),
    }
}

/// The axis inputs an action may take, as saved: the four stick axes, both
/// triggers as one, none (axis_input_selector).
const AXIS_INPUTS: [(&str, usize); 6] = [
    ("AXIS_LEFTX", 0),
    ("AXIS_LEFTY", 1),
    ("AXIS_RIGHTX", 2),
    ("AXIS_RIGHTY", 3),
    (AXIS_TRIGGERS, 6),
    (AXIS_NONE, 7),
];

/// What a physical axis may give (axis_output_selector): an axis, both
/// triggers, nothing.
const AXIS_OUTPUTS: [u8; 8] = [0, 1, 2, 3, 4, 5, AXIS_OUTPUT_TRIGGER_PAIR, AXIS_OUTPUT_NONE];

fn output_name(code: u8) -> &'static str {
    match code {
        AXIS_OUTPUT_TRIGGER_PAIR => axis_input_name(6),
        AXIS_OUTPUT_NONE => tr!("common-none"),
        n => axis_input_name(usize::from(n)),
    }
}

fn axis_input_label(saved: &str) -> &'static str {
    AXIS_INPUTS.iter().find(|(name, _)| *name == saved).map_or_else(
        || match AXIS_NAMES.iter().position(|n| *n == saved) {
            Some(i) => axis_input_name(i),
            None => tr!("common-none"),
        },
        |(_, i)| axis_input_name(*i),
    )
}

fn button_input_label(saved: &str) -> String {
    match (0..NUM_BUTTONS).find(|&i| gc::button_name(i) == saved) {
        Some(i) => button_input_name(i),
        None => tr!("common-none").to_owned(),
    }
}

/// The input bound to an action, `None` for none.
fn bound(map: &std::collections::BTreeMap<String, String>, action: &str, none: &str) -> Option<String> {
    map.get(action).filter(|v| v.as_str() != none && !v.is_empty()).cloned()
}

fn mode_path(mode: Mode) -> &'static str {
    match mode {
        Mode::Avatar => "game_control.avatar",
        Mode::Mouselook => "game_control.mouselook",
        Mode::Flycam => "game_control.flycam",
        Mode::Captive => "game_control.captive",
        Mode::Cursor | Mode::None => "game_control.cursor",
    }
}

fn enabled_field(mode: Mode) -> F<bool> {
    match mode {
        Mode::Avatar => field!(game_control.avatar.enabled),
        Mode::Mouselook => field!(game_control.mouselook.enabled),
        Mode::Flycam => field!(game_control.flycam.enabled),
        Mode::Captive => field!(game_control.captive.enabled),
        Mode::Cursor | Mode::None => field!(game_control.cursor.enabled),
    }
}

fn speed_field(mode: Mode) -> F<f32> {
    match mode {
        Mode::Mouselook => field!(game_control.mouselook.speed_factor),
        Mode::Flycam => field!(game_control.flycam.speed_factor),
        _ => field!(game_control.avatar.speed_factor),
    }
}

fn actions_tab(cx: &mut Cx) {
    let live = cx.st.game_control.mode;
    let edited = cx
        .st
        .game_control_mode
        .unwrap_or(if live == Mode::None { Mode::Avatar } else { live });
    cx.row_with(
        tr!("game-control-edit-mode"),
        tr!("game-control-edit-mode-hint"),
        &Nothing,
        |ui, _, st, _| {
            let mut m = edited;
            egui::ComboBox::from_id_salt("game-control-mode")
                .selected_text(mode_title(m))
                .show_ui(ui, |ui| {
                    for mode in Mode::ALL {
                        ui.selectable_value(&mut m, mode, mode_title(mode));
                    }
                });
            if m != edited {
                st.game_control_mode = Some(m);
                st.game_control_capture = None;
            }
            false
        },
    );
    cx.note(&tr!("game-control-active-mode", mode = active_name(live)));
    for mode in Mode::ALL {
        cx.conditional(mode == edited, |cx| mode_rows(cx, mode));
    }
}

fn mode_rows(cx: &mut Cx, mode: Mode) {
    let p = cx.p;
    let title = mode_title(mode);
    cx.group(title, |cx| {
        cx.toggle(
            tr!("game-control-mode-enabled"),
            tr!("game-control-mode-enabled-hint"),
            &enabled_field(mode),
        );
        cx.greyed(!cx.s.game_control.mode_enabled(mode));
        if mode == Mode::Flycam {
            cx.toggle(
                tr!("game-control-allow-roll"),
                tr!("game-control-allow-roll-hint"),
                &field!(game_control.flycam_allow_roll),
            );
            cx.toggle(
                tr!("game-control-hide-cursor"),
                tr!("game-control-hide-cursor-hint"),
                &field!(game_control.flycam_hide_cursor),
            );
            cx.toggle(
                tr!("game-control-crosshair"),
                tr!("game-control-crosshair-hint"),
                &field!(game_control.flycam_crosshair),
            );
        }
        if matches!(mode, Mode::Flycam | Mode::Mouselook | Mode::Avatar) {
            cx.slider(
                tr!("game-control-speed"),
                tr!("game-control-speed-hint"),
                &speed_field(mode),
                gc::SPEED_FACTOR_MIN..=gc::SPEED_FACTOR_MAX,
                "×",
            );
        }
        cx.row(
            tr!("game-control-restore-actions"),
            tr!("game-control-restore-actions-hint"),
            &Nothing,
            |ui, s| {
                let on = s.game_control.mode_enabled(mode);
                let clicked = ui
                    .add_enabled_ui(on, |ui| {
                        widgets::flat_button(ui, &p, tr!("game-control-restore-defaults")).clicked()
                    })
                    .inner;
                if clicked {
                    s.game_control.restore_mode_defaults(mode);
                }
                clicked
            },
        );
    });
    let axes_title = format!("{title} · {}", tr!("game-control-axes"));
    cx.group(&axes_title, |cx| {
        cx.greyed(!cx.s.game_control.mode_enabled(mode));
        for &action in mappings::axis_choices(mode) {
            binding_row(cx, mode, true, action);
        }
    });
    let buttons_title = format!("{title} · {}", tr!("game-control-buttons"));
    cx.group(&buttons_title, |cx| {
        cx.greyed(!cx.s.game_control.mode_enabled(mode));
        for &action in mappings::button_choices(mode) {
            binding_row(cx, mode, false, action);
        }
    });
}

/// One action's binding: its input, its invert (axes), and the capture.
fn binding_row(cx: &mut Cx, mode: Mode, axis: bool, action: Action) {
    let p = cx.p;
    let none = if axis { AXIS_NONE } else { BUTTON_NONE };
    let label = action.label();
    let map = move |s: &Settings| {
        if axis {
            s.game_control.axes(mode)
        } else {
            s.game_control.buttons(mode)
        }
    };
    let track = Custom {
        path: mode_path(mode),
        modified: move |s: &Settings, d: &Settings| {
            bound(&map(s), label, none) != bound(&map(d), label, none)
                || (axis && s.game_control.axis_invert(mode, label) != d.game_control.axis_invert(mode, label))
        },
        reset: move |s: &mut Settings, d: &Settings| {
            let input = bound(&map(d), label, none).unwrap_or_else(|| none.to_owned());
            s.game_control.set_binding(mode, axis, label, &input);
            if axis {
                let invert = d.game_control.axis_invert(mode, label);
                s.game_control.set_axis_invert(mode, label, invert);
            }
        },
    };
    let hint = if axis {
        tr!("game-control-axis-binding-hint")
    } else {
        tr!("game-control-button-binding-hint")
    };
    let roll_off = mode == Mode::Flycam && action.is_roll() && !cx.s.game_control.flycam_allow_roll;
    let this = Capture { mode, axis, action };
    cx.row_with(action_name(mode, action), hint, &track, |ui, s, st, _| {
        let current = bound(&map(s), label, none);
        let mut chosen: Option<String> = None;
        ui.horizontal(|ui| {
            let text = match &current {
                Some(input) if axis => axis_input_label(input).to_owned(),
                Some(input) => button_input_label(input),
                None => tr!("common-none").to_owned(),
            };
            let text = with_glyph(ui, current.as_deref().unwrap_or(none), &text, None);
            egui::ComboBox::from_id_salt(("game-control-binding", mode as u8, axis, label))
                .selected_text(text)
                .show_ui(ui, |ui| {
                    if axis {
                        for (name, index) in AXIS_INPUTS {
                            if ui
                                .selectable_label(
                                    current.as_deref().unwrap_or(AXIS_NONE) == name,
                                    with_glyph(ui, name, axis_input_name(index), None),
                                )
                                .clicked()
                            {
                                chosen = Some(name.to_owned());
                            }
                        }
                    } else {
                        for i in 0..NUM_BUTTONS {
                            let name = gc::button_name(i);
                            if ui
                                .selectable_label(
                                    current.as_deref() == Some(name.as_str()),
                                    with_glyph(ui, &name, &button_input_name(i), None),
                                )
                                .clicked()
                            {
                                chosen = Some(name);
                            }
                        }
                        if ui.selectable_label(current.is_none(), tr!("common-none")).clicked() {
                            chosen = Some(BUTTON_NONE.to_owned());
                        }
                    }
                });
            let waiting = st.game_control_capture == Some(this);
            let tip = if waiting {
                tr!("game-control-capture-waiting")
            } else {
                tr!("game-control-capture")
            };
            if widgets::icon_button(ui, &p, "player-record-filled", tip, !roll_off) {
                st.game_control_capture = if waiting { None } else { Some(this) };
            }
            if waiting {
                let found = match st.game_control.active {
                    Some(ActiveInput::Axis(i, _)) if axis => Some(if i >= 4 {
                        AXIS_TRIGGERS.to_owned()
                    } else {
                        AXIS_NAMES[i].to_owned()
                    }),
                    Some(ActiveInput::Button(i)) if !axis => Some(gc::button_name(i)),
                    _ => None,
                };
                if let Some(input) = found {
                    chosen = Some(input);
                    st.game_control_capture = None;
                }
            }
            if axis {
                let mut invert = s.game_control.axis_invert(mode, label);
                let enabled = current.is_some() && !roll_off;
                let changed = ui
                    .add_enabled_ui(enabled, |ui| {
                        widgets::checkbox(ui, &p, &mut invert, tr!("game-control-invert")).changed()
                    })
                    .inner;
                if changed {
                    s.game_control.set_axis_invert(mode, label, invert);
                    chosen.get_or_insert_with(|| current.clone().unwrap_or_else(|| none.to_owned()));
                }
            }
        });
        let Some(input) = chosen else { return false };
        // (roll off: a roll action may only be cleared)
        if roll_off && input != none {
            return false;
        }
        s.game_control.bind_exclusive(mode, axis, label, &input);
        true
    });
}

/// The devices a tab lists: the connected ones, and with `all` the ones
/// saved in the settings too.
fn device_list(cx: &Cx, all: bool) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = cx.st.game_control.devices.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
    if all {
        for (id, config) in &cx.s.game_control.devices {
            if !list.iter().any(|(known, _)| known == id) {
                list.push((id.clone(), device::saved_name(config).unwrap_or_else(|| id.clone())));
            }
        }
    }
    list
}

/// The device picker of the two device tabs; the device chosen, if any.
fn device_picker(cx: &mut Cx, all: bool) -> Option<String> {
    let list = device_list(cx, all);
    if list.is_empty() {
        cx.st.game_control_device = String::new();
        cx.note(tr!("game-control-no-device"));
        return None;
    }
    if !list.iter().any(|(id, _)| *id == cx.st.game_control_device) {
        cx.st.game_control_device = list[0].0.clone();
    }
    let picked = cx.st.game_control_device.clone();
    cx.row_with(tr!("game-control-device"), "", &Nothing, |ui, _, st, _| {
        let name = list
            .iter()
            .find(|(id, _)| *id == picked)
            .map_or(picked.as_str(), |(_, n)| n.as_str());
        egui::ComboBox::from_id_salt("game-control-device")
            .selected_text(name)
            .show_ui(ui, |ui| {
                for (id, name) in &list {
                    if ui.selectable_label(*id == picked, name).clicked() {
                        st.game_control_device = id.clone();
                        st.game_control_calibration = None;
                    }
                }
            });
        false
    });
    Some(cx.st.game_control_device.clone())
}

/// A device's options as saved, and its name.
fn device_options(s: &Settings, devices: &[DeviceView], id: &str) -> (Options, String) {
    let config = s.game_control.devices.get(id).cloned().unwrap_or_default();
    let name = devices
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name.clone())
        .or_else(|| device::saved_name(&config))
        .unwrap_or_else(|| id.to_owned());
    (Options::load(&config), name)
}

fn save_device(s: &mut Settings, id: &str, name: &str, options: &Options) {
    s.game_control.devices.insert(id.to_owned(), options.save(name, true));
}

fn device_mappings_tab(cx: &mut Cx) {
    let p = cx.p;
    let mut all = cx.st.game_control_show_all;
    cx.row(tr!("game-control-show-all-devices"), "", &Nothing, |ui, _| {
        widgets::switch(ui, &p, &mut all);
        false
    });
    cx.st.game_control_show_all = all;
    let Some(id) = device_picker(cx, all) else { return };
    let devices = cx.st.game_control.devices.clone();
    {
        let (id, devices) = (id.clone(), devices.clone());
        cx.row(
            tr!("game-control-restore-device"),
            tr!("game-control-restore-device-hint"),
            &Nothing,
            move |ui, s| {
                if widgets::flat_button(ui, &p, tr!("game-control-restore-defaults")).clicked() {
                    let (_, name) = device_options(s, &devices, &id);
                    save_device(s, &id, &name, &Options::default());
                    return true;
                }
                false
            },
        );
    }
    cx.group(tr!("game-control-axes"), |cx| {
        for phys in 0..NUM_AXES {
            let (id, devices) = (id.clone(), devices.clone());
            cx.row(
                axis_input_name(phys),
                tr!("game-control-device-axis-hint"),
                &Nothing,
                move |ui, s| {
                    let (mut o, name) = device_options(s, &devices, &id);
                    let current = o.axis_map[phys];
                    let mut choice = current;
                    egui::ComboBox::from_id_salt(("game-control-axis-map", phys))
                        .selected_text(with_glyph(ui, output_input(current), output_name(current), None))
                        .show_ui(ui, |ui| {
                            for out in AXIS_OUTPUTS {
                                let text = with_glyph(ui, output_input(out), output_name(out), None);
                                ui.selectable_value(&mut choice, out, text);
                            }
                        });
                    if choice == current {
                        return false;
                    }
                    o.axis_map[phys] = choice;
                    save_device(s, &id, &name, &o);
                    true
                },
            );
        }
    });
    cx.group(tr!("game-control-buttons"), |cx| {
        for phys in 0..NUM_BUTTONS {
            let (id, devices) = (id.clone(), devices.clone());
            cx.row(
                &button_input_name(phys),
                tr!("game-control-device-button-hint"),
                &Nothing,
                move |ui, s| {
                    let (mut o, name) = device_options(s, &devices, &id);
                    let current = usize::from(o.button_map[phys]);
                    let mut choice = current;
                    egui::ComboBox::from_id_salt(("game-control-button-map", phys))
                        .selected_text(with_glyph(ui, &gc::button_name(current), &button_input_name(current), None))
                        .show_ui(ui, |ui| {
                            for out in 0..NUM_BUTTONS {
                                let text = with_glyph(ui, &gc::button_name(out), &button_input_name(out), None);
                                ui.selectable_value(&mut choice, out, text);
                            }
                        });
                    if choice == current {
                        return false;
                    }
                    o.button_map[phys] = choice as u8;
                    save_device(s, &id, &name, &o);
                    true
                },
            );
        }
    });
}

fn device_options_tab(cx: &mut Cx) {
    let p = cx.p;
    let Some(id) = device_picker(cx, false) else { return };
    let devices = cx.st.game_control.devices.clone();
    let live = devices.iter().find(|d| d.id == id).cloned().unwrap_or_default();
    {
        let (id, devices) = (id.clone(), devices.clone());
        cx.row(
            tr!("game-control-restore-device-options"),
            tr!("game-control-restore-device-options-hint"),
            &Nothing,
            move |ui, s| {
                if widgets::flat_button(ui, &p, tr!("game-control-restore-defaults")).clicked() {
                    let (mut o, name) = device_options(s, &devices, &id);
                    o.axes = Default::default();
                    save_device(s, &id, &name, &o);
                    return true;
                }
                false
            },
        );
    }
    {
        let id = id.clone();
        cx.row_with(
            tr!("game-control-auto-calibrate"),
            tr!("game-control-auto-calibrate-hint"),
            &Nothing,
            move |ui, _, st, _| {
                let busy = st.game_control_calibration.is_some();
                if ui
                    .add_enabled_ui(!busy, |ui| {
                        widgets::flat_button(ui, &p, tr!("game-control-auto-calibrate")).clicked()
                    })
                    .inner
                {
                    st.game_control_calibration = Some(Calibration {
                        device: id,
                        samples: Vec::new(),
                    });
                }
                false
            },
        );
    }
    cx.group(tr!("game-control-axes"), |cx| {
        for axis in 0..NUM_AXES {
            let (id, devices) = (id.clone(), devices.clone());
            let (raw, fixed) = (live.raw[axis], live.fixed[axis]);
            cx.wide().row(
                axis_input_name(axis),
                tr!("game-control-axis-options-hint"),
                &Nothing,
                move |ui, s| {
                    let (mut o, name) = device_options(s, &devices, &id);
                    let before = o.axes;
                    ui.horizontal(|ui| {
                        // (fixed widths: the live values do not shift the row)
                        ui.add_sized(
                            egui::vec2(VALUE_W, 20.0),
                            egui::Label::new(tr!("game-control-raw-value", value = raw.to_string())),
                        );
                        let a = &mut o.axes[axis];
                        let mut offset = i32::from(a.offset);
                        ui.label(tr!("game-control-offset"));
                        ui.add(egui::DragValue::new(&mut offset).range(-MAX_AXIS_OFFSET..=MAX_AXIS_OFFSET));
                        a.offset = offset as i16;
                        let mut invert = a.multiplier == -1;
                        if widgets::checkbox(ui, &p, &mut invert, tr!("game-control-invert")).changed() {
                            a.multiplier = if invert { -1 } else { 1 };
                            // the triggers are inverted together
                            if axis >= 4 {
                                o.axes[9 - axis].multiplier = o.axes[axis].multiplier;
                            }
                        }
                        let a = &mut o.axes[axis];
                        ui.label(tr!("game-control-dead-zone"));
                        ui.add(egui::DragValue::new(&mut a.dead_zone).range(0..=MAX_AXIS_DEAD_ZONE));
                        ui.add_sized(
                            egui::vec2(VALUE_W, 20.0),
                            egui::Label::new(tr!("game-control-adjusted-value", value = fixed.to_string())),
                        );
                    });
                    if o.axes == before {
                        return false;
                    }
                    save_device(s, &id, &name, &o);
                    true
                },
            );
        }
    });
    cx.below().row(
        tr!("game-control-buttons"),
        tr!("game-control-buttons-live-hint"),
        &Nothing,
        move |ui, _| {
            ui.horizontal_wrapped(|ui| {
                for b in 0..NUM_BUTTONS {
                    let down = live.buttons & (1 << b) != 0;
                    let color = if down { p.violet } else { p.muted };
                    ui.label(with_glyph(ui, &gc::button_name(b), &button_input_name(b), Some(color)));
                }
            });
            false
        },
    );
}

fn server_tab(cx: &mut Cx) {
    let p = cx.p;
    cx.toggle(
        tr!("game-control-send-to-server"),
        tr!("game-control-send-to-server-hint"),
        &field!(game_control.send_to_server),
    );
    cx.note(tr!("game-control-data-note"));
    let sent = cx.st.game_control.sent;
    cx.below().row(tr!("game-control-data"), "", &Nothing, move |ui, _| {
        let Some(state) = sent else {
            ui.label(tr!("game-control-nothing-sent"));
            return false;
        };
        let mode = match state.mode {
            0 => Mode::Avatar,
            1 => Mode::Mouselook,
            2 => Mode::Flycam,
            3 => Mode::Captive,
            4 => Mode::Cursor,
            _ => Mode::None,
        };
        let line = |ui: &mut egui::Ui, index: usize, name: &str, value: String| {
            ui.horizontal(|ui| {
                let cell = |ui: &mut egui::Ui, width: f32, text: egui::RichText| {
                    ui.allocate_ui_with_layout(egui::vec2(width, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(width);
                        ui.label(text);
                    });
                };
                cell(ui, 28.0, egui::RichText::new(index.to_string()).color(p.muted));
                cell(ui, 220.0, egui::RichText::new(name));
                ui.label(value);
            });
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, name) in AXIS_NAMES.iter().enumerate() {
                line(ui, i, name, state.axes[i].to_string());
            }
            for i in 0..NUM_BUTTONS {
                line(ui, i, &gc::button_name(i), u8::from(state.buttons & (1 << i) != 0).to_string());
            }
            for (i, value) in state.semantic_axes.iter().enumerate().take(mode.semantic_axes()) {
                line(ui, i, mappings::semantic_axis_name(mode, i), value.to_string());
            }
            for i in 0..mappings::semantic_buttons(mode) {
                line(
                    ui,
                    i,
                    &mappings::semantic_button_name(mode, i),
                    u8::from(state.semantic_buttons & (1 << i) != 0).to_string(),
                );
            }
        });
        false
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_input_has_a_pictogram_in_the_font() {
        let face = ttf_parser::Face::parse(crate::ui::fonts::PROMPTFONT_TTF, 0).expect("embedded font");
        let names = gc::BUTTON_NAMES.iter().chain(AXIS_NAMES.iter()).chain([&AXIS_TRIGGERS]);
        for name in names {
            let g = glyph(name).unwrap_or_else(|| panic!("{name} has no pictogram"));
            assert!(g.chars().all(|c| face.glyph_index(c).is_some()), "{name}: {g:?} missing");
        }
        // the extra buttons and none keep their name alone
        for name in [
            gc::button_name(21),
            gc::button_name(31),
            BUTTON_NONE.to_owned(),
            AXIS_NONE.to_owned(),
        ] {
            assert_eq!(glyph(&name), None, "{name}");
        }
    }

    #[test]
    fn axis_outputs_name_their_input() {
        assert_eq!(output_input(0), "AXIS_LEFTX");
        assert_eq!(output_input(5), "AXIS_RIGHT_TRIGGER");
        assert_eq!(output_input(AXIS_OUTPUT_TRIGGER_PAIR), AXIS_TRIGGERS);
        assert_eq!(output_input(AXIS_OUTPUT_NONE), AXIS_NONE);
    }
}
