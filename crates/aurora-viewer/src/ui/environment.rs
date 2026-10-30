//! "Environment" window and "Personal lighting".
//!
//! The selector is Firestorm's quick preferences environment block
//! (FloaterQuickPrefs, newview/quickprefs.cpp, floater_quickprefs.xml:
//! "Sky environment", "Water environment", "Day/Night" lists with < > arrows,
//! "Personal lighting…" and the reset button), plus Aurora's time of day
//! presets. "Personal lighting" follows LLFloaterEnvironmentAdjust
//! (newview/llfloaterenvironmentadjust.cpp, floater_adjust_environment.xml):
//! the same controls, ranges and scales, on the local sky. Both originally
//! LGPL 2.1.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::eep::SkyFrame;
use crate::world::eep_env::{EnvSelector, LocalView};
use crate::world::env_select::{self, SettingsCatalog, SettingsKind, Shown};
use crate::world::inventory::Inventory;
use egui::{RichText, Vec2};
use glam::{Quat, Vec3};
use uuid::Uuid;

/// What the window asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvAction {
    /// Apply a settings item from a list.
    Pick(SettingsKind, Uuid),
    /// "Shared environment" (the reset button).
    Shared,
    /// A time of day preset (1..=4).
    Preset(u8),
}

/// LLFloaterEnvironmentAdjust slider scales.
const SLIDER_SCALE_SUN_AMBIENT: f32 = 3.0;
const SLIDER_SCALE_BLUE_HORIZON_DENSITY: f32 = 2.0;
const SLIDER_SCALE_GLOW_R: f32 = 20.0;
const SLIDER_SCALE_GLOW_B: f32 = -5.0;
/// F_APPROXIMATELY_ZERO (llmath.h).
const APPROXIMATELY_ZERO: f32 = 0.00001;

/// Room for "Sky environment" / "Water environment" (EN) without the combo
/// covering the label. FR/JA strings are shorter.
const LABEL_W: f32 = 120.0;
/// Each ‹ › control (`widgets::icon_button`).
const ARROW_W: f32 = 20.0;

#[derive(Default)]
pub struct EnvironmentUi {
    catalog: SettingsCatalog,
    /// Inventory generation the lists were built from, and when.
    built: Option<(u64, super::inventory::view::Sort)>,
    built_at: Option<std::time::Instant>,
    /// Folders still to load for the lists (0: complete).
    pub scan_left: usize,
    /// "Reset" of "Personal lighting" waits for a confirmation
    /// (PersonalSettingsConfirmReset).
    confirm_reset: bool,
    /// The sun and moon beacons of "Personal lighting" (kept by the app).
    pub beacons: [bool; 2],
    /// Open this list's drop-down on the next frame (demo captures).
    pub open_list: Option<SettingsKind>,
}

