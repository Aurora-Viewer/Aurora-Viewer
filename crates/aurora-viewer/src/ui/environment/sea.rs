//! The Aurora Weather window's "Sea" section: which wind raises the waves
//! of enhanced water, or a sea state set here, the presets, and how the
//! sea is shaped (settings in `weather/sea.rs`). Aurora's own: the Legacy
//! Viewer's water has no waves to set. Unlike the weather above it, the
//! sea stays the user's whoever sets the weather.

use super::{COMBO_W, LABEL_W, compass_point, row_label};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::weather::sea::{self, SeaSettings, SeaSource, SeaStyle};
use egui::Vec2;

/// The section's labels, for the window's label column.
pub(super) fn labels() -> [&'static str; 13] {
    [
        aurora_i18n::tr!("ui-sea-source"),
        aurora_i18n::tr!("ui-sea-state"),
        aurora_i18n::tr!("ui-weather-wind-from"),
        aurora_i18n::tr!("ui-sea-preset"),
        aurora_i18n::tr!("ui-sea-style"),
        aurora_i18n::tr!("ui-sea-swell"),
        aurora_i18n::tr!("ui-sea-chop"),
        aurora_i18n::tr!("ui-sea-fetch"),
        aurora_i18n::tr!("ui-sea-sharpness"),
        aurora_i18n::tr!("ui-sea-foam"),
        aurora_i18n::tr!("ui-sea-save"),
        aurora_i18n::tr!("ui-sea-limit"),
        aurora_i18n::tr!("ui-sea-extreme"),
    ]
}

fn source_label(source: SeaSource) -> &'static str {
    match source {
        SeaSource::Auto => aurora_i18n::tr!("ui-sea-source-auto"),
        SeaSource::Region => aurora_i18n::tr!("ui-sea-source-region"),
        SeaSource::Weather => aurora_i18n::tr!("ui-sea-source-weather"),
        SeaSource::Mine => aurora_i18n::tr!("ui-sea-source-mine"),
    }
}

fn preset_label(key: &str) -> &'static str {
    match key {
        "calm" => aurora_i18n::tr!("ui-sea-preset-calm"),
        "breeze" => aurora_i18n::tr!("ui-sea-preset-breeze"),
        "swell" => aurora_i18n::tr!("ui-sea-preset-swell"),
        "storm" => aurora_i18n::tr!("ui-sea-preset-storm"),
        _ => aurora_i18n::tr!("ui-sea-preset-stylized"),
    }
}

/// The Beaufort number of a wind of `speed` m/s.
pub(crate) fn beaufort(speed: f32) -> u32 {
    const UPPER: [f32; 12] = [0.5, 1.5, 3.3, 5.5, 7.9, 10.7, 13.8, 17.1, 20.7, 24.4, 28.4, 32.6];
    UPPER.iter().position(|&u| speed < u).unwrap_or(12) as u32
}

