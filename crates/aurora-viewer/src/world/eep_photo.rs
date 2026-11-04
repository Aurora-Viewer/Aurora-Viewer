//! The photo's own environment (`snapshot::light`), laid over the one
//! shown while the photo window is open: a day cycle, a sky, a water or a
//! time of day picked in its Light section, and the sun turned by its
//! azimuth and elevation.
//!
//! The Legacy Viewer's Phototools set the local environment itself (the Env
//! tab of floater_phototools.xml: FloaterQuickPrefs::selectSkyPreset,
//! selectWaterPreset and selectDayCyclePreset, newview/quickprefs.cpp) and
//! turn the sun in "Personal lighting" (LLFloaterEnvironmentAdjust::
//! onSunAzimElevChanged), until changed back. Here the layer is the photo's
//! only, like the rest of the Light section: the user's environment is
//! neither changed nor saved, closing the window gives it back as it was,
//! and "Keep this light" (`EnvSelector::keep_photo`) makes the layer theirs.
//! The same choices apply in the same way: a day replaces the fixed sky and
//! water, a sky or a water is fixed over the day (LLEnvironment::
//! setEnvironment), a time of day is its library sky.

use super::eep::{self, DayCycle, Settings, SkyFrame, WaterFrame};
use super::eep_env::{FrameInput, LocalOrigin, LocalView};
use super::env_select::{SettingsKind, Shown};
use glam::Quat;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// What the photo picked over the environment shown (None: what is shown).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PhotoChoice {
    pub day: Option<Uuid>,
    pub sky: Option<Uuid>,
    pub water: Option<Uuid>,
    /// A time of day preset (1..=5, `Panels::time_of_day`): its library sky.
    pub time: Option<u8>,
    /// The sun's rotation, over whatever sky is shown.
    pub sun: Option<Quat>,
    /// The sky, water and time a day replaced: restoring the day gives
    /// them back, so that its restore button undoes the day alone.
    pub replaced: Replaced,
}

/// The photo's picks a day replaced (`PhotoChoice::replaced`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Replaced {
    sky: Option<Uuid>,
    water: Option<Uuid>,
    time: Option<u8>,
}

impl PhotoChoice {
    /// Nothing over the environment shown (what a day replaced shows
    /// nothing).
    pub fn is_empty(&self) -> bool {
        self.day.is_none() && self.sky.is_none() && self.water.is_none() && self.time.is_none() && self.sun.is_none()
    }

    /// An item from one of the lists (None: the user's own again). A day
    /// replaces the fixed sky, water and time, which come back with the
    /// user's own day; a sky replaces the time. A sky, a day or a time of
    /// day brings its own sun, as a preset replaces the whole sky in the
    /// Legacy Viewer (LLEnvironment::setEnvironment): the photo's turned
    /// sun goes, otherwise Midnight would be lit by a morning sun.
    pub fn pick(&mut self, kind: SettingsKind, asset: Option<Uuid>) {
        match kind {
            SettingsKind::Day => {
                if asset.is_some() {
                    // (a day over a day: what the first one replaced stays,
                    // under the picks made over the first)
                    let r = self.replaced;
                    let (sky, time) = if self.sky.is_some() || self.time.is_some() {
                        (self.sky, self.time)
                    } else {
                        (r.sky, r.time)
                    };
                    self.replaced = Replaced {
                        sky,
                        water: self.water.or(r.water),
                        time,
                    };
                    self.sky = None;
                    self.water = None;
                    self.time = None;
                } else if self.day.is_some() {
                    // picks made over the day win over those it replaced
                    let r = std::mem::take(&mut self.replaced);
                    if self.sky.is_none() && self.time.is_none() {
                        self.sky = r.sky;
                        self.time = r.time;
                    }
                    self.water = self.water.or(r.water);
                }
                self.day = asset;
                if asset.is_some() {
                    self.sun = None;
                }
            }
            SettingsKind::Sky => {
                self.sky = asset;
                if asset.is_some() {
                    self.time = None;
                    self.sun = None;
                }
            }
            SettingsKind::Water => self.water = asset,
        }
    }