impl EnvironmentUi {
    /// Rebuild the lists when the inventory changed (loadPresets), at most
    /// once a second while folders keep arriving.
    pub fn refresh(&mut self, ctx: &egui::Context, inv: &Inventory) {
        let version = (inv.generation, super::inventory::view::picker_sort(ctx));
        let due = self.built_at.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) || self.scan_left == 0;
        let sort_changed = self.built.is_some_and(|built| built.1 != version.1);
        if self.built != Some(version) && (due || sort_changed) {
            let order = super::inventory::view::picker_order(ctx, inv);
            self.catalog = SettingsCatalog::build(inv, |id| order.rank(id));
            self.built = Some(version);
            self.built_at = Some(std::time::Instant::now());
        }
    }

    pub fn catalog(&self) -> &SettingsCatalog {
        &self.catalog
    }

    /// The selector window. `lighting` opens "Personal lighting".
    pub fn show_selector(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        view: LocalView,
        loading: bool,
        time_of_day: u8,
        open: &mut bool,
        lighting: &mut bool,
    ) -> Vec<EnvAction> {
        let mut out = Vec::new();
        let screen = ctx.content_rect();
        // left of the notification toasts (top right)
        let pos = egui::pos2(screen.right() - 740.0, screen.top() + 90.0);
        Floater::new(
            "environment_selector",
            aurora_i18n::tr!("common-environment"),
            pos,
            Vec2::new(440.0, 300.0),
        )
        .fixed()
        .help(aurora_i18n::tr!("ui-environment-choose-a-sky-water-or-day-cycle-from-your"))
        .show(ctx, p, open, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
            let rows = [
                (
                    SettingsKind::Sky,
                    aurora_i18n::tr!("ui-environment-sky-environment"),
                    aurora_i18n::tr!("ui-environment-sky-settings"),
                    view.sky,
                ),
                (
                    SettingsKind::Water,
                    aurora_i18n::tr!("ui-environment-water-environment"),
                    aurora_i18n::tr!("ui-environment-water-settings"),
                    view.water,
                ),
                (
                    SettingsKind::Day,
                    aurora_i18n::tr!("extra-day-night"),
                    aurora_i18n::tr!("ui-environment-day-cycle-settings"),
                    view.day,
                ),
            ];
            for (kind, label, tip, shown) in rows {
                widgets::surface(ui, p, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        // Fixed label column so long names cannot paint under
                        // the combo (LABEL_W was 74 px and overflowed EN).
                        ui.add_sized(
                            Vec2::new(LABEL_W, 20.0),
                            egui::Label::new(RichText::new(label).size(12.5).color(p.ink)).truncate(),
                        )
                        .on_hover_text(tip);
                        if let Some(a) = self.list_row(ui, p, kind, shown) {
                            out.push(a);
                        }
                    });
                });
            }
            ui.add_space(6.0);
            widgets::divider(ui, p);
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W + 4.0);
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-personal-lighting"))
                    .on_hover_text(aurora_i18n::tr!("ui-environment-adjust-the-current-sky-for-your-view-only"))
                    .clicked()
                {
                    *lighting = !*lighting;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(
                        ui,
                        p,
                        "restore",
                        aurora_i18n::tr!("ui-environment-return-to-shared-environment"),
                        true,
                    ) {
                        out.push(EnvAction::Shared);
                    }
                });
            });
            widgets::divider(ui, p);
            ui.horizontal(|ui| {
                ui.add_sized(
                    Vec2::new(LABEL_W, 20.0),
                    egui::Label::new(RichText::new(aurora_i18n::tr!("ui-environment-time")).size(12.5).color(p.ink)).truncate(),
                );
                for (i, name) in [
                    aurora_i18n::tr!("ui-environment-sunrise"),
                    aurora_i18n::tr!("common-midday"),
                    aurora_i18n::tr!("ui-environment-lie-down"),
                    aurora_i18n::tr!("common-midnight"),
                ]
                .iter()
                .enumerate()
                {
                    let preset = i as u8 + 1;
                    let on = time_of_day == preset;
                    let text = RichText::new(*name).size(12.0).color(if on { p.on_accent } else { p.muted });
                    let b = egui::Button::new(text)
                        .fill(if on { p.violet } else { p.raised })
                        .corner_radius(ui.visuals().window_corner_radius)
                        .min_size(Vec2::new(0.0, 20.0));
                    if ui.add(b).on_hover_text(super::bars::time_of_day()[preset as usize]).clicked() {
                        // FSRepeatedEnvTogglesShared is off by default: the
                        // same preset again stays
                        out.push(EnvAction::Preset(preset));
                    }
                }
            });
            let status = if self.scan_left > 0 {
                aurora_i18n::tr!("ui-environment-loading-inventory-folders", value0 = format!("{}", self.scan_left))
            } else if loading {
                aurora_i18n::tr!("ui-environment-loading-settings").to_owned()
            } else {
                let c = &self.catalog;
                aurora_i18n::tr!(
                    "ui-environment-skies-waters-day-cycles",
                    value0 = format!("{}", c.sky.len()),
                    value1 = format!("{}", c.water.len()),
                    value2 = format!("{}", c.day.len())
                )
            };
            ui.label(RichText::new(status).size(11.0).color(p.muted_dim));
        });
        out
    }

    /// One list with its < > arrows.
    fn list_row(&mut self, ui: &mut egui::Ui, p: &Palette, kind: SettingsKind, shown: Shown) -> Option<EnvAction> {
        let salt = ("env_list", kind as u8);
        if self.open_list == Some(kind) {
            self.open_list = None;
            let id = ui.make_persistent_id(egui::IdSalt::new(salt));
            egui::Popup::open_id(ui.ctx(), id.with("popup"));
        }
        let list = self.catalog.list(kind);
        let second = if kind == SettingsKind::Day {
            aurora_i18n::tr!("common-nothing")
        } else {
            aurora_i18n::tr!("ui-environment-based-on-day-cycle")
        };
        let text = match shown {
            Shown::Shared => aurora_i18n::tr!("ui-environment-shared-environment").to_owned(),
            Shown::DayBased => second.to_owned(),
            Shown::Item(id) => self
                .catalog
                .name_of(kind, id)
                .map_or_else(|| aurora_i18n::tr!("ui-environment-inventory-settings").to_owned(), str::to_owned),
            Shown::Preset(n) => super::bars::time_of_day()
                .get(n as usize)
                .copied()
                .unwrap_or(aurora_i18n::tr!("common-time-of-day"))
                .to_owned(),
            Shown::Personal => aurora_i18n::tr!("ui-environment-personal-lighting").to_owned(),
        };
        let mut picked = None;
        // Pin ‹ › to the trailing edge (right_to_left), then give the combo
        // only the leftover width. egui ComboBox::width is a minimum: without
        // a max, long windlight names grew the button and the arrows drifted.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("common-next"), !list.is_empty()) {
                picked = env_select::step(list, env_select::shown_index(list, shown), true);
            }
            if widgets::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("common-previous"), !list.is_empty()) {
                picked = env_select::step(list, env_select::shown_index(list, shown), false);
            }
            let combo_w = ui.available_width().max(ARROW_W);
            ui.allocate_ui_with_layout(Vec2::new(combo_w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_max_width(combo_w);
                egui::ComboBox::from_id_salt(salt)
                    .width(combo_w)
                    .height(420.0)
                    .selected_text(RichText::new(text).size(12.0).color(p.ink))
                    .truncate()
                    .show_ui(ui, |ui| {
                        // the two fixed entries show the state; they cannot be picked
                        // (setDefaultPresetsEnabled(false))
                        for (i, label) in [aurora_i18n::tr!("ui-environment-shared-environment"), second]
                            .into_iter()
                            .enumerate()
                        {
                            let on = env_select::shown_index(list, shown) == i && !matches!(shown, Shown::Preset(_) | Shown::Personal);
                            ui.add_enabled(false, egui::Button::selectable(on, RichText::new(label).size(12.0)));
                        }
                        widgets::divider(ui, p);
                        for c in list {
                            let on = shown == Shown::Item(c.asset);
                            if ui.selectable_label(on, RichText::new(&c.name).size(12.0)).clicked() && !on {
                                picked = Some(c.asset);
                            }
                        }
                    });
            });
        });
        picked.map(|id| EnvAction::Pick(kind, id))
    }

    /// "Personal lighting": edits the local sky at once. Returns
    /// [`EnvAction::Shared`] when "Reset" is confirmed.
    pub fn show_lighting(&mut self, ctx: &egui::Context, p: &Palette, eep: &mut EnvSelector, open: &mut bool) -> Option<EnvAction> {
        let mut out = None;
        let screen = ctx.content_rect();
        let pos = egui::pos2(screen.right() - 1150.0, screen.top() + 90.0);
        let was_open = *open;
        Floater::new(
            "personal_lighting",
            aurora_i18n::tr!("ui-environment-personal-lighting"),
            pos,
            Vec2::new(400.0, 560.0),
        )
        .fixed()
        .help(aurora_i18n::tr!("ui-environment-adjusts-the-sky-for-your-view-only-other-people"))
        .show(ctx, p, open, |ui| {
            let Some(mut s) = eep.personal_sky() else {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-environment-no-local-sky-to-edit"))
                        .size(12.0)
                        .color(p.muted),
                );
                return;
            };
            let before = s;
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            ui.spacing_mut().slider_width = 160.0;
            section(ui, p, aurora_i18n::tr!("ui-environment-sky"));
            egui::Grid::new("personal_lighting_colors")
                .num_columns(4)
                .spacing(Vec2::new(8.0, 4.0))
                .show(ui, |ui| {
                    color(ui, p, aurora_i18n::tr!("common-ambient"), &mut s.ambient, SLIDER_SCALE_SUN_AMBIENT);
                    color(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-environment-sun"),
                        &mut s.sunlight,
                        SLIDER_SCALE_SUN_AMBIENT,
                    );
                    ui.end_row();
                    color(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-environment-blue-horizon"),
                        &mut s.blue_horizon,
                        SLIDER_SCALE_BLUE_HORIZON_DENSITY,
                    );
                    color(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-environment-blue-density"),
                        &mut s.blue_density,
                        SLIDER_SCALE_BLUE_HORIZON_DENSITY,
                    );
                    ui.end_row();
                    color(ui, p, aurora_i18n::tr!("ui-environment-clouds"), &mut s.cloud_color, 1.0);
                    ui.end_row();
                });
            slider(
                ui,
                p,
                aurora_i18n::tr!("ui-environment-haze-horizon"),
                &mut s.haze_horizon,
                0.0..=5.0,
            );
            slider(
                ui,
                p,
                aurora_i18n::tr!("ui-environment-haze-density"),
                &mut s.haze_density,
                0.0..=5.0,
            );
            slider(
                ui,
                p,
                aurora_i18n::tr!("ui-environment-cloud-coverage"),
                &mut s.cloud_shadow,
                0.0..=1.0,
            );
            slider(ui, p, aurora_i18n::tr!("ui-environment-cloud-size"), &mut s.cloud_scale, 0.01..=3.0);
            let mut ambiance = s.probe_ambiance;
            if slider(ui, p, aurora_i18n::tr!("ui-environment-probe-ambiance"), &mut ambiance, 0.0..=10.0)
                .on_hover_text(aurora_i18n::tr!(
                    "ui-environment-intensity-of-indirect-environment-lighting-at-zero-hdr-scale"
                ))
                .changed()
            {
                // setReflectionProbeAmbiance: the sky is no longer "classic"
                s.probe_ambiance = ambiance;
                s.can_auto_adjust = false;
            }
            // updateGammaLabel
            let hdr = s.probe_ambiance != 0.0;
            let r = slider(
                ui,
                p,
                if hdr {
                    aurora_i18n::tr!("ui-environment-hdr-scale")
                } else {
                    aurora_i18n::tr!("common-brightness")
                },
                &mut s.gamma,
                0.0..=20.0,
            );
            if hdr {
                r.on_hover_text(aurora_i18n::tr!(
                    "ui-environment-intensity-of-lighting-effects-such-as-realistic-bright-skies"
                ));
            }
            section(ui, p, aurora_i18n::tr!("ui-environment-sun"));
            // floater_adjust_environment's sunbeacon / moonbeacon
            widgets::checkbox(ui, p, &mut self.beacons[0], aurora_i18n::tr!("ui-env-editor-sun-beacon"));
            let (mut az, mut el) = azimuth_elevation(s.sun_rotation);
            let a = slider(ui, p, aurora_i18n::tr!("ui-environment-azimuth"), &mut az, 0.0..=359.99).changed();
            let e = slider(ui, p, aurora_i18n::tr!("ui-environment-elevation"), &mut el, -90.0..=90.0).changed();
            if a || e {
                s.sun_rotation = rotation_from(az, el);
            }
            slider(ui, p, aurora_i18n::tr!("common-scale"), &mut s.sun_scale, 0.25..=20.0);
            // glow: 40..0.2 shown as 0..1.99 (size), focus scaled by -5
            let mut focus = s.glow.z / SLIDER_SCALE_GLOW_B;
            let mut size = 2.0 - s.glow.x / SLIDER_SCALE_GLOW_R;
            let f = slider(ui, p, aurora_i18n::tr!("ui-environment-glare-sharpness"), &mut focus, -2.0..=2.0).changed();
            let z = slider(ui, p, aurora_i18n::tr!("ui-environment-glow-size"), &mut size, 0.0..=1.99).changed();
            if f || z {
                s.glow = Vec3::new((2.0 - size) * SLIDER_SCALE_GLOW_R, 0.0, focus * SLIDER_SCALE_GLOW_B);
            }
            slider(
                ui,
                p,
                aurora_i18n::tr!("ui-environment-star-brightness"),
                &mut s.star_brightness,
                0.0..=500.0,
            );
            section(ui, p, aurora_i18n::tr!("ui-environment-moon"));
            widgets::checkbox(ui, p, &mut self.beacons[1], aurora_i18n::tr!("ui-env-editor-moon-beacon"));
            let (mut az, mut el) = azimuth_elevation(s.moon_rotation);
            let a = slider(ui, p, aurora_i18n::tr!("ui-environment-azimuth"), &mut az, 0.0..=359.99).changed();
            let e = slider(ui, p, aurora_i18n::tr!("ui-environment-elevation"), &mut el, -90.0..=90.0).changed();
            if a || e {
                s.moon_rotation = rotation_from(az, el);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.confirm_reset {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-environment-return-to-shared-environment-2"))
                            .size(12.0)
                            .color(p.ink),
                    );
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("common-yes")).clicked() {
                        self.confirm_reset = false;
                        out = Some(EnvAction::Shared);
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("common-no")).clicked() {
                        self.confirm_reset = false;
                    }
                } else if widgets::flat_button(ui, p, aurora_i18n::tr!("common-reset"))
                    .on_hover_text(aurora_i18n::tr!("ui-environment-close-and-return-to-shared-environment"))
                    .clicked()
                {
                    self.confirm_reset = true;
                }
            });
            if !same_sky(&before, &s) {
                eep.edit_personal(|sky| {
                    *sky = s;
                    true
                });
            }
        });
        if out.is_some() {
            *open = false;
        }
        if was_open && !*open {
            self.confirm_reset = false;
        }
        out
    }
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(13.0).strong().color(p.ink));
}

