//! Native transfers follow LLPreviewNotecard::loadAsset/saveIfNeeded
//! (indra/newview/llpreviewnotecard.cpp). Reuse the shared COPY-aware picker
//! and recheck rights, the asset and the current layout when a download returns.
use super::{
    ToolbarSettings, profile,
    selection::{Selection, Source},
};
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
mod removal;
mod save_picker;

struct Prepared {
    name: String,
    data: Vec<u8>,
    snapshot: profile::Profile,
    target: Option<(Uuid, Uuid)>,
    epoch: u64,
    owner: Uuid,
    root: Uuid,
    request: Option<Uuid>,
    attempted: bool,
    since: Instant,
}

struct Saving {
    request: Uuid,
    item: Option<Uuid>,
    snapshot: profile::Profile,
    epoch: u64,
    owner: Uuid,
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
    saving: Option<Saving>,
    completed_selection: Option<Selection>,
    selection_epoch: u64,
    last_saved: Option<Uuid>,
    loading: Option<Loading>,
    message: Option<String>,
    dismissed: bool,
    prepared: Option<Prepared>,
    catalog: library::Catalog,
    selected_profile: Option<Uuid>,
    removal: removal::State,
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.saving.is_some() || self.loading.is_some() || self.prepared.is_some() || self.removal.active()
    }

    pub(crate) fn active(&self) -> bool {
        self.mode.is_some()
            || self.busy()
            || self.removal.active()
            || self.selected_profile.is_some()
            || self.message.is_some() && !self.dismissed
    }

    /// The personal interfaces that can be loaded, for the Interfaces block.
    pub(super) fn interfaces(&self) -> impl Iterator<Item = (Uuid, &str)> {
        self.catalog
            .entries
            .iter()
            .filter(|entry| entry.available)
            .map(|entry| (entry.id, entry.name.as_str()))
    }

    /// Loads the personal interface `item`, as a click on it in the editor.
    pub(super) fn request_load(&mut self, item: Uuid) {
        if !self.busy() {
            self.selected_profile = Some(item);
        }
    }

    pub(super) fn can_remove(&self, source: &Source) -> bool {
        source
            .personal(self.catalog.owner)
            .is_some_and(|id| self.catalog.entries.iter().any(|entry| entry.id == id && entry.removable))
    }

    pub(crate) fn remove(&mut self, source: &Source) {
        if !self.busy()
            && self.can_remove(source)
            && let Some(item) = source.personal(self.catalog.owner)
        {
            self.message = None;
            self.dismissed = false;
            self.removal.queue(self.catalog.owner, item);
        }
    }

    pub(crate) fn removal_command(
        &mut self,
        inv: &mut Inventory,
        protected: &std::collections::HashSet<Uuid>,
        confirm: bool,
    ) -> Option<NetCommand> {
        match self.removal.command(inv, protected, self.selection_epoch, confirm) {
            Ok(command) => command,
            Err(reason) => {
                self.message = Some(reason);
                self.dismissed = false;
                None
            }
        }
    }

    pub(crate) fn removed(&mut self, request: Uuid, result: &Result<aurora_net::inventory::operations::Reply, String>) -> bool {
        if !self.removal.finished(request, result, self.selection_epoch) {
            return false;
        }
        self.message = result.as_ref().err().cloned();
        self.dismissed = false;
        true
    }

    pub(crate) fn take_removed(&mut self) -> Option<(Uuid, Uuid)> {
        self.removal.take_removed()
    }

    pub(crate) fn demo_remove(&mut self, source: &Source) {
        self.remove(source);
        self.removal.approve();
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

    pub(crate) fn selection_changed(&mut self) {
        self.selection_epoch = self.selection_epoch.wrapping_add(1);
        self.loading = None;
        self.completed_selection = None;
    }

    pub(crate) fn take_selection(&mut self) -> Option<Selection> {
        self.completed_selection.take()
    }

    pub(crate) fn open_save(&mut self, source: Option<&Source>) {
        if self.busy() {
            return;
        }
        self.open(Mode::Export);
        if let Some(source) = source {
            self.name = source.label().chars().take(63).collect();
            self.selected = source.personal(self.catalog.owner);
            if let Some(entry) = self.catalog.entries.iter().find(|entry| Some(entry.id) == self.selected) {
                self.name.clone_from(&entry.name);
            }
        }
    }

    pub(super) fn preset_label<'a>(&'a self, source: &'a Source) -> &'a str {
        self.catalog
            .entries
            .iter()
            .find(|entry| source.personal(self.catalog.owner) == Some(entry.id))
            .map_or(source.label(), |entry| &entry.name)
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
        library::retry(inv);
        self.catalog.sync(inv);
        let target = if self.catalog.loading && self.selected.is_none() {
            if self.name.chars().count() > 63 || self.name.chars().any(char::is_control) {
                return Err(aurora_i18n::tr!("toolbar-preset-save-unavailable").into());
            }
            None
        } else {
            self.catalog.target(&self.name, self.selected)?.map(|entry| (entry.id, entry.asset))
        };
        let text = profile::write(toolbar, layout, islands)?;
        let snapshot = profile::read(&text)?;
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
            snapshot,
            target,
            epoch: self.selection_epoch,
            owner: inv.owner,
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
                let available = if let Some((item, asset)) = pending.target {
                    library::replaceable(inv, item, parent, asset)
                } else {
                    !inv.folders
                        .get(&parent)
                        .into_iter()
                        .flat_map(|folder| &folder.items)
                        .filter_map(|id| inv.items.get(id))
                        .any(|item| item.parent == parent && item.asset_type == 7 && item.name == pending.name)
                };
                if !available {
                    self.prepared = None;
                    return Err(aurora_i18n::tr!("toolbar-preset-save-unavailable").into());
                }
                let Some(pending) = self.prepared.take() else { return Ok(None) };
                let request = Uuid::new_v4();
                self.saving = Some(Saving {
                    request,
                    item: pending.target.map(|(id, _)| id),
                    snapshot: pending.snapshot,
                    epoch: pending.epoch,
                    owner: pending.owner,
                });
                Ok(Some(if let Some((item, _)) = pending.target {
                    NetCommand::SaveInventoryContent {
                        request,
                        item,
                        asset_type: 7,
                        lua: false,
                        data: pending.data,
                    }
                } else {
                    NetCommand::CreateNotecardItem {
                        request,
                        parent,
                        name: pending.name,
                        data: pending.data,
                        permissions: None,
                    }
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

    /// The personal interfaces of the editor, each with its eye
    /// (`editor::interface_eye`): the eye a click opened or closed, if any.
    pub(super) fn presets(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        locked: bool,
        active: Option<&Source>,
        hidden: &std::collections::BTreeSet<String>,
    ) -> Option<(String, bool)> {
        let mut toggled = None;
        widgets::section_header(ui, p, "folder-filled", aurora_i18n::tr!("toolbar-saved-interfaces"));
        if self.catalog.loading {
            ui.label(aurora_i18n::tr!("common-loading"));
        } else if self.catalog.entries.is_empty() {
            ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-saved-interfaces-empty")).color(p.muted));
        } else {
            // the interface lock never stops a change of interface (human
            // request, 2026-11-26): only the eyes keep to it
            let enabled = !self.busy() && self.catalog.can_preview;
            // no scroll area of their own: the editor's interface section
            // scrolls, and one inside it hid the rows past its 100 points
            // (human report, 2026-11-26). Each place is taken in the row
            // first, which wraps it as the presets' row does: a group per
            // button (`push_id`, `add_enabled_ui`) is placed without wrapping,
            // and widened the editor past its window.
            ui.horizontal_wrapped(|ui| {
                for entry in &self.catalog.entries {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(190.0, 30.0), egui::Sense::hover());
                    let current = active.and_then(|source| source.personal(self.catalog.owner)) == Some(entry.id);
                    let mut button = ui.new_child(egui::UiBuilder::new().id_salt(entry.id).max_rect(rect));
                    if !(enabled && entry.available) {
                        button.disable();
                    }
                    let response = widgets::action_button_trailing(
                        &mut button,
                        p,
                        "file-text-filled",
                        &entry.name,
                        if current {
                            widgets::ActionKind::Primary
                        } else {
                            widgets::ActionKind::Flat
                        },
                        rect.size(),
                        super::editor::EYE_ROOM,
                    );
                    let key = entry.id.to_string();
                    let shown = !hidden.contains(&key);
                    let mut eye = ui.new_child(egui::UiBuilder::new().id_salt(("eye", entry.id)).max_rect(rect));
                    if locked {
                        eye.disable();
                    }
                    if let Some(shown) = super::editor::interface_eye(&mut eye, p, response.rect, &key, shown, current) {
                        toggled = Some((key.clone(), shown));
                    }
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("toolbar_saved_interface", entry.id)), response.rect));
                    if response.clicked() {
                        self.selected_profile = Some(entry.id);
                    }
                }
            });
        }
        toggled
    }

    /// Personal interfaces named `names`, without an inventory.
    #[cfg(test)]
    pub(super) fn test_interfaces(&mut self, names: &[&str]) -> Vec<Uuid> {
        self.catalog.can_preview = true;
        self.catalog.entries = names
            .iter()
            .map(|name| library::Entry {
                id: Uuid::new_v4(),
                name: (*name).into(),
                available: true,
                writable: true,
                removable: true,
                asset: Uuid::new_v4(),
            })
            .collect();
        self.catalog.entries.iter().map(|entry| entry.id).collect()
    }

    fn load(&mut self, inv: &Inventory, toolbar: &ToolbarSettings, islands: bool, item: Uuid) -> Option<NetCommand> {
        if self.busy() || !inv.can_preview(7) {
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
        if !self.saving.as_ref().is_some_and(|saving| saving.request == request) {
            return false;
        }
        let Some(saving) = self.saving.take() else { return false };
        self.dismissed = false;
        match result {
            Ok(item) if item.asset_type == 7 && saving.item.is_none_or(|expected| item.id == expected) => {
                self.last_saved = Some(item.id);
                if saving.epoch == self.selection_epoch {
                    self.completed_selection = Some(Selection {
                        source: Source::Personal {
                            item: item.id,
                            name: item.name.clone(),
                            owner: saving.owner,
                        },
                        baseline: saving.snapshot,
                    });
                }
                self.message = None;
            }
            Ok(_) => self.message = Some(aurora_i18n::tr!("toolbar-preset-save-unavailable").into()),
            Err(reason) => self.message = Some(reason.clone()),
        }
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
        let decoded = if *current != pending.before || profile::Base::from_islands(islands) != pending.base {
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
                self.selection_changed();
                if let Some(item) = inv.items.get(&item) {
                    self.completed_selection = Some(Selection {
                        source: Source::Personal {
                            item: item.id,
                            name: item.name.clone(),
                            owner: inv.owner,
                        },
                        baseline: profile.clone(),
                    });
                }
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
        self.catalog.sync(inv);
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
            let (close, selected) = self
                .picker
                .show(ctx, p, inv, &mut self.filter, &mut self.selected, inv.can_preview(7));
            if let Some(item) = selected {
                command = self.load(inv, toolbar, islands, item);
            }
            if close || command.is_some() {
                log::debug!("toolbar notecard picker closed: cancel={close}, submitted={}", command.is_some());
                self.mode = None;
            }
        } else if self.mode == Some(Mode::Export) {
            let (mut close, save) = save_picker::show(ctx, p, &self.catalog, &mut self.name, &mut self.selected);
            if save {
                match self.save(inv, toolbar, layout, islands) {
                    Ok(value) => command = value,
                    Err(reason) => self.message = Some(reason),
                }
                close = true;
            }
            if close {
                self.mode = None;
            }
        }
        self.removal.show(ctx, p);
        if self.mode.is_none() && !self.dismissed && !self.removal.confirming() && (self.message.is_some() || self.busy()) {
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
    fn interface_notecard_saved_preset_click_loads_the_card_even_locked() {
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
                        state.presets(ui, &p, locked, None, &Default::default());
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
            // the interface lock never stops a change of interface (human
            // request, 2026-11-26)
            assert!(
                matches!(command,Some(NetCommand::PreviewInventoryItem(chosen)) if chosen.id == item.id),
                "locked={locked}"
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
                time: Some(ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) as f64 * 0.1),
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
    fn personal_card(inv: &mut Inventory, mut item: InvItem) -> InvItem {
        let library::Path::Ready(parent) = library::resolve(inv).expect("ready path") else {
            panic!("ready path")
        };
        item.parent = parent;
        item.name = "My tools".into();
        inv.add_items(vec![item.clone()]);
        item
    }

    #[test]
    fn toolbar_personal_save_selector_replaces_the_selected_item_without_a_duplicate() {
        let (mut inv, item) = inventory();
        let item = personal_card(&mut inv, item);
        let toolbar = Preset::OldSchool.layout(&Layout::default());
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut state = State::default();
        state.open_save(None);
        for _ in 0..4 {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, Vec::new());
        }
        let row = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("toolbar_save_preset", item.id))))
            .expect("personal row")
            .center();
        for pressed in [true, false] {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(row, pressed));
        }
        assert_eq!(state.selected, Some(item.id));
        assert_eq!(state.name, item.name);
        let save = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("toolbar_note_save")))
            .expect("save button")
            .center();
        ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(save, true));
        let command = ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(save, false)).expect("save command");
        let NetCommand::SaveInventoryContent {
            request,
            item: target,
            asset_type: 7,
            data,
            ..
        } = command
        else {
            panic!("update existing card")
        };
        assert_eq!(target, item.id);
        let baseline = profile::read(&Notecard::decode(&data).expect("notecard").text).expect("profile");
        assert!(state.take_selection().is_none(), "a request is not a confirmed save");
        assert!(!state.saved(Uuid::new_v4(), &Ok(item.clone())));
        assert!(state.saved(request, &Ok(item.clone())));
        let selected = state.take_selection().expect("confirmed selection");
        assert_eq!(selected.source.personal(inv.owner), Some(item.id));
        assert_eq!(selected.baseline, baseline);
        assert!(state.message.is_none());
    }

    #[test]
    fn toolbar_personal_save_failure_wrong_item_and_preset_switch_do_not_mark_it_saved() {
        for outcome in 0..3 {
            let (mut inv, item) = inventory();
            let item = personal_card(&mut inv, item);
            let mut state = State::default();
            state.open_save(None);
            state.name = item.name.clone();
            let Some(NetCommand::SaveInventoryContent { request, .. }) = state
                .save(&mut inv, &ToolbarSettings::default(), &Layout::default(), true)
                .expect("save")
            else {
                panic!("update")
            };
            let mut result = Ok(item.clone());
            if outcome == 0 {
                result = Err("upload failed".into());
            }
            if outcome == 1 {
                result.as_mut().expect("item").id = Uuid::new_v4();
            }
            if outcome == 2 {
                state.selection_changed();
            }
            assert!(state.saved(request, &result));
            assert!(state.take_selection().is_none());
        }
    }

    #[test]
    fn toolbar_personal_save_rechecks_rights_and_asset_before_upload() {
        let (mut inv, item) = inventory();
        let item = personal_card(&mut inv, item);
        let mut state = State::default();
        state.catalog.sync(&mut inv);
        state.name = item.name.clone();
        state.selected = Some(item.id);
        let library::Path::Ready(folder) = library::resolve(&mut inv).expect("path") else {
            panic!("folder")
        };
        for missing in [actions::COPY, actions::MODIFY] {
            inv.items.get_mut(&item.id).expect("item").owner_mask = item.owner_mask & !missing;
            assert!(!library::replaceable(&inv, item.id, folder, item.asset_id));
            assert!(state.save(&mut inv, &ToolbarSettings::default(), &Layout::default(), true).is_err());
            assert!(!state.busy());
        }
        inv.items.insert(item.id, item.clone());
        assert!(!library::replaceable(&inv, item.id, folder, Uuid::new_v4()));
        assert!(library::replaceable(&inv, item.id, folder, item.asset_id));
        assert!(state.catalog.target("New tools", None).expect("new name").is_none());
        assert_eq!(
            state.catalog.target(&item.name, None).expect("same name").expect("target").id,
            item.id
        );
    }
    #[test]
    fn toolbar_preset_switch_cancels_an_older_download_even_for_identical_layouts() {
        let (inv, item) = inventory();
        let toolbar = Preset::OldSchool.layout(&Layout::default());
        let data = Notecard::default()
            .encode(&profile::write(&toolbar, &Layout::default(), true).expect("profile"))
            .expect("card");
        let mut state = State::default();
        assert!(state.load(&inv, &toolbar, true, item.id).is_some());
        state.selection_changed();
        assert!(!state.loading(item.id));
        assert!(state.loaded(&inv, &toolbar, true, item.id, item.asset_id, &Ok(data)).is_none());
        assert!(state.take_selection().is_none());
    }

    #[test]
    fn toolbar_duplicate_personal_names_require_an_explicit_target() {
        let (mut inv, item) = inventory();
        let item = personal_card(&mut inv, item);
        let mut duplicate = item.clone();
        duplicate.id = Uuid::new_v4();
        inv.add_items(vec![duplicate.clone()]);
        let mut state = State::default();
        state.sync_library(&mut inv);
        assert!(state.catalog.target(&item.name, None).is_err());
        for id in [item.id, duplicate.id] {
            assert_eq!(
                state
                    .catalog
                    .target(&item.name, Some(id))
                    .expect("explicit target")
                    .expect("existing")
                    .id,
                id
            );
        }
    }
    fn removable_interface() -> (Inventory, InvItem, Source, State) {
        let (mut inv, item) = inventory();
        let item = personal_card(&mut inv, item);
        let source = Source::Personal {
            item: item.id,
            name: item.name.clone(),
            owner: inv.owner,
        };
        let mut state = State::default();
        state.sync_library(&mut inv);
        (inv, item, source, state)
    }

    #[test]
    fn toolbar_personal_interface_removal_moves_to_trash_after_its_matching_reply() {
        let (mut inv, item, source, mut state) = removable_interface();
        state.remove(&source);
        let NetCommand::EditInventory { request, change } =
            state.removal_command(&mut inv, &Default::default(), false).expect("trash mutation")
        else {
            panic!("inventory mutation")
        };
        assert!(
            change
                .operations
                .iter()
                .all(|operation| !matches!(operation, aurora_net::inventory::operations::Operation::Delete { .. })),
            "never purge a personal interface"
        );
        assert!(state.busy());
        assert!(state.take_removed().is_none());
        let result = actions::demo_mutate(&mut inv, item.owner, change);
        assert!(!state.removed(Uuid::new_v4(), &result));
        assert!(state.removed(request, &result));
        assert_eq!(state.take_removed(), Some((inv.owner, item.id)));
        assert!(actions::in_type(&inv, item.id, 14));
        state.sync_library(&mut inv);
        assert!(!state.catalog.entries.iter().any(|entry| entry.id == item.id));
    }

    #[test]
    fn toolbar_interface_removal_honors_confirmation_cancel_and_rechecks_protection() {
        let (mut inv, item, source, mut state) = removable_interface();
        let toolbar = Preset::OldSchool.layout(&Layout::default());
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        state.remove(&source);
        assert!(state.removal_command(&mut inv, &Default::default(), true).is_none());
        for _ in 0..4 {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, Vec::new());
        }
        let cancel = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("toolbar_interface_delete_cancel")))
            .expect("confirmation cancel")
            .center();
        for pressed in [true, false] {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(cancel, pressed));
        }
        assert!(!state.removal.active());
        assert_eq!(inv.items[&item.id].parent, item.parent);
        state.remove(&source);
        assert!(state.removal_command(&mut inv, &Default::default(), true).is_none());
        for _ in 0..4 {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, Vec::new());
        }
        let confirm = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("toolbar_interface_delete_confirm")))
            .expect("confirmation action")
            .center();
        for pressed in [true, false] {
            ui_frame(&ctx, &mut state, &mut inv, &toolbar, click_events(confirm, pressed));
        }
        assert!(
            state
                .removal_command(&mut inv, &std::collections::HashSet::from([item.id]), true)
                .is_none()
        );
        assert!(state.message.is_some() && !state.removal.active());
        assert_eq!(inv.items[&item.id].parent, item.parent);
    }

    #[test]
    fn toolbar_interface_removal_failures_and_later_selection_do_not_clear_identity() {
        for failed in [true, false] {
            let (mut inv, item, source, mut state) = removable_interface();
            state.remove(&source);
            let NetCommand::EditInventory { request, change } =
                state.removal_command(&mut inv, &Default::default(), false).expect("trash mutation")
            else {
                panic!("mutation")
            };
            let result = if failed {
                Err("refused".into())
            } else {
                state.selection_changed();
                actions::demo_mutate(&mut inv, item.owner, change)
            };
            assert!(state.removed(request, &result));
            assert!(state.take_removed().is_none());
            assert_eq!(state.message.is_some(), failed);
        }
    }
}
