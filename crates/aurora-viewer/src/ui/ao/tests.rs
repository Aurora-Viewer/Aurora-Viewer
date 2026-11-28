use super::*;
use aurora_net::inventory::operations::Operation;

struct Fixture {
    ctx: egui::Context,
    inv: Inventory,
    catalog: inventory::Catalog,
    runtime: Runtime,
    account: Account,
    state: AoUi,
    movement: super::super::movement::MovementUi,
    movement_prefs: crate::movement::Preferences,
    quick: Option<quick::Quick>,
    manager_open: bool,
}

impl Fixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = super::super::icons::Icons::load(&ctx, None);
        let mut inv = Inventory::default();
        let id = crate::ao::demo::seed(&mut inv, Uuid::from_u128(1));
        let mut catalog = inventory::Catalog::default();
        catalog.refresh(&mut inv);
        let mut fixture = Self {
            ctx,
            inv,
            catalog,
            runtime: Runtime::default(),
            account: Account {
                enabled: false,
                selected: id,
                ..Default::default()
            },
            state: AoUi::default(),
            movement: Default::default(),
            movement_prefs: Default::default(),
            quick: None,
            manager_open: true,
        };
        // (the slots open on a click; most tests work in Standing's)
        fixture.state.open_slots[0] = Some(State::Standing);
        fixture.settle();
        fixture
    }

    fn draw(&mut self, events: Vec<egui::Event>) -> (Vec<Action>, Vec<egui::epaint::ClippedShape>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            if let Some(quick) = &mut self.quick {
                egui::Area::new(egui::Id::unique("test_ao_toolbar"))
                    .fixed_pos(egui::pos2(1200.0, 850.0))
                    .show(ui.ctx(), |ui| {
                        let response = ui.button("AO");
                        quick.anchor(&response, egui::RectAlign::TOP);
                    });
                actions.extend(quick.show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &self.catalog,
                    &self.inv,
                    &mut self.runtime,
                    &mut self.account,
                    &mut self.movement_prefs,
                    &mut self.manager_open,
                    false,
                ));
            }
            actions.extend(mini::show(
                ui.ctx(),
                &crate::theme::Theme::default().palette(),
                &self.catalog,
                &self.inv,
                &mut self.runtime,
                &mut self.account,
                &mut self.movement_prefs,
                &mut self.manager_open,
                false,
            ));
            actions.extend(show(
                ui.ctx(),
                &crate::theme::Theme::default().palette(),
                &mut self.inv,
                &self.catalog,
                &mut self.runtime,
                &mut self.account,
                &mut self.state,
                &mut self.manager_open,
                &mut self.movement,
                &mut self.movement_prefs,
                "",
            ));
        });
        output.textures_delta.clear();
        (actions, output.shapes)
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.draw(Vec::new());
        }
    }

    fn click(&mut self, label: &str) -> Vec<Action> {
        fn position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + egui::vec2(3.0, 5.0)),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, label)),
                _ => None,
            }
        }
        let shapes = self.draw(Vec::new()).1;
        let pos = shapes.iter().find_map(|s| position(&s.shape, label)).expect(label);
        self.click_at(pos)
    }

    /// Clicks the button that paints this Tabler icon (icon-only buttons).
    fn click_icon(&mut self, icon: &str) -> Vec<Action> {
        fn position(shape: &egui::Shape, texture: egui::TextureId) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Mesh(m) if m.texture_id == texture => Some(m.calc_bounds().center()),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, texture)),
                _ => None,
            }
        }
        let texture = super::super::icons::global(icon).expect(icon).id();
        let shapes = self.draw(Vec::new()).1;
        let pos = shapes.iter().find_map(|s| position(&s.shape, texture)).expect(icon);
        self.click_at(pos)
    }

    fn click_at(&mut self, pos: egui::Pos2) -> Vec<Action> {
        let mut out = Vec::new();
        for pressed in [true, false] {
            out.extend(
                self.draw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ])
                .0,
            );
        }
        out
    }
}

/// Text shapes' rects, by their exact text.
fn text_rects(shape: &egui::Shape, text: &str, out: &mut Vec<egui::Rect>) {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == text => out.push(egui::Rect::from_min_size(t.pos, t.galley.size())),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text_rects(s, text, out)),
        _ => {}
    }
}

#[test]
fn ao_drop_box_is_fully_visible_after_scrolling_with_large_text() {
    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_interface(&mut f.inv, Uuid::from_u128(1), "interface");
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    crate::theme::Theme::default().apply(&f.ctx, 2.0);
    f.ctx.set_zoom_factor(0.9);
    super::super::widgets::place_floater(&f.ctx, "ao", egui::Pos2::ZERO, egui::vec2(1400.0, 980.0));
    for _ in 0..40 {
        f.draw(Vec::new());
    }
    let window = f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao"))).expect("AO shown");
    let at = window.center();
    for _ in 0..30 {
        f.draw(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -1000.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Default::default(),
            },
        ]);
    }
    // The open slot is the first of the page: its drop box can be scrolled
    // back into view at the top, whole.
    for _ in 0..30 {
        f.draw(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 1000.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Default::default(),
            },
        ]);
    }
    let mut found = false;
    for shape in f.draw(Vec::new()).1 {
        let mut labels = Vec::new();
        text_rects(&shape.shape, aurora_i18n::tr!("ao-drop-box"), &mut labels);
        for rect in labels {
            found = true;
            assert!(
                shape.clip_rect.contains_rect(rect),
                "the drop box label stays inside the page: {rect:?} in {:?}",
                shape.clip_rect
            );
        }
    }
    assert!(found, "the open slot's drop box is visible");
}

#[test]
fn ao_groups_show_their_members_together_and_layers_are_tabs() {
    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_interface(&mut f.inv, Uuid::from_u128(1), "interface-group");
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    f.settle();
    let c = &f.catalog.selected(f.account.selected).expect("set").states[&State::Standing];
    let group = c.steps.iter().find(|s| s.group).expect("group");
    let members: Vec<String> = group.members.iter().map(|m| m.name.clone()).collect();
    assert!(
        shows_text(&mut f, &aurora_i18n::tr!("ao-together").to_uppercase()),
        "a group is marked as playing together"
    );
    for name in &members {
        assert!(shows_text(&mut f, name), "the group's member {name} is named in its row");
    }

    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_interface(&mut f.inv, Uuid::from_u128(1), "interface-track");
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    f.settle();
    let track = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing]
        .tracks
        .first()
        .expect("track")
        .clone();
    assert!(f.click(&track.name).is_empty(), "the layer is a tab of the slot");
    assert_eq!(f.state.container, Some(track.id));
    f.click(aurora_i18n::tr!("ao-main"));
    assert_eq!(f.state.container, None);
}

/// The remove button at the end of the row that shows `name`.
fn row_bin(f: &mut Fixture, name: &str) -> egui::Pos2 {
    fn meshes(shape: &egui::Shape, texture: egui::TextureId, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Mesh(m) if m.texture_id == texture => out.push(m.calc_bounds()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| meshes(s, texture, out)),
            _ => {}
        }
    }
    let shapes = f.draw(Vec::new()).1;
    let (mut texts, mut bins) = (Vec::new(), Vec::new());
    let bin = super::super::icons::global("trash-filled").expect("remove icon").id();
    for shape in &shapes {
        text_rects(&shape.shape, name, &mut texts);
        meshes(&shape.shape, bin, &mut bins);
    }
    let row = *texts.first().expect(name);
    bins.into_iter()
        .find(|b| (b.center().y - row.center().y).abs() < 6.0 && b.left() > row.right())
        .expect("the row's remove button")
        .center()
}

/// A click with the secondary button (or with Shift held) on `name`.
fn click_with(f: &mut Fixture, name: &str, button: egui::PointerButton, modifiers: egui::Modifiers) -> Vec<Action> {
    let mut texts = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        text_rects(&shape.shape, name, &mut texts);
    }
    let pos = texts.first().expect(name).center();
    let mut out = Vec::new();
    for pressed in [true, false] {
        out.extend(
            f.draw(vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                },
            ])
            .0,
        );
    }
    out.extend(f.draw(vec![egui::Event::ModifiersChanged(Default::default())]).0);
    out
}

