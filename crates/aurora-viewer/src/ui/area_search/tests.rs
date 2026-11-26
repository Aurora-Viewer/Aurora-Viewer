use super::*;

fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, label)),
        _ => None,
    }
}

struct Harness {
    ctx: egui::Context,
    p: Palette,
    ui: AreaSearchUi,
    time: f64,
    size: Vec2,
    draw_distance: f32,
    /// What RLVa hides of the right-click menu (none by default).
    locks: MenuLocks,
}
impl Harness {
    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Option<Action>) {
        self.time += 0.1;
        let mut action = None;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                let locks = self.locks;
                let env = Env {
                    parcel_known: false,
                    parcel_part: true,
                    neighbours: true,
                    draw_distance: self.draw_distance,
                    locks: &|_| locks,
                    pics: &HashMap::new(),
                    facts: None,
                };
                action = self.ui.show(&self.ctx, &self.p, &mut true, &env);
            },
        );
        (crate::ui::login::test_output(output), action)
    }
    fn find(&mut self, label: &str) -> egui::Rect {
        let (out, _) = self.frame(Vec::new());
        out.shapes
            .iter()
            .rev()
            .find_map(|s| text_rect(&s.shape, label))
            .unwrap_or_else(|| panic!("missing painted text: {label}"))
    }
    fn shown(&mut self, label: &str) -> bool {
        let (out, _) = self.frame(Vec::new());
        out.shapes.iter().any(|s| text_rect(&s.shape, label).is_some())
    }
    fn settle(&mut self) {
        for _ in 0..8 {
            self.frame(Vec::new());
        }
    }
    fn click(&mut self, label: &str) -> (egui::FullOutput, Option<Action>) {
        let pos = self.find(label).center();
        self.click_at(pos)
    }
    fn click_at(&mut self, pos: egui::Pos2) -> (egui::FullOutput, Option<Action>) {
        self.press(pos, egui::PointerButton::Primary, Default::default())
    }
    fn press(&mut self, pos: egui::Pos2, button: egui::PointerButton, modifiers: egui::Modifiers) -> (egui::FullOutput, Option<Action>) {
        let mut result = None;
        for pressed in [true, false] {
            result = Some(self.frame(vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                },
            ]));
        }
        self.frame(vec![egui::Event::ModifiersChanged(Default::default())]);
        result.unwrap()
    }
    /// A right click on the row of `label`, then a click on the menu's `entry`.
    fn menu(&mut self, label: &str, entry: &str) -> Option<Action> {
        let pos = self.find(label).center();
        self.press(pos, egui::PointerButton::Secondary, Default::default());
        self.click(entry).1
    }
    fn new(scale: f32, size: Vec2, count: usize) -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, scale);
        let mut ui = AreaSearchUi::default();
        let rows = (1..=count)
            .map(|i| {
                let mut r = Row::default();
                r.id = Uuid::from_u128(i as u128);
                r.name = format!("Object {i}");
                r.description = "Searchable object".into();
                r.owner = "Demo owner".into();
                r.owner_id = Uuid::from_u128(0xAA);
                r.creator = "Demo maker".into();
                r.creator_id = Uuid::from_u128(0xBB);
                r.loaded = true;
                r.distance = i as f32;
                r
            })
            .collect();
        ui.table.replace(rows);
        let mut h = Self {
            ctx,
            p: theme.palette(),
            ui,
            time: 0.0,
            size,
            draw_distance: 64.0,
            locks: MenuLocks::default(),
        };
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        h
    }
}

/// The window as the checks of the rows and the menu want it.
fn both(check: impl Fn(Harness)) {
    check(Harness::new(1.0, Vec2::new(1280.0, 900.0), 40));
}

