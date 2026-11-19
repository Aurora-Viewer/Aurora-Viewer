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
    water_volumes: crate::movement::water_volume::Settings,
    keys: crate::keybinds::KeyBindings,
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
            water_volumes: Default::default(),
            keys: Default::default(),
            quick: None,
            manager_open: true,
        };
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
                &mut self.water_volumes,
                &self.keys,
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

#[test]
fn ao_cycle_footer_is_fully_visible_after_scrolling_with_large_text() {
    fn collect(shape: &egui::Shape, text: &str, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => out.push(egui::Rect::from_min_size(t.pos, t.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, text, out)),
            _ => {}
        }
    }
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
    let mut headings = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        collect(&shape.shape, State::Standing.label(), &mut headings);
    }
    let heading = headings
        .into_iter()
        .max_by(|a, b| a.left().total_cmp(&b.left()))
        .expect("detail heading");
    let at = egui::pos2(heading.left() + 40.0, heading.bottom() + 300.0);
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
    let mut found = false;
    for shape in f.draw(Vec::new()).1 {
        let mut labels = Vec::new();
        collect(&shape.shape, aurora_i18n::tr!("common-apply"), &mut labels);
        for rect in labels {
            found = true;
            assert!(
                shape.clip_rect.contains_rect(rect),
                "the complete Apply label stays inside the scroll viewport: {rect:?} in {:?}",
                shape.clip_rect
            );
        }
    }
    assert!(found, "the cycle footer is visible at the end of the detail pane");
}

#[test]
fn ao_group_and_track_rows_keep_their_summary_line() {
    fn galley(shape: &egui::Shape, text: &str) -> Option<std::sync::Arc<egui::Galley>> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| galley(s, text)),
            _ => None,
        }
    }
    for mode in ["interface-group", "interface-track"] {
        let mut f = Fixture::new();
        f.inv = Inventory::default();
        f.account.selected = crate::ao::demo::seed_interface(&mut f.inv, Uuid::from_u128(1), mode);
        f.catalog = Default::default();
        f.catalog.refresh(&mut f.inv);
        f.settle();
        let c = &f.catalog.selected(f.account.selected).expect("set").states[&State::Standing];
        let (name, count) = if mode == "interface-group" {
            let group = c.steps.iter().find(|s| s.group).expect("group");
            (group.name.clone(), group.members.len())
        } else {
            let track = c.tracks.first().expect("track");
            (track.name.clone(), track.members.len())
        };
        let summary = if mode == "interface-group" {
            aurora_i18n::tr!("ao-group-summary", count = count)
        } else {
            aurora_i18n::tr!("ao-track-summary", count = count)
        };
        let text = format!("{name}\n{summary}");
        let shapes = f.draw(Vec::new()).1;
        let text = shapes.iter().find_map(|s| galley(&s.shape, &text)).expect("group/track row");
        assert_eq!(text.rows.len(), 2, "{mode}: name and summary are separate visible rows");
        assert!(!text.elided, "{mode}: the summary is not elided");
    }
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
        super::super::widgets::place_floater(&f.ctx, "ao", egui::pos2(40.0, 40.0), egui::vec2(560.0, 760.0));
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
        super::super::widgets::place_floater(&f.ctx, "ao", before.min, egui::vec2(440.0, 560.0));
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
        let texture = super::super::icons::global("trash-filled").expect("remove icon").id();
        let mut icons = Vec::new();
        fn collect(shape: &egui::Shape, texture: egui::TextureId, icons: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture => icons.push(mesh.calc_bounds()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texture, icons)),
                _ => {}
            }
        }
        for shape in f.draw(Vec::new()).1 {
            collect(&shape.shape, texture, &mut icons);
        }
        assert_eq!(icons.len(), 3, "set and both animation removal buttons");
        assert!(
            icons.iter().all(|r| rects[29].contains_rect(*r)),
            "AO at scale {zoom}: removal icons stay in the window"
        );
        let first = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing].steps[0].id;
        let out = f.click_at(icons[1].center());
        assert!(
            matches!(&out[..], [Action::Edit(change, None)]
                if matches!(&change.operations[..], [Operation::Delete { id, folder: false }] if *id == first)),
            "the first animation's removal button is accessible"
        );
    }
}

