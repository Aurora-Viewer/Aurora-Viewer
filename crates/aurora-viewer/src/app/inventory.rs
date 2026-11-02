//! Inventory actions: execute network mutations only after the UI returns.

use super::*;
use crate::ui::inventory::InvAction;
use crate::world::inventory::actions as rules;
use std::collections::VecDeque;
use uuid::Uuid;

impl App {
    /// `AURORA_DEMO_INVENTORY=big,toggle=…,churn=…`: open and close the
    /// window, and change one item now and then, once in the world.
    pub(super) fn demo_big_inventory(&mut self, big: &crate::world::inventory::demo::Big) {
        use std::sync::OnceLock;
        use std::sync::atomic::{AtomicU64, Ordering};
        static START: OnceLock<Instant> = OnceLock::new();
        static CHURNS: AtomicU64 = AtomicU64::new(0);
        if !matches!(self.screen, Screen::World) {
            return;
        }
        let elapsed = START.get_or_init(Instant::now).elapsed();
        if big.toggle > 0.0 {
            let open = ((elapsed.as_secs_f32() / big.toggle) as u64).is_multiple_of(2);
            if open != self.panels.inventory {
                log::info!("demo inventory window open={open}");
                self.panels.inventory = open;
            }
        }
        if let Some(due) = (elapsed.as_millis() as u64).checked_div(big.churn_ms)
            && CHURNS.swap(due, Ordering::Relaxed) != due
        {
            let id = Uuid::from_u128(21_000_000 + u128::from(due % 2_000));
            if let Some(mut item) = self.world.inventory.items.get(&id).cloned() {
                item.desc = format!("Update {due}");
                self.world.inventory.server_updated(Vec::new(), vec![item]);
            }
        }
    }

    fn take_task_inventory(&mut self, mut request: crate::build::task_inventory::TransferRequest) {
        use crate::build::task_inventory::{self, TransferKind};
        // Both object inventory windows receive the same TaskInventory events.
        // Refresh the item before accepting a confirmation: permissions or the
        // contents can have changed while the dialog was open.
        if let Some(inv) = self.build.contents.objects.get(&request.object) {
            let Some(item) = inv.items.iter().find(|item| item.item_id == request.item.item_id) else {
                self.inventory_ui.message = aurora_i18n::tr!("task-transfer-unavailable").into();
                return;
            };
            request.item = item.clone();
        }
        if !task_inventory::destination(
            &self.world.inventory,
            request.folder,
            &request.item,
            &self.settings.inventory.protected,
        ) {
            self.inventory_ui.message = aurora_i18n::tr!("task-transfer-destination").into();
            return;
        }
        let kind = match self.build.task_transfer_kind(&self.world, request.object, &request.item) {
            Ok(kind) => kind,
            Err(error) => {
                self.inventory_ui.message = error.into();
                self.build.status = error.into();
                return;
            }
        };
        if let TransferKind::Move { scripted } = kind {
            let prefs = &mut self.settings.inventory;
            let warn = if scripted {
                &mut prefs.warn_scripted_task_move
            } else {
                &mut prefs.warn_task_move
            };
            if *warn && !task_inventory::approved(request.approved, scripted) {
                // the items of one drop share the dialog of their object and folder
                let mut request = Some(request);
                for pending in &mut self.task_transfer {
                    request = request.and_then(|r| pending.join(r));
                }
                if let Some(request) = request {
                    self.task_transfer
                        .push_back(ui::inventory::task_drop::PendingTransfer::new(request, scripted));
                }
                return;
            }
            if *warn && request.ignore_warning && request.approved == Some(scripted) {
                *warn = false;
                self.settings.save();
                // the dialogs still waiting for this warning need no answer now
                let (released, waiting): (VecDeque<_>, VecDeque<_>) = std::mem::take(&mut self.task_transfer)
                    .into_iter()
                    .partition(|pending| pending.scripted == scripted);
                self.task_transfer = waiting;
                for waiting in released.into_iter().flat_map(|pending| pending.requests) {
                    self.take_task_inventory(waiting);
                }
            }
        }
        let Some(o) = self
            .world
            .objects
            .index_of_uuid(&request.object)
            .and_then(|i| self.world.objects.get(i))
        else {
            return;
        };
        self.build.send(aurora_net::build::BuildCmd::MoveTaskInventory {
            handle: o.key.region,
            local_id: o.key.local_id,
            item_id: request.item.item_id,
            folder_id: request.folder,
        });
        if matches!(kind, TransferKind::Move { .. }) {
            if let Some(inv) = self.build.contents.objects.get_mut(&request.object) {
                inv.items.retain(|i| i.item_id != request.item.item_id);
            }
            if let Some(c) = self.interactions.contents.as_mut().filter(|c| c.target.object == request.object)
                && let Some(Ok(items)) = c.result.as_mut()
            {
                items.retain(|i| i.id != request.item.item_id);
            }
            self.build.contents_changed(request.object);
        }
        self.flush_build();
        self.world.inventory.request(request.folder);
        self.inventory_ui.message = aurora_i18n::tr!("task-transfer-requested").into();
    }

