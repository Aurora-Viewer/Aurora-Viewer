//! Voice calls outside the spatial channel: with a resident (P2P), a group
//! or an ad-hoc conference. Port of LLVoiceChannel / LLVoiceChannelGroup
//! (indra/newview/llvoicechannel.cpp), LLIMMgr::startCall / endCall,
//! inviteToSession and LLIncomingCallDialog::processCallResponse
//! (indra/newview/llimview.cpp), Copyright (C) Linden Research, Inc. and
//! The Phoenix Firestorm Project, originally LGPL 2.1.
//!
//! On WebRTC regions every call is a "multiagent" channel: P2P calls run as
//! ad-hoc calls (mP2PAsAdhocCall, since LLWebRTCVoiceClient has no outgoing
//! call interface). The credentials come from ChatSessionRequest ("call");
//! a P2P call first starts the voice session ("start p2p voice"). The voice
//! session itself (`crate::voice`) joins the channel while a call is up and
//! goes back to the spatial channel when it ends, as
//! LLVoiceChannel::deactivate activates the proximal channel.

use super::{ChatKind, ChatLine, Origin, World, notifications};
use aurora_llsd::Llsd;
use aurora_net::{CallMethod, NetCommand, SessionMethod, VoiceInvite};
use std::time::{Instant, SystemTime};
use uuid::Uuid;

/// Who is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// A resident (`other`); the conversation is keyed by them.
    P2p { other: Uuid },
    /// A group (session id = group id).
    Group,
    /// An ad-hoc conference.
    AdHoc,
}

/// LLVoiceChannel::EState, reduced to what the interface shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Waiting for the session to start or for the channel credentials
    /// (STATE_NO_CHANNEL_INFO → STATE_CALL_STARTED).
    Requesting,
    /// Joining the channel (STATE_RINGING); an outgoing P2P call rings
    /// until the other resident joins (notify_on_first_join).
    Ringing,
    /// In the call (STATE_CONNECTED).
    Connected,
}

/// The channel of a call (voice_credentials: channel_uri,
/// channel_credentials).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallChannel {
    pub id: String,
    pub credentials: String,
}

impl CallChannel {
    /// From a voice_credentials / voice channel info map; `None` without a
    /// channel ("Received invalid credentials").
    pub fn from_llsd(info: &Llsd) -> Option<CallChannel> {
        let id = info["channel_uri"].as_str();
        (!id.is_empty()).then(|| CallChannel {
            id: id.to_owned(),
            credentials: info["channel_credentials"].as_str().to_owned(),
        })
    }
}

/// A participant of the call, as the voice session reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CallMember {
    pub id: Uuid,
    pub level: f32,
    pub speaking: bool,
}

/// The call in progress (LLVoiceChannel::sCurrentVoiceChannel when it is
/// not the proximal one).
#[derive(Debug, Clone)]
pub struct Call {
    /// The IM session (P2P: agent XOR other, group: the group id).
    pub session: Uuid,
    pub kind: CallKind,
    /// The channel's name for the messages (VOICE_CHANNEL_NAME).
    pub name: String,
    pub incoming: bool,
    pub state: CallState,
    pub channel: Option<CallChannel>,
    /// The group session is being started: the credentials are asked once
    /// it is (autoStartCallOnStartup, mStartCallOnInitialize).
    wait_session: bool,
    /// ERROR_NOT_AVAILABLE retries left (DEFAULT_RETRIES_COUNT).
    retries: u8,
    /// Someone else was in the channel once (a P2P call hangs up when the
    /// last one leaves, hangup_on_last_leave).
    pub others_joined: bool,
    /// When the call was connected, for its duration.
    pub connected_at: Option<Instant>,
    /// The participants, ourselves included, refreshed by the voice session.
    pub members: Vec<CallMember>,
}

impl Call {
    /// The conversation it belongs to (the resident for a P2P call).
    pub fn conversation(&self) -> Uuid {
        match self.kind {
            CallKind::P2p { other } => other,
            _ => self.session,
        }
    }

