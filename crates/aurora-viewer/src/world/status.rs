//! Online status and automatic responses, after Firestorm (originally LGPL 2.1):
//! Communicate > Online status (away, do not disturb, autorespond to
//! everyone / to non-friends, reject teleports, group invitations and
//! friendship requests) and Preferences > Privacy > Autoresponse
//! (llimprocessing.cpp, LLIMProcessing::getAutoresponseTextForAvatar,
//! send_rejecting_tp_offers_message, LLAgent::setAFK / setDoNotDisturb).

use super::notifications::im as d;
use super::{ChatKind, ChatLine, World};
use aurora_net::{InstantMessage, NetCommand};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

/// IM_DO_NOT_DISTURB_AUTO_RESPONSE.
pub const AUTO_RESPONSE: u8 = 20;
const TELEPORT_REQUEST: u8 = 26;
/// ANIM_AGENT_AWAY / ANIM_AGENT_DO_NOT_DISTURB (llanimationstates.cpp).
pub const ANIM_AWAY: Uuid = Uuid::from_u128(0xfd037134_85d4_f241_72c6_4f42164fedee);
pub const ANIM_DO_NOT_DISTURB: Uuid = Uuid::from_u128(0xefcf670c_2d18_8128_973a_034ebc806b67);

/// Modes of the "Statut de connexion" menu. Away and do not disturb last
/// for the session; the others are kept like Firestorm's per-account
/// settings (in `AutoResponse`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusModes {
    pub away: bool,
    pub dnd: bool,
    pub autorespond: bool,
    pub autorespond_nonfriends: bool,
    pub reject_teleports: bool,
    pub reject_group_invites: bool,
    pub reject_friendship: bool,
    /// Conférences ad hoc: FSIgnoreAdHocSessions, FSReportIgnoredAdHocSession,
    /// FSDontIgnoreAdHocFromFriends.
    pub ignore_adhoc: bool,
    pub report_ignored_adhoc: bool,
    pub adhoc_from_friends: bool,
}

/// Texts and options (DoNotDisturbModeResponse, FSAutorespondModeResponse...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoResponse {
    pub autorespond: bool,
    pub autorespond_nonfriends: bool,
    pub reject_teleports: bool,
    pub reject_group_invites: bool,
    pub reject_friendship: bool,
    pub ignore_adhoc: bool,
    pub report_ignored_adhoc: bool,
    pub adhoc_from_friends: bool,
    pub dnd_text: String,
    pub autorespond_text: String,
    pub nonfriends_text: String,
    /// FSSendAwayAvatarResponse.
    pub send_away: bool,
    pub away_text: String,
    /// FSSendMutedAvatarResponse: answer blocked residents' IMs.
    pub send_muted: bool,
    pub muted_text: String,
    pub reject_teleports_text: String,
    /// FSDontRejectTeleportOffersFromFriends.
    pub dont_reject_friends_teleports: bool,
    pub reject_friendship_text: String,
    /// AFKTimeout in minutes (0 = never away automatically).
    pub away_after_minutes: u32,
}

