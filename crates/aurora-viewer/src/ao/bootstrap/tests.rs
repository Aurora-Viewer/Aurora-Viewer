use super::*;
use crate::ao::{Cycle, State, Track, demo};
use crate::world::inventory::{FetchState, Folder, actions};
use aurora_net::inventory::{InvFolder, InvItem, operations::Operation};

fn source() -> (Inventory, Uuid, Uuid) {
    let mut inv = Inventory::default();
    let owner = Uuid::from_u128(1);
    let selected = demo::seed_firestorm(&mut inv, owner);
    (inv, owner, selected)
}

fn firestorm_snapshot(inv: &Inventory) -> std::collections::BTreeMap<Uuid, String> {
    let fs = inventory::child(inv, inv.root, "#Firestorm").expect("source");
    inv.folders
        .iter()
        .filter(|(id, _)| actions::under(inv, **id, fs))
        .map(|(id, f)| (*id, format!("{:?}|{:?}|{:?}|{}", f.info, f.children, f.items, f.library)))
        .chain(
            inv.items
                .iter()
                .filter(|(id, _)| actions::under(inv, **id, fs))
                .map(|(id, i)| (*id, format!("{i:?}"))),
        )
        .collect()
}

fn native(inv: &mut Inventory) -> Catalog {
    let mut catalog = Catalog::default();
    catalog.refresh(inv);
    catalog
}

fn folder(inv: &mut Inventory, parent: Uuid, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    inv.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
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
    inv.generation = inv.generation.wrapping_add(1);
    id
}

#[test]
fn reconstructs_all_sets_groups_tracks_options_and_selection_without_changing_firestorm() {
    let (mut inv, owner, _) = source();
    let mut fs = Catalog::default();
    fs.refresh_firestorm(&mut inv);
    let mut second = fs.sets[0].clone();
    second.name = "Second set".into();
    second.default = false;
    second.mouselook = true;
    let state = second.states.get_mut(&State::Standing).expect("standing");
    state.steps[0].group = true;
    state.steps[0].name = "Together".into();
    let companion = state.steps[1].members[0].clone();
    state.steps[0].members.push(companion);
    state.tracks.push(Track {
        id: Uuid::new_v4(),
        name: "Independent".into(),
        cycle: Cycle {
            cycle: true,
            random: true,
            seconds: 3.0,
        },
        members: state.steps[0].members.clone(),
    });
    let (change, requested) = inventory::create(&inv, &fs, &second).expect("fixture set");
    let reply = actions::demo_mutate(&mut inv, owner, change).expect("fixture creation");
    let selected = reply.created.get(&requested).copied().unwrap_or(requested);
    fs.refresh_firestorm(&mut inv);
    let original = fs.sets.clone();
    let untouched = firestorm_snapshot(&inv);
    let mut startup = Bootstrap::default();
    let mut target = native(&mut inv);
    assert!(target.root.is_none());
    let (change, select) = startup
        .poll(&mut inv, &target, selected, true)
        .expect("fallback")
        .expect("reconstruction");
    assert!(change.copies.is_empty());
    assert!(inventory::editable(&inv, &change, &Default::default()));
    assert!(matches!(change.operations.last(), Some(Operation::Patch { folder: true, body, .. }) if body["name"].as_str() == "AO"));
    let source_folders: Vec<_> = inv.folders.keys().copied().collect();
    assert!(change.operations.iter().all(|op| match op {
        Operation::Folder { .. } | Operation::Links { .. } => true,
        Operation::Patch { id, .. } => !source_folders.contains(id),
        _ => false,
    }));
    let reply = actions::demo_mutate(&mut inv, owner, change).expect("inventory acknowledgement");
    let selected = select.and_then(|id| reply.created.get(&id).copied()).expect("selection remapped");
    target.refresh(&mut inv);
    assert_eq!(target.sets.len(), original.len());
    assert_eq!(target.selected(selected).expect("selected set").name, "Second set");
    for old in &original {
        let new = target.sets.iter().find(|s| s.name == old.name).expect("recreated set");
        assert_ne!(new.id, old.id);
        assert_eq!(
            (new.sits, new.smart, new.mouselook, new.default),
            (old.sits, old.smart, old.mouselook, old.default)
        );
        assert_eq!(new.states.len(), old.states.len());
        for (state, before) in &old.states {
            let after = &new.states[state];
            assert_eq!(after.cycle, before.cycle);
            assert_eq!(after.steps.len(), before.steps.len());
            for (a, b) in after.steps.iter().zip(&before.steps) {
                assert_eq!((a.name.as_str(), a.order, a.group), (b.name.as_str(), b.order, b.group));
                assert_eq!(
                    a.members.iter().map(|m| (m.original, m.asset)).collect::<Vec<_>>(),
                    b.members.iter().map(|m| (m.original, m.asset)).collect::<Vec<_>>()
                );
            }
            assert_eq!(after.tracks.len(), before.tracks.len());
            for (a, b) in after.tracks.iter().zip(&before.tracks) {
                assert_eq!((&a.name, &a.cycle), (&b.name, &b.cycle));
                assert_eq!(
                    a.members.iter().map(|m| (m.original, m.asset)).collect::<Vec<_>>(),
                    b.members.iter().map(|m| (m.original, m.asset)).collect::<Vec<_>>()
                );
            }
        }
    }
    assert_eq!(
        firestorm_snapshot(&inv),
        untouched,
        "all Firestorm metadata and contents survive success"
    );
    fs.refresh_firestorm(&mut inv);
    assert_eq!(fs.sets, original, "Firestorm folders, links and assets stay untouched");
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    assert!(
        Bootstrap::default().poll(&mut inv, &target, selected, true).is_none(),
        "relogging cannot duplicate the import"
    );
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("brand");
    assert_eq!(
        inv.folders[&brand]
            .children
            .iter()
            .filter(|id| inv.folders[id].info.name == "AO")
            .count(),
        1
    );
    assert!(inventory::child(&inv, brand, "AO (Firestorm import)").is_none());
}

