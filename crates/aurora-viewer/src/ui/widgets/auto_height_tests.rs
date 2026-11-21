//! A window sized by its content (`Floater::auto_height` or `fixed`) comes
//! back to its size once the viewer's window, made tiny, is made large
//! again, whatever its body: a scroll area of its own (the Director's), a
//! plain tall body (Aurora Weather's) or a short one.

use super::*;

const ID: &str = "auto_height_test";

#[derive(Clone, Copy, Debug)]
enum Kind {
    AutoHeight,
    Fixed,
}

#[derive(Clone, Copy, Debug)]
enum Body {
    /// `auto_height_body`, as the Director's
    Helper(f32),
    /// a scroll area capped at the room with egui's own floor
    PlainScroll(f32),
    /// no scroll area: the floater scrolls it when it is too tall
    Plain(f32),
    /// Aurora Weather's real body
    Weather,
}

fn pass(ctx: &egui::Context, p: &crate::theme::Palette, screen: Vec2, kind: Kind, body: Body) -> Option<egui::Rect> {
    let mut open = true;
    let mut weather = crate::weather::WeatherSettings::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
            ..Default::default()
        },
        |ui| {
            let ctx = ui.ctx().clone();
            let room = (super::super::layout::work_area(&ctx).height() - 230.0).max(220.0);
            let width = match body {
                Body::Weather => super::super::weather_window::WINDOW_W,
                _ => 340.0,
            };
            let floater = Floater::new(ID, "Test", egui::pos2(100.0, 90.0), Vec2::new(width, 560.0));
            let floater = match kind {
                Kind::AutoHeight => floater.auto_height(),
                Kind::Fixed => floater.fixed(),
            };
            floater.show(&ctx, p, &mut open, |ui| match body {
                Body::Helper(content) => {
                    auto_height_body(room).show(ui, |ui| {
                        ui.allocate_exact_size(Vec2::new(300.0, content), Sense::hover());
                    });
                }
                Body::PlainScroll(content) => {
                    egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
                        ui.allocate_exact_size(Vec2::new(300.0, content), Sense::hover());
                    });
                }
                Body::Plain(content) => {
                    ui.allocate_exact_size(Vec2::new(300.0, content), Sense::hover());
                }
                Body::Weather => {
                    super::super::weather_window::body(ui, p, &mut weather);
                }
            });
        },
    );
    // no renderer takes the font atlas here
    output.textures_delta.clear();
    ctx.memory(|m| m.area_rect(egui::Id::unique(ID)))
}

/// The window's rect at rest on a large screen, then after a few frames on
/// tiny ones and back on the large one.
fn squeezed(kind: Kind, body: Body) -> (egui::Rect, egui::Rect) {
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let large = Vec2::new(1600.0, 900.0);
    let mut before = None;
    for _ in 0..10 {
        before = pass(&ctx, &p, large, kind, body);
    }
    for screen in [Vec2::new(160.0, 90.0), Vec2::new(1.0, 1.0)] {
        for _ in 0..10 {
            pass(&ctx, &p, screen, kind, body);
        }
    }
    let mut after = None;
    for _ in 0..10 {
        after = pass(&ctx, &p, large, kind, body);
    }
    (before.expect("shown"), after.expect("shown"))
}

fn assert_same(kind: Kind, body: Body) -> egui::Rect {
    let (before, after) = squeezed(kind, body);
    assert!(
        (after.width() - before.width()).abs() < 1.0 && (after.height() - before.height()).abs() < 1.0,
        "{kind:?} {body:?}: before {before:?}, after {after:?}"
    );
    before
}

#[test]
fn a_window_of_automatic_height_comes_back_after_a_tiny_viewer() {
    let before = assert_same(Kind::AutoHeight, Body::Helper(1200.0));
    // the body is as tall as the room (900 - 230), with the title above
    assert!(before.height() > 670.0, "{before:?}");
}

#[test]
fn a_short_body_keeps_its_own_height() {
    let before = assert_same(Kind::AutoHeight, Body::Helper(100.0));
    assert!(before.height() < 200.0, "{before:?}");
}

#[test]
fn a_scroll_area_of_its_own_comes_back() {
    for kind in [Kind::AutoHeight, Kind::Fixed] {
        assert_same(kind, Body::PlainScroll(1200.0));
        assert_same(kind, Body::PlainScroll(100.0));
    }
}

#[test]
fn a_plain_body_comes_back() {
    for kind in [Kind::AutoHeight, Kind::Fixed] {
        assert_same(kind, Body::Plain(500.0));
        // taller than the room: the floater scrolls it
        assert_same(kind, Body::Plain(1200.0));
    }
}

#[test]
fn aurora_weather_comes_back() {
    assert_same(Kind::AutoHeight, Body::Weather);
}