#[test]
fn ao_manager_at_reduced_ui_scale_keeps_its_size() {
    for zoom in [0.9, 0.8, 0.85, 0.95, 1.1] {
        let mut f = Fixture::new();
        if zoom == 0.95 {
            let set = f
                .catalog
                .sets
                .iter_mut()
                .find(|s| s.id == f.account.selected)
                .expect("selected set");
            set.states.get_mut(&State::Standing).expect("standing").steps[0].name = super::super::widgets::elide_tests::LONG.repeat(4);
        }
        f.ctx.set_zoom_factor(zoom);
        super::super::widgets::place_floater(&f.ctx, "ao", egui::pos2(40.0, 40.0), egui::vec2(1240.0, 760.0));
        let mut rects = Vec::new();
        for _ in 0..120 {
            f.draw(Vec::new());
            rects.push(f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao"))).expect("AO shown"));
        }
        assert!(
            rects[15..].iter().all(|r| *r == rects[15]),
            "AO at scale {zoom} keeps growing: first {:?}, frame 15 {:?}, last {:?}",
            rects[0],
            rects[15],
            rects[119]
        );
        let before = rects[119];
        super::super::widgets::place_floater(&f.ctx, "ao", before.min, egui::vec2(900.0, 600.0));
        rects.clear();
        for _ in 0..30 {
            f.draw(Vec::new());
            rects.push(f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao"))).expect("AO shown"));
        }
        assert!(
            rects[15..].iter().all(|r| *r == rects[15]),
            "AO at scale {zoom} grows after resizing"
        );
        assert!(rects[29].width() < before.width() - 40.0, "AO at scale {zoom} can be made narrower");
        let c = &f.catalog.selected(f.account.selected).expect("set").states[&State::Standing];
        let (first, name) = (c.steps[1].id, c.steps[1].name.clone());
        let bin = row_bin(&mut f, &name);
        assert!(
            rects[29].contains(bin),
            "AO at scale {zoom}: the row's remove button stays in the window"
        );
        let out = f.click_at(bin);
        assert!(
            matches!(&out[..], [Action::Edit(change, None)]
                if matches!(&change.operations[..], [Operation::Delete { id, folder: false }] if *id == first)),
            "the entry's removal button is accessible"
        );
    }
}

/// #3675: the trash button of an animation asked first, and the reporter never
/// saw the question. An animation is a link: it goes at once, like
/// FloaterAO::onClickTrash.
#[test]
fn an_animation_is_removed_at_once_and_the_next_one_too() {
    let mut f = Fixture::new();
    let steps: Vec<(Uuid, String)> = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing]
        .steps
        .iter()
        .map(|s| (s.id, s.name.clone()))
        .collect();
    for (step, name) in &steps {
        let bin = row_bin(&mut f, name);
        let out = f.click_at(bin);
        let [Action::Edit(change, None)] = out.as_slice() else {
            panic!("one inventory edit per click")
        };
        assert!(matches!(&change.operations[..], [Operation::Delete { id, folder: false }] if id == step));
        assert_eq!(f.state.remove, None, "no question for a single animation");
        assert!(f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao_remove"))).is_none());
    }
}

/// #3675: a set and a group are folders, they still ask. A set asks in place,
/// in "Your sets"; a group's question opens above the AO window each time,
/// and unfolded when it was left minimized.
#[test]
fn removing_a_set_or_a_group_asks_first() {
    let mut f = Fixture::new();
    super::super::widgets::place_floater(&f.ctx, "ao", egui::pos2(100.0, 60.0), egui::vec2(1240.0, 760.0));
    f.settle();
    let set = f.account.selected;
    let name = f.catalog.selected(set).expect("set").name.clone();
    f.click(&name);
    assert!(f.state.sets_open, "the set card opens Your sets");
    f.click_icon("dots");
    assert!(
        f.click(aurora_i18n::tr!("ao-delete-set")).is_empty(),
        "nothing is sent before the answer"
    );
    assert_eq!(f.state.confirm_delete, Some(set));
    assert!(f.click(aurora_i18n::tr!("common-cancel")).is_empty());
    assert_eq!(f.state.confirm_delete, None);
    f.click_icon("dots");
    f.click(aurora_i18n::tr!("ao-delete-set"));
    let out = f.click(aurora_i18n::tr!("common-delete"));
    let expected = inventory::remove(&f.inv, set).expect("set removal plan");
    assert!(
        matches!(&out[..], [Action::Edit(change, None)] if change.operations.len() == expected.operations.len()),
        "the set's folder goes once confirmed, with the same plan as before"
    );
    assert_eq!(f.state.confirm_delete, None);
    f.state.sets_open = false;
    f.settle();

    // a group of animations is a folder too
    let question = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("ao_remove"));
    let chosen = std::collections::HashSet::from([steps_of(&f)[0]]);
    let (change, _) = inventory::editor::combine(
        &f.inv,
        f.catalog.selected(f.account.selected).expect("set"),
        State::Standing,
        "Together",
        false,
        &chosen,
    )
    .expect("group");
    crate::world::inventory::actions::demo_mutate(&mut f.inv, Uuid::from_u128(1), change).expect("server ack");
    f.catalog.invalidate();
    f.catalog.refresh(&mut f.inv);
    f.settle();
    let steps = steps_of(&f);
    let group = steps
        .iter()
        .copied()
        .find(|id| f.inv.folders.contains_key(id))
        .expect("the group is a folder");
    // the stack's row shows its member
    let member = "Aurora Stand A";
    for round in 0..2 {
        f.settle();
        let bin = row_bin(&mut f, member);
        assert!(f.click_at(bin).is_empty(), "a group asks first");
        assert_eq!(f.state.remove, Some(group));
        f.draw(Vec::new());
        let rect = f.ctx.memory(|m| m.area_rect(question.id)).expect("the question is drawn");
        let window = f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao"))).expect("AO shown");
        assert!(window.contains(rect.center()), "the question opens over the AO window");
        assert_eq!(
            f.ctx.layer_id_at(rect.center()),
            Some(question),
            "round {round}: the question is on top"
        );
        assert!(f.click(aurora_i18n::tr!("common-cancel")).is_empty());
        assert_eq!(f.state.remove, None);
        // a click in the AO window raises it above the closed question,
        // which was left folded to its title bar
        f.click_at(window.left_top() + egui::vec2(120.0, 12.0));
        f.ctx
            .data_mut(|d| d.insert_persisted(egui::Id::unique(("ao_remove", "minimized")), true));
    }
    assert!(super::super::widgets::floater_minimized(&f.ctx, "ao_remove"));
    let bin = row_bin(&mut f, member);
    f.click_at(bin);
    assert!(
        !super::super::widgets::floater_minimized(&f.ctx, "ao_remove"),
        "the question opens unfolded"
    );
}

fn steps_of(f: &Fixture) -> Vec<Uuid> {
    f.catalog.selected(f.account.selected).expect("set").states[&State::Standing]
        .steps
        .iter()
        .map(|s| s.id)
        .collect()
}

#[test]
fn ao_manager_keeps_its_size_across_frames() {
    super::super::widgets::steady_tests::assert_window_steady(
        "ao",
        &[egui::vec2(1240.0, 760.0), egui::vec2(900.0, 600.0)],
        aurora_i18n::tr!("ao-main"),
        Fixture::new,
        |f, ctx, p| {
            show(
                ctx,
                p,
                &mut f.inv,
                &f.catalog,
                &mut f.runtime,
                &mut f.account,
                &mut f.state,
                &mut f.manager_open,
                &mut f.movement,
                &mut f.movement_prefs,
                "",
            );
        },
    );
}

#[test]
fn the_title_switch_changes_runtime_without_inventory_writes() {
    let mut f = Fixture::new();
    f.runtime.paused = true;
    f.runtime.stands = false;
    let switch = |f: &mut Fixture| {
        let mut texts = Vec::new();
        for shape in f.draw(Vec::new()).1 {
            text_rects(&shape.shape, aurora_i18n::tr!("ao-switch"), &mut texts);
        }
        // the switch, right of its label in the title bar
        let label = *texts.first().expect("the title's AO switch");
        label.right_center() + egui::vec2(18.0, 0.0)
    };
    let at = switch(&mut f);
    let out = f.click_at(at);
    assert!(f.account.enabled);
    assert!(matches!(&out[..], [Action::Preferences]));
    assert!(!f.runtime.paused, "turned on, the AO plays again");
    assert!(f.runtime.stands, "with its stands and turns");
    let at = switch(&mut f);
    let out = f.click_at(at);
    assert!(!f.account.enabled);
    assert!(matches!(&out[..], [Action::Preferences]));
}

#[test]
fn firestorm_migration_is_explicit_cancellable_and_keeps_native_selection_until_acknowledged() {
    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_migration(&mut f.inv, Uuid::from_u128(1));
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    let selected = f.account.selected;
    let generation = f.inv.generation;
    assert!(f.state.migration.is_none());
    f.state.sets_open = true;
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-migrate-link")).is_empty());
    f.state.migration.as_mut().expect("picker").refresh(&mut f.inv, true);
    f.settle();
    assert!(
        f.click(aurora_i18n::tr!("ao-migrate-confirm")).is_empty(),
        "duplicate name is disabled"
    );
    assert!(f.click("Firestorm Walk Set").is_empty());
    f.settle();
    assert!(matches!(
        f.click(aurora_i18n::tr!("ao-migrate-confirm")).as_slice(),
        [Action::Migrate]
    ));
    assert_eq!(f.account.selected, selected);
    assert_eq!(f.inv.generation, generation);
    assert!(f.click(aurora_i18n::tr!("common-cancel")).is_empty());
    assert!(f.state.migration.is_none());
    assert_eq!(f.account.selected, selected);
    assert_eq!(f.inv.generation, generation);
}

#[test]
fn alchemy_choice_lists_its_sets_and_migrates_one_without_inventory_writes() {
    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_alchemy_migration(&mut f.inv, Uuid::from_u128(1));
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    let generation = f.inv.generation;
    f.state.sets_open = true;
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-migrate-link")).is_empty());
    f.state.migration.as_mut().expect("picker").refresh(&mut f.inv, true);
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-migrate-alchemy")).is_empty());
    let migration = f.state.migration.as_mut().expect("picker");
    assert!(migration.alchemy);
    migration.refresh(&mut f.inv, true);
    f.settle();
    assert!(matches!(
        f.click(aurora_i18n::tr!("ao-migrate-confirm")).as_slice(),
        [Action::Migrate]
    ));
    let migration = f.state.migration.as_ref().expect("picker");
    assert_eq!(migration.name, "Alchemy Dance Set");
    assert_eq!(f.inv.generation, generation);
}

#[test]
fn rename_updates_existing_folder_and_new_set_creates_a_distinct_set() {
    let mut f = Fixture::new();
    f.state.sets_open = true;
    f.state.renaming = Some(f.account.selected);
    f.state.name = "Renamed".into();
    f.settle();
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    let out = f.draw(vec![enter]).0;
    assert!(matches!(&out[..], [Action::Edit(change, None)] if matches!(&change.operations[..],
        [Operation::Patch { id, folder: true, .. }] if *id == f.account.selected)));
    assert!(f.state.renaming.is_none());
    f.state.name = "New set".into();
    f.state.new_dialog = Some(SetDialog::New);
    f.settle();
    let out = f.click(aurora_i18n::tr!("common-save"));
    assert!(matches!(&out[..], [Action::Create(set)] if set.name == "New set" && set.states.is_empty()));
}

#[test]
fn animation_picker_links_into_selected_state_and_import_blocks_edits() {
    let mut f = Fixture::new();
    f.state.state = State::Walking;
    f.state.open_slots[super::category::Category::of(f.state.state).index()] = Some(f.state.state);
    let folder = f.catalog.selected(f.account.selected).expect("set").states[&State::Walking].folder;
    f.click(aurora_i18n::tr!("ao-drop-box"));
    assert_eq!(f.state.picker, Some(false));
    f.state.filter = "Type".into();
    f.settle();
    let out = f.click("Aurora Type");
    assert!(matches!(&out[..], [Action::Edit(change, None)] if change.operations.iter().any(|op|
        matches!(op, Operation::Links { parent, links } if *parent == folder && links.len() == 1))));
    assert!(f.state.picker.is_none());
    f.state.import_folder = Some(crate::ao::demo::NOTECARD);
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-drop-box")).is_empty());
    assert!(f.state.picker.is_none(), "busy controls must not dispatch concurrent changes");
}

#[test]
fn open_animation_and_notecard_pickers_reorder_when_inventory_sort_changes() {
    use crate::ui::inventory::view::{Sort, set_picker_sort};
    let mut f = Fixture::new();
    for (notecard, asset_type) in [(false, 20), (true, 7)] {
        f.inv.add_items(
            [("Sort 10", 200), ("sort 2", 100)]
                .into_iter()
                .enumerate()
                .map(|(i, (name, date))| aurora_net::inventory::InvItem {
                    id: Uuid::from_u128(78000 + i as u128),
                    parent: f.inv.root,
                    name: name.into(),
                    created_at: date,
                    asset_type,
                    ..Default::default()
                })
                .collect(),
        );
        f.state.picker = Some(notecard);
        f.state.filter = "sort".into();
        for by_date in [false, true, false] {
            set_picker_sort(
                &f.ctx,
                Sort {
                    by_date,
                    ..Default::default()
                },
            );
            f.settle();
            let names: Vec<_> = f.state.rows.iter().map(|row| row.1.as_str()).collect();
            assert_eq!(names, if by_date { ["Sort 10", "sort 2"] } else { ["sort 2", "Sort 10"] });
        }
    }
}

#[test]
fn movement_tab_preserves_independent_preferences_without_enabling_ao() {
    let mut f = Fixture::new();
    f.click(aurora_i18n::tr!("ao-tab-movement-short"));
    assert_eq!(f.state.tab, 1);
    let actions = f.click(aurora_i18n::tr!("movement-running"));
    assert!(f.movement_prefs.running);
    assert!(!f.account.enabled);
    assert!(matches!(actions.as_slice(), [Action::MovementPreferences]));
    f.click(super::category::Category::General.label());
    assert_eq!(f.state.tab, 0);
    assert_eq!(f.state.state, State::Standing);
}

#[test]
fn swimming_switch_requires_the_selected_set_to_have_all_eight_slots() {
    let mut f = Fixture::new();
    f.state.tab = 1;
    f.settle();
    f.click(aurora_i18n::tr!("movement-swimming"));
    assert!(f.movement_prefs.swimming);
    f.catalog
        .sets
        .iter_mut()
        .find(|s| s.id == f.account.selected)
        .expect("set")
        .states
        .remove(&State::WaterEntry);
    f.settle();
    assert!(!f.movement_prefs.swimming);
    assert!(f.click(aurora_i18n::tr!("movement-swimming")).is_empty());
    assert!(!f.movement_prefs.swimming, "a disabled switch cannot bypass the missing-slot guard");
    assert_eq!(f.movement.swim_slots, 7);
}

#[test]
fn selecting_another_set_cancels_an_open_animation_picker() {
    let mut f = Fixture::new();
    f.state.picker = Some(false);
    let empty = Set::new("Other set".into());
    f.account.selected = empty.id;
    f.catalog.sets.push(empty);
    f.settle();
    assert!(f.state.picker.is_none());
    assert!(!f.movement.swim_ready);
}

#[test]
fn acknowledged_container_selection_is_remapped_and_bound_to_its_original_editor() {
    let mut f = Fixture::new();
    let requested = Uuid::new_v4();
    let actual = Uuid::new_v4();
    let target = EditorTarget {
        set: f.account.selected,
        state: State::Standing,
        container: Some(requested),
    };
    assert!(f.state.container.is_none());
    let created = std::collections::HashMap::from([(requested, actual)]);
    assert!(f.state.finish_editor(target, &created));
    assert_eq!(f.state.container, Some(actual));
    f.state.state = State::Walking;
    f.state.open_slots[super::category::Category::of(f.state.state).index()] = Some(f.state.state);
    f.state.container = None;
    assert!(!f.state.finish_editor(target, &created));
    assert!(f.state.container.is_none());
}

#[test]
fn guided_combinations_are_cancellable_and_wait_for_acknowledgement() {
    let mut f = Fixture::new();
    let generation = f.inv.generation;
    // an entry's right-click menu makes a stack with it, in the guided dialog
    click_with(&mut f, "Aurora Stand A", egui::PointerButton::Secondary, Default::default());
    assert!(f.click(aurora_i18n::tr!("ao-make-stack")).is_empty());
    let first = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing].steps[0].id;
    assert!(
        f.state.combination.as_ref().is_some_and(|d| d.selected.contains(&first)),
        "the entry is already chosen"
    );
    assert!(f.click(aurora_i18n::tr!("common-cancel")).is_empty());
    assert!(f.state.combination.is_none());
    assert_eq!(f.inv.generation, generation);
    click_with(&mut f, "Aurora Stand A", egui::PointerButton::Secondary, Default::default());
    f.click(aurora_i18n::tr!("ao-make-stack"));
    let chosen = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing].steps[0].id;
    let dialog = f.state.combination.as_mut().expect("dialog");
    dialog.name = "Together".into();
    dialog.selected.insert(chosen);
    f.settle();
    let out = f.click(aurora_i18n::tr!("ao-create-stack"));
    assert!(matches!(
        &out[..],
        [Action::EditContainer(_, EditorTarget { container: Some(_), .. })]
    ));
    assert!(f.state.combination.is_none());
    assert!(f.state.container.is_none(), "do not select an unacknowledged new folder");
    assert_eq!(f.inv.generation, generation);
}

