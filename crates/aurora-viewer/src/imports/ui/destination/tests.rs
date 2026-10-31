use super::*;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::InvFolder;

fn fixture() -> Inventory {
    let mut inventory = Inventory {
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    for (id, parent, name, kind, library) in [
        (1, 0, "Inventory", 8, false),
        (2, 1, "Textures", 0, false),
        (3, 1, "Booth assets", -1, false),
        (4, 1, "Trash", 14, false),
        (5, 4, "Discarded", -1, false),
        (6, 1, "Protected", -1, false),
        (7, 6, "Child", -1, false),
        (8, 0, "Library", 8, true),
        (9, 3, "Fabric", -1, false),
        (10, 6, "Fabric", -1, false),
    ] {
        let id = Uuid::from_u128(id);
        inventory.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent: Uuid::from_u128(parent),
                    name: name.into(),
                    type_default: kind,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library,
            },
        );
    }
    inventory
}

#[test]
fn cached_destinations_keep_rules_and_refresh_for_inventory_and_protection() {
    let mut inventory = fixture();
    let mut picker = Picker::default();
    let ctx = egui::Context::default();
    for protected in [HashSet::new(), HashSet::from([Uuid::from_u128(6)])] {
        picker.refresh(&ctx, &inventory, &protected);
        let expected: HashSet<_> = inventory
            .folders
            .keys()
            .copied()
            .filter(|id| rules::destination(&inventory, *id, &protected))
            .collect();
        assert_eq!(picker.rows.iter().map(|row| row.id).collect::<HashSet<_>>(), expected);
    }
    picker.query = "  BOOTH  ".into();
    picker.filter();
    assert_eq!(picker.visible.len(), 2, "matching folders keep their inventory ancestors");
    assert_eq!(picker.rows[*picker.visible.last().unwrap()].id, Uuid::from_u128(3));
    inventory.folders.get_mut(&Uuid::from_u128(3)).unwrap().info.name = "Renamed".into();
    inventory.generation += 1;
    picker.refresh(&ctx, &inventory, &HashSet::new());
    picker.filter();
    assert!(picker.visible.is_empty());
}

#[test]
fn typing_in_popup_filters_without_closing_or_changing_destination() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let mut picker = Picker::default();
    let inventory = fixture();
    let mut selected = Uuid::from_u128(2);
    let mut result_position = None;
    for step in 0..4 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                events: if step == 2 {
                    vec![egui::Event::Text("Booth".into())]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
            |ui| {
                if step == 0 {
                    egui::Popup::open_id(ui.ctx(), ui.make_persistent_id(egui::IdSalt::new("import_folder")).with("popup"));
                }
                picker.show(ui, &theme.palette(), &inventory, &HashSet::new(), &mut selected);
            },
        );
        output.textures_delta.clear();
        result_position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == "Booth assets" => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .or(result_position);
        drop(output);
    }
    assert!(picker.opened);
    assert_eq!(picker.query, "Booth");
    assert_eq!(selected, Uuid::from_u128(2));
    assert_eq!(picker.visible.len(), 2, "matching folders keep their inventory ancestors");
    let position = result_position.expect("filtered result visible");
    for pressed in [Some(true), Some(false), None] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                events: pressed.map_or_else(Vec::new, |pressed| {
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                }),
                ..Default::default()
            },
            |ui| {
                picker.show(ui, &theme.palette(), &inventory, &HashSet::new(), &mut selected);
            },
        );
        output.textures_delta.clear();
        drop(output);
    }
    assert_eq!(selected, Uuid::from_u128(3));
    assert!(!picker.opened, "choosing a result closes the list");
}

#[test]
fn destination_tree_preserves_parentage_for_duplicate_names_and_search_context() {
    let ctx = egui::Context::default();
    let inventory = fixture();
    let mut picker = Picker::default();
    picker.refresh(&ctx, &inventory, &HashSet::new());
    picker.filter();
    assert!(
        !picker.visible.iter().any(|&index| picker.rows[index].id == Uuid::from_u128(9)),
        "nested folders start folded"
    );
    picker.query = "Fabric".into();
    picker.filter();
    let rows: Vec<_> = picker
        .visible
        .iter()
        .map(|&index| (picker.rows[index].id.as_u128(), picker.rows[index].depth))
        .collect();
    assert_eq!(rows, vec![(1, 0), (3, 1), (9, 2), (6, 1), (10, 2)]);
    let paths: Vec<_> = picker
        .visible
        .iter()
        .filter_map(|&index| (picker.rows[index].name == "Fabric").then_some(&picker.rows[index].path))
        .collect();
    assert_ne!(paths[0], paths[1], "equal names retain their actual inventory ancestry");
    picker.refresh(&ctx, &inventory, &HashSet::from([Uuid::from_u128(6)]));
    picker.filter();
    assert_eq!(
        picker
            .visible
            .iter()
            .map(|&index| picker.rows[index].id.as_u128())
            .collect::<Vec<_>>(),
        vec![1, 3, 9]
    );
}

#[test]
fn destination_tree_carets_expand_without_selecting_then_choose_the_nested_folder() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let inventory = fixture();
    let mut picker = Picker::default();
    let mut selected = Uuid::from_u128(2);
    let frame = |picker: &mut Picker, selected: &mut Uuid, events, open| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(650.0, 520.0))),
                events,
                ..Default::default()
            },
            |ui| {
                if open {
                    egui::Popup::open_id(ui.ctx(), ui.make_persistent_id(egui::IdSalt::new("import_folder")).with("popup"));
                }
                picker.show(ui, &theme.palette(), &inventory, &HashSet::new(), selected);
            },
        );
        output.textures_delta.clear();
    };
    let click = |picker: &mut Picker, selected: &mut Uuid, pos| {
        for pressed in [true, false] {
            frame(
                picker,
                selected,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                false,
            );
        }
        frame(picker, selected, Vec::new(), false);
    };
    for index in 0..4 {
        frame(&mut picker, &mut selected, Vec::new(), index == 0);
    }
    let caret = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_destination_caret", Uuid::from_u128(3)))))
        .unwrap();
    click(&mut picker, &mut selected, caret.center());
    assert!(picker.opened, "expanding retains the popup");
    assert_eq!(selected, Uuid::from_u128(2), "a caret changes disclosure only");
    let row = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_destination_row", Uuid::from_u128(9)))))
        .unwrap();
    click(&mut picker, &mut selected, row.center());
    assert_eq!(selected, Uuid::from_u128(9));
    assert!(!picker.opened);
}
