//! The nearby chat lines over the chat bar keep their bottom on the bar in
//! the frame a line leaves or arrives (#2544): before, that frame drew them
//! a line's height too high (or too low), and the next put them back.

use std::time::{Duration, SystemTime};

use egui::{LayerId, Pos2, Rect, Vec2};
use uuid::Uuid;

use crate::ui::widgets::elide_tests::Frames;
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
    drawn_at(ctx, world, seconds, cards, frame, super::LinesPlace::above(BAR)).map(|r| r.bottom())
}

/// What one frame of the toasts at `place` painted: the bounds of their
/// shapes, each within its clip.
fn drawn_at(ctx: &egui::Context, world: &World, seconds: f32, cards: bool, frame: u32, place: super::LinesPlace) -> Option<Rect> {
    let p = crate::theme::Theme::default().palette();
    let pics = std::collections::HashMap::new();
    let layer = LayerId::new(egui::Order::Background, egui::Id::unique("chat_toasts"));
    let mut bottom = None;
    let mut emoji = crate::ui::emoji::Emoji::load();
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
            &mut emoji,
            world,
            &pics,
            &mut names,
            &mut wanted,
            place,
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
                .map(|s| s.shape.visual_bounding_rect().intersect(s.clip_rect))
                .filter(|r| r.is_positive())
                .fold(Rect::NOTHING, Rect::union);
            r.is_positive().then_some(r)
        });
    });
    out.textures_delta.clear();
    bottom
}

/// The lines follow the chat block wherever its bar is: above it, or under
/// it when there is more room there, unless a direction is asked for and
/// has room.
#[test]
fn the_lines_stand_by_the_chat_block_above_it_or_under_it() {
    use crate::settings::ChatLinesDirection::{Auto, Down, Up};
    use crate::ui::bars::chrome::ChatPlace;
    use crate::ui::toolbar::Bar;
    let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    // what the bars leave: under the top bar, over the bottom one
    let room = (52.0, BAR);
    let block = |x: f32, y: f32, w: f32| Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 28.0));

    // the dock's block: as before, over what the bottom bar keeps, from
    // the block's left
    let dock = ChatPlace::at(block(12.0, 760.0, 380.0), Bar::Bottom, false);
    let at = super::LinesPlace::by(dock, Auto, screen, room);
    assert!(!at.down);
    assert_eq!(at.edge, super::LinesPlace::above(BAR).edge, "the bottom the lines had");
    assert_eq!(at.column(screen, 470.0), (8.0, false), "the left they had");
    assert_eq!(at.ceiling, 52.0, "they stop under the top bar");
    // asked under a bar that has nothing under it: above all the same
    assert_eq!(super::LinesPlace::by(dock, Down, screen, room), at);
    let centred = ChatPlace::at(block(450.0, 760.0, 380.0), Bar::Bottom, false);
    assert_eq!(
        super::LinesPlace::by(centred, Auto, screen, room).column(screen, 470.0),
        (446.0, false)
    );

    // the top bar's block: hung under it, clear of what stands under the bar
    let top = ChatPlace::at(block(300.0, 8.0, 380.0), Bar::Top, false);
    let at = super::LinesPlace::by(top, Auto, screen, room);
    assert!(at.down);
    assert_eq!(at.edge, 52.0 + 6.0);
    assert_eq!(at.limit, BAR - 6.0, "down to the bottom bar");
    assert_eq!(at.column(screen, 470.0), (296.0, false));
    assert_eq!(super::LinesPlace::by(top, Up, screen, room), at, "no room above the top bar");
    // at the right end of the bar, no room from the block's left: the
    // lines end on its right, kept on the screen
    let right = ChatPlace::at(block(890.0, 8.0, 380.0), Bar::Top, false);
    assert_eq!(
        super::LinesPlace::by(right, Auto, screen, room).column(screen, 470.0),
        (1280.0 - 8.0 - 470.0, true)
    );
    let right = ChatPlace::at(block(840.0, 8.0, 380.0), Bar::Top, false);
    assert_eq!(
        super::LinesPlace::by(right, Auto, screen, room).column(screen, 470.0),
        (1224.0 - 470.0, true)
    );

    // a floating block: where there is more room, or as asked
    let high = ChatPlace::at(block(300.0, 200.0, 380.0), Bar::Top, true);
    let at = super::LinesPlace::by(high, Auto, screen, room);
    assert!(at.down);
    assert_eq!(at.edge, 232.0 + 8.0 + 6.0);
    let at = super::LinesPlace::by(high, Up, screen, room);
    assert!(!at.down, "a direction asked for is kept while it has room");
    assert_eq!(at.edge, 196.0 - 8.0 - 6.0);
    let low = ChatPlace::at(block(300.0, 500.0, 380.0), Bar::Top, true);
    assert!(!super::LinesPlace::by(low, Auto, screen, room).down);
    assert!(super::LinesPlace::by(low, Down, screen, room).down);

    // a side bar's block: above it or under it too, never beside it
    let left = ChatPlace::at(block(8.0, 600.0, 380.0), Bar::Left, false);
    let at = super::LinesPlace::by(left, Auto, screen, room);
    assert!(!at.down);
    assert_eq!(at.edge, 596.0 - 8.0 - 6.0);
    assert_eq!(at.column(screen, 470.0), (8.0, false));
    let side = ChatPlace::at(block(892.0, 100.0, 380.0), Bar::Right, false);
    let at = super::LinesPlace::by(side, Auto, screen, room);
    assert!(at.down);
    assert_eq!(at.edge, 132.0 + 8.0 + 6.0);
    assert_eq!(
        at.column(screen, 470.0),
        (1280.0 - 8.0 - 470.0, true),
        "they end on the block's right, kept on the screen"
    );
    assert!(
        super::LinesPlace::by(side, Up, screen, room).down,
        "no room for a line between the top bar and the block"
    );
    let lower = ChatPlace::at(block(892.0, 300.0, 380.0), Bar::Right, false);
    let at = super::LinesPlace::by(lower, Up, screen, room);
    assert!(!at.down, "asked above, with room there");
    assert_eq!(at.edge, 296.0 - 8.0 - 6.0);
    assert_eq!(at.ceiling, 52.0);
}

