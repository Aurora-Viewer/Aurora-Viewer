//! Which environment (sky and water) the agent sees, and the crossfades
//! between environments.
//!
//! Port of LLEnvironment's selection (newview/llenvironment.cpp, Second Life
//! viewer / Firestorm, Copyright (C) Linden Research, Inc., originally LGPL
//! 2.1): recordEnvironment (region and parcel answers), the environment slots
//! with getSelectedEnvironmentInstance (local > parcel > region > default),
//! updateEnvironment with DayTransition (crossfade from what is shown to the
//! new environment) and LLTrackBlenderLoopingTime::switchTrack (altitude
//! tracks). Firestorm keeps a slot until an answer replaces or clears it, so
//! the region of the previous region stays shown until the new region
//! answers; a parcel slot only applies in its own region.
//!
//! The local environment (ENV_LOCAL) is what the user picks: the environment
//! selector's sky, water and day cycle items (LLEnvironment::setEnvironment
//! with a settings asset: a day replaces everything, a sky or a water is
//! fixed on top, the missing half taken from what is shown), Aurora's time
//! of day presets, "Personal lighting" (LLFloaterEnvironmentAdjust) and
//! "Shared environment" (setSharedEnvironment). It survives region
//! changes and, like EnvironmentPersistAcrossLogin, sessions
//! (saveToSettings / loadFromSettings).

use super::eep::{self, DayCycle, EnvAnswer, Settings, SkyFrame, WaterFrame};
use super::eep_photo::{PhotoChoice, PhotoLayer};
use super::eep_push::PushLayer;
use super::env_select::{SettingsKind, Shown as ListShown};
use aurora_llsd::Llsd;
use aurora_net::RegionHandle;
use glam::Vec3;
use std::collections::VecDeque;
use std::sync::Arc;
use uuid::Uuid;

/// LLEnvironment::TRANSITION_DEFAULT (s): environment changes.
pub const TRANSITION_DEFAULT: f32 = 5.0;
/// LLEnvironment::TRANSITION_FAST (s): answers received while teleporting.
pub const TRANSITION_FAST: f32 = 1.0;
/// LLEnvironment::TRANSITION_ALTITUDE (s): sky track switches.
pub const TRANSITION_ALTITUDE: f32 = 5.0;

/// Where a fixed sky or water of the local environment comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalOrigin {
    /// A settings item (its asset).
    Item(Uuid),
    /// A time of day preset (1..=5): its library sky, or the built-in sky
    /// with the preset's sun when that sky could not be fetched.
    Preset(u8),
    /// Edited in "Personal lighting".
    Personal,
    /// What was shown when the other half was picked
    /// (LLEnvironment::setEnvironment(fixedEnvironment_t) fills the missing
    /// half with the current sky or water, frozen).
    Inherited,
}

/// The local environment (DayInstance of ENV_LOCAL): an animated day cycle
/// and / or a fixed sky and water (NO_ANIMATE_SKY / NO_ANIMATE_WATER); a
/// fixed half overrides the day's.
#[derive(Debug, Clone)]
pub struct LocalEnv {
    pub day: Option<(Uuid, Arc<DayCycle>)>,
    pub sky: Option<(LocalOrigin, SkyFrame)>,
    pub water: Option<(LocalOrigin, WaterFrame)>,
    /// Day length and offset (DEFAULT_DAYLENGTH / DEFAULT_DAYOFFSET unless
    /// restored from the last session).
    pub length: f64,
    pub offset: f64,
    generation: u64,
}

impl LocalEnv {
    fn new(generation: u64) -> LocalEnv {
        LocalEnv {
            day: None,
            sky: None,
            water: None,
            length: eep::DEFAULT_DAY_LENGTH,
            offset: eep::DEFAULT_DAY_OFFSET,
            generation,
        }
    }

    /// Sky track, sky and water at a time (the fixed halves first).
    fn frames(&self, altitudes: &[f32; 4], input: &FrameInput) -> (usize, SkyFrame, WaterFrame) {
        let pos = eep::cycle_position(input.unix_secs, self.length, self.offset);
        let track = match (&self.day, &self.sky) {
            (Some((_, d)), None) => d.track_for_altitude(altitudes, input.altitude),
            _ => 0,
        };
        let sky = match (&self.sky, &self.day) {
            (Some((_, s)), _) => *s,
            (None, Some((_, d))) => d.sky_at(pos, track),
            (None, None) => SkyFrame::default(),
        };
        let water = match (&self.water, &self.day) {
            (Some((_, w)), _) => *w,
            (None, Some((_, d))) => d.water_at(pos),
            (None, None) => WaterFrame::default(),
        };
        (track, sky, water)
    }

    /// What the selector's lists show (FloaterQuickPrefs::setSelectedEnvironment):
    /// a day shows in the day list ("Nothing" otherwise) and makes the sky and
    /// water "Based on day cycle" unless a fixed one with an asset
    /// overrides it; a fixed half without an asset matches no entry.
    pub fn shown(&self) -> LocalView {
        let other = if self.day.is_some() {
            ListShown::DayBased
        } else {
            ListShown::Shared
        };
        let half = |o: Option<LocalOrigin>| match o {
            Some(LocalOrigin::Item(id)) => ListShown::Item(id),
            Some(LocalOrigin::Preset(n)) => ListShown::Preset(n),
            Some(LocalOrigin::Personal) => ListShown::Personal,
            Some(LocalOrigin::Inherited) | None => other,
        };
        LocalView {
            sky: half(self.sky.map(|s| s.0)),
            water: half(self.water.map(|w| w.0)),
            day: self.day.as_ref().map_or(ListShown::DayBased, |d| ListShown::Item(d.0)),
        }
    }
}

/// What the selector shows for the sky, water and day lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalView {
    pub sky: ListShown,
    pub water: ListShown,
    pub day: ListShown,
}

impl LocalView {
    pub const SHARED: LocalView = LocalView {
        sky: ListShown::Shared,
        water: ListShown::Shared,
        day: ListShown::Shared,
    };
}

#[derive(Debug, Clone)]
enum PendingState {
    Waiting,
    Ready(Arc<Settings>, Option<LocalOrigin>),
    Failed,
}

/// A settings asset on its way to the local environment, applied in the
/// order they were asked for (a restored day before its fixed sky / water,
/// as loadFromSettings waits for the day).
#[derive(Debug, Clone)]
struct Pending {
    /// Nil for settings restored from the last session's file.
    asset: Uuid,
    kind: SettingsKind,
    state: PendingState,
    /// Restored day length and offset.
    timing: Option<(f64, f64)>,
    /// The time of day preset whose library sky this is (0: none).
    preset: u8,
}

/// LOCAL_ENV_STORAGE_FILE of LLEnvironment (in the account folder).
pub const LOCAL_ENV_FILE: &str = "local_environment_data.bin";

/// A day cycle in one of the environment slots (LLEnvironment::DayInstance).
#[derive(Debug, Clone)]
struct Slot {
    handle: RegionHandle,
    day: Arc<DayCycle>,
    /// The day's settings map as received (None for Firestorm's default
    /// day, whose asset the settings streamer keeps).
    settings: Option<Arc<Llsd>>,
    length: f64,
    offset: f64,
    /// Bumped on every change: a new slot crossfades even with the same source.
    generation: u64,
}

impl Slot {
    fn position(&self, unix_secs: f64) -> f32 {
        eep::cycle_position(unix_secs, self.length, self.offset)
    }
}

/// Where the shown environment comes from, by priority (EnvSelection_t).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// ENV_EDIT: what an open sky or water editor shows, above everything.
    Edit,
    /// ENV_LOCAL: the environment selector, the time-of-day presets,
    /// "Personal lighting".
    Local(u64),
    /// The photo's own sky, water or day over what lies beneath
    /// (`eep_photo`), while the photo window is open.
    Photo(u64),
    /// ENV_PUSH: what an experience's environment controller pushed.
    Push(u64),
    /// ENV_PARCEL of the main region.
    Parcel(u64),
    /// ENV_REGION: the region's day, or Firestorm's default day asset when
    /// the region has no valid day.
    Region(u64),
    /// ENV_DEFAULT (LLSettingsSky / LLSettingsWater defaults): before the
    /// first answer, or while the default day asset loads.
    BuiltIn,
    /// A region without EEP (no ExtEnvironment capability): the simulator's
    /// legacy sun. Never used for an EEP region (process_time_synch ignores
    /// the simulator sun there).
    Legacy,
}

/// What the frame needs from the world to pick the environment.
#[derive(Debug, Clone, Copy)]
pub struct FrameInput {
    pub main_region: Option<RegionHandle>,
    /// The main region has the ExtEnvironment capability (None: its
    /// capabilities have not arrived yet).
    pub eep_region: Option<bool>,
    /// Sun direction for [`Source::Legacy`].
    pub legacy_sun: Vec3,
    pub unix_secs: f64,
    /// Monotonic seconds (crossfades).
    pub now: f64,
    /// Agent altitude (m) for the sky track.
    pub altitude: f32,
}

/// Agent state when an answer is applied.
#[derive(Debug, Clone, Copy)]
pub struct AnswerContext {
    pub main_region: Option<RegionHandle>,
    /// Local id of the parcel the agent stands on, when known.
    pub agent_parcel: Option<i32>,
    pub teleporting: bool,
}

/// What an ExtEnvironment answer did (logged by the caller, and tested).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerOutcome {
    /// Answer for another region than the agent's (recordEnvironment).
    StaleRegion,
    /// Answer for a parcel the agent has left.
    StaleParcel,
    RegionDay,
    /// The region has no valid day: Firestorm's default day applies.
    RegionDefault,
    ParcelDay,
    /// The parcel has no (valid) day of its own: the region's applies.
    ParcelCleared,
}

#[derive(Debug, Clone, Copy)]
struct Shown {
    source: Source,
    track: usize,
    sky: SkyFrame,
    water: WaterFrame,
    /// The selected environment without the crossfade (what Firestorm's
    /// DayInstance holds: the half a fixed sky or water inherits, the sky
    /// "Personal lighting" starts from), without the photo's layer.
    target_sky: SkyFrame,
    target_water: WaterFrame,
    /// The same with the photo's layer (what "Keep this light" keeps).
    photo_sky: SkyFrame,
}

