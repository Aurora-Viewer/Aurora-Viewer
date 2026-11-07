//! LLFloaterJoystick settings, with native axis and button monitoring.

use super::super::cx::{Custom, Cx, F, Many, Nothing, field};
use crate::ui::widgets;
use std::time::Instant;

pub(in crate::ui::options) enum JoystickCapture {
    Button(ButtonCapture),
    Axis(AxisCapture),
    Zoom(ZoomCapture),
}

impl JoystickCapture {
    pub fn hide(&mut self) {
        match self {
            Self::Button(capture) => capture.visible = false,
            Self::Axis(capture) => capture.visible = false,
            Self::Zoom(capture) => capture.button.visible = false,
        }
    }

    pub fn visible(&self) -> bool {
        match self {
            Self::Button(capture) => capture.visible,
            Self::Axis(capture) => capture.visible,
            Self::Zoom(capture) => capture.button.visible,
        }
    }
}

pub(in crate::ui::options) struct ZoomCapture {
    button: ButtonCapture,
    axis: crate::joystick::capture::AxisCapture,
    held_axes: u16,
}

impl ZoomCapture {
    fn poll(&mut self, snapshot: &crate::joystick::Snapshot, settings: &crate::joystick::Settings) -> Result<Option<i8>, ()> {
        if let Some(button) = self.button.poll(snapshot, settings)? {
            return Ok(Some(button));
        }
        let device = snapshot.selected_joystick(settings).ok_or(())?;
        let held = crate::joystick::axis_button_mask(device);
        for (axis, &value) in device.axes.iter().take(device.axis_count.min(crate::joystick::AXES)).enumerate() {
            let pair = 3u16 << (axis * 2);
            if self.held_axes & pair != 0 && held & pair == 0 {
                self.axis.rebase(axis, value);
            }
        }
        self.held_axes &= held;
        if let Some(axis) = self.axis.poll(snapshot, settings, Instant::now())? {
            if let Some(binding) = self.axis.button(axis) {
                let bit = 1u16 << (usize::try_from(binding).map_err(|_| ())? - crate::joystick::BUTTONS);
                if self.held_axes & bit == 0 {
                    return Ok(Some(binding));
                }
            } else {
                self.axis.rebase(axis as usize, device.axes[axis as usize]);
            }
        }
        Ok(None)
    }
}

pub(in crate::ui::options) struct ButtonCapture {
    path: &'static str,
    device: String,
    presses: [u64; crate::joystick::BUTTONS],
    visible: bool,
}

impl ButtonCapture {
    fn new(path: &'static str, device: &crate::joystick::Device) -> Self {
        Self {
            path,
            device: device.id.clone(),
            presses: device.presses,
            visible: true,
        }
    }

    /// Count edges, including a press released between UI frames. A held
    /// button at the start of capture is not a new choice.
    fn poll(&self, snapshot: &crate::joystick::Snapshot, settings: &crate::joystick::Settings) -> Result<Option<i8>, ()> {
        let device = snapshot
            .selected_joystick(settings)
            .filter(|d| d.id == self.device && snapshot.updated.is_some_and(|at| at.elapsed().as_millis() < 250))
            .ok_or(())?;
        // A reconnected device with the same id can restart its counters.
        if device.presses.iter().zip(self.presses).any(|(now, before)| *now < before) {
            return Err(());
        }
        Ok(device
            .presses
            .iter()
            .zip(self.presses)
            .position(|(now, before)| *now != before)
            .map(|n| n as i8))
    }
}

pub(in crate::ui::options) struct AxisCapture {
    channel: usize,
    input: crate::joystick::capture::AxisCapture,
    visible: bool,
}

impl AxisCapture {
    fn new(channel: usize, device: &crate::joystick::Device) -> Self {
        Self {
            channel,
            input: crate::joystick::capture::AxisCapture::new(device),
            visible: true,
        }
    }
}

fn button(cx: &mut Cx, label: &str, field: F<i8>) {
    button_hint(cx, label, aurora_i18n::tr!("joystick-button-hint"), field, false);
}

fn button_hint(cx: &mut Cx, label: &str, hint: &str, field: F<i8>, capture_axes: bool) {
    use aurora_i18n::tr;
    cx.row_with(label, hint, &field, |ui, s, st, _| {
        let mut value = *(field.get)(s);
        let mut changed = false;
        let captured = match st.joystick_capture.as_mut() {
            Some(JoystickCapture::Button(capture)) if capture.path == field.path => {
                capture.visible = true;
                Some(capture.poll(&st.joystick, &s.joystick))
            }
            Some(JoystickCapture::Zoom(capture)) if capture.button.path == field.path => {
                capture.button.visible = true;
                Some(capture.poll(&st.joystick, &s.joystick))
            }
            _ => None,
        };
        match captured {
            Some(Ok(Some(button))) => {
                value = button;
                changed = true;
                st.cancel_capture();
            }
            Some(Err(())) => st.cancel_capture(),
            _ => {}
        }
        let capturing = match &st.joystick_capture {
            Some(JoystickCapture::Button(capture)) => capture.path == field.path,
            Some(JoystickCapture::Zoom(capture)) => capture.button.path == field.path,
            _ => false,
        };
        let available =
            st.joystick.selected_joystick(&s.joystick).is_some() && st.joystick.updated.is_some_and(|at| at.elapsed().as_millis() < 250);
        let text = if capturing {
            if capture_axes {
                tr!("joystick-zoom-press")
            } else {
                tr!("joystick-button-press")
            }
            .to_owned()
        } else if value < 0 {
            tr!("joystick-button-assign").to_owned()
        } else if capture_axes && let Some((axis, positive)) = crate::joystick::button_axis(value) {
            tr!(
                "joystick-axis-button-value",
                axis = axis,
                direction = if positive { "+" } else { "−" }
            )
        } else {
            tr!("joystick-button-value", button = value)
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(available, egui::Button::new(text).min_size(egui::vec2(126.0, 24.0)))
                .clicked()
            {
                st.cancel_capture();
                if !capturing && let Some(device) = st.joystick.selected_joystick(&s.joystick) {
                    let button = ButtonCapture::new(field.path, device);
                    st.joystick_capture = Some(if capture_axes && device.axis_count > 0 {
                        JoystickCapture::Zoom(ZoomCapture {
                            button,
                            axis: crate::joystick::capture::AxisCapture::new(device),
                            held_axes: crate::joystick::axis_button_mask(device),
                        })
                    } else {
                        JoystickCapture::Button(button)
                    });
                }
            }
            if ui
                .add_enabled(
                    capturing || value >= 0,
                    egui::Button::new(if capturing { tr!("common-cancel") } else { tr!("common-none") }),
                )
                .clicked()
            {
                st.cancel_capture();
                if !capturing {
                    value = -1;
                    changed = true;
                }
            }
        });
        *(field.get_mut)(s) = value;
        changed
    });
}

