//! Which windows may be sent outside the viewer's, and their size there.

use super::*;

fn window(id: &str) -> Floater<'_> {
    Floater::new(id, "Window", egui::pos2(20.0, 20.0), Vec2::new(400.0, 300.0))
}

#[test]
fn every_plain_window_may_go_outside_but_the_dev_toolbox() {
    assert!(window("inventory").goes_outside());
    assert!(window("conversations").chat().steady_size().attention(true).goes_outside());
    assert!(window("note_1234").compact_header().help("Help").goes_outside());
    // an inspector's framed header, its states and its summary are drawn outside
    let mut inspector = window("build_inspector").framed_header("tool-filled");
    inspector
        .header_status
        .push(("circle-filled", "State".to_owned(), Color32::TRANSPARENT));
    inspector.header_summary = Some("1 object".into());
    assert!(inspector.goes_outside());
    // a window sized by its content: its window outside follows it
    assert!(window("profile").fixed().goes_outside());
    assert!(window("weather").auto_height().goes_outside());
    // its tools read the world under the viewer's pointer
    assert!(!window(crate::ui::dev_toolbox::WINDOW).goes_outside());
}

#[test]
fn a_window_whose_sizing_is_not_followed_outside_stays_inside() {
    let mut tall = window("e");
    tall.height_only = true;
    assert!(!tall.goes_outside());
    // a picture of a kept shape keeps it in its window outside, a compact
    // view is drawn there too, and a window over an overlay of the
    // viewer's is a window of its own there
    assert!(window("g").keep_aspect(1.5, Vec2::ZERO).goes_outside());
    assert!(window("d").compact_when_minimized(240.0).goes_outside());
    let mut over = window("f");
    over.order = egui::Order::Foreground;
    assert!(over.goes_outside());
}

/// A window with a compact view keeps it when it goes outside, and has
/// the button that changes the view there: its body draws from the state.
#[test]
fn a_compact_view_is_kept_and_changed_outside() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside compact";
    let window = egui::Id::unique(name);
    let ctx = frames.ctx.clone();
    ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((name, "minimized")), true));
    crate::outside::want(&ctx, window, true);
    crate::outside::set_native(&ctx, window, true);
    let mut seen_compact = None;
    let mut open = true;
    frames.run(Vec::new(), |ui| {
        Floater::new(name, "Controls", egui::pos2(10.0, 10.0), Vec2::new(300.0, 200.0))
            .silent()
            .fixed()
            .auto_height()
            .compact_when_minimized(140.0)
            .show(ui.ctx(), &p, &mut open, |ui| {
                seen_compact = Some(floater_minimized(ui.ctx(), name));
            });
    });
    assert_eq!(seen_compact, Some(true));
    // the size asked of its window outside is the compact view's width
    let asked = crate::outside::resizes(&ctx);
    assert!(
        asked.iter().any(|(id, size, held)| *id == window && size.x == 140.0 && *held),
        "{asked:?}"
    );
}

/// An expand asked of a window outside in its compact view (its toolbar
/// button, a tab that shows in its full form) is taken there as inside:
/// the full view shows, and the request does not stay pending, which the
/// Controls window reads to tell its title button from a request.
#[test]
fn an_expand_asked_of_a_compact_window_outside_is_taken_there() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside expand";
    let window = egui::Id::unique(name);
    let ctx = frames.ctx.clone();
    ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((name, "minimized")), true));
    crate::outside::want(&ctx, window, true);
    crate::outside::set_native(&ctx, window, true);
    let draw = |frames: &mut elide_tests::Frames| {
        let mut open = true;
        let mut compact = None;
        frames.run(Vec::new(), |ui| {
            Floater::new(name, "Controls", egui::pos2(10.0, 10.0), Vec2::new(300.0, 200.0))
                .silent()
                .fixed()
                .auto_height()
                .compact_when_minimized(140.0)
                .show(ui.ctx(), &p, &mut open, |ui| compact = Some(floater_minimized(ui.ctx(), name)));
        });
        compact
    };
    assert_eq!(draw(&mut frames), Some(true));
    expand_floater(&ctx, name);
    assert!(floater_expand_pending(&ctx, name));
    assert_eq!(draw(&mut frames), Some(false));
    assert!(!floater_expand_pending(&ctx, name));
    assert!(!floater_minimized(&ctx, name));
}

