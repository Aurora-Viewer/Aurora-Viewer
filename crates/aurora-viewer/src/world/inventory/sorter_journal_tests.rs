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
        }]);
    }
    assert_eq!(journal.runs.len(), RUNS);
    assert_eq!(journal.last().map(|run| run.at), Some(RUNS as i64 + 2));
    journal.drop_last();
    assert_eq!(journal.last().map(|run| run.at), Some(RUNS as i64 + 1));
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
        },
        Entry {
            id: id(101),
            folder: false,
            from: id(12),
            name: "b".into(),
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
    let snapshot = Snapshot::take(&inv, 42);
    assert!(snapshot.items.iter().any(|(item, parent, _)| *item == id(100) && *parent == id(10)));
    let _ = sort(&mut inv);
    // a folder the snapshot knew is gone: its entries are counted, not guessed
    let (moves, missing) = restore_moves(&inv, &snapshot, &HashSet::new(), &HashSet::new());
    assert_eq!(missing, 0);
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
    let (_, missing) = restore_moves(&inv, &gone, &HashSet::new(), &HashSet::new());
    assert_eq!(missing, 1);
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
        }],
    };
    save(&unfinished_path(&dir), &unfinished).expect("save");
    assert_eq!(load::<Unfinished>(&unfinished_path(&dir)).expect("load"), unfinished);
    let inv = fixture();
    for at in 0..(SNAPSHOTS as i64 + 2) {
        save(&snapshot_path(&dir, at), &Snapshot::take(&inv, at)).expect("snapshot");
    }
    prune_snapshots(&dir);
    let kept = snapshots(&dir);
    assert_eq!(kept.len(), SNAPSHOTS);
    assert_eq!(kept.first().map(|(at, _)| *at), Some(SNAPSHOTS as i64 + 1));
    let _ = std::fs::remove_dir_all(&dir);
}
