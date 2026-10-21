//! Preferences › Privacy › Anti-spam. Blocks are local to this login.

use super::{OptionsResult, group, row, toggle};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::anti_spam::{Queue, Settings};
use uuid::Uuid;

pub fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, blocks: &[(Queue, Uuid, String)], result: &mut OptionsResult) -> bool {
    let mut changed = false;
    group(ui, p, aurora_i18n::tr!("anti-spam-title"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("anti-spam-enable"),
            aurora_i18n::tr!("anti-spam-enable-hint"),
            |ui| toggle(ui, p, &mut s.enabled),
        );
        ui.add_enabled_ui(s.enabled, |ui| {
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-own"),
                aurora_i18n::tr!("anti-spam-own-hint"),
                |ui| toggle(ui, p, &mut s.own_objects),
            );
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-global"),
                aurora_i18n::tr!("anti-spam-global-hint"),
                |ui| toggle(ui, p, &mut s.global),
            );
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-amount"),
                aurora_i18n::tr!("anti-spam-amount-hint"),
                |ui| ui.add(egui::Slider::new(&mut s.amount, 1..=1000)).changed(),
            );
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-period"),
                aurora_i18n::tr!("anti-spam-period-hint"),
                |ui| ui.add(egui::Slider::new(&mut s.period, 1..=60).suffix(" s")).changed(),
            );
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-newlines"),
                aurora_i18n::tr!("anti-spam-newlines-hint"),
                |ui| ui.add(egui::Slider::new(&mut s.newlines, 5..=1000)).changed(),
            );
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-sound-multiplier"),
                aurora_i18n::tr!("anti-spam-multiplier-hint"),
                |ui| ui.add(egui::Slider::new(&mut s.sound_multiplier, 1..=100)).changed(),
            );
            changed |= row(
                ui,
                p,
                aurora_i18n::tr!("anti-spam-preload-multiplier"),
                aurora_i18n::tr!("anti-spam-multiplier-hint"),
                |ui| ui.add(egui::Slider::new(&mut s.preload_multiplier, 1..=100)).changed(),
            );
        });
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("anti-spam-session-hint"))
                .color(p.muted)
                .size(11.5),
        );
        if widgets::flat_button(ui, p, aurora_i18n::tr!("anti-spam-reset")).clicked() {
            result.reset_anti_spam = true;
        }
        if !blocks.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("anti-spam-blocks")
                .max_height(160.0)
                .show_rows(ui, ui.spacing().interact_size.y, blocks.len(), |ui, rows| {
                    for &(queue, source, ref name) in &blocks[rows] {
                        ui.push_id((queue, source), |ui| {
                            ui.horizontal(|ui| {
                                let kind = if s.global {
                                    aurora_i18n::tr!("anti-spam-all")
                                } else {
                                    queue.label()
                                };
                                ui.add(widgets::elided_label(
                                    ui,
                                    egui::RichText::new(format!("{kind} · {name}")).color(p.ink),
                                    (ui.available_width() - 95.0).max(40.0),
                                ))
                                .on_hover_text(format!("{name}\n{source}"));
                                if widgets::flat_button(ui, p, aurora_i18n::tr!("anti-spam-unblock")).clicked() {
                                    result.unblock_anti_spam = Some((queue, source));
                                }
                            });
                        });
                    }
                });
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, text)),
            _ => None,
        }
    }

    #[test]
    fn anti_spam_native_buttons_reset_all_or_unblock_only_the_chosen_source() {
        for reset in [true, false] {
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut settings = Settings {
                enabled: true,
                ..Default::default()
            };
            let source = Uuid::from_u128(678);
            let blocks = vec![(Queue::Sound, source, "Test source".to_owned())];
            let label = if reset {
                aurora_i18n::tr!("anti-spam-reset")
            } else {
                aurora_i18n::tr!("anti-spam-unblock")
            };
            let mut draw = |events| {
                let mut result = OptionsResult::default();
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(850.0, 1400.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        page(ui, &p, &mut settings, &blocks, &mut result);
                    },
                );
                let rect = output
                    .shapes
                    .iter()
                    .find_map(|s| text_rect(&s.shape, label))
                    .expect("action visible");
                (result, rect.center())
            };
            let mut at = egui::Pos2::ZERO;
            for _ in 0..4 {
                at = draw(Vec::new()).1;
            }
            let mut clicked = OptionsResult::default();
            for pressed in [true, false] {
                clicked = draw(vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ])
                .0;
            }
            assert_eq!(clicked.reset_anti_spam, reset);
            assert_eq!(clicked.unblock_anti_spam, (!reset).then_some((Queue::Sound, source)));
            assert!(!clicked.changed, "session actions must not save preferences");
        }
    }
}
