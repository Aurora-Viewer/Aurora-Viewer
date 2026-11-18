use super::*;
use egui::{FullOutput, Id, LayerId, Modifiers, Order, PointerButton, Pos2, RawInput, Rect, Sense, TextEdit, pos2, vec2};

use crate::theme::Theme;

fn key(key: Key, shift: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers { shift, ..Modifiers::NONE },
    }
}

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
        events,
        ..Default::default()
    }
}

/// Look at the painted result, not an implementation flag: navigation rows
/// paint themselves and did not show egui's built-in focused-button style.
fn rings(output: &FullOutput, palette: Palette) -> Vec<(Rect, Rect)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.stroke == Stroke::new(2.0, palette.ink) && rect.fill == egui::Color32::TRANSPARENT => {
                Some((rect.rect, shape.clip_rect))
            }
            _ => None,
        })
        .collect()
}

struct Bench {
    ctx: Context,
    palette: Palette,
    text: String,
    field: Id,
    navigation: Id,
    switch: Id,
    on: bool,
    chosen: bool,
}

impl Bench {
    fn new() -> Self {
        let ctx = Context::default();
        let theme = Theme::default();
        theme.apply(&ctx, 1.0);
        let mut bench = Self {
            ctx,
            palette: theme.palette(),
            text: String::new(),
            field: Id::unique("keyboard_focus_field"),
            navigation: Id::NULL,
            switch: Id::NULL,
            on: false,
            chosen: false,
        };
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        bench
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        let Self {
            ctx,
            palette,
            text,
            field,
            navigation,
            switch,
            on,
            chosen,
        } = self;
        let mut output = ctx.run_ui(input(events.clone()), |ui| {
            // InputState marks a held key as repeating before plugins run.
            // Compare what matters here without treating that as our edit.
            let observed = ui.input(|i| {
                i.events
                    .iter()
                    .cloned()
                    .map(|mut event| {
                        if let Event::Key { repeat, .. } = &mut event {
                            *repeat = false;
                        }
                        event
                    })
                    .collect::<Vec<_>>()
            });
            assert_eq!(observed, events, "the plugin must not consume or rewrite input");
            ui.set_width(200.0);
            ui.add(TextEdit::singleline(text).id(*field));
            let row = crate::ui::widgets::navigation(ui, palette, "search", "Preferences", false);
            *navigation = row.id;
            *chosen |= row.clicked();
            *switch = crate::ui::widgets::switch(ui, palette, on).id;
        });
        output.textures_delta.clear();
        output
    }

    fn expect_ring(&self, output: &FullOutput, id: Id) {
        let widget = self
            .ctx
            .viewport(|v| v.prev_pass.widgets.get(id).copied())
            .expect("rendered control");
        assert_eq!(self.ctx.memory(|m| m.focused()), Some(id));
        assert_eq!(rings(output, self.palette), vec![(widget.rect, widget.interact_rect)]);
    }
}

#[test]
fn tab_and_shift_tab_show_the_real_navigation_destination_and_keep_typing() {
    let mut bench = Bench::new();
    bench.ctx.memory_mut(|m| m.request_focus(bench.field));
    let output = bench.frame(vec![Event::Text("draft".into())]);
    assert_eq!(bench.text, "draft");
    assert!(
        rings(&output, bench.palette).is_empty(),
        "programmatic/mouse-style focus has no extra ring"
    );

    let output = bench.frame(vec![key(Key::Tab, false)]);
    bench.expect_ring(&output, bench.navigation);
    assert!(!bench.chosen, "Tab moves focus without activating the navigation entry");
    let output = bench.frame(vec![key(Key::Enter, false)]);
    assert!(bench.chosen, "egui still handles activation");
    bench.expect_ring(&output, bench.navigation);

    bench.frame(vec![key(Key::Tab, true)]);
    let output = bench.frame(Vec::new()); // egui's intentional backwards-navigation delay
    bench.expect_ring(&output, bench.field);
    let output = bench.frame(vec![Event::Text(" more".into())]);
    assert!(bench.text.contains("more"));
    bench.expect_ring(&output, bench.field);

    bench.frame(vec![key(Key::Tab, false)]);
    let output = bench.frame(vec![key(Key::Tab, false)]);
    bench.expect_ring(&output, bench.switch);
}

