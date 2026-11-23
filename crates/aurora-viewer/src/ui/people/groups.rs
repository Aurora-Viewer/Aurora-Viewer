//! People › Groups in Aurora's look (the People mock-up v2): our groups as
//! the contact list shows them (FSFloaterContacts' groups tab,
//! panel_fs_contacts_groups.xml: "none" on top, the active group, the
//! pinned ones first, the count against the account's limit), with a sheet
//! for the group clicked: chat, call, profile, invitation and its menu,
//! the active group and the title we wear, our settings in it (notices,
//! chat, profile listing, pinning: also in the group's profile), its SLurl
//! and leaving it. Nothing of the tab's buttons or menu is lost. Each row
//! also switches three of those settings in one click (#4714).

use super::rows::{self, RICH_GAP, RICH_H};
use super::sheet::{BUTTON_H, line, section};
use super::{Ctx, PeopleAction, PeopleUi};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::menu;
use crate::ui::widgets::{self, ActionKind};
use crate::world::groups::ChatRefusal;
use aurora_net::GroupMembership;
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// GP_SESSION_JOIN (roles_constants.h): the group's chat can be joined.
const GP_SESSION_JOIN: u64 = 1 << 16;
/// A badge's side on a row, a setting's icon too.
const BADGE: f32 = 14.0;
/// Between two parts of a row's right end.
const PART_GAP: f32 = 8.0;
/// Where a setting's icon takes the pointer: its cell and half the gap on
/// each side, so that a click between two icons still reaches one of them
/// rather than the row.
const SETTING_HIT: Vec2 = Vec2::new(BADGE + PART_GAP, 22.0);

/// The groups tab: the list, and the sheet of the group clicked.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    // Conversations › Contacts closes the sheet when the pane gets too narrow
    // for it; widened again, it shows the selected group, as the friends'.
    let shows_group = st.sheet.is_some() && st.sheet_group;
    if c.embedded_contacts
        && c.contact_sheet_on_click
        && !shows_group
        && let Some(id) = st.sel.first().copied().filter(|id| c.world.groups.group(id).is_some())
    {
        open_group_sheet(c, st, id);
    }
    super::split(ui, c, data, st, "people-groups-list", list, group_sheet);
}

fn list(ui: &mut egui::Ui, c: &mut Ctx, _data: &super::RadarData, st: &mut PeopleUi, beside_sheet: bool) {
    let p = c.p;
    if beside_sheet {
        ui.set_max_width(ui.max_rect().width() - 10.0);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    // in Conversations › Contacts, the button that expands the sheet ends the
    // toolbar while the sheet is collapsed or has no room, as on the friends
    // (`friends::sheet_toggle`; the sheet's head has the one that collapses it)
    let toggle = c.embedded_contacts && !c.contact_sheet_on_click;
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let size = Vec2::new(0.0, rows::TOOLBAR_H);
        // its place is kept first and the button drawn last, so that its
        // coming and going does not shift the search field's automatic ids
        let slot = toggle.then(|| {
            let right = ui.max_rect().right();
            let top = ui.max_rect().center().y - rows::TOOLBAR_H / 2.0;
            ui.add_space(rows::TOOLBAR_H + 6.0);
            Rect::from_min_size(Pos2::new(right - rows::TOOLBAR_H, top), Vec2::splat(rows::TOOLBAR_H))
        });
        if widgets::action_button(
            ui,
            p,
            "world-search",
            aurora_i18n::tr!("people-groups-join"),
            ActionKind::Flat,
            size,
        )
        .on_hover_text(aurora_i18n::tr!("people-groups-join-tip"))
        .clicked()
        {
            c.actions.push(PeopleAction::SearchGroups);
        }
        // LLPanelPeople's "+" › New group: the window, or
        // JoinedTooManyGroups at the limit (onGroupPlusButtonValidate)
        if widgets::action_button(ui, p, "plus", aurora_i18n::tr!("ui-contacts-create"), ActionKind::Flat, size)
            .on_hover_text(aurora_i18n::tr!("ui-group-create-create-tip"))
            .clicked()
        {
            crate::ui::group_profile::request_create(ui.ctx());
        }
        let w = ui.available_width();
        widgets::clearable_search_box(ui, p, &mut st.filter, aurora_i18n::tr!("people-groups-filter"), w);
        if let Some(slot) = slot {
            ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                super::friends::sheet_toggle(ui, c, true)
            });
        }
    });
    ui.add_space(6.0);
    count_line(ui, c);
    ui.add_space(5.0);
    let sorted = c.world.groups.sorted();
    let plan = crate::world::groups::plan_group_list(&sorted, &st.filter, |g| &g.name);
    egui::ScrollArea::vertical()
        .id_salt("people-groups-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if sorted.is_empty() {
                rows::empty(ui, p, aurora_i18n::tr!("ui-contacts-looking-for-a-group-to-join-try-search"));
                return;
            }
            // "None (no group tag)" on top, kept while filtering (#2788)
            if plan.show_none {
                none_row(ui, c);
            }
            if let Some(query) = &plan.no_match {
                rows::empty(
                    ui,
                    p,
                    &aurora_i18n::tr!("ui-contacts-no-group-containing-was-found", value0 = query.clone()),
                );
            }
            for &g in &plan.items {
                row(ui, c, st, g);
            }
        });
}

/// "You belong to 3 groups out of 42", the count in the ink.
fn count_line(ui: &mut egui::Ui, c: &Ctx) {
    let p = c.p;
    let count = c.world.groups.groups.len();
    let limit = crate::ui::contacts::group_limit(c.world);
    let n = aurora_i18n::integer(count as i64);
    let text = if limit > 0 {
        aurora_i18n::tr!("people-groups-count", count = count as i64, limit = limit as i64)
    } else {
        aurora_i18n::tr!("people-groups-count-unlimited", count = count as i64)
    };
    let mut job = egui::text::LayoutJob::default();
    let muted = egui::TextFormat::simple(FontId::proportional(12.0), p.muted);
    match text.find(&n) {
        Some(at) => {
            job.append(&text[..at], 0.0, muted.clone());
            job.append(&n, 0.0, rows::semibold(ui, 12.0, p.ink));
            job.append(&text[at + n.len()..], 0.0, muted);
        }
        None => job.append(&text, 0.0, muted),
    }
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(job);
    });
}