#[test]
fn area_search_rows_select_and_dispatch_inspect_zoom_and_copy() {
    both(|mut h| {
        h.click("Object 1");
        assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(1)));
        assert!(matches!(h.click(aurora_i18n::tr!("area-search-inspect")).1, Some(Action::Inspect(ids)) if ids == [Uuid::from_u128(1)]));
        assert!(matches!(h.click(aurora_i18n::tr!("area-search-zoom")).1, Some(Action::Zoom(id)) if id == Uuid::from_u128(1)));
        let (out, _) = h.click(aurora_i18n::tr!("area-search-copy"));
        assert!(
            out.platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(s) if *s == Uuid::from_u128(1).to_string()))
        );
        h.ui.table.filters.query = "does not match".into();
        h.frame(Vec::new());
        assert!(h.ui.selection.is_empty(), "a filtered-out selection cannot trigger an action");
        assert!(
            !h.shown(aurora_i18n::tr!("area-search-inspect")),
            "nothing selected, nothing to act on"
        );
        h.ui.table.filters.query.clear();
        h.settle();
        assert_eq!(h.ui.table.visible.len(), 40);
    });
}

#[test]
fn area_search_select_several_makes_clicks_add() {
    both(|mut h| {
        // the row picked for the user is no part of what they choose
        assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(1)));
        h.click(aurora_i18n::tr!("area-search-several"));
        h.click("Object 2");
        assert_eq!(h.ui.selection.ids, [Uuid::from_u128(2)]);
        h.click("Object 2");
        h.settle();
        assert!(h.ui.selection.is_empty(), "taken out by hand, none is picked");
        h.click("Object 1");
        h.click("Object 3");
        assert_eq!(h.ui.selection.ids, [Uuid::from_u128(1), Uuid::from_u128(3)]);
        h.click("Object 1");
        assert_eq!(h.ui.selection.ids, [Uuid::from_u128(3)]);
    });
}

#[test]
fn area_search_keyboard_moves_in_the_list_and_selects() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 2000);
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    h.click("Object 1");
    // the list has one stop for Tab: the row last used
    let id = egui::Id::unique((
        "board_nav",
        h.ctx.memory(|m| m.layer_ids().find(|l| l.id == egui::Id::unique(WINDOW))).unwrap(),
        egui::Id::unique("area_search_objects"),
    ));
    let at = h.ctx.data(|d| d.get_temp::<usize>(id)).expect("the list's place");
    h.ctx.memory_mut(|m| m.request_focus(id.with(at)));
    h.frame(Vec::new());
    h.frame(vec![key(egui::Key::ArrowDown, Default::default())]);
    h.frame(Vec::new());
    h.frame(vec![key(egui::Key::Enter, Default::default())]);
    assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(2)));
    h.frame(vec![key(egui::Key::End, Default::default())]);
    h.settle();
    h.frame(vec![key(egui::Key::Enter, Default::default())]);
    assert_eq!(
        h.ui.selection.single(),
        Some(Uuid::from_u128(2000)),
        "the last row of a long list is reached"
    );
    h.frame(vec![
        egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
        key(egui::Key::A, egui::Modifiers::COMMAND),
    ]);
    assert_eq!(h.ui.selection.ids.len(), 2000);
}