fn axis_binding(
    ui: &mut egui::Ui,
    settings: &mut crate::joystick::Settings,
    st: &mut super::super::OptionsUi,
    channel: usize,
    p: &crate::theme::Palette,
) -> bool {
    use aurora_i18n::tr;
    let mut changed = false;
    let captured = match st.joystick_capture.as_mut() {
        Some(JoystickCapture::Axis(capture)) if capture.channel == channel => {
            capture.visible = true;
            Some(capture.input.poll(&st.joystick, settings, Instant::now()))
        }
        _ => None,
    };
    match captured {
        Some(Ok(Some(axis))) => {
            settings.mapping[channel] = axis;
            if channel == 6 && settings.flycam.gain[channel] == 0.0 {
                // Explicitly assigning zoom should produce a usable lens
                // control, rather than retain the inactive preset's zero gain.
                settings.flycam.gain[channel] = 1.0f32.copysign(settings.flycam.gain[channel]);
            }
            changed = true;
            st.cancel_capture();
        }
        Some(Err(())) => st.cancel_capture(),
        _ => {}
    }
    let capturing = matches!(&st.joystick_capture, Some(JoystickCapture::Axis(capture)) if capture.channel == channel);
    let available = st.joystick.selected_joystick(settings).is_some_and(|device| device.axis_count != 0)
        && st.joystick.updated.is_some_and(|at| at.elapsed().as_millis() < 250);
    let text = if capturing {
        tr!("joystick-axis-move").to_owned()
    } else if settings.mapping[channel] < 0 {
        tr!("joystick-button-assign").to_owned()
    } else {
        tr!("joystick-axis-value", axis = settings.mapping[channel])
    };
    ui.horizontal(|ui| {
        if ui
            .add_enabled(available, egui::Button::new(text).min_size(egui::vec2(126.0, 24.0)))
            .clicked()
        {
            st.cancel_capture();
            if !capturing && let Some(device) = st.joystick.selected_joystick(settings) {
                st.joystick_capture = Some(JoystickCapture::Axis(AxisCapture::new(channel, device)));
            }
        }
        if ui
            .add_enabled(
                capturing || settings.mapping[channel] >= 0,
                egui::Button::new(if capturing { tr!("common-cancel") } else { tr!("common-none") }),
            )
            .clicked()
        {
            st.cancel_capture();
            if !capturing {
                settings.mapping[channel] = -1;
                changed = true;
            }
        }
        let mut inverted = settings.axis_inverted(channel);
        if widgets::switch_labeled(ui, p, &mut inverted, tr!("joystick-axis-invert")).changed() {
            settings.set_axis_inverted(channel, inverted);
            changed = true;
        }
    });
    changed
}

fn tuning(ui: &mut egui::Ui, label: &str, tune: &mut crate::joystick::Tuning, channel: usize, inverted: bool) -> bool {
    use aurora_i18n::tr;
    let mut changed = false;
    let mut gain = tune.gain[channel].abs();
    let mut gain_changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        gain_changed |= ui
            .add(
                egui::DragValue::new(&mut gain)
                    .speed(0.01)
                    .range(0.0..=20.0)
                    .prefix(tr!("joystick-gain-prefix")),
            )
            .changed();
        changed |= ui
            .add(
                egui::DragValue::new(&mut tune.dead_zone[channel])
                    .speed(0.01)
                    .range(0.0..=1.0)
                    .prefix(tr!("joystick-zone-prefix")),
            )
            .changed();
    });
    if gain_changed {
        // Preserve the shared direction when editing a mode's magnitude.
        tune.gain[channel] = if inverted { -gain } else { gain };
    }
    changed || gain_changed
}

fn axis_controls(
    ui: &mut egui::Ui,
    settings: &mut crate::joystick::Settings,
    st: &mut super::super::OptionsUi,
    p: &crate::theme::Palette,
    channel: usize,
) -> bool {
    use aurora_i18n::tr;
    let mut changed = false;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let label_width = [tr!("joystick-avatar"), tr!("common-camera")]
        .iter()
        .map(|label| ui.painter().layout_no_wrap((*label).to_owned(), font.clone(), p.ink).size().x)
        .fold(0.0, f32::max);
    let column_width = ui.spacing().interact_size.x * 2.0 + label_width + 16.0;
    let kinds: &[usize] = if channel == 6 { &[2] } else { &[1, 2] };
    let columns = ((ui.available_width() + 16.0) / (column_width + 16.0))
        .floor()
        .clamp(1.0, kinds.len() as f32) as usize;
    ui.vertical(|ui| {
        changed |= axis_binding(ui, settings, st, channel, p);
        let inverted = settings.axis_inverted(channel);
        ui.spacing_mut().item_spacing.x = 16.0;
        for chunk in kinds.chunks(columns) {
            ui.columns(columns, |cells| {
                for (offset, ui) in cells.iter_mut().enumerate() {
                    let Some(&kind) = chunk.get(offset) else { continue };
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.push_id((channel, kind), |ui| match kind {
                        1 if matches!(channel, 3 | 6) => {
                            ui.horizontal(|ui| {
                                ui.set_min_height(24.0);
                                ui.weak(tr!("joystick-avatar"));
                                ui.weak(tr!("joystick-axis-camera-only"));
                            });
                        }
                        1 => changed |= tuning(ui, tr!("joystick-avatar"), &mut settings.avatar, channel, inverted),
                        2 => changed |= tuning(ui, tr!("common-camera"), &mut settings.flycam, channel, inverted),
                        _ => {}
                    });
                }
            });
        }
        ui.add_space(4.0);
    });
    changed
}

fn axis_row(cx: &mut Cx, n: usize, label: &str, hint: &str) {
    let p = cx.p;
    let paths = [
        "joystick.mapping[0]",
        "joystick.mapping[1]",
        "joystick.mapping[2]",
        "joystick.mapping[3]",
        "joystick.mapping[4]",
        "joystick.mapping[5]",
        "joystick.mapping[6]",
    ];
    let track = Custom {
        path: paths[n],
        modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| {
            let (s, d) = (&s.joystick, &d.joystick);
            s.mapping[n] != d.mapping[n]
                || s.axis_inverted(n) != d.axis_inverted(n)
                || s.avatar.gain[n] != d.avatar.gain[n]
                || s.flycam.gain[n] != d.flycam.gain[n]
                || s.avatar.dead_zone[n] != d.avatar.dead_zone[n]
                || s.flycam.dead_zone[n] != d.flycam.dead_zone[n]
        },
        reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
            let (s, d) = (&mut s.joystick, &d.joystick);
            s.mapping[n] = d.mapping[n];
            s.avatar.gain[n] = d.avatar.gain[n];
            s.flycam.gain[n] = d.flycam.gain[n];
            s.avatar.dead_zone[n] = d.avatar.dead_zone[n];
            s.flycam.dead_zone[n] = d.flycam.dead_zone[n];
        },
    };
    cx.below()
        .row_with(label, hint, &track, |ui, s, st, _| axis_controls(ui, &mut s.joystick, st, &p, n));
}

