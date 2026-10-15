//! The members' panel of a group or conference conversation, at the right
//! of its messages: its moderators, its participants and the residents
//! invited who have not joined yet, each with their picture and status,
//! the speaker of who speaks in our call and the struck microphone of who a
//! moderator muted. A click opens the profile, a right click the resident's
//! menu. Firestorm's participant list of a session (LLParticipantList,
//! FSParticipantList), as a column.
//!
//! A session only lists who is in it: its offline members are not known
//! (Firestorm shows none either).

use super::{ChatUi, ConvAction, presence};
use crate::theme::Palette;
use crate::world::World;
use egui::RichText;
use std::collections::HashMap;
use uuid::Uuid;

pub(super) const WIDTH: f32 = 210.0;
const ROW_H: f32 = 32.0;
const PIC: f32 = 24.0;

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
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, height), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.bar.gamma_multiply(0.45));
    ui.painter().vline(
        rect.left() + 0.5,
        rect.y_range(),
        egui::Stroke::new(1.0, p.raised.gamma_multiply(0.6)),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8.0, 4.0))).layout(egui::Layout::top_down(egui::Align::Min)));
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
    let invited: Vec<Member> = world
        .calls
        .invited
        .get(&conversation)
        .into_iter()
        .flatten()
        .filter(|id| !info.is_some_and(|i| i.participants.contains(id)))
        .map(member)
        .collect();
    for list in [&mut moderators, &mut participants] {
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
            let sections = [
                (aurora_i18n::tr!("ui-chat-members-moderators"), &moderators, false),
                (aurora_i18n::tr!("ui-chat-members-participants"), &participants, false),
                (aurora_i18n::tr!("ui-chat-members-invited"), &invited, true),
            ];
            for (title, list, dim) in sections {
                if list.is_empty() {
                    continue;
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("{} — {}", title.to_uppercase(), list.len()))
                        .size(10.5)
                        .strong()
                        .color(p.muted_dim),
                );
                ui.add_space(4.0);
                for m in list.iter() {
                    row(ui, p, pics, world, st, actions, m, dim, info.is_some_and(|i| i.moderators.contains(&m.id)));
                }
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
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::click());
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
