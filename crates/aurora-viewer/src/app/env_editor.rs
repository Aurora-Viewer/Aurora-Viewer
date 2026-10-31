//! The sky and water editors' work outside the window: opening on a
//! snapshot or an item, loading its asset, saving to the inventory and
//! applying (LLFloaterFixedEnvironment and LLFloaterEditEnvironmentBase,
//! newview/llfloaterfixedenvironment.cpp, llfloatereditenvironmentbase.cpp,
//! originally LGPL 2.1).

use super::App;
use crate::ui::env_editor::{self, EditorAction};
use crate::world::env_edit::{self, Rights, SettingsDoc};
use crate::world::env_select::SettingsKind;
use aurora_net::NetCommand;
use uuid::Uuid;

impl App {
    /// "new" editors start from what is shown (onOpen without an item).
    pub(super) fn open_env_editor_snapshot(&mut self, kind: SettingsKind) {
        let (sky, water) = self.world.eep.current();
        let doc = if kind == SettingsKind::Water {
            SettingsDoc::snapshot_water(&water, aurora_i18n::tr!("ui-env-editor-snapshot-water"))
        } else {
            SettingsDoc::snapshot_sky(&sky, aurora_i18n::tr!("ui-env-editor-snapshot-sky"))
        };
        self.env_editor.start(doc, None, Rights::FREE);
        self.env_editor.message = None;
        self.env_editor_scan();
        self.show_env_edit();
    }

    /// The Load list needs the settings items: the selector's folder scan.
    fn env_editor_scan(&mut self) {
        if !self.env_scan {
            self.env_scan = true;
            self.env_ui.scan_left = usize::MAX;
        }
    }

    /// loadInventoryItem: the item's asset is fetched, then edited.
    pub(super) fn env_editor_load(&mut self, item: Uuid) {
        let Some(it) = self.world.inventory.items.get(&item).cloned() else {
            self.env_editor.message = Some((aurora_i18n::tr!("ui-env-editor-item-not-found").into(), true));
            return;
        };
        let Some(kind) = SettingsKind::from_flags(it.flags) else {
            return;
        };
        if kind == SettingsKind::Day {
            self.day_editor_load(item);
            return;
        }
        if it.asset_id.is_nil() {
            // UnableEditItem
            self.env_editor.message = Some((aurora_i18n::tr!("ui-env-editor-unable-to-edit").into(), true));
            return;
        }
        self.env_editor.start_loading(kind, item);
        self.env_editor_scan();
        self.send(NetCommand::PreviewInventoryItem(it));
    }

    /// onAssetLoaded: the item's settings, its name and permissions.
    pub(super) fn env_editor_asset(&mut self, item: Uuid, result: Result<Vec<u8>, String>) {
        self.env_editor.loading = None;
        let it = self.world.inventory.items.get(&item).cloned();
        let doc = result.ok().and_then(|data| SettingsDoc::from_asset(&data));
        let (Some(it), Some(mut doc)) = (it, doc) else {
            // FailedToFindSettings (Firestorm closes the editor; the window
            // stays to say why, the previous settings with it if any)
            let name = self.world.inventory.items.get(&item).map(|it| it.name.clone()).unwrap_or_default();
            log::warn!("environment settings of item {item} could not be loaded");
            self.env_editor.message = Some((aurora_i18n::tr!("ui-env-editor-load-failed", name = name), true));
            return;
        };
        let rights = Rights::from_item(it.owner_mask, doc.flags());
        if rights.save {
            doc.set_name(&it.name);
            rights.apply(&mut doc);
        }
        self.env_editor.start(doc, Some(item), rights);
        self.show_env_edit();
    }

    /// updateEditEnvironment: the edit slot shows the document.
    fn show_env_edit(&mut self) {
        let Some(doc) = &self.env_editor.doc else {
            return;
        };
        match env_editor::frames(doc) {
            (Some(sky), _) => self.world.eep.set_edit_sky(sky),
            (_, Some(water)) => self.world.eep.set_edit_water(water),
            _ => {}
        }
    }

