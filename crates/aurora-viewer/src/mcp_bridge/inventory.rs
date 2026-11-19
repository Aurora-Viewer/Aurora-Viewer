//! Agent inventory through MCP: folder paths (resolve one, or name an item's),
//! and moving items to the Trash with the move read back.
//!
//! `inventory_delete` moves items to the Trash exactly as the inventory
//! window's Delete does with confirmation off (`rules::move_selection` into the
//! Trash folder, then `edit_inventory`), never a purge, and answers once the
//! items' parent is the Trash.

use super::*;
use crate::world::inventory::actions as rules;

/// How long a move to the Trash may take to show.
const TRASH_DEADLINE: Duration = Duration::from_secs(15);

pub(super) struct PendingTrash {
    items: Vec<(Uuid, String)>,
    trash: Uuid,
    started: Instant,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

impl App {
    /// "/My Inventory/SLC Builder/Builds/Sausage Table" (the first segment may
    /// be the root's own name, or left out) to that folder: exact names, the
    /// agent's inventory only, an error naming the segment that does not
    /// resolve or is ambiguous. Never a silent fallback to the root.
    pub(super) fn resolve_inventory_path(&self, path: &str) -> Result<Uuid, McpError> {
        let inv = &self.world.inventory;
        let mut here = inv.root;
        let mut segments: Vec<&str> = path.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();
        if let Some(root) = inv.folders.get(&inv.root)
            && segments.first().is_some_and(|s| *s == root.info.name)
        {
            segments.remove(0);
        }
        let mut walked = String::new();
        for seg in segments {
            let children: Vec<Uuid> = inv
                .folders
                .values()
                .filter(|f| !f.library && f.info.parent == here && f.info.name == seg)
                .map(|f| f.info.id)
                .collect();
            walked.push('/');
            walked.push_str(seg);
            match children[..] {
                [one] => here = one,
                [] => {
                    return Err(McpError::InvalidParams(format!(
                        "no folder '{seg}' at '{walked}' (inventory_browse the parent to see its folders)"
                    )));
                }
                _ => {
                    return Err(McpError::InvalidParams(format!(
                        "{} folders are named '{seg}' at '{walked}': browse by folder UUID instead",
                        children.len()
                    )));
                }
            }
        }
        Ok(here)
    }

    /// The folder path of an inventory folder, from the root ("/My Inventory/...").
    pub(super) fn inventory_folder_path(&self, folder: Uuid) -> String {
        let inv = &self.world.inventory;
        let mut names = Vec::new();
        let mut here = folder;
        for _ in 0..64 {
            let Some(f) = inv.folders.get(&here) else { break };
            names.push(f.info.name.clone());
            if f.info.parent.is_nil() || f.info.id == inv.root {
                break;
            }
            here = f.info.parent;
        }
        names.reverse();
        format!("/{}", names.join("/"))
    }

    /// inventory_delete: to the Trash (never purged), read back.
    pub(super) fn start_mcp_inventory_delete(&mut self, item_ids: Vec<Uuid>, reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>) {
        let inv = &self.world.inventory;
        if item_ids.is_empty() {
            let _ = reply_tx.send(Err(McpError::InvalidParams("give item_ids".into())));
            return;
        }
        let Some(trash) = rules::system(inv, 14) else {
            let _ = reply_tx.send(Err(McpError::Internal("no Trash folder found".into())));
            return;
        };
        let mut items = Vec::new();
        for id in &item_ids {
            let Some(it) = inv.items.get(id) else {
                let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                    "no item {id} in your inventory (folders are not deleted by this tool)"
                ))));
                return;
            };
            if inv.folders.get(&it.parent).is_some_and(|f| f.library) {
                let _ = reply_tx.send(Err(McpError::PermissionDenied(format!("'{}' is in the Library", it.name))));
                return;
            }
            if it.parent == trash {
                let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                    "'{}' is already in the Trash (this tool never purges)",
                    it.name
                ))));
                return;
            }
            items.push((*id, it.name.clone()));
        }
        // worn attachments and protected system items are refused, as in the window
        let worn = self.world.outfit_attachment_items();
        let protected = HashSet::new();
        let change = match rules::move_selection(inv, &item_ids, trash, &protected, &worn) {
            Ok(change) => change,
            Err(reason) => {
                let _ = reply_tx.send(Err(McpError::PermissionDenied(reason)));
                return;
            }
        };
        self.edit_inventory(change, false, Uuid::nil());
        self.mcp_bridge.pending_trash.push_back(PendingTrash {
            items,
            trash,
            started: Instant::now(),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_inventory_delete(&mut self) {
        let now = Instant::now();
        let pending = std::mem::take(&mut self.mcp_bridge.pending_trash);
        for p in pending {
            let moved = p
                .items
                .iter()
                .all(|(id, _)| self.world.inventory.items.get(id).is_some_and(|it| it.parent == p.trash));
            if moved {
                let names: Vec<&str> = p.items.iter().map(|(_, n)| n.as_str()).collect();
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "verified": true,
                    "moved_to_trash": names,
                    "trash_folder_id": p.trash.to_string(),
                    "note": "in the Trash, not purged: it can be restored from there",
                }))));
            } else if now.duration_since(p.started) > TRASH_DEADLINE {
                let _ = p.reply_tx.send(Err(McpError::Internal(
                    "timed out: the move to the Trash was sent but the items never showed there".into(),
                )));
            } else {
                self.mcp_bridge.pending_trash.push_back(p);
            }
        }
    }
}
