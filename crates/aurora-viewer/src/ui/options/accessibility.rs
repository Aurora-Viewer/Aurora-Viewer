//! Preferences › Accessibility (Aurora's own page; Firestorm has no colour
//! blindness option): the colour vision the interface adapts to
//! (`crate::colorblind`, #418), with a preview of the status dots.

use super::{group, row, slider_whole};
use crate::colorblind::Deficiency;
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::presence::{self, Status};
use egui::{RichText, Sense, Vec2};

pub fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("ui-options-color-blindness"), |ui| {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-options-color-blindness-help"))
                .size(12.0)
                .color(p.muted),
        );
        ui.add_space(4.0);
        for d in Deficiency::ALL {
            c |= crate::ui::widgets::radio_value(ui, p, &mut s.color_blindness, d, d.label()).changed();
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                ui.label(RichText::new(d.help()).size(11.5).color(p.muted_dim));
            });
        }
        ui.add_space(4.0);
        let on = s.color_blindness != Deficiency::Off;
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-color-blindness-strength"),
            aurora_i18n::tr!("ui-options-color-blindness-strength-help"),
            |ui| {
                let mut percent = (s.color_blindness_strength * 100.0).round();
                let changed = ui.add_enabled_ui(on, |ui| slider_whole(ui, &mut percent, 0.0..=100.0, "%")).inner;
                if changed {
                    s.color_blindness_strength = (percent / 100.0).clamp(0.0, 1.0);
                }
                changed
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-color-blindness-preview"),
            aurora_i18n::tr!("ui-options-color-blindness-preview-help"),
            |ui| {
                preview(ui, p);
                false
            },
        );
    });
    c
}

/// The four status dots as the lists draw them, each with its name.
fn preview(ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for status in [Status::Online, Status::Away, Status::Busy, Status::Offline] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 16.0), Sense::hover());
            // cut in the card's face (`group`)
            presence::paint_mark(ui.painter(), rect.center(), 5.0, status.shape(), status.color(p), p.field);
            ui.label(RichText::new(status.label()).size(12.0).color(p.ink));
            ui.add_space(8.0);
        }
    });
}
