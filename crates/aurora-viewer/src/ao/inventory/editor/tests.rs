use super::*;

fn fixture() -> (Inventory, Set, Uuid) {
    let owner = Uuid::from_u128(1);
    let mut inv = Inventory::default();
    let id = crate::ao::demo::seed(&mut inv, owner);
    let mut catalog = Catalog::default();
    catalog.refresh(&mut inv);
    (inv, catalog.selected(id).expect("set").clone(), owner)
}

fn apply(inv: &mut Inventory, owner: Uuid, change: Mutation) {
    assert!(editable(inv, &change, &HashSet::new()));
    crate::world::inventory::actions::demo_mutate(inv, owner, change).expect("acknowledged edit");
}

fn reload(inv: &mut Inventory, id: Uuid) -> Set {
    let mut catalog = Catalog::default();
    catalog.refresh(inv);
    catalog.selected(id).expect("set").clone()
}

fn originals(inv: &Inventory) -> Vec<(Uuid, Uuid, String, Uuid)> {
    let mut originals: Vec<_> = inv
        .items
        .values()
        .filter(|a| a.asset_type == 20)
        .map(|a| (a.id, a.parent, a.name.clone(), a.asset_id))
        .collect();
    originals.sort();
    originals
}

#[test]
fn guided_group_and_dissociation_preserve_originals_and_main_cycle() {
    let (mut inv, set, owner) = fixture();
    let before = originals(&inv);
    let c = &set.states[&State::Standing];
    let chosen: HashSet<_> = c.steps.iter().map(|s| s.id).collect();
    let (change, _) = combine(&inv, &set, State::Standing, "Body and hands", false, &chosen).expect("combine");
    let links = change
        .operations
        .iter()
        .position(|op| matches!(op, Operation::Links { .. }))
        .expect("destination links");
    let deletion = change
        .operations
        .iter()
        .position(|op| matches!(op, Operation::Delete { .. }))
        .expect("old links");
    assert!(
        links < deletion,
        "create acknowledged destination links before deleting old AO links"
    );
    apply(&mut inv, owner, change);
    let combined = reload(&mut inv, set.id);
    let group = &combined.states[&State::Standing].steps[0];
    assert!(group.group);
    assert_eq!(group.members.len(), 2);
    assert_eq!(combined.states[&State::Standing].steps.len(), 1);
    assert_eq!(originals(&inv), before);
    let change = dissolve(&inv, &combined, State::Standing, group.id).expect("dissolve");
    apply(&mut inv, owner, change);
    let restored = reload(&mut inv, set.id);
    let c = &restored.states[&State::Standing];
    assert_eq!(c.steps.len(), 2);
    assert!(c.steps.iter().all(|s| !s.group));
    assert_eq!(c.cycle, set.states[&State::Standing].cycle);
    assert_eq!(originals(&inv), before);
}

#[test]
fn member_drag_order_survives_reload_and_ungrouping() {
    let (mut inv, set, owner) = fixture();
    let selected = set.states[&State::Standing].steps.iter().map(|s| s.id).collect();
    let (change, _) = combine(&inv, &set, State::Standing, "Together", false, &selected).expect("group");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    let group = &set.states[&State::Standing].steps[0];
    let change = reorder(
        &inv,
        &set,
        State::Standing,
        List::Members(group.id),
        group.members[1].link,
        group.members[0].link,
        false,
    )
    .expect("reorder");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    let group = &set.states[&State::Standing].steps[0];
    assert_eq!(group.members[0].name, "Aurora Stand B");
    let change = dissolve(&inv, &set, State::Standing, group.id).expect("dissolve");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    assert_eq!(set.states[&State::Standing].steps[0].name, "Aurora Stand B");
}

#[test]
fn tracks_keep_drag_order_and_own_cycles_after_rename_and_save() {
    let (mut inv, set, owner) = fixture();
    for name in ["A track", "Z track"] {
        let set = reload(&mut inv, set.id);
        let (change, _) = combine(&inv, &set, State::Standing, name, true, &HashSet::new()).expect("track");
        apply(&mut inv, owner, change);
    }
    let set = reload(&mut inv, set.id);
    let c = &set.states[&State::Standing];
    let change = reorder(&inv, &set, State::Standing, List::Tracks, c.tracks[1].id, c.tracks[0].id, false).expect("track order");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    assert_eq!(set.states[&State::Standing].tracks[0].name, "Z track");
    let mut track = set.states[&State::Standing].tracks[0].clone();
    track.cycle.seconds = 12.0;
    let change = save_track(&inv, &track).expect("cycle");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    let change = rename_container(&inv, &set, State::Standing, track.id, "Renamed").expect("rename");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    let c = &set.states[&State::Standing];
    assert_eq!(c.tracks[0].name, "Renamed");
    assert_eq!(c.tracks[0].cycle.seconds, 12.0);
    assert_eq!(c.cycle.seconds, 6.0);
    assert!(rename_container(&inv, &set, State::Standing, track.id, "A track").is_err());
}

#[test]
fn main_drag_order_moves_in_both_directions_and_rejects_other_lists() {
    let (mut inv, set, owner) = fixture();
    let c = &set.states[&State::Standing];
    let (a, b) = (c.steps[0].id, c.steps[1].id);
    let change = reorder(&inv, &set, State::Standing, List::Main, b, a, false).expect("up");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    assert_eq!(set.states[&State::Standing].steps[0].id, b);
    let change = reorder(&inv, &set, State::Standing, List::Main, b, a, true).expect("down");
    apply(&mut inv, owner, change);
    let set = reload(&mut inv, set.id);
    assert_eq!(set.states[&State::Standing].steps[0].id, a);
    assert!(
        reorder(&inv, &set, State::Standing, List::Main, a, a, false)
            .expect("no change")
            .operations
            .is_empty()
    );
    assert!(reorder(&inv, &set, State::Standing, List::Tracks, a, b, false).is_err());
    let walk = set.states[&State::Walking].steps[0].id;
    assert!(reorder(&inv, &set, State::Standing, List::Main, a, walk, false).is_err());
}

#[test]
fn stale_selection_and_unloaded_sources_never_plan_edits() {
    let (mut inv, set, _) = fixture();
    assert!(combine(&inv, &set, State::Standing, "Bad:name", false, &HashSet::new()).is_err());
    assert!(combine(&inv, &set, State::Standing, "Group", false, &HashSet::from([Uuid::new_v4()])).is_err());
    assert!(dissolve(&inv, &set, State::Standing, Uuid::new_v4()).is_err());
    let folder = set.states[&State::Standing].folder;
    inv.folders.get_mut(&folder).expect("state").state = FetchState::Unknown;
    assert!(combine(&inv, &set, State::Standing, "Group", false, &HashSet::new()).is_err());
}
