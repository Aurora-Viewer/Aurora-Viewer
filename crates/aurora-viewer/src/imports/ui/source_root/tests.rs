use super::*;

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
