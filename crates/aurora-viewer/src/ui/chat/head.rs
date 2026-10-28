//! The head of a conversation in Aurora's workspace: a 48 px band with its
//! picture, its name and a line on it (the resident's user name and status,
//! a group's participants and call, the nearby chat's region and who hears
//! us), then its actions: call (green while in the call), the resident's
//! profile and, while they are not our friend, a friendship offer, the
//! members' panel, find, and the menu of the others (those of
//! Firestorm's IM toolbar, FSFloaterIM, kept in `header::more_menu`).

use super::{ChatUi, ConvAction, find, header, presence, sidebar};
use crate::theme::Palette;
use crate::world::World;
use egui::Color32;
use std::collections::HashMap;
use uuid::Uuid;

pub(super) const HEIGHT: f32 = 48.0;
/// From the middle of the title to the middle of the line under it.
const LINE_GAP: f32 = 17.0;
const PIC: f32 = 32.0;
const TOOL: f32 = 30.0;
/// Adding people to a conversation, wherever it is offered: the
/// conversation's bubble with a plus, apart from the friendship offer's
/// `user-plus`.
pub(super) const ADD_PEOPLE_ICON: &str = "message-circle-plus";

/// What the head says of the conversation shown.
struct Identity {
    title: String,
    subtitle: String,
    avatar: Option<Uuid>,
    kind: Option<sidebar::Kind>,
}

fn identity(world: &World, st: &ChatUi) -> Identity {
    let Some(id) = st.selected else {
        // where we stand: the parcel (the location bar's name), else the region
        let region = Some(world.rlva_parcel_label())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| world.rlva_region_label());
        let near = aurora_i18n::tr!("ui-chat-head-in-earshot", n = st.nearby_count);
        return Identity {
            title: aurora_i18n::tr!("common-nearby-chat").to_owned(),
            subtitle: if region.is_empty() { near } else { format!("{region} · {near}") },
            avatar: None,
            kind: None,
        };
    };
    let kind = sidebar::kind(world, &id);
    let mut parts: Vec<String> = Vec::new();
    let title = match kind {
        sidebar::Kind::Private => {
            let names = &world.social.avatar_names;
            let cached = names.get(&id);
            if let Some(user) = cached.and_then(|n| n.chat_username(&names.options)) {
                parts.push(user);
            }
            let status = presence::of(world, &id);
            if status.pill() {
                parts.push(status.label().to_lowercase());
            }
            if status == presence::Status::Offline {
                parts.push(aurora_i18n::tr!("ui-chat-head-offline-delivery").to_owned());
            }
            cached
                .map(|n| n.display(&names.options))
                .unwrap_or_else(|| world.session_title(&id))
        }
        sidebar::Kind::Group | sidebar::Kind::Conference => {
            parts.push(
                if kind == sidebar::Kind::Group {
                    aurora_i18n::tr!("ui-chat-head-group")
                } else {
                    aurora_i18n::tr!("ui-chat-head-conference")
                }
                .to_owned(),
            );
            let n = world.groups.sessions.get(&id).map(|s| s.participants.len()).unwrap_or(0);
            if n > 0 {
                parts.push(aurora_i18n::tr!("ui-chat-head-participants", n = n));
            }
            world.session_title(&id)
        }
    };
    if world.call_of(&id).is_some() || world.call_launched(&id).is_some() {
        parts.push(aurora_i18n::tr!("ui-chat-head-call-on").to_owned());
    }
    Identity {
        title,
        subtitle: parts.join(" · "),
        avatar: (kind == sidebar::Kind::Private).then_some(id),
        kind: Some(kind),
    }
}

