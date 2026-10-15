//! The column of conversations of Aurora's workspace (FSFloaterIMContainer's
//! tabs, laid out as a desktop chat's list): a search field that filters
//! them and starts new ones, Contacts and the nearby chat pinned on top,
//! then the private messages, the groups and the conferences, each section
//! with its "+" (start an IM, a group chat, a conference). An entry shows
//! the picture with the status shape, the name and what happens there (the
//! last message, a call, someone typing); unread conversations are bold,
//! marked at the left edge and counted. Narrow, the column becomes a rail
//! of pictures. A call in another conversation keeps its panel at the
//! bottom (microphone, back to it, hang up).

use super::{ChatUi, ConvAction, close_tip, presence};
use crate::theme::Palette;
use crate::world::World;
use egui::{Color32, RichText};
use std::collections::HashMap;
use uuid::Uuid;

/// Width of the column, and of the rail it becomes in a narrow window.
pub(super) const WIDTH: f32 = 232.0;
pub(super) const RAIL: f32 = 58.0;
const ENTRY_H: f32 = 44.0;
const PIC: f32 = 32.0;

/// Who a conversation is with, for the column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Private,
    Group,
    Conference,
}

/// The kind of a conversation of the list.
pub(super) fn kind(world: &World, id: &Uuid) -> Kind {
    match world.groups.sessions.get(id) {
        Some(s) if s.group => Kind::Group,
        Some(_) => Kind::Conference,
        None if world.groups.is_member(id) => Kind::Group,
        None => Kind::Private,
    }
}

