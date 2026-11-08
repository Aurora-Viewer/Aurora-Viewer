//! The nearby chat lines over the chat bar keep their bottom on the bar in
//! the frame a line leaves or arrives (#2544): before, that frame drew them
//! a line's height too high (or too low), and the next put them back.

use std::time::{Duration, SystemTime};

use egui::{LayerId, Pos2, Rect, Vec2};
use uuid::Uuid;

use crate::world::{ChatKind, ChatLine, World};
use aurora_net::ChatType;

const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);
/// The top of the chat bar.
const BAR: f32 = 740.0;

/// A resident's line said `age` ago, added to the nearby chat.
fn say(world: &mut World, who: u128, text: &str, age: u64) {
    world.system_message(text);
    let line: &mut ChatLine = world.chat.back_mut().expect("the line just added");
    line.time = SystemTime::now() - Duration::from_secs(age);
    line.from = format!("Resident {who}");
    line.source = Uuid::from_u128(who);
    line.kind = ChatKind::Local(ChatType::Normal);
}

/// What one frame of the toasts painted: the bottom of their shapes.
fn frame(ctx: &egui::Context, world: &World, seconds: f32, cards: bool, frame: u32) -> Option<f32> {
    let p = crate::theme::Theme::default().palette();
    let pics = std::collections::HashMap::new();
    let layer = LayerId::new(egui::Order::Background, egui::Id::unique("chat_toasts"));
    let mut bottom = None;
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
        time: Some(0.1 * f64::from(frame)),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        let (mut names, mut wanted) = Default::default();
        super::toasts(
            ui.ctx(),
            &p,
            world,
            &pics,
            &mut names,
            &mut wanted,
            BAR,
            seconds,
            // ConsoleMaxLines default (`settings::chat_console_max_lines`).
            40,
            true,
            false,
            cards,
            false,
            true,
        );
        bottom = ui.ctx().graphics(|g| {
            let shapes = g.get(layer)?;
            let r = shapes
                .all_entries()
                .map(|s| s.shape.visual_bounding_rect())
                .fold(Rect::NOTHING, Rect::union);
            r.is_positive().then_some(r.bottom())
        });
    });
    out.textures_delta.clear();
    bottom
}

/// The bottom of the lines frame by frame while `change` happens between
/// the fourth and fifth frame, and once they have settled.
fn bottoms(cards: bool, change: impl FnOnce(&mut World, &mut f32)) -> (f32, f32, f32) {
    let ctx = egui::Context::default();
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    // the oldest line is 100 s old: shown with a lifetime of 200 s, gone
    // with one of 60 s
    say(&mut world, 1, "y'all still alive?", 100);
    say(&mut world, 2, "nope", 0);
    say(
        &mut world,
        3,
        "Computer says no, a line long enough to wrap onto a second row of the card",
        0,
    );
    let mut seconds = 200.0;
    let mut before = None;
    for n in 0..4 {
        before = frame(&ctx, &world, seconds, cards, n);
    }
    change(&mut world, &mut seconds);
    let during = frame(&ctx, &world, seconds, cards, 4);
    let mut after = None;
    for n in 5..8 {
        after = frame(&ctx, &world, seconds, cards, n);
    }
    (
        before.expect("lines before"),
        during.expect("lines in the frame of the change"),
        after.expect("lines after"),
    )
}

fn assert_held(case: &str, (before, during, after): (f32, f32, f32)) {
    assert!(
        (before - after).abs() < 0.5,
        "{case}: the lines settled elsewhere: {before} then {after}"
    );
    assert!(
        (during - after).abs() < 0.5,
        "{case}: the frame of the change drew the lines' bottom at {during}, not on the bar at {after}"
    );
}

#[test]
fn a_line_that_leaves_does_not_move_the_others_for_a_frame() {
    for cards in [true, false] {
        let held = bottoms(cards, |_, seconds| *seconds = 60.0);
        assert_held(&format!("cards {cards}, a line leaves"), held);
    }
}

#[test]
fn a_line_that_arrives_does_not_hang_below_the_bar_for_a_frame() {
    for cards in [true, false] {
        let held = bottoms(cards, |world, _| say(world, 4, "a new line, from someone else", 0));
        assert_held(&format!("cards {cards}, a line arrives"), held);
    }
}

/// How many shapes one frame of the toasts painted.
fn painted(ctx: &egui::Context, world: &World, cards: bool, frame: u32) -> usize {
    let p = crate::theme::Theme::default().palette();
    let pics = std::collections::HashMap::new();
    let layer = LayerId::new(egui::Order::Background, egui::Id::unique("chat_toasts"));
    let mut count = 0;
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
        time: Some(0.1 * f64::from(frame)),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        let (mut names, mut wanted) = Default::default();
        super::toasts(
            ui.ctx(),
            &p,
            world,
            &pics,
            &mut names,
            &mut wanted,
            BAR,
            200.0,
            40,
            true,
            false,
            cards,
            false,
            true,
        );
        count = ui.ctx().graphics(|g| g.get(layer).map_or(0, |l| l.all_entries().count()));
    });
    out.textures_delta.clear();
    count
}

