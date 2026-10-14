//! The favourites: the landmarks of the inventory's Favorites folder, in
//! the order the user gave them, shared by the three ways the bars show
//! them (the place island's menu, the row under the islands, the dock's
//! grid). Port of the order of LLFavoritesBarCtrl and LLFavoritesOrderStorage
//! (indra/newview/llfavoritesbar.cpp, originally LGPL 2.1): a sort index per
//! item kept for the account (Firestorm's `landmarks_sorting.xml`, here
//! `favorites_order.json`), the items without one first, newest first, then
//! by name (LLFavoritesSort), and every item numbered again 1, 2, 3… as soon
//! as one lacks its index (LLFavoritesBarCtrl::needToSaveItemsOrder).
//! Firestorm also sends each reordered item back to the server unchanged
//! (`updateServer(false)`); the order lives only in the account's file, so
//! Aurora sends nothing.

use super::inventory::Inventory;
use crossbeam_channel::{self as mpsc, Receiver, Sender};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;
use uuid::Uuid;

pub const FILE: &str = "favorites_order.json";
/// FT_FAVORITE.
pub const FOLDER_FAVORITES: i32 = 23;
/// LLAssetType::AT_LANDMARK.
const ASSET_LANDMARK: i32 = 3;

/// The account's file, read and written by a thread of its own.
struct Storage {
    loaded: Receiver<HashMap<Uuid, i32>>,
    writer: Option<Sender<HashMap<Uuid, i32>>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub struct Favorites {
    /// Sort index of each favourite (LLFavoritesOrderStorage::mSortIndexes).
    order: HashMap<Uuid, i32>,
    storage: Option<Storage>,
    /// The list, rebuilt when the inventory or the order changes.
    cache: Vec<Uuid>,
    cache_key: Option<(u64, u64)>,
    /// Bumped by every change of the order.
    revision: u64,
    /// A favourite added from the bars this session, and when it arrived:
    /// its row is marked as just added.
    pub added: Option<(Uuid, std::time::Instant)>,
    /// Waiting for the next new favourite (an add was asked).
    expecting: bool,
    known: std::collections::HashSet<Uuid>,
}

impl Favorites {
    /// The account's order, read off the frame thread; the demo keeps it in
    /// memory (Default).
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::unbounded();
        let (writer, writes) = mpsc::unbounded::<HashMap<Uuid, i32>>();
        let worker = std::thread::Builder::new().name("aurora-favorites".into()).spawn(move || {
            let order = match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice::<HashMap<Uuid, i32>>(&bytes).unwrap_or_else(|error| {
                    log::warn!("Cannot read the favourites' order: {error}");
                    HashMap::new()
                }),
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("Cannot read the favourites' order: {error}");
                    }
                    HashMap::new()
                }
            };
            let _ = loaded_tx.send(order);
            while let Ok(mut order) = writes.recv() {
                while let Ok(newer) = writes.try_recv() {
                    order = newer;
                }
                let save = || -> std::io::Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = serde_json::to_vec(&order).map_err(std::io::Error::other)?;
                    crate::atomic_file::write(&path, &bytes)
                };
                if let Err(error) = save() {
                    log::warn!("Cannot save the favourites' order: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Favorites {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    worker: Some(worker),
                }),
                ..Favorites::default()
            },
            Err(error) => {
                log::warn!("Cannot start the favourites' storage: {error}");
                Favorites::default()
            }
        }
    }

    /// The favourites in their order: the landmarks under the Favorites
    /// folder (its sub-folders too, as LLInventoryModel::collectDescendentsIf).
    /// Numbers them again when one has no index yet (a new favourite).
    pub fn list(&mut self, inv: &Inventory) -> &[Uuid] {
        if let Some(storage) = &self.storage
            && let Ok(order) = storage.loaded.try_recv()
        {
            // what was ordered before the file was read stays ahead
            for (id, index) in order {
                self.order.entry(id).or_insert(index);
            }
            self.revision += 1;
        }
        let key = (inv.generation, self.revision);
        if self.cache_key != Some(key) {
            let mut items = landmarks(inv);
            sort(&mut items, inv, &self.order);
            if items.iter().any(|id| !self.order.contains_key(id)) {
                self.number(&items);
            }
            self.notice_new(&items);
            self.cache = items;
            self.cache_key = Some((inv.generation, self.revision));
        }
        &self.cache
    }

    /// The favourites are numbered 1, 2, 3… in `items`' order and saved.
    fn number(&mut self, items: &[Uuid]) {
        self.order = items.iter().enumerate().map(|(n, id)| (*id, n as i32 + 1)).collect();
        self.revision += 1;
        self.save();
    }

    /// Move `source` before the favourite at `rank` of the list (its end
    /// when past it), as LLFavoritesOrderStorage::rearrangeFavoriteLandmarks.
    pub fn rearrange(&mut self, inv: &Inventory, source: Uuid, rank: usize) {
        let mut items = self.list(inv).to_vec();
        let Some(from) = items.iter().position(|id| *id == source) else {
            return;
        };
        let item = items.remove(from);
        let to = if rank > from { rank - 1 } else { rank }.min(items.len());
        items.insert(to, item);
        if items.as_slice() != self.cache.as_slice() {
            self.number(&items);
        }
    }

    /// An add was asked (the star, the menu): the next favourite that shows
    /// up is the one, marked as just added.
    pub fn expect_added(&mut self) {
        self.expecting = true;
    }

    fn notice_new(&mut self, items: &[Uuid]) {
        if self.expecting
            && let Some(new) = items.iter().find(|id| !self.known.contains(*id))
            && !self.known.is_empty()
        {
            self.added = Some((*new, std::time::Instant::now()));
            self.expecting = false;
        }
        self.known = items.iter().copied().collect();
    }

    fn save(&self) {
        if let Some(writer) = self.storage.as_ref().and_then(|s| s.writer.as_ref()) {
            let _ = writer.send(self.order.clone());
        }
    }
}

