use super::*;
use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

fn cooling_read(inv: &mut Inventory, children: Vec<Uuid>, now: Instant) -> BackgroundFetch {
    let mut fetch = BackgroundFetch::default();
    let mut out = Vec::new();
    fetch.send(inv, AGENT, Kind::Recursive, id(10), children, MAX_FOLDER_DEPTH, &mut out, 0);
    fetch.on_reply_at(inv, 0, &[], Err(FetchError::new(FetchFailure::Transient, Some(503))), now);
    fetch
}

#[test]
fn superseded_403_retries_the_same_read_without_spending_its_backoff() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let now = Instant::now();
    let mut fetch = BackgroundFetch::default();
    let mut sent = Vec::new();
    let folder = id(10);
    fetch.send(
        &mut inv,
        AGENT,
        Kind::FolderAndContent,
        folder,
        Vec::new(),
        0,
        &mut sent,
        BACKGROUND_RETRIES,
    );
    fetch.on_reply_at(
        &mut inv,
        0,
        &[],
        Err(FetchError::new(FetchFailure::SupersededCapability, Some(403))),
        now,
    );
    assert!(!inv.too_big.contains(&folder));
    let mut reads = fetch.pump_at(&mut inv, AGENT, now);
    let NetCommand::FetchInventoryTree {
        request, parent, depth, ..
    } = reads.pop().expect("immediate retry")
    else {
        panic!("expected the folder read");
    };
    assert_eq!((parent, depth), (folder, 0));
    assert_eq!(fetch.in_flight[&request].attempt, BACKGROUND_RETRIES);
    fetch.on_reply_at(&mut inv, request, &[], Err(FetchError::new(FetchFailure::TooBig, Some(403))), now);
    assert!(inv.too_big.contains(&folder));
}

#[test]
fn recovery_automatic_folder_reads_cannot_bypass_background_cooldown() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let now = Instant::now();
    let mut fetch = cooling_read(&mut inv, Vec::new(), now);
    for _ in 0..60 {
        inv.request(id(10));
    }
    assert!(inv.queue.is_empty(), "Appearance must not bypass the 15-second backoff");
    assert!(fetch.pump_at(&mut inv, AGENT, now).is_empty());
    assert_eq!(fetch.pump_at(&mut inv, AGENT, now + BACKGROUND_BACKOFF[0]).len(), 1);
}

#[test]
fn a_split_step_consumes_its_mark_without_stealing_a_cooling_retry() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let now = Instant::now();
    let mut fetch = cooling_read(&mut inv, Vec::new(), now);
    let folder = id(10);
    fetch.split.insert(folder);
    fetch.queue.push_back((folder, Kind::FolderAndContent));

    let reads = fetch.pump_at(&mut inv, AGENT, now);
    assert!(reads.is_empty(), "the split step cannot bypass the pending cooldown");
    assert!(!fetch.split.contains(&folder));
    assert!(inv.retrying_folders.contains(&folder));

    let reads = fetch.pump_at(&mut inv, AGENT, now + BACKGROUND_BACKOFF[0]);
    assert!(matches!(&reads[..], [NetCommand::FetchInventoryTree { parent, .. }] if *parent == folder));
}

#[test]
fn recovery_a_recovered_folder_is_not_reissued_by_a_delayed_retry() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let now = Instant::now();
    let mut fetch = cooling_read(&mut inv, Vec::new(), now);
    inv.folders.get_mut(&id(10)).unwrap().state = FetchState::Fetched;
    assert!(fetch.pump_at(&mut inv, AGENT, now + BACKGROUND_BACKOFF[0]).is_empty());
    assert_eq!(inv.folders[&id(10)].state, FetchState::Fetched);
    assert!(fetch.is_idle());
}

#[test]
fn a_reply_after_the_deadline_cannot_complete_its_replacement() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let now = Instant::now();
    let folder = id(10);
    let mut fetch = BackgroundFetch {
        clock: Some(now),
        ..Default::default()
    };
    fetch.send(
        &mut inv,
        AGENT,
        Kind::Recursive,
        folder,
        Vec::new(),
        MAX_FOLDER_DEPTH,
        &mut Vec::new(),
        0,
    );
    let reads = fetch.pump_at(&mut inv, AGENT, now + TREE_DEADLINE);
    let [NetCommand::FetchInventoryTree { request, .. }] = reads.as_slice() else {
        panic!("expected the timed-out read to be split: {reads:?}");
    };
    assert_ne!(*request, 0);
    let snapshot = |item| FolderContents {
        folder_id: folder,
        owner_id: AGENT,
        version: 3,
        folders: Vec::new(),
        items: vec![InvItem {
            id: item,
            parent: folder,
            ..Default::default()
        }],
    };
    assert!(!fetch.on_reply(&mut inv, 0, Ok(vec![snapshot(id(100))])));
    assert_eq!(inv.folders[&folder].state, FetchState::Fetching);
    assert!(!inv.items.contains_key(&id(100)));
    assert!(inv.take_accepted_snapshot_items().is_empty());
    assert!(fetch.on_reply(&mut inv, *request, Ok(vec![snapshot(id(101))])));
    assert_eq!(inv.folders[&folder].state, FetchState::Fetched);
    assert_eq!(inv.take_accepted_snapshot_items(), vec![id(101)]);
}

#[test]
fn recovery_a_subset_retry_only_reads_children_that_still_need_it() {
    let mut inv = skeleton(&[(10, 1, -1), (11, 10, -1), (12, 10, -1)]);
    let now = Instant::now();
    let mut fetch = cooling_read(&mut inv, vec![id(11), id(12)], now);
    inv.folders.get_mut(&id(11)).unwrap().state = FetchState::Fetched;
    let reads = fetch.pump_at(&mut inv, AGENT, now + BACKGROUND_BACKOFF[0]);
    assert!(matches!(&reads[..], [NetCommand::FetchInventoryTree { children, .. }] if children == &[id(12)]));
    assert_eq!(inv.folders[&id(11)].state, FetchState::Fetched);
}

#[test]
fn recovery_a_due_retry_waits_for_an_external_read_instead_of_duplicating_it() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let now = Instant::now();
    let mut fetch = cooling_read(&mut inv, Vec::new(), now);
    inv.folders.get_mut(&id(10)).unwrap().state = FetchState::Fetching;
    assert!(fetch.pump_at(&mut inv, AGENT, now + BACKGROUND_BACKOFF[0]).is_empty());
    assert!(!fetch.is_idle());
    inv.folders.get_mut(&id(10)).unwrap().state = FetchState::Fetched;
    assert!(fetch.pump_at(&mut inv, AGENT, now + BACKGROUND_BACKOFF[0]).is_empty());
    assert!(fetch.is_idle());
}

