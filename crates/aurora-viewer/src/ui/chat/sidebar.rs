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
use crate::world::contact_sets::appearance::ICONS;
use egui::{Color32, RichText};
use std::collections::HashMap;
use uuid::Uuid;

/// Width of the column, and of the rail it becomes in a narrow window.
pub(super) const WIDTH: f32 = 232.0;
pub(super) const RAIL: f32 = 58.0;
const ENTRY_H: f32 = 44.0;
/// From the middle of an entry's name to the middle of its subtitle.
const LINE_GAP: f32 = 17.0;
const PIC: f32 = 32.0;
/// The glyph on an entry's square (nearby chat, a group without insignia).
const GLYPH: f32 = 17.0;

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

/// A section of the column: a contact set whose members' conversations
/// are shown apart (an index in `ContactSets::chat_sections`, #1104), then
/// the kinds. A conversation is moved only within its section.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Section {
    Set(usize),
    Kind(Kind),
}

impl Section {
    /// The kind of the conversations it holds.
    fn kind(self) -> Kind {
        match self {
            Section::Set(_) => Kind::Private,
            Section::Kind(k) => k,
        }
    }
}

/// The section of a conversation: a private one with a member of a set
/// shown apart goes to the highest-priority such set.
pub(super) fn section(world: &World, id: &Uuid) -> Section {
    match kind(world, id) {
        Kind::Private => world
            .contact_sets
            .chat_section(id)
            .map_or(Section::Kind(Kind::Private), Section::Set),
        k => Section::Kind(k),
    }
}

/// The sections from top to bottom: the sets shown apart (highest
/// priority first), the private messages, the groups, the conferences.
fn sections(world: &World) -> impl Iterator<Item = Section> {
    (0..world.contact_sets.chat_sections().len())
        .map(Section::Set)
        .chain([Kind::Private, Kind::Group, Kind::Conference].map(Section::Kind))
}

/// The insignia of a group conversation's group: our membership's, else
/// the group profile the viewer last received (a group we are not in);
/// None when it has none, the `users-group` glyph then.
pub(crate) fn group_insignia(world: &World, id: &Uuid) -> Option<Uuid> {
    let set = |i: Uuid| (!i.is_nil()).then_some(i);
    world
        .groups
        .group(id)
        .and_then(|g| set(g.insignia))
        .or_else(|| world.search.groups.get(id).and_then(|g| set(g.insignia)))
}

/// A group's picture over the square `rect`: its insignia cropped square on
/// the theme's radius, else the `users-group` glyph (of side `glyph`) on a
/// raised square, as People › Groups draws it.
pub(super) fn paint_group_picture(ui: &egui::Ui, p: &Palette, rect: egui::Rect, insignia: Option<&egui::TextureHandle>, glyph: f32) {
    match insignia {
        Some(t) => crate::ui::widgets::square_picture(ui, rect, t),
        None => {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
            super::thread::paint_icon(
                ui.painter(),
                "users-group",
                egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(glyph)),
                p.muted,
            );
        }
    }
}

/// The id of a control of the workspace by its name ("panel-mic",
/// "head-call"…): the same every frame, for the tests to reach it; each
/// window of its own has its own (`popout::window`).
pub(super) fn control_id(name: &str) -> egui::Id {
    egui::Id::unique(("conv_control", name, super::popout::window()))
}

/// The id of the column's search field.
pub(super) fn search_id() -> egui::Id {
    egui::Id::unique("conv_column_search")
}

