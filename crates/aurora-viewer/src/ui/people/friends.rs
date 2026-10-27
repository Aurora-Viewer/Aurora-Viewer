//! People › Friends in Aurora's look (the People mock-up v2): the friend
//! list of Conversations › Contacts (FSFloaterContacts, fsfloatercontacts.cpp,
//! panel_fs_contacts_friends.xml) as rich rows, online and offline apart.
//! Each row carries the three rights we grant, clickable as the list's
//! boxes are (applyRightsToFriends: the map goes with the online status,
//! the right to modify our objects is confirmed first), and after a line
//! the two we receive. Ctrl / Shift choose several for the menu of several;
//! a long press opens the sheet. Search, pictures and rights stay available;
//! Legacy column options apply only to the contact table. The sort is in
//! pills (FriendsSortOrder), and a contact set can filter the list.

use super::rows::{self, RICH_GAP, RICH_H};
use super::{Ctx, PeopleUi, SORT_NAME, SORT_STATUS, SORT_USERNAME};
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::{menu, widgets};
use crate::world::social::{Friend, rights};
use egui::{Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// Icon of a sort of the friends in the menus.
pub(super) const fn sort_icon(sort: u8) -> &'static str {
    match sort {
        SORT_STATUS => "user-check",
        SORT_USERNAME => "at",
        _ => "sort-ascending-letters",
    }
}

/// One right's cell on a row, and the gap between two.
const CELL: Vec2 = Vec2::new(20.0, 22.0);
const CELL_GAP: f32 = 2.0;
/// The rights we grant, then the ones we receive (after their line).
const GRANTED_W: f32 = CELL.x * 3.0 + CELL_GAP * 2.0;
const RECEIVED_W: f32 = 9.0 + CELL.x * 2.0 + CELL_GAP;
/// Gap between the parts of a row's right end.
const PARTS_GAP: f32 = 8.0;

/// The rights we grant: the bit, its icon, what it says.
const GRANTED: [(i32, &str, &str); 3] = [
    (
        rights::ONLINE_STATUS,
        "eye-filled",
        "ui-contacts-this-person-can-see-when-you-are-online",
    ),
    (rights::MAP_LOCATION, "map-pin-filled", "ui-contacts-this-person-can-locate-you"),
    (
        rights::MODIFY_OBJECTS,
        "pencil-filled",
        "ui-contacts-this-person-can-modify-delete-or-take-your-objects",
    ),
];
/// The ones the friend grants us that the list shows (as Contacts does).
const RECEIVED: [(i32, &str, &str); 2] = [
    (rights::MAP_LOCATION, "map-pin-filled", "ui-contacts-you-can-locate-this-person"),
    (
        rights::MODIFY_OBJECTS,
        "pencil-filled",
        "ui-contacts-you-can-edit-delete-or-take-this-person-s",
    ),
];

/// The friends tab: the list, and the sheet of the friend held.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    super::split(ui, c, data, st, "people-friends-list", list, super::person_sheet);
}

/// A friend's place in the list, as the row shows it.
struct Line {
    friend: Friend,
    /// The first line: the name the view options choose.
    name: String,
    /// The full name in its chosen format (on hover).
    full: String,
    username: String,
    /// The second line: their status and where they are, when known.
    status: String,
    /// Their contact set's symbol (the highest-priority set with one), else
    /// the color of their first contact set.
    set: Option<rows::SetMark>,
}

