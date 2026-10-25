//! People › Groups in Aurora's look (the People mock-up v2): our groups as
//! the contact list shows them (FSFloaterContacts' groups tab,
//! panel_fs_contacts_groups.xml: "none" on top, the active group, the
//! pinned ones first, the count against the account's limit), with a sheet
//! for the group clicked: chat, call, profile, invitation and its menu,
//! the active group and the title we wear, our settings in it (notices,
//! chat, profile listing, pinning: also in the group's profile), its SLurl
//! and leaving it. Nothing of the tab's buttons or menu is lost.

use super::rows::{self, RICH_GAP, RICH_H};
use super::sheet::{BUTTON_H, line, section};
use super::{Ctx, PeopleAction, PeopleUi};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::menu;
use crate::ui::widgets::{self, ActionKind};
use aurora_net::GroupMembership;
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// GP_SESSION_JOIN (roles_constants.h): the group's chat can be joined.
const GP_SESSION_JOIN: u64 = 1 << 16;
/// A badge's side on a row.
const BADGE: f32 = 14.0;

/// The groups tab: the list, and the sheet of the group clicked.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    super::split(ui, c, data, st, "people-groups-list", list, group_sheet);
}

fn list(ui: &mut egui::Ui, c: &mut Ctx, _data: &super::RadarData, st: &mut PeopleUi, beside_sheet: bool) {
    let p = c.p;
    if beside_sheet {
        ui.set_max_width(ui.max_rect().width() - 10.0);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let size = Vec2::new(0.0, rows::TOOLBAR_H);
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
        widgets::search_box(ui, p, &mut st.filter, aurora_i18n::tr!("people-groups-filter"), w);
    });
    ui.add_space(6.0);
    count_line(ui, c);
    ui.add_space(5.0);
    let filter = st.filter.trim().to_lowercase();
    let groups: Vec<GroupMembership> = c
        .world
        .groups
        .sorted()
        .into_iter()
        .filter(|g| filter.is_empty() || g.name.to_lowercase().contains(&filter))
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("people-groups-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if c.world.groups.groups.is_empty() {
                rows::empty(ui, p, aurora_i18n::tr!("ui-contacts-looking-for-a-group-to-join-try-search"));
                return;
            }
            // "none" on top when not filtering (LLGroupList::refresh)
            if filter.is_empty() {
                none_row(ui, c);
            } else if groups.is_empty() {
                rows::empty(
                    ui,
                    p,
                    &aurora_i18n::tr!("ui-contacts-no-group-containing-was-found", value0 = st.filter.trim().to_string()),
                );
            }
            for g in &groups {
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
/// our name. Activated from its menu or a double click.
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
    let tag = active.then(|| (aurora_i18n::tr!("people-groups-active-tag"), p.teal, p.teal.gamma_multiply(0.14)));
    rows::names(
        ui,
        p,
        rect,
        left,
        rect.right() - 8.0,
        aurora_i18n::tr!("people-groups-none"),
        p.ink,
        aurora_i18n::tr!("people-groups-none-sub"),
        None,
        tag,
    );
    ui.set_opacity(opacity);
    if resp.double_clicked() && !active {
        context::request(ui.ctx(), CtxAction::ActivateGroup(Uuid::nil()));
    }
    menu::context_menu(&resp, p, |ui| {
        if menu::item_if(ui, p, "circle-check-filled", aurora_i18n::tr!("common-enable"), !active) {
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

/// One of our groups: insignia, name and the "active" tag, our title, and
/// its badges (pinned, unread notices, chat blocked).
fn row(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, g: &GroupMembership) {
    let p = c.p;
    let world = c.world;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let selected = st.sheet == Some(g.id) && st.sheet_group;
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
    // the badges, from the right edge: chat blocked, notices, pinned
    let unread = world.group_data.unread(&g.id);
    let notices = (unread > 0).then(|| aurora_i18n::tr!("people-groups-notices", count = unread as i64));
    let notices_w = notices.as_ref().map(|t| tag_width(ui, t));
    let mut parts: Vec<(f32, Badge)> = Vec::new();
    if world.mutes.group_chat_muted(&g.id) {
        parts.push((BADGE, Badge::ChatOff));
    }
    if let (Some(text), Some(w)) = (&notices, notices_w) {
        parts.push((w, Badge::Notices(text.clone())));
    }
    if world.groups.favorites.contains(&g.id) {
        parts.push((BADGE, Badge::Pinned));
    }
    let widths: Vec<f32> = parts.iter().map(|(w, _)| *w).collect();
    let end = rows::end(rect, left, &widths, 8.0, true);
    let pointer = resp.hover_pos();
    let mut tip = None;
    for ((_, badge), cell) in parts.iter().zip(&end.items) {
        let text = match badge {
            Badge::ChatOff => {
                super::radar::icon(ui, "message-off", *cell, BADGE, p.muted_dim);
                aurora_i18n::tr!("ui-people-group-chat-blocked").to_owned()
            }
            Badge::Pinned => {
                super::radar::icon(ui, "pin-filled", *cell, 13.0, p.amber);
                aurora_i18n::tr!("people-groups-pinned").to_owned()
            }
            Badge::Notices(text) => {
                paint_tag(ui, p, *cell, text);
                text.clone()
            }
        };
        if pointer.is_some_and(|at| cell.expand(2.0).contains(at)) {
            tip = Some(text);
        }
    }
    if let Some((cell, n)) = end.more {
        rows::paint_more(ui, p, cell, n);
    }
    let active = world.groups.active == g.id;
    let tag = active.then(|| (aurora_i18n::tr!("people-groups-active-tag"), p.teal, p.teal.gamma_multiply(0.14)));
    let color = if g.list_in_profile { p.ink } else { p.ink.gamma_multiply(0.85) };
    rows::names(ui, p, rect, left, end.names_right, &g.name, color, &second_line(c, g), None, tag);
    if resp.clicked() {
        open_group_sheet(c, st, g.id);
    }
    if resp.double_clicked() {
        c.actions.push(PeopleAction::GroupChat(g.id));
    }
    // menu_people_groups.xml
    let (id, world) = (g.id, c.world);
    menu::context_menu(&resp, p, |ui| context::group_menu(ui, p, world, id));
    if let Some(tip) = tip {
        resp.on_hover_text(tip);
    }
}

enum Badge {
    ChatOff,
    Pinned,
    Notices(String),
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
    let cross = Rect::from_center_size(Pos2::new(rect.right() - 10.0, rect.center().y), Vec2::splat(20.0));
    let resp = ui.interact(cross, ui.scope_id().with(("people-group-sheet-close", g.id)), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(cross, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    super::radar::icon(ui, "x", cross, 14.0, if resp.hovered() { p.ink } else { p.muted });
    let left = tile.right() + 10.0;
    let width = (cross.left() - 8.0 - left).max(20.0);
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
    resp.on_hover_text(aurora_i18n::tr!("common-close")).clicked()
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
    let titles = world.group_data.get(&g.id).map(|d| d.titles.clone()).unwrap_or_default();
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
                };
                group_sheet(ui, &mut c, &data, &mut st, group);
            });
            assert!(bar.is_some(), "{height}: the content overflows, the bar shows");
            assert!(crossing.is_empty(), "{height}: the bar {bar:?} crosses {crossing:?}");
        }
    }
}