/// The search field of the column (the mockup's `.search`): a filled
/// box with its magnifier, no outline.
fn search_box(ui: &mut egui::Ui, p: &Palette, text: &mut String, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, SEARCH_H), egui::Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field);
    let glass = egui::Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), egui::Vec2::splat(13.0));
    super::thread::paint_icon(ui.painter(), "search-filled", glass, p.muted_dim);
    let field = egui::Rect::from_min_max(
        egui::pos2(glass.right() + 6.0, rect.top()),
        egui::pos2(rect.right() - 6.0, rect.bottom()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(field)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.add(
        egui::TextEdit::singleline(text)
            .id(search_id())
            .hint_text(
                RichText::new(aurora_i18n::tr!("ui-chat-search-or-start"))
                    .size(12.0)
                    .color(p.muted_dim),
            )
            .font(egui::FontId::proportional(12.0))
            .frame(egui::Frame::NONE.inner_margin(crate::ui::widgets::field_text_margin(ui.ctx(), &egui::FontId::proportional(12.0))))
            .desired_width(field.width()),
    );
}

/// Height of the search field, and the side of the button beside it.
const SEARCH_H: f32 = 30.0;

/// The one button that starts a conversation, beside the search field (the
/// sections lost their "+" with the empty ones, the human's choice of
/// 2026-10-16): a private message or a conference (the resident picker),
/// or the chat of one of our groups.
fn new_conversation(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut ChatUi, actions: &mut Vec<ConvAction>) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(SEARCH_H), egui::Sense::hover());
    let resp = ui.interact(rect, control_id("new-conversation"), egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let face = if resp.hovered() || open { p.raised } else { p.field };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
    super::thread::paint_icon(
        ui.painter(),
        "plus",
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)),
        if resp.hovered() || open { p.ink } else { p.muted },
    );
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-chat-new-conversation"));
    if std::mem::take(&mut st.demo_new_menu) {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
    }
    crate::ui::menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        if crate::ui::menu::item(ui, p, "message-circle-filled", aurora_i18n::tr!("ui-chat-start-private")) {
            st.start_picker(super::Start::Im);
        }
        crate::ui::menu::submenu(ui, p, "users-group", aurora_i18n::tr!("ui-chat-start-group"), true, |ui| {
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
        if crate::ui::menu::item(ui, p, "users", aurora_i18n::tr!("ui-chat-start-conference")) {
            st.start_picker(super::Start::Conference);
        }
    });
}

/// What an entry of the column is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Pinned {
    Contacts,
    Local,
    Conversation(Uuid),
}

/// The id of an entry of the column (its close button: `.with("close")`).
pub(super) fn entry_id(entry: Pinned) -> egui::Id {
    egui::Id::unique(("conversation_entry", entry))
}

/// The picture of an entry.
pub(super) enum Icon<'a> {
    /// A resident: their picture and status.
    Avatar(Option<&'a egui::TextureHandle>, presence::Status),
    /// A group: its insignia, once loaded (none: the group glyph).
    Group(Option<&'a egui::TextureHandle>),
    /// A Tabler icon on a tinted square (nearby chat, Contacts, a conference).
    Glyph(&'static str, Color32),
}

struct Entry<'a> {
    icon: Icon<'a>,
    name_color: Option<Color32>,
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
    /// The violet wash of a conversation with news (`flash.rs`), 0 for none.
    flash: f32,
    /// The conversation is in its own window (`popout.rs`): a mark where
    /// the unread count goes.
    popped: bool,
}

struct Picked {
    resp: egui::Response,
    close: bool,
}

/// What the Contacts entry's second line says: the friends online (the tab
/// shows People's friends) or what the contact list holds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ContactsSub {
    Friends,
    Lists,
}

