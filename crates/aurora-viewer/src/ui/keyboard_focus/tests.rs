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
            egui::Shape::Rect(rect)
                if (rect.stroke == Stroke::new(2.0, palette.ink) || rect.stroke == Stroke::new(1.0, palette.ink))
                    && rect.fill == egui::Color32::TRANSPARENT =>
            {
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
        let painted = rings(output, self.palette);
        assert_eq!(painted.len(), 1);
        assert_eq!(painted[0].0, widget.rect);
        assert!(painted[0].1.intersects(widget.interact_rect));
        for shape in &output.shapes {
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.stroke == Stroke::new(2.0, self.palette.ink)
            {
                assert_eq!(rect.stroke_kind, StrokeKind::Outside, "never cover text inside the control");
            }
        }
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
    assert_eq!(
        painted[0].0, painted[0].1,
        "the constrained indicator outlines only the visible portion"
    );
    let index = |stroke: bool| {
        output.shapes.iter().position(|s| match &s.shape {
            egui::Shape::Rect(r) if stroke => r.stroke == Stroke::new(1.0, theme.palette().ink),
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

#[test]
fn sized_buttons_and_a_scroll_edge_have_a_real_visible_indicator() {
    for scrolled in [false, true] {
        for shared in [false, true] {
            let ctx = Context::default();
            let theme = Theme::default();
            theme.apply(&ctx, 1.0);
            let palette = theme.palette();
            let mut painter = crate::crash::software::Painter::default();
            let mut pixels = vec![0; 600 * 400];
            let background = (u32::from(palette.panel.r()) << 16) | (u32::from(palette.panel.g()) << 8) | u32::from(palette.panel.b());
            let mut visible = Rect::NOTHING;
            for pass in 0..3 {
                let events = if pass == 2 { vec![key(Key::Tab, false)] } else { Vec::new() };
                let mut output = ctx.run_ui(input(events), |ui| {
                    egui::Frame::new().inner_margin(20.0).show(ui, |ui| {
                        let mut button = |ui: &mut Ui| {
                            let response = if shared {
                                crate::ui::widgets::flat_button_sized(ui, &palette, "Wider glyphs gypé", vec2(180.0, 26.0))
                            } else {
                                ui.add_sized(vec2(180.0, 26.0), egui::Button::new("Wider glyphs gypé"))
                            };
                            response.request_focus();
                            visible = response.rect.intersect(ui.clip_rect());
                            if scrolled {
                                assert_eq!(response.rect.top(), ui.clip_rect().top(), "exercise an exact scroll edge");
                                ui.add_space(200.0);
                            }
                        };
                        if scrolled {
                            egui::ScrollArea::vertical().max_height(70.0).show(ui, button);
                        } else {
                            button(ui);
                        }
                    });
                });
                painter.update_textures(&output.textures_delta);
                if pass == 2 {
                    let ring = output
                        .shapes
                        .iter()
                        .filter(|shape| {
                            matches!(&shape.shape, egui::Shape::Rect(r)
                            if r.stroke == Stroke::new(1.0, palette.ink)
                                && r.fill == egui::Color32::TRANSPARENT
                                && r.stroke_kind == StrokeKind::Inside)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    assert_eq!(ring.len(), 1, "shared={shared}, scrolled={scrolled}");
                    let primitives = ctx.tessellate(ring, output.pixels_per_point);
                    painter.fill_and_draw(&mut pixels, [600, 400], &primitives, output.pixels_per_point, palette.panel);
                    let mut painted = 0;
                    for (at, pixel) in pixels.iter().enumerate() {
                        if *pixel != background {
                            painted += 1;
                            assert!(visible.contains(pos2((at % 600) as f32 + 0.5, (at / 600) as f32 + 0.5)));
                        }
                    }
                    assert!(painted > 100, "the outline must survive rasterization and clipping");
                }
                output.textures_delta.clear();
            }
        }
    }
}

/// A visual check without opening/activating a native window or injecting
/// input into the human's desktop. Uses Aurora's real fonts, widgets and
/// software painter (the splash/recovery window), and ordinary Tab events.
#[test]
#[ignore = "writes local focus captures to .scratch/keyboard-focus-captures"]
fn capture_controls_offscreen() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".scratch/keyboard-focus-captures");
    std::fs::create_dir_all(&folder).expect("capture directory");
    for (look, theme) in [
        ("dark", Theme::default()),
        (
            "light",
            Theme {
                ink: "#151515".into(),
                bar: "#EEEEEE".into(),
                field: "#FFFFFF".into(),
                panel: "#FAFAFA".into(),
                raised: "#D8D8D8".into(),
                ..Theme::default()
            },
        ),
    ] {
        let ctx = Context::default();
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, "");
        let palette = theme.palette();
        let mut painter = crate::crash::software::Painter::default();
        let mut pixels = vec![0u32; 480 * 420];
        let field = Id::unique("visual_focus_search");
        let mut text = "Parcelles : glyphes gypé".to_owned();
        let mut on = false;
        let mut height = 0.0;
        let mut frame = |events: Vec<Event>, save: Option<&str>| {
            let mut raw = input(events);
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 420.0)));
            let mut output = ctx.run_ui(raw, |ui| {
                egui::Frame::new().inner_margin(20.0).show(ui, |ui| {
                    ui.set_width(380.0);
                    ui.heading("Navigation au clavier");
                    ui.add_space(12.0);
                    ui.add(TextEdit::singleline(&mut text).id(field).desired_width(380.0));
                    crate::ui::widgets::navigation(ui, &palette, "search", "Réglages modifiés", false);
                    crate::ui::widgets::flat_button(ui, &palette, "Enregistrer");
                    crate::ui::widgets::switch_labeled(ui, &palette, &mut on, "Afficher les lignes");
                    ui.hyperlink_to("Documentation", "https://example.com");
                    crate::ui::widgets::flat_button_sized(ui, &palette, "Vérifier les mises à jour", vec2(220.0, 26.0));
                    ui.add_sized(vec2(220.0, 26.0), egui::Button::new("Bouton direct : gypé"));
                    crate::ui::widgets::height_slider(ui, &palette, &mut height, -1.0..=1.0, false);
                    ui.scope(|ui| ui.add(egui::Label::new("MW|_gypé").sense(Sense::click())));
                });
            });
            let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
            painter.paint(
                &mut pixels,
                [480, 420],
                &primitives,
                &output.textures_delta,
                output.pixels_per_point,
                palette.panel,
            );
            output.textures_delta.clear();
            if let Some(name) = save {
                let data = pixels
                    .iter()
                    .flat_map(|pixel| [(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])
                    .collect();
                let image = image::RgbImage::from_raw(480, 420, data).expect("capture pixels");
                let file = folder.join(format!("{look}-{name}.png"));
                image.save(&file).expect("write capture");
                println!("{}", file.display());
            }
        };
        for _ in 0..3 {
            frame(Vec::new(), None);
        }
        ctx.memory_mut(|m| m.request_focus(field));
        frame(Vec::new(), Some("mouse-field"));
        frame(vec![key(Key::Tab, false)], Some("navigation"));
        frame(vec![key(Key::Tab, true)], None);
        frame(Vec::new(), Some("field"));
        frame(vec![key(Key::Tab, false)], None);
        frame(vec![key(Key::Tab, false)], Some("button"));
        frame(vec![key(Key::Tab, false)], Some("switch"));
        frame(vec![key(Key::Tab, false)], Some("switch-label"));
        frame(vec![key(Key::Tab, false)], Some("link"));
        frame(vec![key(Key::Tab, false)], Some("sized-button"));
        frame(vec![key(Key::Tab, false)], Some("direct-button"));
        frame(vec![key(Key::Tab, false)], Some("height-slider"));
        frame(vec![key(Key::Tab, false)], Some("height-value"));
        frame(vec![key(Key::Tab, false)], Some("tight-label"));
    }
}
