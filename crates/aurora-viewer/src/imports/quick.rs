//! The toolbar's quick imports (human request, 2026-11-25): "Import
//! textures", "Import materials" and "Import sounds" send the files picked
//! without the import list. A free upload starts at once, a paid one asks
//! once with its price, and images that could go up at 2048 × 2048 ask the
//! size with both prices, unless they are free and the settings send free
//! images at 2048 (`Preferences::quick_free_full_size`). Cancelling empties the files picked from the
//! queue. What cannot go up as it is (an error, a sound to cut, no price)
//! opens its type's import window instead. The upload itself is the import
//! window's (`start_confirmed`), in the Legacy Viewer's order and messages.

use super::*;

/// Quick imports for these types; the others pick files for their window.
pub const QUICK: [Choice; 3] = [Choice::Texture, Choice::Material, Choice::Sound];

/// One quick import, from the files picked to the end of their upload.
pub struct Quick {
    pub choice: Choice,
    /// The file request of the pick.
    request: Uuid,
    /// The entries the pick loaded, once it did.
    ids: Option<Vec<Uuid>>,
    /// The question shown, while one is.
    pub ask: Option<Ask>,
    /// Sent: followed until it is uploaded, for a failure to show.
    sent: bool,
}

/// The question of a quick import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Its price (L$`cost` for `count` files).
    Price { count: usize, cost: i32 },
    /// The image size: L$`small` at 1024 × 1024, L$`large` at 2048 × 2048.
    Size { count: usize, small: i32, large: i32 },
}

/// An answer to a quick import's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Upload, at 2048 × 2048 when `Some(true)`, 1024 × 1024 when
    /// `Some(false)`, at the size each file has otherwise.
    Upload(Option<bool>),
    Cancel,
}

/// An image larger than 1024 on a side: it goes up smaller at 1024.
fn could_be_large(entry: &Entry) -> bool {
    let large = |w: u32, h: u32| w.max(h) > 1024;
    match &entry.source {
        Some(Source::Texture(image)) => large(image.width, image.height),
        Some(Source::Material { images, .. }) => images
            .iter()
            .flatten()
            .filter(|image| image.asset.is_none())
            .any(|image| large(image.pixels.width, image.pixels.height)),
        _ => false,
    }
}

impl Imports {
    /// The toolbar's typed import buttons: the file chooser at once. The
    /// quick types go up without the list; the others, and a pick while
    /// another import is under way, open their window.
    pub fn import_type(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, options: import_texture::Options, choice: Choice) {
        let free = !self.busy() && self.pending_files.is_none() && !self.confirm && self.quick.is_none();
        if !free || !QUICK.contains(&choice) {
            self.open_type_window(ctx, choice);
        }
        if !free {
            return;
        }
        self.choose_type_files(jobs, ctx, options, choice);
        if QUICK.contains(&choice)
            && let Some(request) = self.pending_files
        {
            self.quick = Some(Quick {
                choice,
                request,
                ids: None,
                ask: None,
                sent: false,
            });
        }
    }

    /// The entries a pick loaded: those of a quick import are its own.
    pub(super) fn quick_loaded(&mut self, request: Uuid, entries: &[Entry]) {
        if let Some(quick) = &mut self.quick
            && quick.request == request
        {
            quick.ids = Some(entries.iter().map(|entry| entry.id).collect());
        }
    }

    /// Runs `body` on the entries `ids` alone: the others of the same type
    /// leave the selection meanwhile, and get their tick back after.
    pub(crate) fn with_only<R>(&mut self, ids: &[Uuid], body: impl FnOnce(&mut Self) -> R) -> R {
        let mut others = Vec::new();
        for entry in &mut self.entries {
            if !ids.contains(&entry.id) && entry.selected {
                entry.selected = false;
                others.push(entry.id);
            }
        }
        let saved = self.only.replace(ids.to_vec());
        let result = body(self);
        self.only = saved;
        for entry in &mut self.entries {
            if others.contains(&entry.id) {
                entry.selected = true;
            }
        }
        result
    }

    /// The quick import's price with its files at `large` (2048 × 2048) or
    /// 1024 × 1024 (`Some(false)`), or as they are.
    fn quick_total(&mut self, ids: &[Uuid], choice: Choice, prices: &Prices, large: Option<bool>) -> Option<i32> {
        if let Some(large) = large {
            self.set_quick_size(ids, large);
        }
        self.with_choice(choice, |imports| imports.with_only(ids, |imports| imports.total(prices)))
    }

    fn set_quick_size(&mut self, ids: &[Uuid], large: bool) {
        let limit = if large {
            ImageSizeLimit::Size2048
        } else {
            ImageSizeLimit::Size1024
        };
        let shared = self.load_preferences.limit_texture_size;
        for entry in self.entries.iter_mut().filter(|entry| ids.contains(&entry.id)) {
            entry.image_size_limit = limit;
            entry.sync_image_size(shared);
        }
    }