impl Default for AutoResponse {
    fn default() -> Self {
        // Firestorm's French defaults (strings.xml), [APP_NAME] = Aurora Viewer
        AutoResponse {
            autorespond: false,
            autorespond_nonfriends: false,
            reject_teleports: false,
            reject_group_invites: false,
            reject_friendship: false,
            ignore_adhoc: false,
            report_ignored_adhoc: false,
            adhoc_from_friends: false,
            dnd_text: "Ce résident a activé Ne pas déranger et verra votre message plus tard.".into(),
            autorespond_text: "Le résident auquel vous avez envoyé un message a activé le mode réponse automatique du client \
                Aurora Viewer, ce qui signifie qu'il a demandé à ne pas être dérangé. Votre message sera quand même affiché \
                dans la fenêtre de messagerie instantanée pour être consulté ultérieurement."
                .into(),
            nonfriends_text: "Le résident auquel vous avez envoyé un message a activé le mode réponse automatique du client \
                Aurora Viewer, ce qui signifie qu'il a demandé à ne pas être dérangé. Votre message sera quand même affiché \
                dans la fenêtre de messagerie instantanée pour être consulté ultérieurement."
                .into(),
            send_away: false,
            away_text: "Le résident auquel vous avez envoyé un message est actuellement absent. Votre message sera affiché \
                dans la fenêtre de messagerie instantanée pour être consulté ultérieurement."
                .into(),
            send_muted: false,
            muted_text: "Le résident à qui vous avez envoyé un message a bloqué l'envoi de tout message de votre part.".into(),
            reject_teleports_text: "Le résident auquel vous avez envoyé un message a activé le mode rejet de toutes les offres \
                ou demandes de téléportation du client Aurora Viewer, ce qui signifie qu'il a demandé à ne pas être dérangé \
                par les offres ou demandes de téléportation. Vous pouvez quand même lui envoyer un message privé."
                .into(),
            dont_reject_friends_teleports: false,
            reject_friendship_text: "Le résident auquel vous avez envoyé un message a activé le mode rejet de toutes les \
                demandes d'amitié du client Aurora Viewer, ce qui signifie qu'il a demandé à ne pas être dérangé par les \
                demandes d'amitié. Vous pouvez quand même lui envoyer un message privé."
                .into(),
            away_after_minutes: 0,
        }
    }
}

/// Runtime status.
#[derive(Debug, Default)]
pub struct Status {
    pub away: bool,
    pub dnd: bool,
    /// Copy of the settings (refreshed by the app).
    pub cfg: AutoResponse,
}

impl Status {
    pub fn modes(&self) -> StatusModes {
        StatusModes {
            away: self.away,
            dnd: self.dnd,
            autorespond: self.cfg.autorespond,
            autorespond_nonfriends: self.cfg.autorespond_nonfriends,
            reject_teleports: self.cfg.reject_teleports,
            reject_group_invites: self.cfg.reject_group_invites,
            reject_friendship: self.cfg.reject_friendship,
            ignore_adhoc: self.cfg.ignore_adhoc,
            report_ignored_adhoc: self.cfg.report_ignored_adhoc,
            adhoc_from_friends: self.cfg.adhoc_from_friends,
        }
    }

    /// Some automatic response mode is on (getAutoresponseTextForAvatar).
    fn response_for(&self, friend: bool) -> Option<&str> {
        let c = &self.cfg;
        if self.dnd {
            Some(&c.dnd_text)
        } else if c.autorespond_nonfriends && !friend {
            Some(&c.nonfriends_text)
        } else if c.autorespond {
            Some(&c.autorespond_text)
        } else if self.away && c.send_away {
            Some(&c.away_text)
        } else {
            None
        }
    }
}

impl World {
    fn is_friend(&self, id: &Uuid) -> bool {
        self.social.friends.iter().any(|f| f.id == *id)
    }

    fn auto_im(&mut self, to: Uuid, session: Uuid, text: &str) {
        self.groups.out.push(NetCommand::SendImDialog {
            to,
            dialog: AUTO_RESPONSE,
            id: session,
            message: text.to_owned(),
            bucket: Vec::new(),
        });
    }

    /// Away (LLAgent::setAFK / clearAFK): the "away" animation; the AWAY
    /// control flag is added to the agent updates by the app.
    pub fn set_away(&mut self, on: bool) {
        if self.status.away != on {
            self.status.away = on;
            self.groups.out.push(NetCommand::AgentAnimation {
                anim: ANIM_AWAY,
                start: on,
            });
        }
    }

    /// Do not disturb (LLAgent::setDoNotDisturb).
    pub fn set_dnd(&mut self, on: bool) {
        if self.status.dnd != on {
            self.status.dnd = on;
            self.groups.out.push(NetCommand::AgentAnimation {
                anim: ANIM_DO_NOT_DISTURB,
                start: on,
            });
        }
    }