#[test]
fn recovery_automatic_readers_do_not_restart_failed_or_over_limit_folders() {
    let mut inv = skeleton(&[(10, 1, -1), (11, 1, -1)]);
    inv.folders.get_mut(&id(10)).unwrap().state = FetchState::Failed;
    inv.too_big.insert(id(11));
    for _ in 0..60 {
        inv.request(id(10));
        inv.request(id(11));
    }
    assert!(inv.queue.is_empty());
}

#[test]
fn recovery_region_retry_rearms_only_failed_folders_without_an_existing_owner() {
    let mut inv = skeleton(&[(10, 1, -1), (11, 1, -1), (12, 1, -1)]);
    for id in [id(10), id(11), id(12)] {
        inv.folders.get_mut(&id).unwrap().state = FetchState::Failed;
    }
    inv.retrying_folders.insert(id(11));
    inv.too_big.insert(id(12));
    inv.retry_failed();
    assert_eq!(inv.queue, [(id(10), false)]);
}

const AGENT: Uuid = Uuid::from_u128(2);

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn startup_orphan_discovery_runs_once_with_a_warm_cache_and_retries_only_after_region_change() {
    let mut inv = skeleton(&[]);
    inv.folders.values_mut().for_each(|f| f.state = FetchState::Fetched);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let reads = fetch.pump(&mut inv, AGENT);
    let [NetCommand::FetchInventoryOrphans { request }] = &reads[..] else {
        panic!("only orphan discovery is needed");
    };
    let request = *request;
    assert!(!fetch.is_idle());
    assert!(!fetch.on_orphans_reply(request + 1, true));
    assert!(fetch.pump(&mut inv, AGENT).is_empty());
    assert!(fetch.on_orphans_reply(request, false));
    assert!(fetch.is_idle());
    assert!(fetch.pump(&mut inv, AGENT).is_empty(), "no failed-read loop");
    fetch.resume(&inv);
    let reads = fetch.pump(&mut inv, AGENT);
    let [NetCommand::FetchInventoryOrphans { request: retry }] = &reads[..] else {
        panic!("retry after capabilities change");
    };
    assert!(fetch.on_orphans_reply(*retry, true));
    fetch.resume(&inv);
    assert!(
        fetch.pump(&mut inv, AGENT).is_empty(),
        "successful startup discovery is not repeated per teleport"
    );
    assert!(!fetch.on_orphans_reply(*retry, true), "duplicate replies are obsolete");
}

fn record(n: u128, parent: u128, type_default: i32) -> InvFolder {
    InvFolder {
        id: id(n),
        parent: id(parent),
        name: format!("Folder {n}"),
        type_default,
        version: 1,
        ..Default::default()
    }
}

/// The login skeleton of `tree` (id, parent, type): every folder known,
/// none fetched. Folder 1 is the root, 900 the library's.
fn skeleton(tree: &[(u128, u128, i32)]) -> Inventory {
    let mut inv = Inventory {
        root: id(1),
        lib_root: id(900),
        lib_owner: id(3),
        ..Default::default()
    };
    inv.add_folders(vec![record(1, 0, 8)], false);
    inv.add_folders(vec![record(900, 0, 8)], true);
    inv.add_folders(tree.iter().map(|&(n, parent, kind)| record(n, parent, kind)).collect(), false);
    inv
}

struct Read {
    parent: Uuid,
    children: Vec<Uuid>,
    depth: i32,
}

/// An AIS server over `tree` that returns at most `cap` levels, gives every
/// folder `items` items, and fails the reads `failure` picks with that kind
/// (a 403 for a reply that would be too big, a 503 for a throttled read).
struct Server<'a> {
    tree: &'a [(u128, u128, i32)],
    cap: i32,
    items: usize,
    failure: &'a dyn Fn(&Read) -> Option<FetchFailure>,
}

impl Server<'_> {
    fn subtree(&self, folder: Uuid, depth: i32, out: &mut Vec<FolderContents>) {
        let children: Vec<InvFolder> = self
            .tree
            .iter()
            .filter(|&&(_, parent, _)| id(parent) == folder)
            .map(|&(n, parent, kind)| record(n, parent, kind))
            .collect();
        let next: Vec<Uuid> = children.iter().map(|f| f.id).collect();
        out.push(FolderContents {
            folder_id: folder,
            owner_id: AGENT,
            version: 1,
            folders: children,
            items: (0..self.items)
                .map(|n| InvItem {
                    id: id(1_000_000_000 + folder.as_u128() * 1_000 + n as u128),
                    parent: folder,
                    name: format!("Item {n}"),
                    asset_type: 6,
                    inv_type: 6,
                    ..Default::default()
                })
                .collect(),
        });
        if depth > 0 {
            for child in next {
                self.subtree(child, depth - 1, out);
            }
        }
    }

    fn answer(&self, read: &Read) -> Result<Vec<FolderContents>, FetchError> {
        if let Some(kind) = (self.failure)(read) {
            let status = match kind {
                FetchFailure::TooBig | FetchFailure::SupersededCapability => 403,
                FetchFailure::Transient => 503,
                FetchFailure::Fatal => 500,
            };
            return Err(FetchError::new(kind, Some(status)));
        }
        let mut out = Vec::new();
        if read.children.is_empty() {
            self.subtree(read.parent, read.depth.min(self.cap), &mut out);
        } else {
            for child in &read.children {
                self.subtree(*child, (read.depth - 1).min(self.cap), &mut out);
            }
        }
        Ok(out)
    }
}

/// Run the fetch to its end against `server`; every read, in order.
fn run(inv: &mut Inventory, server: &Server) -> Vec<Read> {
    run_with(BackgroundFetch::default(), inv, server)
}

fn run_with(fetch: BackgroundFetch, inv: &mut Inventory, server: &Server) -> Vec<Read> {
    run_keeping_fetch(fetch, inv, server).0
}

fn run_keeping_fetch(fetch: BackgroundFetch, inv: &mut Inventory, server: &Server) -> (Vec<Read>, BackgroundFetch) {
    run_from(fetch, inv, server, Instant::now())
}