/// The picture of an entry.
enum Icon<'a> {
    /// A resident: their picture and status.
    Avatar(Option<&'a egui::TextureHandle>, presence::Status),
    /// A group: its initials on the accent.
    Group(&'a str),
    /// A Tabler icon on a tinted square (nearby chat, Contacts, a conference).
    Glyph(&'static str, Color32),
}

struct Entry<'a> {
    icon: Icon<'a>,
    name: &'a str,
    sub: Option<String>,
    sub_icon: Option<(&'static str, Color32)>,
    current: bool,
    unread: usize,
    /// The tooltip of its close button (none: it cannot be closed).
    close: Option<&'a str>,
    /// A call rings there: the picture pulses.
    ringing: bool,
    /// A call is in progress there: the speaker at its right.
    in_call: bool,
}

struct Picked {
    resp: egui::Response,
    close: bool,
}

/// The column (or rail). `height`: what the window gives it.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    show_contacts_tab: bool,
    docked: bool,
    rail: bool,
    height: f32,
) {
    let width = if rail { RAIL } else { WIDTH };
    let (column, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter().rect_filled(column, 0.0, p.bar.gamma_multiply(0.75));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(column.shrink2(egui::vec2(6.0, 0.0))));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.add_space(8.0);
    if !rail {
        let hint = aurora_i18n::tr!("ui-chat-search-or-start");
        crate::ui::widgets::search_field(ui, &mut st.side_filter, hint, ui.available_width());
        ui.add_space(4.0);
    }
    let filter = st.side_filter.trim().to_lowercase();
    let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
    let voice = voice_panel_wanted(world, st);
    let list_h = (column.bottom() - ui.cursor().top() - if voice { voice_panel_h(rail) } else { 0.0 } - 6.0).max(ENTRY_H);
    egui::ScrollArea::vertical()
        .id_salt("conversation_column")
        .max_height(list_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // Contacts and the nearby chat, pinned
            if show_contacts_tab && matches(aurora_i18n::tr!("common-contacts")) {
                let e = Entry {
                    icon: Icon::Glyph("users", p.violet_pale),
                    name: aurora_i18n::tr!("common-contacts"),
                    sub: Some(aurora_i18n::tr!("ui-chat-contacts-sub").to_owned()),
                    sub_icon: None,
                    current: st.contacts,
                    unread: 0,
                    close: None,
                    ringing: false,
                    in_call: false,
                };
                if entry(ui, p, &e, rail).resp.clicked() {
                    if docked {
                        st.contacts = true;
                        st.show_profile = false;
                    } else {
                        actions.push(ConvAction::OpenContacts);
                    }
                }
            }
            let local = aurora_i18n::tr!("common-nearby-chat");
            if matches(local) {
                let shown = st.selected.is_none() && !st.contacts;
                let e = Entry {
                    icon: Icon::Glyph("map-pin-filled", p.teal),
                    name: local,
                    sub: Some(local_sub(world, st)),
                    sub_icon: None,
                    current: shown,
                    unread: if shown { 0 } else { world.chat_unread },
                    close: None,
                    ringing: false,
                    in_call: false,
                };
                if entry(ui, p, &e, rail).resp.clicked() {
                    st.select_tab(None);
                }
            }
            let reveal = st.reveal;
            for section in [Kind::Private, Kind::Group, Kind::Conference] {
                let ids: Vec<(Uuid, usize)> = world
                    .social
                    .ims
                    .iter()
                    .filter(|s| kind(world, &s.other) == section)
                    .map(|s| (s.other, s.unread))
                    .collect();
                let titled: Vec<(Uuid, usize, String)> = ids
                    .into_iter()
                    .map(|(id, unread)| (id, unread, world.session_title(&id)))
                    .filter(|(_, _, name)| matches(name))
                    .collect();
                section_head(ui, p, world, st, actions, section, rail);
                for (id, unread, name) in titled {
                    if section == Kind::Private {
                        st.wanted_pics.insert(id);
                    }
                    let current = st.selected == Some(id) && !st.contacts;
                    let status = presence::of(world, &id);
                    let initials = crate::ui::widgets::initials(&name);
                    let icon = match section {
                        Kind::Private => Icon::Avatar(pics.get(&id), status),
                        Kind::Group => Icon::Group(&initials),
                        Kind::Conference => Icon::Glyph("users", p.muted),
                    };
                    let (sub, sub_icon) = sub_line(p, world, id, section, status);
                    let e = Entry {
                        icon,
                        name: &name,
                        sub: Some(sub),
                        sub_icon,
                        current,
                        unread: if current { 0 } else { unread },
                        close: Some(close_tip(world, &id)),
                        ringing: world.calls.invite_for(&id).is_some(),
                        in_call: world.call_of(&id).is_some() || world.call_launched(&id).is_some(),
                    };
                    let picked = entry(ui, p, &e, rail);
                    if reveal == Some(id) {
                        picked.resp.scroll_to_me(None);
                        st.reveal = None;
                    }
                    if picked.close {
                        actions.push(ConvAction::Close(id));
                    } else if picked.resp.clicked() {
                        st.reveal = None;
                        st.select_tab(Some(id));
                    }
                }
            }
            // the search starts new conversations: friends without one, and
            // the resident search
            if !filter.is_empty() && !rail {
                let mut friends: Vec<(Uuid, String)> = world
                    .social
                    .friends
                    .iter()
                    .filter(|f| !world.social.has_session(&f.id))
                    .map(|f| (f.id, world.social.name_of(&f.id)))
                    .filter(|(_, n)| n.to_lowercase().contains(&filter))
                    .collect();
                friends.sort_by_key(|(_, n)| n.to_lowercase());
                if !friends.is_empty() {
                    plain_head(ui, p, aurora_i18n::tr!("ui-chat-section-start-with"));
                    for (id, name) in friends.into_iter().take(8) {
                        st.wanted_pics.insert(id);
                        let e = Entry {
                            icon: Icon::Avatar(pics.get(&id), presence::of(world, &id)),
                            name: &name,
                            sub: Some(aurora_i18n::tr!("ui-chat-start-im").to_owned()),
                            sub_icon: Some(("message-circle-filled", p.muted_dim)),
                            current: false,
                            unread: 0,
                            close: None,
                            ringing: false,
                            in_call: false,
                        };
                        if entry(ui, p, &e, rail).resp.clicked() {
                            st.side_filter.clear();
                            actions.push(ConvAction::OpenIm(id));
                        }
                    }
                }
                let e = Entry {
                    icon: Icon::Glyph("search", p.muted),
                    name: aurora_i18n::tr!("ui-chat-find-resident"),
                    sub: Some(st.side_filter.trim().to_owned()),
                    sub_icon: None,
                    current: false,
                    unread: 0,
                    close: None,
                    ringing: false,
                    in_call: false,
                };
                if entry(ui, p, &e, rail).resp.clicked() {
                    st.start_picker(super::Start::Im);
                }
            }
        });
    if voice {
        voice_panel(ui, p, world, st, actions, rail);
    }
}

/// What the nearby chat's entry says: who can hear us, and its last line.
fn local_sub(world: &World, st: &ChatUi) -> String {
    if let Some(t) = super::typing::text(&st.nearby_typing_names(world)) {
        return t;
    }
    let near = aurora_i18n::tr!("ui-chat-in-earshot", n = st.nearby_count.to_string()).to_owned();
    match world.chat.iter().rev().find(|l| !l.kind.is_notice()) {
        Some(l) => format!("{near} · {}", preview(world, l, true)),
        None => near,
    }
}

