//! Place profiles, drawn in the Places window (`places.rs`) or in the
//! standalone "Place Details" windows of FSFloaterPlaceDetails
//! (FSUseStandalonePlaceDetailsFloater). Two panels, as in Firestorm
//! (indra/newview/llpanelplaceinfo.cpp, llpanelplaceprofile.cpp,
//! llpanellandmarkinfo.cpp, fsfloaterplacedetails.cpp, panel_place_profile.xml,
//! panel_landmark_info.xml, floater_fs_placedetails.xml, originally LGPL 2.1):
//! - the place profile of a place link or a teleport history entry
//!   (LLPanelPlaceProfile, PLACE / TELEPORT_HISTORY): snapshot, area and
//!   traffic (FIRE-2717), parcel name, "Region: Name (x, y, z)",
//!   description, maturity. The owner line and the accordions (parcel,
//!   region, estate, for sale) are only shown by Firestorm for the agent's own
//!   parcel (AGENT), not for these;
//! - the landmark profile (LLPanelLandmarkInfo, LANDMARK): snapshot, parcel
//!   name, region and maturity, parcel owner, description, the item's owner,
//!   creator and date, traffic and area, title and notes, "Edit".
//!
//! Network state: `world/place_details.rs`.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::place_details::{Place, PlaceError, Source, Stage};
use aurora_net::ParcelSummary;
use aurora_net::land::RemoteParcelError;
use egui::{RichText, Vec2};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// LLTrans "LoadingData".
pub fn loading() -> &'static str {
    aurora_i18n::tr!("common-loading")
}
/// "not_available" of both panels.
const NOT_AVAILABLE: &str = "(N\\A)";

pub enum PlaceAction {
    /// onTeleportButtonClicked for a place link or a history entry:
    /// gAgent.teleportViaLocation + trackLocation, no confirmation.
    Teleport(DVec3),
    /// A landmark: TeleportFromLandmark confirmation, then
    /// teleport_via_landmark.
    TeleportLandmark {
        asset: Uuid,
        name: String,
    },
    /// onShowOnMapButtonClicked: trackLocation, world map centered on it.
    ShowOnMap(DVec3),
    Close(u64),
}

/// What the profile panels read and what they ask for.
pub struct Ctx<'a> {
    pub p: &'a Palette,
    pub world: &'a World,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub emoji: &'a mut super::emoji::Emoji,
    pub want_names: &'a mut HashSet<Uuid>,
    pub wanted_images: &'a mut HashSet<Uuid>,
    /// Group names to look up (UUIDGroupNameRequest).
    pub want_groups: &'a mut HashSet<Uuid>,
}

#[derive(Default)]
pub struct PlaceDetailsUi {
    raise: HashSet<u64>,
}

/// The standalone window's title (FSFloaterPlaceDetails: "title_remote_place"
/// then "title_remote_place_detail" once the parcel answered;
/// "title_landmark" / "title_landmark_detail" with the item's name;
/// "title_teleport_history_item").
pub fn window_title(place: &Place, world: &World) -> String {
    let title = match &place.source {
        Source::Link { .. } | Source::Parcel(_) => match shown(place, world) {
            Shown::Parcel(d) => aurora_i18n::tr!("ui-place-details-location-details-2", value0 = d.name.to_string()),
            _ => aurora_i18n::tr!("ui-place-details-location-details").to_owned(),
        },
        Source::Landmark { item, .. } => match world.inventory.items.get(item) {
            Some(it) => aurora_i18n::tr!("ui-place-details-landmark", value0 = it.name.to_string()),
            None => aurora_i18n::tr!("common-landmark").to_owned(),
        },
        Source::History { title, .. } => aurora_i18n::tr!("ui-place-details-teleport-history-for", title = title.to_string()),
    };
    world.rlva_filter_text(&title).into_owned()
}

