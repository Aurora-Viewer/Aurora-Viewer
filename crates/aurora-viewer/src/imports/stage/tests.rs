use super::*;
use std::sync::atomic::Ordering;

fn world() -> crate::world::World {
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.inventory.root = Uuid::from_u128(1);
    world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
        id: world.inventory.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    world
}

fn local(source: Source) -> Entry {
    let mut entry = Entry::new("cached-source.gltf".into(), Ok(source), Default::default());
    entry.local = true;
    entry.local_published = true;
    entry.local_restore_active = true;
    entry.local_slots.insert(Uuid::new_v4(), 12);
    entry
}

#[test]
fn checked_local_files_become_independent_drafts_without_uploading_or_retiring_the_previews() {
    let world = world();
    let mut mesh = local(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture())));
    mesh.name = "Edited mesh".into();
    mesh.description = "Keep this description".into();
    let token = Uuid::new_v4();
    if let Some(Source::Mesh(draft)) = &mut mesh.source {
        draft.options.scale = 0.75;
        draft.options.physics_source = 5;
        draft.physics_path = Some("collision.dae".into());
        draft.lods[1].source = mesh::LodSource::File;
        draft.lods[1].path = Some("low.dae".into());
        draft.pending = Some((Uuid::new_v4(), 4));
        draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
            token,
            cost: 10,
            data: Default::default(),
        });
        draft.progress.store(19, Ordering::Relaxed);
    }
    let mesh_id = mesh.id;
    let mut unchecked = local(Source::Texture(crate::imports::tests::image(16, 16)));
    unchecked.selected = false;
    let ordinary = Entry::new(
        "existing.png".into(),
        Ok(Source::Texture(crate::imports::tests::image(16, 16))),
        Default::default(),
    );
    let mut material = local(Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([None, None, None, None]),
    });
    material.material_index = 2;
    material.name = "Edited material".into();
    let mut imports = Imports {
        choice: Choice::Local,
        entries: vec![mesh, unchecked, ordinary, material],
        tab: ui::Tab::Folder,
        ..Default::default()
    };
    imports.type_filters[4] = false;
    let original_ids: Vec<_> = imports.entries.iter().map(|entry| entry.id).collect();
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Preferences::default()), 2);
    assert_eq!(imports.entries.len(), 6);
    assert_eq!(imports.entries[..4].iter().map(|entry| entry.id).collect::<Vec<_>>(), original_ids);
    assert_eq!(imports.choice, Choice::Bulk);
    assert_eq!(imports.tab, ui::Tab::Batch);
    assert_eq!(imports.active, Some(imports.entries[4].id));
    assert!(imports.type_filters[4], "reveal selected files previously hidden by a type filter");
    for copy in &imports.entries[4..] {
        assert!(!original_ids.contains(&copy.id));
        assert!(!copy.local && copy.selected && !copy.local_published && !copy.local_restore_active);
        assert!(copy.local_slots.is_empty() && copy.local_mesh_ids.is_empty() && copy.watch.is_none());
        assert_eq!(copy.folder, world.inventory.root);
        assert!(matches!(copy.status, Status::Ready));
    }
    assert_eq!(imports.entries[4].name, "Edited mesh");
    assert_eq!(imports.entries[4].description, "Keep this description");
    assert_eq!(imports.entries[5].name, "Edited material");
    assert_eq!(imports.entries[5].material_index, 2);
    let Some(Source::Mesh(original)) = &imports.entries[0].source else {
        panic!("original")
    };
    let Some(Source::Mesh(copy)) = &imports.entries[4].source else {
        panic!("copy")
    };
    assert_eq!(imports.entries[0].id, mesh_id);
    assert!(imports.entries[0].local_published && imports.entries[0].local_restore_active);
    assert_eq!(original.quote.as_ref().unwrap().token, token);
    assert!(original.pending.is_some() && !original.cancel.load(Ordering::Relaxed));
    assert_eq!(original.progress.load(Ordering::Relaxed), 19);
    assert!(!Arc::ptr_eq(&original.cancel, &copy.cancel) && !Arc::ptr_eq(&original.progress, &copy.progress));
    assert!(copy.quote.is_none() && copy.pending.is_none() && copy.prepared.is_none() && copy.discarded_quotes.is_empty());
    assert_eq!(copy.options.scale, 0.75);
    assert_eq!(copy.options.physics_source, 5);
    assert_eq!(copy.physics_path, original.physics_path);
    assert_eq!(copy.lods[1].path, original.lods[1].path);
    assert!(imports.queue.is_empty() && imports.current.is_none() && imports.pending_confirmed.is_none() && !imports.confirm);
    assert_eq!(imports.spent, 0);
}

