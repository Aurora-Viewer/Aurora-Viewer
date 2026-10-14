//! Preferences › Graphics › Depth of field (panel_preferences_graphics1.xml,
//! "DOF panel": RenderDepthOfField, RenderDepthOfFieldInEditMode,
//! FSFocusPointFollowsPointer, CameraFNumber, CameraFocalLength,
//! CameraFieldOfView, CameraFocusTransitionTime, CameraDoFResScale, with
//! Phototools' CameraMaxCoF and focus lock).

use super::{SLIDER_W, group, row, toggle};
use crate::dof::{FocusMode, Settings};
use crate::theme::Palette;

/// A slider whose steps grow with its value (f-numbers, focal lengths).
fn log_slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, prefix: &str, suffix: &str, decimals: usize) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    ui.add(
        egui::Slider::new(v, range)
            .logarithmic(true)
            .prefix(prefix)
            .suffix(suffix)
            .fixed_decimals(decimals)
            .custom_parser(aurora_i18n::parse_number)
            .trailing_fill(true),
    )
    .changed()
}

fn plain_slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, decimals: usize) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    ui.add(
        egui::Slider::new(v, range)
            .suffix(suffix)
            .fixed_decimals(decimals)
            .custom_parser(aurora_i18n::parse_number)
            .trailing_fill(true),
    )
    .changed()
}

/// The group; true when a setting changed.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("ui-options-dof"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-dof-enable"),
            aurora_i18n::tr!("ui-options-dof-enable-help"),
            |ui| toggle(ui, p, &mut s.enabled),
        );
        let on = s.enabled;
        ui.add_enabled_ui(on, |ui| {
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-edit"),
                aurora_i18n::tr!("ui-options-dof-edit-help"),
                |ui| toggle(ui, p, &mut s.in_edit),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-focus"),
                aurora_i18n::tr!("ui-options-dof-focus-help"),
                |ui| {
                    let modes = [FocusMode::Camera, FocusMode::Pointer, FocusMode::Click, FocusMode::Locked];
                    let labels = [
                        aurora_i18n::tr!("ui-options-dof-focus-camera"),
                        aurora_i18n::tr!("ui-options-dof-focus-pointer"),
                        aurora_i18n::tr!("ui-options-dof-focus-click"),
                        aurora_i18n::tr!("ui-options-dof-focus-locked"),
                    ];
                    let mut i = modes.iter().position(|m| *m == s.focus).unwrap_or(0);
                    let options: Vec<(&str, bool)> = labels.iter().map(|l| (*l, true)).collect();
                    let changed = crate::ui::widgets::segmented(ui, p, &mut i, &options);
                    if changed {
                        s.focus = modes[i];
                    }
                    changed
                },
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-aperture"),
                aurora_i18n::tr!("ui-options-dof-aperture-help"),
                |ui| log_slider(ui, &mut s.f_number, 1.0..=64.0, "f/", "", 1),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-focal-length"),
                aurora_i18n::tr!("ui-options-dof-focal-length-help"),
                |ui| log_slider(ui, &mut s.focal_length, 1.0..=1000.0, "", " mm", 0),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-reference-fov"),
                aurora_i18n::tr!("ui-options-dof-reference-fov-help"),
                |ui| plain_slider(ui, &mut s.reference_fov, 10.0..=200.0, "°", 0),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-max-blur"),
                aurora_i18n::tr!("ui-options-dof-max-blur-help"),
                |ui| plain_slider(ui, &mut s.max_coc, 1.0..=64.0, " px", 0),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-transition"),
                aurora_i18n::tr!("ui-options-dof-transition-help"),
                |ui| plain_slider(ui, &mut s.transition, 0.1..=10.0, " s", 2),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-dof-quality"),
                aurora_i18n::tr!("ui-options-dof-quality-help"),
                |ui| plain_slider(ui, &mut s.quality, 0.25..=1.0, "", 2),
            );
        });
    });
    c
}