/// The header of the Places window's profile (LLPanelPlaceProfile::
/// setInfoType "title_place" / "title_teleport_history",
/// LLPanelLandmarkInfo "title_landmark" / "title_edit_landmark").
pub fn header_title(place: &Place) -> &'static str {
    match place.source {
        Source::Link { .. } | Source::Parcel(_) => aurora_i18n::tr!("common-place-profile"),
        Source::History { .. } => aurora_i18n::tr!("ui-place-details-teleport-history"),
        Source::Landmark { .. } => aurora_i18n::tr!("common-landmark"),
    }
}

/// LLPanelPlaceProfile::processParcelInfo: "HACK: Flag 0x2 == adult region,
/// Flag 0x1 == mature region, otherwise assume PG".
pub fn maturity(flags: u8) -> &'static str {
    if flags & 0x2 != 0 {
        aurora_i18n::tr!("common-adult")
    } else if flags & 0x1 != 0 {
        aurora_i18n::tr!("common-moderate")
    } else {
        aurora_i18n::tr!("common-general")
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

/// FIRE-2717 "information_text" of the place profile ("[AREA] m² | 🚶
/// [TRAFFIC]", the walker drawn as a Tabler icon): the area part and the
/// traffic, `(S32)dwell`.
pub fn information(area: i32, dwell: f32) -> (String, String) {
    (format!("{area} m² |"), format!("{}", dwell as i32))
}

/// "information_text" of the French landmark panel.
pub fn landmark_information(area: i32, dwell: f32) -> String {
    aurora_i18n::tr!(
        "ui-place-details-traffic-area-m2",
        value0 = format!("{}", dwell as i32),
        area = format!("{}", area)
    )
}

/// "acquired_date" of the French landmark panel, in SLT:
/// "Tue 02 Apr 2024 07:42:28"; 0 is "(unknown)".
pub fn acquired_date(secs: i64) -> String {
    if secs == 0 {
        return aurora_i18n::tr!("status-unknown").to_owned();
    }
    let (y, m, d, w, h, mi, s) = super::land::slt_parts(secs);
    format!(
        "{} {d:02} {} {y} {h:02}:{mi:02}:{s:02}",
        super::land::weekdays()[w],
        super::land::months()[m - 1]
    )
}

/// LLPanelPlaceInfo::setErrorStatus and displayParcelInfo texts (skin fr;
/// the landmark panel has its own wording).
pub fn error_text(e: PlaceError, landmark: bool) -> &'static str {
    match (e, landmark) {
        // LandmarkLocationUnknown: Firestorm opens nothing for an unknown
        // region (the profile only opens once the name is resolved)
        (PlaceError::UnknownRegion, _) => {
            aurora_i18n::tr!("ui-place-details-could-not-find-the-region-s-location-it-may")
        }
        (PlaceError::Remote(RemoteParcelError::NoCapability), false) => {
            aurora_i18n::tr!("ui-place-details-land-information-will-be-unavailable-until-the-next-region")
        }
        (PlaceError::Remote(RemoteParcelError::NoCapability), true) => {
            aurora_i18n::tr!("ui-place-details-place-information-requires-a-server-update")
        }
        // HTTP_INTERNAL_ERROR
        (PlaceError::Remote(RemoteParcelError::Status(499)), false) => {
            aurora_i18n::tr!("ui-place-details-land-information-is-unavailable-because-of-restricted-permissions-contact")
        }
        (PlaceError::Remote(RemoteParcelError::Status(499)), true) => {
            aurora_i18n::tr!("ui-place-details-place-information-is-unavailable-because-access-is-restricted-check")
        }
        // 404, other errors, a null parcel and an unreadable landmark
        // (Firestorm keeps loading for these two)
        (_, false) => aurora_i18n::tr!("ui-place-details-land-information-is-currently-unavailable-please-try-again-later"),
        (_, true) => aurora_i18n::tr!("ui-place-details-information-about-this-place-is-unavailable-please-try-again"),
    }
}

/// What a panel shows: loading, an error or the parcel.
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

