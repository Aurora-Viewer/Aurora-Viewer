//! Online status and automatic responses, after Firestorm (originally LGPL 2.1):
//! Communicate > Online status (away, do not disturb, autorespond to
//! everyone / to non-friends, reject teleports, group invitations and
//! friendship requests) and Preferences > Privacy > Autoresponse
//! (llimprocessing.cpp, LLIMProcessing::getAutoresponseTextForAvatar,
//! send_rejecting_tp_offers_message, LLAgent::setAFK / setDoNotDisturb).
//! "Invisible" is Firestorm's button of the friends list that hides our
//! online status from every friend (GlobalOnlineStatusToggle,
//! applyGlobalOnlineStatusChange in llviewercontrol.cpp), moved to the
//! status choices at the human's request.

use super::notifications::im as d;
use super::social::{Friend, Social, rights};
use super::{ChatKind, ChatLine, Origin, World};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::{InstantMessage, NetCommand, control};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};
use uuid::Uuid;

/// The "Invisible" status of the account, kept between sessions like
/// Firestorm's per-account GlobalOnlineStatusToggle.
pub const INVISIBLE_FILE: &str = "invisible.xml";

/// IM_DO_NOT_DISTURB_AUTO_RESPONSE.
pub const AUTO_RESPONSE: u8 = 20;
/// ANIM_AGENT_AWAY / ANIM_AGENT_DO_NOT_DISTURB (llanimationstates.cpp).
pub const ANIM_AWAY: Uuid = Uuid::from_u128(0xfd037134_85d4_f241_72c6_4f42164fedee);
pub const ANIM_DO_NOT_DISTURB: Uuid = Uuid::from_u128(0xefcf670c_2d18_8128_973a_034ebc806b67);

/// Modes of the "Online status" menu. Away and do not disturb last
/// for the session; the others are kept like Firestorm's per-account
/// settings (in `AutoResponse`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusModes {
    pub away: bool,
    pub dnd: bool,
    /// Our online status is hidden from every friend.
    pub invisible: bool,
    pub autorespond: bool,
    pub autorespond_nonfriends: bool,
    pub reject_teleports: bool,
    pub reject_group_invites: bool,
    pub reject_friendship: bool,
    /// Ad hoc conferences: FSIgnoreAdHocSessions, FSReportIgnoredAdHocSession,
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
    /// AvatarSitOnAway: sit on the ground when going away (off).
    pub sit_when_away: bool,
    /// QuitAfterSecondsOfAFK in minutes: quit the viewer after being away
    /// this long (0 = never, the default).
    pub quit_after_away_minutes: u32,
    /// "Only friends and groups can call or IM me" (VoiceCallsFriendsOnly,
    /// per account, off).
    pub friends_only: bool,
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
            dnd_text: aurora_i18n::tr!("world-status-this-resident-has-enabled-do-not-disturb-and-will").into(),
            autorespond_text: aurora_i18n::tr!("world-status-the-resident-you-messaged-has-enabled-aurora-viewer-s-3").into(),
            nonfriends_text: aurora_i18n::tr!("world-status-the-resident-you-messaged-has-enabled-aurora-viewer-s-3").into(),
            send_away: false,
            away_text: aurora_i18n::tr!("world-status-the-resident-you-messaged-is-away-your-message-will").into(),
            send_muted: false,
            muted_text: aurora_i18n::tr!("world-status-the-resident-you-messaged-has-blocked-all-messages-from").into(),
            reject_teleports_text: aurora_i18n::tr!("world-status-the-resident-you-messaged-has-enabled-aurora-viewer-s-2").into(),
            dont_reject_friends_teleports: false,
            reject_friendship_text: aurora_i18n::tr!("world-status-the-resident-you-messaged-has-enabled-aurora-viewer-s").into(),
            away_after_minutes: 0,
            sit_when_away: false,
            quit_after_away_minutes: 0,
            friends_only: false,
        }
    }
}

