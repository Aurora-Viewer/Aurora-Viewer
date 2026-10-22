//! Teleports from other residents: offers (IM_LURE_USER), requests
//! (IM_TELEPORT_REQUEST), gods' lures and the answers to our own offers, as
//! process_improved_im handles them (indra/newview/llimprocessing.cpp,
//! originally LGPL 2.1), with lure_callback and teleport_request_callback
//! (llviewermessage.cpp) for the answers.
//!
//! Each offer or request is an `Invite` waiting for its answer, shown twice:
//! a card in the conversation with the sender (opened if needed, as
//! LLHandlerUtil::spawnIMSession / addNotifPanelToIM do) and an entry of the
//! notification centre, which stands for Firestorm's toast. Whichever
//! answers first closes the entry and leaves the card with its answer
//! (`World::answer_teleport_invite`).

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
    /// Teleport to the sender: the lure to accept (a god's when `godlike`).
    Offer { lure: Uuid, place: Option<Place>, godlike: bool },
    /// The sender asks to come to us: "Yes" offers them a teleport.
    Request,
}

/// What the agent's content rating makes of a lure (process_improved_im):
/// the region's rating (SIM_ACCESS_*) against the account's ceiling and its
/// preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Allowed,
    /// Above the preference: "Change and Continue" sets the preference to
    /// this rating, then teleports (TeleportOffered_MaturityExceeded,
    /// mature_lure_callback).
    Exceeded(u8),
    /// Above what the account may enter: no teleport, and the sender is told
    /// (TeleportOffered_MaturityBlocked).
    Blocked(u8),
}

