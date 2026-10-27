//! The head of a conversation: who it is with (picture, status, names) and
//! what can be done with them. The actions of the toolbar of Firestorm's
//! IM panel (FSFloaterIM: profile, teleport, call, block…) are kept: the
//! frequent ones as buttons, the others in its menu.

use super::{ChatUi, ConvAction, SNOOZE_ICON, close_tip, find, presence, tool};
use crate::theme::Palette;
use crate::ui::context::CtxAction;
use crate::ui::menu;
use crate::world::World;
use egui::RichText;
use std::collections::HashMap;

/// Side of the picture of the head.
const PIC: f32 = 40.0;
/// Width of one action button (`tool`) and the spacing after it.
const TOOL_W: f32 = 28.0;

/// The pill of a status that is not the usual one.
fn status_pill(ui: &mut egui::Ui, p: &Palette, status: presence::Status) {
    let color = status.color(p);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(status.label()).size(11.0).strong().color(color));
        });
}

/// What the head says of the conversation.
struct Identity {
    title: String,
    subtitle: Option<String>,
    /// The avatar of a conversation with one resident.
    avatar: Option<uuid::Uuid>,
    /// A Tabler icon instead of a picture (the nearby chat).
    icon: Option<&'static str>,
}

fn identity(world: &World, st: &ChatUi) -> Identity {
    match st.selected {
        None => Identity {
            title: aurora_i18n::tr!("common-nearby-chat").to_owned(),
            subtitle: Some(world.rlva_region_label()).filter(|r| !r.is_empty()),
            avatar: None,
            icon: Some("message-filled"),
        },
        Some(other) if world.is_group_session(&other) => {
            let n = world.groups.sessions.get(&other).map(|s| s.participants.len()).unwrap_or(0);
            Identity {
                title: world.session_title(&other),
                subtitle: (n > 0).then(|| aurora_i18n::tr!("ui-chat-participants", n = n.to_string())),
                avatar: None,
                icon: None,
            }
        }
        Some(other) => {
            let names = &world.social.avatar_names;
            let cached = names.get(&other);
            Identity {
                title: cached
                    .map(|n| n.display(&names.options))
                    .unwrap_or_else(|| world.session_title(&other)),
                subtitle: cached.and_then(|n| n.chat_username(&names.options)),
                avatar: Some(other),
                icon: None,
            }
        }
    }
}

/// The head, then the filter field under it while the filter is on.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &crate::ui::icons::Icons,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
) {
    let who = identity(world, st);
    let status = who.avatar.map(|a| presence::of(world, &a));
    let group = st.selected.is_some_and(|s| world.is_group_session(&s));
    // buttons at the right: their count fixes the room the names keep
    // (a conversation's own close button among them)
    let snooze = st.selected.filter(|s| super::can_snooze(world, s));
    let tools = match st.selected {
        None => 1,
        Some(_) if group => 3,
        Some(_) => 4,
    } + 1
        + usize::from(snooze.is_some());
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let finding = st.searches.get(&st.selected).is_some_and(|s| s.find.is_some());
            ui.horizontal(|ui| {
                let names_w = (ui.available_width() - tools as f32 * TOOL_W - PIC - 16.0).max(40.0);
                if finding {
                    // the find field in the picture's and the names' place,
                    // as tall as the picture (no strip under the lines)
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(PIC + 4.0 + names_w, PIC), egui::Sense::hover());
                    let selected = st.selected;
                    let ChatUi { searches, views, .. } = &mut *st;
                    let view = views.entry(selected).or_default();
                    if let Some(search) = searches.get_mut(&selected)
                        && let Some(f) = search.find.as_mut()
                        && find::field(ui, p, rect, p.field, f, view, true, selected)
                    {
                        search.find = None;
                        ui.ctx().memory_mut(|m| m.request_focus(egui::Id::unique(("conv_input", selected))));
                    }
                } else {
                    picture_and_names(ui, p, icons, pics, world, st, actions, &who, status, names_w);
                }
                // ---- actions, from the right edge
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // at the right edge, the conversation's close button
                    // (the X of its tab); the nearby chat cannot be closed
                    if let Some(other) = st.selected
                        && tool(ui, p, icons, "x", super::close_tip(world, &other), true, false)
                    {
                        actions.push(ConvAction::Close(other));
                    }
                    // a group chat's snooze beside its close (FSFloaterIM's
                    // title bar)
                    if let Some(other) = snooze
                        && tool(ui, p, icons, SNOOZE_ICON, aurora_i18n::tr!("ui-chat-snooze-tip"), true, false)
                    {
                        actions.push(ConvAction::Snooze(other));
                    }
                    more_menu(ui, p, world, st, actions, group, false);
                    match st.selected {
                        None => {
                            if tool(ui, p, icons, "users", aurora_i18n::tr!("common-nearby-people"), true, false) {
                                actions.push(ConvAction::OpenPeople(0));
                            }
                        }
                        Some(_) if group => {
                            let n = world
                                .groups
                                .sessions
                                .get(&st.selected.unwrap_or_default())
                                .map(|s| s.participants.len())
                                .unwrap_or(0);
                            let tip = aurora_i18n::tr!("ui-chat-participants", n = n.to_string());
                            if tool(ui, p, icons, "users-group", &tip, true, st.show_profile) {
                                st.show_profile = !st.show_profile;
                            }
                            call_tool(ui, p, icons, world, st, actions);
                        }
                        Some(other) => {
                            call_tool(ui, p, icons, world, st, actions);
                            if tool(ui, p, icons, "send-filled", aurora_i18n::tr!("common-offer-teleport"), true, false) {
                                actions.push(ConvAction::OfferTeleport(other));
                            }
                            if tool(ui, p, icons, "user-filled", aurora_i18n::tr!("common-profile"), true, false) {
                                actions.push(ConvAction::Profile(other));
                            }
                        }
                    }
                });
            });
        });
    // the filter field, while the filter is on
    let search = st.searches.entry(st.selected).or_default();
    if let Some(q) = search.filter.as_mut() {
        ui.add_space(2.0);
        ui.add(
            egui::TextEdit::singleline(q)
                .id(find::filter_id(st.selected))
                .hint_text(aurora_i18n::tr!("ui-chat-filter"))
                .desired_width(ui.available_width()),
        );
    }
}