/// Region name for a SLURL of the place (LLLandmarkActions::
/// getSLURLfromPosGlobal): the parcel's region, the map's, or the link's.
fn slurl_of(place: &Place, world: &World) -> Option<String> {
    let global = place.global?;
    let region = match shown(place, world) {
        Shown::Parcel(d) if !d.sim_name.is_empty() => d.sim_name.clone(),
        _ => world
            .map
            .sim_at_global(global.x, global.y)
            .map_or_else(|| place.region_hint().to_owned(), |(_, s)| s.name.clone()),
    };
    (!region.is_empty() && !world.rlva_private_region(&region)).then(|| crate::slurl::make(&region, place.region_pos))
}

/// "Teleport" of a profile (onTeleportButtonClicked).
pub fn teleport_action(place: &Place, world: &World) -> Option<PlaceAction> {
    match &place.source {
        Source::Landmark { item, asset } => Some(PlaceAction::TeleportLandmark {
            asset: *asset,
            name: world.inventory.items.get(item).map(|i| i.name.clone()).unwrap_or_default(),
        }),
        _ => place.global.map(PlaceAction::Teleport),
    }
}

/// The overflow menu of a profile: menu_place.xml for a place link or a
/// history entry ("Create landmark" only for the agent's own parcel,
/// STORM-411), menu_landmark.xml for a landmark.
pub fn overflow_menu(ui: &mut egui::Ui, p: &Palette, world: &World, place: &Place) {
    let slurl = slurl_of(place, world);
    if super::menu::item_if(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl"), slurl.is_some())
        && let Some(s) = slurl
    {
        ui.ctx().copy_text(s);
    }
    if matches!(place.source, Source::Landmark { .. }) {
        super::menu::todo(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"));
        super::menu::todo(ui, p, "star-filled", aurora_i18n::tr!("common-create-favorite"));
        super::menu::todo(ui, p, "pin-filled", aurora_i18n::tr!("ui-place-details-add-to-favorites-bar"));
    } else {
        super::menu::todo(ui, p, "star-filled", aurora_i18n::tr!("common-create-favorite"));
    }
}

impl PlaceDetailsUi {
    /// Bring a window to the front (opened again).
    pub fn raise(&mut self, serial: u64) {
        self.raise.insert(serial);
    }

    pub fn show(&mut self, ctx: &egui::Context, c: &mut Ctx) -> Vec<PlaceAction> {
        let mut actions = Vec::new();
        let screen = ctx.content_rect();
        let (p, world) = (c.p, c.world);
        for (i, place) in world.place_details.windows.iter().enumerate() {
            let win_id = format!("place-details-{}", place.serial);
            if self.raise.remove(&place.serial) {
                widgets::expand_floater(ctx, &win_id);
            }
            // floater_fs_placedetails.xml: 322 x 590 (250 x 570 at least)
            let size = Vec2::new(322.0, 590.0);
            let pos = screen.center() - size * 0.5 + Vec2::splat(24.0 * (i % 6) as f32);
            let mut floater = Floater::new(&win_id, window_title(place, world), pos, size);
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
                window_buttons(ui, p, world, place, &mut actions);
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("place-window", place.serial))
                        .auto_shrink([false, false])
                        .show(ui, |ui| profile_body(ui, c, place));
                });
            });
            if !open {
                actions.push(PlaceAction::Close(place.serial));
            }
        }
        actions
    }
}

/// Bottom bar of floater_fs_placedetails.xml (updateVerbs: "Teleport" and
/// "Map" enabled with a position; "Edit" for a landmark) and the
/// overflow menu.
fn window_buttons(ui: &mut egui::Ui, p: &Palette, world: &World, place: &Place, actions: &mut Vec<PlaceAction>) {
    let global = place.global;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if button(
            ui,
            p,
            aurora_i18n::tr!("common-teleport-3"),
            aurora_i18n::tr!("ui-place-details-teleport-to-the-specified-location"),
            global.is_some(),
            108.0,
        ) && let Some(a) = teleport_action(place, world)
        {
            actions.push(a);
        }
        if button(
            ui,
            p,
            aurora_i18n::tr!("common-map"),
            aurora_i18n::tr!("ui-place-details-show-the-corresponding-location-on-the-map"),
            global.is_some(),
            85.0,
        ) && let Some(g) = global
        {
            actions.push(PlaceAction::ShowOnMap(g));
        }
        if matches!(place.source, Source::Landmark { .. }) {
            ui.add_enabled_ui(false, |ui| {
                button(
                    ui,
                    p,
                    aurora_i18n::tr!("common-edit"),
                    aurora_i18n::tr!("ui-place-details-edit-this-landmark"),
                    true,
                    70.0,
                )
            })
            .response
            .on_disabled_hover_text(super::menu::not_yet());
        }
        widgets::icon_menu(
            ui,
            p,
            "menu-2-filled",
            aurora_i18n::tr!("ui-place-details-show-more-options"),
            |ui| overflow_menu(ui, p, world, place),
        );
    });
}