/// "No active group": the row of the nil group, its title gone from above
/// our name. Activated from its menu, a double click or its hover button.
fn none_row(ui: &mut egui::Ui, c: &mut Ctx) {
    let p = c.p;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let active = c.world.groups.active.is_nil();
    let hot = ui.rect_contains_pointer(rect);
    let opacity = ui.opacity();
    ui.set_opacity(opacity * 0.8);
    rows::face(ui, p, rect, false, hot);
    let left = rows::tile(ui, p, rect, None, "ban", "", p.raised);
    let (tag_ink, tag_fill) = widgets::active_tag_colors(p);
    let tag = active.then(|| (aurora_i18n::tr!("people-groups-active-tag"), tag_ink, tag_fill));
    let switch = hot.then(|| switch_width(ui, Uuid::nil(), c.world.groups.active)).flatten();
    rows::names(
        ui,
        p,
        rect,
        left,
        rect.right() - 8.0 - switch.map_or(0.0, |w| w + SWITCH_GAP),
        aurora_i18n::tr!("people-groups-none"),
        p.ink,
        aurora_i18n::tr!("people-groups-none-sub"),
        None,
        tag,
    );
    ui.set_opacity(opacity);
    if switch.is_some() {
        show_switch(ui, p, c.world, Uuid::nil(), rect.right() - 8.0, rect.center().y);
    }
    if resp.double_clicked() && !active {
        context::request(ui.ctx(), CtxAction::ActivateGroup(Uuid::nil()));
    }
    menu::context_menu(&resp, p, |ui| {
        if menu::item_if(ui, p, "circle-check-filled", aurora_i18n::tr!("common-activate"), !active) {
            context::request(ui.ctx(), CtxAction::ActivateGroup(Uuid::nil()));
        }
    });
}

/// The second line of a group's row: the title we wear in it (when known),
/// how many members are online (when its members are loaded), or that its
/// chat is blocked.
fn second_line(c: &Ctx, g: &GroupMembership) -> String {
    let world = c.world;
    let data = world.group_data.get(&g.id);
    let title = if world.groups.active == g.id && !world.groups.active_title.is_empty() {
        Some(world.groups.active_title.clone())
    } else {
        data.and_then(|d| d.selected_title())
            .map(|t| t.title.clone())
            .filter(|t| !t.is_empty())
    };
    let mut parts = vec![match title {
        Some(t) => aurora_i18n::tr!("people-groups-title", title = t),
        None => aurora_i18n::tr!("people-groups-member").to_owned(),
    }];
    if world.mutes.group_chat_muted(&g.id) {
        parts.push(aurora_i18n::tr!("people-groups-chat-off").to_owned());
    } else if let Some(d) = data.filter(|d| d.members_complete) {
        parts.push(aurora_i18n::tr!("people-groups-online", count = d.online().count() as i64));
    }
    parts.join(" · ")
}

/// One of our groups: insignia, name and the "active" tag, our title, its
/// badges (pinned, unread notices) and the switches of our settings in it
/// (notices, chat, profile listing). Hovered, it offers Activate, or
/// Deactivate for the active group.
fn row(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, g: &GroupMembership) {
    let p = c.p;
    let world = c.world;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click());
    widgets::reveal_search_selection(ui, &resp, st.sheet == Some(g.id) && st.sheet_group);
    if !ui.is_rect_visible(rect) {
        return;
    }
    // In Conversations › Contacts too narrow for the sheet beside the list,
    // a click only selects, as on the friends: the sheet would take the
    // whole pane before the second click of a double click reached the row
    // (it opens from the row's info button instead)
    let narrow_pane = c.embedded_contacts && !c.contact_sheet_on_click;
    let selected = (st.sheet == Some(g.id) && st.sheet_group) || (narrow_pane && st.sel.contains(&g.id));
    let hot = ui.rect_contains_pointer(rect) || st.demo_hover == Some(g.id);
    rows::face(ui, p, rect, selected, hot);
    let insignia = (!g.insignia.is_nil()).then_some(g.insignia);
    if let Some(i) = insignia {
        st.wanted_images.insert(i);
    }
    let left = rows::tile(
        ui,
        p,
        rect,
        insignia.and_then(|i| c.images.get(&i)),
        "",
        &rows::initials(&g.name),
        p.violet,
    );
    // The right end, from the edge: our settings, then the badges (unread
    // notices, pinned). Past them the names keep their minimum: the badges
    // fold into the "+n" first, then the settings, the profile listing
    // before the notices and the chat.
    let unread = world.group_data.unread(&g.id);
    let mut parts: Vec<(f32, Part)> = SETTINGS.iter().map(|s| (BADGE, Part::Setting(*s))).collect();
    if unread > 0 {
        let text = aurora_i18n::tr!("people-groups-notices", count = unread as i64);
        parts.push((tag_width(ui, &text), Part::Notices(text)));
    }
    if world.groups.favorites.contains(&g.id) {
        parts.push((BADGE, Part::Pinned));
    }
    let widths: Vec<f32> = parts.iter().map(|(w, _)| *w).collect();
    // The hovered row's Activate / Deactivate sits before them. Its room is
    // kept whether the row is hovered or not, so that nothing folds nor
    // moves as the pointer comes onto a setting.
    let switch_w = switch_width(ui, g.id, world.groups.active);
    let reserved = switch_w.map_or(0.0, |w| w + SWITCH_GAP);
    let end = rows::end(rect, left + reserved, &widths, PART_GAP, true);
    let switch = switch_w.filter(|_| hot);
    let names_right = end.names_right - if switch.is_some() { reserved } else { 0.0 };
    // the tooltips show under the pointer only: formatted for the hot row
    let tips: Vec<String> = if hot {
        parts.iter().map(|(_, part)| part.tip(world, g)).collect()
    } else {
        vec![String::new(); parts.len()]
    };
    let pointer = resp.hover_pos();
    let mut tip = None;
    let mut switched = false;
    for (((_, part), cell), text) in parts.iter().zip(&end.items).zip(&tips) {
        match part {
            Part::Setting(s) => {
                switched |= setting_switch(ui, c, g, *s, *cell, text);
                continue;
            }
            Part::Pinned => super::radar::icon(ui, "pin-filled", *cell, 13.0, p.amber),
            Part::Notices(label) => paint_tag(ui, p, *cell, label),
        }
        if pointer.is_some_and(|at| cell.expand(2.0).contains(at)) {
            tip = Some(text.clone());
        }
    }
    if let Some((cell, n)) = end.more {
        rows::paint_more(ui, p, cell, n);
        // what folded, settings included, with their states
        if pointer.is_some_and(|at| cell.contains(at)) {
            tip = Some(tips[end.items.len()..].join("\n"));
        }
    }
    #[cfg(test)]
    ui.ctx().data_mut(|d| {
        let settings = parts
            .iter()
            .zip(&end.items)
            .filter_map(|((_, part), cell)| match part {
                Part::Setting(s) => Some((*s, *cell)),
                _ => None,
            })
            .collect();
        let folded = end.more.map_or(0, |(_, n)| n);
        d.insert_temp(
            egui::Id::unique(("people-group-row-drawn", g.id)),
            Drawn {
                settings,
                names_right,
                folded,
            },
        );
    });
    let active = world.groups.active == g.id;
    let (tag_ink, tag_fill) = widgets::active_tag_colors(p);
    let tag = active.then(|| (aurora_i18n::tr!("people-groups-active-tag"), tag_ink, tag_fill));
    let color = if g.list_in_profile { p.ink } else { p.ink.gamma_multiply(0.85) };
    rows::names(ui, p, rect, left, names_right, &g.name, color, &second_line(c, g), None, tag);
    if switch.is_some() {
        show_switch(ui, p, world, g.id, end.names_right, rect.center().y);
    }
    if narrow_pane && hot {
        let slot = rows::actions_slot(rect, left, names_right, 1, rows::BUTTON);
        let tip = aurora_i18n::tr!("people-show-group-sheet");
        if rows::actions(ui, p, slot, &[("info-circle-filled", tip)], ("people-group-act", g.id))[0].clicked() {
            // a sheet the button collapsed is expanded again where there is
            // room, as a friend's (`friends::show_sheet`)
            if c.contact_sheet_room && c.friends.sheet_collapsed {
                c.friends.sheet_collapsed = false;
            }
            st.sel = vec![g.id];
            st.anchor = Some(g.id);
            open_group_sheet(c, st, g.id);
            st.sheet_grow = false;
        }
    }
    // A setting's switch lies over the row and takes its click: the row is
    // neither selected nor opened, as under the hovered row's button
    if resp.clicked() && !switched {
        st.sel = vec![g.id];
        st.anchor = Some(g.id);
        if !narrow_pane {
            open_group_sheet(c, st, g.id);
        }
    }
    if resp.double_clicked() && !switched {
        c.actions.push(PeopleAction::GroupChat(g.id));
    }
    // menu_people_groups.xml
    let (id, world) = (g.id, c.world);
    menu::context_menu(&resp, p, |ui| context::group_menu(ui, p, world, id));
    if let Some(tip) = tip {
        resp.on_hover_text(tip);
    }
}

