//! The tabs of the Aurora profile after Profile (the tabs mock-up, variant A
//! of each, human choice of 2026-10-14): Favorite places as a featured
//! place over thumbnails, Classifieds as cards that unfold, First life as a
//! portrait beside its text, Web in a browser card, Notes in the turquoise
//! card of the Profile tab. The data and the actions are Firestorm's
//! (LLPanelProfilePicks / LLPanelProfilePick, LLPanelProfileClassifieds /
//! LLPanelProfileClassified with LLPublishClassifiedFloater,
//! LLPanelProfileFirstLife, LLPanelProfileWeb, LLPanelProfileNotes in
//! `newview/llpanelprofile*.cpp`, originally LGPL 2.1).

use super::super::{
    ClassifiedEdit, PickEdit, PictureSlot, ProfileAction, Publish, Window, classified_of, loading, new_pick, picks_limit, rich,
    web_username,
};
use super::{Env, card, display_name, paint_icon, semibold};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::widgets::ActionKind;
use crate::ui::{menu, widgets};
use crate::world::World;
use crate::world::profiles;
use aurora_llsd::llsd_map;
use aurora_net::profile::{ClassifiedClick, MINIMUM_PRICE_FOR_LISTING, classified_flags};
use aurora_net::{ClassifiedInfo, NetCommand, ParcelSummary};
use egui::{Color32, FontId, Rect, RichText, Sense, Vec2, pos2};
use glam::DVec3;
use uuid::Uuid;

/// Heights of the featured place's picture and of a thumbnail's.
const FEATURE_H: f32 = 196.0;
const FEATURE_W: f32 = 300.0;
const THUMB_IMG_H: f32 = 62.0;
/// Limits of Firestorm's fields (panel_profile_pick.xml,
/// panel_profile_classified.xml, panel_profile_firstlife.xml,
/// panel_profile_notes.xml).
const PICK_NAME_MAX: usize = 63;
const PICK_DESC_MAX: usize = 1023;
const CLASSIFIED_NAME_BYTES: usize = 30;
const CLASSIFIED_DESC_MAX: usize = 256;
const FIRST_LIFE_MAX: usize = 65_000;
const NOTES_MAX: usize = 65_530;

/// "Place, Region (x, y, z)" of the mock-up: the parts known, then the
/// position in the region.
fn place_line(parcel: &str, sim: &str, pos: DVec3) -> String {
    let names: Vec<&str> = [parcel, sim].into_iter().filter(|s| !s.trim().is_empty()).collect();
    let mut s = names.join(" · ");
    if pos != DVec3::ZERO {
        let local = |v: f64| (v.rem_euclid(256.0)).round() as i64;
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("({}, {}, {})", local(pos.x), local(pos.y), pos.z.round() as i64));
    }
    s
}

/// The rating of a parcel's region as the badge's name (ParcelInfoReply
/// flags: 0x1 moderate, 0x2 adult).
fn maturity_of(parcel: Option<&ParcelSummary>) -> Option<&'static str> {
    let flags = parcel?.flags;
    Some(if flags & 0x2 != 0 {
        aurora_i18n::text("common-adult")
    } else if flags & 0x1 != 0 {
        aurora_i18n::text("common-moderate")
    } else {
        aurora_i18n::text("common-general")
    })
}

/// LLPanelProfileClassified::isValidName: a name, starting with a letter
/// from A to Z or a digit (isalnum of its first byte).
pub(super) fn classified_name_error(name: &str) -> Option<&'static str> {
    match name.bytes().next() {
        None => Some(aurora_i18n::tr!("ui-profile-aurora-classified-blank-name")),
        Some(b) if !b.is_ascii_alphanumeric() => Some(aurora_i18n::tr!("ui-profile-aurora-classified-alphanumeric")),
        _ => None,
    }
}

/// The name cut to the 30 bytes of the field (max_length_bytes), on a
/// character boundary.
pub(super) fn clip_bytes(s: &mut String, max: usize) {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
}

/// LLPanelProfileClassified::onPublishFloaterPublishClicked: the price
/// typed, at least the fee.
pub(super) fn publish_price(text: &str) -> Result<i32, String> {
    match text.trim().parse::<i32>() {
        Ok(price) if price >= MINIMUM_PRICE_FOR_LISTING => Ok(price),
        _ => Err(aurora_i18n::tr!(
            "ui-profile-aurora-classified-min-price",
            price = MINIMUM_PRICE_FOR_LISTING
        )),
    }
}

/// A picture covering `rect` (its middle shown), rounded as `radius`.
fn cover(ui: &egui::Ui, tex: &egui::TextureHandle, rect: Rect, radius: egui::CornerRadius) {
    let [tw, th] = tex.size();
    let aspect = tw.max(1) as f32 / th.max(1) as f32;
    let want = rect.width() / rect.height().max(1.0);
    let uv = if aspect > want {
        let u = want / aspect;
        Rect::from_min_max(pos2(0.5 - u / 2.0, 0.0), pos2(0.5 + u / 2.0, 1.0))
    } else {
        let v = aspect / want;
        Rect::from_min_max(pos2(0.0, 0.5 - v / 2.0), pos2(1.0, 0.5 + v / 2.0))
    };
    egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), rect.size()))
        .uv(uv)
        .corner_radius(radius)
        .paint_at(ui, rect);
}

/// The largest centred rect with the picture's aspect that fits in `rect`.
fn contain_rect(rect: Rect, size: [usize; 2]) -> Rect {
    let aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
    let fitted = if aspect > rect.width() / rect.height().max(1.0) {
        Vec2::new(rect.width(), rect.width() / aspect)
    } else {
        Vec2::new(rect.height() * aspect, rect.height())
    };
    Rect::from_center_size(rect.center(), fitted)
}

/// A snapshot shown whole, or the place icon on a darker face while there is
/// none; the bars left by another ratio show that face.
fn snapshot(ui: &egui::Ui, env: &mut Env, id: Uuid, rect: Rect, radius: egui::CornerRadius, icon: &str) {
    if !id.is_nil() {
        env.wanted_images.insert(id);
    }
    ui.painter().rect_filled(rect, radius, env.p.raised.gamma_multiply(0.6));
    match env.images.get(&id).filter(|_| !id.is_nil()) {
        Some(t) => {
            let fitted = contain_rect(rect, t.size());
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), fitted.size())).paint_at(ui, fitted);
        }
        None => paint_icon(ui, icon, rect.center(), 26.0, env.p.muted_dim),
    }
}

/// The empty state of a tab: an icon in a disc, a title, a line, and the
/// button that fills it when there is one. True when the button is clicked.
fn empty(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, text: &str, cta: Option<(&str, &str)>, note: Option<&str>) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(16, 24))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 23.0, p.bar);
                paint_icon(ui, icon, rect.center(), 22.0, p.muted_dim);
                ui.label(RichText::new(title).font(semibold(ui, 13.0)).color(p.ink));
                ui.add(
                    egui::Label::new(RichText::new(text).size(12.0).color(p.muted))
                        .wrap()
                        .halign(egui::Align::Center),
                );
                if let Some((icon, label)) = cta {
                    clicked = widgets::action_button(ui, p, icon, label, ActionKind::Primary, Vec2::new(0.0, 28.0)).clicked();
                }
                if let Some(note) = note {
                    ui.label(RichText::new(note).size(11.5).color(p.muted_dim));
                }
            });
        });
    clicked
}

/// A labelled field of an edit form.
fn field(ui: &mut egui::Ui, p: &Palette, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.label(RichText::new(label).size(11.5).color(p.muted));
        body(ui);
    });
}

/// "n / max" under a text, right aligned.
fn counter(ui: &mut egui::Ui, p: &Palette, text: String, warn: bool) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.label(RichText::new(text).size(11.0).color(if warn { p.warn } else { p.muted_dim }));
    });
}

/// The danger strip that asks before deleting (ProfileDeletePick,
/// ProfileDeleteClassified). Some(true) when confirmed, Some(false) when
/// cancelled.
fn confirm_strip(ui: &mut egui::Ui, p: &Palette, text: &str) -> Option<bool> {
    let mut answer = None;
    strip(ui, p, p.danger, |ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 24.0), Sense::hover());
        paint_icon(ui, "alert-triangle-filled", rect.center(), 13.0, p.danger);
        ui.label(RichText::new(text).font(semibold(ui, 12.0)).color(p.danger));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = Vec2::new(0.0, 24.0);
            if widgets::action_button(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger, size).clicked() {
                answer = Some(true);
            }
            if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
        });
    });
    answer
}