    pub fn p2p(&self) -> bool {
        matches!(self.kind, CallKind::P2p { .. })
    }
}

/// What the voice session tells of the call's channel each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    Connecting,
    Connected,
    /// Full or locked (ERROR_CHANNEL_FULL / ERROR_CHANNEL_LOCKED).
    Full,
    /// Unsupported backend or error the session does not recover from.
    Failed,
}

/// DEFAULT_RETRIES_COUNT (llvoicechannel.cpp).
const RETRIES: u8 = 3;

/// The calls: the one in progress and the invitations waiting for an answer.
#[derive(Default)]
pub struct Calls {
    pub active: Option<Call>,
    /// Voice is enabled and working: invitations are shown only then
    /// (LLViewerChatterBoxInvitation::post).
    pub voice_enabled: bool,
    /// The region's VoiceServerType, sent with the requests.
    pub server_type: String,
    /// The microphone state wanted when a call connects or ends:
    /// Some(on) once, taken by the app (setUserPTTState, AutoDisengageMic).
    pub mic: Option<bool>,
}

/// A voice invitation waiting for its answer, in a notification.
fn invite_text(inv: &VoiceInvite, group: bool) -> String {
    let name = if inv.from_name.is_empty() { "?" } else { inv.from_name.as_str() };
    if inv.p2p {
        aurora_i18n::tr!("world-calls-invite-p2p", name = name)
    } else if group {
        aurora_i18n::tr!("world-calls-invite-group", name = name, group = inv.session_name.as_str())
    } else {
        aurora_i18n::tr!("world-calls-invite-adhoc", name = name)
    }
}

impl World {
    fn call_server_type(&self) -> String {
        if self.calls.server_type.is_empty() {
            aurora_voice::protocol::WEBRTC_VOICE_SERVER_TYPE.to_owned()
        } else {
            self.calls.server_type.clone()
        }
    }

    fn call_request(&mut self, method: CallMethod, session: Uuid) {
        let server_type = self.call_server_type();
        self.groups.out.push(NetCommand::Call {
            method,
            session,
            server_type,
        });
    }

    /// A line in the call's conversation (the notices Firestorm echoes to
    /// the IM window, LLVoiceChannelGroup::handleError).
    fn call_note(&mut self, conversation: Uuid, text: String) {
        self.conversation_push(
            conversation,
            ChatLine {
                time: SystemTime::now(),
                from: String::new(),
                text,
                kind: ChatKind::System,
                source: Uuid::nil(),
                origin: Origin::Live,
            },
            false,
        );
    }

    /// The call of a conversation, if it is the one in progress.
    pub fn call_of(&self, conversation: &Uuid) -> Option<&Call> {
        self.calls.active.as_ref().filter(|c| c.conversation() == *conversation)
    }

    /// Start a call with a resident or a group, or in an ad-hoc conference
    /// (LLAvatarActions::startCall, LLGroupActions::startCall, the call
    /// button of an IM window). The conversation opens with it.
    pub fn start_call(&mut self, target: Uuid) {
        if target.is_nil() || target == self.agent_id {
            return;
        }
        if self.call_of(&target).is_some() {
            return;
        }
        self.end_call();
        let (session, kind, name) = if self.groups.is_member(&target) {
            let name = self.groups.group(&target).map(|g| g.name.clone()).unwrap_or_default();
            (target, CallKind::Group, name)
        } else if self.is_group_session(&target) {
            (target, CallKind::AdHoc, self.session_title(&target))
        } else {
            let session = Uuid::from_u128(self.agent_id.as_u128() ^ target.as_u128());
            (session, CallKind::P2p { other: target }, self.social.name_of(&target))
        };
        log::info!("call: starting {kind:?} {session}");
        let mut wait_session = false;
        match kind {
            CallKind::Group => {
                // a group IM session first (IM_SESSION_GROUP_START), then the
                // credentials once it is started
                wait_session = !self.groups.sessions.get(&target).is_some_and(|s| s.active && !s.closed);
                self.start_group_chat(target);
                if !wait_session {
                    self.call_request(CallMethod::Call, session);
                }
            }
            CallKind::AdHoc => {
                self.social.focus_im = Some(target);
                self.call_request(CallMethod::Call, session);
            }
            CallKind::P2p { other } => {
                self.social.session_mut(other);
                self.social.focus_im = Some(other);
                self.call_request(CallMethod::StartP2p { other }, session);
            }
        }
        self.ui_sounds.push(crate::ui_sound::UiSound::StartIm);
        self.calls.active = Some(Call {
            session,
            kind,
            name,
            incoming: false,
            state: CallState::Requesting,
            channel: None,
            wait_session,
            retries: RETRIES,
            others_joined: false,
            connected_at: None,
            members: Vec::new(),
        });
    }

