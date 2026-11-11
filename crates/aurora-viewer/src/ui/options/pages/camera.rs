//! Preferences › Camera: the Legacy Viewer's "Move & View › View" and
//! "Mouselook" camera settings (panel_preferences_move.xml), the camera
//! presets and the phototools offsets. The page's reset also brings the
//! live view back behind the avatar (`OptionsResult::reset_camera`).

use super::super::COMBO_W;
use super::super::cx::{Cx, Nothing, field, fields};
use crate::camera::{CameraPreset, CameraSettings, DEFAULT_FOV, FOV_RANGE, SMOOTHING_RANGE};
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

/// An offset moved by hand: the preset becomes "Custom".
fn custom(changed: bool, c: &mut CameraSettings) -> bool {
    if changed {
        c.preset = CameraPreset::Custom;
    }
    changed
}

pub fn page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    match tab {
        0 => {
            cx.group(tr!("common-viewpoint"), |cx| {
                cx.greyed(cx.st.rlva_camera_preset_locked);
                let names: Vec<&str> = CameraPreset::ALL.iter().map(|v| v.label()).collect();
                cx.keywords(&names.join(" ")).row(
                    tr!("ui-options-camera-preset"),
                    tr!("ui-options-camera-firestorm-views-camera-presets"),
                    &fields!(camera.preset, camera.camera_offset, camera.focus_offset),
                    |ui, s| {
                        let c = &mut s.camera;
                        let mut ch = false;
                        let mut preset = c.preset;
                        egui::ComboBox::from_id_salt(ui.scope_id().with("camera_preset"))
                            .width(COMBO_W)
                            .selected_text(preset.label())
                            .show_ui(ui, |ui| {
                                for v in CameraPreset::ALL {
                                    ch |= ui.selectable_value(&mut preset, v, v.label()).changed();
                                }
                            });
                        if ch {
                            // the view as the user saved it (Controls window)
                            s.camera.apply_view(preset, &s.camera_views);
                        }
                        ch
                    },
                );
                cx.wide().row(
                    tr!("ui-options-camera-camera-position"),
                    tr!("ui-options-camera-offset-from-the-avatar-in-meters-x-forward-y"),
                    &field!(camera.camera_offset),
                    |ui, s| {
                        let ch = offset(ui, &mut s.camera.camera_offset);
                        custom(ch, &mut s.camera)
                    },
                );
                cx.wide().row(
                    tr!("ui-options-camera-focus-point"),
                    tr!("ui-options-camera-focus-point-relative-to-the-avatar-shift-wheel-moves"),
                    &field!(camera.focus_offset),
                    |ui, s| {
                        let ch = offset(ui, &mut s.camera.focus_offset);
                        custom(ch, &mut s.camera)
                    },
                );
                let p = cx.p;
                cx.row(
                    tr!("ui-options-camera-preset-angles"),
                    tr!("ui-options-camera-restores-the-preset-s-camera-position-and-focus-point"),
                    &Nothing,
                    |ui, s| {
                        let c = &mut s.camera;
                        let reset = crate::ui::widgets::action_button(
                            ui,
                            &p,
                            "restore",
                            tr!("ui-options-camera-reset-angles"),
                            crate::ui::widgets::ActionKind::Flat,
                            egui::vec2(0.0, 26.0),
                        )
                        .clicked();
                        if reset {
                            let preset = if c.preset == CameraPreset::Custom {
                                CameraPreset::Rear
                            } else {
                                c.preset
                            };
                            s.camera.apply_view(preset, &s.camera_views);
                        }
                        reset
                    },
                );
            });
            cx.group(tr!("common-camera-movement"), |cx| {
                let p = cx.p;
                cx.row(
                    tr!("ui-options-camera-viewing-angle"),
                    tr!("ui-options-camera-vertical-field-of-view-60-by-default-as-with"),
                    &field!(camera.fov),
                    |ui, s| {
                        let c = &mut s.camera;
                        let mut deg = c.fov.to_degrees();
                        // Keyboard zoom can go beyond the preference slider's
                        // narrower range; merely displaying it must not clamp it.
                        ui.spacing_mut().slider_width = super::super::SLIDER_W;
                        let ch = ui
                            .add(
                                egui::Slider::new(&mut deg, FOV_RANGE.start().to_degrees()..=FOV_RANGE.end().to_degrees())
                                    .suffix("°")
                                    .custom_parser(aurora_i18n::parse_number)
                                    .trailing_fill(true)
                                    .clamping(egui::SliderClamping::Edits),
                            )
                            .changed();
                        if ch {
                            c.fov = deg.to_radians();
                        }
                        let default = tr!("ui-options-camera-default-value");
                        if crate::ui::widgets::icon_button(ui, &p, "restore", default, true) {
                            c.fov = DEFAULT_FOV;
                            return true;
                        }
                        ch
                    },
                );
                cx.slider(
                    tr!("common-distance"),
                    tr!("ui-options-camera-camera-distance-behind-avatar-1-by-default"),
                    &field!(camera.offset_scale),
                    0.5..=3.0,
                    "×",
                );
                cx.slider(
                    tr!("ui-options-camera-transition-duration"),
                    tr!("ui-options-camera-transition-time-between-views-mouselook-return-behind-avatar-targeted"),
                    &field!(camera.zoom_time),
                    0.0..=4.0,
                    " s",
                );
                cx.slider(
                    tr!("ui-options-camera-smoothing"),
                    tr!("ui-options-camera-the-camera-follows-your-avatar-with-a-slight-lag"),
                    &field!(camera.smoothing),
                    SMOOTHING_RANGE,
                    "",
                );
                cx.slider(
                    tr!("ui-options-camera-flying-camera-lag"),
                    tr!("ui-options-camera-while-airborne-the-camera-lags-according-to-speed-2"),
                    &field!(camera.dynamic_strength),
                    0.0..=30.0,
                    "",
                );
            });
            // the headset's starting viewpoint (app.rs, toggle_vr)
            cx.group(tr!("ui-options-camera-vr"), |cx| {
                use crate::camera::VrView;
                cx.unfolded().choice(
                    tr!("ui-options-camera-vr-view"),
                    tr!("ui-options-camera-vr-view-help"),
                    &field!(camera.vr_view),
                    &[
                        (VrView::FirstPerson, tr!("ui-options-camera-vr-view-first-person"), false),
                        (VrView::DesktopCamera, tr!("ui-options-camera-vr-view-desktop-camera"), false),
                    ],
                );
                // read by open_renderer at the next start (#2309)
                cx.toggle(
                    tr!("ui-options-camera-vr-at-start"),
                    tr!("ui-options-camera-vr-at-start-help"),
                    &field!(camera.vr_at_start),
                );
                use crate::camera::{VrHand, VrLocomotion, VrTurn};
                cx.unfolded().choice(
                    tr!("ui-options-camera-vr-locomotion"),
                    tr!("ui-options-camera-vr-locomotion-help"),
                    &field!(camera.vr.locomotion),
                    &[
                        (VrLocomotion::Smooth, tr!("ui-options-camera-vr-locomotion-smooth"), false),
                        (VrLocomotion::Teleport, tr!("ui-options-camera-vr-locomotion-teleport"), false),
                        (VrLocomotion::CameraOnly, tr!("ui-options-camera-vr-locomotion-camera-only"), false),
                        (VrLocomotion::None, tr!("ui-options-camera-vr-locomotion-none"), false),
                    ],
                );
                use crate::camera::VrWalkDirection;
                cx.unfolded().choice(
                    tr!("ui-options-camera-vr-walk-direction"),
                    tr!("ui-options-camera-vr-walk-direction-help"),
                    &field!(camera.vr.walk_direction),
                    &[
                        (VrWalkDirection::Head, tr!("ui-options-camera-vr-walk-direction-head"), false),
                        (VrWalkDirection::Hand, tr!("ui-options-camera-vr-walk-direction-hand"), false),
                    ],
                );
                cx.slider(
                    tr!("ui-options-camera-vr-movement-speed"),
                    tr!("ui-options-camera-vr-movement-speed-help"),
                    &field!(camera.vr.movement_speed),
                    0.5..=8.0,
                    " m/s",
                );
                cx.unfolded().choice(
                    tr!("ui-options-camera-vr-turning"),
                    tr!("ui-options-camera-vr-turning-help"),
                    &field!(camera.vr.turn),
                    &[
                        (VrTurn::Smooth, tr!("ui-options-camera-vr-turning-smooth"), false),
                        (VrTurn::Snap, tr!("ui-options-camera-vr-turning-snap"), false),
                    ],
                );
                cx.slider(
                    tr!("ui-options-camera-vr-smooth-turn-speed"),
                    tr!("ui-options-camera-vr-smooth-turn-speed-help"),
                    &field!(camera.vr.smooth_turn_speed),
                    0.25..=6.0,
                    " rad/s",
                );
                cx.slider(
                    tr!("ui-options-camera-vr-snap-turn-angle"),
                    tr!("ui-options-camera-vr-snap-turn-angle-help"),
                    &field!(camera.vr.snap_turn_angle),
                    15.0f32.to_radians()..=90.0f32.to_radians(),
                    " rad",
                );
                cx.slider(
                    tr!("ui-options-camera-vr-thumbstick-dead-zone"),
                    tr!("ui-options-camera-vr-thumbstick-dead-zone-help"),
                    &field!(camera.vr.thumbstick_dead_zone),
                    0.0..=0.8,
                    "",
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-comfort-vignette"),
                    tr!("ui-options-camera-vr-comfort-vignette-help"),
                    &field!(camera.vr.comfort_vignette),
                );
                cx.slider(
                    tr!("ui-options-camera-vr-vignette-strength"),
                    tr!("ui-options-camera-vr-vignette-strength-help"),
                    &field!(camera.vr.vignette_strength),
                    0.1..=1.0,
                    "",
                );
                cx.unfolded().choice(
                    tr!("ui-options-camera-vr-dominant-hand"),
                    tr!("ui-options-camera-vr-dominant-hand-help"),
                    &field!(camera.vr.dominant_hand),
                    &[
                        (VrHand::Left, tr!("ui-options-camera-vr-dominant-hand-left"), false),
                        (VrHand::Right, tr!("ui-options-camera-vr-dominant-hand-right"), false),
                    ],
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-controller-pointer"),
                    tr!("ui-options-camera-vr-controller-pointer-help"),
                    &field!(camera.vr.controller_pointer),
                );
                cx.slider(
                    tr!("ui-options-camera-vr-pointer-distance"),
                    tr!("ui-options-camera-vr-pointer-distance-help"),
                    &field!(camera.vr.pointer_distance),
                    0.5..=64.0,
                    " m",
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-body-follows-head"),
                    tr!("ui-options-camera-vr-body-follows-head-help"),
                    &field!(camera.vr.body_follows_head),
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-avatar-hands"),
                    tr!("ui-options-camera-vr-avatar-hands-help"),
                    &field!(camera.vr.avatar_hands),
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-show-controllers"),
                    tr!("ui-options-camera-vr-show-controllers-help"),
                    &field!(camera.vr.show_controllers),
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-wrist-chat"),
                    tr!("ui-options-camera-vr-wrist-chat-help"),
                    &field!(camera.vr.wrist_chat),
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-panel-follows"),
                    tr!("ui-options-camera-vr-panel-follows-help"),
                    &field!(camera.vr.panel_follows),
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-tooltips"),
                    tr!("ui-options-camera-vr-tooltips-help"),
                    &field!(camera.vr.tooltips),
                );
                cx.toggle(
                    tr!("ui-options-camera-vr-haptics"),
                    tr!("ui-options-camera-vr-haptics-help"),
                    &field!(camera.vr.haptics),
                );
                cx.slider(
                    tr!("ui-options-camera-vr-forward-offset"),
                    tr!("ui-options-camera-vr-forward-offset-help"),
                    &field!(camera.vr.forward_offset),
                    0.0..=0.5,
                    " m",
                );
                cx.slider(
                    tr!("ui-options-camera-vr-height-offset"),
                    tr!("ui-options-camera-vr-height-offset-help"),
                    &field!(camera.vr.height_offset),
                    -1.0..=1.0,
                    " m",
                );
                for (label, help, field, range) in [
                    (
                        tr!("ui-options-camera-vr-exposure"),
                        tr!("ui-options-camera-vr-exposure-help"),
                        field!(camera.vr.exposure),
                        -4.0..=4.0,
                    ),
                    (
                        tr!("ui-options-camera-vr-brightness"),
                        tr!("ui-options-camera-vr-brightness-help"),
                        field!(camera.vr.brightness),
                        -1.0..=1.0,
                    ),
                    (
                        tr!("ui-options-camera-vr-contrast"),
                        tr!("ui-options-camera-vr-contrast-help"),
                        field!(camera.vr.contrast),
                        0.0..=2.0,
                    ),
                    (
                        tr!("ui-options-camera-vr-saturation"),
                        tr!("ui-options-camera-vr-saturation-help"),
                        field!(camera.vr.saturation),
                        0.0..=2.0,
                    ),
                    (
                        tr!("ui-options-camera-vr-gamma"),
                        tr!("ui-options-camera-vr-gamma-help"),
                        field!(camera.vr.gamma),
                        0.5..=2.5,
                    ),
                ] {
                    cx.slider(label, help, &field, range, "");
                }
                // every VR setting above back to its default, the rest of the page untouched
                let p = cx.p;
                cx.row(
                    tr!("ui-options-camera-vr-defaults"),
                    tr!("ui-options-camera-vr-defaults-help"),
                    &Nothing,
                    |ui, s| {
                        let d = crate::camera::CameraSettings::default();
                        let c = &mut s.camera;
                        let differs = c.vr != d.vr || c.vr_view != d.vr_view || c.vr_at_start != d.vr_at_start;
                        let reset = crate::ui::widgets::action_button_if(
                            ui,
                            &p,
                            "restore",
                            tr!("ui-options-camera-vr-restore-defaults"),
                            crate::ui::widgets::ActionKind::Flat,
                            egui::vec2(0.0, 26.0),
                            differs,
                        )
                        .clicked();
                        if reset {
                            c.vr = d.vr;
                            c.vr_view = d.vr_view;
                            c.vr_at_start = d.vr_at_start;
                        }
                        reset
                    },
                );
            });
            // what each controller button does, for reading in the headset
            cx.group(tr!("ui-options-camera-vr-controls"), |cx| {
                for (button, action) in [
                    (tr!("ui-vr-control-left-stick"), tr!("ui-vr-control-left-stick-action")),
                    (tr!("ui-vr-control-left-stick-click"), tr!("ui-vr-control-left-stick-click-action")),
                    (tr!("ui-vr-control-right-stick"), tr!("ui-vr-control-right-stick-action")),
                    (tr!("ui-vr-control-right-stick-tap"), tr!("ui-vr-control-right-stick-tap-action")),
                    (tr!("ui-vr-control-right-stick-hold"), tr!("ui-vr-control-right-stick-hold-action")),
                    (tr!("ui-vr-control-trigger"), tr!("ui-vr-control-trigger-action")),
                    (tr!("ui-vr-control-other-trigger"), tr!("ui-vr-control-other-trigger-action")),
                    (tr!("ui-vr-control-grip"), tr!("ui-vr-control-grip-action")),
                    (tr!("ui-vr-control-a"), tr!("ui-vr-control-a-action")),
                    (tr!("ui-vr-control-b"), tr!("ui-vr-control-b-action")),
                    (tr!("ui-vr-control-x"), tr!("ui-vr-control-x-action")),
                    (tr!("ui-vr-control-y"), tr!("ui-vr-control-y-action")),
                    (tr!("ui-vr-control-wrist"), tr!("ui-vr-control-wrist-action")),
                ] {
                    cx.row(button, action, &Nothing, |ui, _| {
                        ui.label(action);
                        false
                    });
                }
            });
        }
        1 => {
            cx.group(tr!("common-behavior"), |cx| {
                cx.toggle(
                    tr!("ui-options-camera-return-when-moving"),
                    tr!("ui-options-camera-moving-your-avatar-returns-the-camera-behind-it-after"),
                    &field!(camera.reset_on_movement),
                );
                // FSResetCameraOnTP: only a teleport inside the region reads it
                cx.toggle(
                    tr!("ui-options-camera-return-after-a-teleport-in-the-region"),
                    tr!("ui-options-camera-after-a-teleport-in-the-region-the-camera-returns"),
                    &field!(camera.reset_on_teleport),
                );
                cx.toggle(
                    tr!("ui-options-camera-esc-turns-avatar"),
                    tr!("ui-options-camera-esc-keeps-the-camera-direction-and-turns-the-avatar"),
                    &field!(camera.reset_view_turns_avatar),
                );
                cx.toggle(
                    tr!("ui-options-camera-click-my-avatar-keep-camera"),
                    tr!("ui-options-camera-clicking-your-avatar-does-not-return-the-camera-behind"),
                    &field!(camera.click_avatar_keeps_camera),
                );
                cx.toggle(
                    tr!("ui-options-camera-follow-the-targeted-object"),
                    tr!("ui-options-camera-the-camera-follows-the-object-it-is-focused-on"),
                    &field!(camera.track_focus_object),
                );
                cx.toggle(
                    tr!("ui-options-camera-follow-hover-height"),
                    tr!("ui-options-camera-avatar-hover-height-also-moves-the-camera"),
                    &field!(camera.hover_affects_camera),
                );
                cx.toggle(
                    tr!("ui-options-camera-no-mouse-wheel-zoom"),
                    tr!("ui-options-camera-the-mouse-wheel-does-not-move-the-camera-closer"),
                    &field!(camera.disable_wheel_zoom),
                );
            });
            cx.group(tr!("common-limits"), |cx| {
                cx.toggle(
                    tr!("ui-options-camera-unrestricted-camera"),
                    tr!("ui-options-camera-the-camera-can-move-beyond-the-draw-distance-below"),
                    &field!(camera.disable_constraints),
                );
                cx.toggle(
                    tr!("ui-options-camera-walk-through-objects"),
                    tr!("ui-options-camera-ignore-the-simulator-pushing-the-camera-in-front-of"),
                    &field!(camera.ignore_sim_constraints),
                );
            });
        }
        _ => {
            mouselook(cx);
            crosshair(cx);
        }
    }
    // the rest of the camera's settings (written by its shortcuts, its
    // windows): the page's reset puts them back too, as "Reset camera" did
    cx.track(&field!(camera));
}