fn zoom_settings(cx: &mut Cx) {
    use crate::joystick::ZoomMode;
    use aurora_i18n::tr;
    cx.group(tr!("joystick-zoom-controls"), |cx| {
        if cx.wide().combo(
            tr!("joystick-zoom-mode"),
            tr!("joystick-zoom-mode-hint"),
            &field!(joystick.zoom_mode),
            &[
                (ZoomMode::Axis, tr!("joystick-zoom-mode-axis")),
                (ZoomMode::Buttons, tr!("joystick-zoom-mode-buttons")),
                (ZoomMode::HoldAxis, tr!("joystick-zoom-mode-hold-axis")),
            ],
        ) {
            cx.st.cancel_joystick_capture();
            if cx.s.joystick.zoom_mode != ZoomMode::Buttons && cx.s.joystick.flycam.gain[6] == 0.0 {
                cx.s.joystick.flycam.gain[6] = 1.0f32.copysign(cx.s.joystick.flycam.gain[6]);
            }
        }
        let mode = cx.s.joystick.zoom_mode;
        cx.conditional(mode == ZoomMode::Buttons, |cx| {
            button_hint(
                cx,
                tr!("joystick-zoom-in-button"),
                tr!("joystick-zoom-button-hint"),
                field!(joystick.zoom_in_button),
                true,
            );
            button_hint(
                cx,
                tr!("joystick-zoom-out-button"),
                tr!("joystick-zoom-button-hint"),
                field!(joystick.zoom_out_button),
                true,
            );
            cx.slider(
                if cx.s.joystick.absolute_zoom() {
                    tr!("joystick-zoom-strength")
                } else {
                    tr!("joystick-zoom-speed")
                },
                if cx.s.joystick.absolute_zoom() {
                    tr!("joystick-zoom-strength-hint")
                } else {
                    tr!("joystick-zoom-speed-hint")
                },
                &field!(joystick.zoom_speed),
                0.1..=5.0,
                "",
            );
        });
        cx.conditional(mode == ZoomMode::HoldAxis, |cx| {
            button_hint(
                cx,
                tr!("joystick-zoom-hold-button"),
                tr!("joystick-zoom-hold-hint"),
                field!(joystick.zoom_hold_button),
                true,
            );
        });
        cx.toggle(
            tr!("joystick-direct-zoom"),
            tr!("joystick-direct-zoom-hint"),
            &field!(joystick.direct_zoom),
        );
        cx.conditional(mode != ZoomMode::Buttons, |cx| {
            let hint = format!("{} {}", tr!("joystick-axis-hint"), tr!("joystick-zoom-hint"));
            axis_row(cx, 6, tr!("joystick-zoom"), &hint);
        });
    });
}

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("joystick-title"), |cx| {
        cx.toggle(tr!("joystick-enabled"), tr!("joystick-enable-hint"), &field!(joystick.enabled));
        device_dropdown(cx, false);
        cx.row_with(tr!("joystick-monitor"), tr!("joystick-center-hint"), &Nothing, |ui, s, st, _| {
            ui.vertical(|ui| {
                ui.set_width(210.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                if let Some(d) = st.joystick.selected_joystick(&s.joystick) {
                    for (row, pair) in d.axes[..d.axis_count].chunks(2).enumerate() {
                        let values = pair
                            .iter()
                            .enumerate()
                            .map(|(n, v)| format!("{}: {v:+.2}", row * 2 + n))
                            .collect::<Vec<_>>()
                            .join("   ");
                        ui.label(values);
                    }
                    let buttons = (0..32)
                        .filter(|n| d.buttons & (1 << n) != 0)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label(tr!(
                        "joystick-buttons-live",
                        buttons = if buttons.is_empty() {
                            tr!("common-none").to_owned()
                        } else {
                            buttons
                        }
                    ));
                } else {
                    ui.label(tr!("joystick-disconnected"));
                }
                if !st.joystick.error.is_empty() {
                    ui.label(tr!("joystick-backend-error")).on_hover_text(&st.joystick.error);
                }
            });
            false
        });
        cx.row(
            tr!("joystick-preset"),
            tr!("joystick-preset-hint"),
            &Many(&[&field!(joystick.mapping), &field!(joystick.avatar), &field!(joystick.flycam)]),
            |ui, s| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    if widgets::flat_button(ui, &p, tr!("joystick-gamepad")).clicked() {
                        s.joystick.raw_gamepad_preset();
                        changed = true;
                    }
                    if widgets::flat_button(ui, &p, tr!("joystick-spacemouse")).clicked() {
                        let d = crate::joystick::Settings::default();
                        s.joystick.mapping = d.mapping;
                        s.joystick.avatar = d.avatar;
                        s.joystick.flycam = d.flycam;
                        s.joystick.rise_button = d.rise_button;
                        s.joystick.fall_button = d.fall_button;
                        changed = true;
                    }
                });
                changed
            },
        );
        cx.toggle(tr!("joystick-avatar-enabled"), "", &field!(joystick.avatar_enabled));
        cx.toggle(tr!("joystick-flycam-enabled"), "", &field!(joystick.flycam_enabled));
        cx.row_with(tr!("joystick-flycam"), tr!("joystick-flycam-hint"), &Nothing, |ui, s, st, r| {
            let available = s.joystick.enabled && s.joystick.flycam_enabled && st.joystick.selected_joystick(&s.joystick).is_some();
            if ui
                .add_enabled(
                    available,
                    egui::Button::new(if st.flycam_active {
                        tr!("joystick-flycam-stop")
                    } else {
                        tr!("joystick-flycam-start")
                    }),
                )
                .clicked()
            {
                r.toggle_flycam = true;
            }
            false
        });
        button(cx, tr!("joystick-flycam-button"), field!(joystick.flycam_button));
        button(cx, tr!("joystick-jump-button"), field!(joystick.jump_button));
        cx.slider(tr!("joystick-run-threshold"), "", &field!(joystick.run_threshold), 0.1..=1.0, "");
    });
    cx.group(tr!("joystick-tuning"), |cx| {
        cx.toggle(
            tr!("joystick-mouselook-yaw"),
            tr!("joystick-mouselook-yaw-hint"),
            &field!(joystick.mouselook_yaw),
        );
        button(cx, tr!("joystick-rise-button"), field!(joystick.rise_button));
        button(cx, tr!("joystick-fall-button"), field!(joystick.fall_button));
        cx.slider(
            tr!("joystick-avatar-feather"),
            tr!("joystick-feather-hint"),
            &field!(joystick.avatar.feather),
            0.0..=32.0,
            "",
        );
        cx.slider(
            tr!("joystick-flycam-feather"),
            tr!("joystick-feather-hint"),
            &field!(joystick.flycam.feather),
            0.0..=32.0,
            "",
        );
        cx.toggle(tr!("joystick-auto-level"), "", &field!(joystick.auto_level));
    });
    cx.group(tr!("joystick-axes"), |cx| {
        let names = [
            tr!("joystick-forward"),
            tr!("joystick-slide"),
            tr!("joystick-up"),
            tr!("joystick-roll"),
            tr!("joystick-pitch"),
            tr!("joystick-yaw"),
        ];
        for (n, label) in names.into_iter().enumerate() {
            axis_row(cx, n, label, tr!("joystick-axis-hint"));
        }
    });
    zoom_settings(cx);
}

fn device_dropdown(cx: &mut Cx, gamepad: bool) {
    use aurora_i18n::tr;
    let (label, field) = if gamepad {
        (tr!("joystick-input-gamepad"), field!(gamepad.device))
    } else {
        (tr!("joystick-device"), field!(joystick.device))
    };
    cx.row_with(label, "", &field, |ui, s, st, _| {
        let profile = if gamepad { &mut s.gamepad } else { &mut s.joystick };
        let selected = if gamepad {
            st.joystick.selected_gamepad(profile)
        } else {
            st.joystick.selected_joystick(profile)
        };
        let name = if profile.device.is_empty() {
            tr!("joystick-first-device")
        } else {
            selected.map_or(tr!("joystick-disconnected"), |d| d.name.as_str())
        };
        let mut changed = false;
        egui::ComboBox::from_id_salt("joystick-device")
            .selected_text(name)
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut profile.device, String::new(), tr!("joystick-first-device"))
                    .changed();
                for d in st.joystick.devices.iter().filter(|device| device.is_gamepad == gamepad) {
                    changed |= ui.selectable_value(&mut profile.device, d.id.clone(), &d.name).changed();
                }
            });
        changed
    });
}

type BindingDevice = (bool, usize, [[Option<usize>; 2]; 2], [Option<usize>; 2], [Option<usize>; 2]);

fn bindings_device(native: Option<BindingDevice>) -> Option<crate::joystick::Device> {
    native.map(|(gamepad, count, sticks, triggers, dpad)| {
        let mut device = crate::joystick::Device::new(String::new(), String::new(), count);
        device.is_gamepad = gamepad;
        device.gamepad_sticks = sticks;
        device.gamepad_triggers = triggers;
        device.gamepad_dpad = dpad;
        device
    })
}

fn effective_gamepad_style(
    style: crate::joystick::GamepadStyle,
    device: Option<&crate::joystick::Device>,
) -> crate::joystick::GamepadStyle {
    use crate::joystick::GamepadStyle;
    match style {
        GamepadStyle::Auto => device.map_or(GamepadStyle::Generic, |device| device.gamepad_style),
        style => style,
    }
}