    fn document_state(&mut self, editor: Uuid) -> Option<&mut crate::ui::inventory::InventoryUi> {
        if self.inventory_ui.ui_id == editor {
            return Some(&mut self.inventory_ui);
        }
        if let Some(index) = self.inventory_ui.windows.iter().position(|w| w.state.ui_id == editor) {
            return Some(self.inventory_ui.windows[index].state.as_mut());
        }
        self.task_document_state(editor)
    }

    pub(super) fn notecard_copy_submitted(&mut self, editor: Uuid, notecard: Uuid, result: Result<(), String>) {
        if let Some(st) = self.document_state(editor).filter(|st| st.preview == Some(notecard)) {
            st.preview_message = result.err().unwrap_or_else(|| aurora_i18n::tr!("notecard-copy-requested").into());
        }
    }

    pub(super) fn notecard_inventory_copied(
        &mut self,
        editor: Uuid,
        notecard: Uuid,
        open: bool,
        result: Result<aurora_net::inventory::InvItem, String>,
    ) {
        match result {
            Ok(item) => {
                self.world.inventory.add_items(vec![item.clone()]);
                if let Some(st) = self.document_state(editor).filter(|st| st.preview == Some(notecard)) {
                    st.preview_message = aurora_i18n::tr!("notecard-copy-complete").into();
                    if open && item.asset_type == 3 {
                        self.show_place_profile(crate::world::place_details::Source::Landmark {
                            item: item.id,
                            asset: item.asset_id,
                        });
                    }
                }
            }
            Err(reason) => self.notecard_copy_submitted(editor, notecard, Err(reason)),
        }
    }

