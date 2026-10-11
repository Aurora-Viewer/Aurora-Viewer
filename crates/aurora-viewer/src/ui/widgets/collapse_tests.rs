use super::*;

struct Fixture {
    ctx: egui::Context,
    frame: usize,
    fixed: bool,
    framed: bool,
    auto_height: bool,
    body_height: f32,
    open: bool,
    text: String,
    bodies: usize,
}

impl Fixture {
    fn new(fixed: bool) -> Self {
        Self::with_header(fixed, false)
    }

    fn with_header(fixed: bool, framed: bool) -> Self {
        let mut fixture = Self {
            ctx: egui::Context::default(),
            frame: 0,
            fixed,
            framed,
            auto_height: false,
            body_height: 180.0,
            open: true,
            text: "kept contents".into(),
            bodies: 0,
        };
        for _ in 0..8 {
            fixture.show(vec![]);
        }
        fixture
    }

    fn show(&mut self, events: Vec<egui::Event>) -> egui::Rect {
        self.frame += 1;
        let p = crate::theme::Theme::default().palette();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                let mut floater = Floater::new(
                    "collapse_fixture",
                    "Window title",
                    egui::pos2(300.0, 200.0),
                    egui::vec2(440.0, 300.0),
                )
                .silent();
                if self.fixed {
                    floater = floater.fixed();
                }
                if self.framed {
                    floater = floater.framed_header("cube");
                }
                if self.auto_height {
                    floater = floater.auto_height();
                }
                floater.show(ui.ctx(), &p, &mut self.open, |ui| {
                    self.bodies += 1;
                    ui.text_edit_singleline(&mut self.text);
                    ui.set_min_height(self.body_height);
                });
            },
        );
        output.textures_delta.clear();
        self.ctx.memory(|m| m.area_rect(egui::Id::new("collapse_fixture")).unwrap())
    }

    fn button(&mut self, pos: egui::Pos2, pressed: bool) {
        self.show(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }

    fn double_click(&mut self, pos: egui::Pos2) -> egui::Rect {
        self.show(vec![egui::Event::PointerMoved(pos)]);
        for _ in 0..2 {
            self.button(pos, true);
            self.button(pos, false);
        }
        self.show(vec![])
    }

    fn title(&self) -> egui::Pos2 {
        self.ctx
            .read_response(egui::Id::new("collapse_fixture").with("title_double_click"))
            .unwrap()
            .rect
            .center()
    }
}

#[test]
fn inspector_height_follows_content_after_growth_and_title_restoration() {
    let mut f = Fixture::with_header(false, true);
    f.auto_height = true;
    f.body_height = 80.0;
    for _ in 0..8 {
        f.show(vec![]);
    }
    let short = f.show(vec![]);
    f.body_height = 340.0;
    for _ in 0..8 {
        f.show(vec![]);
    }
    let tall = f.show(vec![]);
    assert!(tall.height() > short.height() + 250.0, "{short:?} -> {tall:?}");
    assert!((tall.width() - short.width()).abs() < 1.0);
    let collapsed = f.double_click(f.title());
    assert!(collapsed.height() < 50.0);
    f.body_height = 80.0;
    for _ in 0..40 {
        f.show(vec![]);
    }
    f.double_click(f.title());
    for _ in 0..8 {
        f.show(vec![]);
    }
    let restored = f.show(vec![]);
    assert!((restored.size() - short.size()).length() < 1.0, "{short:?} -> {restored:?}");
}

#[test]
fn title_double_click_collapses_and_restores_size_and_contents() {
    for fixed in [false, true] {
        let mut f = Fixture::new(fixed);
        let expanded = f.show(vec![]);
        let title = f.title();
        let collapsed = f.double_click(title);
        assert!(collapsed.height() < 40.0, "{collapsed:?}");
        assert!((collapsed.width() - expanded.width()).abs() < 1.0);
        let bodies = f.bodies;
        f.show(vec![]);
        assert_eq!(f.bodies, bodies, "hidden contents must not run");
        // Let the first double click expire before the restoration gesture.
        for _ in 0..40 {
            f.show(vec![]);
        }
        let restored = f.double_click(f.title());
        assert!((restored.size() - expanded.size()).length() < 1.0, "{expanded:?} -> {restored:?}");
        assert_eq!(f.text, "kept contents");
        assert!(f.open);
    }
}

