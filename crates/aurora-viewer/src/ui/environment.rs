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
    /// "Shared environment" (Reset, or that row in a list).
    Shared,
    /// "Based on day cycle" / day list "Nothing": drop that list's local override.
    Clear(SettingsKind),
    /// A time of day preset (1..=4).
    Preset(u8),
}

/// A sky / water / day list choice (asset or one of the two fixed rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPick {
    Item(Uuid),
    Shared,
    DayBased,
}

/// LLFloaterEnvironmentAdjust slider scales.
const SLIDER_SCALE_SUN_AMBIENT: f32 = 3.0;
const SLIDER_SCALE_BLUE_HORIZON_DENSITY: f32 = 2.0;
const SLIDER_SCALE_GLOW_R: f32 = 20.0;
const SLIDER_SCALE_GLOW_B: f32 = -5.0;
/// F_APPROXIMATELY_ZERO (llmath.h).
const APPROXIMATELY_ZERO: f32 = 0.00001;

/// Fallback when fonts are not ready yet; real width comes from
/// [`label_column_w`] so "Sky environment" / "Water environment" are not
/// painted over the combo (#3109).
const LABEL_W: f32 = 74.0;
const COMBO_W: f32 = 216.0;
/// Height of a list's entries under its search field.
const LIST_H: f32 = 420.0;
const LABEL_FONT: f32 = 12.5;

/// Widest of `labels` at the selector's label size, so the column fits the
/// current language instead of a fixed English guess.
fn label_column_w(ui: &egui::Ui, labels: &[&str]) -> f32 {
    let font = egui::FontId::proportional(LABEL_FONT);
    labels
        .iter()
        .map(|label| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap((*label).to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                    .size()
                    .x
            })
        })
        .fold(LABEL_W, f32::max)
        .ceil()
}

