use super::*;
use crate::scene::avatar::AvatarLibrary;

fn world() -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.parcel = Some(Arc::new(aurora_net::ParcelInfo {
        owner_id: world.agent_id,
        ..Default::default()
    }));
    crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
    world
}
fn pending(world: &World) -> (Pending, BuildCmd) {
    plan(
        world,
        Uuid::new_v4(),
        3,
        Uuid::from_u128(8100),
        "fixture-grid".into(),
        std::env::temp_dir(),
    )
    .expect("copyable object")
}

#[test]
fn inventory_model_rez_is_explicit_preserves_original_and_requires_copy_build_and_single_linkset() {
    let mut world = world();
    let item = Uuid::from_u128(8100);
    let before = world.inventory.items[&item].clone();
    let (pending, command) = pending(&world);
    let BuildCmd::RezInventory(rez) = command else {
        panic!("single rez")
    };
    assert_eq!(rez.item.id, item);
    assert!(rez.selected);
    assert!(!rez.remove, "never consume or wear the original");
    assert_eq!(rez.item.owner_mask, before.owner_mask);
    assert!(pending.saved.path.starts_with(std::env::temp_dir().join("fixture-grid")));
    world.inventory.items.get_mut(&item).unwrap().owner_mask &= !actions::COPY;
    assert!(plan(&world, Uuid::nil(), 0, item, "test".into(), PathBuf::new()).is_err());
    world.inventory.items.get_mut(&item).unwrap().owner_mask = before.owner_mask;
    world.inventory.items.get_mut(&item).unwrap().flags |= crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS;
    assert!(plan(&world, Uuid::nil(), 0, item, "test".into(), PathBuf::new()).is_err());
    world.inventory.items.get_mut(&item).unwrap().flags = before.flags;
    world.parcel = Some(Arc::new(aurora_net::ParcelInfo {
        owner_id: Uuid::new_v4(),
        flags: 0,
        ..Default::default()
    }));
    assert!(plan(&world, Uuid::nil(), 0, item, "test".into(), PathBuf::new()).is_err());
}

#[test]
fn inventory_model_receipt_and_cleanup_reject_other_objects_and_recycled_ids() {
    let mut world = world();
    let (mut pending, command) = pending(&world);
    let BuildCmd::RezInventory(rez) = command else { panic!("rez") };
    let update = crate::demo::inventory_rez(&rez);
    let id = update.full_id;
    world.apply(aurora_net::NetEvent::ObjectUpdates {
        handle: rez.handle,
        objects: vec![update],
    });
    let index = world.objects.index_of_uuid(&id).unwrap();
    let object = world.objects.get(index).unwrap();
    let mut props = ObjectProps {
        object_id: id,
        full: true,
        owner_id: world.agent_id,
        item_id: rez.item.id,
        ..Default::default()
    };
    assert!(pending.matches(object, Some(&props)));
    assert!(!pending.matches(object, None));
    props.item_id = Uuid::nil();
    assert!(!pending.matches(object, Some(&props)), "never guess from name or selection");
    props.item_id = rez.item.id;
    props.full = false;
    assert!(!pending.matches(object, Some(&props)));
    pending.root = Some((id, object.key));
    let cleanup = pending.cleanup(&world).expect("only correlated copy");
    let BuildCmd::DeRez {
        local_ids, destination, ..
    } = cleanup
    else {
        panic!("cleanup")
    };
    assert_eq!(local_ids, vec![object.key.local_id]);
    assert_eq!(destination, aurora_net::build::derez::TRASH);
    pending.root.as_mut().unwrap().1.local_id += 1;
    assert!(pending.cleanup(&world).is_none());
    pending.root = Some((id, object.key));
    pending.known.insert(id);
    assert!(pending.cleanup(&world).is_none(), "existing objects must never be deleted");
}

