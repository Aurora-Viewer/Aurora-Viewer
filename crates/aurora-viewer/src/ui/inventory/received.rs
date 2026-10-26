//! Unread deliveries shared by the compact tabs and wide inventory sidebar.

use crate::world::inventory::FetchState;
use crate::world::{inventory::Inventory, received_items};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

mod storage;

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    initialized: bool,
    known: HashSet<Uuid>,
    unread: HashSet<Uuid>,
    expected: HashMap<Uuid, HashSet<Uuid>>,
}

#[derive(Default)]
pub(super) struct Badge {
    key: Option<(Uuid, u64)>,
    deliveries: HashSet<Uuid>,
    saved: Saved,
    unread: usize,
    storage: Option<storage::Storage>,
    loading: bool,
    bound_root: Option<Uuid>,
    receipts: Vec<received_items::Receipt>,
    shown: HashSet<Uuid>,
    dirty: bool,
    written: Option<Instant>,
    fetched: Option<(u64, Instant, bool)>,
    /// New Marketplace deliveries not yet announced (`take_fresh`, #752).
    fresh: Vec<Uuid>,
    /// Discoveries are announced: a saved baseline is authoritative, while
    /// the unbound demo/test badge only announces after its first enumeration
    /// (its seeded inbox is not news).
    primed: bool,
}

impl Badge {
    pub(super) fn load(path: PathBuf, root: Uuid) -> Self {
        let storage = storage::Storage::start(path);
        Self {
            key: None,
            deliveries: HashSet::new(),
            saved: Saved::default(),
            unread: 0,
            loading: storage.is_some(),
            storage,
            bound_root: Some(root),
            receipts: Vec::new(),
            shown: HashSet::new(),
            dirty: false,
            written: None,
            fetched: None,
            fresh: Vec::new(),
            primed: true,
        }
    }

    fn poll(&mut self) {
        if self.loading {
            match self.storage.as_ref().map(|storage| storage.loaded.try_recv()) {
                Some(Ok(saved)) => {
                    self.saved = saved;
                    self.loading = false;
                }
                Some(Err(crossbeam_channel::TryRecvError::Empty)) => return,
                _ => {
                    self.loading = false;
                }
            }
        }
        for receipt in std::mem::take(&mut self.receipts) {
            match receipt {
                received_items::Receipt::Item(id) => {
                    if !self.saved.known.contains(&id) {
                        self.dirty |= self.saved.unread.insert(id);
                    }
                }
                received_items::Receipt::Scripted { parent, previous } if !self.saved.initialized => {
                    self.saved.expected.entry(parent).or_insert(previous);
                    self.dirty = true;
                }
                _ => {}
            }
        }
    }

    pub(super) fn receive(&mut self, receipts: Vec<received_items::Receipt>) {
        self.receipts.extend(receipts);
        self.poll();
    }

    /// Establish a complete first baseline even while the inventory window is
    /// closed. Lazy folder fetches must not turn old inventory into new receipts.
    pub(super) fn update(&mut self, inv: &mut Inventory) {
        self.poll();
        if self.bound_root.is_none() {
            self.saved.initialized = true;
        }
        if self.loading || inv.held.is_some() || inv.root.is_nil() || self.bound_root.is_some_and(|root| root != inv.root) {
            return;
        }
        let ready = match self.fetched {
            Some((generation, at, ready)) if generation == inv.generation && (ready || at.elapsed() < Duration::from_millis(500)) => ready,
            _ => {
                let ready = baseline_ready(inv);
                self.fetched = Some((inv.generation, Instant::now(), ready));
                ready
            }
        };
        if !self.saved.initialized {
            if !ready {
                self.persist(false);
                return;
            }
            for (parent, previous) in self.saved.expected.drain() {
                if let Some(folder) = inv.folders.get(&parent) {
                    self.saved.unread.extend(
                        folder
                            .children
                            .iter()
                            .chain(&folder.items)
                            .filter(|id| !previous.contains(id))
                            .copied(),
                    );
                }
            }
            self.saved.known = received_items::deliveries(inv).collect();
            self.saved.initialized = true;
            self.dirty = true;
            self.key = None;
        }
        self.prepare(inv);
        self.persist(false);
    }