/// The column (or rail). `height`: what the window gives it. Returns true
/// when the docked Contacts entry was clicked, even if already selected.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    // textures by id (the groups' insignias)
    images: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    show_contacts_tab: bool,
    contacts_sub: ContactsSub,
    docked: bool,
    rail: bool,
    // RAIL or WIDTH, or on its way from one to the other
    width: f32,
    height: f32,
) -> bool {
    let (column, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let mut contacts_clicked = false;
    // the mockup's `.side`: the bar's colour at 75 %, a hairline at its
    // right, its bottom left corner the window's
    let corner = ui.visuals().window_corner_radius;
    let radius = egui::CornerRadius {
        sw: corner.sw,
        ..egui::CornerRadius::ZERO
    };
    ui.painter().rect_filled(column, radius, p.bar.gamma_multiply(0.75));
    ui.painter().vline(
        column.right() - 0.5,
        column.y_range(),
        egui::Stroke::new(1.0, p.ink.gamma_multiply(0.04)),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(column.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    // (while its width eases, nothing spills over the messages)
    child.set_clip_rect(column.intersect(child.clip_rect()));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.add_space(8.0);
    if !rail {
        let row = egui::vec2(ui.available_width(), SEARCH_H);
        ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = (ui.available_width() - SEARCH_H - 4.0).max(40.0);
            search_box(ui, p, &mut st.side_filter, width);
            new_conversation(ui, p, world, st, actions);
        });
        ui.add_space(4.0);
    }
    let filter = st.side_filter.trim().to_lowercase();
    let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
    let voice = voice_panel_wanted(world, st);
    let list_h = (column.bottom() - ui.cursor().top() - if voice { voice_panel_h(rail) + VOICE_PANEL_GAP } else { 0.0 } - 6.0).max(ENTRY_H);
    egui::ScrollArea::vertical()
        .id_salt("conversation_column")
        .max_height(list_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // Contacts and the nearby chat, pinned
            if show_contacts_tab && docked && matches(aurora_i18n::tr!("common-contacts")) {
                let e = Entry {
                    icon: Icon::Glyph("users", p.violet_pale),
                    name_color: None,
                    name: aurora_i18n::tr!("common-contacts"),
                    sub: Some(match contacts_sub {
                        ContactsSub::Friends => aurora_i18n::tr!(
                            "ui-chat-contacts-friends-sub",
                            online = world.social.friends.iter().filter(|f| f.online).count() as i64
                        ),
                        ContactsSub::Lists => aurora_i18n::tr!("ui-chat-contacts-sub").to_owned(),
                    }),
                    sub_icon: None,
                    current: st.contacts,
                    unread: 0,
                    close: None,
                    ringing: false,
                    in_call: false,
                    flash: 0.0,
                    popped: false,
                };
                if entry(ui, p, &e, rail, entry_id(Pinned::Contacts)).resp.clicked() {
                    contacts_clicked = true;
                    st.contacts = true;
                    st.show_profile = false;
                }
            }
            let local = aurora_i18n::tr!("common-nearby-chat");
            if !st.popped.contains(None) && matches(local) {
                let shown = st.selected.is_none() && !st.contacts;
                let e = Entry {
                    icon: Icon::Glyph("map-pin-filled", p.teal),
                    name: local,
                    name_color: None,
                    sub: Some(local_sub(world, st)),
                    sub_icon: None,
                    current: shown,
                    unread: if shown { 0 } else { world.chat_badge },
                    close: None,
                    ringing: false,
                    in_call: false,
                    flash: 0.0,
                    popped: false,
                };
                let picked = entry(ui, p, &e, rail, entry_id(Pinned::Local));
                super::conv_menu::demo_mark(ui.ctx(), world, st, None, picked.resp.rect);
                super::conv_menu::attach(&picked.resp, p, world, actions, None);
                st.reorder.local_row(&picked.resp);
                if st.reorder.dragging(None) {
                    ghost(ui.ctx(), p, st, e, rail, Pinned::Local);
                }
                if picked.resp.clicked() {
                    st.select_tab(None);
                }
            }
            let reveal = st.reveal;
            let now = ui.input(|i| i.time);
            st.flashes.begin(now);
            let sessions = st.reorder.sessions(world);
            for section in sections(world) {
                let kind = section.kind();
                let ids: Vec<(Uuid, usize)> = sessions
                    .iter()
                    .copied()
                    .filter(|(id, _)| !st.popped.contains(Some(*id)) && self::section(world, id) == section)
                    .collect();
                let titled: Vec<(Uuid, usize, String)> = ids
                    .into_iter()
                    .map(|(id, unread)| (id, unread, entry_name(world, &id, kind)))
                    .filter(|(_, _, name)| matches(name))
                    .collect();
                // a section without a conversation is not drawn at all: no
                // head, no space (the human's choice of 2026-10-16)
                if titled.is_empty() {
                    continue;
                }
                section_head(ui, p, world, section, rail);
                for (id, unread, name) in titled {
                    if kind == Kind::Private {
                        st.wanted_pics.insert(id);
                    }
                    let current = st.selected == Some(id) && !st.contacts;
                    let status = presence::of(world, &id);
                    let insignia = if kind == Kind::Group { group_insignia(world, &id) } else { None };
                    if let Some(i) = insignia {
                        st.wanted_images.insert(i);
                    }
                    let icon = match kind {
                        Kind::Private => Icon::Avatar(pics.get(&id), status),
                        Kind::Group => Icon::Group(insignia.and_then(|i| images.get(&i))),
                        Kind::Conference => Icon::Glyph("users", p.muted),
                    };
                    let (sub, sub_icon) = sub_line(p, world, id, kind, status);
                    let sub_icon = sub_icon.filter(|_| {
                        st.conversation_voice_indicator
                            || (world.call_of(&id).is_none() && world.call_launched(&id).is_none() && world.calls.invite_for(&id).is_none())
                    });
                    let unread = if current { 0 } else { unread };
                    let (flash, moving) = st.flashes.wash(id, unread, now, st.flash_unread);
                    if moving {
                        ui.ctx().request_repaint();
                    }
                    let e = Entry {
                        icon,
                        name_color: if kind == Kind::Private {
                            world.contact_sets.appearance(&id).ui_name.map(crate::ui::colors::set_color)
                        } else {
                            None
                        },
                        name: &name,
                        sub: Some(sub),
                        sub_icon,
                        current,
                        unread,
                        close: Some(close_tip(world, &id)),
                        ringing: world.calls.invite_for(&id).is_some(),
                        in_call: st.conversation_voice_indicator && (world.call_of(&id).is_some() || world.call_launched(&id).is_some()),
                        flash,
                        popped: false,
                    };
                    st.reorder.space_before(ui, id);
                    let picked = entry(ui, p, &e, rail, entry_id(Pinned::Conversation(id)));
                    if reveal == Some(id) {
                        picked.resp.scroll_to_me(None);
                        st.reveal = None;
                    }
                    super::conv_menu::demo_mark(ui.ctx(), world, st, Some(id), picked.resp.rect);
                    super::conv_menu::attach(&picked.resp, p, world, actions, Some(id));
                    st.reorder.row(world, id, &picked.resp);
                    st.reorder.space_after(ui, id);
                    if st.reorder.dragging(Some(id)) {
                        ghost(ui.ctx(), p, st, e, rail, Pinned::Conversation(id));
                    }
                    if picked.close {
                        actions.push(ConvAction::Close(id));
                    } else if picked.resp.clicked() {
                        st.reveal = None;
                        st.select_tab(Some(id));
                    }
                }
            }
            if let Some(out) = st.reorder.finish(ui, p, st.demo_reorder) {
                actions.push(ConvAction::PopOutAt(out.conversation, out.pos));
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
                            name_color: world.contact_sets.appearance(&id).ui_name.map(crate::ui::colors::set_color),
                            name: &name,
                            sub: Some(aurora_i18n::tr!("ui-chat-start-im").to_owned()),
                            sub_icon: Some(("message-circle-filled", p.muted_dim)),
                            current: false,
                            unread: 0,
                            close: None,
                            ringing: false,
                            in_call: false,
                            flash: 0.0,
                            popped: false,
                        };
                        if entry(ui, p, &e, rail, egui::Id::unique(("conversation_start", id))).resp.clicked() {
                            st.side_filter.clear();
                            actions.push(ConvAction::OpenIm(id));
                        }
                    }
                }
                let e = Entry {
                    icon: Icon::Glyph("search-filled", p.muted),
                    name: aurora_i18n::tr!("ui-chat-find-resident"),
                    name_color: None,
                    sub: Some(st.side_filter.trim().to_owned()),
                    sub_icon: None,
                    current: false,
                    unread: 0,
                    close: None,
                    ringing: false,
                    in_call: false,
                    flash: 0.0,
                    popped: false,
                };
                if entry(ui, p, &e, rail, egui::Id::unique("conversation_find")).resp.clicked() {
                    st.start_picker(super::Start::Im);
                }
            }
        });
    if voice {
        voice_panel(ui, p, world, st, actions, rail);
    }
    contacts_clicked
}

