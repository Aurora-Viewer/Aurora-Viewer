use super::*;

#[test]
fn auto_sort_is_offered_on_one_folder_of_the_agent_and_one_item_sorts_alone() {
    let agent = Uuid::from_u128(2);
    let mut inv = Inventory::default();
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
