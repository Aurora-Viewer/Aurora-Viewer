//! "Quick preferences" (the toolbar's `quickprefs` command): the settings
//! worth a button, in one small window.
//!
//! Port of FloaterQuickPrefs (newview/quickprefs.cpp, floater_quickprefs.xml,
//! app_settings/quick_preferences.xml, originally LGPL 2.1). From top to
//! bottom, like the Legacy Viewer: the user-defined rows (here seven of its
//! nine default ones, each bound to the typed setting Preferences edits), a
//! line, "Hover Height", "Max Bandwidth", "Max Complexity" (with "No Limit" at the
//! end of the slider), then the environment block (Sky, Water, Day Cycle,
//! "Personal Lighting…" and the return to the shared environment, shared
//! with the Environment window: `EnvironmentUi::selector_body`).
//!
//! A row changes the very field its Preferences row edits, with the same
//! range and the same clamp (`Settings::sanitized`), and the app applies and
//! saves the change like a change of Preferences, so the two never differ.
//! Aurora's ranges win over the Legacy Viewer's (draw distance 32..512 m
//! against 32..1024, volume LOD 0.5..4 against 0..4, fully rendered avatars
//! 1..100 against 1..66, bandwidth 50..4000 against 50..3000). Left out for
//! lack of an Aurora setting: FSNameTagZOffsetCorrection ("Tags Offset") and
//! DebugShowColor ("Color Under Cursor"). Not here yet: the editor of the
//! rows (any debug setting, add / remove / reorder, QuickPrefsEditMode).

use super::environment::{EnvAction, EnvironmentUi};
use super::widgets::{self, Floater};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::world::eep_env::LocalView;
use crate::world::hover_height::{self, HoverHeight};
use egui::Vec2;
use std::ops::RangeInclusive;

/// The window's id (Floater.Toggle "quickprefs").
pub const WINDOW: &str = "quickprefs";

/// The labels' column, for the rows and the environment block alike: wide
/// enough for the longest label (French "Complexité maximale des avatars").
const LABEL_W: f32 = 212.0;
const WIDTH: f32 = 540.0;
const SLIDER_W: f32 = 180.0;

/// A numeric row, bound to the setting of the same name in Preferences.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// RenderFarClip (Preferences › Graphics).
    DrawDistance,
    /// RenderMaxPartCount.
    Particles,
    /// RenderAvatarPhysicsLODFactor.
    PhysicsDetail,
    /// RenderVolumeLODFactor.
    Lod,
    /// IndirectMaxNonImpostors.
    Avatars,
    /// ThrottleBandwidthKBPS (Preferences › Network).
    Bandwidth,
}

impl Slot {
    /// The rows of the Legacy Viewer's quick_preferences.xml that Aurora has
    /// a setting for, in its order.
    pub const ROWS: [Slot; 5] = [Slot::DrawDistance, Slot::Particles, Slot::PhysicsDetail, Slot::Lod, Slot::Avatars];