/// What the nearby chat's entry says: who can hear us, and its last line.
fn local_sub(world: &World, st: &ChatUi) -> String {
    if let Some(t) = super::typing::text(&st.nearby_typing_names(world)) {
        return t;
    }
    let near = aurora_i18n::tr!("ui-chat-in-earshot", n = st.nearby_count);
    match world.chat.iter().rev().find(|l| !l.kind.is_notice()) {
        Some(l) => format!("{near} · {}", preview(world, l, true)),
        None => near,
    }
}

/// The last message of a conversation in a few words: "Name: text", ours
/// "You: text", an emote as it reads; its links by their labels, as in the
/// conversation (#947), the entry eliding the result.
fn preview(world: &World, line: &crate::world::ChatLine, named: bool) -> String {
    let own = world.own_name();
    let name = super::sender_name(world, &own, line);
    let (text, _) = super::line_body(line, &name);
    let text = super::resolved_text(world, &text);
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    if emote || line.kind.is_notice() {
        text
    } else if line.kind == crate::world::ChatKind::Own {
        aurora_i18n::tr!("ui-chat-preview-you", text = text.as_str()).to_owned()
    } else if named && let Some(first) = name.split_whitespace().next() {
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
        .and_then(|s| s.lines.iter().rev().find(|l| !l.kind.is_card()));
    let last = last.map(|l| preview(world, l, kind != Kind::Private)).unwrap_or_default();
    if kind == Kind::Private && matches!(status, presence::Status::Away | presence::Status::Busy | presence::Status::Offline) {
        let label = status.label();
        return (
            if last.is_empty() {
                label.to_owned()
            } else {
                format!("{label} · {last}")
            },
            None,
        );
    }
    if last.is_empty() && kind != Kind::Private {
        let n = world.groups.sessions.get(&id).map(|s| s.participants.len()).unwrap_or(0);
        return (aurora_i18n::tr!("ui-chat-head-participants", n = n), None);
    }
    (last, None)
}

/// The title of a section: small capitals in semi-bold.
fn section_title(ui: &egui::Ui, p: &Palette, title: &str) -> RichText {
    RichText::new(title.to_uppercase())
        .font(egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())))
        .color(p.muted_dim)
}

