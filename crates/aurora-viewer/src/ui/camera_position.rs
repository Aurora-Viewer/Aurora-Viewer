//! The camera position window (#4587): the camera's rest position behind
//! the avatar, exactly, from the Controls window's Position button.
//!
//! Port of LLFloaterPreferenceViewAdvanced
//! (indra/newview/llfloaterpreferenceviewadvanced.cpp,
//! floater_preferences_view_advanced.xml, originally LGPL 2.1): the camera
//! offset (CameraOffsetRearView) and the focus offset (FocusOffsetRearView)
//! in X / Y / Z, the distance (CameraOffsetScale) and "Save as preset...".
//! Like its `draw`, the fields show the live values each frame, so the
//! Ctrl / Shift wheel moves them while the window is open.
//!
//! Differences: Aurora adds "Reset angles" (Preferences › Camera's), the
//! distance keeps Preferences' range (0.5 to 3) where this window of the
//! Legacy Viewer offered -3 to 5, and the save names the camera position in
//! the window instead of opening another one. RLVa's camera preset lock
//! greys the whole window, as it greys Preferences' rows.

use super::controls::{ShotNaming, name_row};
use super::widgets::{self, ActionKind, Floater};
use crate::camera::{CameraPreset, CameraSettings};
use crate::settings::Settings;
use crate::theme::Palette;
use aurora_i18n::tr;
use egui::{Pos2, RichText, Vec2};

/// The window's id.
pub const WINDOW: &str = "camera_position";

const WIDTH: f32 = 330.0;
const BUTTON_H: f32 = 26.0;

/// The offsets' bounds: the Legacy Viewer's spinners (min_val / max_val of
/// floater_preferences_view_advanced.xml), so a value it accepts is kept.
pub const OFFSET_LIMIT: f32 = 1e7;

#[derive(Default)]
pub struct CameraPositionUi {
    naming: ShotNaming,
}

/// What the window did this frame.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    /// A setting changed: the app applies and saves it.
    pub changed: bool,
    /// "Save as camera position", with its name.
    pub save: Option<String>,
}

/// Three fields X / Y / Z (x forward, y left, z up), for Preferences and
/// this window. A value outside the fields' range, which the wheel or Debug
/// Settings may have set, stays as it is until the user edits it: only
/// drawing the field must not change the setting.
pub fn offset_fields(ui: &mut egui::Ui, v: &mut [f32; 3]) -> bool {
    let mut changed = false;
    for (axis, value) in ["X", "Y", "Z"].iter().zip(v.iter_mut()) {
        ui.label(RichText::new(*axis).size(12.0));
        changed |= ui
            .add(
                egui::DragValue::new(value)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(-OFFSET_LIMIT..=OFFSET_LIMIT)
                    .clamp_existing_to_range(false)
                    .speed(0.02)
                    .max_decimals(2),
            )
            .changed();
    }
    changed
}

/// An offset moved by hand: the preset becomes "Custom".
pub fn mark_custom(changed: bool, c: &mut CameraSettings) -> bool {
    if changed {
        c.preset = CameraPreset::Custom;
    }
    changed
}

/// "Reset angles": the view's own offsets back, the rear view's when they
/// were edited by hand (Preferences › Camera's button).
pub fn reset_angles(s: &mut Settings) {
    let preset = match s.camera.preset {
        CameraPreset::Custom => CameraPreset::Rear,
        preset => preset,
    };
    s.camera.apply_view(preset, &s.camera_views);
}

