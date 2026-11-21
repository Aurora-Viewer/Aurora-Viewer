use super::*;

fn long_list(f: &mut Fixture, parent: u128) {
    f.inv.created_folders(
        (0..80)
            .map(|n| aurora_net::inventory::InvFolder {
                id: id(1000 + parent * 100 + n),
                parent: id(parent),
                name: format!("C Folder {n:02}"),
                type_default: -1,
                ..Default::default()
            })
            .chain(std::iter::once(aurora_net::inventory::InvFolder {
                id: id(90000),
                parent: id(parent),
                name: "Z Last destination".into(),
                type_default: -1,
                ..Default::default()
            }))
            .collect(),
    );
    for folder in f.inv.folders.values_mut() {
        folder.state = FetchState::Fetched;
    }
}
fn viewport(f: &Fixture, editor: Uuid) -> (egui::Rect, egui::Id) {
    f.ctx
        .data(|d| d.get_temp(egui::Id::unique(("inventory_drag_viewport", editor))))
        .expect("scroll viewport")
}
fn offset(f: &Fixture, editor: Uuid) -> f32 {
    crate::ui::widgets::scroll_offset::next(&f.ctx, viewport(f, editor).1)
        .expect("scroll state")
        .y
}

#[test]
fn inventory_drag_navigation_toggles_once_per_visit_and_can_collapse_the_source() {
    let mut f = Fixture::new();
    f.click(100, false);
    f.click(101, true);
    f.start(100);
    let to = f.row(20) - Vec2::new(55.0, 0.0); // Folder caret, before the label.
    f.hover(to);
    for _ in 0..25 {
        f.frame(Vec::new(), false);
    }
    assert!(f.state.visible.contains(&id(21)), "closed folder springs open");
    for _ in 0..60 {
        f.frame(Vec::new(), false);
    }
    assert!(f.state.visible.contains(&id(21)), "no oscillation while parked");
    // Leaving and returning starts a fresh one-second visit on an open folder.
    f.hover(egui::pos2(350.0, 450.0));
    f.hover(to);
    for _ in 0..25 {
        f.frame(Vec::new(), false);
    }
    assert!(!f.state.visible.contains(&id(21)), "open folder closes");
    for _ in 0..50 {
        f.frame(Vec::new(), false);
    }
    assert!(!f.state.visible.contains(&id(21)));
    // The source is a no-op drop destination, but must remain navigable.
    let source = f.row(10);
    f.hover(source);
    for _ in 0..25 {
        f.frame(Vec::new(), false);
    }
    assert!(!f.state.visible.contains(&id(100)));
    assert!(f.state.selection.contains(&id(100)) && f.state.selection.contains(&id(101)));
    let to = f.row(20) - Vec2::new(55.0, 0.0);
    f.hover(to);
    f.button(to, false, false);
    assert_eq!(
        f.moved(),
        vec![(id(100), id(20)), (id(101), id(20))],
        "cargo survives hiding its source rows"
    );
}

#[test]
fn inventory_drag_navigation_autoscrolls_both_directions_and_stops_in_the_middle_or_out_of_the_window() {
    for filtered in [false, true] {
        let mut f = Fixture::new();
        long_list(&mut f, 1);
        f.scrolling = Some(filtered);
        f.state.show_original(&f.inv, id(100));
        f.settle();
        f.click(100, false);
        f.click(101, true);
        f.start(100);
        let rect = viewport(&f, f.state.ui_id).0;
        let down = egui::pos2(rect.center().x, rect.bottom() + 25.0);
        f.hover(down);
        for _ in 0..80 {
            f.frame(Vec::new(), false);
        }
        let bottom = offset(&f, f.state.ui_id);
        assert!(bottom > 1000.0, "long inventory scrolled: {bottom} (filtered={filtered})");
        assert_eq!(f.state.selection, HashSet::from([id(100), id(101)]));
        assert!(f.actions.is_empty());
        f.hover(rect.center());
        let stopped = offset(&f, f.state.ui_id);
        for _ in 0..12 {
            f.frame(Vec::new(), false);
        }
        assert!((offset(&f, f.state.ui_id) - stopped).abs() < 0.1, "middle must stop immediately");
        f.hover(egui::pos2(rect.center().x, rect.top() + 4.0));
        for _ in 0..35 {
            f.frame(Vec::new(), false);
        }
        assert!(offset(&f, f.state.ui_id) < bottom - 250.0, "upward scrolling");
        // out of the window (past the bottom of the screen here): it stops
        f.hover(egui::pos2(rect.center().x, 1000.0));
        let out = offset(&f, f.state.ui_id);
        for _ in 0..12 {
            f.frame(Vec::new(), false);
        }
        assert!(
            (offset(&f, f.state.ui_id) - out).abs() < 0.1,
            "out of the window must stop (filtered={filtered})"
        );
        f.hover(down);
        for _ in 0..90 {
            f.frame(Vec::new(), false);
        }
        let end = offset(&f, f.state.ui_id);
        for _ in 0..20 {
            f.frame(Vec::new(), false);
        }
        assert!((offset(&f, f.state.ui_id) - end).abs() < 0.1, "clamped at bottom");
        let to = f.row(90000);
        assert!(rect.contains(to), "previously hidden destination is now visible: {to:?}, {rect:?}");
        f.hover(to);
        f.button(to, false, false);
        assert_eq!(f.moved(), vec![(id(100), id(90000)), (id(101), id(90000))]);
        let after = offset(&f, f.state.ui_id);
        f.settle();
        assert!((offset(&f, f.state.ui_id) - after).abs() < 0.1);
    }
}