/// The name of an entry: a resident's display name, else the session's
/// title (a group, a conference, a name not known yet).
/// The conversations of the column from top to bottom, as `show` lists
/// them (None: the nearby chat): those the search leaves, the sets shown
/// apart, the private messages, then the groups and the conferences.
pub(super) fn tab_order(world: &World, st: &ChatUi) -> Vec<Option<Uuid>> {
    let filter = st.side_filter.trim().to_lowercase();
    let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
    let sessions = st.reorder.sessions(world);
    let mut order: Vec<Option<Uuid>> = Vec::new();
    if matches(aurora_i18n::tr!("common-nearby-chat")) {
        order.push(None);
    }
    for section in sections(world) {
        order.extend(
            sessions
                .iter()
                .filter(|(id, _)| self::section(world, id) == section && matches(&entry_name(world, id, section.kind())))
                .map(|(id, _)| Some(*id)),
        );
    }
    order
}

pub(super) fn entry_name(world: &World, id: &Uuid, kind: Kind) -> String {
    if kind == Kind::Private && world.social.avatar_names.get(id).is_some() {
        return world.social.display_name_of(id);
    }
    world.session_title(id)
}

/// The head of a section (a conversation is started from the button beside
/// the search field, `new_conversation`); a set's is its name after its
/// colour's dot, as its pill in People › Contact sets.
fn section_head(ui: &mut egui::Ui, p: &Palette, world: &World, section: Section, rail: bool) {
    if rail {
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().hline(
            r.center().x - 12.0..=r.center().x + 12.0,
            r.center().y,
            egui::Stroke::new(1.0, p.raised),
        );
        ui.add_space(4.0);
        return;
    }
    let title = match section {
        Section::Set(i) => {
            let name = world.contact_sets.chat_sections().get(i).map_or("", String::as_str);
            let set = world.contact_sets.set(name);
            let icon = set.and_then(|s| s.visuals.icon).and_then(|i| ICONS.get(i).copied());
            let color = set.map_or(p.muted, |s| match icon {
                Some(_) => crate::ui::colors::set_color(s.color),
                None => crate::ui::contacts::color32(s.color),
            });
            set_head(ui, p, name, icon, color);
            return;
        }
        Section::Kind(Kind::Private) => aurora_i18n::tr!("ui-chat-section-private"),
        Section::Kind(Kind::Group) => aurora_i18n::tr!("ui-chat-section-groups"),
        Section::Kind(Kind::Conference) => aurora_i18n::tr!("ui-chat-section-conferences"),
    };
    plain_head(ui, p, title);
}

