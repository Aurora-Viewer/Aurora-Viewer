//! Explicit, read-only selection of one Legacy Viewer or Alchemy set after
//! native AO setup. As AOEngine::cloneSet (indra/newview/aoengine.cpp,
//! originally LGPL 2.1), recreate options, cycles and links; never move or
//! edit the source set. Alchemy keeps the same AOEngine folder format.

use super::inventory::{self, Catalog, Source};
use crate::world::inventory::Inventory;
use aurora_net::inventory::operations::Mutation;
use uuid::Uuid;

#[derive(Default)]
pub struct Migration {
    pub source: Catalog,
    /// Read from Alchemy's root folder instead of the Legacy Viewer's.
    pub alchemy: bool,
    pub selected: Uuid,
    pub name: String,
    pub message: String,
    pub writable: bool,
    refreshed: bool,
}

impl Migration {
    pub fn viewer(&self) -> Source {
        if self.alchemy { Source::Alchemy } else { Source::Legacy }
    }

    /// Another viewer's sets never keep the previous viewer's selection.
    pub fn set_alchemy(&mut self, alchemy: bool) {
        if self.alchemy != alchemy {
            *self = Self {
                alchemy,
                writable: self.writable,
                ..Default::default()
            };
        }
    }

    pub fn refresh(&mut self, inv: &mut Inventory, writable: bool) {
        self.writable = writable;
        self.source.refresh_source(inv, self.viewer());
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

    pub fn missing(&self) -> String {
        aurora_i18n::tr!("ao-migrate-missing", folder = self.viewer().path())
    }

    pub fn validate(&self, native: &Catalog) -> Result<String, String> {
        if !self.ready() || native.loading || native.failed || !self.writable {
            return Err(aurora_i18n::tr!("ao-loading").into());
        }
        if !self.source.sets.iter().any(|s| s.id == self.selected) {
            return Err(self.missing());
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
            return Err(self.missing());
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
