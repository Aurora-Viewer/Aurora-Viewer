//! LLPreviewNotecard for task inventory: the same document editor, with
//! object-scoped permissions, downloads and uploads (Firestorm, LGPL 2.1).

use super::*;
use crate::ui::inventory::{InvAction, InventoryUi};
use crate::world::inventory::{Inventory, actions as rules};
use aurora_net::inventory::InvItem;
use uuid::Uuid;

const GP_OBJECT_MANIPULATE: u64 = 1 << 38;

pub(super) struct TaskNotecard {
    inventory: Inventory,
    pub(super) state: InventoryUi,
}

/// Permissions of the item and its containing object are both required.
/// The notecard as we may use it: our rights on it, no modify on an object
/// we cannot edit. Any god reads any (LLTaskNotecardBridge::openItem and
/// LLPreviewNotecard::loadAsset: isGodlike()); editing goes through
/// LLAgent::allowOperation (LLPreviewNotecard::canModify), so only from
/// GOD_MAINTENANCE ([`rules::with_god`]).
fn editor_item(mut item: InvItem, agent: Uuid, member: bool, manipulate: bool, object_editable: bool, god_level: u8) -> InvItem {
    let mut mask = rules::task_notecard_mask(&item, agent, member, manipulate);
    if !object_editable {
        mask &= !rules::MODIFY;
    }
    let read = if god_level > 0 { rules::COPY } else { 0 };
    item.owner_mask = mask | read | (rules::with_god(0, god_level) & (rules::COPY | rules::MODIFY));
    item
}

impl TaskNotecard {
    fn new(item: InvItem) -> Self {
        let mut state = InventoryUi::document(item.id);
        let mut inventory = Inventory::default();
        inventory.add_items(vec![item.clone()]);
        if item.owner_mask & rules::COPY == 0 {
            state.load_document(
                7,
                false,
                &Err(aurora_i18n::tr!("app-inventory-you-are-not-allowed-to-read-this-content").into()),
            );
        } else if item.asset_id.is_nil() {
            state.load_document(
                7,
                crate::world::inventory::preview::can_edit(&inventory, item.id, 0),
                &Ok(Vec::new()),
            );
        }
        Self { inventory, state }
    }

    fn load(&mut self, item: Uuid, asset: Uuid, result: Result<Vec<u8>, String>) {
        if self.inventory.items.get(&item).is_some_and(|it| it.asset_id == asset) {
            self.state
                .load_document(7, crate::world::inventory::preview::can_edit(&self.inventory, item, 0), &result);
        }
    }
}

impl App {
    pub(crate) fn task_notecard_item(&self, object: Uuid, item: Uuid) -> Option<InvItem> {
        let idx = self.world.objects.index_of_uuid(&object)?;
        let metadata = self
            .build
            .contents
            .objects
            .get(&object)?
            .items
            .iter()
            .find(|it| it.item_id == item && it.asset_type == 7)?;
        let mut item = metadata.to_inv_item()?;
        if let Some(props) = self.build.props.get(&object).filter(|props| !props.name.is_empty()) {
            item.name = aurora_i18n::tr!("notecard-task-title", note = item.name.as_str(), object = props.name.as_str());
        }
        let group = self.world.groups.group(&item.group_id);
        Some(editor_item(
            item,
            self.world.agent_id,
            group.is_some(),
            group.is_some_and(|g| g.powers & GP_OBJECT_MANIPULATE != 0),
            self.build.contents_editable(&self.world, idx),
            self.world.god_level,
        ))
    }

    pub(super) fn open_task_notecard(&mut self, object: Uuid, item: Uuid) {
        if self.world.rlva.has("viewnote") {
            return;
        }
        if self.task_notecards.get(&(object, item)).is_some_and(|d| d.state.preview.is_some()) {
            return;
        }
        let Some(metadata) = self.task_notecard_item(object, item) else {
            return;
        };
        let download = metadata.owner_mask & rules::COPY != 0 && !metadata.asset_id.is_nil();
        self.task_notecards.insert((object, item), TaskNotecard::new(metadata));
        if download {
            self.request_task_notecard(object, item);
        }
    }

    fn request_task_notecard(&mut self, object: Uuid, item: Uuid) {
        let Some(o) = self
            .world
            .objects
            .index_of_uuid(&object)
            .and_then(|idx| self.world.objects.get(idx))
        else {
            return;
        };
        let Some(metadata) = self
            .task_notecards
            .get(&(object, item))
            .and_then(|d| d.inventory.items.get(&item))
            .cloned()
        else {
            return;
        };
        self.send(NetCommand::PreviewTaskNotecard {
            handle: o.key.region,
            object,
            item: Box::new(metadata),
        });
    }

    pub(super) fn task_notecard_preview(&mut self, object: Uuid, item: Uuid, asset: Uuid, result: Result<Vec<u8>, String>) {
        if let Some(d) = self.task_notecards.get_mut(&(object, item)) {
            d.load(item, asset, result);
        }
    }