/// Hiding the interface (mouselook, immersive mode, "Hide the interface")
/// leaves a window outside up: it covers nothing of the world. Brought
/// back inside meanwhile, it is hidden with the others and comes back with
/// the interface.
#[test]
fn a_window_outside_stays_up_while_the_interface_is_hidden() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside hidden";
    let window = egui::Id::unique(name);
    let ctx = frames.ctx.clone();
    crate::outside::want(&ctx, window, true);
    crate::outside::set_native(&ctx, window, true);
    let mut open = true;
    // one frame: whether the window's body ran, and whether it drew outside
    let mut frame = |open: &mut bool| {
        let mut drawn = false;
        frames.run(Vec::new(), |ui| {
            Floater::new(name, "Window", egui::pos2(10.0, 10.0), Vec2::new(300.0, 200.0))
                .silent()
                .show(ui.ctx(), &p, open, |_| drawn = true);
        });
        let outside = crate::outside::seen_now(&frames.ctx).contains(&window);
        frames
            .ctx
            .data_mut(|d| d.remove_temp::<Vec<egui::Id>>(egui::Id::unique("aurora outside seen")));
        (drawn, outside)
    };
    assert_eq!(frame(&mut open), (true, true));
    hide_windows(&ctx, true, HideKeep::None);
    assert_eq!(frame(&mut open), (true, true), "the frame the interface is hidden");
    assert_eq!(frame(&mut open), (true, true), "and after");
    // brought back inside with the interface still hidden
    crate::outside::want(&ctx, window, false);
    assert_eq!(frame(&mut open), (false, false), "hidden with the windows inside");
    assert!(open);
    hide_windows(&ctx, false, HideKeep::None);
    assert_eq!(frame(&mut open), (true, false), "back with the interface, inside");
    // a window inside is hidden as before
    hide_windows(&ctx, true, HideKeep::None);
    assert_eq!(frame(&mut open), (false, false));
}

/// A close asked of a window whose code does not run (the interface's
/// windows are not drawn: `outside::keep`) waits for it, where a request
/// is otherwise dropped after a frame.
#[test]
fn a_close_asked_of_a_window_left_as_it_is_waits_for_it() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside close waits";
    let window = egui::Id::unique(name);
    let ctx = frames.ctx.clone();
    crate::outside::want(&ctx, window, true);
    crate::outside::set_native(&ctx, window, true);
    let mut open = true;
    let show = |frames: &mut elide_tests::Frames, open: &mut bool| {
        frames.run(Vec::new(), |ui| {
            Floater::new(name, "Window", egui::pos2(10.0, 10.0), Vec2::new(300.0, 200.0))
                .silent()
                .show(ui.ctx(), &p, open, |_| {});
        });
    };
    show(&mut frames, &mut open);
    // its cross, then frames that draw no window; nothing was asked of
    // another window meanwhile
    close_window(&ctx, window);
    for _ in 0..5 {
        frames.run(Vec::new(), |_| {});
        assert!(keep_close_asked(&ctx, window), "one waits: its window outside goes at once");
        assert!(!keep_close_asked(&ctx, egui::Id::unique("another window")));
    }
    assert!(!close_asked(&ctx, "another window"));
    show(&mut frames, &mut open);
    assert!(!open, "closed when its code ran again");
    // without it, the request is dropped
    open = true;
    show(&mut frames, &mut open);
    close_window(&ctx, window);
    for _ in 0..5 {
        frames.run(Vec::new(), |_| {});
    }
    show(&mut frames, &mut open);
    assert!(open);
}

/// An inspector outside has no title row of its own (the system's bar
/// names it): its buttons end the row of its states, instead of a dark
/// band that holds nothing else. Its summary keeps its room, blank or not,
/// so the tools' bar does not move between tools, as inside.
#[test]
fn an_inspector_outside_has_its_buttons_on_the_row_of_its_states() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside inspector";
    let window = egui::Id::unique(name);
    crate::outside::want(&frames.ctx, window, true);
    crate::outside::set_native(&frames.ctx, window, true);
    // where the window's body starts, under its header
    let body_top = |frames: &mut elide_tests::Frames, summary: &str, states: bool| {
        let (mut open, mut copy) = (true, None);
        let mut top = 0.0;
        frames.run(Vec::new(), |ui| {
            let mut floater = Floater::new(name, "Build", egui::pos2(10.0, 10.0), Vec2::new(360.0, 400.0))
                .silent()
                .help("Help")
                .framed_header("tool-filled");
            if states {
                floater
                    .header_status
                    .push(("circle-filled", "State".to_owned(), Color32::TRANSPARENT));
                floater.header_status_action = Some(("clipboard-text-filled", "UUID", "Copy", &mut copy));
            }
            floater.header_summary = Some(summary.into());
            floater.show(ui.ctx(), &p, &mut open, |ui| top = ui.cursor().top());
        });
        top
    };
    // the header is as tall with the states as with the buttons alone
    let alone = body_top(&mut frames, " ", false);
    let with_states = body_top(&mut frames, " ", true);
    assert!(alone > 0.0);
    assert_eq!(with_states, alone);
    // a summary takes the room the blank one keeps
    assert_eq!(body_top(&mut frames, "1 object", true), alone);
}

