//! LLFloaterHoverHeight (Firestorm llfloaterhoverheight.cpp, LGPL 2.1).
use super::widgets::{self, ActionKind, Floater};
use crate::scene::{HoverDiagnostics, HoverStatus};
use crate::theme::Palette;
use crate::world::hover_height::{self, HoverHeight};

pub struct HoverHeightTest<'a> {
    pub neutral: &'a mut bool,
    pub diagnostics: HoverDiagnostics,
}

#[derive(Default)]
pub struct HoverHeightUi {
    pub open: bool,
    pub measure_requested: bool,
    editing: bool,
}

impl HoverHeightUi {
    /// Returns whether a saved preference changed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        height: &mut HoverHeight,
        enabled: bool,
        preferences: (&mut bool, &mut bool),
        test: Option<HoverHeightTest<'_>>,
    ) -> bool {
        let (camera, automatic) = preferences;
        let enabled = enabled && height.ready();
        let mut camera_changed = false;
        Floater::new(
            "hover_height",
            aurora_i18n::tr!("ui-context-hover-height"),
            egui::pos2(480.0, 100.0),
            egui::vec2(440.0, if test.is_some() { 390.0 } else { 180.0 }),
        )
        .fixed()
        .show(ctx, p, &mut self.open, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let mut value = height.value();
                ui.spacing_mut().slider_width = 260.0;
                let response = ui.add(
                    egui::Slider::new(&mut value, hover_height::MIN..=hover_height::MAX)
                        .step_by(0.001)
                        .fixed_decimals(3)
                        .suffix(" m")
                        .custom_formatter(|value, _| aurora_i18n::number(value, 3))
                        .custom_parser(aurora_i18n::parse_number),
                );
                if response.changed() {
                    height.preview(value);
                    self.editing = true;
                }
                ui.horizontal(|ui| {
                    let step = ui.input(|i| hover_height::step(i.modifiers.alt, i.modifiers.ctrl || i.modifiers.shift));
                    for (icon, label, value) in [
                        ("minus", aurora_i18n::tr!("hover-height-lower"), height.value() - step),
                        ("restore", aurora_i18n::tr!("common-reset"), 0.0),
                        ("plus", aurora_i18n::tr!("hover-height-raise"), height.value() + step),
                    ] {
                        if widgets::action_button(ui, p, icon, label, ActionKind::Flat, egui::vec2(0.0, 26.0)).clicked() {
                            height.preview(value);
                            height.commit();
                        }
                    }
                });
            });
            ui.add_space(4.0);
            camera_changed |= widgets::switch_labeled(ui, p, automatic, aurora_i18n::tr!("hover-height-automatic")).changed();
            camera_changed |= widgets::switch_labeled(ui, p, camera, aurora_i18n::tr!("hover-height-affect-camera")).changed();
            if let Some(test) = test {
                widgets::switch_labeled(ui, p, test.neutral, aurora_i18n::tr!("hover-height-neutral-pose-test"))
                    .on_hover_text(aurora_i18n::tr!("hover-height-neutral-pose-test-tip"));
                let d = test.diagnostics;
                let status = match d.status {
                    HoverStatus::Disabled => aurora_i18n::tr!("hover-test-disabled"),
                    HoverStatus::Unavailable => aurora_i18n::tr!("hover-test-unavailable"),
                    HoverStatus::WaitingAvatar => aurora_i18n::tr!("hover-test-avatar"),
                    HoverStatus::WaitingGround => aurora_i18n::tr!("hover-test-ground"),
                    HoverStatus::WaitingAssets => aurora_i18n::tr!("hover-test-assets"),
                    HoverStatus::Calculating => aurora_i18n::tr!("hover-test-calculating"),
                    HoverStatus::Failed => aurora_i18n::tr!("hover-test-failed"),
                    HoverStatus::Ready => aurora_i18n::tr!("hover-test-ready"),
                };
                ui.add_space(6.0);
                ui.label(status);
                ui.label(if d.contact {
                    aurora_i18n::tr!("hover-test-contact")
                } else {
                    aurora_i18n::tr!("hover-test-estimated-ground")
                });
                let number = |v: Option<f32>| v.map_or_else(|| "—".into(), |v| aurora_i18n::number(v.into(), 3));
                ui.label(aurora_i18n::tr!("hover-test-gap", value = number(d.gap)))
                    .on_hover_text(aurora_i18n::tr!("hover-test-gap-tip"));
                ui.label(aurora_i18n::tr!("hover-test-clearance", value = number(d.clearance)));
                ui.label(aurora_i18n::tr!("hover-test-half-height", value = number(d.half_height)));
                ui.label(aurora_i18n::tr!("hover-test-estimate", value = number(d.estimate)));
                if ui
                    .add_enabled(enabled && *automatic, egui::Button::new(aurora_i18n::tr!("hover-test-recalculate")))
                    .clicked()
                {
                    self.measure_requested = true;
                }
            }
            if !enabled {
                ui.label(egui::RichText::new(aurora_i18n::tr!("hover-height-unavailable")).color(p.muted));
            } else if height.failed {
                ui.label(egui::RichText::new(aurora_i18n::tr!("hover-height-send-failed")).color(p.danger));
                if ui.button(aurora_i18n::tr!("hover-height-retry")).clicked() {
                    height.retry();
                }
            }
        });
        // A drag only previews; release commits once, including release
        // outside the slider or closing the floater during the gesture.
        if self.editing && !ctx.input(|i| i.pointer.any_down()) {
            if enabled {
                height.commit();
            }
            self.editing = false;
        }
        camera_changed
    }
}