fn lines(c: &Ctx, filter: &str, set: Option<&str>) -> Vec<Line> {
    let world = c.world;
    let names = &world.social.avatar_names;
    let o = names.options;
    // where the friends around us are (the radar's sightings)
    let regions: HashMap<Uuid, String> = crate::world::radar::sightings(world)
        .into_iter()
        .filter_map(|s| Some((s.id, world.regions.get(&s.region)?.name.clone())))
        .collect();
    let mut out: Vec<Line> = world
        .social
        .friends
        .iter()
        .filter(|f| set.is_none_or(|s| world.contact_sets.friend_sets(&f.id).iter().any(|x| x == s)))
        .map(|f| {
            let (name, full, username) = match names.get(&f.id) {
                Some(n) => {
                    let username = n.user_name_for_display(&o);
                    (n.display(&o), n.complete(&o), username)
                }
                None => {
                    let legacy = world.social.name_of(&f.id);
                    (legacy.clone(), legacy.clone(), legacy)
                }
            };
            let status = crate::ui::presence::of(world, &f.id);
            let state = if f.online {
                status.label()
            } else {
                aurora_i18n::tr!("common-offline")
            };
            let status = match (f.online, regions.get(&f.id)) {
                (true, Some(region)) if status.pill() => format!("{state} · {region}"),
                (true, Some(region)) => region.clone(),
                _ => state.to_owned(),
            };
            let icon = world.contact_sets.appearance(&f.id).icons.first().map(|i| {
                rows::SetMark::Icon(
                    crate::world::contact_sets::appearance::ICONS[i.icon],
                    crate::ui::colors::set_color(i.color),
                )
            });
            let set = icon.or_else(|| {
                world
                    .contact_sets
                    .friend_color(&f.id, |id| world.social.is_friend(id))
                    .or_else(|| {
                        let first = world.contact_sets.friend_sets(&f.id).into_iter().next()?;
                        Some(world.contact_sets.set(&first)?.color)
                    })
                    .map(|c| rows::SetMark::Dot(crate::ui::contacts::color32(c)))
            });
            Line {
                friend: f.clone(),
                name,
                full,
                username,
                status,
                set,
            }
        })
        .filter(|l| super::matches_name(world, &l.friend.id, &l.full, filter))
        .collect();
    let sort = c.friends.sort;
    let mut friends: Vec<Friend> = out.iter().map(|l| l.friend.clone()).collect();
    let by_name: HashMap<Uuid, (String, String)> = out.iter().map(|l| (l.friend.id, (l.name.clone(), l.username.clone()))).collect();
    super::sort_friends(
        &mut friends,
        sort,
        |id| by_name.get(id).map(|n| n.0.clone()).unwrap_or_default(),
        |id| by_name.get(id).map(|n| n.1.clone()).unwrap_or_default(),
    );
    let order: HashMap<Uuid, usize> = friends.iter().enumerate().map(|(i, f)| (f.id, i)).collect();
    out.sort_by_key(|l| order.get(&l.friend.id).copied().unwrap_or(usize::MAX));
    out
}

/// The list side: toolbar, pills, then the friends.
fn list(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, beside_sheet: bool) {
    if beside_sheet {
        ui.set_max_width(ui.max_rect().width() - 10.0);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    toolbar(ui, c, st);
    ui.add_space(4.0);
    pills(ui, c, st);
    ui.add_space(7.0);
    let filter = st.filter.trim().to_lowercase();
    if st.friend_set.as_deref().is_some_and(|s| !c.world.contact_sets.is_valid(s)) {
        st.friend_set = None;
    }
    let all = lines(c, &filter, st.friend_set.as_deref());
    let order: Vec<Uuid> = all.iter().map(|l| l.friend.id).collect();
    st.sel.retain(|id| order.contains(id));
    if c.contact_sheet_on_click {
        let selected = st
            .sheet
            .filter(|id| order.contains(id))
            .or_else(|| st.sel.first().copied())
            .or_else(|| order.first().copied());
        if let Some(id) = selected {
            if st.sheet != Some(id) {
                super::nearby::open_sheet(st, id);
                st.sheet_grow = false;
            }
            if st.sel.is_empty() {
                st.sel.push(id);
                st.anchor = Some(id);
            }
        } else {
            super::nearby::close_sheet(st);
        }
    }
    let sections = c.friends.sort == SORT_STATUS;
    let online = all.iter().filter(|l| l.friend.online).count();
    egui::ScrollArea::vertical()
        .id_salt("people-friends-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if all.is_empty() {
                rows::empty(ui, c.p, aurora_i18n::tr!("ui-contacts-no-friends"));
            }
            let mut section = None;
            for line in &all {
                if sections && section != Some(line.friend.online) {
                    section = Some(line.friend.online);
                    let title = if line.friend.online {
                        aurora_i18n::tr!("people-friends-online", count = online as i64)
                    } else {
                        aurora_i18n::tr!("people-friends-offline", count = (all.len() - online) as i64)
                    };
                    super::nearby::section_header(ui, c.p, &title);
                }
                row(ui, c, data, st, line, &order);
            }
        });
}

