//! Live UI scaling without feeding the resized control back into its gesture.
//! Firestorm defers resolution updates to LLFloaterPreference::apply
//! (llfloaterpreference.cpp); Aurora keeps a physical reference while dragging.

use egui::{Event, Pos2, RawInput, Vec2};

const MIN: f32 = 0.75;
const MAX: f32 = 2.0;
const NUMERIC_SPEED: f32 = 0.005;

#[derive(Default)]
pub(super) struct ScaleSlider {
    pointer: Pointer,
    gesture: Option<Gesture>,
}

#[derive(Default)]
struct Pointer {
    physical: Option<Pos2>,
    motion: Vec2,
    down: bool,
    pressed: bool,
    focused: bool,
    modifiers: egui::Modifiers,
}

#[derive(Clone, Copy)]
enum Source {
    Track { origin: f32 },
    Numeric { origin: Vec2 },
}

struct Gesture {
    source: Source,
    start: f32,
    per_pixel: f32,
    value: f32,
    frame: u64,
}

impl ScaleSlider {
    /// Read positions before egui applies its pending zoom and before numeric
    /// scrubbing replaces them with a virtual pointer. A cached logical
    /// position multiplied by a later zoom is not a physical position.
    pub fn prepare_input(&mut self, raw: &RawInput, pixels_per_point: f32, numeric_captured: bool) {
        let previous = self.pointer.physical;
        let mut relative = Vec2::ZERO;
        let mut has_relative = false;
        self.pointer.pressed = false;
        self.pointer.focused = raw.focused;
        for event in &raw.events {
            match event {
                Event::PointerMoved(pos) => {
                    let physical = *pos * pixels_per_point;
                    // Conversion to logical points and back can differ below
                    // a hundredth pixel even for the same native position.
                    if self.pointer.physical.is_none_or(|last| last.distance_sq(physical) >= 0.0001) {
                        self.pointer.physical = Some(physical);
                    }
                }
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                    ..
                } => {
                    // winit may cache this position at an earlier zoom; prefer
                    // the last actual PointerMoved event even on release.
                    self.pointer.physical.get_or_insert(*pos * pixels_per_point);
                    self.pointer.down = *pressed;
                    self.pointer.pressed |= pressed;
                    self.pointer.modifiers = *modifiers;
                }
                Event::MouseMoved(delta) => {
                    relative += *delta;
                    has_relative = true;
                }
                Event::ModifiersChanged(modifiers) | Event::Key { modifiers, .. } => self.pointer.modifiers = *modifiers,
                _ => {}
            }
        }
        if !has_relative && !numeric_captured {
            relative = self.pointer.physical.zip(previous).map_or(Vec2::ZERO, |(now, before)| now - before);
        }
        if self.pointer.pressed {
            self.pointer.motion = Vec2::ZERO;
        }
        // Match DragValue's fine adjustment, without changing sensitivity as
        // the UI zooms. Cursor warps contribute no raw device motion.
        self.pointer.motion += relative * if self.pointer.modifiers.shift_only() { 0.1 } else { 1.0 };
        if !raw.focused {
            self.pointer.down = false;
            self.gesture = None;
            self.pointer.modifiers = egui::Modifiers::NONE;
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, saved: &mut f32) -> bool {
        let frame = ui.ctx().cumulative_frame_nr();
        let previous = *saved;
        let had_gesture = self.gesture.is_some();
        let mut gesture = self
            .gesture
            .take()
            .filter(|g| g.value == previous && frame <= g.frame + 1 && self.pointer.focused);
        let mut value = previous;
        if let Some(g) = &mut gesture {
            let travel = match g.source {
                Source::Track { origin } => self.pointer.physical.map_or(0.0, |p| p.x - origin),
                Source::Numeric { origin } => {
                    let delta = self.pointer.motion - origin;
                    delta.x - delta.y
                }
            };
            value = (g.start + travel * g.per_pixel).clamp(MIN, MAX);
            g.value = value;
            g.frame = frame;
        }
        // Keep egui's normal click, keyboard and text editing, but ignore its
        // geometry-based proposals during an established physical gesture.
        let locked = had_gesture || (self.pointer.down && !self.pointer.pressed);
        ui.spacing_mut().slider_width = super::SLIDER_W;
        let mut response = ui.add(
            egui::Slider::from_get_set(MIN as f64..=MAX as f64, |proposed| {
                if !locked
                    && self.pointer.focused
                    && let Some(proposed) = proposed
                {
                    value = proposed as f32;
                }
                value as f64
            })
            .suffix("×")
            .custom_parser(aurora_i18n::parse_number)
            .drag_value_speed(if locked { 0.0 } else { NUMERIC_SPEED as f64 })
            .clamping(egui::SliderClamping::Edits)
            .max_decimals(3)
            .update_while_editing(false)
            .trailing_fill(true),
        );
        if self.pointer.focused
            && self.pointer.pressed
            && response.is_pointer_button_down_on()
            && let Some(pointer) = self.pointer.physical
        {
            let ppp = ui.ctx().pixels_per_point();
            let numeric = pointer.x >= (response.rect.left() + super::SLIDER_W) * ppp;
            let thickness = ui.text_style_height(&egui::TextStyle::Body).max(ui.spacing().interact_size.y);
            // Same handle allowance as Slider::position_range / handle_radius.
            let aspect = match ui.visuals().handle_shape {
                egui::style::HandleShape::Circle => 1.0,
                egui::style::HandleShape::Rect { aspect_ratio } => aspect_ratio,
            };
            let span = super::SLIDER_W - 2.0 * thickness / 2.5 * aspect;
            gesture = Some(Gesture {
                source: if numeric {
                    Source::Numeric {
                        origin: self.pointer.motion,
                    }
                } else {
                    Source::Track { origin: pointer.x }
                },
                start: value,
                per_pixel: if numeric { NUMERIC_SPEED / ppp } else { (MAX - MIN) / (span * ppp) },
                value,
                frame,
            });
        }
        if value != previous {
            response.mark_changed();
            let numeric = gesture.as_ref().is_some_and(|g| matches!(g.source, Source::Numeric { .. }));
            // Let the existing native numeric scrubber keep the cursor at the
            // field. Its raw deltas feed prepare_input, not the resized field.
            if numeric {
                // The child DragValue already exposes this value to
                // accessibility. Do not change the track's widget role.
                ui.ctx().output_mut(|o| {
                    o.events.push(egui::output::OutputEvent::ValueChanged(egui::WidgetInfo::drag_value(
                        ui.is_enabled(),
                        value as f64,
                    )));
                });
            } else {
                response.output_event(egui::output::OutputEvent::ValueChanged(egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    value as f64,
                    "",
                )));
            }
        }
        response.on_hover_text(aurora_i18n::tr!("ui-options-interface-scale-help"));
        self.gesture = gesture.filter(|_| self.pointer.down && self.pointer.focused);
        *saved = value;
        value != previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::numeric_drag::NumericDrag;
    use egui::{Context, Modifiers, PointerButton, Rect};