    /// A time of day (None: the user's own again); it replaces the sky.
    pub fn set_time(&mut self, time: Option<u8>) {
        self.time = time;
        if time.is_some() {
            self.sky = None;
            self.sun = None;
        }
    }

    /// The photo's own item of a list.
    pub fn of(&self, kind: SettingsKind) -> Option<Uuid> {
        match kind {
            SettingsKind::Day => self.day,
            SettingsKind::Sky => self.sky,
            SettingsKind::Water => self.water,
        }
    }

    /// What the lists show: the photo's choices over the user's own
    /// (`own`, FloaterQuickPrefs::setSelectedEnvironment's rules).
    pub fn shown(&self, own: LocalView) -> LocalView {
        let mut v = own;
        if let Some(d) = self.day {
            v = LocalView {
                sky: Shown::DayBased,
                water: Shown::DayBased,
                day: Shown::Item(d),
            };
        }
        if let Some(n) = self.time {
            v.sky = Shown::Preset(n);
        } else if let Some(s) = self.sky {
            v.sky = Shown::Item(s);
        }
        if let Some(w) = self.water {
            v.water = Shown::Item(w);
        }
        v
    }

    /// The settings assets it needs.
    fn assets(&self) -> impl Iterator<Item = Uuid> {
        let time = self.time.and_then(super::env::time_of_day_sky);
        [self.day, self.sky.filter(|_| self.time.is_none()), self.water, time]
            .into_iter()
            .flatten()
    }
}

/// A settings asset loaded for the photo.
#[derive(Debug, Clone)]
enum Loaded {
    Sky(Box<SkyFrame>),
    Water(Box<WaterFrame>),
    Day(Arc<DayCycle>),
    /// It could not be fetched, or is not settings of that list's type.
    Failed,
}

/// What the layer shows once its assets are there (a pick still loading
/// leaves what is shown, like a local pick).
#[derive(Debug, Clone, Default)]
pub(super) struct Resolved {
    pub day: Option<(Uuid, Arc<DayCycle>)>,
    pub sky: Option<(LocalOrigin, SkyFrame)>,
    pub water: Option<(Uuid, WaterFrame)>,
}

impl Resolved {
    /// It changes the environment (not only the sun).
    pub fn any(&self) -> bool {
        self.day.is_some() || self.sky.is_some() || self.water.is_some()
    }

    /// What it is made of, to tell when it changed.
    fn key(&self) -> (Option<Uuid>, Option<LocalOrigin>, Option<Uuid>) {
        (self.day.as_ref().map(|d| d.0), self.sky.map(|s| s.0), self.water.map(|w| w.0))
    }

    /// The photo's sky and water over those shown beneath (`under`: track,
    /// sky, water); its sun goes on top (`EnvSelector::frame`).
    pub fn compose(&self, altitudes: &[f32; 4], input: &FrameInput, under: (usize, SkyFrame, WaterFrame)) -> (usize, SkyFrame, WaterFrame) {
        let (mut track, mut sky, mut water) = under;
        if let Some((_, d)) = &self.day {
            // a local day: DEFAULT_DAYLENGTH / DEFAULT_DAYOFFSET
            let pos = eep::cycle_position(input.unix_secs, eep::DEFAULT_DAY_LENGTH, eep::DEFAULT_DAY_OFFSET);
            track = d.track_for_altitude(altitudes, input.altitude);
            sky = d.sky_at(pos, track);
            water = d.water_at(pos);
        }
        if let Some((_, s)) = self.sky {
            sky = s;
            track = 0;
        }
        if let Some((_, w)) = self.water {
            water = w;
        }
        (track, sky, water)
    }
}

/// The photo's layer in the environment selector.
#[derive(Debug, Default)]
pub(super) struct PhotoLayer {
    pub choice: PhotoChoice,
    loaded: HashMap<Uuid, Loaded>,
    /// Bumped when what it shows changes (a new crossfade, `Source::Photo`).
    pub generation: u64,
    key: (Option<Uuid>, Option<LocalOrigin>, Option<Uuid>),
}