/// The head; it may open the find bar (the search button).
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    // textures by id (a group's insignia)
    images: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    members_room: bool,
) {
    let who = identity(world, st);
    let width = ui.available_width();
    let gap = crate::ui::widgets::two_lines_gap(ui, &egui::FontId::proportional(14.0), &egui::FontId::proportional(12.0), LINE_GAP);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEIGHT + gap - LINE_GAP), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, p.ink.gamma_multiply(0.05)),
    );
    let pic = egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.center().y - PIC * 0.5), egui::Vec2::splat(PIC));
    let radius = ui.visuals().window_corner_radius;
    // while finding, the field takes the picture's and the names' place
    let finding = st.searches.get(&st.selected).is_some_and(|s| s.find.is_some());
    match (who.avatar, who.kind) {
        _ if finding => {}
        (Some(a), _) => {
            st.wanted_pics.insert(a);
            crate::ui::widgets::avatar_disc(ui, p, pic, pics.get(&a), &who.title, false);
            let status = presence::of(world, &a);
            if status != presence::Status::Unknown {
                presence::paint_dot(ui.painter(), p, pic, status, p.panel);
            }
            let r = ui.interact(pic, ui.scope_id().with("head_pic"), egui::Sense::click());
            super::name_menu(&r, p, world, a);
            if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(status.label())
                .clicked()
            {
                actions.push(ConvAction::Profile(a));
            }
        }
        (None, Some(sidebar::Kind::Group)) => {
            let insignia = st.selected.and_then(|id| sidebar::group_insignia(world, &id));
            if let Some(i) = insignia {
                st.wanted_images.insert(i);
            }
            sidebar::paint_group_picture(ui, p, pic, insignia.and_then(|i| images.get(&i)), PIC - 16.0);
        }
        (None, Some(_)) => {
            ui.painter().rect_filled(pic, radius, p.raised);
            super::thread::paint_icon(ui.painter(), "users", pic.shrink(8.0), p.muted);
        }
        (None, None) => {
            ui.painter().rect_filled(pic, radius, p.teal.gamma_multiply(0.16));
            super::thread::paint_icon(ui.painter(), "map-pin-filled", pic.shrink(8.0), p.teal);
        }
    }
    // the actions, from the right edge
    let mut x = rect.right() - 8.0;
    let mut slot = || {
        x -= TOOL + 2.0;
        egui::Rect::from_center_size(egui::pos2(x + TOOL * 0.5, rect.center().y), egui::Vec2::splat(TOOL))
    };
    let selected = st.selected;
    let group = who.kind.is_some_and(|k| k != sidebar::Kind::Private);
    // at the right edge, the close button of the conversation (the X of
    // its tab); the nearby chat cannot be closed
    if let Some(other) = selected
        && tool(ui, p, slot(), "x", super::close_tip(world, &other), Tone::Plain).clicked()
    {
        actions.push(ConvAction::Close(other));
    }
    // a group chat's snooze beside its close (FSFloaterIM's title bar)
    if let Some(other) = selected
        && super::can_snooze(world, &other)
        && tool(
            ui,
            p,
            slot(),
            super::SNOOZE_ICON,
            aurora_i18n::tr!("ui-chat-snooze-tip"),
            Tone::Plain,
        )
        .clicked()
    {
        actions.push(ConvAction::Snooze(other));
    }
    // the menu of the others
    let more = tool(ui, p, slot(), "dots-filled", aurora_i18n::tr!("ui-chat-more"), Tone::Plain);
    crate::ui::menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        header::more_items(ui, p, world, st, actions, group, true);
    });
    // find
    let find_tip = aurora_i18n::tr!("ui-chat-find-in-conversation");
    if tool(ui, p, slot(), "search-filled", find_tip, Tone::active(finding)).clicked() {
        let search = st.searches.entry(selected).or_default();
        if finding {
            search.find = None;
        } else {
            search.open_find();
            ui.ctx().memory_mut(|m| m.request_focus(find::field_id(selected)));
        }
    }
    match (selected, who.kind) {
        // the nearby chat: the people around (the People window)
        (None, _) => {
            if tool(ui, p, slot(), "users", aurora_i18n::tr!("common-nearby-people"), Tone::Plain).clicked() {
                actions.push(ConvAction::OpenPeople(0));
            }
        }
        // a group or a conference: its members' panel
        (Some(_), Some(k)) if k != sidebar::Kind::Private => {
            let open = st.members_open && members_room;
            let tip = if open {
                aurora_i18n::tr!("ui-chat-hide-members")
            } else {
                aurora_i18n::tr!("ui-chat-show-members")
            };
            let r = tool(
                ui,
                p,
                slot(),
                "users",
                tip,
                if members_room { Tone::active(open) } else { Tone::Off },
            );
            if r.clicked() && members_room {
                st.members_open = !st.members_open;
            }
        }
        // a resident: their profile, and a teleport offer to join us (the
        // teleport button of FSFloaterIM's toolbar)
        (Some(other), _) => {
            if tool(ui, p, slot(), "user-filled", aurora_i18n::tr!("common-profile"), Tone::Plain).clicked() {
                actions.push(ConvAction::Profile(other));
            }
            // the friendship offer of FSFloaterIM's add_friend_btn, hidden
            // (not greyed, as there) once they are our friend
            if !world.social.is_friend(&other)
                && tool(ui, p, slot(), "user-plus", aurora_i18n::tr!("ui-chat-add-friend"), Tone::Plain).clicked()
            {
                crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::AddFriend(other));
            }
            if tool(ui, p, slot(), "send-filled", aurora_i18n::tr!("common-offer-teleport"), Tone::Plain).clicked() {
                actions.push(ConvAction::OfferTeleport(other));
            }
        }
    }
    // add people: to a resident's conversation (a new conference, once
    // confirmed) or a conference; a role group's chat cannot
    // (FSFloaterIM's add_participant_panel, isInviteAllowed)
    if let Some(other) = selected
        && can_add_people(world, &other)
        && tool(ui, p, slot(), ADD_PEOPLE_ICON, aurora_i18n::tr!("ui-call-add-people"), Tone::Plain).clicked()
    {
        actions.push(ConvAction::PickPeople(other));
    }
    if let Some(other) = selected {
        call_tool(ui, p, slot(), world, st, actions, other);
    }
    if finding {
        // the field, from the picture's left to the actions, as tall as them
        let field = egui::Rect::from_x_y_ranges(
            pic.left()..=x - 6.0,
            (rect.center().y - TOOL * 0.5)..=(rect.center().y + TOOL * 0.5),
        );
        let ChatUi { searches, views, .. } = st;
        let view = views.entry(selected).or_default();
        if let Some(search) = searches.get_mut(&selected)
            && let Some(f) = search.find.as_mut()
            && find::field(ui, p, field, p.bar, f, view, false, selected)
        {
            // the head comes back, the keyboard goes to the message field
            search.find = None;
            ui.ctx().memory_mut(|m| m.request_focus(egui::Id::unique(("conv_input", selected))));
        }
        return;
    }
    // the names, up to the actions
    let text_left = pic.right() + 10.0;
    let text_right = x - 6.0;
    let title_y = if who.subtitle.is_empty() {
        rect.center().y
    } else {
        rect.center().y - gap * 8.0 / LINE_GAP
    };
    let title_font = egui::FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx()));
    // a member of the Aurora team's tag after the name, cut before it
    let room = text_right - text_left;
    let mark = who.avatar.and_then(|id| crate::world::staff::mark(world, &id));
    let staff = crate::ui::staff::width(ui, p, mark, 14.0);
    let staff = if staff > 0.0 && room - staff - 6.0 >= 60.0 {
        staff + 6.0
    } else {
        0.0
    };
    let cut = crate::ui::widgets::paint_elided(
        ui,
        &who.title,
        egui::pos2(text_left, title_y),
        room - staff,
        title_font.clone(),
        who.avatar
            .map(|id| crate::ui::colors::resident_name(world, &id, p.ink))
            .unwrap_or(p.ink),
    );
    if staff > 0.0 {
        let shown = ui
            .painter()
            .layout_no_wrap(who.title.clone(), title_font, p.ink)
            .size()
            .x
            .min(room - staff);
        crate::ui::staff::paint_at(ui, ui.painter(), p, mark, egui::pos2(text_left + shown + 6.0, title_y), 14.0);
    }
    if !who.subtitle.is_empty() {
        crate::ui::widgets::paint_elided(
            ui,
            &who.subtitle,
            egui::pos2(text_left, rect.center().y + gap * 9.0 / LINE_GAP),
            text_right - text_left,
            egui::FontId::proportional(12.0),
            p.muted_dim,
        );
    }
    let names = egui::Rect::from_x_y_ranges(text_left..=text_right, rect.y_range());
    let r = ui.interact(names, ui.scope_id().with("head_names"), egui::Sense::click());
    if let Some(a) = who.avatar {
        super::name_menu(&r, p, world, a);
        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
        let r = if cut { r.on_hover_text(&who.title) } else { r };
        if r.clicked() {
            actions.push(ConvAction::Profile(a));
        }
    } else if cut {
        r.on_hover_text(&who.title);
    }
}