#[test]
fn native_animation_rows_choose_on_single_click_and_drag_through_inventory_ack() {
    let mut f = Fixture::new();
    let c = &f.catalog.selected(f.account.selected).expect("set").states[&State::Standing];
    let (folder, second) = (c.folder, c.steps[1].id);
    // a click previews; Shift+click plays it in the world
    assert!(f.click("Aurora Stand B").is_empty());
    assert_eq!(f.runtime.index(State::Standing, folder), 0, "a click only previews");
    assert!(f.state.preview.previews(None, 1));
    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    assert!(click_with(&mut f, "Aurora Stand B", egui::PointerButton::Primary, shift).is_empty());
    assert_eq!(f.runtime.index(State::Standing, folder), 1, "Shift+click plays it in the world");
    fn locate(shape: &egui::Shape, name: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == name => Some(t.pos + egui::vec2(5.0, 5.0)),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| locate(s, name)),
            _ => None,
        }
    }
    let shapes = f.draw(Vec::new()).1;
    let from = shapes.iter().find_map(|s| locate(&s.shape, "Aurora Stand B")).expect("source");
    let to = shapes.iter().find_map(|s| locate(&s.shape, "Aurora Stand A")).expect("destination");
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    assert!(f.draw(vec![egui::Event::PointerMoved(from), press(from, true)]).0.is_empty());
    assert!(f.draw(vec![egui::Event::PointerMoved(from + egui::vec2(9.0, 0.0))]).0.is_empty());
    assert!(f.draw(vec![egui::Event::PointerMoved(to)]).0.is_empty());
    let out = f.draw(vec![press(to, false)]).0;
    let [Action::Edit(change, None)] = out.as_slice() else {
        panic!("drop must dispatch one inventory edit")
    };
    crate::world::inventory::actions::demo_mutate(&mut f.inv, Uuid::from_u128(1), change.clone()).expect("server ack");
    f.catalog.invalidate();
    f.catalog.refresh(&mut f.inv);
    assert_eq!(
        f.catalog.selected(f.account.selected).expect("set").states[&State::Standing].steps[0].id,
        second
    );
}

