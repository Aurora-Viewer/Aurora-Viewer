//! The environment editors and the parcel or region environment: "Apply To
//! Parcel / Region" (LLFloaterEditEnvironmentBase::doApplyEnvironment), the
//! day cycle editor's parcel and region contexts (About Land's "Custom",
//! LLPanelEnvironmentInfo::onBtnEdit / onEditCommitted), a new day from
//! Firestorm's default day (setEditDefaultDayCycle), the Windlight day
//! import (loadSettingFromFile) and the local textures check of
//! onButtonApply (newview/llfloatereditenvironmentbase.cpp,
//! llfloatereditextdaycycle.cpp, llfloaterfixedenvironment.cpp,
//! llpanelenvironment.cpp, llenvironment.cpp, llsettingsvo.cpp, originally
//! LGPL 2.1).

use super::App;
use crate::scene::settings::MapState;
use crate::ui::env_day_editor::{self, DayContext};
use crate::ui::env_editor::EditorAccess;
use crate::world::eep;
use crate::world::env_apply::{self, Target};
use crate::world::env_day_edit::{self, DayDoc, LegacyError, LegacyFolder};
use crate::world::env_select::SettingsKind;
use crate::world::land::{AgentRights, EnvState};
use crate::world::notifications::{Data, Kind};
use crate::world::windlight_presets;
use aurora_llsd::Llsd;
use aurora_net::land::LandCommand;
use aurora_net::{NetCommand, ParcelInfo};
use std::path::Path;
use std::sync::Arc;

/// Which editor sent an ExtEnvironment update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Sender {
    Fixed,
    Day,
    /// The day cycle editor's commit: the editor has closed.
    Commit,
}

/// An ExtEnvironment update waiting for its answer.
#[derive(Debug, Clone, Copy)]
pub(super) struct Applying {
    target: Target,
    sender: Sender,
}

/// A Windlight day read for "Import": None when the dialog was cancelled.
pub(super) type DayImport = Option<Result<DayDoc, String>>;

/// The parcel or region day an editor clones or opens, with its timing.
fn day_doc(map: &Llsd) -> Option<DayDoc> {
    let mut map = map.clone();
    if !map.has("type") {
        map.insert("type", "daycycle");
    }
    DayDoc::from_llsd(map)
}

