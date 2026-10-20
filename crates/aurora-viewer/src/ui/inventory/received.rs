//! Unread deliveries shared by the compact tabs and wide inventory sidebar.

use crate::world::{inventory::Inventory, received_items};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Badge {
    key: Option<(Uuid, u64)>,
    deliveries: HashSet<Uuid>,
    seen: HashSet<Uuid>,
    unread: usize,
}

impl Badge {
    fn prepare(&mut self, inv: &Inventory) {
        let key = (inv.root, inv.generation);
        if self.key == Some(key) {
            return;
        }
        if self.key.is_some_and(|(root, _)| root != inv.root) {
            self.seen.clear();
        }
        self.key = Some(key);
        self.deliveries = received_items::deliveries(inv).collect();
        self.unread = self.deliveries.difference(&self.seen).count();
    }

    pub(super) fn unseen(&mut self, inv: &Inventory) -> usize {
        self.prepare(inv);
        self.unread
    }

    pub(super) fn acknowledge(&mut self, inv: &Inventory, shown: &[Uuid]) {
        self.prepare(inv);
        for id in shown {
            if self.deliveries.contains(id) && self.seen.insert(*id) {
                self.unread -= 1;
            }
        }
    }
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
}
