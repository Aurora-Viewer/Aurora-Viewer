//! "Copy to inventory" of an object's contents: a new folder named after the
//! object, every item moved into it, then the folder worn on request. Port of
//! LLFloaterOpenObject::moveToInventory / callbackCreateInventoryCategory,
//! move_inv_category_world_to_agent and LLInventoryCopyAndWearObserver
//! (indra/newview/llfloateropenobject.cpp, llinventorybridge.cpp,
//! llinventoryobserver.cpp, originally LGPL 2.1).

use super::*;
use crate::ui::object_actions::{CopyConfirm, CopyRequest};
use crate::world::appearance::WearMode;
use crate::world::inventory::actions as rules;
use crate::world::notifications::{Data, Kind};
use std::collections::HashMap;
use uuid::Uuid;

/// The Clothing system folder: a copy to wear goes under it.
const FT_CLOTHING: i32 = 5;
/// The items reach the inventory one by one after the moves; past this, the
/// outfit is left as it is.
const WEAR_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct ContentsCopies {
    /// A copy that moves no-copy items, waiting for the user's answer.
    pub confirm: Option<CopyConfirm>,
    /// Folder creations on their way, by request: the copy and the
    /// folder's id as asked.
    folders: HashMap<Uuid, (CopyRequest, Uuid)>,
    /// Folders to wear once all their items are in.
    wear: Vec<Wear>,
}

struct Wear {
    folder: Uuid,
    name: String,
    count: usize,
    mode: WearMode,
    since: Instant,
}

impl ContentsCopies {
    pub fn owns(&self, request: Uuid) -> bool {
        self.folders.contains_key(&request)
    }
}

impl App {
    fn contents_copy_alert(&mut self, text: String) {
        self.world
            .notifications
            .push(Kind::Alert, aurora_i18n::tr!("common-object-contents").to_owned(), text, Data::None);
    }

    /// The contents window's copy: the move warning first when no-copy
    /// items leave the object (warn_move_inventory), then the folder.
    pub(super) fn copy_contents(&mut self, request: CopyRequest) {
        if !request.transfer.moved.is_empty() {
            let prefs = &self.settings.inventory;
            let warn = if request.transfer.scripted {
                prefs.warn_scripted_task_move
            } else {
                prefs.warn_task_move
            };
            if warn {
                self.contents_copies.confirm = Some(CopyConfirm { request, ignore: false });
                return;
            }
        }
        self.create_contents_folder(request);
    }

    pub(super) fn contents_copy_confirmed(&mut self, confirm: CopyConfirm) {
        if confirm.ignore {
            let prefs = &mut self.settings.inventory;
            if confirm.request.transfer.scripted {
                prefs.warn_scripted_task_move = false;
            } else {
                prefs.warn_task_move = false;
            }
            self.settings.save();
        }
        self.create_contents_folder(confirm.request);
    }

    /// createNewCategory under the root, or under Clothing for a copy to wear.
    fn create_contents_folder(&mut self, request: CopyRequest) {
        let inv = &self.world.inventory;
        let parent = if request.wear.is_some() {
            crate::world::appearance::system_folder(inv, FT_CLOTHING).unwrap_or(inv.root)
        } else {
            inv.root
        };
        if !rules::destination(inv, parent, &self.settings.inventory.protected) {
            self.contents_copy_alert(aurora_i18n::tr!("task-transfer-destination").to_owned());
            return;
        }
        let name = crate::build::task_inventory::contents_folder_name(&request.name);
        let change = match rules::create_folder(parent, &name) {
            Ok(change) => change,
            Err(reason) => {
                self.contents_copy_alert(reason);
                return;
            }
        };
        let Some(aurora_net::inventory::operations::Operation::Folder { id: folder, .. }) = change.operations.first() else {
            return;
        };
        let id = Uuid::new_v4();
        log::info!("contents copy: folder for {} item(s) of {}", request.items.len(), request.object);
        self.contents_copies.folders.insert(id, (request, *folder));
        self.send(NetCommand::EditInventory { request: id, change });
    }

