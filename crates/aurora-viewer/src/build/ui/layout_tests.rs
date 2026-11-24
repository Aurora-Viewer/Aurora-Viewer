use super::*;
use crate::build::layout::{FooterColumn, FooterRow, Item, Panel, Profile};

fn settle(inspector: &mut Inspector) {
    for _ in 0..4 {
        inspector.show(vec![]);
    }
}
fn row(inspector: &Inspector, item: Item, panel: Panel) -> egui::Rect {
    inspector
        .tool
        .ui
        .layout
        .targets
        .iter()
        .find(|(key, location, _)| *key == item && *location == panel)
        .unwrap()
        .2
}
fn button(inspector: &mut Inspector, point: egui::Pos2, pressed: bool) {
    inspector.show(vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]);
}
fn click(inspector: &mut Inspector, point: egui::Pos2) {
    inspector.show(vec![egui::Event::PointerMoved(point)]);
    button(inspector, point, true);
    button(inspector, point, false);
}
fn drag(inspector: &mut Inspector, from: egui::Pos2, to: egui::Pos2) {
    inspector.show(vec![egui::Event::PointerMoved(from)]);
    button(inspector, from, true);
    inspector.show(vec![egui::Event::PointerMoved(from + vec2(10.0, 0.0))]);
    inspector.show(vec![egui::Event::PointerMoved(to)]);
    button(inspector, to, false);
    settle(inspector);
}

#[test]
fn organization_cues_and_compatible_targets_do_not_resize_or_reflow_controls() {
    let mut i = Inspector::new();
    settle(&mut i);
    let (window, _) = i.show(vec![]);
    let positions = i.tool.ui.layout.targets.clone();
    let height = i.tool.ui.layout.footer_height;
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    assert_eq!(
        i.tool
            .ui
            .layout
            .targets
            .iter()
            .filter(|(_, panel, _)| *panel == Panel::Footer)
            .collect::<Vec<_>>(),
        positions.iter().filter(|(_, panel, _)| *panel == Panel::Footer).collect::<Vec<_>>()
    );
    let from = row(&i, Item::RootAxis, Panel::Footer).center();
    let to = row(&i, Item::MoveMode, Panel::Footer).center();
    i.show(vec![egui::Event::PointerMoved(from)]);
    button(&mut i, from, true);
    i.show(vec![egui::Event::PointerMoved(from + vec2(10.0, 0.0))]);
    let (drag_window, _) = i.show(vec![egui::Event::PointerMoved(to)]);
    assert_eq!(i.tool.ui.layout.drag, Some(Item::RootAxis));
    assert_eq!(i.tool.ui.layout.footer_height, height);
    assert!((drag_window.size() - window.size()).length() < 1.0);
    assert_eq!(
        i.tool
            .ui
            .layout
            .targets
            .iter()
            .filter(|(_, panel, _)| *panel == Panel::Footer)
            .collect::<Vec<_>>(),
        positions.iter().filter(|(_, panel, _)| *panel == Panel::Footer).collect::<Vec<_>>()
    );
    let outside = egui::pos2(i.screen.x - 10.0, i.screen.y - 10.0);
    button(&mut i, outside, false);
}

#[test]
fn action_drag_and_keyboard_placement_only_offer_complete_column_block_edges() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let bounds = |i: &Inspector| {
        i.tool
            .ui
            .layout
            .targets
            .iter()
            .filter(|(item, panel, _)| *panel == Panel::Footer && item.footer_column_item())
            .map(|(_, _, rect)| *rect)
            .reduce(|a, b| a.union(b))
            .unwrap()
    };
    let original = i.settings.clone();
    let from = row(&i, Item::Actions, Panel::Footer).center();
    let middle = bounds(&i).center();
    i.show(vec![egui::Event::PointerMoved(from)]);
    button(&mut i, from, true);
    i.show(vec![egui::Event::PointerMoved(from + vec2(10.0, 0.0))]);
    i.show(vec![egui::Event::PointerMoved(middle)]);
    assert_eq!(i.tool.ui.layout.drag, Some(Item::Actions));
    assert_eq!(i.tool.ui.layout.footer_cells.len(), 2);
    assert!(i.tool.ui.layout.footer_cells.iter().all(|cell| !cell.rect.contains(middle)));
    button(&mut i, middle, false);
    settle(&mut i);
    assert_eq!(i.settings, original, "Dropping the bar in the columns must do nothing");
    for top in [true, false] {
        let from = row(&i, Item::Actions, Panel::Footer).center();
        let block = bounds(&i);
        let target = egui::pos2(block.center().x, if top { block.top() + 1.0 } else { block.bottom() - 1.0 });
        drag(&mut i, from, target);
        assert_eq!(i.settings.layout.footer_actions_top(), top);
        let rows = i.settings.layout.footer_rows();
        assert_eq!(if top { rows.first() } else { rows.last() }, Some(&FooterRow::Full(Item::Actions)));
    }
    let action = row(&i, Item::Actions, Panel::Footer).center();
    click(&mut i, action);
    for (key, top) in [(egui::Key::ArrowUp, true), (egui::Key::ArrowDown, false)] {
        i.show(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        settle(&mut i);
        assert_eq!(i.settings.layout.footer_actions_top(), top);
    }
    i.settings.layout = original.layout.clone();
    assert_eq!(i.settings, original, "Organization must not change option values");
}

