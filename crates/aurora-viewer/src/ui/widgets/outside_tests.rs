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
    assert!(!window("d").compact_when_minimized(240.0).goes_outside());
    assert!(!window("e").keep_aspect(1.5, Vec2::ZERO).goes_outside());
    let mut over = window("f");
    over.order = egui::Order::Foreground;
    assert!(!over.goes_outside());
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
#[test]
fn close_all_windows_closes_a_window_outside_too() {
    let mut frames = elide_tests::Frames::new();
    let p = crate::theme::Theme::default().palette();
    let window = egui::Id::unique("test outside closes");
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
    let mut frame = |open: &mut bool, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            crate::ui::close_key::begin(ui.ctx());
            Floater::new("test outside closes", "Outside", egui::pos2(10.0, 10.0), Vec2::new(200.0, 100.0))
                .silent()
                .show(ui.ctx(), &p, open, |_| ());
        });
    };
    frame(&mut open, Vec::new());
    frame(&mut open, vec![key(egui::Modifiers::COMMAND)]);
    assert!(open, "Ctrl+W is for the front window inside");
    frame(&mut open, vec![key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)]);
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