/// process_improved_im's check of a lure's region: a teen account cannot
/// enter Moderate, an account that is not adult cannot enter Adult; a
/// preference below the region's rating asks to change it.
pub fn rating_of(region: u8, access: aurora_net::search::AgentAccess) -> Rating {
    use aurora_net::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE};
    match region {
        SIM_ACCESS_MATURE if access.max < SIM_ACCESS_MATURE => Rating::Blocked(region),
        SIM_ACCESS_ADULT if access.max < SIM_ACCESS_ADULT => Rating::Blocked(region),
        SIM_ACCESS_MATURE | SIM_ACCESS_ADULT if access.preferred < region => Rating::Exceeded(region),
        _ => Rating::Allowed,
    }
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
    /// An offer's region against the agent's content rating.
    pub rating: Rating,
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
    fn push(&mut self, from: Uuid, kind: InviteKind, message: String, rating: Rating) -> u64 {
        self.next += 1;
        self.list.push(Invite {
            id: self.next,
            from,
            kind,
            message,
            rating,
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

    pub fn get(&self, id: u64) -> Option<&Invite> {
        self.list.iter().find(|v| v.id == id)
    }

    /// The entries of the pending invitations and their senders: one whose
    /// conversation is in sight raises no toast (LLOfferHandler shows one
    /// only when the IM floater is not open).
    pub fn pending_notifications(&self) -> impl Iterator<Item = (u64, Uuid)> + '_ {
        self.list
            .iter()
            .filter(|v| v.answer == Answer::Pending && v.notification != 0)
            .map(|v| (v.notification, v.from))
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

/// The explanation of an offer above the preference or out of reach
/// (TeleportOffered_MaturityExceeded / _MaturityBlocked).
pub fn rating_note(rating: Rating) -> Option<String> {
    let name = |m: u8| crate::ui::bars::maturity_name(m).to_lowercase();
    match rating {
        Rating::Allowed => None,
        Rating::Exceeded(region) => Some(aurora_i18n::tr!("world-teleport-invites-rating-exceeded", rating = name(region))),
        Rating::Blocked(_) => Some(aurora_i18n::tr!("world-teleport-invites-rating-blocked").into()),
    }
}

/// `secondlife:///app/agent/<id>/completename`: the chat shows the name.
pub(super) fn agent_link(id: Uuid) -> String {
    format!("secondlife:///app/agent/{id}/completename")
}

impl World {
    /// IM_LURE_USER (process_improved_im): a card in the conversation with
    /// the sender and an entry in the notification centre, Teleport /
    /// Decline, with where it leads (FIRE-6786), the TeleportOffered sound,
    /// and its text in the sender's transcript (log_to_im). Muted senders
    /// and the "reject teleports" mode were handled before.
    pub(super) fn on_teleport_offer(&mut self, im: &InstantMessage, from: &str) {
        self.on_lure(im, from, false);
    }

    /// An offer or a god's lure, checked against the agent's content
    /// rating: a region above the preference asks to change it ("Change and
    /// Continue"); a region the account may not enter cannot be accepted,
    /// and the sender gets "The Resident cannot visit this region." then
    /// IM_LURE_DECLINED (send_simple_im, as Firestorm sends them).
    fn on_lure(&mut self, im: &InstantMessage, from: &str, godlike: bool) {
        if !godlike && !self.rlva_can_accept_teleport(im.from_agent_id, false) {
            self.rlva_busy_reply(im.from_agent_id, Uuid::nil(), aurora_i18n::tr!("rlva-im-blocked-teleport-remote"));
            self.rlva_teleport_dnd(im.from_agent_id);
            return;
        }
        let bucket = parse_lure_bucket(&im.binary_bucket);
        let place = bucket.as_ref().map(|(grid, pos, _, _)| Place {
            grid: *grid,
            pos: *pos,
            region: None,
        });
        let region = bucket
            .as_ref()
            .and_then(|(.., access)| access.as_deref())
            .map_or(0, aurora_net::search::short_string_to_access);
        let rating = rating_of(region, self.agent_access());
        if godlike && rating == Rating::Allowed {
            // "you're about to be teleported": no box
            log::info!("teleport: god lure from {}, accepted", im.from_agent_id);
            self.teleport_invites.out.push(NetCommand::AcceptLure {
                lure_id: im.session_id,
                godlike: true,
            });
            return;
        }
        let message = self.rlva_teleport_message(&im.message, im.from_agent_id, godlike);
        let invite = self.teleport_invites.push(
            im.from_agent_id,
            InviteKind::Offer {
                lure: im.session_id,
                place,
                godlike,
            },
            message.clone(),
            rating,
        );
        self.ui_sounds.push(UiSound::TeleportOffer);
        let title = aurora_i18n::tr!("world-mod-offers-you-a-teleport", from = from.to_string());
        let note = rating_note(rating);
        let body = match &note {
            Some(note) if message.is_empty() => note.clone(),
            Some(note) => format!("{message}\n\n{note}"),
            None => message.clone(),
        };
        self.add_card(im, ChatKind::Invite(invite), format!("{title}\n{body}"));
        let data = match rating {
            // TeleportOffered_MaturityBlocked: a notice, nothing to answer
            Rating::Blocked(_) => Data::None,
            _ => Data::Lure {
                from: im.from_agent_id,
                invite,
                change_rating: matches!(rating, Rating::Exceeded(_)),
            },
        };
        let id = self.notifications.push(Kind::Teleport, title, body.clone(), data);
        // the destination's rating, as the bucket gives it
        if let Some(n) = self.notifications.list.iter_mut().find(|n| n.id == id) {
            n.maturity = region;
        }
        self.set_invite_notification(invite, id);
        if rating == Rating::Allowed && self.rlva.auto_accept("accepttp", im.from_agent_id) {
            self.rlva_teleport_dnd(im.from_agent_id);
            let commands = self.answer_teleport_invite(invite, true);
            self.teleport_invites.out.extend(commands);
        }
        if let Rating::Blocked(_) = rating {
            if let Some(v) = self.teleport_invites.list.iter_mut().find(|v| v.id == invite) {
                v.answer = Answer::Declined;
            }
            let to = im.from_agent_id;
            for (dialog, message) in [
                (0, aurora_i18n::tr!("world-teleport-invites-resident-cannot-visit").to_owned()),
                (im::LURE_DECLINED, String::new()),
            ] {
                self.teleport_invites.out.push(NetCommand::SendImDialog {
                    to,
                    dialog,
                    id: im.session_id,
                    message,
                    bucket: Vec::new(),
                });
            }
        }
        self.refresh_teleport_places();
        let line = self.notice_line(aurora_i18n::tr!(
            "world-teleport-invites-offer-received",
            name = agent_link(im.from_agent_id),
            message = body
        ));
        self.transcript_only(im.from_agent_id, &line);
    }

    /// IM_TELEPORT_REQUEST: "<name> is requesting to be teleported to your
    /// location", Yes / No (TeleportRequest), as a card and an entry, with
    /// the same sound and log as an offer.
    pub(super) fn on_teleport_request(&mut self, im: &InstantMessage, from: &str) {
        if !self.rlva_can_accept_teleport(im.from_agent_id, true) {
            self.rlva_busy_reply(im.from_agent_id, Uuid::nil(), aurora_i18n::tr!("rlva-im-blocked-teleport-remote"));
            self.rlva_teleport_dnd(im.from_agent_id);
            return;
        }
        let message = self.rlva_teleport_message(&im.message, im.from_agent_id, false);
        let invite = self
            .teleport_invites
            .push(im.from_agent_id, InviteKind::Request, message.clone(), Rating::Allowed);
        self.ui_sounds.push(UiSound::TeleportOffer);
        let body = if message.is_empty() {
            aurora_i18n::tr!("world-teleport-invites-offer-a-teleport").to_owned()
        } else {
            format!("{message}\n\n{}", aurora_i18n::tr!("world-teleport-invites-offer-a-teleport"))
        };
        let title = aurora_i18n::tr!("world-teleport-invites-requests-a-teleport", from = from.to_string());
        self.add_card(im, ChatKind::Invite(invite), format!("{title}\n{message}"));
        let id = self.notifications.push(
            Kind::Teleport,
            title,
            body,
            Data::TeleportRequest {
                from: im.from_agent_id,
                invite,
            },
        );
        self.set_invite_notification(invite, id);
        if self.rlva.auto_accept("accepttprequest", im.from_agent_id) {
            self.rlva_teleport_dnd(im.from_agent_id);
            let commands = self.answer_teleport_invite(invite, true);
            self.teleport_invites.out.extend(commands);
        }
        let line = self.notice_line(aurora_i18n::tr!(
            "world-teleport-invites-request-received",
            name = agent_link(im.from_agent_id),
            message = message
        ));
        self.transcript_only(im.from_agent_id, &line);
    }

    /// IM_GODLIKE_LURE_USER: accepted at once, without a box ("you're about
    /// to be teleported"), unless the region's rating asks to change the
    /// preference or is out of the account's reach: then as an offer.
    pub(super) fn on_godlike_lure(&mut self, im: &InstantMessage, from: &str) {
        self.on_lure(im, from, true);
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
        if accept && let Some(v) = self.teleport_invites.list.iter().find(|v| v.id == invite) {
            let request = matches!(v.kind, InviteKind::Request);
            let godlike = matches!(v.kind, InviteKind::Offer { godlike: true, .. });
            if !godlike && !self.rlva_can_accept_teleport(v.from, request) {
                return Vec::new();
            }
        }
        let Some(v) = self.teleport_invites.list.iter_mut().find(|v| v.id == invite) else {
            return Vec::new();
        };
        if v.answer != Answer::Pending {
            return Vec::new();
        }
        v.answer = if accept { Answer::Accepted } else { Answer::Declined };
        let (from, kind, notification, rating) = (v.from, v.kind.clone(), v.notification, v.rating);
        self.notifications.note(
            notification,
            if accept {
                super::notification_history::Outcome::Answered
            } else {
                super::notification_history::Outcome::Declined
            },
        );
        self.notifications.list.retain(|n| n.id != notification);
        match (kind, accept) {
            (InviteKind::Offer { lure, godlike, .. }, true) => {
                // mature_lure_callback: the preference first, the teleport
                // waits for the server to keep it (App::send)
                if let Rating::Exceeded(region) = rating {
                    self.set_maturity_preference(region);
                }
                vec![NetCommand::AcceptLure { lure_id: lure, godlike }]
            }
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
    /// unknown region is asked once (getSLURLfromPosGlobal). The card and
    /// the notification then say where the offer leads, above the message
    /// (the card keeps it once answered).
    pub fn refresh_teleport_places(&mut self) {
        let unknown: Vec<(u64, (u32, u32))> = self
            .teleport_invites
            .list
            .iter()
            .filter_map(|v| match &v.kind {
                InviteKind::Offer { place: Some(p), .. } if p.region.is_none() => Some((v.id, p.grid)),
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
            if let Some(note) = rating_note(v.rating) {
                body = format!("{body}\n\n{note}");
            }
            let notification = v.notification;
            if let Some(n) = self.notifications.list.iter_mut().find(|n| n.id == notification) {
                n.body = body;
            }
        }
    }

    /// The card of an invitation or an inventory offer in the conversation
    /// with its sender, which opens if needed (spawnIMSession) and counts it
    /// as unread (addNotifPanelToIM). `text`: what the find bar looks in.
    pub(super) fn add_card(&mut self, im: &InstantMessage, kind: ChatKind, text: String) {
        let from = im.from_agent_id;
        if !self.rlva_can_start_im(from) {
            return;
        }
        if !im.from_name.is_empty() {
            self.social.names.entry(from).or_insert_with(|| im.from_name.clone());
        }
        self.social.session_mut(from);
        let line = ChatLine {
            time: SystemTime::now(),
            from: im.from_name.clone(),
            text,
            kind,
            source: from,
            origin: Origin::Live,
        };
        self.conversation_card(from, line);
    }

    fn set_invite_notification(&mut self, invite: u64, notification: u64) {
        if let Some(v) = self.teleport_invites.list.iter_mut().find(|v| v.id == invite) {
            v.notification = notification;
        }
    }

    pub(super) fn notice_line(&self, text: impl Into<String>) -> ChatLine {
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
        let line = self.notice_line(text);
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
    fn an_offer_shows_a_card_and_an_entry_and_goes_to_the_transcript() {
        let root = temp();
        let mut w = world(&root);
        // an adult account that wants every rating: the Moderate region is fine
        login(&mut w, "A", "A");
        let text = "Join me in Ahern\r\nhttp://maps.secondlife.com/secondlife/Ahern/128/64/22";
        w.on_im(im(im::LURE_USER, text, b"256000|256256|128|64|22|0|1|0|M"));
        let (id, data) = only_notification(&w);
        assert!(matches!(data, Data::Lure { from: JANE, .. }));
        assert!(w.ui_sounds.contains(&UiSound::TeleportOffer));
        // the conversation with Jane opens on the card (spawnIMSession,
        // addNotifPanelToIM): one unread line, nothing in the nearby chat
        assert_eq!(w.social.ims.len(), 1);
        let s = &w.social.ims[0];
        assert_eq!(s.other, JANE);
        assert_eq!(s.unread, 1);
        let invite = w.teleport_invites.list[0].id;
        assert!(matches!(&s.lines.iter().map(|l| l.kind).collect::<Vec<_>>()[..], [ChatKind::Invite(i)] if *i == invite));
        assert!(w.chat.is_empty());
        // the card is not written: the transcript has the offer once
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
        // the card stays, with its answer
        assert_eq!(w.teleport_invites.list[0].answer, Answer::Declined);
        assert!(matches!(w.social.ims[0].lines.back().unwrap().kind, ChatKind::Invite(_)));
        clean(&w, &root);
    }

    #[test]
    fn a_card_joins_the_open_conversation_and_answers_for_the_entry() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(im(0, "hello", b""));
        w.on_im(im(im::TELEPORT_REQUEST, "Can I come?", b""));
        // the same conversation, the card after the message
        assert_eq!(w.social.ims.len(), 1);
        let kinds: Vec<ChatKind> = w.social.ims[0].lines.iter().map(|l| l.kind).collect();
        assert!(matches!(&kinds[..], [ChatKind::Im, ChatKind::Invite(_)]), "{kinds:?}");
        let ChatKind::Invite(invite) = kinds[1] else { unreachable!() };
        // the entry of a pending invitation is known with its sender, for
        // the toast to stay quiet while the conversation is in sight
        let entry = w.notifications.list[0].id;
        assert_eq!(w.teleport_invites.pending_notifications().collect::<Vec<_>>(), [(entry, JANE)]);
        // answered from the card: the entry goes, nothing is pending
        let cmds = w.answer_teleport_invite(invite, false);
        assert!(cmds.is_empty());
        assert!(w.notifications.list.is_empty());
        assert_eq!(w.teleport_invites.pending_notifications().count(), 0);
        assert_eq!(w.teleport_invites.get(invite).unwrap().answer, Answer::Declined);
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
        assert!(w.chat.is_empty());
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

    /// An account of ceiling `max` that wants `preferred` (login letters).
    fn login(w: &mut World, max: &str, preferred: &str) {
        use aurora_llsd::llsd_map;
        w.login = Some(std::sync::Arc::new(aurora_net::LoginResponse {
            agent_id: ME,
            session_id: Uuid::nil(),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: std::net::Ipv4Addr::LOCALHOST,
            sim_port: 0,
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: Vec3::X,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: Uuid::nil(),
            mfa_hash: None,
            raw: llsd_map! { "agent_access_max" => max, "agent_region_access" => preferred },
        }));
    }

    const ADULT_REGION: &[u8] = b"256000|256256|128|64|22|0|1|0|A";

    #[test]
    fn a_region_is_allowed_above_the_preference_or_out_of_reach() {
        use aurora_net::search::{AgentAccess, SIM_ACCESS_ADULT as A, SIM_ACCESS_MATURE as M, SIM_ACCESS_PG as PG};
        let acc = |max, preferred| AgentAccess { max, preferred };
        assert_eq!(rating_of(PG, acc(PG, PG)), Rating::Allowed);
        assert_eq!(rating_of(0, acc(PG, PG)), Rating::Allowed);
        assert_eq!(rating_of(M, acc(A, M)), Rating::Allowed);
        assert_eq!(rating_of(M, acc(A, PG)), Rating::Exceeded(M));
        assert_eq!(rating_of(A, acc(A, M)), Rating::Exceeded(A));
        // a teen account cannot enter Moderate, a non-adult one Adult
        assert_eq!(rating_of(M, acc(PG, PG)), Rating::Blocked(M));
        assert_eq!(rating_of(A, acc(M, M)), Rating::Blocked(A));
        assert_eq!(rating_of(A, acc(A, A)), Rating::Allowed);
    }

    #[test]
    fn change_and_continue_sets_the_rating_then_teleports() {
        let root = temp();
        let mut w = world(&root);
        login(&mut w, "A", "M");
        w.on_im(im(im::LURE_USER, "Join me", ADULT_REGION));
        let v = w.teleport_invites.list[0].clone();
        assert_eq!(v.rating, Rating::Exceeded(aurora_net::search::SIM_ACCESS_ADULT));
        let (_, data) = only_notification(&w);
        assert!(matches!(data, Data::Lure { change_rating: true, .. }));
        let note = rating_note(v.rating).unwrap();
        assert!(w.notifications.list[0].body.contains("Join me") && w.notifications.list[0].body.ends_with(&note));
        // the place arrives: the explanation stays under the message
        w.map.sims.insert(
            (1000, 1001),
            super::super::worldmap::SimInfo {
                name: "Ahern".into(),
                access: 42,
                flags: 0,
                image: Uuid::nil(),
                size_x: 256,
                size_y: 256,
            },
        );
        w.refresh_teleport_places();
        let body = &w.notifications.list[0].body;
        assert!(body.contains("Ahern (128, 64, 22)") && body.ends_with(&note), "{body}");
        // the preference is asked for, the teleport is the lure's
        let cmds = w.answer_teleport_invite(v.id, true);
        assert!(matches!(
            &cmds[..],
            [NetCommand::AcceptLure {
                lure_id: LURE,
                godlike: false
            }]
        ));
        assert!(matches!(
            w.maturity.take_commands()[..],
            [(_, NetCommand::SetMaturityPreference(aurora_net::search::SIM_ACCESS_ADULT))]
        ));
        assert!(!w.maturity.synced());
        // declining such an offer is the plain decline
        w.on_im(im(im::LURE_USER, "Again", ADULT_REGION));
        let id = w.teleport_invites.list[1].id;
        let cmds = w.answer_teleport_invite(id, false);
        assert!(matches!(
            &cmds[..],
            [NetCommand::SendImDialog {
                dialog: im::LURE_DECLINED,
                ..
            }]
        ));
        clean(&w, &root);
    }

    #[test]
    fn a_region_out_of_reach_tells_the_sender_and_cannot_be_accepted() {
        let root = temp();
        let mut w = world(&root);
        login(&mut w, "M", "M");
        w.on_im(im(im::LURE_USER, "Join me", ADULT_REGION));
        let v = w.teleport_invites.list[0].clone();
        assert!(matches!(v.rating, Rating::Blocked(_)));
        // a notice and a card without buttons, nothing left to answer
        let (_, data) = only_notification(&w);
        assert!(matches!(data, Data::None));
        assert_eq!(v.answer, Answer::Declined);
        assert!(w.answer_teleport_invite(v.id, true).is_empty());
        // send_simple_im twice: the reason, then IM_LURE_DECLINED
        let sent = w.teleport_invites.take_commands();
        assert!(
            matches!(
                &sent[..],
                [
                    NetCommand::SendImDialog { to: JANE, dialog: 0, id: LURE, message, .. },
                    NetCommand::SendImDialog { to: JANE, dialog: im::LURE_DECLINED, id: LURE, .. },
                ] if !message.is_empty()
            ),
            "{sent:?}"
        );
        clean(&w, &root);
    }

    #[test]
    fn a_god_lure_above_the_rating_asks_like_an_offer() {
        let root = temp();
        let mut w = world(&root);
        login(&mut w, "A", "PG");
        w.on_im(im(im::GODLIKE_LURE_USER, "", ADULT_REGION));
        assert!(w.teleport_invites.take_commands().is_empty());
        let id = w.teleport_invites.list[0].id;
        let cmds = w.answer_teleport_invite(id, true);
        assert!(matches!(&cmds[..], [NetCommand::AcceptLure { godlike: true, .. }]));
        // out of reach: refused for the god too
        login(&mut w, "M", "M");
        w.on_im(im(im::GODLIKE_LURE_USER, "", ADULT_REGION));
        let sent = w.teleport_invites.take_commands();
        assert!(
            matches!(
                &sent[..],
                [
                    _,
                    NetCommand::SendImDialog {
                        dialog: im::LURE_DECLINED,
                        ..
                    }
                ]
            ),
            "{sent:?}"
        );
        clean(&w, &root);
    }
}