#[test]
fn material_snapshots_keep_distinct_submaterials_from_the_same_file_and_follow_shared_routing() {
    let world = world();
    let mut imports = Imports {
        choice: Choice::Local,
        ..Default::default()
    };
    for index in 0..2 {
        let mut entry = local(Source::Material {
            value: PbrMaterial {
                metallic_factor: index as f32 * 0.5,
                ..Default::default()
            },
            images: Box::new([
                Some(files::local_image((*crate::imports::tests::image(16, 16)).clone(), "Map".into())),
                None,
                None,
                None,
            ]),
        });
        entry.name = format!("Material {index}");
        entry.material_index = index;
        imports.entries.push(entry);
    }
    let prefs = Preferences {
        use_default_folders: false,
        folder_layout: Some(folders::Layout::Type),
        ..Default::default()
    };
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &prefs), 2);
    for (index, copy) in imports.entries[2..].iter().enumerate() {
        let Some(Source::Material { value, images }) = &copy.source else {
            panic!("material")
        };
        assert_eq!(copy.material_index, index);
        assert_eq!(value.metallic_factor, index as f32 * 0.5);
        assert!(images[0].is_some());
        assert_eq!(
            copy.upload_path.as_ref().unwrap(),
            &vec![aurora_net::inventory::operations::folder_name(aurora_i18n::tr!(
                "import-folders-materials"
            ))]
        );
    }
    assert!(imports.queue.is_empty() && imports.spent == 0);
}

#[test]
fn local_staging_obeys_selection_loading_confirmation_and_atomic_capacity_limits() {
    let world = world();
    let mut imports = Imports {
        choice: Choice::Local,
        ..Default::default()
    };
    imports.entries.push(local(Source::Texture(crate::imports::tests::image(16, 16))));
    imports.pending_files = Some(Uuid::new_v4());
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 0);
    imports.pending_files = None;
    imports.confirm = true;
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 0);
    imports.confirm = false;
    imports.entries[0].selected = false;
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 0);
    imports.entries[0].selected = true;
    imports.entries[0].reloading = true;
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 0);
    imports.entries[0].reloading = false;
    while imports.entries.len() < 512 {
        let mut entry = local(Source::Texture(crate::imports::tests::image(1, 1)));
        entry.selected = false;
        imports.entries.push(entry);
    }
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 0);
    assert_eq!(imports.entries.len(), 512);
    assert_eq!(imports.choice, Choice::Local);
    assert_eq!(imports.message, aurora_i18n::tr!("import-batch-limit"));
    imports.entries.clear();
    let image = Arc::new(DecodedImage {
        width: 1024,
        height: 1024,
        components: 4,
        data: vec![128; 1024 * 1024 * 4],
        discard: 0,
    });
    for index in 0..256 {
        let mut entry = local(Source::Texture(image.clone()));
        entry.selected = index == 0;
        imports.entries.push(entry);
    }
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 0);
    assert_eq!(imports.entries.len(), 256, "the retained input budget also rejects the whole batch");
    assert!(imports.queue.is_empty() && imports.spent == 0);
}

#[test]
fn local_texture_references_become_upload_dependencies_and_server_references_stay_existing() {
    let world = world();
    let mut texture = local(Source::Texture(crate::imports::tests::image(16, 16)));
    texture.selected = false;
    let id = texture.id;
    let server = Uuid::new_v4();
    let material = local(Source::Material {
        value: PbrMaterial {
            base_color_texture: Some(id),
            normal_texture: Some(server),
            ..Default::default()
        },
        images: Box::new([None, None, None, None]),
    });
    let mut imports = Imports {
        choice: Choice::Local,
        entries: vec![texture, material],
        ..Default::default()
    };
    assert_eq!(imports.stage_local_uploads(&world, &Default::default(), &Default::default()), 1);
    let Some(Source::Material { value, images }) = &imports.entries[2].source else {
        panic!("material")
    };
    assert!(value.base_color_texture.is_none());
    assert_eq!(value.normal_texture, Some(server));
    let image = images[0].as_ref().unwrap();
    assert_eq!(image.key, id);
    assert!(
        image.asset.is_none(),
        "the ordinary material upload will upload this dependency instead of sending a local UUID"
    );
    assert!(images[1].is_none());
    assert_eq!(
        imports.entries.len(),
        3,
        "an unchecked texture is only a material dependency, not a standalone draft"
    );
    let Some(Source::Material { value, .. }) = &imports.entries[1].source else {
        panic!("original")
    };
    assert_eq!(value.base_color_texture, Some(id));
    assert!(imports.queue.is_empty() && imports.spent == 0);
}
