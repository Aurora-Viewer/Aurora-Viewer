//! Continuous numeric scrubbing: the native cursor stays at the field while
//! a virtual egui pointer integrates relative motion beyond screen boundaries.
//! Detect DragValue edits centrally, including the numeric part of Sliders;
//! slider tracks, window handles and text editing keep their positional input.

use egui::{Context, Event, Id, Pos2, RawInput, Role, Vec2, output::OutputEvent};

struct Drag {
    id: Id,
    /// Physical pixels: a live UI zoom must neither move the native anchor
    /// nor add movement to the virtual pointer.
    anchor: Pos2,
    pointer: Pos2,
}

#[derive(Default)]
pub(crate) struct NumericDrag {
    drag: Option<Drag>,
}

impl NumericDrag {
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }

    pub fn cancel(&mut self) {
        self.drag = None;
    }

    pub fn before(&mut self, input: &mut RawInput, pixels_per_point: f32) {
        if !input.focused || input.events.iter().any(|event| matches!(event, Event::Touch { .. })) {
            self.cancel();
        }
        let Some(drag) = &mut self.drag else { return };
        let delta = input
            .events
            .iter()
            .filter_map(|event| match event {
                Event::MouseMoved(delta) => Some(*delta),
                _ => None,
            })
            .fold(Vec2::ZERO, |sum, delta| sum + delta);
        drag.pointer += delta;
        let pointer = drag.pointer / pixels_per_point;
        // Ignore native positions, including synthetic events from cursor warps.
        // DragValue uses pointer.delta(), rather than raw drag_motion(), in egui 0.36.
        input
            .events
            .retain(|event| !matches!(event, Event::PointerMoved(_) | Event::PointerGone));
        input.events.insert(0, Event::PointerMoved(pointer));
        for event in &mut input.events {
            if let Event::PointerButton { pos, .. } = event {
                *pos = pointer;
            }
        }
    }

    /// Return the logical native cursor position to restore after drawing.
    pub fn after(&mut self, ctx: &Context, events: &[OutputEvent], enabled: bool) -> Option<Pos2> {
        if !enabled {
            self.cancel();
            return None;
        }
        if self.drag.is_none()
            && !events
                .iter()
                .any(|event| matches!(event, OutputEvent::ValueChanged(info) if info.role == Role::SpinButton))
        {
            return None;
        }
        let dragging = ctx.interaction_snapshot(|s| s.dragged);
        let (down, origin, pointer) = ctx.input(|i| {
            (
                i.pointer.primary_down() && !i.any_touches(),
                i.pointer.press_origin(),
                i.pointer.latest_pos(),
            )
        });
        if let Some(drag) = &self.drag {
            let anchor = drag.anchor / ctx.pixels_per_point();
            if !down || dragging != Some(drag.id) || ctx.viewport(|v| v.prev_pass.widgets.get(drag.id).is_none()) {
                self.cancel();
            }
            return Some(anchor);
        }
        if !down {
            return None;
        }
        let id = dragging?;
        let widget = ctx.viewport(|v| v.prev_pass.widgets.get(id).copied())?;
        let rect = ctx
            .layer_transform_to_global(widget.layer_id)
            .map_or(widget.rect, |t| t * widget.rect);
        let anchor = rect.clamp(origin?);
        self.drag = Some(Drag {
            id,
            anchor: anchor * ctx.pixels_per_point(),
            pointer: pointer? * ctx.pixels_per_point(),
        });
        Some(anchor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &Context, drag: &mut NumericDrag, value: &mut f64, events: Vec<Event>, focused: bool) -> (egui::Rect, Option<Pos2>) {
        // The input adapter and egui must agree on the physical pixel scale.
        ctx.set_pixels_per_point(2.0);
        let mut raw = RawInput {
            events,
            focused,
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 200.0))),
            ..Default::default()
        };
        drag.before(&mut raw, 2.0);
        let mut rect = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(raw, |ui| {
            rect = ui.add(egui::DragValue::new(value).speed(1.0).range(0.0..=10000.0)).rect;
        });
        // No renderer is attached to these UI-only tests.
        output.textures_delta.clear();
        let anchor = drag.after(ctx, &output.platform_output.events, focused);
        (rect, anchor)
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn relative_motion_continues_at_screen_edges_reverses_and_ignores_warps() {
        let ctx = Context::default();
        let mut drag = NumericDrag::default();
        let mut value = 100.0;
        let pos = frame(&ctx, &mut drag, &mut value, Vec::new(), true).0.center();
        frame(&ctx, &mut drag, &mut value, vec![Event::PointerMoved(pos), button(pos, true)], true);
        let (_, anchor) = frame(
            &ctx,
            &mut drag,
            &mut value,
            vec![Event::PointerMoved(pos + egui::vec2(12.0, 0.0))],
            true,
        );
        assert_eq!(anchor, Some(pos));
        let start = value;
        for _ in 0..40 {
            assert_eq!(
                frame(
                    &ctx,
                    &mut drag,
                    &mut value,
                    vec![Event::PointerMoved(pos), Event::MouseMoved(egui::vec2(20.0, 0.0))],
                    true
                )
                .1,
                Some(pos)
            );
        }
        assert_eq!(
            value,
            start + 400.0,
            "physical motion is scaled to points and never stops at the screen edge"
        );
        frame(&ctx, &mut drag, &mut value, vec![Event::MouseMoved(egui::vec2(-40.0, 0.0))], true);
        assert_eq!(value, start + 380.0, "reversing direction decreases the value immediately");
        frame(&ctx, &mut drag, &mut value, vec![Event::PointerMoved(pos)], true);
        assert_eq!(value, start + 380.0, "warping the native pointer adds no numeric motion");
        frame(&ctx, &mut drag, &mut value, vec![button(pos, false)], true);
        assert!(!drag.active());
        frame(&ctx, &mut drag, &mut value, vec![Event::PointerMoved(pos)], true);
        assert_eq!(
            value,
            start + 380.0,
            "releasing and returning to the real pointer must not change the value"
        );
    }

    #[test]
    fn click_and_focus_loss_keep_normal_text_entry_and_release_the_cursor() {
        let ctx = Context::default();
        let mut drag = NumericDrag::default();
        let mut value = 100.0;
        let pos = frame(&ctx, &mut drag, &mut value, Vec::new(), true).0.center();
        frame(&ctx, &mut drag, &mut value, vec![Event::PointerMoved(pos), button(pos, true)], true);
        frame(&ctx, &mut drag, &mut value, vec![button(pos, false)], true);
        assert!(!drag.active(), "clicking to type does not capture the pointer");
        frame(
            &ctx,
            &mut drag,
            &mut value,
            vec![
                Event::Text("250".into()),
                Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        assert_eq!(value, 250.0);
        frame(&ctx, &mut drag, &mut value, vec![Event::PointerMoved(pos), button(pos, true)], true);
        frame(
            &ctx,
            &mut drag,
            &mut value,
            vec![Event::PointerMoved(pos + egui::vec2(12.0, 0.0))],
            true,
        );
        assert!(drag.active());
        assert_eq!(frame(&ctx, &mut drag, &mut value, Vec::new(), false).1, None);
        assert!(!drag.active(), "losing focus releases the pointer immediately");
    }

    #[test]
    fn slider_numeric_field_is_continuous_but_its_track_keeps_absolute_position() {
        for numeric in [false, true] {
            let ctx = Context::default();
            let mut drag = NumericDrag::default();
            let mut value = 100.0;
            let mut frame = |drag: &mut NumericDrag, events| {
                let mut raw = RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0))),
                    ..Default::default()
                };
                drag.before(&mut raw, 1.0);
                let mut rect = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(raw, |ui| {
                    rect = ui
                        .add(egui::Slider::new(&mut value, 0.0..=10000.0).drag_value_speed(1.0).fixed_decimals(0))
                        .rect;
                });
                output.textures_delta.clear();
                let anchor = drag.after(&ctx, &output.platform_output.events, true);
                (output, rect, anchor)
            };
            let (output, rect, _) = frame(&mut drag, Vec::new());
            let pos = if numeric {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.text() == "100" => Some(text.pos + text.galley.size() * 0.5),
                        _ => None,
                    })
                    .expect("slider value visible")
            } else {
                rect.left_center() + egui::vec2(20.0, 0.0)
            };
            frame(&mut drag, vec![Event::PointerMoved(pos), button(pos, true)]);
            let (_, _, anchor) = frame(&mut drag, vec![Event::PointerMoved(pos + egui::vec2(20.0, 0.0))]);
            assert_eq!(anchor.is_some(), numeric);
            assert_eq!(drag.active(), numeric);
            let (_, _, anchor) = frame(&mut drag, vec![Event::PointerMoved(pos), Event::MouseMoved(egui::vec2(500.0, 0.0))]);
            assert_eq!(anchor.is_some(), numeric);
            frame(&mut drag, vec![button(pos, false)]);
            assert!(!drag.active());
        }
    }
}