/// The picture and its status, then the names (and the status pill), in
/// `names_w`.
#[allow(clippy::too_many_arguments)]
fn picture_and_names(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &crate::ui::icons::Icons,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    who: &Identity,
    status: Option<presence::Status>,
    names_w: f32,
) {
    // ---- picture and its status
    let (rect, pic_resp) = ui.allocate_exact_size(egui::vec2(PIC, PIC), egui::Sense::click());
    match (who.avatar, who.icon) {
        (Some(a), _) => {
            st.wanted_pics.insert(a);
            crate::ui::widgets::avatar_disc(ui, p, rect, pics.get(&a), &who.title, false);
            if let Some(s) = status {
                presence::paint_dot(ui.painter(), p, rect, s, p.bar);
                pic_resp.clone().on_hover_text(s.label());
            }
            if pic_resp.clicked() {
                actions.push(ConvAction::Profile(a));
            }
        }
        (None, Some(icon)) => {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
            if let Some(t) = icons.get(icon) {
                ui.painter().image(
                    t.id(),
                    egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    p.violet_pale,
                );
            }
        }
        (None, None) => crate::ui::widgets::avatar_picture(ui, p, rect, None, &who.title, false),
    }
    ui.add_space(4.0);
    // ---- names
    ui.allocate_ui_with_layout(egui::vec2(names_w, PIC), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add_space(if who.subtitle.is_some() { 2.0 } else { 11.0 });
        ui.horizontal(|ui| {
            let pill_w = if status.is_some_and(|s| s.pill()) { 110.0 } else { 0.0 };
            let color = who
                .avatar
                .map(|id| crate::ui::colors::resident_name(world, &id, p.ink))
                .unwrap_or(p.ink);
            let title = RichText::new(&who.title).size(14.0).strong().color(color);
            let r = ui.add(crate::ui::widgets::elided_label(ui, title, names_w - pill_w).sense(egui::Sense::click()));
            if let Some(a) = who.avatar {
                super::name_menu(&r, p, world, a);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(aurora_i18n::tr!("common-view-profile"))
                    .clicked()
                {
                    actions.push(ConvAction::Profile(a));
                }
                crate::ui::staff::after_name(ui, p, crate::world::staff::mark(world, &a), 10.0);
            }
            if let Some(s) = status.filter(|s| s.pill()) {
                status_pill(ui, p, s);
            }
        });
        if let Some(sub) = &who.subtitle {
            let text = RichText::new(sub).size(11.5).color(p.muted);
            ui.add(crate::ui::widgets::elided_label(ui, text, names_w));
        }
    });
}