    /// Hang up (LLIMMgr::endCall → LLVoiceChannel::deactivate): back to the
    /// spatial channel, the microphone off (AutoDisengageMic).
    pub fn end_call(&mut self) {
        if let Some(c) = self.calls.active.take() {
            log::info!("call: ended {}", c.session);
            if c.state != CallState::Requesting || c.channel.is_some() {
                self.calls.mic = Some(false);
            }
        }
    }

    /// A conversation is closed: its call ends with it.
    pub fn conversation_closed(&mut self, conversation: &Uuid) {
        if self.call_of(conversation).is_some() {
            self.end_call();
        }
    }

    /// Ends the call because of `text`, echoed to its conversation.
    fn call_failed(&mut self, text: String) {
        if let Some(c) = &self.calls.active {
            let conversation = c.conversation();
            self.call_note(conversation, text);
        }
        self.end_call();
    }

    /// Voice events: an invitation, the answer of a request, a new channel.
    pub(super) fn on_call_event(&mut self, ev: aurora_net::NetEvent) {
        use aurora_net::NetEvent;
        match ev {
            NetEvent::VoiceInvite(inv) => self.on_voice_invite(inv),
            NetEvent::CallReply { method, session, result } => self.on_call_reply(method, session, result),
            NetEvent::SessionVoiceChannel { session, info } => {
                // LLViewerChatterBoxSessionUpdate: a call in progress moves
                if let Some(c) = self.calls.active.as_mut().filter(|c| c.session == session && c.channel.is_some())
                    && let Some(ch) = CallChannel::from_llsd(&info)
                {
                    log::info!("call: new voice channel for {session}");
                    c.channel = Some(ch);
                    c.state = CallState::Ringing;
                }
            }
            _ => {}
        }
    }

    /// The group session we waited for is started: ask the credentials.
    pub(super) fn call_session_started(&mut self, session: Uuid, success: bool) {
        let Some(c) = self.calls.active.as_mut().filter(|c| c.session == session && c.wait_session) else {
            return;
        };
        c.wait_session = false;
        if success {
            self.call_request(CallMethod::Call, session);
        } else {
            let name = c.name.clone();
            self.call_failed(aurora_i18n::tr!("world-calls-generic-error", name = name.as_str()));
        }
    }

    /// The server closed the session: the call goes with it.
    pub(super) fn call_session_closed(&mut self, session: Uuid) {
        if self.calls.active.as_ref().is_some_and(|c| c.session == session) {
            self.end_call();
        }
    }