#[test]
fn area_search_menu_acts_on_the_selected_rows_like_the_legacy_viewer() {
    let id = |n| Uuid::from_u128(n);
    {
        let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 40);
        h.ui.card_open = false;
        h.click("Object 1");
        h.settle();
        let third = h.find("Object 3").center();
        h.press(third, egui::PointerButton::Primary, egui::Modifiers::COMMAND);
        assert_eq!(h.ui.selection.ids, [id(1), id(3)]);
        let fifth = h.find("Object 5").center();
        h.press(fifth, egui::PointerButton::Primary, egui::Modifiers::SHIFT);
        assert_eq!(h.ui.selection.ids, [id(3), id(4), id(5)], "Shift selects from the last clicked row");
        h.press(fifth, egui::PointerButton::Primary, egui::Modifiers::COMMAND);
        h.press(third, egui::PointerButton::Primary, egui::Modifiers::COMMAND);
        h.click("Object 1");
        h.press(third, egui::PointerButton::Primary, egui::Modifiers::COMMAND);

        // several rows: every one, and the single-object entries greyed
        let edit = h.menu("Object 3", aurora_i18n::tr!("common-edit"));
        assert!(
            matches!(edit, Some(Action::Edit(ids)) if ids == [id(1), id(3)]),
            "a right click in the selection keeps it"
        );
        assert!(h.ui.menu_at.is_none(), "the menu closes on its entry");
        for (entry, check) in [
            (
                aurora_i18n::tr!("ui-context-touch"),
                (|a: &Action| matches!(a, Action::Touch(ids) if ids.len() == 2)) as fn(&Action) -> bool,
            ),
            (
                aurora_i18n::tr!("common-delete"),
                |a| matches!(a, Action::Delete(ids) if ids.len() == 2),
            ),
            (
                aurora_i18n::tr!("ui-context-return"),
                |a| matches!(a, Action::Return(ids) if ids.len() == 2),
            ),
            (
                aurora_i18n::tr!("ui-context-derender-blacklist"),
                |a| matches!(a, Action::Blacklist(ids) if ids.len() == 2),
            ),
            (
                aurora_i18n::tr!("ui-context-script-information"),
                |a| matches!(a, Action::ScriptInfo(ids) if ids.len() == 2),
            ),
            (aurora_i18n::tr!("area-search-filter-mine"), |a| matches!(a, Action::FilterMine)),
        ] {
            let action = h.menu("Object 1", entry);
            assert!(action.as_ref().is_some_and(check), "{entry}");
        }
        for entry in [aurora_i18n::tr!("area-search-sit"), aurora_i18n::tr!("common-teleport-to")] {
            assert!(h.menu("Object 1", entry).is_none(), "{entry} needs one row");
            egui::Popup::close_all(&h.ctx);
            h.frame(Vec::new());
        }

        assert!(h.menu("Object 5", aurora_i18n::tr!("common-teleport-to")).is_none());
        assert_eq!(h.ui.selection.ids, [id(1), id(3)], "a right click keeps several selected rows");
        egui::Popup::close_all(&h.ctx);
        h.frame(Vec::new());

        // one selected row: a right click on another selects it instead
        h.click("Object 2");
        let teleport = h.menu("Object 5", aurora_i18n::tr!("common-teleport-to"));
        assert!(matches!(teleport, Some(Action::Teleport(i)) if i == id(5)));
        assert_eq!(h.ui.selection.ids, [id(5)]);
        assert!(matches!(h.menu("Object 5", aurora_i18n::tr!("area-search-sit")), Some(Action::Sit(i)) if i == id(5)));
        assert!(h.menu("Object 5", aurora_i18n::tr!("common-buy-object")).is_none(), "not for sale");
        egui::Popup::close_all(&h.ctx);
        h.ui.table.rows[4].price = Some(10);
        h.ui.table.dirty = true;
        assert!(matches!(h.menu("Object 5", aurora_i18n::tr!("common-buy-object")), Some(Action::Buy(i)) if i == id(5)));

        assert!(h.menu("Object 2", aurora_i18n::tr!("common-select-all")).is_none());
        assert_eq!(h.ui.selection.ids.len(), 40);
        assert!(h.menu("Object 2", aurora_i18n::tr!("area-search-clear-selection")).is_none());
        assert!(h.ui.selection.is_empty());
    }
}

#[test]
fn area_search_menu_zoom_needs_the_object_within_draw_distance() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 800.0), 40);
    let pos = h.find("Object 5").center();
    // the menu's Zoom, below its Teleport entry, does nothing past the draw distance
    let menu_zoom = |h: &mut Harness| {
        h.press(pos, egui::PointerButton::Secondary, Default::default());
        let (out, _) = h.frame(Vec::new());
        let teleport = out
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, aurora_i18n::tr!("common-teleport-to")))
            .expect("menu shown");
        let zoom = out
            .shapes
            .iter()
            .filter_map(|s| text_rect(&s.shape, aurora_i18n::tr!("common-zoom-2")))
            .filter(|r| r.top() > teleport.top())
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("menu Zoom entry");
        let action = h.click_at(zoom.center()).1;
        egui::Popup::close_all(&h.ctx);
        h.frame(Vec::new());
        action
    };
    h.draw_distance = 4.0;
    assert!(menu_zoom(&mut h).is_none());
    h.draw_distance = 64.0;
    assert!(matches!(menu_zoom(&mut h), Some(Action::Zoom(id)) if id == Uuid::from_u128(5)));
}

