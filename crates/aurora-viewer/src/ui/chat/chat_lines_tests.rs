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