/// A flat button of the Places bottom bars.
pub fn button(ui: &mut egui::Ui, p: &Palette, label: &str, tip: &str, enabled: bool, width: f32) -> bool {
    let b = egui::Button::new(RichText::new(label).size(12.0).color(p.ink))
        .fill(p.raised)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius);
    ui.add_enabled(enabled, |ui: &mut egui::Ui| ui.add_sized([width, 23.0], b))
        .on_hover_text(tip)
        .clicked()
}

/// The panel of a place: landmark info for a landmark, place profile else.
pub fn profile_body(ui: &mut egui::Ui, c: &mut Ctx, place: &Place) {
    let data = shown(place, c.world);
    snapshot(ui, c, &data);
    match place.source {
        Source::Landmark { item, .. } => landmark_info(ui, c, place, item, &data),
        _ => place_profile(ui, c, place, &data),
    }
}

/// "logo": the parcel snapshot, default_land_picture while there is none.
fn snapshot(ui: &mut egui::Ui, c: &mut Ctx, data: &Shown) {
    let p = c.p;
    let width = ui.available_width();
    let tex = match data {
        Shown::Parcel(d) if !d.snapshot.is_nil() => {
            c.wanted_images.insert(d.snapshot);
            c.images.get(&d.snapshot)
        }
        _ => None,
    };
    let size = Vec2::new(width, width * 197.0 / 315.0);
    match tex {
        // LLTextureCtrl stretches the snapshot over the whole control
        Some(t) => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(t.id(), rect, uv, widgets::PICTURE_TINT);
            ui.painter().rect_stroke(
                rect,
                ui.visuals().window_corner_radius,
                egui::Stroke::new(1.0, p.raised),
                egui::StrokeKind::Inside,
            );
        }
        None => {
            widgets::picture(ui, p, None, size, "mountain-filled", "");
        }
    }
    ui.add_space(2.0);
}

