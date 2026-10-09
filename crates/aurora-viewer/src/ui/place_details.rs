//! "Détails de l'emplacement" windows: what a click on a place link opens.
//! Port of FSFloaterPlaceDetails in its "remote_place" mode with
//! LLPanelPlaceProfile set to PLACE (indra/newview/fsfloaterplacedetails.cpp,
//! llpanelplaceinfo.cpp, llpanelplaceprofile.cpp,
//! floater_fs_placedetails.xml, panel_place_profile.xml, originally
//! LGPL 2.1): the parcel snapshot, area and traffic (FIRE-2717), parcel
//! name, "Région : Name (x, y, z)", description, maturity, and the
//! Téléporter / Carte buttons with the overflow menu (Copier la SLurl; "Créer
//! un repère" is hidden for a remote place, STORM-411, and "Créer un favori"
//! is hidden by Firestorm). The owner line and the accordions (parcel,
//! region, estate, for sale) are only shown by Firestorm for the agent's own
//! parcel (setInfoType AGENT), not for a remote place. Network state:
//! `world/place_details.rs`.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::place_details::{Place, PlaceError, Stage};
use aurora_net::ParcelSummary;
use aurora_net::land::RemoteParcelError;
use egui::{RichText, Vec2};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// LLTrans "LoadingData".
const LOADING: &str = "Chargement...";
/// panel_place_profile.xml "not_available".
const NOT_AVAILABLE: &str = "(N\\A)";

pub enum PlaceAction {
    /// onTeleportButtonClicked: gAgent.teleportViaLocation + trackLocation.
    Teleport(DVec3),
    /// onShowOnMapButtonClicked: trackLocation, world map centered on it.
    ShowOnMap(DVec3),
    Close(u64),
}

#[derive(Default)]
pub struct PlaceDetailsUi {
    raise: HashSet<u64>,
    /// Parcel snapshots wanted (streamed by the app like the profiles').
    pub wanted_images: HashSet<Uuid>,
}

/// FSFloaterPlaceDetails "title_remote_place" / "title_remote_place_detail"
/// (processParcelDetails, once the parcel answered).
pub fn title(parcel_name: Option<&str>) -> String {
    match parcel_name {
        Some(name) => format!("Détails de l'emplacement : {name}"),
        None => "Détails de l'emplacement".to_owned(),
    }
}

/// LLPanelPlaceProfile::processParcelInfo: "HACK: Flag 0x2 == adult region,
/// Flag 0x1 == mature region, otherwise assume PG".
pub fn maturity(flags: u8) -> &'static str {
    if flags & 0x2 != 0 {
        "Adulte"
    } else if flags & 0x1 != 0 {
        "Modéré"
    } else {
        "Général"
    }
}

/// LLPanelPlaceInfo::processParcelInfo: "%s (%d, %d, %d)" with the position
/// in the region rounded, or the parcel's global position modulo 256 when
/// there is none.
pub fn region_name_pos(sim_name: &str, pos: glam::Vec3, global: DVec3) -> String {
    let (x, y, z) = if pos == glam::Vec3::ZERO {
        (
            (global.x.round() as i64).rem_euclid(256),
            (global.y.round() as i64).rem_euclid(256),
            global.z.round() as i64,
        )
    } else {
        (pos.x.round() as i64, pos.y.round() as i64, pos.z.round() as i64)
    };
    format!("{sim_name} ({x}, {y}, {z})")
}

/// FIRE-2717 "information_text" ("[AREA] m² | 🚶 [TRAFFIC]", the walker
/// drawn as a Phosphor icon): the area part and the traffic, `(S32)dwell`.
pub fn information(area: i32, dwell: f32) -> (String, String) {
    (format!("{area} m² |"), format!("{}", dwell as i32))
}