/// Label in a fixed column, clipped so a long string cannot cover the combo.
fn paint_selector_label(ui: &mut egui::Ui, p: &Palette, label: &str, tip: &str, label_w: f32) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(label_w, 20.0), egui::Sense::hover());
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(LABEL_FONT),
        p.ink,
    );
    if !tip.is_empty() {
        resp.on_hover_text(tip);
    }
}

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
    /// Where the real night sky is seen from (kept by the app in the
    /// settings; `None` with the Legacy Viewer's stars).
    pub night: Option<NightSkyPick>,
    /// Open this list's drop-down on the next frame (demo captures).
    pub open_list: Option<SettingsKind>,
    /// Type this into an open list's search field (demo captures).
    pub search_list: Option<(SettingsKind, &'static str)>,
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
            Vec2::new(480.0, 300.0),
        )
        .fixed()
        .help(aurora_i18n::tr!("ui-environment-choose-a-sky-water-or-day-cycle-from-your"))
        .show(ctx, p, open, |ui| {
            self.selector_body(ui, p, view, loading, Some(time_of_day), LABEL_W, lighting, &mut out);
        });
        out
    }

    /// The lists, "Personal lighting…", the reset button and (with
    /// `time_of_day`) the time presets, shared by the Environment window and
    /// the Quick preferences window (the Legacy Viewer's has no presets);
    /// `label_w` is the width of the labels' column.
    #[allow(clippy::too_many_arguments)]
    pub fn selector_body(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        view: LocalView,
        loading: bool,
        time_of_day: Option<u8>,
        label_w: f32,
        lighting: &mut bool,
        out: &mut Vec<EnvAction>,
    ) {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
        let sky = aurora_i18n::tr!("ui-environment-sky-environment");
        let water = aurora_i18n::tr!("ui-environment-water-environment");
        let day = aurora_i18n::tr!("extra-day-night");
        let time = aurora_i18n::tr!("ui-environment-time");
        // Quick preferences passes a wide column; the Environment window's
        // 74 px was too narrow for English "Water environment" (#3109).
        let label_w = label_w.max(label_column_w(ui, &[sky, water, day, time]));
        let rows = [
            (SettingsKind::Sky, sky, aurora_i18n::tr!("ui-environment-sky-settings"), view.sky),
            (
                SettingsKind::Water,
                water,
                aurora_i18n::tr!("ui-environment-water-settings"),
                view.water,
            ),
            (
                SettingsKind::Day,
                day,
                aurora_i18n::tr!("ui-environment-day-cycle-settings"),
                view.day,
            ),
        ];
        for (kind, label, tip, shown) in rows {
            widgets::surface(ui, p, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    paint_selector_label(ui, p, label, tip, label_w);
                    if let Some(a) = self.list_row(ui, p, kind, shown) {
                        out.push(a);
                    }
                });
            });
        }
        ui.add_space(6.0);
        widgets::divider(ui, p);
        ui.horizontal(|ui| {
            ui.add_space(label_w + 4.0);
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
        if let Some(time_of_day) = time_of_day {
            widgets::divider(ui, p);
            ui.horizontal(|ui| {
                paint_selector_label(ui, p, time, "", label_w);
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
        }
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
    }

    /// One list with its < > arrows.
    fn list_row(&mut self, ui: &mut egui::Ui, p: &Palette, kind: SettingsKind, shown: Shown) -> Option<EnvAction> {
        let salt = ("env_list", kind as u8);
        // opened by `settings_list` itself: its combo sits in a child `Ui`,
        // whose ids are not this one's
        if self.open_list == Some(kind) {
            self.open_list = None;
            ui.data_mut(|d| d.insert_temp(demo_open_id(&salt), String::new()));
        }
        if let Some((k, text)) = self.search_list
            && k == kind
        {
            self.search_list = None;
            ui.data_mut(|d| d.insert_temp(demo_open_id(&salt), text.to_owned()));
        }
        match settings_list(ui, p, &self.catalog, kind, shown, salt, COMBO_W)? {
            SettingsPick::Item(id) => Some(EnvAction::Pick(kind, id)),
            SettingsPick::Shared => Some(EnvAction::Shared),
            SettingsPick::DayBased => Some(EnvAction::Clear(kind)),
        }
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
            if let Some(night) = &mut self.night {
                night_sky_rows(ui, p, night);
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

/// Personal Lighting's night sky rows: the same place, latitude and
/// longitude as
/// Preferences › Graphics › Environment (world/night_sky.rs), so a photo's
/// stars can be set from the photo window's light panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightSkyPick {
    pub place: crate::world::night_sky::SkyPlace,
    pub latitude: f32,
    pub longitude: f32,
    /// The longitude counts (the star hour is the real time).
    pub real_time: bool,
    /// Changed this frame.
    pub changed: bool,
}

fn night_sky_rows(ui: &mut egui::Ui, p: &Palette, night: &mut NightSkyPick) {
    use crate::world::night_sky::SkyPlace;
    section(ui, p, aurora_i18n::tr!("ui-options-night-sky"));
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(130.0, 18.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            aurora_i18n::tr!("ui-options-night-sky-place"),
            egui::FontId::proportional(12.0),
            p.ink,
        );
        egui::ComboBox::from_id_salt("personal_lighting_sky_place")
            .width(160.0)
            .selected_text(night.place.label())
            .show_ui(ui, |ui| {
                for place in SkyPlace::ALL {
                    night.changed |= ui.selectable_value(&mut night.place, place, place.label()).changed();
                }
            });
    })
    .response
    .on_hover_text(aurora_i18n::tr!("ui-options-night-sky-place-help"));
    if let Some(latitude) = night.place.latitude() {
        night.latitude = latitude;
    }
    if let Some(longitude) = night.place.longitude() {
        night.longitude = longitude;
    }
    // a preset's latitude is shown, greyed; Custom sets its own
    let custom = night.place == SkyPlace::Custom;
    ui.add_enabled_ui(custom, |ui| {
        night.changed |= slider(
            ui,
            p,
            aurora_i18n::tr!("ui-options-night-sky-latitude"),
            &mut night.latitude,
            -90.0..=90.0,
        )
        .changed();
    });
    // the longitude counts with the real time only (Preferences, "Star hour")
    ui.add_enabled_ui(custom && night.real_time, |ui| {
        night.changed |= slider(
            ui,
            p,
            aurora_i18n::tr!("ui-options-night-sky-longitude"),
            &mut night.longitude,
            -180.0..=180.0,
        )
        .on_hover_text(aurora_i18n::tr!("ui-options-night-sky-longitude-help"))
        .changed();
    });
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
        ui.add(widgets::restricted_number(
            egui::Slider::new(v, range)
                .custom_parser(aurora_i18n::parse_number)
                .step_by(0.01)
                .max_decimals(2)
                .clamping(egui::SliderClamping::Edits)
                .trailing_fill(true),
            widgets::TextPolicy::Decimal,
        ))
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

/// One sky, water or day cycle list with its < > arrows (FloaterQuickPrefs'
/// combos and their Prev / Next buttons), `width` wide: the item picked.
/// Also the photo's Light section.
pub fn settings_list(
    ui: &mut egui::Ui,
    p: &Palette,
    catalog: &SettingsCatalog,
    kind: SettingsKind,
    shown: Shown,
    salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
) -> Option<SettingsPick> {
    let list = catalog.list(kind);
    let second = if kind == SettingsKind::Day {
        aurora_i18n::tr!("common-nothing")
    } else {
        aurora_i18n::tr!("ui-environment-based-on-day-cycle")
    };
    let text = match shown {
        Shown::Shared => aurora_i18n::tr!("ui-environment-shared-environment").to_owned(),
        Shown::DayBased => second.to_owned(),
        Shown::Item(id) => catalog
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
    // `width` at most: a long name is cut, never widening its window
    let size = Vec2::new(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_max_width(width);
        // the search text lives with the list; a list that opens starts it
        // empty, with the field focused so typing filters at once
        let combo_id = ui.make_persistent_id(egui::IdSalt::new(&salt));
        let query_id = combo_id.with("search");
        let demo = ui.data_mut(|d| d.remove_temp::<String>(demo_open_id(&salt)));
        if let Some(text) = &demo {
            egui::Popup::open_id(ui.ctx(), combo_id.with("popup"));
            ui.data_mut(|d| d.insert_temp(query_id, text.clone()));
        }
        let opening = !egui::ComboBox::is_open(ui.ctx(), combo_id) || demo.is_some();
        if opening && demo.is_none() {
            ui.data_mut(|d| d.remove_temp::<String>(query_id));
        }
        let mut query: String = ui.data_mut(|d| d.get_temp(query_id)).unwrap_or_default();
        // No ComboBox `.height`: that wraps the popup in its own scroll and
        // stacked with the list's ScrollArea as a double scrollbar (#3109).
        egui::ComboBox::from_id_salt(salt)
            .width(width)
            .selected_text(RichText::new(text).size(12.0).color(p.ink))
            .truncate()
            // a click in the search field must not close the list
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                // Aurora addition: the Legacy Viewer's lists only jump to
                // the first name starting with the letters typed
                // (LLScrollListCtrl::handleUnicodeCharHere). `search_field`
                // with ↑ ↓ left to the list, so ↓ reaches the names.
                ui.set_min_width(width);
                let search = {
                    let width = ui.available_width();
                    crate::ui::widgets::search_text_edit(ui, p, &mut query, |edit| {
                        edit.hint_text(aurora_i18n::tr!("ui-environment-search-by-name"))
                            .desired_width(width)
                            .event_filter(egui::EventFilter {
                                horizontal_arrows: true,
                                vertical_arrows: false,
                                ..Default::default()
                            })
                    })
                };
                if opening {
                    search.request_focus();
                }
                let matches = env_select::matching(list, &query);
                // Enter takes the first match
                let enter = search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter && let Some(c) = matches.first() {
                    if shown != Shown::Item(c.asset) {
                        picked = Some(SettingsPick::Item(c.asset));
                    }
                    ui.close();
                }
                // Sticky search above one list: keep LIST_H even after a
                // filter is cleared, so the popup matches the first open
                // (auto_shrink would leave a short panel).
                egui::ScrollArea::vertical()
                    .max_height(LIST_H)
                    .min_scrolled_height(LIST_H)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if query.trim().is_empty() {
                            // Aurora: Shared and DayBased / Nothing are
                            // clickable (Legacy greys them with
                            // setDefaultPresetsEnabled(false) and only Reset
                            // returns to Shared; the list entries are clearer).
                            let fixed = [
                                (SettingsPick::Shared, aurora_i18n::tr!("ui-environment-shared-environment")),
                                (SettingsPick::DayBased, second),
                            ];
                            for (choice, label) in fixed {
                                let on = matches!(
                                    (choice, shown),
                                    (SettingsPick::Shared, Shown::Shared) | (SettingsPick::DayBased, Shown::DayBased)
                                );
                                if widgets::search_list_item(ui, on, RichText::new(label).size(12.0)).clicked() {
                                    if !on {
                                        picked = Some(choice);
                                    }
                                    ui.close();
                                }
                            }
                            widgets::divider(ui, p);
                        } else if matches.is_empty() {
                            // cut to the list's width: the drop-down does not wrap,
                            // and a long query would widen it
                            ui.add(
                                egui::Label::new(
                                    RichText::new(aurora_i18n::tr!("ui-environment-no-match", query = query.trim().to_string()))
                                        .size(12.0)
                                        .color(p.muted),
                                )
                                .truncate(),
                            );
                        }
                        for c in matches {
                            let on = shown == Shown::Item(c.asset);
                            if widgets::search_list_item(ui, on, RichText::new(&c.name).size(12.0)).clicked() {
                                if !on {
                                    picked = Some(SettingsPick::Item(c.asset));
                                }
                                ui.close();
                            }
                        }
                    });
            });
        ui.data_mut(|d| d.insert_temp(query_id, query));
    });
    if widgets::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("common-previous"), !list.is_empty()) {
        picked = env_select::step(list, env_select::shown_index(list, shown), false).map(SettingsPick::Item);
    }
    if widgets::icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("common-next"), !list.is_empty()) {
        picked = env_select::step(list, env_select::shown_index(list, shown), true).map(SettingsPick::Item);
    }
    picked
}

/// A list to open on its next frame, with this text in its search field
/// (demo captures).
fn demo_open_id(salt: &(impl std::hash::Hash + std::fmt::Debug)) -> egui::Id {
    egui::Id::unique("env_list_demo_open").with(salt)
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

    #[test]
    fn selector_label_column_fits_water_environment() {
        let ctx = egui::Context::default();
        let mut width = 0.0;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    width = label_column_w(
                        ui,
                        &[
                            aurora_i18n::tr!("ui-environment-sky-environment"),
                            aurora_i18n::tr!("ui-environment-water-environment"),
                            aurora_i18n::tr!("extra-day-night"),
                            aurora_i18n::tr!("ui-environment-time"),
                        ],
                    );
                });
            },
        );
        assert!(
            width > LABEL_W,
            "English Water environment needs more than the old {LABEL_W} px column, got {width}"
        );
        let water = aurora_i18n::tr!("ui-environment-water-environment");
        let water_w = ctx.fonts_mut(|f| {
            f.layout_no_wrap(water.to_owned(), egui::FontId::proportional(LABEL_FONT), egui::Color32::WHITE)
                .size()
                .x
        });
        assert!(width + 0.5 >= water_w, "column {width} must cover {water} ({water_w})");
    }

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
}
