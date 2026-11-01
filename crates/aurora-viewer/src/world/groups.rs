//! Groups and multi-agent chat sessions (group chat, ad-hoc conferences),
//! after Firestorm's LLIMMgr / LLIMModel (llimview.cpp, llimprocessing.cpp)
//! and exoGroupMuteList (originally LGPL 2.1):
//! - the group list comes from AgentGroupDataUpdate;
//! - someone else's first message arrives as a ChatterBoxInvitation, which
//!   we accept through ChatSessionRequest ("accept invitation"); the next
//!   ones are ImprovedInstantMessage IM_SESSION_SEND;
//! - opening a group chat ourselves sends IM_SESSION_GROUP_START (session id
//!   = group id), answered by ChatterBoxSessionStartReply, then the recent
//!   messages are fetched ("fetch history", FetchGroupChatHistory);
//! - closing sends IM_SESSION_LEAVE; a blocked group chat is left at once;
//! - snoozing a group chat closes it without leaving: its messages are
//!   dropped until the snooze ends, then the next one opens it again
//!   (LLIMMgr::leaveSession with CLOSE_SNOOZE, checkSnoozeExpiration,
//!   restoreSnoozedSession).
//!
//! The lines live in `social.ims` like 1:1 conversations (keyed by session
//! id); this module keeps what is specific to multi-agent sessions.

use super::social::ImSession;
use super::{ChatKind, ChatLine, Origin, World};
use aurora_net::{GroupMembership, NetCommand, NetEvent, SessionAgent, SessionMethod};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// Pinned groups of the account (FSFavoriteGroups, a per-account setting in
/// Firestorm).
pub const FAVORITE_GROUPS_FILE: &str = "favorite_groups.xml";

/// IM dialogs of chat sessions (llinstantmessage.h).
pub mod dialog {
    pub const SESSION_INVITE: u8 = 13;
    pub const SESSION_GROUP_START: u8 = 15;
    pub const SESSION_SEND: u8 = 17;
    pub const SESSION_LEAVE: u8 = 18;
}

/// Largest IM text (MAX_MSG_BUF_SIZE - 1): longer messages are split
/// (FIRE-787).
const MAX_IM_BYTES: usize = 1023;

#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    pub name: String,
    /// Group chat (session id = group id); else an ad-hoc conference.
    pub group: bool,
    pub participants: BTreeSet<Uuid>,
    /// Those who were in it and left (a conference's offline members).
    pub left: BTreeSet<Uuid>,
    pub moderators: HashSet<Uuid>,
    /// Our text muted by a moderator.
    pub text_muted: bool,
    /// The server confirmed the session (start reply / accepted invitation).
    pub active: bool,
    /// The server closed it (ForceCloseChatterBoxSession).
    pub closed: bool,
    history_fetched: bool,
    /// We opened it ourselves (IM_SESSION_GROUP_START), no message did.
    pub(super) started_by_us: bool,
}

#[derive(Default)]
pub struct GroupChats {
    pub groups: Vec<GroupMembership>,
    pub sessions: HashMap<Uuid, SessionInfo>,
    /// Group chats that arrived before the mute list (restored once it is
    /// loaded, exoGroupMuteList::addDeferredGroupChat).
    deferred: Vec<Uuid>,
    /// Snoozed group chats and when their snooze ends
    /// (LLIMMgr::mSnoozedSessions).
    snoozed: HashMap<Uuid, Instant>,
    /// FSMuteAllGroups / FSMuteGroupWhenNoticesDisabled (from the settings).
    pub mute_all_groups: bool,
    pub mute_when_notices_off: bool,
    /// FSReportBlockToNearbyChat: say in nearby chat when the list changes.
    pub report_blocks: bool,
    /// FSShowJoinedGroupInvitations: show the invitations to a group we are
    /// already in (from the settings).
    pub show_joined_invitations: bool,
    /// Group list asked (AgentDataUpdateRequest).
    requested: bool,
    /// Our active group (AgentDataUpdate; nil = none) and its title.
    pub active: Uuid,
    pub active_title: String,
    /// Pinned groups, listed first (FSFavoriteGroups).
    pub favorites: HashSet<Uuid>,
    favorites_path: Option<PathBuf>,
    /// The name of the group whose CreateGroupRequest waits for its reply:
    /// the new group is listed under it until the list comes back.
    pub creating: Option<String>,
    pub out: Vec<NetCommand>,
}

impl GroupChats {
    pub fn group(&self, id: &Uuid) -> Option<&GroupMembership> {
        self.groups.iter().find(|g| g.id == *id)
    }

    pub fn is_member(&self, id: &Uuid) -> bool {
        self.group(id).is_some()
    }

    /// Snooze a group chat until `until`; a second snooze moves the end.
    fn snooze(&mut self, group: Uuid, until: Instant) {
        self.snoozed.insert(group, until);
    }

    /// LLIMMgr::checkSnoozeExpiration: snoozed, and over at `now`.
    fn snooze_over(&self, group: &Uuid, now: Instant) -> bool {
        self.snoozed.get(group).is_some_and(|until| *until <= now)
    }

    /// The group chat is no longer snoozed (it was opened again).
    fn unsnooze(&mut self, group: &Uuid) -> bool {
        self.snoozed.remove(group).is_some()
    }