/// A tinted strip with its color on the left edge (the confirmations and
/// the publication of a classified).
fn strip(ui: &mut egui::Ui, p: &Palette, color: Color32, body: impl FnOnce(&mut egui::Ui)) {
    let resp = egui::Frame::new()
        .fill(color.gamma_multiply(0.10))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: 13,
            right: 10,
            top: 5,
            bottom: 5,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                body(ui);
            });
        })
        .response;
    let r = resp.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(r.min, pos2(r.left() + 3.0, r.bottom())),
        egui::CornerRadius::ZERO,
        color,
    );
    let _ = p;
}

// ------------------------------------------------------------- favorite places

/// What the featured place shows.
struct PickView {
    name: String,
    desc: String,
    snapshot: Uuid,
    pos: DVec3,
    place: String,
    /// The region, for the place link.
    sim: String,
    maturity: Option<&'static str>,
}

fn pick_view(env: &mut Env, w: &mut Window, id: Uuid) -> Option<PickView> {
    let world = env.world;
    if w.pick_edit.as_ref().is_some_and(|(e, _)| e.id == id)
        && let Some(i) = world.profiles.picks.get(&id).filter(|i| !i.parcel.is_nil())
    {
        w.ask(i.parcel, NetCommand::ParcelInfoRequest(i.parcel), env.actions);
    }
    if let Some((e, _)) = w.pick_edit.as_ref().filter(|(e, _)| e.id == id) {
        let info = world.profiles.picks.get(&id);
        let parcel = info.and_then(|i| world.profiles.parcels.get(&i.parcel));
        let place = match (&e.location, info) {
            (Some(l), _) => l.clone(),
            (None, Some(i)) => place_line(parcel.map_or("", |p| p.name.as_str()), &i.sim_name, e.pos),
            (None, None) => String::new(),
        };
        return Some(PickView {
            name: e.name.clone(),
            desc: e.desc.clone(),
            snapshot: e.snapshot,
            pos: e.pos,
            place,
            sim: info.map(|i| i.sim_name.clone()).unwrap_or_else(|| world.region_name()),
            maturity: maturity_of(parcel),
        });
    }
    let info = world.profiles.picks.get(&id)?;
    let parcel = if info.parcel.is_nil() {
        None
    } else {
        let pa = world.profiles.parcels.get(&info.parcel);
        if pa.is_none() {
            w.ask(info.parcel, NetCommand::ParcelInfoRequest(info.parcel), env.actions);
        }
        pa
    };
    Some(PickView {
        name: info.name.clone(),
        desc: info.desc.clone(),
        snapshot: info.snapshot,
        pos: info.pos_global,
        place: place_line(parcel.map_or("", |p| p.name.as_str()), &info.sim_name, info.pos_global),
        sim: info.sim_name.clone(),
        maturity: maturity_of(parcel),
    })
}

/// Start a new pick at our position (LLPanelProfilePicks::onClickNewBtn).
fn start_new_pick(w: &mut Window, world: &World) {
    if world.rlva.has("showloc") {
        return;
    }
    w.editing = true;
    w.pick_edit = Some((new_pick(world), PickEdit::default()));
    w.pick_sel = Some(Uuid::nil());
    w.confirm_delete = false;
}

/// Favorite places: the chosen one featured, the others as thumbnails under
/// it; our own edited in place (name, description, snapshot, our position
/// here, delete), with the account's limit on the card that adds one.
pub(in super::super) fn picks(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, compact: bool) {
    let (p, world) = (env.p, env.world);
    let own = w.id == world.agent_id;
    let Some(pr) = world.profiles.get(&w.id) else {
        loading(ui, p);
        return;
    };
    let limit = picks_limit(world);
    let editing = own && w.editing;
    if !editing && w.pick_edit.as_ref().is_some_and(|(e, base)| e == base) {
        w.pick_edit = None;
    }
    let mut items: Vec<(Uuid, String)> = pr.picks.clone();
    if let Some((e, _)) = &w.pick_edit
        && e.id.is_nil()
    {
        items.push((Uuid::nil(), e.name.clone()));
    }
    if items.is_empty() {
        let name = display_name(world, &w.id);
        let count = aurora_i18n::tr!("ui-profile-aurora-picks-count", count = 0, limit = limit as i64);
        let clicked = if own {
            empty(
                ui,
                p,
                "map-pin-filled",
                aurora_i18n::tr!("ui-profile-aurora-picks-empty-own"),
                aurora_i18n::tr!("ui-profile-aurora-picks-empty-own-text"),
                Some(("plus", aurora_i18n::tr!("ui-profile-aurora-add-here"))),
                Some(&count),
            )
        } else {
            empty(
                ui,
                p,
                "map-pin-filled",
                aurora_i18n::tr!("ui-profile-aurora-picks-empty"),
                &aurora_i18n::tr!("ui-profile-aurora-picks-empty-text", name = name),
                None,
                None,
            )
        };
        if clicked && !world.rlva.has("showloc") {
            start_new_pick(w, world);
        }
        return;
    }
    if w.pick_sel.is_none_or(|s| !items.iter().any(|(id, _)| *id == s)) {
        w.pick_sel = items.first().map(|(id, _)| *id);
    }
    let sel = w.pick_sel.unwrap_or_default();
    // every pick's details, as Firestorm's panel of each pick asks them
    for (id, _) in &items {
        w.ask(*id, NetCommand::PickInfoRequest { creator: w.id, pick: *id }, env.actions);
    }
    // our selected pick is edited in a copy of the server's
    if editing && let Some(i) = world.profiles.picks.get(&sel) {
        let server = PickEdit {
            id: i.id,
            name: i.name.clone(),
            desc: i.desc.clone(),
            snapshot: i.snapshot,
            pos: i.pos_global,
            location: None,
        };
        let follow = match &w.pick_edit {
            Some((e, base)) => e.id != i.id || (e == base && *base != server),
            None => true,
        };
        if follow {
            w.pick_edit = Some((server.clone(), server));
        }
    }
    match pick_view(env, w, sel) {
        Some(v) => feature(ui, env, w, &v, editing, compact),
        None => loading(ui, p),
    }
    if editing && w.confirm_delete {
        let name = w.pick_edit.as_ref().map(|(e, _)| e.name.clone()).unwrap_or_default();
        match confirm_strip(ui, p, &aurora_i18n::tr!("ui-profile-aurora-delete-pick", name = name)) {
            Some(true) => {
                if !sel.is_nil() {
                    env.actions.push(ProfileAction::Net(NetCommand::PickDelete(sel)));
                }
                w.pick_edit = None;
                w.pick_sel = None;
                w.confirm_delete = false;
            }
            Some(false) => w.confirm_delete = false,
            None => {}
        }
    }
    let add = editing && !world.rlva.has("showloc") && pr.picks.len() < limit && !items.iter().any(|(id, _)| id.is_nil());
    thumbs(ui, env, w, &items, add.then_some((pr.picks.len(), limit)), compact);
}

/// The featured place: its snapshot and rating, name, place, description
/// and actions; the edit form on our own when editing.
fn feature(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, v: &PickView, editing: bool, compact: bool) {
    let p = env.p;
    let radius = ui.visuals().window_corner_radius;
    let width = ui.available_width();
    egui::Frame::new().fill(p.bar).corner_radius(radius).show(ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let image_size = if compact {
            Vec2::new(width, 120.0)
        } else {
            Vec2::new(FEATURE_W.min(width * 0.45), FEATURE_H)
        };
        let body = |ui: &mut egui::Ui, env: &mut Env, w: &mut Window| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.set_min_height(if compact { 0.0 } else { FEATURE_H - 24.0 });
                if editing {
                    pick_form(ui, env, w, v);
                } else {
                    pick_read(ui, env, v, compact);
                }
            });
        };
        let layout = if compact {
            egui::Layout::top_down(egui::Align::Min)
        } else {
            egui::Layout::left_to_right(egui::Align::Min)
        };
        ui.with_layout(layout, |ui| {
            let (rect, picture) = ui.allocate_exact_size(image_size, Sense::click());
            let corners = if compact {
                egui::CornerRadius {
                    nw: radius.nw,
                    ne: radius.ne,
                    sw: 0,
                    se: 0,
                }
            } else {
                egui::CornerRadius {
                    nw: radius.nw,
                    sw: radius.sw,
                    ne: 0,
                    se: 0,
                }
            };
            snapshot(ui, env, v.snapshot, rect, corners, "map-pin-filled");
            if !editing && !v.snapshot.is_nil() && picture.on_hover_text(aurora_i18n::tr!("ui-profile-aurora-big-photo")).clicked() {
                w.open_photo(PictureSlot::Pick(w.id), v.snapshot);
            }
            if let Some(m) = v.maturity {
                let badge = Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::splat(15.0));
                widgets::paint_maturity_badge(ui.painter(), p, m, badge, widgets::inner_radius(ui.visuals()));
            }
            if editing {
                let chip = Rect::from_min_size(rect.right_bottom() - Vec2::new(8.0 + 132.0, 8.0 + 24.0), Vec2::new(132.0, 24.0));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(chip));
                if widgets::action_button(
                    &mut c,
                    p,
                    "photo-filled",
                    aurora_i18n::tr!("ui-profile-aurora-change-image"),
                    ActionKind::Primary,
                    chip.size(),
                )
                .clicked()
                {
                    *env.pick_picture = Some((PictureSlot::Pick(w.id), v.snapshot));
                }
            }
            ui.vertical(|ui| body(ui, env, w));
        });
    });
}