#[test]
fn area_search_filter_crosses_clear_only_their_field() {
    {
        let mut h = Harness::new(1.0, Vec2::new(1280.0, 1100.0), 40);
        h.ui.demo_controls("filters");
        h.ui.table.filters.name = "no match".into();
        h.ui.table.filters.creator = "no match".into();
        h.settle();
        for i in [0usize, 4] {
            let id = egui::Id::unique(("area_search_field", i));
            let clear = h.ctx.read_response(id.with("clear")).expect("filled filter's clear button");
            h.click_at(clear.rect.center());
            h.settle();
        }
        let f = &h.ui.table.filters;
        assert!(f.name.is_empty() && f.creator.is_empty());
        assert_eq!(h.ui.table.visible.len(), 40, "the results come back at once");
    }
}

#[test]
fn area_search_long_results_draw_only_what_shows_and_keep_the_footer_in_the_window() {
    for (scale, size) in [(1.0, Vec2::new(1280.0, 900.0)), (1.5, Vec2::new(1280.0, 900.0))] {
        {
            let mut h = Harness::new(scale, size, 2000);
            h.settle();
            let (out, _) = h.frame(Vec::new());
            let painted = (1..=2000)
                .filter(|i| out.shapes.iter().any(|s| text_rect(&s.shape, &format!("Object {i}")).is_some()))
                .count();
            assert!((1..60).contains(&painted), "only the viewport's rows: {painted} ({scale})");
            // (a row is picked for the user: the footer has its buttons)
            let rect = h.find(aurora_i18n::tr!("area-search-actions"));
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                "the footer left the screen at {scale}: {rect:?}"
            );
        }
    }
}

#[test]
fn area_search_kind_tiles_go_from_only_to_hidden_to_any() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 40);
    h.ui.table.rows[0].flags = 1 << 10;
    h.ui.table.dirty = true;
    let phantom = aurora_i18n::tr!("area-search-phantom");
    h.click(phantom);
    assert!(h.ui.table.filters.phantom);
    h.settle();
    assert_eq!(h.ui.table.visible.len(), 1);
    h.click(phantom);
    assert!(h.ui.table.filters.exclude_phantom && !h.ui.table.filters.phantom);
    h.settle();
    assert_eq!(h.ui.table.visible.len(), 39);
    h.click(phantom);
    h.settle();
    assert_eq!(h.ui.table.visible.len(), 40);
}

#[test]
fn area_search_sheet_saves_resets_and_copies_the_link() {
    {
        let mut h = Harness::new(1.0, Vec2::new(1280.0, 1000.0), 40);
        h.ui.table.filters.query = "Object".into();
        h.ui.table.filters.sale = true;
        h.ui.demo_controls("filters");
        h.settle();
        h.click(aurora_i18n::tr!("area-search-save-defaults"));
        assert!(h.ui.save_defaults);
        let (out, _) = h.click(aurora_i18n::tr!("area-search-copy-link"));
        assert!(
            out.platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(s) if s.starts_with("secondlife:///app/openfloater/area_search")))
        );
        h.click(aurora_i18n::tr!("common-reset"));
        assert!(!h.ui.table.filters.sale);
        assert_eq!(h.ui.table.filters.query, "Object", "the search text stays");
    }
}