    fn on_call_reply(&mut self, method: CallMethod, session: Uuid, result: Result<Llsd, u16>) {
        let Some(c) = self.calls.active.as_mut().filter(|c| c.session == session) else {
            return;
        };
        let name = c.name.clone();
        match (method, result) {
            // startP2PVoiceCoro: the session exists, the credentials next
            (CallMethod::StartP2p { .. }, Ok(_)) => self.call_request(CallMethod::Call, session),
            (CallMethod::StartP2p { .. }, Err(status)) => {
                log::warn!("call: start p2p voice failed ({status})");
                self.call_failed(aurora_i18n::tr!("world-calls-generic-error", name = name.as_str()));
            }
            // voiceCallCapCoro → setChannelInfo(result["voice_credentials"])
            (CallMethod::Call, Ok(body)) => match CallChannel::from_llsd(&body["voice_credentials"]) {
                Some(ch) => {
                    c.channel = Some(ch);
                    c.state = CallState::Ringing;
                }
                None => {
                    log::warn!("call: invalid credentials for {session}");
                    self.call_failed(aurora_i18n::tr!("world-calls-generic-error", name = name.as_str()));
                }
            },
            // 403: "VoiceNotAllowed", other failures "VoiceCallGenericError"
            (CallMethod::Call, Err(403)) => self.call_failed(aurora_i18n::tr!("world-calls-not-allowed", name = name.as_str())),
            (CallMethod::Call, Err(_)) => self.call_failed(aurora_i18n::tr!("world-calls-generic-error", name = name.as_str())),
            (CallMethod::DeclineP2p, _) => {}
        }
    }

    /// What the voice session reports of the call's channel this frame.
    pub fn call_voice_update(&mut self, status: ChannelStatus, members: Vec<CallMember>) {
        let me = self.agent_id;
        let Some(c) = self.calls.active.as_mut() else {
            return;
        };
        if c.channel.is_none() {
            return;
        }
        let others = members.iter().any(|m| m.id != me);
        c.members = members;
        let name = c.name.clone();
        match status {
            ChannelStatus::Connecting => {}
            ChannelStatus::Connected => {
                // an outgoing P2P call rings until the other joins
                let joined = !c.p2p() || c.incoming || others;
                if joined && c.state != CallState::Connected {
                    c.state = CallState::Connected;
                    c.connected_at = Some(Instant::now());
                    c.retries = RETRIES;
                    // mic off when joining a group / ad-hoc call, on for
                    // P2P (LLVoiceChannelGroup::handleStatusChange)
                    self.calls.mic = Some(c.p2p());
                }
                if others {
                    c.others_joined = true;
                } else if c.p2p() && c.others_joined {
                    // the other hung up (STATUS_LEFT_CHANNEL)
                    self.call_failed(aurora_i18n::tr!("world-calls-ended-by-other", name = name.as_str()));
                }
            }
            ChannelStatus::Full => self.call_failed(aurora_i18n::tr!("world-calls-channel-full", name = name.as_str())),
            ChannelStatus::Failed => {
                // ERROR_NOT_AVAILABLE: ask the credentials again, three times
                if c.retries > 0 && !c.p2p() {
                    c.retries -= 1;
                    c.channel = None;
                    c.state = CallState::Requesting;
                    let session = c.session;
                    self.call_request(CallMethod::Call, session);
                } else if c.p2p() {
                    self.call_failed(aurora_i18n::tr!("world-calls-no-answer", name = name.as_str()))
                } else {
                    self.call_failed(aurora_i18n::tr!("world-calls-join-failed", name = name.as_str()))
                }
            }
        }
    }

    /// LLIMMgr::inviteToSession for a voice invitation.
    fn on_voice_invite(&mut self, inv: VoiceInvite) {
        if inv.from == self.agent_id || inv.session.is_nil() {
            return;
        }
        // Don't display voice invites unless the user has voice enabled
        if !self.calls.voice_enabled {
            log::info!("call: invitation from {} ignored, voice is off", inv.from);
            return;
        }
        let group = !inv.p2p && self.groups.is_member(&inv.session);
        // a call we started with them: just accept (callStarted)
        if self.calls.active.as_ref().is_some_and(|c| c.session == inv.session) {
            self.accept_voice_invite(inv);
            return;
        }
        // muted residents and do not disturb decline silently
        let blocked = self.is_avatar_blocked(&inv.from);
        if blocked || self.status.dnd {
            log::info!(
                "call: invitation from {} declined ({})",
                inv.from,
                if blocked { "blocked" } else { "do not disturb" }
            );
            if self.status.dnd && !blocked && inv.p2p {
                self.social.session_mut(inv.from);
                self.call_note(inv.from, aurora_i18n::tr!("world-calls-auto-rejected").to_owned());
            }
            self.decline_voice_invite(&inv);
            return;
        }
        // one card per session
        if self
            .notifications
            .list
            .iter()
            .any(|n| matches!(&n.data, notifications::Data::Call { invite } if invite.session == inv.session))
        {
            return;
        }
        let text = invite_text(&inv, group);
        let title = if inv.from_name.is_empty() {
            self.social.name_of(&inv.from)
        } else {
            inv.from_name.clone()
        };
        self.notifications.push(
            notifications::Kind::Call,
            title,
            text,
            notifications::Data::Call { invite: Box::new(inv) },
        );
        self.ui_sounds.push(crate::ui_sound::UiSound::StartIm);
    }