/// The pick as others see it.
fn pick_read(ui: &mut egui::Ui, env: &mut Env, v: &PickView, compact: bool) {
    let (p, world) = (env.p, env.world);
    let top = ui.cursor().top();
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.label(RichText::new(&v.name).font(semibold(ui, 16.0)).color(p.ink));
    place_row(ui, p, &world.rlva_filter_text(&v.place));
    if !v.desc.trim().is_empty() {
        rich(ui, p, env.emoji, world, env.want_names, &v.desc);
    }
    // the buttons at the bottom of the card, as high as the picture beside
    let bottom = if compact { 0.0 } else { top + FEATURE_H - 24.0 - 28.0 };
    ui.add_space((bottom - ui.cursor().top()).max(4.0));
    pick_buttons(ui, env, v.pos, &v.sim);
}

/// "map-pin" and the place, in the muted color.
fn place_row(ui: &mut egui::Ui, p: &Palette, place: &str) {
    if place.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(13.0, 16.0), Sense::hover());
        paint_icon(ui, "map-pin-filled", rect.center(), 12.0, p.muted);
        ui.add(egui::Label::new(RichText::new(place).size(12.0).color(p.muted)).truncate());
    });
}

/// Teleport, Map and copy the place link (LLPanelProfilePick::onClickTeleport,
/// onClickMap).
fn pick_buttons(ui: &mut egui::Ui, env: &mut Env, pos: DVec3, region: &str) {
    let p = env.p;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let has_pos = pos != DVec3::ZERO;
        let size = Vec2::new(0.0, 28.0);
        if widgets::action_button_if(
            ui,
            p,
            "send-filled",
            aurora_i18n::tr!("ui-profile-aurora-teleport"),
            ActionKind::Primary,
            size,
            has_pos,
        )
        .clicked()
        {
            env.actions.push(ProfileAction::Teleport(pos));
        }
        if widgets::action_button_if(ui, p, "map", aurora_i18n::tr!("common-map"), ActionKind::Flat, size, has_pos).clicked() {
            env.actions.push(ProfileAction::ShowOnMap(pos));
        }
        if let Some(slurl) = slurl(region, pos)
            && widgets::action_button(ui, p, "copy-filled", "", ActionKind::Flat, Vec2::new(30.0, 28.0))
                .on_hover_text(aurora_i18n::tr!("ui-profile-aurora-copy-slurl"))
                .clicked()
        {
            ui.ctx().copy_text(slurl);
        }
    });
}

/// The place link of a position in a region (LLSLURL's maps address).
fn slurl(region: &str, pos: DVec3) -> Option<String> {
    if pos == DVec3::ZERO || region.trim().is_empty() {
        return None;
    }
    let local = |v: f64| v.rem_euclid(256.0).round() as i64;
    Some(format!(
        "http://maps.secondlife.com/secondlife/{}/{}/{}/{}",
        region.trim().replace(' ', "%20"),
        local(pos.x),
        local(pos.y),
        pos.z.round() as i64
    ))
}

/// Our pick edited in place (LLPanelProfilePick in edit mode).
fn pick_form(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, v: &PickView) {
    let (p, world) = (env.p, env.world);
    ui.spacing_mut().item_spacing.y = 8.0;
    let Some((e, _)) = w.pick_edit.as_mut() else {
        return;
    };
    field(ui, p, aurora_i18n::tr!("common-name"), |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut e.name)
                .desired_width(ui.available_width())
                .char_limit(PICK_NAME_MAX)
                .font(semibold(ui, 12.5)),
        );
    });
    field(ui, p, aurora_i18n::tr!("ui-profile-aurora-description"), |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut e.desc)
                .desired_width(ui.available_width())
                .desired_rows(2)
                .char_limit(PICK_DESC_MAX)
                .font(FontId::proportional(12.5)),
        );
        counter(
            ui,
            p,
            aurora_i18n::tr!(
                "ui-profile-aurora-char-count",
                count = aurora_i18n::integer(e.desc.chars().count() as i64),
                max = aurora_i18n::integer(PICK_DESC_MAX as i64)
            ),
            false,
        );
    });
    place_row(ui, p, &world.rlva_filter_text(&v.place));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let size = Vec2::new(0.0, 28.0);
        if widgets::action_button_if(
            ui,
            p,
            "current-location-filled",
            aurora_i18n::tr!("ui-profile-aurora-here"),
            ActionKind::Flat,
            size,
            !world.rlva.has("showloc"),
        )
        .clicked()
        {
            // onClickSetLocation: our position, parcel and region now
            let here = new_pick(world);
            e.pos = here.pos;
            e.location = here.location;
        }
        let has_pos = e.pos != DVec3::ZERO;
        if widgets::action_button_if(
            ui,
            p,
            "send-filled",
            aurora_i18n::tr!("ui-profile-aurora-teleport"),
            ActionKind::Flat,
            size,
            has_pos,
        )
        .clicked()
        {
            env.actions.push(ProfileAction::Teleport(e.pos));
        }
        if widgets::action_button_if(ui, p, "map", aurora_i18n::tr!("common-map"), ActionKind::Flat, size, has_pos).clicked() {
            env.actions.push(ProfileAction::ShowOnMap(e.pos));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::action_button(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger, size).clicked() {
                w.confirm_delete = true;
            }
        });
    });
}

