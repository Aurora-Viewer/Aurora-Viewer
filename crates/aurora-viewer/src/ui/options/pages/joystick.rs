//! LLFloaterJoystick settings, with native axis and button monitoring.

use super::super::cx::{Custom, Cx, F, Many, Nothing, field};
use crate::ui::widgets;
use std::time::Instant;

pub(in crate::ui::options) enum JoystickCapture {
    Button(ButtonCapture),
    Axis(AxisCapture),
}

impl JoystickCapture {
    pub fn hide(&mut self) {
        match self {
            Self::Button(capture) => capture.visible = false,
            Self::Axis(capture) => capture.visible = false,
        }
    }

    pub fn visible(&self) -> bool {
        match self {
            Self::Button(capture) => capture.visible,
            Self::Axis(capture) => capture.visible,
        }
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
            .selected(settings)
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
    use aurora_i18n::tr;
    cx.row_with(label, tr!("joystick-button-hint"), &field, |ui, s, st, _| {
        let mut value = *(field.get)(s);
        let mut changed = false;
        let captured = match st.joystick_capture.as_mut() {
            Some(JoystickCapture::Button(capture)) if capture.path == field.path => {
                capture.visible = true;
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
        let capturing = matches!(&st.joystick_capture, Some(JoystickCapture::Button(capture)) if capture.path == field.path);
        let available = st.joystick.selected(&s.joystick).is_some() && st.joystick.updated.is_some_and(|at| at.elapsed().as_millis() < 250);
        let text = if capturing {
            tr!("joystick-button-press").to_owned()
        } else if value < 0 {
            tr!("joystick-button-assign").to_owned()
        } else {
            tr!("joystick-button-value", button = value)
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(available, egui::Button::new(text).min_size(egui::vec2(126.0, 24.0)))
                .clicked()
            {
                st.cancel_capture();
                if !capturing && let Some(device) = st.joystick.selected(&s.joystick) {
                    st.joystick_capture = Some(JoystickCapture::Button(ButtonCapture::new(field.path, device)));
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

fn axis_binding(ui: &mut egui::Ui, settings: &mut crate::joystick::Settings, st: &mut super::super::OptionsUi, channel: usize) -> bool {
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
    let available = st.joystick.selected(settings).is_some_and(|device| device.axis_count != 0)
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
            if !capturing && let Some(device) = st.joystick.selected(settings) {
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
    });
    changed
}

fn tuning(ui: &mut egui::Ui, p: &crate::theme::Palette, label: &str, tune: &mut crate::joystick::Tuning, channel: usize) -> bool {
    use aurora_i18n::tr;
    let mut changed = false;
    let mut inverted = tune.gain[channel].is_sign_negative();
    let mut gain = tune.gain[channel].abs();
    let mut gain_changed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.label(label);
        gain_changed |= widgets::switch_labeled(ui, p, &mut inverted, tr!("joystick-axis-invert")).changed();
    });
    ui.horizontal(|ui| {
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
        // Keep the existing signed-gain format, including a signed zero
        // when inversion is chosen before enabling an axis's gain.
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
    let column_width = (ui.spacing().interact_size.x * 2.0 + 8.0).max(210.0);
    let columns = ((ui.available_width() + 16.0) / (column_width + 16.0)).floor().clamp(1.0, 3.0) as usize;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for start in (0..3).step_by(columns) {
            ui.columns(columns, |cells| {
                for (offset, ui) in cells.iter_mut().enumerate() {
                    let kind = start + offset;
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.push_id((channel, kind), |ui| match kind {
                        0 => {
                            ui.horizontal(|ui| {
                                ui.set_min_height(24.0);
                                ui.weak(tr!("joystick-axis-assignment"));
                            });
                            changed |= axis_binding(ui, settings, st, channel);
                        }
                        1 if matches!(channel, 3 | 6) => {
                            ui.horizontal(|ui| {
                                ui.set_min_height(24.0);
                                ui.weak(tr!("joystick-avatar"));
                            });
                            ui.weak(tr!("joystick-axis-camera-only"));
                        }
                        1 => changed |= tuning(ui, p, tr!("joystick-avatar"), &mut settings.avatar, channel),
                        2 => changed |= tuning(ui, p, tr!("common-camera"), &mut settings.flycam, channel),
                        _ => {}
                    });
                }
            });
        }
        ui.add_space(4.0);
    });
    changed
}

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("joystick-title"), |cx| {
        cx.toggle(tr!("joystick-enabled"), tr!("joystick-enable-hint"), &field!(joystick.enabled));
        cx.row_with(tr!("joystick-device"), "", &field!(joystick.device), |ui, s, st, _| {
            let name = if s.joystick.device.is_empty() {
                tr!("joystick-first-device")
            } else {
                st.joystick
                    .selected(&s.joystick)
                    .map_or(tr!("joystick-disconnected"), |d| d.name.as_str())
            };
            let mut changed = false;
            egui::ComboBox::from_id_salt("joystick-device")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut s.joystick.device, String::new(), tr!("joystick-first-device"))
                        .changed();
                    for d in &st.joystick.devices {
                        changed |= ui.selectable_value(&mut s.joystick.device, d.id.clone(), &d.name).changed();
                    }
                });
            changed
        });
        cx.row_with(tr!("joystick-monitor"), tr!("joystick-center-hint"), &Nothing, |ui, s, st, _| {
            ui.vertical(|ui| {
                ui.set_width(210.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                if let Some(d) = st.joystick.selected(&s.joystick) {
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
                        s.joystick.gamepad();
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
            let available = s.joystick.enabled && s.joystick.flycam_enabled && st.joystick.selected(&s.joystick).is_some();
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
        cx.toggle(
            tr!("joystick-direct-zoom"),
            tr!("joystick-direct-zoom-hint"),
            &field!(joystick.direct_zoom),
        );
    });
    cx.group(tr!("joystick-axes"), |cx| {
        let zoom_hint = format!("{} {}", tr!("joystick-axis-hint"), tr!("joystick-zoom-hint"));
        let names = [
            tr!("joystick-forward"),
            tr!("joystick-slide"),
            tr!("joystick-up"),
            tr!("joystick-roll"),
            tr!("joystick-pitch"),
            tr!("joystick-yaw"),
            tr!("joystick-zoom"),
        ];
        let paths = [
            "joystick.mapping[0]",
            "joystick.mapping[1]",
            "joystick.mapping[2]",
            "joystick.mapping[3]",
            "joystick.mapping[4]",
            "joystick.mapping[5]",
            "joystick.mapping[6]",
        ];
        for (n, label) in names.into_iter().enumerate() {
            let track = Custom {
                path: paths[n],
                modified: move |s: &crate::settings::Settings, d: &crate::settings::Settings| {
                    let (s, d) = (&s.joystick, &d.joystick);
                    s.mapping[n] != d.mapping[n]
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
            cx.below().row_with(
                label,
                if n == 6 { &zoom_hint } else { tr!("joystick-axis-hint") },
                &track,
                |ui, s, st, _| axis_controls(ui, &mut s.joystick, st, &p, n),
            );
        }
    });
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
            ui.set_width(210.0);
            changed |= axis_binding(ui, &mut settings.joystick, state, channel);
            changed |= tuning(
                ui,
                &crate::theme::Theme::default().palette(),
                aurora_i18n::tr!("joystick-avatar"),
                &mut settings.joystick.avatar,
                channel,
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
            crate::joystick::Settings::default().flycam.gain[0]
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
    fn joystick_axis_inversion_preserves_old_negative_gains_and_a_disabled_axis_choice() {
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
        let json = serde_json::to_string(&settings.joystick).expect("controller settings");
        let restored: crate::joystick::Settings = serde_json::from_str(&json).expect("restored controller settings");
        assert!(
            restored.avatar.gain[0].is_sign_negative(),
            "inversion survives a restart before setting a non-zero gain"
        );
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
