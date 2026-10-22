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
fn inventory_drag_navigation_autoscrolls_both_directions_outside_the_field_and_stops_in_the_middle() {
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
        f.hover(egui::pos2(rect.center().x, rect.top() - 25.0));
        for _ in 0..35 {
            f.frame(Vec::new(), false);
        }
        assert!(offset(&f, f.state.ui_id) < bottom - 250.0, "upward scrolling");
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
