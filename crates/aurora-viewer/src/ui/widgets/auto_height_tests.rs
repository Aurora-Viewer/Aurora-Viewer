//! A window of automatic height whose body scrolls (the Director's,
//! `auto_height_body`) comes back to its size once the viewer's window, made
//! tiny, is made large again; a short body keeps its own height.

use super::*;

const ID: &str = "auto_height_test";

fn pass(ctx: &egui::Context, p: &crate::theme::Palette, screen: Vec2, content: f32) -> Option<egui::Rect> {
    let mut open = true;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
            ..Default::default()
        },
        |ui| {
            let ctx = ui.ctx().clone();
            let room = (super::super::layout::work_area(&ctx).height() - 230.0).max(220.0);
            Floater::new(ID, "Test", egui::pos2(900.0, 90.0), Vec2::new(340.0, 560.0))
                .auto_height()
                .show(&ctx, p, &mut open, |ui| {
                    auto_height_body(room).show(ui, |ui| {
                        ui.allocate_exact_size(Vec2::new(300.0, content), Sense::hover());
                    });
                });
        },
    );
    ctx.memory(|m| m.area_rect(egui::Id::unique(ID)))
}

/// The window's rect at rest on a large screen, then after a few frames on
/// tiny ones and back on the large one.
fn squeezed(content: f32) -> (egui::Rect, egui::Rect) {
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let large = Vec2::new(1600.0, 900.0);
    let mut before = None;
    for _ in 0..10 {
        before = pass(&ctx, &p, large, content);
    }
    for screen in [Vec2::new(160.0, 90.0), Vec2::new(1.0, 1.0)] {
        for _ in 0..10 {
            pass(&ctx, &p, screen, content);
        }
    }
    let mut after = None;
    for _ in 0..10 {
        after = pass(&ctx, &p, large, content);
    }
    (before.expect("shown"), after.expect("shown"))
}

#[test]
fn a_window_of_automatic_height_comes_back_after_a_tiny_viewer() {
    let (before, after) = squeezed(1200.0);
    // the body is as tall as the room (900 - 230), with the title above
    assert!(before.height() > 670.0, "{before:?}");
    assert!(
        (after.width() - before.width()).abs() < 1.0 && (after.height() - before.height()).abs() < 1.0,
        "before {before:?}, after {after:?}"
    );
}

#[test]
fn a_short_body_keeps_its_own_height() {
    let (before, after) = squeezed(100.0);
    assert!(before.height() < 200.0, "{before:?}");
    assert!((after.height() - before.height()).abs() < 1.0, "before {before:?}, after {after:?}");
}
