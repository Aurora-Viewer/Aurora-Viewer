//! Explicit, read-only selection of one Legacy Viewer set after native AO setup.
//! As AOEngine::cloneSet (indra/newview/aoengine.cpp, originally LGPL 2.1),
//! recreate options, cycles and links; never move or edit the source set.

use super::inventory::{self, Catalog};
use crate::world::inventory::Inventory;
use aurora_net::inventory::operations::Mutation;
use uuid::Uuid;

#[derive(Default)]
pub struct Migration {
    pub source: Catalog,
    pub selected: Uuid,
    pub name: String,
    pub message: String,
    pub writable: bool,
    refreshed: bool,
}

impl Migration {
    pub fn refresh(&mut self, inv: &mut Inventory, writable: bool) {
        self.writable = writable;
        self.source.refresh_firestorm(inv);
        self.refreshed = true;
        if self.selected.is_nil()
            && self.ready()
            && let Some(set) = self.source.sets.first()
        {
            self.selected = set.id;
            self.name = set.name.clone();
        }
    }

    pub fn ready(&self) -> bool {
        self.refreshed && !self.source.loading && !self.source.failed && self.source.missing.is_empty()
    }

    pub fn select(&mut self, id: Uuid) {
        if let Some(set) = self.source.sets.iter().find(|s| s.id == id) {
            self.selected = id;
            self.name = set.name.clone();
            self.message.clear();
        }
    }

    pub fn validate(&self, native: &Catalog) -> Result<String, String> {
        if !self.ready() || native.loading || native.failed || !self.writable {
            return Err(aurora_i18n::tr!("ao-loading").into());
        }
        if !self.source.sets.iter().any(|s| s.id == self.selected) {
            return Err(aurora_i18n::tr!("ao-migrate-missing").into());
        }
        let name = inventory::valid_name(&self.name)?;
        if native.sets.iter().any(|s| s.name == name) {
            return Err(aurora_i18n::tr!("ao-migrate-name-exists").into());
        }
        Ok(name)
    }

    pub fn plan(&self, inv: &Inventory, native: &Catalog) -> Result<(Mutation, Uuid), String> {
        let name = self.validate(native)?;
        let Some(mut set) = self.source.sets.iter().find(|s| s.id == self.selected).cloned() else {
            return Err(aurora_i18n::tr!("ao-migrate-missing").into());
        };
        set.name = name;
        // Adding an explicit set must not change the existing native default.
        set.default = native.sets.is_empty();
        super::completion::swimming(&mut set);
        inventory::create(inv, native, &set)
    }
}

#[cfg(test)]
mod tests;