    struct Harness {
        ctx: Context,
        control: ScaleSlider,
        numeric: NumericDrag,
        anchor: Option<Pos2>,
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
                numeric: NumericDrag::default(),
                anchor: None,
                saved,
                native_scale,
                time: 0.0,
                rect: Rect::NOTHING,
            };
            h.frame(vec![], true, true);
            h.frame(vec![], true, true);
            h
        }

        /// Match App::frame: winit positions use the previous zoom, prepare
        /// reads physical input, numeric scrubbing rewrites the logical input,
        /// egui applies its pending zoom, then changed settings request a zoom.
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
            self.control.prepare_input(&input, ppp, self.numeric.active());
            self.numeric.before(&mut input, ppp);
            let mut changed = false;
            let mut rect = Rect::NOTHING;
            let mut output = self.ctx.run_ui(input, |ui| {
                if visible {
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
            self.anchor = self
                .numeric
                .after(&self.ctx, &output.platform_output.events, focused)
                .map(|p| p * self.ctx.pixels_per_point());
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
            self.rect.left_center() + egui::vec2((2.0 + fraction * (super::super::SLIDER_W - 4.0)) * self.ctx.pixels_per_point(), 0.0)
        }

        fn press(&mut self, pos: Pos2) -> bool {
            self.frame(vec![Event::PointerMoved(pos), Self::button(pos, true)], true, true)
        }

        fn settled(&mut self) {
            self.frame(vec![], true, true);
            self.frame(vec![], true, true);
        }
    }

    #[test]
    fn scale_track_is_live_gradual_and_stationary_without_events_at_each_dpi() {
        for native in [1.0, 1.5, 2.0] {
            for initial in [0.75, 1.0, 1.6, 2.0] {
                let mut h = Harness::new(initial, native);
                for direction in [1.0, -1.0] {
                    let start = h.track((h.saved - MIN) / (MAX - MIN));
                    let distance = super::super::SLIDER_W * h.ctx.pixels_per_point() * direction;
                    h.press(start);
                    assert!(matches!(
                        h.control.gesture.as_ref().expect("track gesture").source,
                        Source::Track { .. }
                    ));
                    assert!(!h.numeric.active(), "the track never captures the native numeric pointer");
                    let mut previous = h.saved;
                    for step in 1..=40 {
                        let pos = start + egui::vec2(distance * step as f32 / 40.0, 0.0);
                        h.frame(vec![Event::PointerMoved(pos)], true, true);
                        let value = h.saved;
                        assert!((value - previous).abs() <= 0.05, "small physical moves stay gradual");
                        assert!(
                            (value - previous) * direction >= -0.00001,
                            "the value follows the physical direction"
                        );
                        if step == 1 && ((direction > 0.0 && previous < MAX) || (direction < 0.0 && previous > MIN)) {
                            assert_ne!(value, previous, "the value changes before release");
                        }
                        // In particular, zooming with NO native mouse event
                        // must not multiply a cached logical position.
                        for _ in 0..4 {
                            assert!(!h.frame(vec![], true, true));
                            assert_eq!(h.saved, value, "a stationary pointer cannot drift during live zoom");
                        }
                        assert!(!h.frame(vec![Event::PointerMoved(pos)], true, true));
                        assert_eq!(h.saved, value, "repeated native positions cannot add movement");
                        assert!(
                            (h.ctx.zoom_factor() - value).abs() < 0.00001,
                            "zoom applies while the button is held"
                        );
                        previous = value;
                    }
                    let target = if direction > 0.0 { MAX } else { MIN };
                    assert_eq!(h.saved, target);
                    // winit's release position may still be cached at a
                    // different zoom: it must not edit the scale.
                    assert!(!h.frame(vec![Harness::button(Pos2::ZERO, false)], true, true));
                    h.settled();
                    assert_eq!(h.saved, target);
                    assert!(h.control.gesture.is_none());
                }
            }
        }
    }

    #[test]
    fn scale_numeric_scrubbing_is_live_and_keeps_its_physical_cursor_anchor() {
        for native in [1.0, 1.5, 2.0] {
            let mut h = Harness::new(1.0, native);
            let start = h.rect.right_center() - egui::vec2(15.0, 0.0);
            h.press(start);
            assert!(matches!(
                h.control.gesture.as_ref().expect("numeric gesture").source,
                Source::Numeric { .. }
            ));
            assert!(h.frame(
                vec![
                    Event::PointerMoved(start + egui::vec2(24.0, 0.0)),
                    Event::MouseMoved(egui::vec2(24.0, 0.0))
                ],
                true,
                true
            ));
            assert!(
                h.numeric.active(),
                "the existing native scrubber captures the numeric field at DPI {native}"
            );
            assert!(h.saved > 1.0);
            for _ in 0..12 {
                let previous = h.saved;
                assert!(h.frame(
                    vec![Event::PointerMoved(start), Event::MouseMoved(egui::vec2(5.0, 0.0))],
                    true,
                    true
                ));
                assert!(
                    (h.saved - previous - 0.025 / native).abs() < 0.00001,
                    "sensitivity stays fixed as zoom changes"
                );
                assert!(h.anchor.expect("native anchor").distance(start) < 0.01);
                let value = h.saved;
                for _ in 0..3 {
                    assert!(!h.frame(vec![Event::PointerMoved(start)], true, true));
                    assert_eq!(h.saved, value, "a native cursor warp adds no numeric motion");
                }
            }
            let previous = h.saved;
            assert!(h.frame(
                vec![Event::PointerMoved(start), Event::MouseMoved(egui::vec2(-10.0, 0.0))],
                true,
                true
            ));
            assert!(h.saved < previous, "reversing direction responds immediately");
            let value = h.saved;
            assert!(!h.frame(vec![Harness::button(start, false)], true, true));
            assert_eq!(h.saved, value);
            assert!(!h.numeric.active());
            h.settled();
            assert_eq!(h.saved, value, "formatting the idle control cannot round the saved scale");
        }
    }

    #[test]
    fn scale_text_entry_commits_on_enter_and_keyboard_arrows_stay_live() {
        assert_eq!(
            Harness::new(1.234567, 1.0).saved,
            1.234567,
            "opening preferences cannot edit the saved scale"
        );
        let mut h = Harness::new(1.0, 1.5);
        let numeric = h.rect.right_center() - egui::vec2(15.0, 0.0);
        h.press(numeric);
        h.frame(vec![Harness::button(numeric, false)], true, true);
        assert!(!h.frame(vec![Event::Text("1.25".into())], true, true));
        assert_eq!(h.saved, 1.0, "typing does not resize its own editor");
        assert!(h.frame(vec![Harness::key(egui::Key::Enter)], true, true));
        assert_eq!(h.saved, 1.25);
        h.settled();

        let mut h = Harness::new(1.0, 1.0);
        assert!(h.press(h.track(0.6)), "clicking the track applies immediately");
        assert!((h.saved - 1.5).abs() < 0.03);
        let value = h.saved;
        h.frame(vec![Harness::button(Pos2::ZERO, false)], true, true);
        h.settled();
        h.frame(vec![Harness::key(egui::Key::Tab)], true, true);
        assert!(h.frame(vec![Harness::key(egui::Key::ArrowRight)], true, true));
        assert!(h.saved > value && h.saved - value < 0.03);
    }

    #[test]
    fn scale_interrupted_gestures_do_not_resume_or_overwrite_external_resets() {
        for interrupted in ["focus", "hidden", "reset"] {
            let mut h = Harness::new(1.0, 1.0);
            h.press(h.track(0.8));
            let value = h.saved;
            assert!(value > 1.0, "the initial click is already live");
            match interrupted {
                "focus" => {
                    h.frame(vec![], false, true);
                }
                "hidden" => {
                    h.frame(vec![], true, false);
                }
                "reset" => h.saved = MIN,
                _ => unreachable!(),
            }
            h.frame(vec![Harness::button(Pos2::ZERO, false)], true, interrupted == "reset");
            h.settled();
            assert_eq!(h.saved, if interrupted == "reset" { MIN } else { value });
            assert!(h.control.gesture.is_none());
        }
    }
}