/// The head of a contact set's section: `plain_head` after the set's
/// symbol in its colour, or its colour's dot when it has none.
fn set_head(ui: &mut egui::Ui, p: &Palette, name: &str, icon: Option<&str>, color: Color32) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.add_space(1.0);
        match icon {
            Some(icon) => {
                let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(11.0), egui::Sense::hover());
                super::thread::paint_icon(ui.painter(), icon, r, color);
            }
            None => {
                let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(7.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
            }
        }
        ui.add(crate::ui::widgets::elided_label(
            ui,
            section_title(ui, p, name),
            ui.available_width() - 6.0,
        ));
    });
    ui.add_space(2.0);
}

fn plain_head(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(section_title(ui, p, title));
    });
    ui.add_space(2.0);
}

/// The entry being dragged, under the pointer, as the conversation shown:
/// its picture alone in the rail, and the mark of a window of its own
/// while a drop pops it out there (`reorder::Ghost`).
fn ghost(ctx: &egui::Context, p: &Palette, st: &ChatUi, e: Entry, rail: bool, pinned: Pinned) {
    let Some(g) = st.reorder.ghost(ctx) else { return };
    let e = Entry {
        current: true,
        unread: 0,
        close: None,
        flash: 0.0,
        popped: g.out,
        ..e
    };
    crate::ui::widgets::drag_ghost(ctx, p, "conversation_drag_ghost", g.pos, |ui| {
        ui.set_width(g.size.x);
        // the column's colour under it, which the entry's own face washes
        let face = egui::Rect::from_min_size(ui.cursor().min, g.size);
        ui.painter().rect_filled(face, ui.visuals().window_corner_radius, p.bar);
        entry(ui, p, &e, rail, entry_id(pinned).with("ghost"));
    });
}

