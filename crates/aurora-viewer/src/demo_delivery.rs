//! AURORA_DEMO_DELIVERY: Marketplace deliveries arriving in Received items
//! with no offer (#752), added the way the server's BulkUpdateInventory adds
//! them. The inbox is seeded at frame 600 (not news), the delivery lands at
//! 700: `1` one purchase, its card unrolls from the bell; `several` three at
//! once, one card counting them; `open` one, its "Show in inventory"
//! pressed at 900; `history` one, closed at 900, then the bell's History tab
//! opens on it with its "Show in inventory".

use crate::world::inventory::Inventory;
use aurora_net::NetEvent;
use aurora_net::inventory::InvItem;
use uuid::Uuid;

/// The scenario, if asked: `1`, `several`, `open` or `history`.
pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_DELIVERY").ok()
}

/// The inbox and its earlier purchase appear (`seed_received`).
pub const SEED_AT: u64 = 600;
/// The new delivery lands in the inbox.
pub const ARRIVE_AT: u64 = 700;
/// `open`: "Show in inventory" is pressed; `history`: the card is closed,
/// then the list opens on History.
pub const READ_AT: u64 = 900;

/// The deliveries of `ARRIVE_AT`: objects in the seeded inbox, as one
/// unsolicited inventory update. Objects, not folders: the demo would fill
/// a folder it has never seen with its own made-up contents when fetched.
pub fn arrival(inv: &Inventory, agent: Uuid) -> Option<NetEvent> {
    let inbox = crate::world::inventory::actions::system(inv, 50)?;
    let count = if mode().as_deref() == Some("several") { 3 } else { 1 };
    let name = aurora_i18n::tr!("received-delivery-demo");
    let items = (0..count)
        .map(|n| InvItem {
            id: Uuid::from_u128(8_310_000 + n),
            parent: inbox,
            asset_id: Uuid::from_u128(8_310_100 + n),
            name: if n == 0 { name.to_owned() } else { format!("{name} {}", n + 1) },
            asset_type: 6,
            inv_type: 6,
            owner: agent,
            creator: agent,
            ..Default::default()
        })
        .collect();
    Some(NetEvent::InventoryUpdated {
        folders: Vec::new(),
        items,
    })
}