/// "50" or "25.5": a position in percent as LLSD streams a real.
fn percent(pos: f32) -> String {
    let v = pos * 100.0;
    if v.fract().abs() < 1e-4 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

impl App {
    /// LLSettingsBridge::performAction(apply_settings_parcel): apply the
    /// inventory asset directly, retaining the ordinary access/ack path.
    pub(super) fn inventory_environment_parcel(&mut self, item: uuid::Uuid) {
        let Some(item) = self
            .world
            .inventory
            .items
            .get(&item)
            .filter(|item| item.asset_type == 56 && !item.asset_id.is_nil())
            .cloned()
        else {
            return;
        };
        let title = aurora_i18n::tr!("ui-env-editor-apply-parcel");
        let Some(target) = self.apply_target(false, title) else { return };
        let command = LandCommand::EnvironmentApply {
            local_id: target.local_id(),
            track: None,
            asset: item.asset_id,
            name: item.name,
            flags: env_apply::apply_flags(Some(item.owner_mask), item.flags),
            day_length: -1,
            day_offset: -1,
        };
        self.send_env_apply(target, command, Sender::Commit);
    }
    /// LLEnvironment::isExtendedEnvironmentEnabled: the agent's region has
    /// ExtEnvironment (always in the offline demo).
    pub(super) fn extended_environment(&self) -> bool {
        self.demo
            || self
                .world
                .main_region
                .and_then(|h| self.world.regions.get(&h))
                .is_some_and(|r| r.caps.contains_key("ExtEnvironment"))
    }

    /// LLAgent::canManageEstate (we are never godlike): an estate manager or
    /// the region's owner.
    pub(super) fn can_manage_estate(&self) -> bool {
        self.world
            .main_region
            .and_then(|h| self.world.regions.get(&h))
            .and_then(|r| r.info.as_ref())
            .is_some_and(|i| i.is_estate_manager || i.owner == self.world.agent_id)
    }

    /// LLViewerParcelMgr::getAgentOrSelectedParcel: About Land's selection,
    /// else the parcel the agent stands on.
    fn agent_or_selected_parcel(&self) -> Option<Arc<ParcelInfo>> {
        self.world
            .land
            .sel
            .as_ref()
            .and_then(|s| s.parcel.clone())
            .or_else(|| self.world.parcel.clone())
    }

    /// LLEnvironment::isInventoryEnabled, canApplyParcel and canApplyRegion.
    pub(super) fn env_editor_access(&self) -> EditorAccess {
        let inventory =
            self.demo
                || self.world.main_region.and_then(|h| self.world.regions.get(&h)).is_some_and(|r| {
                    r.caps.contains_key("UpdateSettingsAgentInventory") && r.caps.contains_key("UpdateSettingsTaskInventory")
                });
        let rights = AgentRights {
            id: self.world.agent_id,
            groups: &self.world.groups.groups,
        };
        let extended = self.extended_environment();
        EditorAccess {
            inventory,
            parcel: self
                .agent_or_selected_parcel()
                .is_some_and(|p| env_apply::can_update_parcel(&p, &rights, extended)),
            region: self.can_manage_estate(),
        }
    }

    /// LLEnvironment::saveBeaconsState / revertBeaconsState: the beacons as
    /// they were when the first editor opened; when the last one closes, a
    /// beacon switched on in an editor goes off again (one already on stays).
    pub(super) fn env_beacons_frame(&mut self) {
        let current = [self.settings.sun_beacon, self.settings.moon_beacon];
        // the sky, water and day editors and "Personal lighting"
        // (LLFloaterEnvironmentAdjust counts as an editor too)
        let editing = self.env_editor.open || self.day_editor.open || self.panels.personal_lighting;
        match (editing, self.env_beacons_saved) {
            (true, None) => self.env_beacons_saved = Some(current),
            (false, Some(_)) => self.env_beacons_release(),
            _ => {}
        }
    }

    /// revertBeaconsState: the editors have closed (or the viewer quits).
    pub(super) fn env_beacons_release(&mut self) {
        if let Some(saved) = self.env_beacons_saved.take() {
            let current = [self.settings.sun_beacon, self.settings.moon_beacon];
            self.set_env_beacons([saved[0] && current[0], saved[1] && current[1]]);
        }
    }

    /// The Personal Lighting window (`open` its state this frame): with the
    /// interface, and over the photo viewfinder, which hides the rest. Its
    /// Reset, if pressed, for [`Self::env_actions`].
    pub(super) fn personal_lighting(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        mut open: bool,
    ) -> Option<crate::ui::environment::EnvAction> {
        // captureCurrentEnvironment: on opening, and again when another
        // local change left no fixed sky to edit (onEnvironmentUpdated)
        if open && (!self.lighting_was_open || self.world.eep.personal_sky().is_none()) {
            self.world.eep.capture_personal();
        }
        self.env_ui.beacons = [self.settings.sun_beacon, self.settings.moon_beacon];
        self.env_ui.night =
            (self.settings.night_sky == crate::world::night_sky::NightSky::Real).then_some(crate::ui::environment::NightSkyPick {
                place: self.settings.night_sky_place,
                latitude: self.settings.night_sky_latitude,
                changed: false,
            });
        let reset = self.env_ui.show_lighting(ctx, p, &mut self.world.eep, &mut open);
        if open {
            self.set_env_beacons(self.env_ui.beacons);
        }
        // the night sky's place, live while the latitude is dragged, saved
        // once the button is released
        if let Some(night) = self.env_ui.night.take().filter(|n| n.changed) {
            self.settings.night_sky_place = night.place;
            self.settings.night_sky_latitude = night.latitude;
            self.settings.sync_night_sky();
            self.night_sky_unsaved = true;
        }
        if self.night_sky_unsaved && !ctx.input(|i| i.pointer.any_down()) {
            self.night_sky_unsaved = false;
            self.settings.save();
        }
        self.lighting_was_open = open;
        self.panels.personal_lighting = open;
        reset
    }

    /// The environment lists' and Personal Lighting's choices.
    pub(super) fn env_actions(&mut self, actions: impl IntoIterator<Item = crate::ui::environment::EnvAction>) {
        use crate::ui::environment::EnvAction;
        for action in actions {
            match action {
                EnvAction::Pick(kind, asset) => self.world.eep.request_local(asset, kind),
                EnvAction::Shared => {
                    self.world.eep.clear_local();
                    self.panels.time_of_day = 0;
                }
                EnvAction::Preset(n) => self.panels.time_of_day = n,
            }
        }
    }

    /// The sunbeacon / moonbeacon settings, saved when changed.
    pub(super) fn set_env_beacons(&mut self, beacons: [bool; 2]) {
        if beacons != [self.settings.sun_beacon, self.settings.moon_beacon] {
            self.settings.sun_beacon = beacons[0];
            self.settings.moon_beacon = beacons[1];
            self.settings.save();
        }
    }

    /// One of Firestorm's alertmodal notifications of the editors.
    fn env_alert(&mut self, title: &str, text: impl Into<String>) {
        self.world.notifications.push(Kind::Alert, title, text, Data::None);
    }

    fn fixed_title(&self) -> &'static str {
        if self.env_editor.kind == Some(SettingsKind::Water) {
            aurora_i18n::tr!("ui-env-editor-edit-water")
        } else {
            aurora_i18n::tr!("ui-env-editor-edit-sky")
        }
    }

    fn is_local_texture(&self) -> impl Fn(uuid::Uuid) -> bool + '_ {
        |id| !id.is_nil() && self.world.local_assets.contains_key(&id)
    }

    /// onButtonApply's local texture check of the sky and water editors:
    /// false (and WLLocalTextureFixedBlock) when a texture is a local file.
    pub(super) fn env_editor_settings_ok(&mut self) -> bool {
        let Some(doc) = &self.env_editor.doc else { return false };
        let water = doc.kind() == SettingsKind::Water;
        let Some(field) = env_apply::local_texture(doc.llsd_ref(), water, &self.is_local_texture()) else {
            return true;
        };
        let title = self.fixed_title();
        self.env_alert(title, aurora_i18n::tr!("ui-env-apply-local-fixed", field = field.label()));
        false
    }

    /// The day cycle editor's brute-force local texture scan
    /// (WLLocalTextureDayBlock).
    pub(super) fn day_editor_settings_ok(&mut self) -> bool {
        let Some(doc) = &self.day_editor.doc else { return false };
        let Some(found) = env_apply::local_texture_in_day(doc, &self.is_local_texture()) else {
            return true;
        };
        let track = env_day_editor::track_label(found.track, self.day_editor.altitudes.as_ref());
        let text = aurora_i18n::tr!(
            "ui-env-apply-local-day",
            track = track,
            frame = found.frame as i64,
            pct = percent(found.pos),
            field = found.field.label()
        );
        self.env_alert(aurora_i18n::tr!("ui-day-editor-title"), text);
        false
    }

    /// Where "Apply To Parcel / Region" goes; WLParcelApplyFail when no
    /// parcel can be identified.
    fn apply_target(&mut self, region: bool, title: &str) -> Option<Target> {
        if region {
            return Some(Target::Region);
        }
        match self.agent_or_selected_parcel() {
            Some(p) if p.local_id != aurora_net::land::REGION_PARCEL_ID => Some(Target::Parcel(p.local_id)),
            _ => {
                log::warn!("environment: cannot identify the parcel, not applying");
                self.env_alert(title, aurora_i18n::tr!("ui-env-apply-parcel-fail"));
                None
            }
        }
    }

    /// The current day of the parcel (or the region's) or of the region, as
    /// createDayCycleFromEnvironment clones it.
    fn current_day_settings(&mut self, target: Target) -> Option<Arc<Llsd>> {
        match self.world.eep.day_settings(matches!(target, Target::Parcel(_))) {
            Ok((map, ..)) => Some(map),
            Err(true) => match self.scene.settings.map(eep::DEFAULT_DAY_ASSET) {
                MapState::Ready(map) => Some(map),
                _ => None,
            },
            Err(false) => None,
        }
    }

    /// Send an ExtEnvironment update and wait for its answer.
    fn send_env_apply(&mut self, target: Target, cmd: LandCommand, sender: Sender) {
        log::info!("environment: applying to {target:?}");
        let text = match target {
            Target::Region => aurora_i18n::tr!("ui-env-apply-sent-region"),
            Target::Parcel(_) => aurora_i18n::tr!("ui-env-apply-sent-parcel"),
        };
        match sender {
            Sender::Fixed => self.env_editor.message = Some((text.into(), false)),
            Sender::Day => self.day_editor.message = Some((text.into(), false)),
            Sender::Commit => {}
        }
        // About Land shows its environment loading when it is that parcel's
        if let Target::Parcel(id) = target
            && let Some(sel) = self.world.land.sel.as_mut()
            && sel.local_id() == Some(id)
        {
            sel.env = EnvState::Loading;
        }
        self.env_applying = Some(Applying { target, sender });
        self.send(NetCommand::Land(cmd));
    }

    /// doApplyEnvironment(ACTION_APPLY_PARCEL / REGION) of the sky and water
    /// editors: the item's asset when unchanged, else the settings inline in
    /// the current day.
    pub(super) fn env_editor_apply(&mut self, region: bool) {
        let Some(mut doc) = self.env_editor.doc.clone() else { return };
        let title = self.fixed_title();
        let Some(target) = self.apply_target(region, title) else { return };
        let item = self.env_editor.item.and_then(|id| self.world.inventory.items.get(&id).cloned());
        let flags = env_apply::apply_flags(item.as_ref().map(|i| i.owner_mask), doc.flags());
        doc.set_flags(flags);
        let cmd = match item.filter(|_| !self.env_editor.dirty) {
            Some(it) => {
                if region && !self.extended_environment() {
                    self.env_alert(title, aurora_i18n::tr!("ui-env-apply-no-environment"));
                    return;
                }
                LandCommand::EnvironmentApply {
                    local_id: target.local_id(),
                    track: None,
                    asset: it.asset_id,
                    name: it.name,
                    flags,
                    day_length: -1,
                    day_offset: -1,
                }
            }
            None => {
                let day = self
                    .current_day_settings(target)
                    .and_then(|current| env_apply::day_around(&current, &doc));
                let Some(day) = day else {
                    log::warn!("environment: no day to build the settings into");
                    self.env_alert(title, aurora_i18n::tr!("ui-env-apply-build-failed"));
                    return;
                };
                LandCommand::EnvironmentApplyDay {
                    local_id: target.local_id(),
                    track: None,
                    day: day.to_llsd(),
                    flags: day.flags(),
                    day_length: -1,
                    day_offset: -1,
                }
            }
        };
        self.send_env_apply(target, cmd, Sender::Fixed);
    }

    /// doApplyEnvironment of the day cycle editor (inventory context): the
    /// item's asset when unchanged, else the day inline. Firestorm's base
    /// class tests the type "day" where a day cycle's is "daycycle", so a
    /// changed day is never sent there; Aurora sends it.
    pub(super) fn day_editor_apply(&mut self, region: bool) {
        let Some(mut doc) = self.day_editor.doc.clone() else { return };
        let title = aurora_i18n::tr!("ui-day-editor-title");
        let Some(target) = self.apply_target(region, title) else { return };
        let item = self.day_editor.item.and_then(|id| self.world.inventory.items.get(&id).cloned());
        let flags = env_apply::apply_flags(item.as_ref().map(|i| i.owner_mask), doc.flags());
        doc.set_flags(flags);
        let cmd = match item.filter(|_| !self.day_editor.dirty) {
            Some(it) => {
                if region && !self.extended_environment() {
                    self.env_alert(title, aurora_i18n::tr!("ui-env-apply-no-environment"));
                    return;
                }
                LandCommand::EnvironmentApply {
                    local_id: target.local_id(),
                    track: None,
                    asset: it.asset_id,
                    name: it.name,
                    flags,
                    day_length: -1,
                    day_offset: -1,
                }
            }
            None => LandCommand::EnvironmentApplyDay {
                local_id: target.local_id(),
                track: None,
                day: doc.to_llsd(),
                flags,
                day_length: -1,
                day_offset: -1,
            },
        };
        self.send_env_apply(target, cmd, Sender::Day);
    }

    /// doApplyCommit then onEditCommitted: the editor closes and the edit
    /// slot goes; a changed day is applied with the environment's day length
    /// and offset.
    pub(super) fn day_editor_commit(&mut self) {
        let Some(doc) = self.day_editor.doc.clone() else { return };
        let (context, timing) = (self.day_editor.context, self.day_editor.timing);
        let day = doc.to_llsd();
        let changed = self.day_editor.committed.as_ref() != Some(&day);
        self.day_editor.close();
        self.day_fetch = None;
        self.world.eep.clear_edit();
        let target = match context {
            DayContext::Parcel(id) => Target::Parcel(id),
            DayContext::Region => Target::Region,
            DayContext::Inventory => return,
        };
        if !changed {
            log::info!("environment: the day is unchanged, nothing to apply");
            return;
        }
        let cmd = LandCommand::EnvironmentApplyDay {
            local_id: target.local_id(),
            track: None,
            flags: doc.flags(),
            day,
            day_length: timing.0,
            day_offset: timing.1,
        };
        self.send_env_apply(target, cmd, Sender::Commit);
    }

    /// The answers of the frame to an update an editor sent: "applied", or
    /// WLRegionApplyFail with the server's reason.
    pub(super) fn env_apply_answers(&mut self) {
        for (local_id, result) in std::mem::take(&mut self.world.environment_answers) {
            let Some(applying) = self.env_applying.filter(|a| a.target.local_id() == local_id) else {
                continue;
            };
            self.env_applying = None;
            let title = match applying.sender {
                Sender::Fixed => self.fixed_title(),
                _ => aurora_i18n::tr!("ui-day-editor-title"),
            };
            let message = match &result {
                Ok(()) => {
                    log::info!("environment: applied to {:?}", applying.target);
                    let text = match applying.target {
                        Target::Region => aurora_i18n::tr!("ui-env-apply-done-region"),
                        Target::Parcel(_) => aurora_i18n::tr!("ui-env-apply-done-parcel"),
                    };
                    (text.to_owned(), false)
                }
                Err(reason) => {
                    log::warn!("environment: not applied to {:?}: {reason}", applying.target);
                    self.env_alert(title, aurora_i18n::tr!("ui-env-apply-region-fail", reason = reason.as_str()));
                    // the alert holds the reason, the footer only the outcome
                    let text = match applying.target {
                        Target::Region => aurora_i18n::tr!("ui-env-apply-not-region"),
                        Target::Parcel(_) => aurora_i18n::tr!("ui-env-apply-not-parcel"),
                    };
                    (text.to_owned(), true)
                }
            };
            match applying.sender {
                Sender::Fixed if self.env_editor.open => self.env_editor.message = Some(message),
                Sender::Day if self.day_editor.open => self.day_editor.message = Some(message),
                _ => {}
            }
        }
    }

    /// Open the day cycle editor on a parcel's or region's day, or on
    /// Firestorm's default day when there is none (onBtnEdit).
    fn open_day_editor_in(&mut self, context: DayContext, day: Option<DayDoc>, timing: (i32, i32)) {
        self.day_editor.close();
        self.day_fetch = None;
        let extended = self.extended_environment();
        self.day_editor.open = true;
        self.day_editor.context = context;
        self.day_editor.timing = timing;
        self.day_editor.extended = extended;
        // onOpen: the sky tracks named by their altitude for a parcel or a region
        self.day_editor.altitudes = (extended && context != DayContext::Inventory).then(|| self.world.eep.altitudes());
        if !self.env_scan {
            self.env_scan = true;
            self.env_ui.scan_left = usize::MAX;
        }
        match day {
            Some(doc) => {
                self.day_editor.start_in(doc, context, timing);
                self.show_day_edit();
            }
            None => self.day_editor.wait_default = true,
        }
    }

    /// About Land's "Custom": the selected parcel's day in the parcel
    /// context, named as the environment names it (setEditName).
    pub(super) fn open_parcel_day_editor(&mut self) {
        let Some(sel) = self.world.land.sel.as_ref() else { return };
        let Some(local_id) = sel.local_id() else { return };
        let summary = match &sel.env {
            EnvState::Ready(e) => Some(e.clone()),
            _ => None,
        };
        let timing = summary.as_ref().map_or((-1, -1), |e| (e.day_length as i32, e.day_offset as i32));
        let day = summary
            .as_ref()
            .and_then(|e| e.day_cycle.as_deref())
            .and_then(day_doc)
            .map(|mut d| {
                if let Some(e) = &summary {
                    d.set_name(&e.day_name);
                }
                d
            });
        log::info!(
            "environment: custom day of parcel {local_id} ({})",
            if day.is_some() { "its own" } else { "default" }
        );
        let followed = summary.and_then(|e| e.day_cycle);
        self.open_day_editor_in(DayContext::Parcel(local_id), day, timing);
        self.day_editor.followed = followed;
    }

    /// LLPanelEnvironmentInfo::updateEditFloater while the parcel context's
    /// editor is open: About Land's environment changed (another parcel
    /// selected, a day applied or reset elsewhere, the right lost). A new day
    /// replaces the edited one, changes or not; without a day (or the right)
    /// the editor closes, or stays without its commit when it has changes.
    /// About Land closed: the editor keeps its parcel, as the hidden panel.
    pub(super) fn day_editor_follow_parcel(&mut self) {
        let DayContext::Parcel(edited) = self.day_editor.context else {
            return;
        };
        if !self.day_editor.open {
            return;
        }
        let extended = self.extended_environment();
        let Some(sel) = self.world.land.sel.as_ref() else { return };
        let (Some(local_id), EnvState::Ready(summary)) = (sel.local_id(), &sel.env) else {
            return;
        };
        let rights = AgentRights {
            id: self.world.agent_id,
            groups: &self.world.groups.groups,
        };
        let enabled = sel
            .parcel
            .as_ref()
            .is_some_and(|p| env_apply::can_update_parcel(p, &rights, extended));
        let summary = summary.clone();
        let day = summary.day_cycle.clone();
        let same = local_id == edited && day == self.day_editor.followed;
        if same && (enabled || self.day_editor.commit_lost) {
            return;
        }
        match day.as_deref().and_then(day_doc).filter(|_| enabled) {
            Some(mut doc) if !same => {
                log::info!("environment: the parcel's day changed, the editor follows it (parcel {local_id})");
                doc.set_name(&summary.day_name);
                let timing = (summary.day_length as i32, summary.day_offset as i32);
                self.open_day_editor_in(DayContext::Parcel(local_id), Some(doc), timing);
                self.day_editor.followed = day;
            }
            Some(_) => {}
            None if self.day_editor.dirty => {
                log::info!("environment: the parcel can no longer take the edited day, its commit is off");
                self.day_editor.commit_lost = true;
            }
            None => {
                log::info!("environment: the parcel has no day to edit any more, the editor closes");
                self.day_editor.close();
                self.day_fetch = None;
                self.world.eep.clear_edit();
            }
        }
    }

    /// The region's day in the region context (Region / Estate's
    /// "Customize", LLPanelRegionEnvironment).
    pub(super) fn open_region_day_editor(&mut self) {
        let timing = self.world.eep.region_timing().map_or((-1, -1), |(l, o)| (l as i32, o as i32));
        let day = match self.world.eep.day_settings(false) {
            Ok((map, ..)) => day_doc(&map),
            _ => None,
        };
        self.open_day_editor_in(DayContext::Region, day, timing);
    }

    /// "New day cycle": the editor on Firestorm's default day, no item.
    pub(super) fn open_new_day_editor(&mut self) {
        self.open_day_editor_in(DayContext::Inventory, None, (-1, -1));
    }

    /// setEditDefaultDayCycle: the default day asset once loaded. In a
    /// parcel or region context it is always committed (the parcel had no
    /// day of its own to compare with).
    pub(super) fn day_editor_default(&mut self) {
        if !self.day_editor.wait_default {
            return;
        }
        match self.scene.settings.map(eep::DEFAULT_DAY_ASSET) {
            MapState::Pending => {}
            MapState::Ready(map) => match day_doc(&map) {
                Some(doc) => {
                    let (context, timing) = (self.day_editor.context, self.day_editor.timing);
                    self.day_editor.start_in(doc, context, timing);
                    self.day_editor.committed = None;
                    self.show_day_edit();
                }
                None => self.default_day_failed(),
            },
            MapState::Failed => self.default_day_failed(),
        }
    }

    fn default_day_failed(&mut self) {
        log::warn!("the default day cycle could not be loaded");
        self.day_editor.wait_default = false;
        self.day_editor.message = Some((aurora_i18n::tr!("ui-env-editor-load-failed", name = "Default Day Cycle"), true));
    }

    /// "Import" of the day cycle editor (doImportFromDisk): a Windlight day
    /// file and the skies and water next to its folder, read off the frame.
    pub(super) fn day_editor_import(&mut self) {
        if self.day_import.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.day_import = Some(rx);
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("ui-day-editor-import-tip"))
                .add_filter("XML", &["xml"])
                .pick_file()
                .map(|path| import_day_file(&path));
            let _ = tx.send(result);
        });
    }

    /// `AURORA_DEMO_ENV_EDITOR=parcel|region|custom` (`-refused`: the
    /// simulator refuses inline days):
    /// - `parcel`: the sky editor on "My purple sky" (frame 400), applied to
    ///   the parcel unchanged, as its asset (900), then brightened and
    ///   applied inline in the region's day (1000);
    /// - `region`: the day cycle editor on the region's day in the region
    ///   context (400), its first ground frame brightened (900), committed
    ///   (1000);
    /// - `custom`: About Land on the parcel's environment, "Custom" chosen
    ///   (400): the parcel's day in the parcel context, left to the tester.
    pub(super) fn demo_env_apply(&mut self, scenario: &str, frame: u64) {
        let base = scenario.split('-').next().unwrap_or(scenario);
        match (base, frame) {
            ("parcel", 400) => {
                let item = crate::demo::env::editor_item();
                let folder = uuid::Uuid::from_u128(item.as_u128() & !0xFFFF | crate::demo::env::USER_SETTINGS);
                self.world.inventory.request(folder);
                self.env_editor.wait_item = Some(item);
            }
            ("parcel", 900) => {
                log::info!("demo environment: the sky item applied to the parcel");
                self.env_editor_apply(false);
            }
            ("parcel", 1000) => {
                if let Some(doc) = &mut self.env_editor.doc {
                    let before = doc.sky();
                    let after = eep::SkyFrame { gamma: 1.6, ..before };
                    doc.edit_sky(&before, &after);
                    self.env_editor.dirty = true;
                }
                log::info!("demo environment: the edited sky applied to the parcel");
                self.env_editor_apply(false);
            }
            ("region", 400) => self.open_region_day_editor(),
            ("region", 900) => {
                self.day_editor.set_pos(0.0);
                self.day_editor.edit_selected_sky(|s| s.gamma = 2.2);
                self.show_day_edit();
            }
            ("region", 1000) => {
                log::info!("demo environment: the region's day committed");
                self.day_editor_commit();
            }
            ("custom", 400) => self.land_ui.custom_day = true,
            // custom-follow: another day applied to the parcel meanwhile
            // (the library's "Dynamic Day", as About Land's picker does)
            ("custom", 900) if scenario == "custom-follow" => {
                log::info!("demo environment: another day applied to the parcel");
                let asset = crate::demo::env::library_day_asset();
                self.world.land.environment(|local_id| LandCommand::EnvironmentApply {
                    local_id,
                    track: None,
                    asset,
                    name: "Dynamic Day".into(),
                    flags: 0,
                    day_length: -1,
                    day_offset: -1,
                });
            }
            _ => {}
        }
    }

    /// The imported day, or WLImportFail.
    pub(super) fn day_editor_imported(&mut self) {
        let Some(rx) = &self.day_import else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.day_import = None;
        match result {
            None => {}
            Some(Ok(doc)) => {
                log::info!("windlight day imported: {}", doc.name());
                self.day_editor.imported(doc);
                self.show_day_edit();
            }
            Some(Err(text)) => {
                log::warn!("windlight day import failed");
                self.env_alert(aurora_i18n::tr!("ui-day-editor-title"), text);
            }
        }
    }
}

