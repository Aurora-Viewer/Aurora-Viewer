//! Aurora's workspace (the conversations' "Aurora" and "Compact" styles,
//! the human's choice of 2026-10-15): the window of conversations laid
//! out as a desktop chat. The column of conversations (`sidebar.rs`) at
//! the left; at the right the conversation shown: its head (`head.rs`),
//! its call (`stage.rs`), its messages grouped by author or in compact rows
//! (`thread.rs`, on the virtualised view of `view.rs`) with the members'
//! panel of a group or conference beside them (`members.rs`), the find
//! bar, then the field (`compose.rs`). FSFloaterIMContainer and its
//! FSFloaterIM / FSFloaterNearbyChat panels: the same conversations and
//! actions, another layout.

use super::{
    ChatUi, ConvAction, Lines, Rows, ToField, compose, conversation_lines, find, head, mark_read, members, sidebar, stage, thread,
};
use crate::theme::Palette;
use crate::world::{ChatKind, ChatLine, World};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

/// Below this width the column becomes a rail of pictures.
pub(super) const RAIL_BELOW: f32 = 600.0;
/// The members' panel shows when the messages keep at least this width.
const MESSAGES_MIN: f32 = 360.0;
/// How far past a threshold a window must go back before the layout
/// switches back (the human's report of 2026-10-16: resized around 600
/// points the column and the rail swapped at every point).
const HYSTERESIS: f32 = 40.0;
/// Time the column and the members' panel take to reach their new width
/// once the window's size crossed a threshold: the messages slide instead
/// of jumping 170 or 210 points in one frame.
const LAYOUT_EASE: f32 = 0.15;

/// A width easing to `target` in `LAYOUT_EASE`, linearly. It moves on the
/// frame its target changes, a frame's step already: egui's
/// `animate_value_with_time` gives the old value then, so while the window
/// was dragged narrower past a threshold the messages lost both the window's
/// step and none of the column's, and wrapped again for that one frame.
fn ease(ctx: &egui::Context, id: egui::Id, target: f32) -> f32 {
    let (now, dt) = ctx.input(|i| (i.time, f64::from(i.stable_dt.min(0.1))));
    // (from, to, start)
    let Some((from, to, start)) = ctx.data(|d| d.get_temp::<(f32, f32, f64)>(id)) else {
        ctx.data_mut(|d| d.insert_temp(id, (target, target, now)));
        return target;
    };
    let at = |from: f32, to: f32, start: f64| from + (to - from) * ((now - start) / f64::from(LAYOUT_EASE)).clamp(0.0, 1.0) as f32;
    let (from, start) = if to == target {
        (from, start)
    } else {
        let here = at(from, to, start);
        ctx.data_mut(|d| d.insert_temp(id, (here, target, now - dt)));
        (here, now - dt)
    };
    let value = at(from, target, start);
    if value != target {
        ctx.request_repaint();
    }
    value
}

/// The column is a rail at this window width, given what it was last frame.
fn rail_now(was: Option<bool>, width: f32) -> bool {
    match was {
        Some(true) => width < RAIL_BELOW + HYSTERESIS,
        _ => width < RAIL_BELOW,
    }
}

/// The messages leave room for the members' panel at this pane width,
/// given what they did last frame.
fn members_room_now(was: Option<bool>, width: f32) -> bool {
    let left = width - members::WIDTH;
    match was {
        Some(true) => left >= MESSAGES_MIN - HYSTERESIS,
        _ => left >= MESSAGES_MIN,
    }
}

/// What the workspace draws with.
pub(super) struct Env<'a> {
    pub p: &'a Palette,
    pub icons: &'a crate::ui::icons::Icons,
    pub emoji: &'a mut crate::ui::emoji::Emoji,
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub gestures: &'a crate::gestures::Manager,
    pub contacts: &'a mut crate::ui::contacts::ContactsUi,
    pub contacts_settings: &'a mut crate::ui::contacts::ContactsSettings,
    pub show_contacts_tab: bool,
    /// People's friends tab, the Contacts tab in Aurora's look of People;
    /// None: the contact list.
    pub friends: Option<crate::ui::people::FriendsPane<'a, 'a>>,
    pub docked: bool,
    pub compact: bool,
}

