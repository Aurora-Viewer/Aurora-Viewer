//! Movement settings embedded in the AO manager; animations belong to AO state lists.
use super::widgets;
use crate::{keybinds::KeyBindings, movement::Preferences, theme::Palette};

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
    pub animations: bool,
}

impl MovementUi {
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences, keys: &KeyBindings, status: &str) -> Actions {
        let before = prefs.clone();
        let mut actions = Actions::default();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("movement-title"))
                .strong()
                .color(p.violet_pale),
        );
        if status == aurora_i18n::tr!("movement-physics-unavailable") {
            ui.label(egui::RichText::new(status).color(p.violet_pale));
        }
        ui.columns(2, |columns| {
            widgets::switch_labeled(&mut columns[0], p, &mut prefs.running, aurora_i18n::tr!("movement-running"));
            columns[0].add_enabled_ui(prefs.running, |ui| {
                ui.add(egui::Slider::new(&mut prefs.run_multiplier, 1.0..=5.0).prefix("×").step_by(0.1))
                    .on_hover_text(aurora_i18n::tr!("movement-run-speed"));
            });
            widgets::switch_labeled(&mut columns[1], p, &mut prefs.fly_assist, aurora_i18n::tr!("movement-fly-assist"));
            columns[1].add_enabled_ui(prefs.fly_assist, |ui| {
                ui.add(egui::Slider::new(&mut prefs.fly_multiplier, 1.0..=5.0).prefix("×").step_by(0.1))
                    .on_hover_text(aurora_i18n::tr!("movement-flight-speed"));
            });
        });
        widgets::divider(ui, p);
        widgets::switch_labeled(ui, p, &mut prefs.pass_wall, aurora_i18n::tr!("movement-pass-enable"));
        ui.label(aurora_i18n::tr!("movement-pass-shortcut", shortcut = keys.pass_wall_label()));
        ui.label(egui::RichText::new(aurora_i18n::tr!("movement-pass-help")).color(p.muted));
        if !self.message.is_empty() {
            let message = ui.label(&self.message);
            if std::mem::take(&mut self.demo_reveal_message) {
                message.scroll_to_me(Some(egui::Align::Center));
            }
        }
        widgets::divider(ui, p);
        if !self.swim_ready {
            prefs.swimming = false;
        }
        ui.add_enabled_ui(self.swim_ready, |ui| {
            widgets::switch_labeled(ui, p, &mut prefs.swimming, aurora_i18n::tr!("movement-swimming"));
        });
        if !self.swim_ready {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!(
                    "ao-swim-required",
                    count = self.swim_slots,
                    total = crate::ao::State::SWIMMING.len()
                ))
                .color(p.muted),
            );
        }
        ui.add_enabled_ui(prefs.swimming, |ui| {
            ui.add(
                egui::Slider::new(&mut prefs.swim_speed, 0.5..=8.0)
                    .suffix(" m/s")
                    .text(aurora_i18n::tr!("movement-swim-speed")),
            );
            ui.add(
                egui::Slider::new(&mut prefs.float_height, -0.5..=0.5)
                    .suffix(" m")
                    .step_by(0.01)
                    .text(aurora_i18n::tr!("movement-float-height")),
            )
            .on_hover_text(aurora_i18n::tr!("movement-float-help"));
            widgets::checkbox(ui, p, &mut prefs.jump_obstacles, aurora_i18n::tr!("movement-jump-obstacles"));
        });
        ui.label(egui::RichText::new(aurora_i18n::tr!("movement-keys")).color(p.muted));
        ui.label(egui::RichText::new(aurora_i18n::tr!("ao-swim-help")).color(p.muted));
        actions.animations = ui.button(aurora_i18n::tr!("ao-swim-animations")).clicked();
        widgets::divider(ui, p);
        ui.label(aurora_i18n::tr!(
            "movement-measured-speed",
            speed = format!("{:.1}", self.measured_speed)
        ));
        if !self.motor_status.is_empty() {
            ui.label(&self.motor_status);
        }
        prefs.sanitize();
        actions.changed = *prefs != before;
        actions
    }
}