    /// Opens the quick import's window on what it could not send as it is.
    fn quick_to_window(&mut self, ctx: &egui::Context) {
        if let Some(quick) = self.quick.take() {
            self.open_type_window(ctx, quick.choice);
        }
    }

    /// Moves the quick import on, every frame: its question once the files
    /// are read, its upload when it is free, its window when they cannot go
    /// up as they are, or when one fails.
    pub fn advance_quick(
        &mut self,
        ctx: &egui::Context,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
        prefs: &Preferences,
        prices: &Prices,
    ) {
        let Some(quick) = &self.quick else { return };
        if quick.ask.is_some() {
            return;
        }
        let (choice, sent) = (quick.choice, quick.sent);
        let Some(ids) = quick.ids.clone() else {
            // the chooser closed with nothing
            if self.pending_files != Some(quick.request) {
                self.quick = None;
            }
            return;
        };
        let ids: Vec<Uuid> = ids
            .into_iter()
            .filter(|id| self.entries.iter().any(|entry| entry.id == *id))
            .collect();
        let entries = || self.entries.iter().filter(|entry| ids.contains(&entry.id));
        if sent {
            // uploaded files leave the queue; one that failed, or that the
            // upload left behind, is shown in its window
            let left = !self.busy() && self.pending_confirmed.is_none() && entries().any(|entry| !matches!(entry.status, Status::Done(_)));
            if ids.is_empty() {
                self.quick = None;
            } else if entries().any(|entry| matches!(entry.status, Status::Error(_))) || left {
                self.quick_to_window(ctx);
            }
            return;
        }
        if ids.is_empty() {
            self.quick = None;
            return;
        }
        if entries().any(|entry| !matches!(entry.status, Status::Ready) || !choice.accepts(entry)) {
            self.quick_to_window(ctx);
            return;
        }
        let count = ids.len();
        let ask = if matches!(choice, Choice::Texture | Choice::Material) && entries().any(could_be_large) {
            let small = self.quick_total(&ids, choice, prices, Some(false));
            let large = self.quick_total(&ids, choice, prices, Some(true));
            match (small, large) {
                // free at 2048 too: no question when the settings say so
                (Some(_), Some(0)) if prefs.quick_free_full_size => {
                    self.set_quick_size(&ids, true);
                    self.send_quick(ctx, world, inventory, prefs, prices);
                    return;
                }
                (Some(small), Some(large)) => Some(Ask::Size { count, small, large }),
                _ => None,
            }
        } else {
            match self.quick_total(&ids, choice, prices, None) {
                Some(0) => {
                    self.send_quick(ctx, world, inventory, prefs, prices);
                    return;
                }
                Some(cost) => Some(Ask::Price { count, cost }),
                None => None,
            }
        };
        match ask {
            Some(ask) => {
                if let Some(quick) = &mut self.quick {
                    quick.ask = Some(ask);
                }
            }
            // no price: the window says why
            None => self.quick_to_window(ctx),
        }
    }

    /// The question's answer: the upload, or the files picked out of the
    /// queue.
    #[allow(clippy::too_many_arguments)]
    pub fn answer_quick(
        &mut self,
        answer: Answer,
        ctx: &egui::Context,
        jobs: &crate::scene::jobs::Jobs,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
        prefs: &Preferences,
        prices: &Prices,
    ) {
        let Some(quick) = &mut self.quick else { return };
        quick.ask = None;
        let ids = quick.ids.clone().unwrap_or_default();
        match answer {
            Answer::Upload(size) => {
                if let Some(large) = size {
                    self.set_quick_size(&ids, large);
                }
                self.send_quick(ctx, world, inventory, prefs, prices);
            }
            Answer::Cancel => {
                self.quick = None;
                for id in ids {
                    if let Some(index) = self.entries.iter().position(|entry| entry.id == id) {
                        jobs.discard(self.entries.remove(index));
                    }
                }
                self.reconcile_active();
            }
        }
    }

    fn send_quick(
        &mut self,
        ctx: &egui::Context,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
        prefs: &Preferences,
        prices: &Prices,
    ) {
        let Some(quick) = &mut self.quick else { return };
        quick.sent = true;
        let choice = quick.choice;
        let ids = quick.ids.clone().unwrap_or_default();
        self.message.clear();
        self.with_choice(choice, |imports| {
            imports.with_only(&ids, |imports| imports.start_confirmed(world, inventory, prefs, prices));
        });
        // refused before anything was sent (balance, names): the window
        // shows the reason
        if !self.message.is_empty() && !self.busy() && self.pending_confirmed.is_none() {
            self.quick_to_window(ctx);
        }
    }
}

#[cfg(test)]
#[path = "quick_tests.rs"]
mod tests;