pub(super) fn show(ui: &mut egui::Ui, env: Env, world: &mut World, st: &mut ChatUi, actions: &mut Vec<ConvAction>) {
    let Env {
        p,
        icons,
        emoji,
        pics,
        images,
        gestures,
        contacts,
        contacts_settings,
        show_contacts_tab,
        friends,
        docked,
        compact,
    } = env;
    let size = ui.available_size();
    let h = size.y.max(160.0);
    let rail_wanted = rail_now(st.layout_rail, size.x);
    st.layout_rail = Some(rail_wanted);
    let column = ease(
        ui.ctx(),
        ui.id().with("column_width"),
        if rail_wanted { sidebar::RAIL } else { sidebar::WIDTH },
    );
    // the entries take the rail's look once the column is closer to it
    let rail = column < (sidebar::RAIL + sidebar::WIDTH) / 2.0;
    // the window moves by its title bar only: a drag on the workspace's
    // background (the column, the head, the strips) leaves it in place, as
    // one on the lines (#427)
    let all = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(size.x, h));
    ui.interact(all, ui.id().with("workspace_still"), egui::Sense::drag());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let contacts_sub = if friends.is_some() {
            sidebar::ContactsSub::Friends
        } else {
            sidebar::ContactsSub::Lists
        };
        sidebar::show(
            ui,
            p,
            pics,
            world,
            st,
            actions,
            show_contacts_tab,
            contacts_sub,
            docked,
            rail,
            column,
            h,
        );
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.set_height(h);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            if st.contacts {
                let pane = ui.available_rect_before_wrap();
                let pane = egui::Rect::from_min_size(pane.min, egui::vec2(pane.width(), h));
                contacts_pane(ui, p, pane, |ui| match friends {
                    Some(friends) => {
                        for a in crate::ui::people::friends_pane(ui, p, world, friends, contacts, contacts_settings) {
                            actions.push(ConvAction::People(a));
                        }
                    }
                    None => {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                        let pics = crate::ui::contacts::Pics { avatars: pics, images };
                        for a in crate::ui::contacts::panel(ui, p, world, contacts, contacts_settings, &pics) {
                            actions.push(ConvAction::Contacts(a));
                        }
                    }
                });
                return;
            }
            pane(ui, p, icons, emoji, pics, gestures, world, st, actions, compact, rail, h);
        });
    });
}