#[test]
fn an_existing_aurora_ao_is_authoritative_even_when_empty_and_firestorm_is_populated() {
    let (mut inv, _, selected) = source();
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("brand");
    let root = folder(&mut inv, brand, "AO");
    let target = native(&mut inv);
    assert_eq!(target.root, Some(root));
    assert!(target.sets.is_empty());
    let mut startup = Bootstrap::default();
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    assert!(startup.source.root.is_none(), "no Firestorm catalogue is consulted");
}

#[test]
fn no_source_does_nothing_even_when_old_swim_items_exist() {
    let mut inv = Inventory::default();
    inv.root = folder(&mut inv, Uuid::nil(), "Inventory");
    let root = inv.root;
    inv.add_items(vec![InvItem {
        id: Uuid::new_v4(),
        parent: root,
        name: "Aurora Swim Breaststroke".into(),
        asset_id: Uuid::new_v4(),
        asset_type: 20,
        inv_type: 19,
        ..Default::default()
    }]);
    let before = inv.folders.len();
    let target = native(&mut inv);
    let mut startup = Bootstrap::default();
    assert!(startup.poll(&mut inv, &target, Uuid::nil(), true).is_none());
    assert_eq!(inv.folders.len(), before);
    assert!(startup.missing().is_empty());
}

#[test]
fn waits_for_source_contents_and_original_items_before_reconstructing() {
    let (mut inv, _, selected) = source();
    let fs = inventory::child(&inv, inv.root, "#Firestorm").expect("source");
    let source_root = inventory::child(&inv, fs, "#AO").expect("source root");
    inv.folders.get_mut(&source_root).expect("folder").state = FetchState::Unknown;
    let target = native(&mut inv);
    let mut startup = Bootstrap::default();
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    assert!(startup.source.loading);
    inv.folders.get_mut(&source_root).expect("folder").state = FetchState::Fetched;
    let original = inv.items.values().find(|i| i.asset_type == 20).expect("original").id;
    let item = inv.items.remove(&original).expect("item");
    inv.generation = inv.generation.wrapping_add(1);
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    assert!(startup.missing().contains(&original));
    inv.add_items(vec![item]);
    assert!(startup.poll(&mut inv, &target, selected, true).is_some_and(|result| result.is_ok()));
}

#[test]
fn interrupted_imports_are_not_published_or_blindly_replayed() {
    let (mut inv, owner, selected) = source();
    let untouched = firestorm_snapshot(&inv);
    let target = native(&mut inv);
    let (mut change, _) = Bootstrap::default()
        .poll(&mut inv, &target, selected, true)
        .expect("fallback")
        .expect("plan");
    change.operations.truncate(3);
    change.refresh = vec![inventory::child(&inv, inv.root, "#Aurora").expect("brand")];
    actions::demo_mutate(&mut inv, owner, change).expect("simulate acknowledged prefix before network failure");
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("brand");
    assert!(inventory::child(&inv, brand, "AO").is_none());
    assert!(inventory::child(&inv, brand, "AO (Firestorm import)").is_some());
    assert_eq!(
        firestorm_snapshot(&inv),
        untouched,
        "interrupted import cannot modify Firestorm either"
    );
    let target = native(&mut inv);
    let mut startup = Bootstrap::default();
    assert!(startup.poll(&mut inv, &target, selected, true).is_some_and(|r| r.is_err()));
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
}

