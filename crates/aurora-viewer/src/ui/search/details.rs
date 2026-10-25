//! Details of the row selected in the search window, with their actions:
//! what FSFloaterSearch::displayAvatarDetails / displayGroupDetails /
//! displayParcelDetails / displayClassifiedDetails / displayEventDetails
//! show, completed with the fields of the profile panels Firestorm embeds
//! there today (key, online status, account, groups for a resident; founder
//! and enrollment for a group; category, date and fee for an event), and the
//! buttons of onBtnPeople* / onBtnGroup* / onBtnTeleport / onBtnMap /
//! onBtnEventReminder (indra/newview/fsfloatersearch.cpp, originally
//! LGPL 2.1). The place profile is the one of "Places" (`place_details.rs`).

use super::SearchAction;
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::place_details::{self, Ctx};
use crate::ui::widgets::{self, ActionKind};
use crate::ui::{colors, menu};
use crate::world::place_details::{Place, Source, Stage};
use crate::world::profiles::{self, Online};
use crate::world::search::{self, Key, List, Row, RowIcon, Tab};
use aurora_net::profile::classified_flags;
use egui::{RichText, Vec2};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const BUTTON_H: f32 = 26.0;
const BUTTON_GAP: f32 = 4.0;
/// The column of the labels before their values.
const LABEL_W: f32 = 118.0;

/// A button under the details.
pub(super) struct Btn<'a> {
    icon: &'a str,
    label: &'a str,
    kind: ActionKind,
    enabled: bool,
    /// Why it is greyed out, or what it does.
    tip: Option<&'a str>,
    /// The button opens this menu instead of acting.
    menu: Option<&'a dyn Fn(&mut egui::Ui)>,
}

impl<'a> Btn<'a> {
    pub(super) fn new(icon: &'a str, label: &'a str) -> Btn<'a> {
        Btn {
            icon,
            label,
            kind: ActionKind::Flat,
            enabled: true,
            tip: None,
            menu: None,
        }
    }

    pub(super) fn primary(mut self) -> Self {
        self.kind = ActionKind::Primary;
        self
    }

    pub(super) fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// A Firestorm action Aurora does not do yet: greyed, with the tooltip
    /// of the menus.
    pub(super) fn not_yet(mut self) -> Self {
        self.enabled = false;
        self.tip = Some(menu::not_yet());
        self
    }

    pub(super) fn menu(mut self, body: &'a dyn Fn(&mut egui::Ui)) -> Self {
        self.menu = Some(body);
        self
    }
}

/// Split the buttons in rows that fit `width` (their natural widths, in
/// order).
fn button_rows(ui: &egui::Ui, buttons: &[Btn], width: f32) -> Vec<Vec<usize>> {
    let font = egui::FontId::proportional(12.5);
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (i, b) in buttons.iter().enumerate() {
        let text = ui
            .painter()
            .layout_no_wrap(b.label.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x;
        let w = text + 22.0 + 24.0;
        match rows.last_mut() {
            Some(row) if used + BUTTON_GAP + w <= width => {
                row.push(i);
                used += BUTTON_GAP + w;
            }
            _ => {
                rows.push(vec![i]);
                used = w;
            }
        }
    }
    rows
}

/// The details in a scrolling area over the buttons, which stay in view.
/// Returns the index of the button clicked.
pub(super) fn pane(ui: &mut egui::Ui, p: &Palette, salt: (&str, Key), buttons: &[Btn], body: impl FnOnce(&mut egui::Ui)) -> Option<usize> {
    let rows = button_rows(ui, buttons, ui.available_width());
    let buttons_h = rows.len() as f32 * BUTTON_H + rows.len().saturating_sub(1) as f32 * BUTTON_GAP;
    // the layout also puts its spacing around the gap before the buttons
    let gaps = 8.0 + 2.0 * ui.spacing().item_spacing.y;
    let body_h = (ui.available_height() - buttons_h - gaps).max(60.0);
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .max_height(body_h)
        .show(ui, body);
    ui.add_space(8.0);
    let mut clicked = None;
    ui.spacing_mut().item_spacing = Vec2::splat(BUTTON_GAP);
    for row in rows {
        ui.horizontal(|ui| {
            for i in row {
                let b = &buttons[i];
                let resp = widgets::action_button_if(ui, p, b.icon, b.label, b.kind, Vec2::new(0.0, BUTTON_H), b.enabled);
                if let Some(body) = b.menu {
                    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| body(ui));
                } else if resp.clicked() {
                    clicked = Some(i);
                }
                if let Some(tip) = b.tip {
                    resp.on_hover_text(tip);
                }
            }
        });
    }
    clicked
}