#[derive(Debug, Clone, Copy)]
struct Fade {
    sky: SkyFrame,
    water: WaterFrame,
    start: f64,
    duration: f32,
}

/// The environment slots and the transition state.
#[derive(Debug, Default)]
pub struct EnvSelector {
    pub rlva_locked: bool,
    local: Option<LocalEnv>,
    /// Time of day preset shown by the local sky (0: none).
    local_preset: u8,
    /// The preset was dropped by another local change: the panels follow.
    preset_dropped: bool,
    pending: VecDeque<Pending>,
    /// Assets that could not be loaded, for the FailedToFindSettings message.
    failed_local: Vec<Uuid>,
    /// The local environment changed since the last save.
    local_dirty: bool,
    /// FSEnvironmentManualTransitionTime (s): crossfade of manual changes.
    manual_transition: f32,
    /// Next local change without crossfade (TRANSITION_INSTANT).
    instant_next: bool,
    parcel: Option<Slot>,
    region: Option<Slot>,
    /// Region track altitudes (LLEnvironment::mTrackAltitudes), from the
    /// last valid region answer; parcels use them too.
    altitudes: [f32; 4],
    /// The region has no valid day: (region, length, offset) waiting for
    /// the default day asset.
    default_pending: Option<(RegionHandle, f64, f64)>,
    /// Firestorm's default day asset once loaded.
    default_day: Option<Arc<DayCycle>>,
    default_failed: bool,
    generation: u64,
    /// Crossfade length of the next environment change.
    next_fade: f32,
    shown: Option<Shown>,
    fade: Option<Fade>,
    /// ENV_EDIT (setEnvironment(ENV_EDIT, sky or water)): the edited half,
    /// the other one frozen at what was shown when the editor opened.
    edit: Option<(SkyFrame, WaterFrame)>,
    /// The photo's own light over the local environment (`eep_photo`).
    photo: PhotoLayer,
    /// What experience environment controllers pushed (`eep_push`).
    push: PushLayer,
    /// The selector changed the photo's choice (a change made elsewhere, a
    /// camera preset, "Keep this light"): the photo window follows.
    photo_changed: bool,
}

impl EnvSelector {
    pub fn has_fixed_sky(&self) -> bool {
        self.edit.is_some() || self.local.as_ref().is_some_and(|l| l.sky.is_some())
    }

    pub fn day_sky_at(&self, pos: f32, altitude: f32) -> Option<SkyFrame> {
        self.local
            .as_ref()
            .and_then(|l| l.day.as_ref().map(|(_, d)| d))
            .or_else(|| self.parcel.as_ref().map(|s| &s.day))
            .or_else(|| self.region.as_ref().map(|s| &s.day))
            .map(|day| day.sky_at(pos, day.track_for_altitude(&self.altitudes, altitude)))
    }
    /// An editor shows its sky or water (LLFloaterFixedEnvironment::
    /// updateEditEnvironment, TRANSITION_INSTANT); the other half stays what
    /// was shown when the editing began.
    pub fn set_edit_sky(&mut self, sky: SkyFrame) {
        let water = self.edit.map_or_else(|| self.current().1, |e| e.1);
        self.edit = Some((sky, water));
    }

    /// The day cycle editor: the day at its time cursor.
    pub fn set_edit(&mut self, sky: SkyFrame, water: WaterFrame) {
        self.edit = Some((sky, water));
    }

    /// "Apply only to myself" of the day cycle editor (setEnvironment(
    /// ENV_LOCAL, day)): a local day without an asset.
    pub fn apply_local_day(&mut self, day: DayCycle) {
        if self.rlva_locked {
            return;
        }
        self.photo_yield(SettingsKind::Day);
        self.apply_settings(Uuid::nil(), &Settings::Day(day), None, None);
    }

    pub fn set_edit_water(&mut self, water: WaterFrame) {
        let sky = self.edit.map_or_else(|| self.current().0, |e| e.0);
        self.edit = Some((sky, water));
    }

    /// The editor closed (clearEnvironment(ENV_EDIT)): back to what lies
    /// beneath, with the default transition.
    pub fn clear_edit(&mut self) {
        self.edit = None;
    }

    #[cfg(test)]
    pub fn editing(&self) -> bool {
        self.edit.is_some()
    }

    /// The selected sky and water without the crossfade (ENV_CURRENT), what
    /// a new editor starts from.
    pub fn current(&self) -> (SkyFrame, WaterFrame) {
        self.shown
            .map_or((SkyFrame::default(), WaterFrame::default()), |s| (s.target_sky, s.target_water))
    }

    /// "Apply only to myself" of an editor (setEnvironment(ENV_LOCAL, sky or
    /// water)): a fixed half of the local environment, without an asset.
    pub fn apply_local_sky(&mut self, sky: SkyFrame) {
        if self.rlva_locked {
            return;
        }
        // an edited sky without an asset: "Personal lighting" edits it next
        self.photo_yield(SettingsKind::Sky);
        self.apply_sky(LocalOrigin::Personal, sky);
    }

    pub fn apply_local_water(&mut self, water: WaterFrame) {
        if self.rlva_locked {
            return;
        }
        self.photo_yield(SettingsKind::Water);
        self.apply_water(LocalOrigin::Inherited, water);
    }
    fn slot(&mut self, handle: RegionHandle, day: Arc<DayCycle>, settings: Option<Arc<Llsd>>, length: f64, offset: f64) -> Slot {
        self.generation += 1;
        Slot {
            handle,
            day,
            settings,
            length,
            offset,
            generation: self.generation,
        }
    }

    /// Apply an ExtEnvironment answer for `handle`, requested for
    /// `requested_parcel` (-1: the region). Port of recordEnvironment.
    pub fn on_answer(&mut self, handle: RegionHandle, requested_parcel: i32, answer: EnvAnswer, ctx: AnswerContext) -> AnswerOutcome {
        if ctx.main_region != Some(handle) {
            return AnswerOutcome::StaleRegion;
        }
        let parcel_id = answer.parcel_id;
        if parcel_id != eep::REGION_PARCEL_ID && ctx.agent_parcel.is_some_and(|p| p != requested_parcel) {
            return AnswerOutcome::StaleParcel;
        }
        // the parcel slot of a region the agent left never applies again
        if self.parcel.as_ref().is_some_and(|p| p.handle != handle) {
            self.parcel = None;
        }
        // requestParcel: TRANSITION_FAST while a teleport is in progress
        self.next_fade = if ctx.teleporting { TRANSITION_FAST } else { TRANSITION_DEFAULT };
        let altitudes = answer.altitudes;
        let settings = answer.day_settings.clone();
        let day = answer.valid_day();
        if parcel_id == eep::REGION_PARCEL_ID {
            // a valid region day leaves the parcel's in place (RegionInfo
            // re-requests the region while the agent stays on its parcel)
            match day {
                Some(d) => {
                    self.altitudes = altitudes;
                    self.default_pending = None;
                    let (length, offset) = (d.length, d.offset);
                    self.region = Some(self.slot(handle, Arc::new(d), settings, length, offset));
                    AnswerOutcome::RegionDay
                }
                None => {
                    self.parcel = None;
                    // setEnvironment(ENV_REGION, GetDefaultAssetId()): the
                    // current region slot stays until the asset is loaded,
                    // and gives its length and offset to the default day
                    let (length, offset) = self
                        .region
                        .as_ref()
                        .map_or((eep::DEFAULT_DAY_LENGTH, eep::DEFAULT_DAY_OFFSET), |r| (r.length, r.offset));
                    self.default_pending = Some((handle, length, offset));
                    self.apply_default_day();
                    AnswerOutcome::RegionDefault
                }
            }
        } else {
            match day {
                Some(d) => {
                    let (length, offset) = (d.length, d.offset);
                    self.parcel = Some(self.slot(handle, Arc::new(d), settings, length, offset));
                    AnswerOutcome::ParcelDay
                }
                None => {
                    self.parcel = None;
                    AnswerOutcome::ParcelCleared
                }
            }
        }
    }

