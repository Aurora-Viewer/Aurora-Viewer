use super::*;
use egui::{Event as Input, Modifiers};

struct UiTest {
    ctx: egui::Context,
    picker: ColorPicker,
    prefs: Preferences,
    time: f64,
    owner: Option<Pos2>,
    owner_visible: bool,
    screen_size: Vec2,
}

impl UiTest {
    fn new() -> Self {
        let mut test = Self {
            ctx: egui::Context::default(),
            picker: ColorPicker::default(),
            prefs: Preferences::default(),
            time: 0.0,
            owner: None,
            owner_visible: false,
            screen_size: Vec2::new(1000.0, 760.0),
        };
        crate::theme::Theme::default().apply(&test.ctx, 1.0);
        test.picker.open([0.5, 0.25, 1.0], false);
        for _ in 0..4 {
            test.frame(Vec::new());
        }
        test
    }
    fn frame(&mut self, events: Vec<Input>) -> (Option<Event>, egui::FullOutput) {
        self.time += 1.0 / 60.0;
        let mut result = None;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen_size)),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                if let Some(pos) = self.owner {
                    widgets::place_floater(ui.ctx(), "color_picker_owner_fixture", pos, Vec2::new(300.0, 120.0));
                    self.owner_visible = true;
                    self.owner = None;
                }
                if self.owner_visible {
                    let mut open = true;
                    Floater::new(
                        "color_picker_owner_fixture",
                        "owner",
                        Pos2::new(12.0, 40.0),
                        Vec2::new(300.0, 120.0),
                    )
                    .auto_height()
                    .framed_header("box")
                    .show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut open, |ui| {
                        ui.label("owner");
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
                        self.picker.set_origin(rect);
                    });
                }
                result = result.or(self
                    .picker
                    .show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut self.prefs));
            },
        );
        // Headless frames inspect shapes and commands without uploading fonts
        // to a renderer. egui requires acknowledging these unused deltas.
        output.textures_delta.clear();
        (result, output)
    }
    fn button(pos: Pos2, pressed: bool) -> Input {
        Input::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Modifiers::NONE,
        }
    }
    fn click(&mut self, control: Control) -> (Option<Event>, egui::FullOutput) {
        let pos = self.picker.controls[&control].center();
        self.frame(vec![Input::PointerMoved(pos), Self::button(pos, true)]);
        self.frame(vec![Self::button(pos, false)])
    }
    fn text(&mut self, control: Control, text: &str) -> Option<Event> {
        self.click(control);
        self.frame(vec![
            Input::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            Input::Text(text.to_owned()),
        ])
        .0
    }

    fn drag_to(&mut self, id: egui::Id, pos: Pos2) {
        let rect = self.ctx.memory(|m| m.area_rect(id)).unwrap();
        let start = rect.min + egui::vec2(70.0, 20.0);
        let end = start + (pos - rect.min);
        self.frame(vec![Input::PointerMoved(start), Self::button(start, true)]);
        self.frame(vec![Input::PointerMoved(end)]);
        self.frame(vec![Self::button(end, false)]);
        self.frame(Vec::new());
    }
}

#[test]
fn adjacent_position_prefers_right_then_left_and_keeps_dialog_on_screen() {
    let screen = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1000.0, 760.0));
    let size = Vec2::new(460.0, 525.0);
    let parent = |x, y| Rect::from_min_size(Pos2::new(x, y), Vec2::new(344.0, 550.0));
    assert_eq!(
        adjacent_position(screen, parent(32.0, 40.0), size, DockSide::Right).0,
        Pos2::new(376.0, 40.0)
    );
    assert_eq!(
        adjacent_position(screen, parent(664.0, 40.0), size, DockSide::Right).0,
        Pos2::new(204.0, 40.0)
    );
    assert_eq!(
        adjacent_position(screen, parent(300.0, 500.0), size, DockSide::Right).0,
        Pos2::new(20.0, 265.0)
    );
    assert_eq!(
        adjacent_position(screen, parent(32.0, -10.0), size, DockSide::Right).0,
        Pos2::new(376.0, 30.0)
    );
}

#[test]
fn opening_from_a_moved_parent_overrides_saved_position_and_aligns_both_sides() {
    let mut t = UiTest::new();
    let owner = egui::Id::unique("color_picker_owner_fixture");
    for pos in [Pos2::new(12.0, 40.0), Pos2::new(680.0, 40.0), Pos2::new(680.0, 540.0)] {
        t.owner = Some(pos);
        t.picker.open_from([0.5, 0.25, 1.0], false, owner);
        for _ in 0..4 {
            t.frame(Vec::new());
        }
        let parent = t.ctx.memory(|m| m.area_rect(owner)).unwrap();
        let picker = t.ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW_ID))).unwrap();
        if pos.x < 500.0 {
            assert!((picker.left() - parent.right()).abs() < 0.5, "{parent:?} -> {picker:?}");
        } else {
            assert!((picker.right() - parent.left()).abs() < 0.5, "{parent:?} -> {picker:?}");
        }
        if pos.y < 500.0 {
            assert!((picker.top() - parent.top()).abs() < 0.5);
        } else {
            assert!(picker.bottom() <= 760.5, "{picker:?}");
        }
        assert_eq!(t.picker.parent, Some(owner));
        t.picker.close();
    }
}

