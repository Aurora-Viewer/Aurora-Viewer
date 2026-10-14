//! LLFloaterHoverHeight (Firestorm llfloaterhoverheight.cpp, LGPL 2.1).
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::hover_height::{self, HoverHeight};

pub struct HoverHeightOptions<'a> {
    pub camera: &'a mut bool,
    pub automatic: &'a mut bool,
    pub offset: &'a mut f32,
    pub automatic_base: Option<f32>,
    pub neutral_pose_test: &'a mut Option<bool>,
}

#[derive(Default)]
pub struct HoverHeightUi {
    pub open: bool,
    editing: bool,
    offset_editing: bool,
    #[cfg(test)]
    offset_rect: Option<egui::Rect>,
}

impl HoverHeightUi {
    pub fn previewing_offset(&self) -> bool {
        self.offset_editing
    }

    /// Returns whether a saved preference changed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        height: &mut HoverHeight,
        enabled: bool,
        options: HoverHeightOptions<'_>,
    ) -> bool {
        let HoverHeightOptions {
            camera,
            automatic,
            offset,
            automatic_base,
            neutral_pose_test,
        } = options;
        let enabled = enabled && height.ready();
        let mut preferences_changed = false;
        Floater::new(
            "hover_height",
            aurora_i18n::tr!("ui-context-hover-height"),
            egui::pos2(480.0, 100.0),
            egui::vec2(440.0, if neutral_pose_test.is_some() { 280.0 } else { 244.0 }),
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
            preferences_changed |= widgets::switch_labeled(ui, p, automatic, aurora_i18n::tr!("hover-height-automatic")).changed();
            ui.add_enabled_ui(*automatic, |ui| {
                ui.spacing_mut().slider_width = 260.0;
                ui.label(aurora_i18n::tr!("hover-height-auto-offset"))
                    .on_hover_text(aurora_i18n::tr!("hover-height-auto-offset-tip"));
                let response = ui.add(
                    egui::Slider::new(offset, hover_height::MIN..=hover_height::MAX)
                        .step_by(0.001)
                        .fixed_decimals(3)
                        .suffix(" m")
                        .custom_formatter(|value, _| aurora_i18n::number(value, 3))
                        .custom_parser(|text| aurora_i18n::parse_number(text).filter(|value| value.is_finite())),
                );
                #[cfg(test)]
                {
                    self.offset_rect = Some(response.rect);
                }
                if response.changed() {
                    if enabled && let Some(base) = automatic_base {
                        height.preview(base + *offset);
                    }
                    self.offset_editing = true;
                }
            });
            preferences_changed |= widgets::switch_labeled(ui, p, camera, aurora_i18n::tr!("hover-height-affect-camera")).changed();
            if let Some(neutral) = neutral_pose_test {
                widgets::switch_labeled(ui, p, neutral, aurora_i18n::tr!("hover-height-neutral-pose-test"))
                    .on_hover_text(aurora_i18n::tr!("hover-height-neutral-pose-test-tip"));
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
        if self.offset_editing && !ctx.input(|i| i.pointer.any_down()) {
            if enabled && *automatic {
                height.commit();
            }
            preferences_changed = true;
            self.offset_editing = false;
        }
        preferences_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetCommand;

    struct Fixture {
        ctx: egui::Context,
        panel: HoverHeightUi,
        height: HoverHeight,
        automatic: bool,
        offset: f32,
    }
    impl Fixture {
        fn new() -> Self {
            let mut height = HoverHeight::default();
            height.preview(0.2);
            height.commit();
            let Some(NetCommand::SetHoverHeight { request, .. }) = height.next_update(Some(1)) else {
                panic!("initial height");
            };
            height.reply(request, 1, true);
            Self {
                ctx: egui::Context::default(),
                panel: HoverHeightUi {
                    open: true,
                    ..Default::default()
                },
                height,
                automatic: true,
                offset: 0.0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            let mut saved = false;
            let mut camera = false;
            let mut neutral = Some(true);
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    saved = self.panel.show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &mut self.height,
                        true,
                        HoverHeightOptions {
                            camera: &mut camera,
                            automatic: &mut self.automatic,
                            offset: &mut self.offset,
                            automatic_base: Some(0.2),
                            neutral_pose_test: &mut neutral,
                        },
                    );
                },
            );
            output.textures_delta.clear(); // no renderer in this input test
            saved
        }
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    #[test]
    fn automatic_offset_slider_previews_then_saves_on_release_and_disables_with_auto() {
        let mut f = Fixture::new();
        for _ in 0..3 {
            f.frame(Vec::new());
        }
        let rect = f.panel.offset_rect.unwrap();
        let point = egui::pos2(rect.left() + 195.0, rect.center().y);
        assert!(!f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]));
        assert!(f.panel.previewing_offset());
        assert!(f.offset > 0.0);
        assert!((f.height.value() - hover_height::bounded(0.2 + f.offset).unwrap()).abs() < 1e-5);
        assert!(
            f.height.next_update(Some(1)).is_none(),
            "dragging must not persist or send the preview"
        );
        let point = point - egui::vec2(20.0, 0.0);
        assert!(!f.frame(vec![egui::Event::PointerMoved(point)]));
        assert!(f.height.next_update(Some(1)).is_none());
        assert!(f.frame(vec![button(point, false)]));
        assert!(!f.panel.previewing_offset());
        let Some(NetCommand::SetHoverHeight { request, value, .. }) = f.height.next_update(Some(1)) else {
            panic!("released offset");
        };
        assert_eq!(value, f.height.value());
        f.height.reply(request, 1, true);
        let saved_offset = f.offset;
        f.automatic = false;
        f.frame(Vec::new());
        assert!(!f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]));
        assert!(!f.frame(vec![button(point, false)]));
        assert_eq!(f.offset, saved_offset);
        assert!(f.height.next_update(Some(1)).is_none());
    }
}