/// The face selector shows its buttons 0 to 8; "Edit linked parts", once
/// on, "Link n" (`number`, empty while it is off).
fn part_shown(item: Item, number: &str, texts: &[(String, egui::Rect)], rect: egui::Rect) -> bool {
    let shown = |label: &str| texts.iter().any(|(text, at)| text == label && rect.contains(at.center()));
    if item == Item::Face {
        (0..crate::build::face_buttons::FACE_BUTTONS).all(|face| shown(&face.to_string()))
    } else if number.is_empty() {
        shown(aurora_i18n::tr!("build-compact-linked"))
    } else {
        shown(&aurora_i18n::tr!("build-compact-link-value", number = number))
    }
}

/// The footer's face selector, or the "Edit linked parts" option row.
fn part_rect(i: &Inspector, item: Item) -> egui::Rect {
    if item == Item::Face {
        i.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_part_navigation", item))))
            .unwrap()
    } else {
        row(i, item, Panel::Footer)
    }
}

#[test]
fn part_selectors_keep_visible_values_in_the_shared_reference_row() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    for organizing in [false, true] {
        i.tool.ui.layout.organizing = organizing;
        i.tool.edit_mode = EditMode::Move;
        i.settings.edit_linked = false;
        settle(&mut i);
        let (_, texts) = i.show(vec![]);
        for item in [Item::Face, Item::Linked] {
            let rect = part_rect(&i, item);
            assert!(
                part_shown(item, if item == Item::Face { "-" } else { "" }, &texts, rect),
                "Inactive {item:?} must reserve a visible shared value, organizing={organizing}"
            );
        }
        i.tool.edit_mode = EditMode::Face;
        i.settings.edit_linked = true;
        settle(&mut i);
        let (_, texts) = i.show(vec![]);
        for (item, value) in [
            (Item::Face, i.tool.selected_face_numbers(&i.world)),
            (Item::Linked, i.tool.selected_link_number(&i.world).unwrap().to_string()),
        ] {
            let rect = part_rect(&i, item);
            assert!(
                part_shown(item, &value, &texts, rect),
                "Active {item:?} must show its value, organizing={organizing}"
            );
        }
    }
    i.tool.deselect_all(&i.world);
    for organizing in [false, true] {
        i.tool.ui.layout.organizing = organizing;
        settle(&mut i);
        let (_, texts) = i.show(vec![]);
        for item in [Item::Face, Item::Linked] {
            let rect = part_rect(&i, item);
            assert!(
                part_shown(item, "-", &texts, rect),
                "Unavailable {item:?} must keep its shared placeholder, organizing={organizing}"
            );
        }
    }
}

#[test]
fn a_face_button_click_selects_that_face_and_a_missing_face_stays_greyed() {
    let mut i = Inspector::new();
    let idx = i.tool.sel_prims(&i.world)[0];
    let faces = i.tool.num_faces(&i.world, idx);
    assert!(
        (3..crate::build::face_buttons::FACE_BUTTONS).contains(&faces),
        "a demo prim with 3 to 8 faces"
    );
    i.tool.edit_mode = EditMode::Face;
    settle(&mut i);
    let rects = i
        .ctx
        .data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::unique("build_face_buttons")))
        .unwrap();
    assert_eq!(rects.len(), crate::build::face_buttons::FACE_BUTTONS);
    let footer = i
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_part_navigation", Item::Face))))
        .unwrap();
    assert!(rects.iter().all(|rect| footer.contains_rect(*rect) && rect.width() > 4.0));
    click(&mut i, rects[2].center());
    assert_eq!(i.tool.sel_faces(&i.world, idx), [2]);
    click(&mut i, rects[faces].center());
    assert_eq!(i.tool.sel_faces(&i.world, idx), [2]);
    // in Move too, which stays the mode; greyed in Align
    i.tool.edit_mode = EditMode::Move;
    i.tool.faces.clear();
    settle(&mut i);
    click(&mut i, rects[1].center());
    assert_eq!(i.tool.sel_faces(&i.world, idx), [1]);
    assert_eq!(i.tool.edit_mode, EditMode::Move);
    i.tool.edit_mode = EditMode::Align;
    i.tool.faces.clear();
    settle(&mut i);
    click(&mut i, rects[0].center());
    assert!(i.tool.faces.is_empty());
}

#[test]
fn shift_click_on_a_section_shows_it_beside_the_active_one_in_a_wider_window() {
    let mut i = Inspector::new();
    i.tool.tab = 6;
    settle(&mut i);
    let width = |i: &Inspector| {
        i.ctx
            .memory(|m| m.area_rect(egui::Id::unique(super::super::super::INSPECTOR_ID)))
            .unwrap()
            .width()
    };
    let column = |i: &Inspector, n: usize| {
        i.ctx
            .data(|d| d.get_temp::<(usize, egui::Rect)>(egui::Id::unique(("build_section_column", n))))
    };
    let single = width(&i);
    let materials = row(&i, Item::Materials, Panel::Navigation);
    i.mods.shift = true;
    click(&mut i, materials.center());
    i.mods.shift = false;
    for _ in 0..8 {
        i.show(vec![]);
    }
    assert!(width(&i) >= 2.0 * single - 1.0, "the window holds both: {} for {single}", width(&i));
    assert_eq!(i.tool.tab, 6);
    let (left, right) = (column(&i, 0).unwrap(), column(&i, 1).unwrap());
    assert_eq!((left.0, right.0), (6, 3));
    assert!(left.1.right() <= right.1.left() + 0.5 && right.1.width() > single * 0.8);
    // a click replaces the left one
    let object = row(&i, Item::Shape, Panel::Navigation);
    click(&mut i, object.center());
    settle(&mut i);
    assert_eq!((i.tool.tab, i.tool.ui.second.tab), (5, Some(3)));
    // Shift+click on the second closes it, the width given back
    let materials = row(&i, Item::Materials, Panel::Navigation);
    i.mods.shift = true;
    click(&mut i, materials.center());
    i.mods.shift = false;
    for _ in 0..8 {
        i.show(vec![]);
    }
    assert_eq!(i.tool.ui.second.tab, None);
    assert!((width(&i) - single).abs() < 1.0, "back to {single}: {}", width(&i));
}