#[test]
fn dragging_the_parent_follows_every_frame_and_switches_back_to_either_side() {
    let mut t = UiTest::new();
    let owner = egui::Id::unique("color_picker_owner_fixture");
    t.owner = Some(Pos2::new(12.0, 40.0));
    t.picker.open_from([0.5, 0.25, 1.0], false, owner);
    for _ in 0..4 {
        t.frame(Vec::new());
    }
    let parent = t.ctx.memory(|m| m.area_rect(owner)).unwrap();
    let start = parent.min + egui::vec2(70.0, 20.0);
    t.frame(vec![Input::PointerMoved(start), UiTest::button(start, true)]);
    for dx in [100.0, 220.0, 420.0, 560.0, 220.0, 0.0] {
        t.frame(vec![Input::PointerMoved(start + egui::vec2(dx, 24.0))]);
        let parent = t.ctx.memory(|m| m.area_rect(owner)).unwrap();
        let picker = t.ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW_ID))).unwrap();
        let expected = adjacent_position(t.ctx.content_rect(), parent, picker.size(), t.picker.dock.unwrap()).0;
        assert!(picker.min.distance(expected) < 0.5, "{parent:?} -> {picker:?}, wanted {expected:?}");
    }
    let parent = t.ctx.memory(|m| m.area_rect(owner)).unwrap();
    assert!((parent.top() - 64.0).abs() < 0.5, "the parent must really move: {parent:?}");
    t.frame(vec![UiTest::button(start + egui::vec2(0.0, 24.0), false)]);
    assert!(t.picker.is_open());
}

#[test]
fn detached_dialog_moves_independently_and_snapping_to_either_side_restores_following() {
    let mut t = UiTest::new();
    t.screen_size.x = 1600.0;
    let owner = egui::Id::unique("color_picker_owner_fixture");
    let picker = egui::Id::unique(WINDOW_ID);
    t.owner = Some(Pos2::new(12.0, 40.0));
    t.picker.open_from([0.5, 0.25, 1.0], false, owner);
    for _ in 0..4 {
        t.frame(Vec::new());
    }
    assert_eq!(t.picker.dock, Some(DockSide::Right));
    t.drag_to(picker, Pos2::new(450.0, 180.0));
    assert_eq!(t.picker.dock, None);
    let detached = t.ctx.memory(|m| m.area_rect(picker)).unwrap();
    t.drag_to(owner, Pos2::new(40.0, 80.0));
    assert_eq!(t.ctx.memory(|m| m.area_rect(picker)).unwrap(), detached);

    let parent = t.ctx.memory(|m| m.area_rect(owner)).unwrap();
    t.drag_to(picker, Pos2::new(parent.right() + 7.0, parent.top() + 6.0));
    assert_eq!(t.picker.dock, Some(DockSide::Right));
    let snapped = t.ctx.memory(|m| m.area_rect(picker)).unwrap();
    assert!(snapped.min.distance(Pos2::new(parent.right(), parent.top())) < 0.5);
    t.drag_to(owner, Pos2::new(100.0, 100.0));
    let followed = t.ctx.memory(|m| m.area_rect(picker)).unwrap();
    assert!(followed.min.distance(Pos2::new(400.0, 100.0)) < 0.5);

    t.drag_to(picker, Pos2::new(800.0, 260.0));
    assert_eq!(t.picker.dock, None);
    let detached = t.ctx.memory(|m| m.area_rect(picker)).unwrap();
    t.drag_to(owner, Pos2::new(600.0, 80.0));
    assert_eq!(t.ctx.memory(|m| m.area_rect(picker)).unwrap(), detached);
    t.drag_to(picker, Pos2::new(600.0 - detached.width() - 6.0, 85.0));
    assert_eq!(t.picker.dock, Some(DockSide::Left));
    t.drag_to(owner, Pos2::new(620.0, 100.0));
    let followed = t.ctx.memory(|m| m.area_rect(picker)).unwrap();
    assert!(followed.min.distance(Pos2::new(620.0 - followed.width(), 100.0)) < 0.5);
    assert_eq!(t.picker.dock, Some(DockSide::Left), "keep the chosen side even when both sides fit");
    t.drag_to(owner, Pos2::new(200.0, 80.0));
    let followed = t.ctx.memory(|m| m.area_rect(picker)).unwrap();
    assert!(followed.min.distance(Pos2::new(500.0, 80.0)) < 0.5);
    assert_eq!(t.picker.dock, Some(DockSide::Right));
}