#[test]
fn both_water_states_use_the_normal_list_and_combination_editor() {
    let mut f = Fixture::new();
    for state in [State::SwimmingForward, State::UnderwaterSwimming] {
        f.state.state = state;
        f.state.open_slots[super::category::Category::of(f.state.state).index()] = Some(f.state.state);
        f.state.combination = None;
        f.state.container = None;
        f.settle();
        // the slot's "+" opens the guided editor for a new layer
        assert!(f.click_icon("plus").is_empty());
        assert_eq!(f.state.combination.as_ref().expect("ordinary combination editor").state, state);
    }
}

#[test]
fn swimming_rail_marks_only_required_slots_and_clears_the_warning_when_ready() {
    fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, label)),
            _ => None,
        }
    }
    fn icon_rects(shape: &egui::Shape, color: egui::Color32, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Mesh(mesh) if !mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| v.color == color) => {
                out.push(mesh.calc_bounds());
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| icon_rects(s, color, out)),
            _ => {}
        }
    }
    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_movement_design(&mut f.inv, Uuid::from_u128(1));
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    let set = f.catalog.sets.first_mut().expect("set");
    let floating = set.states.remove(&State::Floating).expect("required slot");
    set.states.remove(&State::UnderwaterSwimming).expect("optional slot");
    f.state.state = State::SwimmingForward;
    f.state.open_slots[super::category::Category::of(f.state.state).index()] = Some(f.state.state);
    f.settle();
    let palette = crate::theme::Theme::default().palette();
    let shapes = f.draw(Vec::new()).1;
    let row = |state: State| {
        shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, state.label()))
            .expect("visible state")
    };
    let mut warning_icons = Vec::new();
    let mut ready_icons = Vec::new();
    for s in &shapes {
        icon_rects(&s.shape, palette.warn, &mut warning_icons);
        icon_rects(&s.shape, palette.success, &mut ready_icons);
    }
    // The slot's mark comes before its name.
    let beside = |icon: &egui::Rect, row: egui::Rect| {
        (icon.center().y - row.center().y).abs() < 2.0 && icon.right() <= row.left() && icon.right() > row.left() - 40.0
    };
    assert!(warning_icons.iter().any(|icon| beside(icon, row(State::Floating))));
    assert!(ready_icons.iter().any(|icon| beside(icon, row(State::SwimmingForward))));
    assert!(
        !warning_icons
            .iter()
            .chain(&ready_icons)
            .any(|icon| beside(icon, row(State::UnderwaterSwimming)))
    );
    assert!(
        shapes
            .iter()
            .any(|s| text_rect(&s.shape, aurora_i18n::tr!("ao-swim-required")).is_some())
    );
    f.catalog.sets[0].states.insert(State::Floating, floating);
    f.settle();
    let shapes = f.draw(Vec::new()).1;
    assert!(
        f.catalog.swimming_ready(f.account.selected),
        "the optional underwater clip cannot block swimming"
    );
    assert!(
        !shapes
            .iter()
            .any(|s| text_rect(&s.shape, aurora_i18n::tr!("ao-swim-required")).is_some())
    );
    assert!(
        shapes
            .iter()
            .any(|s| text_rect(&s.shape, aurora_i18n::tr!("ao-swim-ready")).is_some())
    );
}

#[test]
fn categories_keep_every_state_and_restore_the_previous_selection() {
    use super::category::Category;
    let states: Vec<_> = Category::ALL.into_iter().flat_map(Category::states).collect();
    assert_eq!(states.len(), State::ALL.len());
    assert!(
        State::ALL
            .into_iter()
            .all(|s| states.iter().filter(|item| **item == s).count() == 1)
    );
    for state in [
        State::Standing,
        State::Sitting,
        State::GroundSit,
        State::StandingUp,
        State::Typing,
        State::Always,
    ] {
        assert_eq!(Category::of(state), Category::General);
    }
    assert_eq!(
        Category::General.states().take(4).collect::<Vec<_>>(),
        [State::Standing, State::Sitting, State::GroundSit, State::StandingUp],
        "sitting states follow Standing immediately"
    );
    assert_eq!(Category::of(State::Running), Category::Ground);
    assert_eq!(
        Category::Ground.states().collect::<Vec<_>>(),
        [
            State::Walking,
            State::TurningRight,
            State::TurningLeft,
            State::Running,
            State::Crouching,
            State::CrouchWalk
        ]
    );
    assert_eq!(Category::ALL.last(), Some(&Category::Water), "swimming is the last category");
    assert_eq!(Category::ALL[Category::ALL.len() - 2], Category::Flight, "flying is penultimate");
    for state in [
        State::PreJumping,
        State::Jumping,
        State::Falling,
        State::SoftLanding,
        State::Landing,
    ] {
        assert_eq!(Category::of(state), Category::Jumping);
    }
    assert_eq!(
        Category::Jumping.states().collect::<Vec<_>>(),
        [
            State::PreJumping,
            State::Jumping,
            State::Falling,
            State::SoftLanding,
            State::Landing
        ],
        "jump states follow preparation, jump, fall, soft landing and hard landing"
    );
    assert_eq!(
        Category::Flight.states().collect::<Vec<_>>(),
        [
            State::Hovering,
            State::FlyingSlow,
            State::Flying,
            State::FlyingDown,
            State::FlyingUp
        ],
        "flight states follow hovering, slow flight, flight, descent and ascent"
    );
    assert_eq!(Category::of(State::UnderwaterSwimming), Category::Water);
    let water: Vec<_> = Category::Water.states().collect();
    assert!(water.windows(2).any(|pair| pair == [State::WaterFlightExit, State::WaterShoreExit]));
    assert!(water.windows(2).any(|pair| pair == [State::WaterCannonball, State::WaterEntry]));
    assert!(
        Category::Water
            .states()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| { pair == [State::UnderwaterFloating, State::UnderwaterSwimming] }),
        "underwater floating immediately precedes underwater swimming"
    );
    let mut f = Fixture::new();
    f.state.state = State::Walking;
    f.state.open_slots[super::category::Category::of(f.state.state).index()] = Some(f.state.state);
    f.settle();
    f.click(aurora_i18n::tr!("ao-category-water"));
    assert_eq!(f.state.state, State::Floating);
    f.click(State::UnderwaterSwimming.label());
    f.click(Category::Ground.label());
    assert_eq!(f.state.state, State::Walking);
    f.click(aurora_i18n::tr!("ao-category-water"));
    assert_eq!(f.state.state, State::UnderwaterSwimming);
    assert!(f.state.container.is_none());
    f.click(Category::Jumping.label());
    assert_eq!(f.state.state, State::Jumping);
    f.click(State::Falling.label());
    f.click(Category::Ground.label());
    f.click(State::Running.label());
    assert_eq!(f.state.state, State::Running);
    f.click(Category::General.label());
    // Always is the last slot of General, below the open one.
    f.state.select_state(State::Always);
    f.settle();
    f.click(Category::Ground.label());
    assert_eq!(f.state.state, State::Running, "walking and running share one category");
    f.click(Category::General.label());
    assert_eq!(
        f.state.state,
        State::Always,
        "always has its own remembered selection outside ground movement"
    );
    for state in [State::Sitting, State::GroundSit, State::StandingUp] {
        f.click(state.label());
        assert_eq!(f.state.state, state);
        f.click(Category::Ground.label());
        f.click(Category::General.label());
        assert_eq!(f.state.state, state, "General remembers its selected sitting state");
    }
    f.click(Category::Jumping.label());
    assert_eq!(f.state.state, State::Falling, "jumping retains its own selected state");
    f.state.state = State::Flying;
    f.state.open_slots[super::category::Category::of(f.state.state).index()] = Some(f.state.state);
    f.settle();
    assert_eq!(
        Category::of(f.state.state),
        Category::Flight,
        "external state links select their category"
    );
}

#[test]
fn category_keyboard_navigation_wraps_inside_its_own_states() {
    use super::category::Category;
    let key = |key| {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]
    };
    for category in Category::ALL {
        let mut f = Fixture::new();
        f.click(category.label());
        assert_eq!(f.state.state, category.initial_state());
        let first = category.states().next().expect("nonempty category");
        let last = category.states().last().expect("nonempty category");
        f.state.state = first;
        f.state.state_focus = Some(first);
        f.settle();
        f.draw(key(egui::Key::ArrowUp));
        assert_eq!(f.state.state, last, "wrap inside {category:?}");
        f.settle();
        f.draw(key(egui::Key::ArrowDown));
        assert_eq!(f.state.state, first);
    }
}

fn quick_fixture(ready: bool) -> Fixture {
    let mut f = Fixture::new();
    if ready {
        f.inv = Inventory::default();
        f.account.selected = crate::ao::demo::seed_movement_design(&mut f.inv, Uuid::from_u128(1));
        f.catalog = Default::default();
        f.catalog.refresh(&mut f.inv);
    }
    f.manager_open = false;
    f.quick = Some(Default::default());
    f.settle();
    assert!(f.click("AO").is_empty());
    f.settle();
    assert!(f.quick.as_ref().is_some_and(|q| q.open));
    assert!(!f.manager_open, "toolbar opens only the compact controls");
    f
}