#[test]
fn a_window_outside_is_read_and_sized_at_its_size_there() {
    let ctx = egui::Context::default();
    let id = egui::Id::unique("inventory");
    assert_eq!(floater_size(&ctx, "inventory"), None);
    // wished outside, its window not opened yet: still the one inside
    crate::outside::want(&ctx, id, true);
    ctx.data_mut(|d| d.insert_persisted(id.with(OUTSIDE_SIZE), Vec2::new(900.0, 700.0)));
    assert_eq!(floater_size(&ctx, "inventory"), None);
    crate::outside::set_native(&ctx, id, true);
    assert_eq!(floater_size(&ctx, "inventory"), Some(Vec2::new(900.0, 700.0)));
    // sized by its name, it is the window of the system that is asked
    size_floater(&ctx, "inventory", Vec2::new(1180.0, 700.0));
    assert_eq!(crate::outside::resizes(&ctx), [(id, Vec2::new(1180.0, 700.0), false)]);
    assert!(ctx.data_mut(|d| d.get_persisted::<Vec2>(id.with("expanded_size"))).is_none());
    // back inside, the window inside is sized as before
    crate::outside::want(&ctx, id, false);
    size_floater(&ctx, "inventory", Vec2::new(500.0, 400.0));
    assert_eq!(
        ctx.data_mut(|d| d.get_persisted::<Vec2>(id.with("expanded_size"))),
        Some(Vec2::new(500.0, 400.0))
    );
}

#[test]
fn a_window_outside_takes_the_size_of_its_content_once_per_size() {
    let (size, content) = (Vec2::new(350.0, 400.0), Vec2::new(350.0, 610.0));
    assert!(follows_content(size, content, None));
    // asked already: the system gave less (a small screen), or nothing
    assert!(!follows_content(size, content, Some(content)));
    // the content changed since
    assert!(follows_content(size, content, Some(Vec2::new(350.0, 500.0))));
    // as large as its content, to the pixel the system rounds to
    assert!(!follows_content(Vec2::new(350.4, 609.6), content, None));
}

/// A window outside that wants room at its right asks the system for a
/// wider window, once, and keeps its own width while the system has not
/// answered: its content does not jump to the width of both.
#[test]
fn a_window_outside_asks_for_the_room_at_its_right_and_keeps_its_width() {
    assert_eq!(side_room(None, 440.0, 0.0, 300.0, 1.0, 0.0), (None, None));
    // the room is wanted: asked once, the window still 440 wide
    let (side, ask) = side_room(None, 440.0, 356.0, 300.0, 1.0, 1.0);
    assert_eq!(ask, Some(796.0));
    let side = side.unwrap();
    assert_eq!((side.own, side.room, side.waits()), (440.0, 356.0, true));
    let (waiting, ask) = side_room(Some(side), 440.0, 356.0, 300.0, 1.0, 1.1);
    assert_eq!((waiting, ask), (Some(side), None));
    // the system gave it: the window keeps its 440
    let (given, ask) = side_room(waiting, 796.0, 356.0, 300.0, 1.0, 1.2);
    let given = given.unwrap();
    assert_eq!((given.own, given.back, given.asked, ask), (440.0, 440.0, None, None));
    // the user widens the window by its edge: the room keeps its width,
    // and that width is the one to go back to
    let (wider, ask) = side_room(Some(given), 900.0, 356.0, 300.0, 1.0, 3.0);
    let wider = wider.unwrap();
    assert_eq!((wider.own, wider.back, ask), (544.0, 544.0, None));
}

/// The room is given back when nothing wants it any more, and the window
/// keeps its own width until the system has taken it.
#[test]
fn a_window_outside_gives_the_room_back() {
    let (open, _) = side_room(None, 440.0, 356.0, 300.0, 1.0, 1.0);
    let (open, _) = side_room(open, 796.0, 356.0, 300.0, 1.0, 1.1);
    let (closing, ask) = side_room(open, 796.0, 0.0, 300.0, 1.0, 5.0);
    assert_eq!(ask, Some(440.0));
    let closing = closing.unwrap();
    assert_eq!((closing.own, closing.room), (440.0, 0.0));
    // still as wide for a frame: the window does not spread over the room
    assert_eq!(
        side_room(Some(closing), 796.0, 0.0, 300.0, 1.0, 5.02).0.map(|side| side.own),
        Some(440.0)
    );
    // taken back: nothing is kept
    assert_eq!(side_room(Some(closing), 440.0, 0.0, 300.0, 1.0, 5.05), (None, None));
    // a system that never takes it: nothing is kept past the wait either
    assert_eq!(side_room(Some(closing), 796.0, 0.0, 300.0, 1.0, 5.1 + SIDE_WAIT), (None, None));
}