    pub(super) fn open_notecard_attachment(&mut self, editor: Uuid, notecard: Uuid, marker: char, copy: bool, object: Uuid) {
        let Some(st) = self.document_state(editor).filter(|st| st.preview == Some(notecard)) else {
            return;
        };
        if st.save_pending
            || !st
                .preview_text
                .as_ref()
                .is_some_and(|r| r.as_ref().is_ok_and(|text| text.contains(marker)))
        {
            return;
        }
        let Some(attachment) = st.preview_source.as_ref().and_then(|source| source.attachment(marker)) else {
            return;
        };
        if !attachment.saved {
            st.preview_message = aurora_i18n::tr!("notecard-attachment-unsaved").into();
            st.dialog = Some(crate::ui::inventory::EditDialog::SaveEmbedded(notecard));
            return;
        }
        let item = attachment.item.clone();
        if copy || item.asset_type == 3 {
            if !copy
                && let Some(original) = self
                    .world
                    .inventory
                    .items
                    .values()
                    .find(|it| it.asset_type == 3 && it.asset_id == item.asset_id)
            {
                self.show_place_profile(crate::world::place_details::Source::Landmark {
                    item: original.id,
                    asset: original.asset_id,
                });
                return;
            }
            let handle = self
                .world
                .objects
                .index_of_uuid(&object)
                .and_then(|idx| self.world.objects.get(idx))
                .map(|o| o.key.region);
            self.send(NetCommand::CopyNotecardInventory {
                editor,
                open: !copy,
                handle,
                object,
                notecard,
                item: Box::new(item),
            });
            return;
        }
        match item.asset_type {
            0 => st.attachment_preview = Some(item),
            2 => {
                let avatar = item.desc.parse::<Uuid>().ok().filter(|id| !id.is_nil()).unwrap_or(item.creator);
                if !avatar.is_nil() {
                    self.profile_ui.open(&mut self.world, avatar);
                }
            }
            1 => {
                st.dialog = Some(crate::ui::inventory::EditDialog::CopyEmbedded(notecard, marker));
                self.scene.sounds.want(item.asset_id);
                self.scene.sounds.play_ui(self.audio_engine.as_ref(), item.asset_id);
            }
            _ => st.dialog = Some(crate::ui::inventory::EditDialog::CopyEmbedded(notecard, marker)),
        }
    }