/// What a group's row shows at its right end.
enum Part {
    Setting(Setting),
    Pinned,
    Notices(String),
}

impl Part {
    /// What the part says under the pointer (and in the "+n" once folded).
    fn tip(&self, world: &crate::world::World, g: &GroupMembership) -> String {
        match self {
            Part::Setting(s) => setting_tip(*s, setting_state(world, g, *s)),
            Part::Pinned => aurora_i18n::tr!("people-groups-pinned").to_owned(),
            Part::Notices(text) => text.clone(),
        }
    }
}

/// Our settings in a group that its row switches, an Aurora addition: the
/// Legacy Viewer's main group list has no such switch (its row,
/// panel_group_list_item.xml, opens the profile or the notices); the
/// settings are the group profile's check boxes (panel_group_general.xml:
/// `receive_notices`, `receive_chat`, `list_groups_in_profile`), and the
/// profile's own group list switches the listing from the row
/// (panel_group_list_item_short.xml `visibility_hide_btn` /
/// `visibility_show_btn`, LLGroupListItem::onVisibilityBtnClick →
/// LLAgent::setUserGroupFlags, indra/newview/llgrouplist.cpp). The same
/// messages as the sheet's switches, nothing new on the network.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Setting {
    /// The group chat (the block list's "Group:" entry).
    Chat,
    /// Its notices (SetGroupAcceptNotices' AcceptNotices).
    Notices,
    /// Listed in our profile (SetGroupAcceptNotices' ListInProfile).
    Profile,
}

/// The settings from the row's right edge, which is also the order they
/// fold last: the profile listing goes first, the chat stays longest
/// (the switches people use most, #4714). Left to right they read as the
/// sheet's: notices, then chat, after the profile listing.
const SETTINGS: [Setting; 3] = [Setting::Chat, Setting::Notices, Setting::Profile];

impl Setting {
    /// The sheet's icon of the setting.
    fn icon(self) -> &'static str {
        match self {
            Setting::Chat => "messages-filled",
            Setting::Notices => "bell-filled",
            Setting::Profile => "id-filled",
        }
    }

    /// The sheet's name of the setting.
    fn label(self) -> &'static str {
        match self {
            Setting::Chat => aurora_i18n::tr!("people-groups-receive-chat"),
            Setting::Notices => aurora_i18n::tr!("people-groups-receive-notices"),
            Setting::Profile => aurora_i18n::tr!("people-groups-in-profile"),
        }
    }
}

/// A setting of `g` as its switch shows it: on or off, and for the chat the
/// preference that refuses it whatever the group's own block.
fn setting_state(world: &crate::world::World, g: &GroupMembership, s: Setting) -> (bool, Option<ChatRefusal>) {
    match s {
        Setting::Chat => (!world.mutes.group_chat_muted(&g.id), world.groups.chat_refusal(g)),
        Setting::Notices => (g.accept_notices, None),
        Setting::Profile => (g.list_in_profile, None),
    }
}

/// Lit while on; dimmed while off, or while a preference refuses the chat.
fn setting_lit((on, refused): (bool, Option<ChatRefusal>)) -> bool {
    on && refused.is_none()
}

/// "Receive group notices: on", and for a refused chat the preference that
/// refuses it.
fn setting_tip(s: Setting, (on, refused): (bool, Option<ChatRefusal>)) -> String {
    let mut tip = if on {
        aurora_i18n::tr!("people-groups-switch-on", setting = s.label())
    } else {
        aurora_i18n::tr!("people-groups-switch-off", setting = s.label())
    };
    if let Some(refusal) = refused {
        let preference = match refusal {
            ChatRefusal::AllGroups => aurora_i18n::tr!("ui-options-no-group-chats"),
            ChatRefusal::WithoutNotices => aurora_i18n::tr!("ui-options-groups-without-notices"),
        };
        tip.push('\n');
        tip.push_str(&aurora_i18n::tr!("people-groups-chat-refused", preference = preference));
    }
    tip
}

/// The switch of a setting in `cell` of a group's row: the sheet's icon,
/// in the muted ink while lit, clearly dimmed otherwise. A click turns the
/// group's own setting over, as the sheet's switch (a refused chat's click
/// still blocks or unblocks the group), and does nothing else; true then.
fn setting_switch(ui: &egui::Ui, c: &mut Ctx, g: &GroupMembership, s: Setting, cell: Rect, tip: &str) -> bool {
    let p = c.p;
    let state = setting_state(c.world, g, s);
    let hit = Rect::from_center_size(cell.center(), SETTING_HIT);
    let resp = ui.interact(hit, ui.scope_id().with(("people-group-setting", g.id, s)), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(hit.shrink(1.0), widgets::inner_radius(ui.visuals()), p.raised.gamma_multiply(0.6));
    }
    let tint = if setting_lit(state) {
        p.muted
    } else {
        // dimmer than a friend's right not received (0.7): off at a glance
        p.muted_dim.gamma_multiply(0.5)
    };
    super::radar::icon(ui, s.icon(), cell, BADGE, tint);
    let resp = resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
    if !resp.clicked() {
        return false;
    }
    let on = state.0;
    match s {
        Setting::Chat => context::request(ui.ctx(), CtxAction::GroupChatBlocked(g.id, on)),
        Setting::Notices => c.actions.push(PeopleAction::GroupNotices(g.id, !on)),
        Setting::Profile => c.actions.push(PeopleAction::GroupFlags {
            group: g.id,
            notices: g.accept_notices,
            profile: !on,
        }),
    }
    true
}

