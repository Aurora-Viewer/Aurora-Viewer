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
    let (change, effective) = history.plan(inv, undo, &rules_of, false)?;
    // (the keys: a stale step stops the undo, `plan`)
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
    assert!(history.plan(&inv, true, &rules_of, false).is_err());
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
    let (_, effective) = history.plan(&inv, true, &rules_of, false).expect("plan");
    let request = Uuid::new_v4();
    history.begin(request, true, effective);
    assert!(history.busy() && history.next_undo().is_none());
    history.finish(request, &inv, false, &HashMap::new());
    assert!(!history.busy());
    assert!(history.next_undo().is_some() && history.next_redo().is_none());
}

#[test]
fn a_step_whose_entries_moved_since_is_dropped_and_the_keys_stop_there() {
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
    // the keys: the stale step is dropped, and nothing else is undone
    assert_eq!(
        step(&mut history, &mut inv, &ids, true, &none()).unwrap_err(),
        aurora_i18n::tr!("inventory-undo-dropped")
    );
    assert_eq!(parent(&inv, ids.hat), Some(ids.objects), "the step before waits");
    // the next press takes the step before
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo the step before");
    assert_eq!(parent(&inv, ids.hat), Some(ids.clothes));
    assert_eq!(parent(&inv, ids.shirt), Some(inv.root), "left where it went");
    assert!(history.next_undo().is_none());
}

#[test]
fn the_menu_names_the_step_it_takes_past_a_stale_one() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let first = move_selection(&inv, &[ids.hat], ids.objects, &none(), &none()).expect("move");
    user(&mut history, &mut inv, &ids, first);
    let second = move_selection(&inv, &[ids.shirt], ids.objects, &none(), &none()).expect("move");
    user(&mut history, &mut inv, &ids, second);
    let elsewhere = move_selection(&inv, &[ids.shirt], inv.root, &none(), &none()).expect("move");
    demo_mutate(&mut inv, ids.agent, elsewhere).expect("moved");
    assert!(matches!(history.shown(&inv, true), Some(Step::Move { moves, .. }) if moves[0].id == ids.hat));
    let protected = none();
    let rules_of = Rules {
        protected: &protected,
        worn: &protected,
    };
    let (_, step) = history.plan(&inv, true, &rules_of, true).expect("the named step");
    assert!(matches!(step, Step::Move { moves, .. } if moves[0].id == ids.hat));
}

#[test]
fn undoing_a_restore_puts_the_entry_back_in_the_trash() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    // deleted from elsewhere (not recorded), then restored here
    let delete = move_selection(&inv, &[ids.shirt], ids.trash, &none(), &none()).expect("delete");
    demo_mutate(&mut inv, ids.agent, delete).expect("deleted");
    let restore = move_selection(&inv, &[ids.shirt], ids.clothes, &none(), &none()).expect("restore");
    user(&mut history, &mut inv, &ids, restore);
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo the Restore");
    assert_eq!(parent(&inv, ids.shirt), Some(ids.trash));
    step(&mut history, &mut inv, &ids, false, &none()).expect("redo it");
    assert_eq!(parent(&inv, ids.shirt), Some(ids.clothes));
    // out of a folder of the Trash: back to the Trash's top
    let delete = move_selection(&inv, &[ids.hat], ids.trash, &none(), &none()).expect("delete");
    demo_mutate(&mut inv, ids.agent, delete).expect("deleted");
    inv.items.get_mut(&ids.hat).expect("hat").parent = ids.trash_sub;
    let out = move_selection(&inv, &[ids.hat], ids.objects, &none(), &none()).expect("drag out");
    user(&mut history, &mut inv, &ids, out);
    step(&mut history, &mut inv, &ids, true, &none()).expect("undo the drag out");
    assert_eq!(parent(&inv, ids.hat), Some(ids.trash));
}

#[test]
fn the_history_waits_while_a_change_it_records_is_on_its_way() {
    let (inv, ids) = fixture();
    let mut history = History::default();
    let copy = paste(&inv, &[ids.hat], ids.objects, false, false, &none(), &none()).expect("copy");
    history.record(Uuid::new_v4(), draft(&inv, &copy).expect("draft"));
    assert!(history.busy(), "its copies may arrive long after");
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

#[test]
fn an_empty_history_says_so_and_a_gone_step_says_why() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let protected = none();
    let rules_of = Rules {
        protected: &protected,
        worn: &protected,
    };
    assert_eq!(
        history.plan(&inv, true, &rules_of, false).unwrap_err(),
        aurora_i18n::tr!("inventory-undo-empty")
    );
    let change = move_selection(&inv, &[ids.hat], ids.objects, &none(), &none()).expect("move");
    user(&mut history, &mut inv, &ids, change);
    inv.items.get_mut(&ids.hat).expect("hat").parent = ids.clothes;
    assert!(history.shown(&inv, true).is_none(), "the menu names no step that is gone");
    assert_eq!(
        history.plan(&inv, true, &rules_of, false).unwrap_err(),
        aurora_i18n::tr!("inventory-undo-nothing")
    );
}