/// People can be added to a conversation with a resident or a
/// conference, not to a role group's chat (FSFloaterIM::isInviteAllowed:
/// IM_NOTHING_SPECIAL, IM_SESSION_CONFERENCE_START, an invitation that is
/// not a group's).
pub(super) fn can_add_people(world: &World, conversation: &Uuid) -> bool {
    !world.groups.is_member(conversation) && !world.groups.sessions.get(conversation).is_some_and(|s| s.group)
}

/// The look of a head button.
#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Plain,
    /// Lit: its panel or bar is open.
    Active,
    /// Green: the call is in progress.
    Live,
    /// Greyed: it cannot be used now.
    Off,
}

impl Tone {
    fn active(on: bool) -> Tone {
        if on { Tone::Active } else { Tone::Plain }
    }
}

fn tool(ui: &mut egui::Ui, p: &Palette, rect: egui::Rect, icon: &str, tip: &str, tone: Tone) -> egui::Response {
    let id = sidebar::control_id(&format!("head-{icon}"));
    let resp = ui.interact(
        rect,
        id,
        if tone == Tone::Off {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if tone == Tone::Active || (resp.hovered() && tone != Tone::Off) {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.7));
    }
    let color: Color32 = match tone {
        Tone::Live => p.success,
        Tone::Off => p.muted_dim.gamma_multiply(0.6),
        Tone::Active => p.ink,
        Tone::Plain if resp.hovered() => p.ink,
        Tone::Plain => p.muted,
    };
    super::thread::paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(18.0)),
        color,
    );
    resp.on_hover_text(tip)
}