pub(super) fn muted(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(p.muted));
}

/// A labelled value: the label in a fixed column, the value wrapping.
pub(super) fn field(ui: &mut egui::Ui, p: &Palette, label: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, 16.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            p.muted,
        );
        ui.vertical(value);
    });
}

pub(super) fn text_field(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    field(ui, p, label, |ui| {
        ui.add(egui::Label::new(RichText::new(value).size(12.0).color(p.ink)).wrap());
    });
}

/// A resident's name as a link to the profile (LLSLURL "agent" "inspect").
pub(super) fn agent_link(ui: &mut egui::Ui, c: &mut Ctx, id: Uuid) {
    if id.is_nil() {
        ui.label(RichText::new(aurora_i18n::tr!("common-none")).size(12.0).color(c.p.ink));
        return;
    }
    c.want_names.insert(id);
    let name = c.world.social.name_of(&id);
    let link = ui.add(egui::Link::new(
        RichText::new(name).size(12.0).color(colors::text(colors::get().chat_slurl)),
    ));
    if link.clicked() {
        crate::ui::profile::request_open(ui.ctx(), id);
    }
}

/// A text with emoji and clickable links in a sunken box at least `height`
/// high.
pub(super) fn text_box(ui: &mut egui::Ui, c: &mut Ctx, text: &str, height: f32) {
    let p = c.p;
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.82))
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width((width - 14.0).max(40.0));
            ui.set_min_height(height);
            if text.trim().is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("common-none")).size(12.5).color(p.muted));
            }
            for line in text.lines() {
                if line.is_empty() {
                    ui.add_space(6.0);
                } else {
                    crate::ui::chat::chat_text(ui, p, c.emoji, c.world, c.want_names, line, 12.5, p.ink, false);
                }
            }
        });
}

/// A key on one line, selectable, with its copy button.
fn key_row(ui: &mut egui::Ui, p: &Palette, id: Uuid) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::Label::new(RichText::new(id.to_string()).size(11.0).monospace().color(p.muted)).selectable(true));
        if widgets::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("search-copy-key"), true) {
            ui.ctx().copy_text(id.to_string());
        }
    });
}

/// The rating badge and its name.
pub(super) fn rating(ui: &mut egui::Ui, p: &Palette, name: &str) {
    let size = Vec2::new(ui.available_width(), 16.0);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        widgets::maturity_badge(ui, p, name, 16.0);
        ui.label(RichText::new(name).size(12.0).color(p.ink));
    });
}

