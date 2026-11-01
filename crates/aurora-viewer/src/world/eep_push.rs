//! The pushed environment (ENV_PUSH): what an experience's environment
//! controller sets while the agent has allowed the experience.
//!
//! Port of LLEnvironment::handleEnvironmentPush and LLEnvironment::DayInjection
//! (indra/newview/llenvironment.cpp, originally LGPL 2.1).
//! `PushFullEnvironment` names a settings asset: a day replaces the
//! shared day (setInjectedDay), a sky or a water replaces that half
//! (setInjectedSky / setInjectedWater). `PushPartialEnvironment` overrides
//! single keys of the sky and the water (injectSkySettings /
//! injectWaterSettings), each key owned by the last experience that set it.
//! `ClearEnvironment` removes what one experience pushed, or everything for
//! the null experience (clearInjections). The layer sits above the parcel and
//! region environments and under the user's own (local) one.
use super::eep::{DayCycle, Settings, SkyFrame, WaterFrame};
use aurora_llsd::Llsd;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

/// A full environment waiting for its settings asset.
#[derive(Debug, Clone, PartialEq)]
struct Waiting {
    experience: Uuid,
    asset: Uuid,
    transition: f32,
}

#[derive(Debug, Default)]
pub struct PushLayer {
    day: Option<(Uuid, Arc<DayCycle>)>,
    sky: Option<(Uuid, SkyFrame)>,
    water: Option<(Uuid, WaterFrame)>,
    sky_keys: BTreeMap<String, (Llsd, Uuid)>,
    water_keys: BTreeMap<String, (Llsd, Uuid)>,
    waiting: Vec<Waiting>,
    generation: u64,
    transition: f32,
}

impl PushLayer {
    pub fn is_active(&self) -> bool {
        self.day.is_some() || self.sky.is_some() || self.water.is_some() || !self.sky_keys.is_empty() || !self.water_keys.is_empty()
    }

    /// Changes with every push or clear: a new source for the crossfade.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Crossfade (s) of the last change.
    pub fn transition(&self) -> f32 {
        self.transition
    }

    /// The pushed day, which replaces the shared one.
    pub fn day(&self) -> Option<&Arc<DayCycle>> {
        self.day.as_ref().map(|(_, day)| day)
    }

    /// Settings assets the layer waits for.
    pub fn wanted(&self) -> Vec<Uuid> {
        let mut out = Vec::new();
        for w in &self.waiting {
            if !out.contains(&w.asset) {
                out.push(w.asset);
            }
        }
        out
    }

    /// Experiences that pushed something, or wait for their asset.
    #[cfg(test)]
    pub fn experiences(&self) -> Vec<Uuid> {
        let mut out: Vec<Uuid> = self
            .day
            .iter()
            .map(|(e, _)| *e)
            .chain(self.sky.iter().map(|(e, _)| *e))
            .chain(self.water.iter().map(|(e, _)| *e))
            .chain(self.sky_keys.values().map(|(_, e)| *e))
            .chain(self.water_keys.values().map(|(_, e)| *e))
            .chain(self.waiting.iter().map(|w| w.experience))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Handle a `PushExpEnvironment` message (handleEnvironmentPush).
    pub fn handle(&mut self, experience: Uuid, message: &Llsd) {
        let data = &message["action_data"];
        let transition = data["transition_time"].as_f32().max(0.0);
        match message["action"].as_str() {
            "ClearEnvironment" => self.clear(experience, transition),
            "PushFullEnvironment" => self.waiting.push(Waiting {
                experience,
                asset: data["asset_id"].as_uuid(),
                transition,
            }),
            "PushPartialEnvironment" => self.push_partial(experience, &data["settings"], transition),
            other => log::warn!("unknown environment push action '{other}'"),
        }
    }

    /// setExperienceEnvironment(experience, LLSD).
    fn push_partial(&mut self, experience: Uuid, settings: &Llsd, transition: f32) {
        let (sky, water) = (&settings["sky"], &settings["water"]);
        if sky.is_undef() && water.is_undef() {
            self.clear(experience, transition);
            return;
        }
        for (keys, values) in [(&mut self.sky_keys, sky), (&mut self.water_keys, water)] {
            for (key, value) in values.as_map().into_iter().flatten() {
                keys.insert(key.clone(), (value.clone(), experience));
            }
        }
        self.changed(transition);
    }

    /// A settings asset arrived (`None`: it could not be loaded). Returns the
    /// assets that failed, for FailedToFindSettings.
    pub fn on_settings(&mut self, id: Uuid, settings: Option<&Arc<Settings>>) -> Vec<Uuid> {
        let mut failed = Vec::new();
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.asset == id);
        self.waiting = rest;
        for w in ready {
            let Some(settings) = settings else {
                log::warn!("experience environment {} of {} could not be loaded", w.asset, w.experience);
                failed.push(w.asset);
                continue;
            };
            match Settings::clone(settings) {
                Settings::Day(day) => {
                    // setInjectedDay: the day owns the sky and the water too
                    self.sky = None;
                    self.water = None;
                    self.day = Some((w.experience, Arc::new(day)));
                }
                Settings::Sky(sky) => self.sky = Some((w.experience, sky)),
                Settings::Water(water) => self.water = Some((w.experience, water)),
            }
            self.changed(w.transition);
        }
        failed
    }