/// Search and "Add" (the friendship offer to a resident chosen in the picker).
fn toolbar(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi) {
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    let add_label = aurora_i18n::tr!("common-add");
    let add_label = if line.x < widgets::action_button_fit(ui, "user-plus", add_label) + 6.0 + 100.0 {
        ""
    } else {
        add_label
    };
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let add = widgets::action_button(
            ui,
            c.p,
            "user-plus",
            add_label,
            widgets::ActionKind::Flat,
            Vec2::new(0.0, rows::TOOLBAR_H),
        )
        .on_hover_text(aurora_i18n::tr!("people-friends-add"));
        if add.clicked() {
            c.contacts.pick_new_friend();
        }
        let w = ui.available_width();
        widgets::search_box(ui, c.p, &mut st.filter, aurora_i18n::tr!("people-friends-filter"), w);
    });
}

/// "Sort" and its three pills, then "Set" and the contact sets that fit
/// (the others in "+2"): a lit set shows its members only.
fn pills(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi) {
    let p = c.p;
    // the sets with the most friends first: the ones this list filters best
    let mut sets = c.world.contact_sets.set_names();
    let friends_in = |s: &String| {
        c.world
            .contact_sets
            .set(s)
            .map_or(0, |set| set.friends.iter().filter(|id| c.world.social.is_friend(id)).count())
    };
    sets.sort_by_key(|s| std::cmp::Reverse(friends_in(s)));
    let sets_below = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            label(ui, p, aurora_i18n::tr!("people-sort"));
            let sorts = [
                (SORT_STATUS, "people-sort-online-first"),
                (SORT_NAME, "common-name"),
                (SORT_USERNAME, "common-username"),
            ];
            let width: f32 = sorts
                .iter()
                .map(|(_, key)| widgets::filter_pill_width(ui, "", aurora_i18n::text(key)))
                .sum::<f32>()
                + 10.0;
            if width <= ui.available_width() {
                for (sort, key) in sorts {
                    if widgets::filter_pill(ui, p, "", aurora_i18n::text(key), c.friends.sort == sort).clicked() {
                        c.friends.sort = sort;
                    }
                }
            } else {
                let key = sorts.iter().find(|(sort, _)| *sort == c.friends.sort).unwrap_or(&sorts[0]).1;
                let sort = widgets::filter_pill(ui, p, "chevron-down", aurora_i18n::text(key), true);
                menu::dropdown(&sort, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    for (sort, key) in sorts {
                        if menu::check(ui, p, sort_icon(sort), aurora_i18n::text(key), c.friends.sort == sort, true) {
                            c.friends.sort = sort;
                        }
                    }
                });
            }
            if sets.is_empty() {
                return false;
            }
            let label_width = ui
                .painter()
                .layout_no_wrap(
                    aurora_i18n::tr!("people-set").to_owned(),
                    egui::FontId::proportional(11.0),
                    p.muted_dim,
                )
                .size()
                .x;
            let needed = 6.0 + label_width + 5.0 + widgets::filter_pill_width(ui, "", "+00");
            if ui.available_width() < needed {
                return true;
            }
            ui.add_space(6.0);
            label(ui, p, aurora_i18n::tr!("people-set"));
            set_pills(ui, c, &sets, &mut st.friend_set);
            false
        })
        .inner;
    if sets_below {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            label(ui, p, aurora_i18n::tr!("people-set"));
            set_pills(ui, c, &sets, &mut st.friend_set);
        });
    }
}

/// The small muted word before a row of pills.
pub(super) fn label(ui: &mut egui::Ui, p: &crate::theme::Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(p.muted_dim));
}

