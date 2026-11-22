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
