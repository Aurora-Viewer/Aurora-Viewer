use super::list::{Column, use_label};
use super::*;
use aurora_net::pathfinding::linkset::{Category, Linkset, Use};
use aurora_net::pathfinding::{Detail, Error, Row};
use std::collections::HashMap;

fn linkset(id: u128, name: &str, use_: Use, walkability: [u8; 4], modifiable: bool, can_be_volume: bool) -> Row {
    Row {
        id: Uuid::from_u128(id),
        name: name.into(),
        description: format!("{name} description"),
        owner: Uuid::from_u128(900),
        group_owned: false,
        position: [f64::from(id as u32), 0.0, 20.0],
        detail: Detail::Object {
            land_impact: id as u32,
            scripted: Some(id.is_multiple_of(2)),
            linkset: Linkset {
                use_,
                walkability,
                modifiable,
                can_be_volume,
                terrain: false,
            },
        },
    }
}

fn terrain() -> Row {
    let mut row = linkset(0, "", Use::Walkable, [100; 4], false, false);
    row.id = Uuid::nil();
    if let Detail::Object { linkset, .. } = &mut row.detail {
        linkset.terrain = true;
    }
    row
}

fn loaded(rows: Vec<Row>) -> List {
    let mut list = List::new(Kind::Objects);
    let request = list.start_get(1);
    list.on_list(request, 1, Ok(rows));
    prepare(&mut list);
    list
}

fn prepare(list: &mut List) {
    let mut owners = HashMap::new();
    owners.insert(Uuid::from_u128(900), Some("Owner Name".to_owned()));
    list.prepare(&Context {
        owners: &owners,
        agent: [0.0, 0.0, 20.0],
    });
}

fn names(list: &List) -> Vec<String> {
    (0..list.visible_len()).map(|n| list.cell(n, Column::Name).to_owned()).collect()
}

#[test]
fn replies_are_correlated_with_their_request_and_region() {
    let mut list = List::new(Kind::Objects);
    let request = list.start_get(1);
    list.on_list(Uuid::new_v4(), 1, Ok(vec![linkset(1, "a", Use::Walkable, [100; 4], true, true)]));
    list.on_list(request, 2, Ok(Vec::new()));
    assert!(list.is_pending());
    list.on_list(request, 1, Err(Error::Unsupported));
    assert_eq!(list.messaging, Messaging::NotEnabled);
    assert!(!list.is_pending());
    let request = list.start_get(1);
    list.on_list(request, 1, Err(Error::Failed));
    assert_eq!(list.messaging, Messaging::GetError);
}

#[test]
fn filters_apply_on_demand_and_match_without_case() {
    let mut list = loaded(vec![
        linkset(1, "Stool", Use::MovableObstacle, [100; 4], true, true),
        linkset(2, "Floor", Use::Walkable, [100; 4], true, true),
        linkset(3, "Bar stool", Use::Walkable, [100; 4], true, true),
    ]);
    list.filter.name = "STOOL".into();
    prepare(&mut list);
    assert_eq!(list.visible_len(), 3, "typing alone does not filter");
    list.apply_filters();
    prepare(&mut list);
    assert_eq!(names(&list), ["Bar stool", "Stool"]);
    list.filter.use_ = Some(Use::Walkable);
    list.apply_filters();
    prepare(&mut list);
    assert_eq!(names(&list), ["Bar stool"]);
    list.clear_filters();
    prepare(&mut list);
    assert_eq!(list.visible_len(), 3);
}

#[test]
fn the_name_filter_finds_the_terrain_by_its_shown_name() {
    // the terrain has no name of its own: the filter matches "[Terrain]",
    // as the Legacy Viewer's (linkset_terrain_name)
    let shown = aurora_i18n::text("pathfinding-terrain-name").to_owned();
    let mut list = loaded(vec![terrain(), linkset(1, "Floor", Use::Walkable, [100; 4], true, true)]);
    list.filter.name = shown.trim_matches(['[', ']']).to_uppercase();
    list.apply_filters();
    prepare(&mut list);
    assert_eq!(names(&list), [shown]);
}

