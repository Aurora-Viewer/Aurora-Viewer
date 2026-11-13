//! The Favorite wearables window (`ui/wearable_favorites.rs`): its folders
//! read while it is open, and its actions.
use super::*;
use crate::ui::wearable_favorites::{FavoritesAction, Pending};
use crate::world::wearable_favorites as favorites;
use uuid::Uuid;

impl App {
    /// Every frame while the window is open: the folders, and the originals
    /// of their links that the inventory has not read, once each.
    pub(super) fn wearable_favorites_tick(&mut self) {
        if self.demo {
            self.wearable_favorites_demo();
        }
        if !self.panels.wearable_favorites {
            return;
        }
        // a wear asked here is refused here too, once the outfit has answered
        if self.wearable_favorites_ui.wearing && self.appearance_ui.pending.is_none() {
            self.wearable_favorites_ui.wearing = false;
            self.wearable_favorites_ui.message.clone_from(&self.appearance_ui.message);
        }
        favorites::fetch(&mut self.world.inventory);
        let requested = &mut self.wearable_favorites_ui.requested;
        let items: Vec<Uuid> = favorites::missing(&self.world.inventory)
            .into_iter()
            .filter(|id| requested.insert(*id))
            .collect();
        for chunk in items.chunks(100) {
            self.send(NetCommand::FetchItems {
                items: chunk.to_vec(),
                owner: self.world.agent_id,
            });
        }
    }

    /// AURORA_DEMO_WEARABLE_FAVORITES: the wardrobe and the window at frame
    /// 600; in `sync`, the Sync button at frame 720.
    fn wearable_favorites_demo(&mut self) {
        let Some(mode) = crate::demo::wearable_favorites::mode() else {
            return;
        };
        if self.frame_count == 600 {
            let folder = crate::demo::wearable_favorites::seed(&mut self.world.inventory, self.world.agent_id, &mode);
            self.wearable_favorites_ui.show_folder(folder);
            self.panels.wearable_favorites = true;
        }
        if self.frame_count == 660 && mode.starts_with("wear") {
            let items = crate::demo::wearable_favorites::worn_items().to_vec();
            self.wearable_favorites_apply(vec![FavoritesAction::Wear(crate::world::appearance::Action::WearItems {
                items,
                replace: false,
                point: 0,
            })]);
        }
        if self.frame_count == 720
            && mode == "delete-real"
            && let Err(reason) = favorites::delete_folder(&self.world.inventory, crate::demo::wearable_favorites::evening())
        {
            self.wearable_favorites_ui.message = reason;
        }
        if self.frame_count == 720 && mode == "sync" {
            let mut actions = Vec::new();
            self.wearable_favorites_ui.sync(&self.world, &mut actions);
            self.wearable_favorites_apply(actions);
        }
    }

    pub(super) fn wearable_favorites_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let mut open = self.panels.wearable_favorites;
        let sort = self.settings.wearable_favorites_sort;
        let actions = self.wearable_favorites_ui.show(ctx, p, &self.world, sort, &mut open);
        self.panels.wearable_favorites = open;
        self.wearable_favorites_apply(actions);
    }

    fn wearable_favorites_apply(&mut self, actions: Vec<FavoritesAction>) {
        for action in actions {
            match action {
                FavoritesAction::Wear(action) => {
                    self.appearance_ui.message.clear();
                    self.wearable_favorites_ui.wearing = true;
                    self.apply_appearance_action(action);
                }
                FavoritesAction::Sort(order) => {
                    self.settings.wearable_favorites_sort = order;
                    if !self.demo {
                        self.settings.save();
                    }
                }
                FavoritesAction::Edit { change, select, synced } => self.wearable_favorites_edit(change, select, synced),
            }
        }
    }

    fn wearable_favorites_edit(
        &mut self,
        change: aurora_net::inventory::operations::Mutation,
        select: Option<Uuid>,
        synced: Option<usize>,
    ) {
        // one at a time: a second request would take the first one's reply
        if change.operations.is_empty() || self.wearable_favorites_ui.pending.is_some() {
            return;
        }
        let request = Uuid::new_v4();
        let refresh = change.refresh.clone();
        let command = NetCommand::EditInventory { request, change };
        // `send` drops what RLVa refuses: no reply would free the window
        if !self.world.rlva_allows_command(&command, Uuid::nil()) {
            self.wearable_favorites_ui.message = aurora_i18n::tr!("rlva-blocked").into();
            return;
        }
        self.wearable_favorites_ui.pending = Some(Pending {
            request,
            select,
            synced,
            refresh,
        });
        self.wearable_favorites_ui.message.clear();
        self.send(command);
    }

    pub(super) fn wearable_favorites_result(&mut self, result: Result<aurora_net::inventory::operations::Reply, String>) {
        let Some(pending) = self.wearable_favorites_ui.pending.take() else {
            return;
        };
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents);
                self.world.inventory.created_folders(reply.folders);
                if let Some(id) = pending.select.and_then(|id| reply.created.get(&id).copied()) {
                    self.wearable_favorites_ui.show_folder(Some(id));
                }
                if let Some(count) = pending.synced {
                    self.wearable_favorites_ui.message = aurora_i18n::tr!("wearable-favorites-synced", count = count);
                }
            }
            Err(reason) => {
                log::warn!("favorite wearables not saved: {reason}");
                self.wearable_favorites_ui.message = reason;
                // a change can stop half way (inventory_result)
                for id in &pending.refresh {
                    if let Some(f) = self.world.inventory.folders.get_mut(id) {
                        f.state = crate::world::inventory::FetchState::Unknown;
                    }
                    self.world.inventory.request(*id);
                }
            }
        }
    }
}
