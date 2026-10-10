//! Inventory actions: execute network mutations only after the UI returns.

use super::*;
use crate::ui::inventory::InvAction;
use crate::world::inventory::actions as rules;
use uuid::Uuid;

impl App {
    pub(super) fn apply_inventory_action(&mut self, action: InvAction) {
        match action {
            InvAction::Edit(change) => {
                if self.inventory_ui.pending.is_some() {
                    return;
                }
                if crate::world::appearance::cof(&self.world.inventory).is_some_and(|id| change.refresh.contains(&id))
                    && self.appearance_ui.pending.is_some()
                {
                    self.inventory_ui.message = "Attendez la mise à jour de la tenue avant de remplacer ses liens.".into();
                    return;
                }
                let request = Uuid::new_v4();
                self.inventory_ui.pending = Some(request);
                self.inventory_ui.pending_refresh.clone_from(&change.refresh);
                self.inventory_ui.created_selection = change.operations.iter().find_map(|op| {
                    if let aurora_net::inventory::operations::Operation::Folder { id, .. } = op {
                        Some(*id)
                    } else {
                        None
                    }
                });
                self.inventory_ui.message.clear();
                self.send(NetCommand::EditInventory { request, change });
            }
            InvAction::Create { parent, kind, name } => {
                if rules::destination(&self.world.inventory, parent, &self.settings.inventory.protected) {
                    if let aurora_net::inventory::operations::NewItem::Wearable(kind) = kind {
                        match crate::world::inventory::wearable::create(kind, &name, self.world.agent_id) {
                            Ok(data) => self.send(NetCommand::CreateInventoryWearable { parent, kind, name, data }),
                            Err(reason) => self.inventory_ui.message = reason,
                        }
                    } else {
                        self.send(NetCommand::CreateInventoryItem { parent, kind, name });
                    }
                }
            }
            InvAction::Appearance(action) => {
                self.inventory_ui.message.clear();
                self.apply_appearance_action(action);
            }
            InvAction::Animation { asset, local, start } => {
                if let Some((playing, was_local)) = self.inventory_ui.playing.take() {
                    if was_local || self.demo {
                        self.world.preview_animation(playing, false);
                    }
                    if !was_local {
                        self.send(NetCommand::AgentAnimation {
                            anim: playing,
                            start: false,
                        });
                    }
                }
                if start {
                    self.inventory_ui.preview = self
                        .world
                        .inventory
                        .items
                        .values()
                        .find(|it| it.asset_type == 20 && it.asset_id == asset)
                        .map(|it| it.id);
                    if local || self.demo {
                        self.world.preview_animation(asset, true);
                    }
                    if !local {
                        self.send(NetCommand::AgentAnimation { anim: asset, start: true });
                    }
                    self.inventory_ui.playing = Some((asset, local));
                }
            }
            InvAction::Sound(asset) => {
                self.scene.sounds.want(asset);
                self.scene.sounds.play_ui(self.audio_engine.as_ref(), asset);
            }
            InvAction::Preview(id) => {
                if let Some(f) = self.world.inventory.folders.get_mut(&id) {
                    f.state = crate::world::inventory::FetchState::Unknown;
                    self.world.inventory.request(id);
                } else if let Some(it) = self.world.inventory.items.get(&id).cloned() {
                    if matches!(it.asset_type, 7 | 10) && it.owner_mask & rules::COPY == 0 {
                        self.inventory_preview(id, Err("Vous n’avez pas le droit de lire ce contenu.".into()));
                    } else if it.asset_id.is_nil() && matches!(it.asset_type, 7 | 10) {
                        self.inventory_preview(
                            id,
                            Ok(if it.asset_type == 10 {
                                "default\n{\n    state_entry()\n    {\n        llSay(0, \"Hello, Avatar!\");\n    }\n}\n"
                                    .as_bytes()
                                    .to_vec()
                            } else {
                                Vec::new()
                            }),
                        );
                    } else if matches!(it.asset_type, 7 | 10 | 21 | 56 | 57) {
                        self.send(NetCommand::PreviewInventoryItem(it));
                    }
                }
            }
            InvAction::SaveContent { item, text } => {
                if let Some(it) = self.world.inventory.items.get(&item)
                    && matches!(it.asset_type, 7 | 10)
                    && it.owner_mask & (rules::MODIFY | rules::COPY) == rules::MODIFY | rules::COPY
                {
                    let data = if it.asset_type == 7 {
                        format!(
                            "Linden text version 2\n{{\nLLEmbeddedItems version 1\n{{\ncount 0\n}}\nText length {}\n{text}}}\n",
                            text.len()
                        )
                        .into_bytes()
                    } else {
                        text.into_bytes()
                    };
                    self.inventory_ui.save_pending = true;
                    self.send(NetCommand::SaveInventoryContent {
                        item,
                        asset_type: it.asset_type,
                        data,
                    });
                }
            }
            InvAction::Environment(asset, flags) => {
                if let Some(kind) = crate::world::env_select::SettingsKind::from_flags(flags) {
                    self.world.eep.request_local(asset, kind);
                }
            }
            InvAction::Restore(id) => {
                if let Some(it) = self.world.inventory.items.get(&id).cloned() {
                    self.send(NetCommand::RestoreInventoryObject(it));
                }
            }
            InvAction::Share { items, resident } => {
                if resident == self.world.agent_id || resident.is_nil() {
                    return;
                }
                let worn = crate::ui::inventory::Facts::from_world(&self.world, false).worn;
                for id in rules::roots(&self.world.inventory, &items) {
                    match rules::offer(&self.world.inventory, id, &self.settings.inventory.protected, &worn) {
                        Ok(bucket) => {
                            // LLGiveInventory removes copy-protected originals
                            // locally after the reliable offer, including folder contents.
                            let removed: Vec<_> = bucket
                                .as_chunks::<17>()
                                .0
                                .iter()
                                .filter_map(|tuple| {
                                    let id = Uuid::from_slice(&tuple[1..]).ok()?;
                                    self.world
                                        .inventory
                                        .items
                                        .get(&id)
                                        .filter(|it| it.owner_mask & rules::COPY == 0)
                                        .map(|_| id)
                                })
                                .collect();
                            self.send(NetCommand::SendImDialog {
                                to: resident,
                                dialog: 4,
                                id: Uuid::new_v4(),
                                message: rules::name(&self.world.inventory, id),
                                bucket,
                            });
                            self.world.inventory.remove(&removed);
                        }
                        Err(reason) => self.inventory_ui.message = reason,
                    }
                }
            }
            InvAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
            InvAction::AboutLandmark(item, asset) => self.show_place_profile(crate::world::place_details::Source::Landmark { item, asset }),
            InvAction::ThumbnailCopied(asset) => {
                self.inventory_ui.clipboard = vec![asset];
                self.inventory_ui.cut = false;
            }
            InvAction::Thumbnail { request, item, input } => self.inventory_image_action(request, item, input),
            InvAction::TeleportLandmark(_) => {}
        }
    }