/// "Invisible": no friend sees us online. The right lives on the server,
/// one friend at a time (GrantUserRights); unlike Firestorm, which gives it
/// back to every friend, the friends who had it are remembered, so that
/// leaving the status restores exactly what it took.
#[derive(Debug, Default)]
pub struct Invisible {
    pub on: bool,
    /// Friends who saw us online when the status was set.
    restore: Vec<Uuid>,
    path: Option<PathBuf>,
}

impl Invisible {
    /// The status the account was left in, kept until it is left like the
    /// Legacy Viewer's per-account GlobalOnlineStatusToggle, whatever the
    /// buddy list of the login says: `Status::resume_invisible` hides again
    /// the remembered friends it still shows us to. (The status used to be
    /// dropped as soon as one of them saw us online, which left all the
    /// others hidden under "Available".)
    pub fn load(path: PathBuf) -> Invisible {
        let saved = std::fs::read(&path).ok().and_then(|b| aurora_llsd::from_xml(&b).ok());
        let restore: Vec<Uuid> = saved
            .as_ref()
            .map(|v| {
                v["restore"]
                    .as_array()
                    .iter()
                    .map(Llsd::as_uuid)
                    .filter(|id| !id.is_nil())
                    .collect()
            })
            .unwrap_or_default();
        let on = saved.is_some_and(|v| v["invisible"].as_bool());
        Invisible {
            on,
            restore: if on { restore } else { Vec::new() },
            path: Some(path),
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let restore: Vec<Llsd> = self.restore.iter().copied().map(Llsd::from).collect();
        let state = llsd_map! {"invisible" => self.on, "restore" => restore};
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&state)) {
            log::warn!("invisible status not saved ({e}): {}", path.display());
        }
    }
}

/// The friend may see us online.
fn sees_us(f: &Friend) -> bool {
    f.rights_given & rights::ONLINE_STATUS != 0
}

/// The right to see us online, taken from or given back to one friend
/// (LLAvatarPropertiesProcessor::sendFriendRights): the other rights stay
/// as they are, as in applyGlobalOnlineStatusChange. True when it changed.
fn show_online_to(social: &mut Social, out: &mut Vec<NetCommand>, friend: Uuid, visible: bool) -> bool {
    let Some(f) = social.friends.iter_mut().find(|f| f.id == friend) else {
        return false;
    };
    let given = if visible {
        f.rights_given | rights::ONLINE_STATUS
    } else {
        f.rights_given & !rights::ONLINE_STATUS
    };
    let changed = given != f.rights_given;
    if changed {
        f.rights_given = given;
        out.push(NetCommand::GrantUserRights { friend, rights: given });
    }
    changed
}

/// Runtime status.
#[derive(Debug, Default)]
pub struct Status {
    pub away: bool,
    /// When away started (LLAgent's gAwayTimer), for the quit after a long
    /// time away.
    pub away_since: Option<Instant>,
    /// We sat down because we went away (LLAgent::mIsAfkSitting): coming
    /// back stands us up again.
    afk_sitting: bool,
    pub dnd: bool,
    pub invisible: Invisible,
    /// Copy of the settings (refreshed by the app).
    pub cfg: AutoResponse,
}