pub(super) fn slider(ui: &mut egui::Ui, p: &Palette, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    slider_w(ui, p, label, v, range, 130.0)
}

/// [`slider`] with a wider label column (the sky and water editors).
pub(super) fn slider_w(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label_w: f32,
) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(label_w, 18.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            p.ink,
        );
        // clamped and stepped when moved only: a value off the 0.01 steps
        // (any real sky) stays as it is, and the settings unchanged
        ui.add(
            egui::Slider::new(v, range)
                .custom_parser(aurora_i18n::parse_number)
                .step_by(0.01)
                .max_decimals(2)
                .clamping(egui::SliderClamping::Edits)
                .trailing_fill(true),
        )
    })
    .inner
}

/// A label and its color swatch (LLColorSwatchCtrl) showing `value / scale`,
/// two grid cells.
pub(super) fn color(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut Vec3, scale: f32) {
    color_w(ui, p, label, value, scale, 122.0);
}

/// [`color`] with a wider label column (the sky and water editors).
pub(super) fn color_w(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut Vec3, scale: f32, label_w: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(label_w, 20.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        p.ink,
    );
    // linear components, as egui's color button edited them
    let [r, g, b] = (*value / scale).clamp(Vec3::ZERO, Vec3::ONE).to_array();
    let mut srgba = egui::Color32::from(egui::Rgba::from_rgb(r, g, b)).to_srgba_unmultiplied();
    ui.spacing_mut().interact_size = Vec2::new(44.0, 18.0);
    if super::widgets::color_button(ui, p, ("sky_color", label), &mut srgba, false).changed() {
        let linear = egui::Rgba::from(egui::Color32::from_rgb(srgba[0], srgba[1], srgba[2]));
        *value = Vec3::new(linear.r(), linear.g(), linear.b()) * scale;
    }
}

