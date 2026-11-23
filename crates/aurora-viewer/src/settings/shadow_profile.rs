//! Each enabled shadow tier has its own optional custom settings. Off only
//! gates rendering; the editing mode and other tiers survive a slider change.
use super::Settings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const SUN_DIAMETER_DEGREES: std::ops::RangeInclusive<f32> = 0.10..=2.0;
pub(super) const DEFAULT_SUN_DIAMETER: f32 = 0.53;

pub(super) fn bounded_sun_diameter(degrees: f32) -> f32 {
    if degrees.is_finite() {
        degrees.clamp(*SUN_DIAMETER_DEGREES.start(), *SUN_DIAMETER_DEGREES.end())
    } else {
        DEFAULT_SUN_DIAMETER
    }
}

pub(super) fn effective_sun_diameter(s: &Settings) -> f32 {
    if s.shadows && s.shadow_quality > 0 && s.sun_shadow_contact_hardening {
        bounded_sun_diameter(s.sun_shadow_angular_diameter_deg)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    Off,
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

// A v2.3.8 viewer retains this unknown field via the newer-document merger.
// Remembering the mode lets us recognize its automatic import of a new stock
// recipe as a Custom slot when someone switches back and forth between builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ProjectorRecipe {
    Stock(Tier),
    Custom(Tier),
}

// A v2.3.9 writer merges JSON objects key by key in a newer document. A
// Stock ↔ Custom change can therefore leave both tags in its cached marker.
// The actual selection lives in selected/use_custom and can rebuild it.
fn read_projector_recipe<'de, D: serde::Deserializer<'de>>(reader: D) -> Result<Option<ProjectorRecipe>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Recipe(ProjectorRecipe),
        Merged {
            #[serde(rename = "Stock")]
            _stock: Tier,
            #[serde(rename = "Custom")]
            _custom: Tier,
        },
    }
    Ok(match Option::<Stored>::deserialize(reader)? {
        Some(Stored::Recipe(recipe)) => Some(recipe),
        Some(Stored::Merged { .. }) | None => None,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, deserialize_with = "read_projector_recipe")]
    projector_recipe: Option<ProjectorRecipe>,
    // v2.3.9 can switch tiers while preserving newer fields verbatim. Record
    // which selection supplied the flattened directional-shadow settings.
    #[serde(default)]
    sun_recipe: Option<(Tier, bool)>,
    // v2.4.0 understood the angle but all its stock tiers used the fixed filter.
    #[serde(default)]
    soft_defaults_selection: Option<(Tier, bool)>,
    selected: Tier,
    use_custom: bool,
    // v2.3.6 clears use_custom at Off and on an untouched fixed recipe.
    // The older document merger preserves this key. Recording the tier lets
    // us also recognize an explicit switch to another preset in that client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    editing_tier: Option<Tier>,
    // A missing map identifies a v2.3.6 profile, even when its single slot was empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slots: Option<BTreeMap<Tier, CustomShadow>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom: Option<LegacyCustom>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CustomShadow {
    coverage: u8,
    tuning: Tuning,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paused: Option<PausedAdvanced>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PausedAdvanced {
    override_three: bool,
    override_four: bool,
    tuning: Tuning,
}

// The v2.3.6 checkbox controlled both visibility and whether overrides rendered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct LegacyCustom {
    base: Tier,
    coverage: u8,
    advanced: bool,
    override_three: bool,
    override_four: bool,
    tuning: Tuning,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Diagnostics {
    visibility: u8,
    cascades: bool,
    mask: bool,
    candidates: bool,
    failures: bool,
    view: bool,
}

impl Diagnostics {
    fn read(s: &Settings) -> Self {
        Self {
            visibility: s.shadow_visibility_debug,
            cascades: s.near_shadow_debug,
            mask: s.experimental_contact_mask_debug,
            candidates: s.experimental_contact_hit_debug,
            failures: s.experimental_contact_failure_debug,
            view: s.contact_view_debug,
        }
    }
    fn write(self, s: &mut Settings) {
        s.shadow_visibility_debug = if self.visibility <= 4 { self.visibility } else { 0 };
        s.near_shadow_debug = self.cascades;
        s.experimental_contact_mask_debug = self.mask;
        s.experimental_contact_hit_debug = self.candidates;
        s.experimental_contact_failure_debug = self.failures;
        s.contact_view_debug = self.view;
    }
}

// These have direct Settings counterparts and keep their existing units.
// Inactive-layout and inactive-backend values round-trip along with active ones.
macro_rules! tuning_fields {
    ($($field:ident : $ty:ty = $standard:expr),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        struct Tuning {
            map: u16,
            legacy_tier: u8,
            three_distance: u16,
            diagnostics: Diagnostics,
            $($field: $ty,)+
        }
        impl Default for Tuning {
            fn default() -> Self {
                Self {
                    map: 2048, legacy_tier: 3, three_distance: 192,
                    diagnostics: Diagnostics::default(), $($field: $standard,)+
                }
            }
        }
        impl Tuning {
            fn read(s: &Settings) -> Self {
                Self {
                    map: s.shadow_map_size(),
                    legacy_tier: if s.shadow_quality > 0 { s.shadow_quality } else { s.shadow_resume_quality }.clamp(1, 4),
                    three_distance: s.three_level_shadow_reach(),
                    diagnostics: Diagnostics::read(s),
                    $($field: s.$field,)+
                }
            }
            fn write(self, s: &mut Settings) {
                s.shadows = true;
                s.shadow_quality = self.legacy_tier;
                s.shadow_resume_quality = self.legacy_tier;
                s.shadow_map_resolution = self.map;
                s.three_level_shadow_distance = self.three_distance;
                self.diagnostics.write(s);
                $(s.$field = self.$field;)+
            }
        }
    }
}

tuning_fields! {
    graded_sun_shadows: bool = true,
    graded_shadow_distance: u16 = 192,
    graded_shadow_detail_metres: u8 = 2,
    graded_shadow_edge_smoothing: bool = true,
    sun_shadow_contact_hardening: bool = false,
    sun_shadow_angular_diameter_deg: f32 = DEFAULT_SUN_DIAMETER,
    near_shadow_detail: bool = true,
    near_shadow_detail_metres: u8 = 2,
    near_shadow_precision: bool = false,
    near_shadow_cascade_percent: u8 = 4,
    contact_shadows: bool = true,
    contact_ray_method: u8 = 0,
    experimental_contact_reach_mode: u8 = 0,
    experimental_contact_reach_metres: f32 = 0.5,
    experimental_contact_occluder_depth_metres: f32 = 0.05,
    contact_view_reach: f32 = 0.8,
    contact_view_thickness: f32 = 0.1,
    contact_view_budget: u32 = 96,
    contact_integrity_depth: bool = false,
    contact_integrity_hole_repair: bool = false,
    projector_shadows: bool = false,
    projector_shadow_limit: u8 = 2,
}

impl Tuning {
    fn use_tier_sun(&mut self, tier: Tier) {
        self.sun_shadow_contact_hardening = tier != Tier::Off;
        self.sun_shadow_angular_diameter_deg = tier.sun_diameter();
    }