/// The last message of a conversation in a few words: "Name: text", ours
/// "You: text", an emote as it reads.
fn preview(world: &World, line: &crate::world::ChatLine, named: bool) -> String {
    let own = world.own_name();
    let name = super::sender_name(world, &own, line);
    let (text, _) = super::line_body(line, &name);
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    if emote || line.kind.is_notice() {
        text
    } else if line.kind == crate::world::ChatKind::Own {
        aurora_i18n::tr!("ui-chat-preview-you", text = text.as_str()).to_owned()
    } else if named {
        let first = name.split_whitespace().next().unwrap_or(&name);
        format!("{first} : {text}")
    } else {
        text
    }
}

/// What an entry says under its name, by priority: a call, someone typing,
/// a status that is not online (residents), the last message.
fn sub_line(p: &Palette, world: &World, id: Uuid, kind: Kind, status: presence::Status) -> (String, Option<(&'static str, Color32)>) {
    if let Some(invite) = world.calls.invite_for(&id) {
        let text = if invite.invite.p2p {
            aurora_i18n::tr!("ui-chat-sub-calls-you")
        } else {
            aurora_i18n::tr!("ui-chat-sub-call-started")
        };
        return (text.to_owned(), Some(("phone-filled", p.success)));
    }
    if let Some(call) = world.call_of(&id) {
        let secs = call.connected_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let time = super::call::duration(secs);
        let text = aurora_i18n::tr!("ui-chat-sub-in-call", time = time.as_str());
        return (text.to_owned(), Some(("volume", p.success)));
    }
    if world.call_launched(&id).is_some() {
        return (aurora_i18n::tr!("ui-chat-sub-call-ongoing").to_owned(), Some(("volume", p.success)));
    }
    if kind == Kind::Private && world.social.is_typing(&id) {
        return (aurora_i18n::tr!("ui-chat-sub-typing").to_owned(), None);
    }
    let last = world
        .social
        .ims
        .iter()
        .find(|s| s.other == id)
        .and_then(|s| s.lines.iter().rev().find(|l| !matches!(l.kind, crate::world::ChatKind::Invite(_))));
    let last = last.map(|l| preview(world, l, kind != Kind::Private)).unwrap_or_default();
    if kind == Kind::Private && matches!(status, presence::Status::Away | presence::Status::Busy | presence::Status::Offline) {
        let label = status.label();
        return (if last.is_empty() { label.to_owned() } else { format!("{label} · {last}") }, None);
    }
    if last.is_empty() && kind != Kind::Private {
        let n = world.groups.sessions.get(&id).map(|s| s.participants.len()).unwrap_or(0);
        return (aurora_i18n::tr!("ui-chat-participants", n = n.to_string()).to_owned(), None);
    }
    (last, None)
}

/// The head of a section, with its "+": start an IM (the resident picker),
/// a group's chat (the list of our groups), a conference (several
/// residents).
fn section_head(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut ChatUi, actions: &mut Vec<ConvAction>, section: Kind, rail: bool) {
    if rail {
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter()
            .hline(r.center().x - 12.0..=r.center().x + 12.0, r.center().y, egui::Stroke::new(1.0, p.raised));
        ui.add_space(4.0);
        return;
    }
    let (title, tip) = match section {
        Kind::Private => (aurora_i18n::tr!("ui-chat-section-private"), aurora_i18n::tr!("ui-chat-new-im")),
        Kind::Group => (aurora_i18n::tr!("ui-chat-section-groups"), aurora_i18n::tr!("ui-chat-new-group-chat")),
        Kind::Conference => (aurora_i18n::tr!("ui-chat-section-conferences"), aurora_i18n::tr!("ui-chat-new-conference")),
    };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(title.to_uppercase()).size(10.5).strong().color(p.muted_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::click());
            super::thread::paint_icon(
                ui.painter(),
                "plus",
                rect.shrink(2.0),
                if resp.hovered() { p.ink } else { p.muted_dim },
            );
            let resp = resp.on_hover_text(tip);
            match section {
                Kind::Private => {
                    if resp.clicked() {
                        st.start_picker(super::Start::Im);
                    }
                }
                Kind::Conference => {
                    if resp.clicked() {
                        st.start_picker(super::Start::Conference);
                    }
                }
                Kind::Group => {
                    crate::ui::menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        let groups = world.groups.sorted();
                        if groups.is_empty() {
                            crate::ui::menu::info(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-chat-no-groups"));
                        }
                        for g in groups {
                            if crate::ui::menu::item(ui, p, "users-group", &g.name) {
                                actions.push(ConvAction::StartGroupChat(g.id));
                            }
                        }
                    });
                }
            }
        });
    });
    ui.add_space(2.0);
}