#[test]
fn quick_controls_share_preferences_and_preserve_water_mode_exclusion() {
    let mut f = quick_fixture(true);
    let generation = f.inv.generation;
    f.runtime.paused = true;
    f.runtime.stands = false;
    assert!(matches!(f.click_icon("power").as_slice(), [Action::Preferences]));
    assert!(f.account.enabled && !f.runtime.paused && f.runtime.stands);
    f.click(aurora_i18n::tr!("ao-tile-swimming"));
    assert!(f.movement_prefs.swimming && !f.movement_prefs.water_walk);
    f.click(aurora_i18n::tr!("ao-tile-water-walk"));
    assert!(f.movement_prefs.water_walk && !f.movement_prefs.swimming);
    f.click(aurora_i18n::tr!("ao-tile-water-walk"));
    assert!(!f.movement_prefs.water_walk && !f.movement_prefs.swimming);
    f.click_icon("settings-filled");
    assert!(f.manager_open);
    assert!(!f.quick.as_ref().expect("quick").open);
    assert_eq!(f.inv.generation, generation, "quick controls must not mutate inventory");
}

#[test]
fn quick_popup_names_the_selected_animation_and_tracks_manual_steps() {
    let mut f = quick_fixture(true);
    // (with the AO off the card says the default animations play)
    f.account.enabled = true;
    f.runtime.current = Some(State::Standing);
    let set = f.catalog.selected(f.account.selected).expect("set").clone();
    let selected_text = set.animation_config(State::Standing).expect("stands").steps[0].name.clone();
    let mut shapes = f.draw(Vec::new()).1;
    assert!(
        shapes.iter_mut().any(|shape| contains_text(&shape.shape, &selected_text)),
        "{selected_text}"
    );
    f.runtime.step(&set, State::Standing, true);
    let next_text = set.animation_config(State::Standing).expect("stands").steps[1].name.clone();
    let mut shapes = f.draw(Vec::new()).1;
    assert!(
        shapes.iter_mut().any(|shape| contains_text(&shape.shape, &next_text)),
        "{next_text}"
    );
}

#[test]
fn quick_pin_opens_the_persistent_mini_window() {
    let mut f = quick_fixture(true);
    assert!(!f.account.mini_open);
    f.click_icon("pin-filled");
    assert!(f.account.mini_open);
    assert!(!f.quick.as_ref().expect("quick").open);
}

fn shows_text(f: &mut Fixture, text: &str) -> bool {
    let mut shapes = f.draw(Vec::new()).1;
    shapes.iter_mut().any(|shape| contains_text(&shape.shape, text))
}

/// The panel's pin opens the mini window (LLFloaterReg::showInstance); lit
/// while the strip is shown, it closes it. The strip cannot be folded: a
/// mini window left minimized by an older layout comes back whole.
#[test]
fn quick_pin_opens_and_closes_the_mini_window_and_an_old_minimized_strip_expands() {
    let mut f = quick_fixture(true);
    f.click_icon("pin-filled");
    assert!(f.account.mini_open);
    f.settle();
    super::super::widgets::minimize_floater(&f.ctx, mini::WINDOW);
    f.settle();
    assert!(
        !super::super::widgets::floater_minimized(&f.ctx, mini::WINDOW),
        "the strip expands itself"
    );
    f.click("AO");
    f.settle();
    assert!(f.quick.as_ref().is_some_and(|q| q.open));
    // the panel's pin, at the right of its header (the strip has its own
    // buttons, none of them a pin)
    f.click_icon("pin-filled");
    assert!(!f.account.mini_open, "the lit pin closes the strip");
}

/// The strip has no close button: its right-click menu closes it.
#[test]
fn the_mini_strip_closes_from_its_menu() {
    let mut f = Fixture::new();
    f.account.mini_open = true;
    f.manager_open = false;
    f.settle();
    let name = f.catalog.selected(f.account.selected).expect("set").name.to_uppercase();
    click_with(&mut f, &name, egui::PointerButton::Secondary, Default::default());
    f.click(aurora_i18n::tr!("ao-mini-close"));
    assert!(!f.account.mini_open);
}

/// Unlike FloaterAO::onClickMore, which turns its one floater into the full
/// AO, the mini window's Settings button opens the manager beside it.
#[test]
fn mini_ao_settings_opens_the_manager_and_keeps_the_mini_window_open() {
    let mut f = quick_fixture(true);
    f.quick = None;
    f.account.mini_open = true;
    f.settle();
    assert!(!f.manager_open);
    f.click_icon("settings-filled");
    f.settle();
    assert!(f.manager_open, "Settings opens the full AO window");
    assert!(f.account.mini_open, "the mini window stays open");
}

/// A Sits switch whose save is refused says why in the window, as the
/// manager does with `state.message`, until a later save succeeds.
#[test]
fn mini_ao_shows_a_failed_sits_save_in_the_window() {
    let mut f = quick_fixture(true);
    f.quick = None;
    f.account.mini_open = true;
    let selected = f.account.selected;
    let set = f.catalog.sets.iter_mut().find(|s| s.id == selected).expect("set");
    let name = std::mem::replace(&mut set.name, "bad:name".into());
    let reason = inventory::save_set(&f.inv, set).expect_err("a name with ':' is refused");
    let reason = reason.as_str();
    f.settle();
    assert!(!shows_text(&mut f, reason));
    let actions = f.click_icon("armchair");
    assert!(!actions.iter().any(|a| matches!(a, Action::Edit(..))), "nothing to send");
    assert!(shows_text(&mut f, reason), "the refusal is shown in the window");
    f.catalog.sets.iter_mut().find(|s| s.id == selected).expect("set").name = name;
    assert!(matches!(f.click_icon("armchair").as_slice(), [Action::Edit(_, None)]));
    assert!(!shows_text(&mut f, reason), "a saved switch clears the message");
}

#[test]
fn mini_ao_open_at_login_keeps_swimming_while_the_ao_loads() {
    // The mini window stays open across logins: it is drawn before the AO
    // folder arrives, and must not switch swimming off (and save that).
    let mut f = Fixture::new();
    f.account.mini_open = true;
    f.movement_prefs.swimming = true;
    f.catalog.loading = true;
    let (actions, _) = f.draw(Vec::new());
    assert!(f.movement_prefs.swimming, "swimming kept while the AO loads");
    assert!(!actions.iter().any(|a| matches!(a, Action::MovementPreferences)), "nothing to save");
    f.catalog.loading = false;
    f.catalog.failed = true;
    f.draw(Vec::new());
    assert!(f.movement_prefs.swimming, "kept after a failed fetch too");
    // loaded, without the swimming animations: off, as app/movement.rs does
    f.catalog.failed = false;
    f.catalog
        .sets
        .iter_mut()
        .find(|s| s.id == f.account.selected)
        .expect("set")
        .states
        .remove(&State::WaterEntry);
    f.draw(Vec::new());
    assert!(!f.movement_prefs.swimming);
}

#[test]
fn mini_ao_movement_switches_emit_preference_changes() {
    let mut f = quick_fixture(true);
    f.quick = None;
    f.account.mini_open = true;
    f.settle();
    assert!(
        f.click_icon("swimming")
            .iter()
            .any(|action| matches!(action, Action::MovementPreferences))
    );
    assert!(f.movement_prefs.swimming);
}

#[test]
fn mini_ao_can_toggle_sit_override_without_opening_the_manager() {
    let mut f = quick_fixture(true);
    f.quick = None;
    f.account.mini_open = true;
    f.settle();
    assert!(matches!(f.click_icon("armchair").as_slice(), [Action::Edit(_, None)]));
}

#[test]
fn mini_ao_shows_the_current_step_and_stays_open_after_outside_clicks() {
    let mut f = quick_fixture(true);
    f.quick = None;
    f.account.enabled = true;
    f.runtime.current = Some(State::Standing);
    f.account.mini_open = true;
    f.settle();
    let set = f.catalog.selected(f.account.selected).expect("set").clone();
    let name = set.animation_config(State::Standing).expect("stands").steps[0].name.clone();
    let text = name;
    let mut shapes = f.draw(Vec::new()).1;
    assert!(shapes.iter_mut().any(|shape| contains_text(&shape.shape, &text)), "{text}");
    f.runtime.step(&set, State::Standing, true);
    let next = set.animation_config(State::Standing).expect("stands").steps[1].name.clone();
    let next_text = next;
    let mut shapes = f.draw(Vec::new()).1;
    assert!(
        shapes.iter_mut().any(|shape| contains_text(&shape.shape, &next_text)),
        "{next_text}"
    );
    f.click_at(egui::pos2(20.0, 20.0));
    assert!(f.account.mini_open, "world clicks do not dismiss the persistent window");
}

fn contains_text(shape: &egui::Shape, text: &str) -> bool {
    match shape {
        egui::Shape::Text(t) => t.galley.text() == text,
        egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains_text(shape, text)),
        _ => false,
    }
}

#[test]
fn quick_arrows_step_the_playing_state_and_wrap() {
    let mut f = quick_fixture(true);
    let set = f.catalog.selected(f.account.selected).expect("set").clone();
    let stands = &set.states[&State::Standing];
    let len = stands.steps.len();
    assert!(len >= 2, "the demo set has several stands");
    let index = |f: &Fixture| f.runtime.index(State::Standing, stands.folder);
    // AOEngine::cycle does nothing while the AO is off.
    assert!(f.click_icon("caret-right-filled").is_empty());
    assert_eq!(index(&f), 0);
    f.account.enabled = true;
    assert!(f.click_icon("caret-right-filled").is_empty(), "stepping writes no inventory");
    assert_eq!(index(&f), 1);
    f.click_icon("caret-left-filled");
    f.click_icon("caret-left-filled");
    assert_eq!(index(&f), len - 1, "previous wraps to the last stand");
    f.click_icon("caret-right-filled");
    assert_eq!(index(&f), 0, "next wraps to the first stand");
    // The buttons follow the playing state, as AOEngine::cycle uses mLastMotion.
    let walks = set.states[&State::Walking].folder;
    f.runtime.current = Some(State::Walking);
    f.click_icon("caret-right-filled");
    assert_eq!(f.runtime.index(State::Walking, walks), 0, "a single walk has nothing to step to");
    assert_eq!(index(&f), 0);
    assert!(f.quick.as_ref().expect("quick").open, "the arrows keep the panel open");
}