#[test]
fn waits_for_the_bridge_owned_shared_parent_instead_of_creating_a_duplicate() {
    let (mut inv, _, selected) = source();
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("brand");
    inv.folders.remove(&brand);
    let target = native(&mut inv);
    let mut startup = Bootstrap::default();
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    let root = inv.root;
    let brand = folder(&mut inv, root, "#Aurora");
    let target = native(&mut inv);
    let (change, _) = startup
        .poll(&mut inv, &target, selected, true)
        .expect("parent acknowledged")
        .expect("plan");
    assert!(
        change
            .operations
            .iter()
            .all(|op| !matches!(op,Operation::Folder{name,..} if name=="#Aurora"))
    );
    assert!(matches!(change.operations.first(),Some(Operation::Folder{parent,..}) if *parent==brand));
}

#[test]
fn firestorm_swimming_is_never_imported_or_required_to_finish_reading_the_source() {
    let (mut inv, owner, selected) = source();
    let fs = inventory::child(&inv, inv.root, "#Firestorm").expect("source");
    let ao = inventory::child(&inv, fs, "#AO").expect("source AO");
    let mut excluded = Vec::new();
    for set in inv.folders[&ao].children.clone() {
        for id in inv.folders[&set].children.clone() {
            let f = inv.folders.get_mut(&id).expect("state");
            if State::parse(f.info.name.split(':').next().expect("name")).is_some_and(State::swimming) {
                f.state = FetchState::Unknown;
                excluded.push(id);
            }
        }
    }
    assert!(!excluded.is_empty());
    let untouched = firestorm_snapshot(&inv);
    let target = native(&mut inv);
    let (change, _) = Bootstrap::default()
        .poll(&mut inv, &target, selected, true)
        .expect("fallback")
        .expect("no swimming fetch needed");
    assert!(excluded.iter().all(|id| !inv.queue.iter().any(|(queued, _)| queued == id)));
    actions::demo_mutate(&mut inv, owner, change).expect("reconstruct");
    let target = native(&mut inv);
    assert!(
        target
            .sets
            .iter()
            .all(|set| set.swimming_slots() == 0 && set.states.keys().all(|state| !state.swimming()))
    );
    assert_eq!(firestorm_snapshot(&inv), untouched);
}

#[test]
fn ao_edits_reject_every_firestorm_write_but_can_link_its_original_animations() {
    use aurora_llsd::llsd_map;
    use aurora_net::inventory::operations::{CopyItem, Mutation};
    use aurora_net::outfits::OutfitLink;
    let (mut inv, _, _) = source();
    let fs = inventory::child(&inv, inv.root, "#Firestorm").expect("source");
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("target");
    let original = inv.items.values().find(|i| i.asset_type == 20).expect("animation").id;
    let item = inv.items.get_mut(&original).expect("original");
    item.parent = fs;
    let link = OutfitLink {
        target: original,
        name: "Read-only original".into(),
        desc: String::new(),
        folder: false,
        inv_type: 19,
    };
    let allowed = Mutation {
        operations: vec![Operation::Links {
            parent: brand,
            links: vec![link.clone()],
        }],
        ..Default::default()
    };
    assert!(inventory::editable(&inv, &allowed, &Default::default()));
    for op in [
        Operation::Folder {
            id: Uuid::new_v4(),
            parent: fs,
            name: "Forbidden".into(),
            type_default: -1,
        },
        Operation::Links {
            parent: fs,
            links: vec![link],
        },
        Operation::Patch {
            id: original,
            folder: false,
            body: llsd_map! { "name" => "Forbidden" },
        },
        Operation::Patch {
            id: brand,
            folder: true,
            body: llsd_map! { "parent_id" => fs },
        },
        Operation::Delete {
            id: original,
            folder: false,
        },
        Operation::Purge(fs),
    ] {
        assert!(!inventory::editable(
            &inv,
            &Mutation {
                operations: vec![op],
                ..Default::default()
            },
            &Default::default()
        ));
    }
    assert!(!inventory::editable(
        &inv,
        &Mutation {
            copies: vec![CopyItem {
                item: original,
                owner: Uuid::nil(),
                parent: fs,
                name: "Forbidden".into()
            }],
            ..Default::default()
        },
        &Default::default()
    ));
}

