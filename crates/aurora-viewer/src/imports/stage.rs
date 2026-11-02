//! Snapshot checked local previews into independent, ordinary upload drafts.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32};

impl Imports {
    pub fn selected_local_uploads(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| {
            entry.local && entry.selected && !entry.reloading && entry.source.is_some() && matches!(entry.status, Status::Ready)
        })
    }

    pub fn stage_local_uploads(
        &mut self,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
        prefs: &Preferences,
    ) -> usize {
        if self.choice != Choice::Local || self.busy() || self.confirm || self.pending_files.is_some() {
            return 0;
        }
        let selected: Vec<_> = self.selected_local_uploads().collect();
        if selected.is_empty() {
            return 0;
        }
        // Apply the loader's draft and conservative non-mesh input budgets
        // atomically: a large selection must never create a partial batch.
        if self.entries.len() + selected.len() > 512 {
            self.message = aurora_i18n::tr!("import-batch-limit").into();
            return 0;
        }
        let maps = local_maps(&self.entries);
        let mut copies = Vec::with_capacity(selected.len());
        for original in selected {
            let Some(mut source) = original.source.clone() else { continue };
            resolve_local_maps(&mut source, &maps);
            if let Source::Mesh(draft) = &mut source {
                // Do not cancel the original preview's work or reuse its fee
                // request, prepared geometry or per-job synchronization.
                draft.cancel = Arc::new(AtomicBool::new(false));
                draft.progress = Arc::new(AtomicU32::new(0));
                draft.prepared = None;
                draft.quote = None;
                draft.quoted_at = None;
                draft.pending = None;
                draft.discarded_quotes.clear();
            }
            let kind = source.kind();
            let mut copy = Entry::with_cached_alpha(original.path.clone(), Ok(source), original.options, Some(original.empty_alpha));
            copy.name.clone_from(&original.name);
            copy.description.clone_from(&original.description);
            copy.source_folders.clone_from(&original.source_folders);
            copy.material_index = original.material_index;
            copy.folder = if prefs.use_default_folders {
                Self::default_folder(world, inventory, kind)
            } else {
                Self::batch_folder(world, prefs.batch_folder)
            };
            copies.push(copy);
        }
        let bytes: usize = self
            .entries
            .iter()
            .chain(copies.iter())
            .filter_map(|entry| entry.source.as_ref())
            .filter(|source| !matches!(source, Source::Mesh(_)))
            .map(Source::retained_bytes)
            .sum();
        if bytes > 1024 * 1024 * 1024 {
            self.message = aurora_i18n::tr!("import-batch-limit").into();
            return 0;
        }
        let Some(first) = copies.first().map(|entry| entry.id) else {
            return 0;
        };
        let count = copies.len();
        for copy in &copies {
            if let Some(index) = Self::type_index(Self::type_choice(copy)) {
                self.type_filters[index] = true;
            }
        }
        self.entries.extend(copies);
        self.sync_destinations(prefs, world, inventory);
        self.open(Choice::Bulk);
        self.tab = ui::Tab::Batch;
        self.active = Some(first);
        self.message.clear();
        count
    }
}

/// Local UUIDs are preview handles, never existing server texture assets.
fn local_maps(entries: &[Entry]) -> HashMap<Uuid, LocalImage> {
    let mut maps = HashMap::new();
    for entry in entries.iter().filter(|entry| entry.local) {
        match &entry.source {
            Some(Source::Texture(pixels)) => {
                maps.insert(
                    entry.id,
                    LocalImage {
                        path: Some(entry.path.clone()),
                        key: entry.id,
                        name: entry.name.clone(),
                        pixels: pixels.clone(),
                        asset: None,
                        empty_alpha: entry.empty_alpha,
                    },
                );
            }
            Some(Source::Material { images, .. }) => {
                maps.extend(
                    images
                        .iter()
                        .flatten()
                        .filter(|image| image.asset.is_none())
                        .map(|image| (image.key, image.clone())),
                );
            }
            Some(Source::Mesh(draft)) => {
                maps.extend(
                    draft
                        .images
                        .values()
                        .filter(|image| image.asset.is_none())
                        .map(|image| (image.key, image.clone())),
                );
            }
            _ => {}
        }
    }
    maps
}

fn resolve_local_maps(source: &mut Source, maps: &HashMap<Uuid, LocalImage>) {
    match source {
        Source::Material { value, images } => {
            for (slot, id) in super::maps::ids(value).into_iter().enumerate() {
                let asset = images[slot].as_ref().and_then(|image| image.asset).or(id);
                if let Some(image) = asset.and_then(|asset| maps.get(&asset)) {
                    images[slot] = Some(image.clone());
                    super::maps::set(value, slot, None);
                }
            }
        }
        Source::Mesh(draft) => {
            for image in draft.images.values_mut() {
                if let Some(local) = image.asset.and_then(|asset| maps.get(&asset)) {
                    *image = local.clone();
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