#[test]
fn quick_cycle_button_toggles_the_state_cycle_option() {
    let mut f = quick_fixture(true);
    let cycle = |f: &Fixture| {
        f.catalog.selected(f.account.selected).expect("set").states[&State::Standing]
            .cycle
            .clone()
    };
    let before = cycle(&f);
    assert!(before.cycle && before.seconds > 0.0, "the demo stands cycle on a timer");
    for expected in [false, true] {
        let out = f.click_icon("repeat");
        let [Action::Edit(change, None)] = out.as_slice() else {
            panic!("the cycle button dispatches one inventory edit")
        };
        crate::world::inventory::actions::demo_mutate(&mut f.inv, Uuid::from_u128(1), change.clone()).expect("server ack");
        f.catalog.invalidate();
        f.catalog.refresh(&mut f.inv);
        let after = cycle(&f);
        assert_eq!(after.cycle, expected);
        assert_eq!(after.seconds, before.seconds, "the interval is kept");
        assert_eq!(after.random, before.random);
    }
}

#[test]
fn quick_cycle_without_interval_arms_a_timer() {
    let off = Cycle::default();
    let on = quick::toggled(&off);
    assert!(
        on.cycle && on.seconds > 0.0,
        "a 0 s cycle would never change a stand while standing"
    );
    let off = quick::toggled(&on);
    assert!(!off.cycle && off.seconds == on.seconds);
    let random = Cycle {
        cycle: false,
        random: true,
        seconds: 12.0,
    };
    assert_eq!(
        quick::toggled(&random),
        Cycle {
            cycle: true,
            random: true,
            seconds: 12.0
        }
    );
}

#[test]
fn quick_set_combo_stays_open_and_invalid_swimming_is_disabled() {
    let mut f = quick_fixture(true);
    f.click(aurora_i18n::tr!("ao-tile-swimming"));
    assert!(f.movement_prefs.swimming);
    let old_name = f.catalog.selected(f.account.selected).expect("set").name.clone();
    let other = Set::new("Other set".into());
    let other_id = other.id;
    f.catalog.sets.push(other);
    f.click(&old_name);
    f.settle();
    let actions = f.click("Other set");
    assert!(actions.iter().any(|a| matches!(a, Action::Preferences)));
    assert!(actions.iter().any(|a| matches!(a, Action::MovementPreferences)));
    assert_eq!(f.account.selected, other_id);
    f.settle();
    assert!(f.quick.as_ref().expect("quick").open, "child combo selection keeps its parent");
    assert!(!f.movement_prefs.swimming);
    assert!(f.click(aurora_i18n::tr!("ao-tile-swimming")).is_empty());
    assert!(!f.movement_prefs.swimming);
    f.click("AO");
    assert!(!f.quick.as_ref().expect("quick").open, "second toolbar click dismisses the panel");
}

#[test]
fn quick_panel_dismisses_on_escape_and_outside_click_without_changing_preferences() {
    let mut f = quick_fixture(false);
    let before = f.movement_prefs.clone();
    assert!(
        f.draw(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default()
        }])
        .0
        .is_empty()
    );
    assert!(!f.quick.as_ref().expect("quick").open);
    f.click("AO");
    f.settle();
    let pos = egui::pos2(30.0, 50.0);
    for pressed in [true, false] {
        assert!(
            f.draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default()
                }
            ])
            .0
            .is_empty()
        );
    }
    assert!(!f.quick.as_ref().expect("quick").open);
    assert_eq!(f.movement_prefs, before);
}

/// A selection dragged from the inventory onto the AO's list
/// (FloaterAO::handleDragAndDrop, ACCEPT_YES_MULTI): one edit links every
/// animation; a selection holding a folder is refused and says why.
#[test]
fn a_selection_dragged_from_the_inventory_adds_every_animation_at_once() {
    let mut f = Fixture::new();
    let set = f.catalog.selected(f.account.selected).expect("set").clone();
    let walks: Vec<Uuid> = set.states[&State::Walking]
        .steps
        .iter()
        .flat_map(|s| &s.members)
        .map(|a| a.original)
        .chain(
            set.states[&State::Running]
                .steps
                .iter()
                .flat_map(|s| &s.members)
                .map(|a| a.original),
        )
        .take(2)
        .collect();
    assert_eq!(walks.len(), 2, "the demo set walks and runs");
    fn locate(shape: &egui::Shape, name: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == name => Some(t.pos + egui::vec2(5.0, 5.0)),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| locate(s, name)),
            _ => None,
        }
    }
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let release = |f: &mut Fixture, ids: Vec<Uuid>| {
        let shapes = f.draw(Vec::new()).1;
        let over = shapes.iter().find_map(|s| locate(&s.shape, "Aurora Stand A")).expect("list");
        let away = egui::pos2(4.0, 4.0);
        f.draw(vec![egui::Event::PointerMoved(away), press(away, true)]);
        crate::ui::inventory::drag::demo_carry(&f.ctx, ids);
        f.draw(Vec::new());
        assert!(
            f.draw(vec![egui::Event::PointerMoved(over)]).0.is_empty(),
            "nothing before the release"
        );
        let out = f.draw(vec![press(over, false)]).0;
        assert!(!egui::DragAndDrop::has_any_payload(&f.ctx), "the drop takes the cargo");
        out
    };

    let out = release(&mut f, walks.clone());
    let [Action::Edit(change, None)] = out.as_slice() else {
        panic!("one inventory edit for the whole selection: {}", out.len())
    };
    assert!(
        matches!(change.operations.last(), Some(Operation::Links { links, .. }) if links.len() == 2),
        "{:?}",
        change.operations
    );

    let out = release(&mut f, vec![walks[0], set.id]);
    assert!(out.is_empty(), "a folder in the selection refuses the batch");
    assert_eq!(f.state.message, aurora_i18n::tr!("ao-not-animation"));
}

/// The order menu covers every saved `:CY` / `:RN` pair, Manual keeping the
/// random flag; the slider's stops round a typed time to the nearest one.
#[test]
fn order_menu_and_slider_stops_map_the_saved_cycle() {
    use super::slot::{Order, STOPS, stop_of};
    let cycle = |cycle, random, seconds| Cycle { cycle, random, seconds };
    assert_eq!(Order::of(&cycle(false, false, 0.0)), Order::Manual);
    assert_eq!(Order::of(&cycle(false, true, 0.0)), Order::Manual, "random alone never switches");
    assert_eq!(Order::of(&cycle(true, false, 0.0)), Order::Sequence);
    assert_eq!(Order::of(&cycle(true, true, 30.0)), Order::Shuffle);
    let shuffled = cycle(true, true, 20.0);
    let manual = Order::Manual.apply(&shuffled);
    assert_eq!(manual, cycle(false, true, 20.0), "Manual keeps the random flag and the time");
    assert_eq!(Order::Sequence.apply(&manual), cycle(true, false, 20.0));
    assert_eq!(Order::Shuffle.apply(&manual), cycle(true, true, 20.0));
    assert_eq!(stop_of(0.0), 0, "Each time");
    assert_eq!(stop_of(-3.0), 0);
    assert_eq!(stop_of(12.0), 1);
    assert_eq!(stop_of(45.1), 4);
    assert_eq!(stop_of(3600.0), STOPS.len() - 1);
    for (i, s) in STOPS.iter().enumerate() {
        assert_eq!(stop_of(*s), i);
    }
}

/// Duplicate copies the set it is asked on, and the set in use stays.
#[test]
fn duplicating_another_set_copies_it_and_keeps_the_set_in_use() {
    let mut f = Fixture::new();
    let mut other = f.catalog.selected(f.account.selected).expect("set").clone();
    other.id = Uuid::from_u128(0x0e7e);
    other.name = "Other set".into();
    other.states.retain(|s, _| *s == State::Walking);
    f.catalog.sets.push(other.clone());
    let in_use = f.account.selected;
    f.state.sets_open = true;
    f.settle();
    // the second set's ⋯ menu
    fn meshes(shape: &egui::Shape, texture: egui::TextureId, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Mesh(m) if m.texture_id == texture => out.push(m.calc_bounds()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| meshes(s, texture, out)),
            _ => {}
        }
    }
    let dots = super::super::icons::global("dots").expect("dots").id();
    let mut found = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        meshes(&shape.shape, dots, &mut found);
    }
    found.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let second = *found.get(1).expect("a menu per set");
    f.click_at(second.center());
    assert!(f.click(aurora_i18n::tr!("ao-duplicate")).is_empty());
    assert_eq!(f.account.selected, in_use, "the set in use stays");
    assert_eq!(f.state.new_dialog, Some(SetDialog::Clone(other.id)));
    f.state.name = "Copy of other".into();
    f.settle();
    let out = f.click(aurora_i18n::tr!("common-save"));
    let [Action::Create(copy)] = out.as_slice() else {
        panic!("one set made")
    };
    assert_eq!(copy.name, "Copy of other");
    assert_eq!(copy.states.keys().collect::<Vec<_>>(), [&State::Walking], "the other set's slots");
    assert_eq!(f.account.selected, in_use);
}

