use super::*;
use crate::world::inventory::Folder;
use aurora_net::inventory::InvFolder;

fn folder(inv: &mut Inventory, id: u128, parent: Uuid, kind: i32, name: &str) -> Uuid {
    let id = Uuid::from_u128(id);
    inv.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
                type_default: kind,
                name: name.into(),
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    if let Some(parent) = inv.folders.get_mut(&parent) {
        parent.children.push(id);
    }
    id
}
fn inventory() -> (Inventory, Uuid, Uuid) {
    let mut inv = Inventory::default();
    let root = folder(&mut inv, 1, Uuid::nil(), 8, "Inventory");
    inv.root = root;
    folder(&mut inv, 2, root, 14, "Trash");
    let aurora = folder(&mut inv, 3, root, -1, catalogue::AURORA);
    let library = folder(&mut inv, 4, aurora, -1, catalogue::LIBRARY);
    let source = folder(&mut inv, 5, root, -1, "Stone");
    for (n, name) in catalogue::DEFAULT_FOLDERS.into_iter().enumerate() {
        folder(&mut inv, 1000 + n as u128, library, -1, name);
    }
    (inv, library, source)
}
#[test]
fn library_creates_unified_root_and_default_folders_once() {
    let (mut inv, library, _) = inventory();
    inv.remove(&[library]);
    let mut ui = LibraryUi::default();
    let mut actions = Vec::new();
    ui.prepare_root(&mut inv, &mut actions);
    let [Action::Change(change)] = actions.as_slice() else {
        panic!("creation")
    };
    assert_eq!(change.operations.len(), 9);
    let Operation::Folder {
        id: root, parent, name, ..
    } = &change.operations[0]
    else {
        panic!("root")
    };
    assert_eq!(*parent, Uuid::from_u128(3));
    assert_eq!(name, catalogue::LIBRARY);
    let folders: HashSet<_> = change.operations[1..]
        .iter()
        .map(|op| {
            let Operation::Folder { parent, name, .. } = op else {
                panic!("default folder")
            };
            assert_eq!(parent, root);
            name.as_str()
        })
        .collect();
    assert_eq!(folders, catalogue::DEFAULT_FOLDERS.into_iter().collect::<HashSet<_>>());
    ui.prepare_root(&mut inv, &mut actions);
    assert_eq!(actions.len(), 1, "do not repeat while awaiting server confirmation");
}
#[test]
fn library_waits_for_listing_and_reuses_defaults_and_legacy_item_ids() {
    let (mut inv, library, _) = inventory();
    let mut actions = Vec::new();
    LibraryUi::default().prepare_root(&mut inv, &mut actions);
    assert!(actions.is_empty());
    inv.folders.get_mut(&library).unwrap().state = FetchState::Unknown;
    let mut ui = LibraryUi::default();
    ui.prepare_root(&mut inv, &mut actions);
    assert!(actions.is_empty());
    assert!(inv.queue.iter().any(|(id, _)| *id == library));
    inv.folders.get_mut(&library).unwrap().state = FetchState::Fetched;
    let textures = catalogue::child(&inv, library, "Textures").unwrap();
    inv.remove(&[textures]);
    let legacy = folder(&mut inv, 66, Uuid::from_u128(3), -1, catalogue::LEGACY_TEXTURES);
    ui.prepare_root(&mut inv, &mut actions);
    let [Action::Change(change)] = actions.as_slice() else {
        panic!("adopt legacy folder")
    };
    assert!(matches!(change.operations.as_slice(), [Operation::Patch {id, folder: true, body}]
        if *id == legacy && body["parent_id"].as_uuid() == library && body["name"].as_str() == "Textures"));
}
#[test]
fn texture_library_presentation_changes_preserve_search_and_selection() {
    let ui = LibraryUi {
        search: "marble".into(),
        folder: Some(Uuid::from_u128(5)),
        kind: Kind::Material,
        selected: Some(Uuid::from_u128(6)),
        ..Default::default()
    };
    let mut preferences = Preferences::default();
    preferences.presentation.flip();
    assert_eq!(preferences.presentation, Presentation::Horizontal);
    assert_eq!(ui.search, "marble");
    assert_eq!(ui.folder, Some(Uuid::from_u128(5)));
    assert!(ui.kind == Kind::Material);
    assert_eq!(ui.selected, Some(Uuid::from_u128(6)));
    let encoded = serde_json::to_string(&preferences).expect("preferences");
    let restored: Preferences = serde_json::from_str(&encoded).expect("restore");
    assert_eq!(restored.presentation, Presentation::Horizontal);
}
#[test]
fn texture_library_recent_resources_are_bounded_and_deduplicated() {
    let mut preferences = Preferences::default();
    for id in 1..=50 {
        preferences.record(Uuid::from_u128(id));
    }
    preferences.record(Uuid::from_u128(49));
    assert_eq!(preferences.recent.len(), 32);
    assert_eq!(preferences.recent[0], Uuid::from_u128(49));
    assert_eq!(preferences.recent.iter().filter(|id| **id == Uuid::from_u128(49)).count(), 1);
}
#[test]
fn texture_library_object_filters_share_folder_search_favorites_and_recent() {
    let (mut inv, library, outside) = inventory();
    for (n, kind, flags) in [(80, 0, 0), (81, 57, 0), (82, 6, 0), (83, 6, 0x200000)] {
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id: Uuid::from_u128(n),
            parent: library,
            asset_id: Uuid::from_u128(n + 100),
            asset_type: kind,
            flags,
            name: format!("Resource {n}"),
            ..Default::default()
        }]);
    }
    let mut ui = LibraryUi::default();
    ui.catalogue.prepare(&mut inv);
    let mut prefs = Preferences::default();
    let ids = |ui: &LibraryUi, prefs: &Preferences| ui.filtered(prefs).iter().map(|&i| ui.catalogue.entries[i].id).collect::<Vec<_>>();
    assert_eq!(ids(&ui, &prefs).len(), 4);
    for (kind, expected) in [(Kind::Texture, vec![80]), (Kind::Material, vec![81]), (Kind::Object, vec![82, 83])] {
        ui.kind = kind;
        assert_eq!(ids(&ui, &prefs), expected.into_iter().map(Uuid::from_u128).collect::<Vec<_>>());
    }
    ui.folder = Some(library);
    ui.search = "83".into();
    assert_eq!(ids(&ui, &prefs), vec![Uuid::from_u128(83)]);
    ui.search.clear();
    prefs.favorites.insert(Uuid::from_u128(82));
    ui.scope = Scope::Favorites;
    assert_eq!(ids(&ui, &prefs), vec![Uuid::from_u128(82)]);
    prefs.record(Uuid::from_u128(82));
    prefs.record(Uuid::from_u128(83));
    ui.scope = Scope::Recent;
    assert_eq!(ids(&ui, &prefs), vec![Uuid::from_u128(83), Uuid::from_u128(82)]);
    ui.folder = Some(outside);
    assert!(ids(&ui, &prefs).is_empty());
}
#[test]
fn texture_library_shortcut_is_ctrl_space_and_rebindable() {
    use crate::keybinds::{Action, Input, KeyBindings, Mods};
    let mut bindings = KeyBindings::default();
    let space = Input::key(winit::keyboard::KeyCode::Space);
    let modifiers = Mods {
        ctrl: true,
        ..Default::default()
    };
    assert_eq!(bindings.triggered(&space, modifiers), Some(Action::TextureLibrary));
    assert_eq!(bindings.triggered(&space, Mods::default()), None);
    bindings.set(Action::TextureLibrary, 0, None);
    assert_eq!(bindings.triggered(&space, modifiers), None);
}

