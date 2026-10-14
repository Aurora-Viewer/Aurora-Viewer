//! The day cycle editor's work outside the window: loading the day item,
//! the skies, waters and days it loads frames and tracks from, saving and
//! applying (LLFloaterEditExtDayCycle, newview/llfloatereditextdaycycle.cpp,
//! and LLFloaterEditEnvironmentBase, originally LGPL 2.1).

use super::App;
use crate::ui::env_day_editor::{DayAction, DayContext, NoTransLoad};
use crate::world::env_day_edit::DayDoc;
use crate::world::env_edit::FLAG_NOTRANS;
use crate::world::env_edit::{Rights, SettingsDoc};
use aurora_net::NetCommand;
use uuid::Uuid;

/// What a fetched settings item is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DayFetch {
    /// "Load [frame]": a sky or water for the time cursor.
    Frame,
    /// "Load Track From": a day whose track is copied.
    Track,
}

impl App {
    /// loadInventoryItem for a day cycle item (edit_context "inventory").
    pub(super) fn day_editor_load(&mut self, item: Uuid) {
        let Some(it) = self.world.inventory.items.get(&item).cloned() else {
            return;
        };
        if it.asset_id.is_nil() {
            self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-unable-to-edit").into(), true));
            return;
        }
        // LLSettingsBridgeAction opens the inventory context
        self.day_editor.context = DayContext::Inventory;
        self.day_editor.committed = None;
        self.day_editor.timing = (-1, -1);
        self.day_editor.day_length = None;
        self.day_editor.altitudes = None;
        self.day_editor.wait_default = false;
        self.day_editor.start_loading(item);
        if !self.env_scan {
            self.env_scan = true;
            self.env_ui.scan_left = usize::MAX;
        }
        self.send(NetCommand::PreviewInventoryItem(it));
    }

    /// An inventory asset arrived for the day editor; false when it is not
    /// one it waits for.
    pub(super) fn day_editor_asset(&mut self, item: Uuid, result: &Result<Vec<u8>, String>) -> bool {
        if self.day_editor.loading == Some(item) {
            self.day_editor.loading = None;
            let it = self.world.inventory.items.get(&item).cloned();
            let doc = result.as_ref().ok().and_then(|d| DayDoc::from_asset(d));
            let (Some(it), Some(mut doc)) = (it, doc) else {
                let name = self.world.inventory.items.get(&item).map(|it| it.name.clone()).unwrap_or_default();
                log::warn!("day cycle of item {item} could not be loaded");
                self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-load-failed", name = name), true));
                return true;
            };
            let rights = Rights::from_item(it.owner_mask, doc.flags());
            if rights.save {
                doc.set_name(&it.name);
                doc.set_flags(rights.flags(doc.flags()));
            }
            self.day_editor.start(doc, Some(item), rights);
            self.show_day_edit();
            return true;
        }
        let Some((wanted, purpose)) = self.day_fetch else {
            return false;
        };
        if wanted != item {
            return false;
        }
        self.day_fetch = None;
        let mut out = Vec::new();
        // onAssetLoadedForInsertion: settings that cannot be transferred or
        // copied make the edited day no-transfer, after SettingsMakeNoTrans
        let restricted = self
            .world
            .inventory
            .items
            .get(&item)
            .is_some_and(|it| it.owner_mask & 0x2000 == 0 || it.owner_mask & 0x8000 == 0);
        let edited_no_trans = match self.day_editor.item.and_then(|id| self.world.inventory.items.get(&id)) {
            Some(it) => it.owner_mask & 0x2000 == 0,
            None => self.day_editor.doc.as_ref().is_some_and(|d| d.flags() & FLAG_NOTRANS != 0),
        };
        let make_no_trans = restricted && !edited_no_trans;
        // the question's "ignore": its default answer, Yes, without asking
        let ask = make_no_trans && self.settings.confirm_no_transfer_settings;
        match (purpose, result) {
            (DayFetch::Frame, Ok(data)) => match SettingsDoc::from_asset(data) {
                Some(frame) if ask => self.day_editor.no_trans_ask = Some(NoTransLoad::Frame(frame)),
                Some(frame) if make_no_trans => self.day_editor.accept_no_trans(NoTransLoad::Frame(frame), &mut out),
                Some(frame) => self.day_editor.put_loaded_frame(&frame, &mut out),
                None => self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-load-failed", name = ""), true)),
            },
            (DayFetch::Track, Ok(data)) => match DayDoc::from_asset(data) {
                Some(day) if ask => self.day_editor.no_trans_ask = Some(NoTransLoad::Track(day)),
                Some(day) if make_no_trans => self.day_editor.accept_no_trans(NoTransLoad::Track(day), &mut out),
                Some(day) => self.day_editor.track_source = Some(day),
                None => self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-load-failed", name = ""), true)),
            },
            (_, Err(e)) => self.day_editor.message = Some((e.clone(), true)),
        }
        if !out.is_empty() {
            self.show_day_edit();
        }
        true
    }