/// [`run_keeping_fetch`] with its clock starting at `start`.
fn run_from(mut fetch: BackgroundFetch, inv: &mut Inventory, server: &Server, start: Instant) -> (Vec<Read>, BackgroundFetch) {
    fetch.start(inv);
    let mut reads = Vec::new();
    let mut now = start;
    for _ in 0..1000 {
        now += std::time::Duration::from_secs(60);
        let commands = fetch.pump_at(inv, AGENT, now);
        let max = if fetch.legacy[0] { LEGACY_IN_FLIGHT } else { MAX_IN_FLIGHT };
        // the end-of-frame legacy batches may each add one read
        assert!(fetch.in_flight.len() <= max + 2, "the pool bounds the reads in flight");
        if commands.is_empty() && fetch.is_idle() {
            return (reads, fetch);
        }
        for command in commands {
            if let NetCommand::FetchInventoryOrphans { request } = command {
                assert!(fetch.on_orphans_reply(request, true));
                continue;
            }
            let NetCommand::FetchInventoryTree {
                request,
                parent,
                children,
                depth,
                owner,
                library,
            } = command
            else {
                panic!("only tree reads");
            };
            assert_eq!(owner, if library { inv.lib_owner } else { AGENT });
            assert_eq!(library, inv.folders[&parent].library);
            assert!(children.iter().all(|c| inv.folders[c].library == library), "one tree per read");
            let read = Read { parent, children, depth };
            let (stale, outcome) = match server.answer(&read) {
                Ok(contents) => (inv.apply(contents), Ok(())),
                Err(error) => (Vec::new(), Err(error)),
            };
            fetch.on_reply_at(inv, request, &stale, outcome, now);
            reads.push(read);
        }
    }
    panic!(
        "the background fetch never ended: queue {:?}, {} in flight, {} retries, unfetched {:?}",
        fetch.queue,
        fetch.in_flight.len(),
        fetch.retries.len(),
        unfetched(inv)
    );
}

fn unfetched(inv: &Inventory) -> Vec<Uuid> {
    let mut left: Vec<Uuid> = inv
        .folders
        .iter()
        .filter(|(_, f)| f.state != FetchState::Fetched)
        .map(|(id, _)| *id)
        .collect();
    left.sort();
    left
}

/// 45 top folders, each with a folder holding another one.
fn wide_tree() -> Vec<(u128, u128, i32)> {
    let mut tree = Vec::new();
    for n in 0..45 {
        let top = 10 + n;
        tree.push((top, 1, -1));
        tree.push((1000 + n, top, -1));
        tree.push((2000 + n, 1000 + n, -1));
    }
    tree.push((901, 900, -1));
    tree
}

#[test]
fn whole_inventory_loads_in_a_few_batched_recursive_reads() {
    let tree = wide_tree();
    let mut inv = skeleton(&tree);
    let reads = run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 0,
            failure: &|_| None,
        },
    );
    assert!(unfetched(&inv).is_empty());
    // the root alone, then its 45 folders by 20 with their subtrees; the
    // library from its root
    assert_eq!((reads[0].parent, reads[0].children.len(), reads[0].depth), (id(1), 0, 0));
    let batches: Vec<usize> = reads
        .iter()
        .filter(|r| r.parent == id(1) && !r.children.is_empty())
        .map(|r| r.children.len())
        .collect();
    assert_eq!(batches, [20, 20, 5]);
    assert!(
        reads
            .iter()
            .all(|r| r.parent == id(1) && r.depth == 0 || r.depth == MAX_FOLDER_DEPTH)
    );
    assert!(reads.iter().any(|r| r.parent == id(900) && r.children.is_empty()));
    assert_eq!(reads.len(), 5, "one read per folder would be {}", inv.folders.len());
}

#[test]
fn a_stale_folder_reply_is_refetched_instead_of_left_unloaded() {
    let tree = [(10, 1, -1)];
    let mut inv = skeleton(&tree);
    let target = inv.folders.get_mut(&id(10)).expect("folder");
    target.state = FetchState::Fetched;
    target.info.version = 5;
    // A delayed snapshot older than the contents already confirmed locally.
    inv.apply(vec![FolderContents {
        folder_id: id(10),
        owner_id: AGENT,
        version: 4,
        folders: Vec::new(),
        items: Vec::new(),
    }]);
    assert_eq!(
        inv.folders[&id(10)].state,
        FetchState::Fetching,
        "a stale snapshot asks for the folder again"
    );
    assert_eq!(inv.queue, [(id(10), false)], "the refetch is queued");
}

#[test]
fn marketplace_listings_are_left_to_their_window() {
    let tree = [(10, 1, -1), (11, 1, MARKETPLACE_LISTINGS), (12, 11, -1)];
    let mut inv = skeleton(&tree);
    run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 0,
            failure: &|_| None,
        },
    );
    assert_eq!(unfetched(&inv), [id(11), id(12)]);
}

#[test]
fn what_the_depth_limit_leaves_is_fetched_by_the_verify_walk() {
    // a chain deeper than what the server returns at once
    let tree: Vec<(u128, u128, i32)> = (10..20).map(|n| (n, if n == 10 { 1 } else { n - 1 }, -1)).collect();
    let mut inv = skeleton(&tree);
    let reads = run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: 2,
            items: 0,
            failure: &|_| None,
        },
    );
    assert!(unfetched(&inv).is_empty());
    // what each read left is read by batches from its parent, never one
    // read a folder (only the two roots are read alone)
    let alone: Vec<Uuid> = reads
        .iter()
        .filter(|r| r.children.is_empty() && r.depth > 0)
        .map(|r| r.parent)
        .collect();
    assert_eq!(alone, [id(900)], "{alone:?}");
    assert!(
        reads.iter().any(|r| r.children == [id(13)]),
        "the chain goes on from where the limit left it"
    );
}

#[test]
fn reads_too_big_fall_back_to_the_folder_then_its_folders() {
    // folder 10 is too big to read with its subtree, its folders are not
    let tree = [(10, 1, -1), (11, 1, -1), (20, 10, -1), (21, 10, -1), (30, 20, -1)];
    let mut inv = skeleton(&tree);
    let big = |r: &Read| {
        (r.depth > 0 && (r.parent == id(10) && r.children.is_empty() || r.children.contains(&id(10)))).then_some(FetchFailure::TooBig)
    };
    let reads = run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 0,
            failure: &big,
        },
    );
    assert!(unfetched(&inv).is_empty());
    let shapes: Vec<(Uuid, Vec<Uuid>, i32)> = reads.iter().skip(1).map(|r| (r.parent, r.children.clone(), r.depth)).collect();
    let deep = MAX_FOLDER_DEPTH;
    assert_eq!(
        shapes,
        [
            (id(900), vec![], deep),             // the library
            (id(1), vec![id(10), id(11)], deep), // the batch fails…
            (id(10), vec![], deep),              // …each folder on its own
            (id(11), vec![], deep),
            (id(10), vec![], 0),                  // 10 alone
            (id(10), vec![id(20), id(21)], deep), // then its folders
        ]
    );
}