#[test]
fn numbers_sort_by_value_and_names_without_case() {
    let mut list = loaded(vec![
        linkset(10, "b", Use::Walkable, [9, 100, 100, 100], true, true),
        linkset(2, "A", Use::Walkable, [80, 100, 100, 100], true, true),
        linkset(30, "c", Use::Walkable, [100, 100, 100, 100], true, true),
    ]);
    assert_eq!(names(&list), ["A", "b", "c"]);
    list.sort_by(Column::Impact);
    prepare(&mut list);
    assert_eq!(names(&list), ["A", "b", "c"]);
    list.sort_by(Column::Impact);
    prepare(&mut list);
    assert_eq!(names(&list), ["c", "b", "A"], "a second click reverses");
    list.sort_by(Column::A);
    prepare(&mut list);
    assert_eq!(names(&list), ["b", "A", "c"], "9 < 80 < 100");
    list.sort_by(Column::Distance);
    prepare(&mut list);
    assert_eq!(names(&list), ["A", "b", "c"]);
}

#[test]
fn terrain_and_state_suffixes_read_like_the_legacy_list() {
    let list = loaded(vec![
        terrain(),
        linkset(1, "Locked", Use::StaticObstacle, [100; 4], false, true),
        linkset(2, "Concave", Use::Walkable, [100; 4], true, false),
        linkset(3, "Both", Use::Walkable, [100; 4], false, false),
    ]);
    let row = |name: &str| {
        (0..list.visible_len())
            .find(|&n| list.cell(n, Column::Name) == name)
            .expect("a row")
    };
    let terrain = row(aurora_i18n::text("pathfinding-terrain-name"));
    assert_eq!(list.cell(terrain, Column::Owner), aurora_i18n::text("pathfinding-terrain-dash"));
    assert!(
        list.cell(terrain, Column::Use)
            .ends_with(aurora_i18n::text("pathfinding-use-terrain"))
    );
    assert!(
        list.cell(row("Locked"), Column::Use)
            .ends_with(aurora_i18n::text("pathfinding-use-restricted"))
    );
    assert!(
        list.cell(row("Concave"), Column::Use)
            .ends_with(aurora_i18n::text("pathfinding-use-concave"))
    );
    assert!(
        list.cell(row("Both"), Column::Use)
            .ends_with(aurora_i18n::text("pathfinding-use-restricted-concave"))
    );
    assert_eq!(list.cell(row("Locked"), Column::Owner), "Owner Name");
    assert_eq!(list.cell(row("Locked"), Column::A), "100");
}

#[test]
fn owners_wait_for_their_names() {
    let mut list = List::new(Kind::Objects);
    let request = list.start_get(1);
    list.on_list(request, 1, Ok(vec![linkset(1, "a", Use::Walkable, [100; 4], true, true)]));
    list.prepare(&Context {
        owners: &HashMap::new(),
        agent: [0.0; 3],
    });
    assert!(list.owners_loading());
    assert_eq!(list.cell(0, Column::Owner), aurora_i18n::text("pathfinding-owner-loading"));
}

#[test]
fn status_line_counts_the_selection_and_the_shown_rows() {
    let mut list = loaded(vec![
        linkset(1, "a", Use::Walkable, [100; 4], true, true),
        linkset(2, "b", Use::Walkable, [100; 4], true, true),
    ]);
    list.selection.select_all(vec![Uuid::from_u128(2)]);
    assert_eq!(
        list.status(),
        aurora_i18n::tr!("pathfinding-linksets-count", selected = 1i64, total = 2i64)
    );
    let empty = loaded(Vec::new());
    assert_eq!(empty.status(), aurora_i18n::text("pathfinding-linksets-none"));
}