fn plain_head(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(title.to_uppercase()).size(10.5).strong().color(p.muted_dim));
    });
    ui.add_space(2.0);
}

/// One entry of the column (or of the rail: its picture alone, the name
/// in its tooltip).
fn entry(ui: &mut egui::Ui, p: &Palette, e: &Entry, rail: bool) -> Picked {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ENTRY_H), egui::Sense::click());
    let over = resp.hovered() || (e.close.is_some() && ui.rect_contains_pointer(rect));
    let radius = ui.visuals().window_corner_radius;
    if e.current {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.22));
    } else if over {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.45));
    }
    if e.unread > 0 && !e.current {
        // the mark of an unread conversation, against the column's edge
        let mark = egui::Rect::from_min_size(egui::pos2(rect.left() - 6.0, rect.center().y - 4.0), egui::vec2(3.0, 8.0));
        ui.painter().rect_filled(mark, crate::ui::widgets::inner_radius(ui.visuals()), p.ink);
    }
    let pic_x = if rail { rect.center().x - PIC * 0.5 } else { rect.left() + 6.0 };
    let pic = egui::Rect::from_min_size(egui::pos2(pic_x, rect.center().y - PIC * 0.5), egui::Vec2::splat(PIC));
    if e.ringing {
        let t = ui.input(|i| i.time) as f32;
        let pulse = (t * 3.0).sin().abs();
        ui.painter().circle_stroke(
            pic.center(),
            PIC * 0.5 + 2.0 + pulse * 3.0,
            egui::Stroke::new(2.0, p.success.gamma_multiply(1.0 - pulse * 0.7)),
        );
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    let ring = if e.current {
        p.violet.gamma_multiply(0.22)
    } else {
        p.bar
    };
    paint_picture(ui, p, &e.icon, e.name, pic, ring);
    let tip_name = || e.name.to_owned();
    if rail {
        if e.unread > 0 {
            crate::ui::widgets::UnreadCount::new(e.unread, crate::ui::widgets::CountSize::List)
                .place(ui.ctx(), p, egui::pos2(pic.right() + 3.0, pic.top() - 1.0), egui::Align2::RIGHT_TOP)
                .paint(ui.painter());
        }
        let resp = resp.on_hover_text(tip_name());
        return Picked { resp, close: false };
    }
    // the right end: the close button under the pointer, else the speaker of
    // a call and the unread count
    let slot = egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::Vec2::splat(18.0));
    let mut right = rect.right() - 6.0;
    let mut close = false;
    let closable = e.close.is_some() && over;
    if let (Some(tip), true) = (e.close, closable) {
        let button = ui.interact(slot, resp.id.with("close"), egui::Sense::click());
        if button.hovered() {
            ui.painter().rect_filled(slot, ui.visuals().widgets.inactive.corner_radius, p.raised);
        }
        super::thread::paint_icon(
            ui.painter(),
            "x",
            egui::Rect::from_center_size(slot.center(), egui::Vec2::splat(12.0)),
            if button.hovered() { p.ink } else { p.muted },
        );
        close = button.on_hover_text(tip).clicked();
        right = slot.left() - 2.0;
    } else {
        if e.unread > 0 {
            let count = crate::ui::widgets::UnreadCount::new(e.unread, crate::ui::widgets::CountSize::List).place(
                ui.ctx(),
                p,
                egui::pos2(rect.right() - 6.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
            );
            right = count.rect.left() - 4.0;
            count.paint(ui.painter());
        }
        if e.in_call {
            let r = egui::Rect::from_center_size(egui::pos2(right - 8.0, rect.center().y), egui::Vec2::splat(15.0));
            super::thread::paint_icon(ui.painter(), "volume", r, p.success);
            right = r.left() - 3.0;
        }
    }
    let x = pic.right() + 9.0;
    let bold = e.current || e.unread > 0;
    let name_color = if bold { p.ink } else { p.muted };
    let font = if bold {
        egui::FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()))
    } else {
        egui::FontId::proportional(13.0)
    };
    let has_sub = e.sub.as_ref().is_some_and(|s| !s.is_empty());
    let name_y = if has_sub { rect.center().y - 8.0 } else { rect.center().y };
    let cut = crate::ui::widgets::paint_elided(ui, e.name, egui::pos2(x, name_y), right - x, font, name_color);
    if let Some(sub) = e.sub.as_ref().filter(|s| !s.is_empty()) {
        let mut sx = x;
        if let Some((icon, color)) = e.sub_icon {
            let r = egui::Rect::from_min_size(egui::pos2(sx, rect.center().y + 3.0), egui::Vec2::splat(11.0));
            super::thread::paint_icon(ui.painter(), icon, r, color);
            sx += 14.0;
        }
        crate::ui::widgets::paint_elided(
            ui,
            sub,
            egui::pos2(sx, rect.center().y + 9.0),
            right - sx,
            egui::FontId::proportional(11.0),
            p.muted_dim,
        );
    }
    let resp = if cut { resp.on_hover_text(tip_name()) } else { resp };
    Picked { resp, close }
}