    /// Groups sorted like LLGroupComparator: pinned ones first, then by
    /// name.
    pub fn sorted(&self) -> Vec<GroupMembership> {
        let mut v = self.groups.clone();
        v.sort_by_cached_key(|g| (!self.favorites.contains(&g.id), g.name.to_uppercase()));
        v
    }

    /// FSFavoriteGroups::loadFavorites (an LLSD array of group ids).
    pub fn load_favorites(&mut self, path: PathBuf) {
        self.favorites = std::fs::read(&path)
            .ok()
            .and_then(|b| aurora_llsd::from_xml(&b).ok())
            .map(|v| v.as_array().iter().map(|g| g.as_uuid()).filter(|g| !g.is_nil()).collect())
            .unwrap_or_default();
        self.favorites_path = Some(path);
    }

    /// Pin / unpin a group (FSFavoriteGroups::toggleFavorite), saved at once.
    pub fn toggle_favorite(&mut self, group: Uuid) {
        if !self.favorites.remove(&group) {
            self.favorites.insert(group);
        }
        let Some(path) = &self.favorites_path else {
            return;
        };
        let mut ids: Vec<Uuid> = self.favorites.iter().copied().collect();
        ids.sort();
        let list = aurora_llsd::Llsd::from(ids.into_iter().map(aurora_llsd::Llsd::from).collect::<Vec<_>>());
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&list)) {
            log::warn!("pinned groups not saved ({e}): {}", path.display());
        }
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }

    /// Receive the notices of a group or not (LLAgent::setUserGroupFlags,
    /// which Firestorm reaches from the group profile): our copy follows at
    /// once, the simulator does not answer. The listing in our profile
    /// travels in the same message and is sent as it is.
    pub fn set_accept_notices(&mut self, group: Uuid, on: bool) {
        let Some(g) = self.groups.iter_mut().find(|g| g.id == group) else {
            return;
        };
        g.accept_notices = on;
        self.out.push(NetCommand::SetGroupFlags {
            group,
            accept_notices: on,
            list_in_profile: g.list_in_profile,
        });
    }

    /// Both flags of LLAgent::setUserGroupFlags at once (the group
    /// profile's "Receive notices" and "Show in my profile").
    pub fn set_flags(&mut self, group: Uuid, accept_notices: bool, list_in_profile: bool) {
        let Some(g) = self.groups.iter_mut().find(|g| g.id == group) else {
            return;
        };
        g.accept_notices = accept_notices;
        g.list_in_profile = list_in_profile;
        self.out.push(NetCommand::SetGroupFlags {
            group,
            accept_notices,
            list_in_profile,
        });
    }

    /// LLAgent::setGroupContribution: our copy follows at once, then
    /// SetGroupContribution (the simulator does not answer). False when we
    /// are not in the group.
    pub fn set_contribution(&mut self, group: Uuid, contribution: i32) -> bool {
        let Some(g) = self.groups.iter_mut().find(|g| g.id == group) else {
            return false;
        };
        g.contribution = contribution;
        self.out.push(NetCommand::SetGroupContribution { group, contribution });
        true
    }

    fn im(&mut self, to: Uuid, dialog: u8, session: Uuid, message: String) {
        self.out.push(NetCommand::SendImDialog {
            to,
            dialog,
            id: session,
            message,
            bucket: Vec::new(),
        });
    }
}

fn line(from: String, text: String, kind: ChatKind, source: Uuid, time: SystemTime) -> ChatLine {
    ChatLine {
        time,
        from,
        text,
        kind,
        source,
        origin: Origin::Live,
    }
}