impl PhotoLayer {
    /// The photo window's choice; its assets are forgotten once nothing is
    /// picked (the window closed, or everything back to the user's own).
    pub fn set(&mut self, choice: PhotoChoice) {
        if choice != self.choice {
            log::info!("photo light: {choice:?}");
            self.choice = choice;
        }
        if choice.is_empty() {
            self.loaded.clear();
        }
    }

    /// Assets still to load.
    pub fn wanted(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.choice.assets().filter(|id| !self.loaded.contains_key(id))
    }

    pub fn loading(&self) -> bool {
        self.wanted().next().is_some()
    }

    /// A settings asset arrived (None: it could not be loaded).
    pub fn on_settings(&mut self, id: Uuid, settings: Option<&Arc<Settings>>) {
        if !self.wanted().any(|w| w == id) {
            return;
        }
        let loaded = match settings.map(|s| &**s) {
            Some(Settings::Sky(s)) => Loaded::Sky(Box::new(*s)),
            Some(Settings::Water(w)) => Loaded::Water(Box::new(*w)),
            Some(Settings::Day(d)) => Loaded::Day(Arc::new(d.clone())),
            None => {
                log::warn!("photo light: environment settings {id} could not be loaded");
                Loaded::Failed
            }
        };
        self.loaded.insert(id, loaded);
    }

    /// What it shows now; the generation follows its changes.
    pub fn resolve(&mut self) -> Resolved {
        let r = self.resolved();
        if r.key() != self.key {
            self.key = r.key();
            self.generation += 1;
        }
        r
    }