/// Preferences › Camera › Mouselook: the Legacy Viewer's mouselook rows.
fn mouselook(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("common-mouselook"), |cx| {
        cx.toggle(
            tr!("ui-options-camera-enable-first-person-view"),
            tr!("ui-options-camera-press-m-or-zoom-into-your-avatar-s-head"),
            &field!(camera.enable_mouselook),
        );
        cx.toggle(
            tr!("ui-options-camera-show-avatar-mouselook"),
            tr!("ui-options-camera-show-avatar-mouselook-tip"),
            &field!(camera.show_avatar_in_mouselook),
        );
        cx.toggle(
            tr!("ui-options-camera-follow-eyes"),
            tr!("ui-options-camera-follow-eyes-tip"),
            &field!(camera.mouselook_follow_eyes),
        );
        cx.toggle(
            tr!("ui-options-camera-alt-right-click-menu"),
            tr!("ui-options-camera-alt-right-click-menu-tip"),
            &field!(camera.mouselook_alt_right_menu),
        );
        cx.toggle(
            tr!("ui-options-camera-exit-with-mouse-wheel"),
            tr!("ui-options-camera-scroll-toward-yourself-to-leave-mouselook"),
            &field!(camera.wheel_exits_mouselook),
        );
        cx.toggle(
            tr!("ui-options-camera-show-how-to-leave"),
            tr!("ui-options-camera-show-how-to-leave-tip"),
            &field!(camera.show_mouselook_instructions),
        );
        cx.toggle(
            tr!("ui-options-camera-show-interface"),
            tr!("ui-options-camera-show-interface-tip"),
            &field!(camera.show_interface_in_mouselook),
        );
    });
}

