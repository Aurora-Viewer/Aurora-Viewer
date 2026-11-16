use super::*;

#[test]
fn auto_sort_is_offered_on_one_folder_of_the_agent_and_one_item_sorts_alone() {
    let agent = Uuid::from_u128(2);
    // a loaded inventory has its root (nothing is open before it)
    let mut inv = Inventory {
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    inv.folders.insert(
        inv.root,
        crate::world::inventory::Folder {
            info: aurora_net::inventory::InvFolder {
                id: inv.root,
                name: "My Inventory".into(),
                type_default: 8,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: crate::world::inventory::FetchState::Fetched,
            library: false,
        },
    );
    crate::world::inventory::demo::seed(&mut inv, agent);
    let prefs = InventoryPreferences::default();
    let facts = Facts {
        agent,
        ..Default::default()
    };
    let st = InventoryUi::default();
    let offered = |id: u128, inv: &Inventory| {
        let catalog = super::super::context_catalog::build(inv, Uuid::from_u128(id), &st, &prefs, &facts);
        catalog.entries.iter().find(|e| e.id == "auto-sort").map(|e| e.enabled)
    };
    assert_eq!(offered(8000, &inv), Some(true));
    // an item gets the entry that sorts it alone, the Library a disabled one
    let entries = |id: u128, inv: &Inventory| {
        let catalog = super::super::context_catalog::build(inv, Uuid::from_u128(id), &st, &prefs, &facts);
        catalog.entries.iter().find(|e| e.id == "auto-sort").map(|e| e.label.clone())
    };
    assert_eq!(offered(8100, &inv), Some(true));
    assert_eq!(
        entries(8100, &inv).as_deref(),
        Some(aurora_i18n::tr!("inventory-sorter-menu-entries"))
    );
    let library = inv.lib_root.as_u128();
    assert_eq!(offered(library, &inv), Some(false));
}

/// A move of an item 9000 + `n` out of folder 1 into the sort folder.
fn moves(range: std::ops::Range<u128>) -> Vec<Move> {
    range
        .map(|n| Move {
            id: Uuid::from_u128(9000 + n),
            folder: false,
            name: format!("Item {n}"),
            from: Uuid::from_u128(1),
            to: crate::world::inventory::sorter::Dest::Path(vec!["Tops".into()]),
            label: "Sorted / Tops".into(),
            unpacker: false,
            restore: false,
        })
        .collect()
}

/// A batch of `range` the app took as `request`.
fn in_flight(st: &mut InventoryUi, range: std::ops::Range<u128>, request: Uuid) {
    let mut job = Job::new(VecDeque::new(), "Sorted".into());
    job.total = 400;
    let batch = moves(range);
    let change = aurora_net::inventory::operations::Mutation {
        operations: batch
            .iter()
            .map(|m| aurora_net::inventory::operations::Operation::Patch {
                id: m.id,
                folder: false,
                body: aurora_llsd::llsd_map! { "parent_id" => Uuid::from_u128(2) },
            })
            .collect(),
        ..Default::default()
    };
    job.in_flight = Some(InFlight {
        moves: batch,
        request: None,
        frames: 0,
        sent_at: Instant::now(),
    });
    st.sorter.job = Some(job);
    st.sorter.submitted = true;
    assert!(st.claim_sorter_request(request, &change), "the app recognises the sorter's batch");
}

#[test]
fn a_batch_counts_only_its_own_confirmed_result() {
    let mut st = InventoryUi::default();
    st.open_sorter(Uuid::from_u128(8000));
    let request = Uuid::from_u128(77);
    in_flight(&mut st, 0..200, request);
    // another change's result is not ours
    st.finish_sorter(Uuid::from_u128(78), None);
    assert!(st.sorter.job.as_ref().is_some_and(|job| job.done == 0 && job.in_flight.is_some()));
    st.finish_sorter(request, None);
    let job = st.sorter.job.as_ref().expect("job");
    assert_eq!(job.done, 200);
    assert!(job.in_flight.is_none() && !st.sorter.submitted);
    // the next batch waits for the pace
    assert!(job.next_at.is_some_and(|at| at > Instant::now()));
}

#[test]
fn a_refused_batch_is_retried_then_skipped_never_the_whole_job() {
    let mut st = InventoryUi::default();
    st.open_sorter(Uuid::from_u128(8000));
    for attempt in 1..=RETRIES {
        let request = Uuid::from_u128(100 + attempt as u128);
        let queued = st.sorter.job.as_ref().map_or(0, |job| job.queue.len());
        in_flight(&mut st, 0..200, request);
        if let Some(job) = st.sorter.job.as_mut() {
            job.retries = attempt - 1;
            job.queue = moves(200..200 + queued as u128).into();
        }
        st.finish_sorter(request, Some("network down"));
        let job = st.sorter.job.as_ref().expect("the job goes on");
        assert_eq!(job.queue.len(), 200 + queued, "the batch is back in the queue");
        assert_eq!(
            job.queue.front().map(|m| m.id),
            Some(Uuid::from_u128(9000)),
            "at the front, in order"
        );
        assert!(st.sorter.status.contains("network down"));
    }
    // after its retries the batch stays put and the job goes on
    let request = Uuid::from_u128(200);
    in_flight(&mut st, 0..200, request);
    if let Some(job) = st.sorter.job.as_mut() {
        job.retries = RETRIES;
    }
    st.finish_sorter(request, Some("network down"));
    let job = st.sorter.job.as_ref().expect("the job goes on");
    assert_eq!(job.failed, 200);
}

#[test]
fn a_batch_the_app_never_took_goes_back_to_the_queue() {
    let mut st = InventoryUi::default();
    let mut job = Job::new(VecDeque::new(), "Sorted".into());
    job.in_flight = Some(InFlight {
        moves: moves(0..3),
        request: None,
        frames: 3,
        sent_at: Instant::now(),
    });
    st.sorter.job = Some(job);
    st.sorter.submitted = true;
    // the requeue runs at the top of the window's frame; a change that is
    // not the sorter's is never claimed
    let other = aurora_net::inventory::operations::Mutation::default();
    assert!(!st.claim_sorter_request(Uuid::from_u128(5), &other));
    if let Some(job) = st.sorter.job.as_mut()
        && let Some(batch) = job.in_flight.take()
    {
        job.requeue(batch.moves);
    }
    assert_eq!(st.sorter.job.as_ref().map(|job| job.queue.len()), Some(3));
}

/// A large inventory's plan is made on another thread from a copy: the
/// same plan as on the frame, and one made for another source is dropped.
#[test]
fn a_background_plan_matches_the_frames_and_follows_its_source() {
    let mut inv = Inventory::default();
    let agent = Uuid::from_u128(2);
    crate::world::inventory::sorter::demo::seed(&mut inv, agent, 1);
    let objects = crate::world::inventory::sorter::demo::OBJECTS;
    let options = sorter::Preferences::default();
    let (none, worn) = (HashSet::new(), HashSet::new());
    let on_frame = sorter::plan(&inv, objects, &options, &none, &worn);
    let source = Source::Folder(objects);
    let make = {
        let options = options.clone();
        move |inv: &Inventory| sorter::plan(inv, objects, &options, &HashSet::new(), &HashSet::new())
    };
    let mut slot = Some(Planning::spawn(&inv, &options, &source, None, make));
    let started = Instant::now();
    let (task, plan) = loop {
        if let Some(done) = Planning::take(&mut slot) {
            break done;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "the plan arrives");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(slot.is_none());
    assert_eq!(task.source, source);
    assert_eq!(task.generation, inv.generation);
    assert_eq!(plan.moves, on_frame.moves);
    assert_eq!(plan.kept, on_frame.kept);
    // the window keeps it only for the source it still shows
    let mut st = InventoryUi::default();
    st.sorter.planning = Some(Planning::spawn(&inv, &options, &Source::Whole, None, |_: &Inventory| {
        (Plan::default(), crate::world::inventory::sorter::journal::Snapshot::default())
    }));
    st.sorter.source = Some(source.clone());
    std::thread::sleep(std::time::Duration::from_millis(50));
    let arrived = Planning::take(&mut st.sorter.planning).expect("arrived");
    assert_ne!(arrived.0.source, source, "a plan for another source is not shown");
}

/// After Stop, a refused batch is not retried: nothing more is sent.
#[test]
fn after_stop_a_refused_batch_is_not_sent_again() {
    let mut st = InventoryUi::default();
    st.open_sorter(Uuid::from_u128(8000));
    let request = Uuid::from_u128(300);
    in_flight(&mut st, 0..200, request);
    if let Some(job) = st.sorter.job.as_mut() {
        // Stop, as the footer's button does
        job.stopping = true;
        job.queue.clear();
    }
    st.finish_sorter(request, Some("network down"));
    let job = st.sorter.job.as_ref().expect("the job ends on its own");
    assert!(job.queue.is_empty(), "nothing put back to retry");
    assert_eq!(job.failed, 200);
}

/// A batch the server applied after all (late, or in part): each move found
/// where it was sent counts as done and goes in the journal, whether it
/// was back in the queue or counted as failed; the others stay unsure.
#[test]
fn a_batch_applied_late_counts_as_done_and_is_journaled() {
    use crate::world::inventory::sorter::Dest;
    let mut inv = Inventory {
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    for (id, parent, name) in [(1u128, 0u128, "My Inventory"), (2, 1, "Old stuff"), (3, 1, "Keep")] {
        inv.folders.insert(
            Uuid::from_u128(id),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: Uuid::from_u128(id),
                    parent: Uuid::from_u128(parent),
                    name: name.into(),
                    type_default: if id == 1 { 8 } else { -1 },
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
    }
    inv.add_items(
        (0..4u128)
            .map(|n| aurora_net::inventory::InvItem {
                id: Uuid::from_u128(9000 + n),
                parent: Uuid::from_u128(2),
                name: format!("Item {n}"),
                asset_type: 6,
                ..Default::default()
            })
            .collect(),
    );
    let to_keep = |n: u128| Move {
        id: Uuid::from_u128(9000 + n),
        folder: false,
        name: format!("Item {n}"),
        from: Uuid::from_u128(2),
        to: Dest::Folder(Uuid::from_u128(3)),
        label: String::new(),
        unpacker: false,
        restore: false,
    };
    let mut job = Job::new(VecDeque::new(), "Sorted".into());
    let now = Instant::now();
    // a refused batch of two, back in the queue
    job.in_flight = Some(InFlight {
        moves: vec![to_keep(0), to_keep(1)],
        request: Some(Uuid::from_u128(1)),
        frames: 0,
        sent_at: now,
    });
    job.finish(Some("timeout"), now);
    assert_eq!(job.queue.len(), 2);
    // a batch of two refused for good
    job.retries = RETRIES;
    job.in_flight = Some(InFlight {
        moves: vec![to_keep(2), to_keep(3)],
        request: Some(Uuid::from_u128(2)),
        frames: 0,
        sent_at: now,
    });
    job.finish(Some("refused"), now);
    assert_eq!(job.failed, 2);
    // the server applied item 0 and item 2 late
    for n in [0u128, 2] {
        if let Some(it) = inv.items.get_mut(&Uuid::from_u128(9000 + n)) {
            it.parent = Uuid::from_u128(3);
        }
    }
    job.settle(&inv);
    assert_eq!(job.done, 2);
    assert_eq!(job.failed, 1, "item 2 is no longer a failure");
    assert_eq!(
        job.queue.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![Uuid::from_u128(9001)],
        "item 0 is not sent again"
    );
    assert_eq!(job.confirmed.len(), 2, "both go in the journal, for Undo");
    assert_eq!(job.unsure.len(), 2, "the others stay unsure");
    // nothing counted twice
    job.settle(&inv);
    assert_eq!(job.done, 2);
}

/// A Trash batch counts and journals only what is in the Trash once its
/// delete is over: an entry the request left behind is a failure.
#[test]
fn a_trash_batch_counts_what_reached_the_trash() {
    let mut inv = Inventory {
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    for (id, parent, name, kind) in [(1u128, 0u128, "My Inventory", 8), (2, 1, "Old stuff", -1), (14, 1, "Trash", 14)] {
        inv.folders.insert(
            Uuid::from_u128(id),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: Uuid::from_u128(id),
                    parent: Uuid::from_u128(parent),
                    name: name.into(),
                    type_default: kind,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
    }
    if let Some(root) = inv.folders.get_mut(&Uuid::from_u128(1)) {
        root.children = vec![Uuid::from_u128(2), Uuid::from_u128(14)];
    }
    inv.add_items(
        [(9000u128, 14u128), (9001, 2)]
            .into_iter()
            .map(|(id, parent)| aurora_net::inventory::InvItem {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(parent),
                name: format!("Item {id}"),
                asset_type: 6,
                ..Default::default()
            })
            .collect(),
    );
    let mut st = InventoryUi::default();
    st.sorter.journal.begin(crate::world::inventory::sorter::journal::RunKind::Trash, 1);
    st.sorter.trash = Some(TrashJob {
        queue: VecDeque::new(),
        skipped: 0,
        total: 2,
        sent: 0,
        failed: 0,
        // taken by the app, and its delete is over
        in_flight: Some((
            vec![
                (Uuid::from_u128(9000), Uuid::from_u128(2)),
                (Uuid::from_u128(9001), Uuid::from_u128(2)),
            ],
            3,
            true,
        )),
        next_at: None,
        begun: true,
    });
    let prefs = InventoryPreferences::default();
    let mut actions = Vec::new();
    send_trash_batch(&inv, &mut st, &prefs, &HashSet::new(), &mut actions, false);
    let trash = st.sorter.trash.as_ref().expect("the job goes on");
    assert_eq!((trash.sent, trash.failed), (1, 1));
    let run = st.sorter.journal.last().expect("a run");
    assert_eq!(run.entries.iter().map(|e| e.id).collect::<Vec<_>>(), vec![Uuid::from_u128(9000)]);
}