    /// The folder is there: every item goes into it (MoveTaskInventory, one
    /// per item, like LLViewerObject::moveInventory).
    pub(super) fn contents_copy_result(&mut self, id: Uuid, result: Result<aurora_net::inventory::operations::Reply, String>) {
        let Some((request, asked)) = self.contents_copies.folders.remove(&id) else {
            return;
        };
        let reply = match result {
            Ok(reply) => reply,
            Err(reason) => {
                log::warn!("contents copy: folder not created: {reason}");
                self.contents_copy_alert(aurora_i18n::tr!("contents-copy-failed", reason = reason));
                return;
            }
        };
        let folder = reply.created.get(&asked).copied().unwrap_or(asked);
        self.world.inventory.remove(&reply.removed);
        self.world.inventory.apply(reply.contents);
        self.world.inventory.created_folders(reply.folders);
        let Some((handle, local_id)) = self
            .world
            .objects
            .index_of_uuid(&request.object)
            .and_then(|i| self.world.objects.get(i))
            .map(|o| (o.key.region, o.key.local_id))
        else {
            self.contents_copy_alert(aurora_i18n::tr!("task-transfer-source-gone").to_owned());
            return;
        };
        let items: Vec<_> = request.items.iter().filter(|item| !item.is_folder).collect();
        for item in &items {
            self.build.send(aurora_net::build::BuildCmd::MoveTaskInventory {
                handle,
                local_id,
                item_id: item.item_id,
                folder_id: folder,
            });
        }
        // the moved items leave the object: both lists forget them and the
        // next look reads the contents again
        if !request.transfer.moved.is_empty() {
            let moved = &request.transfer.moved;
            if let Some(inv) = self.build.contents.objects.get_mut(&request.object) {
                inv.items.retain(|i| !moved.contains(&i.item_id));
            }
            self.build.contents_changed(request.object);
        }
        self.flush_build();
        self.world.inventory.request(folder);
        // callbackMoveInventory: the new folder selected in the inventory
        self.inventory_ui.show_original(&self.world.inventory, folder);
        let name = crate::build::task_inventory::contents_folder_name(&request.name);
        self.inventory_ui.message = aurora_i18n::tr!("contents-copy-requested", folder = name.as_str());
        if let Some(mode) = request.wear {
            self.contents_copies.wear.push(Wear {
                folder,
                name,
                count: items.len(),
                mode,
                since: Instant::now(),
            });
        }
    }

    /// LLInventoryCopyAndWearObserver: the folder is worn once it holds as
    /// many items as the object gave.
    pub(super) fn contents_copy_tick(&mut self, now: Instant) {
        if self.contents_copies.wear.is_empty() {
            return;
        }
        let inv = &self.world.inventory;
        // one outfit change at a time: a wear sent meanwhile would be dropped
        let busy = self.appearance_ui.pending.is_some();
        let mut ready = Vec::new();
        let mut late = Vec::new();
        let mut fetch = Vec::new();
        self.contents_copies.wear.retain(|w| {
            let arrived = inv.folders.get(&w.folder).map_or(0, |f| f.items.len());
            let complete = crate::world::appearance::complete(inv, w.folder);
            if arrived >= w.count && !complete {
                fetch.push(w.folder);
            }
            if arrived >= w.count && complete && !busy && ready.is_empty() {
                ready.push((w.folder, w.mode));
                false
            } else if now.duration_since(w.since) > WEAR_TIMEOUT {
                log::warn!("contents copy: {arrived} of {} item(s) in the folder, outfit unchanged", w.count);
                late.push(w.name.clone());
                false
            } else {
                true
            }
        });
        for folder in fetch {
            self.world.inventory.request(folder);
        }
        for (folder, mode) in ready {
            log::info!("contents copy: all items arrived, wearing the folder ({mode:?})");
            self.apply_appearance_action(crate::world::appearance::Action::WearContents { folder, mode });
        }
        for name in late {
            self.contents_copy_alert(aurora_i18n::tr!("contents-copy-wear-timeout", folder = name.as_str()));
        }
    }

    /// `AURORA_DEMO_ACTIONS=open-copy|open-replace|open-add[-yes]`: the
    /// contents window's copy at frame 420, the move warning confirmed at
    /// 480 with `-yes`, the inventory opened on the new folder at 540.
    pub(super) fn demo_contents_copy(&mut self, mode: &str) {
        let (action, yes) = mode.strip_suffix("-yes").map_or((mode, false), |action| (action, true));
        let wear = match action {
            "copy" => None,
            "replace" => Some(WearMode::ReplaceOutfit),
            "add" => Some(WearMode::Append),
            _ => return,
        };
        match self.frame_count {
            420 => {
                let Some(c) = &self.interactions.contents else { return };
                match crate::ui::object_actions::copy_request(c, self.interactions.props.get(&c.target.root), &self.world) {
                    Ok(request) => {
                        log::info!(
                            "demo contents copy: {} item(s), {} moved, wear {wear:?}",
                            request.items.len(),
                            request.transfer.moved.len()
                        );
                        self.interactions.contents = None;
                        self.copy_contents(CopyRequest { wear, ..request });
                    }
                    Err(reason) => log::warn!("demo contents copy refused: {reason}"),
                }
            }
            480 if yes => {
                if let Some(confirm) = self.contents_copies.confirm.take() {
                    self.contents_copy_confirmed(CopyConfirm { ignore: false, ..confirm });
                }
            }
            540 => ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory),
            _ => {}
        }
    }
}