/// A system that gives less than asked (a small screen): the window takes
/// what is left beside the room, at once, down to its least width, and
/// goes back to the width it had when the room goes.
#[test]
fn a_window_outside_makes_do_with_the_width_the_system_gives() {
    let (side, _) = side_room(None, 440.0, 356.0, 300.0, 1.0, 1.0);
    let (side, ask) = side_room(side, 700.0, 356.0, 300.0, 1.0, 1.05);
    let less = side.unwrap();
    assert_eq!((less.own, less.back, less.waits(), ask), (344.0, 440.0, false, None));
    // the answer came in two steps: the second is not the user's hand
    let (side, _) = side_room(Some(less), 720.0, 356.0, 300.0, 1.0, 1.1);
    assert_eq!((side.unwrap().own, side.unwrap().back), (364.0, 440.0));
    // past the wait, a new width is the user's
    let (side, _) = side_room(side, 740.0, 356.0, 300.0, 1.0, 2.0 + SIDE_WAIT);
    assert_eq!((side.unwrap().own, side.unwrap().back), (384.0, 384.0));
    // the room goes: the width it had
    let (_, ask) = side_room(Some(less), 700.0, 0.0, 300.0, 1.0, 9.0);
    assert_eq!(ask, Some(440.0));
}

/// A system that gives no room (a tiled window), or too little for the
/// window's least width: past the wait the window keeps the whole width,
/// and the panel is drawn over its right end, as inside.
#[test]
fn a_window_outside_without_room_keeps_its_width_under_the_panel() {
    let whole = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(440.0, 600.0));
    let (side, _) = side_room(None, 440.0, 356.0, 300.0, 1.0, 1.0);
    let waiting = side.unwrap();
    assert!(side_rect(whole, waiting).is_none(), "not while the system may still answer");
    let (side, ask) = side_room(side, 440.0, 356.0, 300.0, 1.0, 1.1 + SIDE_WAIT);
    let over = side.unwrap();
    assert_eq!((over.own, over.back, over.waits(), ask), (440.0, 440.0, false, None));
    assert_eq!(
        side_rect(whole, over),
        Some(egui::Rect::from_min_max(egui::pos2(92.0, 8.0), egui::pos2(432.0, 592.0)))
    );
    // a system's window narrower than the panel: the panel takes its width
    let narrow = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(300.0, 600.0));
    assert_eq!(side_rect(narrow, over).map(|rect| rect.width()), Some(284.0));
    // too little: 100 left beside the room, under the least width of 300
    let (side, _) = side_room(None, 440.0, 356.0, 300.0, 1.0, 1.0);
    let (side, _) = side_room(side, 456.0, 356.0, 300.0, 1.0, 1.05);
    assert_eq!(side.unwrap().own, 456.0);
    // the room goes: the window is as wide as it was, nothing to wait for
    assert_eq!(side_room(Some(over), 440.0, 0.0, 300.0, 1.0, 9.0), (None, Some(440.0)));
    // where the room is there, the panel is beside the window
    let (side, _) = side_room(Some(waiting), 796.0, 356.0, 300.0, 1.0, 1.05);
    let wide = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(796.0, 600.0));
    assert_eq!(
        side_rect(wide, side.unwrap()),
        Some(egui::Rect::from_min_max(egui::pos2(448.0, 8.0), egui::pos2(788.0, 592.0)))
    );
}

/// At another zoom the same window is counted in other points: the room,
/// as wide in points as before, is asked again beside the window's width
/// there, and the window is not narrowed by what the room gained.
#[test]
fn the_room_of_a_window_outside_is_asked_again_at_another_zoom() {
    let (side, _) = side_room(None, 440.0, 356.0, 300.0, 1.0, 1.0);
    let (side, _) = side_room(side, 796.0, 356.0, 300.0, 1.0, 1.1);
    // zoom 1 to 1.25: 796 points are 636.8, the window's 440 are 352
    let (side, ask) = side_room(side, 636.8, 356.0, 300.0, 1.25, 4.0);
    let zoomed = side.unwrap();
    assert_eq!(ask, Some(708.0));
    assert_eq!((zoomed.own, zoomed.back, zoomed.zoom), (352.0, 352.0, 1.25));
    let (side, ask) = side_room(Some(zoomed), 708.0, 356.0, 300.0, 1.25, 4.1);
    assert_eq!((side.unwrap().own, side.unwrap().back, ask), (352.0, 352.0, None));
    // a window under its least width there is given that width
    let (_, ask) = side_room(Some(zoomed), 318.4, 356.0, 300.0, 2.5, 6.0);
    assert_eq!(ask, Some(656.0));
}