    /// Settings assets the selector waits for: Firestorm's default day and
    /// the items picked for the local environment.
    pub fn wanted_settings(&self) -> Vec<Uuid> {
        let mut out = Vec::new();
        if self.default_pending.is_some() && self.default_day.is_none() && !self.default_failed {
            out.push(eep::DEFAULT_DAY_ASSET);
        }
        for p in &self.pending {
            if matches!(p.state, PendingState::Waiting) && !out.contains(&p.asset) {
                out.push(p.asset);
            }
        }
        for id in self.photo.wanted().chain(self.push.wanted()) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    /// A `PushExpEnvironment` message from an experience controller.
    pub fn push_experience(&mut self, experience: Uuid, message: &Llsd) {
        self.push.handle(experience, message);
    }

    /// The agent forgot or blocked an experience: its environment goes
    /// (Forget fades fast, Block is instant: TRANSITION_FAST / INSTANT).
    pub fn clear_experience(&mut self, experience: Uuid, transition: f32) {
        self.push.clear(experience, transition);
    }

    /// Experiences with something pushed or waiting (the ExperienceQuery of
    /// the parcel check).
    pub fn pushing_experiences(&self) -> Vec<Uuid> {
        self.push.experiences()
    }

    /// A settings asset arrived (`None`: it could not be fetched or parsed).
    pub fn on_settings(&mut self, id: Uuid, settings: Option<Arc<Settings>>) {
        self.photo.on_settings(id, settings.as_ref());
        // onSetEnvAssetLoaded for a pushed environment: FailedToFindSettings
        let failed = self.push.on_settings(id, settings.as_ref());
        self.failed_local.extend(failed);
        let mut local = false;
        for p in self.pending.iter_mut().filter(|p| p.asset == id) {
            if matches!(p.state, PendingState::Waiting) {
                local = true;
                p.state = match &settings {
                    Some(s) => PendingState::Ready(s.clone(), (p.preset != 0).then_some(LocalOrigin::Preset(p.preset))),
                    None => PendingState::Failed,
                };
            }
        }
        if local {
            self.drain_pending();
        }
        if id != eep::DEFAULT_DAY_ASSET {
            return;
        }
        match settings {
            Some(s) => {
                let day = Settings::clone(&s).into_day();
                log::info!(
                    "EEP default day loaded: {} sky keys, {} water keys",
                    day.sky_tracks[0].len(),
                    day.water.len()
                );
                self.default_day = Some(Arc::new(day));
                self.apply_default_day();
            }
            None => {
                // onSetEnvAssetLoaded: Firestorm keeps the current environment
                log::warn!("EEP default day {id} unavailable; keeping the current sky");
                self.default_failed = true;
            }
        }
    }

    fn apply_default_day(&mut self) {
        let (Some((handle, length, offset)), Some(day)) = (self.default_pending, self.default_day.clone()) else {
            return;
        };
        self.default_pending = None;
        self.region = Some(self.slot(handle, day, None, length, offset));
    }

    /// The day of the parcel slot (`parcel`, falling back to the region's,
    /// like createDayCycleFromEnvironment) or of the region slot, as a
    /// settings map with its length and offset (s). `Err(true)`: the region
    /// shows Firestorm's default day, whose map the settings streamer keeps;
    /// `Err(false)`: no day at all.
    pub fn day_settings(&self, parcel: bool) -> Result<(Arc<Llsd>, f64, f64), bool> {
        let slot = if parcel {
            self.parcel.as_ref().or(self.region.as_ref())
        } else {
            self.region.as_ref()
        };
        match slot {
            Some(s) => match &s.settings {
                Some(m) => Ok((m.clone(), s.length, s.offset)),
                None => Err(true),
            },
            None => Err(false),
        }
    }

    /// Length and offset (s) of the region's day (LLPanelRegionEnvironment's
    /// mCurrentEnvironment), when it has one.
    pub fn region_timing(&self) -> Option<(f64, f64)> {
        self.region.as_ref().map(|r| (r.length, r.offset))
    }

    /// Where sky tracks 1..=4 start (LLEnvironment::getRegionAltitudes).
    pub fn altitudes(&self) -> [f32; 4] {
        self.altitudes
    }

    /// Time-of-day preset (0 = none) as the local environment: its library
    /// sky as a fixed local sky, as World > Environment does
    /// (setManualEnvironment(ENV_LOCAL, KNOWN_SKY_*)); 0 is "Use shared
    /// environments" (setSharedEnvironment). The sky shown stays until the
    /// asset arrives, like any settings item picked. If it cannot be
    /// fetched the Legacy Viewer keeps the sky shown (FailedToFindSettings);
    /// Aurora shows its built-in sky with the preset's sun instead, so the
    /// preset still does what its name says.
    pub fn set_time_of_day(&mut self, preset: u8) {
        if self.rlva_locked {
            return;
        }
        if preset == self.local_preset {
            return;
        }
        if preset == 0 {
            self.clear_local();
            self.preset_dropped = false;
            return;
        }
        let Some(asset) = super::env::time_of_day_sky(preset) else {
            return;
        };
        self.photo_yield(SettingsKind::Sky);
        self.pending
            .retain(|p| !(p.kind == SettingsKind::Sky && matches!(p.state, PendingState::Waiting) && !p.asset.is_nil()));
        self.pending.push_back(Pending {
            asset,
            kind: SettingsKind::Sky,
            state: PendingState::Waiting,
            timing: None,
            preset,
        });
        self.local_preset = preset;
        self.preset_dropped = false;
    }

    /// The preset to show after a local change that replaced its sky (the
    /// panels' time of day follows), once.
    pub fn take_preset_change(&mut self) -> Option<u8> {
        std::mem::take(&mut self.preset_dropped).then_some(self.local_preset)
    }

    /// FSEnvironmentManualTransitionTime (s, 0 = instant).
    pub fn set_manual_transition(&mut self, seconds: f32) {
        self.manual_transition = if seconds.is_finite() { seconds.clamp(0.0, 60.0) } else { 0.0 };
    }

    /// Pick a settings item from the `kind` list (selectSkyPreset /
    /// selectWaterPreset / selectDayCyclePreset → setManualEnvironment): its
    /// asset is fetched, then applied by its own type. A newer pick from the
    /// same list replaces one still loading.
    pub fn request_local(&mut self, asset: Uuid, kind: SettingsKind) {
        if self.rlva_locked {
            return;
        }
        self.photo_yield(kind);
        let mut preset_replaced = false;
        self.pending.retain(|p| {
            let replaced = p.kind == kind && matches!(p.state, PendingState::Waiting) && !p.asset.is_nil();
            preset_replaced |= replaced && p.preset != 0;
            !replaced
        });
        if preset_replaced {
            // the preset's sky will not come: the panels no longer show it
            self.drop_preset();
        }
        self.pending.push_back(Pending {
            asset,
            kind,
            state: PendingState::Waiting,
            timing: None,
            preset: 0,
        });
    }

    /// "Shared environment" (setSharedEnvironment: clearEnvironment(ENV_LOCAL)).
    pub fn clear_local(&mut self) {
        if self.rlva_locked {
            return;
        }
        self.pending.clear();
        if self.local.take().is_some() {
            self.local_dirty = true;
        }
        self.drop_preset();
    }

    pub fn has_local(&self) -> bool {
        self.local.is_some()
    }

    /// An item is still loading for the local environment.
    pub fn local_loading(&self) -> bool {
        !self.pending.is_empty()
    }

    /// What the selector's lists show.
    pub fn local_view(&self) -> LocalView {
        self.local.as_ref().map_or(LocalView::SHARED, LocalEnv::shown)
    }

    /// Assets that could not be loaded since the last call.
    pub fn take_failed(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.failed_local)
    }

    fn drain_pending(&mut self) {
        while self.pending.front().is_some_and(|p| !matches!(p.state, PendingState::Waiting)) {
            let Some(p) = self.pending.pop_front() else {
                return;
            };
            match p.state {
                PendingState::Ready(s, origin) => self.apply_settings(p.asset, &s, origin, p.timing),
                PendingState::Failed if p.preset != 0 => {
                    log::warn!(
                        "library sky {} of time of day preset {} could not be loaded: built-in sky with the preset's sun",
                        p.asset,
                        p.preset
                    );
                    let sky = SkyFrame::with_sun(super::env::time_of_day_sun(p.preset));
                    self.apply_sky(LocalOrigin::Preset(p.preset), sky);
                }
                PendingState::Failed => {
                    // onSetEnvAssetLoaded: FailedToFindSettings, nothing changes
                    log::warn!("environment settings {} could not be loaded", p.asset);
                    self.failed_local.push(p.asset);
                }
                PendingState::Waiting => {}
            }
        }
    }

    /// LLEnvironment::setEnvironment(ENV_LOCAL, settings) by the settings type.
    fn apply_settings(&mut self, asset: Uuid, settings: &Settings, origin: Option<LocalOrigin>, timing: Option<(f64, f64)>) {
        let origin = origin.unwrap_or(LocalOrigin::Item(asset));
        match settings {
            Settings::Day(d) => {
                // DayInstance::clear + setDay: the fixed sky and water go
                self.generation += 1;
                let mut l = LocalEnv::new(self.generation);
                if let Some((length, offset)) = timing {
                    l.length = if length > 0.0 { length } else { eep::DEFAULT_DAY_LENGTH };
                    l.offset = offset;
                }
                l.day = Some((asset, Arc::new(d.clone())));
                log::info!("local environment: day cycle {asset}");
                self.local = Some(l);
                self.drop_preset();
                self.local_dirty = true;
            }
            Settings::Sky(s) => self.apply_sky(origin, *s),
            Settings::Water(w) => self.apply_water(origin, *w),
        }
    }

    fn drop_preset(&mut self) {
        if self.local_preset != 0 {
            self.local_preset = 0;
            self.preset_dropped = true;
        }
    }

    /// The local environment to change, created with the frozen current
    /// sky and water (setEnvironment(fixedEnvironment_t): a half missing in a
    /// new local environment is what is shown).
    fn local_mut(&mut self) -> &mut LocalEnv {
        self.generation += 1;
        let generation = self.generation;
        let (sky, water) = self
            .shown
            .map_or((SkyFrame::default(), WaterFrame::default()), |s| (s.target_sky, s.target_water));
        let l = self.local.get_or_insert_with(|| LocalEnv::new(generation));
        l.generation = generation;
        if l.day.is_none() {
            l.sky.get_or_insert((LocalOrigin::Inherited, sky));
            l.water.get_or_insert((LocalOrigin::Inherited, water));
        }
        l
    }

    fn apply_sky(&mut self, origin: LocalOrigin, sky: SkyFrame) {
        log::info!("local environment: sky {origin:?}");
        self.local_mut().sky = Some((origin, sky));
        match origin {
            // the preset's own sky: the panels keep showing it
            LocalOrigin::Preset(n) => {
                self.local_preset = n;
                self.preset_dropped = false;
            }
            _ => self.drop_preset(),
        }
        self.local_dirty = true;
    }

    fn apply_water(&mut self, origin: LocalOrigin, water: WaterFrame) {
        log::info!("local environment: water {origin:?}");
        self.local_mut().water = Some((origin, water));
        self.local_dirty = true;
    }

    /// "Personal lighting" opens (LLFloaterEnvironmentAdjust::
    /// captureCurrentEnvironment): the sky and water shown become a fixed
    /// local environment at once (a local day is frozen; a fixed local sky
    /// is edited as it is).
    pub fn capture_personal(&mut self) {
        if self.rlva_locked {
            return;
        }
        // "Personal lighting" edits the light shown: the photo's becomes the
        // user's first (opened from the photo's Light section, or meanwhile)
        self.keep_photo();
        if self.local.as_ref().is_some_and(|l| l.day.is_none()) {
            return;
        }
        let (sky, water) = self
            .shown
            .map_or((SkyFrame::default(), WaterFrame::default()), |s| (s.target_sky, s.target_water));
        self.generation += 1;
        let mut l = self.local.take().unwrap_or_else(|| LocalEnv::new(self.generation));
        l.generation = self.generation;
        l.sky = Some((LocalOrigin::Personal, sky));
        l.water = Some((LocalOrigin::Inherited, water));
        self.local = Some(l);
        self.instant_next = true;
        self.drop_preset();
        self.local_dirty = true;
    }

