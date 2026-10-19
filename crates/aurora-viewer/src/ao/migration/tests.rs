use super::*;
use crate::ao::{State, bootstrap::Bootstrap, demo};
use crate::world::inventory::actions;
use std::collections::BTreeMap;

fn snapshot(inv: &Inventory, root: Uuid) -> BTreeMap<Uuid, String> {
    inv.folders
        .iter()
        .filter(|(id, _)| actions::under(inv, **id, root))
        .map(|(id, f)| (*id, format!("{:?}|{:?}|{:?}", f.info, f.children, f.items)))
        .chain(
            inv.items
                .iter()
                .filter(|(id, _)| actions::under(inv, **id, root))
                .map(|(id, item)| (*id, format!("{item:?}"))),
        )
        .collect()
}

fn fixture() -> (Inventory, Catalog, Migration, Uuid, Uuid) {
    let owner = Uuid::from_u128(1);
    let mut inv = Inventory::default();
    let selected = demo::seed_migration(&mut inv, owner);
    let mut native = Catalog::default();
    native.refresh(&mut inv);
    let mut migration = Migration::default();
    migration.refresh(&mut inv, true);
    (inv, native, migration, owner, selected)
}

#[test]
fn manual_migration_adds_only_the_chosen_set_and_never_restarts_automatic_copying() {
    let (mut inv, mut native, mut migration, owner, selected) = fixture();
    let fs = inventory::child(&inv, inv.root, "#Firestorm").expect("source");
    let untouched_fs = snapshot(&inv, fs);
    let untouched_native = snapshot(&inv, selected);
    let old = migration
        .source
        .sets
        .iter()
        .find(|s| s.name == "Firestorm Walk Set")
        .expect("choice")
        .clone();
    migration.select(old.id);
    let (plan, id) = migration.plan(&inv, &native).expect("explicit plan");
    assert!(inventory::editable(&inv, &plan, &Default::default()));
    assert_eq!(native.sets.len(), 1, "planning never changes selection or inventory");
    let reply = actions::demo_mutate(&mut inv, owner, plan).expect("acknowledgement");
    let id = reply.created.get(&id).copied().unwrap_or(id);
    native.invalidate();
    native.refresh(&mut inv);
    assert_eq!(native.sets.len(), 2);
    let new = native.sets.iter().find(|s| s.id == id).expect("new set");
    assert_ne!(new.id, old.id);
    assert_eq!(new.name, old.name);
    assert_eq!((new.sits, new.smart, new.mouselook), (old.sits, old.smart, old.mouselook));
    assert!(!new.default, "the native default is not replaced");
    assert!(native.sets.iter().any(|s| s.id == selected && s.default));
    assert_eq!(new.swimming_slots(), State::SWIMMING.len());
    assert!(
        State::LEGACY_VIEWER_SWIMMING
            .into_iter()
            .all(|state| new.swimming_animation(state).is_some())
    );
    assert!(new.swimming_ready());
    for (target, donor) in [
        (State::UnderwaterFloating, State::Floating),
        (State::UnderwaterSwimming, State::SwimmingForward),
        (State::WaterFlightExit, State::FlyingUp),
        (State::WaterShoreExit, State::Jumping),
        (State::WaterDiveJump, State::Flying),
        (State::WaterDiveEntry, State::SwimmingDown),
        (State::WaterCannonball, State::Crouching),
        (State::WaterEntry, State::Falling),
    ] {
        assert_eq!(new.states[&target].cycle, old.states[&donor].cycle);
        assert_eq!(
            new.swimming_animation(target).expect("completed").original,
            old.states[&donor].steps[0].members[0].original
        );
        assert_ne!(new.states[&target].folder, old.states[&donor].folder);
    }
    for (state, config) in &old.states {
        let after = &new.states[state];
        assert_eq!(after.cycle, config.cycle);
        assert_eq!(
            after
                .steps
                .iter()
                .flat_map(|s| &s.members)
                .map(|a| (a.original, a.asset))
                .collect::<Vec<_>>(),
            config
                .steps
                .iter()
                .flat_map(|s| &s.members)
                .map(|a| (a.original, a.asset))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(snapshot(&inv, selected), untouched_native);
    assert_eq!(snapshot(&inv, fs), untouched_fs);
    for _ in 0..3 {
        assert!(Bootstrap::default().poll(&mut inv, &native, selected, true).is_none());
    }
    assert_eq!(snapshot(&inv, fs), untouched_fs);
    assert_eq!(native.sets.len(), 2);
}

#[test]
fn missing_flight_ascent_keeps_the_migrated_swimming_gate_locked() {
    let (mut inv, mut native, mut migration, owner, _) = fixture();
    let source = migration.selected;
    let folder = migration.source.sets.iter().find(|s| s.id == source).expect("source").states[&State::FlyingUp].folder;
    inv.folders
        .get_mut(&source)
        .expect("source set")
        .children
        .retain(|id| *id != folder);
    inv.generation = inv.generation.wrapping_add(1);
    migration.refresh(&mut inv, true);
    migration.name = "Partial migration".into();
    let (change, id) = migration.plan(&inv, &native).expect("partial sets can migrate");
    let reply = actions::demo_mutate(&mut inv, owner, change).expect("acknowledgement");
    let id = reply.created.get(&id).copied().unwrap_or(id);
    native.invalidate();
    native.refresh(&mut inv);
    let set = native.selected(id).expect("migrated");
    assert_eq!(set.core_swimming_slots(), 7);
    assert!(set.swimming_animation(State::SwimmingUp).is_some());
    assert!(set.swimming_animation(State::WaterFlightExit).is_none());
    assert!(!native.swimming_ready(id));
}

#[test]
fn manual_migration_requires_a_distinct_valid_name_and_ready_services() {
    let (mut inv, native, mut migration, _, _) = fixture();
    let generation = inv.generation;
    assert_eq!(migration.name, "Aurora AO");
    assert!(migration.plan(&inv, &native).is_err(), "a collision cannot overwrite native data");
    for name in ["", "Bad:name", "  Aurora AO  "] {
        migration.name = name.into();
        assert!(migration.plan(&inv, &native).is_err());
    }
    migration.name = "Manual copy".into();
    migration.refresh(&mut inv, false);
    assert!(migration.plan(&inv, &native).is_err());
    migration.refresh(&mut inv, true);
    assert!(migration.plan(&inv, &native).is_ok());
    assert_eq!(inv.generation, generation, "opening, renaming and cancelling are read-only");
}

#[test]
fn a_missing_selected_source_never_falls_back_to_a_different_firestorm_set() {
    let (mut inv, native, mut migration, _, _) = fixture();
    let selected = migration.selected;
    let root = migration.source.root.expect("source root");
    inv.folders.get_mut(&root).expect("root").children.retain(|id| *id != selected);
    inv.generation = inv.generation.wrapping_add(1);
    migration.refresh(&mut inv, true);
    migration.name = "Chosen copy".into();
    assert_eq!(migration.source.sets.len(), 1);
    assert!(migration.plan(&inv, &native).is_err());
    assert_eq!(migration.selected, selected);
}

#[test]
fn unresolved_source_originals_block_manual_copy_until_fetched() {
    for state in [State::Standing, State::SwimmingForward] {
        let (mut inv, native, mut migration, _, _) = fixture();
        let item = migration.source.sets[0].states[&state].steps[0].members[0].original;
        let original = inv.items.remove(&item).expect("original");
        inv.generation = inv.generation.wrapping_add(1);
        migration.refresh(&mut inv, true);
        migration.name = "Complete copy".into();
        assert!(migration.source.missing.contains(&item));
        assert!(migration.plan(&inv, &native).is_err());
        inv.add_items(vec![original]);
        migration.refresh(&mut inv, true);
        assert!(migration.source.missing.is_empty());
        assert!(migration.plan(&inv, &native).is_ok());
    }
}
