//! Preferences › Camera: Firestorm's "Move & View › View" and
//! "Mouselook" camera settings (panel_preferences_move.xml), the
//! camera presets and the phototools offsets.

use super::{SLIDER_W, group, row, slider, toggle};
use crate::camera::{CameraPreset, CameraSettings, DEFAULT_FOV, FOV_RANGE};
use crate::settings::Settings;
use crate::theme::Palette;
use egui::RichText;

/// Three fields x / y / z (x forward, y left, z up).
fn offset(ui: &mut egui::Ui, v: &mut [f32; 3]) -> bool {
    let mut changed = false;
    for (axis, value) in ["X", "Y", "Z"].iter().zip(v.iter_mut()) {
        ui.label(RichText::new(*axis).size(12.0));
        changed |= ui
            .add(
                egui::DragValue::new(value)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(-10.0..=10.0)
                    .speed(0.02)
                    .max_decimals(2),
            )
            .changed();
    }
    changed
}

/// `reset_view`: the live view is to go back behind the avatar too.
pub(super) fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, reset_view: &mut bool) -> bool {
    let c = &mut s.camera;
    let mut changed = false;
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-options-camera-alt-click-focus-on-a-point-alt-drag-zoom"))
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(6.0);
    // everything on this page back to its default: the shortcuts that move
    // the follow camera (offsets, angle of view, distance) write these
    // settings, and nothing else brought them all back at once
    let reset_all = crate::ui::widgets::action_button(
        ui,
        p,
        "restore",
        aurora_i18n::tr!("ui-options-camera-reset-all"),
        crate::ui::widgets::ActionKind::Flat,
        egui::Vec2::new(0.0, 26.0),
    )
    .on_hover_text(aurora_i18n::tr!("ui-options-camera-reset-all-tip"))
    .clicked();
    if reset_all {
        *c = CameraSettings::default();
        *reset_view = true;
        changed = true;
    }
    ui.add_space(8.0);
    group(ui, p, aurora_i18n::tr!("common-viewpoint"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-preset"),
            aurora_i18n::tr!("ui-options-camera-firestorm-views-camera-presets"),
            |ui| {
                let mut ch = false;
                let mut preset = c.preset;
                egui::ComboBox::from_id_salt("camera_preset")
                    .width(SLIDER_W)
                    .selected_text(preset.label())
                    .show_ui(ui, |ui| {
                        for v in CameraPreset::ALL {
                            ch |= ui.selectable_value(&mut preset, v, v.label()).changed();
                        }
                    });
                if ch {
                    c.apply_preset(preset);
                }
                ch
            },
        );
        let custom = |ch: bool, c: &mut CameraSettings| {
            if ch {
                c.preset = CameraPreset::Custom;
            }
            ch
        };
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-camera-position"),
            aurora_i18n::tr!("ui-options-camera-offset-from-the-avatar-in-meters-x-forward-y"),
            |ui| {
                let ch = offset(ui, &mut c.camera_offset);
                custom(ch, c)
            },
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-focus-point"),
            aurora_i18n::tr!("ui-options-camera-focus-point-relative-to-the-avatar-shift-wheel-moves"),
            |ui| {
                let ch = offset(ui, &mut c.focus_offset);
                custom(ch, c)
            },
        );
        changed |= row(ui, p, "", "", |ui| {
            let reset = crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-camera-reset-angles"))
                .on_hover_text(aurora_i18n::tr!(
                    "ui-options-camera-restores-the-preset-s-camera-position-and-focus-point"
                ))
                .clicked();
            if reset {
                let preset = if c.preset == CameraPreset::Custom {
                    CameraPreset::Rear
                } else {
                    c.preset
                };
                c.apply_preset(preset);
            }
            reset
        });
    });
    group(ui, p, aurora_i18n::tr!("common-camera-movement"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-viewing-angle"),
            aurora_i18n::tr!("ui-options-camera-vertical-field-of-view-60-by-default-as-with"),
            |ui| {
                let mut deg = c.fov.to_degrees();
                let ch = slider(ui, &mut deg, FOV_RANGE.start().to_degrees()..=FOV_RANGE.end().to_degrees(), "°");
                if ch {
                    c.fov = deg.to_radians();
                }
                let default = aurora_i18n::tr!("ui-options-camera-default-value");
                if crate::ui::widgets::icon_button(ui, p, "restore", default, true) {
                    c.fov = DEFAULT_FOV;
                    return true;
                }
                ch
            },
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("common-distance"),
            aurora_i18n::tr!("ui-options-camera-camera-distance-behind-avatar-1-by-default"),
            |ui| slider(ui, &mut c.offset_scale, 0.5..=3.0, "×"),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-transition-duration"),
            aurora_i18n::tr!("ui-options-camera-transition-time-between-views-mouselook-return-behind-avatar-targeted"),
            |ui| slider(ui, &mut c.zoom_time, 0.0..=4.0, " s"),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-smoothing"),
            aurora_i18n::tr!("ui-options-camera-the-camera-follows-your-avatar-with-a-slight-lag"),
            |ui| slider(ui, &mut c.smoothing, 0.0..=9.0, ""),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-flying-camera-lag"),
            aurora_i18n::tr!("ui-options-camera-while-airborne-the-camera-lags-according-to-speed-2"),
            |ui| slider(ui, &mut c.dynamic_strength, 0.0..=30.0, ""),
        );
    });
    group(ui, p, aurora_i18n::tr!("common-behavior"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-return-when-moving"),
            aurora_i18n::tr!("ui-options-camera-moving-your-avatar-returns-the-camera-behind-it-after"),
            |ui| toggle(ui, p, &mut c.reset_on_movement),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-return-after-teleporting"),
            aurora_i18n::tr!("ui-options-camera-the-camera-returns-behind-your-avatar-on-arrival"),
            |ui| toggle(ui, p, &mut c.reset_on_teleport),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-esc-turns-avatar"),
            aurora_i18n::tr!("ui-options-camera-esc-keeps-the-camera-direction-and-turns-the-avatar"),
            |ui| toggle(ui, p, &mut c.reset_view_turns_avatar),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-click-my-avatar-keep-camera"),
            aurora_i18n::tr!("ui-options-camera-clicking-your-avatar-does-not-return-the-camera-behind"),
            |ui| toggle(ui, p, &mut c.click_avatar_keeps_camera),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-follow-the-targeted-object"),
            aurora_i18n::tr!("ui-options-camera-the-camera-follows-the-object-it-is-focused-on"),
            |ui| toggle(ui, p, &mut c.track_focus_object),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-follow-hover-height"),
            aurora_i18n::tr!("ui-options-camera-avatar-hover-height-also-moves-the-camera"),
            |ui| toggle(ui, p, &mut c.hover_affects_camera),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-no-mouse-wheel-zoom"),
            aurora_i18n::tr!("ui-options-camera-the-mouse-wheel-does-not-move-the-camera-closer"),
            |ui| toggle(ui, p, &mut c.disable_wheel_zoom),
        );
    });
    group(ui, p, aurora_i18n::tr!("common-limits"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-unrestricted-camera"),
            aurora_i18n::tr!("ui-options-camera-the-camera-can-move-beyond-the-draw-distance-below"),
            |ui| toggle(ui, p, &mut c.disable_constraints),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-walk-through-objects"),
            aurora_i18n::tr!("ui-options-camera-ignore-the-simulator-pushing-the-camera-in-front-of"),
            |ui| toggle(ui, p, &mut c.ignore_sim_constraints),
        );
    });
    group(ui, p, aurora_i18n::tr!("common-mouselook"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-enable-first-person-view"),
            aurora_i18n::tr!("ui-options-camera-press-m-or-zoom-into-your-avatar-s-head"),
            |ui| toggle(ui, p, &mut c.enable_mouselook),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-exit-with-mouse-wheel"),
            aurora_i18n::tr!("ui-options-camera-scroll-toward-yourself-to-leave-mouselook"),
            |ui| toggle(ui, p, &mut c.wheel_exits_mouselook),
        );
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-camera-show-how-to-leave"),
            aurora_i18n::tr!("ui-options-camera-show-how-to-leave-tip"),
            |ui| toggle(ui, p, &mut c.show_mouselook_instructions),
        );
    });
    changed
}