    /// The local sky "Personal lighting" edits (nothing without a local
    /// fixed sky). `edit` returns true when it changed something: the sky
    /// becomes the personal one, shown at once.
    pub fn edit_personal(&mut self, edit: impl FnOnce(&mut SkyFrame) -> bool) -> bool {
        if self.rlva_locked {
            return false;
        }
        if !self.photo.choice.is_empty() {
            // the photo's light chosen while the window was open: it is
            // what the user sees, and edits
            self.keep_photo();
            if self.personal_sky().is_none() {
                self.capture_personal();
            }
        }
        let Some((origin, sky)) = self.local.as_mut().and_then(|l| l.sky.as_mut()) else {
            return false;
        };
        let mut s = *sky;
        if !edit(&mut s) {
            return false;
        }
        *sky = s;
        *origin = LocalOrigin::Personal;
        self.drop_preset();
        self.local_dirty = true;
        true
    }

    /// The local fixed sky, if any ("Personal lighting" shows it).
    pub fn personal_sky(&self) -> Option<SkyFrame> {
        self.local.as_ref().and_then(|l| l.sky.map(|s| s.1))
    }

    /// The local environment changed since the last call (to save it).
    pub fn take_local_dirty(&mut self) -> bool {
        std::mem::take(&mut self.local_dirty)
    }

    /// The local environment to keep for the next session
    /// (LLEnvironment::saveToSettings): asset ids, or the settings themselves
    /// for a sky / water without an asset; None when there is nothing to keep
    /// (the file is removed). A time of day preset is kept by the settings.
    pub fn saved_local(&self) -> Option<Llsd> {
        self.local.as_ref().and_then(saved_of)
    }

    /// What a camera preset keeps of the light shown: the photo's, as "Keep
    /// this light" would make it (`saved_local` form, time of day preset);
    /// None without a photo light.
    pub fn saved_photo(&self) -> Option<(Option<Llsd>, u8)> {
        if self.photo.choice.is_empty() {
            return None;
        }
        let (local, preset) = self.kept_photo();
        Some((local.as_ref().and_then(saved_of), preset))
    }

    /// The photo's choice, as the selector last left it.
    pub fn photo_choice(&self) -> PhotoChoice {
        self.photo.choice
    }

    /// The photo window's choice for this frame (nothing while RLVa holds
    /// the environment, @setenv: the user's own light is not theirs to
    /// change).
    pub fn set_photo(&mut self, choice: PhotoChoice) {
        if self.rlva_locked && !choice.is_empty() {
            self.photo.set(PhotoChoice::default());
            self.photo_changed = true;
            return;
        }
        self.photo.set(choice);
    }

    /// A camera preset's light replaces the photo's too.
    pub fn drop_photo(&mut self) {
        if !self.photo.choice.is_empty() {
            self.photo.set(PhotoChoice::default());
            self.photo_changed = true;
        }
    }

    /// The selector changed the photo's choice since the last call.
    pub fn take_photo_changed(&mut self) -> bool {
        std::mem::take(&mut self.photo_changed)
    }

    /// An item of the photo's is still loading.
    pub fn photo_loading(&self) -> bool {
        self.photo.loading()
    }

    /// The sky shown without the crossfade, the photo's layer included.
    pub fn shown_sky(&self) -> Option<SkyFrame> {
        self.shown.map(|s| s.photo_sky)
    }

    /// A change made elsewhere (environment window, World menu, an editor's
    /// "Apply only to myself") takes over the photo's own choice of that
    /// list: like the photo's time of day before, it is the user's.
    fn photo_yield(&mut self, kind: SettingsKind) {
        let mut c = self.photo.choice;
        match kind {
            SettingsKind::Sky => {
                c.sky = None;
                c.time = None;
            }
            SettingsKind::Water => c.water = None,
            SettingsKind::Day => c.day = None,
        }
        if c != self.photo.choice {
            self.photo.set(c);
            self.photo_changed = true;
        }
    }

    /// The local environment and its preset as "Keep this light" leaves
    /// them: the photo's loaded choices applied as the lists apply them
    /// (a day replaces the fixed halves, a sky or water is fixed on top
    /// with the other half inherited from what is shown), then its sun on a
    /// fixed sky, the day's frozen if there is none
    /// (captureCurrentEnvironment).
    fn kept_photo(&self) -> (Option<LocalEnv>, u8) {
        let r = self.photo.resolved();
        let (under_sky, under_water) = self.current();
        let mut local = self.local.clone();
        let mut preset = self.local_preset;
        if let Some((id, day)) = r.day {
            let mut l = LocalEnv::new(self.generation);
            l.day = Some((id, day));
            local = Some(l);
            preset = 0;
        }
        if let Some((origin, sky)) = r.sky {
            fixed_halves(&mut local, under_sky, under_water).sky = Some((origin, sky));
            preset = match origin {
                LocalOrigin::Preset(n) => n,
                _ => 0,
            };
        }
        if let Some((id, water)) = r.water {
            fixed_halves(&mut local, under_sky, under_water).water = Some((LocalOrigin::Item(id), water));
        }
        if let Some(sun) = self.photo.choice.sun {
            let shown = self.shown.map_or_else(SkyFrame::default, |s| s.photo_sky);
            let l = fixed_halves(&mut local, under_sky, under_water);
            match &mut l.sky {
                Some((origin, sky)) => {
                    sky.sun_rotation = sun;
                    *origin = LocalOrigin::Personal;
                }
                None => {
                    l.sky = Some((
                        LocalOrigin::Personal,
                        SkyFrame {
                            sun_rotation: sun,
                            ..shown
                        },
                    ));
                }
            }
            preset = 0;
        }
        (local, preset)
    }

    /// "Keep this light": the photo's light becomes the user's local
    /// environment, kept for the next session like any choice, and the
    /// photo's layer goes. False when there was nothing to keep.
    pub fn keep_photo(&mut self) -> bool {
        if self.rlva_locked || self.photo.choice.is_empty() {
            return false;
        }
        let (local, preset) = self.kept_photo();
        log::info!("photo light kept as the local environment (time of day {preset})");
        self.generation += 1;
        self.local = local.map(|mut l| {
            l.generation = self.generation;
            l
        });
        if preset != self.local_preset {
            // the panels' time of day follows (`take_preset_change`)
            self.local_preset = preset;
            self.preset_dropped = true;
        }
        self.pending.clear();
        self.local_dirty = true;
        // the same light, now the user's: no crossfade
        self.instant_next = true;
        self.photo.set(PhotoChoice::default());
        self.photo_changed = true;
        true
    }

    /// A local environment kept by a camera preset (`saved_local`), chosen
    /// as one from the lists: it replaces the local one and is kept for the
    /// next session like any choice.
    pub fn apply_saved(&mut self, data: &Llsd) {
        if self.rlva_locked {
            return;
        }
        self.drop_photo();
        self.clear_local();
        self.restore_local(data);
        self.local_dirty = true;
    }

    /// Restore the last session's local environment (loadFromSettings): the
    /// day first, then the fixed sky and water.
    pub fn restore_local(&mut self, data: &Llsd) {
        if data.has("day_id") && !data["day_id"].as_uuid().is_nil() {
            self.pending.push_back(Pending {
                asset: data["day_id"].as_uuid(),
                kind: SettingsKind::Day,
                state: PendingState::Waiting,
                timing: Some((data["day_length"].as_i32() as f64, data["day_offset"].as_i32() as f64)),
                preset: 0,
            });
        }
        for (kind, id_key, llsd_key) in [
            (SettingsKind::Sky, "sky_id", "sky_llsd"),
            (SettingsKind::Water, "water_id", "water_llsd"),
        ] {
            if data.has(id_key) && !data[id_key].as_uuid().is_nil() {
                self.pending.push_back(Pending {
                    asset: data[id_key].as_uuid(),
                    kind,
                    state: PendingState::Waiting,
                    timing: None,
                    preset: 0,
                });
            } else if data.has(llsd_key)
                && let Some(s) = Settings::from_llsd(&data[llsd_key])
            {
                let origin = if kind == SettingsKind::Sky && data["sky_personal"].as_bool() {
                    LocalOrigin::Personal
                } else {
                    LocalOrigin::Inherited
                };
                self.pending.push_back(Pending {
                    asset: Uuid::nil(),
                    kind,
                    state: PendingState::Ready(Arc::new(s), Some(origin)),
                    timing: None,
                    preset: 0,
                });
            }
        }
        self.drain_pending();
        // restoring is not a change to save again
        self.local_dirty = false;
    }

    /// The selected shared slot (getSelectedEnvironmentInstance below ENV_LOCAL).
    fn selected(&self, input: &FrameInput) -> (Source, Option<&Slot>) {
        if input.eep_region == Some(false) {
            return (Source::Legacy, None);
        }
        if let Some(p) = self.parcel.as_ref().filter(|p| Some(p.handle) == input.main_region) {
            return (Source::Parcel(p.generation), Some(p));
        }
        if let Some(r) = &self.region {
            return (Source::Region(r.generation), Some(r));
        }
        if input.eep_region == Some(true) || self.default_pending.is_some() {
            return (Source::BuiltIn, None);
        }
        (Source::Legacy, None)
    }

    /// Source of the environment shown last (tests, sky log).
    pub fn shown_source(&self) -> Option<Source> {
        self.shown.map(|s| s.source)
    }