fn same_sky(a: &SkyFrame, b: &SkyFrame) -> bool {
    a.ambient == b.ambient
        && a.blue_horizon == b.blue_horizon
        && a.blue_density == b.blue_density
        && a.sunlight == b.sunlight
        && a.cloud_color == b.cloud_color
        && a.haze_horizon == b.haze_horizon
        && a.haze_density == b.haze_density
        && a.cloud_shadow == b.cloud_shadow
        && a.cloud_scale == b.cloud_scale
        && a.probe_ambiance == b.probe_ambiance
        && a.can_auto_adjust == b.can_auto_adjust
        && a.gamma == b.gamma
        && a.sun_rotation == b.sun_rotation
        && a.sun_scale == b.sun_scale
        && a.glow == b.glow
        && a.star_brightness == b.star_brightness
        && a.moon_rotation == b.moon_rotation
}

/// Azimuth and elevation (degrees) of a sun or moon rotation
/// (LLVirtualTrackball::getAzimuthAndElevationDeg).
pub fn azimuth_elevation(q: Quat) -> (f32, f32) {
    let d = q * Vec3::X;
    let mut az = if d.x.abs() > APPROXIMATELY_ZERO || d.y.abs() > APPROXIMATELY_ZERO {
        d.x.atan2(d.y)
    } else {
        0.0
    };
    az -= std::f32::consts::FRAC_PI_2;
    if az < 0.0 {
        az += std::f32::consts::TAU;
    }
    (az.to_degrees(), d.z.clamp(-1.0, 1.0).asin().to_degrees())
}

