//! LLFloaterHoverHeight (Firestorm llfloaterhoverheight.cpp, LGPL 2.1).
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::hover_height::{self, HoverHeight, Posture};

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
    /// 0 standing, 1 sitting on an object, 2 sitting on the ground.
    tab: usize,
    /// The avatar's posture when the tab last followed it.
    followed: Option<Posture>,
    seated_editing: Option<Posture>,
    #[cfg(test)]
    seated_rects: Vec<egui::Rect>,
    #[cfg(test)]
    offset_rect: Option<egui::Rect>,
    #[cfg(test)]
    manual_rect: Option<egui::Rect>,
    #[cfg(test)]
    button_rects: Vec<egui::Rect>,
    #[cfg(test)]
    auto_button_rects: Vec<egui::Rect>,
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
            egui::vec2(440.0, if neutral_pose_test.is_some() { 368.0 } else { 332.0 }),
        )
        .fixed()
        .show(ctx, p, &mut self.open, |ui| {
            // Sitting down or standing up shows the height that now applies.
            let posture = height.posture();
            if self.followed != Some(posture) {
                self.followed = Some(posture);
                self.tab = match posture {
                    Posture::Standing => 0,
                    Posture::OnObject => 1,
                    Posture::OnGround => 2,
                };
            }
            widgets::stretched_tabs(
                ui,
                p,
                &mut self.tab,
                &[
                    (aurora_i18n::tr!("hover-height-standing"), true),
                    (aurora_i18n::tr!("hover-height-on-object"), true),
                    (aurora_i18n::tr!("hover-height-on-ground"), true),
                ],
            );
            if self.tab != 0 {
                let posture = if self.tab == 1 { Posture::OnObject } else { Posture::OnGround };
                let _rects = seated_tab(ui, p, height, enabled, posture, &mut self.seated_editing);
                #[cfg(test)]
                {
                    self.seated_rects = _rects.to_vec();
                }
            } else {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.scope(|ui| {
                        if *automatic {
                            ui.set_opacity(0.72); // visually secondary, still interactive
                        }
                        let mut value = height.value();
                        ui.spacing_mut().slider_width = 260.0;
                        let response = widgets::height_slider(ui, p, &mut value, hover_height::MIN..=hover_height::MAX, *automatic)
                            .on_hover_text(aurora_i18n::tr!("hover-height-manual-tip"));
                        #[cfg(test)]
                        {
                            self.manual_rect = Some(response.rect);
                            self.button_rects.clear();
                        }
                        if response.changed() {
                            preferences_changed |= std::mem::replace(automatic, false);
                            self.offset_editing = false;
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
                                let response = widgets::action_button(ui, p, icon, label, ActionKind::Flat, egui::vec2(0.0, 26.0));
                                #[cfg(test)]
                                {
                                    self.button_rects.push(response.rect);
                                }
                                if response.clicked() {
                                    preferences_changed |= std::mem::replace(automatic, false);
                                    self.offset_editing = false;
                                    height.preview(value);
                                    height.commit();
                                }
                            }
                        });
                    });
                });
                widgets::divider(ui, p);
                preferences_changed |= widgets::switch_labeled(ui, p, automatic, aurora_i18n::tr!("hover-height-automatic")).changed();
                ui.add_enabled_ui(*automatic, |ui| {
                    ui.spacing_mut().slider_width = 260.0;
                    ui.label(aurora_i18n::tr!("hover-height-auto-offset"))
                        .on_hover_text(aurora_i18n::tr!("hover-height-auto-offset-tip"));
                    let response = widgets::height_slider(ui, p, offset, hover_height::MIN..=hover_height::MAX, !*automatic);
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
                    #[cfg(test)]
                    self.auto_button_rects.clear();
                    ui.horizontal(|ui| {
                        let step = ui.input(|i| hover_height::step(i.modifiers.alt, i.modifiers.ctrl || i.modifiers.shift));
                        for (icon, label, value) in [
                            ("minus", aurora_i18n::tr!("hover-height-lower"), *offset - step),
                            ("restore", aurora_i18n::tr!("common-reset"), 0.0),
                            ("plus", aurora_i18n::tr!("hover-height-raise"), *offset + step),
                        ] {
                            let response = widgets::action_button(ui, p, icon, label, ActionKind::Flat, egui::vec2(0.0, 26.0));
                            #[cfg(test)]
                            self.auto_button_rects.push(response.rect);
                            if response.clicked()
                                && let Some(value) = hover_height::bounded(value)
                            {
                                *offset = value;
                                self.offset_editing = false;
                                preferences_changed = true;
                                if enabled && let Some(base) = automatic_base {
                                    height.preview(base + *offset);
                                    height.commit();
                                }
                            }
                        }
                    });
                });
            }
            widgets::divider(ui, p);
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
        if let Some(posture) = self.seated_editing
            && !ctx.input(|i| i.pointer.any_down())
        {
            if enabled {
                height.commit_seated(posture);
            }
            self.seated_editing = None;
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

/// A sitting posture's tab: its own height, or the standing one. Returns
/// where its slider and its three buttons are.
fn seated_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    height: &mut HoverHeight,
    enabled: bool,
    posture: Posture,
    editing: &mut Option<Posture>,
) -> [egui::Rect; 4] {
    let mut rects = [egui::Rect::NOTHING; 4];
    ui.add_enabled_ui(enabled, |ui| {
        let mut separate = height.seated(posture).separate;
        let label = if posture == Posture::OnObject {
            aurora_i18n::tr!("hover-height-separate-on-object")
        } else {
            aurora_i18n::tr!("hover-height-separate-on-ground")
        };
        if widgets::switch_labeled(ui, p, &mut separate, label)
            .on_hover_text(aurora_i18n::tr!("hover-height-separate-tip"))
            .changed()
        {
            height.set_separate(posture, separate);
        }
        ui.add_enabled_ui(separate, |ui| {
            let mut value = height.seated_value(posture);
            ui.spacing_mut().slider_width = 260.0;
            let response = widgets::height_slider(ui, p, &mut value, hover_height::MIN..=hover_height::MAX, !separate);
            rects[0] = response.rect;
            if response.changed() {
                height.preview_seated(posture, value);
                *editing = Some(posture);
            }
            ui.horizontal(|ui| {
                let step = ui.input(|i| hover_height::step(i.modifiers.alt, i.modifiers.ctrl || i.modifiers.shift));
                let value = height.seated_value(posture);
                for (i, (icon, label, value)) in [
                    ("minus", aurora_i18n::tr!("hover-height-lower"), value - step),
                    ("restore", aurora_i18n::tr!("common-reset"), 0.0),
                    ("plus", aurora_i18n::tr!("hover-height-raise"), value + step),
                ]
                .into_iter()
                .enumerate()
                {
                    let response = widgets::action_button(ui, p, icon, label, ActionKind::Flat, egui::vec2(0.0, 26.0));
                    rects[i + 1] = response.rect;
                    if response.clicked() {
                        *editing = None;
                        height.preview_seated(posture, value);
                        height.commit_seated(posture);
                    }
                }
            });
        });
        if !separate {
            ui.label(egui::RichText::new(aurora_i18n::tr!("hover-height-follows-standing")).color(p.muted));
        }
    });
    rects
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
    fn sitting_shows_its_tab_and_its_slider_saves_on_release_without_touching_standing() {
        let mut f = Fixture::new();
        f.height.set_posture(Posture::OnGround);
        for _ in 0..3 {
            f.frame(Vec::new());
        }
        assert_eq!(f.panel.tab, 2, "the tab follows the posture");
        let slider = f.panel.seated_rects[0];
        let point = egui::pos2(slider.left() + 180.0, slider.center().y);
        f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]);
        f.frame(vec![button(point, false)]);
        assert_eq!(f.height.seated(Posture::OnGround), Default::default(), "inactive until switched on");
        f.height.set_separate(Posture::OnGround, true);
        f.frame(Vec::new());
        f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]);
        let dragged = f.height.effective();
        assert_ne!(dragged, 0.2);
        assert!(f.height.next_update(Some(1)).is_none(), "previews until release");
        assert!(!f.frame(vec![button(point, false)]), "not a viewer setting");
        let Some(NetCommand::SetHoverHeight { request, value, .. }) = f.height.next_update(Some(1)) else {
            panic!("seated commit");
        };
        assert_eq!(value, dragged);
        f.height.reply(request, 1, true);
        assert_eq!(f.height.value(), 0.2);
        assert!(f.automatic, "a sitting height leaves automatic mode alone");
        let reset = f.panel.seated_rects[2].center();
        f.frame(vec![egui::Event::PointerMoved(reset), button(reset, true)]);
        f.frame(vec![button(reset, false)]);
        assert_eq!(f.height.seated_value(Posture::OnGround), 0.0);
        f.height.set_posture(Posture::Standing);
        f.frame(Vec::new());
        assert_eq!(f.panel.tab, 0);
    }

    #[test]
    fn painting_an_automatic_height_never_rounds_it_or_switches_modes() {
        let mut f = Fixture::new();
        f.height.preview(0.234567);
        f.height.commit();
        for _ in 0..4 {
            assert!(!f.frame(Vec::new()));
            assert!(f.automatic);
            assert_eq!(f.height.value(), 0.234567);
        }
    }

    #[test]
    fn subdued_manual_slider_is_interactive_and_immediately_switches_off_auto() {
        let mut f = Fixture::new();
        for _ in 0..3 {
            f.frame(Vec::new());
        }
        let rect = f.panel.manual_rect.unwrap();
        let point = egui::pos2(rect.left() + 180.0, rect.center().y);
        assert!(f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]));
        assert!(!f.automatic, "manual input must turn auto off during the drag");
        assert_ne!(f.height.value(), 0.2);
        assert_eq!(f.offset, 0.0);
        assert!(f.height.next_update(Some(1)).is_none(), "manual height previews until release");
        f.frame(vec![button(point, false)]);
        let Some(NetCommand::SetHoverHeight { value, .. }) = f.height.next_update(Some(1)) else {
            panic!("manual commit");
        };
        assert_eq!(value, f.height.value());
        assert!(!f.automatic);
    }

    #[test]
    fn every_manual_button_takes_over_including_resetting_an_already_zero_height() {
        for index in 0..3 {
            let mut f = Fixture::new();
            if index == 1 {
                f.height.preview(0.0);
                f.height.commit();
            }
            for _ in 0..3 {
                f.frame(Vec::new());
            }
            let before = f.height.value();
            let point = f.panel.button_rects[index].center();
            f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]);
            assert!(f.frame(vec![button(point, false)]));
            assert!(!f.automatic);
            let expected = match index {
                0 => before - 0.01,
                1 => 0.0,
                _ => before + 0.01,
            };
            assert!((f.height.value() - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn manual_numeric_entry_takes_over_without_losing_the_saved_auto_offset() {
        let mut f = Fixture::new();
        f.offset = -0.8;
        for _ in 0..3 {
            f.frame(Vec::new());
        }
        let rect = f.panel.manual_rect.unwrap();
        let point = egui::pos2(rect.right() - 39.0, rect.center().y);
        f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]);
        f.frame(vec![button(point, false)]);
        f.frame(Vec::new());
        f.frame(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        }]);
        assert!(f.frame(vec![egui::Event::Text("0.075".into())]));
        assert!(!f.automatic);
        assert!((f.height.value() - 0.075).abs() < 1e-5);
        assert_eq!(f.offset, -0.8);
        f.frame(vec![egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
        assert!(
            (f.height.value() - 0.076).abs() < 1e-5,
            "numeric keyboard steps remain one millimetre"
        );
    }

    #[test]
    fn automatic_buttons_adjust_the_saved_offset_and_keep_automatic_mode_active() {
        for (index, offset, expected) in [
            (0, 0.05, 0.04),
            (1, -0.8, 0.0),
            (2, -0.05, -0.04),
            (0, hover_height::MIN, hover_height::MIN),
            (2, hover_height::MAX, hover_height::MAX),
        ] {
            let mut f = Fixture::new();
            f.offset = offset;
            for _ in 0..3 {
                f.frame(Vec::new());
            }
            let point = f.panel.auto_button_rects[index].center();
            assert!(!f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]));
            assert!(f.frame(vec![button(point, false)]));
            assert!(f.automatic, "offset buttons must retain automatic mode");
            assert!((f.offset - expected).abs() < 1e-5);
            assert_eq!(f.height.value(), hover_height::bounded(0.2 + f.offset).unwrap());
            assert!(!f.panel.previewing_offset());
            if expected != 0.0 {
                let Some(NetCommand::SetHoverHeight { value, .. }) = f.height.next_update(Some(1)) else {
                    panic!("offset button must commit immediately");
                };
                assert_eq!(value, f.height.value());
            }
        }
    }

    #[test]
    fn automatic_buttons_are_inactive_when_automatic_mode_is_off() {
        let mut f = Fixture::new();
        f.automatic = false;
        f.offset = -0.8;
        for _ in 0..3 {
            f.frame(Vec::new());
        }
        for index in 0..3 {
            let point = f.panel.auto_button_rects[index].center();
            assert!(!f.frame(vec![egui::Event::PointerMoved(point), button(point, true)]));
            assert!(!f.frame(vec![button(point, false)]));
            assert!(!f.automatic);
            assert_eq!(f.offset, -0.8);
            assert_eq!(f.height.value(), 0.2);
            assert!(f.height.next_update(Some(1)).is_none());
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