    fn bounded(mut self) -> Self {
        fn finite(x: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
            if x.is_finite() { x.clamp(lo, hi) } else { fallback }
        }
        self.legacy_tier = self.legacy_tier.clamp(1, 4);
        self.map = Settings::shadow_resolution_for(self.legacy_tier, self.graded_sun_shadows, self.map) as u16;
        self.three_distance = self.three_distance.clamp(16, 512);
        self.sun_shadow_angular_diameter_deg = bounded_sun_diameter(self.sun_shadow_angular_diameter_deg);
        self.graded_shadow_distance = self.graded_shadow_distance.clamp(16, 512);
        self.graded_shadow_detail_metres = self.graded_shadow_detail_metres.clamp(2, 12);
        self.near_shadow_detail_metres = self.near_shadow_detail_metres.clamp(2, 12);
        self.near_shadow_cascade_percent = self.near_shadow_cascade_percent.clamp(3, 12);
        self.contact_ray_method = self.contact_ray_method.min(1);
        self.experimental_contact_reach_mode = self.experimental_contact_reach_mode.min(4);
        self.experimental_contact_reach_metres = finite(self.experimental_contact_reach_metres, 0.1, 1.2, 0.5);
        self.experimental_contact_occluder_depth_metres = finite(self.experimental_contact_occluder_depth_metres, 0., 0.3, 0.05);
        self.contact_view_reach = finite(self.contact_view_reach, 0.1, 2., 0.8);
        self.contact_view_thickness = finite(self.contact_view_thickness, 0.01, 0.3, 0.1);
        self.contact_view_budget = self.contact_view_budget.clamp(48, 128);
        self.projector_shadow_limit = self
            .projector_shadow_limit
            .clamp(1, aurora_render::projector_shadows::MAX_PROJECTOR_SHADOWS as u8);
        if self.diagnostics.visibility > 4 {
            self.diagnostics.visibility = 0;
        }
        self
    }
}

// Only this recipe table defines the visual/cost steps. The backend still
// reads Settings and has no dependency on preset names.
impl Tier {
    fn sun_diameter(self) -> f32 {
        match self {
            Self::Low | Self::Medium => 0.65,
            Self::Off | Self::High | Self::Ultra => DEFAULT_SUN_DIAMETER,
        }
    }

    fn recipe(self) -> Tuning {
        let mut tuning = match self {
            Self::Off | Self::Low => Tuning {
                map: 1024,
                legacy_tier: 1,
                three_distance: 48,
                graded_shadow_distance: 48,
                projector_shadows: self != Self::Off,
                projector_shadow_limit: 1,
                experimental_contact_reach_mode: 1,
                ..Tuning::default()
            },
            Self::Medium => Tuning {
                map: 2048,
                legacy_tier: 2,
                three_distance: 96,
                graded_shadow_distance: 96,
                projector_shadows: true,
                projector_shadow_limit: 2,
                experimental_contact_reach_mode: 1,
                ..Tuning::default()
            },
            Self::High => Tuning {
                map: 4096,
                legacy_tier: 3,
                three_distance: 144,
                graded_shadow_distance: 144,
                graded_shadow_detail_metres: 3,
                near_shadow_detail_metres: 3,
                experimental_contact_reach_mode: 1,
                projector_shadows: true,
                projector_shadow_limit: 4,
                ..Tuning::default()
            },
            Self::Ultra => Tuning {
                map: 4096,
                legacy_tier: 4,
                experimental_contact_reach_mode: 3,
                projector_shadows: true,
                projector_shadow_limit: 8,
                ..Tuning::default()
            },
        };
        tuning.use_tier_sun(self);
        tuning
    }
    fn distances(self, coverage: u8) -> (u16, u16) {
        let base = self.recipe();
        let scale = |m: u16| {
            match coverage {
                0 => m / 2,
                2 => m.saturating_mul(2),
                _ => m,
            }
            .clamp(16, 512)
        };
        (scale(base.three_distance), scale(base.graded_shadow_distance))
    }
    /// Fields owned by the whole-viewer graphics preset as well as Shadows.
    pub(crate) fn global_controls(self) -> (u16, bool, bool, u8) {
        if self == Self::Off {
            return (0, false, false, 2);
        }
        let t = self.recipe();
        (t.map, t.contact_shadows, t.projector_shadows, t.projector_shadow_limit)
    }
}

impl Tier {
    // Stock v2.3.9–2.4.0 used a fixed-width sun filter at every tier.
    fn fixed_sun_recipe(self) -> Tuning {
        Tuning {
            sun_shadow_contact_hardening: false,
            sun_shadow_angular_diameter_deg: DEFAULT_SUN_DIAMETER,
            ..self.recipe()
        }
    }

    // The v2.3.6–2.3.8 recipes also used different projector limits.
    fn previous_recipe(self) -> Tuning {
        let (projector_shadows, projector_shadow_limit) = match self {
            Self::Off | Self::Low | Self::Medium => (false, 2),
            Self::High => (true, 1),
            Self::Ultra => (true, 2),
        };
        Tuning {
            projector_shadows,
            projector_shadow_limit,
            ..self.fixed_sun_recipe()
        }
    }
}

impl CustomShadow {
    fn seeded(base: Tier) -> Self {
        Self {
            coverage: 1,
            tuning: base.recipe(),
            paused: None,
        }
    }
    fn imported(base: Tier, tuning: Tuning) -> Self {
        let coverage = (0..3)
            .find(|&level| base.distances(level) == (tuning.three_distance, tuning.graded_shadow_distance))
            .unwrap_or(1);
        Self {
            coverage,
            tuning,
            paused: None,
        }
    }
}

impl PausedAdvanced {
    fn resolve(&self, simple: Tuning, distances: (u16, u16)) -> Tuning {
        let mut t = self.tuning.bounded();
        t.sun_shadow_contact_hardening = simple.sun_shadow_contact_hardening;
        t.sun_shadow_angular_diameter_deg = simple.sun_shadow_angular_diameter_deg;
        t.projector_shadows = simple.projector_shadows;
        t.projector_shadow_limit = simple.projector_shadow_limit;
        if !self.override_three {
            t.three_distance = distances.0;
        }
        if !self.override_four {
            t.graded_shadow_distance = distances.1;
        }
        t
    }
}

impl LegacyCustom {
    fn into_slot(self) -> (Tier, CustomShadow) {
        let base = if self.base == Tier::Off { infer(self.tuning) } else { self.base };
        let coverage = self.coverage.min(2);
        let mut simple = base.previous_recipe();
        let scale = |m: u16| {
            match coverage {
                0 => m / 2,
                2 => m.saturating_mul(2),
                _ => m,
            }
            .clamp(16, 512)
        };
        let distances = (scale(simple.three_distance), scale(simple.graded_shadow_distance));
        simple.three_distance = distances.0;
        simple.graded_shadow_distance = distances.1;
        simple.projector_shadows = self.tuning.projector_shadows;
        simple.projector_shadow_limit = self.tuning.projector_shadow_limit;
        let paused = PausedAdvanced {
            tuning: self.tuning,
            override_three: self.override_three,
            override_four: self.override_four,
        };
        let advanced = paused.resolve(simple, distances);
        let mut slot = CustomShadow {
            coverage,
            tuning: simple.bounded(),
            paused: None,
        };
        if self.advanced {
            slot.tuning = advanced;
        } else if advanced != slot.tuning {
            slot.paused = Some(paused);
        }
        (base, slot)
    }
}

