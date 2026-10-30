use super::*;

#[test]
fn import_folder_preview_folds_without_creating_folders_or_losing_edited_paths() {
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
    let mut entry = Entry::new(
        "Fabric.png".into(),
        Ok(Source::Texture(crate::imports::tests::image(16, 16))),
        Default::default(),
    );
    entry.source_folders = vec!["Project".into(), "Coat".into()];
    let mut imports = Imports {
        entries: vec![entry],
        choice: Choice::Bulk,
        ..Default::default()
    };
    imports.active = Some(imports.entries[0].id);
    imports.folders.enabled = true;
    let prefs = crate::ui::inventory::InventoryPreferences::default();
    imports.sync_folder_plan(&world, &prefs);
    let frame = |imports: &mut Imports, events| {
        let actions: Vec<Action> = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 720.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(ui, &theme.palette(), imports, &world, 600.0, false);
            },
        );
        output.textures_delta.clear();
        (output, actions)
    };
    let position = |output: &egui::FullOutput, label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .unwrap_or_else(|| panic!("visible {label}"))
    };
    let click = |imports: &mut Imports, pos| {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            let (_, mut next) = frame(
                imports,
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
            actions.append(&mut next);
        }
        actions
    };
    let (output, _) = frame(&mut imports, Vec::new());
    assert!(!imports.busy());
    assert_eq!(world.inventory.folders.len(), 1);
    position(&output, aurora_i18n::tr!("import-folders-auto-create"));
    for removed in [
        aurora_i18n::tr!("import-folders-create"),
        aurora_i18n::tr!("common-all"),
        aurora_i18n::tr!("common-none"),
    ] {
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == removed))
        );
    }
    drop(output);
    assert!(!imports.busy());
    assert_eq!(world.inventory.folders.len(), 1);
    let path = imports.entries[0].upload_path.clone().unwrap();
    let caret = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_plan_caret", path.clone()))))
        .unwrap();
    assert!(click(&mut imports, caret.center()).is_empty());
    assert_eq!(imports.folders.collapsed.len(), 1);
    assert_eq!(imports.entries[0].upload_path.as_ref(), Some(&path));
    assert!(imports.entries[0].selected, "folding the source tree does not exclude its files");
    assert!(click(&mut imports, caret.center()).is_empty());
    assert!(imports.folders.collapsed.is_empty());
}

#[test]
fn inline_destinations_keep_equal_names_independent_and_validate_without_changing_upload_selection() {
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
    let root = destination_path(&world, world.inventory.root);
    let mut entries = vec![
        Entry::new(
            "same.dae".into(),
            Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
            Default::default(),
        ),
        Entry::new(
            "same.png".into(),
            Ok(Source::Texture(crate::imports::tests::image(16, 16))),
            Default::default(),
        ),
    ];
    for (entry, path) in entries.iter_mut().zip(["A", "B"]) {
        entry.upload_path = Some(vec![path.into()]);
        entry.folder = world.inventory.root;
    }
    entries[0].selected = false;
    let token = Uuid::new_v4();
    let generation = entries[0].preview_generation;
    if let Some(Source::Mesh(draft)) = &mut entries[0].source {
        draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
            token,
            cost: 20,
            data: Default::default(),
        });
        draft.pending = Some((Uuid::new_v4(), generation));
    }
    let frame = |entries: &mut Vec<Entry>, events, busy: bool, custom: bool| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 720.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(460.0);
                for entry in entries.iter_mut() {
                    ui.push_id(entry.id, |ui| {
                        file_row(ui, &theme.palette(), entry, &world, custom.then_some(root.as_str()), false, busy);
                    });
                }
            },
        );
        output.textures_delta.clear();
        output
    };
    let click = |entries: &mut Vec<Entry>, position| {
        for pressed in [true, false] {
            frame(
                entries,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                false,
                true,
            );
        }
    };
    let type_path = |entries: &mut Vec<Entry>, index: usize, text: &str| {
        let field = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_file_path_field", entries[index].id))))
            .unwrap();
        click(entries, field.center());
        frame(
            entries,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text(text.into()),
            ],
            false,
            true,
        );
    };
    for _ in 0..3 {
        frame(&mut entries, Vec::new(), false, true);
    }
    type_path(&mut entries, 1, "Pending/Second");
    type_path(&mut entries, 0, "Changed/First");
    let apply = |id| {
        ctx.data(|data| data.get_temp::<(egui::Rect, bool)>(egui::Id::unique(("import_file_path_apply", id))))
            .unwrap()
    };
    assert!(apply(entries[0].id).1);
    let position = apply(entries[0].id).0.center();
    click(&mut entries, position);
    assert_eq!(
        entries[0].upload_path.as_ref().unwrap(),
        &vec!["Changed".to_owned(), "First".into()]
    );
    assert_eq!(entries[1].upload_path.as_ref().unwrap(), &vec!["B".to_owned()]);
    let second = ctx
        .data_mut(|data| data.get_temp::<FilePathEdit>(egui::Id::unique(("import_file_path_edit", entries[1].id))))
        .unwrap();
    assert_eq!(second.text, "Pending/Second");
    let Some(Source::Mesh(draft)) = &entries[0].source else {
        panic!("mesh")
    };
    assert!(draft.quote.is_none() && draft.pending.is_none() && draft.discarded_quotes.contains(&token));
    assert!(!entries[0].selected && entries[1].selected);
    type_path(&mut entries, 0, "Bad//Name");
    assert!(!apply(entries[0].id).1);
    click(&mut entries, position);
    assert_eq!(
        entries[0].upload_path.as_ref().unwrap(),
        &vec!["Changed".to_owned(), "First".into()]
    );
    frame(&mut entries, Vec::new(), true, true);
    assert!(entries.iter().all(|entry| !apply(entry.id).1));
    let defaults = frame(&mut entries, Vec::new(), false, false);
    assert!(entries.iter().all(|entry| !apply(entry.id).1));
    assert!(
        defaults
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == root))
    );
    assert_eq!(world.inventory.folders.len(), 1, "editing destinations never creates folders");
}