/// The crosshair in mouselook (#1166): ShowCrosshairs, and a game's
/// choices of shape, size and colour (`ui::crosshair`).
fn crosshair(cx: &mut Cx) {
    use crate::ui::crosshair::{self, Shape};
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-crosshair"), |cx| {
        cx.unfolded().toggle(
            tr!("ui-options-crosshair-show"),
            tr!("ui-options-crosshair-show-tip"),
            &field!(crosshair.show),
        );
        let off = !cx.s.crosshair.show;
        cx.subs(off, |cx| {
            let shape = cx.s.crosshair.shape;
            let lines = matches!(shape, Shape::Cross | Shape::Circle);
            cx.choice(
                tr!("ui-options-crosshair-shape"),
                tr!("ui-options-crosshair-shape-tip"),
                &field!(crosshair.shape),
                &[
                    (Shape::Dot, tr!("ui-options-crosshair-dot"), false),
                    (Shape::Cross, tr!("ui-options-crosshair-cross"), false),
                    (Shape::Circle, tr!("ui-options-crosshair-circle"), false),
                    (Shape::Legacy, tr!("ui-options-crosshair-legacy"), false),
                    (Shape::Image, tr!("common-image"), false),
                ],
            );
            cx.wide().row(tr!("ui-options-crosshair-preview"), "", &Nothing, |ui, s| {
                // AURORA_DEMO_CROSSHAIR's shape in the demo's captures too
                let shown = crosshair::demo(&s.crosshair);
                crosshair::preview(ui, &p, shown.as_ref().unwrap_or(&s.crosshair));
                false
            });
            cx.greyed(shape == Shape::Dot);
            cx.slider_whole(
                tr!("ui-options-crosshair-size"),
                tr!("ui-options-crosshair-size-tip"),
                &field!(crosshair.size),
                crosshair::SIZE,
                " px",
            );
            cx.greyed(!(shape == Shape::Dot || lines && cx.s.crosshair.center_dot));
            cx.slider_whole(
                tr!("ui-options-crosshair-dot-size"),
                "",
                &field!(crosshair.dot_size),
                crosshair::DOT_SIZE,
                " px",
            );
            cx.greyed(!lines);
            cx.slider_whole(
                tr!("ui-options-crosshair-thickness"),
                "",
                &field!(crosshair.thickness),
                crosshair::THICKNESS,
                " px",
            );
            cx.greyed(shape != Shape::Cross);
            cx.slider_whole(
                tr!("ui-options-crosshair-gap"),
                tr!("ui-options-crosshair-gap-tip"),
                &field!(crosshair.gap),
                crosshair::GAP,
                " px",
            );
            cx.greyed(!lines);
            cx.toggle(tr!("ui-options-crosshair-center-dot"), "", &field!(crosshair.center_dot));
            cx.greyed(false);
            cx.color(tr!("common-color"), tr!("ui-options-crosshair-color-tip"), &field!(crosshair.color));
            cx.greyed(shape == Shape::Image);
            cx.toggle(
                tr!("ui-options-crosshair-outline"),
                tr!("ui-options-crosshair-outline-tip"),
                &field!(crosshair.outline),
            );
            cx.greyed(shape != Shape::Image);
            cx.wide().row_with(
                tr!("ui-options-crosshair-image"),
                tr!("ui-options-crosshair-image-tip"),
                &field!(crosshair.image),
                |ui, s, st, _| pick_image(ui, &p, &mut s.crosshair.image, &mut st.crosshair_dialog),
            );
            cx.greyed(false);
        });
    });
}

