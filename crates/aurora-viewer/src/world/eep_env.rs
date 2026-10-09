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

use super::eep::{self, DayCycle, EnvAnswer, Settings, SkyFrame, WaterFrame};
use aurora_net::RegionHandle;
use glam::Vec3;
use std::sync::Arc;
use uuid::Uuid;

/// LLEnvironment::TRANSITION_DEFAULT (s): environment changes.
pub const TRANSITION_DEFAULT: f32 = 5.0;
/// LLEnvironment::TRANSITION_FAST (s): answers received while teleporting.
pub const TRANSITION_FAST: f32 = 1.0;
/// LLEnvironment::TRANSITION_ALTITUDE (s): sky track switches.
pub const TRANSITION_ALTITUDE: f32 = 5.0;

/// A day cycle in one of the environment slots (LLEnvironment::DayInstance).
#[derive(Debug, Clone)]
struct Slot {
    handle: RegionHandle,
    day: Arc<DayCycle>,
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
    /// ENV_LOCAL: the time-of-day presets (later the environment selector).
    Local(u64),
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
    local: Option<Slot>,
    local_preset: u8,
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
}

impl EnvSelector {
    fn slot(&mut self, handle: RegionHandle, day: Arc<DayCycle>, length: f64, offset: f64) -> Slot {
        self.generation += 1;
        Slot {
            handle,
            day,
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
        let day = answer.valid_day();
        if parcel_id == eep::REGION_PARCEL_ID {
            // a valid region day leaves the parcel's in place (RegionInfo
            // re-requests the region while the agent stays on its parcel)
            match day {
                Some(d) => {
                    self.altitudes = altitudes;
                    self.default_pending = None;
                    let (length, offset) = (d.length, d.offset);
                    self.region = Some(self.slot(handle, Arc::new(d), length, offset));
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
                    self.parcel = Some(self.slot(handle, Arc::new(d), length, offset));
                    AnswerOutcome::ParcelDay
                }
                None => {
                    self.parcel = None;
                    AnswerOutcome::ParcelCleared
                }
            }
        }
    }

    /// Settings assets the selector waits for (the default day for now;
    /// later the items picked in the environment selector).
    pub fn wanted_settings(&self) -> Option<Uuid> {
        (self.default_pending.is_some() && self.default_day.is_none() && !self.default_failed).then_some(eep::DEFAULT_DAY_ASSET)
    }

    /// A settings asset arrived (`None`: it could not be fetched or parsed).
    pub fn on_settings(&mut self, id: Uuid, settings: Option<Arc<Settings>>) {
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
        self.region = Some(self.slot(handle, day, length, offset));
    }

    /// Time-of-day preset (0 = none) as the local environment: a fixed sky
    /// with the preset sun.
    pub fn set_time_of_day(&mut self, preset: u8) {
        if preset == self.local_preset {
            return;
        }
        self.local_preset = preset;
        self.next_fade = TRANSITION_DEFAULT;
        self.local = (preset != 0).then(|| {
            let sky = SkyFrame::with_sun(super::env::time_of_day_sun(preset));
            let day = Arc::new(DayCycle::fixed(sky, WaterFrame::default()));
            self.slot(0, day, eep::DEFAULT_DAY_LENGTH, 0.0)
        });
    }

    /// The selected slot (getSelectedEnvironmentInstance).
    fn selected(&self, input: &FrameInput) -> (Source, Option<&Slot>) {
        if let Some(l) = &self.local {
            return (Source::Local(l.generation), Some(l));
        }
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
        let (source, slot) = self.selected(input);
        let (track, sky, water) = match slot {
            Some(s) => {
                let pos = s.position(input.unix_secs);
                let track = s.day.track_for_altitude(&self.altitudes, input.altitude);
                (track, s.day.sky_at(pos, track), s.day.water_at(pos))
            }
            None if source == Source::Legacy => (0, SkyFrame::with_sun(input.legacy_sun), WaterFrame::default()),
            None => (0, SkyFrame::default(), WaterFrame::default()),
        };
        if let Some(prev) = self.shown
            && (prev.source != source || prev.track != track)
        {
            // updateEnvironment: from what is shown (itself possibly mid-fade)
            let duration = if prev.source == source {
                TRANSITION_ALTITUDE
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
        self.shown = Some(Shown { source, track, sky, water });
        (sky, water)
    }
}

impl super::World {
    /// Sky and water to render this frame (`time_of_day`: the local preset,
    /// 0 for the shared environment).
    pub fn environment_frames(&mut self, time_of_day: u8) -> (SkyFrame, WaterFrame) {
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
        self.eep.set_time_of_day(time_of_day);
        self.eep.frame(&input)
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
        assert!(sel.wanted_settings().is_none());
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
        assert_eq!(sel.wanted_settings(), Some(eep::DEFAULT_DAY_ASSET));
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
        assert!(sel.wanted_settings().is_none());
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
        assert!(sel.wanted_settings().is_none());
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
        sel.frame(&input(2.0));
        assert!(matches!(sel.shown_source(), Some(Source::Local(_))));
        let (sky, _) = sel.frame(&input(7.1));
        assert!(sky.sun_direction().z < 0.0, "midnight preset");
    }
}