#[test]
fn inventory_drag_navigation_only_scrolls_the_entered_window_and_cancels_on_escape() {
    let mut f = Fixture::new();
    long_list(&mut f, 1);
    long_list(&mut f, 20);
    f.scrolling = Some(false);
    f.two_windows = true;
    f.settle();
    f.click(100, false);
    f.click(101, true);
    f.start(100);
    let left = viewport(&f, f.state.ui_id).0;
    let right = viewport(&f, f.other.ui_id).0;
    f.hover(right.center());
    f.hover(egui::pos2(right.center().x, right.bottom() + 30.0));
    for _ in 0..35 {
        f.frame(Vec::new(), false);
    }
    assert!(offset(&f, f.other.ui_id) > 100.0);
    assert!(offset(&f, f.state.ui_id) < 1.0);
    let before = offset(&f, f.other.ui_id);
    f.hover(egui::pos2(left.center().x, right.bottom() + 30.0));
    for _ in 0..12 {
        f.frame(Vec::new(), false);
    }
    assert!((offset(&f, f.other.ui_id) - before).abs() < 0.1, "outside the owning column");
    f.hover(left.center());
    f.hover(egui::pos2(left.center().x, left.bottom() + 30.0));
    for _ in 0..20 {
        f.frame(Vec::new(), false);
    }
    assert!(offset(&f, f.state.ui_id) > 50.0);
    f.frame(
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        false,
    );
    let stopped = offset(&f, f.state.ui_id);
    f.settle();
    assert!((offset(&f, f.state.ui_id) - stopped).abs() < 0.1);
    assert!(f.actions.is_empty());
}

#[test]
fn inventory_drag_scroll_direction_stays_between_the_search_row_and_the_window() {
    let list = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(300.0, 500.0));
    let search = Some(60.0);
    let at = |y: f32| egui::pos2(150.0, y);
    // the edges of the list, the window's footer under it
    assert_eq!(drag::navigation::direction(list, at(105.0), true, search), -1.0);
    assert_eq!(drag::navigation::direction(list, at(495.0), true, search), 1.0);
    assert_eq!(drag::navigation::direction(list, at(560.0), true, search), 1.0);
    assert_eq!(drag::navigation::direction(list, at(300.0), true, search), 0.0);
    // up over the search row, not past it (the tabs and buttons above)
    assert_eq!(drag::navigation::direction(list, at(70.0), true, search), -1.0);
    assert_eq!(drag::navigation::direction(list, at(55.0), true, search), 0.0);
    // a window with no search row: up as far as its own top
    assert_eq!(drag::navigation::direction(list, at(55.0), true, None), -1.0);
    // out of the window, or beside the list
    assert_eq!(drag::navigation::direction(list, at(560.0), false, search), 0.0);
    assert_eq!(drag::navigation::direction(list, egui::pos2(350.0, 495.0), true, search), 0.0);
}

#[test]
fn inventory_held_over_a_tab_opens_it_after_a_short_delay_and_only_there() {
    let ctx = egui::Context::default();
    let tab = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), Vec2::new(80.0, 30.0));
    let tab_id = egui::Id::unique("inventory_drag_tab_test");
    let mut time = 0.0;
    // one frame with the pointer at `pos`, its button held; `carry` sets an
    // inventory cargo first
    let mut frame = |pos: egui::Pos2, press: bool, carry: bool| -> bool {
        time += 0.05;
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if press {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut opened = false;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                if carry {
                    egui::DragAndDrop::set_payload(ui.ctx(), InvDrag(id(1)));
                }
                opened = drag::navigation::tab_held_over(ui, tab, tab_id);
            },
        );
        opened
    };
    let on = tab.center();
    let off = egui::pos2(400.0, 400.0);
    // no cargo: a pressed pointer over the tab opens nothing
    assert!(!frame(on, true, false));
    for _ in 0..10 {
        assert!(!frame(on, false, false));
    }
    // cargo held over the tab: not at once, then open
    assert!(!frame(off, false, true));
    assert!(!frame(on, false, false), "not as soon as the pointer is over the tab");
    assert!(!frame(on, false, false));
    let opened = (0..6).any(|_| frame(on, false, false));
    assert!(opened, "the tab opens after the delay");
    // away and back: a fresh delay
    assert!(!frame(off, false, false));
    assert!(!frame(on, false, false));
}