impl CameraPositionUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, s: &mut Settings, locked: bool) -> Output {
        let mut out = Output::default();
        if !*open {
            self.naming = ShotNaming::default();
            return out;
        }
        Floater::new(
            WINDOW,
            tr!("ui-options-camera-camera-position"),
            Pos2::new(340.0, 200.0),
            Vec2::new(WIDTH, 200.0),
        )
        .auto_height()
        .show(ctx, p, open, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let body = ui.add_enabled_ui(!locked, |ui| self.body(ui, p, s, &mut out));
            if locked {
                body.response.on_disabled_hover_text(tr!("camera-preset-locked"));
            }
        });
        out
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, out: &mut Output) {
        ui.label(RichText::new(tr!("camera-position-hint")).size(11.5).color(p.muted));
        egui::Grid::new("camera_position_grid")
            .num_columns(2)
            .spacing(Vec2::new(10.0, 8.0))
            .show(ui, |ui| {
                ui.label(tr!("common-camera"));
                ui.horizontal(|ui| {
                    let ch = offset_fields(ui, &mut s.camera.camera_offset);
                    out.changed |= mark_custom(ch, &mut s.camera);
                });
                ui.end_row();
                ui.label(tr!("ui-options-camera-focus-point"));
                ui.horizontal(|ui| {
                    let ch = offset_fields(ui, &mut s.camera.focus_offset);
                    out.changed |= mark_custom(ch, &mut s.camera);
                });
                ui.end_row();
                ui.label(tr!("common-distance"));
                let distance = ui
                    .add(widgets::restricted_number(
                        egui::Slider::new(&mut s.camera.offset_scale, 0.5..=3.0)
                            .suffix("×")
                            .custom_parser(aurora_i18n::parse_number)
                            .trailing_fill(true)
                            .clamping(egui::SliderClamping::Edits),
                        widgets::TextPolicy::Decimal,
                    ))
                    .on_hover_text(tr!("ui-options-camera-camera-distance-behind-avatar-1-by-default"));
                out.changed |= mark_custom(distance.changed(), &mut s.camera);
                ui.end_row();
            });
        let height = BUTTON_H;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if self.naming.open {
                let width = ui.available_width() - height - 4.0;
                if let Some(name) = name_row(ui, p, &mut self.naming, width) {
                    out.save = Some(name);
                }
                return;
            }
            let reset = widgets::action_button(
                ui,
                p,
                "restore",
                tr!("ui-options-camera-reset-angles"),
                ActionKind::Flat,
                Vec2::new(0.0, height),
            )
            .on_hover_text(tr!("ui-options-camera-restores-the-preset-s-camera-position-and-focus-point"));
            if reset.clicked() {
                reset_angles(s);
                out.changed = true;
            }
            let save = widgets::action_button(
                ui,
                p,
                "device-floppy-filled",
                tr!("camera-position-save"),
                ActionKind::Flat,
                Vec2::new(0.0, height),
            )
            .on_hover_text(tr!("controls-shot-save-tip"));
            if save.clicked() {
                self.naming.start();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame; no renderer here: the font atlas would trip
    /// TexturesDelta's drop check (debug assertions, as in the merge queue).
    fn run(ctx: &egui::Context, mut f: impl FnMut(&mut egui::Ui)) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                ..Default::default()
            },
            |ui| f(ui),
        );
        output.textures_delta.clear();
    }

    fn frame(ui_fn: impl FnOnce(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let mut ui_fn = Some(ui_fn);
        run(&ctx, |ui| {
            if let Some(f) = ui_fn.take() {
                f(ui);
            }
        });
    }

    /// The window drawn for a few frames, with the theme's palette.
    fn window(w: &mut CameraPositionUi, open: &mut bool, s: &mut Settings, locked: bool) -> Vec<Output> {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let mut outs = Vec::new();
        for _ in 0..3 {
            run(&ctx, |ui| outs.push(w.show(ui.ctx(), &p, open, s, locked)));
        }
        outs
    }

    #[test]
    fn drawing_the_fields_keeps_a_value_beyond_ten_metres() {
        // the Ctrl wheel or Debug Settings may set it: Preferences used to
        // cut it back to 10 m, and save it, only by being shown
        let mut v = [-25.0, 0.0, 14.5];
        let mut changed = true;
        frame(|ui| changed = offset_fields(ui, &mut v));
        assert!(!changed, "showing the fields changes nothing");
        assert_eq!(v, [-25.0, 0.0, 14.5]);
    }

    #[test]
    fn the_fields_take_the_legacy_viewers_range() {
        // floater_preferences_view_advanced.xml: min_val -1e7, max_val 1e7
        assert_eq!(OFFSET_LIMIT, 1e7);
    }

    #[test]
    fn an_offset_moved_by_hand_makes_the_preset_custom() {
        let mut c = CameraSettings::default();
        assert!(!mark_custom(false, &mut c));
        assert_eq!(c.preset, CameraPreset::Rear);
        assert!(mark_custom(true, &mut c));
        assert_eq!(c.preset, CameraPreset::Custom);
    }

    #[test]
    fn reset_angles_brings_the_views_offsets_back() {
        let mut s = Settings::default();
        let rear = CameraSettings::default();
        s.camera.camera_offset = [-9.0, 2.0, 4.0];
        s.camera.focus_offset = [1.0, -1.0, 3.0];
        s.camera.preset = CameraPreset::Custom;
        reset_angles(&mut s);
        assert_eq!(
            s.camera.preset,
            CameraPreset::Rear,
            "a hand-made position goes back to the rear view"
        );
        assert_eq!(s.camera.camera_offset, rear.camera_offset);
        assert_eq!(s.camera.focus_offset, rear.focus_offset);
        // a view keeps its own values
        s.camera.apply_view(CameraPreset::Front, &s.camera_views.clone());
        let front = (s.camera.camera_offset, s.camera.focus_offset);
        s.camera.camera_offset[2] += 1.0;
        reset_angles(&mut s);
        assert_eq!(s.camera.preset, CameraPreset::Front);
        assert_eq!((s.camera.camera_offset, s.camera.focus_offset), front);
    }

    #[test]
    fn a_closed_window_drops_a_name_being_typed() {
        let mut w = CameraPositionUi::default();
        w.naming.start();
        w.naming.name.push_str("Close");
        let mut s = Settings::default();
        let mut open = false;
        let outs = window(&mut w, &mut open, &mut s, false);
        let out = outs.last().expect("frames");
        assert_eq!(*out, Output::default());
        assert!(!w.naming.open && w.naming.name.is_empty());
    }

    #[test]
    fn the_open_window_shows_offsets_past_ten_metres_without_changing_them() {
        for locked in [false, true] {
            let mut s = Settings::default();
            s.camera.camera_offset = [-30.0, 0.5, 12.0];
            s.camera.focus_offset = [0.0, 0.0, -11.0];
            let before = s.camera.clone();
            let mut open = true;
            let outs = window(&mut CameraPositionUi::default(), &mut open, &mut s, locked);
            assert!(open, "the window stays open");
            assert!(outs.iter().all(|o| *o == Output::default()), "nothing changes by being shown");
            assert_eq!(s.camera, before);
        }
    }
}