/// Drawn: the window keeps the width it had beside the room it asks for,
/// its size is read without the room, the room is told to its pass only
/// once the system has answered, and its content keeps its ids (its
/// scroll places, its focus) whether the room is there or not. A size
/// asked of the window by its name is asked with the room beside it.
#[test]
fn a_window_outside_draws_at_its_own_width_beside_its_room() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside side";
    let id = egui::Id::unique(name);
    crate::outside::want(&frames.ctx, id, true);
    crate::outside::set_native(&frames.ctx, id, true);
    let draw = |frames: &mut elide_tests::Frames, side: f32| {
        let mut open = true;
        let mut seen = None;
        frames.run(Vec::new(), |ui| {
            let mut floater = Floater::new(name, "Build", egui::pos2(10.0, 10.0), Vec2::new(360.0, 400.0)).silent();
            floater.side_outside = side;
            floater.show(ui.ctx(), &p, &mut open, |ui| {
                seen = Some((ui.max_rect(), side_room_outside(ui.ctx(), name), ui.make_persistent_id("list")));
            });
        });
        seen.unwrap()
    };
    // (this screen never answers a size asked: the wait runs out)
    let wait = |frames: &mut elide_tests::Frames, side: f32| {
        let mut last = draw(frames, side);
        for _ in 0..(SIDE_WAIT * 60.0) as usize + 2 {
            last = draw(frames, side);
        }
        last
    };
    let (alone, room, list) = draw(&mut frames, 0.0);
    assert!(alone.width() > 0.0 && room.is_none());
    let size = floater_size(&frames.ctx, name).unwrap();
    // (the system's window: the content and its margins, the same on both sides)
    let right = alone.center().x + size.x / 2.0;
    assert!(crate::outside::take_resizes(&frames.ctx).is_empty());
    // a narrow panel, which this window has the width for
    let (narrow, gaps) = (60.0, 2.0 * SIDE_GAP);
    assert!(size.x - narrow - gaps >= 220.0 && size.x - 340.0 - gaps < 220.0);
    let (beside, room, list_beside) = draw(&mut frames, narrow);
    assert_eq!(
        crate::outside::take_resizes(&frames.ctx),
        [(id, Vec2::new(size.x + narrow + gaps, size.y), false)]
    );
    assert_eq!(beside, alone);
    assert_eq!(list_beside, list);
    assert!(room.is_none(), "the system has not made the room yet");
    assert_eq!(floater_size(&frames.ctx, name), Some(size));
    size_floater(&frames.ctx, name, Vec2::new(size.x, 300.0));
    assert_eq!(
        crate::outside::take_resizes(&frames.ctx),
        [(id, Vec2::new(size.x + narrow + gaps, 300.0), false)]
    );
    // asked once
    draw(&mut frames, narrow);
    assert!(crate::outside::take_resizes(&frames.ctx).is_empty());
    // past the wait the window takes what is left beside the room, and
    // the panel has its place at the right of it
    let (narrowed, room, list_narrowed) = wait(&mut frames, narrow);
    let room = room.expect("the room is told once the wait is over");
    assert!((room.width() - narrow).abs() < 0.01);
    assert!((room.right() - (right - SIDE_GAP)).abs() < 0.01);
    assert!((narrowed.width() - (alone.width() - narrow - gaps)).abs() < 0.01);
    assert!(narrowed.right() <= room.left() - SIDE_GAP + 0.01);
    assert_eq!(list_narrowed, list);
    assert_eq!(floater_size(&frames.ctx, name), Some(Vec2::new(size.x - narrow - gaps, size.y)));
    size_floater(&frames.ctx, name, Vec2::new(500.0, 300.0));
    assert_eq!(
        crate::outside::take_resizes(&frames.ctx),
        [(id, Vec2::new(500.0 + narrow + gaps, 300.0), false)]
    );
    // the room goes: asked back to the width it had, and nothing is added
    // to a size asked from then on
    draw(&mut frames, 0.0);
    assert_eq!(crate::outside::take_resizes(&frames.ctx), [(id, size, false)]);
    size_floater(&frames.ctx, name, Vec2::new(500.0, 300.0));
    assert_eq!(crate::outside::take_resizes(&frames.ctx), [(id, Vec2::new(500.0, 300.0), false)]);
    let (back, room, _) = wait(&mut frames, 0.0);
    assert_eq!((back, room), (alone, None));
    assert!(crate::outside::take_resizes(&frames.ctx).is_empty());
    // A panel this window has no width for, on a system that gives none:
    // the window keeps its whole width and the panel is over its right
    // end; a size asked of the window is then the system's window's.
    draw(&mut frames, 340.0);
    assert_eq!(
        crate::outside::take_resizes(&frames.ctx),
        [(id, Vec2::new(size.x + 340.0 + gaps, size.y), false)]
    );
    let (under, room, list_under) = wait(&mut frames, 340.0);
    let room = room.expect("the panel has a place without the room");
    assert_eq!((under, list_under), (alone, list));
    assert!((room.width() - (size.x - gaps)).abs() < 0.01);
    assert!((room.right() - (right - SIDE_GAP)).abs() < 0.01);
    assert_eq!(floater_size(&frames.ctx, name), Some(size));
    size_floater(&frames.ctx, name, Vec2::new(500.0, 300.0));
    assert_eq!(crate::outside::take_resizes(&frames.ctx), [(id, Vec2::new(500.0, 300.0), false)]);
}