/// The other places as thumbnails; the card that adds one, with the
/// account's limit (LLAgentBenefitsMgr picks_limit).
fn thumbs(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, items: &[(Uuid, String)], add: Option<(usize, usize)>, compact: bool) {
    let p = env.p;
    let columns = if compact { 2 } else { 4 };
    let gap = 8.0;
    let col_w = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor();
    let mut chosen = None;
    let mut adding = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(gap);
        for (id, name) in items {
            let v = pick_view(env, w, *id);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, THUMB_IMG_H + 40.0), Sense::click());
            let radius = ui.visuals().window_corner_radius;
            ui.painter().rect_filled(rect, radius, p.bar);
            let img = Rect::from_min_size(rect.min, Vec2::new(col_w, THUMB_IMG_H));
            let top = egui::CornerRadius {
                nw: radius.nw,
                ne: radius.ne,
                sw: 0,
                se: 0,
            };
            snapshot(ui, env, v.as_ref().map_or(Uuid::nil(), |v| v.snapshot), img, top, "map-pin-filled");
            if let Some(m) = v.as_ref().and_then(|v| v.maturity) {
                let badge = Rect::from_min_size(img.min + Vec2::splat(5.0), Vec2::splat(14.0));
                widgets::paint_maturity_badge(ui.painter(), p, m, badge, widgets::inner_radius(ui.visuals()));
            }
            let title = v.as_ref().map_or(name.as_str(), |v| v.name.as_str());
            widgets::paint_elided(
                ui,
                title,
                pos2(rect.left() + 8.0, img.bottom() + 13.0),
                col_w - 16.0,
                semibold(ui, 12.0),
                p.ink,
            );
            let place = v.as_ref().map_or("", |v| v.place.as_str());
            widgets::paint_elided(
                ui,
                place,
                pos2(rect.left() + 8.0, img.bottom() + 29.0),
                col_w - 16.0,
                FontId::proportional(11.0),
                p.muted,
            );
            if w.pick_sel == Some(*id) {
                ui.painter()
                    .rect_stroke(rect, radius, egui::Stroke::new(2.0, p.violet), egui::StrokeKind::Outside);
            } else if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                chosen = Some(*id);
            }
        }
        if let Some((count, limit)) = add {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, THUMB_IMG_H + 40.0), Sense::click());
            dashed(ui, rect, p.violet_light.gamma_multiply(0.45));
            let c = rect.center();
            paint_icon(ui, "plus", c - Vec2::new(0.0, 22.0), 18.0, p.violet_light);
            let g = ui.painter().layout(
                aurora_i18n::tr!("ui-profile-aurora-add-here").to_owned(),
                semibold(ui, 12.0),
                p.violet_light,
                col_w - 16.0,
            );
            let gh = g.size().y;
            ui.painter().galley(pos2(c.x - g.size().x / 2.0, c.y - 8.0), g, p.violet_light);
            let n = aurora_i18n::tr!("ui-profile-aurora-picks-count", count = count as i64, limit = limit as i64);
            let g = ui.painter().layout(n, FontId::proportional(11.0), p.muted_dim, col_w - 16.0);
            ui.painter()
                .galley(pos2(c.x - g.size().x / 2.0, c.y - 8.0 + gh + 2.0), g, p.muted_dim);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            adding = resp.clicked();
        }
    });
    if let Some(id) = chosen {
        // an edit not saved keeps its pick (the unsaved bar is there)
        let dirty = w.pick_edit.as_ref().is_some_and(|(e, base)| e != base);
        if !dirty || w.pick_edit.as_ref().is_some_and(|(e, _)| e.id == id) {
            if w.pick_edit.as_ref().is_some_and(|(e, _)| e.id != id) {
                w.pick_edit = None;
            }
            w.pick_sel = Some(id);
            w.confirm_delete = false;
        }
    }
    if adding {
        start_new_pick(w, env.world);
    }
}

/// A dashed outline (the cards that add something, the Give zone).
fn dashed(ui: &egui::Ui, rect: Rect, color: Color32) {
    let r = rect.shrink(0.75);
    let path = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    ui.painter()
        .extend(egui::Shape::dashed_line(&path, egui::Stroke::new(1.5, color), 5.0, 4.0));
}

// ------------------------------------------------------------------ classifieds

/// The classified being edited, from the server's or a new one at our
/// position (LLPanelProfileClassified::setEditMode, resetControls).
fn edit_of(c: &ClassifiedInfo) -> ClassifiedEdit {
    ClassifiedEdit {
        id: c.id,
        name: c.name.clone(),
        desc: c.desc.clone(),
        snapshot: c.snapshot,
        category: c.category,
        mature: classified_flags::is_mature(c.flags),
        auto_renew: c.flags & classified_flags::AUTO_RENEW != 0,
        pos: c.pos_global,
        parcel: c.parcel,
        sim_name: c.sim_name.clone(),
        parcel_name: c.parcel_name.clone(),
        location: None,
        price: c.price,
    }
}

fn new_classified(world: &World) -> ClassifiedEdit {
    if world.rlva.has("showloc") {
        return ClassifiedEdit::default();
    }
    let here = new_pick(world);
    let (parcel_name, snapshot) = world
        .parcel
        .as_ref()
        .map_or((String::new(), Uuid::nil()), |pa| (pa.name.clone(), pa.snapshot_id));
    ClassifiedEdit {
        id: Uuid::nil(),
        snapshot,
        category: world.classified_categories().first().map_or(1, |(id, _)| *id),
        pos: here.pos,
        sim_name: world.region_name(),
        parcel_name,
        location: here.location,
        price: MINIMUM_PRICE_FOR_LISTING,
        ..Default::default()
    }
}

/// Classifieds: one card per classified, the chosen one unfolded with its
/// description and actions; ours are edited in their card, and a new one
/// is published after its price is confirmed.
pub(in super::super) fn classifieds(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, compact: bool) {
    let (p, world) = (env.p, env.world);
    let own = w.id == world.agent_id;
    if !w.classifieds_asked {
        w.classifieds_asked = true;
        env.actions.push(ProfileAction::Net(NetCommand::ClassifiedsRequest(w.id)));
    }
    let mut ids: Vec<Uuid> = world
        .profiles
        .classified_lists
        .get(&w.id)
        .map(|l| l.iter().map(|(id, _)| *id).collect())
        .unwrap_or_default();
    if let Some((e, _)) = &w.classified_edit
        && e.id.is_nil()
    {
        ids.push(Uuid::nil());
    }
    if ids.is_empty() {
        let name = display_name(world, &w.id);
        let clicked = if own {
            empty(
                ui,
                p,
                "speakerphone",
                aurora_i18n::tr!("ui-profile-aurora-classifieds-empty-own"),
                &aurora_i18n::tr!("ui-profile-aurora-classifieds-empty-own-text", price = MINIMUM_PRICE_FOR_LISTING),
                Some(("plus", aurora_i18n::tr!("ui-profile-aurora-new-classified"))),
                None,
            )
        } else {
            empty(
                ui,
                p,
                "speakerphone",
                aurora_i18n::tr!("ui-profile-aurora-classifieds-empty"),
                &aurora_i18n::tr!("ui-profile-aurora-classifieds-empty-text", name = name),
                None,
                None,
            )
        };
        if clicked && !world.rlva.has("showloc") {
            w.classified_edit = Some((new_classified(world), ClassifiedEdit::default()));
            w.classified_sel = Some(Uuid::nil());
        }
        return;
    }
    if w.classified_sel.is_none_or(|s| !ids.contains(&s)) {
        w.classified_sel = ids.first().copied();
    }
    // AURORA_DEMO_PROFILE=self:3:form / self:3:publish
    match w.demo_classified {
        Some(false) if own => {
            if let Some(c) = ids.first().and_then(|id| world.profiles.classifieds.get(id)) {
                let e = edit_of(c);
                w.classified_edit = Some((e.clone(), e));
                w.demo_classified = None;
            }
        }
        Some(true) if own => {
            let e = ClassifiedEdit {
                name: aurora_i18n::tr!("ui-profile-aurora-new-classified").to_owned(),
                ..new_classified(world)
            };
            w.classified_edit = Some((e, ClassifiedEdit::default()));
            w.publish = Some(Publish::default());
            w.demo_classified = None;
        }
        _ => {}
    }
    // each classified's details, statistics and profile click, once
    // (LLPanelProfileClassified::onOpen)
    for id in &ids {
        if id.is_nil() {
            continue;
        }
        w.ask(*id, NetCommand::ClassifiedInfoRequest(*id), env.actions);
        if let Some(info) = world.profiles.classifieds.get(id)
            && w.classifieds_opened.insert(*id)
        {
            env.actions.push(ProfileAction::Net(NetCommand::ClassifiedStatsRequest(*id)));
            env.actions.push(ProfileAction::Net(NetCommand::ClassifiedClick {
                kind: ClassifiedClick::Profile,
                classified: Box::new(info.clone()),
            }));
        }
    }
    ui.spacing_mut().item_spacing.y = 8.0;
    for id in ids {
        classified_card(ui, env, w, id, own, compact);
    }
    if let Some(mut publish) = w.publish.take() {
        let name = w.classified_edit.as_ref().map(|(e, _)| e.name.clone()).unwrap_or_default();
        match publish_strip(ui, p, &mut publish, &name) {
            Some(true) => match publish_price(&publish.price) {
                Ok(price) => {
                    if let Some((e, _)) = w.classified_edit.take() {
                        let c = classified_of(&ClassifiedEdit { price, ..e });
                        w.classified_sel = Some(c.id);
                        env.actions.push(ProfileAction::Net(NetCommand::ClassifiedUpdate(Box::new(c))));
                    }
                }
                Err(e) => {
                    publish.error = Some(e);
                    w.publish = Some(publish);
                }
            },
            Some(false) => {}
            None => w.publish = Some(publish),
        }
    }
    let creating = w.classified_edit.as_ref().is_some_and(|(e, _)| e.id.is_nil());
    if own && !creating && !world.rlva.has("showloc") {
        ui.horizontal(|ui| {
            ui.add_space(0.0);
            if widgets::action_button(
                ui,
                p,
                "plus",
                aurora_i18n::tr!("ui-profile-aurora-new-classified"),
                ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            )
            .clicked()
            {
                w.classified_edit = Some((new_classified(world), ClassifiedEdit::default()));
                w.classified_sel = Some(Uuid::nil());
                w.confirm_delete = false;
            }
        });
    }
}