fn small_icon(ui: &mut egui::Ui, name: &str, size: f32, tint: egui::Color32) {
    if let Some(t) = super::icons::global(name) {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// Parcel name (SansSerifLarge) and "Region: Name (x, y, z)".
fn titles(data: &Shown, place: &Place, world: &World) -> (String, Option<String>) {
    let (name, region) = match data {
        Shown::Parcel(d) => (
            if d.name.is_empty() {
                NOT_AVAILABLE.to_owned()
            } else {
                d.name.clone()
            },
            (!d.sim_name.is_empty()).then(|| {
                aurora_i18n::tr!(
                    "ui-place-details-region-3",
                    value0 = region_name_pos(&d.sim_name, place.region_pos, d.global).to_string()
                )
            }),
        ),
        Shown::Loading => (
            loading().to_owned(),
            Some(aurora_i18n::tr!("ui-place-details-region", LOADING = loading().to_string())),
        ),
        Shown::Failed(_) => (
            NOT_AVAILABLE.to_owned(),
            Some(aurora_i18n::tr!(
                "ui-place-details-region-2",
                NOT_AVAILABLE = NOT_AVAILABLE.to_string()
            )),
        ),
    };
    (
        world.rlva_filter_text(&name).into_owned(),
        region.map(|r| world.rlva_filter_text(&r).into_owned()),
    )
}

/// "maturity_icon" / "maturity_value", icon first in reading order (a
/// right-to-left layout draws its first widget at the right).
fn maturity_row(ui: &mut egui::Ui, p: &Palette, data: &Shown) {
    let (icon, text, known): (Option<&str>, &str, bool) = match data {
        Shown::Parcel(d) => (None, maturity(d.flags), true),
        Shown::Loading => (Some("help-circle-filled"), loading(), false),
        Shown::Failed(_) => (Some("help-circle-filled"), NOT_AVAILABLE, false),
    };
    let rtl = ui.layout().prefer_right_to_left();
    let draw_icon = |ui: &mut egui::Ui| match icon {
        // Unknown_Icon
        Some(i) => small_icon(ui, i, 16.0, p.muted),
        None => {
            widgets::maturity_badge(ui, p, text, 16.0);
        }
    };
    let label = RichText::new(text).size(12.5).color(if known { p.ink } else { p.muted });
    if rtl {
        ui.label(label);
        draw_icon(ui);
    } else {
        draw_icon(ui);
        ui.label(label);
    }
}

/// "description": the parcel's text with clickable links (expandable_text),
/// in a box `height` high.
fn description(ui: &mut egui::Ui, c: &mut Ctx, place: &Place, data: &Shown, height: f32, landmark: bool) {
    let p = c.p;
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.82))
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(5, 3))
        .show(ui, |ui| {
            ui.set_width(width - 10.0);
            egui::ScrollArea::vertical()
                .id_salt(("place-desc", place.serial))
                .min_scrolled_height(height - 6.0)
                .max_height(height - 6.0)
                .auto_shrink([false, false])
                .show(ui, |ui| match data {
                    Shown::Parcel(d) if !d.desc.trim().is_empty() => {
                        for line in d.desc.lines() {
                            if line.is_empty() {
                                ui.add_space(6.0);
                            } else {
                                super::chat::chat_text(ui, p, c.emoji, c.world, c.want_names, line, 12.5, p.ink, false);
                            }
                        }
                    }
                    Shown::Parcel(_) => {
                        ui.label(RichText::new(NOT_AVAILABLE).size(12.5).color(p.muted));
                    }
                    Shown::Loading => {
                        ui.label(RichText::new(loading()).size(12.5).color(p.muted));
                    }
                    Shown::Failed(e) => {
                        ui.label(RichText::new(error_text(*e, landmark)).size(12.5).color(p.ink));
                    }
                });
        });
}

/// panel_place_profile.xml in PLACE / TELEPORT_HISTORY mode, top to bottom.
fn place_profile(ui: &mut egui::Ui, c: &mut Ctx, place: &Place, data: &Shown) {
    let p = c.p;
    let width = ui.available_width();
    // "information": area and traffic, right-aligned
    ui.allocate_ui_with_layout(Vec2::new(width, 16.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let small = |t: &str| RichText::new(t).size(11.0).color(p.muted);
        match data {
            Shown::Parcel(d) => {
                let (area, traffic) = information(d.actual_area, d.dwell);
                ui.label(small(&traffic));
                small_icon(ui, "walk", 12.0, p.muted);
                ui.label(small(&area));
            }
            Shown::Loading => {
                ui.label(small(loading()));
            }
            Shown::Failed(_) => {
                ui.label(small(NOT_AVAILABLE));
            }
        }
    });
    let (parcel_name, region) = titles(data, place, c.world);
    ui.add(egui::Label::new(RichText::new(parcel_name).size(15.0).strong().color(p.ink)).truncate());
    ui.add_space(2.0);
    if let Some(r) = region {
        ui.add(egui::Label::new(RichText::new(r).size(12.0).color(p.muted)).truncate());
    }
    ui.add_space(8.0);
    // 150 px high for these (STORM-1311), less when the window is short so
    // the classification under it stays in view: inside the scroll area the
    // height left is unbounded, the visible part ends with the clip rectangle
    let room = ui.clip_rect().bottom() - ui.cursor().top();
    let desc_h = 150.0_f32.min((room - 36.0).max(40.0));
    description(ui, c, place, data, desc_h, false);
    ui.add_space(6.0);
    ui.horizontal(|ui| maturity_row(ui, p, data));
}