/// A content a fraction of a point taller than the room its window has (the
/// system gives whole pixels) does not scroll: the scroll bar would narrow
/// it, and a content that wraps would grow and shrink with its window at
/// every frame. A content really taller than its room scrolls. The content
/// stays under what is above it (the window's header).
#[test]
fn a_window_outside_a_fraction_short_of_its_content_does_not_scroll() {
    // the content's width and top, under a header 30 high, in a room 200
    // high left under it
    let drawn = |tall: f32| {
        let mut frames = elide_tests::Frames::new();
        let mut seen = (0.0, 0.0);
        // (the bar shows from the frame after the one that measures)
        for _ in 0..4 {
            frames.run(Vec::new(), |ui| {
                let room = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::new(300.0, 230.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    // (the viewer's scroll bars: a strip of their own beside the content)
                    ui.spacing_mut().scroll = reserved_scroll_style();
                    ui.allocate_space(Vec2::new(300.0, 30.0));
                    sized_body(ui, |ui| {
                        seen = (ui.available_width(), ui.cursor().top());
                        ui.allocate_space(Vec2::new(10.0, tall));
                    });
                });
            });
        }
        seen
    };
    assert_eq!(drawn(180.0), (300.0, 50.0));
    assert_eq!(drawn(200.6), (300.0, 50.0), "0.6 point short, as at an interface scale of 0.8333");
    let (width, top) = drawn(203.0);
    assert!(width < 300.0, "taller than its room: the scroll bar has its strip");
    assert_eq!(top, 50.0);
}

/// Ctrl+Shift+W in the viewer's window closes a window outside with the
/// others; Ctrl+W there closes the front window inside, never this one.
/// (Without a host, egui runs the window outside in the viewer's pass: it
/// reads the viewer's keys, those the front window inside left.)
#[test]
fn close_all_windows_closes_a_window_outside_too() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let window = egui::Id::unique("test outside closes");
    crate::outside::want(&frames.ctx, window, true);
    crate::outside::set_native(&frames.ctx, window, true);
    let key = |pressed: bool, modifiers: egui::Modifiers| egui::Event::Key {
        key: egui::Key::W,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    let (mut open, mut inside) = (true, true);
    let mut frame = |open: &mut bool, inside: &mut bool, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            crate::ui::close_key::begin(ui.ctx());
            Floater::new("test inside front", "Inside", egui::pos2(10.0, 10.0), Vec2::new(200.0, 100.0))
                .silent()
                .show(ui.ctx(), &p, inside, |_| ());
            Floater::new("test outside closes", "Outside", egui::pos2(10.0, 10.0), Vec2::new(200.0, 100.0))
                .silent()
                .show(ui.ctx(), &p, open, |_| ());
        });
    };
    frame(&mut open, &mut inside, Vec::new());
    let (command, all) = (egui::Modifiers::COMMAND, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    frame(&mut open, &mut inside, vec![key(true, command)]);
    frame(&mut open, &mut inside, vec![key(false, command)]);
    assert!(!inside, "Ctrl+W is for the front window inside");
    assert!(open);
    inside = true;
    frame(&mut open, &mut inside, Vec::new());
    frame(&mut open, &mut inside, vec![key(true, all)]);
    assert!(!open);
    assert!(!inside);
}