    pub fn label(self) -> &'static str {
        match self {
            Slot::DrawDistance => aurora_i18n::tr!("common-draw-distance"),
            Slot::Particles => aurora_i18n::tr!("common-particles"),
            Slot::PhysicsDetail => aurora_i18n::tr!("ui-options-avatar-physics-detail"),
            Slot::Lod => aurora_i18n::tr!("ui-options-level-of-detail-lod"),
            Slot::Avatars => aurora_i18n::tr!("ui-options-fully-rendered-avatars"),
            Slot::Bandwidth => aurora_i18n::tr!("ui-options-maximum-bandwidth"),
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Slot::DrawDistance => aurora_i18n::tr!("ui-options-objects-beyond-this-distance-are-not-drawn-96-m"),
            Slot::Particles => aurora_i18n::tr!("ui-options-maximum-particles-0-disabled"),
            Slot::PhysicsDetail => aurora_i18n::tr!("ui-options-avatar-physics-detail-help"),
            Slot::Lod => aurora_i18n::tr!("ui-options-lod-factor-for-sl-objects-and-meshes-2-by"),
            Slot::Avatars => aurora_i18n::tr!("ui-options-nearest-avatars-are-fully-rendered-others-use-impostors-periodically"),
            Slot::Bandwidth => aurora_i18n::tr!("ui-options-maximum-bandwidth-hint"),
        }
    }

    /// The slider's range: the one of Preferences.
    pub fn range(self) -> RangeInclusive<f64> {
        match self {
            Slot::DrawDistance => 32.0..=512.0,
            Slot::Particles => 0.0..=8192.0,
            Slot::PhysicsDetail => 0.0..=1.0,
            Slot::Lod => 0.5..=4.0,
            Slot::Avatars => 1.0..=100.0,
            Slot::Bandwidth => 50.0..=f64::from(aurora_net::throttle::DEFAULT_KBPS),
        }
    }

    /// The slider's increment, the one of the Preferences row (none: free).
    pub fn step(self) -> Option<f64> {
        match self {
            Slot::DrawDistance | Slot::Avatars => None,
            Slot::Particles => Some(256.0),
            Slot::PhysicsDetail => Some(0.05),
            Slot::Lod => Some(0.125),
            Slot::Bandwidth => Some(50.0),
        }
    }

    /// Fixed decimals of the Preferences row's value box (none: egui's).
    pub fn decimals(self) -> Option<usize> {
        (self == Slot::PhysicsDetail).then_some(2)
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Slot::DrawDistance => " m",
            Slot::Bandwidth => aurora_i18n::tr!("bandwidth-kbps-suffix"),
            _ => "",
        }
    }

    #[cfg(test)]
    pub fn get(self, s: &Settings) -> f64 {
        match self {
            Slot::DrawDistance => f64::from(s.draw_distance),
            Slot::Particles => f64::from(s.max_particles),
            Slot::PhysicsDetail => f64::from(s.avatar_physics_detail),
            Slot::Lod => f64::from(s.lod_factor),
            Slot::Avatars => f64::from(s.max_avatars),
            Slot::Bandwidth => f64::from(s.bandwidth_kbps),
        }
    }

    /// Sets the row's value, bounded like `Settings::sanitized` bounds it
    /// (the same clamp as the field of Preferences).
    pub fn set(self, s: &mut Settings, value: f64) {
        let value = value.clamp(*self.range().start(), *self.range().end());
        match self {
            Slot::DrawDistance => s.draw_distance = value as f32,
            Slot::Particles => s.max_particles = value.round() as u32,
            Slot::PhysicsDetail => s.avatar_physics_detail = value as f32,
            Slot::Lod => s.lod_factor = value as f32,
            Slot::Avatars => s.max_avatars = value.round() as u32,
            Slot::Bandwidth => s.bandwidth_kbps = value.round() as u32,
        }
    }
}

/// Max Complexity (RenderAvatarMaxComplexity) through the Legacy Viewer's
/// logarithmic slider (IndirectMaxComplexity, 1..101, "No Limit" at the end).
pub fn complexity_position(s: &Settings) -> u32 {
    crate::scene::complexity::slider::from_limit(s.max_complexity)
}

pub fn set_complexity_position(s: &mut Settings, position: u32) {
    s.max_complexity = crate::scene::complexity::slider::to_limit(position);
}

/// AvatarNameTagMode (0 off, 1 on, 2 fade) as Aurora keeps it: the friends'
/// and the other residents' "show name tags" switches and the "briefly" one
/// of Preferences › Avatars. The own tag's switch is RenderNameShowSelf, a
/// setting of its own that the Legacy Viewer's quick preferences leave
/// alone, so this row never touches it. One difference: the Legacy Viewer's
/// Off also hides the own tag (LLVOAvatar::idleUpdateNameTag draws it
/// on RenderNameShowSelf && AvatarNameTagMode); Aurora has no mode apart
/// from these switches to remember, so Off keeps the own tag's choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameTags {
    Off,
    On,
    Brief,
}

impl NameTags {
    pub fn of(s: &Settings) -> NameTags {
        if !(s.show_friend_name_tags || s.show_other_name_tags) {
            NameTags::Off
        } else if s.temporary_name_tags {
            NameTags::Brief
        } else {
            NameTags::On
        }
    }

    /// Off hides the residents' tags (the timer's switch stays as it is); On
    /// and Brief show them, the second only for a while. The own tag keeps
    /// its Preferences switch.
    pub fn apply(self, s: &mut Settings) {
        let shown = self != NameTags::Off;
        s.show_friend_name_tags = shown;
        s.show_other_name_tags = shown;
        match self {
            NameTags::Off => {}
            NameTags::On => s.temporary_name_tags = false,
            NameTags::Brief => s.temporary_name_tags = true,
        }
    }
}