    pub(super) fn draw_env_editor(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        self.env_apply_answers();
        self.env_beacons_frame();
        if let Some(item) = self.env_editor.wait_item
            && self.world.inventory.items.contains_key(&item)
        {
            self.env_editor.wait_item = None;
            self.env_editor_load(item);
        }
        if let Some(rx) = &self.env_import
            && let Ok(file) = rx.try_recv()
        {
            self.env_import = None;
            self.env_editor_imported(file);
        }
        if !self.env_editor.open {
            return;
        }
        let access = self.env_editor_access();
        self.env_editor.beacons = [self.settings.sun_beacon, self.settings.moon_beacon];
        let actions = self.env_editor.show(ctx, p, &self.world.inventory, &self.ui_images, access);
        if self.env_editor.open {
            self.set_env_beacons(self.env_editor.beacons);
        }
        if self.env_editor.show_picker(ctx, p, &mut self.world, &self.ui_images) {
            self.show_env_edit();
        }
        for action in actions {
            // onButtonApply: nothing leaves with a local texture
            let checked = matches!(
                action,
                EditorAction::Save
                    | EditorAction::SaveAsAsk
                    | EditorAction::ApplyLocal
                    | EditorAction::ApplyParcel
                    | EditorAction::ApplyRegion
            );
            if checked && !self.env_editor_settings_ok() {
                continue;
            }
            match action {
                EditorAction::Changed => self.show_env_edit(),
                EditorAction::Load(item) => self.env_editor_load(item),
                EditorAction::Import => self.env_editor_import(),
                EditorAction::Save => self.env_editor_save(None),
                EditorAction::SaveAsAsk => self.env_editor.ask_save_as(),
                EditorAction::SaveAs(name) => self.env_editor_save(Some(name)),
                EditorAction::ApplyParcel => self.env_editor_apply(false),
                EditorAction::ApplyRegion => self.env_editor_apply(true),
                EditorAction::ApplyLocal => {
                    // doApplyEnvironment(ACTION_APPLY_LOCAL): ENV_LOCAL, at once
                    // shown once the editor closes
                    if let Some(doc) = &self.env_editor.doc {
                        match env_editor::frames(doc) {
                            (Some(sky), _) => self.world.eep.apply_local_sky(sky),
                            (_, Some(water)) => self.world.eep.apply_local_water(water),
                            _ => {}
                        }
                        self.env_editor.message = Some((aurora_i18n::tr!("ui-env-editor-applied").into(), false));
                    }
                }
                EditorAction::Closed => self.world.eep.clear_edit(),
            }
        }
    }

