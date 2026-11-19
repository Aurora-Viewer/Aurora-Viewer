use super::tests_support::{Ids, fixture};
use super::*;
use crate::world::inventory::actions::{create_folder, demo_mutate, move_selection, paste, patch};

fn none() -> HashSet<Uuid> {
    HashSet::new()
}

/// Sends `change` as the user's own, as the app does: drafted before it
/// leaves, kept once the (demo) server accepted it.
fn user(history: &mut History, inv: &mut Inventory, ids: &Ids, change: Mutation) {
    let request = Uuid::new_v4();
    if let Some(draft) = draft(inv, &change) {
        history.record(request, draft);
    }
    let reply = demo_mutate(inv, ids.agent, change).expect("accepted");
    history.finish(request, inv, true, &reply.created);
}

/// Undo (or redo) through the history, accepted by the server.
fn step(history: &mut History, inv: &mut Inventory, ids: &Ids, undo: bool, worn: &HashSet<Uuid>) -> Result<(), String> {
    let protected = none();
    let rules_of = Rules {
        protected: &protected,
        worn,
    };
    let (change, effective) = history.plan(inv, undo, &rules_of)?;
    let request = Uuid::new_v4();
    history.begin(request, undo, effective);
    let reply = demo_mutate(inv, ids.agent, change).expect("accepted");
    history.finish(request, inv, true, &reply.created);
    Ok(())
}

fn parent(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    rules::parent(inv, id)
}

#[test]
fn a_move_goes_back_and_comes_again() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let change = move_selection(&inv, &[ids.shirt, ids.hat], ids.objects, &none(), &none()).expect("move");
    user(&mut history, &mut inv, &ids, change);
    assert_eq!(parent(&inv, ids.shirt), Some(ids.objects));
    assert!(matches!(history.next_undo(), Some(Step::Move { moves, trash: false }) if moves.len() == 2));
    assert!(!history.next_undo().expect("step").what().is_empty());

    step(&mut history, &mut inv, &ids, true, &none()).expect("undo");
    assert_eq!(parent(&inv, ids.shirt), Some(ids.clothes));
    assert_eq!(parent(&inv, ids.hat), Some(ids.clothes));
    assert!(history.next_undo().is_none());
    assert!(history.next_redo().is_some());

    step(&mut history, &mut inv, &ids, false, &none()).expect("redo");
    assert_eq!(parent(&inv, ids.shirt), Some(ids.objects));
    assert!(history.next_undo().is_some() && history.next_redo().is_none());
}

#[test]
fn a_delete_comes_back_from_the_trash_and_its_lost_folder_falls_back_to_restore() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let change = move_selection(&inv, &[ids.shirt], ids.trash, &none(), &none()).expect("delete");
    user(&mut history, &mut inv, &ids, change);
    assert!(matches!(history.next_undo(), Some(Step::Move { trash: true, .. })));
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo");
    assert_eq!(parent(&inv, ids.shirt), Some(ids.clothes));

    // its folder is gone (purged from the Trash meanwhile): the Legacy
    // Viewer's Restore place, the type's folder or the root
    step(&mut history, &mut inv, &ids, false, &none()).expect("redo");
    assert_eq!(parent(&inv, ids.shirt), Some(ids.trash));
    let gone = Mutation {
        operations: vec![Operation::Delete {
            id: ids.clothes,
            folder: true,
        }],
        refresh: vec![inv.root],
        ..Default::default()
    };
    demo_mutate(&mut inv, ids.agent, gone).expect("gone");
    inv.folders.remove(&ids.clothes);
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo to Restore's place");
    assert_eq!(parent(&inv, ids.shirt), Some(inv.root));
}

#[test]
fn a_rename_is_taken_back_only_while_the_name_is_the_steps() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let change = patch(&inv, ids.shirt, llsd_map! { "name" => "Blue shirt" }).expect("rename");
    user(&mut history, &mut inv, &ids, change);
    assert!(matches!(history.next_undo(), Some(Step::Rename { .. })));
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo");
    assert_eq!(inv.items.get(&ids.shirt).expect("shirt").name, "Shirt");
    step(&mut history, &mut inv, &ids, false, &none()).expect("redo");
    assert_eq!(inv.items.get(&ids.shirt).expect("shirt").name, "Blue shirt");

    // renamed again from elsewhere (not through the history): the step is
    // stale and dropped, nothing is renamed
    inv.items.get_mut(&ids.shirt).expect("shirt").name = "Red shirt".into();
    let protected = none();
    let worn = none();
    let rules_of = Rules {
        protected: &protected,
        worn: &worn,
    };
    assert!(history.plan(&inv, true, &rules_of).is_err());
    assert!(history.next_undo().is_none(), "dropped");
    // a rename to the same name is no step
    let same = patch(&inv, ids.shirt, llsd_map! { "name" => "Red shirt" }).expect("same");
    assert!(draft(&inv, &same).is_none());
}

#[test]
fn a_new_folder_goes_to_the_trash_and_comes_back_with_its_id() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let change = create_folder(ids.objects, "New folder").expect("folder");
    let Operation::Folder { id: new, .. } = change.operations[0] else {
        panic!("a folder");
    };
    user(&mut history, &mut inv, &ids, change);
    assert!(matches!(history.next_undo(), Some(Step::Created { made: Made::Folder, ids: made, .. }) if made == &[new]));
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo");
    assert_eq!(parent(&inv, new), Some(ids.trash), "never purged");
    step(&mut history, &mut inv, &ids, false, &none()).expect("redo");
    assert_eq!(parent(&inv, new), Some(ids.objects));
}