/// The strip that asks the price of a new classified (floater_price_for_listing.xml).
/// Some(true) on Publish, Some(false) on Cancel.
fn publish_strip(ui: &mut egui::Ui, p: &Palette, publish: &mut Publish, name: &str) -> Option<bool> {
    let mut answer = None;
    if publish.price.is_empty() {
        publish.price = MINIMUM_PRICE_FOR_LISTING.to_string();
    }
    strip(ui, p, p.amber, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 24.0), Sense::hover());
                paint_icon(ui, "coin-filled", rect.center(), 13.0, p.amber);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-aurora-publish", name = name))
                        .font(semibold(ui, 12.0))
                        .color(p.amber),
                );
                ui.add(egui::TextEdit::singleline(&mut publish.price).desired_width(56.0).char_limit(9));
                ui.label(RichText::new("L$").font(semibold(ui, 12.0)).color(p.amber));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let size = Vec2::new(0.0, 24.0);
                    if widgets::action_button(
                        ui,
                        p,
                        "coin-filled",
                        aurora_i18n::tr!("ui-profile-aurora-publish-button"),
                        ActionKind::Primary,
                        size,
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            });
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-profile-aurora-publish-help"))
                        .size(11.5)
                        .color(p.muted),
                )
                .wrap(),
            );
            if let Some(e) = &publish.error {
                ui.label(RichText::new(e).size(11.5).color(p.danger));
            }
        });
    });
    answer
}

/// One classified: its head (snapshot, title, category, place, date,
/// renewal, cost); unfolded, its description and actions, or its edit form.
fn classified_card(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, id: Uuid, own: bool, compact: bool) {
    let (p, world) = (env.p, env.world);
    let info = world.profiles.classifieds.get(&id);
    let editing = w.classified_edit.as_ref().is_some_and(|(e, _)| e.id == id);
    let open = w.classified_sel == Some(id) || editing;
    let radius = ui.visuals().window_corner_radius;
    let width = ui.available_width();
    let mut toggle = false;
    let resp = egui::Frame::new()
        .fill(p.bar)
        .corner_radius(radius)
        .show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 0.0);
            // the head: a click unfolds it
            let head = egui::Frame::new().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(width - 16.0);
                if editing {
                    return;
                }
                let Some(c) = info else {
                    loading(ui, p);
                    return;
                };
                classified_head(ui, env, c, compact);
            });
            if !editing && head.response.interact(Sense::click()).clicked() {
                toggle = true;
            }
            if editing {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
                    ui.set_width(width - 20.0);
                    classified_form(ui, env, w, compact);
                });
                ui.add_space(10.0);
            } else if open && let Some(c) = info {
                widgets::divider(ui, p);
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 9)).show(ui, |ui| {
                    ui.set_width(width - 20.0);
                    classified_more(ui, env, w, c, own);
                });
            }
        })
        .response;
    if open {
        ui.painter()
            .rect_stroke(resp.rect, radius, egui::Stroke::new(2.0, p.violet), egui::StrokeKind::Inside);
    }
    if toggle {
        w.classified_sel = if w.classified_sel == Some(id) { None } else { Some(id) };
        w.confirm_delete = false;
    }
    if open && w.confirm_delete && !id.is_nil() {
        let name = info.map(|c| c.name.clone()).unwrap_or_default();
        match confirm_strip(ui, p, &aurora_i18n::tr!("ui-profile-aurora-delete-classified", name = name)) {
            Some(true) => {
                env.actions.push(ProfileAction::Net(NetCommand::ClassifiedDelete(id)));
                w.classified_sel = None;
                w.classified_edit = None;
                w.confirm_delete = false;
            }
            Some(false) => w.confirm_delete = false,
            None => {}
        }
    }
}

/// Snapshot with its rating, title, category and place, date and renewal,
/// cost on the right.
fn classified_head(ui: &mut egui::Ui, env: &mut Env, c: &ClassifiedInfo, compact: bool) {
    let (p, world) = (env.p, env.world);
    let radius = ui.visuals().window_corner_radius;
    let rating = if classified_flags::is_mature(c.flags) {
        aurora_i18n::text("common-moderate")
    } else {
        aurora_i18n::text("common-general")
    };
    let picture = |ui: &mut egui::Ui, env: &mut Env, size: Vec2| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        snapshot(ui, env, c.snapshot, rect, radius, "speakerphone");
        let badge = Rect::from_min_size(rect.min + Vec2::splat(5.0), Vec2::splat(14.0));
        widgets::paint_maturity_badge(ui.painter(), p, rating, badge, widgets::inner_radius(ui.visuals()));
    };
    let text = |ui: &mut egui::Ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add(egui::Label::new(RichText::new(&c.name).font(semibold(ui, 13.5)).color(p.ink)).truncate());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let category = world.classified_category(c.category);
                if !category.is_empty() {
                    chip(ui, p, &category);
                }
                let (r, _) = ui.allocate_exact_size(Vec2::new(12.0, 16.0), Sense::hover());
                paint_icon(ui, "map-pin-filled", r.center(), 11.0, p.muted);
                ui.label(
                    RichText::new(world.rlva_filter_text(&place_line(&c.parcel_name, &c.sim_name, c.pos_global)))
                        .size(11.5)
                        .color(p.muted),
                );
            });
            let date = profiles::slt_date(c.creation_date);
            let line = if c.flags & classified_flags::AUTO_RENEW != 0 {
                aurora_i18n::tr!("ui-profile-aurora-published-renewed", date = date)
            } else {
                aurora_i18n::tr!("ui-profile-aurora-published", date = date)
            };
            ui.label(RichText::new(line).size(11.5).color(p.muted));
        });
    };
    let price = |ui: &mut egui::Ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                ui.label(
                    RichText::new(format!("L$ {}", aurora_i18n::integer(c.price as i64)))
                        .font(semibold(ui, 12.5))
                        .color(p.amber),
                );
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-aurora-listing-cost"))
                        .size(11.0)
                        .color(p.muted_dim),
                );
            });
        });
    };
    if compact {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            picture(ui, env, Vec2::new(ui.available_width(), 110.0));
            text(ui);
            ui.label(
                RichText::new(format!("L$ {}", aurora_i18n::integer(c.price as i64)))
                    .font(semibold(ui, 12.0))
                    .color(p.amber),
            );
        });
    } else {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            picture(ui, env, Vec2::new(120.0, 76.0));
            let price_w = 110.0;
            let text_w = ui.available_width() - price_w - 10.0;
            ui.allocate_ui_with_layout(Vec2::new(text_w, 76.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(text_w);
                text(ui);
            });
            ui.allocate_ui_with_layout(Vec2::new(price_w, 76.0), egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.set_width(price_w);
                price(ui);
            });
        });
    }
}

/// A small grey chip (the category).
fn chip(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let g = ui.painter().layout_no_wrap(text.to_owned(), semibold(ui, 11.0), p.muted);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(g.size().x + 14.0, 17.0), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    widgets::paint_ink_left(ui.painter(), g, pos2(rect.left() + 7.0, rect.center().y), p.muted);
}