/// What a group's row drew at its right end, for the tests.
#[cfg(test)]
#[derive(Clone, Debug)]
struct Drawn {
    /// The settings shown, each with its cell.
    settings: Vec<(Setting, Rect)>,
    /// Where the names stop.
    names_right: f32,
    /// How many parts the "+n" stands for.
    folded: usize,
}

/// What a hovered row offers in place, as the inventory's rows offer Add
/// and Take off: an Aurora shortcut to the menu's Activate. The Legacy
/// Viewer's list has none (LLGroupList, FSFloaterContacts: the Activate
/// button and menu item), and it sends the same ActivateGroup.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Switch {
    /// Make this group (or "No active group") the active one.
    Activate,
    /// Wear no group tag: what activating "No active group" does.
    Deactivate,
}

impl Switch {
    fn label(self) -> &'static str {
        match self {
            Switch::Activate => aurora_i18n::tr!("common-activate"),
            Switch::Deactivate => aurora_i18n::tr!("people-groups-deactivate"),
        }
    }
}

/// Between the hovered row's button and the names.
const SWITCH_GAP: f32 = 6.0;
/// The button's height, the inventory's quick buttons'.
const SWITCH_H: f32 = 22.0;

/// The button of the row of `row` (nil: "No active group") while `active`
/// is the active group, and the group it activates; none on the "No active
/// group" row when no group is active.
fn switch_for(row: Uuid, active: Uuid) -> Option<(Switch, Uuid)> {
    if row == active {
        (!row.is_nil()).then_some((Switch::Deactivate, Uuid::nil()))
    } else {
        Some((Switch::Activate, row))
    }
}

fn switch_width(ui: &egui::Ui, row: Uuid, active: Uuid) -> Option<f32> {
    switch_for(row, active).map(|(s, _)| widgets::action_button_fit(ui, "", s.label()))
}

/// Draw the hovered row's button ending at `right`, centred on `center_y`.
/// Greyed under RLVa @setgroup, as the menu's Activate
/// (LLGroupList::onContextMenuItemEnable, indra/newview/llgrouplist.cpp).
fn show_switch(ui: &mut egui::Ui, p: &Palette, world: &crate::world::World, row: Uuid, right: f32, center_y: f32) {
    let Some((switch, to)) = switch_for(row, world.groups.active) else {
        return;
    };
    let w = widgets::action_button_fit(ui, "", switch.label());
    let rect = Rect::from_min_size(Pos2::new(right - w, center_y - SWITCH_H / 2.0), Vec2::new(w, SWITCH_H));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique(("people-group-switch-rect", row)), rect));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("people-group-switch", row)));
    let enabled = world.rlva_can_change_group();
    if widgets::action_button_if(&mut child, p, "", switch.label(), ActionKind::Flat, rect.size(), enabled).clicked() {
        context::request(ui.ctx(), CtxAction::ActivateGroup(to));
    }
}

/// "2 notices" in a violet pill.
fn tag_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx())),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
        + 14.0
}

fn paint_tag(ui: &egui::Ui, p: &Palette, cell: Rect, text: &str) {
    let r = Rect::from_center_size(cell.center(), Vec2::new(cell.width(), 18.0));
    ui.painter().rect_filled(r, ui.visuals().window_corner_radius, p.violet);
    let g = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx())),
        p.on_accent,
    );
    let left = r.center().x - g.size().x / 2.0;
    crate::ui::widgets::paint_row_text(ui.painter(), g, left, r.center().y, p.on_accent);
}

/// Open the sheet of a group; its titles, members and notices are asked
/// for (LLGroupMgr, as the profile does).
pub(super) fn open_group_sheet(c: &mut Ctx, st: &mut PeopleUi, id: Uuid) {
    super::nearby::open_sheet(st, id);
    st.sheet_group = true;
    c.actions.push(PeopleAction::GroupData(id));
}

/// The sheet of a group.
fn group_sheet(ui: &mut egui::Ui, c: &mut Ctx, _data: &super::RadarData, st: &mut PeopleUi, id: Uuid) -> bool {
    let p = c.p;
    let world = c.world;
    let Some(g) = world.groups.group(&id).cloned() else {
        // left, or the list changed: nothing to show
        return true;
    };
    let (content, bottom) = super::sheet::frame(ui, p);
    // the head stays put above what scrolls, the bar in its own strip
    let (top, rest) = super::sheet::split_head(content);
    let closed = head(&mut super::sheet::child(ui, top, ("people-group-sheet-head", id)), c, st, &g);
    egui::ScrollArea::vertical()
        .id_salt(("people-group-sheet-scroll", id))
        .auto_shrink([false, false])
        .show(&mut super::sheet::child(ui, rest, ("people-group-sheet", id)), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            buttons(ui, c, &g);
            ui.add_space(12.0);
            me_in_group(ui, c, &g);
            ui.add_space(10.0);
            settings(ui, c, &g);
        });
    let mut foot = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("people-group-sheet-foot", id))
            .max_rect(bottom)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    foot.spacing_mut().item_spacing.x = 5.0;
    let size = Vec2::new(0.0, BUTTON_H);
    if widgets::action_button(
        &mut foot,
        p,
        "copy-filled",
        aurora_i18n::tr!("people-groups-slurl"),
        ActionKind::Flat,
        size,
    )
    .on_hover_text(aurora_i18n::tr!("common-copy-slurl-2"))
    .clicked()
    {
        foot.ctx().copy_text(format!("secondlife:///app/group/{id}/about"));
    }
    if widgets::action_button(
        &mut foot,
        p,
        "door-exit",
        aurora_i18n::tr!("people-groups-leave"),
        ActionKind::SoftDanger,
        size,
    )
    .clicked()
    {
        context::request(foot.ctx(), CtxAction::LeaveGroup(id));
    }
    closed
}