pub fn show(
    ui: &mut egui::Ui,
    c: &mut Ctx,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    wanted_pics: &mut HashSet<Uuid>,
    list: &List,
    actions: &mut Vec<SearchAction>,
) {
    let Some(key) = list.selected else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| muted(ui, c.p, aurora_i18n::tr!("search-select-a-result")));
        return;
    };
    let row = list.rows.iter().find(|r| r.key == key);
    let row_name = row.map_or("", |r| r.name.as_str());
    match (list.tab, key) {
        (Tab::People, Key::Id(id)) => person(ui, c, pics, wanted_pics, id, row_name),
        (Tab::Groups, Key::Id(id)) => group(ui, c, id, row_name, false, actions),
        (Tab::Places | Tab::Land, Key::Id(id)) => place(ui, c, id, row, actions),
        (Tab::Classifieds, Key::Id(id)) => classified(ui, c, id, row_name, actions),
        (Tab::Events, Key::Event(id)) => event(ui, c, id, row_name, actions),
        _ => {}
    }
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
    let blocked = world.is_avatar_blocked(&id);
    let here = context::avatar_position(world, &id).is_some();
    if profile.is_some_and(|pr| !pr.sl_image.is_nil()) {
        wanted_pics.insert(id);
    }
    c.want_names.insert(id);
    let known = world.social.avatar_names.complete(&id);
    let name = known.unwrap_or_else(|| row_name.to_owned());
    let gear = |ui: &mut egui::Ui| context::avatar_list_menu(ui, p, world, id, context::AvatarList::Name);
    let block = if blocked {
        aurora_i18n::tr!("common-unblock")
    } else {
        aurora_i18n::tr!("common-block")
    };
    let buttons = [
        Btn::new("map", aurora_i18n::tr!("ui-profile-show-on-map-2")).enabled(here && !own),
        Btn::new("send-filled", aurora_i18n::tr!("ui-profile-offer-teleport")).enabled(!own),
        Btn::new("user-plus", aurora_i18n::tr!("ui-profile-add-friend")).enabled(friend.is_none() && !own),
        Btn::new("coin-filled", aurora_i18n::tr!("common-pay")).enabled(!own),
        Btn::new("message-circle-filled", aurora_i18n::tr!("ui-profile-instant-message")).enabled(!own),
        Btn::new("ban", block).enabled(!own),
        Btn::new("settings-filled", aurora_i18n::tr!("ui-profile-more")).menu(&gear),
        Btn::new("user-filled", aurora_i18n::tr!("search-show-profile")).primary(),
    ];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let clicked = pane(ui, p, ("search-person", Key::Id(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal_top(|ui| {
            widgets::picture(
                ui,
                p,
                pics.get(&id),
                Vec2::splat(116.0),
                "user-filled",
                aurora_i18n::tr!("ui-profile-profile-picture"),
            );
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(&name).size(15.0).strong().color(p.ink)).truncate());
                key_row(ui, p, id);
                let status = profiles::online_status(profile, own, friend.map(|f| (f.online, f.rights_has & 1 != 0)));
                let (text, color) = match status {
                    Online::Yes => (aurora_i18n::tr!("ui-profile-connected"), p.success),
                    Online::No => (aurora_i18n::tr!("ui-profile-disconnected"), p.muted),
                    Online::Unknown => (aurora_i18n::tr!("common-unknown"), p.warn),
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    widgets::status_dot(ui, color);
                    ui.label(RichText::new(text).size(12.0).color(color));
                });
                match profile {
                    Some(pr) => {
                        // "03/19/2008 (18 years 6 months; 6781 days)" on one line
                        if let Some(birth) = profiles::birth_text(pr, own, now) {
                            ui.label(RichText::new(birth.replace('\n', " ")).size(12.0).color(p.ink));
                        }
                        ui.label(RichText::new(profiles::account_text(pr)).size(12.0).color(p.muted));
                    }
                    None => muted(ui, p, aurora_i18n::tr!("common-loading-2")),
                }
            });
        });
        let Some(pr) = profile else {
            return;
        };
        ui.add_space(2.0);
        field(ui, p, aurora_i18n::tr!("ui-profile-partner"), |ui| agent_link(ui, c, pr.partner));
        muted(ui, p, aurora_i18n::tr!("ui-profile-info"));
        text_box(ui, c, &pr.sl_about, 70.0);
        // the legacy reply (no AgentProfile capability) has no groups
        if !pr.legacy {
            field(ui, p, aurora_i18n::tr!("ui-profile-groups"), |ui| {
                let mut groups: Vec<&str> = pr.groups.iter().map(|g| g.name.as_str()).collect();
                groups.sort_by_key(|g| g.to_lowercase());
                let text = if groups.is_empty() {
                    aurora_i18n::tr!("common-none").to_owned()
                } else {
                    groups.join(", ")
                };
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ink)).wrap());
            });
        }
        // "Give: drop the object here" (LLDropTarget), not done yet
        field(ui, p, aurora_i18n::tr!("ui-profile-give"), |ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-profile-drop-the-object-here"))
                    .size(12.0)
                    .color(p.muted_dim),
            )
            .on_hover_text(menu::not_yet());
        });
    });
    let act = |a: CtxAction| context::request(ui.ctx(), a);
    match clicked {
        Some(0) => act(CtxAction::ShowOnMap(id)),
        Some(1) => act(CtxAction::OfferTeleport(id)),
        Some(2) => act(CtxAction::AddFriend(id)),
        Some(3) => act(CtxAction::PayResident(id)),
        Some(4) => act(CtxAction::Im(id)),
        Some(5) => act(CtxAction::ToggleBlock(id)),
        Some(7) => act(CtxAction::Profile(id)),
        _ => {}
    }
}