/// The removal buttons of the AO window, top to bottom: the set's, then one
/// per row of the list.
fn trash_icons(f: &mut Fixture) -> Vec<egui::Rect> {
    fn collect(shape: &egui::Shape, texture: egui::TextureId, icons: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => icons.push(mesh.calc_bounds()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texture, icons)),
            _ => {}
        }
    }
    let texture = super::super::icons::global("trash-filled").expect("remove icon").id();
    let mut icons = Vec::new();
    for shape in f.draw(Vec::new()).1 {
        collect(&shape.shape, texture, &mut icons);
    }
    icons
}

/// #3675: the trash button of an animation asked first, and the reporter never
/// saw the question. An animation is a link: it goes at once, like
/// FloaterAO::onClickTrash.
#[test]
fn an_animation_is_removed_at_once_and_the_next_one_too() {
    let mut f = Fixture::new();
    let steps: Vec<Uuid> = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing]
        .steps
        .iter()
        .map(|s| s.id)
        .collect();
    let icons = trash_icons(&mut f);
    assert_eq!(icons.len(), 1 + steps.len(), "the set's button, then one per animation");
    for (icon, step) in icons[1..].iter().zip(&steps) {
        let out = f.click_at(icon.center());
        let [Action::Edit(change, None)] = out.as_slice() else {
            panic!("one inventory edit per click")
        };
        assert!(matches!(&change.operations[..], [Operation::Delete { id, folder: false }] if id == step));
        assert_eq!(f.state.remove, None, "no question for a single animation");
        assert!(f.ctx.memory(|m| m.area_rect(egui::Id::unique("ao_remove"))).is_none());
    }
}

/// #3675: a set and a group are folders, they still ask. The question opens
/// above the AO window each time, and unfolded when it was left minimized.
#[test]
fn removing_a_set_or_a_group_asks_on_top_of_the_ao_window() {
    let mut f = Fixture::new();
    // the AO window over the place where the question opens
    super::super::widgets::place_floater(&f.ctx, "ao", egui::pos2(200.0, 100.0), egui::vec2(560.0, 760.0));
    f.settle();
    let question = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("ao_remove"));
    for round in 0..2 {
        let icons = trash_icons(&mut f);
        assert!(f.click_at(icons[0].center()).is_empty(), "nothing is sent before the answer");
        assert_eq!(f.state.remove, Some(f.account.selected));
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
    let icons = trash_icons(&mut f);
    f.click_at(icons[0].center());
    assert!(
        !super::super::widgets::floater_minimized(&f.ctx, "ao_remove"),
        "the question opens unfolded"
    );
    f.click(aurora_i18n::tr!("common-cancel"));

    // a group of animations is a folder too
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
        .position(|id| f.inv.folders.contains_key(id))
        .expect("the group is a folder");
    let icons = trash_icons(&mut f);
    assert_eq!(icons.len(), 1 + steps.len());
    assert!(f.click_at(icons[1 + group].center()).is_empty(), "a group asks first");
    assert_eq!(f.state.remove, Some(steps[group]));
    f.draw(Vec::new());
    let rect = f.ctx.memory(|m| m.area_rect(question.id)).expect("the question is drawn");
    assert_eq!(f.ctx.layer_id_at(rect.center()), Some(question));
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
        &[egui::vec2(560.0, 760.0), egui::vec2(440.0, 560.0)],
        aurora_i18n::tr!("ao-add"),
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
                &mut f.water_volumes,
                &f.keys,
                "",
            );
        },
    );
}

