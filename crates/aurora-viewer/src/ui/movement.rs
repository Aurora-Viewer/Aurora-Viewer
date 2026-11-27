//! Movement settings, the AO window's Movement page: one card per enhancement
//! (running, flight, obstacle passage, water walk, swimming); animations belong
//! to the AO slots.
use super::widgets;
use crate::{movement::Preferences, theme::Palette};

#[derive(Default)]
pub struct MovementUi {
    pub message: String,
    pub measured_speed: f32,
    pub motor_status: String,
    pub swim_ready: bool,
    pub swim_slots: usize,
    /// One-shot reveal for the explicit offline passage capture scenarios.
    pub demo_reveal_message: bool,
}

#[derive(Default)]
pub struct Actions {
    pub changed: bool,
}

impl MovementUi {
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences, status: &str) -> Actions {
        let before = prefs.clone();
        let mut actions = Actions::default();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        if status == aurora_i18n::tr!("movement-physics-unavailable") {
            ui.label(egui::RichText::new(status).color(p.violet_pale));
        }
        // Four enhancements two by two, then swimming across (the board's
        // Movement page): each an icon, its name and its switch, its
        // settings under them.
        let swim_ready = self.swim_ready;
        ui.columns(2, |columns| {
            let (left, right) = columns.split_at_mut(1);
            let (left, right) = (&mut left[0], &mut right[0]);
            card(
                left,
                p,
                "run",
                aurora_i18n::tr!("movement-running"),
                &mut prefs.running,
                true,
                |ui, on| {
                    labelled_stepper(
                        ui,
                        p,
                        "ao_run",
                        aurora_i18n::tr!("ao-run-multiplier"),
                        &mut prefs.run_multiplier,
                        on,
                        Step::times(1.0, 5.0),
                    );
                },
            );
            card(
                right,
                p,
                "plane-tilt-filled",
                aurora_i18n::tr!("movement-fly-assist"),
                &mut prefs.fly_assist,
                true,
                |ui, on| {
                    labelled_stepper(
                        ui,
                        p,
                        "ao_fly",
                        aurora_i18n::tr!("ao-flight-multiplier"),
                        &mut prefs.fly_multiplier,
                        on,
                        Step::times(1.0, 5.0),
                    );
                },
            );
            card(
                left,
                p,
                "arrows-move",
                aurora_i18n::tr!("movement-pass-enable"),
                &mut prefs.pass_wall,
                true,
                |ui, _| {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("movement-pass-help")).small().color(p.muted));
                },
            );
            let mut walking = prefs.water_walk;
            card(
                right,
                p,
                "ripple",
                aurora_i18n::tr!("movement-water-walk"),
                &mut walking,
                true,
                |ui, _| {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("ao-water-walk-line")).small().color(p.muted));
                },
            );
            if walking != prefs.water_walk {
                prefs.set_water_mode(true, walking, swim_ready);
            }
        });
        if !self.message.is_empty() {
            let message = ui.label(&self.message);
            if std::mem::take(&mut self.demo_reveal_message) {
                message.scroll_to_me(Some(egui::Align::Center));
            }
        }
        if !self.swim_ready {
            prefs.swimming = false;
        }
        let swim_slots = self.swim_slots;
        let mut swimming = prefs.swimming;
        card(
            ui,
            p,
            "swimming",
            aurora_i18n::tr!("movement-swimming"),
            &mut swimming,
            swim_ready,
            |ui, on| {
                if !swim_ready && swim_slots < crate::ao::State::CORE_SWIMMING.len() {
                    super::ao::swimming_warning(ui, p);
                }
                ui.horizontal_wrapped(|ui| {
                    labelled_stepper(
                        ui,
                        p,
                        "ao_swim_speed",
                        aurora_i18n::tr!("ao-swim-speed-label"),
                        &mut prefs.swim_speed,
                        on,
                        Step::metres_per_second(0.5, 8.0),
                    );
                    ui.add_space(14.0);
                    labelled_stepper(
                        ui,
                        p,
                        "ao_float",
                        aurora_i18n::tr!("movement-float-height"),
                        &mut prefs.float_height,
                        on,
                        Step::metres(-0.5, 0.5),
                    )
                    .on_hover_text(aurora_i18n::tr!("movement-float-help"));
                });
                ui.add_enabled_ui(on, |ui| {
                    widgets::checkbox(ui, p, &mut prefs.jump_obstacles, aurora_i18n::tr!("movement-jump-obstacles"));
                });
                widgets::checkbox(ui, p, &mut prefs.water_jumps, aurora_i18n::tr!("movement-water-jumps"));
            },
        );
        if swimming != prefs.swimming {
            prefs.set_water_mode(false, swimming, self.swim_ready);
        }
        ui.label(
            egui::RichText::new(aurora_i18n::tr!(
                "movement-measured-speed",
                speed = format!("{:.1}", self.measured_speed)
            ))
            .color(p.muted),
        );
        if !self.motor_status.is_empty() {
            ui.label(egui::RichText::new(&self.motor_status).color(p.muted));
        }
        prefs.sanitize();
        actions.changed = *prefs != before;
        actions
    }
}

