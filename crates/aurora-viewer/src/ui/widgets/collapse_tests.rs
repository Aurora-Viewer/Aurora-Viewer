use super::*;
use crate::ui::layout::{Edges, TITLE_BAR_H, TITLE_GRIP, keep_title_in};

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
        // as the theme sets it: the short title of a minimized bar is
        // dragged, not selected
        fixture.ctx.all_styles_mut(|s| s.interaction.selectable_labels = false);
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
                    floater = floater.framed_header("box");
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
        self.ctx.memory(|m| m.area_rect(egui::Id::unique("collapse_fixture")).unwrap())
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
            .read_response(egui::Id::unique("collapse_fixture").with("title_double_click"))
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
        assert!((collapsed.width() - MINIMIZED_W).abs() < 1.0, "{collapsed:?}");
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
    assert!((collapsed.width() - MINIMIZED_W).abs() < 1.0, "{collapsed:?}");
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

/// The toolbar button of a minimized window expands it instead of closing
/// it (issue #269, LLFloaterReg::toggleInstance); shown, it closes.
#[test]
fn toggling_a_minimized_floater_expands_it_and_a_shown_one_closes() {
    let mut f = Fixture::new(false);
    let expanded = f.show(vec![]);
    f.double_click(f.title());
    assert!(floater_minimized(&f.ctx, "collapse_fixture"));
    let bodies = f.bodies;
    toggle_floater(&f.ctx, "collapse_fixture", &mut f.open);
    assert!(f.open, "a minimized window stays open");
    for _ in 0..8 {
        f.show(vec![]);
    }
    let restored = f.show(vec![]);
    assert!(!floater_minimized(&f.ctx, "collapse_fixture"));
    assert!((restored.size() - expanded.size()).length() < 1.0, "{expanded:?} -> {restored:?}");
    assert!(f.bodies > bodies, "the contents are shown again");
    toggle_floater(&f.ctx, "collapse_fixture", &mut f.open);
    assert!(!f.open, "a window shown closes");
}

/// A window closed while minimized, by its close button or from outside,
/// opens again expanded (LLFloater::closeFloater and openFloater unminimize).
#[test]
fn a_floater_closed_minimized_opens_again_expanded() {
    // closed by its own close button
    let mut f = Fixture::new(false);
    let expanded = f.show(vec![]);
    f.double_click(f.title());
    for _ in 0..40 {
        f.show(vec![]);
    }
    // the close button ends the bar, inside its margin
    let bar = f.show(vec![]);
    let close = egui::pos2(bar.right() - 18.0, f.title().y);
    f.show(vec![egui::Event::PointerMoved(close)]);
    f.button(close, true);
    f.button(close, false);
    f.show(vec![]);
    assert!(!f.open, "the close button closes the minimized window");
    toggle_floater(&f.ctx, "collapse_fixture", &mut f.open);
    assert!(f.open);
    for _ in 0..8 {
        f.show(vec![]);
    }
    let reopened = f.show(vec![]);
    assert!((reopened.size() - expanded.size()).length() < 1.0, "{expanded:?} -> {reopened:?}");

    // closed from outside (an older layout, a command): opened by a menu entry
    let mut f = Fixture::new(false);
    let expanded = f.show(vec![]);
    f.double_click(f.title());
    f.open = false;
    f.show(vec![]);
    open_floater(&f.ctx, "collapse_fixture", &mut f.open);
    for _ in 0..8 {
        f.show(vec![]);
    }
    let reopened = f.show(vec![]);
    assert!(f.open);
    assert!((reopened.size() - expanded.size()).length() < 1.0, "{expanded:?} -> {reopened:?}");
}

/// The bar of a minimized window is short (issue #276, UIMinimizedWidth);
/// a window narrower than it keeps its own width.
#[test]
fn a_minimized_floater_is_a_short_bar() {
    assert_eq!(minimized_width(440.0), MINIMIZED_W);
    assert_eq!(minimized_width(150.0), 150.0);
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

/// The room the floaters of the fixture live in: its screen without the bars.
fn room() -> egui::Rect {
    crate::ui::layout::default_work_area(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0)))
}

