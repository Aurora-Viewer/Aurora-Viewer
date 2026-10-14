//! People › Friends in Aurora's look (the People mock-up v2): the friend
//! list of Conversations › Contacts (FSFloaterContacts, fsfloatercontacts.cpp,
//! panel_fs_contacts_friends.xml) as rich rows, online and offline apart.
//! Each row carries the three rights we grant, clickable as the list's
//! boxes are (applyRightsToFriends: the map goes with the online status,
//! the right to modify our objects is confirmed first), and after a line
//! the two we receive. Ctrl / Shift choose several for the menu of several;
//! a plain click opens the sheet. The view options are the list's own
//! (menu "Options", FSFriendListColumnShow* and the others), the sort is in
//! pills (FriendsSortOrder), and a contact set can filter the list.

use super::rows::{self, RICH_GAP, RICH_H};
use super::{Ctx, PeopleUi, SORT_NAME, SORT_STATUS, SORT_USERNAME};
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::{menu, widgets};
use crate::world::social::{Friend, rights};
use egui::{Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

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

/// The friends tab: the list, and the sheet of the friend clicked.
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
    /// The color of their first contact set.
    set: Option<egui::Color32>,
}

fn lines(c: &Ctx, filter: &str, set: Option<&str>) -> Vec<Line> {
    let world = c.world;
    let names = &world.social.avatar_names;
    let o = names.options;
    let cs = &*c.contact_settings;
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
                    let name = if cs.column_username && !cs.column_display_name && !cs.column_full_name {
                        username.clone()
                    } else {
                        n.display(&o)
                    };
                    (name, crate::ui::contacts::full_name(n, &o, cs.full_name_display_first), username)
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
            let set = world
                .contact_sets
                .friend_color(&f.id, |id| world.social.is_friend(id))
                .or_else(|| {
                    let first = world.contact_sets.friend_sets(&f.id).into_iter().next()?;
                    Some(world.contact_sets.set(&first)?.color)
                })
                .map(crate::ui::contacts::color32);
            Line {
                friend: f.clone(),
                name,
                full,
                username,
                status,
                set,
            }
        })
        .filter(|l| filter.is_empty() || [&l.name, &l.full, &l.username].iter().any(|t| t.to_lowercase().contains(filter)))
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
    let filter = if c.contact_settings.show_search {
        st.filter.trim().to_lowercase()
    } else {
        String::new()
    };
    if st.friend_set.as_deref().is_some_and(|s| !c.world.contact_sets.is_valid(s)) {
        st.friend_set = None;
    }
    let all = lines(c, &filter, st.friend_set.as_deref());
    let order: Vec<Uuid> = all.iter().map(|l| l.friend.id).collect();
    st.sel.retain(|id| order.contains(id));
    let sections = c.friends.sort == SORT_STATUS;
    let online = all.iter().filter(|l| l.friend.online).count();
    // its bar in a strip of its own, never over a row's right end
    widgets::reserved_scroll_bar(ui);
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