    pub fn resolved(&self) -> Resolved {
        let c = &self.choice;
        let day = c.day.and_then(|id| match self.loaded.get(&id) {
            Some(Loaded::Day(d)) => Some((id, d.clone())),
            _ => None,
        });
        let sky = match (c.time, c.sky) {
            (Some(n), _) => super::env::time_of_day_sky(n).and_then(|id| match self.loaded.get(&id) {
                Some(Loaded::Sky(s)) => Some((LocalOrigin::Preset(n), **s)),
                // the built-in sky with the preset's sun, as for a local preset
                Some(Loaded::Failed) => Some((LocalOrigin::Preset(n), SkyFrame::with_sun(super::env::time_of_day_sun(n)))),
                _ => None,
            }),
            (None, Some(id)) => match self.loaded.get(&id) {
                Some(Loaded::Sky(s)) => Some((LocalOrigin::Item(id), **s)),
                _ => None,
            },
            (None, None) => None,
        };
        let water = c.water.and_then(|id| match self.loaded.get(&id) {
            Some(Loaded::Water(w)) => Some((id, **w)),
            _ => None,
        });
        Resolved { day, sky, water }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_day_replaces_the_fixed_halves_and_a_sky_the_time() {
        let mut c = PhotoChoice::default();
        c.set_time(Some(3));
        c.pick(SettingsKind::Water, Some(id(2)));
        c.pick(SettingsKind::Sky, Some(id(1)));
        assert_eq!((c.time, c.sky, c.water), (None, Some(id(1)), Some(id(2))));
        c.set_time(Some(4));
        assert_eq!((c.time, c.sky), (Some(4), None));
        c.pick(SettingsKind::Day, Some(id(3)));
        assert_eq!((c.day, c.time, c.sky, c.water), (Some(id(3)), None, None, None));
        // back to the user's own day: what the day replaced comes back
        c.pick(SettingsKind::Day, None);
        assert_eq!((c.day, c.time, c.sky, c.water), (None, Some(4), None, Some(id(2))));
        c.set_time(None);
        c.pick(SettingsKind::Water, None);
        assert!(c.is_empty());
    }

    #[test]
    fn restoring_the_day_keeps_the_picks_made_over_it() {
        let mut c = PhotoChoice::default();
        c.pick(SettingsKind::Sky, Some(id(1)));
        c.pick(SettingsKind::Water, Some(id(2)));
        c.pick(SettingsKind::Day, Some(id(3)));
        // a sky over the day, then another day: that sky waits now, with
        // the water the first day replaced
        c.pick(SettingsKind::Sky, Some(id(5)));
        c.pick(SettingsKind::Day, Some(id(6)));
        assert_eq!((c.sky, c.water), (None, None));
        c.pick(SettingsKind::Day, None);
        assert_eq!((c.sky, c.water), (Some(id(5)), Some(id(2))));
        c.pick(SettingsKind::Day, Some(id(6)));
        c.set_time(Some(2));
        c.pick(SettingsKind::Day, None);
        assert_eq!(
            (c.day, c.time, c.sky, c.water),
            (None, Some(2), None, Some(id(2))),
            "the time picked over the day stays, the replaced water comes back"
        );
        // nothing left to give back
        c.pick(SettingsKind::Day, Some(id(3)));
        c.pick(SettingsKind::Day, None);
        assert_eq!((c.time, c.sky, c.water), (Some(2), None, Some(id(2))));
    }

    #[test]
    fn the_lists_show_the_photos_choices_over_the_users() {
        let own = LocalView {
            sky: Shown::Item(id(9)),
            water: Shown::Shared,
            day: Shown::DayBased,
        };
        assert_eq!(PhotoChoice::default().shown(own), own);
        let c = PhotoChoice {
            day: Some(id(3)),
            water: Some(id(2)),
            ..PhotoChoice::default()
        };
        let v = c.shown(own);
        assert_eq!((v.sky, v.water, v.day), (Shown::DayBased, Shown::Item(id(2)), Shown::Item(id(3))));
        let t = PhotoChoice {
            time: Some(1),
            ..PhotoChoice::default()
        };
        assert_eq!(t.shown(own).sky, Shown::Preset(1));
    }

    #[test]
    fn a_pick_shows_once_loaded_and_a_failed_time_keeps_its_sun() {
        let mut layer = PhotoLayer::default();
        let sky = SkyFrame {
            gamma: 1.7,
            ..SkyFrame::default()
        };
        layer.set(PhotoChoice {
            sky: Some(id(1)),
            ..PhotoChoice::default()
        });
        assert!(layer.loading());
        assert!(!layer.resolve().any(), "the sky shown stays until the asset arrives");
        let g = layer.generation;
        layer.on_settings(id(1), Some(&Arc::new(Settings::Sky(sky))));
        let r = layer.resolve();
        assert_eq!(r.sky.map(|s| (s.0, s.1.gamma)), Some((LocalOrigin::Item(id(1)), 1.7)));
        assert!(layer.generation > g, "a new source: a new crossfade");
        // a time of day whose library sky cannot be fetched
        let mut c = layer.choice;
        c.set_time(Some(3));
        layer.set(c);
        let asset = crate::world::env::time_of_day_sky(3).expect("a preset");
        layer.on_settings(asset, None);
        let (origin, s) = layer.resolve().sky.expect("the built-in sky");
        assert_eq!(origin, LocalOrigin::Preset(3));
        assert!((s.sun_direction() - crate::world::env::time_of_day_sun(3)).length() < 1e-3);
        // a water asset picked from the sky list shows nothing
        layer.set(PhotoChoice {
            sky: Some(id(5)),
            ..PhotoChoice::default()
        });
        layer.on_settings(id(5), Some(&Arc::new(Settings::Water(WaterFrame::default()))));
        assert!(!layer.resolve().any());
        // nothing picked any more: the assets go
        layer.set(PhotoChoice::default());
        assert!(layer.loaded.is_empty());
    }

    #[test]
    fn a_new_sky_brings_its_own_sun() {
        let q = crate::ui::environment::rotation_from(90.0, 30.0);
        let turned = PhotoChoice {
            sun: Some(q),
            ..PhotoChoice::default()
        };
        // a water or a restore leaves the turned sun
        let mut c = turned;
        c.pick(SettingsKind::Water, Some(id(2)));
        c.pick(SettingsKind::Sky, None);
        c.set_time(None);
        assert_eq!(c.sun, Some(q));
        for pick in [
            |c: &mut PhotoChoice| c.pick(SettingsKind::Sky, Some(id(1))),
            |c: &mut PhotoChoice| c.pick(SettingsKind::Day, Some(id(3))),
            |c: &mut PhotoChoice| c.set_time(Some(4)),
        ] {
            let mut c = turned;
            pick(&mut c);
            assert_eq!(c.sun, None, "{c:?}");
        }
    }
}
