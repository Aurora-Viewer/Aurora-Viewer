//! The details pane of Aurora's search window: the selected row's picture
//! or date first, its name and its rating, its figures, labelled fields and
//! text, the actions in a bar at the bottom. What it shows and does is what
//! the Legacy panes of `details.rs` show and do (the same replies:
//! AvatarProperties, GroupProfileReply, ParcelInfoReply, EventInfoReply,
//! ClassifiedInfoReply), laid out the Aurora way.

use super::SearchAction;
use super::aurora::{linden, pill, sale_pill, started_pill};
use super::details::{Btn, agent_link, field, muted, pane, rating, text_box, text_field};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::place_details::{self, Ctx};
use crate::ui::{colors, menu, place_card, widgets};
use crate::world::place_details::{Place, Source, Stage};
use crate::world::profiles::{self, Online};
use crate::world::search::{self, Key, Row, RowIcon, Tab, Value};
use aurora_net::profile::classified_flags;
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Vec2};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A picture's height for the pane's width (196 for 372 in the mockup).
const MEDIA_RATIO: f32 = 0.53;

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    c: &mut Ctx,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    wanted_pics: &mut HashSet<Uuid>,
    tab: Tab,
    key: Key,
    row: Option<&Row>,
    actions: &mut Vec<SearchAction>,
) {
    let name = row.map_or("", |r| r.name.as_str());
    match (tab, key) {
        (Tab::People, Key::Id(id)) => person(ui, c, pics, wanted_pics, id, name),
        (Tab::Groups, Key::Id(id)) => group(ui, c, id, name, actions),
        (Tab::Places | Tab::Land, Key::Id(id)) => place(ui, c, id, tab, row, actions),
        (Tab::Classifieds, Key::Id(id)) => classified(ui, c, id, name, actions),
        (Tab::Events, Key::Event(id)) => event(ui, c, id, name, row, actions),
        _ => {}
    }
}

/// A snapshot as wide as the pane, a pill over its top right corner.
fn media(ui: &mut egui::Ui, c: &mut Ctx, snapshot: Uuid, pill_over: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, (width * MEDIA_RATIO).round()), Sense::hover());
    let tex = (!snapshot.is_nil())
        .then(|| {
            c.wanted_images.insert(snapshot);
            c.images.get(&snapshot)
        })
        .flatten();
    place_card::picture(ui, c.p, tex, rect, 36.0);
    let corner = Rect::from_min_max(rect.min + Vec2::new(8.0, 8.0), rect.max - Vec2::new(8.0, 8.0));
    let mut over = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(corner)
            .layout(egui::Layout::right_to_left(egui::Align::Min)),
    );
    pill_over(&mut over);
}

/// The name in large type, then the line under it: a status dot, the
/// rating's badge and a muted text.
fn head(ui: &mut egui::Ui, p: &Palette, title: &str, dot: Option<Color32>, rating: Option<&str>, sub: &str) {
    ui.add(egui::Label::new(RichText::new(title).size(15.0).strong().color(p.ink)).wrap());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(color) = dot {
            widgets::status_dot(ui, color);
        }
        if let Some(r) = rating {
            widgets::maturity_badge(ui, p, r, 16.0);
        }
        ui.add(egui::Label::new(RichText::new(sub).size(12.0).color(p.muted)).truncate());
    });
}

/// Figures with their icons on one muted line.
fn stats(ui: &mut egui::Ui, p: &Palette, items: &[(&str, String)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (icon, text)) in items.iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            widgets::glyph(ui, icon, p.muted, 13.0);
            ui.label(RichText::new(text).size(12.0).color(p.muted));
        }
    });
}

/// The small capitals over a text box ("ABOUT", "DESCRIPTION").
fn caps(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .color(p.muted)
            .family(crate::ui::fonts::semibold(ui.ctx())),
    );
}