/// Split a message at character boundaries into IM-sized pieces.
pub fn split_im(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while rest.len() > MAX_IM_BYTES {
        let mut cut = MAX_IM_BYTES;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        // prefer a space near the end
        if let Some(sp) = rest[..cut].rfind(' ').filter(|&i| i > cut / 2) {
            cut = sp + 1;
        }
        out.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

impl World {
    /// Group used by LLToolPlacer::addObject, LLToolDragAndDrop::dropObject
    /// and cmdline_rezplat: FSCommon::getGroupForRezzing
    /// (indra/newview/fscommon.cpp, originally LGPL 2.1).
    pub fn group_for_rez(&self, under_land: bool) -> Uuid {
        self.parcel
            .as_ref()
            .filter(|_| under_land)
            .map(|p| p.group_id)
            .filter(|id| !id.is_nil() && self.groups.is_member(id))
            .unwrap_or(self.groups.active)
    }

    /// The current count shared by group profiles and search results.
    pub fn group_member_count(&self, id: Uuid) -> Option<i64> {
        self.group_data.member_count(id, self.search.groups.get(&id))
    }

    /// A multi-agent session (group or conference).
    pub fn is_group_session(&self, session: &Uuid) -> bool {
        self.groups.sessions.contains_key(session)
    }

    /// Title of a conversation: group / conference name, else the avatar.
    pub fn session_title(&self, session: &Uuid) -> String {
        match self.groups.sessions.get(session) {
            Some(s) if !s.name.is_empty() => s.name.clone(),
            Some(_) => aurora_i18n::tr!("world-groups-conference").to_owned(),
            None => self.social.name_of(session),
        }
    }

    /// The conversation lines of a session (created when missing, without a
    /// name lookup since its id is not an avatar).
    pub(super) fn session_lines(&mut self, session: Uuid) -> &mut ImSession {
        let i = match self.social.ims.iter().position(|s| s.other == session) {
            Some(i) => i,
            None => {
                self.social.ims.push(ImSession::new(session));
                self.social.ims.len() - 1
            }
        };
        &mut self.social.ims[i]
    }

    /// A message in a group chat or conference: its sound follows the
    /// group / conference IM mode (LLIMMgr::addMessage).
    fn session_sound(&mut self, session: Uuid, new_session: bool) {
        let group = self.groups.sessions.get(&session).is_some_and(|s| s.group) || self.groups.is_member(&session);
        self.im_messages.push(crate::ui_sound::ImMessage {
            kind: if group {
                crate::ui_sound::ImKind::Group
            } else {
                crate::ui_sound::ImKind::Conference
            },
            session,
            new_session,
        });
    }

    fn session_push(&mut self, session: Uuid, l: ChatLine, unread: bool) {
        self.conversation_push(session, l, unread);
    }

    fn session_system(&mut self, session: Uuid, text: impl Into<String>) {
        self.session_push(
            session,
            line(String::new(), text.into(), ChatKind::System, Uuid::nil(), SystemTime::now()),
            false,
        );
    }

    pub(super) fn info_mut(&mut self, session: Uuid, name: &str) -> &mut SessionInfo {
        let group = self.groups.is_member(&session);
        let group_name = self.groups.group(&session).map(|g| g.name.clone());
        let s = self.groups.sessions.entry(session).or_default();
        s.group |= group;
        if let Some(n) = group_name {
            s.name = n;
        } else if s.name.is_empty() && !name.is_empty() {
            s.name = name.to_owned();
        }
        s
    }

    /// Open (or focus) the chat of one of our groups (LLGroupActions::startIM).
    pub fn start_group_chat(&mut self, group: Uuid) {
        let Some(g) = self.groups.group(&group).cloned() else {
            return;
        };
        if !self.rlva_start_ims(&[group]) {
            return;
        }
        if self.mutes.group_chat_muted(&group) {
            // opening it means receiving it again (exoGroupMuteList::remove)
            self.set_group_chat_blocked(group, false);
        }
        self.groups.unsnooze(&group);
        self.social.focus_im = Some(group);
        let known = self.groups.sessions.get(&group).is_some_and(|s| s.active && !s.closed);
        self.info_mut(group, &g.name);
        self.session_lines(group);
        if !known {
            // LLIMModel::sendStartSession(IM_SESSION_GROUP_START)
            self.groups.im(group, dialog::SESSION_GROUP_START, group, String::new());
            if let Some(s) = self.groups.sessions.get_mut(&group) {
                s.started_by_us = true;
            }
        }
    }

    /// Send text to a group / conference session (IM_SESSION_SEND).
    pub fn send_session_im(&mut self, session: Uuid, text: &str) {
        let text = self.rlva_outgoing_im_text(session, true, text).into_owned();
        let me = self.own_name();
        for part in split_im(&text) {
            self.groups.im(session, dialog::SESSION_SEND, session, part.to_owned());
        }
        let l = line(me, text, ChatKind::OWN, Uuid::nil(), SystemTime::now());
        self.session_push(session, l, false);
        if let Some(s) = self.groups.sessions.get(&session)
            && s.closed
        {
            self.session_system(
                session,
                aurora_i18n::tr!("world-groups-the-server-closed-this-session-the-message-may-not"),
            );
        }
    }

    /// Close a conversation: a group / conference session is left
    /// (LLIMModel::sendLeaveSession), a 1:1 conversation only leaves the list
    /// (nothing is sent; the next message from that resident opens it again).
    pub fn leave_session(&mut self, session: Uuid) {
        if self.groups.sessions.remove(&session).is_some() {
            self.groups.im(session, dialog::SESSION_LEAVE, session, String::new());
        }
        self.groups.unsnooze(&session);
        self.social.ims.retain(|s| s.other != session);
    }

    /// Snooze a group chat for `seconds` (FSFloaterIM::snooze, then
    /// LLIMMgr::leaveSession with CLOSE_SNOOZE): the conversation closes but
    /// the session is not left, so the server keeps sending its messages;
    /// they are dropped until the snooze ends, and the first one after it
    /// opens the conversation again. 0 seconds only closes it until the next
    /// message. False when it is not one of our group chats.
    pub fn snooze_group_chat(&mut self, group: Uuid, seconds: u32) -> bool {
        // LLIMSession::isGroupSessionType: only group chats are snoozed
        if !self.groups.sessions.get(&group).is_some_and(|s| s.group) {
            return false;
        }
        self.groups.sessions.remove(&group);
        self.social.ims.retain(|s| s.other != group);
        self.groups.snooze(group, Instant::now() + Duration::from_secs(seconds.into()));
        log::info!("group chat snoozed for {seconds} s: {group}");
        true
    }

    /// Block / unblock a group's chat (exoGroupMuteList "Group:<id>").
    pub fn set_group_chat_blocked(&mut self, group: Uuid, blocked: bool) {
        use super::mutes::{Mute, MuteType, group_chat_name};
        let name = group_chat_name(&group);
        if blocked {
            // LLGroupActions::endIM first
            if self.groups.sessions.contains_key(&group) {
                self.leave_session(group);
            }
            match self.mutes.add(
                Mute {
                    id: Uuid::nil(),
                    name,
                    kind: MuteType::ByName,
                    flags: 0,
                },
                0,
            ) {
                Ok(cmd) => self.groups.out.push(cmd),
                Err(e) => log::info!("group chat block: {}", e.message()),
            }
        } else if let Some(cmd) = self.mutes.remove(&Uuid::nil(), &name, 0) {
            self.groups.out.push(cmd);
        }
    }

    /// Group chat we do not want (Firestorm's group chat mutes).
    fn group_chat_unwanted(&self, session: &Uuid) -> bool {
        let Some(g) = self.groups.group(session) else {
            return false;
        };
        self.mutes.group_chat_muted(session) || self.groups.mute_all_groups || (self.groups.mute_when_notices_off && !g.accept_notices)
    }

    /// Ask for the group list once we are in world (STATE_INVENTORY_SEND).
    pub(super) fn request_groups_once(&mut self) {
        if !self.groups.requested {
            self.groups.requested = true;
            self.groups.out.push(NetCommand::RequestGroups);
        }
    }

    /// Group chats deferred until the mute list was loaded: open the ones
    /// still wanted (exoGroupMuteList::restoreDeferredGroupChat).
    pub(super) fn restore_deferred_group_chats(&mut self) {
        for group in std::mem::take(&mut self.groups.deferred) {
            if self.group_chat_unwanted(&group) || self.groups.sessions.contains_key(&group) {
                continue;
            }
            let Some(g) = self.groups.group(&group).cloned() else {
                continue;
            };
            log::info!("restoring group chat from {}", g.name);
            self.reopen_group_chat(group, &g.name);
            self.im_messages.push(crate::ui_sound::ImMessage {
                kind: crate::ui_sound::ImKind::Group,
                session: group,
                new_session: true,
            });
        }
    }

    /// A group chat comes back without its invitation (a deferred or a
    /// snoozed one): its conversation opens and the session is joined again
    /// (LLIMMgr::addSession, LLIMModel::sendStartSession).
    fn reopen_group_chat(&mut self, group: Uuid, name: &str) {
        self.info_mut(group, name);
        self.session_lines(group);
        self.groups.im(group, dialog::SESSION_GROUP_START, group, String::new());
    }

    /// LLIMMgr::restoreSnoozedSession, for a message of a snoozed group chat
    /// whose snooze is over: true when the conversation opened again.
    fn restore_snoozed_group_chat(&mut self, group: Uuid) -> bool {
        if !self.groups.unsnooze(&group) {
            return false;
        }
        let Some(g) = self.groups.group(&group).cloned() else {
            return false;
        };
        log::info!("group chat snooze over, reopening {}", g.name);
        self.reopen_group_chat(group, &g.name);
        true
    }

    /// Network events of groups and chat sessions; returns the event when it
    /// is not one of them.
    pub(super) fn on_group_event(&mut self, ev: NetEvent) -> Option<NetEvent> {
        match ev {
            NetEvent::Groups(list) => {
                for g in list {
                    if let Some(s) = self.groups.sessions.get_mut(&g.id) {
                        s.group = true;
                        s.name = g.name.clone();
                    }
                    match self.groups.groups.iter_mut().find(|e| e.id == g.id) {
                        Some(e) => *e = g,
                        None => self.groups.groups.push(g),
                    }
                }
                log::info!("groups: {}", self.groups.groups.len());
            }
            NetEvent::ActiveGroup { id, name, title } => {
                log::info!("active group: {}", if id.is_nil() { "none" } else { &name });
                self.groups.active = id;
                self.groups.active_title = title;
            }
            NetEvent::GroupDropped(id) => {
                self.groups.groups.retain(|g| g.id != id);
                if self.groups.sessions.contains_key(&id) {
                    self.session_system(id, aurora_i18n::tr!("world-groups-you-no-longer-belong-to-this-group"));
                    if let Some(s) = self.groups.sessions.get_mut(&id) {
                        s.closed = true;
                    }
                }
            }
            NetEvent::SessionInvite(inv) => self.on_session_invite(inv),
            NetEvent::SessionStarted {
                temp_session,
                session,
                success,
                error,
                agents,
            } => {
                if !success {
                    // LLIMMgr::showSessionStartError
                    let id = if self.groups.sessions.contains_key(&temp_session) {
                        temp_session
                    } else {
                        session
                    };
                    self.session_system(
                        id,
                        aurora_i18n::tr!("world-groups-could-not-join-the-session", value0 = reason(&error).to_string()),
                    );
                    if let Some(s) = self.groups.sessions.get_mut(&id) {
                        s.closed = true;
                    }
                    return None;
                }
                if temp_session != session && !temp_session.is_nil() {
                    // processSessionInitializedReply: the server's id replaces ours
                    if let Some(info) = self.groups.sessions.remove(&temp_session) {
                        self.groups.sessions.insert(session, info);
                    }
                    if let Some(s) = self.social.ims.iter_mut().find(|s| s.other == temp_session) {
                        s.other = session;
                    }
                }
                let info = self.info_mut(session, "");
                info.active = true;
                info.closed = false;
                apply_agents(info, &agents);
                let fetch = !info.history_fetched;
                info.history_fetched = true;
                if fetch {
                    self.groups.out.push(NetCommand::ChatSession {
                        method: SessionMethod::FetchHistory,
                        session,
                    });
                }
            }
            NetEvent::SessionAgents { session, agents } => {
                if let Some(s) = self.groups.sessions.get_mut(&session) {
                    apply_agents(s, &agents);
                }
                // the residents we invited who joined are announced
                self.conference_joined(session, &agents);
                let me = self.agent_id;
                if let Some(a) = agents.iter().find(|a| a.agent == me)
                    && let (Some(muted), Some(s)) = (a.text_muted, self.groups.sessions.get_mut(&session))
                {
                    let changed = s.text_muted != muted;
                    s.text_muted = muted;
                    if changed {
                        let text = if muted {
                            aurora_i18n::tr!("world-groups-a-moderator-has-muted-your-chat-in-this-session")
                        } else {
                            aurora_i18n::tr!("world-groups-a-moderator-has-unmuted-you")
                        };
                        self.session_system(session, text);
                    }
                }
            }
            NetEvent::SessionError { session, event, error } => {
                // LLIMMgr::showSessionEventError
                let what = if event == "message" {
                    aurora_i18n::tr!("world-groups-message-not-sent")
                } else {
                    aurora_i18n::tr!("world-groups-action-denied")
                };
                self.session_system(session, format!("{what}{}", reason(&error)));
            }
            NetEvent::SessionClosed { session, reason: r } => {
                // LLIMMgr::showSessionForceClose
                if let Some(s) = self.groups.sessions.get_mut(&session) {
                    s.closed = true;
                    s.active = false;
                    self.session_system(
                        session,
                        aurora_i18n::tr!("world-groups-the-session-was-closed", value0 = reason(&r).to_string()),
                    );
                }
            }
            NetEvent::ChatSessionReply {
                method,
                session,
                result,
                not_found,
            } => match method {
                SessionMethod::AcceptInvitation => match result {
                    Some(r) => {
                        let agents = aurora_net::social::parse_session_agents(&r);
                        if let Some(s) = self.groups.sessions.get_mut(&session) {
                            s.active = true;
                            apply_agents(s, &agents);
                        }
                    }
                    // chatterBoxInvitationCoro: only a 404 is shown
                    // (session_does_not_exist_error), any other failure is
                    // logged and the chat goes on
                    None if not_found => {
                        self.session_system(
                            session,
                            aurora_i18n::tr!("world-groups-could-not-join-the-session-it-no-longer-exists"),
                        );
                        if let Some(s) = self.groups.sessions.get_mut(&session) {
                            s.closed = true;
                        }
                    }
                    None => {}
                },
                SessionMethod::FetchHistory => {
                    if let Some(r) = result {
                        self.add_server_history(session, aurora_net::social::parse_history(&r));
                    }
                }
                SessionMethod::DeclineInvitation => {}
            },
            other => return Some(other),
        }
        None
    }

    /// ChatterBoxInvitation with a message (LLViewerChatterBoxInvitation::post
    /// and LLIMMgr::addMessage for a new session).
    fn on_session_invite(&mut self, mut inv: aurora_net::SessionInvite) {
        let session = inv.session;
        if inv.from == self.agent_id || session.is_nil() {
            return;
        }
        let in_group = self.groups.is_member(&session);
        if in_group && !self.rlva_allows_im("recvim", session) {
            return;
        }
        if !in_group && !self.rlva_allows_im("recvim", inv.from) {
            inv.message = aurora_i18n::tr!("rlva-im-blocked-receive").into();
        }
        // VoiceCallsFriendsOnly: a conference started by a non-friend
        // (LLIMMgr::addMessage, IM_SESSION_INVITE outside our groups)
        if !in_group && self.friends_only_drops(&inv.from, &inv.from_name) {
            return;
        }
        if !self.groups.sessions.contains_key(&session) {
            if in_group && !self.mutes.is_loaded() {
                // FS: wait for the mute list before deciding
                if !self.groups.deferred.contains(&session) {
                    self.groups.deferred.push(session);
                }
                return;
            }
            if self.group_chat_unwanted(&session) {
                log::info!("group chat blocked: {}", self.session_title(&session));
                self.groups.im(inv.from, dialog::SESSION_LEAVE, session, String::new());
                return;
            }
            // a session started by someone we blocked: not opened, not left
            // (other members' messages come back as a new invitation)
            if in_group && self.mutes.text_muted(&inv.from, &inv.from_name) {
                return;
            }
            // Communicate > Online status > Ad hoc conferences (FS:PP)
            let c = &self.status.cfg;
            let friend = self.social.friends.iter().any(|f| f.id == inv.from);
            if !in_group && c.ignore_adhoc && !(c.adhoc_from_friends && friend) && !crate::world::mutes::is_linden(&inv.from_name) {
                let report = c.report_ignored_adhoc;
                log::info!("ignoring conference (ad-hoc) chat from {}", inv.from_name);
                self.groups.im(inv.from, dialog::SESSION_LEAVE, session, String::new());
                if report {
                    self.system_message(aurora_i18n::tr!(
                        "world-groups-you-were-invited-to-an-ad-hoc-conference-by",
                        value0 = inv.from_name.to_string()
                    ));
                }
                return;
            }
        }
        if !self.mutes.text_muted(&inv.from, &inv.from_name) {
            let new = !self.groups.sessions.contains_key(&session);
            // an invitation opens the chat even when snoozed, as in the
            // Legacy Viewer (LLViewerChatterBoxInvitation does not look)
            self.groups.unsnooze(&session);
            self.info_mut(session, &inv.session_name).participants.insert(inv.from);
            self.session_sound(session, new);
            if !inv.from_name.is_empty() {
                self.social.names.entry(inv.from).or_insert_with(|| inv.from_name.clone());
            }
            let text = saved_prefix(inv.offline, inv.timestamp) + &inv.message;
            let l = line(inv.from_name.clone(), text, ChatKind::Im, inv.from, SystemTime::now());
            self.session_push(session, l, true);
        }
        self.groups.out.push(NetCommand::ChatSession {
            method: SessionMethod::AcceptInvitation,
            session,
        });
        // recent messages of a session we did not have yet (FetchGroupChatHistory)
        if let Some(s) = self.groups.sessions.get_mut(&session)
            && !s.history_fetched
        {
            s.history_fetched = true;
            self.groups.out.push(NetCommand::ChatSession {
                method: SessionMethod::FetchHistory,
                session,
            });
        }
    }

    /// IM_SESSION_SEND / IM_SESSION_INVITE over UDP (llimprocessing.cpp).
    /// True when the message belonged to a session.
    pub(super) fn on_session_im(&mut self, im: &aurora_net::InstantMessage) -> bool {
        if im.dialog != dialog::SESSION_SEND && im.dialog != dialog::SESSION_INVITE {
            return false;
        }
        let session = im.session_id;
        let mut reopened = false;
        if !self.groups.sessions.contains_key(&session) {
            // only shown once a session is open (after its invitation)
            if self.groups.deferred.contains(&session) && self.mutes.is_loaded() {
                self.restore_deferred_group_chats();
            }
            // a snoozed group chat: dropped until its snooze is over, then
            // the message opens it again (llimprocessing.cpp, IM_SESSION_SEND:
            // not for a blocked sender nor a group we left)
            if !self.groups.sessions.contains_key(&session)
                && self.groups.is_member(&session)
                && !self.mutes.text_muted(&im.from_agent_id, &im.from_name)
                && self.groups.snooze_over(&session, Instant::now())
            {
                reopened = self.restore_snoozed_group_chat(session);
            }
            if !self.groups.sessions.contains_key(&session) {
                if self.groups.snoozed.contains_key(&session) {
                    log::info!("snoozed group chat {session}: message dropped");
                }
                return true;
            }
        }
        if im.from_agent_id == self.agent_id || self.mutes.text_muted(&im.from_agent_id, &im.from_name) {
            return true;
        }
        if !im.from_name.is_empty() {
            self.social.names.entry(im.from_agent_id).or_insert_with(|| im.from_name.clone());
        }
        if let Some(s) = self.groups.sessions.get_mut(&session) {
            s.participants.insert(im.from_agent_id);
        }
        let l = line(
            im.from_name.clone(),
            im.message.clone(),
            ChatKind::Im,
            im.from_agent_id,
            SystemTime::now(),
        );
        // a snoozed chat that opens again sounds as a new one
        // (UISndNewIncomingGroupIMSession in restoreSnoozedSession)
        self.session_sound(session, reopened);
        // on receipt, whether or not the chat is in front (FIRE-10178)
        let open = super::attention::Open::Conversation(session);
        self.keyword_alert(im.from_agent_id, &im.from_name, &im.message, false, open);
        self.session_push(session, l, true);
        true
    }
}

fn apply_agents(s: &mut SessionInfo, agents: &[SessionAgent]) {
    for a in agents {
        match a.present {
            Some(true) => {
                s.participants.insert(a.agent);
                s.left.remove(&a.agent);
            }
            Some(false) if s.participants.remove(&a.agent) => {
                s.left.insert(a.agent);
            }
            Some(false) | None => {}
        }
        match a.moderator {
            Some(true) => {
                s.moderators.insert(a.agent);
            }
            Some(false) => {
                s.moderators.remove(&a.agent);
            }
            None => {}
        }
    }
}

fn reason(r: &str) -> String {
    if r.is_empty() { ".".to_owned() } else { format!(" : {r}") }
}

/// "(Saved <date>) " of offline messages.
fn saved_prefix(offline: bool, timestamp: u32) -> String {
    if !offline {
        return String::new();
    }
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(timestamp as u64);
    aurora_i18n::tr!("world-groups-saved-at", value0 = crate::ui::chat::hhmm(t).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_changes_the_notices_and_sends_the_profile_listing_as_it_is() {
        let group = |n: u128, list_in_profile: bool| aurora_net::GroupMembership {
            id: Uuid::from_u128(n),
            name: format!("Group {n}"),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile,
            contribution: 0,
        };
        let mut chats = GroupChats {
            groups: vec![group(1, true), group(2, false)],
            ..Default::default()
        };
        chats.set_accept_notices(Uuid::from_u128(2), false);
        // not one of ours: nothing
        chats.set_accept_notices(Uuid::from_u128(9), false);
        assert!(chats.groups[0].accept_notices && !chats.groups[1].accept_notices);
        assert!(matches!(
            chats.take_commands()[..],
            [NetCommand::SetGroupFlags {
                group,
                accept_notices: false,
                list_in_profile: false,
            }] if group == Uuid::from_u128(2)
        ));
    }

    /// Online status › Ad hoc conferences (LLIMMgr::addMessage, FSIgnoreAdHocSessions):
    /// a stranger's conference is left unopened, a friend's opens when allowed,
    /// a Linden's and a group's always do.
    #[test]
    fn ignored_conferences_are_left_unless_a_friend_or_a_linden_started_them() {
        let (stranger, pal) = (Uuid::from_u128(7), Uuid::from_u128(8));
        let invite = |n: u128, from: Uuid, name: &str| aurora_net::SessionInvite {
            session: Uuid::from_u128(n),
            from,
            from_name: name.into(),
            message: "hi".into(),
            session_name: "Chat".into(),
            offline: false,
            timestamp: 0,
        };
        let left = |w: &mut World| -> Vec<Uuid> {
            let out = std::mem::take(&mut w.groups.out);
            out.into_iter()
                .filter_map(|c| match c {
                    NetCommand::SendImDialog { dialog, id, .. } if dialog == dialog::SESSION_LEAVE => Some(id),
                    _ => None,
                })
                .collect()
        };
        let system_lines = |w: &World| w.chat.iter().filter(|l| l.kind == ChatKind::System).count();
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.social.friends = vec![crate::world::social::Friend {
            id: pal,
            online: true,
            rights_given: 1,
            rights_has: 1,
        }];
        // off (the default): every conference opens
        w.on_session_invite(invite(1, stranger, "Tess Touch"));
        assert!(w.groups.sessions.contains_key(&Uuid::from_u128(1)));
        assert!(left(&mut w).is_empty());
        w.status.cfg.ignore_adhoc = true;
        w.on_session_invite(invite(2, stranger, "Tess Touch"));
        assert!(!w.groups.sessions.contains_key(&Uuid::from_u128(2)), "not opened");
        assert_eq!(left(&mut w), vec![Uuid::from_u128(2)]);
        assert_eq!(system_lines(&w), 0, "silent unless reported");
        // a friend is ignored too until "except my friends" is on
        w.on_session_invite(invite(3, pal, "Pal Friend"));
        assert_eq!(left(&mut w), vec![Uuid::from_u128(3)]);
        w.status.cfg.adhoc_from_friends = true;
        w.on_session_invite(invite(4, pal, "Pal Friend"));
        assert!(w.groups.sessions.contains_key(&Uuid::from_u128(4)));
        assert!(left(&mut w).is_empty());
        w.on_session_invite(invite(5, stranger, "Tess Touch"));
        assert_eq!(left(&mut w), vec![Uuid::from_u128(5)]);
        // the report names who invited us, in nearby chat
        w.status.cfg.report_ignored_adhoc = true;
        w.on_session_invite(invite(6, stranger, "Tess Touch"));
        assert_eq!(left(&mut w), vec![Uuid::from_u128(6)]);
        assert_eq!(system_lines(&w), 1);
        assert!(w.chat.back().is_some_and(|l| l.text.contains("Tess Touch")));
        // a Linden always gets through
        w.on_session_invite(invite(7, Uuid::from_u128(9), "Oz Linden"));
        assert!(w.groups.sessions.contains_key(&Uuid::from_u128(7)));
        assert!(left(&mut w).is_empty());
        // a group's chat is not a conference
        let group = Uuid::from_u128(10);
        w.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Builders".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        w.mutes.on_server(aurora_net::MuteListSource::File(String::new()), None);
        w.on_session_invite(invite(10, stranger, "Tess Touch"));
        assert!(w.groups.sessions.contains_key(&group));
        assert!(left(&mut w).is_empty());
    }

    /// LLIMMgr::checkSnoozeExpiration: over once its time has come; a second
    /// snooze moves the end; reopening forgets it.
    #[test]
    fn a_snooze_ends_at_its_time() {
        let (mut chats, group, t) = (GroupChats::default(), Uuid::from_u128(10), Instant::now());
        assert!(!chats.snooze_over(&group, t), "not snoozed");
        chats.snooze(group, t + Duration::from_secs(60));
        assert!(!chats.snooze_over(&group, t));
        assert!(chats.snooze_over(&group, t + Duration::from_secs(60)));
        chats.snooze(group, t + Duration::from_secs(120));
        assert!(!chats.snooze_over(&group, t + Duration::from_secs(60)));
        assert!(chats.unsnooze(&group));
        assert!(!chats.snooze_over(&group, t + Duration::from_secs(600)));
        assert!(!chats.unsnooze(&group));
    }

    /// FSFloaterIM::snooze and LLIMMgr::restoreSnoozedSession: the chat
    /// closes without leaving, its messages are dropped during the snooze,
    /// the first one after it opens the chat again and joins the session.
    #[test]
    fn a_snoozed_group_chat_drops_its_messages_until_the_snooze_is_over() {
        let (group, tess) = (Uuid::from_u128(10), Uuid::from_u128(7));
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Builders".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        w.mutes.on_server(aurora_net::MuteListSource::File(String::new()), None);
        w.on_session_invite(aurora_net::SessionInvite {
            session: group,
            from: tess,
            from_name: "Tess Touch".into(),
            message: "hi".into(),
            session_name: "Builders".into(),
            offline: false,
            timestamp: 0,
        });
        assert!(w.groups.sessions.contains_key(&group));
        let im = |text: &str| aurora_net::InstantMessage {
            from_agent_id: tess,
            from_name: "Tess Touch".into(),
            to_agent_id: Uuid::nil(),
            dialog: dialog::SESSION_SEND,
            session_id: group,
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        };
        let sent = |w: &mut World| -> Vec<u8> {
            std::mem::take(&mut w.groups.out)
                .into_iter()
                .filter_map(|c| match c {
                    NetCommand::SendImDialog { dialog, .. } => Some(dialog),
                    _ => None,
                })
                .collect()
        };
        sent(&mut w);
        // an hour: closed, not left
        assert!(w.snooze_group_chat(group, 3600));
        assert!(!w.groups.sessions.contains_key(&group));
        assert!(w.social.ims.iter().all(|s| s.other != group));
        assert!(sent(&mut w).is_empty(), "the session is not left");
        // during it, nothing opens
        assert!(w.on_session_im(&im("during")));
        assert!(!w.groups.sessions.contains_key(&group), "dropped");
        assert!(sent(&mut w).is_empty());
        // once it is over, the next message opens it and joins again
        w.groups.snooze(group, Instant::now());
        assert!(w.on_session_im(&im("after")));
        assert!(w.groups.sessions.contains_key(&group));
        assert_eq!(sent(&mut w), vec![dialog::SESSION_GROUP_START]);
        let lines = w.social.ims.iter().find(|s| s.other == group).map(|s| s.lines.len());
        assert_eq!(lines, Some(1), "only the message after the snooze");
        assert!(!w.groups.snooze_over(&group, Instant::now()), "forgotten once open");
        // a conference is not snoozed; closing a chat still leaves it
        assert!(!w.snooze_group_chat(Uuid::from_u128(99), 60));
        w.leave_session(group);
        assert_eq!(sent(&mut w), vec![dialog::SESSION_LEAVE]);
    }

    /// A snoozed group chat never flashes the Conversations button: its
    /// messages are dropped before they reach the app; the first one after
    /// the snooze flashes it like any group chat.
    #[test]
    fn a_snoozed_group_chat_does_not_flash_the_conversations_button() {
        let (group, tess) = (Uuid::from_u128(10), Uuid::from_u128(7));
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Builders".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        w.mutes.on_server(aurora_net::MuteListSource::File(String::new()), None);
        w.on_session_invite(aurora_net::SessionInvite {
            session: group,
            from: tess,
            from_name: "Tess Touch".into(),
            message: "hi".into(),
            session_name: "Builders".into(),
            offline: false,
            timestamp: 0,
        });
        let im = |text: &str| aurora_net::InstantMessage {
            from_agent_id: tess,
            from_name: "Tess Touch".into(),
            to_agent_id: Uuid::nil(),
            dialog: dialog::SESSION_SEND,
            session_id: group,
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        };
        w.im_messages.clear();
        assert!(w.snooze_group_chat(group, 3600));
        assert!(w.on_session_im(&im("during")));
        assert!(w.im_messages.is_empty(), "nothing reaches the app during the snooze");
        w.groups.snooze(group, Instant::now());
        assert!(w.on_session_im(&im("after")));
        let flashes: Vec<bool> = std::mem::take(&mut w.im_messages)
            .iter()
            .map(|m| w.message_flashes_conversations(m))
            .collect();
        assert_eq!(flashes, vec![true], "the message after the snooze flashes");
    }

    /// A failed "accept invitation" says nothing and keeps the chat, unless
    /// the server answered 404 (chatterBoxInvitationCoro).
    #[test]
    fn a_refused_invitation_is_silent_unless_the_session_is_gone() {
        let (group, tess) = (Uuid::from_u128(10), Uuid::from_u128(7));
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Builders".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        w.mutes.on_server(aurora_net::MuteListSource::File(String::new()), None);
        w.on_session_invite(aurora_net::SessionInvite {
            session: group,
            from: tess,
            from_name: "Tess Touch".into(),
            message: "hi".into(),
            session_name: "Builders".into(),
            offline: false,
            timestamp: 0,
        });
        let reply = |not_found| NetEvent::ChatSessionReply {
            method: SessionMethod::AcceptInvitation,
            session: group,
            result: None,
            not_found,
        };
        let lines = |w: &World| w.social.ims.iter().find(|s| s.other == group).map(|s| s.lines.len());
        assert_eq!(lines(&w), Some(1));
        assert!(w.on_group_event(reply(false)).is_none());
        assert_eq!(lines(&w), Some(1), "no error line");
        assert!(!w.groups.sessions[&group].closed);
        assert!(w.on_group_event(reply(true)).is_none());
        assert_eq!(lines(&w), Some(2), "the session no longer exists");
        assert!(w.groups.sessions[&group].closed);
    }

    #[test]
    fn splitting() {
        assert_eq!(split_im("hello"), vec!["hello"]);
        let long = "é".repeat(700);
        let parts = split_im(&long);
        assert!(parts.iter().all(|p| p.len() <= MAX_IM_BYTES));
        assert_eq!(parts.concat(), long);
        let words = "mot ".repeat(400);
        let parts = split_im(&words);
        assert!(parts[0].ends_with(' '));
        assert_eq!(parts.concat(), words);
    }
}