    pub(super) fn inventory_result(&mut self, request: Uuid, result: Result<aurora_net::inventory::operations::Reply, String>) {
        if self.inventory_ui.pending != Some(request) {
            return;
        }
        self.inventory_ui.pending = None;
        self.finish_inventory_image(request, result.as_ref().err().cloned());
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents);
                if crate::world::appearance::cof(&self.world.inventory).is_some_and(|id| self.inventory_ui.pending_refresh.contains(&id)) {
                    for cmd in crate::world::appearance::sync_commands(
                        &self.world.inventory,
                        self.world.agent_id,
                        &self.world.worn_attachment_items(),
                    ) {
                        self.send(cmd);
                    }
                }
                if let Some(id) = self.inventory_ui.created_selection.and_then(|id| reply.created.get(&id)) {
                    self.inventory_ui.show_original(&self.world.inventory, *id);
                    self.inventory_ui.begin_rename(&self.world.inventory, *id);
                }
                if !reply.copies.is_empty() {
                    self.send(NetCommand::CopyInventoryItems(reply.copies));
                }
                if self.inventory_ui.consume_clipboard {
                    self.inventory_ui.clipboard.clear();
                    self.inventory_ui.cut = false;
                }
                self.inventory_ui.message.clear();
            }
            Err(reason) => {
                self.inventory_ui.message = reason;
                // A multi-operation request can partially succeed. Re-read parents
                // before another mutation instead of restoring speculative state.
                for id in &self.inventory_ui.pending_refresh {
                    if let Some(f) = self.world.inventory.folders.get_mut(id) {
                        f.state = crate::world::inventory::FetchState::Unknown;
                    }
                    self.world.inventory.request(*id);
                }
            }
        }
        self.inventory_ui.consume_clipboard = false;
        self.inventory_ui.pending_refresh.clear();
        self.inventory_ui.created_selection = None;
        for w in &mut self.inventory_ui.windows {
            w.state.message.clone_from(&self.inventory_ui.message);
        }
    }

    pub(super) fn inventory_preview(&mut self, item: Uuid, result: Result<Vec<u8>, String>) {
        let kind = self.world.inventory.items.get(&item).map(|it| it.asset_type);
        let mut editable = self
            .world
            .inventory
            .items
            .get(&item)
            .is_some_and(|it| matches!(it.asset_type, 7 | 10) && it.owner_mask & rules::MODIFY != 0);
        let result = result.and_then(|data| {
            let (text, can_edit) = crate::world::inventory::preview::text(kind.unwrap_or(-1), &data)?;
            editable &= can_edit;
            Ok(text)
        });
        if self.inventory_ui.preview == Some(item) {
            self.inventory_ui.preview_can_edit = editable;
            self.inventory_ui.preview_text = Some(result.clone());
        }
        for window in &mut self.inventory_ui.windows {
            if window.state.preview == Some(item) {
                window.state.preview_can_edit = editable;
                window.state.preview_text = Some(result.clone());
            }
        }
    }

    pub(super) fn apply_appearance_action(&mut self, action: crate::world::appearance::Action) {
        if let crate::world::appearance::Action::ShowOriginal(id) = action {
            self.panels.inventory = true;
            self.inventory_ui.show_original(&self.world.inventory, id);
            return;
        }
        if let crate::world::appearance::Action::Favorite(id) = action {
            if let Some(it) = self.world.inventory.items.get(&id)
                && self.appearance_ui.favorite_pending.insert(id)
            {
                self.send(NetCommand::SetInventoryFavorite {
                    item: id,
                    favorite: !it.favorite,
                });
            }
            return;
        }
        if self.appearance_ui.pending.is_some() {
            return;
        }
        if let crate::world::appearance::Action::Category(id, update) = action {
            self.appearance_ui.saved_new = false;
            match crate::world::appearance::category_plan(&self.world.inventory, id, update) {
                Ok(change) => {
                    let request = uuid::Uuid::new_v4();
                    self.appearance_ui.pending = Some((request, false));
                    self.appearance_ui.message.clear();
                    self.send(NetCommand::UpdateOutfitCategory { request, change });
                }
                Err(reason) => {
                    self.inventory_ui.message.clone_from(&reason);
                    self.appearance_ui.message = reason;
                }
            }
            return;
        }
        self.appearance_ui.saved_new = matches!(&action, crate::world::appearance::Action::Save(Some(_)));
        let attachments = crate::world::appearance::attachments_for_action(&self.world.inventory, self.world.agent_id, &action);
        match crate::world::appearance::plan(&self.world.inventory, action, &self.world.worn_attachment_points()) {
            Ok((change, sync)) => {
                let request = uuid::Uuid::new_v4();
                self.appearance_ui.pending = Some((request, sync));
                self.appearance_ui.attachments = attachments;
                self.appearance_ui.message.clear();
                self.send(NetCommand::UpdateOutfit { request, change });
            }
            Err(reason) => {
                self.inventory_ui.message.clone_from(&reason);
                self.appearance_ui.message = reason;
            }
        }
    }
}
