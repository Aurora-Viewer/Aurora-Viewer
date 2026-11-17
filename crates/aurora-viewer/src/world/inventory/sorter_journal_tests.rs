use super::super::super::{FetchState, Folder, Inventory, actions};
use super::super::{Dest, Move, Preferences, Step, plan, step};
use super::*;
use aurora_net::inventory::{InvFolder, InvItem};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const AGENT: u128 = 2;

fn folder(id: Uuid, parent: Uuid, name: &str, kind: i32) -> Folder {
    Folder {
        info: InvFolder {
            id,
            parent,
            name: name.into(),
            type_default: kind,
            version: 1,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    }
}

fn add_folder(inv: &mut Inventory, at: u128, parent: u128, name: &str, kind: i32) {
    inv.folders.insert(id(at), folder(id(at), id(parent), name, kind));
    if let Some(p) = inv.folders.get_mut(&id(parent)) {
        p.children.push(id(at));
    }
}

fn add_item(inv: &mut Inventory, at: u128, parent: u128, name: &str) {
    inv.add_items(vec![InvItem {
        id: id(at),
        parent: id(parent),
        name: name.into(),
        asset_type: 6,
        inv_type: 6,
        owner: id(AGENT),
        created_at: 1_400_000_000,
        ..Default::default()
    }]);
}

/// My Inventory, Objects with two skirts and a lamp, a Trash, a folder of
/// the user's and My Outfits with an outfit.
fn fixture() -> Inventory {
    let mut inv = Inventory {
        root: id(1),
        ..Default::default()
    };
    inv.folders.insert(id(1), folder(id(1), Uuid::nil(), "My Inventory", 8));
    add_folder(&mut inv, 10, 1, "Objects", 6);
    add_folder(&mut inv, 11, 1, "Trash", 14);
    add_folder(&mut inv, 12, 1, "My stuff", -1);
    add_folder(&mut inv, 13, 1, "My Outfits", 48);
    add_folder(&mut inv, 14, 13, "Party", 47);
    add_item(&mut inv, 100, 10, "Denim skirt Reborn");
    add_item(&mut inv, 101, 10, "Velvet skirt Reborn");
    add_item(&mut inv, 102, 10, "Brass floor lamp");
    inv.generation += 1;
    inv
}

/// Sort Objects to the end, recording each confirmed batch.
fn sort(inv: &mut Inventory) -> Journal {
    let prefs = Preferences::default();
    let none = HashSet::new();
    let planned = plan(inv, id(10), &prefs, &none, &none);
    let mut queue: VecDeque<Move> = planned.moves.into();
    let mut journal = Journal::default();
    journal.begin(RunKind::Move, 1);
    while let Step::Change(change, moves) = step(inv, &prefs.base_name(), &mut queue, &none, &none, &mut 0) {
        actions::demo_mutate(inv, id(AGENT), *change).expect("demo server");
        journal.record(moves.iter().map(|m| Entry {
            id: m.id,
            folder: m.folder,
            from: m.from,
            name: m.name.clone(),
            to: None,
        }));
    }
    journal
}

/// Send `moves` through the same batches as a sort.
fn send(inv: &mut Inventory, moves: Vec<Move>) {
    let none = HashSet::new();
    let mut queue: VecDeque<Move> = moves.into();
    while let Step::Change(change, _) = step(inv, "Sorted", &mut queue, &none, &none, &mut 0) {
        actions::demo_mutate(inv, id(AGENT), *change).expect("demo server");
    }
}

#[test]
fn the_journal_keeps_the_last_runs() {
    let mut journal = Journal::default();
    for at in 0..(RUNS as i64 + 3) {
        journal.begin(RunKind::Move, at);
        journal.record([Entry {
            id: id(at as u128),
            folder: false,
            from: id(1),
            name: "x".into(),
            to: None,
        }]);
    }
    assert_eq!(journal.runs.len(), RUNS);
    assert_eq!(journal.last().map(|run| run.at), Some(RUNS as i64 + 2));
}

fn entry(n: u128) -> Entry {
    Entry {
        id: id(n),
        folder: false,
        from: id(1),
        name: "x".into(),
        to: None,
    }
}

/// An undo forgets what it put back, and only that: a stopped or partial
/// undo leaves the rest to undo, an undo that moved nothing forgets nothing,
/// and runs are told apart by their id, never by their time.
#[test]
fn an_undo_forgets_only_what_it_put_back() {
    let mut journal = Journal::default();
    let old = journal.begin(RunKind::Move, 10);
    journal.record_in(old, [entry(1)]);
    // a run begun in the same second as the next one
    let run = journal.begin(RunKind::Move, 20);
    journal.record_in(run, [entry(2), entry(3), entry(4)]);
    let other = journal.begin(RunKind::Trash, 20);
    journal.record_in(other, [entry(9)]);
    // a partial undo of `run`: two of three put back
    let undo = journal.begin(RunKind::Move, 30);
    journal.record_in(undo, [entry(2), entry(3)]);
    journal.forget_undone(undo, run);
    let left: Vec<u64> = journal.runs.iter().map(|r| r.id).collect();
    assert_eq!(left, vec![old, run, other]);
    let kept = journal.runs.iter().find(|r| r.id == run).expect("the run");
    assert_eq!(kept.entries.iter().map(|e| e.id).collect::<Vec<_>>(), vec![id(4)]);
    // an undo that moved nothing: the undone run stays whole
    let undo = journal.begin(RunKind::Move, 40);
    journal.forget_undone(undo, run);
    assert!(journal.runs.iter().any(|r| r.id == run && r.entries.len() == 1));
    // the rest put back: the run goes, the others are untouched
    let undo = journal.begin(RunKind::Move, 50);
    journal.record_in(undo, [entry(4)]);
    journal.forget_undone(undo, run);
    assert_eq!(journal.runs.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old, other]);
    // the undone run no longer there (pruned): only the undo's run goes
    let undo = journal.begin(RunKind::Move, 60);
    journal.forget_undone(undo, 999);
    assert_eq!(journal.runs.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old, other]);
}