    pub(super) fn submit_inventory_change(&mut self, mut change: aurora_net::inventory::operations::Mutation) {
        if self.inventory_ui.pending.is_some() {
            self.inventory_ui.consume_clipboard = false;
            return;
        }
        if crate::world::appearance::cof(&self.world.inventory).is_some_and(|id| change.refresh.contains(&id))
            && self.appearance_ui.pending.is_some()
        {
            self.inventory_ui.message = aurora_i18n::tr!("app-inventory-wait-for-the-outfit-update-before-replacing-its-links").into();
            return;
        }
        let request = Uuid::new_v4();
        change.defer_folder_contents = true;
        log::info!(
            "inventory mutation: {} operations, {} item copies",
            change.operations.len(),
            change.copies.len()
        );
        self.inventory_ui.begin_mutation(request, &change);
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

    pub(super) fn apply_inventory_action(&mut self, action: InvAction) {
        if let InvAction::Share { resident, .. } = &action
            && !self.world.rlva.allows("share", &resident.to_string())
        {
            self.world
                .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            return;
        }
        match action {
            InvAction::DefaultPermissions => self.inventory_ui.default_permissions_open = true,
            InvAction::ExportMenuNotecard { name, text } => {
                if let Some(command) = ui::inventory::context_notecards::export(&mut self.inventory_ui, &self.world.inventory, name, text) {
                    self.send(command);
                }
            }
            InvAction::LoadMenuNotecard(item) => {
                if let Some(command) = ui::inventory::context_notecards::load(&mut self.inventory_ui, &self.world.inventory, item) {
                    self.send(command);
                }
            }
            InvAction::TakeTask(request) => self.take_task_inventory(*request),
            InvAction::Edit(change) => {
                let cut = std::mem::take(&mut self.inventory_ui.consume_clipboard);
                if let Some(change) = self.listings_ui.guard_inventory_change(&self.world.inventory, change, cut) {
                    self.inventory_ui.consume_clipboard = cut;
                    self.submit_inventory_change(change);
                }
            }
            InvAction::Create { parent, kind, name } => {
                if rules::destination(&self.world.inventory, parent, &self.settings.inventory.protected) {
                    if let aurora_net::inventory::operations::NewItem::Wearable(kind) = kind {
                        match crate::world::inventory::wearable::create_with_permissions(
                            kind,
                            &name,
                            self.world.agent_id,
                            self.settings.inventory.creation_permissions.wearables.permissions(),
                        ) {
                            Ok(data) => self.send(NetCommand::CreateInventoryWearable {
                                parent,
                                kind,
                                name,
                                data,
                                permissions: None,
                            }),
                            Err(reason) => self.inventory_ui.message = reason,
                        }
                    } else {
                        self.send(NetCommand::CreateInventoryItem {
                            parent,
                            kind,
                            name,
                            permissions: None,
                        });
                    }
                }
            }
            InvAction::PrepareOutfit(folder) => {
                for command in self
                    .appearance_ui
                    .prepare_contents(&mut self.world.inventory, self.world.agent_id, folder)
                {
                    self.send(command);
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
                    self.record_library_asset(asset, 20);
                    self.inventory_ui.open_animation(&self.egui_ctx, &self.world.inventory, asset);
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
                self.record_library_asset(asset, 1);
                self.scene.sounds.want(asset);
                self.scene.sounds.play_ui(self.audio_engine.as_ref(), asset);
            }
            InvAction::TriggerSound(asset) => {
                self.record_library_asset(asset, 1);
                self.send_sound_trigger(asset);
            }
            InvAction::WearCopy { item, replace } => self.wear_copy(item, replace),
            InvAction::EditEnvironment(id) => self.env_editor_load(id),
            InvAction::Preview(id) => {
                if self
                    .world
                    .inventory
                    .items
                    .get(&id)
                    .is_some_and(|it| !self.world.inventory.can_preview(it.asset_type))
                {
                    return;
                }
                if crate::ui::texture_library::catalogue::resource(&self.world.inventory, id).is_some() {
                    self.texture_library.open_preview(&self.egui_ctx, &self.world, id);
                    return;
                }
                if self.world.inventory.items.get(&id).is_some_and(|it| it.asset_type == 21) {
                    self.gesture_action(crate::ui::gestures::Action::Open(id));
                    return;
                }
                if let Some(f) = self.world.inventory.folders.get_mut(&id) {
                    f.state = crate::world::inventory::FetchState::Unknown;
                    self.world.inventory.request(id);
                } else if let Some(it) = self.world.inventory.items.get(&id).cloned() {
                    if Some(it.id) == crate::shape_editor::worn_shape(&self.world) {
                        self.open_shape_editor(it.id);
                        return;
                    }
                    if Some(it.id) == crate::physics_editor::worn_physics(&self.world) {
                        self.open_physics_editor(it.id);
                        return;
                    }
                    if matches!(it.asset_type, 7 | 10) && it.owner_mask & rules::COPY != 0 {
                        self.record_library_use(id);
                    }
                    if it.asset_type == 10 {
                        self.inventory_ui.preview = self.inventory_ui.preview.filter(|preview| *preview != id);
                        self.open_script(aurora_net::scripts::ScriptTarget {
                            object: Uuid::nil(),
                            item: id,
                        });
                        return;
                    }
                    if !self.inventory_ui.open_document(&self.egui_ctx, id, it.asset_type) {
                        return;
                    }
                    if matches!(it.asset_type, 7 | 10) && it.owner_mask & rules::COPY == 0 {
                        self.inventory_preview(
                            id,
                            it.asset_id,
                            Err(aurora_i18n::tr!("app-inventory-you-are-not-allowed-to-read-this-content").into()),
                        );
                    } else if it.asset_id.is_nil() && matches!(it.asset_type, 7 | 10) {
                        self.inventory_preview(
                            id,
                            it.asset_id,
                            Ok(if it.asset_type == 10 {
                                "default\n{\n    state_entry()\n    {\n        llSay(0, \"Hello, Avatar!\");\n    }\n}\n"
                                    .as_bytes()
                                    .to_vec()
                            } else {
                                Vec::new()
                            }),
                        );
                    } else if matches!(it.asset_type, 7 | 10 | 21 | 56) {
                        self.send(NetCommand::PreviewInventoryItem(it));
                    }
                }
            }
            InvAction::SaveContent { editor, item, text, close } => {
                let allowed = crate::world::inventory::preview::can_edit(&self.world.inventory, item);
                let kind = self.world.inventory.items.get(&item).map_or(-1, |it| it.asset_type);
                let busy = self.inventory_ui.preview == Some(item) && self.inventory_ui.save_pending
                    || self
                        .inventory_ui
                        .windows
                        .iter()
                        .any(|w| w.state.preview == Some(item) && w.state.save_pending);
                let st = if self.inventory_ui.ui_id == editor {
                    Some(&mut self.inventory_ui)
                } else {
                    self.inventory_ui
                        .windows
                        .iter_mut()
                        .find(|w| w.id == editor)
                        .map(|w| w.state.as_mut())
                };
                let Some(st) = st.filter(|st| st.preview == Some(item)) else {
                    return;
                };
                if !allowed {
                    st.preview_message = aurora_i18n::tr!("notecard-edit-denied").into();
                    st.close_after_save = false;
                    return;
                }
                if busy {
                    st.preview_message = aurora_i18n::tr!("notecard-saving").into();
                    return;
                }
                let request = Uuid::new_v4();
                match st.start_content_save(request, kind, text, close) {
                    Ok(data) => self.send(NetCommand::SaveInventoryContent {
                        request,
                        item,
                        asset_type: kind,
                        data,
                    }),
                    Err(reason) => st.preview_message = reason,
                }
            }
            InvAction::Embedded {
                editor,
                notecard,
                marker,
                copy,
            } => {
                self.open_notecard_attachment(editor, notecard, marker, copy, Uuid::nil());
            }
            InvAction::Environment(asset, flags) => {
                if let Some(kind) = crate::world::env_select::SettingsKind::from_flags(flags) {
                    self.world.eep.request_local(asset, kind);
                }
            }
            InvAction::EnvironmentParcel(item) => self.inventory_environment_parcel(item),
            InvAction::Marketplace(item, command) => self.inventory_marketplace_command(item, command),
            InvAction::ShowMain(item) => {
                self.inventory_ui.show_original(&self.world.inventory, item);
                ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
            }
            InvAction::FindLinks(item) => {
                self.inventory_ui.find_links(item);
                ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
            }
            InvAction::MarketplacePaste { to, ids, copy } => self.paste_marketplace_inventory(to, ids, copy),
            InvAction::ExportTextures { items, png } => self.texture_library.begin_exports(&self.egui_ctx, &self.world, items, png),
            InvAction::WearLibrary { item, replace, point } => self.wear_copy_at(item, replace, point),
            InvAction::WearSelection { items, replace, point } => self.wear_selection(items, replace, point),
            InvAction::RefreshAttachment(item) => {
                if let Some(id) = self
                    .world
                    .worn_attachment(&item)
                    .and_then(|index| self.world.objects.get(index))
                    .map(|object| object.full_id)
                {
                    self.refresh_object_textures(id);
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
                            let name = rules::name(&self.world.inventory, id);
                            let folder = self.world.inventory.folders.contains_key(&id);
                            self.send(NetCommand::SendImDialog {
                                to: resident,
                                dialog: 4,
                                id: Uuid::new_v4(),
                                message: name.clone(),
                                bucket,
                            });
                            self.world.inventory_offer_sent(resident, &name, folder);
                            self.world.inventory.remove(&removed);
                        }
                        Err(reason) => self.inventory_ui.message = reason,
                    }
                }
            }
            InvAction::Profile(id) => self.profile_ui.open(&mut self.world, id),
            InvAction::AboutLandmark(item, asset) => self.show_place_profile(crate::world::place_details::Source::Landmark { item, asset }),
            InvAction::CopyLandmarkSlurl(asset) => {
                self.world
                    .landmarks
                    .request_action(asset, crate::world::landmarks::Action::CopySlurl, Instant::now());
            }
            InvAction::ShowLandmarkOnMap(asset) => {
                self.world
                    .landmarks
                    .request_action(asset, crate::world::landmarks::Action::ShowOnMap, Instant::now());
            }
            InvAction::ThumbnailCopied(asset) => {
                self.inventory_ui.clipboard = vec![asset];
                self.inventory_ui.cut = false;
            }
            InvAction::Thumbnail { request, item, input } => self.inventory_image_action(request, item, input),
            InvAction::TeleportLandmark(_) => {}
            InvAction::Gesture(action) => self.gesture_action(action),
        }
    }