/// The contact sets as dot pills in what is left of the line, the others
/// in a "+2" that lists them; clicking one chooses it (again: none).
pub(super) fn set_pills(ui: &mut egui::Ui, c: &Ctx, sets: &[String], chosen: &mut Option<String>) {
    let p = c.p;
    let mut room = ui.available_width();
    let more_w = widgets::filter_pill_width(ui, "", "+00");
    for (i, set) in sets.iter().enumerate() {
        let w = widgets::dot_pill_width(ui, set, None, false);
        let rest = sets.len() - i - 1;
        if w + if rest > 0 { 5.0 + more_w } else { 0.0 } > room {
            let more = widgets::filter_pill(ui, p, "", &format!("+{}", sets.len() - i), false);
            menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                for set in &sets[i..] {
                    if menu::check(ui, p, "users-group", set, chosen.as_deref() == Some(set.as_str()), true) {
                        *chosen = (chosen.as_deref() != Some(set.as_str())).then(|| set.clone());
                    }
                }
            });
            return;
        }
        let color = c
            .world
            .contact_sets
            .set(set)
            .map_or(p.muted, |s| crate::ui::contacts::color32(s.color));
        let on = chosen.as_deref() == Some(set.as_str());
        if widgets::dot_pill(ui, p, color, set, None, on, false).0.clicked() {
            *chosen = (!on).then(|| set.clone());
        }
        room -= w + 5.0;
    }
}

/// One friend: picture, names, set dot, rights, and the quick actions
/// under the pointer.
fn row(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, line: &Line, order: &[Uuid]) {
    let p = c.p;
    let f = &line.friend;
    let width = ui.available_width().min(ui.clip_rect().right() - ui.cursor().left()).max(0.0);
    let bounds = Rect::from_min_size(ui.cursor().min, Vec2::new(width, RICH_H));
    let widths = [RECEIVED_W, GRANTED_W];
    let end = rows::end_before(
        bounds,
        rows::names_left(bounds) + rows::actions_width_for(5, 18.0) + PARTS_GAP,
        &widths,
        PARTS_GAP,
        false,
    );
    let right = end.items.last().map_or(bounds.right(), |r| r.left()) - PARTS_GAP;
    let count = 4 + usize::from(f.online);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, RICH_H), Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    if !data.pics.contains_key(&f.id) {
        st.want_pic(f.id, ui.input(|i| i.time));
    }
    let selected = st.sel.contains(&f.id) || st.sheet == Some(f.id);
    let hot = (ui.rect_contains_pointer(rect) && !resp.dragged()) || st.row_menu == Some(f.id) || st.demo_hover == Some(f.id);
    let opacity = ui.opacity();
    if !f.online {
        ui.set_opacity(opacity * 0.75);
    }
    rows::face(ui, p, rect, selected, hot);
    let identity = rect;
    let left = rows::names_left(rect);
    // Keep the permission cells anchored when hover actions appear, as
    // LLAvatarListItem::showPermissions / updateChildren reserve their slots.
    let slot = rows::actions_slot(rect, left, right, count, rows::BUTTON);
    let actions = hot.then(|| (quick_actions(f), slot));
    rows::picture(ui, p, c.world, identity, data.pics.get(&f.id), f.id, &line.name, false);
    let mut clicked_right = false;
    if let Some(cell) = end.items.first() {
        received(ui, p, f, *cell);
    }
    if let Some(cell) = end.items.get(1) {
        clicked_right = granted(ui, c, f, *cell);
    }
    let color = crate::ui::colors::resident_name(c.world, &f.id, p.ink);
    let cut = if end.names_right > left {
        rows::person_names(
            ui,
            p,
            identity,
            left,
            end.names_right,
            &line.name,
            color,
            &line.status,
            line.set,
            None,
            crate::world::staff::mark(c.world, &f.id),
        )
    } else {
        true
    };
    // The requested hover strip covers the text without taking any of its width.
    if let Some((buttons, slot)) = &actions {
        hover_actions(ui, c, st, f, *slot, buttons);
    }
    ui.set_opacity(opacity);
    let action = if clicked_right {
        rows::RowAction::default()
    } else {
        rows::select(ui, &resp, &mut st.sel, &mut st.anchor, order, f.id)
    };
    let click_sheet =
        c.contact_sheet_on_click && !clicked_right && resp.clicked() && ui.input(|i| !i.modifiers.command && !i.modifiers.shift);
    if action.sheet || click_sheet {
        super::nearby::open_sheet(st, f.id);
        if c.embedded_contacts {
            st.sheet_grow = false;
        }
    }
    if action.activate {
        c.actions.push(super::PeopleAction::OpenIm(f.id));
    }
    rows::person_menu(&resp, p, c.world, &st.sel, f.id);
    let on_names = resp.hover_pos().is_some_and(|at| at.x >= left && at.x <= end.names_right);
    rows::contact_tip(resp, (on_names && (cut || line.full != line.name)).then_some(line.full.as_str()));
}