fn infer(t: Tuning) -> Tier {
    let tiers = [Tier::Low, Tier::Medium, Tier::High, Tier::Ultra];
    if let Some(exact) = tiers.into_iter().find(|tier| tier.recipe() == t) {
        return exact;
    }
    // Import the nearest resolution first. Reach breaks ties between the
    // two 4096 steps. This only picks a reset anchor; no raw input is changed.
    let distance = if t.graded_sun_shadows {
        t.graded_shadow_distance
    } else {
        t.three_distance
    };
    tiers
        .into_iter()
        .min_by_key(|tier| {
            let p = tier.recipe();
            (
                p.map.abs_diff(t.map),
                distance.abs_diff(if t.graded_sun_shadows {
                    p.graded_shadow_distance
                } else {
                    p.three_distance
                }),
            )
        })
        .unwrap_or(Tier::High)
}

impl Profile {
    fn imported(s: &Settings) -> Self {
        let t = Tuning::read(s).bounded();
        Self::from_flat(t, s.shadows && s.shadow_quality != 0)
    }

    fn new(selected: Tier) -> Self {
        Self {
            projector_recipe: Some(ProjectorRecipe::Stock(selected)),
            sun_recipe: Some((selected, false)),
            soft_defaults_selection: Some((selected, false)),
            selected,
            use_custom: false,
            editing_tier: None,
            slots: Some(BTreeMap::new()),
            custom: None,
        }
    }
    fn from_flat(t: Tuning, on: bool) -> Self {
        let anchor = infer(t);
        let mut p = Self::new(if on { anchor } else { Tier::Off });
        if anchor.recipe() != t {
            p.put(anchor, CustomShadow::imported(anchor, t));
            p.use_custom = on;
        }
        p.with_editing_tier()
    }
    fn selection(&self) -> ProjectorRecipe {
        if self.use_custom {
            ProjectorRecipe::Custom(self.selected)
        } else {
            ProjectorRecipe::Stock(self.selected)
        }
    }
    fn with_editing_tier(mut self) -> Self {
        self.projector_recipe = Some(self.selection());
        self.sun_recipe = Some((self.selected, self.use_custom));
        self.soft_defaults_selection = self.sun_recipe;
        self.editing_tier = self.use_custom.then_some(self.selected);
        self
    }
    fn needs_upgrade(&self) -> bool {
        self.projector_recipe != Some(self.selection())
            || self.sun_recipe != Some((self.selected, self.use_custom))
            || self.soft_defaults_selection != Some((self.selected, self.use_custom))
            || self.slots.is_none()
            || self.custom.is_some()
            || self.editing_tier.is_some_and(|tier| !self.use_custom || self.selected != tier)
    }
    pub(crate) fn saved_needs_upgrade(&self, quality: &crate::graphics_presets::Values, soft_sun: bool) -> bool {
        let old = self.selected.previous_recipe();
        self.needs_upgrade()
            || (self.selected != Tier::Off && !self.use_custom && !soft_sun)
            || (self.selected != Tier::Off
                && self.active().is_none()
                && (quality.projector_shadows, quality.projector_shadow_limit) == (old.projector_shadows, old.projector_shadow_limit))
    }
    fn upgrade(&mut self) {
        let slots = self.slots.get_or_insert_with(BTreeMap::new);
        if let Some(old) = self.custom.take() {
            let (mut base, mut slot) = old.into_slot();
            // v42 rebuilt its one slot by inferring the tier from the flat
            // pixel count. In v43 a High slot may deliberately use a Low map.
            // Route v42's edits back to the originating slot in that case.
            if let Some(editor) = self.editing_tier
                && let Some(original) = slots.get(&editor)
                && infer(original.tuning) == base
            {
                base = editor;
            }
            if let Some(original) = slots.get(&base)
                && slot.paused.is_none()
            {
                slot.paused = original.paused.clone();
            }
            if self.use_custom {
                // v2.3.6 displayed the slot's base even if selected still
                // named a different fixed tier behind the Custom checkbox.
                self.selected = base;
            }
            self.put(base, slot);
        }
    }
    fn put(&mut self, base: Tier, slot: CustomShadow) {
        let slots = self.slots.get_or_insert_with(BTreeMap::new);
        if slot == CustomShadow::seeded(base) {
            slots.remove(&base);
        } else {
            slots.insert(base, slot);
        }
    }
    fn active(&self) -> Option<&CustomShadow> {
        if self.use_custom && self.selected != Tier::Off {
            self.slots.as_ref()?.get(&self.selected)
        } else {
            None
        }
    }
    fn edit(&mut self, edit: impl FnOnce(Tier, &mut CustomShadow)) {
        if self.use_custom && self.selected != Tier::Off {
            let base = self.selected;
            let mut slot = self.active().cloned().unwrap_or_else(|| CustomShadow::seeded(base));
            edit(base, &mut slot);
            self.put(base, slot);
        }
    }
    fn write(&self, s: &mut Settings) {
        if self.selected == Tier::Off {
            s.set_shadow_enabled(false);
            s.sun_shadow_contact_hardening = false;
            s.contact_shadows = false;
            s.projector_shadows = false;
            Diagnostics::default().write(s);
        } else {
            self.active().map_or_else(|| self.selected.recipe(), |c| c.tuning).write(s);
        }
    }
}

/// The old integer tier also encodes map size in some editing paths. The
/// selected profile identifies the actual tier after a Custom map edit.
pub(crate) fn ultra_detail(s: &Settings) -> bool {
    if !s.shadows || s.shadow_quality == 0 || !s.graded_sun_shadows {
        return false;
    }
    if s.photo_held.is_some() && s.shadow_quality == 4 {
        return true;
    }
    s.shadow_profile
        .as_ref()
        .map_or(s.shadow_quality == 4, |p| p.selected == Tier::Ultra)
}

pub fn tier(s: &Settings) -> Tier {
    if let Some(p) = &s.shadow_profile {
        p.selected
    } else if s.shadows && s.shadow_quality != 0 {
        infer(Tuning::read(s))
    } else {
        Tier::Off
    }
}
/// Stock beta profiles serialized soft_shadows=false even when the user had
/// never touched that switch. Recognize the beta's shadow choices before
/// selecting the P3 recipe; preserve distinct manual controls in Custom.
pub(crate) fn import_upstream_controls(s: &mut Settings, beta: &serde_json::Value, stock_schema: bool) -> bool {
    let soft = beta["soft_shadows"].as_bool();
    if soft.is_none() && !stock_schema {
        return false;
    }
    let softness = beta["shadow_softness"]
        .as_f64()
        .filter(|v| v.is_finite())
        .map_or(super::SHADOW_SOFTNESS, |v| v as f32);
    let diameter = bounded_sun_diameter(Settings::diameter_from_softness(softness));
    if !s.shadows || s.shadow_quality == 0 {
        if let Some(soft) = soft {
            s.sun_shadow_contact_hardening = soft && softness > 0.0;
            s.sun_shadow_angular_diameter_deg = diameter;
        }
        return true;
    }

    let controls = (s.contact_shadows, s.projector_shadows, s.projector_shadow_limit);
    let stock = crate::graphics_presets::Preset::ALL.into_iter().find(|p| {
        let v = p.values();
        v.shadow_quality == s.shadow_quality && (v.effects.contact_shadows, v.projector_shadows, v.projector_shadow_limit) == controls
    });
    let factory = s.shadow_quality == 3 && controls == (false, true, 2);
    let tier = match stock {
        Some(crate::graphics_presets::Preset::High) => Tier::High,
        Some(crate::graphics_presets::Preset::Deluxe) => Tier::Ultra,
        _ => match s.shadow_quality {
            1 => Tier::Low,
            2 => Tier::Medium,
            3 => Tier::High,
            _ => Tier::Ultra,
        },
    };
    // Upstream has no P3 slots. Seed the tier directly so a later switch
    // to Custom cannot revive a synthetic slot inferred from upstream fields.
    apply(s, Profile::new(tier));
    let tuned_effect = soft == Some(true) || (softness - super::SHADOW_SOFTNESS).abs() > f32::EPSILON;
    let tuned_controls = stock.is_none() && !factory;
    if tuned_controls || tuned_effect {
        set_custom_enabled(s, true);
        if tuned_controls {
            (s.contact_shadows, s.projector_shadows, s.projector_shadow_limit) = controls;
        }
        if let Some(soft) = soft {
            s.sun_shadow_contact_hardening = soft && softness > 0.0;
            s.sun_shadow_angular_diameter_deg = diameter;
        }
        commit_edits(s);
    }
    true
}