/// A group: the details of the groups tab, and the body of the group
/// profile window (`standalone`), which the last button opens from the tab.
pub(super) fn group(ui: &mut egui::Ui, c: &mut Ctx, id: Uuid, row_name: &str, standalone: bool, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
    let data = world.search.groups.get(&id);
    let member = world.groups.is_member(&id);
    let name = data.map_or(row_name, |g| g.name.as_str()).to_owned();
    let mut buttons = vec![
        Btn::new("refresh", aurora_i18n::tr!("search-refresh")),
        Btn::new("messages-filled", aurora_i18n::tr!("search-group-chat")).enabled(member),
        // LLGroupActions::join: fee confirmation and JoinGroupRequest
        Btn::new("users-plus", aurora_i18n::tr!("search-group-join")).enabled(!member && data.is_some_and(|g| g.open_enrollment)),
    ];
    if !standalone {
        // LLGroupActions::show
        buttons.push(Btn::new("users-group", aurora_i18n::tr!("search-show-profile")).primary());
    }
    let clicked = pane(ui, p, ("search-group", Key::Id(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal_top(|ui| {
            let insignia = data.map(|g| g.insignia).filter(|i| !i.is_nil());
            if let Some(i) = insignia {
                c.wanted_images.insert(i);
            }
            let tex = insignia.and_then(|i| c.images.get(&i));
            widgets::picture(ui, p, tex, Vec2::splat(96.0), "users-group", "");
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(&name).size(15.0).strong().color(p.ink)).truncate());
                key_row(ui, p, id);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let size = Vec2::new(0.0, 22.0);
                    let uri = aurora_i18n::tr!("search-group-copy-uri");
                    if widgets::action_button(ui, p, "link-filled", uri, ActionKind::Flat, size).clicked() {
                        ui.ctx().copy_text(format!("secondlife:///app/group/{id}/about"));
                    }
                    let copy_name = aurora_i18n::tr!("search-group-copy-name");
                    if widgets::action_button(ui, p, "copy-filled", copy_name, ActionKind::Flat, size).clicked() {
                        ui.ctx().copy_text(name.clone());
                    }
                });
                if let Some(members) = world.group_member_count(id) {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("search-group-members", count = members))
                            .size(12.0)
                            .color(p.ink),
                    );
                }
            });
        });
        let Some(g) = data else {
            muted(ui, p, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        ui.add_space(2.0);
        field(ui, p, aurora_i18n::tr!("search-group-founder"), |ui| agent_link(ui, c, g.founder));
        muted(ui, p, aurora_i18n::tr!("search-group-charter"));
        text_box(ui, c, &g.charter, 90.0);
        if member && !g.member_title.is_empty() {
            text_field(ui, p, aurora_i18n::tr!("search-group-my-title"), &g.member_title);
        }
        // the group's settings, read only here
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
        field(ui, p, aurora_i18n::tr!("search-content"), |ui| {
            rating(
                ui,
                p,
                if g.mature_publish {
                    aurora_i18n::tr!("common-moderate")
                } else {
                    aurora_i18n::tr!("common-general")
                },
            )
        });
        // members, titles and status need GroupMembersRequest
        field(ui, p, aurora_i18n::tr!("search-col-members"), |ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("search-group-member-list-later"))
                    .size(12.0)
                    .color(p.muted_dim),
            )
            .on_hover_text(menu::not_yet());
        });
    });
    match clicked {
        Some(0) => actions.push(SearchAction::RefreshGroup(id)),
        Some(1) => context::request(ui.ctx(), CtxAction::GroupChat(id)),
        Some(2) => actions.push(SearchAction::JoinGroup(id)),
        Some(3) => crate::ui::group_profile::request_open(ui.ctx(), id),
        _ => {}
    }
}