/// The picture of the crosshair: its file's name, and a button that opens
/// the file dialog on its own thread (the frames go on meanwhile).
fn pick_image(
    ui: &mut egui::Ui,
    p: &crate::theme::Palette,
    image: &mut Option<std::path::PathBuf>,
    dialog: &mut Option<std::sync::mpsc::Receiver<std::path::PathBuf>>,
) -> bool {
    use crate::ui::widgets::{self, ActionKind};
    use aurora_i18n::tr;
    use std::sync::mpsc;
    let mut changed = false;
    if let Some(rx) = dialog {
        match rx.try_recv() {
            Ok(path) => {
                *image = Some(path);
                *dialog = None;
                changed = true;
            }
            Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after_secs(0.2),
            Err(mpsc::TryRecvError::Disconnected) => *dialog = None,
        }
    }
    ui.horizontal(|ui| {
        if widgets::action_button_if(
            ui,
            p,
            "photo-filled",
            tr!("ui-inventory-thumbnail-choose-image"),
            ActionKind::Flat,
            egui::Vec2::new(0.0, 26.0),
            dialog.is_none(),
        )
        .clicked()
        {
            let (tx, rx) = mpsc::channel();
            let title = tr!("ui-options-crosshair-image");
            let spawned = std::thread::Builder::new().name("aurora-crosshair-dialog".into()).spawn(move || {
                let picked = rfd::FileDialog::new()
                    .set_title(title)
                    .add_filter(title, &["png", "jpg", "jpeg", "tga", "bmp", "gif", "webp"])
                    .pick_file();
                if let Some(path) = picked {
                    let _ = tx.send(path);
                }
            });
            match spawned {
                Ok(_) => *dialog = Some(rx),
                Err(e) => log::warn!("crosshair picture: cannot open the file dialog: {e}"),
            }
        }
        if let Some(name) = image.as_deref().and_then(|path| path.file_name()) {
            ui.label(egui::RichText::new(name.to_string_lossy()).color(p.muted));
        }
    });
    changed
}