/// The unfolded classified: description, content rating, our clicks,
/// Teleport and Map (with their click reports), Edit and Delete for ours.
fn classified_more(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, c: &ClassifiedInfo, own: bool) {
    let (p, world) = (env.p, env.world);
    ui.spacing_mut().item_spacing.y = 8.0;
    if !c.desc.trim().is_empty() {
        rich(ui, p, env.emoji, world, env.want_names, &c.desc);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rating, content) = if classified_flags::is_mature(c.flags) {
            (
                aurora_i18n::text("common-moderate"),
                aurora_i18n::tr!("ui-profile-aurora-content-moderate"),
            )
        } else {
            (
                aurora_i18n::text("common-general"),
                aurora_i18n::tr!("ui-profile-aurora-content-general"),
            )
        };
        widgets::maturity_badge(ui, p, rating, 14.0);
        ui.label(RichText::new(content).size(11.5).color(p.muted));
    });
    if own {
        stats(ui, env, c);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let size = Vec2::new(0.0, 28.0);
        let has_pos = c.pos_global != DVec3::ZERO;
        if widgets::action_button_if(
            ui,
            p,
            "send-filled",
            aurora_i18n::tr!("ui-profile-aurora-teleport"),
            ActionKind::Primary,
            size,
            has_pos,
        )
        .clicked()
        {
            env.actions.push(ProfileAction::Net(NetCommand::ClassifiedClick {
                kind: ClassifiedClick::Teleport,
                classified: Box::new(c.clone()),
            }));
            env.actions.push(ProfileAction::Teleport(c.pos_global));
        }
        if widgets::action_button_if(ui, p, "map", aurora_i18n::tr!("common-map"), ActionKind::Flat, size, has_pos).clicked() {
            env.actions.push(ProfileAction::Net(NetCommand::ClassifiedClick {
                kind: ClassifiedClick::Map,
                classified: Box::new(c.clone()),
            }));
            env.actions.push(ProfileAction::ShowOnMap(c.pos_global));
        }
        if own {
            if widgets::action_button(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit"), ActionKind::Flat, size).clicked() {
                let e = edit_of(c);
                w.classified_edit = Some((e.clone(), e));
                w.confirm_delete = false;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::action_button(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger, size).clicked() {
                    w.confirm_delete = true;
                }
            });
        }
    });
}

/// Our clicks and what the classified cost (click_through_text_fmt).
fn stats(ui: &mut egui::Ui, env: &Env, c: &ClassifiedInfo) {
    let p = env.p;
    let s = env.world.profiles.classified_stats.get(&c.id).copied().unwrap_or_default();
    let cells: [(String, String); 4] = [
        (
            aurora_i18n::integer(s.teleport as i64),
            aurora_i18n::tr!("ui-profile-aurora-stat-teleports").to_owned(),
        ),
        (
            aurora_i18n::integer(s.map as i64),
            aurora_i18n::tr!("ui-profile-aurora-stat-maps").to_owned(),
        ),
        (
            aurora_i18n::integer(s.profile as i64),
            aurora_i18n::tr!("ui-profile-aurora-stat-profiles").to_owned(),
        ),
        (
            format!("L$ {}", aurora_i18n::integer(c.price as i64)),
            aurora_i18n::tr!("ui-profile-aurora-stat-cost", date = profiles::slt_date(c.creation_date)),
        ),
    ];
    let gap = 6.0;
    let cell_w = ((ui.available_width() - gap * 3.0) / 4.0).floor();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (value, label) in cells {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(cell_w, 40.0), Sense::hover());
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
            ui.painter().text(
                rect.left_top() + Vec2::new(9.0, 5.0),
                egui::Align2::LEFT_TOP,
                value,
                semibold(ui, 15.0),
                p.ink,
            );
            widgets::paint_elided(
                ui,
                &label,
                pos2(rect.left() + 9.0, rect.bottom() - 10.0),
                cell_w - 18.0,
                FontId::proportional(11.0),
                p.muted,
            );
        }
    });
}

/// Our classified as a form (panel_profile_classified.xml edit_panel):
/// snapshot, title, description, category, content, place, renewal; Save
/// goes through the unsaved bar, a new one through Publish.
fn classified_form(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, compact: bool) {
    let (p, world) = (env.p, env.world);
    let window = w.id;
    let balance = world.balance;
    let Some((e, _)) = w.classified_edit.as_mut() else {
        return;
    };
    let creating = e.id.is_nil();
    let mut publish_asked = false;
    let mut cancel = false;
    let mut delete = false;
    let radius = ui.visuals().window_corner_radius;
    ui.spacing_mut().item_spacing = Vec2::new(10.0, 8.0);
    ui.add_space(8.0);
    let top = |ui: &mut egui::Ui, env: &mut Env, e: &mut ClassifiedEdit| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(if compact { ui.available_width() } else { 180.0 }, 120.0), Sense::hover());
        snapshot(ui, env, e.snapshot, rect, radius, "speakerphone");
        let chip = Rect::from_min_size(rect.right_bottom() - Vec2::new(8.0 + 84.0, 8.0 + 24.0), Vec2::new(84.0, 24.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(chip));
        if widgets::action_button(
            &mut c,
            p,
            "photo-filled",
            aurora_i18n::tr!("ui-profile-aurora-image"),
            ActionKind::Primary,
            chip.size(),
        )
        .clicked()
        {
            *env.pick_picture = Some((PictureSlot::Classified(window), e.snapshot));
        }
    };
    let texts = |ui: &mut egui::Ui, e: &mut ClassifiedEdit| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field(ui, p, aurora_i18n::tr!("ui-profile-aurora-title"), |ui| {
                let typed = ui.add(
                    egui::TextEdit::singleline(&mut e.name)
                        .desired_width(ui.available_width())
                        .font(semibold(ui, 12.5)),
                );
                if typed.changed() {
                    clip_bytes(&mut e.name, CLASSIFIED_NAME_BYTES);
                }
                counter(
                    ui,
                    p,
                    aurora_i18n::tr!(
                        "ui-profile-aurora-bytes-count",
                        count = e.name.len() as i64,
                        max = CLASSIFIED_NAME_BYTES as i64
                    ),
                    e.name.len() >= CLASSIFIED_NAME_BYTES,
                );
            });
            field(ui, p, aurora_i18n::tr!("ui-profile-aurora-description"), |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut e.desc)
                        .desired_width(ui.available_width())
                        .desired_rows(2)
                        .char_limit(CLASSIFIED_DESC_MAX)
                        .font(FontId::proportional(12.5)),
                );
                counter(
                    ui,
                    p,
                    aurora_i18n::tr!(
                        "ui-profile-aurora-char-count",
                        count = aurora_i18n::integer(e.desc.chars().count() as i64),
                        max = aurora_i18n::integer(CLASSIFIED_DESC_MAX as i64)
                    ),
                    false,
                );
            });
        });
    };
    if compact {
        top(ui, env, e);
        texts(ui, e);
    } else {
        ui.horizontal_top(|ui| {
            top(ui, env, e);
            texts(ui, e);
        });
    }
    let half = ((ui.available_width() - 10.0) / 2.0).floor();
    let row = |ui: &mut egui::Ui, left: &mut dyn FnMut(&mut egui::Ui), right: &mut dyn FnMut(&mut egui::Ui)| {
        if compact {
            left(ui);
            right(ui);
        } else {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(half, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(half);
                    left(ui);
                });
                ui.allocate_ui_with_layout(Vec2::new(half, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(half);
                    right(ui);
                });
            });
        }
    };
    let categories = world.classified_categories();
    let mut category = e.category;
    let mut mature = e.mature;
    row(
        ui,
        &mut |ui| {
            field(ui, p, aurora_i18n::tr!("ui-profile-aurora-category"), |ui| {
                let current = categories
                    .iter()
                    .find(|(id, _)| *id == category)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt(("classified-category", window))
                    .width(ui.available_width())
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (id, name) in &categories {
                            ui.selectable_value(&mut category, *id, name);
                        }
                    });
            });
        },
        &mut |ui| {
            field(ui, p, aurora_i18n::tr!("ui-profile-aurora-content"), |ui| {
                let options = [
                    (aurora_i18n::tr!("common-general"), true),
                    (aurora_i18n::tr!("common-moderate"), true),
                ];
                let mut selected = usize::from(mature);
                if widgets::segmented(ui, p, &mut selected, &options) {
                    mature = selected == 1;
                }
            });
        },
    );
    e.category = category;
    e.mature = mature;
    let place = e.location.clone().unwrap_or_else(|| place_line(&e.parcel_name, &e.sim_name, e.pos));
    let place = world.rlva_filter_text(&place);
    let mut here = false;
    let mut renew = e.auto_renew;
    row(
        ui,
        &mut |ui| {
            field(ui, p, aurora_i18n::tr!("ui-profile-aurora-place"), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(place.as_ref()).size(12.0).color(p.muted)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        here = widgets::action_button_if(
                            ui,
                            p,
                            "current-location-filled",
                            aurora_i18n::tr!("ui-profile-aurora-here"),
                            ActionKind::Flat,
                            Vec2::new(0.0, 24.0),
                            !world.rlva.has("showloc"),
                        )
                        .clicked();
                    });
                });
            });
        },
        &mut |ui| {
            field(ui, p, aurora_i18n::tr!("ui-profile-aurora-renewal"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-profile-aurora-renew-weekly"))
                            .size(12.5)
                            .color(p.ink),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::switch(ui, p, &mut renew);
                    });
                });
            });
        },
    );
    e.auto_renew = renew;
    if here {
        // LLPanelProfileClassified::onSetLocationClick
        let n = new_classified(world);
        e.pos = n.pos;
        e.sim_name = n.sim_name;
        e.parcel_name = n.parcel_name;
        e.parcel = Uuid::nil();
        e.location = n.location;
    }
    if !creating && let Some(c) = world.profiles.classifieds.get(&e.id) {
        stats(ui, env, c);
    }
    let name_error = classified_name_error(&e.name).filter(|_| !e.name.is_empty() || creating);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let size = Vec2::new(0.0, 28.0);
        if creating {
            if widgets::action_button(
                ui,
                p,
                "coin-filled",
                aurora_i18n::tr!("ui-profile-aurora-publish-ellipsis"),
                ActionKind::Primary,
                size,
            )
            .clicked()
            {
                publish_asked = true;
            }
            if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                cancel = true;
            }
        } else {
            let has_pos = e.pos != DVec3::ZERO;
            if widgets::action_button_if(
                ui,
                p,
                "send-filled",
                aurora_i18n::tr!("ui-profile-aurora-teleport"),
                ActionKind::Flat,
                size,
                has_pos,
            )
            .clicked()
            {
                env.actions.push(ProfileAction::Teleport(e.pos));
            }
            if widgets::action_button_if(ui, p, "map", aurora_i18n::tr!("common-map"), ActionKind::Flat, size, has_pos).clicked() {
                env.actions.push(ProfileAction::ShowOnMap(e.pos));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                delete =
                    widgets::action_button(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger, size).clicked();
            });
        }
    });
    if let (true, Some(err)) = (publish_asked || !creating && !e.name.is_empty(), name_error) {
        ui.label(RichText::new(err).size(11.5).color(p.danger));
    }
    if publish_asked && name_error.is_none() {
        // LLPanelProfileClassified::onSaveClick: the balance first
        if balance.unwrap_or(0) < MINIMUM_PRICE_FOR_LISTING {
            w.publish = Some(Publish {
                price: MINIMUM_PRICE_FOR_LISTING.to_string(),
                error: Some(aurora_i18n::tr!("ui-profile-aurora-classified-no-funds").to_owned()),
            });
        } else {
            w.publish.get_or_insert_with(Publish::default);
        }
    }
    if cancel {
        w.classified_edit = None;
        w.publish = None;
        w.classified_sel = None;
    }
    if delete {
        w.confirm_delete = true;
    }
}