/// What the window asks of the app.
#[derive(Default)]
pub struct Out {
    /// The environment block's choices.
    pub env: Vec<EnvAction>,
    /// A setting changed: apply them like a change of Preferences.
    pub changed: bool,
}

/// The hover height the window edits, and whether the region lets it.
pub struct Hover<'a> {
    pub height: &'a mut HoverHeight,
    pub enabled: bool,
}

#[derive(Default)]
pub struct QuickPrefsUi {
    /// A drag of the hover height is under way (previewed, sent on release).
    hover_editing: bool,
    /// A setting changed and waits for the button to be released to be saved.
    unsaved: bool,
}

impl QuickPrefsUi {
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        settings: &mut Settings,
        hover: Hover<'_>,
        env: &mut EnvironmentUi,
        view: LocalView,
        loading: bool,
        lighting: &mut bool,
    ) -> Out {
        let mut out = Out::default();
        let screen = ctx.content_rect();
        let pos = egui::pos2(screen.right() - WIDTH - 300.0, screen.top() + 90.0);
        let Hover { height, enabled } = hover;
        let enabled = enabled && height.ready();
        Floater::new(WINDOW, aurora_i18n::tr!("toolbar-command-quickprefs"), pos, Vec2::new(WIDTH, 420.0))
            .auto_height()
            .show(ctx, p, open, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                // the user-defined rows
                for slot in Slot::ROWS {
                    out.changed |= slot_row(ui, p, settings, slot);
                }
                out.changed |= name_tags_row(ui, p, settings);
                out.changed |= widgets::switch_labeled(
                    ui,
                    p,
                    &mut settings.lookat.show_targets,
                    aurora_i18n::tr!("ui-options-show-look-at-targets"),
                )
                .changed();
                widgets::divider(ui, p);
                // the fixed part: Hover Height, Max Bandwidth, Max Complexity
                out.changed |= self.hover_row(ui, p, settings, height, enabled);
                out.changed |= slot_row(ui, p, settings, Slot::Bandwidth);
                out.changed |= complexity_row(ui, p, settings);
                widgets::divider(ui, p);
                // Aurora Weather after "Personal lighting", each with its own
                // arrow back to the region's, as in the Environment window
                // (its window is drawn with that one's)
                let mut weather_open = env.weather_open;
                let before = settings.weather.region_weather;
                env.selector_body(
                    ui,
                    p,
                    view,
                    loading,
                    None,
                    LABEL_W,
                    lighting,
                    Some((&mut weather_open, &mut settings.weather)),
                    &mut out.env,
                );
                env.weather_open = weather_open;
                out.changed |= settings.weather.region_weather != before;
            });
        // saved once the button is released, not at every frame of a drag
        self.unsaved |= out.changed;
        if self.unsaved && !ctx.input(|i| i.pointer.any_down()) {
            self.unsaved = false;
            settings.save();
        }
        if self.hover_editing && !ctx.input(|i| i.pointer.any_down()) {
            if enabled {
                height.commit();
            }
            self.hover_editing = false;
        }
        out
    }

    /// HoverHeightSlider: the standing height, previewed while dragged and
    /// sent on release; dragging it ends the automatic adjustment, as in the
    /// Hover Height window.
    fn hover_row(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, height: &mut HoverHeight, enabled: bool) -> bool {
        let mut changed = false;
        let tip = if enabled {
            aurora_i18n::tr!("hover-height-manual-tip")
        } else {
            aurora_i18n::tr!("hover-height-unavailable")
        };
        ui.horizontal(|ui| {
            label(ui, p, aurora_i18n::tr!("ui-context-hover-height"), tip);
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().slider_width = SLIDER_W - 14.0;
                let mut value = height.value();
                let response = widgets::height_slider(ui, p, &mut value, hover_height::MIN..=hover_height::MAX, settings.auto_hover_height)
                    .on_hover_text(tip);
                if response.changed() {
                    changed |= std::mem::replace(&mut settings.auto_hover_height, false);
                    height.preview(value);
                    self.hover_editing = true;
                }
            });
        });
        changed
    }
}

fn label(ui: &mut egui::Ui, p: &Palette, text: &str, tip: &str) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(LABEL_W, 20.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(12.5),
        p.ink,
    );
    response.on_hover_text(tip);
}

