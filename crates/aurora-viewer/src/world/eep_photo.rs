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
}

impl PhotoChoice {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// An item from one of the lists (None: the user's own again). A day
    /// replaces the fixed sky, water and time; a sky replaces the time.
    pub fn pick(&mut self, kind: SettingsKind, asset: Option<Uuid>) {
        match kind {
            SettingsKind::Day => {
                self.day = asset;
                if asset.is_some() {
                    self.sky = None;
                    self.water = None;
                    self.time = None;
                }
            }
            SettingsKind::Sky => {
                self.sky = asset;
                if asset.is_some() {
                    self.time = None;
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
    /// sky, water), then its sun.
    pub fn compose(
        &self,
        sun: Option<Quat>,
        altitudes: &[f32; 4],
        input: &FrameInput,
        under: (usize, SkyFrame, WaterFrame),
    ) -> (usize, SkyFrame, WaterFrame) {
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
        if let Some(q) = sun {
            sky.sun_rotation = q;
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
        assert_eq!(
            c,
            PhotoChoice {
                day: Some(id(3)),
                ..PhotoChoice::default()
            }
        );
        // back to the user's own day
        c.pick(SettingsKind::Day, None);
        assert!(c.is_empty());
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
    fn the_sun_turns_over_any_sky() {
        let input = FrameInput {
            main_region: None,
            eep_region: None,
            legacy_sun: glam::Vec3::Z,
            unix_secs: 0.0,
            now: 0.0,
            altitude: 20.0,
        };
        let under = (2, SkyFrame::default(), WaterFrame::default());
        let q = crate::ui::environment::rotation_from(90.0, 30.0);
        let (track, sky, _) = Resolved::default().compose(Some(q), &[0.0; 4], &input, under);
        assert_eq!(track, 2, "the sun alone changes no source");
        assert!(sky.sun_rotation.abs_diff_eq(q, 1e-6));
    }
}