/// The call button of FSFloaterIM: calls the conversation, or hangs up
/// its call in progress ("Leave call"). Greyed while voice is off.
fn call_tool(ui: &mut egui::Ui, p: &Palette, icons: &crate::ui::icons::Icons, world: &World, st: &ChatUi, actions: &mut Vec<ConvAction>) {
    let Some(other) = st.selected else {
        return;
    };
    if world.call_of(&other).is_some() {
        // in a call: the strip under the head hangs up; the phone stays,
        // greyed and green
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::hover());
        if let Some(t) = icons.get("phone-filled") {
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                p.success.gamma_multiply(0.7),
            );
        }
        resp.on_hover_text(aurora_i18n::tr!("ui-call-in-progress"));
    } else if world.calls.invite_for(&other).is_some() {
        // ringing: answered from the strip
        tool(ui, p, icons, "phone-filled", aurora_i18n::tr!("ui-call-ringing"), false, false);
    } else if st.voice_on {
        if tool(ui, p, icons, "phone-filled", aurora_i18n::tr!("common-voice-call"), true, false) {
            actions.push(ConvAction::Call(other));
        }
    } else {
        tool(ui, p, icons, "phone-filled", aurora_i18n::tr!("ui-call-voice-off"), false, false);
    }
}

/// The other actions of the conversation: what Firestorm's IM toolbar has
/// and Aurora does not do yet stays in it, greyed. The workspace's head
/// (`head.rs`) has no teleport button: its offer is here.
pub(super) fn more_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    group: bool,
    workspace: bool,
) {
    crate::ui::widgets::icon_menu(ui, p, "dots-filled", aurora_i18n::tr!("ui-chat-more"), |ui| {
        more_items(ui, p, world, st, actions, group, workspace);
    });
}

/// The rows of the conversation's menu (`more_menu`, the workspace head's
/// "…").
pub(super) fn more_items(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    group: bool,
    workspace: bool,
) {
    let selected = st.selected;
    let search = st.searches.entry(selected).or_default();
    let filtering = search.filter.is_some();
    {
        match selected {
            Some(other) if group => {
                if workspace
                    && super::head::can_add_people(world, &other)
                    && menu::item(ui, p, super::head::ADD_PEOPLE_ICON, aurora_i18n::tr!("ui-call-add-people"))
                {
                    actions.push(ConvAction::PickPeople(other));
                }
                if world.groups.is_member(&other)
                    && menu::item(ui, p, "message-off", aurora_i18n::tr!("common-stop-receiving-this-group-s-chat"))
                {
                    actions.push(ConvAction::BlockGroupChat(other));
                }
            }
            Some(other) => {
                if workspace && menu::item(ui, p, "send-filled", aurora_i18n::tr!("common-offer-teleport")) {
                    actions.push(ConvAction::OfferTeleport(other));
                }
                if workspace && menu::item(ui, p, super::head::ADD_PEOPLE_ICON, aurora_i18n::tr!("ui-call-add-people")) {
                    actions.push(ConvAction::PickPeople(other));
                }
                if !workspace {
                    menu::todo(ui, p, "user-plus", aurora_i18n::tr!("ui-chat-add-friend-coming-soon"));
                    menu::todo(ui, p, "gift-filled", aurora_i18n::tr!("ui-chat-give-object-coming-soon"));
                } else {
                    // the friendship offer of the residents' menus, and the
                    // inventory to drag what to give onto the conversation
                    if !world.social.is_friend(&other) && menu::item(ui, p, "user-plus", aurora_i18n::tr!("common-become-friends")) {
                        crate::ui::context::request(ui.ctx(), CtxAction::AddFriend(other));
                    }
                    if menu::item(ui, p, "gift-filled", aurora_i18n::tr!("ui-chat-give-from-inventory")) {
                        actions.push(ConvAction::OpenInventory);
                    }
                }
                if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("ui-chat-pay")) {
                    crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::PayResident(other));
                }
                let blocked = world.is_avatar_blocked(&other);
                let label = if blocked {
                    aurora_i18n::tr!("common-unblock")
                } else {
                    aurora_i18n::tr!("common-block")
                };
                if menu::item(ui, p, "ban", label) {
                    actions.push(ConvAction::ToggleBlock(other));
                }
            }
            None => {}
        }
        if menu::check(
            ui,
            p,
            "search-filled",
            aurora_i18n::tr!("ui-chat-filter-conversation"),
            filtering,
            true,
        ) {
            search.filter = if filtering { None } else { Some(String::new()) };
        }
        // the whole transcript, when this account keeps them
        if world.chat_log.dir().is_some() && menu::item(ui, p, "history", aurora_i18n::tr!("common-view-conversation-history")) {
            crate::ui::context::request(ui.ctx(), CtxAction::ShowHistory(selected));
        }
        // the nearby chat cannot be closed, as in Firestorm
        if let Some(other) = selected {
            menu::separator(ui, p);
            if menu::item(ui, p, "x", close_tip(world, &other)) {
                actions.push(ConvAction::Close(other));
            }
        }
    }
}
