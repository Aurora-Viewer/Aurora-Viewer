//! Live interface and text scaling without feeding the resized control back into its gesture.
//! The Legacy Viewer defers resolution updates to LLFloaterPreference::apply
//! (llfloaterpreference.cpp); Aurora keeps a physical reference while dragging.

use egui::{Event, Pos2, RawInput, Vec2};

const MIN: f32 = 0.75;
const MAX: f32 = 2.0;
const NUMERIC_SPEED: f32 = 0.005;

/// Consume egui's zoom shortcuts after the text fields, before its end-pass
/// handler can change only the context. Like UIScaleFactor in
/// LLViewerWindow::calcDisplayScale (llviewerwindow.cpp), every UI scale
/// change must use the saved preference, including its limits.
pub(crate) fn keyboard(ctx: &egui::Context, scale: &mut f32) -> bool {
    use egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};

    let before = *scale;
    ctx.input_mut(|input| {
        if input.consume_shortcut(&ZOOM_RESET) {
            *scale = 1.0;
        } else {
            if input.consume_shortcut(&ZOOM_IN) || input.consume_shortcut(&ZOOM_IN_SECONDARY) {
                *scale = ((*scale + 0.1) * 10.0).round() / 10.0;
            }
            if input.consume_shortcut(&ZOOM_OUT) {
                *scale = ((*scale - 0.1) * 10.0).round() / 10.0;
            }
        }
    });
    *scale = scale.clamp(MIN, MAX);
    *scale != before
}

// Keep the established limits while placing the default at the track's center.
// Numeric editing always uses percentages, independently of this track mapping.
fn track_position(scale: f32) -> f32 {
    if scale <= 1.0 {
        0.5 * (scale - MIN) / (1.0 - MIN)
    } else {
        0.5 + 0.5 * (scale - 1.0) / (MAX - 1.0)
    }
}