    /// Accept (processCallResponse 0): the conversation opens and the call
    /// joins the channel the invitation gave.
    pub(super) fn accept_voice_invite(&mut self, inv: VoiceInvite) {
        let channel = CallChannel::from_llsd(&inv.voice);
        let (kind, conversation, name) = if inv.p2p {
            self.social.session_mut(inv.from);
            (CallKind::P2p { other: inv.from }, inv.from, inv.from_name.clone())
        } else {
            // accept the session (chatterBoxInvitationCoro)
            self.groups.out.push(NetCommand::ChatSession {
                method: SessionMethod::AcceptInvitation,
                session: inv.session,
            });
            let group = self.groups.is_member(&inv.session);
            let name = if inv.session_name.is_empty() {
                inv.from_name.clone()
            } else {
                inv.session_name.clone()
            };
            if group {
                self.start_group_chat(inv.session);
            }
            (if group { CallKind::Group } else { CallKind::AdHoc }, inv.session, name)
        };
        self.social.focus_im = Some(conversation);
        if self.calls.active.as_ref().is_some_and(|c| c.session != inv.session) {
            self.end_call();
        }
        let session = inv.session;
        let call = self.calls.active.get_or_insert_with(|| Call {
            session,
            kind,
            name,
            incoming: true,
            state: CallState::Requesting,
            channel: None,
            wait_session: false,
            retries: RETRIES,
            others_joined: false,
            connected_at: None,
            members: Vec::new(),
        });
        call.incoming = true;
        call.wait_session = false;
        match channel {
            Some(ch) => {
                call.channel = Some(ch);
                call.state = CallState::Ringing;
            }
            // no channel in the invitation: ask it (STATE_NO_CHANNEL_INFO)
            None => self.call_request(CallMethod::Call, session),
        }
    }

    /// Decline (processCallResponse 1): "decline p2p voice" for a resident,
    /// "decline invitation" for a group or a conference.
    pub(super) fn decline_voice_invite(&mut self, inv: &VoiceInvite) {
        if inv.p2p {
            self.call_request(CallMethod::DeclineP2p, inv.session);
        } else {
            self.groups.out.push(NetCommand::ChatSession {
                method: SessionMethod::DeclineInvitation,
                session: inv.session,
            });
        }
    }

