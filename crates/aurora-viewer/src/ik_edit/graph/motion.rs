//! Delta keys and bounded procedural curves for virtual controls.
use super::*;
use data::{Curve, Function, Key, Motion, Playback};

fn time(motion: &Motion, time: f64) -> f32 {
    let d = f64::from(motion.duration());
    let result = if (0.0..=d).contains(&time) {
        time
    } else {
        match motion.playback() {
            Playback::Once => time.clamp(0.0, d),
            Playback::Loop => time.rem_euclid(d),
            Playback::PingPong => d - ((time / d).rem_euclid(2.0) - 1.0).abs() * d,
        }
    };
    result as f32
}
pub(super) fn key(node: &mut Node, at: f64, transform: Transform, selected: &mut usize) {
    if node.motion.is_none() {
        node.motion = Some(Motion::Keys {
            duration: at.clamp(1.0, 600.0) as f32,
            playback: Playback::Loop,
            keys: vec![Key {
                time: 0.0,
                transform: Transform::default(),
                curve: Curve::Smooth,
            }],
        });
    }
    let Some(motion) = node.motion.as_mut() else { return };
    let at = time(motion, at);
    if let Motion::Keys { keys, .. } = motion {
        if let Some(index) = keys.iter().position(|k| (k.time - at).abs() < 1.0 / 300.0) {
            keys[index].transform = transform;
            *selected = index;
        } else if keys.len() < 32 {
            keys.push(Key {
                time: at,
                transform,
                curve: Curve::Smooth,
            });
            keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            *selected = keys.iter().position(|k| (k.time - at).abs() < 0.0001).unwrap_or(0);
        }
    }
}
fn curve_ui(ui: &mut egui::Ui, palette: &Palette, curve: &mut Curve) {
    let mut kind = match curve {
        Curve::Step => 0,
        Curve::Linear => 1,
        Curve::Smooth => 2,
        Curve::Bezier { .. } => 3,
    };
    egui::ComboBox::from_id_salt("curve-kind")
        .selected_text(aurora_i18n::text(
            ["ik-curve-step", "ik-curve-linear", "ik-curve-smooth", "ik-curve-bezier"][kind],
        ))
        .show_ui(ui, |ui| {
            for (i, label) in ["ik-curve-step", "ik-curve-linear", "ik-curve-smooth", "ik-curve-bezier"]
                .iter()
                .enumerate()
            {
                if ui.selectable_value(&mut kind, i, aurora_i18n::text(label)).changed() {
                    *curve = match i {
                        0 => Curve::Step,
                        1 => Curve::Linear,
                        2 => Curve::Smooth,
                        _ => Curve::Bezier {
                            controls: [0.25, 0.1, 0.75, 0.9],
                        },
                    };
                }
            }
        });
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(310.0), 110.0), Sense::hover());
    let r = rect.shrink(9.0);
    let (low, high) = match curve {
        Curve::Bezier { controls } => (controls[1].min(controls[3]).min(0.0), controls[1].max(controls[3]).max(1.0)),
        _ => (0.0, 1.0),
    };
    let point = |x: f32, y: f32| egui::pos2(egui::lerp(r.x_range(), x), egui::lerp(r.y_range(), (high - y) / (high - low)));
    let painter = ui.painter();
    painter.line_segment([point(0.0, 0.0), point(1.0, 0.0)], Stroke::new(1.0, palette.muted));
    painter.line_segment([point(0.0, 0.0), point(0.0, 1.0)], Stroke::new(1.0, palette.muted));
    let line: Vec<_> = (0..=64)
        .map(|i| {
            let t = i as f32 / 64.0;
            point(t, curve.sample(t))
        })
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, palette.violet_light)));
    if let Curve::Bezier { controls } = curve {
        for i in 0..2 {
            let p = point(controls[i * 2], controls[i * 2 + 1]);
            ui.painter()
                .line_segment([point(i as f32, i as f32), p], Stroke::new(1.0, palette.teal));
            let response = ui.interact(
                egui::Rect::from_center_size(p, egui::Vec2::splat(14.0)),
                ui.scope_id().with(("bezier", i)),
                Sense::drag(),
            );
            ui.painter().circle_filled(p, 4.0, palette.teal);
            if response.dragged()
                && let Some(p) = response.interact_pointer_pos()
            {
                controls[i * 2] = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0);
                controls[i * 2 + 1] = (high - (p.y - r.top()) / r.height() * (high - low)).clamp(-2.0, 3.0);
            }
        }
        if controls[0] > controls[2] {
            controls.swap(0, 2);
            controls.swap(1, 3);
        }
        for i in 0..2 {
            ui.horizontal(|ui| {
                ui.label(format!("P{}", i + 1));
                ui.add(egui::DragValue::new(&mut controls[i * 2]).speed(0.01).range(0.0..=1.0));
                ui.add(egui::DragValue::new(&mut controls[i * 2 + 1]).speed(0.01).range(-2.0..=3.0));
            });
        }
        controls[0] = controls[0].min(controls[2]);
    }
}
impl State {
    pub(super) fn motion_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, index: usize) {
        ui.strong(aurora_i18n::tr!("ik-graph-motion"));
        let can_key = !matches!(self.definition.nodes[index].motion, Some(Motion::Function { .. }));
        if !can_key {
            self.auto_key = false;
        }
        ui.horizontal(|ui| {
            if ui
                .button(if self.playing {
                    aurora_i18n::tr!("ik-graph-pause")
                } else {
                    aurora_i18n::tr!("ik-graph-play")
                })
                .clicked()
            {
                self.playing = !self.playing;
            }
            if ui.button(aurora_i18n::tr!("ik-graph-rewind")).clicked() {
                self.time = 0.0;
                self.playing = false;
            }
            ui.add_enabled_ui(can_key, |ui| {
                widgets::switch_labeled(ui, palette, &mut self.auto_key, aurora_i18n::tr!("ik-graph-auto-key"))
            });
        });
        let duration = self.definition.nodes[index].motion.as_ref().map_or(1.0, Motion::duration);
        // Display the selected control's phase without clamping the shared
        // clock at the slider endpoint and inadvertently stopping every loop.
        let mut sample = self.definition.nodes[index]
            .motion
            .as_ref()
            .map_or_else(|| self.time.rem_euclid(1.0), |m| f64::from(time(m, self.time)));
        ui.horizontal(|ui| {
            let mut changed = ui
                .add(egui::Slider::new(&mut sample, 0.0..=f64::from(duration)).show_value(false))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut sample).speed(1.0 / 30.0).range(0.0..=600.0).suffix(" s"))
                .changed();
            if changed {
                self.time = sample;
                self.playing = false;
            }
        });
        let node = &mut self.definition.nodes[index];
        let mut kind = match &node.motion {
            None => 0,
            Some(Motion::Keys { .. }) => 1,
            Some(Motion::Function { .. }) => 2,
        };
        egui::ComboBox::from_id_salt("virtual-motion-kind")
            .selected_text(aurora_i18n::text(["ik-graph-static", "ik-graph-keys", "ik-graph-function"][kind]))
            .show_ui(ui, |ui| {
                for (i, label) in ["ik-graph-static", "ik-graph-keys", "ik-graph-function"].iter().enumerate() {
                    if ui.selectable_value(&mut kind, i, aurora_i18n::text(label)).changed() {
                        self.playing = false;
                        self.key = 0;
                        node.motion = match i {
                            1 => Some(Motion::Keys {
                                duration: 1.0,
                                playback: Playback::Loop,
                                keys: vec![
                                    Key {
                                        time: 0.0,
                                        transform: Transform::default(),
                                        curve: Curve::Smooth,
                                    },
                                    Key {
                                        time: 1.0,
                                        transform: Transform::default(),
                                        curve: Curve::Smooth,
                                    },
                                ],
                            }),
                            2 => Some(Motion::Function {
                                duration: 2.0,
                                playback: Playback::Loop,
                                function: Function::Sine,
                                position: [0.0, 0.0, 0.05],
                                degrees: [0.0; 3],
                                phase: 0.0,
                                curve: Curve::Linear,
                            }),
                            _ => None,
                        };
                    }
                }
            });
        let Some(motion) = node.motion.as_mut() else { return };
        let old_duration = motion.duration();
        let (duration, playback) = match motion {
            Motion::Keys { duration, playback, .. } | Motion::Function { duration, playback, .. } => (duration, playback),
        };
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ik-graph-duration"));
            if ui
                .add(
                    egui::DragValue::new(duration)
                        .speed(1.0 / 30.0)
                        .range(1.0 / 30.0..=600.0)
                        .suffix(" s"),
                )
                .changed()
            {
                self.playing = false;
            }
        });
        egui::ComboBox::from_id_salt("virtual-playback")
            .selected_text(aurora_i18n::text(match playback {
                Playback::Once => "ik-graph-once",
                Playback::Loop => "ik-graph-loop",
                Playback::PingPong => "ik-graph-ping-pong",
            }))
            .show_ui(ui, |ui| {
                for (value, label) in [
                    (Playback::Once, "ik-graph-once"),
                    (Playback::Loop, "ik-graph-loop"),
                    (Playback::PingPong, "ik-graph-ping-pong"),
                ] {
                    ui.selectable_value(playback, value, aurora_i18n::text(label));
                }
            });
        let duration = *duration;
        match motion {
            Motion::Keys { keys, .. } => {
                if duration != old_duration {
                    for key in keys.iter_mut() {
                        key.time *= duration / old_duration;
                    }
                }
                let mut add = false;
                let mut remove = false;
                ui.horizontal(|ui| {
                    add = ui
                        .add_enabled(keys.len() < 32, egui::Button::new(aurora_i18n::tr!("ik-graph-add-key")))
                        .clicked();
                    remove = ui
                        .add_enabled(keys.len() > 1, egui::Button::new(aurora_i18n::tr!("ik-graph-remove-key")))
                        .clicked();
                });
                if remove && self.key < keys.len() {
                    keys.remove(self.key);
                    self.key = self.key.saturating_sub(1);
                }
                if add {
                    let transform = motion.sample(self.time);
                    key(node, self.time, transform, &mut self.key);
                    return;
                }
                let Some(Motion::Keys { keys, .. }) = node.motion.as_mut() else {
                    return;
                };
                ui.horizontal_wrapped(|ui| {
                    for (i, key) in keys.iter().enumerate() {
                        if ui.selectable_label(self.key == i, format!("{:.2} s", key.time)).clicked() {
                            self.key = i;
                            self.time = f64::from(key.time);
                            self.playing = false;
                        }
                    }
                });
                self.key = self.key.min(keys.len().saturating_sub(1));
                let key = &mut keys[self.key];
                let mut p = key.transform.p();
                let mut d = degrees(key.transform.q());
                ui.label(aurora_i18n::tr!("ik-graph-key-delta"));
                let mut changed = vector_fields(ui, &mut p, 0.005);
                changed |= vector_fields(ui, &mut d, 0.5);
                if changed {
                    key.transform = Transform::new(p, rotation(d));
                    self.time = f64::from(key.time);
                    self.playing = false;
                }
                ui.label(aurora_i18n::tr!("ik-graph-curve"));
                curve_ui(ui, palette, &mut key.curve);
                if self.reveal_curve {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                    self.reveal_curve = false;
                }
            }
            Motion::Function {
                function,
                position,
                degrees: amplitude,
                phase,
                curve,
                ..
            } => {
                egui::ComboBox::from_id_salt("virtual-function")
                    .selected_text(aurora_i18n::text(match function {
                        Function::Sine => "ik-function-sine",
                        Function::Triangle => "ik-function-triangle",
                        Function::Saw => "ik-function-saw",
                        Function::Pulse => "ik-function-pulse",
                        Function::Ramp => "ik-function-ramp",
                    }))
                    .show_ui(ui, |ui| {
                        for (value, label) in [
                            (Function::Sine, "ik-function-sine"),
                            (Function::Triangle, "ik-function-triangle"),
                            (Function::Saw, "ik-function-saw"),
                            (Function::Pulse, "ik-function-pulse"),
                            (Function::Ramp, "ik-function-ramp"),
                        ] {
                            ui.selectable_value(function, value, aurora_i18n::text(label));
                        }
                    });
                ui.label(aurora_i18n::tr!("ik-graph-position-amplitude"));
                let mut p = Vec3::from_array(*position);
                if vector_fields(ui, &mut p, 0.005) {
                    *position = p.to_array();
                }
                ui.label(aurora_i18n::tr!("ik-graph-rotation-amplitude"));
                let mut d = Vec3::from_array(*amplitude);
                if vector_fields(ui, &mut d, 0.5) {
                    *amplitude = d.to_array();
                }
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ik-graph-phase"));
                    ui.add(egui::DragValue::new(phase).speed(0.01).range(-1000.0..=1000.0));
                });
                curve_ui(ui, palette, curve);
            }
        }
    }
}