#[test]
fn area_search_separate_resets_preserve_the_other_sections() {
    let filled = Filters {
        query: "Object".into(),
        owner: "Demo owner".into(),
        phantom: true,
        click_action: 2,
        sale: true,
        min_price: 2,
        max_price: 3,
        distance_on: true,
        min_distance: 1.0,
        max_distance: 8.0,
        parcel_only: true,
        children: true,
        exclude_physical: true,
        ..Default::default()
    };
    let after = |section| {
        let mut ui = AreaSearchUi::default();
        ui.table.filters = filled.clone();
        ui.visible_columns = Some([true; 10]);
        ui.reset_section(section);
        ui
    };
    let f = after(Section::Search).table.filters;
    assert!(f.query.is_empty() && f.owner.is_empty() && f.phantom && f.sale && f.distance_on);
    let f = after(Section::Filters).table.filters;
    assert_eq!((&*f.query, &*f.owner), ("Object", "Demo owner"));
    assert!(!f.phantom && f.click_action == 0 && !f.parcel_only && !f.children && !f.exclude_physical);
    assert!(f.sale && f.distance_on && (f.min_price, f.max_price) == (2, 3));
    let f = after(Section::Ranges).table.filters;
    assert!(!f.sale && !f.distance_on && f.phantom && f.parcel_only && f.click_action == 2);
    let ui = after(Section::Columns);
    assert_eq!(ui.saved_columns(), DEFAULT_COLUMNS);
    assert!(ui.table.filters.phantom);
    assert_eq!(after(Section::All).table.filters, Filters::default());
}

#[test]
fn area_search_columns_are_chosen_kept_and_the_name_stays_first() {
    {
        let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 40);
        assert_eq!(
            h.ui.shown_columns(),
            [Column::Name, Column::Distance, Column::LandImpact, Column::Owner]
        );
        h.ui.demo_controls("table");
        h.settle();
        h.click(aurora_i18n::tr!("area-search-last-owner"));
        h.settle();
        let columns = h.ui.saved_columns();
        assert!(columns[Column::LastOwner.index()]);
        let saved = AreaSearch {
            filters: h.ui.table.filters.without_search(),
            columns,
        };
        let restored = AreaSearchUi::from_saved(&serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap());
        assert!(restored.saved_columns()[Column::LastOwner.index()]);
        h.click(aurora_i18n::tr!("area-search-reset-columns"));
        assert_eq!(h.ui.saved_columns(), DEFAULT_COLUMNS);
        h.ui.visible_columns = Some([false; 10]);
        assert_eq!(h.ui.shown_columns(), [Column::Name]);
    }
}

#[test]
fn area_search_regex_error_is_said_and_recovers_when_corrected() {
    both(|mut h| {
        h.ui.table.filters.regex = true;
        h.ui.table.filters.name = "(".into();
        h.frame(Vec::new());
        assert!(h.ui.table.regex_error.is_some());
        assert!(h.find(aurora_i18n::tr!("area-search-regex-invalid")).is_positive());
        h.ui.table.filters.name = "Object .*".into();
        h.frame(Vec::new());
        assert!(h.ui.table.regex_error.is_none());
        assert!(!h.shown(aurora_i18n::tr!("area-search-regex-invalid")));
    });
}

#[test]
fn area_search_where_to_look_is_one_click() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 10);
    for (n, row) in h.ui.table.rows.iter_mut().enumerate() {
        row.on_parcel = n < 4;
    }
    h.ui.table.dirty = true;
    h.settle();
    h.click(aurora_i18n::tr!("area-search-scope-parcel"));
    h.settle();
    assert!(h.ui.table.filters.parcel_only);
    assert_eq!(h.ui.table.visible.len(), 4);
    h.click(aurora_i18n::tr!("area-search-scope-neighbours"));
    h.settle();
    assert!(!h.ui.table.filters.parcel_only && h.ui.table.filters.neighbors);
    h.click(aurora_i18n::tr!("area-search-scope-region"));
    h.settle();
    assert!(!h.ui.table.filters.neighbors);
    assert_eq!(h.ui.table.visible.len(), 10);
}

#[test]
fn area_search_waits_for_the_parcel_aloud() {
    both(|mut h| {
        h.ui.table.filters.parcel_only = true;
        h.frame(Vec::new());
        assert!(h.find(aurora_i18n::tr!("area-search-parcel-pending")).is_positive());
    });
}

#[test]
fn area_search_an_owner_narrows_everything_and_lets_go() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 40);
    for row in &mut h.ui.table.rows[..5] {
        row.owner_id = Uuid::from_u128(0xCC);
        row.owner = "Other owner".into();
    }
    h.ui.table.dirty = true;
    h.settle();
    h.click("Other owner");
    h.settle();
    assert_eq!(h.ui.table.visible.len(), 5);
    h.click("Other owner");
    h.settle();
    assert_eq!(h.ui.table.visible.len(), 40);
    // an owner whose objects all left holds nothing back
    h.ui.table.owner = Some(Uuid::from_u128(0xDD));
    h.settle();
    assert_eq!(h.ui.table.visible.len(), 40);
}