/// The picture of an entry: a resident's with its status, a group's
/// initials on the accent, a glyph on a tinted square.
fn paint_picture(ui: &egui::Ui, p: &Palette, icon: &Icon, name: &str, rect: egui::Rect, ring: Color32) {
    let radius = ui.visuals().window_corner_radius;
    match icon {
        Icon::Avatar(pic, status) => {
            crate::ui::widgets::avatar_disc(ui, p, rect, *pic, name, false);
            if *status != presence::Status::Unknown {
                presence::paint_dot(ui.painter(), p, rect, *status, ring);
            }
        }
        Icon::Group(initials) => {
            ui.painter().rect_filled(rect, radius, p.violet);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                egui::FontId::proportional(12.0),
                p.on_accent,
            );
        }
        Icon::Glyph(glyph, color) => {
            ui.painter().rect_filled(rect, radius, color.gamma_multiply(0.16));
            super::thread::paint_icon(
                ui.painter(),
                glyph,
                egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(17.0)),
                *color,
            );
        }
    }
}

/// The panel of a call at the bottom of the column, while another
/// conversation (or Contacts) is shown.
fn voice_panel_wanted(world: &World, st: &ChatUi) -> bool {
    world
        .calls
        .active
        .as_ref()
        .is_some_and(|c| st.contacts || st.selected != Some(c.conversation()))
}

fn voice_panel_h(rail: bool) -> f32 {
    if rail { 110.0 } else { 74.0 }
}

fn voice_panel(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut ChatUi, actions: &mut Vec<ConvAction>, rail: bool) {
    let Some(call) = world.calls.active.as_ref() else {
        return;
    };
    let conversation = call.conversation();
    let bottom = ui.max_rect().bottom() - 6.0;
    let rect = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), bottom - voice_panel_h(rail)),
        egui::pos2(ui.max_rect().right(), bottom),
    );
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)));
    let ui = &mut child;
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
    if !rail {
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "phone-filled", p.success, 14.0);
            ui.label(RichText::new(aurora_i18n::tr!("ui-chat-call-connected")).size(12.0).strong().color(p.success));
        });
        let secs = call.connected_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let text = format!("{} · {}", world.session_title(&conversation), super::call::duration(secs));
        ui.add(crate::ui::widgets::elided_label(ui, RichText::new(text).size(11.0).color(p.muted), ui.available_width()));
    }
    let layout = if rail {
        egui::Layout::top_down(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    ui.with_layout(layout, |ui| {
        let w = if rail { ui.available_width() } else { (ui.available_width() - 12.0) / 3.0 };
        let size = egui::vec2(w, 26.0);
        if let Some(on) = st.mic {
            let (icon, tip) = if on {
                ("microphone-filled", aurora_i18n::tr!("ui-call-mute"))
            } else {
                ("microphone-off", aurora_i18n::tr!("ui-call-unmute"))
            };
            if square_button(ui, p, icon, tip, size, !on).clicked() {
                actions.push(ConvAction::ToggleMic);
            }
        }
        if square_button(ui, p, "message-circle-filled", aurora_i18n::tr!("ui-chat-back-to-call"), size, false).clicked() {
            st.select_tab(Some(conversation));
        }
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            if resp.hovered() { p.danger } else { p.danger.gamma_multiply(0.85) },
        );
        super::thread::paint_icon(
            ui.painter(),
            "phone-off",
            egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)),
            p.on_accent,
        );
        if resp.on_hover_text(aurora_i18n::tr!("ui-call-hang-up")).clicked() {
            actions.push(ConvAction::HangUp);
        }
    });
}

/// A small square button of an icon (`off`: the danger tint of a muted
/// microphone).
pub(super) fn square_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, size: egui::Vec2, off: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let face = match (off, resp.hovered()) {
        (true, _) => p.danger.gamma_multiply(0.2),
        (false, true) => p.raised.gamma_multiply(1.3),
        (false, false) => p.raised,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
    super::thread::paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)),
        if off { p.danger } else { p.ink },
    );
    resp.on_hover_text(tip)
}