/// Ctrl+W pressed in the window outside itself closes it, at the next
/// frame of the interface; the key is taken from its fields.
#[test]
fn ctrl_w_in_a_window_outside_closes_it() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let window = egui::Id::unique("test outside closes itself");
    crate::outside::want(&frames.ctx, window, true);
    crate::outside::set_native(&frames.ctx, window, true);
    let key = |modifiers: egui::Modifiers| egui::Event::Key {
        key: egui::Key::W,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let mut open = true;
    let mut left_to_fields = false;
    let mut frame = |open: &mut bool, left: &mut bool, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            crate::ui::close_key::begin(ui.ctx());
            Floater::new(
                "test outside closes itself",
                "Outside",
                egui::pos2(10.0, 10.0),
                Vec2::new(200.0, 100.0),
            )
            .silent()
            .show(ui.ctx(), &p, open, |ui| {
                *left |= ui.input(|i| i.key_pressed(egui::Key::W));
            });
        });
    };
    frame(&mut open, &mut left_to_fields, Vec::new());
    // (no window inside: the key reaches the window outside, as its own)
    frame(&mut open, &mut left_to_fields, vec![key(egui::Modifiers::COMMAND)]);
    assert!(open);
    assert!(!left_to_fields);
    frame(&mut open, &mut left_to_fields, Vec::new());
    assert!(!open);
}

/// The size asked of a window outside is counted in points: at another
/// zoom the same points are another size of the window.
#[test]
fn the_size_asked_of_a_window_outside_is_forgotten_at_another_zoom() {
    let content = Vec2::new(350.0, 610.0);
    assert!(!follows_content(
        Vec2::new(350.0, 400.0),
        content,
        asked_at(Some((content, 1.0)), 1.0)
    ));
    assert!(follows_content(
        Vec2::new(292.0, 508.0),
        content,
        asked_at(Some((content, 1.0)), 1.2)
    ));
    assert!(follows_content(Vec2::new(350.0, 400.0), content, asked_at(None, 1.0)));
}

#[test]
fn the_button_of_a_window_outside_brings_it_in_front_before_it_closes_it() {
    let ctx = egui::Context::default();
    let id = egui::Id::unique("inventory");
    crate::outside::set_native(&ctx, id, true);
    // behind, as far as the viewer can tell: the press brings it in front
    let mut open = true;
    toggle_floater(&ctx, "inventory", &mut open);
    assert!(open);
    assert_eq!(crate::outside::raises(&ctx), [id]);
    assert!(!toggle_floater_closing(&ctx, "inventory", true));
    assert!(!close_asked(&ctx, "inventory"));
    // with the focus: it closes, as inside
    crate::outside::set_front(&ctx, Some(id));
    toggle_floater(&ctx, "inventory", &mut open);
    assert!(!open);
    assert!(!toggle_floater_closing(&ctx, "inventory", true));
    assert!(close_asked(&ctx, "inventory"));
    // a window inside keeps its toggle
    let mut people = true;
    toggle_floater(&ctx, "people", &mut people);
    assert!(!people);
}

/// A document's name is drawn in its window outside, whose title is the
/// system's, and renamed there as in the title inside.
#[test]
fn a_document_outside_is_renamed_in_its_window() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside renamed";
    let window = egui::Id::unique(name);
    let ctx = frames.ctx.clone();
    crate::outside::want(&ctx, window, true);
    crate::outside::set_native(&ctx, window, true);
    let typed = |ctx: &egui::Context, text: &str| {
        ctx.data_mut(|d| {
            d.insert_temp(
                window.with("title_rename_draft"),
                FloaterTitleDraft {
                    text: text.to_owned(),
                    focus: true,
                },
            );
        });
    };
    let enter = || egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut frame = |allowed: bool, events: Vec<egui::Event>| {
        let mut open = true;
        frames.run(events, |ui| {
            Floater::new(name, "Old name", egui::pos2(10.0, 10.0), Vec2::new(300.0, 200.0))
                .silent()
                .rename_title("Old name", allowed)
                .show(ui.ctx(), &p, &mut open, |_| ());
        });
    };
    typed(&ctx, "New name");
    frame(true, Vec::new());
    frame(true, vec![enter()]);
    assert_eq!(take_title_rename(&ctx, name).as_deref(), Some("New name"));
    // without the right to rename, what was typed is dropped
    typed(&ctx, "Another");
    frame(false, Vec::new());
    frame(false, vec![enter()]);
    assert_eq!(take_title_rename(&ctx, name), None);
}

#[test]
fn a_picture_keeps_its_shape_in_a_window_outside_of_any_shape() {
    // a wide window: bars at the sides; a tall one: above and below
    assert_eq!(letterboxed(Vec2::new(1000.0, 300.0), 2.0), Vec2::new(600.0, 300.0));
    assert_eq!(letterboxed(Vec2::new(400.0, 900.0), 2.0), Vec2::new(400.0, 200.0));
    assert_eq!(letterboxed(Vec2::new(640.0, 360.0), 16.0 / 9.0), Vec2::new(640.0, 360.0));
}