/// LLEnvironment::createDayCycleFromLegacyPreset on a file: the preset name
/// is the file's, the skies and water are read from the `skies` and `water`
/// folders beside its folder, or from Firestorm's built-in presets when a
/// folder is missing.
fn import_day_file(path: &Path) -> Result<DayDoc, String> {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let shown = path.display().to_string();
    let fail = |reasons: String| {
        aurora_i18n::tr!(
            "ui-day-editor-import-fail",
            name = crate::world::env_edit::legacy_preset_name(&name),
            file = shown.as_str(),
            reasons = reasons
        )
    };
    let data = std::fs::read(path).map_err(|e| {
        log::warn!("windlight day {}: {e}", path.display());
        fail(aurora_i18n::tr!("ui-day-editor-import-file-error", file = shown.as_str()))
    })?;
    let base = path.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default();
    let mut read = |folder: LegacyFolder, preset: &str| -> Result<(Vec<u8>, String), String> {
        let dir = base.join(match folder {
            LegacyFolder::Skies => "skies",
            LegacyFolder::Water => "water",
        });
        let names = env_day_edit::legacy_file_names(preset);
        // no such folder beside the day: Firestorm's own presets, built in
        if !dir.is_dir() {
            return names
                .iter()
                .find_map(|n| windlight_presets::file(folder, n).map(|d| (d, windlight_presets::shown_path(folder, n))))
                .ok_or_else(|| windlight_presets::shown_path(folder, &names[0]));
        }
        for file in &names {
            let p = dir.join(file);
            if let Ok(data) = std::fs::read(&p) {
                return Ok((data, p.display().to_string()));
            }
        }
        Err(dir.join(&names[0]).display().to_string())
    };
    DayDoc::import_legacy(&data, &shown, &name, &mut read).map_err(|e| {
        let reasons = match e {
            LegacyError::Open(file) | LegacyError::Parse(file) => {
                aurora_i18n::tr!("ui-day-editor-import-file-error", file = file)
            }
            LegacyError::Invalid(preset) => aurora_i18n::tr!("ui-day-editor-import-invalid", name = preset),
        };
        fail(reasons)
    })
}
