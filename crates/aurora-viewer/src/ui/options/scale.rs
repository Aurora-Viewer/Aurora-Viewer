//! Keep the scale control still until the pointer is released. Firestorm's
//! LLFloaterPreference::apply (llfloaterpreference.cpp) likewise delays the
//! resolution update; Aurora commits on release instead of needing Apply.

#[derive(Default)]
pub(super) struct ScaleSlider {
    draft: Option<Draft>,
}

struct Draft {
    saved: f32,
    value: f32,
    frame: u64,
}

impl ScaleSlider {
    pub fn show(&mut self, ui: &mut egui::Ui, saved: &mut f32) -> bool {
        let frame = ui.ctx().cumulative_frame_nr();
        // A reset or an interrupted gesture (closed window / another page)
        // must not restore an old draft when this control next appears.
        let mut value = self
            .draft
            .take()
            .filter(|d| d.saved == *saved && frame <= d.frame + 1)
            .map_or(*saved, |d| d.value);
        ui.spacing_mut().slider_width = super::SLIDER_W;
        ui.add(
            egui::Slider::new(&mut value, 0.75..=2.0)
                .suffix("×")
                .custom_parser(aurora_i18n::parse_number)
                .update_while_editing(false)
                .trailing_fill(true),
        )
        .on_hover_text(aurora_i18n::tr!("ui-options-interface-scale-help"));
        let (down, focused) = ui.input(|i| (i.pointer.any_down(), i.focused));
        if !focused {
            return false;
        }
        if down {
            // Changing egui's zoom here would move and resize this very
            // slider under the pointer, feeding back into the next value.
            self.draft = Some(Draft {
                saved: *saved,
                value,
                frame,
            });
            return false;
        }
        if value == *saved {
            return false;
        }
        *saved = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    struct Harness {
        ctx: Context,
        control: ScaleSlider,
        saved: f32,
        native_scale: f32,
        time: f64,
        rect: Rect,
    }

    impl Harness {
        fn new(saved: f32, native_scale: f32) -> Self {
            let ctx = Context::default();
            ctx.set_zoom_factor(saved);
            let mut h = Self {
                ctx,
                control: ScaleSlider::default(),
                saved,
                native_scale,
                time: 0.0,
                rect: Rect::NOTHING,
            };
            h.frame(vec![], true, true);
            h.frame(vec![], true, true);
            h
        }

        /// Feed physical pointer positions, like winit, and apply changed
        /// settings to egui after the frame, like App::frame does.
        fn frame(&mut self, mut events: Vec<Event>, focused: bool, visible: bool) -> bool {
            let ppp = self.ctx.pixels_per_point();
            for event in &mut events {
                match event {
                    Event::PointerMoved(pos) | Event::PointerButton { pos, .. } => *pos = *pos / ppp,
                    _ => {}
                }
            }
            self.time += 0.05;
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(3440.0, 1369.0) / ppp)),
                events,
                focused,
                time: Some(self.time),
                ..Default::default()
            };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(self.native_scale);
            let mut changed = false;
            let mut rect = Rect::NOTHING;
            let mut output = self.ctx.run_ui(input, |ui| {
                if visible {
                    // The scale row sits away from the window's origin,
                    // making the old feedback readily reproducible.
                    ui.add_space(180.0);
                    rect = ui
                        .horizontal(|ui| {
                            ui.add_space(420.0);
                            ui.horizontal(|ui| changed = self.control.show(ui, &mut self.saved)).response.rect
                        })
                        .inner;
                }
            });
            self.rect = rect * output.pixels_per_point;
            output.textures_delta.clear();
            if changed {
                self.ctx.set_zoom_factor(self.saved);
            }
            changed
        }

        fn button(pos: Pos2, pressed: bool) -> Event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }
        }

        fn key(key: egui::Key) -> Event {
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }
        }

        fn track(&self, fraction: f32) -> Pos2 {
            let ppp = self.ctx.pixels_per_point();
            self.rect.left_center() + egui::vec2((2.0 + fraction * (super::super::SLIDER_W - 4.0)) * ppp, 0.0)
        }

        fn press(&mut self, pos: Pos2) {
            assert!(!self.frame(vec![Event::PointerMoved(pos), Self::button(pos, true)], true, true));
        }
    }

    #[test]
    fn scale_track_stays_still_and_commits_once_on_release_at_each_dpi() {
        for native in [1.0, 1.5, 2.0] {
            for initial in [0.75, 1.0, 1.6, 2.0] {
                let mut h = Harness::new(initial, native);
                for target in [2.0, 0.75] {
                    let saved = h.saved;
                    let geometry = h.rect;
                    let start = h.track((saved - 0.75) / 1.25);
                    let end = h.track(if target == 2.0 { 1.0 } else { 0.0 });
                    h.press(start);
                    let mut previous = h.control.draft.as_ref().expect("held control").value;
                    for step in 1..=30 {
                        let pos = start.lerp(end, step as f32 / 30.0);
                        assert!(!h.frame(vec![Event::PointerMoved(pos)], true, true));
                        assert_eq!(h.saved, saved, "dragging must not resize the UI");
                        assert_eq!(h.rect.min, geometry.min, "the track stays under the pointer");
                        assert_eq!(h.rect.height(), geometry.height(), "the control cannot resize the UI");
                        let value = h.control.draft.as_ref().expect("draft during drag").value;
                        assert!((value - previous).abs() <= 0.08, "small movements must stay gradual");
                        for _ in 0..3 {
                            assert!(!h.frame(vec![Event::PointerMoved(pos)], true, true));
                            assert_eq!(
                                h.control.draft.as_ref().expect("draft").value,
                                value,
                                "a still pointer cannot drift"
                            );
                        }
                        previous = value;
                    }
                    assert_eq!(h.frame(vec![Harness::button(end, false)], true, true), saved != target);
                    assert_eq!(h.saved, target);
                    for _ in 0..5 {
                        assert!(!h.frame(vec![Event::PointerMoved(end)], true, true));
                        assert_eq!(h.saved, target, "rescaling after release cannot edit the value again");
                    }
                }
            }
        }
    }

    #[test]
    fn scale_numeric_drag_and_text_entry_wait_until_the_gesture_is_finished() {
        let mut h = Harness::new(1.0, 1.5);
        let numeric = h.rect.right_center() - egui::vec2(15.0, 0.0);
        h.press(numeric);
        let end = numeric + egui::vec2(35.0, 0.0);
        assert!(!h.frame(vec![Event::PointerMoved(end)], true, true));
        assert_eq!(h.saved, 1.0);
        let draft = h.control.draft.as_ref().expect("numeric draft").value;
        assert!(draft > 1.0 && draft < 2.0);
        assert!(h.frame(vec![Harness::button(end, false)], true, true));
        assert_eq!(h.saved, draft);
        h.frame(vec![], true, true);
        h.frame(vec![], true, true);
        let numeric = h.rect.right_center() - egui::vec2(15.0, 0.0);
        h.press(numeric);
        h.frame(vec![Harness::button(numeric, false)], true, true);
        let saved = h.saved;
        assert!(!h.frame(vec![Event::Text("1.25".into())], true, true));
        assert_eq!(h.saved, saved, "typing cannot resize its own text field");
        assert!(h.frame(vec![Harness::key(egui::Key::Enter)], true, true));
        assert_eq!(h.saved, 1.25);
    }

    #[test]
    fn scale_keyboard_arrows_and_track_clicks_apply_without_an_extra_click() {
        let mut h = Harness::new(1.0, 1.0);
        let pos = h.track(0.6);
        h.press(pos);
        assert!(h.frame(vec![Harness::button(pos, false)], true, true));
        let saved = h.saved;
        assert!((saved - 1.5).abs() < 0.03);
        h.frame(vec![], true, true);
        h.frame(vec![Harness::key(egui::Key::Tab)], true, true);
        assert!(h.frame(vec![Harness::key(egui::Key::ArrowRight)], true, true));
        assert!(h.saved > saved && h.saved - saved < 0.03);
    }

    #[test]
    fn scale_aborted_gestures_and_external_resets_discard_the_draft() {
        for interrupted in ["focus", "hidden", "reset"] {
            let mut h = Harness::new(1.0, 1.0);
            let pos = h.track(0.8);
            h.press(pos);
            assert!(h.control.draft.as_ref().expect("draft").value > 1.0);
            match interrupted {
                "focus" => {
                    h.frame(vec![], false, true);
                }
                "hidden" => {
                    h.frame(vec![], true, false);
                }
                "reset" => h.saved = 0.75,
                _ => unreachable!(),
            }
            // Release out of the row while it is hidden, then reopen it.
            h.frame(vec![Harness::button(Pos2::ZERO, false), Event::PointerGone], true, false);
            h.frame(vec![], true, true);
            assert_eq!(h.saved, if interrupted == "reset" { 0.75 } else { 1.0 });
            assert!(h.control.draft.is_none());
        }
    }
}