#[test]
fn edit_linked_reads_link_n_with_its_arrows_at_the_same_height() {
    let mut i = Inspector::new();
    // a linkset of several prims
    for event in crate::demo::selection_cycle_events() {
        i.world.apply(event);
    }
    let root = i.world.objects.index_of_uuid(&crate::demo::action_id(99610)).unwrap();
    i.tool.edit_mode = EditMode::Move;
    i.settings.edit_linked = false;
    i.tool.click_select(&mut i.world, &i.settings, Some(root), false);
    settle(&mut i);
    let off = row(&i, Item::Linked, Panel::Footer);
    let arrow = |i: &Inspector, next: bool| {
        i.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_link_arrow", next))))
    };
    let (_, texts) = i.show(vec![]);
    assert!(
        texts
            .iter()
            .any(|(t, r)| *t == aurora_i18n::tr!("build-compact-linked") && off.contains(r.center()))
    );
    click(&mut i, off.left_center() + vec2(8.0, 0.0));
    settle(&mut i);
    assert!(i.settings.edit_linked);
    let on = row(&i, Item::Linked, Panel::Footer);
    assert_eq!(on.height(), off.height(), "the row keeps its height");
    let number = i.tool.selected_link_number(&i.world).expect("a link selected");
    let (_, texts) = i.show(vec![]);
    let label = aurora_i18n::tr!("build-compact-link-value", number = number.to_string());
    assert!(texts.iter().any(|(t, r)| t == &label && on.contains(r.center())));
    let (back, next) = (arrow(&i, false).unwrap(), arrow(&i, true).unwrap());
    assert!(on.contains_rect(back) && on.contains_rect(next) && back.right() <= next.left());
    click(&mut i, next.center());
    settle(&mut i);
    let stepped = i.tool.selected_link_number(&i.world);
    assert!(stepped.is_some() && stepped != Some(number), "► steps the link");
    click(&mut i, back.center());
    assert_eq!(i.tool.selected_link_number(&i.world), Some(number), "◄ steps back");
}

#[test]
fn the_face_row_holds_invert_all_and_the_references() {
    let mut i = Inspector::new();
    let idx = i.tool.sel_prims(&i.world)[0];
    let faces = i.tool.num_faces(&i.world, idx);
    i.tool.edit_mode = EditMode::Move;
    settle(&mut i);
    let navigation = |i: &Inspector, item: Item| {
        i.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_part_navigation", item))))
            .unwrap()
    };
    let action = |i: &Inspector, invert: bool| {
        i.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_face_action", invert))))
            .unwrap()
    };
    let face = navigation(&i, Item::Face);
    let (invert, all) = (action(&i, true), action(&i, false));
    assert!(invert.left() >= face.right() && all.left() >= invert.right());
    // Global / Local / Reference after them, on the same row
    let global = i
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_grid_mode", 0usize))))
        .unwrap();
    assert!(global.left() >= all.right());
    for rect in [invert, all, global] {
        assert!(
            (rect.center().y - face.center().y).abs() < 1.0,
            "{rect:?} off the face row {face:?}"
        );
    }
    click(&mut i, all.center());
    assert_eq!(i.tool.sel_faces(&i.world, idx).len(), faces);
    assert!(i.tool.picks_faces(), "All shows every face");
    click(&mut i, invert.center());
    assert!(i.tool.sel_faces(&i.world, idx).is_empty());
    click(&mut i, invert.center());
    assert_eq!(i.tool.sel_faces(&i.world, idx).len(), faces);
    assert_eq!(i.tool.edit_mode, EditMode::Move, "the mode stays");
}

#[test]
fn organization_blocks_option_clicks_without_changing_row_geometry() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    settle(&mut i);
    for item in [Item::Face, Item::Linked] {
        let before = row(&i, item, Panel::Footer);
        let footer_height = i.tool.ui.layout.footer_height;
        click(&mut i, before.left_center() + vec2(8.0, 0.0));
        settle(&mut i);
        let checked = row(&i, item, Panel::Footer);
        // Linked selection can change the summary and window minimum size;
        // the footer's own controls must keep their measured geometry.
        assert_eq!(before.size(), checked.size(), "{item:?}");
        assert_eq!(i.tool.ui.layout.footer_height, footer_height, "{item:?}");
        let settings = i.settings.clone();
        let mode = i.tool.edit_mode;
        i.tool.ui.layout.organizing = true;
        settle(&mut i);
        assert_eq!(
            i.tool.ui.layout.footer_height, footer_height,
            "Organizing must not append an empty row"
        );
        let organizing = row(&i, item, Panel::Footer);
        assert_eq!(organizing.size(), checked.size());
        click(&mut i, organizing.left_center() + vec2(8.0, 0.0));
        assert_eq!(row(&i, item, Panel::Footer), organizing);
        assert_eq!(i.settings, settings);
        assert_eq!(i.tool.edit_mode, mode);
        i.tool.ui.layout.organizing = false;
        settle(&mut i);
    }
}