#[test]
fn a_folder_the_user_opened_is_not_read_twice() {
    let tree = [(10, 1, -1), (11, 1, -1), (20, 11, -1)];
    let mut inv = skeleton(&tree);
    let server = Server {
        tree: &tree,
        cap: 0,
        items: 0,
        failure: &|_| None,
    };
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let answer = |inv: &mut Inventory, fetch: &mut BackgroundFetch, commands: Vec<NetCommand>| {
        let mut batches = Vec::new();
        for command in commands {
            if let NetCommand::FetchInventoryOrphans { request } = command {
                assert!(fetch.on_orphans_reply(request, true));
                continue;
            }
            if let NetCommand::FetchInventoryTree {
                request,
                parent,
                children,
                depth,
                ..
            } = command
            {
                batches.push(children.clone());
                let read = Read { parent, children, depth };
                let (stale, outcome) = match server.answer(&read) {
                    Ok(contents) => (inv.apply(contents), Ok(())),
                    Err(error) => (Vec::new(), Err(error)),
                };
                fetch.on_reply_at(inv, request, &stale, outcome, Instant::now());
            }
        }
        batches
    };
    let first = fetch.pump(&mut inv, AGENT);
    answer(&mut inv, &mut fetch, first);
    // the user opens folder 11 before the batch leaves (Inventory::request)
    inv.request(id(11));
    assert_eq!(inv.queue, [(id(11), false)]);
    let commands = fetch.pump(&mut inv, AGENT);
    let batches = answer(&mut inv, &mut fetch, commands);
    assert!(batches.contains(&vec![id(10)]), "{batches:?}");
    // its listing arrives: the walk goes on below it
    let read = Read {
        parent: id(11),
        children: Vec::new(),
        depth: 0,
    };
    if let Ok(contents) = server.answer(&read) {
        inv.apply(contents);
    }
    for _ in 0..10 {
        let commands = fetch.pump(&mut inv, AGENT);
        answer(&mut inv, &mut fetch, commands);
    }
    assert!(unfetched(&inv).is_empty());
    assert!(fetch.is_idle());
}

#[test]
fn a_root_already_read_at_login_still_loads_by_batches() {
    let tree = wide_tree();
    let mut inv = skeleton(&tree);
    // the login's own read of the root (Inventory::request) arrived first
    let server = Server {
        tree: &tree,
        cap: MAX_FOLDER_DEPTH,
        items: 0,
        failure: &|_| None,
    };
    inv.apply(
        server
            .answer(&Read {
                parent: id(1),
                children: Vec::new(),
                depth: 0,
            })
            .unwrap_or_default(),
    );
    let reads = run(&mut inv, &server);
    assert!(unfetched(&inv).is_empty());
    let batches: Vec<usize> = reads.iter().filter(|r| r.parent == id(1)).map(|r| r.children.len()).collect();
    assert_eq!(batches, [20, 20, 5]);
    assert_eq!(reads.len(), 4);
}

/// A grid without AIS: `FetchInventoryDescendents2` reads one level of each
/// folder it is given, whatever the depth.
fn legacy_server(tree: &[(u128, u128, i32)]) -> Server<'_> {
    Server {
        tree,
        cap: 0,
        items: 0,
        failure: &|_| None,
    }
}

fn legacy_fetch() -> BackgroundFetch {
    let mut fetch = BackgroundFetch::default();
    fetch.set_legacy(true, true);
    fetch
}

#[test]
fn without_ais_the_whole_tree_loads_level_by_level_in_batches_of_ten() {
    let tree = wide_tree();
    let mut inv = skeleton(&tree);
    let reads = run_with(legacy_fetch(), &mut inv, &legacy_server(&tree));
    assert!(unfetched(&inv).is_empty(), "{:?}", unfetched(&inv));
    assert!(reads.iter().all(|r| r.depth == 0), "the legacy capability reads one level");
    assert!(reads.iter().all(|r| r.children.len() <= LEGACY_BATCH));
    // root alone, 135 + 2 folders by 10 (any parent), never one read each
    assert_eq!((reads[0].parent, reads[0].children.len()), (id(1), 0));
    let folders: usize = reads.iter().map(|r| r.children.len().max(1)).sum();
    assert_eq!(folders, inv.folders.len(), "each folder read once");
    assert!(reads.len() <= 20, "{} reads", reads.len());
}

#[test]
fn without_ais_a_failed_batch_is_read_folder_by_folder() {
    // folder 11 makes every read holding it fail: the others still load
    let tree = [(10, 1, -1), (11, 1, -1), (12, 1, -1), (20, 10, -1), (21, 11, -1)];
    let mut inv = skeleton(&tree);
    let fails = |r: &Read| (r.parent == id(11) || r.children.contains(&id(11))).then_some(FetchFailure::TooBig);
    let reads = run_with(
        legacy_fetch(),
        &mut inv,
        &Server {
            tree: &tree,
            cap: 0,
            items: 0,
            failure: &fails,
        },
    );
    // 21 is unknown to the viewer until 11 is read
    assert_eq!(unfetched(&inv), [id(11)]);
    assert_eq!(
        reads.iter().filter(|r| fails(r).is_some()).count(),
        2,
        "the batch, then 11 alone, then no loop"
    );
}

#[test]
fn without_ais_marketplace_listings_are_left_to_their_window() {
    let tree = [(10, 1, -1), (11, 1, MARKETPLACE_LISTINGS), (12, 11, -1)];
    let mut inv = skeleton(&tree);
    run_with(legacy_fetch(), &mut inv, &legacy_server(&tree));
    assert_eq!(unfetched(&inv), [id(11), id(12)]);
}

#[test]
fn without_ais_only_for_the_library_the_inventory_keeps_its_recursive_reads() {
    let tree = wide_tree();
    let mut inv = skeleton(&tree);
    let mut fetch = BackgroundFetch::default();
    fetch.set_legacy(false, true);
    let reads = run_with(
        fetch,
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 0,
            failure: &|_| None,
        },
    );
    assert!(unfetched(&inv).is_empty());
    let library: Vec<&Read> = reads.iter().filter(|r| inv.folders[&r.parent].library).collect();
    assert!(library.iter().all(|r| r.depth == 0), "library level by level");
    assert!(
        reads.iter().any(|r| !inv.folders[&r.parent].library && r.depth > 0),
        "inventory still through AIS"
    );
}