/// Flat documents from before the tier editor omitted source size. Resolve
/// their initial angle using the same resolution/reach anchor as Custom reset.
pub(crate) fn imported_sun_diameter(s: &Settings) -> f32 {
    infer(Tuning::read(s)).sun_diameter()
}

pub fn base_tier(s: &Settings) -> Tier {
    tier(s)
}
pub fn custom_enabled(s: &Settings) -> bool {
    s.shadow_profile.as_ref().is_some_and(|p| p.use_custom)
}
pub fn advanced_enabled(s: &Settings) -> bool {
    s.shadow_advanced_open && custom_enabled(s) && tier(s) != Tier::Off
}
pub fn can_reset(s: &Settings) -> bool {
    s.shadow_profile.as_ref().and_then(Profile::active).is_some()
}
pub fn has_paused_advanced(s: &Settings) -> bool {
    s.shadow_profile
        .as_ref()
        .and_then(Profile::active)
        .is_some_and(|c| c.paused.is_some())
}
pub fn coverage(s: &Settings) -> u8 {
    s.shadow_profile.as_ref().and_then(Profile::active).map_or(1, |c| c.coverage)
}

/// An Advanced override can lie between all three safe coverage choices.
pub fn active_coverage(s: &Settings) -> Option<u8> {
    let level = coverage(s);
    let actual = if s.graded_sun_shadows {
        s.graded_shadow_distance
    } else {
        s.three_level_shadow_reach()
    };
    (coverage_distance(s, level) == actual).then_some(level)
}

pub fn coverage_distance(s: &Settings, level: u8) -> u16 {
    let (three, four) = base_tier(s).distances(level);
    if s.graded_sun_shadows { four } else { three }
}

fn user_tuning(s: &Settings, quality: &crate::graphics_presets::Values) -> Tuning {
    let mut t = Tuning::read(s);
    t.map = quality.shadow_map_resolution;
    t.legacy_tier = if quality.shadow_quality == 0 {
        quality.shadow_resume_quality
    } else {
        quality.shadow_quality
    };
    if s.three_level_shadow_distance == 0 {
        t.three_distance = Settings::shadow_params_of(t.legacy_tier.clamp(1, 4)).1 as u16;
    }
    t.projector_shadows = quality.projector_shadows;
    t.projector_shadow_limit = quality.projector_shadow_limit;
    t.contact_shadows = quality.effects.contact_shadows;
    t.bounded()
}

pub(crate) fn imported_profile(s: &Settings, quality: &crate::graphics_presets::Values, shadows: bool) -> Profile {
    Profile::from_flat(user_tuning(s, quality), shadows && quality.shadow_quality != 0)
}

/// A named preset holds both its effective rendering inputs and its editing
/// history. A caller can modify the flat inputs without visiting the Shadows
/// page (photo mode, a settings import or an older viewer).
pub(crate) fn captured(s: &Settings, quality: &crate::graphics_presets::Values, shadows: bool) -> Profile {
    reconcile(s, quality, shadows, false)
}

fn reconcile(s: &Settings, quality: &crate::graphics_presets::Values, shadows: bool, reopening: bool) -> Profile {
    let mut t = user_tuning(s, quality);
    let on = shadows && quality.shadow_quality != 0;
    let Some(mut p) = s.shadow_profile.clone() else {
        return Profile::from_flat(t, on);
    };
    let upgrading = p.needs_upgrade();
    let predates_sun_control = p.sun_recipe.is_none() && p.soft_defaults_selection.is_none();
    p.upgrade();
    if predates_sun_control {
        // Earlier snapshots had no sun filter choice. Normalize the flat
        // inputs to their original recipe before upgrading projector limits.
        t.sun_shadow_contact_hardening = false;
        t.sun_shadow_angular_diameter_deg = DEFAULT_SUN_DIAMETER;
        if let Some(slots) = &mut p.slots {
            for (tier, slot) in slots {
                slot.tuning.use_tier_sun(*tier);
                if let Some(paused) = &mut slot.paused {
                    paused.tuning.use_tier_sun(*tier);
                }
            }
        }
    }
    if reopening && p.sun_recipe.is_some_and(|previous| previous != (p.selected, p.use_custom)) {
        // The older viewer selected a different tier but could not update the
        // flattened values. Use the retained destination slot's settings.
        let current = p.active().map_or_else(|| p.selected.recipe(), |slot| slot.tuning);
        t.sun_shadow_contact_hardening = current.sun_shadow_contact_hardening;
        t.sun_shadow_angular_diameter_deg = current.sun_shadow_angular_diameter_deg;
    }
    if (upgrading || reopening) && p.selected != Tier::Off && p.active().is_none() && t == p.selected.previous_recipe() {
        t = p.selected.recipe();
    }
    // A checked Custom tier in v2.4.0 may be explicitly off even when
    // its full snapshot matched the old defaults and no slot was stored.
    if predates_sun_control {
        let tier = if p.selected == Tier::Off { infer(t) } else { p.selected };
        t.use_tier_sun(tier);
    } else if (upgrading || reopening) && !p.use_custom && p.selected != Tier::Off && t == p.selected.fixed_sun_recipe() {
        t = p.selected.recipe();
    }
    // An older viewer can manufacture a Custom slot containing exactly our
    // stock tier. A modern edit to any other field or count must survive.
    let was_fixed = p.projector_recipe == Some(ProjectorRecipe::Stock(p.selected))
        || (p.soft_defaults_selection == Some((p.selected, false)) && p.sun_recipe == Some((p.selected, true)));
    if was_fixed && p.use_custom && p.active() == Some(&CustomShadow::seeded(p.selected)) && t == p.selected.recipe() {
        p.use_custom = false;
        p.editing_tier = None;
    }
    if let Some(slots) = &mut p.slots {
        slots.retain(|&base, slot| {
            slot.coverage = slot.coverage.min(2);
            slot.tuning = slot.tuning.bounded();
            if let Some(paused) = &mut slot.paused {
                paused.tuning = paused.tuning.bounded();
            }
            base != Tier::Off && slot != &CustomShadow::seeded(base)
        });
    }
    if !p.use_custom && p.editing_tier == Some(p.selected) {
        let stored = p.slots.as_ref().and_then(|slots| slots.get(&p.selected));
        let same_look = stored.map_or_else(|| p.selected.recipe(), |c| c.tuning) == t;
        if (!on && p.selected == Tier::Off) || (on && p.selected != Tier::Off && same_look) {
            p.use_custom = true;
        }
    }
    if !on {
        p.selected = Tier::Off;
        return p.with_editing_tier();
    }
    if p.selected != Tier::Off && p.use_custom {
        // Keep the old slot and any paused legacy overrides when a different
        // preferences panel or an older client edits the flattened controls.
        p.edit(|base, slot| {
            if slot.tuning != t {
                if let Some(level) = (0..3).find(|&level| base.distances(level) == (t.three_distance, t.graded_shadow_distance)) {
                    slot.coverage = level;
                }
                slot.tuning = t;
            }
        });
    } else if p.selected == Tier::Off || p.selected.recipe() != t {
        let anchor = infer(t);
        p.selected = anchor;
        if anchor.recipe() != t {
            p.use_custom = true;
            p.edit(|base, slot| {
                let incoming = CustomShadow::imported(base, t);
                slot.coverage = incoming.coverage;
                slot.tuning = incoming.tuning;
            });
        }
    }
    p.with_editing_tier()
}

