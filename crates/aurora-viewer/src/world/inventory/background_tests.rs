use super::*;
use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

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
                FetchFailure::TooBig => 403,
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

fn run_keeping_fetch(mut fetch: BackgroundFetch, inv: &mut Inventory, server: &Server) -> (Vec<Read>, BackgroundFetch) {
    fetch.start(inv);
    let mut reads = Vec::new();
    let mut now = Instant::now();
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
    panic!("the background fetch never ended");
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
    // each recursive read goes from the first folder the previous one left
    let deep: Vec<Uuid> = reads
        .iter()
        .filter(|r| r.children.is_empty() && r.depth > 0)
        .map(|r| r.parent)
        .collect();
    assert!(deep.contains(&id(13)) && deep.contains(&id(16)), "{deep:?}");
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
        reads.iter().any(|r| !inv.folders[&r.parent].library && r.depth == MAX_FOLDER_DEPTH),
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
    assert_eq!(fetch.take_too_big().0, [id(11)], "the alert names the folder once");
    assert!(fetch.take_too_big().0.is_empty());
}