/// One enhancement: its icon in a disc, its name and its switch on the
/// right, then `body` with whether it is on. `ready`: the switch can be
/// turned on.
fn card(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, on: &mut bool, ready: bool, body: impl FnOnce(&mut egui::Ui, bool)) {
    ui.horizontal(|ui| {
        let (disc, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        ui.painter().circle_filled(disc.center(), 14.0, p.violet.gamma_multiply(0.22));
        widgets::paint_glyph(ui.painter(), icon, disc.center(), 15.0, p.violet_pale);
        // the name turns the switch too
        let can = ready || *on;
        if ui
            .add_enabled(
                can,
                egui::Label::new(egui::RichText::new(title).strong()).sense(egui::Sense::click()),
            )
            .clicked()
        {
            *on = !*on;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(can, |ui| widgets::switch(ui, p, on));
        });
    });
    let on = *on;
    body(ui, on);
    ui.add_space(14.0);
}

/// What a stepper holds: its range, its step and how its value is written.
struct Step {
    least: f32,
    most: f32,
    step: f32,
    write: fn(f32) -> String,
}

impl Step {
    fn times(least: f32, most: f32) -> Self {
        Self {
            least,
            most,
            step: 0.25,
            write: |v| format!("×{v:.2}"),
        }
    }

    fn metres_per_second(least: f32, most: f32) -> Self {
        Self {
            least,
            most,
            step: 0.25,
            write: |v| format!("{v:.2} m/s"),
        }
    }

    fn metres(least: f32, most: f32) -> Self {
        Self {
            least,
            most,
            step: 0.05,
            write: |v| format!("{v:.2} m"),
        }
    }
}

/// A setting's name and its value between − and + on the field's face.
fn labelled_stepper(ui: &mut egui::Ui, p: &Palette, id: &str, label: &str, value: &mut f32, enabled: bool, step: Step) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).color(if enabled { p.ink } else { p.muted }));
            let text = (step.write)(*value);
            let font = egui::FontId::proportional(13.0);
            let text_w = ui.painter().layout_no_wrap(text.clone(), font.clone(), p.ink).size().x;
            let (rect, response) = ui.allocate_exact_size(egui::vec2(text_w + 56.0, 26.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field);
            let ink = if enabled { p.ink } else { p.muted };
            widgets::paint_ink_centered(ui.painter(), ui.painter().layout_no_wrap(text, font, ink), rect.center(), ink);
            for (sign, icon, x) in [(-1.0, "minus", rect.left() + 14.0), (1.0, "plus", rect.right() - 14.0)] {
                let hit = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(24.0, 24.0));
                let button = ui.interact(hit, ui.scope_id().with((id, icon)), egui::Sense::click());
                let tint = if button.hovered() && enabled { p.ink } else { p.muted };
                widgets::paint_glyph(ui.painter(), icon, hit.center(), 12.0, tint);
                if button.clicked() && enabled {
                    *value = ((*value + sign * step.step) / step.step).round() * step.step;
                    *value = value.clamp(step.least, step.most);
                }
            }
            response
        })
        .inner
    })
    .inner
}