    pub(super) fn task_notecard_saved(&mut self, request: Uuid, object: Uuid, item: Uuid, result: Result<Uuid, String>) {
        let Some(d) = self.task_notecards.get_mut(&(object, item)) else {
            return;
        };
        if !d.state.finish_content_save(request, result.as_ref().err().map(String::as_str)) {
            return;
        }
        if let Ok(asset) = result {
            if let Some(metadata) = d.inventory.items.get_mut(&item) {
                metadata.asset_id = asset;
            }
            if let Some(metadata) = self
                .build
                .contents
                .objects
                .get_mut(&object)
                .and_then(|inv| inv.items.iter_mut().find(|it| it.item_id == item))
            {
                metadata.asset_id = asset;
            }
            // Re-download records the simulator may have rewritten for embedded
            // permissions. The editor preserves any draft made since the save.
            self.request_task_notecard(object, item);
            if !self.demo
                && let Some(o) = self
                    .world
                    .objects
                    .index_of_uuid(&object)
                    .and_then(|idx| self.world.objects.get(idx))
            {
                self.send(NetCommand::RequestTaskInventory {
                    handle: o.key.region,
                    local_id: o.key.local_id,
                    object,
                });
            }
        }
    }

    pub(super) fn show_task_notecards(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.world.rlva.has("viewnote") {
            return;
        }
        if let Some((object, item)) = self.build.ui.open_notecard.take() {
            self.open_task_notecard(object, item);
        }
        let keys: Vec<_> = self.task_notecards.keys().copied().collect();
        let mut saves = Vec::new();
        let mut embedded = Vec::new();
        let mut descriptions = Vec::new();
        let notecard_view = self.settings.inventory.notecard_view;
        for (object, item) in keys {
            let metadata = self.task_notecard_item(object, item);
            let Some(d) = self.task_notecards.get_mut(&(object, item)) else {
                continue;
            };
            let allowed = metadata
                .as_ref()
                .is_some_and(|it| it.owner_mask & (rules::COPY | rules::MODIFY) == rules::COPY | rules::MODIFY);
            if let Some(model) = d.inventory.items.get_mut(&item) {
                model.owner_mask = metadata.as_ref().map_or(0, |it| it.owner_mask);
                if let Some(metadata) = &metadata {
                    model.desc.clone_from(&metadata.desc);
                }
            }
            if d.state.preview_source.is_some() {
                d.state.preview_can_edit = allowed;
            }
            if metadata.is_none() {
                d.state.preview_message = aurora_i18n::tr!("notecard-task-unavailable").into();
            }
            for action in ui::inventory::show_document(
                ctx,
                p,
                &self.skin.icons,
                &d.inventory,
                &mut d.state,
                &self.world.inventory,
                &self.ui_images,
                &mut self.settings.inventory,
                &self.settings.script_editor,
            ) {
                if let InvAction::SaveContent { text, close, .. } = action {
                    saves.push((object, item, text, close));
                } else if let InvAction::Edit(change) = action {
                    for operation in change.operations {
                        if let aurora_net::inventory::operations::Operation::Patch { id, folder: false, body } = operation
                            && id == item
                            && body.has("desc")
                        {
                            descriptions.push((object, item, body["desc"].as_str().to_owned()));
                        }
                    }
                } else if let InvAction::Embedded {
                    editor,
                    notecard,
                    marker,
                    copy,
                } = action
                {
                    embedded.push((editor, notecard, marker, copy, object));
                }
            }
        }
        if self.settings.inventory.notecard_view != notecard_view {
            self.settings.save();
        }
        for (editor, notecard, marker, copy, object) in embedded {
            self.open_notecard_attachment(editor, notecard, marker, copy, object);
        }
        for (object, item, text, close) in saves {
            self.save_task_notecard(object, item, text, close);
        }
        for (object, item, description) in descriptions {
            if self
                .task_notecard_item(object, item)
                .is_some_and(|it| it.owner_mask & rules::MODIFY != 0)
                && let Some(idx) = self.world.objects.index_of_uuid(&object)
            {
                self.build.describe_item(&self.world, idx, item, &description);
            }
        }
        self.task_notecards.retain(|_, d| d.state.preview.is_some());
    }

    pub(super) fn task_document_state(&mut self, editor: Uuid) -> Option<&mut InventoryUi> {
        self.task_notecards
            .values_mut()
            .find(|d| d.state.ui_id == editor)
            .map(|d| &mut d.state)
    }