/// Profile, IM, a teleport offer to a friend online, the contact
/// contact sheet, then the whole menu.
fn quick_actions(f: &Friend) -> Vec<(&'static str, &'static str)> {
    let mut b = vec![
        ("user-filled", aurora_i18n::tr!("common-view-profile")),
        ("message-circle-filled", aurora_i18n::tr!("ui-context-send-im")),
    ];
    if f.online {
        b.push(("send-filled", aurora_i18n::tr!("common-offer-teleport")));
    }
    b.push(("info-circle-filled", aurora_i18n::tr!("people-show-contact-sheet")));
    b.push(("dots-filled", aurora_i18n::tr!("ui-chat-more")));
    b
}

fn hover_actions(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, f: &Friend, slot: Rect, buttons: &[(&str, &str)]) {
    let mut menu_open = false;
    for ((icon, _), resp) in buttons
        .iter()
        .zip(rows::actions(ui, c.p, slot, buttons, ("people-friend-act", f.id)))
    {
        match *icon {
            "user-filled" if resp.clicked() => c.actions.push(super::PeopleAction::Profile(f.id)),
            "message-circle-filled" if resp.clicked() => c.actions.push(super::PeopleAction::OpenIm(f.id)),
            "send-filled" if resp.clicked() => c.actions.push(super::PeopleAction::OfferTeleport(f.id)),
            "info-circle-filled" if resp.clicked() => {
                st.sel = vec![f.id];
                st.anchor = Some(f.id);
                super::nearby::open_sheet(st, f.id);
                if c.embedded_contacts {
                    st.sheet_grow = false;
                }
            }
            "dots-filled" => {
                let (p, world) = (c.p, c.world);
                menu_open = menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    context::avatar_list_menu(ui, p, world, f.id, AvatarList::Friend);
                });
            }
            _ => {}
        }
    }
    super::keep_row_menu(st, f.id, menu_open);
}

/// The three rights we grant, each a click to change (lit in the accent
/// when granted); true when one was clicked.
fn granted(ui: &mut egui::Ui, c: &mut Ctx, f: &Friend, slot: Rect) -> bool {
    let p = c.p;
    let mut clicked = false;
    for (i, (bit, icon, tip)) in GRANTED.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(slot.left() + i as f32 * (CELL.x + CELL_GAP), slot.center().y - CELL.y / 2.0),
            CELL,
        );
        let on = f.rights_given & bit != 0;
        let resp = ui.interact(cell, ui.scope_id().with(("people-right", f.id, i)), Sense::click());
        let face = if on {
            Some(p.violet.gamma_multiply(if resp.hovered() { 0.5 } else { 0.32 }))
        } else {
            resp.hovered().then(|| p.raised.gamma_multiply(0.6))
        };
        if let Some(face) = face {
            ui.painter()
                .rect_filled(cell.shrink2(Vec2::new(1.0, 2.0)), widgets::inner_radius(ui.visuals()), face);
        }
        super::radar::icon(ui, icon, cell, 13.0, if on { p.violet_pale } else { p.muted_dim });
        let resp = resp
            .on_hover_text(aurora_i18n::text(tip))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            clicked = true;
            let new = crate::ui::contacts::rights_after(f.rights_given, bit, !on);
            if bit == rights::MODIFY_OBJECTS {
                c.contacts.ask_modify_rights(f.id, new, !on);
            } else {
                context::request(ui.ctx(), CtxAction::GrantRights { friend: f.id, rights: new });
            }
        }
    }
    clicked
}