fn gamepad_button_label(button: crate::joystick::GamepadButton, index: usize, style: crate::joystick::GamepadStyle) -> String {
    use crate::joystick::{GamepadButton as B, GamepadStyle as S};
    use aurora_i18n::tr;
    if style == S::Generic || button == B::Other {
        return tr!("joystick-button-value", button = index + 1);
    }
    match (button, style) {
        (B::South, S::Xbox) => tr!("joystick-button-south-xbox"),
        (B::East, S::Xbox) => tr!("joystick-button-east-xbox"),
        (B::North, S::Xbox) => tr!("joystick-button-north-xbox"),
        (B::West, S::Xbox) => tr!("joystick-button-west-xbox"),
        (B::South, S::Playstation) => tr!("joystick-button-south-playstation"),
        (B::East, S::Playstation) => tr!("joystick-button-east-playstation"),
        (B::North, S::Playstation) => tr!("joystick-button-north-playstation"),
        (B::West, S::Playstation) => tr!("joystick-button-west-playstation"),
        (B::LeftBumper, S::Xbox) => tr!("joystick-button-left-bumper-xbox"),
        (B::RightBumper, S::Xbox) => tr!("joystick-button-right-bumper-xbox"),
        (B::LeftTrigger, S::Xbox) => tr!("joystick-button-left-trigger-xbox"),
        (B::RightTrigger, S::Xbox) => tr!("joystick-button-right-trigger-xbox"),
        (B::LeftBumper, S::Playstation) => tr!("joystick-button-left-bumper-playstation"),
        (B::RightBumper, S::Playstation) => tr!("joystick-button-right-bumper-playstation"),
        (B::LeftTrigger, S::Playstation) => tr!("joystick-button-left-trigger-playstation"),
        (B::RightTrigger, S::Playstation) => tr!("joystick-button-right-trigger-playstation"),
        (B::Select, S::Xbox) => tr!("joystick-button-select-xbox"),
        (B::Start, S::Xbox) => tr!("joystick-button-start-xbox"),
        (B::Select, S::Playstation) => tr!("joystick-button-select-playstation"),
        (B::Start, S::Playstation) => tr!("joystick-button-start-playstation"),
        (B::Mode, S::Xbox) => tr!("joystick-button-mode-xbox"),
        (B::Mode, S::Playstation) => tr!("joystick-button-mode-playstation"),
        (B::LeftStick, _) => tr!("joystick-button-left-stick"),
        (B::RightStick, _) => tr!("joystick-button-right-stick"),
        (B::DpadUp, _) => tr!("joystick-button-dpad-up"),
        (B::DpadDown, _) => tr!("joystick-button-dpad-down"),
        (B::DpadLeft, _) => tr!("joystick-button-dpad-left"),
        (B::DpadRight, _) => tr!("joystick-button-dpad-right"),
        _ => tr!("common-none"),
    }
    .to_owned()
}

fn dpad_label(direction: usize) -> &'static str {
    use aurora_i18n::tr;
    match direction {
        0 => tr!("joystick-button-dpad-up"),
        1 => tr!("joystick-button-dpad-down"),
        2 => tr!("joystick-button-dpad-left"),
        _ => tr!("joystick-button-dpad-right"),
    }
}

fn automatic_binding_label(binding: i8, style: crate::joystick::GamepadStyle) -> Option<String> {
    use crate::joystick::*;
    use aurora_i18n::tr;
    if let Some(button) = automatic_button(binding) {
        let index = GamepadButton::STANDARD.iter().position(|candidate| *candidate == button)?;
        return Some(gamepad_button_label(button, index, style));
    }
    Some(
        match binding {
            AUTO_DPAD_UP => dpad_label(0),
            AUTO_DPAD_DOWN => dpad_label(1),
            AUTO_DPAD_LEFT => dpad_label(2),
            AUTO_DPAD_RIGHT => dpad_label(3),
            AUTO_LEFT_TRIGGER => match style {
                GamepadStyle::Xbox => tr!("joystick-button-left-trigger-xbox"),
                GamepadStyle::Playstation => tr!("joystick-button-left-trigger-playstation"),
                _ => tr!("joystick-button-left-trigger"),
            },
            AUTO_RIGHT_TRIGGER => match style {
                GamepadStyle::Xbox => tr!("joystick-button-right-trigger-xbox"),
                GamepadStyle::Playstation => tr!("joystick-button-right-trigger-playstation"),
                _ => tr!("joystick-button-right-trigger"),
            },
            _ => return None,
        }
        .to_owned(),
    )
}

fn gamepad_button_binding(cx: &mut Cx, label: &str, field: F<i8>) {
    gamepad_button_binding_with_fallback(cx, label, field, None);
}

fn gamepad_button_binding_with_fallback(cx: &mut Cx, label: &str, field: F<i8>, inherited: Option<F<i8>>) {
    use aurora_i18n::tr;
    let hint = tr!("joystick-control-mapping-hint");
    cx.row_with(label, hint, &field, |ui, s, st, _| {
        let device = st.joystick.selected_gamepad(&s.gamepad);
        let style = effective_gamepad_style(s.gamepad.gamepad_style, device);
        let buttons = device.map_or_else(
            || crate::joystick::GamepadButton::STANDARD.to_vec(),
            |device| device.gamepad_buttons.clone(),
        );
        let mut choices: Vec<(i8, String)> = buttons
            .iter()
            .enumerate()
            .take(crate::joystick::BUTTONS)
            .map(|(index, button)| (index as i8, gamepad_button_label(*button, index, style)))
            .collect();
        if let Some(device) = device {
            for (side, axis) in device.gamepad_triggers.iter().enumerate() {
                if let Some(axis) = axis.filter(|axis| *axis < crate::joystick::AXES) {
                    let binding = crate::joystick::BUTTONS + axis * 2;
                    let label = match (side, style) {
                        (0, crate::joystick::GamepadStyle::Xbox) => tr!("joystick-button-left-trigger-xbox"),
                        (1, crate::joystick::GamepadStyle::Xbox) => tr!("joystick-button-right-trigger-xbox"),
                        (0, crate::joystick::GamepadStyle::Playstation) => tr!("joystick-button-left-trigger-playstation"),
                        (1, crate::joystick::GamepadStyle::Playstation) => tr!("joystick-button-right-trigger-playstation"),
                        (0, _) => tr!("joystick-button-left-trigger"),
                        _ => tr!("joystick-button-right-trigger"),
                    };
                    choices.push((binding as i8, label.to_owned()));
                }
            }
        }
        if let Some(device) = device {
            for direction in 0..4 {
                if let Some(binding) = crate::joystick::gamepad_dpad_binding(device, direction)
                    && !choices.iter().any(|(value, _)| *value == binding)
                {
                    choices.push((binding, dpad_label(direction).to_owned()));
                }
            }
        }
        let stored = *(field.get)(s);
        let effective = if stored == crate::joystick::INHERIT_BUTTON {
            inherited.map_or(-1, |fallback| *(fallback.get)(s))
        } else {
            stored
        };
        let mut value = device.map_or(effective, |device| crate::joystick::resolve_binding(device, effective));
        let selected = choices
            .iter()
            .find(|(binding, _)| *binding == value)
            .map(|(_, label)| label.clone())
            .or_else(|| device.is_none().then(|| automatic_binding_label(effective, style)).flatten())
            .unwrap_or_else(|| tr!("common-none").to_owned());
        let mut changed = false;
        egui::ComboBox::from_id_salt(field.path).selected_text(selected).show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut value, -1, tr!("common-none")).changed();
            for (binding, label) in &choices {
                changed |= ui.selectable_value(&mut value, *binding, label).changed();
            }
        });
        if changed {
            s.gamepad.bind_button(field.path, value, device);
        }
        changed
    });
}