#[test]
fn footer_column_drops_target_empty_and_occupied_cells_without_toggling_options() {
    let mut i = Inspector::new();
    // One continuous block with three left options and two right options.
    // Empty tails must not depend on placing options after the action bar.
    for item in i.settings.layout.in_panel(Panel::Footer) {
        if ![
            Item::MoveMode,
            Item::Face,
            Item::Linked,
            Item::StretchTextures,
            Item::ScaleBoth,
            Item::Actions,
        ]
        .contains(&item)
        {
            i.settings.layout.place(item, Panel::Hidden, None, false);
        }
    }
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    i.settings.layout.place(Item::Probes, Panel::Settings, None, false);
    i.settings.layout.place(Item::Highlight, Panel::Settings, None, false);
    i.settings.layout.place(Item::RootAxis, Panel::Settings, None, false);
    let before = i.settings.clone();
    settle(&mut i);
    // An empty column tail appends after that column's own last option.
    let to = i
        .tool
        .ui
        .layout
        .footer_cells
        .iter()
        .find(|cell| cell.column == FooterColumn::Right && matches!(cell.row, FooterRow::Cells(cells) if cells[1].is_none()))
        .unwrap()
        .rect
        .center();
    let from = row(&i, Item::RootAxis, Panel::Settings).center();
    drag(&mut i, from, to);
    let face = row(&i, Item::Face, Panel::Footer);
    let root = row(&i, Item::RootAxis, Panel::Footer);
    assert!(face.right() < root.left());
    assert_eq!(i.settings.layout.footer_columns[&Item::RootAxis], FooterColumn::Right);

    // Append at the lower edge of the last existing row. Changing columns
    // must remain possible even when the dragged item is its row's only item.
    assert!(
        i.tool
            .ui
            .layout
            .footer_cells
            .iter()
            .all(|cell| cell.row != FooterRow::Cells([None; 2]))
    );
    let last = i
        .tool
        .ui
        .layout
        .footer_cells
        .iter()
        .rfind(|cell| cell.column == FooterColumn::Left)
        .unwrap()
        .rect;
    let to = egui::pos2(last.center().x, last.bottom() - 2.0);
    drag(&mut i, root.center(), to);
    let to = i
        .tool
        .ui
        .layout
        .footer_cells
        .iter()
        .find(|cell| cell.column == FooterColumn::Right && cell.row == FooterRow::Cells([Some(Item::RootAxis), None]))
        .unwrap()
        .rect
        .center();
    let from = row(&i, Item::RootAxis, Panel::Footer).center();
    drag(&mut i, from, to);
    assert_eq!(i.settings.layout.footer_columns[&Item::RootAxis], FooterColumn::Right);
    assert_eq!(
        i.settings
            .layout
            .in_panel(Panel::Footer)
            .into_iter()
            .rfind(|item| i.settings.layout.footer_columns.get(item) == Some(&FooterColumn::Right)),
        Some(Item::RootAxis)
    );

    // Occupied targets use the selected column and an insertion edge.
    let target = row(&i, Item::Linked, Panel::Footer);
    let from = row(&i, Item::RootAxis, Panel::Footer).center();
    drag(&mut i, from, target.left_top() + vec2(8.0, 2.0));
    assert_eq!(i.settings.layout.footer_columns[&Item::RootAxis], FooterColumn::Left);
    assert!(row(&i, Item::RootAxis, Panel::Footer).bottom() < row(&i, Item::Linked, Panel::Footer).top());
    assert_eq!(i.settings.layout.footer_columns[&Item::StretchTextures], FooterColumn::Right);
    let mut after = i.settings.clone();
    after.layout = before.layout.clone();
    assert_eq!(after, before, "Organization changes layout only");
}

#[test]
fn footer_column_drops_move_face_and_link_checkboxes_to_the_right() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    for item in [Item::Face, Item::Linked] {
        let last = i
            .tool
            .ui
            .layout
            .footer_cells
            .iter()
            .rfind(|cell| cell.column == FooterColumn::Right)
            .unwrap()
            .rect;
        let to = egui::pos2(last.center().x, last.bottom() - 2.0);
        let from = row(&i, item, Panel::Footer).center();
        drag(&mut i, from, to);
        assert_eq!(
            i.settings
                .layout
                .footer_rows()
                .into_iter()
                .rfind(|row| matches!(row, FooterRow::Cells(_))),
            Some(FooterRow::Cells([None, Some(item)]))
        );
        assert_eq!(i.settings.layout.footer_columns[&item], FooterColumn::Right);
        assert_eq!(
            row(&i, item, Panel::Footer).width(),
            row(&i, Item::StretchTextures, Panel::Footer).width()
        );
    }
}