/// Insignia, name, what we are there, and the cross.
fn head(ui: &mut egui::Ui, c: &Ctx, st: &mut PeopleUi, g: &GroupMembership) -> bool {
    let p = c.p;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    let tile = Rect::from_center_size(Pos2::new(rect.left() + 20.0, rect.center().y), Vec2::splat(40.0));
    let insignia = (!g.insignia.is_nil()).then_some(g.insignia);
    if let Some(i) = insignia {
        st.wanted_images.insert(i);
    }
    rows::paint_tile(
        ui,
        p,
        tile,
        insignia.and_then(|i| c.images.get(&i)),
        "",
        &rows::initials(&g.name),
        p.violet,
    );
    // no cross where the sheet stays beside the list (as a friend's)
    let cross = Rect::from_center_size(Pos2::new(rect.right() - 10.0, rect.center().y), Vec2::splat(20.0));
    let resp = (!c.contact_sheet_on_click).then(|| {
        let resp = ui.interact(cross, ui.scope_id().with(("people-group-sheet-close", g.id)), Sense::click());
        if resp.hovered() {
            ui.painter()
                .rect_filled(cross, ui.visuals().widgets.inactive.corner_radius, p.raised);
        }
        super::radar::icon(ui, "x", cross, 14.0, if resp.hovered() { p.ink } else { p.muted });
        resp
    });
    let left = tile.right() + 10.0;
    let right = if resp.is_some() { cross.left() - 8.0 } else { rect.right() };
    let width = (right - left).max(20.0);
    let mut sub = Vec::new();
    if c.world.groups.active == g.id {
        sub.push(aurora_i18n::tr!("people-groups-active").to_owned());
    }
    let unread = c.world.group_data.unread(&g.id);
    if unread > 0 {
        sub.push(aurora_i18n::tr!("people-groups-unread", count = unread as i64));
    }
    if sub.is_empty() {
        sub.push(aurora_i18n::tr!("people-groups-member").to_owned());
    }
    let mut job = egui::text::LayoutJob::single_section(g.name.clone(), rows::semibold(ui, 15.0, p.ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let name_g = ui.painter().layout_job(job);
    let mut job = egui::text::LayoutJob::single_section(sub.join(" · "), egui::TextFormat::simple(FontId::proportional(12.0), p.muted));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let sub_g = ui.painter().layout_job(job);
    let top = rect.center().y - (name_g.size().y + sub_g.size().y) / 2.0;
    let name_h = name_g.size().y;
    ui.painter().galley(Pos2::new(left, top), name_g, p.ink);
    ui.painter().galley(Pos2::new(left, top + name_h), sub_g, p.muted);
    resp.is_some_and(|resp| resp.on_hover_text(aurora_i18n::tr!("common-close")).clicked())
}

/// The size of a sheet's button a little tighter than the theme's (icon,
/// gap, label and 9 points each side), so that the mock-up's four fit on
/// a line.
fn tight(ui: &egui::Ui, label: &str) -> Vec2 {
    let w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(12.5), egui::Color32::PLACEHOLDER)
        .size()
        .x;
    Vec2::new(15.0 + 7.0 + w + 18.0, BUTTON_H)
}

/// Chat, call, the profile, an invitation, and the group's whole menu.
fn buttons(ui: &mut egui::Ui, c: &mut Ctx, g: &GroupMembership) {
    let p = c.p;
    let world = c.world;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
        let chat = aurora_i18n::tr!("common-chat");
        if widgets::action_button_if(
            ui,
            p,
            "messages-filled",
            chat,
            ActionKind::Primary,
            tight(ui, chat),
            g.powers & GP_SESSION_JOIN != 0,
        )
        .clicked()
        {
            c.actions.push(PeopleAction::GroupChat(g.id));
        }
        let call = aurora_i18n::tr!("ui-profile-aurora-call");
        if widgets::action_button_if(
            ui,
            p,
            "phone-filled",
            call,
            ActionKind::Flat,
            tight(ui, call),
            world.calls.voice_enabled,
        )
        .clicked()
        {
            context::request(ui.ctx(), CtxAction::Call(g.id));
        }
        let info = aurora_i18n::tr!("people-groups-info");
        if widgets::action_button(ui, p, "info-circle-filled", info, ActionKind::Flat, tight(ui, info)).clicked() {
            crate::ui::group_profile::request_open(ui.ctx(), g.id);
        }
        let invite = aurora_i18n::tr!("people-groups-invite");
        if widgets::action_button(ui, p, "user-plus", invite, ActionKind::Flat, tight(ui, invite)).clicked() {
            c.actions.push(PeopleAction::InviteToGroup(g.id));
        }
        let more = widgets::action_button(ui, p, "dots-filled", "", ActionKind::Flat, Vec2::new(BUTTON_H + 4.0, BUTTON_H))
            .on_hover_text(aurora_i18n::tr!("ui-chat-more"));
        let id = g.id;
        menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            context::group_menu(ui, p, world, id);
        });
    });
}

/// "Me in this group": the active group, and the title we wear in it
/// (LLGroupMgr::sendGroupTitleUpdate).
fn me_in_group(ui: &mut egui::Ui, c: &mut Ctx, g: &GroupMembership) {
    let p = c.p;
    let world = c.world;
    section(ui, p, aurora_i18n::tr!("people-groups-me"));
    let mut active = world.groups.active == g.id;
    line(
        ui,
        p,
        "star-filled",
        aurora_i18n::tr!("people-groups-active-switch"),
        widgets::SWITCH_SIZE.x,
        |ui| {
            if widgets::switch(ui, p, &mut active).changed() {
                let to = if active { g.id } else { Uuid::nil() };
                context::request(ui.ctx(), CtxAction::ActivateGroup(to));
            }
        },
    );
    // Like the group profile's "Title shown" combo, this menu leaves a blank
    // title out (the Legacy Viewer never offers one); only the Group titles
    // window lists it (#3147).
    let titles: Vec<_> = world
        .group_data
        .get(&g.id)
        .map(|d| d.titles.iter().filter(|t| !t.title.is_empty()).cloned().collect())
        .unwrap_or_default();
    let current = titles
        .iter()
        .find(|t| t.selected)
        .map(|t| t.title.clone())
        .or_else(|| (active && !world.groups.active_title.is_empty()).then(|| world.groups.active_title.clone()))
        .unwrap_or_else(|| aurora_i18n::tr!("people-groups-member").to_owned());
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let w = ui.painter().layout_no_wrap(current.clone(), font, p.ink).size().x + 34.0;
    let mut chosen = None;
    line(ui, p, "tag-filled", aurora_i18n::tr!("people-groups-title-shown"), w, |ui| {
        let resp = choice_button(ui, p, &current, w, !titles.is_empty());
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            for t in &titles {
                if menu::check(ui, p, "tag-filled", &t.title, t.selected, true) {
                    chosen = Some(t.role);
                }
            }
        });
    });
    if let Some(role) = chosen {
        c.actions.push(PeopleAction::GroupTitle { group: g.id, role });
    }
}

