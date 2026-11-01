//! LLFloaterJoystick settings, with native axis and button monitoring.

use super::super::cx::{Custom, Cx, F, Many, Nothing, field};
use crate::ui::widgets;

pub(in crate::ui::options) struct ButtonCapture {
    path: &'static str,
    device: String,
    presses: [u64; crate::joystick::BUTTONS],
    pub visible: bool,
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

fn button(cx: &mut Cx, label: &str, field: F<i8>) {
    use aurora_i18n::tr;
    cx.row_with(label, tr!("joystick-button-hint"), &field, |ui, s, st, _| {
        let mut value = *(field.get)(s);
        let mut changed = false;
        let captured = st
            .joystick_capture
            .as_mut()
            .filter(|capture| capture.path == field.path)
            .map(|capture| {
                capture.visible = true;
                capture.poll(&st.joystick, &s.joystick)
            });
        match captured {
            Some(Ok(Some(button))) => {
                value = button;
                changed = true;
                st.cancel_capture();
            }
            Some(Err(())) => st.cancel_capture(),
            _ => {}
        }
        let capturing = st.joystick_capture.as_ref().is_some_and(|capture| capture.path == field.path);
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
                    st.joystick_capture = Some(ButtonCapture::new(field.path, device));
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
            cx.row(
                label,
                if n == 6 { &zoom_hint } else { tr!("joystick-axis-hint") },
                &track,
                |ui, s| {
                    let s = &mut s.joystick;
                    let mut changed = false;
                    ui.vertical(|ui| {
                        ui.set_width(210.0);
                        egui::ComboBox::from_id_salt(("joystick-axis", n))
                            .selected_text(if s.mapping[n] < 0 {
                                tr!("common-none").to_owned()
                            } else {
                                s.mapping[n].to_string()
                            })
                            .show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut s.mapping[n], -1, tr!("common-none")).changed();
                                for a in 0..8 {
                                    changed |= ui.selectable_value(&mut s.mapping[n], a, a.to_string()).changed();
                                }
                            });
                        for (avatar, label, tune) in [
                            (true, tr!("joystick-avatar"), &mut s.avatar),
                            (false, tr!("common-camera"), &mut s.flycam),
                        ] {
                            // Avatar movement has neither roll nor a lens axis.
                            if avatar && matches!(n, 3 | 6) {
                                continue;
                            }
                            ui.push_id((n, label), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(label);
                                    changed |= ui
                                        .add(
                                            egui::DragValue::new(&mut tune.gain[n])
                                                .speed(0.01)
                                                .range(-20.0..=20.0)
                                                .prefix(tr!("joystick-gain-prefix")),
                                        )
                                        .changed();
                                    changed |= ui
                                        .add(
                                            egui::DragValue::new(&mut tune.dead_zone[n])
                                                .speed(0.01)
                                                .range(0.0..=1.0)
                                                .prefix(tr!("joystick-zone-prefix")),
                                        )
                                        .changed();
                                })
                            });
                        }
                    });
                    changed
                },
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
        frame(frames, settings, state, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
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
        frame(frames, settings, state, Vec::new()).0
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