#[test]
fn links_into_a_folder_not_read_yet_are_no_step_and_a_copy_is_claimed_once() {
    let (mut inv, ids) = fixture();
    inv.folders.get_mut(&ids.objects).expect("objects").state = crate::world::inventory::FetchState::Unknown;
    let links = paste(&inv, &[ids.shirt], ids.objects, false, true, &none(), &none()).expect("links");
    assert!(draft(&inv, &links).is_none(), "its new entries cannot be told from the old");
    inv.folders.get_mut(&ids.objects).expect("objects").state = crate::world::inventory::FetchState::Fetched;

    // two copies of the same name, finished after both arrived
    let mut history = History::default();
    let copy = paste(&inv, &[ids.hat], ids.objects, false, false, &none(), &none()).expect("copy");
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    history.record(first, draft(&inv, &copy).expect("draft"));
    history.record(second, draft(&inv, &copy).expect("draft"));
    let mut made = Vec::new();
    for n in [77, 78] {
        let mut it = inv.items.get(&ids.hat).cloned().expect("hat");
        it.id = Uuid::from_u128(n);
        it.parent = ids.objects;
        made.push(it);
    }
    inv.add_items(made);
    history.finish(first, &inv, true, &HashMap::new());
    history.finish(second, &inv, true, &HashMap::new());
    let named: Vec<Uuid> = history
        .undo
        .iter()
        .filter_map(|step| match step {
            Step::Created { ids, .. } => Some(ids.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(named.len(), 2, "each copy once: {named:?}");
    assert_ne!(named[0], named[1]);
}

#[test]
fn an_undo_whose_folder_went_to_the_trash_falls_back_to_restores_place() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let protected = none();
    let rules_of = Rules {
        protected: &protected,
        worn: &protected,
    };
    // the shirt deleted, then its folder deleted too: the undo takes it to
    // Restore's place, not into the Trash's folder
    let change = move_selection(&inv, &[ids.shirt], ids.trash, &none(), &none()).expect("delete");
    user(&mut history, &mut inv, &ids, change);
    let folder = move_selection(&inv, &[ids.clothes], ids.trash, &none(), &none()).expect("folder");
    demo_mutate(&mut inv, ids.agent, folder).expect("moved");
    let (change, _) = history.plan(&inv, true, &rules_of, false).expect("plan");
    let Operation::Patch { body, .. } = &change.operations[0] else {
        panic!("a move");
    };
    assert_ne!(body["parent_id"].as_uuid(), ids.clothes);
    assert!(rules::destination(&inv, body["parent_id"].as_uuid()));
}

#[test]
fn a_new_folder_that_holds_entries_since_is_not_sent_to_the_trash() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let change = create_folder(ids.objects, "Box").expect("folder");
    let Operation::Folder { id: new, .. } = change.operations[0] else {
        panic!("a folder");
    };
    user(&mut history, &mut inv, &ids, change);
    // an item lands in it by a route the history does not record
    let into = move_selection(&inv, &[ids.hat], new, &none(), &none()).expect("move");
    demo_mutate(&mut inv, ids.agent, into).expect("moved");
    assert_eq!(
        step(&mut history, &mut inv, &ids, true, &none()).unwrap_err(),
        aurora_i18n::tr!("inventory-undo-folder-not-empty")
    );
    assert_eq!(parent(&inv, new), Some(ids.objects));
}

#[test]
fn a_same_named_item_of_another_kind_is_not_taken_for_a_copy() {
    let (mut inv, ids) = fixture();
    let mut history = History::default();
    let copy = paste(&inv, &[ids.hat], ids.objects, false, false, &none(), &none()).expect("copy");
    let request = Uuid::new_v4();
    history.record(request, draft(&inv, &copy).expect("draft"));
    let hat = inv.items.get(&ids.hat).cloned().expect("hat");
    // a stranger's "Hat" arrives first, then the copy
    let mut other = hat.clone();
    other.id = Uuid::from_u128(90);
    other.parent = ids.objects;
    other.creator = Uuid::from_u128(4242);
    let mut copied = hat;
    copied.id = Uuid::from_u128(91);
    copied.parent = ids.objects;
    inv.add_items(vec![other, copied]);
    history.finish(request, &inv, true, &HashMap::new());
    assert!(matches!(history.next_undo(), Some(Step::Created { ids: made, .. }) if made == &[Uuid::from_u128(91)]));
}

#[test]
fn restores_place_for_a_snapshot_is_the_photo_album() {
    // LLItemBridge::restoreItem: IT_SNAPSHOT goes to FT_SNAPSHOT_CATEGORY
    let (mut inv, ids) = fixture();
    let (album, textures) = (Uuid::from_u128(4500), Uuid::from_u128(4501));
    let root = inv.root;
    super::tests_support::folder(&mut inv, album, root, "Photo Album", 15);
    super::tests_support::folder(&mut inv, textures, root, "Textures", 0);
    inv.generation += 1;
    if let Some(it) = inv.items.get_mut(&ids.hat) {
        it.asset_type = 0;
        it.inv_type = 15;
    }
    if let Some(it) = inv.items.get_mut(&ids.shirt) {
        it.asset_type = 0;
        it.inv_type = 0;
    }
    assert_eq!(rules::default_folder(&inv, ids.hat), Some(album));
    assert_eq!(
        rules::default_folder(&inv, ids.shirt),
        Some(textures),
        "a texture stays in Textures"
    );
}
