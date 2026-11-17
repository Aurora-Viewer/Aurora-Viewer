//! A press on a solid background area (the Director's timeline) is the
//! interface's, before egui's pass has seen it; on a plain background area
//! (the chat lines) or beside the area, it stays the world's; a window over
//! the area keeps it, as any window does.

use super::*;

const STRIP: egui::Rect = egui::Rect {
    min: egui::pos2(100.0, 300.0),
    max: egui::pos2(500.0, 340.0),
};

/// One pass: a background area over `STRIP` (solid or not, shown or not)
/// and, when asked, a window over its left end.
fn pass(ctx: &egui::Context, solid: bool, shown: bool, window: bool, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 400.0))),
            events,
            ..Default::default()
        },
        |ui| {
            let ctx = ui.ctx().clone();
            let id = egui::Id::unique("solid_test_strip");
            if shown {
                egui::Area::new(id)
                    .order(egui::Order::Background)
                    .fixed_pos(STRIP.min)
                    .show(&ctx, |ui| {
                        ui.allocate_exact_size(STRIP.size(), Sense::click());
                    });
                if solid {
                    solid_background(&ctx, id);
                }
            }
            if window {
                egui::Area::new(egui::Id::unique("solid_test_window"))
                    .order(egui::Order::Middle)
                    .fixed_pos(egui::pos2(80.0, 280.0))
                    .show(&ctx, |ui| {
                        ui.allocate_exact_size(Vec2::new(100.0, 100.0), Sense::click());
                    });
            }
        },
    );
    // no renderer takes the font atlas here
    output.textures_delta.clear();
}

/// After a few passes with the pointer in the open world, as when the user
/// moves to the strip and presses at once: egui has not seen it there yet.
fn settled(solid: bool, shown: bool, window: bool) -> egui::Context {
    let ctx = egui::Context::default();
    for _ in 0..4 {
        pass(
            &ctx,
            solid,
            shown,
            window,
            vec![egui::Event::PointerMoved(egui::pos2(300.0, 100.0))],
        );
    }
    ctx
}

#[test]
fn a_press_on_a_solid_background_area_is_the_interfaces() {
    let ctx = settled(true, true, false);
    let on_strip = egui::pos2(300.0, 320.0);
    assert!(!ctx.is_pointer_over_egui(), "egui still has the pointer in the world");
    assert!(pointer_over_ui(&ctx, on_strip), "the strip keeps its press");
    // beside it, the world's
    assert!(!pointer_over_ui(&ctx, egui::pos2(300.0, 200.0)));
    assert!(!pointer_over_ui(&ctx, egui::pos2(520.0, 320.0)));
}

#[test]
fn a_plain_background_area_still_leaves_its_press_to_the_world() {
    // egui's own rule, kept for the areas that are part of the world
    let ctx = settled(false, true, false);
    assert!(!pointer_over_ui(&ctx, egui::pos2(300.0, 320.0)));
}

#[test]
fn a_solid_area_no_longer_shown_leaves_its_room_to_the_world() {
    let ctx = settled(true, true, false);
    for _ in 0..3 {
        pass(&ctx, true, false, false, vec![]);
    }
    assert!(!pointer_over_ui(&ctx, egui::pos2(300.0, 320.0)));
}

#[test]
fn a_window_over_a_solid_area_stays_above_it() {
    let ctx = settled(true, true, true);
    // the window is the topmost area where it lies over the strip
    let over = egui::pos2(120.0, 320.0);
    assert!(ctx.memory(|m| m.layer_id_at(over)).is_some_and(|l| l.order == egui::Order::Middle));
    // the strip's part beside it keeps its press
    assert!(pointer_over_ui(&ctx, egui::pos2(300.0, 320.0)));
    // the window's own part, once egui has the pointer there: egui's rule
    pass(&ctx, true, true, true, vec![egui::Event::PointerMoved(over)]);
    assert!(pointer_over_ui(&ctx, over));
}
