//! Native transfers follow LLPreviewNotecard::loadAsset/saveIfNeeded
//! (indra/newview/llpreviewnotecard.cpp). Reuse the shared COPY-aware picker
//! and recheck rights, the asset and the current layout when a download returns.
use super::{ToolbarSettings, profile};
use crate::{
    theme::Palette,
    ui::{
        inventory::context_notecards::{self, Mode, picker::Picker},
        skin::Layout,
        widgets,
    },
    world::inventory::Inventory,
};
use aurora_net::{
    NetCommand,
    inventory::{
        InvItem,
        notecard::{MAX_TEXT_BYTES, Notecard},
    },
};
use std::time::{Duration, Instant};
use uuid::Uuid;

mod library;

struct Prepared {
    name: String,
    data: Vec<u8>,
    root: Uuid,
    request: Option<Uuid>,
    attempted: bool,
    since: Instant,
}

struct Loading {
    item: Uuid,
    asset: Uuid,
    before: ToolbarSettings,
    base: profile::Base,
}

#[derive(Default)]
pub(crate) struct State {
    mode: Option<Mode>,
    name: String,
    filter: String,
    selected: Option<Uuid>,
    picker: Picker,
    saving: Option<Uuid>,
    last_saved: Option<Uuid>,
    loading: Option<Loading>,
    message: Option<String>,
    dismissed: bool,
    prepared: Option<Prepared>,
    catalog: library::Catalog,
    selected_profile: Option<Uuid>,
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.saving.is_some() || self.loading.is_some() || self.prepared.is_some()
    }

    pub(crate) fn active(&self) -> bool {
        self.mode.is_some() || self.busy() || self.selected_profile.is_some() || self.message.is_some() && !self.dismissed
    }

    pub(crate) fn open(&mut self, mode: Mode) {
        if self.busy() {
            return;
        }
        self.mode = Some(mode);
        self.last_saved = None;
        self.name = aurora_i18n::tr!("toolbar-notecard-name").into();
        self.filter.clear();
        self.selected = None;
        self.picker = Picker::with_id("toolbar_interface_notecard_picker");
        self.message = None;
        self.dismissed = false;
        log::debug!("toolbar notecard dialog opened: {mode:?}");
    }

    fn save(
        &mut self,
        inv: &mut Inventory,
        toolbar: &ToolbarSettings,
        layout: &Layout,
        islands: bool,
    ) -> Result<Option<NetCommand>, String> {
        if self.busy() || self.name.trim().is_empty() {
            return Err(aurora_i18n::tr!("toolbar-notecard-unavailable").into());
        }
        let text = profile::write(toolbar, layout, islands)?;
        let data = Notecard::default().encode(&text)?;
        if inv.root.is_nil() {
            return Err(aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into());
        }
        self.message = None;
        self.dismissed = false;
        library::retry(inv);
        self.prepared = Some(Prepared {
            name: self.name.trim().chars().take(63).collect(),
            data,
            root: inv.root,
            request: None,
            attempted: false,
            since: Instant::now(),
        });
        self.advance(inv)
    }

    fn advance(&mut self, inv: &mut Inventory) -> Result<Option<NetCommand>, String> {
        let Some(pending) = self.prepared.as_mut() else { return Ok(None) };
        if pending.root != inv.root || pending.since.elapsed() > Duration::from_secs(60) {
            self.prepared = None;
            return Err(library::unavailable());
        }
        if pending.request.is_some() {
            return Ok(None);
        }
        let path = match library::resolve(inv) {
            Ok(path) => path,
            Err(reason) => {
                self.prepared = None;
                return Err(reason);
            }
        };
        match path {
            library::Path::Loading => Ok(None),
            library::Path::Missing { parent, brand } => {
                if pending.attempted {
                    self.prepared = None;
                    return Err(library::unavailable());
                }
                let request = Uuid::new_v4();
                pending.request = Some(request);
                pending.attempted = true;
                Ok(Some(NetCommand::EditInventory {
                    request,
                    change: library::create(parent, brand),
                }))
            }
            library::Path::Ready(parent) => {
                let Some(pending) = self.prepared.take() else { return Ok(None) };
                let request = Uuid::new_v4();
                self.saving = Some(request);
                Ok(Some(NetCommand::CreateNotecardItem {
                    request,
                    parent,
                    name: pending.name,
                    data: pending.data,
                    permissions: None,
                }))
            }
        }
    }

    pub(crate) fn folders_created(&mut self, request: Uuid, result: &Result<aurora_net::inventory::operations::Reply, String>) -> bool {
        if !self.prepared.as_ref().is_some_and(|pending| pending.request == Some(request)) {
            return false;
        }
        if let Err(reason) = result {
            self.prepared = None;
            self.message = Some(reason.clone());
            self.dismissed = false;
        } else if let Some(pending) = &mut self.prepared {
            pending.request = None;
        }
        true
    }

    pub(crate) fn sync_library(&mut self, inv: &mut Inventory) {
        self.catalog.sync(inv);
    }

    pub(super) fn presets(&mut self, ui: &mut egui::Ui, p: &Palette, locked: bool) {
        widgets::section_header(ui, p, "folder-filled", aurora_i18n::tr!("toolbar-saved-interfaces"));
        if self.catalog.loading {
            ui.label(aurora_i18n::tr!("common-loading"));
        } else if self.catalog.entries.is_empty() {
            ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-saved-interfaces-empty")).color(p.muted));
        } else {
            let enabled = !locked && !self.busy() && self.catalog.can_preview;
            egui::ScrollArea::vertical()
                .id_salt("toolbar_saved_interfaces")
                .max_height(100.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for entry in &self.catalog.entries {
                            ui.push_id(entry.id, |ui| {
                                let response = ui
                                    .add_enabled_ui(enabled && entry.available, |ui| {
                                        widgets::action_button(
                                            ui,
                                            p,
                                            "file-text-filled",
                                            &entry.name,
                                            widgets::ActionKind::Flat,
                                            egui::vec2(190.0, 30.0),
                                        )
                                    })
                                    .inner;
                                #[cfg(test)]
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(egui::Id::unique(("toolbar_saved_interface", entry.id)), response.rect)
                                });
                                if response.clicked() {
                                    self.selected_profile = Some(entry.id);
                                }
                            });
                        }
                    });
                });
        }
    }

    fn load(&mut self, inv: &Inventory, toolbar: &ToolbarSettings, islands: bool, item: Uuid) -> Option<NetCommand> {
        if self.busy() || toolbar.locked || !inv.can_preview(7) {
            return None;
        }
        let Some(item) = context_notecards::available(inv, item).cloned() else {
            self.message = Some(aurora_i18n::tr!("toolbar-notecard-unavailable").into());
            return None;
        };
        self.message = None;
        self.loading = Some(Loading {
            item: item.id,
            asset: item.asset_id,
            before: toolbar.clone(),
            base: profile::Base::from_islands(islands),
        });
        self.dismissed = false;
        Some(NetCommand::PreviewInventoryItem(item))
    }

    pub(crate) fn saved(&mut self, request: Uuid, result: &Result<InvItem, String>) -> bool {
        if self.saving != Some(request) {
            return false;
        }
        self.saving = None;
        self.last_saved = result.as_ref().ok().map(|item| item.id);
        self.dismissed = false;
        self.message = Some(
            result
                .as_ref()
                .map(|_| aurora_i18n::tr!("toolbar-notecard-saved").into())
                .unwrap_or_else(Clone::clone),
        );
        true
    }

    pub(crate) fn demo_saved_item(&self) -> Option<Uuid> {
        self.last_saved
    }

    pub(crate) fn demo_save(
        &mut self,
        inv: &mut Inventory,
        toolbar: &ToolbarSettings,
        layout: &Layout,
        islands: bool,
    ) -> Option<NetCommand> {
        self.mode = None;
        match self.save(inv, toolbar, layout, islands) {
            Ok(command) => command,
            Err(reason) => {
                self.message = Some(reason);
                None
            }
        }
    }

    pub(crate) fn demo_load(
        &mut self,
        inv: &Inventory,
        toolbar: Option<&ToolbarSettings>,
        islands: bool,
        item: Uuid,
    ) -> Option<NetCommand> {
        let default = ToolbarSettings::default();
        let command = self.load(inv, toolbar.unwrap_or(&default), islands, item);
        if command.is_some() {
            self.mode = None;
        }
        command
    }

    pub(crate) fn loading(&self, item: Uuid) -> bool {
        self.loading.as_ref().is_some_and(|pending| pending.item == item)
    }

    pub(crate) fn loaded(
        &mut self,
        inv: &Inventory,
        current: &ToolbarSettings,
        islands: bool,
        item: Uuid,
        asset: Uuid,
        result: &Result<Vec<u8>, String>,
    ) -> Option<profile::Profile> {
        if !self.loading(item) {
            return None;
        }
        let pending = self.loading.take()?;
        self.dismissed = false;
        let decoded = if current.locked || *current != pending.before || profile::Base::from_islands(islands) != pending.base {
            Err(aurora_i18n::tr!("toolbar-notecard-layout-changed").into())
        } else if !inv.can_preview(7)
            || asset != pending.asset
            || !context_notecards::available(inv, item).is_some_and(|item| item.asset_id == pending.asset)
        {
            log::debug!(
                "toolbar notecard unavailable: preview={}, asset_matches={}, item_matches={}",
                inv.can_preview(7),
                asset == pending.asset,
                context_notecards::available(inv, item).is_some_and(|item| item.asset_id == pending.asset)
            );
            Err(aurora_i18n::tr!("toolbar-notecard-unavailable").into())
        } else {
            result.as_ref().map_err(Clone::clone).and_then(|bytes| {
                if bytes.len() > MAX_TEXT_BYTES + 4096 {
                    return Err(aurora_i18n::tr!("toolbar-notecard-invalid").into());
                }
                Notecard::decode(bytes).and_then(|note| profile::read(&note.text))
            })
        };
        match decoded {
            Ok(profile) => {
                self.message = None;
                Some(profile)
            }
            Err(reason) => {
                self.message = Some(reason);
                None
            }
        }
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        inv: &mut Inventory,
        toolbar: &ToolbarSettings,
        layout: &Layout,
        islands: bool,
    ) -> Option<NetCommand> {
        let mut command = match self.advance(inv) {
            Ok(command) => command,
            Err(reason) => {
                self.message = Some(reason);
                self.dismissed = false;
                None
            }
        };
        if let Some(item) = self.selected_profile.take() {
            self.mode = None;
            command = self.load(inv, toolbar, islands, item);
        }
        if self.mode == Some(Mode::Load) {
            let (close, selected) = self.picker.show(
                ctx,
                p,
                inv,
                &mut self.filter,
                &mut self.selected,
                !toolbar.locked && inv.can_preview(7),
            );
            if let Some(item) = selected {
                command = self.load(inv, toolbar, islands, item);
            }
            if close || command.is_some() {
                log::debug!("toolbar notecard picker closed: cancel={close}, submitted={}", command.is_some());
                self.mode = None;
            }
        } else if self.mode == Some(Mode::Export) {
            let mut close = false;
            let mut save = false;
            let modal = widgets::Modal::new(egui::Id::unique("toolbar_notecard_save")).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-notecard-save")).color(p.violet_light));
                ui.label(aurora_i18n::tr!("toolbar-notecard-help"));
                ui.label(aurora_i18n::tr!("toolbar-notecard-destination", path = library::PATH));
                ui.label(aurora_i18n::tr!("common-name-2"));
                let name = ui.add(
                    egui::TextEdit::singleline(&mut self.name)
                        .char_limit(63)
                        .desired_width(ui.available_width()),
                );
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_note_name"), name.rect));
                let _ = name;
                ui.horizontal(|ui| {
                    close = ui.button(aurora_i18n::tr!("common-cancel")).clicked();
                    let button = widgets::default_button(
                        ui.add_enabled(!self.name.trim().is_empty(), egui::Button::new(aurora_i18n::tr!("common-save"))),
                    );
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_note_save"), button.rect));
                    save = button.clicked();
                });
            });
            if save {
                match self.save(inv, toolbar, layout, islands) {
                    Ok(value) => command = value,
                    Err(reason) => self.message = Some(reason),
                }
                close = true;
            }
            let dismiss = modal.should_close();
            if close || dismiss {
                log::debug!("toolbar notecard save dialog closed: action={close}, backdrop_or_escape={dismiss}");
                self.mode = None;
            }
        }
        if self.mode.is_none() && !self.dismissed && (self.message.is_some() || self.busy()) {
            let mut close = false;
            let pending = self.busy();
            let modal = widgets::Modal::new(egui::Id::unique("toolbar_notecard_result")).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(self.message.clone().unwrap_or_else(|| aurora_i18n::tr!("common-loading").into()));
                if pending {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                close = widgets::default_button(ui.button(aurora_i18n::tr!("common-close"))).clicked();
            });
            if close || modal.should_close() {
                self.message = None;
                self.dismissed = true;
            }
        }
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ui::toolbar::presets::Preset, world::inventory::actions};

    fn inventory() -> (Inventory, InvItem) {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: inv.root,
                    type_default: 8,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let change = library::create(inv.root, true);
        actions::demo_mutate(&mut inv, Uuid::from_u128(2), change).expect("interface folders");
        let item = inv.items.values().find(|item| item.asset_type == 7).expect("demo notecard").clone();
        (inv, item)
    }

    #[test]
    fn interface_native_transfer_correlates_replies_and_restores_only_valid_copyable_cards() {
        let (mut inv, item) = inventory();
        let layout = Layout::default();
        let original = Preset::OldSchool.layout(&layout);
        let mut state = State::default();
        state.open(Mode::Export);
        let Some(NetCommand::CreateNotecardItem { request, data, .. }) = state.save(&mut inv, &original, &layout, true).expect("save")
        else {
            panic!("native upload")
        };
        assert!(!state.saved(Uuid::new_v4(), &Ok(item.clone())));
        assert!(state.load(&inv, &original, true, item.id).is_none());
        assert!(state.saved(request, &Ok(item.clone())));
        state.mode = None;
        assert!(state.load(&inv, &original, true, item.id).is_some());
        assert!(
            state
                .loaded(&inv, &original, true, Uuid::new_v4(), item.asset_id, &Ok(data.clone()))
                .is_none()
        );
        assert!(state.loading(item.id));
        let restored = state
            .loaded(&inv, &original, true, item.id, item.asset_id, &Ok(data.clone()))
            .expect("valid layout");
        assert_eq!(restored.toolbar.top, original.top);
        assert_eq!(restored.base, profile::Base::Aurora);
        assert!(state.message.is_none() && !state.active(), "successful presets load silently");
        assert!(
            state
                .loaded(&inv, &original, true, item.id, item.asset_id, &Ok(data.clone()))
                .is_none()
        );
        inv.items.get_mut(&item.id).expect("card").owner_mask &= !actions::COPY;
        assert!(state.load(&inv, &original, true, item.id).is_none());
        inv.items.insert(item.id, item.clone());
        assert!(state.load(&inv, &original, true, item.id).is_some());
        inv.items.get_mut(&item.id).expect("card").owner_mask &= !actions::COPY;
        assert!(state.loaded(&inv, &original, true, item.id, item.asset_id, &Ok(data)).is_none());
        assert!(state.message.is_some());
    }

    #[test]
    fn interface_notecard_save_waits_for_inventory_and_confirmed_remapped_folders() {
        let (mut inv, _) = inventory();
        let library::Path::Ready(folder) = library::resolve(&mut inv).expect("path") else {
            panic!("folder")
        };
        let brand = inv.folders[&folder].info.parent;
        inv.remove(&[folder, brand]);
        inv.queue.clear();
        let root = inv.root;
        inv.folders.get_mut(&root).expect("root").state = crate::world::inventory::FetchState::Unknown;
        let toolbar = Preset::OldSchool.layout(&Layout::default());
        let mut state = State::default();
        state.open(Mode::Export);
        state.name = "Studio".into();
        assert!(state.save(&mut inv, &toolbar, &Layout::default(), true).expect("queued").is_none());
        assert_eq!(inv.queue, vec![(root, false)]);
        assert!(state.advance(&mut inv).expect("still fetching").is_none());
        assert_eq!(inv.queue.len(), 1);
        inv.folders.get_mut(&root).expect("root").state = crate::world::inventory::FetchState::Fetched;
        let Some(NetCommand::EditInventory { request, mut change }) = state.advance(&mut inv).expect("prepare folders") else {
            panic!("folder creation")
        };
        assert_eq!(change.operations.len(), 2);
        assert!(state.advance(&mut inv).expect("waiting for acknowledgement").is_none());
        assert!(!state.folders_created(Uuid::new_v4(), &Err("unrelated".into())));
        let mut remapped = std::collections::HashMap::new();
        for operation in &mut change.operations {
            if let aurora_net::inventory::operations::Operation::Folder { id, parent, .. } = operation {
                *parent = remapped.get(parent).copied().unwrap_or(*parent);
                let actual = Uuid::new_v4();
                remapped.insert(*id, actual);
                *id = actual;
            }
        }
        change.refresh = change.refresh.iter().map(|id| remapped.get(id).copied().unwrap_or(*id)).collect();
        let owner = inv.owner;
        let mut reply = actions::demo_mutate(&mut inv, owner, change).expect("native folder reply");
        reply.created = remapped;
        assert!(state.folders_created(request, &Ok(reply)));
        let library::Path::Ready(destination) = library::resolve(&mut inv).expect("created path") else {
            panic!("confirmed path")
        };
        let Some(NetCommand::CreateNotecardItem { parent, name, data, .. }) = state.advance(&mut inv).expect("upload") else {
            panic!("notecard creation")
        };
        assert_eq!(parent, destination);
        assert_eq!(name, "Studio");
        assert!(profile::read(&Notecard::decode(&data).expect("notecard").text).is_ok());
        assert!(state.advance(&mut inv).expect("only one upload").is_none());
    }

    #[test]
    fn interface_notecard_folder_failure_timeout_and_account_change_cancel_the_save() {
        for case in 0..3 {
            let (mut inv, _) = inventory();
            let library::Path::Ready(folder) = library::resolve(&mut inv).expect("path") else {
                panic!("folder")
            };
            inv.remove(&[folder]);
            let mut state = State::default();
            state.open(Mode::Export);
            let Some(NetCommand::EditInventory { request, change }) = state
                .save(&mut inv, &ToolbarSettings::default(), &Layout::default(), true)
                .expect("save")
            else {
                panic!("create child only")
            };
            assert_eq!(change.operations.len(), 1, "retain the existing #Aurora folder");
            if case == 0 {
                assert!(state.folders_created(request, &Err("folder rejected".into())));
                assert_eq!(state.message.as_deref(), Some("folder rejected"));
            } else {
                if case == 1 {
                    inv.root = Uuid::new_v4();
                } else {
                    state.prepared.as_mut().expect("pending").since = Instant::now() - Duration::from_secs(61);
                }
                assert!(state.advance(&mut inv).is_err());
            }
            assert!(!state.busy());
        }
    }

    #[test]
    fn interface_notecard_catalog_follows_names_moves_types_permissions_and_removal() {
        let (mut inv, mut item) = inventory();
        let library::Path::Ready(folder) = library::resolve(&mut inv).expect("path") else {
            panic!("folder")
        };
        let previous = item.parent;
        item.parent = folder;
        item.name = "Studio".into();
        inv.add_items(vec![item.clone()]);
        let mut state = State::default();
        state.sync_library(&mut inv);
        assert_eq!(state.catalog.entries.len(), 1);
        assert_eq!(state.catalog.entries[0].name, "Studio");
        item.name = "Portrait".into();
        item.owner_mask &= !actions::COPY;
        inv.add_items(vec![item.clone()]);
        state.sync_library(&mut inv);
        assert_eq!(state.catalog.entries[0].name, "Portrait");
        assert!(!state.catalog.entries[0].available);
        inv.rlva_preview_mask = 2;
        state.sync_library(&mut inv);
        assert!(!state.catalog.can_preview);
        inv.rlva_preview_mask = 0;
        item.parent = previous;
        inv.add_items(vec![item.clone()]);
        state.sync_library(&mut inv);
        assert!(state.catalog.entries.is_empty());
        item.parent = folder;
        item.asset_type = 0;
        inv.add_items(vec![item.clone()]);
        state.sync_library(&mut inv);
        assert!(state.catalog.entries.is_empty());
        item.asset_type = 7;
        inv.add_items(vec![item.clone()]);
        state.sync_library(&mut inv);
        assert_eq!(state.catalog.entries.len(), 1);
        inv.remove(&[item.id]);
        state.sync_library(&mut inv);
        assert!(state.catalog.entries.is_empty());
    }

    #[test]
    fn interface_notecard_saved_preset_click_loads_the_card_and_honors_locks() {
        for locked in [false, true] {
            let (mut inv, mut item) = inventory();
            let library::Path::Ready(folder) = library::resolve(&mut inv).expect("path") else {
                panic!("folder")
            };
            item.parent = folder;
            item.name = "Studio".into();
            inv.add_items(vec![item.clone()]);
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let p = crate::theme::Theme::default().palette();
            let mut state = State::default();
            state.sync_library(&mut inv);
            let toolbar = ToolbarSettings {
                locked,
                ..Default::default()
            };
            let mut frame = |events| {
                let mut command = None;
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 720.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        state.presets(ui, &p, locked);
                        command = state.show(ui.ctx(), &p, &mut inv, &toolbar, &Layout::default(), true);
                    },
                );
                command
            };
            for _ in 0..4 {
                assert!(frame(Vec::new()).is_none());
            }
            let point = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("toolbar_saved_interface", item.id))))
                .expect("saved preset")
                .center();
            assert!(frame(click_events(point, true)).is_none());
            let command = frame(click_events(point, false));
            assert_eq!(
                matches!(command,Some(NetCommand::PreviewInventoryItem(chosen)) if chosen.id == item.id),
                !locked
            );
        }
    }

    #[test]
    fn interface_load_preserves_current_layout_on_changes_permissions_errors_or_invalid_data() {
        let (mut inv, item) = inventory();
        let layout = Layout::default();
        let current = Preset::Light.layout(&layout);
        let text = profile::write(&Preset::Photographer.layout(&layout), &layout, true).expect("profile");
        let data = Notecard::default().encode(&text).expect("notecard");
        for case in 0..7 {
            let mut state = State::default();
            assert!(state.load(&inv, &current, true, item.id).is_some());
            let mut now = current.clone();
            let mut asset = item.asset_id;
            let mut result = Ok(data.clone());
            let islands = case != 6;
            match case {
                0 => now.locked = true,
                1 => now.top.reverse(),
                2 => inv.rlva_preview_mask = 2,
                3 => asset = Uuid::new_v4(),
                4 => result = Ok(b"{}".to_vec()),
                5 => result = Err("download failed".into()),
                _ => {}
            }
            assert!(state.loaded(&inv, &now, islands, item.id, asset, &result).is_none());
            assert!(!state.loading(item.id));
            assert!(state.message.is_some());
            inv.rlva_preview_mask = 0;
        }
    }
    fn ui_frame(
        ctx: &egui::Context,
        state: &mut State,
        inv: &mut Inventory,
        toolbar: &ToolbarSettings,
        events: Vec<egui::Event>,
    ) -> Option<NetCommand> {
        let p = crate::theme::Theme::default().palette();
        let mut command = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 720.0))),
                time: Some(ctx.cumulative_frame_nr() as f64 * 0.1),
                events,
                ..Default::default()
            },
            |ui| {
                let mut editor = true;
                widgets::Floater::new(
                    "toolbar_editor",
                    aurora_i18n::tr!("toolbar-title"),
                    egui::pos2(150.0, 60.0),
                    egui::vec2(700.0, 550.0),
                )
                .show(ui.ctx(), &p, &mut editor, |ui| {
                    ui.label(aurora_i18n::tr!("toolbar-presets"));
                });
                if let Some(value) = state.show(ui.ctx(), &p, inv, toolbar, &Layout::default(), true) {
                    command = Some(value);
                }
            },
        );
        command
    }

    fn click_events(point: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    }

    #[test]
    fn interface_dialog_edits_the_name_and_picker_waits_for_the_load_button() {
        let (mut inv, item) = inventory();
        let toolbar = Preset::OldSchool.layout(&Layout::default());
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut state = State::default();
        state.open(Mode::Export);
        for _ in 0..4 {
            assert!(ui_frame(&ctx, &mut state, &mut inv, &toolbar, Vec::new()).is_none());
        }
        let name = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("toolbar_note_name")))
            .expect("name field")
            .center();
        for pressed in [true, false] {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(name, pressed));
        }
        ui_frame(&ctx, &mut state, &mut inv, &toolbar, vec![egui::Event::Text(" portable".into())]);
        assert!(state.name.contains("portable"));
        let expected = state.name.trim().to_string();
        let save = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("toolbar_note_save")))
            .expect("save button")
            .center();
        ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(save, true));
        let command = ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(save, false)).expect("save command");
        let NetCommand::CreateNotecardItem { request, name, data, .. } = command else {
            panic!("native save")
        };
        assert_eq!(name, expected);
        assert!(profile::read(&Notecard::decode(&data).expect("native card").text).is_ok());
        assert!(state.saved(request, &Ok(item.clone())));
        state.open(Mode::Load);
        state.filter = item.name.clone();
        for _ in 0..4 {
            assert!(ui_frame(&ctx, &mut state, &mut inv, &toolbar, Vec::new()).is_none());
        }
        let row = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("menu_note_row", item.id))))
            .expect("notecard row")
            .center();
        assert_eq!(
            ctx.layer_id_at(row).map(|layer| layer.id),
            Some(egui::Id::unique("toolbar_interface_notecard_picker")),
            "the inventory chooser opens above the interface editor"
        );
        for pressed in [true, false] {
            assert!(ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(row, pressed)).is_none());
        }
        assert_eq!(state.selected, Some(item.id));
        let load = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("menu_note_reload")))
            .expect("load button")
            .center();
        ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(load, true));
        assert!(
            matches!(ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(load, false)), Some(NetCommand::PreviewInventoryItem(selected)) if selected.id == item.id)
        );
    }
}
