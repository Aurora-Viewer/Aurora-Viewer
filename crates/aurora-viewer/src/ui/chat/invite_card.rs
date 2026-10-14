//! The card of a teleport offer or request in the conversation with its
//! sender, Firestorm's notification panel in an IM (LLIMToastNotifyPanel,
//! fschathistory.cpp): what it is, where it leads, the sender's message and
//! the buttons, then its answer once given. A button goes through
//! `World::answer_teleport_invite`, like the entry of the notification
//! centre: whichever answers first closes the entry and leaves the card
//! with its answer, greyed.
//!
//! With the bubbles, a card on the sender's side (teal edge, Tabler icon,
//! the accent on the expected answer); with the legacy lines, one line with
//! its buttons at the end.

use super::{LineEnv, chat_text_marked, hhmm};
use crate::theme::Palette;
use crate::ui::context::{CtxAction, request};
use crate::ui::widgets;
use crate::world::ChatLine;
use crate::world::teleport_invites::{Answer, Invite, InviteKind};
use egui::{RichText, Vec2};

/// Width of a card at most.
const CARD_WIDTH: f32 = 300.0;
const BUTTON: Vec2 = Vec2::new(88.0, 24.0);
/// The icon of teleport offers, the same as in the notification centre.
const ICON: &str = "plane-departure-filled";

/// What a card says, from its invitation.
struct Texts {
    title: String,
    place: Option<String>,
    message: String,
    /// The question of a request ("Offer a teleport?").
    question: Option<String>,
    /// The two buttons: (accept, refuse).
    buttons: (String, String),
}

fn texts(env: &LineEnv, line: &ChatLine, v: &Invite) -> Texts {
    let name = env.world.person_name(&v.from).unwrap_or_else(|| line.from.clone());
    match &v.kind {
        InviteKind::Offer { place, .. } => Texts {
            title: aurora_i18n::tr!("world-mod-offers-you-a-teleport", from = name),
            place: place.as_ref().and_then(|p| p.label()),
            message: v.message.clone(),
            question: None,
            buttons: (
                aurora_i18n::tr!("common-teleport-3").into(),
                aurora_i18n::tr!("ui-notifications-decline").into(),
            ),
        },
        InviteKind::Request => Texts {
            title: aurora_i18n::tr!("world-teleport-invites-requests-a-teleport", from = name),
            place: None,
            message: v.message.clone(),
            question: Some(aurora_i18n::tr!("world-teleport-invites-offer-a-teleport").into()),
            buttons: (aurora_i18n::tr!("common-yes").into(), aurora_i18n::tr!("common-no").into()),
        },
    }
}

fn answer_label(answer: Answer) -> Option<&'static str> {
    match answer {
        Answer::Pending => None,
        Answer::Accepted => Some(aurora_i18n::tr!("ui-chat-invite-accepted")),
        Answer::Declined => Some(aurora_i18n::tr!("ui-chat-invite-declined")),
    }
}

fn icon(ui: &mut egui::Ui, env: &LineEnv, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    if let Some(t) = env.icons.get(ICON) {
        ui.painter().image(
            t.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

/// The two buttons of a pending invitation, sent as a request of the
/// interface (the app answers it).
fn buttons(ui: &mut egui::Ui, p: &Palette, invite: u64, (yes, no): &(String, String), size: Vec2) {
    if widgets::primary_button_sized(ui, p, yes, size).clicked() {
        request(ui.ctx(), CtxAction::AnswerTeleport { invite, accept: true });
    }
    if widgets::flat_button_sized(ui, p, no, size).clicked() {
        request(ui.ctx(), CtxAction::AnswerTeleport { invite, accept: false });
    }
}

/// The card, on the sender's side after `indent` (the column of the
/// senders' pictures).
pub(super) fn draw_card(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, invite: u64, indent: f32) {
    let p = env.p;
    let Some(v) = env.world.teleport_invites.get(invite).cloned() else {
        // the invitation was dropped (old and answered): its text only
        notice(ui, env, line);
        return;
    };
    let t = texts(env, line, &v);
    let done = answer_label(v.answer);
    let inset = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - inset * 2.0 - indent).clamp(160.0, CARD_WIDTH);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(inset + indent);
        let frame = egui::Frame::new()
            .fill(p.bar)
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin {
                left: 11,
                right: 11,
                top: 9,
                bottom: 9,
            })
            .show(ui, |ui| {
                ui.set_width(width - 22.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let (ink, muted) = if done.is_some() { (p.muted, p.muted_dim) } else { (p.ink, p.muted) };
                    ui.horizontal(|ui| {
                        icon(ui, env, if done.is_some() { p.muted_dim } else { p.teal });
                        ui.add(egui::Label::new(RichText::new(&t.title).size(13.0).strong().color(ink)).wrap());
                    });
                    if let Some(place) = &t.place {
                        ui.label(RichText::new(place).size(12.0).color(muted));
                    }
                    if !t.message.is_empty() {
                        let world = env.world;
                        chat_text_marked(ui, p, env.emoji, world, env.want_names, &t.message, 12.5, ink, false, None);
                    }
                    if let Some(q) = &t.question {
                        ui.label(RichText::new(q).size(12.0).color(muted));
                    }
                    ui.add_space(5.0);
                    ui.horizontal(|ui| match done {
                        None => buttons(ui, p, invite, &t.buttons, BUTTON),
                        Some(label) => {
                            ui.label(RichText::new(label).size(12.0).color(p.muted_dim));
                        }
                    });
                });
            });
        // the teal edge of the cards (mock-up C), grey once answered
        let r = frame.response.rect;
        let edge = if done.is_some() { p.muted_dim } else { p.teal };
        ui.painter().line_segment(
            [
                egui::pos2(r.left() + 1.0, r.top() + 1.0),
                egui::pos2(r.left() + 1.0, r.bottom() - 1.0),
            ],
            egui::Stroke::new(2.0, edge),
        );
    });
    let time = hhmm(line.time);
    if !time.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(inset + indent + 2.0);
            ui.label(RichText::new(time).size(10.5).color(p.muted_dim));
        });
    }
    ui.add_space(4.0);
}

/// The legacy lines: the card's texts on one line, its buttons (or its
/// answer) at the end.
pub(super) fn draw_line(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, invite: u64) {
    let p = env.p;
    let Some(v) = env.world.teleport_invites.get(invite).cloned() else {
        notice(ui, env, line);
        return;
    };
    let t = texts(env, line, &v);
    let done = answer_label(v.answer);
    let mut text = t.title.clone();
    if let Some(place) = &t.place {
        text = format!("{text} ({place})");
    }
    if !t.message.is_empty() {
        text = format!("{text} : {}", t.message.replace('\n', " "));
    }
    if let Some(q) = &t.question {
        text = format!("{text} {q}");
    }
    ui.horizontal_wrapped(|ui| {
        icon(ui, env, if done.is_some() { p.muted_dim } else { p.teal });
        let world = env.world;
        let color = if done.is_some() { p.muted } else { p.ink };
        chat_text_marked(ui, p, env.emoji, world, env.want_names, &text, 12.5, color, false, None);
        match done {
            None => buttons(ui, p, invite, &t.buttons, Vec2::new(80.0, 20.0)),
            Some(label) => {
                ui.label(RichText::new(format!("· {label}")).size(12.0).color(p.muted_dim));
            }
        }
    });
}

/// An invitation no longer kept: what it said, as a notice.
fn notice(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine) {
    let world = env.world;
    chat_text_marked(
        ui,
        env.p,
        env.emoji,
        world,
        env.want_names,
        &line.text,
        12.5,
        env.p.muted,
        true,
        None,
    );
}