#[test]
fn title_bar_positions_are_kept_in_the_room() {
    let area = egui::Rect::from_min_max(egui::pos2(0.0, 30.0), egui::pos2(1280.0, 860.0));
    let all = Edges::default();
    let open = Edges {
        left: false,
        right: false,
        bottom: false,
    };
    let keep = |x, y, edges| keep_title_in(area, egui::pos2(x, y), egui::vec2(440.0, 300.0), edges);
    // well inside: untouched
    assert_eq!(keep(300.0, 200.0, all), egui::pos2(300.0, 200.0));
    // by default the whole window comes back in
    assert_eq!(keep(300.0, -700.0, all).y, 30.0);
    assert_eq!(keep(300.0, 5000.0, all).y, 860.0 - 300.0);
    assert_eq!(keep(5000.0, 200.0, all).x, 1280.0 - 440.0);
    assert_eq!(keep(-5000.0, 200.0, all).x, 0.0);
    // edges let open: a width of its title bar to grab stays in, its
    // whole title bar above the bottom, never above the top
    assert_eq!(keep(300.0, -700.0, open).y, 30.0);
    assert_eq!(keep(300.0, 5000.0, open).y, 860.0 - TITLE_BAR_H);
    assert_eq!(keep(5000.0, 200.0, open).x, 1280.0 - TITLE_GRIP);
    assert_eq!(keep(-5000.0, 200.0, open).x, -(440.0 - TITLE_GRIP));
    // a default place is brought back whole when the window fits
    assert_eq!(
        fit_in(area, egui::pos2(1100.0, 700.0), egui::vec2(440.0, 300.0)),
        egui::pos2(840.0, 560.0)
    );
    assert_eq!(
        fit_in(area, egui::pos2(100.0, 0.0), egui::vec2(440.0, 2000.0)),
        egui::pos2(100.0, 30.0)
    );
}

/// A window as tall as its content, with more content than the room is
/// high: it stops at the bars and its body scrolls.
#[test]
fn a_content_sized_floater_taller_than_the_room_stays_between_the_bars() {
    let mut f = Fixture::new(true);
    let short = f.show(vec![]);
    assert!(room().contains_rect(short), "{short:?}");
    f.body_height = 1400.0;
    for _ in 0..8 {
        f.show(vec![]);
    }
    let tall = f.show(vec![]);
    assert!(tall.height() <= room().height() + 0.5, "{tall:?}");
    assert!(
        tall.top() >= room().top() - 0.5 && tall.bottom() <= room().bottom() + 0.5,
        "{tall:?}"
    );
    assert!(room().contains(f.title()));
    // and it takes its own height again when its content fits
    f.body_height = 180.0;
    for _ in 0..8 {
        f.show(vec![]);
    }
    assert!((f.show(vec![]).height() - short.height()).abs() < 1.0);
}

/// Wherever a window is sent, and even larger than the room, its title bar
/// can be reached.
#[test]
fn a_floater_cannot_be_put_with_its_title_out_of_reach() {
    for (pos, size) in [
        (egui::pos2(300.0, -700.0), egui::vec2(440.0, 300.0)),
        (egui::pos2(300.0, 5000.0), egui::vec2(440.0, 300.0)),
        (egui::pos2(5000.0, 200.0), egui::vec2(440.0, 300.0)),
        (egui::pos2(-5000.0, 200.0), egui::vec2(440.0, 300.0)),
        (egui::pos2(300.0, -900.0), egui::vec2(440.0, 2000.0)),
        (egui::pos2(-900.0, -900.0), egui::vec2(3000.0, 2000.0)),
    ] {
        let mut f = Fixture::new(false);
        place_floater(&f.ctx, "collapse_fixture", pos, size);
        for _ in 0..6 {
            f.show(vec![]);
        }
        let rect = f.show(vec![]);
        // every edge stops it by default: the whole window comes back in
        assert!(
            room().expand(0.5).contains_rect(rect),
            "sent to {pos:?} at {size:?}: window {rect:?} out of {:?}",
            room()
        );
        // one larger than the room is brought down to it: nothing of it is
        // left under a bar
        assert!(
            rect.width() <= room().width() + 0.5 && rect.height() <= room().height() + 0.5,
            "sent to {pos:?} at {size:?}: {rect:?}"
        );
    }
}

/// What a window paints, and where it takes the pointer, stops at the bars:
/// a window never covers them, it passes behind.
#[test]
fn a_floater_paints_between_the_bars_only() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let (mut clip, mut open) = (egui::Rect::NOTHING, true);
    for _ in 0..4 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                Floater::new("clip_fixture", "Window title", egui::pos2(300.0, 200.0), egui::vec2(440.0, 300.0))
                    .silent()
                    .show(ui.ctx(), &p, &mut open, |ui| clip = ui.clip_rect());
            },
        );
        output.textures_delta.clear();
    }
    assert!(clip.is_positive(), "{clip:?}");
    assert!(
        clip.top() >= room().top() - 0.5 && clip.bottom() <= room().bottom() + 0.5,
        "{clip:?}"
    );
}