/// Lines hung under a chat bar start under it; when they reach the bottom
/// bar, the newest stay over it and the oldest go under the chat bar's
/// edge, cut there.
#[test]
fn lines_hung_under_the_chat_bar_start_there_and_stop_over_the_bottom_bar() {
    for cards in [true, false] {
        let ctx = egui::Context::default();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        say(&mut world, 1, "y'all still alive?", 0);
        say(&mut world, 2, "nope", 0);
        let place = |limit: f32| super::LinesPlace {
            left: Some(300.0),
            right: None,
            edge: 58.0,
            down: true,
            limit,
            ceiling: 0.0,
        };
        let draw = |world: &World, limit: f32| {
            let mut drawn = None;
            for n in 0..4 {
                drawn = drawn_at(&ctx, world, 200.0, cards, n, place(limit));
            }
            drawn.expect("lines")
        };
        let few = draw(&world, 734.0);
        assert!(
            (57.5..70.0).contains(&few.top()) && (299.0..310.0).contains(&few.left()),
            "cards {cards}: the lines start under the bar, at its left: {few:?}"
        );
        assert!(few.bottom() < 200.0, "cards {cards}: two lines: {few:?}");
        // by a bar of the right side: every line ends on the edge given
        let ended = super::LinesPlace {
            left: None,
            right: Some(900.0),
            ..place(734.0)
        };
        let mut drawn = None;
        for n in 0..4 {
            drawn = drawn_at(&ctx, &world, 200.0, cards, n, ended);
        }
        let drawn = drawn.expect("lines");
        assert!(
            (895.0..=900.5).contains(&drawn.right()) && drawn.left() > 430.0,
            "cards {cards}: the lines end on the bar's side: {drawn:?}"
        );
        assert!(
            (few.height() - drawn.height()).abs() < 0.5 && (few.width() - drawn.width()).abs() < 1.5,
            "cards {cards}: aligned, the lines keep their size: {few:?}, {drawn:?}"
        );
        for who in 10..35 {
            say(&mut world, who, "one more line of the nearby chat", 0);
        }
        // going up from a bar, the same lines stop under the top bar: the
        // newest stay on the chat bar, the oldest are cut
        let up = super::LinesPlace {
            edge: 700.0,
            down: false,
            ceiling: 420.0,
            ..place(734.0)
        };
        let mut drawn = None;
        for n in 0..4 {
            drawn = drawn_at(&ctx, &world, 200.0, cards, n, up);
        }
        let drawn = drawn.expect("lines");
        assert!(
            drawn.top() >= 419.5 && drawn.top() < 440.0,
            "cards {cards}: the oldest lines are cut under the top bar: {drawn:?}"
        );
        assert!(
            (680.0..=700.5).contains(&drawn.bottom()),
            "cards {cards}: the newest line stays on the chat bar: {drawn:?}"
        );
        let many = draw(&world, 400.0);
        assert!(
            many.top() >= 57.5,
            "cards {cards}: the oldest lines are cut under the bar: {many:?}"
        );
        assert!(
            (380.0..=400.5).contains(&many.bottom()),
            "cards {cards}: the newest line stays over the bottom bar: {many:?}"
        );
    }
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
    let mut emoji = crate::ui::emoji::Emoji::load();
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
            &mut emoji,
            world,
            &pics,
            &mut names,
            &mut wanted,
            super::LinesPlace::above(BAR),
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
    let mut emoji = crate::ui::emoji::Emoji::load();
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
            &mut emoji,
            world,
            &pics,
            &mut names,
            &mut wanted,
            super::LinesPlace::above(BAR),
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

/// One frame of the toasts, with a line holding `🫶`: the pictures it
/// painted for the emoji.
fn toast_emoji_pictures(cards: bool) -> usize {
    let mut frames = Frames::new();
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    say(&mut world, 1, "Emoji 🫶 here", 0);
    let p = crate::theme::Theme::default().palette();
    let pics = std::collections::HashMap::new();
    let mut emoji = crate::ui::emoji::Emoji::load();
    let mut draw = |frames: &mut Frames| {
        frames.shapes(Vec::new(), |ui| {
            let (mut names, mut wanted) = Default::default();
            super::toasts(
                ui.ctx(),
                &p,
                &mut emoji,
                &world,
                &pics,
                &mut names,
                &mut wanted,
                super::LinesPlace::above(BAR),
                200.0,
                40,
                true,
                false,
                cards,
                false,
                true,
            );
        })
    };
    // two passes: a picture's texture is only ready for the second one
    draw(&mut frames);
    let shapes = draw(&mut frames);
    fn textured(shape: &egui::Shape) -> bool {
        match shape {
            egui::epaint::Shape::Mesh(m) => m.texture_id != egui::TextureId::default(),
            egui::epaint::Shape::Rect(r) => r.brush.as_ref().is_some_and(|b| b.fill_texture_id != egui::TextureId::default()),
            _ => false,
        }
    }
    shapes.iter().filter(|s| textured(s)).count()
}

#[test]
fn a_toast_draws_its_emoji_as_pictures_in_both_styles() {
    for cards in [true, false] {
        assert_eq!(toast_emoji_pictures(cards), 1, "cards {cards}: one picture for the heart hands");
    }
}