/// "Add to this stack" picks into the stack once: the slot's own drops and
/// the next pick go to the main list again.
#[test]
fn adding_to_a_stack_does_not_send_later_animations_into_it() {
    let mut f = Fixture::new();
    let chosen = std::collections::HashSet::from([steps_of(&f)[0]]);
    let (change, _) = inventory::editor::combine(
        &f.inv,
        f.catalog.selected(f.account.selected).expect("set"),
        State::Standing,
        "Together",
        false,
        &chosen,
    )
    .expect("stack");
    crate::world::inventory::actions::demo_mutate(&mut f.inv, Uuid::from_u128(1), change).expect("server ack");
    f.catalog.invalidate();
    f.catalog.refresh(&mut f.inv);
    let stack = steps_of(&f).into_iter().find(|id| f.inv.folders.contains_key(id)).expect("stack");
    f.settle();
    // the stack's right-click menu
    click_with(&mut f, "Aurora Stand A", egui::PointerButton::Secondary, Default::default());
    f.click(aurora_i18n::tr!("ao-add-to-stack"));
    assert_eq!(f.state.picker, Some(false));
    assert_eq!(f.state.pick_into, Some(stack));
    assert_eq!(f.state.container, None, "the slot still shows its main list");
    f.state.filter = "Type".into();
    f.settle();
    // the picker's row, over the page (its Typing slot shows the same name)
    let mut rows = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        text_rects(&shape.shape, "Aurora Type", &mut rows);
    }
    let out = f.click_at(rows.last().expect("the picker's row").center());
    assert!(matches!(&out[..], [Action::Edit(change, None)] if change.operations.iter().any(|op|
        matches!(op, Operation::Links { parent, .. } if *parent == stack))));
    assert_eq!(f.state.pick_into, None);
    f.settle();
    f.click(aurora_i18n::tr!("ao-drop-box"));
    assert_eq!(f.state.pick_into, None, "the drop box picks into the main list");
}

/// A window saved at the old one-column width opens once at the three
/// columns' size; a width chosen after that stays.
#[test]
fn an_old_narrow_ao_window_widens_once() {
    let mut f = Fixture::new();
    // the old window's size, as a saved layout brings it back (below the
    // new minimum: the narrowest it can be now)
    super::super::widgets::place_floater(&f.ctx, "ao", egui::pos2(40.0, 40.0), egui::vec2(560.0, 760.0));
    for _ in 0..10 {
        f.draw(Vec::new());
    }
    // a layout saved before the three columns: not widened yet
    f.ctx.data_mut(|d| d.remove::<bool>(egui::Id::unique("ao_three_columns")));
    for _ in 0..10 {
        f.draw(Vec::new());
    }
    let width = |f: &Fixture| f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao"))).expect("AO shown").width();
    assert!(width(&f) >= 1200.0, "widened: {}", width(&f));
    super::super::widgets::place_floater(&f.ctx, "ao", egui::pos2(40.0, 40.0), egui::vec2(900.0, 600.0));
    for _ in 0..10 {
        f.draw(Vec::new());
    }
    assert!(width(&f) < 1000.0, "a width chosen later stays: {}", width(&f));
}

/// The window opens with every slot folded; a click on a slot's title
/// unfolds it, another folds it, and an unfolded slot stays so while the
/// groups are flipped through.
#[test]
fn slots_start_folded_and_stay_unfolded_across_groups() {
    use super::category::Category;
    let mut f = Fixture::new();
    f.state.open_slots = Default::default();
    f.settle();
    assert!(f.state.open_slots.iter().all(Option::is_none), "nothing unfolded at first");
    f.click(State::Standing.label());
    assert_eq!(f.state.open_slots[Category::General.index()], Some(State::Standing));
    f.click(Category::Ground.label());
    assert_eq!(f.state.open_slots[Category::Ground.index()], None, "another group: as it was");
    f.click(Category::General.label());
    assert_eq!(
        f.state.open_slots[Category::General.index()],
        Some(State::Standing),
        "back to General: Standing is still unfolded"
    );
    assert_eq!(f.state.state, State::Standing);
    f.settle();
    f.click(State::Standing.label());
    assert_eq!(f.state.open_slots[Category::General.index()], None, "its title folds it again");
}

/// A folded slot unfolds from a click anywhere on its card, its chips too.
#[test]
fn a_folded_slot_unfolds_from_anywhere_on_its_card() {
    let mut f = Fixture::new();
    f.state.open_slots = Default::default();
    f.settle();
    let mut chips = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        text_rects(&shape.shape, "Aurora Sit", &mut chips);
    }
    let chip = chips.iter().min_by(|a, b| a.top().total_cmp(&b.top())).expect("Sitting's chip");
    // beside the chip, under the title: no widget of its own there
    f.click_at(chip.right_center() + egui::vec2(60.0, 0.0));
    assert_eq!(f.state.open_slots[0], Some(State::Sitting));
}

/// The preview's left edge drags it narrower and wider again (kept), the
/// page keeping its least.
#[test]
fn the_preview_resizes_from_its_left_edge() {
    let mut f = Fixture::new();
    let title = |f: &mut Fixture| {
        let mut rects = Vec::new();
        for shape in f.draw(Vec::new()).1 {
            text_rects(&shape.shape, aurora_i18n::tr!("ao-preview"), &mut rects);
        }
        // (the previewed row says "Preview" too: the title is the one right)
        rects.iter().map(|r| r.left()).fold(f32::MIN, f32::max)
    };
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    // the column's edge is left of its title (the eye icon, then the title)
    let drag = |f: &mut Fixture, dx: f32| {
        let edge = egui::pos2(title(f) - 36.0, 400.0);
        f.draw(vec![egui::Event::PointerMoved(edge), press(edge, true)]);
        for step in [0.25, 0.5, 1.0] {
            f.draw(vec![egui::Event::PointerMoved(edge + egui::vec2(dx * step, 0.0))]);
        }
        let end = edge + egui::vec2(dx, 0.0);
        f.draw(vec![egui::Event::PointerMoved(end), press(end, false)]);
        f.settle();
    };
    let before = title(&mut f);
    drag(&mut f, 60.0);
    let narrower = title(&mut f);
    assert!(
        (narrower - before - 60.0).abs() < 2.0,
        "dragged right, narrower: {before} -> {narrower}"
    );
    drag(&mut f, -40.0);
    let wider = title(&mut f);
    assert!((narrower - wider - 40.0).abs() < 2.0, "dragged left, wider: {narrower} -> {wider}");
    // never past the page's least
    drag(&mut f, -600.0);
    // (the window's body starts at x 160: the sidebar, then the page)
    let page = title(&mut f) - 36.0 - 160.0 - super::SIDEBAR;
    assert!((page - super::PAGE_LEAST).abs() < 3.0, "the page keeps its least: {page}");
}

/// A saved edit says so over the preview a moment, not above the page.
#[test]
fn a_saved_edit_shows_a_moment_over_the_preview() {
    let mut f = Fixture::new();
    f.state.message = "an older line".into();
    f.state.note_saved();
    assert!(f.state.message.is_empty());
    assert!(shows_text(&mut f, aurora_i18n::tr!("ao-saved")));
    f.state.saved_at = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(3));
    f.settle();
    assert!(!shows_text(&mut f, aurora_i18n::tr!("ao-saved")), "gone after its moment");
}

/// Opening a set in "Your sets" opens it to edit and leaves the set in use
/// playing; "Use this set" puts it in use (FloaterAO::onSelectSet and
/// onClickActivate).
#[test]
fn opening_a_set_edits_it_and_use_this_set_plays_it() {
    let mut f = Fixture::new();
    let mut other = f.catalog.selected(f.account.selected).expect("set").clone();
    other.id = Uuid::from_u128(0x0e7e);
    other.name = "Other set".into();
    other.default = false;
    f.catalog.sets.push(other.clone());
    let in_use = f.account.selected;
    f.state.sets_open = true;
    f.settle();
    let out = f.click("Other set");
    assert!(out.is_empty(), "nothing saved: {}", out.len());
    assert_eq!(f.state.editing, Some(other.id), "open to edit");
    assert_eq!(f.account.selected, in_use, "the set in use keeps playing");
    assert!(!f.state.sets_open);
    f.settle();
    let out = f.click(aurora_i18n::tr!("ao-use-this-set"));
    assert!(matches!(&out[..], [Action::Preferences]));
    assert_eq!(f.account.selected, other.id, "in use now");
    f.settle();
    assert!(!shows_text(&mut f, aurora_i18n::tr!("ao-use-this-set")), "the banner goes");
}

/// An entry of the open set played in the world puts its set in use first,
/// as FloaterAO's double-click does.
#[test]
fn playing_an_entry_of_another_set_puts_it_in_use() {
    let mut f = Fixture::new();
    let mut other = f.catalog.selected(f.account.selected).expect("set").clone();
    other.id = Uuid::from_u128(0x0e7f);
    other.name = "Other set".into();
    other.default = false;
    f.catalog.sets.push(other.clone());
    f.state.editing = Some(other.id);
    let in_use = f.account.selected;
    // another state plays (walking): the set is left alone, as FloaterAO's
    // double-click does, and the entry waits for its state
    f.runtime.current = Some(State::Walking);
    f.settle();
    click_with(&mut f, "Aurora Stand B", egui::PointerButton::Secondary, Default::default());
    assert!(f.click(aurora_i18n::tr!("ao-play-in-world")).is_empty());
    assert_eq!(f.account.selected, in_use, "not for another state");
    // standing: the set goes in use with the entry
    f.runtime.current = Some(State::Standing);
    f.settle();
    click_with(&mut f, "Aurora Stand B", egui::PointerButton::Secondary, Default::default());
    let out = f.click(aurora_i18n::tr!("ao-play-in-world"));
    assert!(matches!(&out[..], [Action::Preferences]));
    assert_eq!(f.account.selected, other.id);
}

