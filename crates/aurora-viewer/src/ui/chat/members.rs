//! The members' panel of a group or conference conversation, at the right
//! of its messages: its moderators, its participants and the residents
//! invited who have not joined yet, each with their picture and status,
//! the speaker of who speaks in our call and the struck microphone of who a
//! moderator muted. A click opens the profile, a right click the resident's
//! menu. Firestorm's participant list of a session (LLParticipantList,
//! FSParticipantList), as a column.
//!
//! A conference also lists who is not in it: the residents it was started
//! with who never joined (FSFloaterIM's mInitialInvitedIDs) and those who
//! left. A role group's chat lists only who is in it (its whole roll would
//! be thousands of names), which the panel says; a conference and a
//! resident's conversation can take more people from its head.

use super::{ChatUi, ConvAction, presence};
use crate::theme::Palette;
use crate::world::World;
use egui::RichText;
use std::collections::HashMap;
use uuid::Uuid;

pub(super) const WIDTH: f32 = 210.0;
const ROW_H: f32 = 34.0;
const PIC: f32 = 26.0;

struct Member {
    id: Uuid,
    name: String,
    status: presence::Status,
    speaking: bool,
    muted: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    conversation: Uuid,
    // WIDTH, or less while the panel eases in or out with the window's size
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    // the mockup's `.members`: the bar's colour at 55 %, a hairline at its left
    ui.painter().rect_filled(rect, 0.0, p.bar.gamma_multiply(0.55));
    ui.painter().vline(
        rect.left() + 0.5,
        rect.y_range(),
        egui::Stroke::new(1.0, p.ink.gamma_multiply(0.05)),
    );
    // its content keeps the panel's whole width, cut at the window's edge
    // while the panel eases: the names do not re-wrap as it moves
    let full = egui::Rect::from_min_size(rect.min, egui::vec2(WIDTH, height));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(full.shrink2(egui::vec2(8.0, 4.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(child.clip_rect()));
    let ui = &mut child;
    let info = world.groups.sessions.get(&conversation);
    let call = world.call_of(&conversation);
    let member = |id: &Uuid| {
        let in_call = call.and_then(|c| c.members.iter().find(|m| m.id == *id));
        Member {
            id: *id,
            name: world.social.display_name_of(id),
            status: if *id == world.agent_id {
                presence::Status::Online
            } else {
                presence::of(world, id)
            },
            speaking: in_call.is_some_and(|m| m.speaking),
            muted: in_call.is_some_and(|m| m.muted),
        }
    };
    let mut moderators = Vec::new();
    let mut participants = Vec::new();
    if let Some(info) = info {
        for id in &info.participants {
            st.wanted_names.insert(*id);
            let m = member(id);
            if info.moderators.contains(id) {
                moderators.push(m);
            } else {
                participants.push(m);
            }
        }
    }
    let present = |id: &Uuid| info.is_some_and(|i| i.participants.contains(id));
    let pending: Vec<Uuid> = world
        .calls
        .invited
        .get(&conversation)
        .into_iter()
        .flatten()
        .copied()
        .filter(|id| !present(id))
        .collect();
    let mut invited: Vec<Member> = pending.iter().map(member).collect();
    let role_group = info.is_some_and(|i| i.group) || world.groups.is_member(&conversation);
    let mut offline: Vec<Member> = if role_group {
        Vec::new()
    } else {
        offline_ids(world, conversation, &pending).iter().map(member).collect()
    };
    for list in [&mut moderators, &mut participants, &mut invited, &mut offline] {
        list.sort_by_key(|m| m.name.to_lowercase());
    }
    egui::ScrollArea::vertical()
        .id_salt(("members", conversation))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if moderators.is_empty() && participants.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-chat-loading-participants"))
                        .size(11.5)
                        .color(p.muted),
                );
            }
            if super::head::can_add_people(world, &conversation) {
                ui.add_space(8.0);
                let size = egui::vec2(ui.available_width(), 26.0);
                let label = aurora_i18n::tr!("ui-call-add");
                let add = crate::ui::widgets::action_button(
                    ui,
                    p,
                    super::head::ADD_PEOPLE_ICON,
                    label,
                    crate::ui::widgets::ActionKind::Flat,
                    size,
                );
                if add.on_hover_text(aurora_i18n::tr!("ui-call-add-people")).clicked() {
                    actions.push(ConvAction::PickPeople(conversation));
                }
            }
            let sections = [
                (aurora_i18n::tr!("ui-chat-members-moderators"), &moderators, false),
                (aurora_i18n::tr!("ui-chat-members-participants"), &participants, false),
                (aurora_i18n::tr!("ui-chat-members-invited"), &invited, true),
                (aurora_i18n::tr!("ui-chat-members-offline"), &offline, true),
            ];
            for (title, list, dim) in sections {
                if list.is_empty() {
                    continue;
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("{} — {}", title.to_uppercase(), list.len()))
                        .font(egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())))
                        .color(p.muted_dim),
                );
                ui.add_space(4.0);
                for m in list.iter() {
                    row(
                        ui,
                        p,
                        pics,
                        world,
                        st,
                        actions,
                        m,
                        dim,
                        info.is_some_and(|i| i.moderators.contains(&m.id)),
                    );
                }
            }
            if role_group {
                ui.add_space(10.0);
                ui.add(egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-chat-members-present-only"))
                        .size(11.0)
                        .color(p.muted_dim),
                ));
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    m: &Member,
    dim: bool,
    moderator: bool,
) {
    st.wanted_pics.insert(m.id);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
    let resp = ui.interact(rect, member_id(m.id), egui::Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.45));
    }
    let pic = egui::Rect::from_min_size(egui::pos2(rect.left() + 4.0, rect.center().y - PIC * 0.5), egui::Vec2::splat(PIC));
    crate::ui::widgets::avatar_disc(ui, p, pic, pics.get(&m.id), &m.name, dim);
    if m.status != presence::Status::Unknown {
        presence::paint_dot(ui.painter(), p, pic, m.status, p.bar);
    }
    let mut right = rect.right() - 4.0;
    let icon = if m.muted {
        Some(("microphone-off", p.muted_dim))
    } else if m.speaking {
        Some(("volume", p.success))
    } else {
        None
    };
    if let Some((icon, color)) = icon {
        let r = egui::Rect::from_center_size(egui::pos2(right - 8.0, rect.center().y), egui::Vec2::splat(15.0));
        super::thread::paint_icon(ui.painter(), icon, r, color);
        right = r.left() - 4.0;
    }
    let x = pic.right() + 8.0;
    let color = if dim {
        p.muted_dim
    } else if moderator {
        p.violet_light
    } else if m.id == world.agent_id {
        p.teal
    } else if m.status == presence::Status::Online || m.speaking {
        p.ink
    } else {
        p.muted
    };
    let cut = crate::ui::widgets::paint_elided(
        ui,
        &m.name,
        egui::pos2(x, rect.center().y),
        right - x,
        egui::FontId::proportional(12.5),
        color,
    );
    super::name_menu(&resp, p, world, m.id);
    let tip = if moderator {
        aurora_i18n::tr!("ui-chat-moderator-click-profile")
    } else {
        aurora_i18n::tr!("ui-chat-click-profile")
    };
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = if cut {
        resp.on_hover_text(format!("{}\n{tip}", m.name))
    } else {
        resp.on_hover_text(tip)
    };
    if resp.clicked() {
        actions.push(ConvAction::Profile(m.id));
    }
}

/// The offline members of a conference: those it was started with who are
/// not in it (and not still being asked), and those who left; never us.
fn offline_ids(world: &World, conversation: Uuid, pending: &[Uuid]) -> Vec<Uuid> {
    let info = world.groups.sessions.get(&conversation);
    let present = |id: &Uuid| info.is_some_and(|i| i.participants.contains(id));
    let mut out: Vec<Uuid> = world
        .calls
        .conferences
        .get(&conversation)
        .into_iter()
        .flatten()
        .chain(info.into_iter().flat_map(|i| i.left.iter()))
        .copied()
        .filter(|id| *id != world.agent_id && !present(id) && !pending.contains(id))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The id of a member's row.
pub(super) fn member_id(id: Uuid) -> egui::Id {
    super::sidebar::control_id(&format!("member-{id}"))
}
