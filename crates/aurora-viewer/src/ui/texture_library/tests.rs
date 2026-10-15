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
    (inv, library, source)
}
#[test]
fn texture_library_root_creation_reuses_aurora_and_does_not_repeat() {
    let (mut inv, library, _) = inventory();
    inv.remove(&[library]);
    let mut ui = LibraryUi::default();
    let mut actions = Vec::new();
    ui.prepare_root(&mut inv, &mut actions);
    assert_eq!(actions.len(), 1);
    let Action::Change(change) = &actions[0] else { panic!("creation") };
    assert_eq!(change.operations.len(), 1);
    assert!(matches!(&change.operations[0],Operation::Folder {parent,name,..} if *parent==Uuid::from_u128(3)&&name==catalogue::LIBRARY));
    ui.prepare_root(&mut inv, &mut actions);
    assert_eq!(actions.len(), 1);
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
        let _ = ctx.run_ui(
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
    };
    for _ in 0..8 {
        show(vec![], &mut ui, &mut prefs, &mut open);
    }
    let rect = ctx.memory(|m| m.area_rect(egui::Id::new(VERTICAL))).expect("vertical window");
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
    let rect = ctx.memory(|m| m.area_rect(egui::Id::new(HORIZONTAL))).expect("horizontal window");
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
        for kind in [0, 57] {
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
                    |host| {
                        actions = ui.show(
                            host.ctx(),
                            &crate::theme::Theme::default().palette(),
                            &mut world,
                            &mut scene,
                            &HashMap::new(),
                            &mut prefs,
                            &mut true,
                        );
                    },
                );
                output.textures_delta.clear();
                actions
            };
            for _ in 0..8 {
                frame(Vec::new(), &mut ui);
            }
            let source = ctx
                .data(|data| data.get_temp::<egui::Pos2>(egui::Id::new("texture_library_first_card")))
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
            assert!(matches!(actions.as_slice(), [Action::Apply(id)] if *id == item));
            assert_eq!(ui.selected, Some(item));
            frame(button(source, true), &mut ui);
            assert!(
                frame(button(source, false), &mut ui).is_empty(),
                "the second click opens preview without reapplying"
            );
            assert_eq!(ui.preview, Some(item));
        }
    }
}