/// A parcel of the places and land sales tabs: the place profile of
/// "Places", with Teleport, Map and its menu.
fn place(ui: &mut egui::Ui, c: &mut Ctx, parcel: Uuid, row: Option<&Row>, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
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
        Btn::new("location-filled", aurora_i18n::tr!("common-teleport"))
            .primary()
            .enabled(global.is_some()),
        Btn::new("map", aurora_i18n::tr!("common-map")).enabled(global.is_some()),
        Btn::new("menu-2-filled", aurora_i18n::tr!("ui-profile-more")).menu(&more),
    ];
    let clicked = pane(ui, p, ("search-place", Key::Id(parcel)), &buttons, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("common-place-profile")).size(12.0).color(p.muted));
        ui.set_max_width(ui.available_width().min(340.0));
        place_details::profile_body(ui, c, &place);
        // ParcelInfoReply keeps the last asking price after a sale ends: only
        // a row the search reply marked for sale shows it
        let for_sale = row.is_some_and(|r| r.icon == RowIcon::ForSale);
        if let Some(d) = data.filter(|d| for_sale && d.sale_price > 0) {
            ui.add_space(4.0);
            text_field(ui, p, aurora_i18n::tr!("search-col-price"), &format!("L${}", d.sale_price));
        }
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
    let clicked = pane(ui, p, ("search-classified", Key::Id(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        muted(ui, p, aurora_i18n::tr!("search-classified-info"));
        let Some(info) = info else {
            ui.label(RichText::new(row_name).size(15.0).strong().color(p.ink));
            muted(ui, p, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        if !info.snapshot.is_nil() {
            c.wanted_images.insert(info.snapshot);
        }
        let width = ui.available_width().min(280.0);
        widgets::picture(
            ui,
            p,
            c.images.get(&info.snapshot),
            Vec2::new(width, width * 161.0 / 260.0),
            "article-filled",
            "",
        );
        ui.add(egui::Label::new(RichText::new(&info.name).size(15.0).strong().color(p.ink)).wrap());
        text_field(
            ui,
            p,
            aurora_i18n::tr!("ui-profile-location"),
            &aurora_net::profile::location_text(&[&info.parcel_name, &info.sim_name], info.pos_global),
        );
        // is_cf_mature: the mature flag of the ad or of its query
        let mature = info.flags & (classified_flags::MATURE | classified_flags::QUERY_INC_MATURE) != 0;
        field(ui, p, aurora_i18n::tr!("ui-profile-content-type"), |ui| {
            rating(
                ui,
                p,
                if mature {
                    aurora_i18n::tr!("common-moderate")
                } else {
                    aurora_i18n::tr!("common-general")
                },
            )
        });
        text_field(
            ui,
            p,
            aurora_i18n::tr!("common-category"),
            &world.classified_category(info.category),
        );
        text_field(
            ui,
            p,
            aurora_i18n::tr!("ui-profile-creation-date"),
            &profiles::slt_date(info.creation_date),
        );
        text_field(ui, p, aurora_i18n::tr!("ui-profile-listing-cost"), &format!("L${}", info.price));
        field(ui, p, aurora_i18n::tr!("search-classified-creator"), |ui| {
            agent_link(ui, c, info.creator)
        });
        muted(ui, p, aurora_i18n::tr!("common-description"));
        text_box(ui, c, &info.desc, 80.0);
    });
    match (clicked, global) {
        (Some(0), Some(g)) => actions.push(SearchAction::Teleport(g)),
        (Some(1), Some(g)) => actions.push(SearchAction::ShowOnMap(g)),
        _ => {}
    }
}

fn event(ui: &mut egui::Ui, c: &mut Ctx, id: u32, row_name: &str, actions: &mut Vec<SearchAction>) {
    let (p, world) = (c.p, c.world);
    let info = world.search.events.get(&id);
    let global = info.map(|e| e.global).filter(|g| *g != DVec3::ZERO);
    let buttons = [
        Btn::new("location-filled", aurora_i18n::tr!("common-teleport"))
            .primary()
            .enabled(global.is_some()),
        Btn::new("map", aurora_i18n::tr!("search-event-show-on-map")).enabled(global.is_some()),
        // onBtnEventReminder: LLEventNotifier is not ported
        Btn::new("bell-filled", aurora_i18n::tr!("search-event-remind")).not_yet(),
    ];
    let clicked = pane(ui, p, ("search-event", Key::Event(id)), &buttons, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let Some(e) = info else {
            ui.label(RichText::new(row_name).size(15.0).strong().color(p.ink));
            muted(ui, p, aurora_i18n::tr!("common-loading-2"));
            return;
        };
        ui.add(egui::Label::new(RichText::new(&e.name).size(15.0).strong().color(p.ink)).wrap());
        text_field(ui, p, aurora_i18n::tr!("common-type-2"), &e.category);
        field(ui, p, aurora_i18n::tr!("search-content"), |ui| {
            rating(
                ui,
                p,
                match search::event_rating(e.flags) {
                    aurora_net::search::SIM_ACCESS_ADULT => aurora_i18n::tr!("common-adult"),
                    aurora_net::search::SIM_ACCESS_MATURE => aurora_i18n::tr!("common-moderate"),
                    _ => aurora_i18n::tr!("common-general"),
                },
            )
        });
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
        muted(ui, p, aurora_i18n::tr!("common-description"));
        text_box(ui, c, &e.desc, 80.0);
    });
    match (clicked, global) {
        (Some(0), Some(g)) => actions.push(SearchAction::Teleport(g)),
        (Some(1), Some(g)) => actions.push(SearchAction::ShowOnMap(g)),
        _ => {}
    }
}