#[test]
fn enable_label_and_pause_change_runtime_without_inventory_writes() {
    let mut f = Fixture::new();
    let out = f.click(aurora_i18n::tr!("ao-enabled"));
    assert!(f.account.enabled);
    assert!(matches!(&out[..], [Action::Preferences]));
    assert!(f.click(aurora_i18n::tr!("ao-pause")).is_empty());
    assert!(f.runtime.paused);
    assert!(f.click(aurora_i18n::tr!("ao-stands")).is_empty());
    assert!(!f.runtime.stands);
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
    assert!(f.click(aurora_i18n::tr!("ao-migrate")).is_empty());
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
    assert!(f.click(aurora_i18n::tr!("ao-migrate")).is_empty());
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
    f.state.name = "Renamed".into();
    f.state.new_dialog = Some(SetDialog::Rename);
    f.settle();
    let out = f.click(aurora_i18n::tr!("common-save"));
    assert!(matches!(&out[..], [Action::Edit(change, None)] if matches!(&change.operations[..],
        [Operation::Patch { id, folder: true, .. }] if *id == f.account.selected)));
    assert!(f.state.new_dialog.is_none());
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
    let folder = f.catalog.selected(f.account.selected).expect("set").states[&State::Walking].folder;
    f.click(aurora_i18n::tr!("ao-add"));
    assert_eq!(f.state.picker, Some(false));
    f.state.filter = "Type".into();
    f.settle();
    let out = f.click("Aurora Type");
    assert!(matches!(&out[..], [Action::Edit(change, None)] if change.operations.iter().any(|op|
        matches!(op, Operation::Links { parent, links } if *parent == folder && links.len() == 1))));
    assert!(f.state.picker.is_none());
    f.state.import_folder = Some(crate::ao::demo::NOTECARD);
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-add")).is_empty());
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
fn design_movement_switches_are_independent_of_ao_and_never_write_inventory() {
    let mut f = Fixture::new();
    f.state.prototype = Some(prototype::Prototype::new("design-movement"));
    f.settle();
    assert!(f.click(aurora_i18n::tr!("movement-running")).is_empty());
    let draft = f.state.prototype.as_ref().expect("design");
    assert!(draft.movement.running && !draft.movement.fly_assist && !draft.movement.swimming);
    assert!(!f.account.enabled);
    assert!(f.click(aurora_i18n::tr!("ao-design-animations")).is_empty());
    assert!(matches!(&f.click(aurora_i18n::tr!("ao-enabled"))[..], [Action::Preferences]));
    assert!(f.state.prototype.as_ref().expect("design").movement.running);
}

#[test]
fn design_swimming_choice_returns_to_the_actual_ao_state_editor() {
    let mut f = Fixture::new();
    f.state.prototype = Some(prototype::Prototype::new("design-movement"));
    f.settle();
    assert!(f.click(aurora_i18n::tr!("ao-design-swim-animations")).is_empty());
    assert_eq!(f.state.state, State::SwimmingForward);
    let draft = f.state.prototype.as_mut().expect("design");
    assert_eq!(draft.tab, 0);
    assert!(draft.underwater());
    let now = std::time::Instant::now();
    let first = draft.signal(now);
    let later = draft.signal(now + std::time::Duration::from_secs(1));
    assert_eq!(first, later, "a preview must not restart its animation every frame");
}

#[test]
fn movement_tab_preserves_independent_preferences_without_enabling_ao() {
    let mut f = Fixture::new();
    f.click(aurora_i18n::tr!("ao-tab-movement"));
    assert_eq!(f.state.tab, 1);
    let actions = f.click(aurora_i18n::tr!("movement-running"));
    assert!(f.movement_prefs.running);
    assert!(!f.account.enabled);
    assert!(matches!(actions.as_slice(), [Action::MovementPreferences]));
    f.click(aurora_i18n::tr!("ao-tab-animations"));
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
    f.state.container = None;
    assert!(!f.state.finish_editor(target, &created));
    assert!(f.state.container.is_none());
}

#[test]
fn guided_combinations_are_cancellable_and_wait_for_acknowledgement() {
    let mut f = Fixture::new();
    let generation = f.inv.generation;
    assert!(f.click(aurora_i18n::tr!("ao-combine")).is_empty());
    assert!(f.state.combination.is_some());
    assert!(f.click(aurora_i18n::tr!("common-cancel")).is_empty());
    assert!(f.state.combination.is_none());
    assert_eq!(f.inv.generation, generation);
    f.click(aurora_i18n::tr!("ao-combine"));
    let chosen = f.catalog.selected(f.account.selected).expect("set").states[&State::Standing].steps[0].id;
    let dialog = f.state.combination.as_mut().expect("dialog");
    dialog.name = "Together".into();
    dialog.selected.insert(chosen);
    f.settle();
    let out = f.click(aurora_i18n::tr!("ao-create-group"));
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
    assert!(f.click("Aurora Stand B").is_empty());
    assert_eq!(f.runtime.index(State::Standing, folder), 1);
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
fn water_control_hints_follow_rebound_secondary_modified_mouse_and_unassigned_keys() {
    use crate::keybinds::{Action as KeyAction, Binding, Input, Mods};
    use winit::keyboard::KeyCode;
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let contents = |f: &mut Fixture| {
        let mut out = Vec::new();
        for shape in f.draw(Vec::new()).1 {
            texts(&shape.shape, &mut out);
        }
        out
    };
    // Passage is Control plus the jump keys. The cap is "Ctrl" elsewhere and
    // "⌘" on macOS (LLKeyboard::stringFromAccelerator); either way it is the
    // same cap the water hints draw.
    let modifier = |binding: &Binding| binding.caps().into_iter().next().expect("modifier cap").label;
    let mut f = Fixture::new();
    f.click(aurora_i18n::tr!("ao-tab-movement"));
    f.settle();
    let initial = contents(&mut f);
    let passage = Binding::captured(
        Input::key(KeyCode::PageUp),
        Mods {
            ctrl: true,
            ..Default::default()
        },
    );
    assert!(initial.contains(&Input::key(KeyCode::PageUp).label()));
    assert!(
        initial.contains(&modifier(&passage)),
        "passage modifiers use the same keycaps as water gestures"
    );
    let first = Binding::captured(
        Input::key(KeyCode::F10),
        Mods {
            ctrl: true,
            ..Default::default()
        },
    );
    let mouse = Binding::captured(
        Input::Mouse("Other9".into()),
        Mods {
            alt: true,
            ..Default::default()
        },
    );
    let ctrl_cap = modifier(&first);
    let alt_cap = modifier(&mouse);
    f.keys.set(KeyAction::Up, 0, Some(first));
    f.keys.set(KeyAction::Up, 1, Some(mouse.clone()));
    f.keys.set(
        KeyAction::Down,
        0,
        Some(Binding::captured(Input::key(KeyCode::F11), Default::default())),
    );
    f.keys.set(KeyAction::Down, 1, None);
    f.settle();
    let shown = contents(&mut f);
    for key in ["F10".to_owned(), "F11".to_owned(), mouse.input.label(), ctrl_cap, alt_cap] {
        assert!(shown.contains(&key), "live keycap {key} is visible");
    }
    assert!(!shown.contains(&Input::key(KeyCode::PageUp).label()));
    assert!(!shown.contains(&Input::key(KeyCode::PageDown).label()));
    f.keys.set(
        KeyAction::Up,
        0,
        Some(Binding::captured(Input::key(KeyCode::F12), Default::default())),
    );
    f.keys.set(KeyAction::Up, 1, None);
    f.keys.set(KeyAction::Down, 0, None);
    f.settle();
    let shown = contents(&mut f);
    assert!(shown.contains(&"F12".to_owned()));
    assert!(shown.contains(&aurora_i18n::tr!("movement-key-unbound").to_owned()));
    for old in ["F10".to_owned(), "F11".to_owned(), mouse.input.label()] {
        assert!(!shown.contains(&old), "old binding {old} is no longer shown");
    }
}

#[test]
fn both_water_states_use_the_normal_list_and_combination_editor() {
    let mut f = Fixture::new();
    for state in [State::SwimmingForward, State::UnderwaterSwimming] {
        f.state.state = state;
        f.state.combination = None;
        f.state.container = None;
        f.settle();
        assert!(f.click(aurora_i18n::tr!("ao-combine")).is_empty());
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
    let beside = |icon: &egui::Rect, row: egui::Rect| {
        (icon.center().y - row.center().y).abs() < 1.0 && icon.left() > row.right() && icon.left() < row.right() + 150.0
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
    f.click(State::Always.label());
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
    assert!(matches!(f.click(aurora_i18n::tr!("ao-enabled")).as_slice(), [Action::Preferences]));
    assert!(f.account.enabled && !f.runtime.paused && f.runtime.stands);
    f.click(aurora_i18n::tr!("movement-swimming"));
    assert!(f.movement_prefs.swimming && !f.movement_prefs.water_walk);
    f.click(aurora_i18n::tr!("movement-water-walk"));
    assert!(f.movement_prefs.water_walk && !f.movement_prefs.swimming);
    f.click(aurora_i18n::tr!("movement-water-walk"));
    assert!(!f.movement_prefs.water_walk && !f.movement_prefs.swimming);
    f.click(aurora_i18n::tr!("ao-quick-manager"));
    assert!(f.manager_open);
    assert!(!f.quick.as_ref().expect("quick").open);
    assert_eq!(f.inv.generation, generation, "quick controls must not mutate inventory");
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
    f.click(aurora_i18n::tr!("movement-swimming"));
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
    assert!(f.click(aurora_i18n::tr!("movement-swimming")).is_empty());
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