impl Status {
    pub fn modes(&self) -> StatusModes {
        StatusModes {
            away: self.away,
            dnd: self.dnd,
            invisible: self.invisible.on,
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

    /// A friendship made while invisible: the server lets a new friend see
    /// us online, so the right is taken back at once and remembered.
    pub(super) fn hide_from_new_friend(&mut self, social: &mut Social, out: &mut Vec<NetCommand>, friend: Uuid) {
        if !self.invisible.on || self.invisible.restore.contains(&friend) {
            return;
        }
        show_online_to(social, out, friend, false);
        self.invisible.restore.push(friend);
        self.invisible.save();
    }

    /// At login, while invisible: a remembered friend whom the buddy list
    /// still shows us to (a right not applied before we left, or given back
    /// from another viewer) is hidden again, so that the status says what
    /// the friends see. The number of friends hidden again.
    pub(super) fn resume_invisible(&mut self, social: &mut Social, out: &mut Vec<NetCommand>) -> usize {
        if !self.invisible.on {
            return 0;
        }
        self.invisible
            .restore
            .iter()
            .filter(|id| show_online_to(social, out, **id, false))
            .count()
    }

    /// The right to see us online given back by hand to a remembered friend
    /// while invisible (Contacts, profile, menus): the user's choice for
    /// that friend, which the status no longer touches, neither at login nor
    /// when it is left.
    pub(super) fn rights_set_by_hand(&mut self, friend: Uuid, given: i32) {
        if !self.invisible.on || given & rights::ONLINE_STATUS == 0 {
            return;
        }
        let before = self.invisible.restore.len();
        self.invisible.restore.retain(|id| *id != friend);
        if self.invisible.restore.len() != before {
            self.invisible.save();
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
    pub(super) fn is_friend(&self, id: &Uuid) -> bool {
        self.social.friends.iter().any(|f| f.id == *id)
    }

    pub(super) fn auto_im(&mut self, to: Uuid, session: Uuid, text: &str) {
        self.groups.out.push(NetCommand::SendImDialog {
            to,
            dialog: AUTO_RESPONSE,
            id: session,
            message: text.to_owned(),
            bucket: Vec::new(),
        });
    }

    /// Away (LLAgent::setAFK / clearAFK): the "away" animation; the AWAY
    /// control flag is added to the agent updates by the app. With "Sit
    /// when away" (AvatarSitOnAway) the avatar sits on the ground when it
    /// goes away standing, and stands up when it comes back; an avatar that
    /// was already sitting stays seated.
    pub fn set_away(&mut self, on: bool) {
        if self.status.away == on {
            return;
        }
        self.status.away = on;
        self.groups.out.push(NetCommand::AgentAnimation {
            anim: ANIM_AWAY,
            start: on,
        });
        if on {
            self.status.away_since = Some(Instant::now());
            if self.status.cfg.sit_when_away && !self.agent.is_sitting() {
                self.groups.out.push(NetCommand::OneShotControl(control::SIT_ON_GROUND));
                self.status.afk_sitting = true;
            }
        } else {
            self.status.away_since = None;
            if std::mem::take(&mut self.status.afk_sitting) && self.status.cfg.sit_when_away && self.agent.ground_sit {
                self.groups.out.push(NetCommand::OneShotControl(control::STAND_UP));
            }
        }
    }

    /// QuitAfterSecondsOfAFK: away for longer than the setting, the viewer
    /// quits (LLAppViewer::idle). The Legacy Viewer restarts its away timer
    /// on every idle frame, so its check seldom fires after an automatic
    /// away; here the time counts from the moment we went away, whatever
    /// made us away.
    pub fn away_too_long(&self, now: Instant) -> bool {
        let limit = self.status.cfg.quit_after_away_minutes as u64 * 60;
        limit > 0 && self.status.away_since.is_some_and(|t| now.duration_since(t).as_secs() >= limit)
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

    /// "Invisible" (applyGlobalOnlineStatusChange): one GrantUserRights
    /// per friend, which the servers take a while to apply. Firestorm asks
    /// for a confirmation because it cannot undo it; here leaving the
    /// status gives the right back to the friends who had it, and to them
    /// only.
    pub fn set_invisible(&mut self, on: bool) {
        if self.status.invisible.on == on {
            return;
        }
        let friends: Vec<Uuid> = if on {
            self.social.friends.iter().filter(|f| sees_us(f)).map(|f| f.id).collect()
        } else {
            std::mem::take(&mut self.status.invisible.restore)
        };
        // (a remembered friend may be gone, or see us again already)
        let count = friends
            .iter()
            .filter(|id| show_online_to(&mut self.social, &mut self.groups.out, **id, !on))
            .count();
        let text = if on {
            aurora_i18n::tr!("world-status-invisible-on", count = count)
        } else {
            aurora_i18n::tr!("world-status-invisible-off", count = count)
        };
        if on {
            self.status.invisible.restore = friends;
        }
        self.status.invisible.on = on;
        self.status.invisible.save();
        self.system_message(text);
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
            d::LURE_USER | d::TELEPORT_REQUEST => {
                // Forced acceptance overrides the user's rejection/status
                // modes (LLIMProcessing::processNewMessage). DnD is still
                // reported when the offer is actually accepted or blocked.
                if self
                    .rlva
                    .auto_accept(if im.dialog == d::LURE_USER { "accepttp" } else { "accepttprequest" }, from)
                {
                    return false;
                }
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
                self.social.auto_responded.insert(from);
                if !im.message.is_empty() {
                    if !im.from_name.is_empty() {
                        self.social.names.entry(from).or_insert_with(|| im.from_name.clone());
                    }
                    self.social.session_mut(from);
                    let line = ChatLine {
                        time: SystemTime::now(),
                        from: im.from_name.clone(),
                        text: aurora_i18n::tr!("world-status-auto-response", value0 = im.message.to_string()),
                        kind: ChatKind::Im,
                        source: from,
                        origin: Origin::Live,
                    };
                    self.conversation_push(from, line, true);
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
        // LLIMMgr::addMessage / processNewMessage: receiving a staff IM
        // remains allowed under recvim; it must not disclose our auto reply.
        if !self.rlva_allows_im("recvim", im.from_agent_id) || super::mutes::is_linden(&im.from_name) {
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
        self.social.session_mut(im.from_agent_id);
        let line = ChatLine {
            time: SystemTime::now(),
            from: String::new(),
            text: aurora_i18n::tr!("world-status-auto-reply-sent", text = text.to_string()),
            kind: ChatKind::System,
            source: Uuid::nil(),
            origin: Origin::Live,
        };
        self.conversation_push(im.from_agent_id, line, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn friend(n: u128, given: i32) -> Friend {
        Friend {
            id: Uuid::from_u128(n),
            online: false,
            rights_given: given,
            rights_has: 0,
        }
    }

    fn rights_sent(world: &mut World) -> Vec<(Uuid, i32)> {
        world
            .groups
            .take_commands()
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::GrantUserRights { friend, rights } => Some((friend, rights)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn invisible_takes_the_online_right_and_gives_it_back_to_who_had_it() {
        let dir = std::env::temp_dir().join(format!("aurora-invisible-{}", Uuid::new_v4()));
        let path = dir.join(INVISIBLE_FILE);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        // online + map + modify, online only, map without online, nothing
        world.social.friends = vec![friend(1, 7), friend(2, 1), friend(3, 2), friend(4, 0)];
        world.status.invisible = Invisible::load(path.clone());
        assert!(!world.status.invisible.on);
        let id = Uuid::from_u128;
        world.set_invisible(true);
        // the other rights are kept; who did not see us is left alone
        assert_eq!(rights_sent(&mut world), vec![(id(1), 6), (id(2), 0)]);
        assert!(world.status.modes().invisible);
        // a friendship made meanwhile is hidden too
        world.social.form_friendship(id(5));
        world.status.hide_from_new_friend(&mut world.social, &mut world.groups.out, id(5));
        assert_eq!(rights_sent(&mut world), vec![(id(5), 0)]);
        // the next session finds the status and who to give the right back to
        let again = Invisible::load(path.clone());
        assert!(again.on);
        assert_eq!(again.restore, vec![id(1), id(2), id(5)]);
        world.set_invisible(false);
        assert_eq!(rights_sent(&mut world), vec![(id(1), 7), (id(2), 1), (id(5), 1)]);
        assert!(!Invisible::load(path.clone()).on);
        // the login's buddy list still shows us to a remembered friend (a
        // right not applied, or given back from another viewer): the status
        // holds and that friend is hidden again, the others left alone
        world.set_invisible(true);
        rights_sent(&mut world);
        world.social.friends[0].rights_given = 7;
        world.status.invisible = Invisible::load(path.clone());
        assert!(world.status.modes().invisible);
        assert_eq!(world.status.resume_invisible(&mut world.social, &mut world.groups.out), 1);
        assert_eq!(rights_sent(&mut world), vec![(id(1), 6)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_friend_shown_by_hand_while_invisible_is_left_to_the_user() {
        let dir = std::env::temp_dir().join(format!("aurora-invisible-{}", Uuid::new_v4()));
        let path = dir.join(INVISIBLE_FILE);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.social.friends = vec![friend(1, 1), friend(2, 3)];
        world.status.invisible = Invisible::load(path.clone());
        let id = Uuid::from_u128;
        world.set_invisible(true);
        rights_sent(&mut world);
        // a map right changed by hand: still hidden, still remembered
        world.profile_command(&NetCommand::GrantUserRights { friend: id(2), rights: 0 });
        // the online right given back by hand to friend 1 (Contacts)
        world.profile_command(&NetCommand::GrantUserRights { friend: id(1), rights: 1 });
        assert_eq!(Invisible::load(path.clone()).restore, vec![id(2)]);
        // not hidden again at the next login, not touched when leaving
        world.status.invisible = Invisible::load(path);
        assert_eq!(world.status.resume_invisible(&mut world.social, &mut world.groups.out), 0);
        world.set_invisible(false);
        assert_eq!(rights_sent(&mut world), vec![(id(2), 1)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_automatic_response_marks_its_sender_until_their_next_message() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let tess = Uuid::from_u128(7);
        let im = |dialog: u8, message: &str| InstantMessage {
            from_agent_id: tess,
            from_name: "Tess Touch".into(),
            to_agent_id: Uuid::nil(),
            dialog,
            session_id: Uuid::nil(),
            message: message.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        };
        world.on_im(im(AUTO_RESPONSE, "Busy"));
        assert!(world.social.auto_responded.contains(&tess));
        // her next ordinary message: the hint is gone
        world.on_im(im(0, "back!"));
        assert!(!world.social.auto_responded.contains(&tess));
    }

    fn sit_controls(world: &mut World) -> Vec<u32> {
        world
            .groups
            .take_commands()
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::OneShotControl(flags) => Some(flags),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sit_when_away_sits_a_standing_avatar_and_stands_it_up_on_return() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        // off: away only plays the animation
        world.set_away(true);
        world.set_away(false);
        assert!(sit_controls(&mut world).is_empty());
        world.status.cfg.sit_when_away = true;
        world.set_away(true);
        assert_eq!(sit_controls(&mut world), vec![control::SIT_ON_GROUND]);
        world.agent.ground_sit = true;
        world.set_away(false);
        assert_eq!(sit_controls(&mut world), vec![control::STAND_UP]);
        // already sitting: stays seated on the way back
        world.agent.ground_sit = true;
        world.set_away(true);
        world.set_away(false);
        assert!(sit_controls(&mut world).is_empty());
        // stood up meanwhile: nothing to stand up from
        world.agent.ground_sit = false;
        world.set_away(true);
        sit_controls(&mut world);
        world.set_away(false);
        assert!(sit_controls(&mut world).is_empty());
    }

    #[test]
    fn quit_after_away_counts_from_the_moment_away_started() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let now = Instant::now();
        world.set_away(true);
        assert!(!world.away_too_long(now + std::time::Duration::from_secs(3600)), "off by default");
        world.status.cfg.quit_after_away_minutes = 5;
        assert!(!world.away_too_long(now + std::time::Duration::from_secs(4 * 60)));
        assert!(world.away_too_long(now + std::time::Duration::from_secs(5 * 60 + 1)));
        world.set_away(false);
        assert!(!world.away_too_long(now + std::time::Duration::from_secs(3600)));
    }

    #[test]
    fn an_invitation_to_a_group_we_are_in_is_dropped_unless_asked_for() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let group = Uuid::from_u128(9);
        world.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Builders".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        let invite = |from: Uuid| InstantMessage {
            from_agent_id: from,
            from_name: "Builders".into(),
            to_agent_id: Uuid::nil(),
            dialog: d::GROUP_INVITATION,
            session_id: Uuid::from_u128(1),
            message: "Join us".into(),
            offline: false,
            from_group: true,
            binary_bucket: Vec::new(),
            timestamp: 0,
        };
        let shown = |world: &World| world.notifications.list.len();
        world.on_im(invite(group));
        assert_eq!(shown(&world), 0, "already a member");
        world.on_im(invite(Uuid::from_u128(8)));
        assert_eq!(shown(&world), 1, "another group");
        world.groups.show_joined_invitations = true;
        world.on_im(invite(group));
        assert_eq!(shown(&world), 2);
    }

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
