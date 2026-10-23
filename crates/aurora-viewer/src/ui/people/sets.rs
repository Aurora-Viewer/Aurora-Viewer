//! People › Contact sets in Aurora's look (the People mock-up v2): the
//! contact sets of Conversations › Contacts (FSPanelContactSets,
//! fspanelcontactsets.cpp, panel_fs_contacts_sets.xml, LGGContactSets) with
//! the sets as colored pills, "New", and a gear for the set's color, name
//! and deletion (FSFloaterContactSetConfiguration), the pseudo-sets (all,
//! none, with a nickname, non-friends) and the list's options. The members
//! are rich rows; under the pointer a row offers to move the resident to
//! another set, to take them out of this one (confirmed) and its menu.
//! Non-friends stay in their sets. Nicknames are in the sheet.

use super::rows::{self, RICH_GAP, RICH_H};
use super::{Ctx, PeopleUi};
use crate::ui::context::{self, AvatarList};
use crate::ui::{menu, widgets};
use crate::world::contact_sets::{self, ContactSets, DN_REMOVED};
use egui::{Rect, Sense, Vec2};
use uuid::Uuid;

/// The contact sets tab: the list, and the sheet of the person clicked.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    super::split(ui, c, data, st, "people-sets-list", list, super::person_sheet);
}

/// The set shown: the one chosen while it exists, else the first set, else
/// all of them (FSPanelContactSets::refreshSetList).
fn choice(c: &mut Ctx) -> String {
    let names = c.world.contact_sets.set_names();
    let choice = match &c.contacts.set_choice {
        Some(s) if ContactSets::is_internal(s) || c.world.contact_sets.is_valid(s) => s.clone(),
        _ => names.first().cloned().unwrap_or_else(|| contact_sets::ALL_SETS.to_owned()),
    };
    c.contacts.set_choice = Some(choice.clone());
    choice
}

fn list(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, beside_sheet: bool) {
    let p = c.p;
    if beside_sheet {
        ui.set_max_width(ui.max_rect().width() - 10.0);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let set = choice(c);
    let mutable = !ContactSets::is_internal(&set);
    let world = c.world;
    let friends: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let is_friend = |id: &Uuid| world.social.is_friend(id);
    set_pills(ui, c, &set);
    ui.add_space(6.0);
    // search, and "Add" (residents chosen in the picker)
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let add = widgets::action_button_if(
            ui,
            p,
            "user-plus",
            aurora_i18n::tr!("common-add"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, rows::TOOLBAR_H),
            mutable,
        )
        .on_hover_text(aurora_i18n::tr!("ui-contacts-add-a-resident-to-the-selected-contact-set"));
        if add.clicked() {
            c.contacts.pick_for_set(&set);
        }
        let w = ui.available_width();
        widgets::search_box(ui, p, &mut st.filter, aurora_i18n::tr!("ui-contacts-filter-people"), w);
    });
    ui.add_space(7.0);
    let names = &world.social.avatar_names;
    let o = names.options;
    let name_of = |id: &Uuid| {
        names
            .get(id)
            .map(|n| n.complete(&o))
            .or_else(|| world.social.names.get(id).cloned())
            .unwrap_or_else(|| "…".into())
    };
    let online = |id: &Uuid| world.social.friends.iter().any(|f| f.id == *id && f.online);
    let filter = st.filter.trim().to_lowercase();
    let mut members = world.contact_sets.members(&set, &friends, is_friend);
    members.retain(|id| filter.is_empty() || name_of(id).to_lowercase().contains(&filter));
    // FSPanelContactSets::updateAvatarListSorting
    if world.contact_sets.sorts_by_online(&set) {
        members.sort_by_cached_key(|id| (!online(id), name_of(id).to_lowercase()));
    } else {
        members.sort_by_cached_key(|id| name_of(id).to_lowercase());
    }
    st.sel.retain(|id| members.contains(id));
    let area = egui::ScrollArea::vertical()
        .id_salt("people-sets-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if members.is_empty() {
                rows::empty(ui, p, aurora_i18n::tr!("ui-contacts-this-contact-set-is-empty"));
            }
            for id in &members {
                row(ui, c, data, st, &set, *id, &rows::display_name(world, id), &members);
            }
        });
    let drop = ui.interact(area.inner_rect, ui.scope_id().with("contact_set_drop"), Sense::hover());
    crate::ui::contacts::drag::set_target(&drop, &set);
}

