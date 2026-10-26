use super::*;
use aurora_net::inventory::{FolderContents, InvFolder};

const AGENT: Uuid = Uuid::from_u128(2);

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
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

/// An AIS server over `tree` that returns at most `cap` levels and fails
/// the reads `fails` picks, like a 403 for a reply that would be too big.
struct Server<'a> {
    tree: &'a [(u128, u128, i32)],
    cap: i32,
    fails: &'a dyn Fn(&Read) -> bool,
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
            items: Vec::new(),
        });
        if depth > 0 {
            for child in next {
                self.subtree(child, depth - 1, out);
            }
        }
    }

    fn answer(&self, read: &Read) -> Option<Vec<FolderContents>> {
        if (self.fails)(read) {
            return None;
        }
        let mut out = Vec::new();
        if read.children.is_empty() {
            self.subtree(read.parent, read.depth.min(self.cap), &mut out);
        } else {
            for child in &read.children {
                self.subtree(*child, (read.depth - 1).min(self.cap), &mut out);
            }
        }
        Some(out)
    }
}

/// Run the fetch to its end against `server`; every read, in order.
fn run(inv: &mut Inventory, server: &Server) -> Vec<Read> {
    let mut fetch = BackgroundFetch::default();
    fetch.start(inv);
    let mut reads = Vec::new();
    for _ in 0..1000 {
        let commands = fetch.pump(inv, AGENT);
        assert!(fetch.in_flight.len() <= MAX_IN_FLIGHT, "PoolSizeAIS bounds the reads in flight");
        if commands.is_empty() && fetch.is_idle() {
            return reads;
        }
        for command in commands {
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
            assert_eq!(library, parent == inv.lib_root);
            let read = Read { parent, children, depth };
            if let Some(contents) = server.answer(&read) {
                inv.apply(contents);
            }
            fetch.on_reply(inv, request);
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
            fails: &|_| false,
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
fn marketplace_listings_are_left_to_their_window() {
    let tree = [(10, 1, -1), (11, 1, MARKETPLACE_LISTINGS), (12, 11, -1)];
    let mut inv = skeleton(&tree);
    run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            fails: &|_| false,
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
            fails: &|_| false,
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
    let big = |r: &Read| r.depth > 0 && (r.parent == id(10) && r.children.is_empty() || r.children.contains(&id(10)));
    let reads = run(
        &mut inv,
        &Server {
            tree: &tree,
            cap: MAX_FOLDER_DEPTH,
            fails: &big,
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
        fails: &|_| false,
    };
    let mut fetch = BackgroundFetch::default();
    fetch.start(&inv);
    let answer = |inv: &mut Inventory, fetch: &mut BackgroundFetch, commands: Vec<NetCommand>| {
        let mut batches = Vec::new();
        for command in commands {
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
                inv.apply(server.answer(&read).unwrap_or_default());
                fetch.on_reply(inv, request);
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
    inv.apply(
        server
            .answer(&Read {
                parent: id(11),
                children: Vec::new(),
                depth: 0,
            })
            .unwrap_or_default(),
    );
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
        fails: &|_| false,
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