#[test]
fn footer_column_drop_packs_an_empty_left_column_without_moving_the_right() {
    let mut i = Inspector::new();
    for item in i.settings.layout.in_panel(Panel::Footer) {
        if item.footer_column_item() && ![Item::Highlight, Item::RootAxis].contains(&item) {
            i.settings.layout.place(item, Panel::Hidden, None, false);
        }
    }
    i.settings.layout.place_in_footer(Item::Highlight, FooterColumn::Right, None, false);
    i.settings.layout.place_in_footer(Item::RootAxis, FooterColumn::Right, None, false);
    i.settings.layout.place(Item::Probes, Panel::Settings, None, false);
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let highlight = row(&i, Item::Highlight, Panel::Footer);
    let root_before = row(&i, Item::RootAxis, Panel::Footer);
    let to = i
        .tool
        .ui
        .layout
        .footer_cells
        .iter()
        .find(|cell| cell.column == FooterColumn::Left && cell.row == FooterRow::Cells([None, Some(Item::RootAxis)]))
        .unwrap()
        .rect
        .center();
    let from = row(&i, Item::Probes, Panel::Settings).center();
    drag(&mut i, from, to);
    let probes = row(&i, Item::Probes, Panel::Footer);
    assert!(
        i.settings
            .layout
            .footer_rows()
            .contains(&FooterRow::Cells([Some(Item::Probes), Some(Item::Highlight)]))
    );
    assert!((probes.top() - highlight.top()).abs() < 1.0);
    assert_eq!(row(&i, Item::Highlight, Panel::Footer), highlight);
    assert_eq!(row(&i, Item::RootAxis, Panel::Footer), root_before);
}

#[test]
fn inserting_in_one_column_keeps_the_opposite_controls_in_place() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let positions = |i: &Inspector, column: FooterColumn| {
        i.tool
            .ui
            .layout
            .targets
            .iter()
            .filter_map(|(item, panel, rect)| {
                (*panel == Panel::Footer && i.settings.layout.footer_columns.get(item) == Some(&column)).then_some((*item, *rect))
            })
            .collect::<Vec<_>>()
    };
    let original = i.settings.clone();
    for (item, column, opposite, target) in [
        (Item::OwnedOnly, FooterColumn::Left, FooterColumn::Right, Item::MoveMode),
        (Item::MovableOnly, FooterColumn::Right, FooterColumn::Left, Item::ScaleBoth),
    ] {
        let before = positions(&i, opposite);
        let from = row(&i, item, Panel::Settings).center();
        let to = row(&i, target, Panel::Footer).left_top() + vec2(8.0, 2.0);
        drag(&mut i, from, to);
        assert_eq!(i.settings.layout.footer_columns[&item], column);
        assert_eq!(positions(&i, opposite), before, "Inserting {item:?} moved the opposite column");
        assert!(i.settings.layout.place(item, Panel::Settings, None, false));
        settle(&mut i);
        assert_eq!(positions(&i, opposite), before, "Removing {item:?} moved the opposite column");
    }
    let before = positions(&i, FooterColumn::Right);
    i.settings
        .layout
        .place_in_footer(Item::FollowEdit, FooterColumn::Left, Some(Item::MoveMode), false);
    settle(&mut i);
    assert_eq!(
        positions(&i, FooterColumn::Right),
        before,
        "Wrapped labels must not stretch the opposite column"
    );
    let mut after = i.settings.clone();
    after.layout = original.layout.clone();
    assert_eq!(after, original, "Organizing changes layout only");
}

#[test]
fn footer_column_keyboard_choice_keeps_focus_and_option_values() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let rect = row(&i, Item::RootAxis, Panel::Footer);
    click(&mut i, rect.center());
    let before = i.settings.clone();
    for (key, column) in [
        (egui::Key::ArrowLeft, FooterColumn::Left),
        (egui::Key::ArrowRight, FooterColumn::Right),
    ] {
        i.show(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        settle(&mut i);
        assert_eq!(i.settings.layout.footer_columns[&Item::RootAxis], column);
    }
    let mut after = i.settings.clone();
    after.layout = before.layout.clone();
    assert_eq!(after, before);
}

#[test]
fn pointer_organization_moves_options_toolbars_and_navigation_to_hidden_and_back() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let before = i.settings.clone();
    let from = row(&i, Item::RootAxis, Panel::Footer).center();
    let to = row(&i, Item::OwnedOnly, Panel::Settings).left_top() + vec2(80.0, 2.0);
    drag(&mut i, from, to);
    assert_eq!(i.settings.layout.locations[&Item::RootAxis], Panel::Settings);
    let settings = i.settings.layout.in_panel(Panel::Settings);
    assert_eq!(settings[0], Item::RootAxis);
    for item in [Item::RootAxis, Item::MoveMode, Item::Actions, Item::Shape] {
        let panel = i.settings.layout.locations[&item];
        let from = row(&i, item, panel).center();
        let to = i
            .tool
            .ui
            .layout
            .zones
            .iter()
            .find(|(panel, _)| *panel == Panel::Hidden)
            .unwrap()
            .1
            .center();
        drag(&mut i, from, to);
        assert_eq!(i.settings.layout.locations[&item], Panel::Hidden, "{item:?}");
        let from = row(&i, item, Panel::Hidden).center();
        let dest = if item.navigation() { Panel::Navigation } else { Panel::Footer };
        let to = if item == Item::Actions {
            let block = i
                .tool
                .ui
                .layout
                .targets
                .iter()
                .filter(|(item, panel, _)| *panel == Panel::Footer && item.footer_column_item())
                .map(|(_, _, rect)| *rect)
                .reduce(|a, b| a.union(b))
                .unwrap();
            egui::pos2(block.center().x, block.bottom() - 1.0)
        } else {
            i.tool.ui.layout.zones.iter().find(|(panel, _)| *panel == dest).unwrap().1.center()
        };
        drag(&mut i, from, to);
        assert_eq!(i.settings.layout.locations[&item], dest, "{item:?}");
    }
    let mut after = i.settings.clone();
    after.layout = before.layout.clone();
    assert_eq!(before, after, "Moving controls must not change option values");
    let json = serde_json::to_string(&i.settings).unwrap();
    let restored: BuildSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(i.settings.layout, restored.layout);
}