#[test]
fn texture_library_header_switch_and_toolbar_restore_keep_the_active_state() {
    use std::sync::Arc;
    let (mut inv, library, _) = inventory();
    folder(&mut inv, 71, library, -1, "Stone");
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut world = World::new(avatars.clone());
    world.agent_id = Uuid::from_u128(70);
    world.inventory = inv;
    let mut scene = Scene::new(std::env::temp_dir(), avatars);
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut ui = LibraryUi {
        agent: world.agent_id,
        search: "Stone".into(),
        folder: Some(Uuid::from_u128(71)),
        kind: Kind::Material,
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut open = true;
    let mut frame = 0;
    let mut show = |events: Vec<egui::Event>, ui: &mut LibraryUi, prefs: &mut Preferences, open: &mut bool| {
        frame += 1;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                time: Some(frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |host| {
                ui.show(host.ctx(), &p, &mut world, &mut scene, &HashMap::new(), prefs, open);
            },
        );
        output.textures_delta.clear();
    };
    for _ in 0..8 {
        show(vec![], &mut ui, &mut prefs, &mut open);
    }
    let rect = ctx.memory(|m| m.area_rect(egui::Id::unique(VERTICAL))).expect("vertical window");
    let switch = egui::pos2(rect.right() - 63.0, rect.top() + 18.0);
    for pressed in [true, false] {
        show(
            vec![
                egui::Event::PointerMoved(switch),
                egui::Event::PointerButton {
                    pos: switch,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            &mut ui,
            &mut prefs,
            &mut open,
        );
    }
    assert_eq!(
        prefs.presentation,
        Presentation::Horizontal,
        "the native title action must switch the view"
    );
    for _ in 0..8 {
        show(vec![], &mut ui, &mut prefs, &mut open);
    }
    assert_eq!(ui.search, "Stone");
    assert_eq!(ui.kind, Kind::Material);
    assert_eq!(ui.folder, Some(Uuid::from_u128(71)));
    let rect = ctx
        .memory(|m| m.area_rect(egui::Id::unique(HORIZONTAL)))
        .expect("horizontal window");
    let minimize = egui::pos2(rect.right() - 41.0, rect.top() + 18.0);
    for pressed in [true, false] {
        show(
            vec![
                egui::Event::PointerMoved(minimize),
                egui::Event::PointerButton {
                    pos: minimize,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            &mut ui,
            &mut prefs,
            &mut open,
        );
    }
    assert!(widgets::floater_minimized(&ctx, HORIZONTAL));
    let mut panels = super::super::Panels::from_settings(&crate::settings::Settings::default());
    panels.texture_library = open;
    panels.texture_library_presentation = prefs.presentation;
    super::super::bars::run_command(&ctx, "texture_library", &mut panels);
    assert!(panels.texture_library);
    open = panels.texture_library;
    show(vec![], &mut ui, &mut prefs, &mut open);
    assert!(!widgets::floater_minimized(&ctx, HORIZONTAL));
    assert_eq!(ui.search, "Stone");
}

#[test]
fn texture_library_failed_fetch_refreshes_the_catalogue_and_waits_for_explicit_retry() {
    let (mut inv, library, _) = inventory();
    inv.folders.get_mut(&library).unwrap().state = FetchState::Unknown;
    let mut catalogue = Catalogue::default();
    catalogue.prepare(&mut inv);
    assert!(catalogue.loading);
    assert_eq!(inv.queue.len(), 1);
    inv.failed(&[library]);
    catalogue.prepare(&mut inv);
    assert!(!catalogue.loading);
    assert_eq!(catalogue.failed, vec![library]);
    catalogue.prepare(&mut inv);
    assert_eq!(inv.queue.len(), 1, "failed folders must not flood retries");
    inv.request(library);
    assert_eq!(inv.queue.len(), 2, "the retry can enqueue a failed folder");
}

#[test]
fn texture_library_cards_apply_on_click_and_keep_dragging_separate_in_both_layouts() {
    use aurora_net::inventory::InvItem;
    use std::sync::Arc;
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    for presentation in [Presentation::Vertical, Presentation::Horizontal] {
        for kind in [0, 57, 6, 1, 7, 3, 20, 10] {
            let (mut inv, library, _) = inventory();
            let item = Uuid::from_u128(80);
            inv.add_items(vec![InvItem {
                id: item,
                parent: library,
                asset_id: Uuid::from_u128(81),
                asset_type: kind,
                name: "Resource".into(),
                ..Default::default()
            }]);
            let mut world = World::new(avatars.clone());
            world.agent_id = Uuid::from_u128(70);
            world.inventory = inv;
            let mut scene = Scene::new(std::env::temp_dir(), avatars.clone());
            let ctx = egui::Context::default();
            let mut ui = LibraryUi {
                agent: world.agent_id,
                ..Default::default()
            };
            let mut prefs = Preferences {
                presentation,
                ..Default::default()
            };
            let mut time = 0.0;
            let mut frame = |events: Vec<egui::Event>, ui: &mut LibraryUi| {
                time += 1.0 / 60.0;
                let mut actions = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    // every pass's actions, as the app handles them (a
                    // pass laid out again has no input of its own)
                    |host| {
                        actions.extend(ui.show(
                            host.ctx(),
                            &crate::theme::Theme::default().palette(),
                            &mut world,
                            &mut scene,
                            &HashMap::new(),
                            &mut prefs,
                            &mut true,
                        ));
                    },
                );
                output.textures_delta.clear();
                actions
            };
            for _ in 0..8 {
                frame(Vec::new(), &mut ui);
            }
            let source = ctx
                .data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique("texture_library_first_card")))
                .expect("resource card");
            let button = |pos, pressed| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: Default::default(),
                    },
                ]
            };
            assert!(frame(button(source, true), &mut ui).is_empty());
            assert!(frame(vec![egui::Event::PointerMoved(source + egui::vec2(-20.0, 0.0))], &mut ui).is_empty());
            assert_eq!(egui::DragAndDrop::payload::<InvDrag>(&ctx).expect("inventory cargo").0, item);
            assert!(
                frame(button(egui::pos2(50.0, 500.0), false), &mut ui).is_empty(),
                "a drag must never apply to edit selection"
            );
            egui::DragAndDrop::clear_payload(&ctx);
            for _ in 0..30 {
                frame(Vec::new(), &mut ui);
            }
            assert!(frame(button(source, true), &mut ui).is_empty());
            let actions = frame(button(source, false), &mut ui);
            if !matches!(kind, 0 | 57) {
                assert!(actions.is_empty(), "non-surface resources only select on single click");
            } else {
                assert!(matches!(actions.as_slice(), [Action::Apply(id)] if *id == item));
            }
            assert_eq!(ui.selected, Some(item));
            frame(button(source, true), &mut ui);
            let opened = frame(button(source, false), &mut ui);
            if matches!(kind, 0 | 57 | 6) {
                assert!(opened.is_empty());
            } else {
                assert!(matches!(opened.as_slice(), [Action::Open(id)] if *id == item));
            }
            assert_eq!(ui.windows.iter().any(|window| window.id == item), matches!(kind, 0 | 57));
        }
    }
}