/// A button showing a choice and its chevron (the title we wear).
fn choice_button(ui: &mut egui::Ui, p: &Palette, text: &str, width: f32, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let face = ui.style().interact(&resp).weak_bg_fill;
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
    let ink = if enabled { p.ink } else { p.muted };
    let g = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx())), ink);
    crate::ui::widgets::paint_row_text(ui.painter(), g, rect.left() + 9.0, rect.center().y, ink);
    super::radar::icon(
        ui,
        "chevron-down",
        Rect::from_center_size(Pos2::new(rect.right() - 12.0, rect.center().y), Vec2::splat(14.0)),
        12.0,
        p.muted,
    );
    resp
}

/// "Settings": our notices, the chat, the profile listing (the group
/// profile's own boxes, SetGroupAcceptNotices) and the pin.
fn settings(ui: &mut egui::Ui, c: &mut Ctx, g: &GroupMembership) {
    let p = c.p;
    let world = c.world;
    section(ui, p, aurora_i18n::tr!("people-groups-settings"));
    let mut notices = g.accept_notices;
    let mut chat = !world.mutes.group_chat_muted(&g.id);
    let mut profile = g.list_in_profile;
    let mut pinned = world.groups.favorites.contains(&g.id);
    let s = widgets::SWITCH_SIZE.x;
    let mut changed = None;
    line(ui, p, "bell-filled", aurora_i18n::tr!("people-groups-receive-notices"), s, |ui| {
        if widgets::switch(ui, p, &mut notices).changed() {
            changed = Some(PeopleAction::GroupNotices(g.id, notices));
        }
    });
    line(ui, p, "messages-filled", aurora_i18n::tr!("people-groups-receive-chat"), s, |ui| {
        if widgets::switch(ui, p, &mut chat).changed() {
            context::request(ui.ctx(), CtxAction::GroupChatBlocked(g.id, !chat));
        }
    });
    line(ui, p, "id-filled", aurora_i18n::tr!("people-groups-in-profile"), s, |ui| {
        if widgets::switch(ui, p, &mut profile).changed() {
            changed = Some(PeopleAction::GroupFlags {
                group: g.id,
                notices: g.accept_notices,
                profile,
            });
        }
    });
    line(ui, p, "pin-filled", aurora_i18n::tr!("people-groups-pin"), s, |ui| {
        if widgets::switch(ui, p, &mut pinned).changed() {
            context::request(ui.ctx(), CtxAction::PinGroup(g.id));
        }
    });
    c.actions.extend(changed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A group's sheet shorter than its content: the scroll bar shows in a
    /// strip of its own, crossing no switch, button, title nor the cross.
    #[test]
    fn the_scroll_bar_never_covers_a_groups_sheet() {
        let p = crate::theme::Theme::default().palette();
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let group = Uuid::from_u128(9);
        world.groups.groups.push(GroupMembership {
            id: group,
            name: "Aurora Builders".into(),
            insignia: Uuid::nil(),
            powers: GP_SESSION_JOIN,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        world.groups.active = group;
        world.groups.active_title = "Builder".into();
        let radar = crate::world::radar::Radar::default();
        let mut radar_settings = super::super::RadarSettings::default();
        let (voice, pics, images) = (HashMap::new(), HashMap::new(), HashMap::new());
        let data = super::super::RadarData {
            radar: &radar,
            settings: &mut radar_settings,
            voice: &voice,
            draw_distance: 64.0,
            second_life: true,
            now: 0.0,
            pics: &pics,
        };
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut friends = super::super::FriendsSettings::default();
        let mut actions = Vec::new();
        let mut st = PeopleUi::default();
        for height in [200.0, 280.0, 360.0] {
            let (bar, crossing) = super::super::sheet::scroll_bar_crossings(height, |ui| {
                let mut c = Ctx {
                    p: &p,
                    world: &world,
                    contacts: &mut contacts,
                    friends: &mut friends,
                    images: &images,
                    actions: &mut actions,
                    embedded_contacts: false,
                    contact_sheet_on_click: false,
                    contact_sheet_room: false,
                };
                group_sheet(ui, &mut c, &data, &mut st, group);
            });
            // (Japanese wraps fewer lines: its sheet fits in 360 points)
            assert!(bar.is_some() || height > 300.0, "{height}: the content overflows, the bar shows");
            assert!(crossing.is_empty(), "{height}: the bar {bar:?} crosses {crossing:?}");
        }
    }

    const G1: Uuid = Uuid::from_u128(11);
    const G2: Uuid = Uuid::from_u128(12);

    #[test]
    fn a_hovered_group_offers_activate_and_the_active_one_deactivate() {
        let none = Uuid::nil();
        assert_eq!(switch_for(G1, G2), Some((Switch::Activate, G1)));
        assert_eq!(switch_for(G1, none), Some((Switch::Activate, G1)));
        // Deactivate activates "No active group"
        assert_eq!(switch_for(G1, G1), Some((Switch::Deactivate, none)));
        assert_eq!(switch_for(none, G1), Some((Switch::Activate, none)));
        assert_eq!(switch_for(none, none), None, "no group is active: nothing to offer");
        assert_eq!(Switch::Activate.label(), aurora_i18n::tr!("common-activate"));
        assert_eq!(Switch::Deactivate.label(), aurora_i18n::tr!("people-groups-deactivate"));
    }

    fn membership(id: Uuid, name: &str) -> GroupMembership {
        GroupMembership {
            id,
            name: name.into(),
            insignia: Uuid::nil(),
            powers: GP_SESSION_JOIN,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    /// Draw the row of `group`, hovered or not, and click at `click` on the
    /// last frame: the button's rect, and what the row asked for.
    fn draw_row(world: &crate::world::World, group: Uuid, hovered: bool, click: Option<Pos2>) -> (Rect, Option<Rect>, Vec<CtxAction>) {
        let run = run_row(world, group, 360.0, hovered, click);
        (run.row, run.switch, run.asked)
    }

    /// What a row drew and asked for, and the list's state after it.
    struct Run {
        row: Rect,
        /// The hovered row's Activate / Deactivate.
        switch: Option<Rect>,
        drawn: Option<Drawn>,
        asked: Vec<CtxAction>,
        actions: Vec<PeopleAction>,
        st: PeopleUi,
    }

    /// `draw_row` in a list `width` wide, with everything it gives; the nil
    /// group draws the "No active group" row.
    fn run_row(world: &crate::world::World, group: Uuid, width: f32, hovered: bool, click: Option<Pos2>) -> Run {
        let p = crate::theme::Theme::default().palette();
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let images = HashMap::new();
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut friends = super::super::FriendsSettings::default();
        let mut actions = Vec::new();
        let mut st = PeopleUi {
            demo_hover: hovered.then_some(group),
            ..Default::default()
        };
        let g = (!group.is_nil()).then(|| {
            world
                .groups
                .groups
                .iter()
                .find(|g| g.id == group)
                .expect("a group of the test")
                .clone()
        });
        let area = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(width, 200.0));
        let mut row_rect = Rect::NOTHING;
        let frames = if click.is_some() { 4 } else { 2 };
        let mut asked = Vec::new();
        for frame in 0..frames {
            let events = match (click, frame) {
                (Some(at), f) if f == frames - 2 => vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                (Some(at), f) if f == frames - 1 => vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                _ => Vec::new(),
            };
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
                    let mut c = Ctx {
                        p: &p,
                        world,
                        contacts: &mut contacts,
                        friends: &mut friends,
                        images: &images,
                        actions: &mut actions,
                        embedded_contacts: false,
                        contact_sheet_on_click: false,
                        contact_sheet_room: false,
                    };
                    row_rect = Rect::from_min_size(child.cursor().min, Vec2::new(child.available_width(), RICH_H));
                    match &g {
                        Some(g) => row(&mut child, &mut c, &mut st, g),
                        None => none_row(&mut child, &mut c),
                    }
                },
            );
            out.textures_delta.clear();
            asked.extend(context::take_requests(&ctx));
        }
        let switch = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("people-group-switch-rect", group))));
        let drawn = ctx.data(|d| d.get_temp::<Drawn>(egui::Id::unique(("people-group-row-drawn", group))));
        Run {
            row: row_rect,
            switch,
            drawn,
            asked,
            actions,
            st,
        }
    }

    const G3: Uuid = Uuid::from_u128(13);

    /// Aurora Builders with every setting on and pinned; the Night Owls with
    /// every setting off (notices, profile listing, its chat blocked).
    fn settings_world() -> crate::world::World {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.groups.groups.push(membership(G1, "Aurora Builders"));
        world.groups.groups.push(GroupMembership {
            accept_notices: false,
            list_in_profile: false,
            ..membership(G2, "Night Owls")
        });
        world.groups.favorites.insert(G1);
        world.set_group_chat_blocked(G2, true);
        assert!(world.mutes.group_chat_muted(&G2), "the test's block");
        world
    }

    /// What a click on a switch asked for: the people action's and the
    /// context action's settings, as (setting, value).
    fn switched(run: &Run, group: Uuid) -> Vec<(Setting, bool)> {
        let mut out = Vec::new();
        for a in &run.actions {
            match a {
                PeopleAction::GroupNotices(id, on) if *id == group => out.push((Setting::Notices, *on)),
                PeopleAction::GroupFlags {
                    group: id,
                    notices,
                    profile,
                } if *id == group => {
                    out.push((Setting::Profile, *profile));
                    // the notices travel as they are
                    out.push((Setting::Notices, *notices));
                }
                _ => {}
            }
        }
        for a in &run.asked {
            if let CtxAction::GroupChatBlocked(id, blocked) = a
                && *id == group
            {
                out.push((Setting::Chat, !blocked));
            }
        }
        out
    }

    /// A click on each switch turns its setting over, as the sheet's switch
    /// would, and only that: the row is neither selected nor opened, the
    /// group's chat does not open. A click on the names still opens it.
    #[test]
    fn each_setting_switches_from_its_row_without_selecting_or_opening_it() {
        let world = settings_world();
        for (group, on) in [(G1, true), (G2, false)] {
            let drawn = run_row(&world, group, 360.0, false, None).drawn.expect("the row's end");
            let shown: Vec<Setting> = drawn.settings.iter().map(|(s, _)| *s).collect();
            assert_eq!(shown, SETTINGS, "{group}: every setting, nothing folded");
            for (s, cell) in &drawn.settings {
                let run = run_row(&world, group, 360.0, false, Some(cell.center()));
                let mut expected = vec![(*s, !on)];
                if *s == Setting::Profile {
                    // SetGroupAcceptNotices carries the notices unchanged
                    expected.push((Setting::Notices, on));
                }
                assert_eq!(switched(&run, group), expected, "{group} {s:?}");
                assert!(run.st.sel.is_empty(), "{group} {s:?}: selected {:?}", run.st.sel);
                assert_eq!(run.st.sheet, None, "{group} {s:?}: the sheet opened");
                assert!(
                    !run.actions
                        .iter()
                        .any(|a| matches!(a, PeopleAction::GroupData(_) | PeopleAction::GroupChat(_))),
                    "{group} {s:?}: the row acted too"
                );
                assert!(
                    !run.asked.iter().any(|a| matches!(a, CtxAction::ActivateGroup(_))),
                    "{group} {s:?}: {:?}",
                    run.asked
                );
            }
        }
        // the control: the names are the row's, a click there opens it
        let row = run_row(&world, G1, 360.0, false, None).row;
        let run = run_row(
            &world,
            G1,
            360.0,
            false,
            Some(Pos2::new(rows::names_left(row) + 20.0, row.center().y)),
        );
        assert_eq!(run.st.sel, vec![G1]);
        assert_eq!(run.st.sheet, Some(G1));
        assert!(switched(&run, G1).is_empty());
    }

    /// "No active group" switches nothing: no setting there, and a click
    /// where a group's switch stands asks for none.
    #[test]
    fn the_none_row_has_no_setting_switches() {
        let world = settings_world();
        let group = run_row(&world, G1, 360.0, true, None).drawn.expect("a group's row");
        assert_eq!(group.settings.len(), 3, "a group's row has them");
        let none = run_row(&world, Uuid::nil(), 360.0, true, None);
        assert!(none.drawn.is_none(), "{:?}", none.drawn);
        for (_, cell) in &group.settings {
            let run = run_row(&world, Uuid::nil(), 360.0, true, Some(cell.center()));
            assert!(run.actions.is_empty(), "the None row acted");
            assert!(
                !run.asked.iter().any(|a| matches!(a, CtxAction::GroupChatBlocked(..))),
                "{:?}",
                run.asked
            );
        }
    }

    /// "No group chats" refuses every group's chat, "Groups without
    /// notices" the chat of a group whose notices are off: the chat switch
    /// shows off then, its tooltip names the preference, and a click still
    /// blocks or unblocks the group itself.
    #[test]
    fn the_chat_switch_shows_off_under_either_refusing_preference() {
        let mut world = settings_world();
        world.groups.groups.push(GroupMembership {
            accept_notices: false,
            ..membership(G3, "Quiet Corner")
        });
        let g = |world: &crate::world::World, id: Uuid| world.groups.group(&id).expect("a test group").clone();
        let chat = |world: &crate::world::World, id: Uuid| setting_state(world, &g(world, id), Setting::Chat);
        assert_eq!(chat(&world, G1), (true, None));
        assert!(setting_lit(chat(&world, G1)), "on, nothing refuses it");
        assert!(!setting_lit(chat(&world, G2)), "blocked");
        assert!(setting_lit(chat(&world, G3)), "the notices alone do not refuse it");

        let no_chats = aurora_i18n::tr!("ui-options-no-group-chats");
        let without_notices = aurora_i18n::tr!("ui-options-groups-without-notices");
        world.groups.mute_all_groups = true;
        for id in [G1, G2, G3] {
            let state = chat(&world, id);
            assert_eq!(state.1, Some(ChatRefusal::AllGroups), "{id}");
            assert!(!setting_lit(state), "{id}: refused, shown off");
            let tip = setting_tip(Setting::Chat, state);
            assert!(tip.contains(no_chats), "{id}: {tip}");
            // the group's own switch, as a click would turn it over
            let own = aurora_i18n::tr!("people-groups-switch-on", setting = Setting::Chat.label());
            assert_eq!(tip.starts_with(&own), state.0, "{id}: {tip}");
        }
        world.groups.mute_all_groups = false;
        world.groups.mute_when_notices_off = true;
        assert_eq!(chat(&world, G1), (true, None), "its notices are on");
        assert!(setting_lit(chat(&world, G1)));
        let state = chat(&world, G3);
        assert_eq!(state, (true, Some(ChatRefusal::WithoutNotices)));
        assert!(!setting_lit(state));
        let tip = setting_tip(Setting::Chat, state);
        assert!(tip.contains(without_notices) && !tip.contains(no_chats), "{tip}");
        // the other switches keep their own state
        let notices = setting_state(&world, &g(&world, G3), Setting::Notices);
        assert_eq!(notices, (false, None));

        // drawn: the click blocks the group's own chat, still refused
        let drawn = run_row(&world, G3, 360.0, false, None).drawn.expect("the row's end");
        let cell = drawn.settings.iter().find(|(s, _)| *s == Setting::Chat).expect("the chat switch").1;
        let run = run_row(&world, G3, 360.0, false, Some(cell.center()));
        assert_eq!(switched(&run, G3), vec![(Setting::Chat, false)], "{:?}", run.asked);
        assert_eq!(run.st.sheet, None);
    }

    /// A row too narrow for every part keeps the names' minimum: the badges
    /// fold into the "+n" first, then the settings (the profile listing,
    /// then the notices), never over the names; and the pointer coming
    /// onto the row moves and folds nothing.
    #[test]
    fn the_switches_fold_rather_than_cover_the_names() {
        let world = settings_world();
        let wide = run_row(&world, G1, 360.0, true, None);
        let reserved = wide.switch.expect("the hovered row's button").width() + SWITCH_GAP;
        // room for one switch and the "+n" past the names' minimum
        let narrow = 8.0 + (rows::names_left(Rect::ZERO) + rows::NAMES_MIN + 6.0) + reserved + 50.0;
        for (width, shown, folded) in [(360.0, 3, 0), (narrow, 1, 3)] {
            let calm = run_row(&world, G1, width, false, None);
            let hot = run_row(&world, G1, width, true, None);
            let (a, b) = (calm.drawn.expect("drawn"), hot.drawn.expect("drawn"));
            assert_eq!(a.settings.len(), shown, "{width}: {a:?}");
            assert_eq!(a.folded, folded, "{width}: {a:?}");
            let order: Vec<Setting> = a.settings.iter().map(|(s, _)| *s).collect();
            assert_eq!(order, SETTINGS[..shown], "{width}: the chat folds last");
            assert_eq!(
                a.settings.iter().map(|(_, r)| *r).collect::<Vec<_>>(),
                b.settings.iter().map(|(_, r)| *r).collect::<Vec<_>>(),
                "{width}: the hover moves nothing"
            );
            assert_eq!(a.folded, b.folded, "{width}: the hover folds nothing");
            let names_left = rows::names_left(calm.row);
            for (drawn, state) in [(&a, "calm"), (&b, "hovered")] {
                assert!(
                    drawn.names_right - names_left >= rows::NAMES_MIN - 0.5,
                    "{width} {state}: the names keep their minimum ({} to {})",
                    names_left,
                    drawn.names_right
                );
                for (s, cell) in &drawn.settings {
                    assert!(cell.left() >= drawn.names_right, "{width} {state}: {s:?} {cell:?} over the names");
                    assert!(calm.row.contains_rect(*cell), "{width} {state}: {s:?} {cell:?} out of the row");
                }
            }
            let button = hot.switch.expect("the hovered row's button");
            let first = b.settings.iter().map(|(_, r)| r.left()).fold(f32::INFINITY, f32::min);
            assert!(button.right() < first, "{width}: {button:?} before the switches");
            assert!(button.left() > b.names_right, "{width}: {button:?} after the names");
        }
    }

    /// The hovered row shows its button inside the row, before the pinned
    /// badge; a click on it activates the group, or none for the active one.
    #[test]
    fn the_hover_button_activates_and_deactivates_a_group() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.groups.groups.push(membership(G1, "Aurora Builders"));
        world.groups.groups.push(membership(G2, "Night Owls"));
        world.groups.favorites.insert(G1);
        world.groups.active = G2;

        let (_, none, _) = draw_row(&world, G1, false, None);
        assert!(none.is_none(), "no button until the row is hovered");

        let (row, button, _) = draw_row(&world, G1, true, None);
        let button = button.expect("the hovered row's button");
        assert!(row.contains_rect(button), "{button:?} in {row:?}");
        assert!(
            button.right() <= row.right() - 8.0 - BADGE,
            "{button:?} leaves the pinned badge its place"
        );

        let (_, _, asked) = draw_row(&world, G1, true, Some(button.center()));
        assert!(
            asked.iter().any(|a| matches!(a, CtxAction::ActivateGroup(id) if *id == G1)),
            "Activate: {asked:?}"
        );

        let (_, button, _) = draw_row(&world, G2, true, None);
        let button = button.expect("the active group's button");
        let (_, _, asked) = draw_row(&world, G2, true, Some(button.center()));
        assert!(
            asked.iter().any(|a| matches!(a, CtxAction::ActivateGroup(id) if id.is_nil())),
            "Deactivate: {asked:?}"
        );

        // RLVa @setgroup: greyed, as the menu's Activate
        world.agent_id = Uuid::from_u128(1);
        world.rlva_configure(true);
        world.rlva_console_input("@setgroup=n");
        assert!(!world.rlva_can_change_group(), "@setgroup holds");
        let (_, button, _) = draw_row(&world, G1, true, None);
        let button = button.expect("still shown");
        let (_, _, asked) = draw_row(&world, G1, true, Some(button.center()));
        assert!(
            !asked.iter().any(|a| matches!(a, CtxAction::ActivateGroup(_))),
            "@setgroup: {asked:?}"
        );
    }
}
