//! « Environnement » window and « Éclairage personnel ».
//!
//! The selector is Firestorm's quick preferences environment block
//! (FloaterQuickPrefs, newview/quickprefs.cpp, floater_quickprefs.xml:
//! « Env. ciel », « Env. eau », « Jour/Nuit » lists with < > arrows,
//! « Éclairage personnel… » and the reset button), plus Aurora's time of day
//! presets. « Éclairage personnel » follows LLFloaterEnvironmentAdjust
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
    /// « Environnement partagé » (the reset button).
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

const LABEL_W: f32 = 74.0;
const COMBO_W: f32 = 216.0;

#[derive(Default)]
pub struct EnvironmentUi {
    catalog: SettingsCatalog,
    /// Inventory generation the lists were built from, and when.
    built: Option<u64>,
    built_at: Option<std::time::Instant>,
    /// Folders still to load for the lists (0: complete).
    pub scan_left: usize,
    /// « Réinitialiser » of « Éclairage personnel » waits for a confirmation
    /// (PersonalSettingsConfirmReset).
    confirm_reset: bool,
    /// Open this list's drop-down on the next frame (demo captures).
    pub open_list: Option<SettingsKind>,
}

impl EnvironmentUi {
    /// Rebuild the lists when the inventory changed (loadPresets), at most
    /// once a second while folders keep arriving.
    pub fn refresh(&mut self, inv: &Inventory) {
        let due = self.built_at.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) || self.scan_left == 0;
        if self.built != Some(inv.generation) && due {
            self.catalog = SettingsCatalog::build(inv);
            self.built = Some(inv.generation);
            self.built_at = Some(std::time::Instant::now());
        }
    }

    pub fn catalog(&self) -> &SettingsCatalog {
        &self.catalog
    }

    /// The selector window. `lighting` opens « Éclairage personnel ».
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
        Floater::new("environment_selector", "Environnement", pos, Vec2::new(370.0, 230.0))
            .fixed()
            .help("Choisissez un ciel, une eau ou un cycle du jour de votre inventaire ou de la bibliothèque : ils ne s'appliquent qu'à votre vue (environnement local). « Environnement partagé » revient à celui de la région ou de la parcelle.")
            .show(ctx, p, open, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
                let rows = [
                    (SettingsKind::Sky, "Env. ciel", "Paramètres atmosphériques", view.sky),
                    (SettingsKind::Water, "Env. eau", "Paramètres aquatiques", view.water),
                    (SettingsKind::Day, "Jour/Nuit", "Paramètres du cycle journalier", view.day),
                ];
                for (kind, label, tip, shown) in rows {
                    ui.horizontal(|ui| {
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(LABEL_W, 20.0), egui::Sense::hover());
                        ui.painter()
                            .text(rect.left_center(), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), p.ink);
                        resp.on_hover_text(tip);
                        if let Some(a) = self.list_row(ui, p, kind, shown) {
                            out.push(a);
                        }
                    });
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(LABEL_W + 4.0);
                    if widgets::flat_button(ui, p, "Éclairage personnel…")
                        .on_hover_text("Ajuster le ciel actuel pour votre vue uniquement")
                        .clicked()
                    {
                        *lighting = !*lighting;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, p, "arrow-counter-clockwise", "Revenir à l'environnement partagé", true) {
                            out.push(EnvAction::Shared);
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, 20.0), egui::Sense::hover());
                    ui.painter().text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        "Heure",
                        egui::FontId::proportional(12.5),
                        p.ink,
                    );
                    for (i, name) in ["Lever", "Midi", "Coucher", "Minuit"].iter().enumerate() {
                        let preset = i as u8 + 1;
                        let on = time_of_day == preset;
                        let text = RichText::new(*name).size(12.0).color(if on { p.ink } else { p.muted });
                        let b = egui::Button::new(text)
                            .fill(if on { p.violet } else { p.raised })
                            .corner_radius(egui::CornerRadius::same(3))
                            .min_size(Vec2::new(0.0, 20.0));
                        if ui
                            .add(b)
                            .on_hover_text(super::bars::TIME_OF_DAY[preset as usize])
                            .clicked()
                        {
                            // FSRepeatedEnvTogglesShared is off by default: the
                            // same preset again stays
                            out.push(EnvAction::Preset(preset));
                        }
                    }
                });
                let status = if self.scan_left > 0 {
                    format!("Chargement de l'inventaire… ({} dossiers)", self.scan_left)
                } else if loading {
                    "Chargement du réglage…".to_owned()
                } else {
                    let c = &self.catalog;
                    format!("{} ciels · {} eaux · {} cycles du jour", c.sky.len(), c.water.len(), c.day.len())
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
            "Rien"
        } else {
            "Basé sur le cycle du jour"
        };
        let text = match shown {
            Shown::Shared => "Environnement partagé".to_owned(),
            Shown::DayBased => second.to_owned(),
            Shown::Item(id) => self
                .catalog
                .name_of(kind, id)
                .map_or_else(|| "Réglage de l'inventaire".to_owned(), str::to_owned),
            Shown::Preset(n) => super::bars::TIME_OF_DAY
                .get(n as usize)
                .copied()
                .unwrap_or("Heure du jour")
                .to_owned(),
            Shown::Personal => "Éclairage personnel".to_owned(),
        };
        let mut picked = None;
        egui::ComboBox::from_id_salt(salt)
            .width(COMBO_W)
            .height(420.0)
            .selected_text(RichText::new(text).size(12.0).color(p.ink))
            .truncate()
            .show_ui(ui, |ui| {
                // the two fixed entries show the state; they cannot be picked
                // (setDefaultPresetsEnabled(false))
                for (i, label) in ["Environnement partagé", second].into_iter().enumerate() {
                    let on = env_select::shown_index(list, shown) == i && !matches!(shown, Shown::Preset(_) | Shown::Personal);
                    ui.add_enabled(false, egui::Button::selectable(on, RichText::new(label).size(12.0)));
                }
                ui.separator();
                for c in list {
                    let on = shown == Shown::Item(c.asset);
                    if ui.selectable_label(on, RichText::new(&c.name).size(12.0)).clicked() && !on {
                        picked = Some(c.asset);
                    }
                }
            });
        if widgets::icon_button(ui, p, "caret-left", "Précédent", !list.is_empty()) {
            picked = env_select::step(list, env_select::shown_index(list, shown), false);
        }
        if widgets::icon_button(ui, p, "caret-right", "Suivant", !list.is_empty()) {
            picked = env_select::step(list, env_select::shown_index(list, shown), true);
        }
        picked.map(|id| EnvAction::Pick(kind, id))
    }

    /// « Éclairage personnel »: edits the local sky at once. Returns
    /// [`EnvAction::Shared`] when « Réinitialiser » is confirmed.
    pub fn show_lighting(&mut self, ctx: &egui::Context, p: &Palette, eep: &mut EnvSelector, open: &mut bool) -> Option<EnvAction> {
        let mut out = None;
        let screen = ctx.content_rect();
        let pos = egui::pos2(screen.right() - 1150.0, screen.top() + 90.0);
        let was_open = *open;
        Floater::new("personal_lighting", "Éclairage personnel", pos, Vec2::new(360.0, 560.0))
            .fixed()
            .help("Ajuste le ciel pour votre vue uniquement ; les autres ne voient pas ces changements.")
            .show(ctx, p, open, |ui| {
                let Some(mut s) = eep.personal_sky() else {
                    ui.label(RichText::new("Aucun ciel local à modifier.").size(12.0).color(p.muted));
                    return;
                };
                let before = s;
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
                ui.spacing_mut().slider_width = 160.0;
                section(ui, p, "Ciel");
                egui::Grid::new("personal_lighting_colors")
                    .num_columns(4)
                    .spacing(Vec2::new(8.0, 4.0))
                    .show(ui, |ui| {
                        color(ui, p, "Ambiance", &mut s.ambient, SLIDER_SCALE_SUN_AMBIENT);
                        color(ui, p, "Soleil", &mut s.sunlight, SLIDER_SCALE_SUN_AMBIENT);
                        ui.end_row();
                        color(ui, p, "Bleu horizon", &mut s.blue_horizon, SLIDER_SCALE_BLUE_HORIZON_DENSITY);
                        color(ui, p, "Densité bleu", &mut s.blue_density, SLIDER_SCALE_BLUE_HORIZON_DENSITY);
                        ui.end_row();
                        color(ui, p, "Nuages", &mut s.cloud_color, 1.0);
                        ui.end_row();
                    });
                slider(ui, p, "Horizon de la brume", &mut s.haze_horizon, 0.0..=5.0);
                slider(ui, p, "Densité de la brume", &mut s.haze_density, 0.0..=5.0);
                slider(ui, p, "Couverture nuageuse", &mut s.cloud_shadow, 0.0..=1.0);
                slider(ui, p, "Taille des nuages", &mut s.cloud_scale, 0.01..=3.0);
                let mut ambiance = s.probe_ambiance;
                if slider(ui, p, "Ambiance de la sonde", &mut ambiance, 0.0..=10.0)
                    .on_hover_text("L'intensité de l'éclairage indirect basé sur l'environnement. À zéro, l'échelle HDR devient Luminosité")
                    .changed()
                {
                    // setReflectionProbeAmbiance: the sky is no longer « classic »
                    s.probe_ambiance = ambiance;
                    s.can_auto_adjust = false;
                }
                // updateGammaLabel
                let hdr = s.probe_ambiance != 0.0;
                let r = slider(ui, p, if hdr { "Échelle HDR" } else { "Luminosité" }, &mut s.gamma, 0.0..=20.0);
                if hdr {
                    r.on_hover_text("Intensité des effets d'éclairage tels que les ciels lumineux réalistes et l'exposition dynamique. 1.0 est la valeur par défaut, 0 correspond à la désactivation, les valeurs entre 0 et 1 mélangent la lumière ambiante et la lumière HDR.");
                }
                section(ui, p, "Soleil");
                let (mut az, mut el) = azimuth_elevation(s.sun_rotation);
                let a = slider(ui, p, "Azimut", &mut az, 0.0..=359.99).changed();
                let e = slider(ui, p, "Élévation", &mut el, -90.0..=90.0).changed();
                if a || e {
                    s.sun_rotation = rotation_from(az, el);
                }
                slider(ui, p, "Échelle", &mut s.sun_scale, 0.25..=20.0);
                // glow: 40..0.2 shown as 0..1.99 (size), focus scaled by -5
                let mut focus = s.glow.z / SLIDER_SCALE_GLOW_B;
                let mut size = 2.0 - s.glow.x / SLIDER_SCALE_GLOW_R;
                let f = slider(ui, p, "Netteté de l'éclat", &mut focus, -2.0..=2.0).changed();
                let z = slider(ui, p, "Taille de l'éclat", &mut size, 0.0..=1.99).changed();
                if f || z {
                    s.glow = Vec3::new((2.0 - size) * SLIDER_SCALE_GLOW_R, 0.0, focus * SLIDER_SCALE_GLOW_B);
                }
                slider(ui, p, "Brillance des étoiles", &mut s.star_brightness, 0.0..=500.0);
                section(ui, p, "Lune");
                let (mut az, mut el) = azimuth_elevation(s.moon_rotation);
                let a = slider(ui, p, "Azimut", &mut az, 0.0..=359.99).changed();
                let e = slider(ui, p, "Élévation", &mut el, -90.0..=90.0).changed();
                if a || e {
                    s.moon_rotation = rotation_from(az, el);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if self.confirm_reset {
                        ui.label(
                            RichText::new("Revenir à l'environnement partagé ?")
                                .size(12.0)
                                .color(p.ink),
                        );
                        if widgets::flat_button(ui, p, "Oui").clicked() {
                            self.confirm_reset = false;
                            out = Some(EnvAction::Shared);
                        }
                        if widgets::flat_button(ui, p, "Non").clicked() {
                            self.confirm_reset = false;
                        }
                    } else if widgets::flat_button(ui, p, "Réinitialiser")
                        .on_hover_text("Fermer et repasser à l'environnement partagé")
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
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(p.violet_light));
}

fn slider(ui: &mut egui::Ui, p: &Palette, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(130.0, 18.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            p.ink,
        );
        ui.add(egui::Slider::new(v, range).step_by(0.01).max_decimals(2).trailing_fill(true))
    })
    .inner
}

/// A label and its color swatch (LLColorSwatchCtrl) showing `value / scale`,
/// two grid cells.
fn color(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut Vec3, scale: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(122.0, 20.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        p.ink,
    );
    let mut c = (*value / scale).clamp(Vec3::ZERO, Vec3::ONE).to_array();
    ui.spacing_mut().interact_size = Vec2::new(44.0, 18.0);
    if egui::color_picker::color_edit_button_rgb(ui, &mut c).changed() {
        *value = Vec3::from_array(c) * scale;
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