    /// Sky and water for this frame, crossfading when the source or the
    /// sky track changes.
    pub fn frame(&mut self, input: &FrameInput) -> (SkyFrame, WaterFrame) {
        let (source, track, sky, water) = if let Some((sky, water)) = self.edit {
            (Source::Edit, 0, sky, water)
        } else if let Some(l) = &self.local {
            let (track, sky, water) = l.frames(&self.altitudes, input);
            (Source::Local(l.generation), track, sky, water)
        } else {
            let (source, slot) = self.selected(input);
            let pushed_day = self.push.day();
            let (track, sky, water) = match slot {
                Some(s) => {
                    let pos = s.position(input.unix_secs);
                    let day = pushed_day.unwrap_or(&s.day);
                    let track = day.track_for_altitude(&self.altitudes, input.altitude);
                    (track, day.sky_at(pos, track), day.water_at(pos))
                }
                None if source == Source::Legacy => (0, SkyFrame::with_sun(input.legacy_sun), WaterFrame::default()),
                None => (0, SkyFrame::default(), WaterFrame::default()),
            };
            if self.push.is_active() {
                let (sky, water) = self.push.apply(sky, water);
                (Source::Push(self.push.generation()), track, sky, water)
            } else {
                (source, track, sky, water)
            }
        };
        // the photo's layer, under an open editor only (`eep_photo`); the
        // targets stay those beneath it, for what a local change inherits
        let (target_sky, target_water) = (sky, water);
        let (source, track, sky, water) = if source == Source::Edit || self.photo.choice.is_empty() {
            (source, track, sky, water)
        } else {
            let r = self.photo.resolve();
            let (t, s, w) = r.compose(self.photo.choice.sun, &self.altitudes, input, (track, sky, water));
            let source = if r.any() { Source::Photo(self.photo.generation) } else { source };
            (source, t, s, w)
        };
        if let Some(prev) = self.shown
            && (prev.source != source || prev.track != track)
        {
            // updateEnvironment: from what is shown (itself possibly mid-fade).
            // Manual local changes use FSEnvironmentManualTransitionTime
            // (Firestorm only fades into a new local environment and changes
            // a live one at once: Aurora fades both, identical with the
            // default 0 s); going back to the shared environment fades over
            // TRANSITION_DEFAULT (setSelectedEnvironment's default).
            let duration = if prev.source == source {
                TRANSITION_ALTITUDE
            } else if source == Source::Edit {
                // an editor shows its settings at once (TRANSITION_INSTANT)
                0.0
            } else if prev.source == Source::Edit {
                // onClose: setSelectedEnvironment(ENV_LOCAL), default transition
                TRANSITION_DEFAULT
            } else if matches!(source, Source::Local(_) | Source::Photo(_)) {
                // (the photo's choices are manual changes too)
                if std::mem::take(&mut self.instant_next) {
                    0.0
                } else {
                    self.manual_transition
                }
            } else if matches!(prev.source, Source::Local(_) | Source::Photo(_)) {
                TRANSITION_DEFAULT
            } else if matches!(source, Source::Push(_)) || matches!(prev.source, Source::Push(_)) {
                self.push.transition()
            } else {
                self.next_fade
            };
            log::info!(
                "environment: {:?} track {} -> {source:?} track {track} ({duration} s crossfade)",
                prev.source,
                prev.track
            );
            self.fade = (duration > 0.0).then_some(Fade {
                sky: prev.sky,
                water: prev.water,
                start: input.now,
                duration,
            });
        }
        let photo_sky = sky;
        let (sky, water) = match self.fade {
            Some(f) => {
                let t = ((input.now - f.start) as f32 / f.duration).clamp(0.0, 1.0);
                if t >= 1.0 {
                    self.fade = None;
                    (sky, water)
                } else {
                    (f.sky.lerp(&sky, t), f.water.lerp(&water, t))
                }
            }
            None => (sky, water),
        };
        self.shown = Some(Shown {
            source,
            track,
            sky,
            water,
            target_sky,
            target_water,
            photo_sky,
        });
        (sky, water)
    }
}

/// The local environment a fixed half goes into (`EnvSelector::local_mut`
/// without its generation): created with the frozen sky and water shown
/// when it has neither a day nor its own.
fn fixed_halves(local: &mut Option<LocalEnv>, sky: SkyFrame, water: WaterFrame) -> &mut LocalEnv {
    let l = local.get_or_insert_with(|| LocalEnv::new(0));
    if l.day.is_none() {
        l.sky.get_or_insert((LocalOrigin::Inherited, sky));
        l.water.get_or_insert((LocalOrigin::Inherited, water));
    }
    l
}

/// The saved form of a local environment (`EnvSelector::saved_local`).
fn saved_of(l: &LocalEnv) -> Option<Llsd> {
    let mut m = aurora_llsd::Map::new();
    if let Some((id, _)) = &l.day {
        m.insert("day_id".into(), Llsd::from(*id));
        m.insert("day_length".into(), Llsd::from(l.length as i32));
        m.insert("day_offset".into(), Llsd::from(l.offset as i32));
    }
    match l.sky {
        Some((LocalOrigin::Item(id), _)) => {
            m.insert("sky_id".into(), Llsd::from(id));
        }
        Some((o @ (LocalOrigin::Personal | LocalOrigin::Inherited), s)) => {
            m.insert("sky_llsd".into(), s.settings_llsd());
            m.insert("sky_personal".into(), Llsd::from(o == LocalOrigin::Personal));
        }
        Some((LocalOrigin::Preset(_), _)) | None => {}
    }
    match l.water {
        Some((LocalOrigin::Item(id), _)) => {
            m.insert("water_id".into(), Llsd::from(id));
        }
        Some((_, w)) => {
            m.insert("water_llsd".into(), w.settings_llsd());
        }
        None => {}
    }
    (!m.is_empty()).then_some(Llsd::Map(m))
}