#[test]
fn cached_firestorm_set_waits_for_inventory_services_and_reconstructs_once_when_they_arrive() {
    let mut inv = Inventory::default();
    let owner = Uuid::from_u128(1);
    let selected = demo::seed_firestorm_wait(&mut inv, owner);
    let untouched = firestorm_snapshot(&inv);
    let mut target = native(&mut inv);
    let generation = inv.generation;
    let mut startup = Bootstrap::default();
    for _ in 0..100 {
        assert!(startup.poll(&mut inv, &target, selected, false).is_none());
        assert!(
            !startup.finished,
            "cached data must not consume the startup attempt before writes are available"
        );
        assert!(startup.loading());
        assert_eq!(inv.generation, generation, "waiting performs no inventory mutation");
        assert!(target.root.is_none());
    }
    let (change, select) = startup
        .poll(&mut inv, &target, selected, true)
        .expect("capabilities arrived")
        .expect("reconstruction");
    assert!(inventory::editable(&inv, &change, &Default::default()));
    let reply = actions::demo_mutate(&mut inv, owner, change).expect("acknowledgement");
    let selected = select.and_then(|id| reply.created.get(&id).copied()).expect("native selection");
    target.refresh(&mut inv);
    let set = target.selected(selected).expect("native AO");
    assert_eq!(set.name, "MonSet");
    let cycle = &set.states[&State::Standing].cycle;
    assert!(cycle.cycle && cycle.random);
    assert_eq!(cycle.seconds, 15.0);
    assert!(set.states.keys().all(|state| !state.swimming()));
    assert_eq!(set.swimming_slots(), 0);
    assert_eq!(firestorm_snapshot(&inv), untouched);
    assert!(!startup.loading());
    for _ in 0..100 {
        assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    }
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("target parent");
    assert_eq!(
        inv.folders[&brand]
            .children
            .iter()
            .filter(|id| inv.folders[id].info.name == "AO")
            .count(),
        1
    );
}

#[test]
fn startup_write_gate_requires_both_services_arrival_and_a_released_cache_hold() {
    use crate::world::World;
    use std::sync::Arc;
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.movement_complete = true;
    world.teleporting = false;
    demo::inventory_services(&mut world, false);
    assert!(!Bootstrap::writable(&world));
    let region = world.main_region.expect("region");
    Arc::make_mut(&mut world.regions.get_mut(&region).expect("region").caps)
        .insert("InventoryAPIv3".into(), "https://inventory.invalid/offline".into());
    assert!(!Bootstrap::writable(&world), "readback capability is needed too");
    demo::inventory_services(&mut world, true);
    assert!(Bootstrap::writable(&world));
    Arc::make_mut(&mut world.regions.get_mut(&region).expect("region").caps).remove("InventoryAPIv3");
    assert!(!Bootstrap::writable(&world), "readback alone cannot write");
    demo::inventory_services(&mut world, true);
    world.inventory.hold_requests();
    assert!(!Bootstrap::writable(&world));
    world.inventory.release_requests();
    world.movement_complete = false;
    assert!(!Bootstrap::writable(&world));
    world.movement_complete = true;
    world.teleporting = true;
    assert!(!Bootstrap::writable(&world));
    world.teleporting = false;
    assert!(Bootstrap::writable(&world));
}

#[test]
fn a_native_ao_arriving_while_services_are_delayed_cancels_fallback_without_touching_firestorm() {
    let (mut inv, _, selected) = source();
    let untouched = firestorm_snapshot(&inv);
    let mut target = native(&mut inv);
    let mut startup = Bootstrap::default();
    assert!(startup.poll(&mut inv, &target, selected, false).is_none());
    assert!(startup.loading());
    let brand = inventory::child(&inv, inv.root, "#Aurora").expect("brand");
    let native_root = folder(&mut inv, brand, "AO");
    target.refresh(&mut inv);
    assert!(startup.poll(&mut inv, &target, selected, true).is_none());
    assert!(startup.finished && !startup.loading());
    assert_eq!(target.root, Some(native_root));
    assert!(target.sets.is_empty());
    assert_eq!(firestorm_snapshot(&inv), untouched);
}