    /// The edit slot shows the day at the editor's time cursor.
    pub(super) fn show_day_edit(&mut self) {
        if let Some((sky, water)) = self.day_editor.shown() {
            self.world.eep.set_edit(sky, water);
        }
    }

    pub(super) fn draw_day_editor(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if let Some(item) = self.day_editor.wait_item
            && self.world.inventory.items.contains_key(&item)
        {
            self.day_editor.wait_item = None;
            self.day_editor_load(item);
        }
        self.day_editor_imported();
        if !self.day_editor.open {
            return;
        }
        self.day_editor_default();
        self.day_editor_follow_parcel();
        if !self.day_editor.open {
            return;
        }
        self.day_editor.extended = self.extended_environment();
        let access = self.env_editor_access();
        self.day_editor.beacons = [self.settings.sun_beacon, self.settings.moon_beacon];
        let actions = self.day_editor.show(ctx, p, &self.world.inventory, &self.ui_images, access);
        if self.day_editor.open {
            self.set_env_beacons(self.day_editor.beacons);
        }
        if std::mem::take(&mut self.day_editor.stop_asking_no_trans) {
            self.settings.confirm_no_transfer_settings = false;
            self.settings.save();
        }
        if self.day_editor.show_picker(ctx, p, &mut self.world, &self.ui_images) {
            self.show_day_edit();
        }
        for action in actions {
            // onButtonApply: nothing leaves with a local texture
            let checked = matches!(
                action,
                DayAction::Save
                    | DayAction::SaveAsAsk
                    | DayAction::ApplyLocal
                    | DayAction::ApplyParcel
                    | DayAction::ApplyRegion
                    | DayAction::Commit
            );
            if checked && !self.day_editor_settings_ok() {
                continue;
            }
            match action {
                DayAction::Import => self.day_editor_import(),
                DayAction::SaveAsAsk => self.day_editor.ask_save_as(),
                DayAction::ApplyParcel => self.day_editor_apply(false),
                DayAction::ApplyRegion => self.day_editor_apply(true),
                DayAction::Commit if !self.day_editor.commit_lost => self.day_editor_commit(),
                DayAction::Commit => {}
                DayAction::Changed => self.show_day_edit(),
                DayAction::LoadFrame(item) | DayAction::LoadTrack(item) => {
                    let purpose = if matches!(action, DayAction::LoadFrame(_)) {
                        DayFetch::Frame
                    } else {
                        DayFetch::Track
                    };
                    if let Some(it) = self.world.inventory.items.get(&item).cloned() {
                        self.day_fetch = Some((item, purpose));
                        self.send(NetCommand::PreviewInventoryItem(it));
                    }
                }
                DayAction::Save => self.day_editor_save(None),
                DayAction::SaveAs(name) => self.day_editor_save(Some(name)),
                DayAction::ApplyLocal => {
                    if let Some(day) = self.day_editor.rendered().cloned() {
                        self.world.eep.apply_local_day(day);
                        self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-applied").into(), false));
                    }
                }
                DayAction::Closed => {
                    self.day_fetch = None;
                    self.world.eep.clear_edit();
                }
            }
        }
    }

