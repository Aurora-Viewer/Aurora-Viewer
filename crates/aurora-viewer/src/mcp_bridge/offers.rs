//! Inventory passed between avatars, through MCP: the offers waiting for an
//! answer (`inventory_offers`), answering one (`inventory_offer_respond`, the
//! item reported once it is in inventory), and giving an item to someone
//! (`give_inventory`, the inventory window's Share).

use super::*;
use crate::world::notifications::{Data, Response};

const OFFER_DEADLINE: Duration = Duration::from_secs(20);

pub(super) struct PendingOffer {
    item: Uuid,
    name: String,
    started: Instant,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

impl App {
    /// The inventory offers waiting for an answer.
    pub(super) fn mcp_inventory_offers(&self) -> ViewerResponse {
        let offers: Vec<Value> = self
            .world
            .notifications
            .list
            .iter()
            .filter_map(|n| match &n.data {
                Data::Inventory {
                    from,
                    tx,
                    asset_type,
                    object,
                    task,
                    ..
                } => Some(json!({
                    "offer_id": n.id,
                    "from_id": from.to_string(),
                    "item_name": n.title,
                    "message": n.body,
                    "asset_type": asset_type,
                    // an avatar's offer is already in inventory under this id;
                    // an object's (task) offer is delivered once accepted
                    "item_id": (!object.is_nil()).then(|| object.to_string()),
                    "from_object": task,
                    "transaction_id": tx.to_string(),
                })),
                _ => None,
            })
            .collect();
        ViewerResponse::Json(json!({ "count": offers.len(), "offers": offers }))
    }

    /// Accept or decline one offer as its card's buttons do; an accepted
    /// avatar offer is answered once the item is in inventory (not the Trash).
    pub(super) fn start_mcp_offer_respond(&mut self, id: u64, accept: bool, reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>) {
        let Some(n) = self.world.notifications.list.iter().find(|n| n.id == id) else {
            let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                "no open inventory offer {id} (answered or expired?): list them with inventory_offers"
            ))));
            return;
        };
        let Data::Inventory { object, .. } = &n.data else {
            let _ = reply_tx.send(Err(McpError::InvalidParams(format!("notification {id} is not an inventory offer"))));
            return;
        };
        let (object, name) = (*object, n.title.clone());
        let (commands, _) = self
            .world
            .respond_notification(id, if accept { Response::Accept } else { Response::Decline });
        for command in commands {
            self.send(command);
        }
        if !accept {
            let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({ "ok": true, "declined": name }))));
            return;
        }
        if object.is_nil() {
            // an object's offer: the simulator delivers it after the answer
            let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                "ok": true,
                "accepted": name,
                "note": "an object's offer is delivered after accepting: find it with inventory_search by name",
            }))));
            return;
        }
        self.mcp_bridge.pending_offers.push_back(PendingOffer {
            item: object,
            name,
            started: Instant::now(),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_offers(&mut self) {
        let now = Instant::now();
        let trash = crate::world::inventory::actions::system(&self.world.inventory, 14);
        for p in std::mem::take(&mut self.mcp_bridge.pending_offers) {
            let kept = self
                .world
                .inventory
                .items
                .get(&p.item)
                .filter(|it| Some(it.parent) != trash)
                .map(|it| it.parent);
            if let Some(parent) = kept {
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "verified": true,
                    "accepted": p.name,
                    "item_id": p.item.to_string(),
                    "parent_path": self.inventory_folder_path(parent),
                }))));
            } else if now.duration_since(p.started) > OFFER_DEADLINE {
                let _ = p.reply_tx.send(Err(McpError::Internal(format!(
                    "accepted, but '{}' did not show in inventory within 20 s",
                    p.name
                ))));
            } else {
                self.mcp_bridge.pending_offers.push_back(p);
            }
        }
    }

    /// give_inventory: the inventory window's Share of one item to an avatar.
    pub(super) fn mcp_give_inventory(&mut self, item_id: Uuid, to: Uuid) -> Result<ViewerResponse, McpError> {
        if to.is_nil() || to == self.world.agent_id {
            return Err(McpError::InvalidParams("to_avatar_key must be another avatar".into()));
        }
        let Some(item) = self.world.inventory.items.get(&item_id).cloned() else {
            return Err(McpError::InvalidParams(format!("no item {item_id} in your inventory")));
        };
        if self.world.inventory.folders.get(&item.parent).is_some_and(|f| f.library) {
            return Err(McpError::PermissionDenied(format!("'{}' is in the Library", item.name)));
        }
        if (item.owner_mask & perm::TRANSFER) == 0 {
            return Err(McpError::PermissionDenied(format!("'{}' is no-transfer", item.name)));
        }
        let no_copy = (item.owner_mask & perm::COPY) == 0;
        self.apply_inventory_action(crate::ui::inventory::InvAction::Share {
            items: vec![item_id],
            residents: vec![to],
        });
        let still_here = self.world.inventory.items.contains_key(&item_id);
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "offered": item.name,
            "to": to.to_string(),
            "no_copy": no_copy,
            // a no-copy item leaves the giver's inventory with the offer
            "removed_from_inventory": no_copy && !still_here,
            "note": "offered: the recipient still has to accept (their answer arrives as an IM, see comms_read_history)",
        })))
    }
}