/// The body of a window of a kept shape gets, outside, the room of that
/// shape, and its window is asked the picture's shape once per shape.
#[test]
fn a_window_of_a_kept_shape_draws_its_picture_at_its_ratio_outside() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let name = "test outside player";
    let window = egui::Id::unique(name);
    let ctx = frames.ctx.clone();
    crate::outside::want(&ctx, window, true);
    crate::outside::set_native(&ctx, window, true);
    let mut open = true;
    let mut draw = |frames: &mut elide_tests::Frames, ratio: f32| {
        let mut got = None;
        frames.run(Vec::new(), |ui| {
            got = Floater::new(name, "Player", egui::pos2(10.0, 10.0), Vec2::new(480.0, 300.0))
                .silent()
                .flush_body()
                .keep_aspect(ratio, Vec2::ZERO)
                .show(ui.ctx(), &p, &mut open, |ui| ui.available_size());
        });
        got
    };
    let size = draw(&mut frames, 2.0).expect("the body is drawn");
    assert!((size.x / size.y - 2.0).abs() < 0.01, "{size:?}");
    let asked = |ctx: &egui::Context| -> Vec<Vec2> {
        crate::outside::take_resizes(ctx)
            .into_iter()
            .filter(|(id, _, held)| *id == window && !held)
            .map(|(_, size, _)| size)
            .collect()
    };
    let wide = asked(&ctx);
    assert_eq!(wide.len(), 1, "{wide:?}");
    // the same shape again: its window is the user's to size
    draw(&mut frames, 2.0);
    assert_eq!(asked(&ctx), Vec::new());
    // another video, of another shape: asked once more, at the same width
    draw(&mut frames, 1.0);
    let square = asked(&ctx);
    assert_eq!(square.len(), 1, "{square:?}");
    assert_eq!(square[0].x, wide[0].x);
    assert!((square[0].y - wide[0].y - wide[0].x / 2.0).abs() < 0.01, "{wide:?} {square:?}");
}

/// llMapDestination without BEACON_FOCUS_MAP leaves the map's window
/// outside where it is (raising it gives it the system's focus), and only
/// expands it when minimized; a show by the user raises it.
#[test]
fn a_quiet_show_never_raises_a_window_outside() {
    let ctx = egui::Context::default();
    let id = egui::Id::unique("worldmap");
    crate::outside::set_native(&ctx, id, true);
    let mut open = true;
    show_floater_quietly(&ctx, "worldmap", &mut open);
    assert!(crate::outside::raises(&ctx).is_empty());
    assert!(!floater_expand_asked(&ctx, "worldmap"));
    minimize_floater(&ctx, "worldmap");
    show_floater_quietly(&ctx, "worldmap", &mut open);
    assert!(crate::outside::raises(&ctx).is_empty());
    assert!(floater_expand_asked(&ctx, "worldmap"), "a minimized one expands");
    show_floater(&ctx, "worldmap", &mut open);
    assert_eq!(crate::outside::raises(&ctx), [id]);
}

/// A button in a window outside brings a window inside to the front: its
/// `move_to_top` orders the outside window's layers, not the viewer's
/// (emulated: asked after the window drew, the other window going up in
/// the same pass, which egui then keeps above), so the window goes up when
/// it draws next.
#[test]
fn a_window_asked_to_the_front_from_outside_goes_up_when_drawn() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let ctx = frames.ctx.clone();
    let mut open = [true, true];
    let mut draw = |frames: &mut elide_tests::Frames, after: &dyn Fn(&egui::Context)| {
        frames.run(Vec::new(), |ui| {
            for (n, name) in ["test front a", "test front b"].into_iter().enumerate() {
                Floater::new(name, "Window", egui::pos2(20.0 + 40.0 * n as f32, 20.0), Vec2::new(300.0, 200.0))
                    .silent()
                    .show(ui.ctx(), &p, &mut open[n], |ui| {
                        ui.label("body");
                    });
            }
            after(ui.ctx());
        });
    };
    draw(&mut frames, &|_| {});
    draw(&mut frames, &|_| {});
    assert!(floater_above(&ctx, "test front b", "test front a"));
    draw(&mut frames, &|ctx| {
        bring_floater_front(ctx, "test front a");
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("test front b")));
    });
    assert!(floater_above(&ctx, "test front b", "test front a"), "not in the viewer's pass");
    draw(&mut frames, &|_| {});
    assert!(floater_above(&ctx, "test front a", "test front b"));
}
