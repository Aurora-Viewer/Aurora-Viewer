//! Movement settings embedded in the AO manager; animations belong to AO state lists.
use super::widgets;
use crate::{
    keybinds::{Action as KeyAction, Binding, KeyBindings},
    movement::Preferences,
    theme::Palette,
};

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
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences, keys: &KeyBindings, status: &str) -> Actions {
        let before = prefs.clone();
        let mut actions = Actions::default();
        let up = keys.get(KeyAction::Up);
        let down = keys.get(KeyAction::Down);
        let passage_keys = keys.pass_wall_bindings();
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
        ui.columns(2, |columns| {
            let gutter = columns[0].spacing().item_spacing.x * 2.0;
            let passage = &mut columns[0];
            let passage_width = (passage.available_width() - gutter).max(0.0);
            // Columns already reserve their full minimum width. A child scope
            // keeps an inner text gutter without shifting the right switch.
            passage.scope(|ui| {
                ui.set_max_width(passage_width);
                widgets::switch_labeled(ui, p, &mut prefs.pass_wall, aurora_i18n::tr!("movement-pass-enable"));
                key_hint(ui, p, Gesture::Tap(&passage_keys), "");
                ui.label(egui::RichText::new(aurora_i18n::tr!("movement-pass-help")).color(p.muted));
            });

            let water_walk = &mut columns[1];
            let mut walking = prefs.water_walk;
            widgets::switch_labeled(water_walk, p, &mut walking, aurora_i18n::tr!("movement-water-walk"));
            if walking != prefs.water_walk {
                prefs.set_water_mode(true, walking, self.swim_ready);
            }
            key_hint(water_walk, p, Gesture::Tap(&down), aurora_i18n::tr!("movement-water-walk-crouch"));
            key_hint(water_walk, p, Gesture::Tap(&up), aurora_i18n::tr!("movement-water-walk-jump"));
        });
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
        let mut swimming = prefs.swimming;
        ui.add_enabled_ui(self.swim_ready, |ui| {
            widgets::switch_labeled(ui, p, &mut swimming, aurora_i18n::tr!("movement-swimming"));
        });
        if swimming != prefs.swimming {
            prefs.set_water_mode(false, swimming, self.swim_ready);
        }
        if !self.swim_ready && self.swim_slots < crate::ao::State::CORE_SWIMMING.len() {
            super::ao::swimming_warning(ui, p);
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
            ui.add_space(ui.spacing().item_spacing.y * 2.0);
            widgets::checkbox(ui, p, &mut prefs.jump_obstacles, aurora_i18n::tr!("movement-jump-obstacles"));
        });
        key_hint(ui, p, Gesture::Tap(&down), aurora_i18n::tr!("movement-water-submerge"));
        key_hint(ui, p, Gesture::Tap(&up), aurora_i18n::tr!("movement-water-leap"));
        key_hint(ui, p, Gesture::Double(&up), aurora_i18n::tr!("movement-water-takeoff"));
        key_hint(ui, p, Gesture::Tap(&up), aurora_i18n::tr!("movement-water-surface"));
        ui.add_space(ui.spacing().item_spacing.y * 2.0);
        widgets::checkbox(ui, p, &mut prefs.water_jumps, aurora_i18n::tr!("movement-water-jumps")).on_hover_ui(|ui| {
            key_hint(ui, p, Gesture::Tap(&down), aurora_i18n::tr!("movement-water-jumps-help"));
        });
        key_hint(ui, p, Gesture::Then(&up, &down), aurora_i18n::tr!("movement-water-dive"));
        key_hint(ui, p, Gesture::Tap(&down), aurora_i18n::tr!("movement-water-cannonball"));
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

enum Gesture<'a> {
    Tap(&'a [Option<Binding>]),
    Double(&'a [Option<Binding>]),
    Then(&'a [Option<Binding>], &'a [Option<Binding>]),
}

fn keycaps(ui: &mut egui::Ui, p: &Palette, bindings: &[Option<Binding>]) {
    let mut any = false;
    for binding in bindings.iter().flatten() {
        if any {
            ui.label(egui::RichText::new("/").color(p.muted));
        }
        let caps = widgets::keycap::caps(ui, binding, p.panel);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(widgets::keycap::row_width(&caps), widgets::keycap::HEIGHT),
            egui::Sense::hover(),
        );
        widgets::keycap::paint_row(ui, p, &caps, rect.left_center(), p.panel);
        response.on_hover_text(binding.label());
        any = true;
    }
    if !any {
        ui.label(egui::RichText::new(aurora_i18n::tr!("movement-key-unbound")).color(p.muted));
    }
}

fn key_hint(ui: &mut egui::Ui, p: &Palette, gesture: Gesture<'_>, label: &str) {
    // Read-only instructions use the text line height, rather than reserving
    // a full button-height row for every gesture.
    let height = ui.text_style_height(&egui::TextStyle::Body).max(widgets::keycap::HEIGHT);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            match gesture {
                Gesture::Tap(bindings) => keycaps(ui, p, bindings),
                Gesture::Double(bindings) => {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("movement-key-double")).color(p.muted));
                    keycaps(ui, p, bindings);
                }
                Gesture::Then(first, second) => {
                    keycaps(ui, p, first);
                    widgets::glyph(ui, "arrow-right", p.muted, 14.0);
                    keycaps(ui, p, second);
                }
            }
            if !label.is_empty() {
                ui.label(egui::RichText::new(label).color(p.muted));
            }
        },
    );
}