/// One numeric row: its label and a slider of the setting's own type, drawn
/// as the Preferences row draws it (same increment, decimals and suffix).
fn slot_row(ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, slot: Slot) -> bool {
    fn add<T: egui::emath::Numeric>(ui: &mut egui::Ui, v: &mut T, slot: Slot) -> bool {
        let range = *slot.range().start()..=*slot.range().end();
        let mut slider = egui::Slider::new(v, T::from_f64(*range.start())..=T::from_f64(*range.end()))
            .custom_parser(aurora_i18n::parse_number)
            .suffix(slot.suffix())
            .trailing_fill(true);
        if let Some(step) = slot.step() {
            slider = slider.step_by(step);
        }
        if let Some(decimals) = slot.decimals() {
            slider = slider.fixed_decimals(decimals);
        }
        ui.add(slider).changed()
    }
    ui.horizontal(|ui| {
        label(ui, p, slot.label(), slot.tip());
        ui.spacing_mut().slider_width = SLIDER_W;
        // the value is written through `set`, bounded like Preferences' field
        macro_rules! edit {
            ($field:ident) => {{
                let mut value = settings.$field;
                let changed = add(ui, &mut value, slot);
                if changed {
                    slot.set(settings, f64::from(value));
                }
                changed
            }};
        }
        match slot {
            Slot::DrawDistance => edit!(draw_distance),
            Slot::Particles => edit!(max_particles),
            Slot::PhysicsDetail => edit!(avatar_physics_detail),
            Slot::Lod => edit!(lod_factor),
            Slot::Avatars => edit!(max_avatars),
            Slot::Bandwidth => edit!(bandwidth_kbps),
        }
    })
    .inner
}

/// IndirectMaxComplexity with its "No Limit" text.
fn complexity_row(ui: &mut egui::Ui, p: &Palette, settings: &mut Settings) -> bool {
    use crate::scene::complexity::slider;
    let mut changed = false;
    ui.horizontal(|ui| {
        label(
            ui,
            p,
            aurora_i18n::tr!("ui-options-maximum-avatar-complexity"),
            aurora_i18n::tr!("ui-options-above-this-limit-the-avatar-appears-as-a-gray"),
        );
        ui.spacing_mut().slider_width = SLIDER_W;
        let mut position = complexity_position(settings);
        let text = |v: f64| {
            let limit = slider::to_limit(v.round() as u32);
            if limit == 0 {
                aurora_i18n::tr!("ui-options-unlimited").to_owned()
            } else {
                format!("{} k", limit / 1000)
            }
        };
        let response = ui.add(
            egui::Slider::new(&mut position, 1..=slider::OFF)
                .custom_parser(aurora_i18n::parse_number)
                .custom_formatter(move |v, _| text(v))
                .trailing_fill(true),
        );
        if response.changed() {
            set_complexity_position(settings, position);
            changed = true;
        }
    });
    changed
}