#[test]
fn inventory_model_snapshot_round_trips_bento_geometry_material_textures_and_reloads_after_restart() {
    let avatars = AvatarLibrary::load();
    let directory = std::env::temp_dir().join(format!("aurora-animation-snapshot-{}", Uuid::new_v4()));
    let path = directory.join("model.apmodel");
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).unwrap();
    let scene = aurora_assets::import_mesh::collada::parse(demo::tail_dae().as_bytes(), &skeleton).unwrap();
    let source = super::super::animation_model::prepare(path.clone(), mesh::Draft::new(scene, HashMap::new()), &avatars).unwrap();
    snapshot::save(&source, &avatars).expect("atomic snapshot");
    let loaded = super::super::animation_model::load(path.clone(), 768, &avatars).expect("offline cache reload");
    assert_eq!(loaded.base, source.base);
    assert_eq!(loaded.geometry.parts.len(), source.geometry.parts.len());
    for (original, loaded) in source.geometry.parts.iter().zip(&loaded.geometry.parts) {
        assert_eq!(original.vertices.len(), loaded.vertices.len());
        assert_eq!(original.indices, loaded.indices);
        assert!(original.material == loaded.material);
        assert_eq!(original.skin.as_ref().unwrap().joints, loaded.skin.as_ref().unwrap().joints);
        for (a, b) in original.vertices.iter().zip(loaded.vertices.iter()) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.weights, b.weights);
        }
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(snapshot::decode(path.clone(), &bytes[..bytes.len() / 2], &avatars).is_err());
    assert!(snapshot::decode(path.clone(), b"invalid snapshot", &avatars).is_err());
    let animation = demo::tail_animation();
    let entry = Entry::new(
        "test.anim".into(),
        Ok(Source::Animation {
            bytes: Arc::new(Vec::new()),
            animation: Arc::new(animation),
            bvh: None,
        }),
        Default::default(),
    );
    let entry_id = entry.id;
    let saved = Saved {
        id: Uuid::new_v4(),
        name: "Inventory tail".into(),
        path: path.clone(),
        inventory: Some(Origin {
            profile: "test-account-grid".into(),
            item: Uuid::new_v4(),
        }),
    };
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    imports.complete_inventory_model(entry_id, 0, saved.clone(), Ok(source));
    assert_eq!(imports.load_preferences.animation_models, vec![saved.clone()]);
    let restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&imports.load_preferences).unwrap()).unwrap();
    assert_eq!(restored.animation_models, vec![saved]);
    assert!(
        imports.animation_inventory.pending.is_none(),
        "later library selections have no rez state"
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn inventory_model_decodes_the_copied_linkset_without_wearing_it() {
    let mut world = world();
    let (_, command) = pending(&world);
    let BuildCmd::RezInventory(mut rez) = command else { panic!("rez") };
    rez.item.id = DEMO_ITEM;
    let object = crate::demo::inventory_rez(&rez);
    let id = object.full_id;
    world.apply(aurora_net::NetEvent::ObjectUpdates {
        handle: rez.handle,
        objects: vec![object],
    });
    let mut scene = crate::scene::Scene::new(PathBuf::new(), world.avatar_lib.clone());
    let mut wanted = HashSet::new();
    assert!(
        geometry::capture(&world, &mut scene, id, &mut wanted).unwrap().is_none(),
        "wait for mesh data"
    );
    let mesh = demo_asset().expect("synthetic SL mesh asset");
    scene.meshes.insert_local(mesh.id, mesh.data, mesh.header, mesh.skin);
    let input = geometry::capture(&world, &mut scene, id, &mut wanted)
        .expect("linkset")
        .expect("mesh ready");
    let model = input
        .prepare("synthetic.apmodel".into(), &world.avatar_lib)
        .expect("visual snapshot");
    assert!(!model.geometry.parts.is_empty());
    assert!(model.geometry.parts.iter().all(|part| {
        part.skin
            .as_ref()
            .is_some_and(|skin| skin.joints.contains(&world.avatar_lib.rig.names["mTail1"]))
    }));
    let object = world.objects.get(world.objects.index_of_uuid(&id).unwrap()).unwrap();
    assert_eq!(object.parent_id, 0, "never attached to the avatar");
    assert!(world.inventory.items.contains_key(&Uuid::from_u128(8100)), "original preserved");
}