#[test]
fn area_search_card_opens_profiles_but_not_under_shownames() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 900.0), 40);
    h.ui.card_open = true;
    h.click("Object 1");
    h.settle();
    assert!(matches!(h.click("Demo maker").1, Some(Action::Profile(id)) if id == Uuid::from_u128(0xBB)));
    for row in &mut h.ui.table.rows {
        row.names_hidden = true;
    }
    h.settle();
    assert!(h.click("Demo maker").1.is_none());
    // a group that owns opens its own profile
    for row in &mut h.ui.table.rows {
        row.names_hidden = false;
        row.group_id = row.owner_id;
    }
    h.ui.table.dirty = true;
    h.settle();
    let at = h.find(aurora_i18n::tr!("inspect-owned-by")).center();
    assert!(matches!(h.click_at(at).1, Some(Action::Group(id)) if id == Uuid::from_u128(0xAA)));
    for row in &mut h.ui.table.rows {
        row.names_hidden = true;
    }
    h.settle();
    assert!(h.click_at(at).1.is_none(), "a hidden group stays closed");
}

#[test]
fn area_search_windows_keep_their_size() {
    {
        let rows: Vec<Row> = (1..=60u128)
            .map(|i| {
                let mut r = Row::default();
                r.id = Uuid::from_u128(i);
                r.name = format!("Object {i}");
                r.loaded = true;
                r.distance = i as f32;
                r
            })
            .collect();
        // as it opens, and the least it can be
        let sizes = &[SIZE, MIN_SIZE];
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            sizes,
            aurora_i18n::tr!("area-search-actions"),
            || {
                let mut ui = AreaSearchUi::default();
                ui.table.replace(rows.clone());
                ui
            },
            |ui, ctx, p| {
                let env = Env {
                    parcel_known: true,
                    parcel_part: true,
                    neighbours: true,
                    draw_distance: 64.0,
                    locks: &|_| MenuLocks::default(),
                    pics: &HashMap::new(),
                    facts: None,
                };
                ui.show(ctx, p, &mut true, &env);
            },
        );
    }
}

fn count_text(shape: &egui::Shape, label: &str) -> usize {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => 1,
        egui::Shape::Vec(shapes) => shapes.iter().map(|s| count_text(s, label)).sum(),
        _ => 0,
    }
}