/// Runs read from an older file get ids, in order, after the ones they have.
#[test]
fn runs_from_an_older_file_are_numbered() {
    let mut journal = Journal::default();
    for at in [1, 2, 3] {
        journal.runs.push_back(Run {
            id: 0,
            at,
            kind: RunKind::Move,
            entries: Vec::new(),
            undoes: None,
        });
    }
    journal.number_runs();
    assert_eq!(journal.runs.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(journal.begin(RunKind::Move, 4), 4);
}

#[test]
fn undo_puts_a_sort_back_and_skips_what_moved_since() {
    let mut inv = fixture();
    let journal = sort(&mut inv);
    let run = journal.last().expect("a run");
    assert_eq!(run.entries.len(), 3);
    assert!(inv.items.values().all(|it| it.parent != id(10)), "Objects was sorted");
    // the user moves the lamp on by hand: undo leaves it there
    if let Some(lamp) = inv.items.get_mut(&id(102)) {
        lamp.parent = id(12);
    }
    let (moves, skipped) = undo_moves(&inv, run, &HashSet::new(), &HashSet::new());
    assert_eq!(
        skipped, 0,
        "already moved by hand is not 'skipped': it is simply not where the run left it"
    );
    assert!(moves.iter().all(|m| m.restore && m.to == Dest::Folder(id(10))));
    send(&mut inv, moves);
    assert_eq!(inv.items[&id(100)].parent, id(10));
    assert_eq!(inv.items[&id(101)].parent, id(10));
    // the lamp was moved by hand to the user's folder after the sort: undo
    // put it back too, since it left the place the sort gave it for an
    // open folder; nothing went into a hands-off folder
    assert!(inv.items.values().all(|it| it.parent != id(14) && it.parent != id(11)));
}

#[test]
fn undo_of_a_trash_run_takes_entries_out_of_the_trash() {
    let mut inv = fixture();
    let mut journal = Journal::default();
    journal.begin(RunKind::Trash, 1);
    for n in [100, 101] {
        journal.record([Entry {
            id: id(n),
            folder: false,
            from: id(10),
            name: format!("{n}"),
            to: None,
        }]);
        if let Some(it) = inv.items.get_mut(&id(n)) {
            it.parent = id(11);
        }
    }
    let (moves, skipped) = undo_moves(&inv, journal.last().expect("run"), &HashSet::new(), &HashSet::new());
    assert_eq!((moves.len(), skipped), (2, 0));
    send(&mut inv, moves);
    assert_eq!(inv.items[&id(100)].parent, id(10));
}

#[test]
fn undo_never_moves_into_or_out_of_a_hands_off_folder() {
    let mut inv = fixture();
    let mut journal = Journal::default();
    journal.begin(RunKind::Move, 1);
    // an entry recorded as coming from an outfit, and one now in an outfit
    journal.record([
        Entry {
            id: id(100),
            folder: false,
            from: id(14),
            name: "a".into(),
            to: None,
        },
        Entry {
            id: id(101),
            folder: false,
            from: id(12),
            name: "b".into(),
            to: None,
        },
    ]);
    if let Some(it) = inv.items.get_mut(&id(101)) {
        it.parent = id(14);
    }
    let (moves, skipped) = undo_moves(&inv, journal.last().expect("run"), &HashSet::new(), &HashSet::new());
    assert!(moves.is_empty());
    assert_eq!(skipped, 2);
}

#[test]
fn a_snapshot_puts_the_layout_back() {
    let mut inv = fixture();
    let snapshot = Snapshot::take(&inv, 42, id(AGENT));
    assert!(snapshot.items.iter().any(|(item, parent, _)| *item == id(100) && *parent == id(10)));
    let _ = sort(&mut inv);
    // a folder the snapshot knew is gone: its entries are counted, not guessed
    let (moves, summary) = restore_moves(
        &inv,
        &snapshot,
        id(AGENT),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &crate::world::inventory::sorter::Preferences::default(),
    )
    .expect("same account");
    assert_eq!(summary.folder_gone, 0);
    send(&mut inv, moves);
    for n in [100, 101, 102] {
        assert_eq!(inv.items[&id(n)].parent, id(10), "{n} back in Objects");
    }
    let mut gone = snapshot.clone();
    gone.items.push((id(100), id(999), "x".into()));
    gone.items.retain(|(item, parent, _)| *item != id(100) || *parent == id(999));
    if let Some(it) = inv.items.get_mut(&id(100)) {
        it.parent = id(12);
    }
    let (_, summary) = restore_moves(
        &inv,
        &gone,
        id(AGENT),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &crate::world::inventory::sorter::Preferences::default(),
    )
    .expect("same account");
    assert_eq!(summary.folder_gone, 1);
}

#[test]
fn a_snapshot_restore_leaves_a_folder_excluded_since_alone() {
    let mut inv = fixture();
    let snapshot = Snapshot::take(&inv, 42, id(AGENT));
    let _ = sort(&mut inv);
    // Objects excluded after the snapshot: nothing goes back into it
    let prefs = crate::world::inventory::sorter::Preferences {
        excluded: vec![id(10)],
        ..Default::default()
    };
    let (moves, summary) = restore_moves(
        &inv,
        &snapshot,
        id(AGENT),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &prefs,
    )
    .expect("same account");
    assert!(moves.iter().all(|m| m.to != Dest::Folder(id(10))), "never into an excluded folder");
    assert!(summary.kept > 0);
}

#[test]
fn files_round_trip_and_snapshots_are_pruned() {
    let dir = std::env::temp_dir().join(format!("aurora-auto-sort-test-{}", Uuid::new_v4()));
    let mut journal = Journal::default();
    journal.begin(RunKind::Move, 7);
    journal.record([Entry {
        id: id(5),
        folder: false,
        from: id(1),
        name: "Item".into(),
        to: None,
    }]);
    save(&journal_path(&dir), &journal).expect("save");
    assert_eq!(load::<Journal>(&journal_path(&dir)).expect("load"), journal);
    let unfinished = Unfinished {
        base: "Sorted".into(),
        moves: vec![Move {
            id: id(5),
            folder: false,
            name: "Item".into(),
            from: id(1),
            to: Dest::PreTrash,
            label: "Pre-Trash".into(),
            unpacker: false,
            restore: false,
            from_trash: false,
            snapshot: false,
        }],
        base_id: None,
        undoes: None,
        runs: Vec::new(),
    };
    save(&unfinished_path(&dir), &unfinished).expect("save");
    assert_eq!(load::<Unfinished>(&unfinished_path(&dir)).expect("load"), unfinished);
    let inv = fixture();
    for at in 0..(SNAPSHOTS as i64 + 2) {
        save(&snapshot_path(&dir, at), &Snapshot::take(&inv, at, id(AGENT))).expect("snapshot");
    }
    prune_snapshots(&dir);
    let kept = snapshots(&dir);
    assert_eq!(kept.len(), SNAPSHOTS);
    assert_eq!(kept.first().map(|(at, _)| *at), Some(SNAPSHOTS as i64 + 1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// After a snapshot, items arrive, are deleted, trashed, moved by hand,
/// folders are renamed or deleted: putting the layout back never errors,
/// it skips and counts, and lists what was moved by hand on its own.
#[test]
fn a_layout_restore_survives_everything_that_changed_since() {
    let mut inv = fixture();
    add_folder(&mut inv, 15, 1, "Temp", -1);
    add_item(&mut inv, 103, 15, "Old shoes");
    add_item(&mut inv, 104, 12, "Kept note");
    add_item(&mut inv, 105, 12, "To bin");
    add_item(&mut inv, 106, 12, "Gone for good");
    let snapshot = Snapshot::take(&inv, 100, id(AGENT));
    // a run sorts Objects (the journal remembers it)
    let journal = sort(&mut inv);
    let by_runs = moved_since(&journal, 0);
    // then by hand: a new item, one trashed, one deleted, one moved, a
    // folder renamed and one deleted
    add_item(&mut inv, 107, 10, "Arrived later");
    if let Some(it) = inv.items.get_mut(&id(105)) {
        it.parent = id(11);
    }
    inv.items.remove(&id(106));
    if let Some(it) = inv.items.get_mut(&id(104)) {
        it.parent = id(15);
    }
    if let Some(f) = inv.folders.get_mut(&id(12)) {
        f.info.name = "My stuff renamed".into();
    }
    if let Some(it) = inv.items.get_mut(&id(103)) {
        it.parent = id(12);
    }
    inv.folders.remove(&id(15));
    inv.generation += 1;
    let (moves, summary) = restore_moves(
        &inv,
        &snapshot,
        id(AGENT),
        &by_runs,
        &HashSet::new(),
        &HashSet::new(),
        &crate::world::inventory::sorter::Preferences::default(),
    )
    .expect("same account");
    assert_eq!(summary.new_since, 1, "the new item is counted, never moved");
    assert!(!moves.iter().any(|m| m.id == id(107)));
    assert_eq!(summary.trashed, 1, "nothing comes out of the Trash but by Undo");
    assert!(!moves.iter().any(|m| m.id == id(105)));
    // the deleted item and the deleted Temp folder
    assert_eq!(summary.gone, 2);
    // the shoes' snapshot folder (Temp) is gone: not guessed
    assert!(summary.folder_gone >= 1);
    assert!(!moves.iter().any(|m| m.id == id(103)));
    // what the run moved goes back as such; the rest moved by hand is
    // listed on its own
    let back: Vec<_> = moves.iter().filter(|m| [100, 101, 102].map(id).contains(&m.id)).collect();
    assert_eq!(back.len(), 3);
    assert!(
        back.iter()
            .all(|m| !m.label.starts_with(aurora_i18n::tr!("inventory-sorter-moved-by-hand")))
    );
    send(&mut inv, moves);
    for n in [100, 101, 102] {
        assert_eq!(inv.items[&id(n)].parent, id(10));
    }
    assert_eq!(inv.items[&id(107)].parent, id(10), "the new item did not move");
    assert_eq!(inv.items[&id(105)].parent, id(11), "still in the Trash");
    // another account's snapshot is refused
    assert_eq!(
        restore_moves(
            &inv,
            &snapshot,
            id(99),
            &by_runs,
            &HashSet::new(),
            &HashSet::new(),
            &crate::world::inventory::sorter::Preferences::default()
        )
        .err(),
        Some(OtherAccount)
    );
}

/// Undo after the same changes: it skips and counts, and it is the only
/// path that takes things out of the Trash (for a Trash run).
#[test]
fn undo_survives_everything_that_changed_since() {
    let mut inv = fixture();
    let journal = sort(&mut inv);
    let run = journal.last().expect("run").clone();
    // one sorted item deleted, one trashed by hand, one left where the run put it
    inv.items.remove(&id(100));
    if let Some(it) = inv.items.get_mut(&id(101)) {
        it.parent = id(11);
    }
    let (moves, skipped) = undo_moves(&inv, &run, &HashSet::new(), &HashSet::new());
    assert_eq!(skipped, 2, "the deleted and the trashed one are counted");
    assert_eq!(moves.len(), 1);
    send(&mut inv, moves);
    assert_eq!(inv.items[&id(102)].parent, id(10));
    assert_eq!(inv.items[&id(101)].parent, id(11), "a Move run's undo leaves the Trash alone");
}

/// A run that moved nothing never pushes a real one out of the last five,
/// and an id is never given twice, even after runs are forgotten.
#[test]
fn empty_runs_never_push_real_ones_out_and_ids_never_repeat() {
    let mut journal = Journal::default();
    let real = journal.begin(RunKind::Move, 1);
    journal.record_in(real, [entry(1)]);
    for at in 2..20 {
        journal.begin(RunKind::Move, at);
    }
    assert!(journal.runs.iter().any(|r| r.id == real), "the real run is still there");
    let undo = journal.begin(RunKind::Move, 30);
    journal.record_in(undo, [entry(1)]);
    journal.forget_undone(undo, real);
    let next = journal.begin(RunKind::Move, 40);
    assert!(next > undo && next > real, "ids never repeat");
}

/// An Undo's own runs are never "the last run" (undoing them would put the
/// sort back), an Undo resumed in another session forgets all its runs at
/// the end, and a run whose entries are all gone never hides an older one.
#[test]
fn undo_runs_are_never_undone_and_gone_runs_never_hide_older_ones() {
    let mut journal = Journal::default();
    let sort = journal.begin(RunKind::Move, 1);
    journal.record_in(sort, [entry(1), entry(2), entry(3)]);
    // an Undo stopped by a crash, then resumed: two runs of the same Undo
    let first = journal.begin_undo(2, sort);
    journal.record_in(first, [entry(1)]);
    assert_eq!(journal.last().map(|r| r.id), Some(sort), "the crashed Undo is not the last run");
    let second = journal.begin_undo(3, sort);
    journal.record_in(second, [entry(2)]);
    for own in [first, second] {
        journal.forget_undone(own, sort);
    }
    let left = journal.runs.iter().find(|r| r.id == sort).expect("the sort");
    assert_eq!(left.entries.iter().map(|e| e.id).collect::<Vec<_>>(), vec![id(3)]);
    assert!(journal.runs.iter().all(|r| r.undoes.is_none()));
    // a newer run whose entries were all purged: the older one is offered
    let trash = journal.begin(RunKind::Trash, 4);
    journal.record_in(trash, [entry(9)]);
    let found = journal.last_undoable(|e| e.id != id(9)).map(|r| r.id);
    assert_eq!(found, Some(sort));
}

/// Undo leaves an entry the user moved on after the run.
#[test]
fn undo_leaves_what_was_moved_by_hand_after_the_run() {
    let mut inv = fixture();
    let mut journal = sort(&mut inv);
    // where the run put each entry
    for run in journal.runs.iter_mut() {
        for e in run.entries.iter_mut() {
            e.to = actions::parent(&inv, e.id);
        }
    }
    let run = journal.last().expect("a run").clone();
    let moved = run.entries[0].id;
    if let Some(it) = inv.items.get_mut(&moved) {
        it.parent = id(12);
    }
    let (moves, skipped) = undo_moves(&inv, &run, &HashSet::new(), &HashSet::new());
    assert!(moves.iter().all(|m| m.id != moved));
    assert!(skipped >= 1);
}

/// The run an Undo is putting back is never the one dropped to keep the
/// last five, and every Undo run of it is forgotten at its end.
#[test]
fn an_undone_run_is_kept_while_it_is_undone() {
    let mut journal = Journal::default();
    let oldest = journal.begin(RunKind::Move, 1);
    journal.record_in(oldest, [entry(1), entry(2)]);
    for at in 2..(RUNS as i64 + 1) {
        let run = journal.begin(RunKind::Move, at);
        journal.record_in(run, [entry(100 + at as u128)]);
    }
    let undo = journal.begin_undo(50, oldest);
    assert!(journal.runs.iter().any(|r| r.id == oldest), "kept while undone");
    journal.record_in(undo, [entry(1)]);
    // a second Undo of it, begun afresh instead of resumed
    let again = journal.begin_undo(60, oldest);
    journal.record_in(again, [entry(2)]);
    journal.forget_undos_of(oldest);
    assert!(journal.runs.iter().all(|r| r.undoes.is_none()));
    assert!(journal.runs.iter().all(|r| r.id != oldest), "all of it went back");
}

/// A layout restore never puts an entry back into Pre-Trash: what the user
/// took out of it stays out.
#[test]
fn a_layout_restore_puts_nothing_back_into_pre_trash() {
    let mut inv = fixture();
    add_folder(&mut inv, 20, 1, &super::super::pre_trash_name(), -1);
    if let Some(it) = inv.items.get_mut(&id(100)) {
        it.parent = id(20);
    }
    let snapshot = Snapshot::take(&inv, 1, id(AGENT));
    // the user takes it out of Pre-Trash by hand
    if let Some(it) = inv.items.get_mut(&id(100)) {
        it.parent = id(12);
    }
    let (moves, _) = restore_moves(
        &inv,
        &snapshot,
        id(AGENT),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &crate::world::inventory::sorter::Preferences::default(),
    )
    .expect("same account");
    assert!(moves.iter().all(|m| m.id != id(100)));
}

/// A layout to share goes to the Desktop, else Documents, else the current
/// folder, named for its date.
#[test]
fn a_shared_layout_goes_to_the_desktop_then_documents_then_here() {
    use std::path::PathBuf;
    let (desk, docs, here) = (PathBuf::from("desk"), PathBuf::from("docs"), PathBuf::from("here"));
    let name = "Aurora inventory snapshot 2026-11-16 21-40.json";
    assert_eq!(
        share_path(Some(desk.clone()), Some(docs.clone()), here.clone(), "2026-11-16 21-40"),
        desk.join(name)
    );
    assert_eq!(
        share_path(None, Some(docs.clone()), here.clone(), "2026-11-16 21-40"),
        docs.join(name)
    );
    assert_eq!(share_path(None, None, here.clone(), "2026-11-16 21-40"), here.join(name));
}

/// The layout to share carries no account and no inventory id, and nothing
/// of the Trash or the calling cards.
#[test]
fn a_shared_layout_carries_no_account_ids_trash_or_calling_cards() {
    let id = |n: u128| Uuid::from_u128(n + 1000);
    let mut inv = Inventory {
        root: id(1),
        ..Default::default()
    };
    for (n, parent, name, kind) in [
        (1, 0, "My Inventory", 8),
        (14, 1, "Trash", 14),
        (2, 1, "Calling Cards", 2),
        (30, 1, "Shoes", -1),
    ] {
        inv.folders.insert(
            id(n),
            Folder {
                info: InvFolder {
                    id: id(n),
                    parent: id(parent),
                    name: name.into(),
                    type_default: kind,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    // (the folders hang under My Inventory, as the inventory keeps them)
    inv.folders.get_mut(&id(1)).expect("root").children = vec![id(14), id(2), id(30)];
    inv.add_items(
        [
            (900, 14, "Deleted Thing", 6),
            (901, 2, "A Resident", 2),
            (902, 30, "Kira Boots", 6),
            (903, 30, "Snapshot : Some Region (12, 34, 56)", 0),
        ]
        .into_iter()
        .map(|(n, parent, name, kind)| InvItem {
            id: id(n),
            parent: id(parent),
            name: name.into(),
            asset_type: kind,
            ..Default::default()
        })
        .collect(),
    );
    let shared = Snapshot::take_to_share(&inv, 1);
    assert!(shared.account.is_nil());
    let names: Vec<&str> = shared.items.iter().map(|(_, _, n)| n.as_str()).collect();
    assert_eq!(names, ["Kira Boots"]);
    assert!(!shared.folders.iter().any(|(_, _, n, _)| n == "Trash" || n == "Calling Cards"));
    let real: HashSet<Uuid> = [1, 30, 902].into_iter().map(id).collect();
    // (the numbers start at 1, the ids here at 1001)
    assert!(
        !shared.folders.iter().any(|(f, _, _, _)| real.contains(f)),
        "numbers, never inventory ids"
    );
    assert!(!shared.items.iter().any(|(i, _, _)| real.contains(i)));
    // a folder never read is marked unknown, never written as empty
    assert!(shared.unread.is_empty());
    inv.folders.get_mut(&id(30)).expect("folder").state = FetchState::Failed;
    let shared = Snapshot::take_to_share(&inv, 1);
    assert_eq!(shared.unread.len(), 1);
}