#[test]
fn texture_library_material_preview_keeps_orbit_and_drag_handle_independent() {
    use aurora_net::inventory::InvItem;
    use std::sync::Arc;
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    for size in [Vec2::new(1280.0, 800.0), Vec2::new(800.0, 560.0)] {
        let (mut inv, library, _) = inventory();
        let item = Uuid::from_u128(82);
        let asset = Uuid::from_u128(83);
        inv.add_items(vec![InvItem {
            id: item,
            parent: library,
            asset_id: asset,
            asset_type: 57,
            name: "Material".into(),
            ..Default::default()
        }]);
        let mut world = World::new(avatars.clone());
        world.agent_id = Uuid::from_u128(70);
        world.inventory = inv;
        let mut scene = Scene::new(std::env::temp_dir(), avatars.clone());
        scene.materials.insert(asset, aurora_assets::PbrMaterial::default());
        let ctx = egui::Context::default();
        let mut library = LibraryUi {
            agent: world.agent_id,
            ..Default::default()
        };
        library.open_preview(&ctx, &world, item);
        let mut preferences = Preferences::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>, library: &mut LibraryUi| {
            time += 1.0 / 60.0;
            let mut actions = Vec::new();
            let mut dropped = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |host| {
                    actions = library.show(
                        host.ctx(),
                        &crate::theme::Theme::default().palette(),
                        &mut world,
                        &mut scene,
                        &HashMap::new(),
                        &mut preferences,
                        &mut true,
                    );
                    if let Some(drag) = super::super::inventory::world_drag(host.ctx())
                        && drag.released
                    {
                        dropped = Some(drag.item);
                        egui::DragAndDrop::clear_payload(host.ctx());
                    }
                },
            );
            output.textures_delta.clear();
            assert!(actions.is_empty(), "preview gestures must not apply to the edit selection");
            dropped
        };
        // the preview job runs on this thread: its model is there after a
        // few frames, whatever the load of the machine
        let ready = (0..10).any(|_| {
            frame(Vec::new(), &mut library);
            ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_scene", item))))
                .is_some()
        });
        assert!(ready, "material preview not shown");
        for _ in 0..8 {
            frame(Vec::new(), &mut library);
        }
        let rect = |key| {
            ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique((key, item))))
                .expect("preview control")
        };
        let image = rect("resource_material_scene");
        let handle = rect("resource_material_handle");
        let window = ctx
            .memory(|m| m.area_rect(egui::Id::unique(preview_window_id(item))))
            .expect("preview floater");
        assert!(window.contains_rect(handle), "the handle must fit the compact preview");
        assert!(!image.intersects(handle), "the orbit surface and handle must not overlap");
        let button = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                },
            ]
        };
        let before = library.windows[0].view;
        let orbit = image.center() + egui::vec2(40.0, 15.0);
        frame(button(image.center(), true), &mut library);
        frame(vec![egui::Event::PointerMoved(orbit)], &mut library);
        assert_ne!(library.windows[0].view.yaw, before.yaw);
        assert_ne!(library.windows[0].view.pitch, before.pitch);
        assert!(
            egui::DragAndDrop::payload::<InvDrag>(&ctx).is_none(),
            "orbiting must never create inventory cargo"
        );
        frame(button(orbit, false), &mut library);
        let camera = library.windows[0].view;
        frame(button(handle.center(), true), &mut library);
        frame(button(handle.center(), false), &mut library);
        assert!(
            egui::DragAndDrop::payload::<InvDrag>(&ctx).is_none(),
            "a plain click on the handle has no side effect"
        );
        frame(button(handle.center(), true), &mut library);
        frame(
            vec![egui::Event::PointerMoved(handle.center() + egui::vec2(18.0, 0.0))],
            &mut library,
        );
        assert_eq!(egui::DragAndDrop::payload::<InvDrag>(&ctx).expect("material cargo").0, item);
        assert!(
            library.windows[0].view == camera,
            "dragging the handle must not change the preview camera"
        );
        let target = egui::pos2(50.0, size.y - 35.0);
        frame(vec![egui::Event::PointerMoved(target)], &mut library);
        assert_eq!(frame(button(target, false), &mut library), Some(item));
        assert!(library.windows[0].view == camera);
    }
}