/// What the friend grants us, read only: after a faint line, turquoise when
/// granted.
fn received(ui: &mut egui::Ui, p: &crate::theme::Palette, f: &Friend, slot: Rect) {
    let y = slot.center().y;
    ui.painter()
        .vline(slot.left() + 1.0, (y - 9.0)..=(y + 9.0), widgets::divider_stroke(p));
    for (i, (bit, icon, tip)) in RECEIVED.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(slot.left() + 9.0 + i as f32 * (CELL.x + CELL_GAP), y - CELL.y / 2.0),
            CELL,
        );
        let on = f.rights_has & bit != 0;
        super::radar::icon(ui, icon, cell, 12.0, if on { p.teal } else { p.muted_dim.gamma_multiply(0.7) });
        ui.interact(cell, ui.scope_id().with(("people-received", f.id, i)), Sense::hover())
            .on_hover_text(aurora_i18n::text(tip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_controls_fit_the_list_at_transition_widths() {
        let theme = crate::theme::Theme::default();
        let p = theme.palette();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        theme.apply(&frames.ctx, 1.0);
        // Compare hover states without the scroll bar's width animation.
        frames.ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for index in 0..20 {
            let id = Uuid::from_u128(0xF000 + index);
            world.social.friends.push(Friend {
                id,
                online: true,
                rights_given: 3,
                rights_has: 7,
            });
            world.social.names.insert(id, format!("Friend {index:02} with a long display name"));
        }
        for name in ["Family and close friends", "Work and projects", "A circle with a very long name"] {
            world.contact_sets.add_set(name);
        }
        let radar = crate::world::radar::Radar::default();
        let mut radar_settings = super::super::RadarSettings::default();
        let (voice, pics) = (HashMap::new(), HashMap::new());
        let data = super::super::RadarData {
            radar: &radar,
            settings: &mut radar_settings,
            voice: &voice,
            draw_distance: 64.0,
            now: 0.0,
            pics: &pics,
        };
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut settings = super::super::FriendsSettings::default();
        let mut actions = Vec::new();
        let mut st = PeopleUi {
            demo_hover: Some(Uuid::from_u128(0xF000)),
            ..PeopleUi::default()
        };
        for width in [
            240.0, 254.0, 255.0, 256.0, 259.0, 260.0, 261.0, 280.0, 300.0, 320.0, 340.0, 360.0, 380.0, 400.0, 450.0, 510.0, 640.0,
        ] {
            for sort in [SORT_STATUS, SORT_NAME, SORT_USERNAME] {
                for embedded in [false, true] {
                    settings.sort = sort;
                    let bounds = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(width, 500.0));
                    let mut fixed_rights = None;
                    let mut fixed_name = None;
                    for hot in [false, true] {
                        st.demo_hover = hot.then_some(Uuid::from_u128(0xF000));
                        let mut layer = None;
                        let mut shapes = Vec::new();
                        for _ in 0..3 {
                            shapes = frames.shapes(Vec::new(), |ui| {
                                let mut child = super::super::nearby::list_ui(ui, bounds, "bounded-friends-test", true);
                                layer = Some(child.layer_id());
                                let mut c = Ctx {
                                    p: &p,
                                    world: &world,
                                    contacts: &mut contacts,
                                    friends: &mut settings,
                                    images: &pics,
                                    actions: &mut actions,
                                    embedded_contacts: embedded,
                                    contact_sheet_on_click: embedded,
                                };
                                list(&mut child, &mut c, &data, &mut st, true);
                            });
                        }
                        let rects: Vec<_> = frames
                            .ctx
                            .viewport(|v| v.this_pass.widgets.get_layer(layer.expect("drawn")).cloned().collect());
                        for widget in &rects {
                            if widget.sense.senses_click()
                                && widget.rect.height() <= 64.0
                                && widget.rect.top() >= bounds.top()
                                && widget.rect.bottom() <= bounds.bottom()
                            {
                                assert!(
                                    widget.rect.left() >= bounds.left() - 0.5 && widget.rect.right() <= bounds.right() + 0.5,
                                    "{width}, sort {sort}: a control leaves the list: {:?} outside {bounds:?}",
                                    widget.rect
                                );
                            }
                        }
                        let rights: Vec<_> = rects.iter().filter(|w| w.rect.size() == CELL).map(|w| (w.id, w.rect)).collect();
                        if width >= 300.0 {
                            assert!(!rights.is_empty(), "the test exercises visible permissions");
                        }
                        if let Some(fixed) = &fixed_rights {
                            assert_eq!(&rights, fixed, "{width}, sort {sort}: hover must not hide or move permission cells");
                        } else {
                            fixed_rights = Some(rights.clone());
                        }
                        let quick: Vec<_> = rects
                            .iter()
                            .filter(|w| w.sense.senses_click() && w.rect.height() == 24.0 && w.rect.width() <= 26.0)
                            .collect();
                        let (name_index, name) = shapes
                            .iter()
                            .enumerate()
                            .find_map(|(i, shape)| match shape {
                                egui::Shape::Text(t) if t.galley.text() == "Friend 00 with a long display name" => Some((i, t)),
                                _ => None,
                            })
                            .expect("the first friend's name is painted");
                        let shown: String = name.galley.rows.iter().flat_map(|r| r.row.glyphs.iter().map(|g| g.chr)).collect();
                        let layout = (shown, name.pos, name.galley.size());
                        if let Some(fixed) = &fixed_name {
                            assert_eq!(&layout, fixed, "hover actions do not change the name's layout");
                        } else {
                            fixed_name = Some(layout);
                        }
                        if hot {
                            assert_eq!(quick.len(), 5, "{width}: every quick action remains visible");
                            if width <= 400.0 {
                                assert!(
                                    name.pos.x + name.galley.size().x > quick[0].rect.right(),
                                    "the name uses the space underneath the hover actions"
                                );
                            }
                            let pill_index = shapes
                                .iter()
                                .position(|shape| matches!(shape, egui::Shape::Rect(r) if r.fill == p.raised && quick.iter().all(|w| r.rect.contains_rect(w.rect))))
                                .expect("the action strip is painted");
                            assert!(pill_index > name_index, "the hover strip is painted over the name");
                            for action in quick {
                                let row = rects
                                    .iter()
                                    .filter(|w| w.sense.senses_drag() && w.rect.contains_rect(action.rect))
                                    .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
                                    .expect("the action belongs to a row");
                                assert_eq!(
                                    row.rect.height(),
                                    RICH_H,
                                    "{width}: shrinking the list must not make its rows taller"
                                );
                                assert_eq!(action.rect.center().y, row.rect.center().y, "every action stays on one line");
                                assert!(action.rect.left() >= rows::names_left(row.rect), "the avatar remains clickable");
                                for (_, permission) in &rights {
                                    if row.rect.contains(permission.center()) {
                                        assert!(
                                            action.rect.right() < permission.left(),
                                            "{width}: hover actions stay left of permissions"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The right end of a friend's row, at every width, with or without the
    /// rights and the quick actions: nothing overlaps (the generic rule of
    /// `rows::end`, with the friends' own parts).
    #[test]
    fn rights_and_actions_never_cover_each_other() {
        let overlap = |a: &Rect, b: &Rect| a.left() < b.right() - 0.01 && b.left() < a.right() - 0.01;
        for width in (160..=640).step_by(2) {
            let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(width as f32, RICH_H));
            for widths in [
                vec![RECEIVED_W, GRANTED_W],
                vec![RECEIVED_W, GRANTED_W, rows::actions_width(4)],
                vec![rows::actions_width(3)],
            ] {
                let end = rows::end(row, rows::names_left(row), &widths, PARTS_GAP, false);
                for (i, a) in end.items.iter().enumerate() {
                    assert!(a.right() <= row.right());
                    for b in &end.items[i + 1..] {
                        assert!(!overlap(a, b), "{width}: {a:?} over {b:?}");
                    }
                }
            }
        }
        // the mock-up's list (510 points) shows the rights and the actions
        let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, RICH_H));
        let end = rows::end(
            row,
            rows::names_left(row),
            &[RECEIVED_W, GRANTED_W, rows::actions_width(4)],
            PARTS_GAP,
            false,
        );
        assert_eq!(end.items.len(), 3);
    }

    #[test]
    fn view_settings_default_to_the_mockup() {
        // online first, with the online and offline parts
        assert_eq!(super::super::FriendsSettings::default().sort, SORT_STATUS);
    }
}
