//! Preferences › Accessibility (Aurora's own page; the Legacy Viewer has
//! no colour blindness option): the colour vision the interface adapts to
//! (`crate::colorblind`, #418) with a preview of the status dots, the
//! reduced motion, and the ways to a larger interface.

use super::super::Page;
use super::super::cx::{Cx, Nothing, field};
use crate::colorblind::Deficiency;
use crate::ui::presence::{self, Status};
use egui::{RichText, Sense, Vec2};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-color-blindness"), |cx| {
        cx.note(tr!("ui-options-color-blindness-help"));
        let choices: Vec<(Deficiency, &str)> = Deficiency::ALL.iter().map(|d| (*d, d.label())).collect();
        let help = Deficiency::ALL
            .iter()
            .map(|d| format!("{} — {}", d.label(), d.help()))
            .collect::<Vec<_>>()
            .join("\n");
        cx.combo(tr!("ui-options-color-vision"), &help, &field!(color_blindness), &choices);
        let off = cx.s.color_blindness == Deficiency::Off;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-color-blindness-strength"),
                tr!("ui-options-color-blindness-strength-help"),
                &field!(color_blindness_strength),
                |ui, s| {
                    let mut percent = (s.color_blindness_strength * 100.0).round();
                    let changed = super::super::slider_whole(ui, &mut percent, 0.0..=100.0, "%");
                    if changed {
                        s.color_blindness_strength = (percent / 100.0).clamp(0.0, 1.0);
                    }
                    changed
                },
            );
        });
        cx.wide().row(
            tr!("ui-options-color-blindness-preview"),
            tr!("ui-options-color-blindness-preview-help"),
            &Nothing,
            |ui, _| {
                preview(ui, &p);
                false
            },
        );
    });
    cx.group(tr!("ui-options-motion-and-reading"), |cx| {
        cx.toggle(
            tr!("ui-options-reduce-motion"),
            tr!("ui-options-reduce-motion-help"),
            &field!(reduce_motion),
        );
        cx.link(tr!("ui-options-text-size"), "", Page::Appearance.name(), Page::Appearance, 0);
        cx.link(tr!("ui-options-interface-scale"), "", Page::Layout.name(), Page::Layout, 0);
    });
}

/// The four status dots as the lists draw them, each with its name.
fn preview(ui: &mut egui::Ui, p: &crate::theme::Palette) {
    ui.spacing_mut().item_spacing.x = 4.0;
    for status in [Status::Online, Status::Away, Status::Busy, Status::Offline] {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 16.0), Sense::hover());
        presence::paint_mark(ui.painter(), rect.center(), 5.0, status.shape(), status.color(p), p.panel);
        ui.label(RichText::new(status.label()).size(12.0).color(p.ink));
        ui.add_space(8.0);
    }
}
