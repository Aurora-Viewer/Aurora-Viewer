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
use crate::world::teleport_invites::{Answer, Invite, InviteKind, Place, Rating, rating_note};
use egui::{RichText, Vec2};

/// Width of a card at most.
const CARD_WIDTH: f32 = 300.0;
const BUTTON: Vec2 = Vec2::new(88.0, 24.0);
/// The icon of teleport offers, the same as in the notification centre.
const ICON: &str = "plane-departure-filled";

/// What a card says, from its invitation.
struct Texts {
    title: String,
    /// The place as a SLURL, shown as its link.
    place: Option<String>,
    message: String,
    /// The question of a request ("Offer a teleport?"), or what an offer's
    /// region rating means (TeleportOffered_MaturityExceeded / _Blocked).
    question: Option<String>,
    /// The two buttons: (accept, refuse).
    buttons: (String, String),
}

fn texts(env: &LineEnv, line: &ChatLine, v: &Invite) -> Texts {
    let name = env.world.person_name(&v.from).unwrap_or_else(|| line.from.clone());
    match &v.kind {
        InviteKind::Offer { place, .. } => Texts {
            title: aurora_i18n::tr!("world-mod-offers-you-a-teleport", from = name),
            place: place.as_ref().and_then(|p| Some(crate::slurl::make(p.region.as_deref()?, p.pos))),
            message: without_own_place(&v.message, place.as_ref()).to_owned(),
            question: rating_note(v.rating),
            buttons: (
                if matches!(v.rating, Rating::Exceeded(_)) {
                    aurora_i18n::tr!("ui-chat-invite-change-and-continue")
                } else {
                    aurora_i18n::tr!("common-teleport-3")
                }
                .into(),
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

/// The sender's message without its last line when that line is the SLURL
/// of the invitation's own place (send_lures adds where the sender stands):
/// the card already shows the place at its head. A SLURL of another place
/// stays; the transcript keeps the whole message. The lure destination can
/// differ from send_lures' SLURL by a few metres (#899), beyond rounding.
fn without_own_place<'a>(message: &'a str, place: Option<&Place>) -> &'a str {
    let Some((place, region)) = place.and_then(|p| Some((p, p.region.as_deref()?))) else {
        return message;
    };
    let trimmed = message.trim_end();
    let (head, last) = trimmed.rsplit_once('\n').unwrap_or(("", trimmed));
    let last = last.trim();
    let lower = last.to_lowercase();
    if !["http://", "https://", "secondlife:"].iter().any(|s| lower.starts_with(s)) {
        return message;
    }
    match crate::slurl::parse(last) {
        Some(at) if at.region.eq_ignore_ascii_case(region) && (at.pos - place.pos).abs().max_element() <= 3.0 => head.trim_end(),
        _ => message,
    }
}

/// None while it waits for an answer, else what to say instead of the
/// buttons: the answer, or nothing for an offer out of the account's reach
/// (its explanation is already on the card).
fn answer_label(v: &Invite) -> Option<&'static str> {
    if let Rating::Blocked(_) = v.rating {
        return Some("");
    }
    match v.answer {
        Answer::Pending => None,
        Answer::Accepted => Some(aurora_i18n::tr!("ui-chat-invite-accepted")),
        Answer::Declined => Some(aurora_i18n::tr!("ui-chat-invite-declined")),
    }
}

/// A card's Tabler icon, the size of its title.
pub(super) fn icon(ui: &mut egui::Ui, env: &LineEnv, name: &str, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    if let Some(t) = env.icons.get(name) {
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
    // "Change and Continue" is longer than the other answers
    let wide = Vec2::new(size.x.max(yes.chars().count() as f32 * 7.0 + 20.0), size.y);
    if widgets::primary_button_sized(ui, p, yes, wide).clicked() {
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
    let done = answer_label(&v);
    card_frame(ui, env, line, indent, done.is_some(), |ui, env| {
        let (ink, muted) = if done.is_some() { (p.muted, p.muted_dim) } else { (p.ink, p.muted) };
        ui.horizontal(|ui| {
            icon(ui, env, ICON, if done.is_some() { p.muted_dim } else { p.teal });
            ui.add(egui::Label::new(RichText::new(&t.title).size(13.0).strong().color(ink)).wrap());
        });
        if let Some(place) = &t.place {
            let world = env.world;
            chat_text_marked(ui, p, env.emoji, world, env.want_names, place, 12.0, muted, false, None);
        }
        if !t.message.is_empty() {
            let world = env.world;
            chat_text_marked(ui, p, env.emoji, world, env.want_names, &t.message, 12.5, ink, false, None);
        }
        if let Some(q) = &t.question {
            // the region's rating, with the navigation bar's badge
            ui.horizontal_top(|ui| {
                if let Rating::Exceeded(r) | Rating::Blocked(r) = v.rating {
                    widgets::maturity_badge_of(ui, p, r, 16.0);
                }
                ui.add(egui::Label::new(RichText::new(q).size(12.0).color(muted)).wrap());
            });
        }
        match done {
            None => {
                ui.add_space(5.0);
                ui.horizontal(|ui| buttons(ui, p, invite, &t.buttons, BUTTON));
            }
            Some("") => {}
            Some(label) => answered(ui, p, label),
        }
    });
}

/// What a card says once answered, in place of its buttons.
pub(super) fn answered(ui: &mut egui::Ui, p: &Palette, label: &str) {
    ui.add_space(5.0);
    ui.label(RichText::new(label).size(12.0).color(p.muted_dim));
}

/// The frame of a card on the sender's side after `indent` (the column of
/// the senders' pictures), its teal edge grey once `done`, then its time.
pub(super) fn card_frame(
    ui: &mut egui::Ui,
    env: &mut LineEnv,
    line: &ChatLine,
    indent: f32,
    done: bool,
    contents: impl FnOnce(&mut egui::Ui, &mut LineEnv),
) {
    let p = env.p;
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
                    // the theme's gap between the icon and the title, and
                    // between the buttons: the lines around may have none
                    ui.spacing_mut().item_spacing = egui::vec2(ui.ctx().global_style().spacing.item_spacing.x, 3.0);
                    contents(ui, env);
                });
            });
        // the teal edge of the cards (mock-up C), grey once answered
        let r = frame.response.rect;
        let edge = if done { p.muted_dim } else { p.teal };
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
    let done = answer_label(&v);
    let mut text = t.title.clone();
    if let Some(place) = &t.place {
        text = format!("{text} {place}");
    }
    if !t.message.is_empty() {
        text = format!("{text} : {}", t.message.replace('\n', " "));
    }
    if let Some(q) = &t.question {
        text = format!("{text} {q}");
    }
    ui.horizontal_wrapped(|ui| {
        icon(ui, env, ICON, if done.is_some() { p.muted_dim } else { p.teal });
        if let Rating::Exceeded(r) | Rating::Blocked(r) = v.rating {
            widgets::maturity_badge_of(ui, p, r, 14.0);
        }
        let world = env.world;
        let color = if done.is_some() { p.muted } else { p.ink };
        chat_text_marked(ui, p, env.emoji, world, env.want_names, &text, 12.5, color, false, None);
        match done {
            None => buttons(ui, p, invite, &t.buttons, Vec2::new(80.0, 20.0)),
            Some("") => {}
            Some(label) => {
                ui.label(RichText::new(format!("· {label}")).size(12.0).color(p.muted_dim));
            }
        }
    });
}