    /// "Start IM" of a P2P invitation (processCallResponse 2): the
    /// conversation without the call, which is declined.
    pub(super) fn answer_voice_invite_by_im(&mut self, inv: &VoiceInvite) {
        self.social.session_mut(inv.from);
        self.social.focus_im = Some(inv.from);
        self.decline_voice_invite(inv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;
    use aurora_net::NetEvent;

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(1);
        w.calls.voice_enabled = true;
        w
    }

    fn calls_sent(w: &mut World) -> Vec<(CallMethod, Uuid)> {
        std::mem::take(&mut w.groups.out)
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::Call { method, session, .. } => Some((method, session)),
                _ => None,
            })
            .collect()
    }

    fn credentials() -> Llsd {
        llsd_map! { "voice_credentials" => llsd_map! { "channel_uri" => "chan", "channel_credentials" => "secret" } }
    }

    #[test]
    fn a_p2p_call_starts_the_voice_session_then_asks_the_credentials() {
        let mut w = world();
        let other = Uuid::from_u128(2);
        w.start_call(other);
        let session = Uuid::from_u128(1 ^ 2);
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::StartP2p { other }, session)]);
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::StartP2p { other },
            session,
            result: Ok(Llsd::Undef),
        });
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::Call, session)]);
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::Call,
            session,
            result: Ok(credentials()),
        });
        let c = w.calls.active.as_ref().expect("call");
        assert_eq!(c.channel.as_ref().map(|c| c.id.as_str()), Some("chan"));
        assert_eq!(c.state, CallState::Ringing);
        assert_eq!(c.conversation(), other);
    }

    #[test]
    fn an_outgoing_p2p_call_rings_until_the_other_joins_and_ends_when_they_leave() {
        let mut w = world();
        let other = Uuid::from_u128(2);
        w.start_call(other);
        let session = Uuid::from_u128(1 ^ 2);
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::Call,
            session,
            result: Ok(credentials()),
        });
        let me = CallMember {
            id: w.agent_id,
            level: 0.0,
            speaking: false,
        };
        let them = CallMember {
            id: other,
            level: 0.3,
            speaking: true,
        };
        w.call_voice_update(ChannelStatus::Connected, vec![me]);
        assert_eq!(w.calls.active.as_ref().map(|c| c.state), Some(CallState::Ringing));
        w.call_voice_update(ChannelStatus::Connected, vec![me, them]);
        assert_eq!(w.calls.active.as_ref().map(|c| c.state), Some(CallState::Connected));
        assert_eq!(w.calls.mic.take(), Some(true), "the microphone opens for a P2P call");
        w.call_voice_update(ChannelStatus::Connected, vec![me]);
        assert!(w.calls.active.is_none(), "the other hung up");
        assert_eq!(w.calls.mic.take(), Some(false), "AutoDisengageMic");
    }

    #[test]
    fn refused_credentials_end_the_call_with_firestorms_notice() {
        let mut w = world();
        let other = Uuid::from_u128(2);
        w.start_call(other);
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::Call,
            session: Uuid::from_u128(1 ^ 2),
            result: Err(403),
        });
        assert!(w.calls.active.is_none());
        let name = w.social.name_of(&other);
        let last = w
            .social
            .ims
            .iter()
            .find(|s| s.other == other)
            .and_then(|s| s.lines.back())
            .expect("notice");
        assert_eq!(last.text, aurora_i18n::tr!("world-calls-not-allowed", name = name.as_str()));
    }

    #[test]
    fn voice_invitations_need_voice_and_respect_blocks_and_do_not_disturb() {
        let from = Uuid::from_u128(7);
        let inv = VoiceInvite {
            session: Uuid::from_u128(1 ^ 7),
            session_name: String::new(),
            from,
            from_name: "Nova".into(),
            p2p: true,
            voice: llsd_map! { "channel_uri" => "chan", "channel_credentials" => "c" },
        };
        let mut w = world();
        w.calls.voice_enabled = false;
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        assert!(w.notifications.list.is_empty());
        let mut w = world();
        w.status.dnd = true;
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        assert!(w.notifications.list.is_empty());
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::DeclineP2p, inv.session)]);
        let mut w = world();
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        assert_eq!(w.notifications.list.len(), 1, "one card per session");
        w.accept_voice_invite(inv);
        let c = w.calls.active.as_ref().expect("accepted");
        assert!(c.incoming);
        assert_eq!(c.state, CallState::Ringing);
        assert_eq!(c.conversation(), from);
    }

    #[test]
    fn closing_the_conversation_hangs_up() {
        let mut w = world();
        let other = Uuid::from_u128(2);
        w.start_call(other);
        w.conversation_closed(&Uuid::from_u128(3));
        assert!(w.calls.active.is_some());
        w.conversation_closed(&other);
        assert!(w.calls.active.is_none());
    }
}