fn gamepad_stick_binding(cx: &mut Cx, label: &str, field: F<crate::joystick::GamepadStick>) {
    use crate::joystick::GamepadStick as S;
    use aurora_i18n::tr;
    let first_person = field.path.starts_with("gamepad.first_person.");
    let (flycam, camera) = match field.path {
        "gamepad.first_person.move_stick" => (false, false),
        "gamepad.first_person.camera_stick" => (false, true),
        "gamepad.gamepad_move_stick" => (false, false),
        "gamepad.gamepad_camera_stick" => (false, true),
        "gamepad.gamepad_flycam_move_stick" => (true, false),
        _ => (true, true),
    };
    let native = cx.st.joystick.selected_gamepad(&cx.s.gamepad).map(|device| {
        (
            device.is_gamepad,
            device.axis_count,
            device.gamepad_sticks,
            device.gamepad_triggers,
            device.gamepad_dpad,
        )
    });
    let get = field.get;
    let track = Custom {
        path: field.path,
        modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| get(s) != get(d),
        reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
            let device = bindings_device(native);
            if first_person {
                s.gamepad.set_first_person_stick(device.as_ref(), camera, *get(d));
            } else {
                s.gamepad.set_stick(device.as_ref(), flycam, camera, *get(d));
            }
        },
    };
    cx.row_with(label, "", &track, |ui, s, st, _| {
        let mut value = *(field.get)(s);
        let mut changed = false;
        egui::ComboBox::from_id_salt(field.path)
            .selected_text(match value {
                S::Left => tr!("joystick-left-stick"),
                S::Right => tr!("joystick-right-stick"),
                S::None => tr!("common-none"),
                S::Custom => tr!("joystick-custom-mapping"),
            })
            .show_ui(ui, |ui| {
                for (choice, label) in [
                    (S::None, tr!("common-none")),
                    (S::Left, tr!("joystick-left-stick")),
                    (S::Right, tr!("joystick-right-stick")),
                ] {
                    changed |= ui.selectable_value(&mut value, choice, label).changed();
                }
            });
        if changed {
            let device = st.joystick.selected_gamepad(&s.gamepad);
            if first_person {
                s.gamepad.set_first_person_stick(device, camera, value);
            } else {
                s.gamepad.set_stick(device, flycam, camera, value);
            }
        }
        changed
    });
}

fn camera_tuning(s: &crate::joystick::Settings, mode: usize) -> &crate::joystick::Tuning {
    match mode {
        2 => &s.first_person.tuning,
        1 => &s.flycam,
        _ => &s.avatar,
    }
}

fn camera_tuning_mut(s: &mut crate::joystick::Settings, mode: usize) -> &mut crate::joystick::Tuning {
    match mode {
        2 => &mut s.first_person.tuning,
        1 => &mut s.flycam,
        _ => &mut s.avatar,
    }
}

fn camera_inverted(s: &crate::joystick::Settings, mode: usize, channel: usize) -> bool {
    if mode == 2 {
        s.first_person.tuning.gain[channel].is_sign_negative()
    } else {
        s.axis_inverted(channel)
    }
}

fn set_camera_inverted(s: &mut crate::joystick::Settings, mode: usize, channel: usize, inverted: bool) {
    if mode == 2 {
        s.first_person.tuning.gain[channel] = s.first_person.tuning.gain[channel]
            .abs()
            .copysign(if inverted { -1.0 } else { 1.0 });
    } else {
        s.set_axis_inverted(channel, inverted);
    }
}

fn gamepad_camera_gain(cx: &mut Cx, mode: usize, channel: usize) {
    use aurora_i18n::tr;
    let path = match (mode, channel) {
        (2, 4) => "gamepad.first_person.tuning.gain[4]",
        (2, _) => "gamepad.first_person.tuning.gain[5]",
        (0, 4) => "gamepad.avatar.gain[4]",
        (0, _) => "gamepad.avatar.gain[5]",
        (1, 4) => "gamepad.flycam.gain[4]",
        _ => "gamepad.flycam.gain[5]",
    };
    let track = Custom {
        path,
        modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| {
            let s = camera_tuning(&s.gamepad, mode);
            let d = camera_tuning(&d.gamepad, mode);
            s.gain[channel].abs() != d.gain[channel].abs()
        },
        reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
            let sign = if camera_inverted(&s.gamepad, mode, channel) { -1.0 } else { 1.0 };
            let defaults = camera_tuning(&d.gamepad, mode);
            let tuning = camera_tuning_mut(&mut s.gamepad, mode);
            tuning.gain[channel] = defaults.gain[channel].abs() * sign;
        },
    };
    cx.row(
        if channel == 4 {
            tr!("joystick-camera-vertical-sensitivity")
        } else {
            tr!("joystick-camera-horizontal-sensitivity")
        },
        if mode == 2 {
            tr!("joystick-first-person-sensitivity-hint")
        } else {
            tr!("joystick-gamepad-sensitivity-hint")
        },
        &track,
        |ui, s| {
            let sign = if camera_inverted(&s.gamepad, mode, channel) { -1.0 } else { 1.0 };
            let tuning = camera_tuning_mut(&mut s.gamepad, mode);
            let mut gain = tuning.gain[channel].abs();
            let changed = super::super::slider(ui, &mut gain, 0.0..=20.0, "");
            if changed {
                tuning.gain[channel] = gain * sign;
            }
            changed
        },
    );
}

fn gamepad_camera_inversion(cx: &mut Cx, mode: usize, channel: usize) {
    use aurora_i18n::tr;
    let palette = cx.p;
    let track = Custom {
        path: if mode == 2 && channel == 4 {
            "gamepad.first_person.vertical_inverted"
        } else if mode == 2 {
            "gamepad.first_person.horizontal_inverted"
        } else if channel == 4 {
            "gamepad.camera_vertical_inverted"
        } else {
            "gamepad.camera_horizontal_inverted"
        },
        modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| {
            camera_inverted(&s.gamepad, mode, channel) != camera_inverted(&d.gamepad, mode, channel)
        },
        reset: move |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
            set_camera_inverted(&mut s.gamepad, mode, channel, camera_inverted(&d.gamepad, mode, channel))
        },
    };
    cx.row(
        if channel == 4 {
            tr!("joystick-gamepad-invert-vertical")
        } else {
            tr!("joystick-gamepad-invert-horizontal")
        },
        if channel == 4 {
            tr!("joystick-gamepad-invert-vertical-hint")
        } else {
            tr!("joystick-gamepad-invert-horizontal-hint")
        },
        &track,
        |ui, s| {
            let mut inverted = camera_inverted(&s.gamepad, mode, channel);
            let changed = widgets::switch(ui, &palette, &mut inverted).changed();
            if changed {
                set_camera_inverted(&mut s.gamepad, mode, channel, inverted);
            }
            changed
        },
    );
}

fn gamepad_style_selector(cx: &mut Cx) {
    use aurora_i18n::tr;
    let palette = cx.p;
    let field = field!(gamepad.gamepad_style);
    cx.row_with(tr!("joystick-controller-style"), "", &field, |ui, s, st, _| {
        let detected = st.joystick.selected_gamepad(&s.gamepad);
        let mut value = effective_gamepad_style(s.gamepad.gamepad_style, detected);
        let mut changed = false;
        ui.horizontal(|ui| {
            for (style, label) in [
                (crate::joystick::GamepadStyle::Xbox, tr!("joystick-controller-xbox")),
                (crate::joystick::GamepadStyle::Playstation, tr!("joystick-controller-playstation")),
                (crate::joystick::GamepadStyle::Generic, tr!("joystick-controller-generic")),
            ] {
                changed |= widgets::radio_value(ui, &palette, &mut value, style, label).changed();
            }
        });
        if changed {
            s.gamepad.gamepad_style = value;
        }
        changed
    });
}