/// The landmark items under the Favorites folder.
fn landmarks(inv: &Inventory) -> Vec<Uuid> {
    let Some(root) = inv.folders.values().find(|f| f.info.type_default == FOLDER_FAVORITES && !f.library) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![root.info.id];
    let mut depth = 0;
    while let Some(folder) = stack.pop() {
        let Some(f) = inv.folders.get(&folder) else { continue };
        out.extend(
            f.items
                .iter()
                .filter(|id| inv.items.get(*id).is_some_and(|it| it.asset_type == ASSET_LANDMARK)),
        );
        // a malformed cycle of folders stops here
        depth += 1;
        if depth < 4096 {
            stack.extend(f.children.iter().copied());
        }
    }
    out
}

/// LLFavoritesSort: by sort index, the items without one first; between
/// two without one, the newest first, then by name.
fn sort(items: &mut [Uuid], inv: &Inventory, order: &HashMap<Uuid, i32>) {
    items.sort_by(|a, b| {
        let (ia, ib) = (order.get(a).copied().unwrap_or(-1), order.get(b).copied().unwrap_or(-1));
        if ia >= 0 || ib >= 0 {
            return ia.cmp(&ib);
        }
        let (a, b) = (inv.items.get(a), inv.items.get(b));
        let created = |i: Option<&aurora_net::inventory::InvItem>| i.map_or(0, |i| i.created_at);
        let name = |i: Option<&aurora_net::inventory::InvItem>| i.map(|i| i.name.to_lowercase()).unwrap_or_default();
        created(b).cmp(&created(a)).then_with(|| name(a).cmp(&name(b)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// A Favorites folder with landmarks `(n, name, created)`, a sub-folder
    /// with one more and a notecard that is no favourite.
    fn inventory(marks: &[(u128, &str, i64)]) -> Inventory {
        let mut inv = Inventory::default();
        let folder = |n: u128, parent: u128, kind: i32, children: Vec<Uuid>, items: Vec<Uuid>| Folder {
            info: InvFolder {
                id: id(n),
                parent: id(parent),
                name: String::new(),
                type_default: kind,
                version: 1,
                ..Default::default()
            },
            children,
            items,
            state: FetchState::Fetched,
            library: false,
        };
        let mut items = Vec::new();
        for (n, name, created) in marks {
            inv.items.insert(
                id(*n),
                InvItem {
                    id: id(*n),
                    parent: id(1),
                    name: (*name).into(),
                    asset_type: ASSET_LANDMARK,
                    created_at: *created,
                    ..Default::default()
                },
            );
            items.push(id(*n));
        }
        inv.items.insert(
            id(900),
            InvItem {
                id: id(900),
                parent: id(1),
                asset_type: 7,
                ..Default::default()
            },
        );
        items.push(id(900));
        inv.folders.insert(id(1), folder(1, 0, FOLDER_FAVORITES, vec![id(2)], items));
        inv.folders.insert(id(2), folder(2, 1, -1, vec![], vec![]));
        inv.generation = 1;
        inv
    }

    #[test]
    fn new_favourites_come_first_newest_first_then_keep_their_order() {
        let mut inv = inventory(&[(10, "Beta", 100), (11, "Alpha", 200), (12, "Gamma", 100)]);
        let mut fav = Favorites::default();
        // none has an index yet: newest first, then by name; then numbered
        assert_eq!(fav.list(&inv), [id(11), id(10), id(12)]);
        assert_eq!(fav.order.get(&id(12)), Some(&3));
        // a new landmark arrives: it has no index, so it goes first
        inv.items.insert(
            id(13),
            InvItem {
                id: id(13),
                parent: id(1),
                name: "New".into(),
                asset_type: ASSET_LANDMARK,
                created_at: 50,
                ..Default::default()
            },
        );
        if let Some(f) = inv.folders.get_mut(&id(1)) {
            f.items.push(id(13));
        }
        inv.generation += 1;
        assert_eq!(fav.list(&inv), [id(13), id(11), id(10), id(12)]);
    }

    #[test]
    fn rearranging_moves_one_favourite_and_numbers_them_all() {
        let inv = inventory(&[(10, "A", 4), (11, "B", 3), (12, "C", 2), (13, "D", 1)]);
        let mut fav = Favorites::default();
        assert_eq!(fav.list(&inv), [id(10), id(11), id(12), id(13)]);
        // A dropped before D, then D dropped at the very start
        fav.rearrange(&inv, id(10), 3);
        assert_eq!(fav.list(&inv), [id(11), id(12), id(10), id(13)]);
        fav.rearrange(&inv, id(13), 0);
        assert_eq!(fav.list(&inv), [id(13), id(11), id(12), id(10)]);
        // past the end: the last place
        fav.rearrange(&inv, id(13), 99);
        assert_eq!(fav.list(&inv), [id(11), id(12), id(10), id(13)]);
        let indexes: Vec<i32> = fav.list(&inv).to_vec().iter().map(|i| fav.order[i]).collect();
        assert_eq!(indexes, [1, 2, 3, 4]);
    }

    #[test]
    fn an_added_favourite_is_noticed_once_it_arrives() {
        let mut inv = inventory(&[(10, "A", 1)]);
        let mut fav = Favorites::default();
        fav.list(&inv);
        fav.expect_added();
        inv.items.insert(
            id(11),
            InvItem {
                id: id(11),
                parent: id(1),
                asset_type: ASSET_LANDMARK,
                ..Default::default()
            },
        );
        if let Some(f) = inv.folders.get_mut(&id(1)) {
            f.items.push(id(11));
        }
        inv.generation += 1;
        fav.list(&inv);
        assert_eq!(fav.added.map(|a| a.0), Some(id(11)));
    }

    #[test]
    fn the_order_survives_a_restart_per_account() {
        let dir = std::env::temp_dir().join(format!("aurora-favorites-{}", Uuid::new_v4()));
        let path = dir.join("account").join(FILE);
        let inv = inventory(&[(10, "A", 3), (11, "B", 2), (12, "C", 1)]);
        {
            let mut fav = Favorites::load(path.clone());
            // the file does not exist: wait for its empty answer
            while fav.storage.as_ref().is_some_and(|s| s.loaded.is_empty()) {
                std::thread::yield_now();
            }
            fav.list(&inv);
            fav.rearrange(&inv, id(12), 0);
        }
        let mut fav = Favorites::load(path);
        while fav.storage.as_ref().is_some_and(|s| s.loaded.is_empty()) {
            std::thread::yield_now();
        }
        assert_eq!(fav.list(&inv), [id(12), id(10), id(11)]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