/// The call button of FSFloaterIM: calls the conversation, green while its
/// call is in progress (a click then hangs up, like "Leave call"), greyed
/// while voice is off.
fn call_tool(ui: &mut egui::Ui, p: &Palette, rect: egui::Rect, world: &World, st: &ChatUi, actions: &mut Vec<ConvAction>, other: Uuid) {
    if world.call_of(&other).is_some() {
        if tool(ui, p, rect, "phone-filled", aurora_i18n::tr!("ui-call-hang-up"), Tone::Live).clicked() {
            actions.push(ConvAction::HangUp);
        }
    } else if world.calls.invite_for(&other).is_some() {
        tool(
            ui,
            p,
            rect,
            "phone-filled",
            aurora_i18n::tr!("ui-chat-call-ringing-banner"),
            Tone::Live,
        );
    } else if !st.voice_on {
        tool(ui, p, rect, "phone-filled", aurora_i18n::tr!("ui-call-voice-off"), Tone::Off);
    } else {
        let tip = if world.call_launched(&other).is_some() {
            aurora_i18n::tr!("ui-chat-join-call")
        } else {
            aurora_i18n::tr!("common-voice-call")
        };
        let tone = if world.call_launched(&other).is_some() {
            Tone::Live
        } else {
            Tone::Plain
        };
        if tool(ui, p, rect, "phone-filled", tip, tone).clicked() {
            actions.push(ConvAction::Call(other));
        }
    }
}
