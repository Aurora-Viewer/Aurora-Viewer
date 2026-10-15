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
use crate::world::World;
use std::collections::HashMap;
use uuid::Uuid;

/// Below this width the column becomes a rail of pictures.
pub(super) const RAIL_BELOW: f32 = 600.0;
/// The members' panel shows when the messages keep at least this width.
const MESSAGES_MIN: f32 = 360.0;

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
        docked,
        compact,
    } = env;
    let size = ui.available_size();
    let h = size.y.max(160.0);
    let rail = size.x < RAIL_BELOW;
    // the conversation left: its "New" line goes
    if st.shown != Some(st.selected) {
        if let Some(before) = st.shown {
            st.first_unread.remove(&before);
        }
        st.shown = Some(st.selected);
    }
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        sidebar::show(ui, p, pics, world, st, actions, show_contacts_tab, docked, rail, h);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.set_height(h);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            if st.contacts {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                        let pics = crate::ui::contacts::Pics { avatars: pics, images };
                        for a in crate::ui::contacts::panel(ui, p, world, contacts, contacts_settings, &pics) {
                            actions.push(ConvAction::Contacts(a));
                        }
                    });
                });
                return;
            }
            pane(ui, p, icons, emoji, pics, gestures, world, st, actions, compact, rail, h);
        });
    });
}

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
    ui.painter().rect_filled(pane, 0.0, p.field.gamma_multiply(0.55));
    let width = pane.width();
    let kind = st.selected.map(|id| sidebar::kind(world, &id));
    drops(ui, p, pane, st, kind, actions);
    let members_room = !rail && width - members::WIDTH >= MESSAGES_MIN;
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
    let find_h = if search.find.is_some() { find::BAR_H + 8.0 } else { 0.0 };
    let lines_h = (pane.bottom() - ui.cursor().top() - compose::HEIGHT - find_h).max(60.0);
    // ---- the messages, and the members beside them
    let read = mark_read(world, st.selected);
    note_first_unread(world, st, read);
    let w: &World = world;
    let own_name = w.own_name();
    let me = our_names(w, &own_name);
    let moderators = st.selected.and_then(|id| w.groups.sessions.get(&id)).map(|s| &s.moderators);
    let mut asked = Vec::new();
    let members_shown = members_room && st.members_open && matches!(kind, Some(k) if k != sidebar::Kind::Private);
    let lines_w = if members_shown { width - members::WIDTH } else { width };
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
            members::show(ui, p, pics, w, st, actions, id, lines_h);
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
    // ---- the find bar
    let search = st.searches.entry(st.selected).or_default();
    let view = st.views.entry(st.selected).or_default();
    if search.find.is_some() {
        let mut close = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(width - 24.0);
                if let Some(f) = search.find.as_mut() {
                    close = find::bar(ui, p, f, view);
                }
            });
        });
        ui.add_space(4.0);
        if close {
            search.find = None;
            ui.ctx().memory_mut(|m| m.request_focus(conv_input));
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

/// The conversation shown was read: where its unread lines began, for
/// its "New" line (kept until it is left or we answer).
fn note_first_unread(world: &World, st: &mut ChatUi, read: Option<(usize, usize)>) {
    let Some((unread, len)) = read else {
        return;
    };
    if unread == 0 {
        return;
    }
    let first = match st.selected {
        None => world.chat_clock.first,
        Some(id) => match world.social.ims.iter().find(|s| s.other == id) {
            Some(s) => s.clock.first,
            None => return,
        },
    };
    let seq = first + len.saturating_sub(unread) as u64;
    st.first_unread.entry(st.selected).or_insert(seq);
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