#[test]
fn a_hidden_row_leaves_the_selection_and_updates_merge_by_id() {
    let mut list = loaded(vec![
        linkset(1, "keep", Use::Walkable, [100; 4], true, true),
        linkset(2, "hide", Use::Walkable, [100; 4], true, true),
    ]);
    list.selection.select_all(vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
    list.filter.name = "keep".into();
    list.apply_filters();
    prepare(&mut list);
    assert_eq!(list.selection.ids, [Uuid::from_u128(1)]);
    let request = list.start_set();
    assert_eq!(list.messaging, Messaging::Setting);
    list.on_update(
        request,
        1,
        Ok(vec![linkset(1, "keep", Use::StaticObstacle, [50, 100, 100, 100], true, true)]),
    );
    prepare(&mut list);
    assert_eq!(list.messaging, Messaging::Complete);
    assert_eq!(
        list.row(&Uuid::from_u128(1)).and_then(|r| r.linkset()).map(|l| l.use_),
        Some(Use::StaticObstacle)
    );
    let request = list.start_set();
    list.on_update(request, 1, Err(Error::Failed));
    assert_eq!(list.messaging, Messaging::SetError);
}

#[test]
fn the_edit_panel_loads_commits_and_applies_like_the_legacy_one() {
    let rows = [
        linkset(1, "a", Use::MovableObstacle, [100, 90, 80, 70], true, true),
        linkset(2, "b", Use::StaticObstacle, [100, 100, 100, 100], false, true),
    ];
    let mut panel = EditPanel::default();
    panel.load(rows[0].linkset());
    assert_eq!(panel.use_, Some(Use::MovableObstacle));
    assert_eq!(panel.fields, ["100", "90", "80", "70"]);
    panel.fields[1] = "250".into();
    panel.commit(1);
    assert_eq!(panel.fields[1], "100");
    panel.fields[2] = String::new();
    panel.commit(2);
    assert_eq!(panel.fields[2], "80", "an empty field keeps its value");
    panel.use_ = Some(Use::Walkable);
    panel.fields[3] = "40".into();
    let apply = panel.apply(&rows);
    assert!(apply.terrain.is_none());
    assert_eq!(apply.objects.len(), 2);
    let (_, first) = &apply.objects[0];
    assert_eq!(first.category, Some(Category::Include));
    assert_eq!(first.walkability, [None, Some(100), None, Some(40)]);
    let (_, second) = &apply.objects[1];
    assert_eq!(second.phantom, None, "not modifiable: the phantom flag stays");
    assert_eq!(second.walkability, [None, None, Some(80), Some(40)]);
    panel.load(None);
    assert_eq!(panel, EditPanel::default());
}

#[test]
fn the_question_lists_each_warning_once() {
    let warnings = aurora_net::pathfinding::linkset::Warnings {
        phantom_toggle: true,
        restricted: true,
        cannot_be_volume: true,
    };
    let text = edit::question(warnings, Use::MaterialVolume);
    assert!(text.starts_with(aurora_i18n::text("pathfinding-warn-phantom")));
    assert!(text.contains(use_label(Use::Walkable)), "the restricted use is named");
    assert!(text.ends_with(aurora_i18n::text("pathfinding-warn-continue")));
    assert_eq!(text.matches("\n\n").count(), 3);
}

#[test]
fn opening_selects_once_the_list_arrives_and_closing_forgets() {
    let mut ui = PathfindingUi::default();
    let target = Uuid::from_u128(2);
    assert!(ui.open(Kind::Objects, vec![target]), "a closed window asks for its list");
    let request = ui.linksets.start_get(1);
    ui.linksets.on_list(
        request,
        1,
        Ok(vec![
            linkset(1, "a", Use::Walkable, [100; 4], true, true),
            linkset(2, "b", Use::Walkable, [100; 4], true, true),
        ]),
    );
    assert_eq!(ui.linksets.selection.ids, [target]);
    assert!(!ui.open(Kind::Objects, vec![Uuid::from_u128(1)]), "already open: no new request");
    assert_eq!(ui.linksets.selection.ids, [Uuid::from_u128(1)]);
    ui.linkset_beacons = true;
    ui.closed(Kind::Objects);
    assert!(ui.linksets.rows().is_empty() && ui.linksets.selection.is_empty() && !ui.linkset_beacons);
}

#[test]
fn the_capsule_needs_one_selected_character() {
    let mut ui = PathfindingUi {
        characters_open: true,
        capsule: true,
        ..Default::default()
    };
    assert_eq!(ui.capsule_character(), None);
    ui.characters.selection.select_all(vec![Uuid::from_u128(5)]);
    assert_eq!(ui.capsule_character(), Some(Uuid::from_u128(5)));
    ui.characters.selection.select_all(vec![Uuid::from_u128(5), Uuid::from_u128(6)]);
    assert_eq!(ui.capsule_character(), None);
}