/// The sets as pills with their member count (the chosen one lit), "New"
/// and the gear; the sets that do not fit are in a "+2".
fn set_pills(ui: &mut egui::Ui, c: &mut Ctx, set: &str) {
    let p = c.p;
    let world = c.world;
    let names = world.contact_sets.set_names();
    let friends: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let count = |s: &str| world.contact_sets.members(s, &friends, |id| world.social.is_friend(id)).len();
    let new_w = widgets::filter_pill_width(ui, "plus", aurora_i18n::tr!("people-sets-new"));
    let more_w = widgets::filter_pill_width(ui, "", "+00");
    let mut chosen: Option<String> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        // what "New" and the gear keep at the end of the line
        let mut room = ui.available_width() - new_w - 5.0 - (rows::TOOLBAR_H - 2.0) - 5.0;
        for (i, name) in names.iter().enumerate() {
            let n = aurora_i18n::integer(count(name) as i64);
            let w = widgets::dot_pill_width(ui, name, Some(&n), false);
            let rest = names.len() - i - 1;
            if w + if rest > 0 { 5.0 + more_w } else { 0.0 } > room {
                let more = widgets::filter_pill(ui, p, "", &format!("+{}", names.len() - i), false);
                menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    for s in &names[i..] {
                        if menu::check(ui, p, "users-group", s, s == set, true) {
                            chosen = Some(s.clone());
                        }
                    }
                });
                break;
            }
            let color = world
                .contact_sets
                .set(name)
                .map_or(p.muted, |s| crate::ui::contacts::color32(s.color));
            let response = widgets::dot_pill(ui, p, color, name, Some(&n), name == set, false).0;
            crate::ui::contacts::drag::set_target(&response, name);
            if response.clicked() {
                chosen = Some(name.clone());
            }
            room -= w + 5.0;
        }
        if widgets::filter_pill(ui, p, "plus", aurora_i18n::tr!("people-sets-new"), false)
            .on_hover_text(aurora_i18n::tr!("ui-contacts-add-a-new-contact-set"))
            .clicked()
        {
            c.contacts.ask_new_set();
        }
        let gear = rows::gear(ui, p, aurora_i18n::tr!("people-sets-gear"));
        let mutable = !ContactSets::is_internal(set);
        let (contacts, cs) = (&mut *c.contacts, &mut *c.contact_settings);
        menu::dropdown(&gear, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            if menu::item_if(
                ui,
                p,
                "adjustments-filled",
                aurora_i18n::tr!("ui-contacts-configure-selected-contact-set"),
                mutable,
            ) {
                contacts.configure_set(set);
            }
            if menu::item_if(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("ui-contacts-delete-selected-contact-set"),
                mutable,
            ) {
                contacts.ask_remove_set(set);
            }
            menu::separator(ui, p);
            // the pseudo-sets of the contact sets' list
            for s in [
                contact_sets::ALL_SETS,
                contact_sets::NO_SETS,
                contact_sets::PSEUDONYM,
                contact_sets::EXTRA_AVS,
            ] {
                if menu::check(ui, p, "users", &crate::ui::contacts::set_label(s), s == set, true) {
                    chosen = Some(s.to_owned());
                }
            }
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
    });
    if let Some(s) = chosen {
        c.contacts.set_choice = Some(s);
    }
}

/// The second line of a member: their nickname's real name, "not a
/// friend", or their legacy name.
fn second_line(c: &Ctx, id: &Uuid) -> String {
    let world = c.world;
    let legacy = world
        .social
        .avatar_names
        .get(id)
        .map(|n| crate::world::social::format_name(&n.legacy_first, &n.legacy_last))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| world.social.name_of(id));
    if world.contact_sets.pseudonym(id).is_some_and(|a| a != DN_REMOVED) {
        aurora_i18n::tr!("people-sets-nickname-of", name = legacy)
    } else if !world.social.is_friend(id) {
        aurora_i18n::tr!("people-sets-non-friend").to_owned()
    } else {
        legacy
    }
}