#[test]
fn a_large_inventory_recovers_from_a_transient_failed_read() {
    let tree: Vec<(u128, u128, i32)> = (0..400).map(|n| (1000 + n, 1, -1)).collect();
    let mut inv = skeleton(&tree);
    let attempts = std::sync::atomic::AtomicU32::new(0);
    let failure = |r: &Read| {
        let batch = r.depth > 0 && !r.children.is_empty();
        (batch && attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 2).then_some(FetchFailure::Transient)
    };
    let reads = run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 100,
            failure: &failure,
        },
    );
    assert!(unfetched(&inv).is_empty(), "{:?}", unfetched(&inv));
    assert_eq!(inv.item_count(), (tree.len() + 2) * 100, "every folder's items survived");
    let batches = reads.iter().filter(|r| r.parent == id(1) && !r.children.is_empty()).count();
    assert!(batches >= 3, "the failed batch is asked again, then the rest: {batches}");
}

#[test]
fn recovery_a_240k_inventory_keeps_all_items_after_multiple_failed_batches() {
    let tree: Vec<(u128, u128, i32)> = (0..2400).map(|n| (1000 + n, 1, -1)).collect();
    let mut inv = skeleton(&tree);
    let attempts = std::sync::atomic::AtomicU32::new(0);
    let failure = |read: &Read| {
        let batch = read.depth > 0 && !read.children.is_empty();
        (batch && attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10).then_some(FetchFailure::Transient)
    };
    run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 100,
            failure: &failure,
        },
    );
    assert!(unfetched(&inv).is_empty());
    assert_eq!(inv.item_count(), 240_200);
    assert!(inv.retrying_folders.is_empty());
}

#[test]
fn a_folder_too_big_even_at_depth_zero_is_reported_and_its_subfolders_still_load() {
    let tree = [(10, 1, -1), (11, 1, -1), (12, 11, -1), (13, 11, -1)];
    let mut inv = skeleton(&tree);
    let too_big =
        |r: &Read| (r.children.contains(&id(11)) || (r.parent == id(11) && r.children.is_empty())).then_some(FetchFailure::TooBig);
    let (_reads, mut fetch) = run_keeping_fetch(
        BackgroundFetch::default(),
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            items: 10,
            failure: &too_big,
        },
    );
    assert!(inv.too_big.contains(&id(11)), "the over-limit folder is recorded");
    assert_eq!(inv.folders[&id(11)].state, FetchState::Failed);
    assert!(inv.folders[&id(11)].items.is_empty(), "its direct items cannot be read");
    assert_eq!(inv.folders[&id(10)].state, FetchState::Fetched, "the rest of the inventory loads");
    assert_eq!(inv.folders[&id(12)].state, FetchState::Fetched, "its subfolders still load");
    assert_eq!(inv.folders[&id(13)].state, FetchState::Fetched);
    assert_eq!(
        fetch.take_too_big(),
        (vec![id(11)], Some(403)),
        "the alert names the folder once, with the refusal's status"
    );
    assert!(fetch.take_too_big().0.is_empty());
}

/// A read that never gets an answer frees its slot after the deadline and
/// is asked again: lost replies never stall the walk for the session.
#[test]
fn an_unanswered_read_frees_its_slot_and_is_asked_again() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let start = Instant::now();
    let first = fetch.pump_at(&mut inv, AGENT, start);
    let sent = first.iter().filter(|c| matches!(c, NetCommand::FetchInventoryTree { .. })).count();
    assert!(sent > 0 && fetch.in_flight.len() == sent);
    // no reply ever comes: past the deadline the slots are free again, and
    // each read is asked again (a subtree at once in parts, a folder alone
    // after its retry)
    let deadline = start + READ_DEADLINE;
    let mut again = fetch.pump_at(&mut inv, AGENT, deadline);
    assert!(fetch.in_flight.values().all(|r| r.sent == deadline), "the old slots are freed");
    again.extend(fetch.pump_at(&mut inv, AGENT, deadline + BACKGROUND_BACKOFF[0]));
    let asked = again.iter().filter(|c| matches!(c, NetCommand::FetchInventoryTree { .. })).count();
    assert!(asked >= sent, "asked again: {asked} of {sent}");
}

/// A folder the walk gave up on is asked again later in the session, a few
/// times, never in a loop; an idle walk with a folder left unread reads it.
#[test]
fn failed_and_unread_folders_are_asked_again_later() {
    let tree = [(10, 1, -1), (11, 1, -1)];
    let mut inv = skeleton(&tree);
    let failing = std::cell::Cell::new(true);
    let failure =
        |read: &Read| (failing.get() && (read.parent == id(11) || read.children.contains(&id(11)))).then_some(FetchFailure::Fatal);
    let server = Server {
        tree: &tree,
        cap: 50,
        items: 1,
        failure: &failure,
    };
    let (_, mut fetch) = run_keeping_fetch(BackgroundFetch::default(), &mut inv, &server);
    assert_eq!(inv.folders[&id(11)].state, FetchState::Failed);
    // the idle walk's next look asks for it again, then waits its round
    // (the first look starts the round's clock, the round comes after it)
    let later = Instant::now() + std::time::Duration::from_secs(100_000);
    assert!(fetch.pump_at(&mut inv, AGENT, later).is_empty(), "not at the first look");
    let later = later + FAILED_AGAIN;
    let again = fetch.pump_at(&mut inv, AGENT, later);
    assert!(
        again.iter().any(|c| matches!(c, NetCommand::FetchInventoryTree { .. })),
        "asked again"
    );
    assert_eq!(fetch.failed_rounds, 1);
    // (its answer, still a failure)
    for command in again {
        if let NetCommand::FetchInventoryTree {
            request,
            parent,
            children,
            depth,
            ..
        } = command
        {
            let read = Read { parent, children, depth };
            let outcome = server.answer(&read).map(|contents| {
                inv.apply(contents);
            });
            fetch.on_reply_at(&mut inv, request, &[], outcome, later);
        }
    }
    // the server gives it now: the next round reads it
    failing.set(false);
    run_from(fetch, &mut inv, &server, later + FAILED_AGAIN);
    assert_eq!(inv.folders[&id(11)].state, FetchState::Fetched);
    // an idle walk with a folder left unread (a reply lost) reads it
    let mut inv = skeleton(&tree);
    let (_, mut fetch) = run_keeping_fetch(BackgroundFetch::default(), &mut inv, &server);
    if let Some(f) = inv.folders.get_mut(&id(10)) {
        f.state = FetchState::Unknown;
    }
    fetch.swept = None;
    fetch.swept_clean = None;
    let reads = run_with(fetch, &mut inv, &server);
    assert!(!reads.is_empty(), "the watchdog asked again");
    assert_eq!(inv.folders[&id(10)].state, FetchState::Fetched);
}

