use super::*;
use aurora_net::inventory::InvFolder;

pub(super) fn fixture() -> (Imports, crate::world::World, crate::ui::inventory::InventoryPreferences) {
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.inventory.root = Uuid::from_u128(1);
    world.inventory.created_folders(vec![InvFolder {
        id: world.inventory.root,
        parent: Uuid::nil(),
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    imports.folders.enabled = true;
    for path in [vec!["Project", "Coat"], vec!["Project", "Hat"], vec!["Project", "Coat"]] {
        let mut entry = Entry::new(
            "same.png".into(),
            Ok(Source::Texture(super::super::tests::image(16, 16))),
            Default::default(),
        );
        entry.source_folders = path.into_iter().map(str::to_owned).collect();
        imports.entries.push(entry);
    }
    let prefs = Default::default();
    imports.sync_folder_plan(&world, &prefs);
    (imports, world, prefs)
}

#[test]
fn import_folder_discovery_mirrors_nested_sources_filters_formats_and_reports_limits() {
    let root = std::env::temp_dir().join(format!("aurora-folder-discovery-{}", Uuid::new_v4()));
    std::fs::create_dir_all(root.join("Coat/Empty")).unwrap();
    std::fs::create_dir_all(root.join("Hat")).unwrap();
    for name in [
        "Coat/a.PNG",
        "Hat/a.png",
        "sound.wav",
        "animation.anim",
        "material.gltf",
        "mesh.dae",
        "readme.txt",
        "buffer.bin",
        "skip.txt",
        "motion.bvh",
    ] {
        std::fs::write(root.join(name), []).unwrap();
    }
    #[cfg(windows)]
    let link = std::os::windows::fs::symlink_dir(&root, root.join("Hat/loop")).is_ok();
    let files = discover(&root, 10).unwrap();
    assert_eq!(files.len(), 7);
    let coat = files.iter().find(|(p, _)| p.ends_with("Coat/a.PNG")).unwrap();
    assert_eq!(
        coat.1,
        vec![root.file_name().unwrap().to_string_lossy().into_owned(), "Coat".into()]
    );
    assert!(
        discover(&root, 6).is_err(),
        "a large selection is reported, never silently truncated"
    );
    #[cfg(windows)]
    if link {
        std::fs::remove_dir(root.join("Hat/loop")).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn import_folder_plan_edits_whole_subtrees_and_type_or_flat_layouts_keep_distinct_files() {
    let (mut imports, world, prefs) = fixture();
    let old = vec!["Project".into(), "Coat".into()];
    let new = vec!["Release".into(), "Clothing".into()];
    assert!(rename(&mut imports.entries, &old, &new));
    assert_eq!(imports.entries[0].upload_path, Some(new.clone()));
    assert_eq!(imports.entries[2].upload_path, Some(new));
    assert_eq!(imports.entries[1].upload_path.as_ref().unwrap()[1], "Hat");
    assert_eq!(
        layout_path(&imports.entries[0], Layout::Type),
        vec![folder_name("Project"), folder_name(aurora_i18n::tr!("import-folders-textures"))]
    );
    assert_eq!(layout_path(&imports.entries[0], Layout::Flat), vec!["Project"]);
    for invalid in ["/top", "top/", "a//b", "../b", "a/./b", "a|b", "a\\b", " a", "a\0b"] {
        assert!(parse_path(invalid).is_none(), "{invalid}");
    }
    assert_eq!(parse_path(""), Some(Vec::new()));
    assert_eq!(parse_path("Étape"), Some(vec![folder_name("Étape")]));
    assert!(!imports.folder_plan_ready(&world, &prefs));
    assert_eq!(imports.entries.len(), 3);
}

pub(super) fn complete_one(
    imports: &mut Imports,
    world: &mut crate::world::World,
    prefs: &crate::ui::inventory::InventoryPreferences,
    remap: bool,
) -> Option<Uuid> {
    let command = imports.next_import_folder(world, prefs)?;
    let aurora_net::NetCommand::EditInventory { request, mut change } = command else {
        panic!("folder command")
    };
    let Operation::Folder { id: asked, parent, .. } = &change.operations[0] else {
        panic!("folder")
    };
    let asked = *asked;
    assert!(
        world.inventory.folders.contains_key(parent),
        "parent must have been acknowledged first"
    );
    let id = if remap { Uuid::new_v4() } else { asked };
    if let Operation::Folder { id: target, .. } = &mut change.operations[0] {
        *target = id;
    }
    let mut reply = rules::demo_mutate(&mut world.inventory, world.agent_id, change).unwrap();
    reply.created.insert(asked, id);
    imports.import_folder_result(request, Ok(reply), &mut world.inventory);
    Some(id)
}

#[test]
fn type_grouping_creates_each_imported_root_above_its_types_and_keeps_loose_files_unwrapped() {
    let (mut imports, mut world, inventory) = fixture();
    imports.entries[1].source_folders[0] = "Other Pack".into();
    imports.entries[2].source_folders.clear();
    imports.sync_destinations(
        &Preferences {
            use_default_folders: false,
            folder_layout: Some(Layout::Type),
            ..Default::default()
        },
        &world,
        &inventory,
    );
    let kind = folder_name(aurora_i18n::tr!("import-folders-textures"));
    assert_eq!(
        imports.entries[0].upload_path.as_ref().unwrap(),
        &vec!["Project".to_owned(), kind.clone()]
    );
    assert_eq!(
        imports.entries[1].upload_path.as_ref().unwrap(),
        &vec!["Other Pack".to_owned(), kind.clone()]
    );
    assert_eq!(imports.entries[2].upload_path.as_ref().unwrap(), &vec![kind.clone()]);
    imports.prepare_import_folders(&world, &inventory);
    while complete_one(&mut imports, &mut world, &inventory, true).is_some() {}
    assert!(imports.folder_plan_ready(&world, &inventory));
    for entry in &imports.entries {
        let leaf = &world.inventory.folders[&entry.folder].info;
        assert_eq!(leaf.name, kind);
        if let Some(root) = entry.source_folders.first() {
            let parent = &world.inventory.folders[&leaf.parent].info;
            assert_eq!(parent.name, folder_name(root));
            assert_eq!(parent.parent, world.inventory.root);
        } else {
            assert_eq!(leaf.parent, world.inventory.root);
        }
    }
    assert_eq!(world.inventory.folders.len(), 6);
    assert!(imports.queue.is_empty() && imports.spent == 0);
}

#[test]
fn import_folder_creation_waits_for_ack_remaps_ids_and_never_starts_paid_uploads() {
    let (mut imports, mut world, prefs) = fixture();
    imports.entries[1].selected = false;
    imports.prepare_import_folders(&world, &prefs);
    assert!(imports.busy());
    let command = imports.next_import_folder(&world, &prefs).unwrap();
    let aurora_net::NetCommand::EditInventory { request, change } = command else {
        panic!("folder")
    };
    assert!(imports.next_import_folder(&world, &prefs).is_none());
    assert!(!imports.folder_plan_ready(&world, &prefs));
    let reply = rules::demo_mutate(&mut world.inventory, world.agent_id, change).unwrap();
    imports.import_folder_result(request, Ok(reply), &mut world.inventory);
    let leaf = complete_one(&mut imports, &mut world, &prefs, true).unwrap();
    assert!(complete_one(&mut imports, &mut world, &prefs, false).is_none());
    assert!(!imports.busy());
    assert!(imports.folder_plan_ready(&world, &prefs));
    assert_eq!(imports.entries[0].folder, leaf);
    assert_eq!(imports.entries[2].folder, leaf);
    assert!(world.inventory.folders.values().all(|f| f.info.name != "Hat"));
    assert!(imports.queue.is_empty() && imports.current.is_none());
    assert_eq!(imports.spent, 0);
    imports.entries[1].selected = true;
    assert!(!imports.folder_plan_ready(&world, &prefs));
    imports.prepare_import_folders(&world, &prefs);
    assert!(complete_one(&mut imports, &mut world, &prefs, false).is_some());
    assert!(complete_one(&mut imports, &mut world, &prefs, false).is_none());
    assert!(imports.folder_plan_ready(&world, &prefs));
}

#[test]
fn import_folder_failure_cancel_and_protection_stop_work_without_losing_confirmed_folders() {
    let (mut imports, mut world, mut prefs) = fixture();
    imports.prepare_import_folders(&world, &prefs);
    let root = complete_one(&mut imports, &mut world, &prefs, false).unwrap();
    let aurora_net::NetCommand::EditInventory { request, .. } = imports.next_import_folder(&world, &prefs).unwrap() else {
        panic!("folder")
    };
    imports.import_folder_result(Uuid::new_v4(), Err("stale".into()), &mut world.inventory);
    assert!(imports.busy());
    imports.import_folder_result(request, Err("failed".into()), &mut world.inventory);
    assert!(!imports.busy());
    assert!(imports.next_import_folder(&world, &prefs).is_none());
    assert!(world.inventory.folders.contains_key(&root));
    imports.prepare_import_folders(&world, &prefs);
    let aurora_net::NetCommand::EditInventory { request, change } = imports.next_import_folder(&world, &prefs).unwrap() else {
        panic!("folder")
    };
    imports.cancel_remaining();
    let reply = rules::demo_mutate(&mut world.inventory, world.agent_id, change).unwrap();
    imports.import_folder_result(request, Ok(reply), &mut world.inventory);
    assert!(!imports.busy());
    assert!(imports.next_import_folder(&world, &prefs).is_none());
    prefs.protected.insert(world.inventory.root);
    imports.prepare_import_folders(&world, &prefs);
    assert!(!imports.busy());
    assert!(!imports.folder_plan_ready(&world, &prefs));
    assert!(imports.queue.is_empty());
}

#[test]
fn import_folder_external_moves_require_a_new_plan_and_defaults_preserve_edits() {
    let (mut imports, mut world, prefs) = fixture();
    let original = Uuid::from_u128(98);
    world.inventory.created_folders(vec![InvFolder {
        id: original,
        parent: world.inventory.root,
        name: "Manual".into(),
        type_default: -1,
        ..Default::default()
    }]);
    imports.prepare_import_folders(&world, &prefs);
    while complete_one(&mut imports, &mut world, &prefs, false).is_some() {}
    assert!(imports.folder_plan_ready(&world, &prefs));
    let destination = imports.entries[0].folder;
    world.inventory.folders.get_mut(&destination).unwrap().info.parent = original;
    assert!(
        !imports.folder_plan_ready(&world, &prefs),
        "a moved folder no longer matches the tree preview"
    );
    imports.sync_destinations(&Preferences::default(), &world, &prefs);
    assert_eq!(imports.entries[0].folder, world.inventory.root);
    assert!(imports.entries.iter().all(|e| e.upload_path.is_some()));
    assert!(imports.folder_plan_ready(&world, &prefs));
}

#[test]
fn import_folder_upload_queue_keeps_the_material_and_its_dependencies_in_the_confirmed_destination() {
    let (mut imports, mut world, prefs) = fixture();
    imports.entries.truncate(1);
    imports.entries[0].source = Some(Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([
            Some(LocalImage {
                path: None,
                key: Uuid::new_v4(),
                name: "Map".into(),
                pixels: super::super::tests::image(16, 16),
                asset: None,
                empty_alpha: false,
            }),
            None,
            None,
            None,
        ]),
    });
    world.balance = Some(1000);
    let prices = Prices {
        texture: Some(10),
        large_texture: Some(40),
        sound: Some(10),
        animation: Some(10),
    };
    imports.start(&world, &prefs, &Preferences::default(), &prices);
    assert!(imports.queue.is_empty(), "unprepared paths cannot start uploads");
    imports.prepare_import_folders(&world, &prefs);
    while complete_one(&mut imports, &mut world, &prefs, true).is_some() {}
    let folder = imports.entries[0].folder;
    imports.start(&world, &prefs, &Preferences::default(), &prices);
    assert_eq!(imports.queue.len(), 2);
    assert!(imports.queue.iter().all(|step| step.folder == folder));
    assert_eq!(imports.queue[0].kind, Kind::Texture);
    assert_eq!(imports.queue[1].kind, Kind::Material);
    assert_eq!(imports.spent, 0, "queueing is not an acknowledged charge");
}

#[test]
fn import_folder_toggle_discards_mesh_quotes_even_when_the_mesh_destination_is_unchanged() {
    let (mut imports, world, prefs) = fixture();
    imports.folders.enabled = false;
    imports.entries.clear();
    let mut entry = Entry::new(
        "mesh.dae".into(),
        Ok(Source::Mesh(Box::new(super::super::tests::mesh_fixture()))),
        Default::default(),
    );
    entry.folder = world.inventory.root;
    imports.entries.push(entry);
    for enabled in [true, false] {
        let token = Uuid::new_v4();
        let generation = imports.entries[0].preview_generation;
        if let Some(Source::Mesh(draft)) = &mut imports.entries[0].source {
            draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
                token,
                cost: 10,
                data: Default::default(),
            });
            draft.pending = Some((Uuid::new_v4(), generation));
        }
        imports.sync_destinations(
            &Preferences {
                use_default_folders: !enabled,
                folder_layout: Some(Layout::Flat),
                ..Default::default()
            },
            &world,
            &prefs,
        );
        assert_eq!(imports.entries[0].folder, world.inventory.root);
        assert_eq!(imports.entries[0].preview_generation, generation + 1);
        let Some(Source::Mesh(draft)) = &imports.entries[0].source else {
            panic!("mesh")
        };
        assert!(draft.quote.is_none() && draft.pending.is_none());
        assert!(draft.discarded_quotes.contains(&token));
    }
}

#[test]
fn batch_destination_preference_defaults_on_and_persists_the_last_choice() {
    let mut prefs: Preferences = serde_json::from_str("{}").unwrap();
    assert!(prefs.use_default_folders);
    assert!(prefs.include_source_folder);
    assert!(prefs.batch_folder.is_nil());
    assert!(prefs.folder_layout.is_none());
    prefs.use_default_folders = false;
    prefs.include_source_folder = false;
    prefs.batch_folder = Uuid::from_u128(91);
    prefs.folder_layout = Some(Layout::Type);
    let restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
    assert!(!restored.use_default_folders);
    assert!(!restored.include_source_folder);
    assert_eq!(restored.batch_folder, prefs.batch_folder);
    assert_eq!(restored.folder_layout, prefs.folder_layout);
}

#[test]
fn batch_routing_preserves_folder_edits_and_sends_material_dependencies_to_the_common_destination() {
    let (mut imports, mut world, mut inventory) = fixture();
    let common = Uuid::from_u128(90);
    let mut new_folders = vec![InvFolder {
        id: common,
        parent: world.inventory.root,
        name: "Common".into(),
        type_default: -1,
        ..Default::default()
    }];
    for (index, kind) in [Kind::Mesh, Kind::Material, Kind::Texture].into_iter().enumerate() {
        let id = Uuid::from_u128(91 + index as u128);
        new_folders.push(InvFolder {
            id,
            parent: world.inventory.root,
            name: format!("Default {index}"),
            type_default: -1,
            ..Default::default()
        });
        inventory.upload_folders[kind.folder_slot()] = id;
    }
    world.inventory.created_folders(new_folders);
    imports.entries[0].source = Some(Source::Mesh(Box::new(super::super::tests::mesh_fixture())));
    imports.entries[1].source = Some(Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([
            Some(LocalImage {
                path: None,
                key: Uuid::new_v4(),
                name: "Map".into(),
                pixels: super::super::tests::image(16, 16),
                asset: None,
                empty_alpha: false,
            }),
            None,
            None,
            None,
        ]),
    });
    let paths: Vec<_> = imports.entries.iter().map(|entry| entry.upload_path.clone()).collect();
    let mut prefs = Preferences {
        batch_folder: common,
        ..Default::default()
    };
    imports.sync_destinations(&prefs, &world, &inventory);
    for entry in &imports.entries {
        let kind = entry.source.as_ref().unwrap().kind();
        assert_eq!(entry.folder, inventory.upload_folders[kind.folder_slot()]);
    }
    let token = Uuid::new_v4();
    if let Some(Source::Mesh(draft)) = &mut imports.entries[0].source {
        draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
            token,
            cost: 10,
            data: Default::default(),
        });
    }
    prefs.use_default_folders = false;
    prefs.include_source_folder = false;
    imports.sync_destinations(&prefs, &world, &inventory);
    assert!(imports.entries.iter().all(|entry| entry.folder == common));
    let Some(Source::Mesh(draft)) = &imports.entries[0].source else {
        panic!("mesh")
    };
    assert!(draft.quote.is_none());
    assert!(draft.discarded_quotes.contains(&token));
    inventory.protected.insert(common);
    imports.sync_destinations(&prefs, &world, &inventory);
    assert!(
        imports.entries.iter().all(|entry| entry.folder == common),
        "a protected target is never silently redirected"
    );
    assert!(!imports.folder_plan_ready(&world, &inventory));
    inventory.protected.remove(&common);
    prefs.batch_folder = Uuid::from_u128(999);
    imports.sync_destinations(&prefs, &world, &inventory);
    assert!(
        !imports.folder_plan_ready(&world, &inventory),
        "a missing saved target requires a new selection"
    );
    prefs.batch_folder = common;
    imports.sync_destinations(&prefs, &world, &inventory);
    assert!(imports.folder_plan_ready(&world, &inventory));
    prefs.include_source_folder = true;
    prefs.folder_layout = Some(Layout::Mirror);
    imports.sync_destinations(&prefs, &world, &inventory);
    assert_eq!(
        imports.entries.iter().map(|entry| entry.upload_path.clone()).collect::<Vec<_>>(),
        paths
    );
    assert!(
        !imports.folder_plan_ready(&world, &inventory),
        "the edited source plan still needs folder creation"
    );
    prefs.include_source_folder = false;
    prefs.folder_layout = Some(Layout::Flat);
    imports.sync_destinations(&prefs, &world, &inventory);
    imports.entries[0].selected = false;
    world.balance = Some(1000);
    imports.start(
        &world,
        &inventory,
        &prefs,
        &Prices {
            texture: Some(10),
            large_texture: Some(40),
            sound: Some(10),
            animation: Some(10),
        },
    );
    assert_eq!(imports.queue.len(), 3, "texture, material map and material");
    assert!(imports.queue.iter().all(|step| step.folder == common));
    assert_eq!(
        imports.queue[0].kind,
        Kind::Texture,
        "the material dependency also follows the common destination"
    );
}
