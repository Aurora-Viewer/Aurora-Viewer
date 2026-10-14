//! Teleports from other residents: offers (IM_LURE_USER), requests
//! (IM_TELEPORT_REQUEST), gods' lures and the answers to our own offers, as
//! process_improved_im handles them (indra/newview/llimprocessing.cpp,
//! originally LGPL 2.1), with lure_callback and teleport_request_callback
//! (llviewermessage.cpp) for the answers.
//!
//! Each offer or request is an `Invite` waiting for its answer. The
//! notification centre shows it today; a card in the conversation with the
//! sender can answer the same invitation later, and whichever answers first
//! closes the other (`World::answer_teleport_invite`).

use super::notifications::{Data, Kind, im};
use super::{ChatKind, ChatLine, Origin, UiSound, World};
use aurora_net::{InstantMessage, NetCommand};
use glam::Vec3;
use std::time::SystemTime;
use uuid::Uuid;

/// Where an offer leads: the bucket of the lure (parse_lure_bucket), the
/// region's name once the world map knows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// The region's grid cell (global metres / 256).
    pub grid: (u32, u32),
    /// Position in the region.
    pub pos: Vec3,
    pub region: Option<String>,
}

impl Place {
    /// "Region (x, y, z)", once the region's name is known.
    pub fn label(&self) -> Option<String> {
        let region = self.region.as_deref()?;
        Some(super::worldmap::location_label(
            region,
            self.pos.x as f64,
            self.pos.y as f64,
            self.pos.z,
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InviteKind {
    /// Teleport to the sender: the lure to accept.
    Offer { lure: Uuid, place: Option<Place> },
    /// The sender asks to come to us: "Yes" offers them a teleport.
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Pending,
    Accepted,
    Declined,
}

#[derive(Debug, Clone)]
pub struct Invite {
    pub id: u64,
    pub from: Uuid,
    pub kind: InviteKind,
    /// The sender's text, as received.
    pub message: String,
    #[allow(dead_code)] // shown by the conversation card (part B2)
    pub time: SystemTime,
    pub answer: Answer,
    /// Its entry in the notification centre (0 = none).
    pub notification: u64,
}

/// Invitations kept once answered, for the conversation cards.
const MAX_INVITES: usize = 50;

#[derive(Default)]
pub struct TeleportInvites {
    pub list: Vec<Invite>,
    next: u64,
    /// Answers and gods' lures to send; an AcceptLure starts a teleport.
    out: Vec<NetCommand>,
    /// Grid cells whose region was asked from the map.
    asked: std::collections::HashSet<(u32, u32)>,
}

impl TeleportInvites {
    fn push(&mut self, from: Uuid, kind: InviteKind, message: String) -> u64 {
        self.next += 1;
        self.list.push(Invite {
            id: self.next,
            from,
            kind,
            message,
            time: SystemTime::now(),
            answer: Answer::Pending,
            notification: 0,
        });
        if self.list.len() > MAX_INVITES
            && let Some(i) = self.list.iter().position(|v| v.answer != Answer::Pending)
        {
            self.list.remove(i);
        }
        self.next
    }

    #[allow(dead_code)] // the conversation card (part B2)
    pub fn get(&self, id: u64) -> Option<&Invite> {
        self.list.iter().find(|v| v.id == id)
    }

    /// The invitations of a conversation, oldest first.
    #[allow(dead_code)] // the conversation card (part B2)
    pub fn from(&self, sender: Uuid) -> impl Iterator<Item = &Invite> {
        self.list.iter().filter(move |v| v.from == sender)
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

/// The bucket of a lure, "gx|gy|rx|ry|rz|lx|ly|lz|access"
/// (parse_lure_bucket): the region's corner in global metres, the position
/// in it, the look-at and the region's access ("PG", "M", "A").
pub fn parse_lure_bucket(bucket: &[u8]) -> Option<((u32, u32), Vec3, Vec3, Option<String>)> {
    let text = std::str::from_utf8(bucket).ok()?.trim_end_matches('\0');
    let mut it = text.split('|');
    let mut int = || it.next()?.trim().parse::<i32>().ok();
    let (gx, gy) = (int()?, int()?);
    let pos = Vec3::new(int()? as f32, int()? as f32, int()? as f32);
    let look = Vec3::new(int()? as f32, int()? as f32, int()? as f32);
    let access = it.next().map(|a| a.trim().to_owned()).filter(|a| !a.is_empty());
    let grid = (u32::try_from(gx).ok()? / 256, u32::try_from(gy).ok()? / 256);
    Some((grid, pos, look, access))
}

/// `secondlife:///app/agent/<id>/completename`: the chat shows the name.
fn agent_link(id: Uuid) -> String {
    format!("secondlife:///app/agent/{id}/completename")
}

impl World {
    /// IM_LURE_USER (process_improved_im): an entry in the notification
    /// centre, Teleport / Decline, with where it leads (FIRE-6786), the
    /// TeleportOffered sound, and its text in the sender's transcript
    /// (log_to_im). Muted senders and the "reject teleports" mode were
    /// handled before.
    pub(super) fn on_teleport_offer(&mut self, im: &InstantMessage, from: &str) {
        let place = parse_lure_bucket(&im.binary_bucket).map(|(grid, pos, _, _)| Place { grid, pos, region: None });
        let message = im.message.replace('\r', "");
        let invite = self.teleport_invites.push(
            im.from_agent_id,
            InviteKind::Offer {
                lure: im.session_id,
                place,
            },
            message.clone(),
        );
        self.ui_sounds.push(UiSound::TeleportOffer);
        let id = self.notifications.push(
            Kind::Teleport,
            aurora_i18n::tr!("world-mod-offers-you-a-teleport", from = from.to_string()),
            message,
            Data::Lure {
                from: im.from_agent_id,
                invite,
            },
        );
        self.set_invite_notification(invite, id);
        self.refresh_teleport_places();
        let line = self.invite_line(aurora_i18n::tr!(
            "world-teleport-invites-offer-received",
            name = agent_link(im.from_agent_id),
            message = im.message.replace('\r', "")
        ));
        self.transcript_only(im.from_agent_id, &line);
    }

    /// IM_TELEPORT_REQUEST: "<name> is requesting to be teleported to your
    /// location", Yes / No (TeleportRequest), with the same sound and log as
    /// an offer.
    pub(super) fn on_teleport_request(&mut self, im: &InstantMessage, from: &str) {
        let message = im.message.replace('\r', "");
        let invite = self.teleport_invites.push(im.from_agent_id, InviteKind::Request, message.clone());
        self.ui_sounds.push(UiSound::TeleportOffer);
        let body = if message.is_empty() {
            aurora_i18n::tr!("world-teleport-invites-offer-a-teleport").to_owned()
        } else {
            format!("{message}\n\n{}", aurora_i18n::tr!("world-teleport-invites-offer-a-teleport"))
        };
        let id = self.notifications.push(
            Kind::Teleport,
            aurora_i18n::tr!("world-teleport-invites-requests-a-teleport", from = from.to_string()),
            body,
            Data::TeleportRequest {
                from: im.from_agent_id,
                invite,
            },
        );
        self.set_invite_notification(invite, id);
        let line = self.invite_line(aurora_i18n::tr!(
            "world-teleport-invites-request-received",
            name = agent_link(im.from_agent_id),
            message = message
        ));
        self.transcript_only(im.from_agent_id, &line);
    }

    /// IM_GODLIKE_LURE_USER: accepted at once, without a box ("you're about
    /// to be teleported"). The maturity checks come later (part C).
    pub(super) fn on_godlike_lure(&mut self, im: &InstantMessage) {
        log::info!("teleport: god lure from {}, accepted", im.from_agent_id);
        self.teleport_invites.out.push(NetCommand::AcceptLure {
            lure_id: im.session_id,
            godlike: true,
        });
    }

    /// IM_LURE_ACCEPTED / IM_LURE_DECLINED: the answer to our offer, as a
    /// system line in the conversation with that resident, or only in their
    /// transcript. Deliberately not Firestorm's behaviour, which shows
    /// nothing ("unknown dialog"): the sender would never learn the answer.
    pub(super) fn on_teleport_offer_answered(&mut self, im: &InstantMessage, accepted: bool) {
        let name = agent_link(im.from_agent_id);
        let mut text = if accepted {
            aurora_i18n::tr!("world-mod-accepted-your-teleport-offer", from = name)
        } else {
            aurora_i18n::tr!("world-mod-declined-your-teleport-offer", from = name)
        };
        let reason = im.message.trim();
        if !reason.is_empty() {
            text = format!("{text} : {reason}");
        }
        self.note_with(im.from_agent_id, text);
    }

    /// Answer an invitation, from its notification or (later) its card in
    /// the conversation; the other is closed. An offer accepted sends
    /// TeleportLureRequest, declined IM_LURE_DECLINED (lure_callback). A
    /// request accepted offers a teleport with no text, followed by our
    /// location (teleport_request_callback → send_lures); refused, nothing
    /// is sent.
    pub fn answer_teleport_invite(&mut self, invite: u64, accept: bool) -> Vec<NetCommand> {
        let Some(v) = self.teleport_invites.list.iter_mut().find(|v| v.id == invite) else {
            return Vec::new();
        };
        if v.answer != Answer::Pending {
            return Vec::new();
        }
        v.answer = if accept { Answer::Accepted } else { Answer::Declined };
        let (from, kind, notification) = (v.from, v.kind.clone(), v.notification);
        self.notifications.list.retain(|n| n.id != notification);
        match (kind, accept) {
            (InviteKind::Offer { lure, .. }, true) => vec![NetCommand::AcceptLure {
                lure_id: lure,
                godlike: false,
            }],
            (InviteKind::Offer { lure, .. }, false) => vec![NetCommand::SendImDialog {
                to: from,
                dialog: im::LURE_DECLINED,
                id: lure,
                message: String::new(),
                bucket: Vec::new(),
            }],
            (InviteKind::Request, true) => vec![self.offer_teleport(vec![from], "")],
            (InviteKind::Request, false) => Vec::new(),
        }
    }

    /// The region names of the offers' places, from the world map; an
    /// unknown region is asked once (getSLURLfromPosGlobal). The
    /// notification then says where the offer leads, above the message.
    pub fn refresh_teleport_places(&mut self) {
        let unknown: Vec<(u64, (u32, u32))> = self
            .teleport_invites
            .list
            .iter()
            .filter_map(|v| match &v.kind {
                InviteKind::Offer { place: Some(p), .. } if p.region.is_none() && v.answer == Answer::Pending => Some((v.id, p.grid)),
                _ => None,
            })
            .collect();
        for (id, grid) in unknown {
            let Some(region) = self.map.sim_at(grid.0, grid.1).map(|(_, sim)| sim.name.clone()) else {
                if self.teleport_invites.asked.insert(grid) {
                    let (x, y) = (grid.0 as u16, grid.1 as u16);
                    self.map.push(NetCommand::MapBlockRequest {
                        min_x: x,
                        min_y: y,
                        max_x: x,
                        max_y: y,
                        null_sims: true,
                    });
                }
                continue;
            };
            let Some(v) = self.teleport_invites.list.iter_mut().find(|v| v.id == id) else {
                continue;
            };
            let InviteKind::Offer { place: Some(place), .. } = &mut v.kind else {
                continue;
            };
            place.region = Some(region);
            let label = place.label().unwrap_or_default();
            let mut body = aurora_i18n::tr!("world-teleport-invites-place", place = label).to_string();
            if !v.message.is_empty() {
                body = format!("{body}\n{}", v.message);
            }
            let notification = v.notification;
            if let Some(n) = self.notifications.list.iter_mut().find(|n| n.id == notification) {
                n.body = body;
            }
        }
    }

    fn set_invite_notification(&mut self, invite: u64, notification: u64) {
        if let Some(v) = self.teleport_invites.list.iter_mut().find(|v| v.id == invite) {
            v.notification = notification;
        }
    }

    fn invite_line(&self, text: impl Into<String>) -> ChatLine {
        ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: text.into(),
            kind: ChatKind::System,
            source: Uuid::nil(),
            origin: Origin::Live,
        }
    }

    /// A system line about a resident: in the open conversation with them,
    /// without counting as unread, else only in their transcript
    /// (LLHandlerUtil::logToIMP2P).
    pub(super) fn note_with(&mut self, other: Uuid, text: impl Into<String>) {
        let line = self.invite_line(text);
        if self.social.has_session(&other) {
            self.conversation_push(other, line, false);
        } else {
            self.transcript_only(other, &line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::chat_log;
    use super::super::notifications::Response;
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    const ME: Uuid = Uuid::from_u128(1);
    const JANE: Uuid = Uuid::from_u128(2);
    const LURE: Uuid = Uuid::from_u128(0x10);

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-tp-invites-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn world(root: &Path) -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.chat_log = chat_log::ChatLog::in_folder(root.to_path_buf());
        w.chat_log.set_grid("agni".into());
        w.agent_id = ME;
        w.chat_log.login(ME);
        w.social.names.insert(JANE, "Jane Doe".into());
        w
    }

    fn im(dialog: u8, text: &str, bucket: &[u8]) -> InstantMessage {
        InstantMessage {
            from_agent_id: JANE,
            from_name: "Jane Doe".into(),
            to_agent_id: ME,
            dialog,
            session_id: LURE,
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket.to_vec(),
            timestamp: 0,
        }
    }

    /// Jane's transcript, once the writer is done.
    fn transcript(w: &mut World) -> Vec<String> {
        assert!(w.chat_log.flush(Duration::from_secs(10)));
        let path = w.chat_log.file_of("Jane Doe").unwrap();
        chat_log::store::read_tail(&path)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.text)
            .collect()
    }

    fn clean(w: &World, root: &Path) {
        assert!(w.chat_log.flush(Duration::from_secs(10)));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn only_notification(w: &World) -> (u64, Data) {
        assert_eq!(w.notifications.list.len(), 1, "{:?}", w.notifications.list);
        let n = &w.notifications.list[0];
        assert!(n.kind == Kind::Teleport);
        (n.id, n.data.clone())
    }

    #[test]
    fn the_lure_bucket_gives_the_region_and_the_position() {
        let (grid, pos, look, access) = parse_lure_bucket(b"256000|256256|128|64|22|0|1|0|M\0").unwrap();
        assert_eq!(grid, (1000, 1001));
        assert_eq!(pos, Vec3::new(128.0, 64.0, 22.0));
        assert_eq!(look, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(access.as_deref(), Some("M"));
        assert!(parse_lure_bucket(b"").is_none());
        assert!(parse_lure_bucket(b"1|2|3").is_none());
    }

    #[test]
    fn an_offer_waits_in_the_notification_centre_and_its_sender_transcript() {
        let root = temp();
        let mut w = world(&root);
        let text = "Join me in Ahern\r\nhttp://maps.secondlife.com/secondlife/Ahern/128/64/22";
        w.on_im(im(im::LURE_USER, text, b"256000|256256|128|64|22|0|1|0|M"));
        let (id, data) = only_notification(&w);
        assert!(matches!(data, Data::Lure { from: JANE, .. }));
        assert!(w.ui_sounds.contains(&UiSound::TeleportOffer));
        // no conversation opened, nothing in the nearby chat
        assert!(w.social.ims.is_empty());
        assert!(w.chat.is_empty());
        let log = transcript(&mut w);
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("Join me in Ahern"), "{log:?}");
        // where it leads, once the map knows the region
        w.map.sims.insert(
            (1000, 1001),
            super::super::worldmap::SimInfo {
                name: "Ahern".into(),
                access: 13,
                flags: 0,
                image: Uuid::nil(),
                size_x: 256,
                size_y: 256,
            },
        );
        w.refresh_teleport_places();
        let body = &w.notifications.list[0].body;
        assert!(body.contains("Ahern (128, 64, 22)") && body.ends_with("Ahern/128/64/22"), "{body}");
        // declining answers the lure (lure_callback) and closes it
        let (cmds, _) = w.respond_notification(id, Response::Decline);
        assert!(matches!(
            &cmds[..],
            [NetCommand::SendImDialog {
                to: JANE,
                dialog: im::LURE_DECLINED,
                id: LURE,
                ..
            }]
        ));
        assert!(w.notifications.list.is_empty());
        assert_eq!(w.teleport_invites.list[0].answer, Answer::Declined);
        clean(&w, &root);
    }

    #[test]
    fn an_unknown_region_is_asked_once_from_the_map() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(im(im::LURE_USER, "", b"256000|256256|128|64|22|0|1|0|PG"));
        w.refresh_teleport_places();
        w.refresh_teleport_places();
        let asked = w.map.take_commands();
        assert!(
            matches!(
                &asked[..],
                [NetCommand::MapBlockRequest {
                    min_x: 1000,
                    min_y: 1001,
                    null_sims: true,
                    ..
                }]
            ),
            "{asked:?}"
        );
        clean(&w, &root);
    }

    #[test]
    fn the_notification_and_the_card_answer_the_same_invitation_once() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(im(im::LURE_USER, "Join me", b""));
        let invite = w.teleport_invites.list[0].id;
        // the card (part B2) accepts: the notification goes
        let cmds = w.answer_teleport_invite(invite, true);
        assert!(matches!(
            &cmds[..],
            [NetCommand::AcceptLure {
                lure_id: LURE,
                godlike: false
            }]
        ));
        assert!(w.notifications.list.is_empty());
        assert!(w.answer_teleport_invite(invite, false).is_empty());
        assert_eq!(w.teleport_invites.get(invite).unwrap().answer, Answer::Accepted);
        clean(&w, &root);
    }

    #[test]
    fn a_request_asks_yes_or_no_and_yes_offers_a_teleport_with_our_location() {
        let root = temp();
        let mut w = world(&root);
        // an empty request is not lost any more, nor a chat line
        w.on_im(im(im::TELEPORT_REQUEST, "", b""));
        let (id, data) = only_notification(&w);
        assert!(matches!(data, Data::TeleportRequest { from: JANE, .. }));
        assert!(w.chat.is_empty() && w.social.ims.is_empty());
        let (cmds, _) = w.respond_notification(id, Response::Accept);
        let [NetCommand::OfferTeleport { to, message }] = &cmds[..] else {
            panic!("no teleport offer: {cmds:?}");
        };
        assert_eq!(to, &[JANE]);
        assert!(message.starts_with("\r\nhttp://maps.secondlife.com/secondlife/"), "{message}");
        // No sends nothing
        w.on_im(im(im::TELEPORT_REQUEST, "Can I come?", b""));
        let (id, _) = only_notification(&w);
        let (cmds, _) = w.respond_notification(id, Response::Decline);
        assert!(cmds.is_empty());
        assert!(w.notifications.list.is_empty());
        let log = transcript(&mut w);
        assert!(log.iter().any(|l| l.contains("Can I come?")), "{log:?}");
        clean(&w, &root);
    }

    #[test]
    fn rejecting_teleports_answers_without_showing_anything() {
        let root = temp();
        let mut w = world(&root);
        w.status.cfg.reject_teleports = true;
        w.on_im(im(im::LURE_USER, "Join me", b""));
        w.on_im(im(im::TELEPORT_REQUEST, "Can I come?", b""));
        assert!(w.notifications.list.is_empty());
        assert!(w.teleport_invites.list.is_empty());
        let sent = w.groups.take_commands();
        assert_eq!(sent.len(), 2, "{sent:?}");
        clean(&w, &root);
    }

    #[test]
    fn a_god_lure_is_accepted_at_once() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(im(im::GODLIKE_LURE_USER, "Come here", b"256000|256256|128|64|22|0|1|0|PG"));
        assert!(w.notifications.list.is_empty());
        let cmds = w.teleport_invites.take_commands();
        assert!(matches!(
            &cmds[..],
            [NetCommand::AcceptLure {
                lure_id: LURE,
                godlike: true
            }]
        ));
        clean(&w, &root);
    }

    #[test]
    fn the_answer_to_our_offer_is_noted_with_the_resident() {
        let root = temp();
        let mut w = world(&root);
        // no conversation: only in the transcript, no notification
        w.on_im(im(im::LURE_DECLINED, "", b""));
        assert!(w.notifications.list.is_empty());
        assert!(w.social.ims.is_empty() && w.chat.is_empty());
        assert_eq!(transcript(&mut w).len(), 1);
        // a conversation is open: a system line in it, not unread
        w.on_im(im(0, "hello", b""));
        let unread = w.social.ims[0].unread;
        w.on_im(im(im::LURE_ACCEPTED, "", b""));
        let s = &w.social.ims[0];
        let line = s.lines.back().unwrap();
        assert!(
            line.kind == ChatKind::System && line.text.starts_with(&agent_link(JANE)),
            "{}",
            line.text
        );
        assert_eq!(s.unread, unread);
        assert!(w.notifications.list.is_empty());
        clean(&w, &root);
    }
}