/// A folder its parent's read never returns is asked again a few times a
/// session, never every 30 s for the session.
#[test]
fn an_unread_folder_under_a_read_parent_is_asked_again_a_few_times_only() {
    let tree = [(10, 1, -1), (11, 1, -1)];
    let mut inv = skeleton(&tree);
    let failure = |_: &Read| None;
    let server = Server {
        tree: &tree,
        cap: 50,
        items: 1,
        failure: &failure,
    };
    let (_, mut fetch) = run_keeping_fetch(BackgroundFetch::default(), &mut inv, &server);
    let mut now = Instant::now();
    let mut asked = 0;
    for _ in 0..UNREAD_ROUNDS + 4 {
        // (the reply never holds it: still unread after each answer)
        if let Some(f) = inv.folders.get_mut(&id(10)) {
            f.state = FetchState::Unknown;
        }
        now += WATCHDOG;
        let mut commands = fetch.pump_at(&mut inv, AGENT, now);
        if commands.iter().any(|c| matches!(c, NetCommand::FetchInventoryTree { .. })) {
            asked += 1;
        }
        // (a recursive read can queue more reads: answer them all, same round)
        for _ in 0..100 {
            if commands.is_empty() {
                break;
            }
            for command in commands {
                if let NetCommand::FetchInventoryTree {
                    request,
                    parent,
                    children,
                    depth,
                    ..
                } = command
                {
                    let read = Read { parent, children, depth };
                    let outcome = server.answer(&read).map(|contents| {
                        inv.apply(contents);
                    });
                    fetch.on_reply_at(&mut inv, request, &[], outcome, now);
                }
            }
            commands = if fetch.is_idle() {
                Vec::new()
            } else {
                fetch.pump_at(&mut inv, AGENT, now)
            };
        }
        assert!(fetch.is_idle(), "every read answered");
    }
    assert_eq!(asked, UNREAD_ROUNDS as usize, "asked a few times, then left");
}

/// A subtree read that times out is read in parts at once, never repeated
/// as is (onAISFolderCalback's fallback); a folder left fetching with no
/// read of the walk is read again once the deadline is over.
#[test]
fn a_timed_out_subtree_read_falls_back_at_once_and_lost_folders_are_read_again() {
    let mut inv = skeleton(&[(10, 1, -1), (11, 10, -1)]);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let start = Instant::now();
    let mut now = start;
    // read the root alone, then its subtree batch, which times out
    let mut timed_out = false;
    for _ in 0..10 {
        now += std::time::Duration::from_secs(1);
        for command in fetch.pump_at(&mut inv, AGENT, now) {
            if let NetCommand::FetchInventoryOrphans { request } = command {
                fetch.on_orphans_reply(request, true);
                continue;
            }
            let NetCommand::FetchInventoryTree {
                request,
                parent,
                children,
                depth,
                ..
            } = command
            else {
                continue;
            };
            if depth > 0 && !timed_out {
                timed_out = true;
                fetch.on_reply_at(&mut inv, request, &[], Err(FetchError::timeout()), now);
                assert!(fetch.retries.is_empty(), "never repeated as is");
                assert!(
                    fetch.queue.iter().any(|(_, k)| *k == Kind::FolderAndContent),
                    "read in parts at once"
                );
                continue;
            }
            let server = Server {
                tree: &[(10, 1, -1), (11, 10, -1)],
                cap: 50,
                items: 0,
                failure: &|_| None,
            };
            let read = Read { parent, children, depth };
            let outcome = server.answer(&read).map(|contents| {
                inv.apply(contents);
            });
            fetch.on_reply_at(&mut inv, request, &[], outcome, now);
        }
    }
    assert!(timed_out);
    assert!(unfetched(&inv).is_empty(), "everything read in parts");
    // a folder some other read left fetching is read again after the deadline
    inv.folders.get_mut(&id(11)).expect("folder").state = FetchState::Fetching;
    fetch.swept = None;
    fetch.swept_clean = None;
    fetch.pump_at(&mut inv, AGENT, now);
    assert_eq!(inv.folders[&id(11)].state, FetchState::Fetching, "not yet");
    fetch.swept = None;
    fetch.swept_clean = None;
    let later = now + READ_DEADLINE;
    let commands = fetch.pump_at(&mut inv, AGENT, later);
    assert!(
        commands.iter().any(|c| matches!(c, NetCommand::FetchInventoryTree { .. })),
        "asked again"
    );
}

/// The last heavy batch that a gateway gives up on (504) is read in parts
/// at once, never asked again as is after a wait; a deep read left
/// unanswered is freed after its own, shorter deadline.
#[test]
fn a_gateway_timeout_on_the_last_deep_batch_splits_it() {
    let tree = [(10, 1, -1), (11, 1, -1), (20, 10, -1), (21, 11, -1)];
    let mut inv = skeleton(&tree);
    // every deep read of 10 or 11 times out at the gateway
    let gateway = |r: &Read| {
        (r.depth > 0 && (r.children.contains(&id(10)) || r.children.contains(&id(11)) || r.parent == id(10) || r.parent == id(11)))
            .then_some(FetchFailure::Transient)
    };
    let server = Server {
        tree: &tree,
        cap: MAX_FOLDER_DEPTH,
        items: 1,
        failure: &gateway,
    };
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let mut now = Instant::now();
    let mut split = false;
    for _ in 0..100 {
        // (one second a round: a backoff of 15 s would show as many rounds
        // with nothing sent)
        now += std::time::Duration::from_secs(1);
        for command in fetch.pump_at(&mut inv, AGENT, now) {
            match command {
                NetCommand::FetchInventoryOrphans { request } => {
                    fetch.on_orphans_reply(request, true);
                }
                NetCommand::FetchInventoryTree {
                    request,
                    parent,
                    children,
                    depth,
                    ..
                } => {
                    let read = Read { parent, children, depth };
                    let outcome = match server.answer(&read) {
                        Ok(contents) => {
                            inv.apply(contents);
                            Ok(())
                        }
                        // (the server's 503 stands for the gateway's 504)
                        Err(_) => {
                            split = true;
                            Err(FetchError::new(FetchFailure::Transient, Some(504)))
                        }
                    };
                    fetch.on_reply_at(&mut inv, request, &[], outcome, now);
                    assert!(fetch.retries.is_empty(), "never waits to ask the same deep read again");
                }
                _ => {}
            }
        }
        if fetch.is_idle() && unfetched(&inv).is_empty() {
            break;
        }
    }
    assert!(split);
    assert!(unfetched(&inv).is_empty(), "read in parts: {:?}", unfetched(&inv));
    // a deep read never answered is freed after TREE_DEADLINE
    let mut inv = skeleton(&[(10, 1, -1)]);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let start = Instant::now();
    fetch.pump_at(&mut inv, AGENT, start);
    let deep: Vec<u64> = fetch.in_flight.iter().filter(|(_, r)| r.depth > 0).map(|(k, _)| *k).collect();
    assert!(!deep.is_empty());
    fetch.pump_at(&mut inv, AGENT, start + TREE_DEADLINE);
    assert!(
        deep.iter().all(|k| !fetch.in_flight.contains_key(k)),
        "freed after its own deadline"
    );
}

