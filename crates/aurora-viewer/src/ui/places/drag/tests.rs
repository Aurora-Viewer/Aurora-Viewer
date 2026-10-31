use super::*;
use crate::{
    scene::avatar::AvatarLibrary,
    settings::FavoritesAccess,
    ui::{
        bars::favorites,
        place_details::Ctx,
        places::{PlacesAction, PlacesUi, aurora},
    },
    world::World,
};
use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::Arc,
};

fn text_position(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + egui::vec2(4.0, 4.0)),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, text)),
        _ => None,
    }
}

#[test]
fn place_portal_drags_start_from_every_favorites_and_places_view_and_leave_reordering_to_the_list() {
    let library = Arc::new(AvatarLibrary::load());
    // Menu, row, grid, both Places looks and history with/without a landmark.
    for mode in 0..16 {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = crate::ui::icons::Icons::load(&ctx, None);
        let mut world = World::new(library.clone());
        world.inventory.root = Uuid::from_u128(1);
        let favorite = Uuid::from_u128(4);
        let landmark = Uuid::from_u128(5);
        for (id, kind, item, name) in [(2, 23, favorite, "Portal favorite"), (3, 3, landmark, "Portal landmark")] {
            let folder = InvFolder {
                id: Uuid::from_u128(id),
                parent: world.inventory.root,
                type_default: kind,
                ..Default::default()
            };
            world.inventory.apply(vec![FolderContents {
                folder_id: folder.parent,
                owner_id: Uuid::nil(),
                version: 1,
                folders: vec![folder.clone()],
                items: vec![],
            }]);
            world.inventory.apply(vec![FolderContents {
                folder_id: folder.id,
                owner_id: Uuid::nil(),
                version: 1,
                folders: vec![],
                items: vec![InvItem {
                    id: item,
                    parent: folder.id,
                    asset_id: item,
                    name: name.into(),
                    asset_type: 3,
                    inv_type: 3,
                    ..Default::default()
                }],
            }]);
        }
        let access = [FavoritesAccess::Menu, FavoritesAccess::Row, FavoritesAccess::Dock][mode % 3];
        let view = favorites::View {
            list: vec![favorites::Favorite {
                item: favorite,
                asset: favorite,
                name: "Portal favorite".into(),
                region: String::new(),
                rating: "",
                global: None,
                slurl: None,
                thumbnail: None,
            }],
            access,
            ..Default::default()
        };
        let free_history = glam::DVec3::new(20_000.0, 30_000.0, 35.0);
        let backed_history = glam::DVec3::new(701.0, 901.0, 32.0);
        world
            .landmarks
            .on_data(landmark, Some(b"Landmark version 1\nposition 700 900 30\n"));
        world.tp_storage.items = vec![
            crate::world::tphistory::HistoryItem {
                title: "Portal history".into(),
                global: free_history,
                date: 1.0,
                slurl: String::new(),
            },
            crate::world::tphistory::HistoryItem {
                title: "Portal landmark".into(),
                global: backed_history,
                date: 2.0,
                slurl: String::new(),
            },
        ];
        let tab = if mode >= 11 {
            2
        } else if mode % 2 == 0 {
            0
        } else {
            1
        };
        let mut places = PlacesUi::default();
        places.open_tab(tab);
        if matches!(mode, 9 | 10 | 14) {
            places.filter = "Portal".into();
        }
        crate::ui::widgets::place_floater(
            &ctx,
            "places",
            egui::pos2(20.0, 80.0),
            egui::vec2(if matches!(mode, 7..=10 | 13..=15) { 960.0 } else { 400.0 }, 580.0),
        );
        let mut emoji = crate::ui::emoji::Emoji::load();
        let images = HashMap::new();
        let mut names = HashSet::new();
        let mut wanted_images = HashSet::new();
        let mut groups = HashSet::new();
        let world_drop = RefCell::new(None);
        let history_drop = RefCell::new(None);
        let reorder_expected = Cell::new(false);
        let reordered = Cell::new(false);
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |root| {
                    if mode < 3 {
                        let mut action = None;
                        if mode == 1 {
                            let (_, result) = favorites::row(
                                &ctx,
                                &p,
                                &icons,
                                &view,
                                egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(800.0, 30.0)),
                            );
                            if reorder_expected.get() {
                                reordered.set(matches!(
                                    result,
                                    Some(crate::ui::bars::BarAction::Favorite(favorites::FavAction::Move(..)))
                                ));
                            } else {
                                assert!(result.is_none(), "a scene drop must not reorder or teleport, mode {mode}");
                            }
                        } else {
                            egui::Area::new(egui::Id::unique("favorites_test"))
                                .fixed_pos(egui::pos2(20.0, 60.0))
                                .show(&ctx, |ui| {
                                    if mode == 0 {
                                        favorites::menu_body(ui, &p, &icons, &view, &mut action);
                                    } else {
                                        favorites::grid_body(ui, &p, &icons, &view, &mut action);
                                    }
                                });
                            if reorder_expected.get() {
                                reordered.set(matches!(action, Some(favorites::FavAction::Move(..))));
                            } else {
                                assert!(action.is_none(), "a scene drop must not reorder or teleport, mode {mode}");
                            }
                        }
                    } else {
                        let mut c = Ctx {
                            p: &p,
                            world: &world,
                            images: &images,
                            emoji: &mut emoji,
                            want_names: &mut names,
                            wanted_images: &mut wanted_images,
                            want_groups: &mut groups,
                        };
                        let env = aurora::Env {
                            sort: Default::default(),
                            favorites: &[favorite],
                        };
                        let actions = places.show(&ctx, &mut c, &mut true, false, false, (mode >= 5 && mode != 11).then_some(&env));
                        if reorder_expected.get() {
                            reordered.set(actions.iter().any(|a| matches!(a, PlacesAction::MoveFavorite(..))));
                        } else {
                            assert!(
                                !actions.iter().any(|a| matches!(
                                    a,
                                    PlacesAction::MoveFavorite(..) | PlacesAction::Teleport(..) | PlacesAction::TeleportLandmark(..)
                                )),
                                "scene release, mode {mode}"
                            );
                        }
                    }
                    world_drop.replace(crate::ui::inventory::world_drag(root.ctx()));
                    history_drop.replace(history_world_drag(root.ctx()));
                    ghost(root.ctx(), &p);
                },
            );
            output.textures_delta.clear();
            output
        };
        for _ in 0..4 {
            frame(vec![]);
        }
        let title = if mode >= 11 && mode != 15 {
            "Portal history"
        } else if mode < 3 || tab == 0 {
            "Portal favorite"
        } else {
            "Portal landmark"
        };
        let output = frame(vec![]);
        let start = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, title))
            .unwrap_or_else(|| panic!("source in mode {mode}"));
        let button = |pos, pressed, modifiers| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(vec![egui::Event::PointerMoved(start), button(start, true, egui::Modifiers::NONE)]);
        frame(vec![egui::Event::PointerMoved(start + egui::vec2(16.0, 0.0))]);
        let cargo = egui::DragAndDrop::payload::<PlaceDrag>(&ctx).unwrap_or_else(|| panic!("drag started in mode {mode}"));
        assert_eq!(cargo.name, title);
        let expected_history = if mode == 15 { backed_history } else { free_history };
        if mode >= 11 {
            assert_eq!(cargo.target, Target::History(expected_history));
            assert_eq!(cargo.favorite, None);
        }
        let destination = egui::pos2(1200.0, 820.0);
        frame(vec![egui::Event::PointerMoved(destination)]);
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        frame(vec![egui::Event::ModifiersChanged(shift), button(destination, false, shift)]);
        if mode >= 11 {
            let drop = history_drop
                .borrow_mut()
                .take()
                .expect("history reaches the world without an inventory item");
            assert_eq!(drop.global, expected_history);
            assert!(drop.released && drop.shift);
            assert!(world_drop.borrow().is_none(), "history cannot be inserted or given as inventory");
            assert_eq!(world.inventory.items.len(), 2);
            continue;
        }
        let drop = world_drop
            .borrow_mut()
            .take()
            .unwrap_or_else(|| panic!("world drop in mode {mode}"));
        assert_eq!(drop.item, if title == "Portal favorite" { favorite } else { landmark });
        assert!(drop.released && drop.shift && !drop.ctrl);
        assert_eq!(world.inventory.items.len(), 2, "dragging never consumes the landmark");
        if mode < 3 || mode == 6 || mode == 8 {
            frame(vec![egui::Event::PointerMoved(start), button(start, true, egui::Modifiers::NONE)]);
            let over_list = start + egui::vec2(16.0, 0.0);
            frame(vec![egui::Event::PointerMoved(over_list)]);
            reorder_expected.set(true);
            frame(vec![button(over_list, false, egui::Modifiers::NONE)]);
            assert!(reordered.get(), "an in-list release must still reorder, mode {mode}");
            assert!(egui::DragAndDrop::payload::<PlaceDrag>(&ctx).is_none());
            assert!(world_drop.borrow().is_none());
        }
    }
}