/// One entry of the column (or of the rail: its picture alone, the name
/// in its tooltip).
fn entry(ui: &mut egui::Ui, p: &Palette, e: &Entry, rail: bool, id: egui::Id) -> Picked {
    let width = ui.available_width();
    let gap = crate::ui::widgets::two_lines_gap(ui, &egui::FontId::proportional(13.0), &egui::FontId::proportional(11.0), LINE_GAP);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ENTRY_H + gap - LINE_GAP), egui::Sense::hover());
    // a conversation's entry drags, to its place or out of the window; the
    // nearby chat's, pinned, out of the window (#910)
    let sense = if e.close.is_some() || id == entry_id(Pinned::Local) {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let resp = ui.interact(rect, id, sense);
    let over = resp.hovered() || (e.close.is_some() && ui.rect_contains_pointer(rect));
    let radius = ui.visuals().window_corner_radius;
    // the entry's face, opaque: the status dot is cut in it, and a
    // translucent cut let the picture and the crescent's disc through (#744)
    let mut face = p.bar;
    let mut wash = |color: Color32| {
        ui.painter().rect_filled(rect, radius, color);
        face = face.blend(color);
    };
    if e.current {
        wash(p.violet.gamma_multiply(0.22));
    } else if over {
        wash(p.raised.gamma_multiply(0.45));
    }
    if e.flash > 0.0 && !e.current {
        wash(p.violet.gamma_multiply(e.flash));
    }
    if e.unread > 0 && !e.current {
        // the mark of an unread conversation, against the column's edge
        let mark = egui::Rect::from_min_size(egui::pos2(rect.left() - 6.0, rect.center().y - 4.0), egui::vec2(3.0, 8.0));
        ui.painter()
            .rect_filled(mark, crate::ui::widgets::inner_radius(ui.visuals()), p.ink);
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
    paint_picture(ui, p, &e.icon, e.name, pic, face);
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
            ui.painter()
                .rect_filled(slot, ui.visuals().widgets.inactive.corner_radius, p.raised);
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
        if e.popped && e.unread == 0 {
            let r = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), egui::Vec2::splat(15.0));
            super::popout::paint_mark(ui, p, r);
            right = r.left() - 3.0;
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
    let name_color = e.name_color.unwrap_or(name_color);
    let font = if bold {
        egui::FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()))
    } else {
        egui::FontId::proportional(13.0)
    };
    let has_sub = e.sub.as_ref().is_some_and(|s| !s.is_empty());
    let name_y = if has_sub {
        rect.center().y - gap * 8.0 / LINE_GAP
    } else {
        rect.center().y
    };
    let sub_y = rect.center().y + gap * 9.0 / LINE_GAP;
    let cut = crate::ui::widgets::paint_elided(ui, e.name, egui::pos2(x, name_y), right - x, font, name_color);
    if let Some(sub) = e.sub.as_ref().filter(|s| !s.is_empty()) {
        let mut sx = x;
        if let Some((icon, color)) = e.sub_icon {
            let r = egui::Rect::from_min_size(egui::pos2(sx, sub_y - 6.0), egui::Vec2::splat(11.0));
            super::thread::paint_icon(ui.painter(), icon, r, color);
            sx += 14.0;
        }
        crate::ui::widgets::paint_elided(
            ui,
            sub,
            egui::pos2(sx, sub_y),
            right - sx,
            egui::FontId::proportional(11.0),
            p.muted_dim,
        );
    }
    let resp = if cut { resp.on_hover_text(tip_name()) } else { resp };
    Picked { resp, close }
}