/// LLPanelPlaceInfo::setErrorStatus and displayParcelInfo texts (skin fr).
pub fn error_text(e: PlaceError) -> &'static str {
    match e {
        // LandmarkLocationUnknown: Firestorm shows nothing for an unknown
        // region (the floater only opens once the name is resolved)
        PlaceError::UnknownRegion => {
            "Le client n'a pas pu obtenir l'emplacement de la région. La région est peut-être temporairement indisponible ou a été supprimée."
        }
        PlaceError::Remote(RemoteParcelError::NoCapability) => {
            "Les informations du terrain ne seront pas disponibles avant le prochain redémarrage de la région."
        }
        // HTTP_INTERNAL_ERROR
        PlaceError::Remote(RemoteParcelError::Status(499)) => {
            "Les informations de ce terrain sont indisponibles en raison de permissions restreintes, veuillez contacter le propriétaire du terrain."
        }
        // 404, other errors, and a null parcel (Firestorm keeps loading)
        PlaceError::Remote(_) => "Les informations de ce terrain sont indisponibles pour le moment, veuillez réessayer ultérieurement.",
    }
}

/// What the panel shows: loading, an error or the parcel.
enum Shown<'a> {
    Loading,
    Failed(PlaceError),
    Parcel(&'a ParcelSummary),
}

fn shown<'a>(place: &Place, world: &'a World) -> Shown<'a> {
    if let Stage::Failed(e) = place.stage {
        return Shown::Failed(e);
    }
    place
        .parcel()
        .and_then(|id| world.profiles.parcels.get(&id))
        .map_or(Shown::Loading, Shown::Parcel)
}

impl PlaceDetailsUi {
    /// Bring a window to the front (opened again from a link).
    pub fn raise(&mut self, serial: u64) {
        self.raise.insert(serial);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        emoji: &mut super::emoji::Emoji,
        images: &HashMap<Uuid, egui::TextureHandle>,
        world: &World,
        want_names: &mut HashSet<Uuid>,
    ) -> Vec<PlaceAction> {
        let mut actions = Vec::new();
        let screen = ctx.content_rect();
        for (i, place) in world.place_details.places.iter().enumerate() {
            let win_id = format!("place-details-{}", place.serial);
            if self.raise.remove(&place.serial) {
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(&win_id)));
            }
            let data = shown(place, world);
            let name = match data {
                Shown::Parcel(d) => Some(d.name.as_str()),
                _ => None,
            };
            // floater_fs_placedetails.xml: 322 x 590 (250 x 570 at least)
            let size = Vec2::new(322.0, 590.0);
            let pos = screen.center() - size * 0.5 + Vec2::splat(24.0 * (i % 6) as f32);
            let mut floater = Floater::new(&win_id, title(name), pos, size);
            floater.min_size = Vec2::new(250.0, 570.0);
            let mut open = true;
            floater.show(ctx, p, &mut open, |ui| {
                let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
                ui.allocate_rect(body, egui::Sense::hover());
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(body)
                        .layout(egui::Layout::bottom_up(egui::Align::Min)),
                );
                let ui = &mut child;
                buttons(ui, p, world, place, &mut actions);
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    profile(ui, p, emoji, images, world, want_names, place, &data, &mut self.wanted_images);
                });
            });
            if !open {
                actions.push(PlaceAction::Close(place.serial));
            }
        }
        actions
    }
}