#[test]
fn minimized_title_can_be_dragged_and_does_not_expand_on_single_click() {
    let mut f = Fixture::new(false);
    let expanded = f.show(vec![]);
    let collapsed = f.double_click(f.title());
    for _ in 0..40 {
        f.show(vec![]);
    }
    let start = f.title();
    let delta = egui::vec2(80.0, 60.0);
    f.show(vec![egui::Event::PointerMoved(start)]);
    f.button(start, true);
    f.show(vec![egui::Event::PointerMoved(start + delta)]);
    f.button(start + delta, false);
    let moved = f.show(vec![]);
    assert!((moved.min - collapsed.min - delta).length() < 1.0, "{collapsed:?} -> {moved:?}");
    assert!(moved.height() < 40.0);
    for _ in 0..40 {
        f.show(vec![]);
    }
    let restored = f.double_click(f.title());
    assert!((restored.size() - expanded.size()).length() < 1.0);
    assert!((restored.min - moved.min).length() < 1.0);
}

#[test]
fn inspector_header_restores_geometry_and_preserves_hidden_contents() {
    let mut f = Fixture::with_header(false, true);
    let expanded = f.show(vec![]);
    let collapsed = f.double_click(f.title());
    assert!(collapsed.height() >= 40.0 && collapsed.height() < 50.0, "{collapsed:?}");
    assert!((collapsed.width() - expanded.width()).abs() < 1.0);
    let bodies = f.bodies;
    for _ in 0..40 {
        f.show(vec![]);
    }
    assert_eq!(f.bodies, bodies);
    let restored = f.double_click(f.title());
    assert!((restored.size() - expanded.size()).length() < 1.0, "{expanded:?} -> {restored:?}");
    assert_eq!(f.text, "kept contents");
    assert!(f.open);
}

#[test]
fn double_click_in_body_does_not_collapse_window() {
    let mut f = Fixture::new(false);
    let expanded = f.show(vec![]);
    let actual = f.double_click(expanded.center());
    assert!((actual.size() - expanded.size()).length() < 1.0);
}

#[test]
fn expanded_geometry_survives_serialized_layout_and_restoration() {
    let mut f = Fixture::new(false);
    let expanded = f.show(vec![]);
    f.double_click(f.title());
    let layout = f.ctx.memory(|m| ron::to_string(m).unwrap());
    let memory: egui::Memory = ron::from_str(&layout).unwrap();
    let ctx = egui::Context::default();
    ctx.memory_mut(|m| *m = memory);
    f.ctx = ctx;
    for _ in 0..8 {
        f.show(vec![]);
    }
    assert!(f.show(vec![]).height() < 40.0);
    let restored = f.double_click(f.title());
    assert!((restored.size() - expanded.size()).length() < 1.0);
    for _ in 0..10 {
        assert!((f.show(vec![]).size() - expanded.size()).length() < 1.0);
    }
}

#[test]
fn placed_floater_takes_the_asked_position_and_size_expanded() {
    let mut f = Fixture::new(false);
    let before = f.show(vec![]);
    // collapsed, as a previous run may have left it
    f.double_click(f.title());
    assert!(f.show(vec![]).height() < 40.0);
    let bodies = f.bodies;
    let (pos, size) = (egui::pos2(40.0, 60.0), egui::vec2(520.0, 340.0));
    place_floater(&f.ctx, "collapse_fixture", pos, size);
    let mut placed = f.show(vec![]);
    for _ in 0..8 {
        placed = f.show(vec![]);
    }
    assert!((placed.min - pos).length() < 1.0, "{before:?} -> {placed:?}");
    assert!((placed.size() - size).length() < 1.0, "{before:?} -> {placed:?}");
    assert!(f.bodies > bodies, "the contents are shown again");
}