/// FSPanelAreaSearchList::onContextMenuItemVisibleRLV with the `on_visible`
/// parameters of menu_fs_area_search.xml: an entry goes (it is not greyed)
/// when RLVa refuses one of its words.
#[test]
fn area_search_menu_hides_the_entries_rlva_refuses() {
    let script = aurora_i18n::tr!("ui-context-script-information");
    let touch = aurora_i18n::tr!("ui-context-touch");
    let sit = aurora_i18n::tr!("area-search-sit");
    let teleport = aurora_i18n::tr!("common-teleport-to");
    let zoom = aurora_i18n::tr!("common-zoom-2");
    let inspect = aurora_i18n::tr!("ui-context-inspect-2");
    let edit = aurora_i18n::tr!("common-edit");
    let ret = aurora_i18n::tr!("ui-context-return");
    let delete = aurora_i18n::tr!("common-delete");
    let blacklist = aurora_i18n::tr!("ui-context-derender-blacklist");
    let buy = aurora_i18n::tr!("common-buy-object");
    let mine = aurora_i18n::tr!("area-search-filter-mine");
    let select_all = aurora_i18n::tr!("common-select-all");
    let clear = aurora_i18n::tr!("area-search-clear-selection");
    let all = [
        script, touch, sit, teleport, zoom, inspect, edit, ret, delete, blacklist, buy, mine, select_all, clear,
    ];
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 800.0), 40);
    // how often each label is painted with the menu open on one row
    let painted = |h: &mut Harness, locks: MenuLocks| -> Vec<usize> {
        h.locks = locks;
        let pos = h.find("Object 5").center();
        h.press(pos, egui::PointerButton::Secondary, Default::default());
        let (out, _) = h.frame(Vec::new());
        let counts = all
            .iter()
            .map(|label| out.shapes.iter().map(|s| count_text(&s.shape, label)).sum())
            .collect();
        egui::Popup::close_all(&h.ctx);
        h.frame(Vec::new());
        counts
    };
    let free = painted(&mut h, MenuLocks::default());
    assert!(free.iter().all(|n| *n >= 1), "every entry is in the menu: {free:?}");
    let one = |f: fn(&mut MenuLocks)| {
        let mut locks = MenuLocks::default();
        f(&mut locks);
        locks
    };
    for (locks, hidden) in [
        (one(|l| l.touch = true), vec![touch, ret, delete, buy]),
        (one(|l| l.edit = true), vec![script, inspect, edit, buy]),
        (one(|l| l.delete = true), vec![ret, delete, blacklist]),
        (one(|l| l.sit = true), vec![sit]),
        (one(|l| l.teleport = true), vec![teleport]),
        (one(|l| l.buy = true), vec![buy]),
        // @interact, or a row whose object is gone: Zoom and the list's own
        // entries are all that is left
        (
            MenuLocks::ALL,
            vec![script, touch, sit, teleport, inspect, edit, ret, delete, blacklist, buy],
        ),
    ] {
        let counts = painted(&mut h, locks);
        for (i, label) in all.iter().enumerate() {
            let expected = free[i] - usize::from(hidden.contains(label));
            assert_eq!(counts[i], expected, "{label} under {locks:?}");
        }
    }
    // nothing refused any more: the entries are back and answer
    h.locks = MenuLocks::default();
    assert!(matches!(h.menu("Object 5", touch), Some(Action::Touch(ids)) if ids == [Uuid::from_u128(5)]));
}

/// A result list that came anew, as the app makes one every half second.
fn again(h: &mut Harness, change: impl Fn(&mut Vec<Row>)) {
    let mut rows = h.ui.table.rows.clone();
    change(&mut rows);
    h.ui.table.replace(rows);
    h.settle();
}

#[test]
fn area_search_picks_the_top_row_until_the_user_chooses() {
    let id = |n| Some(Uuid::from_u128(n));
    both(|mut h| {
        assert_eq!(h.ui.selection.single(), id(1), "as it opens");
        // another search, another order
        h.ui.table.filters.query = "object 3".into();
        h.settle();
        assert_eq!(h.ui.selection.single(), id(3));
        h.ui.table.filters.query.clear();
        h.settle();
        assert_eq!(
            h.ui.selection.single(),
            id(1),
            "a search cleared: the top row, though 3 still shows"
        );
        h.ui.table.sort_by(Column::Distance);
        h.settle();
        assert_eq!(h.ui.selection.single(), id(40));
        h.ui.table.sort_by(Column::Distance);
        h.settle();
        assert_eq!(h.ui.selection.single(), id(1));
        // nothing found, nothing picked
        h.ui.table.filters.query = "does not match".into();
        h.settle();
        assert!(h.ui.selection.is_empty());
        h.ui.table.filters.query.clear();
        h.settle();
        assert_eq!(h.ui.selection.single(), id(1));
        // the one picked goes: the next top row
        again(&mut h, |rows| rows.retain(|r| r.id != Uuid::from_u128(1)));
        assert_eq!(h.ui.selection.single(), id(2));
        // chosen: kept through searches that still show it
        h.click("Object 5");
        h.ui.table.filters.query = "object".into();
        h.settle();
        assert_eq!(h.ui.selection.single(), id(5));
        h.ui.table.sort_by(Column::Distance);
        h.settle();
        assert_eq!(h.ui.selection.single(), id(5));
        // and the top row once it does not
        h.ui.table.filters.query = "object 7".into();
        h.settle();
        assert_eq!(h.ui.selection.single(), id(7));
    });
}