/// panel_place_profile.xml in PLACE mode, top to bottom.
#[allow(clippy::too_many_arguments)]
fn profile(
    ui: &mut egui::Ui,
    p: &Palette,
    emoji: &mut super::emoji::Emoji,
    images: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    want_names: &mut HashSet<Uuid>,
    place: &Place,
    data: &Shown,
    wanted_images: &mut HashSet<Uuid>,
) {
    let width = ui.available_width();
    // "logo": 315 x 197, default_land_picture while there is no snapshot
    let snapshot = match data {
        Shown::Parcel(d) if !d.snapshot.is_nil() => {
            wanted_images.insert(d.snapshot);
            images.get(&d.snapshot)
        }
        _ => None,
    };
    let size = Vec2::new(width, width * 197.0 / 315.0);
    match snapshot {
        // LLTextureCtrl stretches the snapshot over the whole control
        Some(t) => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(t.id(), rect, uv, egui::Color32::WHITE);
            ui.painter()
                .rect_stroke(rect, 2.0, egui::Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
        }
        None => {
            widgets::picture(ui, p, None, size, "mountains", "");
        }
    }
    ui.add_space(2.0);
    // "information": area and traffic, right-aligned
    ui.allocate_ui_with_layout(Vec2::new(width, 16.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let small = |t: &str| RichText::new(t).size(11.0).color(p.muted);
        match data {
            Shown::Parcel(d) => {
                let (area, traffic) = information(d.actual_area, d.dwell);
                ui.label(small(&traffic));
                if let Some(t) = super::icons::global("person-simple-walk") {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                    ui.painter().image(
                        t.id(),
                        r,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        p.muted,
                    );
                }
                ui.label(small(&area));
            }
            Shown::Loading => {
                ui.label(small(LOADING));
            }
            Shown::Failed(_) => {
                ui.label(small(NOT_AVAILABLE));
            }
        }
    });
    // "parcel_title" (SansSerifLarge, white) and "region_title"
    let (parcel_name, region) = match data {
        Shown::Parcel(d) => (
            if d.name.is_empty() {
                NOT_AVAILABLE.to_owned()
            } else {
                d.name.clone()
            },
            (!d.sim_name.is_empty()).then(|| format!("Région : {}", region_name_pos(&d.sim_name, place.pos, d.global))),
        ),
        Shown::Loading => (LOADING.to_owned(), Some(format!("Région : {LOADING}"))),
        Shown::Failed(_) => (NOT_AVAILABLE.to_owned(), Some(format!("Région : {NOT_AVAILABLE}"))),
    };
    ui.add(egui::Label::new(RichText::new(parcel_name).size(15.0).strong().color(p.ink)).truncate());
    ui.add_space(2.0);
    if let Some(r) = region {
        ui.add(egui::Label::new(RichText::new(r).size(12.0).color(p.muted)).truncate());
    }
    ui.add_space(8.0);
    // "description": 150 px high for a remote place (STORM-1311), links
    // clickable like Firestorm's expandable text
    let desc_h = 150.0_f32.min((ui.available_height() - 30.0).max(40.0));
    egui::Frame::new()
        .fill(p.field)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(5, 3))
        .show(ui, |ui| {
            ui.set_width(width - 10.0);
            egui::ScrollArea::vertical()
                .id_salt(("place-desc", place.serial))
                .min_scrolled_height(desc_h - 6.0)
                .max_height(desc_h - 6.0)
                .auto_shrink([false, false])
                .show(ui, |ui| match data {
                    Shown::Parcel(d) if !d.desc.trim().is_empty() => {
                        for line in d.desc.lines() {
                            if line.is_empty() {
                                ui.add_space(6.0);
                            } else {
                                super::chat::chat_text(ui, p, emoji, world, want_names, line, 12.5, p.ink, false);
                            }
                        }
                    }
                    Shown::Parcel(_) => {
                        ui.label(RichText::new(NOT_AVAILABLE).size(12.5).color(p.muted));
                    }
                    Shown::Loading => {
                        ui.label(RichText::new(LOADING).size(12.5).color(p.muted));
                    }
                    Shown::Failed(e) => {
                        ui.label(RichText::new(error_text(*e)).size(12.5).color(p.ink));
                    }
                });
        });
    ui.add_space(6.0);
    // "maturity_icon" / "maturity_value"
    ui.horizontal(|ui| {
        match data {
            Shown::Parcel(d) => {
                let m = maturity(d.flags);
                widgets::maturity_badge(ui, p, m, 16.0);
                ui.label(RichText::new(m).size(12.5).color(p.ink));
            }
            other => {
                // Unknown_Icon
                if let Some(t) = super::icons::global("question") {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                    ui.painter().image(
                        t.id(),
                        r,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        p.muted,
                    );
                }
                let text = if matches!(other, Shown::Loading) { LOADING } else { NOT_AVAILABLE };
                ui.label(RichText::new(text).size(12.5).color(p.muted));
            }
        }
    });
}