#[test]
fn snapping_requires_both_top_and_edge_proximity_and_a_side_that_fits_on_screen() {
    let screen = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1600.0, 900.0));
    let parent = Rect::from_min_size(Pos2::new(600.0, 80.0), Vec2::new(344.0, 550.0));
    let size = Vec2::new(460.0, 525.0);
    for (side, x) in [(DockSide::Right, parent.right()), (DockSide::Left, parent.left() - size.x)] {
        for offset in [Vec2::ZERO, egui::vec2(-9.0, 9.0), egui::vec2(9.0, -9.0)] {
            let window = Rect::from_min_size(Pos2::new(x, parent.top()) + offset, size);
            assert_eq!(nearby_snap(screen, parent, window), Some(side));
        }
        for offset in [egui::vec2(11.0, 0.0), egui::vec2(0.0, 11.0)] {
            let window = Rect::from_min_size(Pos2::new(x, parent.top()) + offset, size);
            assert_eq!(nearby_snap(screen, parent, window), None);
        }
    }
    let narrow = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(700.0, 900.0));
    let middle = Rect::from_min_size(Pos2::new(300.0, 80.0), parent.size());
    let clamped = Rect::from_min_size(Pos2::new(20.0, 80.0), size);
    assert_eq!(nearby_snap(narrow, middle, clamped), None);
}

#[test]
fn apply_is_checked_without_a_saved_choice_and_retains_the_users_choice_on_reopening() {
    let mut t = UiTest::new();
    let old: Preferences = serde_json::from_str("{}").unwrap();
    assert!(old.apply);
    assert!(t.prefs.apply);
    t.click(Control::Apply);
    assert!(!t.prefs.apply);
    let persisted = serde_json::to_string(&t.prefs).unwrap();
    t.picker.close();
    t.prefs = serde_json::from_str(&persisted).unwrap();
    t.picker.open([0.5, 0.25, 1.0], false);
    for _ in 0..4 {
        t.frame(Vec::new());
    }
    assert!(!t.prefs.apply);
    assert_eq!(t.click(Control::Slot(18)).0, None);
    assert_eq!(t.click(Control::Apply).0, Some(Event::Preview([1.0, 0.0, 0.0])));
    t.picker.close();
    t.picker.open([0.5, 0.25, 1.0], false);
    t.frame(Vec::new());
    assert!(t.prefs.apply);
}

#[test]
fn moving_the_dialog_shows_a_fading_cone_to_the_current_swatch_without_changing_color() {
    let mut t = UiTest::new();
    t.owner = Some(Pos2::new(12.0, 40.0));
    t.picker
        .open_from([0.5, 0.25, 1.0], false, egui::Id::unique("color_picker_owner_fixture"));
    for _ in 0..4 {
        t.frame(Vec::new());
    }
    let window = t.ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW_ID))).unwrap();
    let start = window.min + egui::vec2(80.0, 20.0);
    let end = start + egui::vec2(100.0, 80.0);
    t.frame(vec![Input::PointerMoved(start), UiTest::button(start, true)]);
    t.frame(vec![Input::PointerMoved(end)]);
    for _ in 0..8 {
        let (event, output) = t.frame(Vec::new());
        assert_eq!(event, None);
        assert!(
            output.shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.len() == 4
                && mesh.vertices[..2].iter().all(|v| v.color.a() == 0) && mesh.vertices[2].color.a() > 0
            )),
            "the context cone must appear during the window drag"
        );
    }
    let moved = t.ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW_ID))).unwrap();
    assert!(moved.min.distance(window.min + egui::vec2(100.0, 80.0)) < 0.5);
    t.frame(vec![UiTest::button(end, false)]);
    for _ in 0..12 {
        t.frame(Vec::new());
    }
    let (_, output) = t.frame(Vec::new());
    assert!(!output.shapes.iter().any(|shape| matches!(
        &shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.len() == 4
            && mesh.vertices[..2].iter().all(|v| v.color.a() == 0) && mesh.vertices[2].color.a() > 0
    )));
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, [0.5, 0.25, 1.0]);
}

