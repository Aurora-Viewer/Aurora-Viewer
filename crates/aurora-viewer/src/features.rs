//! Aurora's own features: what Aurora adds that the Legacy Viewer has none
//! of (the weather first), each one an entry of this registry that the user
//! switches on or off in Preferences › Aurora › Features. A feature switched
//! off shows none of its windows, buttons and menu entries, does none of
//! its work in the frame and draws nothing; its settings wait as they were.
//!
//! What a feature has written into shared data (an environment's Aurora
//! Weather code) is still read past and kept by the viewer, so that
//! switching a feature off never damages what other viewers show.
//!
//! Adding one: a variant of [`Feature`] with its texts, its switch in
//! [`Features`] and the setting of its row (`feature_field`,
//! ui/options/pages/aurora.rs), then `settings.features.on(Feature::…)`
//! wherever the feature starts work or shows a way in.

use serde::{Deserialize, Serialize};

/// One of Aurora's own features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// Aurora Weather (weather/mod.rs): rain, snow, wind, thunder and the
    /// northern lights, the user's own or a region's.
    Weather,
}

impl Feature {
    pub const ALL: [Feature; 1] = [Feature::Weather];

    pub fn name(self) -> &'static str {
        match self {
            Feature::Weather => aurora_i18n::tr!("ui-weather-aurora"),
        }
    }

    /// What it does and what switching it off takes away: the help of its
    /// row.
    pub fn about(self) -> &'static str {
        match self {
            Feature::Weather => aurora_i18n::tr!("ui-features-weather-help"),
        }
    }
}

/// Which features are switched on (the settings' `features`): all of them
/// in a fresh file, and one added later is on in a file from before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    pub weather: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self { weather: true }
    }
}

impl Features {
    pub fn on(&self, feature: Feature) -> bool {
        match feature {
            Feature::Weather => self.weather,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_is_on_in_a_fresh_file_and_has_its_texts() {
        let fresh = Features::default();
        for f in Feature::ALL {
            assert!(fresh.on(f), "{f:?}");
            assert!(!f.name().is_empty() && !f.about().is_empty(), "{f:?}");
        }
    }

    #[test]
    fn a_feature_switched_off_is_saved_and_read_back() {
        let off = Features { weather: false };
        assert!(!off.on(Feature::Weather));
        let json = serde_json::to_string(&off).expect("features serialize");
        let back: Features = serde_json::from_str(&json).expect("features read back");
        assert_eq!(back, off);
    }

    /// A file from before the registry, or from before a feature was added
    /// to it, has the feature on; a key this version does not know (a
    /// later version's feature) is read past.
    #[test]
    fn a_file_without_a_feature_has_it_on() {
        let old: Features = serde_json::from_str("{}").expect("empty");
        assert_eq!(old, Features::default());
        let later: Features = serde_json::from_str("{\"weather\": false, \"not-yet\": false}").expect("unknown key");
        assert!(!later.on(Feature::Weather));
        let settings: crate::settings::Settings = serde_json::from_str("{}").expect("empty settings");
        assert!(settings.features.on(Feature::Weather));
    }
}
