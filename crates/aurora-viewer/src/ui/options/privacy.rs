//! Preferences › Privacy: Firestorm's "Privacy › LookAt" tab
//! (panel_preferences_privacy.xml, tab-lookat): the crosshairs of where the
//! avatars look, and what we send of where we look.

use super::{group, row, slider_whole, toggle};
use crate::theme::Palette;
use crate::world::lookat::{MAX_DISTANCE, Settings};

/// Name formats of the crosshairs (DebugLookAtShowNames), in its order.
fn name_formats() -> [&'static str; 5] {
    [
        aurora_i18n::tr!("common-none"),
        aurora_i18n::tr!("ui-options-look-at-complete-name"),
        aurora_i18n::tr!("common-display-name"),
        aurora_i18n::tr!("ui-options-look-at-legacy-name"),
        aurora_i18n::tr!("common-username"),
    ]
}

pub fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("ui-options-look-at-targets"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-show-look-at-targets"),
            aurora_i18n::tr!("ui-options-show-look-at-targets-help"),
            |ui| toggle(ui, p, &mut s.show_targets),
        );
        // enabled_control="DebugLookAt"
        ui.add_enabled_ui(s.show_targets, |ui| {
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-look-at-names"),
                aurora_i18n::tr!("ui-options-look-at-names-help"),
                |ui| {
                    let formats = name_formats();
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("look-at-names")
                        .selected_text(formats[s.show_names.min(4) as usize])
                        .show_ui(ui, |ui| {
                            for (i, label) in formats.iter().enumerate() {
                                changed |= ui.selectable_value(&mut s.show_names, i as u8, *label).changed();
                            }
                        });
                    changed
                },
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-hide-own-look-at"),
                aurora_i18n::tr!("ui-options-hide-own-look-at-help"),
                |ui| toggle(ui, p, &mut s.hide_own),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-look-at-lines"),
                aurora_i18n::tr!("ui-options-look-at-lines-help"),
                |ui| toggle(ui, p, &mut s.lines),
            );
        });
    });
    group(ui, p, aurora_i18n::tr!("ui-options-my-look-at"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-private-look-at"),
            aurora_i18n::tr!("ui-options-private-look-at-help"),
            |ui| toggle(ui, p, &mut s.private),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-private-local-look-at"),
            aurora_i18n::tr!("ui-options-private-local-look-at-help"),
            |ui| toggle(ui, p, &mut s.private_local),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-limit-look-at-distance"),
            aurora_i18n::tr!("ui-options-more-distant-targets-are-limited-to-this-distance-from"),
            |ui| {
                let mut changed = toggle(ui, p, &mut s.limit_distance);
                changed |= slider_whole(ui, &mut s.max_distance, MAX_DISTANCE, " m");
                changed
            },
        );
    });
    c
}