#[test]
fn resource_previews_coexist_keep_their_cameras_and_close_independently() {
    use aurora_net::inventory::InvItem;
    use std::sync::Arc;
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let (mut inv, library_folder, outside) = inventory();
    let items = [91, 92, 93, 94].map(Uuid::from_u128);
    let texture = Uuid::from_u128(95);
    let material = Uuid::from_u128(96);
    for (index, id) in items.into_iter().enumerate() {
        inv.add_items(vec![InvItem {
            id,
            parent: if index % 2 == 0 { library_folder } else { outside },
            asset_id: if index < 2 { texture } else { material },
            asset_type: if index < 2 { 0 } else { 57 },
            name: format!("Preview {index}"),
            ..Default::default()
        }]);
    }
    let link = Uuid::from_u128(97);
    inv.add_items(vec![InvItem {
        id: link,
        asset_id: items[2],
        asset_type: 24,
        ..Default::default()
    }]);
    let mut world = World::new(avatars.clone());
    world.agent_id = Uuid::from_u128(70);
    world.inventory = inv;
    let mut scene = Scene::new(std::env::temp_dir(), avatars);
    scene.materials.insert(material, aurora_assets::PbrMaterial::default());
    let ctx = egui::Context::default();
    let image = ctx.load_texture(
        "concurrent preview",
        egui::ColorImage::filled([128, 64], egui::Color32::GRAY),
        Default::default(),
    );
    let images = HashMap::from([(texture, image)]);
    let mut resources = LibraryUi::default();
    for (index, item) in items.into_iter().enumerate() {
        widgets::place_floater(
            &ctx,
            &preview_window_id(item),
            egui::pos2(20.0 + (index % 2) as f32 * 340.0, 60.0 + (index / 2) as f32 * 360.0),
            Vec2::new(280.0, 300.0),
        );
        resources.open_preview(&ctx, &world, item);
    }
    let mut preferences = Preferences::default();
    let mut time = 0.0;
    let mut frame = |events: Vec<egui::Event>, resources: &mut LibraryUi, world: &mut World| {
        time += 1.0 / 60.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 1000.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |host| {
                // All resource windows survive while the library is closed.
                assert!(
                    resources
                        .show(
                            host.ctx(),
                            &crate::theme::Theme::default().palette(),
                            world,
                            &mut scene,
                            &images,
                            &mut preferences,
                            &mut false
                        )
                        .is_empty()
                );
            },
        );
        output.textures_delta.clear();
    };
    // the preview jobs run on this thread (see the test above)
    let ready = (0..10).any(|_| {
        frame(Vec::new(), &mut resources, &mut world);
        items[2..].iter().all(|item| {
            ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_scene", *item))))
                .is_some()
        })
    });
    assert!(ready, "material previews not shown");
    for _ in 0..8 {
        frame(Vec::new(), &mut resources, &mut world);
    }
    assert_eq!(resources.windows.len(), 4);
    for item in items {
        let window = ctx
            .memory(|m| m.area_rect(egui::Id::unique(preview_window_id(item))))
            .expect("independent preview");
        let key = if item == items[0] || item == items[1] {
            "resource_texture_preview"
        } else {
            "resource_material_scene"
        };
        let content = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique((key, item))))
            .expect("visible preview content");
        assert!(window.contains_rect(content), "{window:?} {content:?}");
    }
    let button = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            },
        ]
    };
    let canvas = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_scene", items[2]))))
        .expect("material scene");
    let initial = resources.windows[2].view;
    let other = resources.windows[3].view;
    frame(button(canvas.center(), true), &mut resources, &mut world);
    let orbit = canvas.center() + egui::vec2(40.0, 15.0);
    frame(vec![egui::Event::PointerMoved(orbit)], &mut resources, &mut world);
    frame(button(orbit, false), &mut resources, &mut world);
    let camera = resources.windows[2].view;
    assert!(camera != initial);
    assert!(
        resources.windows[3].view == other,
        "a second instance of the same asset keeps its camera"
    );
    resources.open_preview(&ctx, &world, link);
    frame(Vec::new(), &mut resources, &mut world);
    assert_eq!(resources.windows.len(), 4, "a link focuses the same item window");
    assert!(resources.windows[2].view == camera, "reopening preserves the camera");
    let id = preview_window_id(items[2]);
    let window = ctx.memory(|m| m.area_rect(egui::Id::unique(&id))).expect("preview");
    let minimize = egui::pos2(window.right() - 40.0, window.top() + 19.0);
    frame(button(minimize, true), &mut resources, &mut world);
    frame(button(minimize, false), &mut resources, &mut world);
    frame(Vec::new(), &mut resources, &mut world);
    assert!(widgets::floater_minimized(&ctx, &id));
    resources.open_preview(&ctx, &world, items[2]);
    for _ in 0..8 {
        frame(Vec::new(), &mut resources, &mut world);
    }
    assert!(!widgets::floater_minimized(&ctx, &id));
    assert!(resources.windows[2].view == camera);
    let last = ctx
        .memory(|m| m.area_rect(egui::Id::unique(preview_window_id(items[3]))))
        .expect("last preview");
    let close = egui::pos2(last.right() - 18.0, last.top() + 19.0);
    frame(button(close, true), &mut resources, &mut world);
    frame(button(close, false), &mut resources, &mut world);
    frame(Vec::new(), &mut resources, &mut world);
    assert_eq!(resources.windows.iter().map(|w| w.id).collect::<Vec<_>>(), items[..3]);
    assert!(resources.windows[2].view == camera);
    // Moving an item out of the library does not replace or close its preview.
    world.inventory.items.get_mut(&items[2]).expect("material").parent = outside;
    frame(Vec::new(), &mut resources, &mut world);
    assert_eq!(resources.windows.len(), 3);
}