    fn save_task_notecard(&mut self, object: Uuid, item: Uuid, text: String, close: bool) {
        let Some(metadata) = self.task_notecard_item(object, item) else {
            if let Some(d) = self.task_notecards.get_mut(&(object, item)) {
                d.state.preview_message = aurora_i18n::tr!("notecard-task-unavailable").into();
            }
            return;
        };
        let allowed = metadata.owner_mask & (rules::COPY | rules::MODIFY) == rules::COPY | rules::MODIFY;
        let Some(o) = self
            .world
            .objects
            .index_of_uuid(&object)
            .and_then(|idx| self.world.objects.get(idx))
        else {
            return;
        };
        let handle = o.key.region;
        let Some(d) = self.task_notecards.get_mut(&(object, item)) else {
            return;
        };
        if !allowed {
            d.state.preview_message = aurora_i18n::tr!("notecard-edit-denied").into();
            return;
        }
        let request = Uuid::new_v4();
        match d.state.start_content_save(request, 7, text, close) {
            Ok(data) => self.send(NetCommand::SaveTaskNotecard {
                request,
                handle,
                object,
                item,
                data,
            }),
            Err(reason) => d.state.preview_message = reason,
        }
    }

    pub(super) fn demo_task_notecard(&mut self, mode: &str) {
        let Some(object) = self
            .world
            .objects
            .iter()
            .find(|(_, o)| o.pcode == 9 && o.parent_id == 0)
            .map(|(_, o)| o.full_id)
        else {
            return;
        };
        let item = Uuid::from_u128(8303);
        self.build.props.insert(
            object,
            aurora_net::build::ObjectProps {
                object_id: object,
                owner_id: self.world.agent_id,
                owner_mask: 0x7fffffff,
                name: aurora_i18n::tr!("notecard-task-demo-object").into(),
                ..Default::default()
            },
        );
        self.build.on_task_inventory(
            &self.world,
            object,
            Some(1),
            vec![aurora_net::build::TaskItem {
                item_id: item,
                parent_id: object,
                owner_id: self.world.agent_id,
                name: aurora_i18n::tr!("notecard-task-demo-title").into(),
                description: aurora_i18n::tr!("notecard-demo-description").into(),
                asset_type: 7,
                inv_type: 7,
                asset_id: Uuid::from_u128(8304),
                owner_mask: if mode == "note-task-readonly" {
                    rules::COPY
                } else {
                    rules::COPY | rules::MODIFY
                },
                ..Default::default()
            }],
        );
        self.open_task_notecard(object, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item() -> InvItem {
        InvItem {
            id: Uuid::from_u128(3),
            owner: Uuid::from_u128(2),
            asset_type: 7,
            owner_mask: rules::COPY | rules::MODIFY,
            ..Default::default()
        }
    }

    #[test]
    fn item_permissions_and_object_permissions_are_both_required() {
        let agent = Uuid::from_u128(2);
        let metadata = editor_item(item(), agent, false, false, false, 0);
        assert_eq!(metadata.owner_mask, rules::COPY);
        let note = TaskNotecard::new(metadata);
        assert_eq!(note.state.preview_text, Some(Ok(String::new())));
        assert!(!note.state.preview_can_edit);
        let mut foreign = item();
        foreign.owner = Uuid::from_u128(8);
        foreign.group_mask = rules::COPY;
        assert_eq!(editor_item(foreign.clone(), agent, false, false, true, 0).owner_mask, 0);
        assert_eq!(editor_item(foreign.clone(), agent, true, false, true, 0).owner_mask, rules::COPY);
        foreign.group_owned = true;
        assert_eq!(
            editor_item(foreign.clone(), agent, true, true, true, 0).owner_mask,
            rules::COPY | rules::MODIFY
        );
        // any god reads a stranger's no-copy notecard; editing it takes
        // GOD_MAINTENANCE (LLAgent::allowOperation), not OpenSim's 200 to 249
        foreign.group_owned = false;
        foreign.group_mask = 0;
        for level in [1, 225, rules::GOD_MAINTENANCE - 1] {
            let god = editor_item(foreign.clone(), agent, false, false, true, level).owner_mask;
            assert_eq!(god & (rules::COPY | rules::MODIFY), rules::COPY, "level {level}");
        }
        let god = editor_item(foreign, agent, false, false, false, rules::GOD_MAINTENANCE).owner_mask;
        assert_eq!(god & (rules::COPY | rules::MODIFY), rules::COPY | rules::MODIFY);
    }

    #[test]
    fn old_assets_and_late_downloads_do_not_replace_object_drafts() {
        let mut metadata = item();
        metadata.asset_id = Uuid::from_u128(4);
        let mut note = TaskNotecard::new(metadata.clone());
        note.load(metadata.id, Uuid::from_u128(5), Ok(b"obsolete".to_vec()));
        assert!(note.state.preview_text.is_none());
        note.load(metadata.id, metadata.asset_id, Ok(b"saved".to_vec()));
        note.state.preview_text = Some(Ok("draft".into()));
        note.state.document_changed();
        note.load(metadata.id, metadata.asset_id, Ok(b"late".to_vec()));
        assert_eq!(note.state.preview_text, Some(Ok("draft".into())));
        assert!(note.state.preview_dirty);
    }
}