fn track_scale(position: f32) -> f32 {
    let position = position.clamp(0.0, 1.0);
    if position <= 0.5 {
        MIN + position * 2.0 * (1.0 - MIN)
    } else {
        1.0 + (position - 0.5) * 2.0 * (MAX - 1.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Interface,
    Text,
}

impl Kind {
    fn limits(self) -> (f32, f32) {
        match self {
            Self::Interface => (MIN, MAX),
            Self::Text => (crate::settings::FONT_SCALE_MIN, crate::settings::FONT_SCALE_MAX),
        }
    }

    fn position(self, scale: f32) -> f32 {
        match self {
            Self::Interface => track_position(scale),
            Self::Text => {
                let (min, max) = self.limits();
                (scale - min) / (max - min)
            }
        }
    }

    fn scale(self, position: f32) -> f32 {
        match self {
            Self::Interface => track_scale(position),
            Self::Text => {
                let (min, max) = self.limits();
                min + position.clamp(0.0, 1.0) * (max - min)
            }
        }
    }
}

#[derive(Default)]
pub(super) struct ScaleSlider {
    pointer: Pointer,
    gesture: Option<Gesture>,
    track_id: Option<egui::Id>,
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
        self.show_kind(ui, saved, Kind::Interface)
    }

    pub fn show_text(&mut self, ui: &mut egui::Ui, saved: &mut f32) -> bool {
        self.show_kind(ui, saved, Kind::Text)
    }

    fn show_kind(&mut self, ui: &mut egui::Ui, saved: &mut f32, kind: Kind) -> bool {
        let (min, max) = kind.limits();
        let (factor, suffix, decimals, label, hint) = match kind {
            Kind::Interface => (
                100.0,
                "%",
                0..=1,
                aurora_i18n::tr!("ui-options-interface-scale"),
                aurora_i18n::tr!("ui-options-interface-scale-help"),
            ),
            Kind::Text => (1.0, "×", 2..=2, aurora_i18n::tr!("ui-options-text-size"), ""),
        };
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
            value = match g.source {
                Source::Track { .. } => kind.scale(kind.position(g.start) + travel * g.per_pixel),
                Source::Numeric { .. } => (g.start + travel * g.per_pixel).clamp(min, max),
            };
            g.value = value;
            g.frame = frame;
        }
        // Keep egui's normal click, keyboard and text editing, but ignore its
        // geometry-based proposals during an established physical gesture.
        let locked = had_gesture || (self.pointer.down && !self.pointer.pressed);
        // The accessible track exposes the displayed unit too. Consume SetValue here
        // before the normalized Slider can interpret it as a track position.
        if let Some(id) = self.track_id {
            ui.input_mut(|input| {
                input.events.retain(|event| {
                    if let Event::AccessKitActionRequest(request) = event
                        && request.target_tree == egui::accesskit::TreeId::ROOT
                        && request.target_node == id.accesskit_id()
                        && request.action == egui::accesskit::Action::SetValue
                        && let Some(egui::accesskit::ActionData::NumericValue(displayed)) = request.data
                    {
                        if !locked && self.pointer.focused && displayed.is_finite() {
                            value = (displayed / factor).clamp(min as f64, max as f64) as f32;
                        }
                        false
                    } else {
                        true
                    }
                });
            });
        }
        ui.spacing_mut().slider_width = super::SLIDER_W;
        let (mut track, mut number) = ui
            .horizontal(|ui| {
                let track = ui.add(
                    egui::Slider::from_get_set(0.0..=1.0, |proposed| {
                        if !locked
                            && self.pointer.focused
                            && let Some(proposed) = proposed
                        {
                            value = kind.scale(proposed as f32);
                        }
                        kind.position(value) as f64
                    })
                    .show_value(false)
                    .clamping(egui::SliderClamping::Edits)
                    .trailing_fill(true),
                );
                let number = ui.add(
                    egui::DragValue::from_get_set(|proposed| {
                        if !locked
                            && self.pointer.focused
                            && let Some(proposed) = proposed
                        {
                            value = (proposed / factor) as f32;
                        }
                        value as f64 * factor
                    })
                    .suffix(suffix)
                    .custom_formatter(|displayed, _| {
                        aurora_i18n::number_text(&egui::emath::format_with_decimals_in_range(displayed, decimals.clone()))
                    })
                    .custom_parser(|text| aurora_i18n::parse_number(text.trim().trim_end_matches(suffix).trim()))
                    .speed(if locked { 0.0 } else { NUMERIC_SPEED as f64 * factor })
                    .range(min as f64 * factor..=max as f64 * factor)
                    .clamp_existing_to_range(false)
                    .min_decimals(*decimals.start())
                    .max_decimals(*decimals.end())
                    .update_while_editing(false),
                );
                (track, number)
            })
            .inner;
        self.track_id = Some(track.id);
        let numeric = number.is_pointer_button_down_on();
        if self.pointer.focused
            && self.pointer.pressed
            && (track.is_pointer_button_down_on() || numeric)
            && let Some(pointer) = self.pointer.physical
        {
            let ppp = ui.ctx().pixels_per_point();
            // Same handle allowance as Slider::position_range / handle_radius.
            let aspect = match ui.visuals().handle_shape {
                egui::style::HandleShape::Circle => 1.0,
                egui::style::HandleShape::Rect { aspect_ratio } => aspect_ratio,
            };
            let span = track.rect.width() - 2.0 * track.rect.height() / 2.5 * aspect;
            gesture = Some(Gesture {
                source: if numeric {
                    Source::Numeric {
                        origin: self.pointer.motion,
                    }
                } else {
                    Source::Track { origin: pointer.x }
                },
                start: value,
                per_pixel: if numeric { NUMERIC_SPEED / ppp } else { 1.0 / (span * ppp) },
                value,
                frame,
            });
        }
        if value != previous {
            let numeric = gesture.as_ref().is_some_and(|g| matches!(g.source, Source::Numeric { .. }));
            // Let the existing native numeric scrubber keep the cursor at the
            // field. Its raw deltas feed prepare_input, not the resized field.
            if numeric {
                number.mark_changed();
                number.output_event(egui::output::OutputEvent::ValueChanged(egui::WidgetInfo::drag_value(
                    ui.is_enabled(),
                    value as f64 * factor,
                )));
            } else {
                track.mark_changed();
            }
        }
        track.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), value as f64 * factor, label));
        ui.ctx().accesskit_node_builder(track.id, |builder| {
            builder.set_min_numeric_value(min as f64 * factor);
            builder.set_max_numeric_value(max as f64 * factor);
        });
        track.union(number).on_hover_text(hint);
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

    fn keyboard_frame(ctx: &Context, saved: &mut f32, key: egui::Key, modifiers: Modifiers) -> bool {
        let events = [true, false]
            .into_iter()
            .map(|pressed| Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .collect();
        let mut changed = false;
        let mut output = ctx.run_ui(
            RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                changed |= keyboard(ui.ctx(), saved);
            },
        );
        output.textures_delta.clear();
        if changed {
            ctx.set_zoom_factor(*saved);
        }
        let mut output = ctx.run_ui(RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert_eq!(ctx.zoom_factor(), *saved);
        changed
    }

    #[test]
    fn keyboard_scale_uses_the_saved_preference_and_its_limits() {
        let ctx = Context::default();
        let mut saved = 1.0;
        for (key, expected) in [
            (egui::Key::Plus, 1.1),
            (egui::Key::Equals, 1.2),
            (egui::Key::Minus, 1.1),
            (egui::Key::Num0, 1.0),
        ] {
            assert!(keyboard_frame(&ctx, &mut saved, key, Modifiers::COMMAND));
            assert_eq!(saved, expected);
        }
        for (key, limit) in [(egui::Key::Plus, MAX), (egui::Key::Minus, MIN)] {
            for _ in 0..30 {
                keyboard_frame(&ctx, &mut saved, key, Modifiers::COMMAND);
            }
            assert_eq!(saved, limit);
            assert!(!keyboard_frame(&ctx, &mut saved, key, Modifiers::COMMAND));
        }
        assert!(keyboard_frame(&ctx, &mut saved, egui::Key::Num0, Modifiers::COMMAND));
        assert!(!keyboard_frame(&ctx, &mut saved, egui::Key::Plus, Modifiers::NONE));
        assert_eq!(saved, 1.0);
    }

    #[test]
    fn keyboard_scale_leaves_consumed_shortcuts_to_the_field() {
        let ctx = Context::default();
        let mut saved = 1.0;
        let event = Event::Key {
            key: egui::Key::Plus,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        let mut output = ctx.run_ui(
            RawInput {
                events: vec![event],
                ..Default::default()
            },
            |ui| {
                ui.input_mut(|input| input.consume_shortcut(&egui::gui_zoom::kb_shortcuts::ZOOM_IN));
                assert!(!keyboard(ui.ctx(), &mut saved));
            },
        );
        output.textures_delta.clear();
        let mut output = ctx.run_ui(RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert_eq!(saved, 1.0);
        assert_eq!(ctx.zoom_factor(), 1.0);
    }

    struct Harness {
        ctx: Context,
        control: ScaleSlider,
        numeric: NumericDrag,
        anchor: Option<Pos2>,
        saved: f32,
        kind: Kind,
        native_scale: f32,
        time: f64,
        rect: Rect,
        text: Vec<String>,
    }

    impl Harness {
        fn new(saved: f32, native_scale: f32) -> Self {
            Self::new_kind(saved, native_scale, Kind::Interface)
        }

        fn new_kind(saved: f32, native_scale: f32, kind: Kind) -> Self {
            let ctx = Context::default();
            match kind {
                Kind::Interface => ctx.set_zoom_factor(saved),
                Kind::Text => {
                    crate::ui::fonts::apply(&ctx, crate::ui::fonts::INTER);
                    crate::theme::Theme::default().apply(&ctx, saved);
                }
            }
            let mut h = Self {
                ctx,
                control: ScaleSlider::default(),
                numeric: NumericDrag::default(),
                anchor: None,
                saved,
                kind,
                native_scale,
                time: 0.0,
                rect: Rect::NOTHING,
                text: Vec::new(),
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
                    // Real font metrics move the row while text is resized.
                    if self.kind == Kind::Text {
                        ui.label(aurora_i18n::tr!("ui-options-the-violet-wolf-howls-under-the-northern-lights"));
                    }
                    rect = ui
                        .horizontal(|ui| {
                            ui.add_space(420.0);
                            if self.kind == Kind::Text {
                                ui.label(aurora_i18n::tr!("ui-options-text-size"));
                            }
                            ui.horizontal(|ui| changed = self.control.show_kind(ui, &mut self.saved, self.kind))
                                .response
                                .rect
                        })
                        .inner;
                }
            });
            self.rect = rect * output.pixels_per_point;
            fn collect_text(shape: &egui::Shape, text: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(t) => text.push(t.galley.text().to_owned()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_text(s, text)),
                    _ => {}
                }
            }
            self.text.clear();
            for shape in &output.shapes {
                collect_text(&shape.shape, &mut self.text);
            }
            self.anchor = self
                .numeric
                .after(&self.ctx, &output.platform_output.events, focused)
                .map(|p| p * self.ctx.pixels_per_point());
            output.textures_delta.clear();
            if changed {
                match self.kind {
                    Kind::Interface => self.ctx.set_zoom_factor(self.saved),
                    Kind::Text => crate::theme::Theme::default().apply(&self.ctx, self.saved),
                }
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
                    let start = h.track(track_position(h.saved));
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
                        assert!((value - previous).abs() <= 0.065, "small physical moves stay gradual");
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
        assert!(!h.frame(vec![Event::Text("125%".into())], true, true));
        assert_eq!(h.saved, 1.0, "typing does not resize its own editor");
        assert!(h.frame(vec![Harness::key(egui::Key::Enter)], true, true));
        assert_eq!(h.saved, 1.25);
        h.settled();

        let mut h = Harness::new(1.0, 1.0);
        assert!(h.press(h.track(0.6)), "clicking the track applies immediately");
        assert!((h.saved - 1.2).abs() < 0.03);
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

    #[test]
    fn percentage_track_places_the_default_at_center_without_changing_the_limits() {
        assert_eq!(track_scale(0.0), MIN);
        assert_eq!(track_scale(0.5), 1.0);
        assert_eq!(track_scale(1.0), MAX);
        assert_eq!(track_position(1.0), 0.5);
        for scale in [MIN, 0.9, 1.0, 1.222, MAX] {
            assert!((track_scale(track_position(scale)) - scale).abs() < 0.000001);
        }
        for native in [1.0, 1.5, 2.0] {
            let mut h = Harness::new(1.0, native);
            assert!(h.text.concat() == "100%", "the default displays as a percentage: {:?}", h.text);
            h.press(h.track(0.5));
            assert_eq!(h.saved, 1.0, "the midpoint click keeps the default");
            h.frame(vec![Harness::button(Pos2::ZERO, false)], true, true);
            for (fraction, expected) in [(0.0, MIN), (1.0, MAX), (0.5, 1.0)] {
                h.settled();
                h.press(h.track(fraction));
                assert_eq!(h.saved, expected);
                h.frame(vec![Harness::button(Pos2::ZERO, false)], true, true);
            }
        }
        let h = Harness::new(1.222, 1.0);
        let text = h.text.concat();
        assert!(text.starts_with("122") && text.ends_with('%'));
        assert!(!h.text.iter().any(|text| text.contains('×')));
    }

    #[test]
    fn accessible_scale_edits_use_percentages_instead_of_track_positions() {
        let mut h = Harness::new(1.0, 1.0);
        let id = h.control.track_id.expect("rendered track");
        for percent in [75.0, 125.5, 200.0, 100.0] {
            assert!(h.frame(
                vec![Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::SetValue,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: id.accesskit_id(),
                    data: Some(egui::accesskit::ActionData::NumericValue(percent)),
                })],
                true,
                true,
            ));
            assert!((h.saved as f64 - percent / 100.0).abs() < 0.000001);
            h.settled();
        }
    }

    #[test]
    fn text_scale_track_stays_gradual_and_stationary_while_fonts_reflow() {
        let (min, max) = Kind::Text.limits();
        for native in [1.0, 1.5, 2.0] {
            for initial in [min, 1.0, max] {
                let mut h = Harness::new_kind(initial, native, Kind::Text);
                for direction in [1.0, -1.0] {
                    let start = h.track(Kind::Text.position(h.saved));
                    h.press(start);
                    let mut previous = h.saved;
                    for step in 1..=40 {
                        let pos = start + egui::vec2(4.0 * native * direction * step as f32, 0.0);
                        h.frame(vec![Event::PointerMoved(pos)], true, true);
                        let value = h.saved;
                        // 4 native pixels move the value by a share of its range
                        assert!((value - previous).abs() < 0.05 * (max - min), "font reflow cannot cause a jump");
                        assert!((value - previous) * direction >= -0.00001);
                        for _ in 0..4 {
                            assert!(!h.frame(Vec::new(), true, true));
                            assert_eq!(h.saved, value, "a stationary pointer cannot follow the resized text");
                        }
                        assert!(!h.frame(vec![Event::PointerMoved(pos)], true, true));
                        assert_eq!(h.saved, value);
                        previous = value;
                    }
                    let target = if direction > 0.0 { max } else { min };
                    assert_eq!(h.saved, target);
                    h.frame(vec![Harness::button(Pos2::ZERO, false)], true, true);
                    h.settled();
                    assert_eq!(h.saved, target);
                    assert_eq!(h.ctx.zoom_factor(), 1.0, "text sizing leaves the UI zoom independent");
                }
            }
        }
    }

    #[test]
    fn text_scale_scrubbing_keeps_its_anchor_sensitivity_and_reverses_during_reflow() {
        for native in [1.0, 1.5, 2.0] {
            let mut h = Harness::new_kind(1.0, native, Kind::Text);
            let start = h.rect.right_center() - egui::vec2(15.0, 0.0);
            h.press(start);
            for (step, delta) in [24.0, 5.0, 5.0, -10.0].into_iter().enumerate() {
                let previous = h.saved;
                let position = if step == 0 { start + egui::vec2(delta, 0.0) } else { start };
                assert!(h.frame(
                    vec![Event::PointerMoved(position), Event::MouseMoved(egui::vec2(delta, 0.0))],
                    true,
                    true,
                ));
                assert!((h.saved - previous - delta * NUMERIC_SPEED / native).abs() < 0.00001);
                assert!(h.numeric.active(), "numeric drag is captured at DPI {native}, step {step}");
                assert!(h.anchor.expect("physical cursor anchor").distance(start) < 0.01);
                let value = h.saved;
                for _ in 0..4 {
                    assert!(!h.frame(vec![Event::PointerMoved(start)], true, true));
                    assert_eq!(h.saved, value, "font changes and cursor warps add no scrub motion");
                }
            }
            let value = h.saved;
            assert!(!h.frame(vec![Harness::button(start, false)], true, true));
            h.settled();
            assert_eq!(h.saved, value);
            assert!(!h.numeric.active());
        }
    }

    #[test]
    fn text_scale_typing_and_accessibility_keep_multiplier_units_and_limits() {
        let mut h = Harness::new_kind(1.234567, 1.5, Kind::Text);
        assert_eq!(h.saved, 1.234567, "rendering cannot round the saved text size");
        assert!(h.text.iter().any(|text| text.ends_with('×')));
        let number = h.rect.right_center() - egui::vec2(15.0, 0.0);
        h.press(number);
        h.frame(vec![Harness::button(number, false)], true, true);
        let input = format!("{}×", aurora_i18n::number_text("1.25"));
        assert!(!h.frame(vec![Event::Text(input)], true, true));
        assert_eq!(h.saved, 1.234567, "typing must not resize its own editor");
        assert!(h.frame(vec![Harness::key(egui::Key::Enter)], true, true));
        assert_eq!(h.saved, 1.25);
        h.settled();
        let id = h.control.track_id.expect("rendered text-size track");
        for (displayed, expected) in [(0.5, 0.8), (1.3, 1.3), (1.8, 1.8), (2.5, 2.0)] {
            assert!(h.frame(
                vec![Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::SetValue,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: id.accesskit_id(),
                    data: Some(egui::accesskit::ActionData::NumericValue(displayed)),
                })],
                true,
                true,
            ));
            assert_eq!(h.saved, expected, "accessible values use multipliers, not percentages");
            h.settled();
        }
    }
}