/// One member: picture and status, names, the friend's heart, or the
/// quick actions under the pointer.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, set: &str, id: Uuid, name: &str, order: &[Uuid]) {
    let p = c.p;
    let world = c.world;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click_and_drag());
    resp.dnd_set_drag_payload(crate::ui::call_invite::PersonDrag(id));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let icons = c.contact_settings.set_list_icons;
    if icons && !data.pics.contains_key(&id) {
        st.want_pic(id, ui.input(|i| i.time));
    }
    let selected = st.sel.contains(&id) || st.sheet == Some(id);
    let hot = (ui.rect_contains_pointer(rect) && !resp.dragged()) || st.row_menu == Some(id) || st.demo_hover == Some(id);
    rows::face(ui, p, rect, selected, hot);
    let pic = if icons { data.pics.get(&id) } else { None };
    let left = rows::picture(ui, p, world, rect, pic, id, name, false);
    let friend = world.social.is_friend(&id);
    let mutable = !ContactSets::is_internal(set);
    // under the pointer: move, take out, the menu; else the friend's heart
    let buttons: Option<Vec<(&str, &str)>> = hot.then(|| {
        let mut b = Vec::new();
        if mutable {
            b.push((
                "arrows-exchange",
                aurora_i18n::tr!("ui-contacts-move-this-resident-to-another-contact-set"),
            ));
            b.push((
                "circle-minus",
                aurora_i18n::tr!("ui-contacts-remove-this-resident-from-the-selected-contact-set"),
            ));
        }
        b.push(("dots-filled", aurora_i18n::tr!("ui-chat-more")));
        b
    });
    let widths: Vec<f32> = match &buttons {
        Some(b) => vec![rows::actions_width(b.len())],
        None if friend => vec![13.0],
        None => Vec::new(),
    };
    let end = rows::end(rect, left, &widths, 8.0, false);
    let mut tip = None;
    match (&buttons, end.items.first()) {
        (Some(buttons), Some(slot)) => hover_actions(ui, c, st, set, id, *slot, buttons),
        (None, Some(cell)) => {
            super::radar::icon(ui, "heart-filled", *cell, 13.0, p.success);
            if resp.hover_pos().is_some_and(|at| cell.expand(2.0).contains(at)) {
                tip = Some(aurora_i18n::tr!("ui-profile-aurora-friend").to_owned());
            }
        }
        _ => {}
    }
    let color = if c.contact_settings.colorize_friends {
        world
            .contact_sets
            .friend_color(&id, |x| world.social.is_friend(x))
            .map_or(p.ink, crate::ui::contacts::color32)
    } else {
        p.ink
    };
    let color = crate::ui::colors::resident_name(world, &id, color);
    let cut = rows::names(ui, p, rect, left, end.names_right, name, color, &second_line(c, &id), None, None);
    if rows::select(ui, &resp, &mut st.sel, &mut st.anchor, order, id) {
        super::nearby::open_sheet(st, id);
    }
    if resp.double_clicked() {
        c.actions.push(super::PeopleAction::OpenIm(id));
    }
    rows::person_menu(&resp, p, world, &st.sel, id);
    if let Some(tip) = tip.or_else(|| (cut && resp.hover_pos().is_some_and(|at| at.x <= end.names_right)).then(|| name.to_owned())) {
        resp.on_hover_text(tip);
    }
}

/// The residents a row's action applies to: the selection when the row is
/// part of it, else the row alone.
fn targets(st: &PeopleUi, id: Uuid) -> Vec<Uuid> {
    if st.sel.contains(&id) { st.sel.clone() } else { vec![id] }
}

fn hover_actions(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, set: &str, id: Uuid, slot: Rect, buttons: &[(&str, &str)]) {
    let p = c.p;
    let world = c.world;
    let mut menu_open = false;
    for ((icon, _), resp) in buttons.iter().zip(rows::actions(ui, p, slot, buttons, ("people-set-act", id))) {
        match *icon {
            "arrows-exchange" => {
                let ids = targets(st, id);
                let others: Vec<String> = world.contact_sets.set_names().into_iter().filter(|s| s != set).collect();
                menu_open |= menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    if others.is_empty() {
                        menu::item_if(ui, p, "users-group", aurora_i18n::tr!("ui-contacts-no-other-contact-sets"), false);
                    }
                    for to in &others {
                        if menu::item(ui, p, "users-group", to) {
                            context::request(
                                ui.ctx(),
                                context::CtxAction::ContactSet(crate::ui::contacts::SetEdit::Move {
                                    from: set.to_owned(),
                                    to: to.clone(),
                                    ids: ids.clone(),
                                }),
                            );
                        }
                    }
                });
            }
            "circle-minus" if resp.clicked() => c.contacts.ask_remove_from_set(set, targets(st, id)),
            "dots-filled" => {
                let kind = if world.social.is_friend(&id) {
                    AvatarList::Friend
                } else {
                    AvatarList::Name
                };
                menu_open |= menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    context::avatar_list_menu(ui, p, world, id, kind);
                });
            }
            _ => {}
        }
    }
    super::keep_row_menu(st, id, menu_open);
}