/// The rotation for an azimuth and elevation (degrees): elevation about Y,
/// then azimuth about Z (onSunAzimElevChanged).
pub fn rotation_from(azimuth: f32, elevation: f32) -> Quat {
    let az = azimuth.to_radians();
    let mut el = elevation.to_radians();
    if el.abs() < APPROXIMATELY_ZERO {
        el = APPROXIMATELY_ZERO;
    }
    Quat::from_rotation_z(-az) * Quat::from_rotation_y(-el)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn azimuth_and_elevation_round_trip() {
        for (az, el) in [(0.0f32, 30.0f32), (30.0, 10.0), (135.0, -20.0), (270.0, 60.0), (359.0, 1.0)] {
            let q = rotation_from(az, el);
            let (a, e) = azimuth_elevation(q);
            assert!((a - az).abs() < 0.01 || (a - az).abs() > 359.9, "{az} -> {a}");
            assert!((e - el).abs() < 0.01, "{el} -> {e}");
            // elevation lifts the direction, azimuth turns it clockwise from east
            let d = q * Vec3::X;
            assert!((d.z - el.to_radians().sin()).abs() < 1e-5);
        }
        let d = rotation_from(90.0, 0.0) * Vec3::X;
        assert!(d.y < -0.99, "90° is south of east: {d}");
        let (_, e) = azimuth_elevation(rotation_from(0.0, 90.0));
        assert!((e - 90.0).abs() < 0.1, "{e}");
    }

    /// One picker row as `list_row` lays it out. Returns
    /// `(label_right, combo_left, combo_right, next_arrow_left)`.
    fn row_edges(ui: &mut egui::Ui, p: &crate::theme::Palette, label: &str, selected: &str, salt: u8) -> (f32, f32, f32, f32) {
        let mut label_right = 0.0;
        let mut combo_left = 0.0;
        let mut combo_right = 0.0;
        let mut next_arrow_left = 0.0;
        ui.horizontal(|ui| {
            let r = ui.add_sized(
                Vec2::new(LABEL_W, 20.0),
                egui::Label::new(RichText::new(label).size(12.5).color(p.ink)).truncate(),
            );
            label_right = r.rect.right();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (next, _) = ui.allocate_exact_size(Vec2::splat(ARROW_W), egui::Sense::click());
                next_arrow_left = next.left();
                let _ = ui.allocate_exact_size(Vec2::splat(ARROW_W), egui::Sense::click());
                let combo_w = ui.available_width().max(ARROW_W);
                let combo = ui
                    .allocate_ui_with_layout(Vec2::new(combo_w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_max_width(combo_w);
                        egui::ComboBox::from_id_salt(("env_list", salt))
                            .width(combo_w)
                            .selected_text(RichText::new(selected).size(12.0).color(p.ink))
                            .truncate()
                            .show_ui(ui, |_| {});
                    })
                    .response
                    .rect;
                combo_left = combo.left();
                combo_right = combo.right();
                let _ = p;
            });
        });
        (label_right, combo_left, combo_right, next_arrow_left)
    }

    #[test]
    fn picker_rows_keep_labels_and_arrows_aligned_for_short_and_long_names() {
        let ctx = egui::Context::default();
        Theme::default().apply(&ctx, 1.0);
        let p = Theme::default().palette();
        let mut short = (0.0, 0.0, 0.0, 0.0);
        let mut long = (0.0, 0.0, 0.0, 0.0);
        for _ in 0..3 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 300.0))),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(400.0);
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
                    short = row_edges(ui, &p, "Sky environment", "Nothing", 1);
                    long = row_edges(ui, &p, "Water environment", "_5BAnaLu_5D_20outdoor_20night 1 and then some more", 2);
                },
            );
            out.textures_delta.clear();
        }
        let (short_label, short_combo_l, short_combo_r, short_arrow) = short;
        let (long_label, long_combo_l, long_combo_r, long_arrow) = long;
        assert!(
            (short_arrow - long_arrow).abs() < 1.0,
            "‹ › must share one column for short and long names: short={short_arrow} long={long_arrow}"
        );
        assert!(
            (short_combo_r - long_combo_r).abs() < 1.0,
            "combo right edges must align: short={short_combo_r} long={long_combo_r}"
        );
        assert!(
            (short_label - long_label).abs() < 1.0,
            "labels must share one column: short={short_label} long={long_label}"
        );
        assert!(
            short_combo_l >= short_label - 0.5,
            "short combo must start after the label: combo={short_combo_l} label_right={short_label}"
        );
        assert!(
            long_combo_l >= long_label - 0.5,
            "long combo must start after the label: combo={long_combo_l} label_right={long_label}"
        );
        assert!(
            (short_combo_r - short_combo_l - (long_combo_r - long_combo_l)).abs() < 1.0,
            "combo widths must match regardless of the selected name"
        );
    }
}