/// A set's line dragged down "Your sets" moves it there, the order kept in
/// the account's settings.
#[test]
fn a_set_dragged_in_your_sets_keeps_its_new_place() {
    let mut f = Fixture::new();
    let first = f.catalog.selected(f.account.selected).expect("set").clone();
    for (n, name) in [(0x0e80u128, "Zeta set"), (0x0e81, "Zulu set")] {
        let mut other = first.clone();
        other.id = Uuid::from_u128(n);
        other.name = name.into();
        other.default = false;
        f.catalog.sets.push(other);
    }
    f.state.sets_open = true;
    f.settle();
    let mut rows = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        text_rects(&shape.shape, &first.name, &mut rows);
    }
    // the set's line in the list (the set card above shows its name too)
    let from = rows.iter().max_by(|a, b| a.top().total_cmp(&b.top())).expect("its line").center();
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    f.draw(vec![egui::Event::PointerMoved(from), press(from, true)]);
    let mut out = Vec::new();
    for dy in [10.0, 40.0, 80.0, 110.0, 110.0] {
        out.extend(f.draw(vec![egui::Event::PointerMoved(from + egui::vec2(0.0, dy))]).0);
    }
    let end = from + egui::vec2(0.0, 110.0);
    out.extend(f.draw(vec![egui::Event::PointerMoved(end), press(end, false)]).0);
    for _ in 0..30 {
        out.extend(f.draw(Vec::new()).0);
    }
    assert!(out.iter().any(|a| matches!(a, Action::Preferences)), "the order is saved");
    let order: Vec<String> = f.account.ordered(&f.catalog.sets).iter().map(|s| s.name.clone()).collect();
    assert_eq!(order.last(), Some(&first.name), "moved to the end: {order:?}");
    assert_eq!(f.account.selected, first.id, "a drag does not change the set in use");
}

/// A dropped entry shows in its new place at once, while it is saved; a
/// move made during that save waits its turn and is sent once it is back.
#[test]
fn a_dropped_entry_stays_put_and_a_move_during_a_save_waits() {
    let mut f = Fixture::new();
    fn top(f: &mut Fixture, name: &str) -> f32 {
        let mut rects = Vec::new();
        for shape in f.draw(Vec::new()).1 {
            text_rects(&shape.shape, name, &mut rects);
        }
        rects.iter().map(|r| r.top()).fold(f32::MAX, f32::min)
    }
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let drag = |f: &mut Fixture, from: &str, to: &str| {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for shape in f.draw(Vec::new()).1 {
            text_rects(&shape.shape, from, &mut a);
            text_rects(&shape.shape, to, &mut b);
        }
        let (from, to) = (a[0].center(), b[0].center());
        let mut out = Vec::new();
        out.extend(f.draw(vec![egui::Event::PointerMoved(from), press(from, true)]).0);
        out.extend(f.draw(vec![egui::Event::PointerMoved(from + egui::vec2(9.0, 0.0))]).0);
        out.extend(f.draw(vec![egui::Event::PointerMoved(to)]).0);
        out.extend(f.draw(vec![press(to, false)]).0);
        out
    };
    let out = drag(&mut f, "Aurora Stand B", "Aurora Stand A");
    let [Action::Edit(first, None)] = out.as_slice() else {
        panic!("one edit sent")
    };
    // the app takes it: a save under way
    f.state.pending = Some(Pending {
        request: Uuid::from_u128(9),
        refresh: Vec::new(),
        select: None,
        editor: None,
    });
    for _ in 0..20 {
        f.draw(Vec::new());
    }
    assert!(
        top(&mut f, "Aurora Stand B") < top(&mut f, "Aurora Stand A"),
        "shown where it was dropped"
    );
    // another move while it saves: nothing sent yet
    let out = drag(&mut f, "Aurora Stand A", "Aurora Stand B");
    assert!(out.is_empty(), "waits its turn");
    for _ in 0..20 {
        assert!(f.draw(Vec::new()).0.is_empty());
    }
    // the slot folded meanwhile: the move still waits, and goes all the same
    f.state.open_slots = Default::default();
    f.settle();
    // the first save comes back: the waiting move goes
    crate::world::inventory::actions::demo_mutate(&mut f.inv, Uuid::from_u128(1), first.clone()).expect("ack");
    f.catalog.invalidate();
    f.catalog.refresh(&mut f.inv);
    f.state.pending = None;
    let out = f.draw(Vec::new()).0;
    assert!(matches!(out.as_slice(), [Action::Edit(_, None)]), "the waiting move is sent");
}

/// The track of stops puts a time at its stop, and a typed time between the
/// two stops around it.
#[test]
fn a_typed_time_sits_between_its_two_stops() {
    use super::slot::stop_place_for_tests as place;
    assert_eq!(place(0.0), 0.0, "Each time: the start");
    assert!((place(10.0) - 0.2).abs() < 1e-5, "10 s: the second stop");
    assert!((place(120.0) - 1.0).abs() < 1e-5, "2 min: the end");
    assert!((place(15.0) - 0.3).abs() < 1e-5, "15 s: half way from 10 s to 20 s");
    assert_eq!(place(600.0), 1.0, "past 2 min: at the end");
}

/// A move saved and read back keeps its place once "Set saved" shows (the
/// first of six moved to the fifth place).
#[test]
fn a_moved_entry_keeps_its_place_after_its_save() {
    let mut f = Fixture::new();
    f.inv = Inventory::default();
    f.account.selected = crate::ao::demo::seed_interface(&mut f.inv, Uuid::from_u128(1), "interface");
    f.catalog = Default::default();
    f.catalog.refresh(&mut f.inv);
    f.state.open_slots[0] = Some(State::Standing);
    f.state.state = State::Standing;
    f.settle();
    let names = |f: &Fixture| -> Vec<String> {
        f.catalog.selected(f.account.selected).expect("set").states[&State::Standing]
            .steps
            .iter()
            .map(|s| s.name.clone())
            .collect()
    };
    eprintln!("DEBUG before {:?}", names(&f));
    let row = |f: &mut Fixture, name: &str| {
        let mut rects = Vec::new();
        for shape in f.draw(Vec::new()).1 {
            text_rects(&shape.shape, name, &mut rects);
        }
        rects.iter().copied().min_by(|a, b| a.left().total_cmp(&b.left())).expect(name)
    };
    let from = row(&mut f, "Aurora Stand A").center();
    let to = row(&mut f, "Aurora Stand E").center() + egui::vec2(0.0, 4.0);
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut out = Vec::new();
    out.extend(f.draw(vec![egui::Event::PointerMoved(from), press(from, true)]).0);
    for k in 1..=10 {
        let at = from + (to - from) * (k as f32 / 10.0);
        out.extend(f.draw(vec![egui::Event::PointerMoved(at)]).0);
    }
    out.extend(f.draw(vec![press(to, false)]).0);
    let edits: Vec<_> = out
        .iter()
        .filter_map(|a| match a {
            Action::Edit(change, None) => Some(change.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(edits.len(), 1, "one edit");
    f.state.pending = Some(Pending {
        request: Uuid::from_u128(9),
        refresh: Vec::new(),
        select: None,
        editor: None,
    });
    for _ in 0..10 {
        let more = f.draw(Vec::new()).0;
        assert!(more.is_empty(), "nothing else sent while it saves");
    }
    crate::world::inventory::actions::demo_mutate(&mut f.inv, Uuid::from_u128(1), edits[0].clone()).expect("ack");
    f.catalog.invalidate();
    f.catalog.refresh(&mut f.inv);
    f.state.pending = None;
    f.state.note_saved();
    eprintln!("DEBUG saved {:?}", names(&f));
    for _ in 0..10 {
        let more = f.draw(Vec::new()).0;
        assert!(more.is_empty(), "nothing sent after the save: {}", more.len());
    }
    assert_eq!(names(&f)[4], "Aurora Stand A", "saved in the fifth place: {:?}", names(&f));
    let shown_a = row(&mut f, "Aurora Stand A").top();
    let shown_e = row(&mut f, "Aurora Stand E").top();
    assert!(shown_a > shown_e, "shown in its new place after the save");
}

/// A click on the strip's field lists the sets (its right-click menu, on
/// the same field, keeps an id of its own), and a set picked there is used.
#[test]
fn the_mini_strip_lists_the_sets_on_a_click() {
    let mut f = Fixture::new();
    let mut other = f.catalog.selected(f.account.selected).expect("set").clone();
    other.id = Uuid::from_u128(0x0e90);
    other.name = "Other set".into();
    other.default = false;
    f.catalog.sets.push(other.clone());
    f.account.mini_open = true;
    f.manager_open = false;
    f.settle();
    let name = f.catalog.selected(f.account.selected).expect("set").name.to_uppercase();
    f.click(&name);
    f.settle();
    let out = f.click("Other set");
    assert!(out.iter().any(|a| matches!(a, Action::Preferences)));
    assert_eq!(f.account.selected, other.id);
}