    fn prepare(&mut self, inv: &Inventory) {
        self.poll();
        // Default is the in-memory demo/test case: its seeded deliveries are new.
        if self.bound_root.is_none() {
            self.saved.initialized = true;
        }
        if self.loading || !self.saved.initialized || self.bound_root.is_some_and(|root| root != inv.root) {
            self.unread = 0;
            return;
        }
        let key = (inv.root, inv.generation);
        if self.key == Some(key) {
            return;
        }
        if self.key.is_some_and(|(root, _)| root != inv.root) {
            self.saved = Saved {
                initialized: true,
                ..Default::default()
            };
            self.primed = false;
        }
        self.key = Some(key);
        self.deliveries = received_items::deliveries(inv).collect();
        let marketplace: HashSet<Uuid> = if self.primed {
            received_items::marketplace_deliveries(inv).collect()
        } else {
            HashSet::new()
        };
        for id in &self.deliveries {
            if self.saved.known.insert(*id) {
                self.saved.unread.insert(*id);
                self.dirty = true;
                if marketplace.contains(id) && self.fresh.len() < 256 {
                    self.fresh.push(*id);
                }
            }
        }
        self.primed = true;
        for id in self.shown.drain() {
            if self.deliveries.contains(&id) {
                self.dirty |= self.saved.unread.remove(&id);
            }
        }
        self.unread = self.deliveries.intersection(&self.saved.unread).count();
        self.persist(false);
    }

    /// The Marketplace deliveries discovered since the last call, offline
    /// arrivals included, never the history of the first baseline.
    pub(super) fn take_fresh(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.fresh)
    }

    pub(super) fn unseen(&mut self, inv: &Inventory) -> usize {
        self.prepare(inv);
        self.unread
    }

    pub(super) fn acknowledge(&mut self, inv: &Inventory, shown: &[Uuid]) {
        self.prepare(inv);
        if self.bound_root.is_some_and(|root| root != inv.root) {
            return;
        }
        if self.loading || !self.saved.initialized {
            self.shown.extend(shown);
            return;
        }
        for id in shown {
            if self.deliveries.contains(id) && self.saved.unread.remove(id) {
                self.unread -= 1;
                self.dirty = true;
            }
        }
        self.persist(false);
    }

    fn persist(&mut self, force: bool) {
        if self.dirty && !self.loading && (force || self.written.is_none_or(|at| at.elapsed() >= Duration::from_secs(1))) {
            if let Some(storage) = &self.storage {
                storage.save(self.saved.clone());
            }
            self.dirty = false;
            self.written = Some(Instant::now());
        }
    }
}

impl Drop for Badge {
    fn drop(&mut self) {
        if self.loading
            && let Some(storage) = &self.storage
        {
            self.saved = storage.loaded.recv().unwrap_or_default();
            self.loading = false;
        }
        self.poll();
        for id in self.shown.drain() {
            self.saved.known.insert(id);
            self.saved.unread.remove(&id);
            self.dirty = true;
        }
        self.persist(true);
    }
}