// ------------------------------------------------------------------ first life

/// First life (LLPanelProfileFirstLife): the picture in a square beside
/// the text; ours edited in place (picture menu, Write / Preview).
pub(in super::super) fn first_life(ui: &mut egui::Ui, env: &mut Env, w: &mut Window, compact: bool) {
    let (p, world) = (env.p, env.world);
    let own = w.id == world.agent_id;
    let Some(pr) = world.profiles.get(&w.id) else {
        loading(ui, p);
        return;
    };
    let image = pr.fl_image;
    let editing = own && w.editing;
    if !image.is_nil() {
        env.wanted_images.insert(image);
    }
    if !editing && image.is_nil() && w.fl_about.text.trim().is_empty() {
        let name = display_name(world, &w.id);
        let (title, text) = if own {
            (
                aurora_i18n::tr!("ui-profile-aurora-first-life-empty-own"),
                aurora_i18n::tr!("ui-profile-aurora-first-life-empty-own-text").to_owned(),
            )
        } else {
            (
                aurora_i18n::tr!("ui-profile-aurora-first-life-empty"),
                aurora_i18n::tr!("ui-profile-aurora-first-life-empty-text", name = name),
            )
        };
        let cta = own.then_some(("pencil-filled", aurora_i18n::tr!("ui-profile-aurora-edit")));
        if empty(ui, p, "user-heart", title, &text, cta, None) {
            w.editing = true;
        }
        return;
    }
    let side = if compact {
        ui.available_width()
    } else if editing {
        150.0
    } else {
        190.0
    };
    let photo = |ui: &mut egui::Ui, env: &mut Env, w: &mut Window| {
        let size = if compact { Vec2::new(side, 150.0) } else { Vec2::splat(side) };
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let radius = ui.visuals().window_corner_radius;
        match env.images.get(&image).filter(|_| !image.is_nil()) {
            Some(t) => cover(ui, t, rect, radius),
            None => {
                dashed(ui, rect, env.p.ink.gamma_multiply(0.16));
                paint_icon(ui, "camera-filled", rect.center() - Vec2::new(0.0, 14.0), 22.0, env.p.muted_dim);
                ui.painter().text(
                    rect.center() + Vec2::new(0.0, 12.0),
                    egui::Align2::CENTER_CENTER,
                    aurora_i18n::tr!("ui-profile-aurora-add-photo"),
                    FontId::proportional(12.0),
                    env.p.muted_dim,
                );
            }
        }
        if editing {
            photo_menu(ui, env, rect, image);
        } else if !image.is_nil() && response.on_hover_text(aurora_i18n::tr!("ui-profile-aurora-big-photo")).clicked() {
            w.open_photo(PictureSlot::FirstLife, image);
        }
    };
    let text = |ui: &mut egui::Ui, env: &mut Env, w: &mut Window| {
        if editing {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    let options = [
                        (aurora_i18n::tr!("ui-profile-aurora-write"), true),
                        (aurora_i18n::tr!("ui-profile-aurora-preview"), true),
                    ];
                    let mut selected = usize::from(w.fl_preview);
                    if widgets::segmented(ui, p, &mut selected, &options) {
                        w.fl_preview = selected == 1;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let count = aurora_i18n::tr!(
                            "ui-profile-aurora-char-count",
                            count = aurora_i18n::integer(w.fl_about.text.chars().count() as i64),
                            max = aurora_i18n::integer(FIRST_LIFE_MAX as i64)
                        );
                        ui.label(RichText::new(count).size(11.0).color(p.muted_dim));
                    });
                });
                if w.fl_preview {
                    rich(ui, p, env.emoji, env.world, env.want_names, &w.fl_about.text);
                } else {
                    ui.add(
                        egui::TextEdit::multiline(&mut w.fl_about.text)
                            .desired_width(ui.available_width())
                            .desired_rows(5)
                            .char_limit(FIRST_LIFE_MAX)
                            .font(FontId::proportional(12.5)),
                    );
                }
            });
        } else if !w.fl_about.text.trim().is_empty() {
            let fl = w.fl_about.text.clone();
            card(
                ui,
                p,
                Some(("user-heart", aurora_i18n::tr!("ui-profile-aurora-tab-first-life"))),
                None,
                false,
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    rich(ui, p, env.emoji, env.world, env.want_names, &fl);
                },
            );
        }
    };
    if compact {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if editing || !image.is_nil() {
                photo(ui, env, w);
            }
            text(ui, env, w);
        });
    } else {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if editing || !image.is_nil() {
                photo(ui, env, w);
            }
            ui.vertical(|ui| text(ui, env, w));
        });
    }
}

/// The camera button over the first life picture and its menu (the
/// classic window's Load / Change / Remove).
fn photo_menu(ui: &mut egui::Ui, env: &mut Env, rect: Rect, image: Uuid) {
    let p = env.p;
    let c = rect.right_bottom() - Vec2::splat(22.0);
    let r = Rect::from_center_size(c, Vec2::splat(30.0));
    let resp = ui.interact(r, ui.scope_id().with("first-life-camera"), Sense::click());
    ui.painter()
        .circle_filled(c, 15.0, if resp.hovered() { p.violet_light } else { p.violet });
    paint_icon(ui, "camera-filled", c, 15.0, p.on_accent);
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-profile-change-image"));
    let mut choose = false;
    let mut remove = false;
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        menu::item_tip(
            ui,
            p,
            "file-upload-filled",
            aurora_i18n::tr!("ui-profile-aurora-upload-image"),
            false,
            aurora_i18n::tr!("ui-profile-coming-soon"),
        );
        choose = menu::item(ui, p, "photo-filled", aurora_i18n::tr!("ui-profile-aurora-choose-from-inventory"));
        remove = menu::item_if(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("ui-profile-aurora-remove-photo"),
            !image.is_nil(),
        );
    });
    if choose {
        *env.pick_picture = Some((PictureSlot::FirstLife, image));
    }
    if remove {
        env.actions.push(ProfileAction::Save {
            target: env.world.agent_id,
            data: llsd_map! { "fl_image_id" => Uuid::nil() },
        });
    }
}

