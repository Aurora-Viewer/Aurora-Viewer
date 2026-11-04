//! The inventory's item records, with an index of the links by parent.
//!
//! A folder readback retires the links its folder no longer lists (#1515),
//! including links that were never in the folder's list. Finding them by
//! scanning every record cost ~6 ms per readback with 220,000 items (#1963);
//! the index finds them among the folder's own links. Reads go through
//! `Deref` to the map; every write goes through this type, so the index
//! cannot miss one. A record lent out by `get_mut` is checked again before
//! the next lookup, since its parent or type may have changed.

use aurora_net::inventory::InvItem;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use uuid::Uuid;

/// Links and folder links (LLAssetType AT_LINK, AT_LINK_FOLDER).
fn is_link(item: &InvItem) -> bool {
    matches!(item.asset_type, 24 | 25)
}

#[derive(Debug, Default, Clone)]
pub struct Items {
    map: HashMap<Uuid, InvItem>,
    /// Link ids by the parent they were indexed under.
    links: HashMap<Uuid, HashSet<Uuid>>,
    /// The parent each indexed link is listed under in `links`.
    indexed: HashMap<Uuid, Uuid>,
    /// Records lent out mutably since the last `sync`.
    lent: HashSet<Uuid>,
    /// Every record was lent out (`iter_mut`, `values_mut`): rebuild.
    all_lent: bool,
}