/// The section's rows. `name`: the preset name being typed.
pub(super) fn body(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings, name: &mut String) {
    ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
    let label_w = ui.data(|d| d.get_temp::<f32>(super::label_width_id())).unwrap_or(LABEL_W);
    let extra = label_w - LABEL_W;
    ui.spacing_mut().slider_width = COMBO_W - 50.0 - extra;
    // which wind raises the waves
    ui.horizontal(|ui| {
        row_label(ui, p, aurora_i18n::tr!("ui-sea-source"), aurora_i18n::tr!("ui-sea-source-tip"));
        egui::ComboBox::from_id_salt("aurora_sea_source")
            .width(COMBO_W - extra)
            .selected_text(source_label(s.source))
            .show_ui(ui, |ui| {
                for source in [SeaSource::Auto, SeaSource::Region, SeaSource::Weather, SeaSource::Mine] {
                    ui.selectable_value(&mut s.source, source, source_label(source));
                }
            });
    });
    // the sea state set here, and where its wind comes from
    let mine = s.source == SeaSource::Mine;
    let state_top = s.state_top();
    ui.add_enabled_ui(mine, |ui| {
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-state"), aurora_i18n::tr!("ui-sea-state-tip"));
            ui.add(egui::Slider::new(&mut s.shape.state, 0.0..=state_top).custom_formatter(|v, _| {
                let speed = v as f32 * sea::MAX_WIND;
                aurora_i18n::tr!("ui-sea-state-value", speed = format!("{speed:.0}"), force = beaufort(speed)).to_string()
            }));
        });
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-weather-wind-from"), aurora_i18n::tr!("ui-sea-from-tip"));
            ui.add(
                egui::Slider::new(&mut s.from, 0.0..=355.0)
                    .step_by(5.0)
                    .custom_formatter(|v, _| format!("{v:.0}° {}", compass_point(v as f32)))
                    .custom_parser(|t| t.trim().trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()),
            );
        });
    });
    // the highest the waves may stand (the top of the slider: no limit)
    let limit_top = s.limit_top();
    ui.horizontal(|ui| {
        row_label(ui, p, aurora_i18n::tr!("ui-sea-limit"), aurora_i18n::tr!("ui-sea-limit-tip"));
        let none = aurora_i18n::tr!("ui-sea-limit-none");
        ui.add(egui::Slider::new(&mut s.max_height, 0.5..=limit_top).custom_formatter(move |v, _| {
            if v >= f64::from(limit_top) {
                none.to_string()
            } else {
                format!("{v:.1} m")
            }
        }));
    });
    // the presets: built in, then saved
    ui.horizontal(|ui| {
        row_label(ui, p, aurora_i18n::tr!("ui-sea-preset"), aurora_i18n::tr!("ui-sea-preset-tip"));
        let current = s.current_preset();
        let shown = match &current {
            Some((key, true)) => preset_label(key).to_string(),
            Some((saved, false)) => saved.clone(),
            None => aurora_i18n::tr!("ui-sea-preset-custom").to_string(),
        };
        let mut pick = None;
        let mut forget = None;
        egui::ComboBox::from_id_salt("aurora_sea_preset")
            .width(COMBO_W - extra - 30.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for key in sea::PRESETS {
                    let on = current.as_ref().is_some_and(|(k, built)| *built && k == key);
                    if ui.selectable_label(on, preset_label(key)).clicked() {
                        pick = sea::preset(key);
                    }
                }
                for saved in &s.saved {
                    let on = current.as_ref().is_some_and(|(k, built)| !*built && *k == saved.name);
                    if ui.selectable_label(on, saved.name.as_str()).clicked() {
                        pick = Some(saved.shape.clone());
                    }
                }
            });
        // a saved preset shown can be forgotten
        if let Some((saved, false)) = &current
            && widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("ui-sea-forget-tip"), true)
        {
            forget = Some(saved.clone());
        }
        if let Some(shape) = pick {
            s.shape = shape;
        }
        if let Some(gone) = forget {
            s.saved.retain(|p| p.name != gone);
        }
    });
    // how the sea is shaped
    widgets::collapsing(ui, p, "aurora_sea_advanced", aurora_i18n::tr!("ui-sea-advanced"), false, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
        ui.spacing_mut().slider_width = COMBO_W - 50.0 - extra;
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-style"), aurora_i18n::tr!("ui-sea-style-tip"));
            let styles = [SeaStyle::Natural, SeaStyle::Ocean, SeaStyle::Stylized];
            let mut i = styles.iter().position(|&st| st == s.shape.style).unwrap_or(0);
            let options = [
                (aurora_i18n::tr!("ui-sea-style-natural"), true),
                (aurora_i18n::tr!("ui-sea-style-ocean"), true),
                (aurora_i18n::tr!("ui-sea-style-stylized"), true),
            ];
            if widgets::segmented(ui, p, &mut i, &options) {
                s.shape.style = styles[i.min(2)];
            }
        });
        let percent = |v: f64, _| format!("{:.0}%", v * 100.0);
        let scale = 0.0..=s.scale_top();
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-swell"), aurora_i18n::tr!("ui-sea-swell-tip"));
            ui.add(egui::Slider::new(&mut s.shape.swell, scale.clone()).custom_formatter(percent));
        });
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-chop"), aurora_i18n::tr!("ui-sea-chop-tip"));
            ui.add(egui::Slider::new(&mut s.shape.chop, scale.clone()).custom_formatter(percent));
        });
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-fetch"), aurora_i18n::tr!("ui-sea-fetch-tip"));
            ui.add_enabled(
                s.shape.style != SeaStyle::Stylized,
                egui::Slider::new(&mut s.shape.fetch, sea::FETCH_RANGE)
                    .logarithmic(true)
                    .max_decimals(1)
                    .suffix(" km"),
            );
        });
        ui.horizontal(|ui| {
            row_label(
                ui,
                p,
                aurora_i18n::tr!("ui-sea-sharpness"),
                aurora_i18n::tr!("ui-sea-sharpness-tip"),
            );
            ui.add_enabled(
                s.shape.style != SeaStyle::Stylized,
                egui::Slider::new(&mut s.shape.choppiness, sea::SHARP_RANGE).custom_formatter(percent),
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-foam"), aurora_i18n::tr!("ui-sea-foam-tip"));
            ui.add(egui::Slider::new(&mut s.shape.foam, scale.clone()).custom_formatter(percent));
        });
        // the safety catch for the seas no one would sail: turned off, what
        // went past nature's comes back within it
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-extreme"), aurora_i18n::tr!("ui-sea-extreme-tip"));
            if widgets::checkbox(ui, p, &mut s.extreme, "").changed() && !s.extreme {
                let top = *sea::SCALE_RANGE.end();
                s.shape.swell = s.shape.swell.min(top);
                s.shape.chop = s.shape.chop.min(top);
                s.shape.foam = s.shape.foam.min(top);
                s.shape.state = s.shape.state.min(1.0);
                s.max_height = s.max_height.min(sea::NO_LIMIT);
            }
        });
        // the shape now, saved under a name
        ui.horizontal(|ui| {
            row_label(ui, p, aurora_i18n::tr!("ui-sea-save"), aurora_i18n::tr!("ui-sea-save-tip"));
            let field = ui.add(
                egui::TextEdit::singleline(name)
                    .hint_text(aurora_i18n::tr!("ui-sea-save-hint"))
                    .desired_width(COMBO_W - extra - 30.0),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = !name.trim().is_empty();
            if (widgets::icon_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("ui-sea-save-tip"), ok) || entered)
                && s.save_preset(name)
            {
                name.clear();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn beaufort_numbers_follow_the_scale() {
        use super::beaufort;
        assert_eq!(beaufort(0.0), 0);
        assert_eq!(beaufort(5.0), 3);
        assert_eq!(beaufort(14.0), 7);
        assert_eq!(beaufort(24.0), 9);
        assert_eq!(beaufort(40.0), 12);
    }
}