/// The picture of an entry: a resident's with its status, a group's
/// insignia, a glyph on a tinted square.
pub(super) fn paint_picture(ui: &egui::Ui, p: &Palette, icon: &Icon, name: &str, rect: egui::Rect, ring: Color32) {
    let radius = ui.visuals().window_corner_radius;
    match icon {
        Icon::Avatar(pic, status) => {
            crate::ui::widgets::avatar_disc(ui, p, rect, *pic, name, false);
            if *status != presence::Status::Unknown {
                presence::paint_dot(ui.painter(), p, rect, *status, ring);
            }
        }
        Icon::Group(insignia) => paint_group_picture(ui, p, rect, *insignia, GLYPH),
        Icon::Glyph(glyph, color) => {
            ui.painter().rect_filled(rect, radius, color.gamma_multiply(0.16));
            super::thread::paint_icon(
                ui.painter(),
                glyph,
                egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(GLYPH)),
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

/// Height of the call panel, its 8 px padding included (the mockup's
/// `.vpanel`): the title, the conversation and its time, then the 28 px
/// buttons; in the rail the three buttons stacked.
fn voice_panel_h(rail: bool) -> f32 {
    if rail {
        8.0 + 3.0 * VOICE_BUTTON_H + 2.0 * 4.0 + 8.0
    } else {
        // padding, the title row (its 14 px glyph) and 1 px, the
        // conversation and its time, the mockup's 6 px, the buttons, padding
        8.0 + 17.0 + 15.0 + 6.0 + VOICE_BUTTON_H + 8.0
    }
}

/// The call panel's buttons (the mockup's `.vpanel .rb`).
const VOICE_BUTTON_H: f32 = 28.0;

/// Room under the call panel, the same as under the conversation's field.
const VOICE_PANEL_GAP: f32 = 12.0;

fn voice_panel(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut ChatUi, actions: &mut Vec<ConvAction>, rail: bool) {
    let Some(call) = world.calls.active.as_ref() else {
        return;
    };
    let conversation = call.conversation();
    let bottom = ui.max_rect().bottom() - VOICE_PANEL_GAP;
    let rect = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), bottom - voice_panel_h(rail)),
        egui::pos2(ui.max_rect().right(), bottom),
    );
    // the mockup's `.vpanel`: a darker card, 8 px inside
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.navy.gamma_multiply(0.6));
    ui.painter().rect_stroke(
        rect,
        ui.visuals().window_corner_radius,
        egui::Stroke::new(1.0, p.ink.gamma_multiply(0.05)),
        egui::StrokeKind::Inside,
    );
    // the text at the top of the card, the row of buttons 8 px from its
    // bottom, whatever the text's height (stacked in the rail)
    let inner = rect.shrink(8.0);
    let buttons_rect = if rail {
        inner
    } else {
        egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - VOICE_BUTTON_H), inner.max)
    };
    if !rail {
        let mut text = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(inner.min, egui::pos2(inner.right(), buttons_rect.top())))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut text;
        // the mockup's two lines held together (1 px apart): no minimum
        // height of an interactive row for the title
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 1.0);
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "phone-filled", p.success, 14.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-chat-call-connected"))
                    .size(12.0)
                    .strong()
                    .color(p.success),
            );
        });
        let secs = call.connected_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let text = format!("{} · {}", world.session_title(&conversation), super::call::duration(secs));
        ui.add(crate::ui::widgets::elided_label(
            ui,
            RichText::new(text).size(11.0).color(p.muted),
            ui.available_width(),
        ));
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(buttons_rect));
    let ui = &mut child;
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
    let layout = if rail {
        egui::Layout::top_down(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    ui.with_layout(layout, |ui| {
        let w = if rail {
            ui.available_width()
        } else {
            (ui.available_width() - 12.0) / 3.0
        };
        let size = egui::vec2(w, VOICE_BUTTON_H);
        if let Some(on) = st.mic {
            let (icon, tip) = if on {
                ("microphone-filled", aurora_i18n::tr!("ui-call-mute"))
            } else {
                ("microphone-off", aurora_i18n::tr!("ui-call-unmute"))
            };
            if square_button(ui, p, "panel-mic", icon, tip, size, !on).clicked() {
                actions.push(ConvAction::ToggleMic);
            }
        }
        if square_button(
            ui,
            p,
            "panel-back",
            "message-circle-filled",
            aurora_i18n::tr!("ui-chat-back-to-call"),
            size,
            false,
        )
        .clicked()
        {
            st.select_tab(Some(conversation));
        }
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let resp = ui.interact(rect, control_id("panel-hang-up"), egui::Sense::click());
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            if resp.hovered() { p.danger } else { p.danger_dim },
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
pub(super) fn square_button(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    icon: &str,
    tip: &str,
    size: egui::Vec2,
    off: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let resp = ui.interact(rect, control_id(name), egui::Sense::click());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn membership(id: Uuid, insignia: Uuid) -> aurora_net::GroupMembership {
        aurora_net::GroupMembership {
            id,
            name: "Group".into(),
            insignia,
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    /// A group conversation gets its group's insignia (ours, else the group
    /// profile the viewer has), the `users-group` glyph without one.
    #[test]
    fn group_entry_takes_the_insignia_or_the_glyph() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (with, without, outside, unknown) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let (picture, seen) = (Uuid::from_u128(0x10), Uuid::from_u128(0x11));
        world.groups.groups = vec![membership(with, picture), membership(without, Uuid::nil())];
        world.search.groups.insert(
            outside,
            aurora_net::search::GroupProfile {
                id: outside,
                insignia: seen,
                ..Default::default()
            },
        );
        assert_eq!(group_insignia(&world, &with), Some(picture));
        assert_eq!(group_insignia(&world, &without), None, "a nil insignia: the glyph");
        assert_eq!(group_insignia(&world, &outside), Some(seen), "a group we are not in: its profile's");
        assert_eq!(group_insignia(&world, &unknown), None);
    }
}