/// A folder the user's own read holds, never answered, is read again after
/// the deadline: it never keeps the walk busy for the session.
#[test]
fn a_users_read_never_answered_is_asked_again() {
    let mut inv = skeleton(&[(10, 1, -1)]);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    // the user's read took folder 10 and never comes back
    inv.folders.get_mut(&id(10)).expect("folder").state = FetchState::Fetching;
    fetch.waiting.push((id(10), Kind::Recursive));
    let start = Instant::now();
    fetch.pump_at(&mut inv, AGENT, start);
    assert_eq!(inv.folders[&id(10)].state, FetchState::Fetching, "not yet");
    fetch.pump_at(&mut inv, AGENT, start + READ_DEADLINE);
    assert_eq!(inv.folders[&id(10)].state, FetchState::Unknown, "given up past the deadline");
    fetch.pump_at(&mut inv, AGENT, start + READ_DEADLINE);
    assert!(
        fetch.pending.contains(&id(10)) || fetch.in_flight.values().any(|r| r.folder == id(10) || r.children.contains(&id(10))),
        "asked again by the walk"
    );
}

/// A transport error after the HTTP layer's tries (a connection refused or
/// reset, no status) is no timeout: the subtree read keeps the walk's
/// backoff, never split into its folders at once; a real timeout is.
#[test]
fn a_transport_error_keeps_the_backoff_and_only_a_timeout_splits() {
    for (error, splits) in [
        (FetchError::new(FetchFailure::Transient, None), false),
        (FetchError::timeout(), true),
    ] {
        let mut inv = skeleton(&[(10, 1, -1), (11, 1, -1)]);
        inv.folders.get_mut(&id(1)).expect("root").state = FetchState::Fetched;
        let mut fetch = BackgroundFetch::default();
        fetch.start(&inv);
        let now = Instant::now();
        fetch.pump_at(&mut inv, AGENT, now);
        let (&request, read) = fetch
            .in_flight
            .iter()
            .find(|(_, r)| r.depth > 0 && r.folder == id(1))
            .expect("the root's subtree batch");
        let targets = read.children.clone();
        fetch.on_reply_at(&mut inv, request, &[], Err(error), now);
        assert_eq!(fetch.retries.is_empty(), splits, "{error}");
        assert_eq!(
            targets.iter().all(|t| fetch.queue.contains(&(*t, Kind::FolderAndContent))),
            splits,
            "{error}"
        );
    }
}

/// A timed-out batch's folders are read in parts first: the parent's batch
/// still queued never sends them deep again before their own reads ran.
#[test]
fn a_split_batch_is_never_batched_again_before_its_parts() {
    // (more folders than the slots' batches: the root's batch read stays
    // queued after the first pump)
    let tree: Vec<(u128, u128, i32)> = (10..510).map(|n| (n, 1, -1)).collect();
    let mut inv = skeleton(&tree);
    inv.folders.get_mut(&id(1)).expect("root").state = FetchState::Fetched;
    let server = Server {
        tree: &tree,
        cap: 50,
        items: 0,
        failure: &|_| None,
    };
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let mut now = Instant::now();
    let mut commands = fetch.pump_at(&mut inv, AGENT, now);
    assert!(
        fetch.queue.contains(&(id(1), Kind::ContentRecursive)),
        "the root's next batch is still queued when the split happens"
    );
    let (&request, read) = fetch
        .in_flight
        .iter()
        .find(|(_, r)| r.depth > 0 && r.folder == id(1) && r.children.len() == BATCH)
        .expect("a full batch of the root's folders");
    let split: Vec<Uuid> = read.children.clone();
    commands.retain(|c| !matches!(c, NetCommand::FetchInventoryTree { request: r, .. } if *r == request));
    fetch.on_reply_at(&mut inv, request, &[], Err(FetchError::timeout()), now);
    // the walk goes on, every other read answered
    for _ in 0..200 {
        for command in std::mem::take(&mut commands) {
            match command {
                NetCommand::FetchInventoryOrphans { request } => {
                    fetch.on_orphans_reply(request, true);
                }
                NetCommand::FetchInventoryTree {
                    request,
                    parent,
                    children,
                    depth,
                    ..
                } => {
                    assert!(
                        depth == 0
                            || children
                                .iter()
                                .all(|c| !split.contains(c) || inv.folders[c].state == FetchState::Fetched),
                        "a split folder was batched deep again before its own read: {children:?}"
                    );
                    let read = Read { parent, children, depth };
                    let outcome = server.answer(&read).map(|contents| {
                        inv.apply(contents);
                    });
                    fetch.on_reply_at(&mut inv, request, &[], outcome, now);
                }
                _ => {}
            }
        }
        if fetch.is_idle() {
            break;
        }
        now += std::time::Duration::from_secs(1);
        commands = fetch.pump_at(&mut inv, AGENT, now);
    }
    assert!(unfetched(&inv).is_empty(), "{:?}", unfetched(&inv));
    assert!(fetch.split.is_empty(), "no split mark left behind");
}