    /// clearInjections: what `experience` pushed (everything for the null one).
    pub fn clear(&mut self, experience: Uuid, transition: f32) {
        let ours = |owner: Uuid| experience.is_nil() || owner == experience;
        let before = self.is_active();
        if self.day.as_ref().is_some_and(|(owner, _)| ours(*owner)) {
            self.day = None;
        }
        if self.sky.as_ref().is_some_and(|(owner, _)| ours(*owner)) {
            self.sky = None;
        }
        if self.water.as_ref().is_some_and(|(owner, _)| ours(*owner)) {
            self.water = None;
        }
        self.sky_keys.retain(|_, (_, owner)| !ours(*owner));
        self.water_keys.retain(|_, (_, owner)| !ours(*owner));
        if experience.is_nil() {
            self.waiting.clear();
        } else {
            self.waiting.retain(|w| w.experience != experience);
        }
        if before {
            self.changed(transition);
        }
    }

    fn changed(&mut self, transition: f32) {
        self.generation += 1;
        self.transition = transition;
    }

    /// The shown sky and water with the pushed halves and key overrides over
    /// `base` (what the shared environment, or the pushed day, shows).
    pub fn apply(&self, mut sky: SkyFrame, mut water: WaterFrame) -> (SkyFrame, WaterFrame) {
        if let Some((_, pushed)) = &self.sky {
            sky = *pushed;
        }
        if let Some((_, pushed)) = &self.water {
            water = *pushed;
        }
        if !self.sky_keys.is_empty() {
            let mut map = sky.settings_llsd();
            for (key, (value, _)) in &self.sky_keys {
                map.insert(key.clone(), value.clone());
            }
            if let Some(Settings::Sky(merged)) = Settings::from_llsd(&map) {
                sky = merged;
            }
        }
        if !self.water_keys.is_empty() {
            let mut map = water.settings_llsd();
            for (key, (value, _)) in &self.water_keys {
                map.insert(key.clone(), value.clone());
            }
            if let Some(Settings::Water(merged)) = Settings::from_llsd(&map) {
                water = merged;
            }
        }
        (sky, water)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;
    use glam::Vec3;

    fn exp(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn partial(sky: Llsd, transition: f64) -> Llsd {
        llsd_map! {
            "action" => "PushPartialEnvironment",
            "action_data" => llsd_map! {
                "transition_time" => transition,
                "settings" => llsd_map! { "sky" => sky },
            },
        }
    }

    #[test]
    fn partial_keys_override_the_sky_and_clear_with_their_experience() {
        let base = SkyFrame::default();
        let red = Llsd::Array(vec![1.0.into(), 0.0.into(), 0.0.into()]);
        let mut layer = PushLayer::default();
        layer.handle(exp(1), &partial(llsd_map! { "sunlight_color" => red.clone() }, 2.0));
        assert!(layer.is_active());
        assert_eq!((layer.generation(), layer.transition()), (1, 2.0));
        let (sky, _) = layer.apply(base, WaterFrame::default());
        assert_eq!(sky.sunlight, Vec3::new(1.0, 0.0, 0.0));
        assert_ne!(base.sunlight, sky.sunlight);

        // another experience cannot clear it, its own clear can
        layer.clear(exp(2), 1.0);
        assert!(layer.is_active());
        layer.handle(
            exp(1),
            &llsd_map! { "action" => "ClearEnvironment", "action_data" => llsd_map! { "transition_time" => 3.0 } },
        );
        assert!(!layer.is_active());
        assert_eq!(layer.transition(), 3.0);
        assert_eq!(layer.apply(base, WaterFrame::default()).0.sunlight, base.sunlight);
    }

    #[test]
    fn a_partial_push_without_sky_or_water_clears_the_experience() {
        let mut layer = PushLayer::default();
        layer.handle(
            exp(1),
            &partial(llsd_map! { "glow" => Llsd::Array(vec![1.0.into(), 0.0.into(), 0.0.into()]) }, 0.0),
        );
        assert!(layer.is_active());
        let empty = llsd_map! {
            "action" => "PushPartialEnvironment",
            "action_data" => llsd_map! { "settings" => llsd_map! {} },
        };
        layer.handle(exp(1), &empty);
        assert!(!layer.is_active());
    }

    #[test]
    fn a_full_push_waits_for_its_asset_and_replaces_one_half() {
        let asset = exp(77);
        let mut layer = PushLayer::default();
        layer.handle(
            exp(1),
            &llsd_map! { "action" => "PushFullEnvironment", "action_data" => llsd_map! { "asset_id" => asset, "transition_time" => 4.0 } },
        );
        assert_eq!(layer.wanted(), vec![asset]);
        assert!(!layer.is_active());
        let pushed = SkyFrame::with_sun(Vec3::new(0.0, 0.0, 1.0));
        let failed = layer.on_settings(asset, Some(&Arc::new(Settings::Sky(pushed))));
        assert!(failed.is_empty());
        assert!(layer.wanted().is_empty());
        let water = WaterFrame::default();
        let (sky, shown_water) = layer.apply(SkyFrame::default(), water);
        assert_eq!(sky.sun_direction(), pushed.sun_direction());
        assert_eq!(shown_water.fog_density, water.fog_density);
        assert_eq!(layer.transition(), 4.0);

        layer.clear(Uuid::nil(), 0.0);
        assert!(!layer.is_active());
    }

    #[test]
    fn a_pushed_day_owns_the_sky_and_a_failed_asset_is_reported() {
        let (asset, missing) = (exp(5), exp(6));
        let mut layer = PushLayer::default();
        for id in [asset, missing] {
            layer.handle(
                exp(1),
                &llsd_map! { "action" => "PushFullEnvironment", "action_data" => llsd_map! { "asset_id" => id } },
            );
        }
        assert_eq!(layer.on_settings(missing, None), vec![missing]);
        let day = DayCycle::fixed(SkyFrame::default(), WaterFrame::default());
        layer.on_settings(asset, Some(&Arc::new(Settings::Day(day))));
        assert!(layer.day().is_some());
        layer.clear(exp(1), 0.0);
        assert!(layer.day().is_none() && !layer.is_active());
    }
}