/// The Contacts tab's pane, at the right of the column: the conversation
/// pane's ground, and its content over the whole height (inside a row of
/// the layout, the lists took the height of one line).
fn contacts_pane(ui: &mut egui::Ui, p: &Palette, pane: egui::Rect, content: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_rect(pane, egui::Sense::hover());
    let corner = ui.visuals().window_corner_radius;
    let radius = egui::CornerRadius {
        se: corner.se,
        ..egui::CornerRadius::ZERO
    };
    ui.painter().rect_filled(pane, radius, p.field.gamma_multiply(0.55));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("conversation_contacts")
            .max_rect(pane.shrink2(egui::vec2(CONTACTS_MARGIN.x, CONTACTS_MARGIN.y)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    content(&mut child);
}

/// The Contacts pane's margins: the conversation's head and lines keep
/// about these.
const CONTACTS_MARGIN: egui::Vec2 = egui::vec2(12.0, 10.0);

/// The conversation shown, at the right of the column.
#[allow(clippy::too_many_arguments)]
fn pane(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &crate::ui::icons::Icons,
    emoji: &mut crate::ui::emoji::Emoji,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    gestures: &crate::gestures::Manager,
    world: &mut World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    compact: bool,
    rail: bool,
    h: f32,
) {
    let pane = ui.available_rect_before_wrap();
    let pane = egui::Rect::from_min_size(pane.min, egui::vec2(pane.width(), h));
    // the mockup's `.main`: the field's colour at 55 %, its bottom right
    // corner the window's
    let corner = ui.visuals().window_corner_radius;
    let radius = egui::CornerRadius {
        se: corner.se,
        ..egui::CornerRadius::ZERO
    };
    ui.painter().rect_filled(pane, radius, p.field.gamma_multiply(0.55));
    let width = pane.width();
    let kind = st.selected.map(|id| sidebar::kind(world, &id));
    drops(ui, p, pane, st, kind, actions);
    let members_room = !rail && members_room_now(st.layout_members_room, width);
    st.layout_members_room = Some(members_room);
    // the panel's width eases to its new one when the room comes or goes
    // with the window's size; the head's button shows or hides it at once
    let room_w = ease(
        ui.ctx(),
        ui.id().with("members_width"),
        if members_room { members::WIDTH } else { 0.0 },
    );
    {
        let w: &World = world;
        head::show(ui, p, pics, w, st, actions, members_room);
        stage::show(ui, p, pics, w, st, actions);
    }
    // the filter field, while the filter is on (the "…" menu)
    let search = st.searches.entry(st.selected).or_default();
    if let Some(q) = search.filter.as_mut() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(
                egui::TextEdit::singleline(q)
                    .id(find::filter_id())
                    .hint_text(aurora_i18n::tr!("ui-chat-filter"))
                    .desired_width(width - 24.0),
            );
        });
    }
    let conv_input = egui::Id::new(("conv_input", st.selected));
    if find::wants_shortcut(ui, &[conv_input, find::field_id(), find::filter_id()]) {
        search.open_find();
    }
    // (the find field is in the head's row: the messages keep their height)
    let lines_h = (pane.bottom() - ui.cursor().top() - compose::HEIGHT).max(60.0);
    // ---- the messages, and the members beside them
    let read = mark_read(world, st.selected);
    note_new_line(ui, world, st, read);
    let w: &World = world;
    let own_name = w.own_name();
    let me = our_names(w, &own_name);
    let moderators = st.selected.and_then(|id| w.groups.sessions.get(&id)).map(|s| &s.moderators);
    let mut asked = Vec::new();
    let members_shown = st.members_open && matches!(kind, Some(k) if k != sidebar::Kind::Private) && room_w >= 1.0;
    let members_w = if members_shown { room_w } else { 0.0 };
    let lines_w = width - members_w;
    let mut to_field = ToField::Nothing;
    let body_top = ui.cursor().top();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(lines_w);
            let mut th = thread::Thread {
                compact,
                times: st.timestamps,
                moderators,
                me: &me,
                out: &mut asked,
            };
            let lines = Lines {
                icons,
                emoji,
                pics,
                world: w,
                own_name: &own_name,
                height: lines_h,
            };
            to_field = conversation_lines(ui, p, lines, st, Rows::Thread(&mut th));
        });
        if members_shown && let Some(id) = st.selected {
            members::show(ui, p, pics, w, st, actions, id, members_w, lines_h);
        }
    });
    ui.set_min_height(0.0);
    let after_lines = body_top + lines_h;
    let mut quote = None;
    for a in asked {
        match a {
            thread::ThreadAction::Quote(q) => quote = Some(q),
        }
    }
    match to_field {
        ToField::Nothing => {}
        ToField::Focus => st.conv_focus_request = true,
        ToField::Text(t) => {
            st.drafts.entry(st.selected).or_default().push_str(&t);
            st.conv_focus_request = true;
        }
    }
    // ---- the field
    let strip_top = after_lines.max(ui.cursor().top() - 1.0);
    compose::show(ui, p, icons, emoji, gestures, w, st, actions, quote);
    // the strip between the messages and the box keeps the normal cursor
    // unless a drag goes through it (#422)
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && pos.y > strip_top
        && pos.y < strip_top + 18.0
        && pane.x_range().contains(pos.x)
        && !ui.input(|i| i.pointer.any_down())
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
}

/// Drops on the conversation: a resident joins it (LLFloaterIMSession::
/// handleDragAndDrop, DAD_PERSON / DAD_CALLINGCARD); inventory is given to
/// the resident of an IM (DAD_* items → LLGiveInventory).
fn drops(ui: &mut egui::Ui, p: &Palette, pane: egui::Rect, st: &ChatUi, kind: Option<sidebar::Kind>, actions: &mut Vec<ConvAction>) {
    let Some(conversation) = st.selected else {
        return;
    };
    let outline = |ui: &egui::Ui| {
        ui.painter().rect_stroke(
            pane.shrink(1.0),
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.5, p.violet),
            egui::StrokeKind::Inside,
        );
    };
    let zone = ui.interact(pane, ui.id().with("person_drop"), egui::Sense::hover());
    if zone.dnd_hover_payload::<crate::ui::call_invite::PersonDrag>().is_some() {
        outline(ui);
    }
    if let Some(person) = zone.dnd_release_payload::<crate::ui::call_invite::PersonDrag>() {
        actions.push(ConvAction::AddPeople(conversation, vec![person.0]));
    }
    if kind == Some(sidebar::Kind::Private)
        && let Some((items, released)) = crate::ui::inventory::drag::give_target(ui, pane)
    {
        outline(ui);
        if released && !items.is_empty() {
            actions.push(ConvAction::GiveItems { to: conversation, items });
        }
    }
}