pub(crate) fn matches_global(s: &Settings, tier: Tier) -> bool {
    if tier == Tier::Off {
        return !s.shadows && s.shadow_quality == 0;
    }
    s.shadows && s.shadow_quality > 0 && Tuning::read(s) == tier.recipe()
}

pub(crate) fn remember_before_global(s: &mut Settings) {
    if s.shadow_profile.is_none() {
        s.shadow_profile = Some(Profile::imported(s));
    }
}

fn take(s: &mut Settings) -> Profile {
    let mut p = s.shadow_profile.take().unwrap_or_else(|| Profile::imported(s));
    p.upgrade();
    p
}
fn apply(s: &mut Settings, p: Profile) {
    p.write(s);
    s.shadow_profile = Some(p.with_editing_tier());
    crate::snapshot::quality::adopt_shadow_selection(s);
}

pub fn select_tier(s: &mut Settings, tier: Tier) {
    let custom = custom_enabled(s);
    let mut p = take(s);
    p.selected = tier;
    p.use_custom = custom;
    apply(s, p);
}

pub(crate) fn select_fixed_tier(s: &mut Settings, tier: Tier) {
    let mut p = take(s);
    p.selected = tier;
    p.use_custom = false;
    s.shadow_advanced_open = false;
    apply(s, p);
}

pub fn set_custom_enabled(s: &mut Settings, enabled: bool) {
    let mut p = take(s);
    p.use_custom = enabled;
    if !enabled {
        s.shadow_advanced_open = false;
    }
    apply(s, p);
}

pub fn set_advanced_enabled(s: &mut Settings, enabled: bool) {
    s.shadow_advanced_open = enabled && custom_enabled(s) && tier(s) != Tier::Off;
}

pub fn set_coverage(s: &mut Settings, value: u8) {
    let mut p = take(s);
    p.edit(|base, c| {
        c.coverage = value.min(2);
        (c.tuning.three_distance, c.tuning.graded_shadow_distance) = base.distances(c.coverage);
    });
    apply(s, p);
}

pub fn restore_paused_advanced(s: &mut Settings) {
    let mut p = take(s);
    p.edit(|base, c| {
        if let Some(paused) = c.paused.take() {
            c.tuning = paused.resolve(c.tuning, base.distances(c.coverage));
        }
    });
    apply(s, p);
}

pub fn reset_to_base(s: &mut Settings) {
    let mut p = take(s);
    if p.use_custom
        && let Some(slots) = &mut p.slots
    {
        slots.remove(&p.selected);
    }
    apply(s, p);
}

pub fn reset_all(s: &mut Settings) {
    s.shadow_advanced_open = false;
    apply(s, Profile::new(Tier::High));
}

pub fn commit_edits(s: &mut Settings) {
    let mut p = take(s);
    p.edit(|_, slot| slot.tuning = Tuning::read(s).bounded());
    s.shadow_profile = Some(p.with_editing_tier());
    crate::snapshot::quality::adopt_shadow_selection(s);
}