// ------------------------------------------------------------------ web

/// The web profile (LLPanelProfileWeb: [WEB_PROFILE_URL]<username>/?feed_only=true)
/// in a browser card: the address, Reload, Open in the browser, then the
/// load time.
pub(in super::super) fn web(ui: &mut egui::Ui, env: &mut Env, w: &mut Window) {
    let (p, world) = (env.p, env.world);
    let Some(base) = env.web.profile_base else {
        empty(
            ui,
            p,
            "world-filled",
            aurora_i18n::tr!("ui-profile-aurora-no-web"),
            aurora_i18n::tr!("ui-profile-aurora-no-web-text"),
            None,
            None,
        );
        return;
    };
    let Some(username) = web_username(world, &w.id) else {
        loading(ui, p);
        return;
    };
    let page = format!("{base}{username}").to_lowercase();
    w.web.navigate(&format!("{page}/?feed_only=true"));
    let shown = page.split("://").nth(1).unwrap_or(&page).to_owned();
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: 10,
            right: 6,
            top: 5,
            bottom: 5,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(Vec2::new(13.0, 20.0), Sense::hover());
                paint_icon(ui, "lock-filled", r.center(), 12.0, p.muted);
                ui.add(egui::Label::new(RichText::new(&shown).size(12.0).color(p.muted)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "external-link-filled",
                        aurora_i18n::tr!("ui-profile-aurora-open-in-browser"),
                        ActionKind::Flat,
                        Vec2::new(0.0, 24.0),
                    )
                    .clicked()
                    {
                        context::request(ui.ctx(), CtxAction::OpenUrl(page.clone()));
                    }
                    if widgets::action_button(ui, p, "refresh", "", ActionKind::Flat, Vec2::new(28.0, 24.0))
                        .on_hover_text(aurora_i18n::tr!("ui-profile-aurora-reload"))
                        .clicked()
                    {
                        w.web.reload();
                    }
                });
            });
        });
    ui.add_space(6.0);
    let size = Vec2::new(ui.available_width(), (ui.available_height() - 24.0).max(100.0));
    w.web.show(ui, p, env.web.paths, env.web.browser, env.web.cookie, size);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let text = match w.web.load_secs {
            Some(s) => aurora_i18n::tr!("ui-profile-aurora-loaded-in", s = aurora_i18n::number_text(&format!("{s:.2}"))),
            None => aurora_i18n::tr!("common-loading-2").to_owned(),
        };
        ui.label(RichText::new(text).size(11.5).color(p.muted_dim));
    });
}

// ------------------------------------------------------------------ notes

/// Our private notes on someone (LLPanelProfileNotes), in the turquoise
/// card: Cancel and Save once something is written, then "Saved".
pub(in super::super) fn notes(ui: &mut egui::Ui, env: &mut Env, w: &mut Window) {
    let (p, world) = (env.p, env.world);
    if world.profiles.get(&w.id).is_none() {
        loading(ui, p);
        return;
    }
    let name = display_name(world, &w.id);
    let dirty = w.notes.dirty();
    let saved = w.notes_saved && !dirty;
    let mut save = false;
    let mut cancel = false;
    card(
        ui,
        p,
        Some(("lock-filled", aurora_i18n::tr!("ui-profile-aurora-notes-private"))),
        None,
        true,
        |ui| {
            // at the right of the head: "Saved", or the count of characters
            let head = ui.min_rect();
            let (text, color) = if saved {
                (aurora_i18n::tr!("ui-profile-aurora-notes-saved").to_owned(), p.success)
            } else {
                (
                    aurora_i18n::tr!(
                        "ui-profile-aurora-char-count",
                        count = aurora_i18n::integer(w.notes.text.chars().count() as i64),
                        max = aurora_i18n::integer(NOTES_MAX as i64)
                    ),
                    p.muted_dim,
                )
            };
            let g = ui.painter().layout_no_wrap(text, semibold(ui, 11.0), color);
            let at = pos2(ui.max_rect().right() - g.size().x, head.top() + 1.0);
            if saved {
                paint_icon(ui, "check", at + Vec2::new(-9.0, 7.0), 12.0, color);
            }
            ui.painter().galley(at, g, color);
            let edit = ui.add(
                egui::TextEdit::multiline(&mut w.notes.text)
                    .desired_width(ui.available_width())
                    .desired_rows(5)
                    .char_limit(NOTES_MAX)
                    .hint_text(aurora_i18n::tr!("ui-profile-aurora-notes-hint", name = name.clone()))
                    .font(FontId::proportional(12.5)),
            );
            if edit.changed() {
                w.notes_saved = false;
            }
            if w.notes.dirty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let size = Vec2::new(0.0, 28.0);
                    save = widgets::action_button(ui, p, "", aurora_i18n::tr!("common-save"), ActionKind::Primary, size).clicked();
                    cancel = widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked();
                });
            } else if w.notes.text.trim().is_empty() {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-aurora-notes-kept"))
                        .size(11.5)
                        .color(p.muted_dim),
                );
            }
        },
    );
    if save {
        w.notes.base = w.notes.text.clone();
        w.notes_saved = true;
        env.actions.push(ProfileAction::Save {
            target: w.id,
            data: llsd_map! { "notes" => w.notes.text.clone() },
        });
    }
    if cancel {
        w.notes.text = w.notes.base.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_classified_name_like_firestorm() {
        assert!(classified_name_error("").is_some());
        assert!(classified_name_error("« Atelier »").is_some(), "punctuation first");
        assert!(
            classified_name_error("Écoles").is_some(),
            "isalnum of the first byte: A to Z or a digit"
        );
        assert!(classified_name_error("Atelier ouvert").is_none());
        assert!(classified_name_error("3 chalets").is_none());
        // 30 bytes at most, never through a character
        let mut s = "Cours de construction du samedi".to_owned();
        clip_bytes(&mut s, 30);
        assert_eq!(s, "Cours de construction du samed");
        let mut s = "ééééééééééééééééé".to_owned();
        clip_bytes(&mut s, 30);
        assert_eq!(s.len(), 30);
        let mut s = "éééééééééééééééé!".to_owned();
        clip_bytes(&mut s, 31);
        assert_eq!(s.chars().count(), 15);
    }

    #[test]
    fn the_price_of_a_new_classified() {
        assert_eq!(publish_price("50"), Ok(50));
        assert_eq!(publish_price(" 120 "), Ok(120));
        assert!(publish_price("49").is_err());
        assert!(publish_price("cinquante").is_err());
    }

    #[test]
    fn a_place_line_like_the_mockup() {
        let pos = DVec3::new(256_000.0 + 140.2, 256_000.0 + 119.6, 25.4);
        assert_eq!(
            place_line("Place d'Aurora", "Aurora Démo", pos),
            "Place d'Aurora · Aurora Démo (140, 120, 25)"
        );
        assert_eq!(place_line("", "Sablière", pos), "Sablière (140, 120, 25)");
        assert_eq!(place_line("", "Sablière", DVec3::ZERO), "Sablière");
    }

    #[test]
    fn a_classified_edit_keeps_its_flags() {
        let c = ClassifiedInfo {
            id: Uuid::from_u128(5),
            flags: classified_flags::AUTO_RENEW | classified_flags::QUERY_INC_MATURE,
            price: 75,
            ..Default::default()
        };
        let e = edit_of(&c);
        assert!(e.mature && e.auto_renew);
        let back = classified_of(&e);
        assert_eq!(back.id, c.id);
        assert_eq!(back.flags, c.flags);
        assert_eq!(back.price, 75, "an edit sends back what it cost");
        // a new one gets its id when sent
        let fresh = ClassifiedEdit::default();
        assert!(!classified_of(&fresh).id.is_nil());
    }
}

#[cfg(test)]
mod contain_tests {
    use super::*;

    #[test]
    fn contain_keeps_the_whole_picture() {
        let rect = Rect::from_min_size(pos2(10.0, 10.0), Vec2::new(200.0, 100.0));
        let tall = contain_rect(rect, [100, 200]);
        assert_eq!(tall.size(), Vec2::new(50.0, 100.0));
        assert_eq!(tall.center(), rect.center());
        let wide = contain_rect(rect, [400, 100]);
        assert_eq!(wide.size(), Vec2::new(200.0, 50.0));
    }
}