/// The conversation shown is in sight: its "New" line (`NewLines`).
fn note_new_line(ui: &egui::Ui, world: &World, st: &mut ChatUi, read: Option<(usize, usize)>) {
    let (first, lines) = match st.selected {
        None => (world.chat_clock.first, &world.chat),
        Some(id) => match world.social.ims.iter().find(|s| s.other == id) {
            Some(s) => (s.clock.first, &s.lines),
            None => return,
        },
    };
    let unread = read.map_or(0, |(unread, _)| unread);
    let looked = ui.input(|i| i.focused);
    st.new_lines
        .frame(st.selected, ui.ctx().cumulative_frame_nr(), looked, first, lines, unread);
}

/// Where the "New" line of each conversation sits (the human's rule of
/// 2026-10-16): it marks what arrived while we were away from the
/// conversation, and stays where it is while we read and write there. Away
/// means not looking at it: another conversation or the Contacts tab
/// shown, the window closed or minimised, the viewer without the focus.
/// Coming back places it before the first line from someone else after
/// the last one seen, or removes it when nothing arrived.
#[derive(Default)]
pub(super) struct NewLines {
    /// The number the next line of each conversation had when it was last
    /// in sight.
    seen: HashMap<Option<Uuid>, u64>,
    /// The "New" line of each: above this line's number.
    at: HashMap<Option<Uuid>, u64>,
    /// The conversation in sight, the frame it was last, and whether the
    /// viewer had the focus then.
    present: Option<(Option<Uuid>, u64, bool)>,
}

impl NewLines {
    /// Nothing seen yet: the next sight of each conversation puts its
    /// "New" line above its unread lines (a demo's lines, given at once).
    pub(super) fn forget(&mut self) {
        *self = NewLines::default();
    }

    /// The line the "New" line of `conv` is above.
    pub(super) fn at(&self, conv: Option<Uuid>) -> Option<u64> {
        self.at.get(&conv).copied()
    }

    /// A frame showing `conv`, whose lines are numbered from `first`;
    /// `looked`: the viewer has the focus; `unread`: its lines nobody saw,
    /// for a conversation not yet seen since the login.
    pub(super) fn frame(&mut self, conv: Option<Uuid>, frame: u64, looked: bool, first: u64, lines: &VecDeque<ChatLine>, unread: usize) {
        let next = first + lines.len() as u64;
        let stayed = matches!(self.present, Some((c, f, _)) if c == conv && (f == frame || f + 1 == frame));
        let refocused = looked && matches!(self.present, Some((_, _, false)));
        if !stayed || refocused {
            let from = self
                .seen
                .get(&conv)
                .copied()
                .unwrap_or(next.saturating_sub(unread as u64))
                .max(first);
            // (without the focus, what is shown is not seen yet)
            self.seen.insert(conv, from);
            let mine = |seq: u64| lines.get((seq - first) as usize).is_none_or(|l| l.kind == ChatKind::Own);
            match (from..next).find(|&seq| !mine(seq)) {
                Some(seq) => {
                    self.at.insert(conv, seq);
                }
                None => {
                    self.at.remove(&conv);
                }
            }
        }
        if looked {
            self.seen.insert(conv, next);
        }
        self.present = Some((conv, frame, looked));
    }
}

/// Our names, lowercase, whose mention marks a message: the name we show
/// and our user name (a first name alone would mark too much: "Aurora").
fn our_names(world: &World, own_name: &str) -> Vec<String> {
    let mut me = vec![own_name.to_lowercase()];
    let names = &world.social.avatar_names;
    if let Some(n) = names.get(&world.agent_id) {
        me.push(n.display(&names.options).to_lowercase());
        if let Some(user) = n.chat_username(&names.options) {
            me.push(user.to_lowercase());
        }
    }
    me.sort();
    me.dedup();
    me
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