#[test]
fn manipulation_choices_use_individual_columns_and_keep_the_text_size() {
    let mut i = Inspector::new();
    settle(&mut i);
    let move_rect = row(&i, Item::MoveMode, Panel::Footer);
    let rotate_rect = row(&i, Item::RotateMode, Panel::Footer);
    let stretch_rect = row(&i, Item::StretchMode, Panel::Footer);
    let align_rect = row(&i, Item::AlignMode, Panel::Footer);
    assert!(move_rect.bottom() < rotate_rect.top());
    assert!(stretch_rect.bottom() < align_rect.top());
    assert!(move_rect.bottom() < stretch_rect.top());
    assert_eq!(move_rect.width(), row(&i, Item::RootAxis, Panel::Footer).width());
    assert_eq!(move_rect.height(), row(&i, Item::RootAxis, Panel::Footer).height());
    for (item, mode) in [
        (Item::RotateMode, EditMode::Rotate),
        (Item::StretchMode, EditMode::Stretch),
        (Item::AlignMode, EditMode::Align),
        (Item::MoveMode, EditMode::Move),
    ] {
        let point = row(&i, item, Panel::Footer).left_center() + vec2(8.0, 0.0);
        click(&mut i, point);
        settle(&mut i);
        assert_eq!(i.tool.edit_mode, mode);
        click(&mut i, point);
        assert_eq!(i.tool.edit_mode, mode, "Clicking the selected mode must not clear it");
        assert!(
            i.text_fonts
                .iter()
                .any(|(text, size)| text == super::super::super::layout::title(item) && (*size - common::BODY).abs() < 0.01)
        );
    }
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let last = row(&i, Item::Actions, Panel::Footer);
    let from = row(&i, Item::RotateMode, Panel::Footer).center();
    drag(&mut i, from, egui::pos2(last.left() + last.width() * 0.25, last.bottom() - 1.0));
    assert_eq!(i.settings.layout.footer_columns[&Item::RotateMode], FooterColumn::Left);
    assert_eq!(i.tool.edit_mode, EditMode::Move, "Organizing does not change the active mode");
    for item in [Item::MoveMode, Item::StretchMode, Item::AlignMode] {
        assert_eq!(i.settings.layout.locations[&item], Panel::Footer);
    }
}

#[test]
fn snap_checkbox_and_inline_choices_fit_one_column_without_changing_row_height() {
    use crate::build::SnapMode;
    let mut i = Inspector::new();
    i.settings
        .layout
        .place_in_footer(Item::Snap, FooterColumn::Right, Some(Item::Actions), false);
    settle(&mut i);
    let size = row(&i, Item::Snap, Panel::Footer).size();
    assert_eq!(size, row(&i, Item::RootAxis, Panel::Footer).size());
    let choice = |i: &Inspector, index: usize| {
        i.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_snap_choice", index))))
            .unwrap()
            .center()
    };
    for (index, mode) in [(1, SnapMode::Objects), (0, SnapMode::Grid)] {
        let point = choice(&i, index);
        click(&mut i, point);
        assert_eq!(i.settings.snap_mode(), mode);
        settle(&mut i);
        assert_eq!(row(&i, Item::Snap, Panel::Footer).size(), size);
        let point = row(&i, Item::Snap, Panel::Footer).left_center() + vec2(8.0, 0.0);
        click(&mut i, point);
        assert_eq!(i.settings.snap_mode(), SnapMode::Off);
        assert_eq!(i.settings.snap_choice(), mode);
        assert_eq!(row(&i, Item::Snap, Panel::Footer).size(), size);
        click(&mut i, point);
        assert_eq!(i.settings.snap_mode(), mode);
    }
    i.settings.set_snap_enabled(false);
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let before = i.settings.clone();
    let from = row(&i, Item::Snap, Panel::Footer).center();
    let to = row(&i, Item::MoveMode, Panel::Footer).left_top() + vec2(8.0, 2.0);
    drag(&mut i, from, to);
    assert_eq!(i.settings.layout.footer_columns[&Item::Snap], FooterColumn::Left);
    let mut after = i.settings.clone();
    after.layout = before.layout.clone();
    assert_eq!(after, before);
    let point = choice(&i, 1);
    click(&mut i, point);
    assert_eq!(i.settings.snap_mode(), SnapMode::Off, "Organization must not enable Snap");
}