#[test]
fn pasted_links_and_copies_are_found_by_their_target_and_name() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    // links
    let links = paste(&inv, &[ids.shirt], ids.objects, false, true, &none(), &none()).expect("links");
    user(&mut history, &mut inv, &ids, links);
    let link = inv
        .items
        .values()
        .find(|it| it.asset_type == 24 && it.asset_id == ids.shirt)
        .map(|it| it.id)
        .expect("link");
    assert!(matches!(history.next_undo(), Some(Step::Created { made: Made::Link, ids: made, .. }) if made == &[link]));
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo");
    assert_eq!(parent(&inv, link), Some(ids.trash));

    // copies: the items arrive with the folder's contents (the server's
    // copy callbacks; demo_mutate leaves them to the caller)
    let copy = paste(&inv, &[ids.hat], ids.objects, false, false, &none(), &none()).expect("copy");
    assert_eq!(copy.copies.len(), 1);
    let request = Uuid::new_v4();
    history.record(request, draft(&inv, &copy).expect("draft"));
    let copied = Uuid::from_u128(77);
    let mut it = inv.items.get(&ids.hat).cloned().expect("hat");
    it.id = copied;
    it.parent = ids.objects;
    // an unrelated item that arrives meanwhile is not the copy's
    let mut other = it.clone();
    other.id = Uuid::from_u128(78);
    other.name = "Gift".into();
    inv.add_items(vec![it, other]);
    history.finish(request, &inv, true, &HashMap::new());
    assert!(matches!(history.next_undo(), Some(Step::Created { made: Made::Copy, ids: made, .. }) if made == &[copied]));
    assert!(history.next_redo().is_none(), "a new step ends the redo");
}

#[test]
fn the_history_keeps_fifty_steps() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    for n in 0..(LIMIT + 5) {
        let change = patch(&inv, ids.hat, llsd_map! { "name" => format!("Hat {n}") }).expect("rename");
        user(&mut history, &mut inv, &ids, change);
    }
    assert_eq!(history.undo.len(), LIMIT);
    assert!(
        matches!(history.undo.front(), Some(Step::Rename { old, .. }) if old == "Hat 4"),
        "the oldest went"
    );
}

#[test]
fn a_refusal_keeps_the_step_and_a_refused_server_answer_records_nothing() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    // a copy then worn: its undo would take a worn item off to the Trash
    let change = create_folder(ids.objects, "Box").expect("folder");
    let Operation::Folder { id: new, .. } = change.operations[0] else {
        panic!("a folder");
    };
    user(&mut history, &mut inv, &ids, change);
    let worn: HashSet<Uuid> = [new].into();
    assert!(step(&mut history, &mut inv, &ids, true, &worn).is_err());
    assert!(history.next_undo().is_some(), "kept for later");
    assert_eq!(parent(&inv, new), Some(ids.objects));

    // a no-modify item's rename back is refused by the rules
    history.push(Step::Rename {
        id: ids.sword,
        old: "Old".into(),
        new: "Sword".into(),
    });
    assert!(step(&mut history, &mut inv, &ids, true, &none()).is_err());
    assert!(matches!(history.next_undo(), Some(Step::Rename { .. })));

    // the server refuses the user's change: no step
    let mut history = History::default();
    let change = move_selection(&inv, &[ids.hat], ids.objects, &none(), &none()).expect("move");
    let request = Uuid::new_v4();
    history.record(request, draft(&inv, &change).expect("draft"));
    history.finish(request, &inv, false, &HashMap::new());
    assert!(history.next_undo().is_none());

    // the server refuses an undo: the step goes back where it was
    user(&mut history, &mut inv, &ids, change);
    let protected = none();
    let rules_of = Rules {
        protected: &protected,
        worn: &protected,
    };
    let (_, effective) = history.plan(&inv, true, &rules_of).expect("plan");
    let request = Uuid::new_v4();
    history.begin(request, true, effective);
    assert!(history.busy() && history.next_undo().is_none());
    history.finish(request, &inv, false, &HashMap::new());
    assert!(!history.busy());
    assert!(history.next_undo().is_some() && history.next_redo().is_none());
}

#[test]
fn a_step_whose_entries_moved_since_is_dropped_for_the_one_before() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let first = move_selection(&inv, &[ids.hat], ids.objects, &none(), &none()).expect("move");
    user(&mut history, &mut inv, &ids, first);
    let second = move_selection(&inv, &[ids.shirt], ids.objects, &none(), &none()).expect("move");
    user(&mut history, &mut inv, &ids, second);
    // the shirt moved on from elsewhere: its step has nothing left
    let elsewhere = move_selection(&inv, &[ids.shirt], ids.trash_sub, &none(), &none());
    assert!(elsewhere.is_err(), "the Trash's folders take no move");
    let elsewhere = move_selection(&inv, &[ids.shirt], inv.root, &none(), &none()).expect("move");
    demo_mutate(&mut inv, ids.agent, elsewhere).expect("moved");
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo the step before");
    assert_eq!(parent(&inv, ids.hat), Some(ids.clothes));
    assert_eq!(parent(&inv, ids.shirt), Some(inv.root), "left where it went");
    assert!(history.next_undo().is_none());
}

#[test]
fn purges_and_other_changes_are_no_step() {
    let (inv, ids) = fixture();
    let purge = Mutation {
        operations: vec![Operation::Purge(ids.trash)],
        refresh: vec![ids.trash],
        ..Default::default()
    };
    assert!(draft(&inv, &purge).is_none());
    let favorite = crate::world::inventory::actions::toggle_favorite(&inv, ids.hat).expect("favorite");
    assert!(draft(&inv, &favorite).is_none());
    let description = patch(&inv, ids.hat, llsd_map! { "desc" => "A hat" }).expect("desc");
    assert!(draft(&inv, &description).is_none());
}