    /// "Import": a Windlight XML file chosen on disk (doImportFromDisk).
    fn env_editor_import(&mut self) {
        if self.env_import.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.env_import = Some(rx);
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("ui-env-editor-import-tip"))
                .add_filter("XML", &["xml"])
                .pick_file()
                .and_then(|path| {
                    let name = path.file_stem()?.to_string_lossy().into_owned();
                    let data = std::fs::read(&path)
                        .map_err(|e| log::warn!("windlight import {}: {e}", path.display()))
                        .ok()?;
                    Some((data, env_edit::legacy_preset_name(&name)))
                });
            let _ = tx.send(file);
        });
    }

    /// loadSkySettingFromFile / loadWaterSettingFromFile: the converted
    /// preset is edited as new settings, unsaved.
    fn env_editor_imported(&mut self, file: Option<(Vec<u8>, String)>) {
        let Some((data, name)) = file else {
            return;
        };
        let kind = self.env_editor.kind.unwrap_or(SettingsKind::Sky);
        let doc = if kind == SettingsKind::Water {
            SettingsDoc::import_legacy_water(&data, &name)
        } else {
            SettingsDoc::import_legacy_sky(&data, &name)
        };
        let Some(doc) = doc else {
            self.env_editor.message = Some((aurora_i18n::tr!("ui-env-editor-import-failed").into(), true));
            return;
        };
        log::info!("windlight preset imported: {name}");
        self.env_editor.start(doc, None, Rights::FREE);
        self.env_editor.dirty = true;
        self.show_env_edit();
    }

    /// Save (doApplyUpdateInventory) or Save As (doApplyCreateNewInventory).
    fn env_editor_save(&mut self, new_name: Option<String>) {
        let Some(mut doc) = self.env_editor.doc.clone() else {
            return;
        };
        if self.env_editor.saving.is_some() {
            return;
        }
        let request = Uuid::new_v4();
        let item = self.env_editor.item.and_then(|id| self.world.inventory.items.get(&id).cloned());
        match (new_name, item) {
            // onSaveAsCommit without modify rights: the item is copied as it
            // is (copy_inventory_item), never re-uploaded with new permissions
            (Some(name), Some(it)) if !self.env_editor.rights.modify => {
                let inv = &self.world.inventory;
                let settings_folder = crate::world::inventory::actions::system(inv, 56).unwrap_or(inv.root);
                let parent = if crate::world::inventory::actions::library(inv, it.id) {
                    settings_folder
                } else {
                    it.parent
                };
                self.send(NetCommand::CopyInventoryItems(vec![aurora_net::inventory::operations::CopyItem {
                    item: it.id,
                    owner: it.owner,
                    parent,
                    name,
                }]));
                self.env_editor.message = Some((aurora_i18n::tr!("ui-env-editor-copied").into(), false));
            }
            (None, Some(it)) => {
                self.env_editor.saving = Some(request);
                self.send(NetCommand::SaveInventoryContent {
                    request,
                    item: it.id,
                    asset_type: 56,
                    data: doc.to_asset(),
                });
            }
            (name, it) => {
                // the edited item's folder, else the Settings folder (a library
                // or Marketplace item goes to the Settings folder)
                let inv = &self.world.inventory;
                let settings_folder = crate::world::inventory::actions::system(inv, 56).unwrap_or(inv.root);
                let own = it.as_ref().filter(|it| !crate::world::inventory::actions::library(inv, it.id));
                let parent = own.map_or(settings_folder, |it| it.parent);
                let next_owner_mask = own.map(|it| it.next_owner_mask);
                let name = name.unwrap_or_else(|| doc.name().to_owned());
                doc.set_name(&name);
                let subtype = if doc.kind() == SettingsKind::Water { 1 } else { 0 };
                self.env_editor.saving = Some(request);
                self.send(NetCommand::CreateSettingsItem {
                    request,
                    parent,
                    name,
                    subtype,
                    next_owner_mask,
                    data: doc.to_asset(),
                });
            }
        }
    }

    /// onInventoryCreated / onInventoryUpdated: the editor follows the
    /// saved item, clean.
    pub(super) fn env_editor_saved(&mut self, result: Result<aurora_net::inventory::InvItem, String>) {
        self.env_editor.saving = None;
        match result {
            Ok(it) => {
                let rights = Rights::from_item(it.owner_mask, 0);
                self.world.inventory.add_items(vec![it.clone()]);
                self.env_editor.saved(it.id, &it.name, rights);
                log::info!("environment settings saved: {} ({})", it.name, it.asset_id);
            }
            Err(e) => {
                log::warn!("environment settings not saved: {e}");
                self.env_editor.message = Some((e, true));
            }
        }
    }

    /// `AURORA_DEMO_ENV_EDITOR=item-save`: brightness raised and saved over
    /// the item (frame 900), then saved as a new item (frame 1100).
    pub(super) fn demo_env_editor_step(&mut self, frame: u64) {
        match frame {
            900 => {
                if let Some(doc) = &mut self.env_editor.doc {
                    let before = doc.sky();
                    let after = crate::world::eep::SkyFrame { gamma: 1.6, ..before };
                    doc.edit_sky(&before, &after);
                    self.env_editor.dirty = true;
                }
                self.show_env_edit();
                self.env_editor_save(None);
            }
            1100 => self.env_editor_save(Some(aurora_i18n::tr!("demo-env-my-purple-sky-copy").into())),
            _ => {}
        }
    }

    /// `AURORA_DEMO_ENV_EDITOR`.
    pub(super) fn demo_env_editor(&mut self, scenario: &str) {
        log::info!("demo environment editor: {scenario}");
        match scenario {
            "water" => self.open_env_editor_snapshot(SettingsKind::Water),
            "item" | "item-save" => {
                let item = crate::demo::env::editor_item();
                let folder = Uuid::from_u128(item.as_u128() & !0xFFFF | crate::demo::env::USER_SETTINGS);
                self.world.inventory.request(folder);
                self.env_editor.wait_item = Some(item);
            }
            other => {
                self.open_env_editor_snapshot(SettingsKind::Sky);
                self.env_editor.set_tab(match other {
                    "clouds" | "clouds-picker" => 1,
                    "sun" | "beacons" => 2,
                    _ => 0,
                });
                if other == "clouds-picker" {
                    self.env_editor.demo_cloud_picker();
                }
                // beacons: both beacons shown (off again once the editor closes)
                if other == "beacons" {
                    self.env_beacons_frame();
                    self.set_env_beacons([true, true]);
                }
            }
        }
    }
}