/// Search, "Add" (the friendship offer to a resident chosen in the picker)
/// and the view options.
fn toolbar(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi) {
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let gear = rows::gear(ui, c.p, aurora_i18n::tr!("people-display-options"));
        let (p, cs) = (c.p, &mut *c.contact_settings);
        menu::dropdown(&gear, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            crate::ui::contacts::options_menu(ui, p, cs, false);
            menu::separator(ui, p);
            menu::toggle(
                ui,
                p,
                "photo-filled",
                aurora_i18n::tr!("ui-contacts-show-profile-pictures-in-contact-set-lists"),
                &mut cs.set_list_icons,
            );
            menu::toggle(
                ui,
                p,
                "palette-filled",
                aurora_i18n::tr!("ui-contacts-friend-list-color-based-on-contact-set"),
                &mut cs.colorize_friends,
            );
        });
        let add = widgets::action_button(
            ui,
            c.p,
            "user-plus",
            aurora_i18n::tr!("common-add"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, rows::TOOLBAR_H),
        )
        .on_hover_text(aurora_i18n::tr!("people-friends-add"));
        if add.clicked() {
            c.contacts.pick_new_friend();
        }
        if c.contact_settings.show_search {
            let w = ui.available_width();
            widgets::search_box(ui, c.p, &mut st.filter, aurora_i18n::tr!("people-friends-filter"), w);
        }
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        label(ui, p, aurora_i18n::tr!("people-sort"));
        for (sort, key) in [
            (SORT_STATUS, "people-sort-online-first"),
            (SORT_NAME, "common-name"),
            (SORT_USERNAME, "common-username"),
        ] {
            if widgets::filter_pill(ui, p, "", aurora_i18n::text(key), c.friends.sort == sort).clicked() {
                c.friends.sort = sort;
            }
        }
        if sets.is_empty() {
            return;
        }
        ui.add_space(6.0);
        label(ui, p, aurora_i18n::tr!("people-set"));
        set_pills(ui, c, &sets, &mut st.friend_set);
    });
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click_and_drag());
    // dragged onto a conversation, a friend joins it
    resp.dnd_set_drag_payload(crate::ui::call_invite::PersonDrag(f.id));
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
    let left = rows::picture(ui, p, c.world, rect, data.pics.get(&f.id), f.id, &line.name, false);
    // the right end, from the right edge: received, granted, quick actions
    let buttons = hot.then(|| quick_actions(f));
    let show_rights = c.contact_settings.column_permissions;
    let mut widths = Vec::new();
    if show_rights {
        widths.extend([RECEIVED_W, GRANTED_W]);
    }
    if let Some(b) = &buttons {
        widths.push(rows::actions_width(b.len()));
    }
    let end = rows::end(rect, left, &widths, PARTS_GAP, false);
    let mut clicked_right = false;
    if show_rights {
        if let Some(cell) = end.items.first() {
            received(ui, p, f, *cell);
        }
        if let Some(cell) = end.items.get(1) {
            clicked_right = granted(ui, c, f, *cell);
        }
    }
    if let (Some(buttons), Some(slot)) = (&buttons, end.items.get(if show_rights { 2 } else { 0 })) {
        hover_actions(ui, c, st, f, *slot, buttons);
    }
    let color = if c.contact_settings.colorize_friends {
        line.set.unwrap_or(p.ink)
    } else {
        p.ink
    };
    let cut = rows::names(ui, p, rect, left, end.names_right, &line.name, color, &line.status, line.set, None);
    ui.set_opacity(opacity);
    if !clicked_right && rows::select(ui, &resp, &mut st.sel, &mut st.anchor, order, f.id) {
        super::nearby::open_sheet(st, f.id);
    }
    if resp.double_clicked() {
        c.actions.push(super::PeopleAction::OpenIm(f.id));
    }
    rows::person_menu(&resp, p, c.world, &st.sel, f.id);
    let on_names = resp.hover_pos().is_some_and(|at| at.x >= left && at.x <= end.names_right);
    if on_names && (cut || line.full != line.name) {
        resp.on_hover_text(&line.full);
    }
}

/// Profile, IM, a teleport offer to a friend online, and the whole menu.
fn quick_actions(f: &Friend) -> Vec<(&'static str, &'static str)> {
    let mut b = vec![
        ("user-filled", aurora_i18n::tr!("common-view-profile")),
        ("message-filled", aurora_i18n::tr!("ui-context-send-im")),
    ];
    if f.online {
        b.push(("send-filled", aurora_i18n::tr!("common-offer-teleport")));
    }
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
            "message-filled" if resp.clicked() => c.actions.push(super::PeopleAction::OpenIm(f.id)),
            "send-filled" if resp.clicked() => c.actions.push(super::PeopleAction::OfferTeleport(f.id)),
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
        let resp = ui.interact(cell, ui.id().with(("people-right", f.id, i)), Sense::click());
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
        ui.interact(cell, ui.id().with(("people-received", f.id, i)), Sense::hover())
            .on_hover_text(aurora_i18n::text(tip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