fn baseline_ready(inv: &mut Inventory) -> bool {
    let mut ready = true;
    let mut requested = 0;
    let mut check = |inv: &mut Inventory, id| {
        let fetched = inv.folders.get(&id).is_some_and(|folder| folder.state == FetchState::Fetched);
        if !fetched && requested < 32 {
            inv.request(id);
            requested += 1;
        }
        ready &= fetched;
    };
    check(inv, inv.root);
    // Each path component must be complete before a missing child means absent.
    let aurora = inv
        .folders
        .get(&inv.root)
        .into_iter()
        .flat_map(|f| &f.children)
        .copied()
        .find(|id| {
            inv.folders
                .get(id)
                .is_some_and(|f| !f.library && f.info.name == crate::world::bridge::FOLDER)
        });
    if let Some(aurora) = aurora {
        check(inv, aurora);
    }
    if let Some(root) = received_items::root(inv) {
        check(inv, root);
        let senders = inv.folders.get(&root).map(|f| f.children.clone()).unwrap_or_default();
        for sender in senders {
            check(inv, sender);
        }
    }
    if let Some(inbox) = crate::world::inventory::actions::system(inv, 50) {
        check(inv, inbox);
    }
    ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::{InvFolder, InvItem};

    fn inventory() -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        for (id, parent, name, kind) in [
            (1, 0, "Inventory", 8),
            (2, 1, "Inbox", 50),
            (3, 1, "#Aurora", -1),
            (4, 3, "Received items", -1),
            (5, 4, "Example Resident", -1),
            (6, 5, "Gift folder", -1),
            (7, 6, "Nested folder", -1),
            (8, 2, "Marketplace purchase", -1),
        ] {
            inv.created_folders(vec![InvFolder {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(parent),
                name: name.into(),
                type_default: kind,
                ..Default::default()
            }]);
        }
        for (id, parent) in [(10, 2), (11, 5), (12, 7)] {
            add_item(&mut inv, id, parent);
        }
        inv
    }

    fn add_item(inv: &mut Inventory, id: u128, parent: u128) {
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            ..Default::default()
        }]);
    }

    fn fetch_sender(inv: &mut Inventory) {
        let folder_id = Uuid::from_u128(5);
        inv.apply(vec![aurora_net::inventory::FolderContents {
            folder_id,
            owner_id: Uuid::from_u128(1),
            version: 1,
            folders: inv
                .folders
                .values()
                .filter(|f| f.info.parent == folder_id)
                .map(|f| f.info.clone())
                .collect(),
            items: inv.items.values().filter(|item| item.parent == folder_id).cloned().collect(),
        }]);
    }

    #[test]
    fn received_badge_counts_deliveries_and_acknowledges_both_items_and_folders() {
        let inv = inventory();
        let mut badge = Badge::default();
        assert_eq!(
            badge.unseen(&inv),
            4,
            "two Marketplace and two sender deliveries; wrappers and nested contents excluded"
        );
        let shown: Vec<_> = [2, 4, 5, 6, 7, 8, 10, 11, 12].map(Uuid::from_u128).into();
        badge.acknowledge(&inv, &shown);
        assert_eq!(badge.unseen(&inv), 0);
        badge.acknowledge(&inv, &shown);
        assert_eq!(badge.unseen(&inv), 0, "acknowledgement is idempotent");
    }

    #[test]
    fn received_badge_returns_for_another_item_from_the_same_sender() {
        let mut inv = inventory();
        let mut badge = Badge::default();
        badge.acknowledge(&inv, &received_items::deliveries(&inv).collect::<Vec<_>>());
        assert_eq!(badge.unseen(&inv), 0);
        add_item(&mut inv, 13, 5);
        assert_eq!(badge.unseen(&inv), 1);
        badge.acknowledge(&inv, &[Uuid::from_u128(5)]);
        assert_eq!(badge.unseen(&inv), 1, "the sender container is not the delivery");
        badge.acknowledge(&inv, &[Uuid::from_u128(13)]);
        assert_eq!(badge.unseen(&inv), 0);
        add_item(&mut inv, 14, 2);
        assert_eq!(badge.unseen(&inv), 1, "Marketplace still uses the same badge");
    }

    #[test]
    fn received_badge_preserves_filtered_out_deliveries_and_drops_moved_items() {
        let mut inv = inventory();
        let mut badge = Badge::default();
        badge.acknowledge(&inv, &[Uuid::from_u128(6), Uuid::from_u128(11)]);
        assert_eq!(badge.unseen(&inv), 2, "hidden Marketplace deliveries remain unread");
        add_item(&mut inv, 10, 1);
        assert_eq!(badge.unseen(&inv), 1, "a delivery moved out no longer waits in Received");
    }

    #[test]
    fn received_badge_announces_new_marketplace_deliveries_once_without_seeded_history() {
        let mut inv = inventory();
        let mut badge = Badge::default();
        badge.update(&mut inv);
        assert!(badge.take_fresh().is_empty(), "the unbound badge's seeded inbox is not news");
        add_item(&mut inv, 13, 5);
        add_item(&mut inv, 14, 2);
        badge.update(&mut inv);
        assert_eq!(
            badge.take_fresh(),
            vec![Uuid::from_u128(14)],
            "a sender folder's delivery came with its offer's card"
        );
        badge.update(&mut inv);
        assert!(badge.take_fresh().is_empty(), "announced once");
    }

    struct Files(PathBuf);

    impl Files {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("aurora-received-read-{}", Uuid::new_v4())))
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn loaded(path: PathBuf, inv: &mut Inventory) -> Badge {
        let mut badge = Badge::load(path, inv.root);
        let started = Instant::now();
        while badge.loading && started.elapsed() < Duration::from_secs(5) {
            badge.update(inv);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!badge.loading, "background state load completed");
        badge.update(inv);
        badge
    }

    #[test]
    fn received_badge_history_stays_read_across_reconnections_and_old_dated_new_gifts_count() {
        let files = Files::new();
        let mut inv = inventory();
        let mut badge = loaded(files.path("account.json"), &mut inv);
        assert_eq!(badge.unseen(&inv), 0, "the first baseline does not reclassify old inventory");
        add_item(&mut inv, 13, 5);
        badge.update(&mut inv);
        assert_eq!(badge.unseen(&inv), 1, "a fresh UUID counts even with an old/zero creation date");
        badge.acknowledge(&inv, &[Uuid::from_u128(13)]);
        drop(badge);

        let mut reconnected = inventory();
        add_item(&mut reconnected, 13, 5);
        let mut badge = loaded(files.path("account.json"), &mut reconnected);
        assert_eq!(badge.unseen(&reconnected), 0, "read deliveries never become new on login");
        add_item(&mut reconnected, 14, 2);
        badge.update(&mut reconnected);
        assert_eq!(badge.unseen(&reconnected), 1, "a genuinely new Marketplace receipt still counts");
    }

    #[test]
    fn received_badge_retains_unread_and_discovers_offline_deliveries_after_restart() {
        let files = Files::new();
        let mut inv = inventory();
        let mut badge = loaded(files.path("account.json"), &mut inv);
        add_item(&mut inv, 13, 5);
        add_item(&mut inv, 14, 2);
        badge.update(&mut inv);
        badge.acknowledge(&inv, &[Uuid::from_u128(13)]);
        assert_eq!(badge.unseen(&inv), 1);
        drop(badge);

        let mut reconnected = inventory();
        for (id, parent) in [(13, 5), (14, 2), (15, 5)] {
            add_item(&mut reconnected, id, parent);
        }
        let mut badge = loaded(files.path("account.json"), &mut reconnected);
        assert_eq!(
            badge.unseen(&reconnected),
            2,
            "one retained unread receipt and one received offline"
        );
        badge.acknowledge(&reconnected, &[Uuid::from_u128(14), Uuid::from_u128(15)]);
        drop(badge);
        assert_eq!(loaded(files.path("account.json"), &mut reconnected).unseen(&reconnected), 0);
    }

    #[test]
    fn received_badge_announces_offline_marketplace_deliveries_but_not_the_first_baseline() {
        let files = Files::new();
        let mut inv = inventory();
        let mut badge = loaded(files.path("account.json"), &mut inv);
        assert!(badge.take_fresh().is_empty(), "the first baseline is history");
        drop(badge);
        let mut reconnected = inventory();
        add_item(&mut reconnected, 14, 2);
        let mut badge = loaded(files.path("account.json"), &mut reconnected);
        assert_eq!(badge.take_fresh(), vec![Uuid::from_u128(14)], "delivered while offline");
        drop(badge);
        let mut again = loaded(files.path("account.json"), &mut reconnected);
        assert!(again.take_fresh().is_empty(), "never announced twice across logins");
    }

    #[test]
    fn received_badge_waits_for_lazy_history_but_preserves_receipts_during_initialization() {
        let files = Files::new();
        let mut inv = inventory();
        inv.folders.get_mut(&Uuid::from_u128(5)).unwrap().state = FetchState::Unknown;
        let mut badge = loaded(files.path("account.json"), &mut inv);
        assert!(!badge.saved.initialized);
        assert_eq!(badge.unseen(&inv), 0);
        add_item(&mut inv, 16, 5); // Older history fetched late.
        badge.receive(vec![
            received_items::Receipt::Item(Uuid::from_u128(13)),
            received_items::Receipt::Scripted {
                parent: Uuid::from_u128(5),
                previous: [6, 11, 16].map(Uuid::from_u128).into(),
            },
        ]);
        add_item(&mut inv, 13, 5);
        add_item(&mut inv, 14, 5);
        fetch_sender(&mut inv);
        badge.update(&mut inv);
        assert!(badge.saved.initialized);
        assert_eq!(badge.unseen(&inv), 2, "only the direct and scripted receipts are new");
        assert!(!badge.saved.unread.contains(&Uuid::from_u128(16)));
    }

    #[test]
    fn received_badge_is_account_scoped_and_flushes_a_read_before_load_finishes() {
        let files = Files::new();
        let mut inv = inventory();
        drop(loaded(files.path("first-account.json"), &mut inv));
        add_item(&mut inv, 13, 5);
        let mut badge = Badge::load(files.path("first-account.json"), inv.root);
        badge.acknowledge(&inv, &[Uuid::from_u128(13)]);
        drop(badge);
        assert_eq!(loaded(files.path("first-account.json"), &mut inv).unseen(&inv), 0);
        let mut second = loaded(files.path("second-account.json"), &mut inv);
        assert_eq!(second.unseen(&inv), 0, "another account initializes its own history");
        add_item(&mut inv, 14, 5);
        second.update(&mut inv);
        assert_eq!(second.unseen(&inv), 1);
        drop(second);
        let mut first_inventory = inventory();
        add_item(&mut first_inventory, 13, 5);
        assert_eq!(
            loaded(files.path("first-account.json"), &mut first_inventory).unseen(&first_inventory),
            0
        );
    }

    #[test]
    fn received_badge_fetches_changed_containers_after_reconnect_without_opening_received() {
        let files = Files::new();
        let mut inv = inventory();
        drop(loaded(files.path("account.json"), &mut inv));
        let mut reconnected = inventory();
        reconnected.items.remove(&Uuid::from_u128(11));
        let sender = reconnected.folders.get_mut(&Uuid::from_u128(5)).unwrap();
        sender.items.clear();
        sender.state = FetchState::Unknown;
        let mut badge = loaded(files.path("account.json"), &mut reconnected);
        assert!(reconnected.queue.iter().any(|(id, _)| *id == Uuid::from_u128(5)));
        assert_eq!(badge.unseen(&reconnected), 0);
        add_item(&mut reconnected, 11, 5);
        add_item(&mut reconnected, 13, 5);
        fetch_sender(&mut reconnected);
        badge.update(&mut reconnected);
        assert_eq!(
            badge.unseen(&reconnected),
            1,
            "late metadata restores old history without marking it new"
        );
    }
}