/// A folder the user's own read takes after the walk swept clean is still
/// timed: never answered, it is read again after the deadline.
#[test]
fn a_users_read_after_a_clean_sweep_is_still_timed() {
    let tree = [(10, 1, -1), (11, 10, -1)];
    let mut inv = skeleton(&tree);
    let server = Server {
        tree: &tree,
        cap: 50,
        items: 1,
        failure: &|_| None,
    };
    let (_, mut fetch) = run_keeping_fetch(BackgroundFetch::default(), &mut inv, &server);
    let start = fetch.clock.expect("pumped") + WATCHDOG;
    fetch.pump_at(&mut inv, AGENT, start);
    assert!(fetch.swept_clean.is_some(), "swept clean");
    // the user's read: queued, taken by the app after the pump, never answered
    inv.folders.get_mut(&id(11)).expect("folder").state = FetchState::Unknown;
    inv.request(id(11));
    fetch.pump_at(&mut inv, AGENT, start + WATCHDOG);
    inv.queue.clear();
    fetch.pump_at(&mut inv, AGENT, start + WATCHDOG * 2);
    assert!(fetch.pump_at(&mut inv, AGENT, start + WATCHDOG * 3).is_empty(), "not yet");
    fetch.pump_at(&mut inv, AGENT, start + WATCHDOG * 3 + READ_DEADLINE);
    assert!(
        fetch.pending.contains(&id(11)) || fetch.queue.iter().any(|(f, _)| *f == id(11)),
        "asked again by the walk past the deadline"
    );
}

/// The first round for failed folders comes a round's time after the walk
/// left them, not at the first idle look.
#[test]
fn failed_folders_wait_a_round_before_they_are_asked_again() {
    let tree = [(10, 1, -1), (11, 1, -1)];
    let mut inv = skeleton(&tree);
    let failure = |read: &Read| (read.parent == id(11) || read.children.contains(&id(11))).then_some(FetchFailure::Fatal);
    let server = Server {
        tree: &tree,
        cap: 50,
        items: 1,
        failure: &failure,
    };
    let (_, mut fetch) = run_keeping_fetch(BackgroundFetch::default(), &mut inv, &server);
    assert_eq!(inv.folders[&id(11)].state, FetchState::Failed);
    // (the run's own clock: its last pump)
    let start = fetch.clock.expect("pumped");
    let first = fetch.pump_at(&mut inv, AGENT, start + WATCHDOG);
    assert!(first.is_empty(), "not at the first idle look");
    assert!(fetch.swept.is_some());
    assert!(fetch.pump_at(&mut inv, AGENT, start + WATCHDOG * 3).is_empty(), "nor 30 s later");
    assert_eq!(fetch.failed_rounds, 0);
    let due = fetch.pump_at(&mut inv, AGENT, start + WATCHDOG + FAILED_AGAIN);
    assert!(due.iter().any(|c| matches!(c, NetCommand::FetchInventoryTree { .. })));
    assert_eq!(fetch.failed_rounds, 1);
}

/// Once everything is read, the idle walk no longer walks the whole tree
/// every 30 s: it looks again only when the inventory changed.
#[test]
fn a_loaded_walk_sweeps_again_only_after_a_change() {
    let tree = wide_tree();
    let mut inv = skeleton(&tree);
    let server = Server {
        tree: &tree,
        cap: 50,
        items: 1,
        failure: &|_| None,
    };
    let (_, mut fetch) = run_keeping_fetch(BackgroundFetch::default(), &mut inv, &server);
    assert!(unfetched(&inv).is_empty());
    let start = fetch.clock.expect("pumped") + WATCHDOG;
    fetch.pump_at(&mut inv, AGENT, start);
    assert!(fetch.swept_clean.is_some(), "nothing left");
    let swept = fetch.swept;
    fetch.pump_at(&mut inv, AGENT, start + WATCHDOG);
    assert_eq!(fetch.swept, swept, "no new sweep while nothing changed");
    inv.generation += 1;
    fetch.pump_at(&mut inv, AGENT, start + WATCHDOG * 2);
    assert_ne!(fetch.swept, swept, "a change brings the sweep back");
}

/// The lost-items request is sent once a session, as findLostItems is; one
/// never answered is given up, shown in N4, and sent again only at the next
/// region's capabilities.
#[test]
fn an_unanswered_lost_items_request_is_given_up_until_the_next_region() {
    let mut inv = skeleton(&[]);
    inv.folders.values_mut().for_each(|f| f.state = FetchState::Fetched);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let start = Instant::now();
    let first = fetch.pump_at(&mut inv, AGENT, start);
    assert!(matches!(first[..], [NetCommand::FetchInventoryOrphans { .. }]));
    assert!(fetch.report(&inv, start).contains("lost items: asked, waiting"));
    let later = start + READ_DEADLINE;
    assert!(fetch.pump_at(&mut inv, AGENT, later).is_empty(), "given up, not asked again");
    assert!(fetch.report(&inv, later).contains("asked again at the next region change"));
    fetch.resume(&inv);
    assert!(
        fetch
            .pump_at(&mut inv, AGENT, later)
            .iter()
            .any(|c| matches!(c, NetCommand::FetchInventoryOrphans { .. }))
    );
}

/// A tree already read (the cache) takes one queue step per folder, as
/// bulkFetchViaAis's "Already fetched" branch: 401 folders end within one
/// pump's 512 steps, where two steps a folder would leave half of them
/// queued.
#[test]
fn a_read_tree_is_walked_in_one_step_per_folder() {
    let mut tree = Vec::new();
    for top in 0..20 {
        tree.push((10 + top, 1, -1));
        for n in 0..19 {
            tree.push((1000 + top * 100 + n, 10 + top, -1));
        }
    }
    let mut inv = skeleton(&tree);
    inv.folders.values_mut().for_each(|f| f.state = FetchState::Fetched);
    assert_eq!(inv.folders.values().filter(|f| !f.library).count(), 401);
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let reads = fetch.pump_at(&mut inv, AGENT, Instant::now());
    assert!(
        reads.iter().all(|r| matches!(r, NetCommand::FetchInventoryOrphans { .. })),
        "nothing to read"
    );
    assert!(fetch.queue.is_empty(), "{} steps left after one pump", fetch.queue.len());
}

/// N4 counts a folder over the server's limit on its own line, never as
/// reading or unread while the walk is idle.
#[test]
fn n4_counts_an_over_limit_folder_only_as_over_the_limit() {
    let tree = [(10, 1, -1), (11, 1, -1)];
    let mut inv = skeleton(&tree);
    inv.folders.values_mut().for_each(|f| f.state = FetchState::Fetched);
    if let Some(f) = inv.folders.get_mut(&id(11)) {
        f.state = FetchState::Fetching;
    }
    inv.too_big.insert(id(11));
    let report = BackgroundFetch::default().report(&inv, Instant::now());
    assert!(
        report.contains("folders: 3 known, 2 read, 0 reading, 0 failed, 0 not read yet, 1 over the server's limit"),
        "{report}"
    );
}