impl super::World {
    /// Sky and water to render this frame (`time_of_day`: the local preset,
    /// 0 for the shared environment; reset when another local choice
    /// replaced the preset's sky).
    pub fn environment_frames(&mut self, time_of_day: &mut u8) -> (SkyFrame, WaterFrame) {
        let main = self.main_region.and_then(|h| self.regions.get(&h));
        let eep_region = main.filter(|r| !r.caps.is_empty()).map(|r| r.caps.contains_key("ExtEnvironment"));
        let input = FrameInput {
            main_region: self.main_region,
            eep_region,
            legacy_sun: self.sun.map(|(s, _)| s.sun_direction).unwrap_or_else(super::env::midday_sun),
            unix_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            now: self.env_clock.elapsed().as_secs_f64(),
            altitude: self.agent.position.z,
        };
        if let Some(p) = self.eep.take_preset_change() {
            *time_of_day = p;
        }
        self.eep.set_time_of_day(*time_of_day);
        let (sky, water) = self.eep.frame(&input);
        self.rlva_environment_frames((sky, water))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::{Llsd, llsd_map};

    const REGION: RegionHandle = 7;

    /// A day whose only sky has the given gamma (an easy marker).
    fn answer(parcel_id: i32, gamma: Option<f64>) -> EnvAnswer {
        let mut env = llsd_map! {
            "parcel_id" => parcel_id,
            "day_length" => 14400,
            "day_offset" => 0,
            "track_altitudes" => Llsd::Array(vec![1000.0.into(), 2000.0.into(), 3000.0.into()]),
        };
        if let Some(g) = gamma {
            let mut frames = aurora_llsd::Map::new();
            frames.insert("s".into(), llsd_map! { "type" => "sky", "gamma" => g });
            frames.insert("w".into(), llsd_map! { "type" => "water" });
            env.insert(
                "day_cycle",
                llsd_map! {
                    "frames" => Llsd::Map(frames),
                    "tracks" => Llsd::Array(vec![
                        Llsd::Array(vec![llsd_map!{"key_keyframe" => 0.0, "key_name" => "w"}]),
                        Llsd::Array(vec![llsd_map!{"key_keyframe" => 0.0, "key_name" => "s"}]),
                    ]),
                },
            );
        }
        EnvAnswer::from_llsd(&env)
    }

    fn ctx(parcel: Option<i32>) -> AnswerContext {
        AnswerContext {
            main_region: Some(REGION),
            agent_parcel: parcel,
            teleporting: false,
        }
    }

    fn input(now: f64) -> FrameInput {
        FrameInput {
            main_region: Some(REGION),
            eep_region: Some(true),
            legacy_sun: Vec3::Z,
            unix_secs: 1000.0,
            now,
            altitude: 25.0,
        }
    }

    fn gamma(sel: &mut EnvSelector, now: f64) -> f32 {
        sel.frame(&input(now)).0.gamma
    }

    #[test]
    fn an_experience_push_overrides_the_parcel_fades_and_clears() {
        let mut sel = EnvSelector::default();
        sel.on_answer(REGION, -1, answer(-1, Some(0.7)), ctx(None));
        gamma(&mut sel, 0.0);
        let experience = Uuid::from_u128(9);
        sel.push_experience(
            experience,
            &llsd_map! {
                "action" => "PushPartialEnvironment",
                "action_data" => llsd_map! {
                    "transition_time" => 2.0,
                    "settings" => llsd_map! { "sky" => llsd_map! { "gamma" => 3.0 } },
                },
            },
        );
        assert!((gamma(&mut sel, 1.0) - 0.7).abs() < 1e-6, "the fade starts from the region sky");
        assert!((gamma(&mut sel, 3.1) - 3.0).abs() < 1e-6);
        assert!(matches!(sel.shown_source(), Some(Source::Push(_))));
        assert_eq!(sel.pushing_experiences(), vec![experience]);
        // Forget: back to the shared environment over the fast transition
        sel.clear_experience(experience, 1.0);
        gamma(&mut sel, 4.0);
        assert!((gamma(&mut sel, 5.1) - 0.7).abs() < 1e-6);
        assert!(matches!(sel.shown_source(), Some(Source::Region(_))));
    }

    #[test]
    fn parcel_without_day_keeps_the_region_environment() {
        // the Yikes log: region day, then the parcel answer without one
        let mut sel = EnvSelector::default();
        assert_eq!(
            sel.on_answer(REGION, -1, answer(-1, Some(0.7)), ctx(None)),
            AnswerOutcome::RegionDay
        );
        assert!((gamma(&mut sel, 0.0) - 0.7).abs() < 1e-6);
        assert_eq!(
            sel.on_answer(REGION, 5, answer(5, None), ctx(Some(5))),
            AnswerOutcome::ParcelCleared
        );
        assert!((gamma(&mut sel, 0.4) - 0.7).abs() < 1e-6);
        assert!(matches!(sel.shown_source(), Some(Source::Region(_))));
        assert!(sel.wanted_settings().is_empty());
    }

    #[test]
    fn parcel_day_overrides_the_region_then_falls_back() {
        let mut sel = EnvSelector::default();
        sel.on_answer(REGION, -1, answer(-1, Some(0.7)), ctx(None));
        gamma(&mut sel, 0.0);
        assert_eq!(
            sel.on_answer(REGION, 5, answer(5, Some(1.9)), ctx(Some(5))),
            AnswerOutcome::ParcelDay
        );
        // crossfades over TRANSITION_DEFAULT, then shows the parcel's day
        let g = gamma(&mut sel, 10.0);
        assert!((g - 0.7).abs() < 1e-6, "fade starts from the region sky: {g}");
        assert!((gamma(&mut sel, 15.1) - 1.9).abs() < 1e-6);
        // the region answers again (RegionInfo): the parcel's day stays
        assert_eq!(
            sel.on_answer(REGION, -1, answer(-1, Some(0.8)), ctx(Some(5))),
            AnswerOutcome::RegionDay
        );
        assert!((gamma(&mut sel, 16.0) - 1.9).abs() < 1e-6);
        // another parcel without a day: back to the region
        sel.on_answer(REGION, 6, answer(6, None), ctx(Some(6)));
        gamma(&mut sel, 20.0);
        assert!((gamma(&mut sel, 25.1) - 0.8).abs() < 1e-6);
        // the parcel slot never applies in another region
        sel.on_answer(REGION, 6, answer(6, Some(1.9)), ctx(Some(6)));
        let other = FrameInput {
            main_region: Some(REGION + 1),
            ..input(40.0)
        };
        sel.frame(&other);
        assert!(matches!(sel.shown_source(), Some(Source::Region(_))));
    }

    #[test]
    fn region_without_valid_day_uses_the_default_day() {
        let mut sel = EnvSelector::default();
        assert_eq!(sel.on_answer(REGION, -1, answer(-1, None), ctx(None)), AnswerOutcome::RegionDefault);
        assert_eq!(sel.wanted_settings(), vec![eep::DEFAULT_DAY_ASSET]);
        // built-in defaults while the asset loads, never the legacy sun
        let (sky, _) = sel.frame(&input(0.0));
        assert_eq!(sky.sun_rotation, SkyFrame::default().sun_rotation);
        assert_eq!(sel.shown_source(), Some(Source::BuiltIn));
        let asset = Settings::Day(DayCycle::fixed(
            SkyFrame {
                gamma: 1.3,
                ..Default::default()
            },
            WaterFrame::default(),
        ));
        sel.on_settings(eep::DEFAULT_DAY_ASSET, Some(Arc::new(asset)));
        assert!(sel.wanted_settings().is_empty());
        gamma(&mut sel, 1.0);
        assert!((gamma(&mut sel, 6.1) - 1.3).abs() < 1e-6);
        assert!(matches!(sel.shown_source(), Some(Source::Region(_))));
        let r = sel.region.as_ref().expect("region slot");
        assert_eq!((r.length, r.offset), (eep::DEFAULT_DAY_LENGTH, eep::DEFAULT_DAY_OFFSET));
        // a region day with an empty water track is invalid too
        let mut a = answer(-1, Some(0.7));
        if let Some(d) = a.day.as_mut() {
            d.water.clear();
        }
        assert_eq!(sel.on_answer(REGION, -1, a, ctx(None)), AnswerOutcome::RegionDefault);
    }

    #[test]
    fn stale_answers_are_ignored() {
        let mut sel = EnvSelector::default();
        sel.on_answer(REGION, -1, answer(-1, Some(0.7)), ctx(None));
        // the agent already left parcel 5 for parcel 6
        assert_eq!(
            sel.on_answer(REGION, 5, answer(5, Some(1.9)), ctx(Some(6))),
            AnswerOutcome::StaleParcel
        );
        // an answer for the region the agent left
        assert_eq!(
            sel.on_answer(REGION + 1, -1, answer(-1, None), ctx(Some(6))),
            AnswerOutcome::StaleRegion
        );
        assert!((gamma(&mut sel, 0.0) - 0.7).abs() < 1e-6);
        assert!(sel.wanted_settings().is_empty());
    }

    #[test]
    fn crossfade_interpolates_and_is_fast_when_teleporting() {
        let mut sel = EnvSelector::default();
        sel.on_answer(REGION, -1, answer(-1, Some(1.0)), ctx(None));
        gamma(&mut sel, 0.0);
        sel.on_answer(REGION, -1, answer(-1, Some(2.0)), ctx(None));
        assert!((gamma(&mut sel, 10.0) - 1.0).abs() < 1e-6);
        assert!((gamma(&mut sel, 12.5) - 1.5).abs() < 1e-5, "halfway through 5 s");
        assert!((gamma(&mut sel, 15.0) - 2.0).abs() < 1e-6);
        let tp = AnswerContext {
            teleporting: true,
            ..ctx(None)
        };
        sel.on_answer(REGION, -1, answer(-1, Some(3.0)), tp);
        gamma(&mut sel, 20.0);
        assert!((gamma(&mut sel, 20.5) - 2.5).abs() < 1e-5, "halfway through 1 s");
        assert!((gamma(&mut sel, 21.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn altitude_track_switch_fades() {
        let mut sel = EnvSelector::default();
        let mut a = answer(-1, Some(1.0));
        if let Some(d) = a.day.as_mut() {
            let high = SkyFrame {
                gamma: 2.0,
                ..Default::default()
            };
            d.sky_tracks[1] = vec![(0.0, high)];
        }
        sel.on_answer(REGION, -1, a, ctx(None));
        assert!((gamma(&mut sel, 0.0) - 1.0).abs() < 1e-6);
        let high = |now| FrameInput {
            altitude: 1500.0,
            ..input(now)
        };
        assert!((sel.frame(&high(1.0)).0.gamma - 1.0).abs() < 1e-6);
        assert!((sel.frame(&high(3.5)).0.gamma - 1.5).abs() < 1e-5);
        assert!((sel.frame(&high(6.0)).0.gamma - 2.0).abs() < 1e-6);
    }

    #[test]
    fn an_editor_shows_its_settings_at_once_and_fades_back_on_close() {
        let mut sel = EnvSelector::default();
        sel.frame(&input(0.0));
        let base = sel.current();
        let edited = SkyFrame { gamma: 3.0, ..base.0 };
        sel.set_edit_sky(edited);
        assert!(sel.editing());
        assert!((gamma(&mut sel, 1.0) - 3.0).abs() < 1e-6, "instant");
        assert_eq!(sel.shown_source(), Some(Source::Edit));
        // a water edit keeps the edited sky
        sel.set_edit_water(WaterFrame {
            fog_density: 7.0,
            ..base.1
        });
        let (sky, water) = sel.frame(&input(1.5));
        assert!((sky.gamma - 3.0).abs() < 1e-6 && water.fog_density == 7.0);
        // applying it to myself changes what lies beneath, not what is shown
        sel.apply_local_sky(edited);
        assert!((gamma(&mut sel, 2.0) - 3.0).abs() < 1e-6);
        sel.clear_edit();
        assert!(!sel.editing());
        assert!((gamma(&mut sel, 3.0) - 3.0).abs() < 1e-6, "the local sky is the edited one");
        assert!(matches!(sel.shown_source(), Some(Source::Local(_))));
        assert_eq!(sel.local_view().sky, ListShown::Personal);
    }

    #[test]
    fn legacy_region_and_local_presets() {
        let mut sel = EnvSelector::default();
        let legacy = FrameInput {
            eep_region: Some(false),
            ..input(0.0)
        };
        let (sky, _) = sel.frame(&legacy);
        assert!(sky.sun_direction().z > 0.99);
        assert_eq!(sel.shown_source(), Some(Source::Legacy));
        // capabilities not there yet and no answer: legacy too
        sel.frame(&FrameInput {
            eep_region: None,
            ..input(1.0)
        });
        assert_eq!(sel.shown_source(), Some(Source::Legacy));
        sel.set_time_of_day(4);
        library_sky_arrives(&mut sel, 4);
        sel.frame(&input(2.0));
        assert!(matches!(sel.shown_source(), Some(Source::Local(_))));
        let (sky, _) = sel.frame(&input(7.1));
        assert!(sky.sun_direction().z < 0.0, "midnight preset");
    }

    /// The library sky of a time of day preset arrives (here the built-in
    /// sky with the preset's sun).
    fn library_sky_arrives(sel: &mut EnvSelector, preset: u8) {
        let id = super::super::env::time_of_day_sky(preset).expect("a preset");
        assert_eq!(sel.wanted_settings(), vec![id]);
        let sky = SkyFrame::with_sun(super::super::env::time_of_day_sun(preset));
        sel.on_settings(id, Some(Arc::new(Settings::Sky(sky))));
    }

    const SKY: Uuid = Uuid::from_u128(0x51);
    const WATER: Uuid = Uuid::from_u128(0x52);
    const DAY: Uuid = Uuid::from_u128(0x53);

    fn sky_asset(gamma: f32) -> Arc<Settings> {
        Arc::new(Settings::Sky(SkyFrame {
            gamma,
            ..Default::default()
        }))
    }

    fn water_asset(fog: f32) -> Arc<Settings> {
        Arc::new(Settings::Water(WaterFrame {
            fog_density: fog,
            ..Default::default()
        }))
    }

    fn day_asset(gamma: f32, fog: f32) -> Arc<Settings> {
        Arc::new(Settings::Day(DayCycle::fixed(
            SkyFrame {
                gamma,
                ..Default::default()
            },
            WaterFrame {
                fog_density: fog,
                ..Default::default()
            },
        )))
    }

    /// A selector showing a region day (gamma 0.7, default water).
    fn region_selector() -> EnvSelector {
        let mut sel = EnvSelector::default();
        sel.on_answer(REGION, -1, answer(-1, Some(0.7)), ctx(None));
        sel.frame(&input(0.0));
        sel
    }

    #[test]
    fn local_sky_and_water_compose_over_the_shared_environment() {
        let mut sel = region_selector();
        sel.request_local(SKY, SettingsKind::Sky);
        assert_eq!(sel.wanted_settings(), vec![SKY]);
        assert!(sel.local_loading());
        sel.on_settings(SKY, Some(sky_asset(1.9)));
        assert!(sel.wanted_settings().is_empty());
        // instant by default (FSEnvironmentManualTransitionTime 0)
        let (sky, water) = sel.frame(&input(1.0));
        assert!((sky.gamma - 1.9).abs() < 1e-6);
        assert!(
            (water.fog_density - WaterFrame::default().fog_density).abs() < 1e-6,
            "water inherited"
        );
        assert!(matches!(sel.shown_source(), Some(Source::Local(_))));
        assert_eq!(
            sel.local_view(),
            LocalView {
                sky: ListShown::Item(SKY),
                water: ListShown::Shared,
                day: ListShown::DayBased,
            }
        );
        // then a water on top: the sky stays
        sel.request_local(WATER, SettingsKind::Water);
        sel.on_settings(WATER, Some(water_asset(9.0)));
        let (sky, water) = sel.frame(&input(2.0));
        assert!((sky.gamma - 1.9).abs() < 1e-6);
        assert!((water.fog_density - 9.0).abs() < 1e-6);
        // the region changes underneath: the local environment stays
        sel.on_answer(REGION, -1, answer(-1, Some(0.3)), ctx(None));
        let other = FrameInput {
            main_region: Some(REGION + 1),
            ..input(3.0)
        };
        assert!((sel.frame(&other).0.gamma - 1.9).abs() < 1e-6, "kept across regions");
        // shared environment: back to the region over TRANSITION_DEFAULT
        sel.clear_local();
        assert!(!sel.has_local());
        assert_eq!(sel.local_view(), LocalView::SHARED);
        assert!((gamma(&mut sel, 10.0) - 1.9).abs() < 1e-6);
        assert!((gamma(&mut sel, 12.5) - 1.1).abs() < 1e-5, "halfway through 5 s");
        assert!((gamma(&mut sel, 15.0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn local_day_replaces_fixed_halves_and_a_sky_overrides_it() {
        let mut sel = region_selector();
        sel.request_local(SKY, SettingsKind::Sky);
        sel.on_settings(SKY, Some(sky_asset(1.9)));
        sel.request_local(DAY, SettingsKind::Day);
        sel.on_settings(DAY, Some(day_asset(1.2, 4.0)));
        let (sky, water) = sel.frame(&input(1.0));
        assert!((sky.gamma - 1.2).abs() < 1e-6, "the day clears the fixed sky");
        assert!((water.fog_density - 4.0).abs() < 1e-6);
        assert_eq!(
            sel.local_view(),
            LocalView {
                sky: ListShown::DayBased,
                water: ListShown::DayBased,
                day: ListShown::Item(DAY),
            }
        );
        // a fixed sky over the day: the water keeps animating from the day
        sel.request_local(SKY, SettingsKind::Sky);
        sel.on_settings(SKY, Some(sky_asset(2.2)));
        let (sky, water) = sel.frame(&input(2.0));
        assert!((sky.gamma - 2.2).abs() < 1e-6);
        assert!((water.fog_density - 4.0).abs() < 1e-6);
        assert_eq!(sel.local_view().sky, ListShown::Item(SKY));
        assert_eq!(sel.local_view().water, ListShown::DayBased);
    }

    #[test]
    fn picks_apply_in_order_and_failures_change_nothing() {
        let mut sel = region_selector();
        // a day picked before a sky: the sky applies over the day even if it
        // loads first
        sel.request_local(DAY, SettingsKind::Day);
        sel.request_local(SKY, SettingsKind::Sky);
        sel.on_settings(SKY, Some(sky_asset(2.0)));
        assert!(!sel.has_local(), "waits for the day");
        sel.on_settings(DAY, Some(day_asset(1.0, 3.0)));
        assert!((sel.frame(&input(1.0)).0.gamma - 2.0).abs() < 1e-6);
        // a newer pick from the same list replaces one still loading
        let other = Uuid::from_u128(0x99);
        sel.request_local(other, SettingsKind::Water);
        sel.request_local(WATER, SettingsKind::Water);
        assert_eq!(sel.wanted_settings(), vec![WATER]);
        // FailedToFindSettings: nothing changes
        sel.on_settings(WATER, None);
        assert_eq!(sel.take_failed(), vec![WATER]);
        assert!(!sel.local_loading());
        assert!((sel.frame(&input(2.0)).1.fog_density - 3.0).abs() < 1e-6);
    }

    #[test]
    fn manual_transition_time_fades_local_changes() {
        let mut sel = region_selector();
        sel.set_manual_transition(2.0);
        sel.request_local(SKY, SettingsKind::Sky);
        sel.on_settings(SKY, Some(sky_asset(1.7)));
        assert!((gamma(&mut sel, 1.0) - 0.7).abs() < 1e-6);
        assert!((gamma(&mut sel, 2.0) - 1.2).abs() < 1e-5, "halfway through 2 s");
        assert!((gamma(&mut sel, 3.0) - 1.7).abs() < 1e-6);
    }

    #[test]
    fn presets_and_selector_share_the_local_sky() {
        let mut sel = region_selector();
        sel.set_time_of_day(2);
        library_sky_arrives(&mut sel, 2);
        sel.frame(&input(1.0));
        assert_eq!(sel.local_view().sky, ListShown::Preset(2));
        assert_eq!(sel.take_preset_change(), None);
        // a water keeps the preset sky
        sel.request_local(WATER, SettingsKind::Water);
        sel.on_settings(WATER, Some(water_asset(5.0)));
        assert_eq!(sel.take_preset_change(), None);
        assert_eq!(sel.local_view().sky, ListShown::Preset(2));
        // a sky item replaces it: the panels' time of day goes back to 0
        sel.request_local(SKY, SettingsKind::Sky);
        sel.on_settings(SKY, Some(sky_asset(1.4)));
        assert_eq!(sel.take_preset_change(), Some(0));
        sel.set_time_of_day(0);
        assert!(sel.has_local(), "dropping the preset is not « shared »");
        assert!((sel.frame(&input(2.0)).1.fog_density - 5.0).abs() < 1e-6);
        // "Use shared environments"
        sel.set_time_of_day(3);
        sel.set_time_of_day(0);
        assert!(!sel.has_local());
    }

    #[test]
    fn each_preset_has_the_legacy_viewers_library_sky() {
        use super::super::env::{TIME_OF_DAY_ORDER, TIME_OF_DAY_PRESETS, time_of_day_sky};
        let ids: Vec<Uuid> = (1..=TIME_OF_DAY_PRESETS).filter_map(time_of_day_sky).collect();
        assert_eq!(ids.len(), 5);
        assert!(ids.iter().enumerate().all(|(i, a)| !ids[i + 1..].contains(a)), "one sky each");
        assert_eq!(time_of_day_sky(0), None);
        assert_eq!(time_of_day_sky(6), None);
        // LLEnvironment::KNOWN_SKY_MIDDAY and KNOWN_SKY_LEGACY_MIDDAY
        assert_eq!(
            time_of_day_sky(2).map(|u| u.to_string()).as_deref(),
            Some("c46226b4-0e43-5a56-9708-d27ca1df3292")
        );
        assert_eq!(
            time_of_day_sky(5).map(|u| u.to_string()).as_deref(),
            Some("6c83e853-e7f8-cad7-8ee6-5f31c453721c")
        );
        let mut order = TIME_OF_DAY_ORDER;
        order.sort_unstable();
        assert_eq!(order, [1, 2, 3, 4, 5], "the menu shows every preset once");
    }

    #[test]
    fn a_preset_shows_its_library_sky_once_loaded() {
        let mut sel = region_selector();
        sel.set_time_of_day(2);
        // the sky shown stays while the asset loads, the preset is already
        // the one picked
        assert!(sel.local_loading());
        assert!((gamma(&mut sel, 1.0) - 0.7).abs() < 1e-6);
        assert_eq!(sel.take_preset_change(), None);
        // another preset picked meanwhile replaces it
        sel.set_time_of_day(5);
        let legacy = super::super::env::time_of_day_sky(5).expect("legacy midday");
        assert_eq!(sel.wanted_settings(), vec![legacy]);
        sel.on_settings(legacy, Some(sky_asset(1.6)));
        assert!((gamma(&mut sel, 2.0) - 1.6).abs() < 1e-6);
        assert_eq!(sel.local_view().sky, ListShown::Preset(5));
        assert_eq!(sel.take_preset_change(), None);
    }

    #[test]
    fn a_preset_whose_sky_cannot_be_loaded_shows_the_built_in_sky() {
        let mut sel = region_selector();
        sel.set_time_of_day(4);
        let id = super::super::env::time_of_day_sky(4).expect("midnight");
        sel.on_settings(id, None);
        assert!(sel.take_failed().is_empty(), "no failure to report: the preset still shows");
        let (sky, _) = sel.frame(&input(1.0));
        assert!(sky.sun_direction().z < 0.0, "midnight sun");
        assert_eq!(sel.local_view().sky, ListShown::Preset(4));
        assert_eq!(sel.take_preset_change(), None);
    }

    #[test]
    fn a_sky_item_picked_while_a_preset_loads_replaces_it() {
        let mut sel = region_selector();
        sel.set_time_of_day(1);
        sel.request_local(SKY, SettingsKind::Sky);
        assert_eq!(sel.wanted_settings(), vec![SKY]);
        assert_eq!(sel.take_preset_change(), Some(0), "the panels no longer show the preset");
        sel.set_time_of_day(0);
        assert!(sel.local_loading(), "dropping the preset keeps the item loading");
        sel.on_settings(SKY, Some(sky_asset(1.3)));
        assert!((gamma(&mut sel, 1.0) - 1.3).abs() < 1e-6);
        assert_eq!(sel.local_view().sky, ListShown::Item(SKY));
    }

    #[test]
    fn personal_lighting_freezes_then_edits_the_sky() {
        let mut sel = region_selector();
        sel.capture_personal();
        assert!(sel.has_local());
        // at once, from the region's sky
        assert!((gamma(&mut sel, 1.0) - 0.7).abs() < 1e-6);
        assert_eq!(sel.local_view().sky, ListShown::Personal);
        assert!(sel.edit_personal(|s| {
            s.gamma = 2.5;
            true
        }));
        assert!((gamma(&mut sel, 1.1) - 2.5).abs() < 1e-6, "edits show at once");
        assert!(!sel.edit_personal(|_| false));
        // a local day is frozen: the personal sky replaces its animation
        sel.request_local(DAY, SettingsKind::Day);
        sel.on_settings(DAY, Some(day_asset(1.3, 6.0)));
        sel.frame(&input(2.0));
        sel.capture_personal();
        let (sky, water) = sel.frame(&input(3.0));
        assert!((sky.gamma - 1.3).abs() < 1e-6);
        assert!((water.fog_density - 6.0).abs() < 1e-6);
        assert_eq!(sel.personal_sky().map(|s| s.gamma), Some(1.3));
    }

    #[test]
    fn local_environment_is_saved_and_restored_like_firestorm() {
        let mut sel = region_selector();
        assert!(sel.saved_local().is_none());
        sel.request_local(DAY, SettingsKind::Day);
        sel.on_settings(DAY, Some(day_asset(1.0, 2.0)));
        sel.request_local(WATER, SettingsKind::Water);
        sel.on_settings(WATER, Some(water_asset(8.0)));
        sel.frame(&input(1.0));
        sel.edit_personal(|_| true); // no fixed sky over the day: nothing
        assert!(sel.take_local_dirty());
        let saved = sel.saved_local().expect("something to keep");
        assert_eq!(saved["day_id"].as_uuid(), DAY);
        assert_eq!(saved["water_id"].as_uuid(), WATER);
        assert!(!saved.has("sky_id") && !saved.has("sky_llsd"));
        // next session: the day loads after the water, still applied first
        let mut next = region_selector();
        next.restore_local(&saved);
        assert!(!next.take_local_dirty());
        let mut wanted = next.wanted_settings();
        wanted.sort();
        assert_eq!(wanted, vec![WATER, DAY]);
        next.on_settings(WATER, Some(water_asset(8.0)));
        next.on_settings(DAY, Some(day_asset(1.0, 2.0)));
        let (sky, water) = next.frame(&input(2.0));
        assert!((sky.gamma - 1.0).abs() < 1e-6);
        assert!((water.fog_density - 8.0).abs() < 1e-6);
        assert_eq!(next.local_view(), sel.local_view());
        // a personal sky is kept as settings
        let mut sel = region_selector();
        sel.capture_personal();
        sel.edit_personal(|s| {
            s.gamma = 3.0;
            true
        });
        let saved = sel.saved_local().expect("personal sky");
        let mut next = region_selector();
        next.restore_local(&saved);
        assert!(next.wanted_settings().is_empty());
        assert!((next.frame(&input(1.0)).0.gamma - 3.0).abs() < 1e-6);
        assert_eq!(next.local_view().sky, ListShown::Personal);
        // shared: nothing to keep
        next.clear_local();
        assert!(next.saved_local().is_none());
    }

    #[test]
    fn a_camera_preset_light_replaces_the_local_environment_and_is_kept() {
        let mut sel = region_selector();
        sel.capture_personal();
        sel.edit_personal(|s| {
            s.gamma = 2.5;
            true
        });
        let preset = sel.saved_local().expect("personal sky");
        // later, a water was chosen: the preset's light comes back whole
        let mut now = region_selector();
        now.request_local(WATER, SettingsKind::Water);
        now.on_settings(WATER, Some(water_asset(8.0)));
        now.take_local_dirty();
        now.apply_saved(&preset);
        assert!(now.take_local_dirty(), "a choice: kept for the next session");
        assert!((now.frame(&input(1.0)).0.gamma - 2.5).abs() < 1e-6);
        assert_eq!(now.local_view().sky, ListShown::Personal);
        assert_ne!(now.local_view().water, ListShown::Item(WATER));
    }

    fn photo(sky: Option<Uuid>, water: Option<Uuid>) -> PhotoChoice {
        PhotoChoice {
            sky,
            water,
            ..PhotoChoice::default()
        }
    }

    #[test]
    fn the_photos_light_lies_over_the_users_and_leaves_it_alone() {
        let mut sel = region_selector();
        sel.request_local(WATER, SettingsKind::Water);
        sel.on_settings(WATER, Some(water_asset(9.0)));
        sel.frame(&input(1.0));
        sel.take_local_dirty();
        let saved = sel.saved_local();
        // the photo's sky and sun: the sky shows once loaded
        let sun = crate::ui::environment::rotation_from(45.0, 10.0);
        sel.set_photo(PhotoChoice {
            sun: Some(sun),
            ..photo(Some(SKY), None)
        });
        assert_eq!(sel.wanted_settings(), vec![SKY]);
        assert!(sel.photo_loading());
        let (sky, _) = sel.frame(&input(2.0));
        assert!((sky.gamma - 0.7).abs() < 1e-6, "the region's sky until the asset arrives");
        assert!(sky.sun_rotation.abs_diff_eq(sun, 1e-6), "the sun turns at once");
        sel.on_settings(SKY, Some(sky_asset(1.9)));
        let (sky, water) = sel.frame(&input(3.0));
        assert!((sky.gamma - 1.9).abs() < 1e-6);
        assert!((water.fog_density - 9.0).abs() < 1e-6, "the user's water beneath");
        assert!(matches!(sel.shown_source(), Some(Source::Photo(_))));
        // the user's environment is neither changed nor to save
        assert!(!sel.take_local_dirty());
        assert_eq!(sel.saved_local(), saved);
        assert_eq!(sel.local_view().sky, ListShown::Shared);
        // the window closes: the user's own again
        sel.set_photo(PhotoChoice::default());
        let (sky, water) = sel.frame(&input(4.0));
        assert!((sky.gamma - 0.7).abs() < 1e-6);
        assert!((water.fog_density - 9.0).abs() < 1e-6);
        assert!(!sel.take_local_dirty());
    }

    #[test]
    fn keeping_the_photos_light_makes_it_the_users() {
        let mut sel = region_selector();
        let sun = crate::ui::environment::rotation_from(200.0, 25.0);
        sel.set_photo(PhotoChoice {
            sun: Some(sun),
            ..photo(Some(SKY), Some(WATER))
        });
        sel.on_settings(SKY, Some(sky_asset(1.9)));
        sel.on_settings(WATER, Some(water_asset(9.0)));
        let shown = sel.frame(&input(1.0));
        // a camera preset saved now keeps that light
        let (data, preset) = sel.saved_photo().expect("a photo light");
        assert_eq!(preset, 0);
        let data = data.expect("a local environment");
        assert!(data.has("sky_llsd"), "the turned sun makes the sky a personal one");
        assert_eq!(data["water_id"].as_uuid(), WATER);
        assert!(sel.keep_photo());
        assert!(sel.take_photo_changed());
        assert!(sel.photo_choice().is_empty());
        assert!(sel.take_local_dirty(), "kept for the next session");
        assert_eq!(sel.local_view().sky, ListShown::Personal);
        assert_eq!(sel.local_view().water, ListShown::Item(WATER));
        let kept = sel.frame(&input(2.0));
        assert!((kept.0.gamma - shown.0.gamma).abs() < 1e-6, "the same light, without a crossfade");
        assert!(kept.0.sun_rotation.abs_diff_eq(sun, 1e-6));
        assert!((kept.1.fog_density - 9.0).abs() < 1e-6);
        // nothing more to keep
        assert!(!sel.keep_photo());
    }

    #[test]
    fn a_kept_time_of_day_is_the_panels_one() {
        let mut sel = region_selector();
        sel.set_photo(PhotoChoice {
            time: Some(3),
            ..PhotoChoice::default()
        });
        let asset = super::super::env::time_of_day_sky(3).expect("a preset");
        sel.on_settings(asset, Some(sky_asset(1.4)));
        sel.frame(&input(1.0));
        assert_eq!(sel.saved_photo().map(|s| s.1), Some(3));
        assert!(sel.keep_photo());
        let mut time = 0;
        let mut world_sel = std::mem::take(&mut sel);
        if let Some(p) = world_sel.take_preset_change() {
            time = p;
        }
        world_sel.set_time_of_day(time);
        assert_eq!(time, 3, "the panels follow the kept preset");
        assert!((world_sel.frame(&input(2.0)).0.gamma - 1.4).abs() < 1e-6);
    }

    #[test]
    fn a_pick_made_elsewhere_takes_over_the_photos_and_a_preset_replaces_it() {
        let mut sel = region_selector();
        sel.set_photo(PhotoChoice {
            time: Some(1),
            water: Some(WATER),
            ..PhotoChoice::default()
        });
        // the environment window's sky: the photo's time goes, its water stays
        sel.request_local(SKY, SettingsKind::Sky);
        assert!(sel.take_photo_changed());
        assert_eq!(sel.photo_choice(), photo(None, Some(WATER)));
        assert!(!sel.take_photo_changed());
        // a camera preset's light replaces all of it
        sel.drop_photo();
        assert!(sel.take_photo_changed());
        assert!(sel.photo_choice().is_empty());
        // RLVa holds the environment: no photo light either
        sel.rlva_locked = true;
        sel.set_photo(photo(Some(SKY), None));
        assert!(sel.photo_choice().is_empty());
        assert!(sel.take_photo_changed());
    }

    #[test]
    fn personal_lighting_starts_from_the_photos_light() {
        let mut sel = region_selector();
        sel.set_photo(photo(Some(SKY), None));
        sel.on_settings(SKY, Some(sky_asset(1.9)));
        sel.frame(&input(1.0));
        sel.capture_personal();
        assert!(sel.photo_choice().is_empty());
        assert!((sel.personal_sky().expect("a personal sky").gamma - 1.9).abs() < 1e-6);
    }

    #[test]
    fn the_photos_sun_over_a_day_freezes_its_sky_when_kept() {
        let mut sel = region_selector();
        let sun = crate::ui::environment::rotation_from(10.0, 40.0);
        sel.set_photo(PhotoChoice {
            day: Some(DAY),
            sun: Some(sun),
            ..PhotoChoice::default()
        });
        sel.on_settings(DAY, Some(day_asset(1.2, 4.0)));
        sel.frame(&input(1.0));
        assert!(sel.keep_photo());
        assert_eq!(sel.local_view().day, ListShown::Item(DAY));
        assert_eq!(sel.local_view().sky, ListShown::Personal);
        let (sky, water) = sel.frame(&input(2.0));
        assert!((sky.gamma - 1.2).abs() < 1e-6);
        assert!(sky.sun_rotation.abs_diff_eq(sun, 1e-6));
        assert!((water.fog_density - 4.0).abs() < 1e-6, "the day's water");
    }
}
