use super::super::tests::{complete_one, fixture};
use super::*;

#[test]
fn every_layout_can_include_rename_or_omit_the_source_root_without_wrapping_loose_files() {
    let (mut imports, world, inventory) = fixture();
    imports.entries[1].source_folders[0] = "Other Pack".into();
    imports.entries[2].source_folders.clear();
    imports.entries[2].upload_path = None;
    assert!(imports.rename_source_root("Project", "Release"));
    assert!(!imports.rename_source_root("Project", "a/b"));
    assert!(!imports.rename_source_root("Project", ""));
    let kind = folder_name(aurora_i18n::tr!("import-folders-textures"));
    for layout in [Layout::Flat, Layout::Type, Layout::Mirror] {
        for include in [true, false, true] {
            imports.sync_destinations(
                &Preferences {
                    use_default_folders: false,
                    include_source_folder: include,
                    folder_layout: Some(layout),
                    ..Default::default()
                },
                &world,
                &inventory,
            );
            let tail = match layout {
                Layout::Flat => vec![],
                Layout::Type => vec![kind.clone()],
                Layout::Mirror => vec!["Coat".into()],
            };
            let expected: Vec<_> = include.then(|| "Release".to_owned()).into_iter().chain(tail).collect();
            assert_eq!(imports.entries[0].upload_path.as_ref(), Some(&expected));
            let loose = if layout == Layout::Type { vec![kind.clone()] } else { vec![] };
            assert_eq!(imports.entries[2].upload_path.as_ref(), Some(&loose));
            assert_eq!(
                imports.entries[1].upload_path.as_ref().unwrap().first().map(String::as_str),
                if include {
                    Some("Other Pack")
                } else {
                    match layout {
                        Layout::Flat => None,
                        Layout::Type => Some(kind.as_str()),
                        Layout::Mirror => Some("Hat"),
                    }
                }
            );
        }
    }
}

#[test]
fn source_root_controls_preserve_inner_edits_and_discard_stale_mesh_quotes() {
    let (mut imports, world, inventory) = fixture();
    imports.entries[0].source = Some(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture())));
    imports.entries[0].upload_path = Some(vec!["Project".into(), "Custom".into(), "Nested".into()]);
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
    assert!(imports.rename_source_root("Project", "Release"));
    assert_eq!(
        imports.entries[0].upload_path.as_ref().unwrap(),
        &vec!["Release", "Custom", "Nested"]
    );
    assert_eq!(imports.entries[0].preview_generation, generation + 1);
    let Some(Source::Mesh(draft)) = &imports.entries[0].source else {
        panic!("mesh")
    };
    assert!(draft.quote.is_none() && draft.pending.is_none());
    assert!(draft.discarded_quotes.contains(&token));
    for include in [false, true] {
        imports.sync_destinations(
            &Preferences {
                use_default_folders: false,
                include_source_folder: include,
                folder_layout: Some(Layout::Mirror),
                ..Default::default()
            },
            &world,
            &inventory,
        );
        assert_eq!(
            imports.entries[0].upload_path.as_ref().unwrap(),
            &if include {
                vec!["Release", "Custom", "Nested"]
            } else {
                vec!["Custom", "Nested"]
            }
        );
    }
    assert_eq!(imports.entries[1].source_folders, vec!["Project", "Hat"]);
    assert!(imports.queue.is_empty() && imports.spent == 0);
}

#[test]
fn single_destination_creates_the_named_wrapper_and_uses_its_acknowledged_uuid() {
    let (mut imports, mut world, inventory) = fixture();
    imports.entries[2].source_folders.clear();
    assert!(imports.rename_source_root("Project", "Release"));
    imports.sync_destinations(
        &Preferences {
            use_default_folders: false,
            folder_layout: Some(Layout::Flat),
            ..Default::default()
        },
        &world,
        &inventory,
    );
    assert!(!imports.folder_plan_ready(&world, &inventory));
    assert_eq!(world.inventory.folders.len(), 1, "inspection creates nothing");
    imports.prepare_import_folders(&world, &inventory);
    let wrapper = complete_one(&mut imports, &mut world, &inventory, true).unwrap();
    assert!(complete_one(&mut imports, &mut world, &inventory, true).is_none());
    assert!(imports.folder_plan_ready(&world, &inventory));
    assert_eq!(world.inventory.folders[&wrapper].info.name, "Release");
    assert_eq!(world.inventory.folders[&wrapper].info.parent, world.inventory.root);
    assert_eq!(imports.entries[0].folder, wrapper);
    assert_eq!(imports.entries[1].folder, wrapper);
    assert_eq!(imports.entries[2].folder, world.inventory.root);
    assert!(imports.queue.is_empty() && imports.spent == 0);
}

#[test]
fn admitting_a_source_folder_preserves_the_current_tab_and_selects_the_first_file() {
    let (_, world, inventory) = fixture();
    let ctx = egui::Context::default();
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    for tab in [
        crate::imports::ui::Tab::Batch,
        crate::imports::ui::Tab::Folder,
        crate::imports::ui::Tab::Permissions,
    ] {
        let request = Uuid::new_v4();
        let mut imports = Imports {
            tab,
            pending_files: Some(request),
            ..Default::default()
        };
        let mut entry = Entry::new(
            "new.png".into(),
            Ok(Source::Texture(crate::imports::tests::image(16, 16))),
            Default::default(),
        );
        entry.source_folders = vec!["Project".into()];
        let id = entry.id;
        imports.tx.send(Worker::Loaded(request, vec![entry])).unwrap();
        assert!(imports.poll(&ctx, &jobs, &world, &inventory).is_empty());
        assert_eq!(imports.tab, tab);
        assert_eq!(imports.active, Some(id));
    }
}