    /// Answer an IM from a blocked resident (FSSendMutedAvatarResponse),
    /// or with the do-not-disturb text when that option is off.
    pub(super) fn answer_blocked_im(&mut self, im: &InstantMessage) {
        if im.dialog != 0 || im.offline || im.to_agent_id.is_nil() {
            return;
        }
        if self.status.cfg.send_muted {
            let t = self.status.cfg.muted_text.clone();
            self.auto_im(im.from_agent_id, im.session_id, &t);
        } else if self.status.dnd && !self.social.has_session(&im.from_agent_id) {
            let t = self.status.cfg.dnd_text.clone();
            self.auto_im(im.from_agent_id, im.session_id, &t);
        }
    }

    /// Offers refused by the status modes (teleports, group invitations,
    /// friendship) and the automatic response to an offer. True when the
    /// IM was consumed.
    pub(super) fn on_status_im(&mut self, im: &InstantMessage) -> bool {
        let from = im.from_agent_id;
        if from.is_nil() || from == self.agent_id {
            return false;
        }
        let friend = self.is_friend(&from);
        match im.dialog {
            d::LURE_USER | TELEPORT_REQUEST => {
                let c = &self.status.cfg;
                if c.reject_teleports && !(c.dont_reject_friends_teleports && friend) {
                    // send_rejecting_tp_offers_message: answered, offer dropped
                    let t = c.reject_teleports_text.clone();
                    self.auto_im(from, im.session_id, &t);
                    return true;
                }
                if let Some(t) = self.status.response_for(friend).map(str::to_owned) {
                    self.auto_im(from, Uuid::nil(), &t);
                }
                false
            }
            d::GROUP_INVITATION => self.status.cfg.reject_group_invites,
            d::FRIENDSHIP_OFFERED => {
                if self.status.cfg.reject_friendship {
                    let t = self.status.cfg.reject_friendship_text.clone();
                    self.auto_im(from, im.session_id, &t);
                    return true;
                }
                false
            }
            AUTO_RESPONSE => {
                // someone's automatic response, in their conversation
                if !im.message.is_empty() {
                    if !im.from_name.is_empty() {
                        self.social.names.entry(from).or_insert_with(|| im.from_name.clone());
                    }
                    let s = self.social.session_mut(from);
                    s.lines.push(ChatLine {
                        time: SystemTime::now(),
                        from: im.from_name.clone(),
                        text: format!("(réponse automatique) {}", im.message),
                        kind: ChatKind::Im,
                        source: from,
                    });
                    s.unread += 1;
                }
                true
            }
            _ => false,
        }
    }

    /// Before a 1:1 IM is shown: the automatic response to send when this
    /// message opens the conversation (old "do not disturb" behaviour: once
    /// per session).
    pub(super) fn autoresponse_due(&self, im: &InstantMessage) -> Option<String> {
        if im.dialog != 0 || im.offline || im.from_agent_id.is_nil() || im.to_agent_id.is_nil() || im.session_id.is_nil() {
            return None;
        }
        if self.social.has_session(&im.from_agent_id) {
            return None;
        }
        self.status.response_for(self.is_friend(&im.from_agent_id)).map(str::to_owned)
    }

    /// Send it and note it in the conversation (FIRE-5389).
    pub(super) fn send_autoresponse(&mut self, im: &InstantMessage, text: Option<String>) {
        let Some(text) = text else {
            return;
        };
        self.auto_im(im.from_agent_id, im.session_id, &text);
        let s = self.social.session_mut(im.from_agent_id);
        s.lines.push(ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: format!("Réponse automatique envoyée : {text}"),
            kind: ChatKind::System,
            source: Uuid::nil(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_priority() {
        let mut s = Status::default();
        assert!(s.response_for(false).is_none());
        s.away = true;
        assert!(s.response_for(false).is_none(), "away answers only with the option");
        s.cfg.send_away = true;
        assert_eq!(s.response_for(false), Some(s.cfg.away_text.as_str()));
        s.cfg.autorespond_nonfriends = true;
        assert_eq!(s.response_for(false), Some(s.cfg.nonfriends_text.as_str()));
        assert_eq!(s.response_for(true), Some(s.cfg.away_text.as_str()));
        s.dnd = true;
        assert_eq!(s.response_for(true), Some(s.cfg.dnd_text.as_str()));
    }
}