#[test]
fn library_recent_history_is_per_avatar_and_survives_settings_round_trip() {
    let mut prefs = Preferences::default();
    let a = Uuid::from_u128(11);
    let b = Uuid::from_u128(12);
    prefs.activate(a);
    prefs.record(Uuid::from_u128(30));
    prefs.activate(b);
    assert!(prefs.recent.is_empty());
    prefs.record(Uuid::from_u128(31));
    let json = serde_json::to_string(&prefs).unwrap();
    let mut prefs: Preferences = serde_json::from_str(&json).unwrap();
    prefs.activate(a);
    assert_eq!(prefs.recent, [Uuid::from_u128(30)]);
    prefs.activate(b);
    assert_eq!(prefs.recent, [Uuid::from_u128(31)]);
}
#[test]
fn library_multiple_types_combine_with_folder_search_and_recent() {
    let (mut inv, library, _) = inventory();
    let objects = catalogue::child(&inv, library, "Objects").unwrap();
    for (n, kind) in [0, 57, 6, 1, 7, 3, 20, 10].into_iter().enumerate() {
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id: Uuid::from_u128(50 + n as u128),
            parent: objects,
            asset_type: kind,
            asset_id: Uuid::from_u128(150 + n as u128),
            name: format!("Resource {n}"),
            ..Default::default()
        }]);
    }
    let mut ui = LibraryUi::default();
    ui.catalogue.prepare(&mut inv);
    ui.types = Some(HashSet::from([ResourceKind::Texture, ResourceKind::Script]));
    ui.folder = Some(objects);
    let mut prefs = Preferences::default();
    assert_eq!(ui.filtered(&prefs).len(), 2);
    ui.search = "7".into();
    assert_eq!(ui.filtered(&prefs).len(), 1);
    ui.search.clear();
    prefs.record(Uuid::from_u128(57));
    ui.scope = Scope::Recent;
    assert_eq!(ui.filtered(&prefs).len(), 1);
    ui.types = Some(HashSet::new());
    assert!(ui.filtered(&prefs).is_empty());
}