/// Outside the viewer's window nothing is beside the build window: the
/// settings take the room its own window keeps at its right, and draw
/// nothing until the system has made that room.
#[test]
fn settings_take_the_room_of_the_window_outside() {
    use super::super::super::layout::{Beside, settings};
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    let p = crate::theme::Theme::default().palette();
    let id = egui::Id::unique("construction-settings");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, i.screen);
    let draw = |i: &mut Inspector, beside: Beside| {
        let ctx = i.ctx.clone();
        crate::ui::login::test_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| settings(ui.ctx(), &p, &mut i.tool, &mut i.world, &mut i.settings, i.mods, beside),
        ));
    };
    draw(&mut i, Beside::Outside(None));
    assert!(i.ctx.memory(|m| m.area_rect(id)).is_none(), "no room yet: nothing is drawn");
    // the room of a window 702 by 700 whose own width is 344
    let room = egui::Rect::from_min_max(egui::pos2(352.0, 8.0), egui::pos2(694.0, 692.0));
    for _ in 0..3 {
        draw(&mut i, Beside::Outside(Some(room)));
    }
    let panel = i.ctx.memory(|m| m.area_rect(id)).unwrap();
    assert!(room.expand(0.5).contains_rect(panel), "{room:?} {panel:?}");
    assert_eq!(panel.left(), room.left());
    assert!((panel.bottom() - room.bottom()).abs() < 0.5, "{panel:?}");
    assert!((panel.width() - room.width()).abs() < 0.5, "{panel:?}");
}

#[test]
fn settings_follow_the_free_side_and_bottom_of_the_inspector() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    // Floating bars use a smaller work area for initial placement while the
    // editor can move across the whole screen.
    crate::ui::layout::set_work_area(&i.ctx, egui::Rect::from_min_max(egui::pos2(8.0, 60.0), egui::pos2(1272.0, 800.0)));
    crate::ui::layout::set_move_area(&i.ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, i.screen));
    for x in [12.0, 880.0] {
        crate::ui::widgets::place_floater(&i.ctx, super::super::super::INSPECTOR_ID, egui::pos2(x, 40.0), vec2(350.0, 820.0));
        settle(&mut i);
        let main = i.show(vec![]).0;
        let settings = i.ctx.memory(|m| m.area_rect(egui::Id::unique("construction-settings"))).unwrap();
        assert!((settings.bottom() - main.bottom()).abs() < 1.5, "{main:?} {settings:?}");
        if x < 500.0 {
            assert!(settings.left() >= main.right());
        } else {
            assert!(settings.right() <= main.left());
        }
    }
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let (_, texts) = i.show(vec![]);
    let settings = i.ctx.memory(|m| m.area_rect(egui::Id::unique("construction-settings"))).unwrap();
    for key in ["build-layout-export-notecard", "build-layout-finish", "build-layout-ruler"] {
        let rect = texts.iter().find(|(text, _)| text == aurora_i18n::text(key)).unwrap().1;
        assert!(
            settings.contains_rect(rect),
            "Organization controls must use the available height: {key}"
        );
    }
}

