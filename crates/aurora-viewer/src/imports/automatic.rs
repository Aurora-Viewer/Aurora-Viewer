//! Folder mutations begin only after the existing paid-upload confirmation.
//! Fresh mesh quotes use acknowledged destination UUIDs and the approved budget.
use super::*;

pub(super) struct Confirmed {
    choice: Choice,
    permissions: Preferences,
    prices: Prices,
    maximum: i32,
    meshes: HashMap<Uuid, i32>,
    requested: HashSet<Uuid>,
}

impl Imports {
    pub fn start_confirmed(
        &mut self,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
        permissions: &Preferences,
        prices: &Prices,
    ) {
        if self.busy() || self.pending_files.is_some() {
            return;
        }
        if !self.folder_plan_valid(world, inventory) {
            self.message = aurora_i18n::tr!("import-invalid-request").into();
            return;
        }
        if self.folder_plan_ready(world, inventory) {
            self.start(world, inventory, permissions, prices);
            return;
        }
        if self
            .entries
            .iter()
            .filter(|entry| self.choice.accepts(entry) && entry.selected && matches!(entry.status, Status::Ready))
            .any(|entry| {
                entry.name.trim().is_empty()
                    || entry.name.len() > 63
                    || entry.description.len() > 127
                    || entry.name.contains('\0')
                    || entry.description.contains('\0')
            })
        {
            self.message = aurora_i18n::tr!("import-invalid-request").into();
            return;
        }
        let Some(maximum) = self.total(prices) else {
            self.message = aurora_i18n::tr!("import-price-unavailable").into();
            return;
        };
        if maximum > 0 && world.balance.is_none_or(|balance| balance < maximum) {
            self.message = aurora_i18n::tr!("import-insufficient-balance").into();
            return;
        }
        let meshes = self
            .entries
            .iter()
            .filter(|entry| self.choice.accepts(entry) && entry.selected && matches!(entry.status, Status::Ready))
            .filter_map(|entry| match &entry.source {
                Some(Source::Mesh(draft)) => draft.quote.as_ref().map(|quote| (entry.id, quote.cost)),
                _ => None,
            })
            .collect();
        self.prepare_import_folders(world, inventory);
        if self.folders.busy() {
            self.confirm = false;
            self.message.clear();
            self.pending_confirmed = Some(Confirmed {
                choice: self.choice,
                permissions: permissions.clone(),
                prices: prices.clone(),
                maximum,
                meshes,
                requested: HashSet::new(),
            });
        }
    }

    /// The app submits returned read-only fee requests through the usual worker.
    pub fn advance_confirmed(
        &mut self,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
    ) -> Vec<(Uuid, Permissions)> {
        let Some(confirmed) = self.pending_confirmed.take() else {
            return Vec::new();
        };
        self.with_choice(confirmed.choice, |imports| imports.advance_for(confirmed, world, inventory))
    }

    fn advance_for(
        &mut self,
        mut confirmed: Confirmed,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
    ) -> Vec<(Uuid, Permissions)> {
        if self.folders.busy() {
            self.pending_confirmed = Some(confirmed);
            return Vec::new();
        }
        if !self.folder_plan_ready(world, inventory) {
            if self.message.is_empty() {
                self.message = aurora_i18n::tr!("import-folders-failed").into();
            }
            return Vec::new();
        }
        let mut requests = Vec::new();
        let mut waiting = false;
        for entry in self
            .entries
            .iter()
            .filter(|entry| self.choice.accepts(entry) && entry.selected && matches!(entry.status, Status::Ready))
        {
            let Some(Source::Mesh(draft)) = &entry.source else { continue };
            if let Some(quote) = &draft.quote {
                if confirmed.meshes.get(&entry.id).is_none_or(|limit| quote.cost > *limit) {
                    self.message = ui::error(Error::PriceChanged(Some(quote.cost)));
                    return Vec::new();
                }
            } else if draft.pending.is_some() {
                waiting = true;
            } else if confirmed.requested.insert(entry.id) {
                requests.push((entry.id, confirmed.permissions.permissions_for(Kind::Mesh)));
            } else {
                if self.message.is_empty() {
                    self.message = aurora_i18n::tr!("import-price-unavailable").into();
                }
                return Vec::new();
            }
        }
        if waiting || !requests.is_empty() {
            self.pending_confirmed = Some(confirmed);
            return requests;
        }
        if self.total(&confirmed.prices).is_none_or(|total| total > confirmed.maximum) {
            self.message = ui::error(Error::PriceChanged(self.total(&confirmed.prices)));
            return Vec::new();
        }
        self.start(world, inventory, &confirmed.permissions, &confirmed.prices);
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
