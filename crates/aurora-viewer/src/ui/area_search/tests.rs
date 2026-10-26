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
                action = self.ui.show(&self.ctx, &self.p, &mut true, true, self.draw_distance);
            },
        );
        (crate::ui::login::test_output(output), action)
    }
    fn find(&mut self, label: &str) -> egui::Rect {
        let (out, _) = self.frame(Vec::new());
        out.shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, label))
            .unwrap_or_else(|| panic!("missing painted text: {label}"))
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
        };
        for _ in 0..6 {
            h.frame(Vec::new());
        }
        h
    }
}

#[test]
fn area_search_native_rows_select_and_dispatch_inspect_zoom_and_copy() {
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 800.0), 2000);
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
    h.click(aurora_i18n::tr!("area-search-refresh"));
    assert!(h.ui.refresh);
    assert_eq!(h.ui.table.filters.query, "does not match");
    h.click("does not match");
    let query_id = h.ctx.memory(|m| m.focused()).expect("search field focus");
    let clear = h.ctx.read_response(query_id.with("clear")).expect("search clear button");
    h.click_at(clear.rect.center());
    assert!(h.ui.table.filters.query.is_empty());
    assert_eq!(h.ui.table.visible.len(), 2000);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(query_id));
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert!(h.ctx.read_response(query_id.with("clear")).is_none());
}

#[test]
fn area_search_menu_acts_on_the_selected_rows_like_the_legacy_viewer() {
    let id = |n| Uuid::from_u128(n);
    let mut h = Harness::new(1.0, Vec2::new(1280.0, 800.0), 40);
    h.click("Object 1");
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
fn area_search_native_filter_crosses_clear_only_their_field_and_restore_results() {
    for scale in [1.0, 1.5] {
        let mut h = Harness::new(scale, Vec2::new(1280.0, 900.0), 40);
        h.click(aurora_i18n::tr!("area-search-filters"));
        h.ui.table.filters.query = "Object 1".into();
        h.ui.table.filters.name = "no match".into();
        h.ui.table.filters.description = "no match".into();
        h.ui.table.filters.owner = "no match".into();
        h.ui.table.filters.creator = "no match".into();
        for _ in 0..10 {
            h.frame(Vec::new());
        }
        let fields = |h: &Harness| {
            [
                h.ui.table.filters.name.clone(),
                h.ui.table.filters.description.clone(),
                h.ui.table.filters.owner.clone(),
                h.ui.table.filters.creator.clone(),
            ]
        };
        for i in 0usize..4 {
            let id = egui::Id::unique(("area_search_field", i));
            let field = h.ctx.read_response(id).expect("filter input");
            let clear = h.ctx.read_response(id.with("clear")).expect("filled filter's clear button");
            assert!(field.rect.contains_rect(clear.rect), "clear button stays inside the input");
            assert_eq!(clear.rect.right(), field.rect.right());
            let mut expected = fields(&h);
            expected[i].clear();
            h.click_at(clear.rect.center());
            assert_eq!(fields(&h), expected);
            assert_eq!(h.ui.table.filters.query, "Object 1");
            assert_eq!(h.ctx.memory(|m| m.focused()), Some(id));
            h.frame(Vec::new());
            h.frame(Vec::new());
            assert!(h.ctx.read_response(id.with("clear")).is_none(), "empty inputs have no clear button");
        }
        assert_eq!(
            h.ui.table.visible.len(),
            11,
            "clearing individual filters immediately reapplies the remaining query"
        );
    }
}

#[test]
fn area_search_native_table_virtualizes_long_results_and_keeps_footer_in_the_window() {
    for (scale, size) in [(1.0, Vec2::new(1280.0, 720.0)), (1.5, Vec2::new(1280.0, 900.0))] {
        let mut h = Harness::new(scale, size, 2000);
        let window_size = |ctx: &egui::Context| {
            ctx.data_mut(|d| d.get_persisted::<Vec2>(egui::Id::unique(WINDOW).with("expanded_size")))
                .expect("shown window size")
        };
        let initial = window_size(&h.ctx);
        for _ in 0..60 {
            h.frame(Vec::new());
        }
        let settled = window_size(&h.ctx);
        assert!(
            (settled.x - initial.x).abs() < 1.0,
            "window grew at scale {scale} from {initial:?} to {settled:?}"
        );
        assert!(
            settled.x < 950.0,
            "default window should leave room for other floaters: {settled:?}"
        );
        let (out, _) = h.frame(Vec::new());
        let mut painted = 0;
        for i in 1..=2000 {
            if out.shapes.iter().any(|s| text_rect(&s.shape, &format!("Object {i}")).is_some()) {
                painted += 1;
            }
        }
        assert!((1..40).contains(&painted), "only the viewport's rows should be drawn: {painted}");
        for label in [aurora_i18n::tr!("area-search-copy"), aurora_i18n::tr!("area-search-inspect")] {
            let rect = h.find(label);
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                "footer left the screen at scale {scale}: {rect:?}"
            );
        }
        h.click(aurora_i18n::tr!("area-search-filters"));
        for _ in 0..10 {
            h.frame(Vec::new());
        }
        let rect = h.find(aurora_i18n::tr!("area-search-copy"));
        for i in 0usize..4 {
            let field = h
                .ctx
                .read_response(egui::Id::unique(("area_search_field", i)))
                .expect("visible search field");
            assert!(field.rect.width() >= 160.0, "grid squeezed search field {i}: {:?}", field.rect);
        }
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
            "expanded filters pushed the footer off screen: {rect:?}"
        );
    }
}