#[test]
fn library_multiple_type_selector_toggles_closed() {
    let ctx = egui::Context::default();
    let mut ui = LibraryUi::default();
    let mut frame = 0;
    let mut draw = |events| {
        frame += 1;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |host| {
                host.horizontal(|host| ui.filters(host, &crate::theme::Theme::default().palette(), &Preferences::default()));
            },
        );
        output.textures_delta.clear();
    };
    for _ in 0..8 {
        draw(Vec::new());
    }
    let point = ctx
        .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("library_type_selector")))
        .unwrap();
    let open = || {
        ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("library_type_menu_open")))
            .unwrap_or(false)
    };
    for expected in [true, false] {
        for pressed in [true, false] {
            draw(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
        draw(Vec::new());
        assert_eq!(open(), expected, "selector must toggle its own popup");
        for _ in 0..24 {
            draw(Vec::new());
        }
    }
}

/// The vertical library keeps its size with a full grid, a folder that
/// failed to load and a message under it.
#[test]
fn the_vertical_library_keeps_its_size_with_its_warnings() {
    use aurora_net::inventory::InvItem;
    use std::sync::Arc;
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    widgets::steady_tests::assert_window_steady(
        VERTICAL,
        &[Vec2::new(252.0, 560.0), Vec2::new(252.0, 300.0)],
        aurora_i18n::tr!("texture-library-retry"),
        || {
            let (mut inv, library, _) = inventory();
            inv.add_items(
                (0..12)
                    .map(|n| InvItem {
                        id: Uuid::from_u128(800 + n),
                        parent: library,
                        asset_id: Uuid::from_u128(900 + n),
                        asset_type: 0,
                        name: format!("Resource {n}"),
                        ..Default::default()
                    })
                    .collect(),
            );
            let broken = folder(&mut inv, 72, library, -1, "Broken");
            inv.folders.get_mut(&broken).expect("folder").state = FetchState::Unknown;
            inv.failed(&[broken]);
            let mut world = World::new(avatars.clone());
            world.agent_id = Uuid::from_u128(70);
            world.inventory = inv;
            let ui = LibraryUi {
                agent: world.agent_id,
                message: "The library could not be prepared: the inventory did not answer.".into(),
                ..Default::default()
            };
            (ui, world, Scene::new(std::env::temp_dir(), avatars.clone()), Preferences::default())
        },
        |(ui, world, scene, prefs), ctx, p| {
            ui.show(ctx, p, world, scene, &HashMap::new(), prefs, &mut true);
        },
    );
}