/// Bottom bar of floater_fs_placedetails.xml for a remote place (updateVerbs:
/// Téléporter and Carte, enabled with a position; Modifier / Enregistrer /
/// Annuler / Fermer hidden) and the overflow menu (menu_place.xml).
fn buttons(ui: &mut egui::Ui, p: &Palette, world: &World, place: &Place, actions: &mut Vec<PlaceAction>) {
    let global = place.global();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if button(
            ui,
            p,
            "Téléporter",
            "Se téléporter à l'emplacement indiqué",
            global.is_some(),
            108.0,
        ) && let Some(g) = global
        {
            actions.push(PlaceAction::Teleport(g));
        }
        if button(
            ui,
            p,
            "Carte",
            "Voir l'emplacement correspondant sur la carte",
            global.is_some(),
            85.0,
        ) && let Some(g) = global
        {
            actions.push(PlaceAction::ShowOnMap(g));
        }
        widgets::icon_menu(ui, p, "list", "Afficher plus d'options", |ui| {
            // "copy": LLLandmarkActions::getSLURLfromPosGlobal (region name
            // as the grid spells it)
            let region = place
                .origin
                .and_then(|(x, y)| world.map.sims.get(&(x / 256, y / 256)))
                .map_or(place.region.as_str(), |s| s.name.as_str());
            if super::menu::item_if(ui, p, "link", "Copier la SLurl", global.is_some()) {
                ui.ctx().copy_text(crate::slurl::make(region, place.pos));
            }
        });
    });
}

fn button(ui: &mut egui::Ui, p: &Palette, label: &str, tip: &str, enabled: bool, width: f32) -> bool {
    let b = egui::Button::new(RichText::new(label).size(12.0).color(p.ink))
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(2));
    ui.add_enabled(enabled, |ui: &mut egui::Ui| ui.add_sized([width, 23.0], b))
        .on_hover_text(tip)
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn titles() {
        assert_eq!(title(None), "Détails de l'emplacement");
        assert_eq!(title(Some("Place d'Aurora")), "Détails de l'emplacement : Place d'Aurora");
    }

    #[test]
    fn maturity_from_flags() {
        assert_eq!(maturity(0), "Général");
        assert_eq!(maturity(0x1), "Modéré");
        assert_eq!(maturity(0x2), "Adulte");
        assert_eq!(maturity(0x3), "Adulte");
    }

    #[test]
    fn region_and_position() {
        let g = DVec3::new(256_140.4, 256_119.6, 24.5);
        assert_eq!(
            region_name_pos("Aurora Démo", Vec3::new(140.4, 119.6, 24.6), g),
            "Aurora Démo (140, 120, 25)"
        );
        // no position of our own: the parcel's, modulo the region width
        assert_eq!(region_name_pos("Aurora Démo", Vec3::ZERO, g), "Aurora Démo (140, 120, 25)");
    }

    #[test]
    fn area_and_traffic() {
        assert_eq!(information(1024, 37.9), ("1024 m² |".to_owned(), "37".to_owned()));
        assert_eq!(information(0, 0.0).1, "0");
    }

    #[test]
    fn error_texts() {
        let e = |r| error_text(PlaceError::Remote(r));
        assert!(e(RemoteParcelError::NoCapability).contains("prochain redémarrage"));
        assert!(e(RemoteParcelError::Status(499)).contains("permissions restreintes"));
        assert!(e(RemoteParcelError::Status(404)).contains("réessayer ultérieurement"));
        assert_eq!(e(RemoteParcelError::Failed), e(RemoteParcelError::NoParcel));
        assert!(error_text(PlaceError::UnknownRegion).contains("emplacement de la région"));
    }
}