pub fn gamepad_page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    if tab == 0 {
        device_dropdown(cx, true);
        cx.toggle(
            tr!("joystick-enable-gamepad"),
            tr!("joystick-enable-gamepad-hint"),
            &field!(gamepad.enabled),
        );
        cx.group(tr!("joystick-avatar-mode"), |cx| {
            gamepad_camera_gain(cx, 0, 5);
            gamepad_camera_gain(cx, 0, 4);
            gamepad_camera_inversion(cx, 0, 5);
            gamepad_camera_inversion(cx, 0, 4);
        });
        cx.group(tr!("joystick-first-person-mode"), |cx| {
            gamepad_camera_gain(cx, 2, 5);
            gamepad_camera_gain(cx, 2, 4);
            gamepad_camera_inversion(cx, 2, 5);
            gamepad_camera_inversion(cx, 2, 4);
        });
        cx.group(tr!("joystick-flycam-section"), |cx| {
            gamepad_camera_gain(cx, 1, 5);
            gamepad_camera_gain(cx, 1, 4);
            cx.slider(
                tr!("joystick-flycam-travel-speed"),
                tr!("joystick-flycam-travel-speed-hint"),
                &field!(gamepad.flycam_speed),
                0.1..=5.0,
                "×",
            );
            cx.slider(
                tr!("joystick-gamepad-flycam-slowdown-speed"),
                tr!("joystick-gamepad-flycam-speed-hint"),
                &field!(gamepad.gamepad_flycam_slowdown_speed),
                0.05..=1.0,
                "×",
            );
            cx.slider(
                tr!("joystick-gamepad-flycam-speedup-speed"),
                tr!("joystick-gamepad-flycam-speed-hint"),
                &field!(gamepad.gamepad_flycam_speedup_speed),
                1.0..=50.0,
                "×",
            );
            cx.toggle(
                tr!("joystick-auto-level"),
                tr!("joystick-auto-level-hint"),
                &field!(gamepad.auto_level),
            );
        });
    } else {
        gamepad_style_selector(cx);
        gamepad_button_binding(cx, tr!("joystick-control-reset-view"), field!(gamepad.reset_view_button));
        cx.group(tr!("joystick-avatar-mode"), |cx| {
            gamepad_stick_binding(cx, tr!("joystick-control-move-avatar"), field!(gamepad.gamepad_move_stick));
            gamepad_stick_binding(cx, tr!("joystick-control-move-camera"), field!(gamepad.gamepad_camera_stick));
            gamepad_button_binding(cx, tr!("joystick-control-jump"), field!(gamepad.jump_button));
            gamepad_button_binding(cx, tr!("joystick-control-crouch"), field!(gamepad.crouch_button));
            gamepad_button_binding(cx, tr!("joystick-control-fly-up"), field!(gamepad.rise_button));
            gamepad_button_binding(cx, tr!("joystick-control-fly-down"), field!(gamepad.fall_button));
            gamepad_button_binding(cx, tr!("joystick-control-flycam"), field!(gamepad.flycam_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-zoom-in"), field!(gamepad.gamepad_avatar_zoom_in_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-zoom-out"), field!(gamepad.gamepad_avatar_zoom_out_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-pan-up"), field!(gamepad.gamepad_avatar_pan_up_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-pan-down"), field!(gamepad.gamepad_avatar_pan_down_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-pan-left"), field!(gamepad.gamepad_avatar_pan_left_button));
            gamepad_button_binding(
                cx,
                tr!("joystick-avatar-pan-right"),
                field!(gamepad.gamepad_avatar_pan_right_button),
            );
            gamepad_button_binding(cx, tr!("joystick-avatar-next"), field!(gamepad.gamepad_avatar_next_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-previous"), field!(gamepad.gamepad_avatar_previous_button));
        });
        cx.group(tr!("joystick-first-person-mode"), |cx| {
            gamepad_stick_binding(cx, tr!("joystick-control-move-avatar"), field!(gamepad.first_person.move_stick));
            gamepad_stick_binding(cx, tr!("joystick-control-move-camera"), field!(gamepad.first_person.camera_stick));
            gamepad_button_binding(cx, tr!("joystick-control-jump"), field!(gamepad.first_person.jump_button));
            gamepad_button_binding(cx, tr!("joystick-control-crouch"), field!(gamepad.first_person.crouch_button));
            gamepad_button_binding(cx, tr!("joystick-control-fly-up"), field!(gamepad.first_person.rise_button));
            gamepad_button_binding(cx, tr!("joystick-control-fly-down"), field!(gamepad.first_person.fall_button));
            gamepad_button_binding_with_fallback(
                cx,
                tr!("joystick-control-flycam"),
                field!(gamepad.first_person.view_button),
                Some(field!(gamepad.flycam_button)),
            );
            gamepad_button_binding(cx, tr!("joystick-avatar-zoom-in"), field!(gamepad.first_person.zoom_in_button));
            gamepad_button_binding(cx, tr!("joystick-avatar-zoom-out"), field!(gamepad.first_person.zoom_out_button));
        });
        cx.group(tr!("joystick-flycam-section"), |cx| {
            gamepad_stick_binding(cx, tr!("joystick-control-flycam-move"), field!(gamepad.gamepad_flycam_move_stick));
            gamepad_stick_binding(
                cx,
                tr!("joystick-control-flycam-camera"),
                field!(gamepad.gamepad_flycam_camera_stick),
            );
            gamepad_button_binding_with_fallback(
                cx,
                tr!("joystick-control-flycam"),
                field!(gamepad.gamepad_flycam_view_button),
                Some(field!(gamepad.flycam_button)),
            );
            gamepad_button_binding_with_fallback(
                cx,
                tr!("joystick-control-flycam-up"),
                field!(gamepad.flycam_rise_button),
                Some(field!(gamepad.rise_button)),
            );
            gamepad_button_binding_with_fallback(
                cx,
                tr!("joystick-control-flycam-down"),
                field!(gamepad.flycam_fall_button),
                Some(field!(gamepad.fall_button)),
            );
            gamepad_button_binding(cx, tr!("joystick-zoom-in-button"), field!(gamepad.zoom_in_button));
            gamepad_button_binding(cx, tr!("joystick-zoom-out-button"), field!(gamepad.zoom_out_button));
            gamepad_button_binding(
                cx,
                tr!("joystick-control-travel-slowdown"),
                field!(gamepad.gamepad_flycam_slowdown_button),
            );
            gamepad_button_binding(
                cx,
                tr!("joystick-control-travel-speedup"),
                field!(gamepad.gamepad_flycam_speedup_button),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::options::{OptionsResult, OptionsUi, Page, cx::Mode};
    use crate::ui::widgets::elide_tests::{Frames, Text};
    use std::time::{Duration, Instant};

    type Render = fn(&mut Frames, &mut crate::settings::Settings, &mut OptionsUi, Vec<egui::Event>) -> (Vec<Text>, bool);

    fn fixture() -> (crate::settings::Settings, OptionsUi) {
        let state = OptionsUi {
            joystick: crate::joystick::Snapshot {
                devices: vec![crate::joystick::Device::new("test".into(), "Test controller".into(), 8)],
                updated: Some(Instant::now()),
                ..Default::default()
            },
            ..Default::default()
        };
        (crate::settings::Settings::default(), state)
    }

    fn frame(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        events: Vec<egui::Event>,
    ) -> (Vec<Text>, bool) {
        state.joystick.updated = Some(Instant::now());
        let defaults = crate::settings::Settings::default();
        let mut result = OptionsResult::default();
        let mut changed = false;
        let texts = frames.run(events, |ui| {
            ui.set_width(700.0);
            let mut cx = Cx::new(
                ui,
                crate::theme::Theme::default().palette(),
                settings,
                &defaults,
                state,
                &mut result,
                Mode::Show,
                "",
            );
            button(&mut cx, aurora_i18n::tr!("joystick-flycam-button"), field!(joystick.flycam_button));
            changed |= cx.changed;
        });
        (texts, changed)
    }

    fn click(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        texts: &[Text],
        label: &str,
    ) -> Vec<Text> {
        click_with(frames, settings, state, texts, label, frame)
    }

    fn zoom_modes_frame(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        events: Vec<egui::Event>,
    ) -> (Vec<Text>, bool) {
        state.joystick.updated = Some(Instant::now());
        let defaults = crate::settings::Settings::default();
        let mut result = OptionsResult::default();
        let mut changed = false;
        let texts = frames.run(events, |ui| {
            ui.set_width(800.0);
            let mut cx = Cx::new(
                ui,
                crate::theme::Theme::default().palette(),
                settings,
                &defaults,
                state,
                &mut result,
                Mode::Show,
                "",
            );
            zoom_settings(&mut cx);
            changed |= cx.changed;
        });
        (texts, changed)
    }

    #[test]
    fn joystick_inherited_view_dropdowns_resolve_native_semantic_defaults() {
        use crate::joystick::{Device, GamepadButton, GamepadStyle};
        let (mut settings, mut state) = fixture();
        let mut buttons = GamepadButton::STANDARD.to_vec();
        buttons.swap(0, 7);
        state.joystick.devices = vec![Device::gamepad(
            "native".into(),
            "Pad".into(),
            8,
            [[Some(0), Some(1)], [Some(3), Some(4)]],
            [Some(6), Some(7)],
            GamepadStyle::Xbox,
            buttons,
        )];
        let defaults = crate::settings::Settings::default();
        let mut frames = Frames::new();
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frames.run(Vec::new(), |ui| {
                ui.set_width(700.0);
                let mut result = OptionsResult::default();
                let mut cx = Cx::new(
                    ui,
                    crate::theme::Theme::default().palette(),
                    &mut settings,
                    &defaults,
                    &mut state,
                    &mut result,
                    Mode::Show,
                    "",
                );
                gamepad_button_binding_with_fallback(
                    &mut cx,
                    "First person",
                    field!(gamepad.first_person.view_button),
                    Some(field!(gamepad.flycam_button)),
                );
                gamepad_button_binding_with_fallback(
                    &mut cx,
                    "Flycam",
                    field!(gamepad.gamepad_flycam_view_button),
                    Some(field!(gamepad.flycam_button)),
                );
            });
        }
        assert_eq!(
            texts.iter().filter(|text| text.source == "A").count(),
            2,
            "both inherited view actions show A despite its nonstandard native index"
        );
    }

    #[test]
    fn joystick_zoom_mode_ui_assigns_both_buttons_and_switches_to_a_captured_modifier() {
        use aurora_i18n::tr;
        let (mut settings, mut state) = fixture();
        settings.joystick.zoom_mode = crate::joystick::ZoomMode::Buttons;
        let mut frames = Frames::new();
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = zoom_modes_frame(&mut frames, &mut settings, &mut state, Vec::new()).0;
        }
        assert!(
            texts.iter().any(|text| text.source == tr!("joystick-direct-zoom")),
            "two-buttons mode exposes absolute zoom"
        );
        for binding in [8, 46] {
            click_with(
                &mut frames,
                &mut settings,
                &mut state,
                &texts,
                tr!("joystick-button-assign"),
                zoom_modes_frame,
            );
            assert!(state.joystick_capturing());
            if binding == 8 {
                state.joystick.devices[0].button(8, true);
                state.joystick.devices[0].button(8, false);
            } else {
                state.joystick.devices[0].axes[7] = 0.8;
                zoom_modes_frame(&mut frames, &mut settings, &mut state, Vec::new());
                state.joystick.devices[0].axes[7] = 0.0;
            }
            let (next, changed) = zoom_modes_frame(&mut frames, &mut settings, &mut state, Vec::new());
            texts = next;
            assert!(changed && !state.joystick_capturing());
        }
        assert_eq!((settings.joystick.zoom_in_button, settings.joystick.zoom_out_button), (8, 46));
        texts = click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-zoom-mode-buttons"),
            zoom_modes_frame,
        );
        texts = click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-zoom-mode-hold-axis"),
            zoom_modes_frame,
        );
        assert_eq!(settings.joystick.zoom_mode, crate::joystick::ZoomMode::HoldAxis);
        assert!(
            texts.iter().any(|text| text.source == tr!("joystick-direct-zoom")),
            "hold-plus-axis mode exposes absolute zoom"
        );
        assert_eq!(settings.joystick.flycam.gain[6], 1.0);
        assert!(!texts.iter().any(|text| text.source == tr!("joystick-zoom-in-button")));
        state.joystick.devices[0].axes[6] = 0.8;
        click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-button-assign"),
            zoom_modes_frame,
        );
        state.joystick.devices[0].axes[6] = 0.0;
        zoom_modes_frame(&mut frames, &mut settings, &mut state, Vec::new());
        assert!(state.joystick_capturing(), "a trigger held before capture must be released first");
        state.joystick.devices[0].axes[6] = 0.8;
        zoom_modes_frame(&mut frames, &mut settings, &mut state, Vec::new());
        state.joystick.devices[0].axes[6] = 0.0;
        let (_, changed) = zoom_modes_frame(&mut frames, &mut settings, &mut state, Vec::new());
        assert!(changed && !state.joystick_capturing());
        assert_eq!(settings.joystick.zoom_hold_button, 44);
        assert_eq!((settings.joystick.zoom_in_button, settings.joystick.zoom_out_button), (8, 46));
    }

    fn click_with(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        texts: &[Text],
        label: &str,
        render: Render,
    ) -> Vec<Text> {
        let pos = texts
            .iter()
            .find(|text| text.source == label)
            .unwrap_or_else(|| {
                panic!(
                    "button {label:?} not found among {:?}",
                    texts.iter().map(|text| &text.source).collect::<Vec<_>>()
                )
            })
            .rect
            .center();
        render(frames, settings, state, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            render(
                frames,
                settings,
                state,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
            );
        }
        render(frames, settings, state, Vec::new()).0
    }

    fn axis_frame(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        events: Vec<egui::Event>,
    ) -> (Vec<Text>, bool) {
        axis_frame_for(frames, settings, state, events, 0)
    }

    fn zoom_frame(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        events: Vec<egui::Event>,
    ) -> (Vec<Text>, bool) {
        axis_frame_for(frames, settings, state, events, 6)
    }

    fn axis_frame_for(
        frames: &mut Frames,
        settings: &mut crate::settings::Settings,
        state: &mut OptionsUi,
        events: Vec<egui::Event>,
        channel: usize,
    ) -> (Vec<Text>, bool) {
        state.joystick.updated = Some(Instant::now());
        let mut changed = false;
        let texts = frames.run(events, |ui| {
            ui.set_width(700.0);
            changed |= axis_binding(
                ui,
                &mut settings.joystick,
                state,
                channel,
                &crate::theme::Theme::default().palette(),
            );
            let inverted = settings.joystick.axis_inverted(channel);
            changed |= tuning(
                ui,
                aurora_i18n::tr!("joystick-avatar"),
                &mut settings.joystick.avatar,
                channel,
                inverted,
            );
        });
        (texts, changed)
    }

    #[test]
    fn joystick_axis_assignment_click_captures_dominant_motion_and_inversion_changes_real_input() {
        use aurora_i18n::tr;
        let (mut settings, mut state) = fixture();
        let mut frames = Frames::new();
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = axis_frame(&mut frames, &mut settings, &mut state, Vec::new()).0;
        }
        click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            &tr!("joystick-axis-value", axis = 1),
            axis_frame,
        );
        assert!(state.joystick_capturing());
        state.joystick.devices[0].axes[0] = 0.06;
        state.joystick.devices[0].axes[7] = -0.8;
        axis_frame(&mut frames, &mut settings, &mut state, Vec::new());
        assert_eq!(
            settings.joystick.mapping[0], 1,
            "keep the previous binding while observing the gesture"
        );
        state.joystick.devices[0].axes = [0.0; crate::joystick::AXES];
        let (next, changed) = axis_frame(&mut frames, &mut settings, &mut state, Vec::new());
        texts = next;
        assert!(changed && !state.joystick_capturing());
        assert_eq!(settings.joystick.mapping[0], 7);
        texts = click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            &tr!("joystick-axis-value", axis = 7),
            axis_frame,
        );
        texts = click_with(&mut frames, &mut settings, &mut state, &texts, tr!("common-cancel"), axis_frame);
        assert_eq!(settings.joystick.mapping[0], 7);
        assert!(!state.joystick_capturing());
        texts = click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-axis-invert"),
            axis_frame,
        );
        assert_eq!(settings.joystick.avatar.gain[0], -1.0);
        assert_eq!(
            settings.joystick.flycam.gain[0],
            -crate::joystick::Settings::default().flycam.gain[0]
        );
        let mut controller = crate::joystick::Controller::default();
        let input_settings = crate::joystick::Settings {
            enabled: true,
            ..settings.joystick.clone()
        };
        controller.sample(&state.joystick, &input_settings, true, false, 0.1);
        state.joystick.devices[0].axes[7] = -0.8;
        assert!(controller.sample(&state.joystick, &input_settings, true, false, 0.1).axes[0] < 0.0);
        state.joystick.devices[0].axes[7] = 0.0;
        texts = click_with(&mut frames, &mut settings, &mut state, &texts, tr!("common-none"), axis_frame);
        assert_eq!(settings.joystick.mapping[0], -1);
        click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-button-assign"),
            axis_frame,
        );
        assert!(state.joystick_capturing());
        state.open(Page::Camera, 0);
        assert!(!state.joystick_capturing());
        assert_eq!(settings.joystick.mapping[0], -1);
    }

    #[test]
    fn joystick_zoom_assignment_enables_a_zero_gain_and_changes_the_flycam_lens() {
        use aurora_i18n::tr;
        for previous_gain in [0.0f32, -0.0, 1.7] {
            let (mut settings, mut state) = fixture();
            settings.joystick.flycam.gain[6] = previous_gain;
            let mut frames = Frames::new();
            let mut texts = Vec::new();
            for _ in 0..3 {
                texts = zoom_frame(&mut frames, &mut settings, &mut state, Vec::new()).0;
            }
            click_with(
                &mut frames,
                &mut settings,
                &mut state,
                &texts,
                tr!("joystick-button-assign"),
                zoom_frame,
            );
            assert!(state.joystick_capturing());
            state.joystick.devices[0].axes[2] = 0.8;
            zoom_frame(&mut frames, &mut settings, &mut state, Vec::new());
            state.joystick.devices[0].axes[2] = 0.0;
            let (_, changed) = zoom_frame(&mut frames, &mut settings, &mut state, Vec::new());
            assert!(changed && !state.joystick_capturing());
            assert_eq!(settings.joystick.mapping[6], 2);
            let gain = settings.joystick.flycam.gain[6];
            assert_eq!(
                gain,
                if previous_gain == 0.0 {
                    1.0f32.copysign(previous_gain)
                } else {
                    previous_gain
                }
            );
            let mut controller = crate::joystick::Controller::default();
            settings.joystick.enabled = true;
            controller.sample(&state.joystick, &settings.joystick, true, true, 0.1);
            state.joystick.devices[0].axes[2] = 0.8;
            let motion = controller.sample(&state.joystick, &settings.joystick, true, true, 0.1);
            let mut camera = crate::joystick::flycam::Flycam::from_view(glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z, 1.0);
            camera.step(motion.axes, &settings.joystick, 0.1);
            assert!(
                (camera.fov - 1.0).abs() > 0.01,
                "a new zoom assignment must change the field of view"
            );
            assert_eq!(camera.fov < 1.0, gain > 0.0);
        }
    }

    #[test]
    fn joystick_shared_axis_inversion_preserves_gain_magnitudes_and_a_disabled_axis_choice() {
        use aurora_i18n::tr;
        let (mut settings, mut state) = fixture();
        settings.joystick.avatar.gain[0] = 0.0;
        let mut frames = Frames::new();
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = axis_frame(&mut frames, &mut settings, &mut state, Vec::new()).0;
        }
        click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-axis-invert"),
            axis_frame,
        );
        assert!(settings.joystick.avatar.gain[0].is_sign_negative());
        assert_eq!(
            settings.joystick.flycam.gain[0],
            -crate::joystick::Settings::default().flycam.gain[0]
        );
        let json = serde_json::to_string(&settings.joystick).expect("controller settings");
        let restored: crate::joystick::Settings = serde_json::from_str(&json).expect("restored controller settings");
        assert!(
            restored.avatar.gain[0].is_sign_negative(),
            "inversion survives a restart before setting a non-zero gain"
        );
        assert!(restored.axis_inverted(0));
        settings.joystick.avatar.gain[0] = -2.0;
        texts = axis_frame(&mut frames, &mut settings, &mut state, Vec::new()).0;
        assert_eq!(settings.joystick.avatar.gain[0], -2.0);
        click_with(
            &mut frames,
            &mut settings,
            &mut state,
            &texts,
            tr!("joystick-axis-invert"),
            axis_frame,
        );
        assert_eq!(settings.joystick.avatar.gain[0], 2.0);
        assert_eq!(
            settings.joystick.flycam.gain[0],
            crate::joystick::Settings::default().flycam.gain[0]
        );
    }

    #[test]
    fn joystick_button_capture_click_assigns_native_buttons_and_cancels_without_losing_the_binding() {
        use aurora_i18n::tr;
        let (mut settings, mut state) = fixture();
        assert!(
            !settings.joystick.enabled,
            "assignment also works with controller movement disabled"
        );
        state.joystick.devices[0].button(4, true);
        let mut frames = Frames::new();
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frame(&mut frames, &mut settings, &mut state, Vec::new()).0;
        }
        for button in [8i8, 9, 31] {
            let label = tr!("joystick-button-value", button = settings.joystick.flycam_button);
            click(&mut frames, &mut settings, &mut state, &texts, &label);
            assert!(state.joystick_capturing());
            frame(&mut frames, &mut settings, &mut state, Vec::new());
            assert!(state.joystick_capturing(), "a button already held is ignored");
            state.joystick.devices[0].button(button as usize, true);
            state.joystick.devices[0].button(button as usize, false);
            let (next, changed) = frame(&mut frames, &mut settings, &mut state, Vec::new());
            texts = next;
            assert!(changed, "the preferences save path is notified");
            assert_eq!(settings.joystick.flycam_button, button);
            assert!(!state.joystick_capturing());
        }
        let label = tr!("joystick-button-value", button = settings.joystick.flycam_button);
        texts = click(&mut frames, &mut settings, &mut state, &texts, &label);
        texts = click(&mut frames, &mut settings, &mut state, &texts, tr!("common-cancel"));
        assert_eq!(settings.joystick.flycam_button, 31);
        assert!(!state.joystick_capturing());
        texts = click(&mut frames, &mut settings, &mut state, &texts, tr!("common-none"));
        assert_eq!(settings.joystick.flycam_button, -1);
        click(&mut frames, &mut settings, &mut state, &texts, tr!("joystick-button-assign"));
        assert!(state.joystick_capturing());
        state.open(Page::Camera, 0);
        assert!(!state.joystick_capturing());
        assert_eq!(settings.joystick.flycam_button, -1);
    }

    #[test]
    fn joystick_button_capture_rejects_changed_disconnected_stale_and_restarted_devices() {
        for scenario in ["changed", "disconnected", "stale", "restarted"] {
            let (settings, mut state) = fixture();
            state.joystick.devices[0].button(2, true);
            let capture = ButtonCapture::new("joystick.flycam_button", &state.joystick.devices[0]);
            match scenario {
                "changed" => state.joystick.devices[0].id = "other".into(),
                "disconnected" => state.joystick.devices.clear(),
                "stale" => state.joystick.updated = Some(Instant::now() - Duration::from_secs(1)),
                "restarted" => state.joystick.devices[0].presses = [0; crate::joystick::BUTTONS],
                _ => unreachable!(),
            }
            assert_eq!(capture.poll(&state.joystick, &settings.joystick), Err(()), "{scenario}");
        }
    }
}