pub(crate) fn loaded(s: &mut Settings) {
    if s.shadow_profile.is_none() {
        // Keep flat legacy inputs on first import, including their dormant
        // values when the master switch is off.
        s.shadow_profile = Some(Profile::imported(s));
        return;
    }
    let p = reconcile(s, &crate::graphics_presets::Values::read(s), s.shadows, true);
    // Older builds allowed dormant switches and diagnostics while shadow maps
    // were off. Keep those flat fields on reopen; only an explicit Off action
    // resets them. The rendering entry point gates all dependent targets.
    if p.selected == Tier::Off && (!s.shadows || s.shadow_quality == 0) {
        s.shadow_profile = Some(p.with_editing_tier());
    } else {
        apply(s, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_custom_tiers_are_independent_and_reset_only_the_selected_tier() {
        let mut s = Settings::default();
        reset_all(&mut s);
        select_tier(&mut s, Tier::Low);
        set_custom_enabled(&mut s, true);
        assert_eq!(tier(&s), Tier::Low);
        assert!(!advanced_enabled(&s) && !can_reset(&s));
        let edits = [(Tier::Low, 61), (Tier::Medium, 103), (Tier::High, 155), (Tier::Ultra, 203)];
        for (base, reach) in edits {
            select_tier(&mut s, base);
            assert!(custom_enabled(&s));
            assert_eq!(Tuning::read(&s), base.recipe());
            set_advanced_enabled(&mut s, true);
            s.graded_shadow_distance = reach;
            s.shadow_visibility_debug = 2;
            commit_edits(&mut s);
            set_advanced_enabled(&mut s, false);
            assert_eq!((s.graded_shadow_distance, s.shadow_visibility_debug), (reach, 2));
        }
        for (base, reach) in edits {
            select_tier(&mut s, base);
            assert_eq!(s.graded_shadow_distance, reach);
            assert!(can_reset(&s));
        }
        select_tier(&mut s, Tier::Medium);
        set_custom_enabled(&mut s, false);
        assert_eq!(Tuning::read(&s), Tier::Medium.recipe());
        set_custom_enabled(&mut s, true);
        assert_eq!((tier(&s), s.graded_shadow_distance), (Tier::Medium, 103));
        reset_to_base(&mut s);
        assert!(custom_enabled(&s) && !can_reset(&s));
        assert_eq!(Tuning::read(&s), Tier::Medium.recipe());
        select_tier(&mut s, Tier::Low);
        assert_eq!((s.graded_shadow_distance, s.shadow_visibility_debug), (61, 2));
    }

    #[test]
    fn saved_state_round_trips_without_rendering_dormant_values() {
        let mut s = Settings::default();
        reset_all(&mut s);
        select_tier(&mut s, Tier::Ultra);
        set_custom_enabled(&mut s, true);
        set_advanced_enabled(&mut s, true);
        s.experimental_contact_hit_debug = true;
        s.three_level_shadow_distance = 123;
        commit_edits(&mut s);
        select_tier(&mut s, Tier::Medium);
        let state = serde_json::to_value(&s).unwrap();
        assert!(state.get("shadow_advanced_open").is_none());
        let mut restored: Settings = serde_json::from_value(state).unwrap();
        loaded(&mut restored);
        assert_eq!(Tuning::read(&restored), Tier::Medium.recipe());
        assert!(custom_enabled(&restored) && !advanced_enabled(&restored));
        select_tier(&mut restored, Tier::Ultra);
        assert_eq!(restored.three_level_shadow_reach(), 123);
        assert!(restored.experimental_contact_hit_debug);
    }

    #[test]
    fn importing_a_legacy_scene_preserves_both_layouts_and_the_debug_view() {
        let mut s = Settings::default();
        s.set_shadow_map_size(4096);
        s.graded_sun_shadows = true;
        s.graded_shadow_distance = 166;
        s.three_level_shadow_distance = 87;
        s.graded_shadow_detail_metres = 7;
        s.projector_shadow_limit = 13;
        s.contact_view_debug = true;
        let original = Tuning::read(&s);
        loaded(&mut s);
        assert_eq!(Tuning::read(&s), original);
        assert!(custom_enabled(&s) && !advanced_enabled(&s));
        set_advanced_enabled(&mut s, true);
        assert_eq!(Tuning::read(&s), original);
        set_advanced_enabled(&mut s, false);
        assert_eq!(Tuning::read(&s), original);
        let anchor = tier(&s);
        s.set_shadow_enabled(false);
        s.shadow_profile = None;
        loaded(&mut s);
        assert_eq!(tier(&s), Tier::Off);
        set_custom_enabled(&mut s, true);
        assert!(!s.shadows);
        select_tier(&mut s, anchor);
        assert_eq!(Tuning::read(&s), original);
    }

    #[test]
    fn old_single_slot_uses_its_own_base_and_recovers_paused_edits_explicitly() {
        for active in [true, false] {
            for advanced in [true, false] {
                let mut s = Settings::default();
                let simple = Tier::High.previous_recipe();
                let mut raw = simple;
                raw.graded_shadow_distance = 131;
                raw.experimental_contact_occluder_depth_metres = 0.13;
                let initial = if active {
                    if advanced { raw } else { simple }
                } else {
                    Tier::Low.previous_recipe()
                };
                initial.write(&mut s);
                s.shadow_profile = Some(
                    serde_json::from_value(serde_json::json!({
                        "selected": "Low", "use_custom": active, "custom": {
                            "base": "High", "coverage": 1, "advanced": advanced, "override_three": false,
                            "override_four": true, "tuning": raw
                        }
                    }))
                    .unwrap(),
                );
                loaded(&mut s);
                let mut simple = simple;
                simple.use_tier_sun(Tier::High);
                raw.use_tier_sun(Tier::High);
                assert_eq!(custom_enabled(&s), active);
                assert!(!advanced_enabled(&s));
                if !active {
                    assert_eq!(Tuning::read(&s), Tier::Low.recipe());
                    set_custom_enabled(&mut s, true);
                    assert_eq!(tier(&s), Tier::Low);
                    select_tier(&mut s, Tier::High);
                }
                assert_eq!(tier(&s), Tier::High);
                assert_eq!(Tuning::read(&s), if advanced { raw } else { simple });
                set_advanced_enabled(&mut s, true);
                assert_eq!(Tuning::read(&s), if advanced { raw } else { simple });
                assert_eq!(has_paused_advanced(&s), !advanced);
                if !advanced {
                    let state = serde_json::to_string(&s).unwrap();
                    s = serde_json::from_str(&state).unwrap();
                    loaded(&mut s);
                    assert!(has_paused_advanced(&s));
                    s.sun_shadow_contact_hardening = true;
                    s.sun_shadow_angular_diameter_deg = 1.0;
                    commit_edits(&mut s);
                    restore_paused_advanced(&mut s);
                    assert_eq!(
                        Tuning::read(&s),
                        Tuning {
                            sun_shadow_contact_hardening: true,
                            sun_shadow_angular_diameter_deg: 1.0,
                            ..raw
                        }
                    );
                    assert!(!has_paused_advanced(&s));
                }
            }
        }
    }

    #[test]
    fn v42_writer_retains_custom_at_off_and_does_not_override_an_explicit_preset_switch() {
        let mut s = Settings::default();
        reset_all(&mut s);
        set_custom_enabled(&mut s, true);
        for (base, reach) in [(Tier::Low, 57), (Tier::High, 139)] {
            select_tier(&mut s, base);
            s.graded_shadow_distance = reach;
            commit_edits(&mut s);
        }
        for selected in [Tier::Off, Tier::Medium] {
            select_tier(&mut s, selected);
            let newer = serde_json::to_value(&s).unwrap();
            let mut known = newer.clone();
            // v42 can retain neither an armed Off nor an unmodified Custom
            // on Medium in its single-slot model. It writes only these keys.
            known["shadow_profile"] = serde_json::json!({
                "selected": selected, "use_custom": false, "custom": null
            });
            let merged = crate::tolerant::merge_over_newer(&newer, known.clone(), &known, &[]);
            let mut reopened: Settings = serde_json::from_value(merged).unwrap();
            loaded(&mut reopened);
            assert_eq!(tier(&reopened), selected);
            assert!(custom_enabled(&reopened));
            assert_eq!(reopened.shadows, selected != Tier::Off);
            for (base, reach) in [(Tier::Low, 57), (Tier::High, 139)] {
                select_tier(&mut reopened, base);
                assert_eq!(reopened.graded_shadow_distance, reach);
            }

            // Picking a different stock tier in the v42 client is intentional.
            let mut old_selection = s.clone();
            Tier::Ultra.recipe().write(&mut old_selection);
            let mut change = serde_json::to_value(old_selection).unwrap();
            change["shadow_profile"] = serde_json::json!({
                "selected": "Ultra", "use_custom": false, "custom": null
            });
            let merged = crate::tolerant::merge_over_newer(&newer, change, &known, &[]);
            let mut reopened: Settings = serde_json::from_value(merged).unwrap();
            loaded(&mut reopened);
            assert_eq!(tier(&reopened), Tier::Ultra);
            assert!(!custom_enabled(&reopened));
            set_custom_enabled(&mut reopened, true);
            select_tier(&mut reopened, Tier::Low);
            assert_eq!(reopened.graded_shadow_distance, 57);
        }
    }

    #[test]
    fn v42_reinferred_map_size_cannot_relocate_one_custom_tier_over_another() {
        let mut s = Settings::default();
        reset_all(&mut s);
        set_custom_enabled(&mut s, true);
        select_tier(&mut s, Tier::Low);
        set_coverage(&mut s, 0);
        let low_reach = s.graded_shadow_distance;
        select_tier(&mut s, Tier::High);
        s.set_shadow_map_size(1024);
        s.graded_shadow_distance = 139;
        commit_edits(&mut s);
        let newer = serde_json::to_value(&s).unwrap();
        // v42 imports the 1024px flat values as a Low-based slot and the
        // user subsequently edits their reach in that older viewer.
        s.graded_shadow_distance = 137;
        let mut older = serde_json::to_value(&s).unwrap();
        older["shadow_profile"] = serde_json::json!({
            "selected": "Low", "use_custom": true, "custom": {
                "base": "Low", "coverage": 1, "advanced": true, "override_three": true,
                "override_four": true, "tuning": Tuning::read(&s)
            }
        });
        let merged = crate::tolerant::merge_over_newer(&newer, older, &newer, &[]);
        let mut reopened: Settings = serde_json::from_value(merged).unwrap();
        loaded(&mut reopened);
        assert_eq!(
            (tier(&reopened), reopened.shadow_map_size(), reopened.graded_shadow_distance),
            (Tier::High, 1024, 137)
        );
        select_tier(&mut reopened, Tier::Low);
        assert_eq!(reopened.graded_shadow_distance, low_reach);
    }

    #[test]
    fn incoming_inactive_slots_are_bounded_before_use() {
        let mut s = Settings::default();
        reset_all(&mut s);
        set_custom_enabled(&mut s, true);
        select_tier(&mut s, Tier::Low);
        set_coverage(&mut s, 0);
        select_tier(&mut s, Tier::Off);
        let mut wire = serde_json::to_value(&s).unwrap();
        let slot = &mut wire["shadow_profile"]["slots"]["Low"];
        slot["coverage"] = 9.into();
        slot["tuning"]["graded_shadow_distance"] = 1.into();
        slot["tuning"]["sun_shadow_contact_hardening"] = true.into();
        slot["tuning"]["sun_shadow_angular_diameter_deg"] = 20.0.into();
        slot["tuning"]["contact_view_budget"] = 300.into();
        slot["tuning"]["experimental_contact_occluder_depth_metres"] = 1.5.into();
        slot["tuning"]["diagnostics"]["visibility"] = 254.into();
        let mut loaded_settings: Settings = serde_json::from_value(wire).unwrap();
        loaded(&mut loaded_settings);
        assert!(!loaded_settings.shadows);
        select_tier(&mut loaded_settings, Tier::Low);
        assert_eq!(coverage(&loaded_settings), 2);
        assert_eq!(loaded_settings.graded_shadow_distance, 16);
        assert_eq!(loaded_settings.render_settings().sun_shadow_angular_diameter_deg, 2.0);
        assert_eq!(loaded_settings.contact_view_budget, 128);
        assert_eq!(loaded_settings.experimental_contact_occluder_depth_metres, 0.3);
        assert_eq!(loaded_settings.shadow_visibility_debug, 0);
    }

    #[test]
    fn contact_hardening_remembers_custom_angles_across_tiers_profiles_and_off() {
        use crate::graphics_presets::saved::Saved;

        let mut s = Settings::default();
        reset_all(&mut s);
        for tier in [Tier::Low, Tier::Medium, Tier::High, Tier::Ultra] {
            select_tier(&mut s, tier);
            assert_eq!(s.render_settings().sun_shadow_angular_diameter_deg, tier.sun_diameter());
        }
        set_custom_enabled(&mut s, true);
        s.sun_shadow_contact_hardening = true;
        s.sun_shadow_angular_diameter_deg = 1.0;
        for four in [false, true] {
            s.graded_sun_shadows = four;
            assert_eq!(s.render_settings().sun_shadow_angular_diameter_deg, 1.0);
        }
        commit_edits(&mut s);
        set_advanced_enabled(&mut s, false);
        assert_eq!(s.render_settings().sun_shadow_angular_diameter_deg, 1.0);
        set_custom_enabled(&mut s, false);
        assert_eq!(s.render_settings().sun_shadow_angular_diameter_deg, Tier::Ultra.sun_diameter());

        let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&Saved::capture(&s)).unwrap()).unwrap();
        let mut restored = Settings::default();
        saved.apply(&mut restored);
        assert!(saved.matches_current(&restored));
        assert!(restored.sun_shadow_contact_hardening);
        set_custom_enabled(&mut restored, true);
        assert_eq!(restored.render_settings().sun_shadow_angular_diameter_deg, 1.0);
        select_tier(&mut restored, Tier::High);
        assert_eq!(
            restored.render_settings().sun_shadow_angular_diameter_deg,
            Tier::High.sun_diameter()
        );
        select_tier(&mut restored, Tier::Off);
        assert_eq!(restored.render_settings().sun_shadow_angular_diameter_deg, 0.0);
        let mut reopened: Settings = serde_json::from_slice(&serde_json::to_vec(&restored).unwrap()).unwrap();
        loaded(&mut reopened);
        select_tier(&mut reopened, Tier::Ultra);
        assert_eq!(reopened.render_settings().sun_shadow_angular_diameter_deg, 1.0);
        reopened.sun_shadow_contact_hardening = false;
        commit_edits(&mut reopened);
        assert_eq!(reopened.render_settings().sun_shadow_angular_diameter_deg, 0.0);
        assert_eq!(reopened.sun_shadow_angular_diameter_deg, 1.0);
        reset_to_base(&mut reopened);
        assert!(!can_reset(&reopened));
        assert_eq!(reopened.sun_shadow_angular_diameter_deg, DEFAULT_SUN_DIAMETER);
    }

    #[test]
    fn old_snapshots_and_untrusted_angles_use_safe_directional_defaults() {
        let mut s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.sun_shadow_angular_diameter_deg, DEFAULT_SUN_DIAMETER);
        assert_eq!(s.render_settings().sun_shadow_angular_diameter_deg, DEFAULT_SUN_DIAMETER);
        reset_all(&mut s);
        set_custom_enabled(&mut s, true);
        s.sun_shadow_contact_hardening = true;
        for (value, expected) in [(20.0, 2.0), (-3.0, 0.10), (f32::NAN, DEFAULT_SUN_DIAMETER)] {
            s.sun_shadow_angular_diameter_deg = value;
            assert_eq!(s.render_settings().sun_shadow_angular_diameter_deg, expected);
            assert_eq!(s.clone().sanitized().sun_shadow_angular_diameter_deg, expected);
        }
        s.sun_shadow_angular_diameter_deg = 1.0;
        s.graded_shadow_distance = 123;
        commit_edits(&mut s);
        let mut wire = serde_json::to_value(&s).unwrap();
        for key in ["sun_shadow_contact_hardening", "sun_shadow_angular_diameter_deg"] {
            wire.as_object_mut().unwrap().remove(key);
            wire["shadow_profile"]["slots"]["High"]["tuning"]
                .as_object_mut()
                .unwrap()
                .remove(key);
        }
        wire["shadow_profile"].as_object_mut().unwrap().remove("sun_recipe");
        wire["shadow_profile"].as_object_mut().unwrap().remove("soft_defaults_selection");
        let mut previous: Settings = serde_json::from_value(wire).unwrap();
        loaded(&mut previous);
        assert!(custom_enabled(&previous));
        assert_eq!(previous.graded_shadow_distance, 123);
        assert_eq!(previous.sun_shadow_angular_diameter_deg, DEFAULT_SUN_DIAMETER);
        assert_eq!(previous.render_settings().sun_shadow_angular_diameter_deg, DEFAULT_SUN_DIAMETER);
    }

    #[test]
    fn previous_viewer_tier_change_keeps_the_new_angle_in_its_original_slot() {
        let mut original = Settings::default();
        reset_all(&mut original);
        set_custom_enabled(&mut original, true);
        original.sun_shadow_contact_hardening = true;
        original.sun_shadow_angular_diameter_deg = 1.25;
        commit_edits(&mut original);
        let mut changed = original.clone();
        select_tier(&mut changed, Tier::Low);
        // A v2.3.9 writer updates known settings and preserves unknown keys.
        let newer = serde_json::to_value(&original).unwrap();
        let mut older = serde_json::to_value(&changed).unwrap();
        older["shadow_profile"].as_object_mut().unwrap().remove("sun_recipe");
        older["shadow_profile"].as_object_mut().unwrap().remove("soft_defaults_selection");
        for key in ["sun_shadow_contact_hardening", "sun_shadow_angular_diameter_deg"] {
            older.as_object_mut().unwrap().remove(key);
        }
        let merged = crate::tolerant::merge_over_newer(&newer, older, &serde_json::Value::Null, &[]);
        let mut reopened: Settings = serde_json::from_value(merged).unwrap();
        loaded(&mut reopened);
        assert_eq!(tier(&reopened), Tier::Low);
        assert_eq!(reopened.render_settings().sun_shadow_angular_diameter_deg, Tier::Low.sun_diameter());
        select_tier(&mut reopened, Tier::High);
        assert_eq!(reopened.render_settings().sun_shadow_angular_diameter_deg, 1.25);
    }

    #[test]
    fn public_tier_selector_routes_measured_recipes_to_the_renderer() {
        let mut s = Settings::default();
        for (tier, resolution, near, far, bend, projectors, diameter) in [
            (Tier::Low, 1024, 2, 48, 1, 1, 0.65),
            (Tier::Medium, 2048, 2, 96, 1, 2, 0.65),
            (Tier::High, 4096, 3, 144, 1, 4, 0.53),
            (Tier::Ultra, 4096, 2, 192, 3, 8, 0.53),
        ] {
            select_tier(&mut s, tier);
            let gpu = s.render_settings();
            assert_eq!((gpu.shadow_resolution, gpu.graded_shadow_detail_metres), (resolution, near));
            assert_eq!(gpu.sun_shadow_angular_diameter_deg, diameter);
            assert!(gpu.graded_sun_shadows && gpu.graded_shadow_edge_smoothing);
            assert_eq!(s.shadow_params(), (resolution, f32::from(far)));
            assert_eq!(s.three_level_shadow_reach(), far);
            assert!(gpu.near_shadow_detail && !gpu.near_shadow_precision);
            assert_eq!(gpu.near_shadow_detail_metres, near);
            assert!(gpu.contact_shadows && !gpu.contact_integrity_hole_repair);
            assert_eq!((gpu.contact_ray_method, gpu.experimental_contact_reach_mode), (0, bend));
            assert_eq!(gpu.experimental_contact_occluder_depth_metres, 0.05);
            assert_eq!(gpu.projector_shadow_limit, projectors);
        }
        select_tier(&mut s, Tier::Off);
        let gpu = s.render_settings();
        assert_eq!(s.shadow_params(), (0, 0.0));
        assert!(!gpu.contact_shadows);
        assert_eq!((gpu.shadow_resolution, gpu.projector_shadow_limit), (0, 0));
    }

    #[test]
    fn v238_fixed_and_untouched_custom_take_the_new_budget_on_reopen() {
        for (base, old_enabled, old_limit, new_limit) in [
            (Tier::Low, false, 2, 1),
            (Tier::Medium, false, 2, 2),
            (Tier::High, true, 1, 4),
            (Tier::Ultra, true, 2, 8),
        ] {
            for custom in [false, true] {
                let mut s = Settings::default();
                reset_all(&mut s);
                select_fixed_tier(&mut s, base);
                set_custom_enabled(&mut s, custom);
                let mut old = serde_json::to_value(&s).unwrap();
                for key in ["projector_recipe", "sun_recipe", "soft_defaults_selection"] {
                    old["shadow_profile"].as_object_mut().unwrap().remove(key);
                }
                old.as_object_mut().unwrap().remove("sun_shadow_contact_hardening");
                old.as_object_mut().unwrap().remove("sun_shadow_angular_diameter_deg");
                old["projector_shadows"] = old_enabled.into();
                old["projector_shadow_limit"] = old_limit.into();
                let mut restored: Settings = serde_json::from_value(old).unwrap();
                loaded(&mut restored);
                assert_eq!(tier(&restored), base);
                assert_eq!(custom_enabled(&restored), custom);
                assert!(!can_reset(&restored));
                assert_eq!(restored.render_settings().projector_shadow_limit, new_limit);
            }
        }
    }

    #[test]
    fn ultra_overlap_follows_selected_tier_through_custom_map_edits_and_photo() {
        let mut s = Settings::default();
        select_tier(&mut s, Tier::High);
        set_custom_enabled(&mut s, true);
        s.set_shadow_map_size(4096);
        commit_edits(&mut s);
        assert_eq!(s.shadow_quality, 4);
        assert!(!s.render_settings().graded_shadow_overlap);

        s.photo_held = Some(crate::snapshot::quality::boost(&mut s));
        assert!(s.render_settings().graded_shadow_overlap);
        s.photo_held.take().unwrap().restore(&mut s);
        assert!(!s.render_settings().graded_shadow_overlap);

        select_tier(&mut s, Tier::Ultra);
        assert!(s.render_settings().graded_shadow_overlap);
        s.graded_sun_shadows = false;
        assert!(!s.render_settings().graded_shadow_overlap);
        select_tier(&mut s, Tier::Off);
        assert!(!s.render_settings().graded_shadow_overlap);
    }

    #[test]
    fn coverage_labels_and_whole_viewer_matching_follow_effective_values() {
        let mut s = Settings::default();
        reset_all(&mut s);
        assert!(matches_global(&s, Tier::High));
        set_custom_enabled(&mut s, true);
        set_coverage(&mut s, 0);
        assert_eq!(active_coverage(&s), Some(0));
        assert_eq!(coverage_distance(&s, 0), s.graded_shadow_distance);
        assert!(!matches_global(&s, Tier::High));
        set_advanced_enabled(&mut s, true);
        s.graded_shadow_distance = 91;
        commit_edits(&mut s);
        assert_eq!(active_coverage(&s), None);
        reset_to_base(&mut s);
        assert!(matches_global(&s, Tier::High));
        s.experimental_contact_occluder_depth_metres = 0.27;
        assert!(!matches_global(&s, Tier::High));
        select_tier(&mut s, Tier::Off);
        assert!(!s.contact_shadows && !s.projector_shadows);
        assert!(matches_global(&s, Tier::Off));
    }

    #[test]
    fn off_can_arm_custom_without_turning_on_any_shadow_maps() {
        let mut s = Settings::default();
        reset_all(&mut s);
        select_tier(&mut s, Tier::Off);
        set_custom_enabled(&mut s, true);
        assert_eq!(tier(&s), Tier::Off);
        assert!(custom_enabled(&s) && !s.shadows && !can_reset(&s));
        let gpu = s.render_settings();
        assert_eq!((gpu.shadow_resolution, gpu.projector_shadow_limit), (0, 0));
        assert!(!gpu.contact_shadows);
        let mut restored: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        loaded(&mut restored);
        assert_eq!(tier(&restored), Tier::Off);
        assert!(custom_enabled(&restored) && !restored.shadows);
        select_tier(&mut restored, Tier::Low);
        assert!(custom_enabled(&restored) && restored.shadows);
        assert_eq!(Tuning::read(&restored), Tier::Low.recipe());
    }
}