/// An invitation or an offer no longer kept: what it said, as a notice.
pub(super) fn notice(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn place(region: Option<&str>) -> Place {
        Place {
            grid: (1000, 1000),
            pos: Vec3::new(128.0, 130.0, 25.0),
            region: region.map(str::to_owned),
        }
    }

    #[test]
    fn teleport_invitation_removes_the_nearby_sender_slurl() {
        let destination = place(Some("Aurora Demo"));
        // The report's links differ by 2 m horizontally and 1 m vertically.
        let message = "Join me\r\nhttp://maps.secondlife.com/secondlife/Aurora%20Demo/130/130/24";
        assert_eq!(without_own_place(message, Some(&destination)), "Join me");
        assert_eq!(without_own_place(message, Some(&place(None))), message);
        assert_eq!(without_own_place(message, None), message);
        for other in [
            "Join me\nhttp://maps.secondlife.com/secondlife/Aurora%20Demo/160/130/25",
            "Join me\nhttp://maps.secondlife.com/secondlife/Aurora%20Demo/128/130/1000",
            "Join me\nhttp://maps.secondlife.com/secondlife/Other%20Region/130/130/24",
        ] {
            assert_eq!(without_own_place(other, Some(&destination)), other);
        }
    }

    #[test]
    fn the_slurl_of_the_offer_s_own_place_leaves_the_message() {
        let ahern = place(Some("Ahern"));
        let sent = "Join me in Ahern\r\nhttp://maps.secondlife.com/secondlife/Ahern/128/130/25";
        // as received (send_lures), and once the carriage returns are gone
        assert_eq!(without_own_place(sent, Some(&ahern)), "Join me in Ahern");
        assert_eq!(without_own_place(&sent.replace('\r', ""), Some(&ahern)), "Join me in Ahern");
        // only the SLURL: nothing left to show
        assert_eq!(
            without_own_place("http://maps.secondlife.com/secondlife/Ahern/128/130/25", Some(&ahern)),
            ""
        );
        // another place, another region, plain text, or a place still unknown: kept
        for kept in [
            "Join me\nhttp://maps.secondlife.com/secondlife/Ahern/12/30/25",
            "Join me\nhttp://maps.secondlife.com/secondlife/Bonifacio/128/130/25",
            "Join me\nAhern/128/130/25",
            "Come over!",
        ] {
            assert_eq!(without_own_place(kept, Some(&ahern)), kept);
        }
        assert_eq!(without_own_place(sent, Some(&place(None))), sent);
        assert_eq!(without_own_place(sent, None), sent);
    }
}