#[test]
fn owner_link_points_to_the_swatch_on_all_sides_and_avoids_overlapping_windows() {
    let origin = Rect::from_min_size(Pos2::new(400.0, 300.0), Vec2::splat(40.0));
    let shade = Color32::from_gray(32).gamma_multiply(0.4);
    for pos in [
        Pos2::new(500.0, 200.0),
        Pos2::new(0.0, 200.0),
        Pos2::new(350.0, 500.0),
        Pos2::new(350.0, 0.0),
    ] {
        let window = Rect::from_min_size(pos, Vec2::new(200.0, 200.0));
        let mesh = owner_link(origin, window, shade);
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.vertices.len(), 4);
        assert!(mesh.vertices[..2].iter().all(|v| origin.contains(v.pos) && v.color.a() == 0));
        assert!(mesh.vertices[2..].iter().all(|v| window.contains(v.pos) && v.color == shade));
    }
    assert!(owner_link(origin, origin.expand(20.0), shade).vertices.is_empty());
}

#[test]
fn real_hex_field_validates_input_and_copy_uses_lsl_decimal_points() {
    let mut t = UiTest::new();
    t.click(Control::Mode(2));
    assert_eq!(t.text(Control::Hex, "#33AA66"), Some(Event::Preview([0.2, 170.0 / 255.0, 0.4])));
    assert_eq!(t.picker.draft.as_ref().unwrap().hex, "33AA66");
    let (_, output) = t.click(Control::CopyLsl);
    assert!(
        output
            .platform_output
            .commands
            .iter()
            .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if text == "<0.200, 0.667, 0.400>"))
    );
    assert_eq!(t.text(Control::Hex, "GG0000"), None);
    assert_eq!(t.click(Control::Accept).0, None);
    assert!(t.picker.is_open());
    assert_eq!(t.click(Control::Cancel).0, Some(Event::Cancel));
    assert!(!t.picker.is_open());
}

#[test]
fn apply_off_defers_edits_and_enabling_it_previews_the_draft() {
    let mut t = UiTest::new();
    t.click(Control::Apply);
    assert!(!t.prefs.apply);
    assert_eq!(t.click(Control::Slot(18)).0, None);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, [1.0, 0.0, 0.0]);
    assert_eq!(t.click(Control::Apply).0, Some(Event::Preview([1.0, 0.0, 0.0])));
    t.click(Control::Apply);
    assert_eq!(t.click(Control::Accept).0, Some(Event::Accept([1.0, 0.0, 0.0])));
}

#[test]
fn palette_drag_saves_current_color_without_applying_another_color() {
    let mut t = UiTest::new();
    let rgb = t.picker.draft.as_ref().unwrap().rgb;
    let source = t.picker.controls[&Control::Current].center();
    let target = t.picker.controls[&Control::Slot(31)].center();
    t.frame(vec![Input::PointerMoved(source), UiTest::button(source, true)]);
    t.frame(vec![Input::PointerMoved(source + egui::vec2(15.0, 0.0))]);
    t.frame(vec![Input::PointerMoved(target)]);
    assert_eq!(t.frame(vec![UiTest::button(target, false)]).0, None);
    assert_eq!(t.prefs.palette[31], rgb);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, rgb);
}

#[test]
fn pipette_is_transient_and_escape_first_stops_it_then_cancels() {
    let mut t = UiTest::new();
    t.click(Control::Pipette);
    assert!(t.picker.pipette);
    assert!(t.picker.needs_sample((20.0, 30.0)));
    assert!(!t.picker.needs_sample((20.0, 30.0)));
    let id = Uuid::from_u128(999);
    t.picker.sample(Some((id, [0.1, 0.2, 0.3])));
    assert_eq!(t.frame(Vec::new()).0, Some(Event::Preview([0.1, 0.2, 0.3])));
    assert_eq!(t.picker.sampled, Some(id));
    let escape = || Input::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    assert_eq!(t.frame(vec![escape()]).0, None);
    assert!(!t.picker.pipette);
    assert_eq!(t.frame(vec![escape()]).0, Some(Event::Cancel));
}

#[test]
fn lsl_and_hsl_numeric_fields_commit_and_clamp_their_values() {
    let mut t = UiTest::new();
    let enter = || Input::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    t.click(Control::Mode(1));
    t.text(Control::Rgb(0), "0.125");
    t.frame(vec![enter()]);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb[0], 0.125);
    t.text(Control::Rgb(1), "2");
    t.frame(vec![enter()]);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb[1], 1.0);
    t.text(Control::Hsl(2), "50");
    t.frame(vec![enter()]);
    assert_eq!(t.picker.draft.as_ref().unwrap().hsl[2], 0.5);
}

#[test]
fn changing_formats_keeps_float_precision_and_mixed_white_can_be_applied() {
    let mut t = UiTest::new();
    let original = t.picker.draft.as_ref().unwrap().rgb;
    t.click(Control::Mode(1));
    t.click(Control::Mode(2));
    t.click(Control::Mode(0));
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, original);
    t.picker.open([1.0; 3], true);
    t.frame(Vec::new());
    assert_eq!(t.click(Control::Slot(31)).0, Some(Event::Preview([1.0; 3])));
}