/// A label of the landmark panel's left column.
fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.allocate_ui_with_layout(Vec2::new(78.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(RichText::new(text).size(12.0).color(p.ink));
    });
}

/// An avatar or group as a link (LLSLURL "agent" / "group" "inspect").
fn who(ui: &mut egui::Ui, c: &mut Ctx, id: Uuid, group: bool) {
    let p = c.p;
    if id.is_nil() {
        ui.label(RichText::new(aurora_i18n::tr!("status-public")).size(12.0).color(p.muted));
        return;
    }
    let name = if group {
        let known = c.world.groups.groups.iter().find(|g| g.id == id).map(|g| g.name.clone());
        known.or_else(|| c.world.land.group_names.get(&id).cloned()).unwrap_or_else(|| {
            c.want_groups.insert(id);
            "…".to_owned()
        })
    } else {
        c.want_names.insert(id);
        c.world.social.name_of(&id)
    };
    let link = ui.add(egui::Link::new(
        RichText::new(name)
            .size(12.0)
            .color(super::colors::text(super::colors::get().chat_slurl)),
    ));
    if !group && link.clicked() {
        super::profile::request_open(ui.ctx(), id);
    }
}

/// panel_landmark_info.xml in LANDMARK mode, top to bottom.
fn landmark_info(ui: &mut egui::Ui, c: &mut Ctx, place: &Place, item: Uuid, data: &Shown) {
    let p = c.p;
    // the 15 px lines of panel_landmark_info.xml
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.spacing_mut().interact_size.y = 16.0;
    let width = ui.available_width();
    let (parcel_name, region) = titles(data, place, c.world);
    ui.add(egui::Label::new(RichText::new(parcel_name).size(15.0).strong().color(p.ink)).truncate());
    ui.add_space(2.0);
    // region on the left, maturity at the right end of the same line
    ui.allocate_ui_with_layout(Vec2::new(width, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            maturity_row(ui, p, data);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if let Some(r) = region {
                    ui.add(egui::Label::new(RichText::new(r).size(12.0).color(p.muted)).truncate());
                }
            });
        });
    });
    // "parcel_owner": the parcel's owner, a group with flag 0x4 (DRTSIM-453)
    ui.horizontal(|ui| {
        // "Owner:" in Firestorm; named apart from the landmark's owner below,
        // on one line (wider than the label column)
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("ui-place-details-landowner"))
                    .size(12.0)
                    .color(p.ink),
            )
            .extend(),
        );
        match data {
            Shown::Parcel(d) => who(ui, c, d.owner, d.flags & 0x4 != 0),
            Shown::Loading => {
                ui.label(RichText::new(loading()).size(12.0).color(p.muted));
            }
            Shown::Failed(_) => {
                ui.label(RichText::new(NOT_AVAILABLE).size(12.0).color(p.muted));
            }
        }
    });
    ui.add_space(4.0);
    description(ui, c, place, data, 90.0, true);
    ui.add_space(4.0);
    // landmark_info_panel: the inventory item (displayItemInfo)
    let it = c.world.inventory.items.get(&item).cloned();
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("common-owner-2"));
        match &it {
            Some(i) => who(ui, c, i.owner, false),
            None => {
                ui.label(RichText::new(loading()).size(12.0).color(p.muted));
            }
        }
    });
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("common-creator-2"));
        match &it {
            Some(i) if !i.creator.is_nil() => who(ui, c, i.creator, false),
            Some(_) => {
                ui.label(RichText::new(aurora_i18n::tr!("status-unknown")).size(12.0).color(p.muted));
            }
            None => {
                ui.label(RichText::new(loading()).size(12.0).color(p.muted));
            }
        }
    });
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("ui-place-details-created-on"));
        let text = it.as_ref().map_or(loading().to_owned(), |i| acquired_date(i.created_at));
        ui.label(RichText::new(text).size(12.0).color(p.ink));
    });
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("ui-place-details-info"));
        let text = match data {
            Shown::Parcel(d) => landmark_information(d.actual_area, d.dwell),
            Shown::Loading => loading().to_owned(),
            Shown::Failed(_) => NOT_AVAILABLE.to_owned(),
        };
        ui.label(RichText::new(text).size(12.0).color(p.ink));
    });
    ui.add_space(6.0);
    // landmark_edit_panel, read only until "Edit" (not ported yet)
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("ui-place-details-title"));
        let mut name = it.as_ref().map(|i| i.name.clone()).unwrap_or_default();
        crate::ui::widgets::text_field(ui, false, &mut name, |b, ui| {
            egui::TextEdit::singleline(b).desired_width(ui.available_width())
        });
    });
    ui.label(RichText::new(aurora_i18n::tr!("ui-place-details-notes")).size(12.0).color(p.ink));
    let mut notes = it.as_ref().map(|i| i.desc.clone()).unwrap_or_default();
    crate::ui::widgets::text_field(ui, false, &mut notes, |b, _| {
        egui::TextEdit::multiline(b).desired_width(width).desired_rows(4).char_limit(127)
    });
    ui.add_space(4.0);
    // setCanEdit: an item of our inventory we may modify
    ui.add_enabled_ui(false, |ui| {
        button(
            ui,
            p,
            aurora_i18n::tr!("common-edit"),
            aurora_i18n::tr!("ui-place-details-edit-landmark-information"),
            true,
            100.0,
        )
    })
    .response
    .on_disabled_hover_text(super::menu::not_yet());
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn maturity_from_flags() {
        assert_eq!(maturity(0), aurora_i18n::tr!("common-general"));
        assert_eq!(maturity(0x1), aurora_i18n::tr!("common-moderate"));
        assert_eq!(maturity(0x2), aurora_i18n::tr!("common-adult"));
        assert_eq!(maturity(0x3), aurora_i18n::tr!("common-adult"));
        // 0x4 is the group-owned flag, not a rating
        assert_eq!(maturity(0x4), aurora_i18n::tr!("common-general"));
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
        assert_eq!(
            landmark_information(12288, 494.6),
            aurora_i18n::tr!("ui-place-details-traffic-area-m2", value0 = "494", area = "12288").as_str()
        );
    }

    #[test]
    fn landmark_dates_in_slt() {
        use aurora_net::profile::days_from_civil;
        // 2024-04-02 14:42:28 UTC = 07:42:28 PDT
        let t = days_from_civil(2024, 4, 2) * 86_400 + 14 * 3600 + 42 * 60 + 28;
        assert_eq!(
            acquired_date(t),
            match aurora_i18n::language() {
                "fr" => "mar. 02 avr. 2024 07:42:28",
                "pt-BR" => "ter 02 abr 2024 07:42:28",
                _ => "Tue 02 Apr 2024 07:42:28",
            }
        );
        assert_eq!(acquired_date(0), aurora_i18n::tr!("status-unknown"));
    }

    #[test]
    fn error_texts() {
        let e = |r| error_text(PlaceError::Remote(r), false);
        assert!(
            e(RemoteParcelError::NoCapability)
                == aurora_i18n::tr!("ui-place-details-land-information-will-be-unavailable-until-the-next-region")
        );
        assert!(
            e(RemoteParcelError::Status(499))
                == aurora_i18n::tr!("ui-place-details-land-information-is-unavailable-because-of-restricted-permissions-contact")
        );
        assert!(
            e(RemoteParcelError::Status(404))
                == aurora_i18n::tr!("ui-place-details-land-information-is-currently-unavailable-please-try-again-later")
        );
        assert_eq!(e(RemoteParcelError::Failed), e(RemoteParcelError::NoParcel));
        assert!(
            error_text(PlaceError::UnknownRegion, false)
                == aurora_i18n::tr!("ui-place-details-could-not-find-the-region-s-location-it-may")
        );
        assert!(
            error_text(PlaceError::LandmarkUnavailable, true)
                == aurora_i18n::tr!("ui-place-details-information-about-this-place-is-unavailable-please-try-again")
        );
        assert!(
            error_text(PlaceError::Remote(RemoteParcelError::Status(499)), true)
                == aurora_i18n::tr!("ui-place-details-place-information-is-unavailable-because-access-is-restricted-check")
        );
    }
}