    pub(super) fn inventory_result(&mut self, request: Uuid, result: Result<aurora_net::inventory::operations::Reply, String>) {
        if let Some(refresh) = self.inventory_ui.finish_copy(request, result.is_ok()) {
            if let Err(reason) = result {
                self.inventory_ui.message = reason;
                for id in refresh {
                    if let Some(folder) = self.world.inventory.folders.get(&id) {
                        self.world.inventory.queue.push((id, folder.library));
                    }
                }
            }
            for w in &mut self.inventory_ui.windows {
                w.state.message.clone_from(&self.inventory_ui.message);
            }
            return;
        }
        if self.inventory_ui.pending != Some(request) {
            return;
        }
        let thumbnail_upload = self.inventory_images.contains_request(request);
        let result = match result {
            Ok(reply) if !reply.copies.is_empty() => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents.clone());
                self.world.inventory.created_folders(reply.folders.clone());
                if let Some(id) = self.inventory_ui.created_selection.and_then(|id| reply.created.get(&id)) {
                    self.inventory_ui.show_original(&self.world.inventory, *id);
                }
                self.inventory_ui.watch_copy(request, &reply);
                self.send(NetCommand::FinishInventoryCopies { request, reply });
                return;
            }
            result => result,
        };
        self.inventory_ui.finish_bulk_permissions(result.as_ref().err().map(String::as_str));
        self.inventory_ui.finish_mutation(result.is_ok());
        if result.as_ref().is_ok_and(|reply| reply.moves_queued) {
            self.inventory_ui.moves_queued_notice();
        }
        if thumbnail_upload {
            self.finish_inventory_image_upload(request, result.as_ref().err().cloned());
        } else {
            self.finish_inventory_image(request, result.as_ref().err().cloned());
        }
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents);
                self.world.inventory.created_folders(reply.folders);
                if crate::world::appearance::cof(&self.world.inventory).is_some_and(|id| self.inventory_ui.pending_refresh.contains(&id)) {
                    let sync = crate::world::appearance::sync_plan(&self.world.inventory, self.world.agent_id, &self.outfit_sync_worn());
                    for cmd in sync.commands {
                        self.send(cmd);
                    }
                    self.world
                        .outfit
                        .wear_when_known(sync.unknown, self.world.agent_id, std::time::Instant::now());
                }
                if let Some(id) = self.inventory_ui.created_selection.and_then(|id| reply.created.get(&id))
                    && !self.inventory_ui.select_created_listing(&self.world.inventory, *id)
                {
                    self.inventory_ui.show_original(&self.world.inventory, *id);
                    self.inventory_ui.begin_rename(&self.world.inventory, *id);
                }
                if self.inventory_ui.consume_clipboard {
                    self.inventory_ui.clipboard.clear();
                    self.inventory_ui.cut = false;
                }
                self.inventory_ui.message.clear();
            }
            Err(reason) => {
                if !thumbnail_upload {
                    self.inventory_ui.message = reason;
                }
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

    pub(super) fn inventory_preview(&mut self, item: Uuid, asset: Uuid, result: Result<Vec<u8>, String>) {
        if self.world.inventory.items.get(&item).is_none_or(|it| it.asset_id != asset) {
            return;
        }
        if self.env_editor.loading == Some(item) {
            self.env_editor_asset(item, result);
            return;
        }
        if self.day_editor_asset(item, &result) {
            return;
        }
        let kind = self.world.inventory.items.get(&item).map_or(-1, |it| it.asset_type);
        let editable = crate::world::inventory::preview::can_edit(&self.world.inventory, item);
        if self.inventory_ui.preview == Some(item) {
            self.inventory_ui.load_document(kind, editable, &result);
        }
        for window in &mut self.inventory_ui.windows {
            if window.state.preview == Some(item) {
                window.state.load_document(kind, editable, &result);
            }
        }
    }

    pub(super) fn inventory_content_saved(&mut self, request: Uuid, item: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        let error = result.as_ref().err().map(String::as_str);
        self.inventory_ui.finish_content_save(request, error);
        for w in &mut self.inventory_ui.windows {
            w.state.finish_content_save(request, error);
        }
        if let Ok(it) = result {
            // Servers can replace the inventory item when embedded permissions
            // change. Follow the returned ID rather than retaining a broken editor.
            if it.id != item {
                if self.inventory_ui.preview == Some(item) {
                    self.inventory_ui.preview = Some(it.id);
                }
                for w in &mut self.inventory_ui.windows {
                    if w.state.preview == Some(item) {
                        w.state.preview = Some(it.id);
                    }
                }
                self.world.inventory.remove(&[item]);
            }
            self.world.inventory.add_items(vec![it.clone()]);
            if it.asset_type == 7 {
                // The simulator may rewrite embedded inventory records.
                // load_document protects any drafts in the other windows.
                self.send(NetCommand::PreviewInventoryItem(it));
            }
        }
    }

    /// A Library wearable double-clicked: LLAppearanceMgr::wearItemsOnAvatar
    /// copies it into the inventory (copy_inventory_item, the server's default
    /// folder of its type) and wears the copy once it arrives (wear_on_avatar_cb).
    fn wear_copy(&mut self, item: Uuid, replace: bool) {
        self.wear_copy_at(item, replace, 0);
    }

    fn wear_copy_at(&mut self, item: Uuid, replace: bool, point: u8) {
        self.wear_selection(vec![item], replace, point);
    }

    fn wear_selection(&mut self, items: Vec<Uuid>, replace: bool, point: u8) {
        let group_id = Uuid::new_v4();
        let mut group = ui::inventory::WearCopyGroup {
            replace,
            point,
            items: vec![None; items.len()],
            pending: 0,
            failed: false,
        };
        let mut copies = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let Some(info) = self.world.inventory.items.get(&item) else {
                return;
            };
            if crate::world::inventory::actions::library(&self.world.inventory, item) {
                let request = Uuid::new_v4();
                copies.push((
                    request,
                    index,
                    aurora_net::inventory::operations::CopyItem {
                        item,
                        owner: info.owner,
                        parent: Uuid::nil(),
                        name: String::new(),
                    },
                ));
                group.pending += 1;
            } else {
                group.items[index] = Some(item);
            }
        }
        if group.pending == 0 {
            self.apply_appearance_action(crate::world::appearance::Action::WearItems {
                items: group.items.into_iter().flatten().collect(),
                replace,
                point,
            });
            return;
        }
        self.inventory_ui.wear_copy_groups.insert(group_id, group);
        for (request, index, copy) in copies {
            self.inventory_ui.wear_copies.insert(request, (group_id, index));
            self.send(NetCommand::CopyInventoryItem { request, copy });
        }
    }

    /// wear_on_avatar_cb: the copy arrived, it is worn like any inventory item.
    pub(super) fn wear_copied(&mut self, request: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        let Some((group_id, index)) = self.inventory_ui.wear_copies.remove(&request) else {
            return;
        };
        let Some(group) = self.inventory_ui.wear_copy_groups.get_mut(&group_id) else {
            return;
        };
        group.pending = group.pending.saturating_sub(1);
        match result {
            Ok(it) => {
                let item = it.id;
                log::info!("library copy arrived, wearing it: {item} in {}", it.parent);
                // the server's inventory update may follow this callback
                self.world.inventory.add_items(vec![it]);
                group.items[index] = Some(item);
            }
            Err(reason) => {
                self.inventory_ui.message = reason;
                group.failed = true;
            }
        }
        if group.pending == 0 {
            let group = self.inventory_ui.wear_copy_groups.remove(&group_id).expect("pending group");
            if !group.failed {
                self.apply_appearance_action(crate::world::appearance::Action::WearItems {
                    items: group.items.into_iter().flatten().collect(),
                    replace: group.replace,
                    point: group.point,
                });
            }
        }
    }

    pub(super) fn apply_appearance_action(&mut self, action: crate::world::appearance::Action) {
        self.apply_appearance_action_from(action, Uuid::nil());
    }

    pub(super) fn apply_appearance_action_from(&mut self, action: crate::world::appearance::Action, actor: Uuid) {
        let allow_recovery = self
            .appearance_ui
            .pending
            .is_none_or(|(request, _)| self.world.attachment_inventory.owns(request));
        let (detach, action) = self.world.recover_attachment_removal(action, allow_recovery);
        let recovered = !detach.is_empty();
        for command in detach {
            self.send_from(command, actor);
        }
        let Some(action) = action else {
            if recovered {
                self.inventory_ui.message.clear();
                self.appearance_ui.message.clear();
            }
            return;
        };
        if let crate::world::appearance::Action::ShowOriginal(id) = action {
            ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
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
        // the groups of My Outfits are plain inventory folders
        if let crate::world::appearance::Action::Folders(change) = action {
            self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(change));
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
                    self.reload_outfit();
                }
            }
            return;
        }
        self.appearance_ui.saved_new = matches!(
            &action,
            crate::world::appearance::Action::Save(Some(_)) | crate::world::appearance::Action::SaveIn(..)
        );
        let attachments = crate::world::appearance::attachments_for_action(&self.world.inventory, self.world.agent_id, &action);
        match crate::world::appearance::plan(&self.world.inventory, action, &self.world.outfit_attachment_points()) {
            Ok((change, sync)) => {
                let request = uuid::Uuid::new_v4();
                self.appearance_ui.pending = Some((request, sync));
                self.appearance_ui.pending_actor = actor;
                self.appearance_ui.attachments = attachments;
                self.appearance_ui.message.clear();
                let mut command = NetCommand::UpdateOutfit { request, change };
                // An attachment the outfit write left out must not reach the
                // rez that follows: RLVa would refuse it as a whole.
                let dropped = self.world.rlva_prepare_outfit(&mut command, actor);
                self.appearance_ui.attachments.retain(|a| !dropped.contains(&a.item_id));
                if !self.world.rlva_allows_command(&command, actor) {
                    self.appearance_ui.pending = None;
                    self.appearance_ui.attachments.clear();
                }
                self.send_from(command, actor);
            }
            Err(reason) => {
                self.inventory_ui.message.clone_from(&reason);
                self.appearance_ui.message = reason;
                self.reload_outfit();
            }
        }
    }

    /// A refused outfit change asks again for the current outfit, the
    /// outfits and their missing link targets (`AppearanceUi::reload`):
    /// "still loading" lasts until they arrive, not for the session.
    fn reload_outfit(&mut self) {
        let worn = self.world.outfit_attachment_items();
        for command in self.appearance_ui.reload(&mut self.world.inventory, self.world.agent_id, &worn) {
            self.send(command);
        }
    }

    /// Ctrl+C, Ctrl+X or Ctrl+V of Appearance, on the inventory's clipboard:
    /// a paste goes through the inventory's paste, outfit rules included.
    pub(super) fn appearance_clipboard(&mut self, ctx: &egui::Context, request: ui::appearance::ClipboardRequest) {
        let facts = ui::inventory::Facts::from_world(&self.world, self.appearance_ui.pending.is_some());
        self.inventory_ui.message.clear();
        let actions = ui::inventory::appearance_clipboard(
            ctx,
            &self.world.inventory,
            &mut self.inventory_ui,
            &self.settings.inventory,
            &facts,
            request,
        );
        self.appearance_ui.message.clone_from(&self.inventory_ui.message);
        for action in actions {
            self.apply_inventory_action(action);
        }
    }
}
