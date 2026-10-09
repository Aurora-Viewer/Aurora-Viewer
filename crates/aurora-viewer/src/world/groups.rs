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
//! - closing sends IM_SESSION_LEAVE; a blocked group chat is left at once.
//!
//! The lines live in `social.ims` like 1:1 conversations (keyed by session
//! id); this module keeps what is specific to multi-agent sessions.

use super::social::ImSession;
use super::{ChatKind, ChatLine, MAX_CHAT, World};
use aurora_net::{GroupMembership, NetCommand, NetEvent, SessionAgent, SessionMethod};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
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
    pub moderators: HashSet<Uuid>,
    /// Our text muted by a moderator.
    pub text_muted: bool,
    /// The server confirmed the session (start reply / accepted invitation).
    pub active: bool,
    /// The server closed it (ForceCloseChatterBoxSession).
    pub closed: bool,
    history_fetched: bool,
}

#[derive(Default)]
pub struct GroupChats {
    pub groups: Vec<GroupMembership>,
    pub sessions: HashMap<Uuid, SessionInfo>,
    /// Group chats that arrived before the mute list (restored once it is
    /// loaded, exoGroupMuteList::addDeferredGroupChat).
    deferred: Vec<Uuid>,
    /// FSMuteAllGroups / FSMuteGroupWhenNoticesDisabled (from the settings).
    pub mute_all_groups: bool,
    pub mute_when_notices_off: bool,
    /// FSReportBlockToNearbyChat: say in nearby chat when the list changes.
    pub report_blocks: bool,
    /// Group list asked (AgentDataUpdateRequest).
    requested: bool,
    /// Our active group (AgentDataUpdate; nil = none) and its title.
    pub active: Uuid,
    pub active_title: String,
    /// Pinned groups, listed first (FSFavoriteGroups).
    pub favorites: HashSet<Uuid>,
    favorites_path: Option<PathBuf>,
    pub out: Vec<NetCommand>,
}

impl GroupChats {
    pub fn group(&self, id: &Uuid) -> Option<&GroupMembership> {
        self.groups.iter().find(|g| g.id == *id)
    }