/// A key shortened to its ends, whole on hover, with its copy button.
fn key_line(ui: &mut egui::Ui, p: &Palette, id: Uuid) {
    let full = id.to_string();
    let short = format!("{}…{}", &full[..8], &full[full.len() - 8..]);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(short).size(11.0).monospace().color(p.muted))
            .on_hover_text(&full);
        if widgets::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("search-copy-key"), true) {
            ui.ctx().copy_text(full.clone());
        }
    });
}

/// The rating's badge and name as a field.
fn content_field(ui: &mut egui::Ui, p: &Palette, name: &str) {
    field(ui, p, aurora_i18n::tr!("search-content"), |ui| rating(ui, p, name));
}

fn person(
    ui: &mut egui::Ui,
    c: &mut Ctx,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    wanted_pics: &mut HashSet<Uuid>,
    id: Uuid,
    row_name: &str,
) {
    let (p, world) = (c.p, c.world);
    let own = id == world.agent_id;
    let profile = world.profiles.get(&id);
    let friend = world.social.friends.iter().find(|f| f.id == id);
    if profile.is_some_and(|pr| !pr.sl_image.is_nil()) {
        wanted_pics.insert(id);
    }
    c.want_names.insert(id);
    let name = world.social.avatar_names.complete(&id).unwrap_or_else(|| row_name.to_owned());
    let gear = |ui: &mut egui::Ui| context::avatar_list_menu(ui, p, world, id, context::AvatarList::Name);
    let buttons = [
        Btn::new("user-filled", aurora_i18n::tr!("search-aurora-profile")).primary(),
        Btn::new("message-circle-filled", aurora_i18n::tr!("search-aurora-message")).enabled(!own),
        Btn::new("send-filled", aurora_i18n::tr!("search-aurora-offer-teleport")).enabled(!own),
        Btn::new("user-plus", aurora_i18n::tr!("ui-profile-add-friend")).enabled(friend.is_none() && !own),
        Btn::new("dots-filled", aurora_i18n::tr!("ui-profile-more")).menu(&gear),
    ];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let clicked = pane(ui, p, ("aurora-search-person", Key::Id(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        let status = profiles::online_status(profile, own, friend.map(|f| (f.online, f.rights_has & 1 != 0)));
        let (text, color) = match status {
            Online::Yes => (aurora_i18n::tr!("ui-profile-connected"), p.success),
            Online::No => (aurora_i18n::tr!("ui-profile-disconnected"), p.muted_dim),
            Online::Unknown => (aurora_i18n::tr!("common-unknown"), p.warn),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            widgets::picture(ui, p, pics.get(&id), Vec2::splat(72.0), "user-filled", "");
            ui.vertical(|ui| {
                ui.add_space(14.0);
                head(ui, p, &name, Some(color), None, text);
            });
        });
        let Some(pr) = profile else {
            muted(ui, p, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        ui.add_space(4.0);
        field(ui, p, aurora_i18n::tr!("search-aurora-key"), |ui| key_line(ui, p, id));
        if let Some(birth) = profiles::birth_text(pr, own, now) {
            text_field(ui, p, aurora_i18n::tr!("search-aurora-born"), &birth.replace('\n', " "));
        }
        text_field(ui, p, aurora_i18n::tr!("ui-profile-account"), &profiles::account_text(pr));
        field(ui, p, aurora_i18n::tr!("ui-profile-partner"), |ui| agent_link(ui, c, pr.partner));
        // the legacy reply (no AgentProfile capability) has no groups
        if !pr.legacy {
            let mut groups: Vec<&str> = pr.groups.iter().map(|g| g.name.as_str()).collect();
            groups.sort_by_key(|g| g.to_lowercase());
            let text = if groups.is_empty() {
                aurora_i18n::tr!("common-none").to_owned()
            } else {
                groups.join(", ")
            };
            text_field(ui, p, aurora_i18n::tr!("ui-profile-groups"), &text);
        }
        caps(ui, p, aurora_i18n::tr!("search-aurora-about"));
        text_box(ui, c, &pr.sl_about, 60.0);
    });
    let act = |a: CtxAction| context::request(ui.ctx(), a);
    match clicked {
        Some(0) => act(CtxAction::Profile(id)),
        Some(1) => act(CtxAction::Im(id)),
        Some(2) => act(CtxAction::OfferTeleport(id)),
        Some(3) => act(CtxAction::AddFriend(id)),
        _ => {}
    }
}

fn group(ui: &mut egui::Ui, c: &mut Ctx, id: Uuid, row_name: &str, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
    let data = world.search.groups.get(&id);
    let member = world.groups.is_member(&id);
    let name = data.map_or(row_name, |g| g.name.as_str()).to_owned();
    let copy_name = name.clone();
    let refresh = std::cell::Cell::new(false);
    let more = |ui: &mut egui::Ui| {
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("search-group-copy-uri")) {
            ui.ctx().copy_text(format!("secondlife:///app/group/{id}/about"));
        }
        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("search-group-copy-name")) {
            ui.ctx().copy_text(copy_name.clone());
        }
        if menu::item(ui, p, "refresh", aurora_i18n::tr!("search-refresh")) {
            refresh.set(true);
        }
    };
    let second = if member {
        Btn::new("messages-filled", aurora_i18n::tr!("search-group-chat"))
    } else {
        // LLGroupActions::join: fee confirmation and JoinGroupRequest
        Btn::new("users-plus", aurora_i18n::tr!("search-group-join")).enabled(data.is_some_and(|g| g.open_enrollment))
    };
    let buttons = [
        Btn::new("users-group", aurora_i18n::tr!("search-aurora-group-profile")).primary(),
        second,
        Btn::new("dots-filled", aurora_i18n::tr!("ui-profile-more")).menu(&more),
    ];
    let clicked = pane(ui, p, ("aurora-search-group", Key::Id(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        let content = data.map(|g| {
            if g.mature_publish {
                aurora_i18n::tr!("common-moderate")
            } else {
                aurora_i18n::tr!("common-general")
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let insignia = data.map(|g| g.insignia).filter(|i| !i.is_nil());
            if let Some(i) = insignia {
                c.wanted_images.insert(i);
            }
            let tex = insignia.and_then(|i| c.images.get(&i));
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
            match tex {
                Some(t) => widgets::square_picture(ui, rect, t),
                None => {
                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
                    let font = FontId::new(22.0, crate::ui::fonts::semibold(ui.ctx()));
                    let g = ui.painter().layout_no_wrap(widgets::initials(&name), font, p.violet_pale);
                    widgets::paint_ink_centered(ui.painter(), g, rect.center(), p.violet_pale);
                }
            }
            ui.vertical(|ui| {
                ui.add_space(14.0);
                let members = world
                    .group_member_count(id)
                    .map_or_else(String::new, |n| aurora_i18n::tr!("search-group-members", count = n));
                head(ui, p, &name, None, content, &members);
                if member {
                    pill(ui, aurora_i18n::tr!("search-aurora-member"), p.violet_pale, p.raised);
                }
            });
        });
        let Some(g) = data else {
            muted(ui, p, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        ui.add_space(4.0);
        field(ui, p, aurora_i18n::tr!("search-group-founder"), |ui| agent_link(ui, c, g.founder));
        let enrollment = if g.open_enrollment {
            aurora_i18n::tr!("search-group-open")
        } else {
            aurora_i18n::tr!("search-group-invite-only")
        };
        text_field(ui, p, aurora_i18n::tr!("search-group-enrollment"), enrollment);
        let fee = if g.membership_fee > 0 {
            format!("L${}", g.membership_fee)
        } else {
            aurora_i18n::tr!("search-none-fee").to_owned()
        };
        text_field(ui, p, aurora_i18n::tr!("search-group-fee"), &fee);
        if let Some(content) = content {
            content_field(ui, p, content);
        }
        if member && !g.member_title.is_empty() {
            text_field(ui, p, aurora_i18n::tr!("search-group-my-title"), &g.member_title);
        }
        field(ui, p, aurora_i18n::tr!("search-aurora-key"), |ui| key_line(ui, p, id));
        caps(ui, p, aurora_i18n::tr!("search-aurora-charter"));
        text_box(ui, c, &g.charter, 70.0);
    });
    if refresh.get() {
        actions.push(SearchAction::RefreshGroup(id));
    }
    match clicked {
        Some(0) => crate::ui::group_profile::request_open(ui.ctx(), id),
        Some(1) if member => context::request(ui.ctx(), CtxAction::GroupChat(id)),
        Some(1) => actions.push(SearchAction::JoinGroup(id)),
        _ => {}
    }
}

/// A parcel of the places and land sales: the place profile's snapshot,
/// region, figures and description; a land row adds its sale's figures.
fn place(ui: &mut egui::Ui, c: &mut Ctx, parcel: Uuid, tab: Tab, row: Option<&Row>, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
    let land = row.filter(|_| tab == Tab::Land);
    let data = world.profiles.parcels.get(&parcel);
    // mParcelGlobal: where the buttons go, once the parcel answered
    let global = data.map(|d| d.global).filter(|g| *g != DVec3::ZERO);
    let place = Place {
        serial: 0,
        source: Source::History {
            title: String::new(),
            global: global.unwrap_or_default(),
        },
        global,
        region_pos: global.map_or(glam::Vec3::ZERO, |g| {
            glam::Vec3::new(g.x.rem_euclid(256.0) as f32, g.y.rem_euclid(256.0) as f32, g.z as f32)
        }),
        stage: Stage::Info(parcel),
    };
    let more = |ui: &mut egui::Ui| place_details::overflow_menu(ui, p, world, &place);
    let buttons = [
        Btn::new("location-filled", aurora_i18n::tr!("common-teleport-3"))
            .primary()
            .enabled(global.is_some()),
        Btn::new("map", aurora_i18n::tr!("common-map")).enabled(global.is_some()),
        Btn::new("dots-filled", aurora_i18n::tr!("ui-profile-more")).menu(&more),
    ];
    // The sale comes from the search reply's ForSale / Auction (the row's
    // icon), never from ParcelInfoReply: its SalePrice is the parcel's last
    // asking price, kept when the sale ends (LLParcel starts at L$10000), so
    // LLPanelPlaceInfo::processParcelInfo never shows it.
    let auction = row.is_some_and(|r| r.icon == RowIcon::Auction);
    let for_sale = land.is_some() || auction || row.is_some_and(|r| r.icon == RowIcon::ForSale);
    let clicked = pane(ui, p, ("aurora-search-place", Key::Id(parcel)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        media(ui, c, data.map_or(Uuid::nil(), |d| d.snapshot), |ui| {
            if for_sale {
                sale_pill(ui, p, auction);
            }
        });
        ui.add_space(4.0);
        let Some(d) = data else {
            let name = land.map_or(aurora_i18n::tr!("common-loading-2"), |r| r.name.as_str());
            head(ui, p, name, None, None, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        let region = place_details::region_name_pos(&d.sim_name, place.region_pos, d.global);
        let maturity = place_details::maturity(d.flags);
        head(ui, p, &d.name, None, Some(maturity), &region);
        stats(
            ui,
            p,
            &[
                ("walk", aurora_i18n::integer(d.dwell as i64)),
                (
                    "dimensions",
                    aurora_i18n::tr!("search-aurora-area", area = aurora_i18n::integer(d.actual_area as i64)),
                ),
            ],
        );
        ui.add_space(4.0);
        if let Some(row) = land {
            let price = match row.values.get(1) {
                Some(Value::Int(n)) if !auction => linden(*n),
                _ => aurora_i18n::tr!("search-aurora-auction").to_owned(),
            };
            text_field(ui, p, aurora_i18n::tr!("search-aurora-price"), &price);
            if let Some(Value::Real(per_meter)) = row.values.get(3).filter(|_| !auction) {
                text_field(
                    ui,
                    p,
                    aurora_i18n::tr!("search-aurora-price-per-meter"),
                    &aurora_i18n::number(*per_meter, 2),
                );
            }
            if let Some(Value::Text(kind)) = row.values.first() {
                text_field(ui, p, aurora_i18n::tr!("common-type-2"), kind);
            }
        } else if for_sale && !auction && d.sale_price > 0 {
            text_field(ui, p, aurora_i18n::tr!("search-aurora-price"), &linden(d.sale_price as i64));
        }
        content_field(ui, p, maturity);
        caps(ui, p, aurora_i18n::tr!("search-aurora-description"));
        text_box(ui, c, &d.desc, 60.0);
    });
    match (clicked, global) {
        (Some(0), Some(g)) => actions.push(SearchAction::Teleport(g)),
        (Some(1), Some(g)) => actions.push(SearchAction::ShowOnMap(g)),
        _ => {}
    }
}

fn classified(ui: &mut egui::Ui, c: &mut Ctx, id: Uuid, row_name: &str, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
    let info = world.profiles.classifieds.get(&id);
    let global = info.map(|i| i.pos_global).filter(|g| *g != DVec3::ZERO);
    let buttons = [
        Btn::new("location-filled", aurora_i18n::tr!("common-teleport-3"))
            .primary()
            .enabled(global.is_some()),
        Btn::new("map", aurora_i18n::tr!("common-map")).enabled(global.is_some()),
    ];
    let clicked = pane(ui, p, ("aurora-search-classified", Key::Id(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        media(ui, c, info.map_or(Uuid::nil(), |i| i.snapshot), |_| {});
        ui.add_space(4.0);
        let Some(info) = info else {
            head(ui, p, row_name, None, None, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        // is_cf_mature: the mature flag of the ad or of its query
        let mature = info.flags & (classified_flags::MATURE | classified_flags::QUERY_INC_MATURE) != 0;
        let content = if mature {
            aurora_i18n::tr!("common-moderate")
        } else {
            aurora_i18n::tr!("common-general")
        };
        let category = world.classified_category(info.category);
        head(ui, p, &info.name, None, Some(content), &category);
        ui.add_space(4.0);
        text_field(
            ui,
            p,
            aurora_i18n::tr!("ui-profile-location-2"),
            &aurora_net::profile::location_text(&[&info.parcel_name, &info.sim_name], info.pos_global),
        );
        field(ui, p, aurora_i18n::tr!("search-classified-creator"), |ui| {
            agent_link(ui, c, info.creator)
        });
        text_field(
            ui,
            p,
            aurora_i18n::tr!("search-aurora-published-on"),
            &profiles::slt_date(info.creation_date),
        );
        text_field(ui, p, aurora_i18n::tr!("ui-profile-listing-cost"), &linden(info.price as i64));
        content_field(ui, p, content);
        caps(ui, p, aurora_i18n::tr!("search-aurora-description"));
        text_box(ui, c, &info.desc, 60.0);
    });
    match (clicked, global) {
        (Some(0), Some(g)) => actions.push(SearchAction::Teleport(g)),
        (Some(1), Some(g)) => actions.push(SearchAction::ShowOnMap(g)),
        _ => {}
    }
}

fn event(ui: &mut egui::Ui, c: &mut Ctx, id: u32, row_name: &str, row: Option<&Row>, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
    let info = world.search.events.get(&id);
    let global = info.map(|e| e.global).filter(|g| *g != DVec3::ZERO);
    let buttons = [
        Btn::new("location-filled", aurora_i18n::tr!("common-teleport-3"))
            .primary()
            .enabled(global.is_some()),
        Btn::new("map", aurora_i18n::tr!("common-map")).enabled(global.is_some()),
        // onBtnEventReminder: LLEventNotifier is not ported
        Btn::new("bell-filled", aurora_i18n::tr!("search-aurora-remind")).not_yet(),
    ];
    let start = info
        .map(|e| e.date_utc as i64)
        .filter(|t| *t > 0)
        .or_else(|| match row.and_then(|r| r.values.first()) {
            Some(Value::Dated { unix, .. }) => Some(*unix as i64),
            _ => None,
        });
    let clicked = pane(ui, p, ("aurora-search-event", Key::Event(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        if let Some(t) = start {
            date_banner(ui, p, t, info.map(|e| search::event_duration(e.duration)));
        }
        ui.add_space(4.0);
        let Some(e) = info else {
            head(ui, p, row_name, None, None, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        let content = match search::event_rating(e.flags) {
            aurora_net::search::SIM_ACCESS_ADULT => aurora_i18n::tr!("common-adult"),
            aurora_net::search::SIM_ACCESS_MATURE => aurora_i18n::tr!("common-moderate"),
            _ => aurora_i18n::tr!("common-general"),
        };
        head(ui, p, &e.name, None, Some(content), &e.category);
        ui.add_space(4.0);
        text_field(ui, p, aurora_i18n::tr!("search-event-date"), &e.date);
        text_field(
            ui,
            p,
            aurora_i18n::tr!("search-event-duration"),
            &search::event_duration(e.duration),
        );
        field(ui, p, aurora_i18n::tr!("search-event-organizer"), |ui| agent_link(ui, c, e.creator));
        // "%s (%d, %d, %d)": the region and the position in it
        let pos = glam::Vec3::new(
            (e.global.x.round() as i64).rem_euclid(256) as f32,
            (e.global.y.round() as i64).rem_euclid(256) as f32,
            e.global.z.round() as f32,
        );
        field(ui, p, aurora_i18n::tr!("ui-profile-location-2"), |ui| {
            let text = format!("{} ({}, {}, {})", e.sim_name, pos.x, pos.y, pos.z);
            let link = ui.add(egui::Link::new(
                RichText::new(text).size(12.0).color(colors::text(colors::get().chat_slurl)),
            ));
            if link.on_hover_text(aurora_i18n::tr!("search-event-place-info")).clicked() {
                context::request(ui.ctx(), CtxAction::ShowPlaceInfo(e.sim_name.clone(), pos));
            }
        });
        let fee = match e.cover {
            Some(amount) => format!("L${amount}"),
            None => aurora_i18n::tr!("search-none-fee").to_owned(),
        };
        text_field(ui, p, aurora_i18n::tr!("search-event-fee"), &fee);
        content_field(ui, p, content);
        caps(ui, p, aurora_i18n::tr!("search-aurora-description"));
        text_box(ui, c, &e.desc, 60.0);
    });
    match (clicked, global) {
        (Some(0), Some(g)) => actions.push(SearchAction::Teleport(g)),
        (Some(1), Some(g)) => actions.push(SearchAction::ShowOnMap(g)),
        _ => {}
    }
}

/// The event's day in large type, its time and duration beside it, and
/// "Started" once its start is past.
fn date_banner(ui: &mut egui::Ui, p: &Palette, start: i64, duration: Option<String>) {
    let (_, month, day, _, hour, minute, _) = crate::ui::land::slt_parts(start);
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(width - 24.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::new(48.0, 44.0), Sense::hover());
                let font = crate::ui::fonts::semibold(ui.ctx());
                let mon = crate::ui::land::months()[(month.max(1) - 1).min(11)]
                    .trim_end_matches('.')
                    .to_uppercase();
                let m = ui.painter().layout_no_wrap(mon, FontId::new(11.0, font.clone()), p.violet_light);
                widgets::paint_ink_centered(ui.painter(), m, Pos2::new(rect.center().x, rect.top() + 7.0), p.violet_light);
                let d = ui.painter().layout_no_wrap(day.to_string(), FontId::new(24.0, font), p.ink);
                widgets::paint_ink_centered(ui.painter(), d, Pos2::new(rect.center().x, rect.bottom() - 13.0), p.ink);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(3.0);
                    ui.label(RichText::new(format!("{hour:02}:{minute:02}")).size(15.0).strong().color(p.ink));
                    let mut sub = aurora_i18n::tr!("search-aurora-slt").to_owned();
                    if let Some(d) = duration {
                        sub = format!("{sub} · {d}");
                    }
                    ui.label(RichText::new(sub).size(12.0).color(p.muted));
                });
                if start <= super::unix_now() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| started_pill(ui, p));
                }
            });
        });
}