    /// Save over the item, or Save As a new day cycle item (the same
    /// requests as the sky and water editors, settings type 2).
    fn day_editor_save(&mut self, new_name: Option<String>) {
        let Some(mut doc) = self.day_editor.doc.clone() else {
            return;
        };
        if self.day_editor.saving.is_some() {
            return;
        }
        let request = Uuid::new_v4();
        let item = self.day_editor.item.and_then(|id| self.world.inventory.items.get(&id).cloned());
        match (new_name, item) {
            (Some(name), Some(it)) if !self.day_editor.rights.modify => {
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
                self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-copied").into(), false));
            }
            (None, Some(it)) => {
                self.day_editor.saving = Some(request);
                self.send(NetCommand::SaveInventoryContent {
                    request,
                    item: it.id,
                    asset_type: 56,
                    data: doc.to_asset(),
                });
            }
            (name, it) => {
                let inv = &self.world.inventory;
                let settings_folder = crate::world::inventory::actions::system(inv, 56).unwrap_or(inv.root);
                let own = it.as_ref().filter(|it| !crate::world::inventory::actions::library(inv, it.id));
                let parent = own.map_or(settings_folder, |it| it.parent);
                let next_owner_mask = own.map(|it| it.next_owner_mask);
                let name = name.unwrap_or_else(|| doc.name().to_owned());
                doc.set_name(&name);
                self.day_editor.saving = Some(request);
                self.send(NetCommand::CreateSettingsItem {
                    request,
                    parent,
                    name,
                    subtype: 2,
                    next_owner_mask,
                    data: doc.to_asset(),
                });
            }
        }
    }

    pub(super) fn day_editor_saved(&mut self, result: Result<aurora_net::inventory::InvItem, String>) {
        self.day_editor.saving = None;
        match result {
            Ok(it) => {
                self.world.inventory.add_items(vec![it.clone()]);
                self.day_editor.saved(it.id, &it.name, Rights::from_item(it.owner_mask, 0));
                log::info!("day cycle saved: {} ({})", it.name, it.asset_id);
            }
            Err(e) => {
                log::warn!("day cycle not saved: {e}");
                self.day_editor.message = Some((e, true));
            }
        }
    }

    /// `AURORA_DEMO_ENV_EDITOR=day|day-play|day-edit`: the user's day cycle
    /// opens (frame 400); `day-play` plays it; `day-edit` selects the
    /// ground frame at the middle of the day, brightens it, adds a frame at
    /// 75 %, copies the track to Sky 3 and saves (frames 900 to 1100).
    pub(super) fn demo_day_editor(&mut self, scenario: &str, frame: u64) {
        match frame {
            400 => {
                let item = crate::demo::env::day_item();
                let folder = Uuid::from_u128(item.as_u128() & !0xFFFF | crate::demo::env::USER_SETTINGS);
                self.world.inventory.request(folder);
                self.day_editor.wait_item = Some(item);
            }
            // day-notrans: the library's no-transfer sky loaded at 60 %
            // (SettingsMakeNoTrans asks first)
            450 if scenario == "day-notrans" => {
                let sky = crate::demo::env::no_transfer_sky_item();
                self.world.inventory.request(crate::demo::env::folder_of(sky));
            }
            900 if scenario == "day-notrans" => {
                self.day_editor.set_pos(0.6);
                let sky = crate::demo::env::no_transfer_sky_item();
                if let Some(it) = self.world.inventory.items.get(&sky).cloned() {
                    self.day_fetch = Some((sky, DayFetch::Frame));
                    self.send(NetCommand::PreviewInventoryItem(it));
                }
            }
            900 if scenario == "day-play" => self.day_editor.start_play(),
            900 if scenario == "day-edit" => {
                self.day_editor.set_pos(0.5);
                self.day_editor.edit_selected_sky(|s| s.gamma = 2.4);
                self.show_day_edit();
            }
            950 if scenario == "day-edit" => {
                self.day_editor.set_pos(0.75);
                self.day_editor.add_frame_at_cursor();
                self.show_day_edit();
            }
            1000 if scenario == "day-edit" => {
                self.day_editor.copy_track(1, 3);
                self.day_editor.select_track(3);
                self.show_day_edit();
            }
            1100 if scenario == "day-edit" => self.day_editor_save(None),
            _ => {}
        }
    }
}