#[test]
fn profile_roundtrip_restores_settings_without_changing_object_transforms() {
    let (mut world, mut tool, mut settings) = fixture();
    settings.layout.place(Item::Shape, Panel::Hidden, None, false);
    settings.layout.place(Item::Actions, Panel::Settings, Some(Item::OwnedOnly), false);
    settings
        .layout
        .place_in_footer(Item::RootAxis, FooterColumn::Left, Some(Item::RootAxis), false);
    settings.grid_resolution = 1.25;
    settings.scale_uniform = true;
    settings.set_snap_mode(crate::build::SnapMode::Objects);
    settings.set_snap_enabled(false);
    let text = serde_json::to_string(&Profile::capture(&tool, &settings)).unwrap();
    let expected = settings.clone();
    let idx = tool.sel_prims(&world)[0];
    let object = world.objects.get(idx).unwrap();
    let transform = (object.position, object.scale, object.rotation);
    settings.layout = Default::default();
    settings.grid_resolution = 0.5;
    settings.scale_uniform = false;
    Profile::read(&text).unwrap().apply(&mut tool, &mut world, &mut settings);
    assert_eq!(settings, expected);
    let object = world.objects.get(idx).unwrap();
    assert_eq!(transform, (object.position, object.scale, object.rotation));
    assert!(!text.contains("position"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut old = value.clone();
    old["layout"].as_object_mut().unwrap().remove("footerColumns");
    let migrated = Profile::read(&old.to_string()).unwrap();
    assert!(migrated.layout.footer_columns.contains_key(&Item::RootAxis));
    let mut changed = value.clone();
    changed["layout"]["footerColumns"]["face"] = serde_json::json!("right");
    assert_eq!(
        Profile::read(&changed.to_string()).unwrap().layout.footer_columns[&Item::Face],
        FooterColumn::Right
    );
    changed["layout"]["footerColumns"]["actions"] = serde_json::json!("right");
    assert!(Profile::read(&changed.to_string()).is_err());
    for (field, bad) in [("version", serde_json::json!(20)), ("format", serde_json::json!("other"))] {
        let mut changed = value.clone();
        changed[field] = bad;
        assert!(Profile::read(&changed.to_string()).is_err());
    }
    let mut changed = value.clone();
    changed["layout"]["order"] = serde_json::json!(["face", "face"]);
    assert!(Profile::read(&changed.to_string()).is_err());
    let mut changed = value.clone();
    changed["layout"]["locations"]["nav-section-general"] = serde_json::json!("footer");
    assert!(Profile::read(&changed.to_string()).is_err());
    let mut changed = value;
    changed["ruler"]["gridUnit"] = serde_json::json!(-1.0);
    assert!(Profile::read(&changed.to_string()).is_err());
    let before = settings.clone();
    tool.ui.notecards.pending_load = Some(uuid::Uuid::from_u128(4));
    assert!(super::super::super::layout_notecard_loaded(
        &mut tool,
        &mut world,
        &mut settings,
        uuid::Uuid::from_u128(4),
        &Ok(changed.to_string().into_bytes())
    ));
    assert_eq!(settings, before);
}

#[test]
fn references_stay_horizontal_below_actions_and_do_not_open_the_values() {
    let mut i = Inspector::new();
    for expanded in [true, false] {
        i.settings.transform_expanded = expanded;
        settle(&mut i);
        let references: Vec<_> = (0u8..3)
            .map(|mode| {
                i.ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_grid_mode", mode))))
                    .unwrap()
            })
            .collect();
        assert!((references[0].center().y - references[2].center().y).abs() < 1.0);
        assert!(references[0].right() < references[1].left());
        assert!(row(&i, Item::Actions, Panel::Footer).bottom() < references[0].top());
        click(&mut i, references[1].center());
        assert_eq!(i.settings.grid_mode, 1);
        assert_eq!(i.settings.transform_expanded, expanded);
    }
}

#[test]
fn complete_controls_and_numeric_panels_follow_their_content_height() {
    for width in [344.0, 560.0] {
        let mut i = Inspector::new();
        i.screen = vec2(1280.0, 1400.0);
        i.settings.transform_expanded = true;
        resize_inspector(&mut i, vec2(width, 1160.0));
        let initial = i.tool.ui.layout.footer_height.unwrap();
        for item in crate::build::layout::ITEMS.into_iter().filter(|item| item.footer_column_item()) {
            i.settings
                .layout
                .place_in_footer(item, FooterColumn::Left, Some(Item::Actions), false);
        }
        settle(&mut i);
        assert!(i.tool.ui.layout.footer_height.unwrap() > initial);
        let controls = i
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("build-controls-viewport")))
            .unwrap();
        let vectors = i
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("build-transform-viewport")))
            .unwrap();
        for item in i.settings.layout.in_panel(Panel::Footer) {
            let control = row(&i, item, Panel::Footer);
            assert!(controls.contains_rect(control), "{item:?} outside its complete controls container");
            assert!(
                control.height() >= crate::ui::widgets::TEXT_BUTTON_HEIGHT - 0.1,
                "{item:?} was clipped"
            );
        }
        let part = i
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("build_part_navigation", Item::Face))))
            .unwrap();
        assert!(
            controls.contains_rect(part),
            "Shared part navigation outside the controls container"
        );
        for which in 0usize..3 {
            let fields = i
                .ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("build-vector-fields", which))))
                .unwrap();
            assert!(
                vectors.contains_rect(fields),
                "Vector {which} clipped at width {width}: {vectors:?}, {fields:?}"
            );
        }
        i.settings.layout = Default::default();
        settle(&mut i);
        assert_eq!(
            i.tool.ui.layout.footer_height.unwrap(),
            initial,
            "Removing options must release the container height"
        );
    }
}

#[test]
fn build_status_is_temporary_below_the_window_without_resizing_its_contents() {
    let mut i = Inspector::new();
    settle(&mut i);
    let (before, _) = i.show(vec![]);
    let height = i.tool.ui.layout.footer_height;
    let message = "Synthetic build status notice";
    i.tool.status = message.into();
    settle(&mut i);
    let (window, texts) = i.show(vec![]);
    assert!(i.tool.status.is_empty());
    assert_eq!(window, before);
    assert_eq!(i.tool.ui.layout.footer_height, height);
    let notice = i
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique("construction-status-notice")))
        .unwrap();
    let text = texts.iter().find(|(text, _)| text == message).unwrap().1;
    assert!(
        notice.top() > window.bottom(),
        "Notice must appear below the editor: {notice:?}, {window:?}"
    );
    assert!(notice.contains_rect(text));
    i.frame += 7 * 60;
    let (_, texts) = i.show(vec![]);
    assert!(i.tool.ui.status_notice.is_none());
    assert!(!texts.iter().any(|(text, _)| text == message));
}

#[test]
fn notecard_callbacks_require_their_own_request_and_keep_settings_on_failure() {
    let (mut world, mut tool, mut settings) = fixture();
    let request = uuid::Uuid::from_u128(40);
    let unrelated = uuid::Uuid::from_u128(41);
    tool.ui.notecards.pending_save = Some(request);
    let result = Err("Synthetic upload failure".into());
    assert!(!super::super::super::layout_notecard_saved(&mut tool, unrelated, &result));
    assert_eq!(tool.ui.notecards.pending_save, Some(request));
    assert!(super::super::super::layout_notecard_saved(&mut tool, request, &result));
    assert!(tool.ui.notecards.pending_save.is_none());
    let before = settings.clone();
    tool.ui.notecards.pending_load = Some(request);
    assert!(!super::super::super::layout_notecard_loaded(
        &mut tool,
        &mut world,
        &mut settings,
        unrelated,
        &Ok(Vec::new())
    ));
    assert!(super::super::super::layout_notecard_loaded(
        &mut tool,
        &mut world,
        &mut settings,
        request,
        &Err("Synthetic download failure".into())
    ));
    assert_eq!(settings, before);
}
