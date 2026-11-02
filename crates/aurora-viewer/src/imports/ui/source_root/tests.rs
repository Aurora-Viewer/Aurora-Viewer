use super::*;

#[test]
fn loose_file_controls_update_names_immediately_and_block_invalid_names_in_full_and_type_windows() {
    for choice in [Choice::Bulk, Choice::Texture] {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory.root = Uuid::from_u128(1);
        world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
            id: world.inventory.root,
            name: "Inventory".into(),
            type_default: 8,
            ..Default::default()
        }]);
        let mut imports = Imports {
            entries: vec![Entry::new(
                "file.png".into(),
                Ok(Source::Texture(crate::imports::tests::image(16, 16))),
                Default::default(),
            )],
            ..Default::default()
        };
        let mut prefs = Preferences {
            folder_layout: Some(folders::Layout::Flat),
            ..Default::default()
        };
        let frame = |imports: &mut Imports, prefs: &mut Preferences, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 650.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    imports.sync_destinations(prefs, &world, &Default::default());
                    imports.with_window(choice, |imports| {
                        folder_panel::destination_row(ui, &theme.palette(), imports, prefs, &world, &Default::default(), false);
                    });
                },
            );
            output.textures_delta.clear();
            output
        };
        let click = |imports: &mut Imports, prefs: &mut Preferences, key| {
            let pos = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(key)))
                .expect("folder control")
                .center();
            for pressed in [true, false] {
                frame(
                    imports,
                    prefs,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        for _ in 0..3 {
            frame(&mut imports, &mut prefs, vec![]);
        }
        click(&mut imports, &mut prefs, "import_create_folder");
        assert!(imports.folders.roots.loose.is_none(), "default folders disable custom grouping");
        prefs.use_default_folders = false;
        let output = frame(&mut imports, &mut prefs, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-include-source-folder"))));
        click(&mut imports, &mut prefs, "import_create_folder");
        frame(&mut imports, &mut prefs, vec![]);
        assert!(!imports.folder_plan_valid(&world, &Default::default()));
        click(&mut imports, &mut prefs, "import_loose_folder_name");
        let output = frame(&mut imports, &mut prefs, vec![egui::Event::Text("Release".into())]);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("common-apply")))
        );
        assert_eq!(imports.entries[0].upload_path.as_ref().unwrap(), &vec!["Release"]);
        assert!(imports.folder_plan_valid(&world, &Default::default()));
        for name in ["", "invalid/name", "Renamed"] {
            let mut events = Vec::new();
            for pressed in [true, false] {
                events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                });
            }
            if name.is_empty() {
                for pressed in [true, false] {
                    events.push(egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            } else {
                events.push(egui::Event::Text(name.into()));
            }
            frame(&mut imports, &mut prefs, events);
            frame(&mut imports, &mut prefs, vec![]);
            assert_eq!(imports.folder_plan_valid(&world, &Default::default()), name == "Renamed");
        }
        assert_eq!(imports.entries[0].upload_path.as_ref().unwrap(), &vec!["Renamed"]);
        assert!(prefs.include_source_folder && imports.entries[0].source_folders.is_empty());
        click(&mut imports, &mut prefs, "import_create_folder");
        assert_eq!(imports.entries[0].upload_path.as_ref().unwrap(), &Vec::<String>::new());
        assert_eq!(world.inventory.folders.len(), 1);
    }
}

#[test]
fn shared_source_root_controls_apply_renames_and_persist_the_checkbox_without_touching_disk_paths() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut entry = Entry::new(
        "file.png".into(),
        Ok(Source::Texture(crate::imports::tests::image(16, 16))),
        Default::default(),
    );
    entry.source_folders = vec!["Pack".into(), "Nested".into()];
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    let mut prefs = Preferences {
        use_default_folders: false,
        folder_layout: Some(folders::Layout::Mirror),
        ..Default::default()
    };
    let frame = |imports: &mut Imports, prefs: &mut Preferences, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 500.0))),
                events,
                ..Default::default()
            },
            |ui| {
                folder_panel::destination_row(ui, &theme.palette(), imports, prefs, &world, &Default::default(), false);
            },
        );
        output.textures_delta.clear();
    };
    let click = |imports: &mut Imports, prefs: &mut Preferences, key| {
        let pos = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(key))).unwrap().center();
        for pressed in [true, false] {
            frame(
                imports,
                prefs,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    };
    for _ in 0..3 {
        frame(&mut imports, &mut prefs, vec![]);
    }
    click(&mut imports, &mut prefs, "import_source_root_name");
    frame(
        &mut imports,
        &mut prefs,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("Release".into()),
        ],
    );
    click(&mut imports, &mut prefs, "import_source_root_apply");
    assert_eq!(imports.entries[0].upload_path.as_ref().unwrap(), &vec!["Release", "Nested"]);
    click(&mut imports, &mut prefs, "import_include_source_folder");
    assert!(!prefs.include_source_folder);
    assert_eq!(imports.entries[0].upload_path.as_ref().unwrap(), &vec!["Nested"]);
    let restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
    assert!(!restored.include_source_folder);
    click(&mut imports, &mut prefs, "import_include_source_folder");
    assert_eq!(imports.entries[0].upload_path.as_ref().unwrap(), &vec!["Release", "Nested"]);
    assert_eq!(imports.entries[0].source_folders, vec!["Pack", "Nested"]);
}