#[test]
fn area_search_pick_follows_a_filling_list_but_not_a_moving_one_nor_under_the_pointer() {
    let id = |n| Some(Uuid::from_u128(n));
    let nearest = |n: usize| move |rows: &mut Vec<Row>| rows[n].distance = 0.1 / (n as f32);
    both(|mut h| {
        again(&mut h, nearest(7));
        assert_eq!(h.ui.table.rows[h.ui.table.visible[0]].id, Uuid::from_u128(8));
        assert_eq!(h.ui.selection.single(), id(1), "passing by changes nothing");
        h.ui.table.pending = 3;
        again(&mut h, |_| ());
        assert_eq!(h.ui.selection.single(), id(8), "the list fills");
        let over = h.find("Object 5").center();
        h.frame(vec![egui::Event::PointerMoved(over)]);
        again(&mut h, nearest(9));
        assert_eq!(h.ui.selection.single(), id(8), "never under the pointer");
        h.frame(vec![egui::Event::PointerGone]);
        again(&mut h, |_| ());
        assert_eq!(h.ui.selection.single(), id(10));
    });
}

#[test]
fn area_search_open_menu_never_moves_to_another_object() {
    both(|mut h| {
        let at = h.find("Object 1").center();
        h.press(at, egui::PointerButton::Secondary, Default::default());
        assert!(h.ui.menu_at.is_some());
        again(&mut h, |rows| rows.retain(|r| r.id != Uuid::from_u128(1)));
        assert!(h.ui.selection.is_empty(), "its object is gone: nothing to delete");
        egui::Popup::close_all(&h.ctx);
        h.settle();
        again(&mut h, |_| ());
        assert!(h.ui.selection.is_empty(), "chosen: the next object never takes its place");
    });
}

#[test]
fn area_search_a_chosen_row_that_goes_leaves_nothing_to_act_on() {
    both(|mut h| {
        // Delete on a chosen row, the object gone: a second Delete has nothing
        h.click("Object 3");
        again(&mut h, |rows| rows.retain(|r| r.id != Uuid::from_u128(3)));
        assert!(h.ui.selection.is_empty());
        assert!(!h.shown(aurora_i18n::tr!("area-search-actions")), "no Actions without a selection");
        assert!(
            h.menu("Object 1", aurora_i18n::tr!("common-delete"))
                .is_some_and(|a| matches!(a, Action::Delete(ids) if ids == [Uuid::from_u128(1)])),
            "a right click chooses its row"
        );
    });
}

#[test]
fn area_search_pick_never_changes_under_a_click_or_a_key() {
    both(|mut h| {
        // another order, its list in a frame with a key: the pick waits
        h.ui.table.sort_by(Column::Distance);
        h.frame(vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(1)));
        h.frame(Vec::new());
        assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(40)), "taken in the next quiet frame");
    });
}

#[test]
fn area_search_pick_stops_following_a_list_still_filling_after_a_while() {
    let nearest = |n: usize| move |rows: &mut Vec<Row>| rows[n].distance = 0.1 / (n as f32);
    both(|mut h| {
        h.ui.table.pending = 3;
        // 10 s after the list was asked for (0.1 s a frame)
        for _ in 0..110 {
            h.frame(Vec::new());
        }
        again(&mut h, nearest(7));
        assert_eq!(
            h.ui.selection.single(),
            Some(Uuid::from_u128(1)),
            "names that never come do not move it"
        );
    });
}

#[test]
fn area_search_cleared_selection_stays_so_until_another_search() {
    both(|mut h| {
        h.menu("Object 2", aurora_i18n::tr!("area-search-clear-selection"));
        again(&mut h, |_| ());
        assert!(h.ui.selection.is_empty());
        h.ui.table.filters.query = "object 2".into();
        h.settle();
        assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(2)));
        // closed, then opened again
        h.menu("Object 2", aurora_i18n::tr!("area-search-clear-selection"));
        h.ui.closed();
        h.ui.selection.reset();
        again(&mut h, |_| ());
        assert_eq!(h.ui.selection.single(), Some(Uuid::from_u128(2)));
    });
}