/// AvatarNameTagMode: Off, On or Briefly.
fn name_tags_row(ui: &mut egui::Ui, p: &Palette, settings: &mut Settings) -> bool {
    let mut mode = NameTags::of(settings);
    let mut changed = false;
    ui.horizontal(|ui| {
        label(
            ui,
            p,
            aurora_i18n::tr!("ui-options-name-tags"),
            aurora_i18n::tr!("ui-options-temporary-tags-help"),
        );
        for (value, text) in [
            (NameTags::Off, aurora_i18n::tr!("quickprefs-name-tags-off")),
            (NameTags::On, aurora_i18n::tr!("quickprefs-name-tags-on")),
            (NameTags::Brief, aurora_i18n::tr!("quickprefs-name-tags-brief")),
        ] {
            changed |= widgets::radio_value(ui, p, &mut mode, value, text).changed();
        }
    });
    if changed {
        mode.apply(settings);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Slot; 6] = [
        Slot::DrawDistance,
        Slot::Particles,
        Slot::PhysicsDetail,
        Slot::Lod,
        Slot::Avatars,
        Slot::Bandwidth,
    ];

    /// What Preferences does to a field: the slider writes the value, the
    /// settings are bounded when they are applied or loaded.
    fn through_preferences(slot: Slot, value: f64) -> Settings {
        let mut s = Settings::default();
        match slot {
            Slot::DrawDistance => s.draw_distance = value as f32,
            Slot::Particles => s.max_particles = value as u32,
            Slot::PhysicsDetail => s.avatar_physics_detail = value as f32,
            Slot::Lod => s.lod_factor = value as f32,
            Slot::Avatars => s.max_avatars = value as u32,
            Slot::Bandwidth => s.bandwidth_kbps = value as u32,
        }
        s.sanitized()
    }

    #[test]
    fn a_row_gives_the_value_preferences_gives_for_any_input() {
        for slot in ALL {
            let (lo, hi) = (*slot.range().start(), *slot.range().end());
            for value in [lo - 1000.0, lo, ((lo + hi) / 2.0).floor(), hi, hi + 1000.0] {
                let mut s = Settings::default();
                slot.set(&mut s, value);
                let expected = through_preferences(slot, value);
                assert_eq!(slot.get(&s), slot.get(&expected), "{slot:?} at {value}");
            }
        }
    }

    #[test]
    fn the_defaults_sit_inside_every_row_and_the_range_ends_are_kept() {
        let s = Settings::default();
        for slot in ALL {
            assert!(slot.range().contains(&slot.get(&s)), "{slot:?}");
            let mut t = Settings::default();
            slot.set(&mut t, *slot.range().end());
            assert_eq!(slot.get(&t), *slot.range().end(), "{slot:?} keeps its top");
        }
    }

    #[test]
    fn max_complexity_follows_the_legacy_slider_and_no_limit_means_zero() {
        let mut s = Settings::default();
        set_complexity_position(&mut s, crate::scene::complexity::slider::OFF);
        assert_eq!(s.max_complexity, 0);
        assert_eq!(complexity_position(&s), crate::scene::complexity::slider::OFF);
        set_complexity_position(&mut s, 1);
        assert_eq!(s.max_complexity, 20_000);
        assert_eq!(s.clone().sanitized().max_complexity, s.max_complexity);
    }

    #[test]
    fn name_tag_modes_map_onto_the_preferences_switches() {
        let mut s = Settings::default();
        assert_eq!(NameTags::of(&s), NameTags::On);
        NameTags::Brief.apply(&mut s);
        assert!(s.temporary_name_tags && s.show_friend_name_tags && s.show_other_name_tags);
        assert_eq!(NameTags::of(&s), NameTags::Brief);
        NameTags::Off.apply(&mut s);
        assert!(!(s.show_friend_name_tags || s.show_other_name_tags));
        assert_eq!(NameTags::of(&s), NameTags::Off);
        NameTags::On.apply(&mut s);
        assert!(!s.temporary_name_tags);
        assert_eq!(NameTags::of(&s), NameTags::On);
        // the tags hidden one by one in Preferences: Off only when both are
        s.show_other_name_tags = false;
        assert_eq!(NameTags::of(&s), NameTags::On);
    }

    /// The window keeps its size, the environment block's status line, its
    /// last text, inside it.
    #[test]
    fn the_window_keeps_its_size() {
        let status = aurora_i18n::tr!("ui-environment-skies-waters-day-cycles", value0 = "0", value1 = "0", value2 = "0");
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[egui::vec2(WIDTH, 420.0)],
            &status,
            || {
                (
                    QuickPrefsUi::default(),
                    Settings::default(),
                    HoverHeight::default(),
                    EnvironmentUi::default(),
                )
            },
            |(quick, settings, height, env), ctx, p| {
                let mut open = true;
                let mut lighting = false;
                quick.show(
                    ctx,
                    p,
                    &mut open,
                    settings,
                    Hover { height, enabled: true },
                    env,
                    LocalView::SHARED,
                    false,
                    &mut lighting,
                );
            },
        );
    }

    #[test]
    fn name_tag_modes_leave_the_own_tag_switch_alone() {
        // RenderNameShowSelf is its own setting: the quick preferences set
        // AvatarNameTagMode only
        for own in [false, true] {
            let mut s = Settings {
                show_own_name_tag: own,
                ..Settings::default()
            };
            for mode in [NameTags::Off, NameTags::On, NameTags::Brief, NameTags::On] {
                mode.apply(&mut s);
                assert_eq!(s.show_own_name_tag, own, "{mode:?} with own tag {own}");
                assert_eq!(NameTags::of(&s), mode, "own tag {own}");
            }
        }
    }
}