#[test]
fn lines_that_arrive_together_are_drawn_whole_in_their_first_frame() {
    for cards in [true, false] {
        let ctx = egui::Context::default();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        say(&mut world, 1, "y'all still alive?", 10);
        for n in 0..4 {
            painted(&ctx, &world, cards, n);
        }
        // more than the room under the bar: laid out below the screen
        // before they are moved up onto the bar
        for who in 2..8 {
            say(&mut world, who, "a burst of lines from everyone at once", 0);
        }
        let first = painted(&ctx, &world, cards, 4);
        let settled = painted(&ctx, &world, cards, 5);
        assert_eq!(first, settled, "cards {cards}: the first frame of the burst lost shapes");
    }
}

/// One frame of the toasts at `ppp` points per pixel: the bounds of what
/// they painted, and the rect the area keeps for the others to avoid (what
/// `notification_center::chat_lines` reads for the friends' toasts, later
/// in the same frame).
fn bounds(ctx: &egui::Context, world: &World, seconds: f32, cards: bool, frame: u32) -> (Rect, Rect) {
    let p = crate::theme::Theme::default().palette();
    let pics = std::collections::HashMap::new();
    let layer = LayerId::new(egui::Order::Background, egui::Id::unique("chat_toasts"));
    let mut got = (Rect::NOTHING, Rect::NOTHING);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
        time: Some(0.1 * f64::from(frame)),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        let (mut names, mut wanted) = Default::default();
        super::toasts(
            ui.ctx(),
            &p,
            world,
            &pics,
            &mut names,
            &mut wanted,
            BAR,
            seconds,
            // ConsoleMaxLines default (`settings::chat_console_max_lines`).
            40,
            true,
            false,
            cards,
            false,
            true,
        );
        let painted = ui.ctx().graphics(|g| {
            g.get(layer).map_or(Rect::NOTHING, |l| {
                l.all_entries()
                    .map(|s| s.shape.visual_bounding_rect())
                    .fold(Rect::NOTHING, Rect::union)
            })
        });
        let kept = ui.ctx().memory(|m| m.area_rect(layer.id)).unwrap_or(Rect::NOTHING);
        got = (painted, kept);
    });
    out.textures_delta.clear();
    got
}

/// The lines before a change, in the frame of the change, and settled.
fn around(ppp: f32, cards: bool, change: impl FnOnce(&mut World, &mut f32)) -> [(Rect, Rect); 3] {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    say(&mut world, 1, "y'all still alive?", 100);
    say(&mut world, 2, "nope", 0);
    say(
        &mut world,
        3,
        "Computer says no, a line long enough to wrap onto a second row of the card",
        0,
    );
    let mut seconds = 200.0;
    let mut before = (Rect::NOTHING, Rect::NOTHING);
    for n in 0..4 {
        before = bounds(&ctx, &world, seconds, cards, n);
    }
    change(&mut world, &mut seconds);
    let during = bounds(&ctx, &world, seconds, cards, 4);
    let mut after = during;
    for n in 5..8 {
        after = bounds(&ctx, &world, seconds, cards, n);
    }
    [before, during, after]
}

#[test]
fn the_frame_of_a_change_matches_the_settled_frame_to_the_pixel() {
    for ppp in [1.0, 1.25, 1.5, 1.75, 2.0] {
        for cards in [true, false] {
            for leave in [true, false] {
                let [_, (during, _), (after, _)] = around(ppp, cards, |world, seconds| {
                    if leave {
                        *seconds = 60.0;
                    } else {
                        say(world, 4, "a new line, from someone else", 0);
                    }
                });
                assert!(
                    (during.top() - after.top()).abs() < 0.01 && (during.bottom() - after.bottom()).abs() < 0.01,
                    "ppp {ppp}, cards {cards}, leave {leave}: top and bottom {:.3}, {:.3} in the frame of the change, {:.3}, {:.3} settled",
                    during.top(),
                    during.bottom(),
                    after.top(),
                    after.bottom()
                );
            }
        }
    }
}

#[test]
fn the_rect_kept_for_the_friends_toasts_is_where_the_lines_are_drawn() {
    for cards in [true, false] {
        for leave in [true, false] {
            let [_, (_, during), (_, after)] = around(1.0, cards, |world, seconds| {
                if leave {
                    *seconds = 60.0;
                } else {
                    say(world, 4, "a new line, from someone else", 0);
                }
            });
            assert!(
                (during.top() - after.top()).abs() < 0.01 && (during.bottom() - after.bottom()).abs() < 0.01,
                "cards {cards}, leave {leave}: kept {during:?} in the frame of the change, {after:?} settled"
            );
        }
    }
}
