//! One startup fallback from the Legacy Viewer AO to Aurora-owned inventory links.
//! Existing Aurora folders, including an empty AO, are always authoritative.
use super::inventory::{self, Catalog};
use crate::world::{
    World,
    inventory::{Inventory, actions},
};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::Mutation;
use uuid::Uuid;

#[derive(Default)]
pub struct Bootstrap {
    source: Catalog,
    finished: bool,
    consolidated: std::collections::HashSet<Uuid>,
}

impl Bootstrap {
    /// Cached folders can be ready before the login seed has supplied these
    /// services. Match Session's EditInventory preconditions before consuming
    /// the single startup attempt; no request has been sent while waiting.
    pub fn writable(world: &World) -> bool {
        world.inventory_writable()
    }

    pub fn loading(&self) -> bool {
        !self.finished && (self.source.loading || !self.source.missing.is_empty() || !self.source.sets.is_empty())
    }

    pub fn missing(&self) -> &[Uuid] {
        &self.source.missing
    }

    pub fn poll(
        &mut self,
        inv: &mut Inventory,
        native: &Catalog,
        selected: Uuid,
        writable: bool,
    ) -> Option<Result<(Mutation, Option<Uuid>), String>> {
        if !native.loading
            && !native.failed
            && native.missing.is_empty()
            && let Some(set) = native.selected(selected)
            && !self.consolidated.contains(&set.id)
            && set.legacy_swimming_pending()
        {
            if !writable {
                return None;
            }
            // One attempt per set/session; partial failures require a fresh
            // acknowledged inventory refresh, never a blind automatic replay.
            self.consolidated.insert(set.id);
            return Some(inventory::consolidate_swimming(inv, set).map(|change| (change, None)));
        }
        if self.finished || native.loading || native.failed {
            return None;
        }
        if let Some(root) = native.root {
            self.source = Catalog::default();
            if inv
                .folders
                .get(&root)
                .is_some_and(|folder| folder.info.name == inventory::OLD_FOLDER)
            {
                if !writable {
                    return None;
                }
                // Rename in place through the ordinary acknowledged edit path.
                // The catalogue remains usable; set/link IDs and selection stay
                // unchanged. A failed/ambiguous edit is not replayed this session.
                self.finished = true;
                return Some(actions::patch(inv, root, llsd_map! { "name" => inventory::FOLDER }).map(|change| (change, None)));
            }
            self.finished = true;
            return None;
        }
        self.source.refresh_firestorm(inv);
        if self.source.failed {
            self.finished = true;
            return Some(Err(aurora_i18n::tr!("ao-fetch-failed").into()));
        }
        if self.source.loading || !self.source.missing.is_empty() {
            return None;
        }
        if self.source.sets.is_empty() {
            self.finished = true;
            return None;
        }
        if !writable {
            return None;
        }
        // The independent bridge factory owns startup creation of #Aurora.
        // Wait for its inventory acknowledgement instead of racing a second
        // parent creation. No bridge attachment or motor is required here.
        inventory::child(inv, inv.root, "#Aurora")?;
        self.finished = true;
        Some(inventory::reconstruct(inv, &self.source, selected))
    }
}

#[cfg(test)]
mod tests;