#[test]
fn pointer_motion_keeps_the_ring_and_a_press_hides_it_without_eating_the_click() {
    let mut bench = Bench::new();
    bench.ctx.memory_mut(|m| m.request_focus(bench.field));
    bench.frame(Vec::new());
    bench.frame(vec![key(Key::Tab, false)]);
    let at = bench.ctx.read_response(bench.navigation).expect("navigation").rect.center();
    let output = bench.frame(vec![Event::PointerMoved(at)]);
    bench.expect_ring(&output, bench.navigation);
    for pressed in [true, false] {
        let output = bench.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
        assert!(rings(&output, bench.palette).is_empty());
    }
    assert!(bench.chosen, "the mouse still activates the row");
}

/// A row in a clipped pane, with a window drawn on top. The ring must
/// remain in the row's layer and clip, rather than an always-on-top overlay.
fn clipped_frame(ctx: &Context, events: Vec<Event>, shown: bool, enabled: bool, covered: bool, focused: bool) -> FullOutput {
    let clip = Rect::from_min_max(pos2(40.0, 50.0), pos2(240.0, 70.0));
    let row = Rect::from_min_size(pos2(40.0, 40.0), vec2(200.0, 40.0));
    let mut raw = input(events);
    raw.focused = focused;
    let mut output = ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx().clone();
        if shown {
            egui::Area::new(Id::unique("focus_pane"))
                .order(Order::Middle)
                .fixed_pos(row.min)
                .show(&ctx, |ui| {
                    ui.set_clip_rect(clip);
                    if !enabled {
                        ui.disable();
                    }
                    let response = ui.interact(row, Id::unique("clipped_focus_control"), Sense::click());
                    response.request_focus();
                });
        }
        if covered {
            let layer = LayerId::new(Order::Foreground, Id::unique("covering_window"));
            Painter::new(ctx, layer, row).rect_filled(row, ui.visuals().window_corner_radius, Theme::default().palette().raised);
        }
    });
    output.textures_delta.clear();
    output
}

#[test]
fn the_ring_keeps_its_controls_clip_and_layer_and_disappears_with_it() {
    let ctx = Context::default();
    let theme = Theme::default();
    theme.apply(&ctx, 1.0);
    for _ in 0..3 {
        clipped_frame(&ctx, Vec::new(), true, true, false, true);
    }
    let output = clipped_frame(&ctx, vec![key(Key::Tab, false)], true, true, true, true);
    let painted = rings(&output, theme.palette());
    assert_eq!(painted.len(), 1);
    assert_eq!(painted[0].1, Rect::from_min_max(pos2(40.0, 50.0), pos2(240.0, 70.0)));
    assert!(painted[0].0.top() < painted[0].1.top(), "exercise a partly scrolled-out control");
    let index = |stroke: bool| {
        output.shapes.iter().position(|s| match &s.shape {
            egui::Shape::Rect(r) if stroke => r.stroke == Stroke::new(2.0, theme.palette().ink),
            egui::Shape::Rect(r) => r.fill == theme.palette().raised,
            _ => false,
        })
    };
    assert!(
        index(true).expect("ring") < index(false).expect("cover"),
        "the covering window paints after the ring"
    );
    for (shown, enabled, focused) in [(true, false, true), (true, true, false), (false, true, true)] {
        let output = clipped_frame(&ctx, Vec::new(), shown, enabled, false, focused);
        assert!(
            rings(&output, theme.palette()).is_empty(),
            "shown={shown}, enabled={enabled}, focused={focused}"
        );
    }
}

#[test]
fn a_skin_reload_updates_the_ring_without_resetting_keyboard_navigation() {
    let mut bench = Bench::new();
    bench.ctx.memory_mut(|m| m.request_focus(bench.field));
    bench.frame(Vec::new());
    bench.frame(vec![key(Key::Tab, false)]);
    let light = Theme {
        ink: "#151515".into(),
        bar: "#EEEEEE".into(),
        ..Theme::default()
    };
    light.apply(&bench.ctx, 1.0);
    bench.palette = light.palette();
    let output = bench.frame(Vec::new());
    bench.expect_ring(&output, bench.navigation);
}
