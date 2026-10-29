//! LLFloaterJoystick settings, with native axis and button monitoring.

use super::super::cx::{Custom, Cx, Many, Nothing, field};
use crate::ui::widgets;

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
            &Many(&[
                &field!(joystick.mapping),
                &field!(joystick.avatar),
                &field!(joystick.flycam),
                &field!(joystick.rise_button),
                &field!(joystick.fall_button),
                &field!(joystick.jump_button),
                &field!(joystick.flycam_button),
                &field!(joystick.auto_level),
            ]),
            |ui, s| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    if widgets::flat_button(ui, &p, tr!("joystick-gamepad")).clicked() {
                        s.joystick.gamepad();
                        changed = true;
                    }
                    if widgets::flat_button(ui, &p, tr!("joystick-xbox")).clicked() {
                        s.joystick.apply_xbox_one();
                        changed = true;
                    }
                    if widgets::flat_button(ui, &p, tr!("joystick-spacemouse")).clicked() {
                        let d = crate::joystick::Settings::default();
                        s.joystick.mapping = d.mapping;
                        s.joystick.avatar = d.avatar;
                        s.joystick.flycam = d.flycam;
                        s.joystick.rise_button = d.rise_button;
                        s.joystick.fall_button = d.fall_button;
                        s.joystick.jump_button = d.jump_button;
                        s.joystick.flycam_button = d.flycam_button;
                        s.joystick.auto_level = d.auto_level;
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
        cx.slider(
            tr!("joystick-flycam-button"),
            tr!("joystick-button-hint"),
            &field!(joystick.flycam_button),
            -1..=31,
            "",
        );
        cx.slider(
            tr!("joystick-jump-button"),
            tr!("joystick-button-hint"),
            &field!(joystick.jump_button),
            -1..=31,
            "",
        );
        cx.slider(tr!("joystick-run-threshold"), "", &field!(joystick.run_threshold), 0.1..=1.0, "");
    });
    cx.group(tr!("joystick-tuning"), |cx| {
        cx.toggle(
            tr!("joystick-mouselook-yaw"),
            tr!("joystick-mouselook-yaw-hint"),
            &field!(joystick.mouselook_yaw),
        );
        cx.slider(
            tr!("joystick-rise-button"),
            tr!("joystick-button-hint"),
            &field!(joystick.rise_button),
            -1..=31,
            "",
        );
        cx.slider(
            tr!("joystick-fall-button"),
            tr!("joystick-button-hint"),
            &field!(joystick.fall_button),
            -1..=31,
            "",
        );
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
        cx.toggle(tr!("joystick-direct-zoom"), "", &field!(joystick.direct_zoom));
    });
    cx.group(tr!("joystick-axes"), |cx| {
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
            cx.row(label, tr!("joystick-axis-hint"), &track, |ui, s| {
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
            });
        }
    });
}