impl Deref for Items {
    type Target = HashMap<Uuid, InvItem>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<'a> IntoIterator for &'a Items {
    type Item = (&'a Uuid, &'a InvItem);
    type IntoIter = std::collections::hash_map::Iter<'a, Uuid, InvItem>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

impl From<HashMap<Uuid, InvItem>> for Items {
    fn from(map: HashMap<Uuid, InvItem>) -> Self {
        let mut items = Self {
            map,
            all_lent: true,
            ..Default::default()
        };
        items.sync();
        items
    }
}

impl FromIterator<(Uuid, InvItem)> for Items {
    fn from_iter<T: IntoIterator<Item = (Uuid, InvItem)>>(iter: T) -> Self {
        HashMap::from_iter(iter).into()
    }
}

impl Extend<(Uuid, InvItem)> for Items {
    fn extend<T: IntoIterator<Item = (Uuid, InvItem)>>(&mut self, iter: T) {
        for (id, item) in iter {
            self.insert(id, item);
        }
    }
}

impl Items {
    pub fn insert(&mut self, id: Uuid, item: InvItem) -> Option<InvItem> {
        self.unindex(id);
        if is_link(&item) {
            self.index(id, item.parent);
        }
        self.map.insert(id, item)
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<InvItem> {
        self.unindex(*id);
        self.lent.remove(id);
        self.map.remove(id)
    }

    pub fn get_mut(&mut self, id: &Uuid) -> Option<&mut InvItem> {
        let item = self.map.get_mut(id)?;
        self.lent.insert(*id);
        Some(item)
    }

    pub fn iter_mut(&mut self) -> std::collections::hash_map::IterMut<'_, Uuid, InvItem> {
        self.all_lent = true;
        self.map.iter_mut()
    }

    pub fn values_mut(&mut self) -> std::collections::hash_map::ValuesMut<'_, Uuid, InvItem> {
        self.all_lent = true;
        self.map.values_mut()
    }

    /// Updates the index record by record: `Inventory::remove` runs this on
    /// every deletion, and a full rebuild would cost the scan it replaces.
    pub fn retain(&mut self, mut keep: impl FnMut(&Uuid, &mut InvItem) -> bool) {
        let mut dropped = Vec::new();
        let lent = &mut self.lent;
        self.map.retain(|id, item| {
            let before = (is_link(item), item.parent);
            if !keep(id, item) {
                dropped.push(*id);
                return false;
            }
            // the closure may also have changed a record it kept
            if (is_link(item), item.parent) != before {
                lent.insert(*id);
            }
            true
        });
        for id in dropped {
            self.unindex(id);
            self.lent.remove(&id);
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.links.clear();
        self.indexed.clear();
        self.lent.clear();
        self.all_lent = false;
    }

    pub fn drain(&mut self) -> std::collections::hash_map::Drain<'_, Uuid, InvItem> {
        self.links.clear();
        self.indexed.clear();
        self.lent.clear();
        self.all_lent = false;
        self.map.drain()
    }

    /// The links whose parent is `folder`, in no order.
    pub fn links_in(&mut self, folder: Uuid) -> Vec<Uuid> {
        self.sync();
        self.links.get(&folder).map(|ids| ids.iter().copied().collect()).unwrap_or_default()
    }

    /// Index again the records lent out mutably since the last call.
    fn sync(&mut self) {
        if std::mem::take(&mut self.all_lent) {
            self.lent.clear();
            self.links.clear();
            self.indexed.clear();
            let links: Vec<_> = self
                .map
                .iter()
                .filter(|(_, it)| is_link(it))
                .map(|(id, it)| (*id, it.parent))
                .collect();
            for (id, parent) in links {
                self.index(id, parent);
            }
            return;
        }
        for id in std::mem::take(&mut self.lent) {
            self.unindex(id);
            if let Some(item) = self.map.get(&id).filter(|it| is_link(it)) {
                let parent = item.parent;
                self.index(id, parent);
            }
        }
    }

    fn index(&mut self, id: Uuid, parent: Uuid) {
        self.links.entry(parent).or_default().insert(id);
        self.indexed.insert(id, parent);
    }

    fn unindex(&mut self, id: Uuid) {
        if let Some(parent) = self.indexed.remove(&id)
            && let Some(ids) = self.links.get_mut(&parent)
        {
            ids.remove(&id);
            if ids.is_empty() {
                self.links.remove(&parent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u128, parent: u128, asset_type: i32) -> InvItem {
        InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            asset_type,
            ..Default::default()
        }
    }

    fn links_in(items: &mut Items, folder: u128) -> Vec<u128> {
        let mut ids: Vec<_> = items.links_in(Uuid::from_u128(folder)).iter().map(|id| id.as_u128()).collect();
        ids.sort();
        ids
    }

    #[test]
    fn indexes_links_by_parent_through_every_write() {
        let mut items = Items::default();
        for it in [record(1, 10, 24), record(2, 10, 25), record(3, 10, 6), record(4, 20, 24)] {
            items.insert(it.id, it);
        }
        assert_eq!(links_in(&mut items, 10), [1, 2], "originals are not links");
        assert_eq!(links_in(&mut items, 20), [4]);
        // replaced under another parent, then removed
        items.insert(Uuid::from_u128(1), record(1, 20, 24));
        items.remove(&Uuid::from_u128(2));
        assert_eq!(links_in(&mut items, 10), Vec::<u128>::new());
        assert_eq!(links_in(&mut items, 20), [1, 4]);
        // moved or retyped through a mutable borrow
        items.get_mut(&Uuid::from_u128(4)).expect("record").parent = Uuid::from_u128(30);
        items.get_mut(&Uuid::from_u128(3)).expect("record").asset_type = 24;
        assert_eq!(links_in(&mut items, 30), [4]);
        assert_eq!(links_in(&mut items, 10), [3]);
        items.get_mut(&Uuid::from_u128(3)).expect("record").asset_type = 6;
        assert_eq!(links_in(&mut items, 10), Vec::<u128>::new());
    }

    #[test]
    fn bulk_writes_index_again() {
        let mut items: Items = [record(1, 10, 24), record(2, 10, 24)].into_iter().map(|it| (it.id, it)).collect();
        assert_eq!(links_in(&mut items, 10), [1, 2]);
        for it in items.values_mut() {
            it.parent = Uuid::from_u128(20);
        }
        assert_eq!(links_in(&mut items, 20), [1, 2]);
        items.retain(|id, _| *id == Uuid::from_u128(2));
        assert_eq!(links_in(&mut items, 20), [2]);
        assert!(!items.all_lent, "a retain updates the index without a rebuild");
        // a record the closure keeps but moves is indexed under its new parent
        items.retain(|_, it| {
            it.parent = Uuid::from_u128(30);
            true
        });
        assert_eq!(links_in(&mut items, 20), Vec::<u128>::new());
        assert_eq!(links_in(&mut items, 30), [2]);
        items.extend([(Uuid::from_u128(5), record(5, 30, 25))]);
        assert_eq!(links_in(&mut items, 30), [2, 5]);
        let drained: Vec<_> = items.drain().collect();
        assert_eq!(drained.len(), 2);
        assert_eq!(links_in(&mut items, 30), Vec::<u128>::new());
        items.clear();
        assert_eq!(links_in(&mut items, 20), Vec::<u128>::new());
    }
}