    pub fn is_member(&self, id: &Uuid) -> bool {
        self.group(id).is_some()
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
    /// A multi-agent session (group or conference).
    pub fn is_group_session(&self, session: &Uuid) -> bool {
        self.groups.sessions.contains_key(session)
    }

    /// Title of a conversation: group / conference name, else the avatar.
    pub fn session_title(&self, session: &Uuid) -> String {
        match self.groups.sessions.get(session) {
            Some(s) if !s.name.is_empty() => s.name.clone(),
            Some(_) => "Conférence".to_owned(),
            None => self.social.name_of(session),
        }
    }

    /// The conversation lines of a session (created when missing, without a
    /// name lookup since its id is not an avatar).
    fn session_lines(&mut self, session: Uuid) -> &mut ImSession {
        if let Some(i) = self.social.ims.iter().position(|s| s.other == session) {
            return &mut self.social.ims[i];
        }
        self.social.ims.push(ImSession {
            other: session,
            lines: Vec::new(),
            unread: 0,
            input: String::new(),
        });
        self.social.ims.last_mut().unwrap()
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
        let s = self.session_lines(session);
        s.lines.push(l);
        if unread {
            s.unread += 1;
        }
        if s.lines.len() > MAX_CHAT {
            s.lines.remove(0);
        }
    }

    fn session_system(&mut self, session: Uuid, text: impl Into<String>) {
        self.session_push(
            session,
            line(String::new(), text.into(), ChatKind::System, Uuid::nil(), SystemTime::now()),
            false,
        );
    }

    fn info_mut(&mut self, session: Uuid, name: &str) -> &mut SessionInfo {
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
        if self.mutes.group_chat_muted(&group) {
            // opening it means receiving it again (exoGroupMuteList::remove)
            self.set_group_chat_blocked(group, false);
        }
        self.social.focus_im = Some(group);
        let known = self.groups.sessions.get(&group).is_some_and(|s| s.active && !s.closed);
        self.info_mut(group, &g.name);
        self.session_lines(group);
        if !known {
            // LLIMModel::sendStartSession(IM_SESSION_GROUP_START)
            self.groups.im(group, dialog::SESSION_GROUP_START, group, String::new());
        }
    }

    /// Send text to a group / conference session (IM_SESSION_SEND).
    pub fn send_session_im(&mut self, session: Uuid, text: &str) {
        let me = self.own_name();
        for part in split_im(text) {
            self.groups.im(session, dialog::SESSION_SEND, session, part.to_owned());
        }
        let l = line(me, text.to_owned(), ChatKind::Own, Uuid::nil(), SystemTime::now());
        self.session_push(session, l, false);
        if let Some(s) = self.groups.sessions.get(&session)
            && s.closed
        {
            self.session_system(
                session,
                "Cette session a été fermée par le serveur : le message n'a peut-être pas été reçu.",
            );
        }
    }

    /// Close a group / conference session (LLIMModel::sendLeaveSession).
    pub fn leave_session(&mut self, session: Uuid) {
        if self.groups.sessions.remove(&session).is_some() {
            self.groups.im(session, dialog::SESSION_LEAVE, session, String::new());
        }
        self.social.ims.retain(|s| s.other != session);
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
            self.info_mut(group, &g.name);
            self.session_lines(group);
            self.groups.im(group, dialog::SESSION_GROUP_START, group, String::new());
            self.im_messages.push(crate::ui_sound::ImMessage {
                kind: crate::ui_sound::ImKind::Group,
                session: group,
                new_session: true,
            });
        }
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
                    self.session_system(id, "Vous ne faites plus partie de ce groupe.");
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
                    self.session_system(id, format!("Impossible de rejoindre la session{}", reason(&error)));
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
                let me = self.agent_id;
                if let Some(a) = agents.iter().find(|a| a.agent == me)
                    && let (Some(muted), Some(s)) = (a.text_muted, self.groups.sessions.get_mut(&session))
                {
                    let changed = s.text_muted != muted;
                    s.text_muted = muted;
                    if changed {
                        let text = if muted {
                            "Un modérateur a coupé votre chat dans cette session."
                        } else {
                            "Un modérateur vous a rendu la parole."
                        };
                        self.session_system(session, text);
                    }
                }
            }
            NetEvent::SessionError { session, event, error } => {
                // LLIMMgr::showSessionEventError
                let what = if event == "message" {
                    "Message non envoyé"
                } else {
                    "Action refusée"
                };
                self.session_system(session, format!("{what}{}", reason(&error)));
            }
            NetEvent::SessionClosed { session, reason: r } => {
                // LLIMMgr::showSessionForceClose
                if let Some(s) = self.groups.sessions.get_mut(&session) {
                    s.closed = true;
                    s.active = false;
                    self.session_system(session, format!("La session a été fermée{}", reason(&r)));
                }
            }
            NetEvent::ChatSessionReply { method, session, result } => match method {
                SessionMethod::AcceptInvitation => match result {
                    Some(r) => {
                        let agents = aurora_net::social::parse_session_agents(&r);
                        if let Some(s) = self.groups.sessions.get_mut(&session) {
                            s.active = true;
                            apply_agents(s, &agents);
                        }
                    }
                    None => self.session_system(
                        session,
                        "Impossible de rejoindre la session (le serveur n'a pas accepté l'invitation).",
                    ),
                },
                SessionMethod::FetchHistory => {
                    if let Some(r) = result {
                        self.add_history(session, aurora_net::social::parse_history(&r));
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
    fn on_session_invite(&mut self, inv: aurora_net::SessionInvite) {
        let session = inv.session;
        if inv.from == self.agent_id || session.is_nil() {
            return;
        }
        let in_group = self.groups.is_member(&session);
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
            // Communiquer > Statut de connexion > Conférences ad hoc (FS:PP)
            let c = &self.status.cfg;
            let friend = self.social.friends.iter().any(|f| f.id == inv.from);
            if !in_group && c.ignore_adhoc && !(c.adhoc_from_friends && friend) && !crate::world::mutes::is_linden(&inv.from_name) {
                let report = c.report_ignored_adhoc;
                log::info!("ignoring conference (ad-hoc) chat from {}", inv.from_name);
                self.groups.im(inv.from, dialog::SESSION_LEAVE, session, String::new());
                if report {
                    self.system_message(format!(
                        "Vous avez été invité à une conférence (ad hoc) par {}, mais elle a été ignorée automatiquement à cause de vos réglages.",
                        inv.from_name
                    ));
                }
                return;
            }
        }
        if !self.mutes.text_muted(&inv.from, &inv.from_name) {
            let new = !self.groups.sessions.contains_key(&session);
            self.info_mut(session, &inv.session_name);
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
        if !self.groups.sessions.contains_key(&session) {
            // only shown once a session is open (after its invitation)
            if self.groups.deferred.contains(&session) && self.mutes.is_loaded() {
                self.restore_deferred_group_chats();
            }
            if !self.groups.sessions.contains_key(&session) {
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
        self.session_sound(session, false);
        self.session_push(session, l, true);
        true
    }

    /// Recent messages from the chat server, before what we already have
    /// (LLIMSession::addMessagesFromServerHistory).
    fn add_history(&mut self, session: Uuid, history: Vec<aurora_net::HistoryLine>) {
        if history.is_empty() {
            return;
        }
        let me = self.agent_id;
        let own = self.own_name();
        let known: HashSet<(Uuid, String)> = self
            .social
            .ims
            .iter()
            .find(|s| s.other == session)
            .map(|s| s.lines.iter().map(|l| (l.source, l.text.clone())).collect())
            .unwrap_or_default();
        let mut older: Vec<ChatLine> = history
            .into_iter()
            .filter(|h| !self.mutes.text_muted(&h.from_id, &h.from))
            .filter(|h| !known.contains(&(if h.from_id == me { Uuid::nil() } else { h.from_id }, h.message.clone())))
            .map(|h| {
                let time = SystemTime::UNIX_EPOCH + Duration::from_secs(h.time as u64);
                if h.from_id == me {
                    line(own.clone(), h.message, ChatKind::Own, Uuid::nil(), time)
                } else {
                    line(h.from, h.message, ChatKind::Im, h.from_id, time)
                }
            })
            .collect();
        if older.is_empty() {
            return;
        }
        let s = self.session_lines(session);
        older.append(&mut s.lines);
        s.lines = older;
        let excess = s.lines.len().saturating_sub(MAX_CHAT);
        s.lines.drain(..excess);
    }
}

fn apply_agents(s: &mut SessionInfo, agents: &[SessionAgent]) {
    for a in agents {
        match a.present {
            Some(true) => {
                s.participants.insert(a.agent);
            }
            Some(false) => {
                s.participants.remove(&a.agent);
            }
            None => {}
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
    format!("(Enregistré à {}) ", crate::ui::chat::hhmm(t))
}

#[cfg(test)]
mod tests {
    use super::*;

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
