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
use std::time::{Duration, Instant, SystemTime};
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
    /// Their voice muted by a moderator (the participant list's muted
    /// speaker icon).
    pub muted: bool,
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

/// IM_SESSION_CONFERENCE_START (llinstantmessage.h).
const IM_SESSION_CONFERENCE_START: u8 = 16;

/// What adding residents to a conversation does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    /// Invited into the conference.
    Invited,
    /// A conversation with one resident: a new conference, once confirmed
    /// (ConfirmAddingChatParticipants).
    Confirm(Vec<Uuid>),
    /// A group's conversation, or nobody new.
    NotAllowed,
}

/// DEFAULT_RETRIES_COUNT (llvoicechannel.cpp).
const RETRIES: u8 = 3;

/// How long an invitation waits for its answer before it is declined: the
/// "lifetime" of floater_incoming_call.xml (LLIncomingCallDialog::
/// onLifetimeExpired; a WebRTC call has no incoming call interface to keep
/// it alive).
pub const INVITE_LIFETIME: Duration = Duration::from_secs(30);

/// How long a group's call started by someone stays offered to join once
/// its invitation is gone: the session does not say when it ends.
pub const LAUNCHED_KEPT: Duration = Duration::from_secs(2 * 3600);

/// A voice invitation waiting for its answer: its card in the notifications
/// and the strip of its conversation answer it alike.
#[derive(Debug, Clone)]
pub struct PendingInvite {
    pub invite: VoiceInvite,
    /// Its notification card.
    pub notification: u64,
    pub since: Instant,
}

impl PendingInvite {
    /// The conversation it shows in (the caller for a resident's call).
    pub fn conversation(&self) -> Uuid {
        if self.invite.p2p { self.invite.from } else { self.invite.session }
    }
}

/// The answers of an invitation (processCallResponse 0, 1 and 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteAnswer {
    Accept,
    Decline,
    /// "Start IM": the conversation without the call (residents only).
    Text,
}

/// The calls: the one in progress and the invitations waiting for an answer.
#[derive(Default)]
pub struct Calls {
    pub active: Option<Call>,
    pub invites: Vec<PendingInvite>,
    /// Conferences we are starting, by temporary session id: the residents
    /// asked, for the start by message after a refusal.
    pub conferences: std::collections::HashMap<Uuid, Vec<Uuid>>,
    /// Residents we invited into a conference and who have not joined yet
    /// (FSFloaterIM::mInvitedParticipants), by session.
    pub invited: std::collections::HashMap<Uuid, Vec<Uuid>>,
    /// Voice is enabled and working: invitations are shown only then
    /// (LLViewerChatterBoxInvitation::post).
    pub voice_enabled: bool,
    /// Calls of a group or a conference someone started, by conversation:
    /// who invited us and when. After their invitation is gone (ignored,
    /// expired) the conversation offers to join them, as Firestorm's IM
    /// window keeps its call button for the session's voice channel.
    pub launched: std::collections::HashMap<Uuid, (Uuid, Instant)>,
    /// The region's VoiceServerType, sent with the requests.
    pub server_type: String,
    /// VoiceCallsRejectGroup / AdHoc / P2P, given by the app.
    pub reject_group: bool,
    pub reject_adhoc: bool,
    pub reject_p2p: bool,
    /// The microphone state wanted when a call connects or ends:
    /// Some(on) once, taken by the app (setUserPTTState, AutoDisengageMic).
    pub mic: Option<bool>,
    /// Volume of each resident heard in voice (0..1, 0.5 = as sent), per
    /// account like LLSpeakerVolumeStorage; residents left at 0.5 are not
    /// kept.
    pub volumes: std::collections::BTreeMap<Uuid, f32>,
    volumes_path: Option<std::path::PathBuf>,
    /// The residents of our voice channel this frame (nearby voice or the
    /// call's), set by the app from the voice session: who gets the volume
    /// row at the top of their menus.
    pub voice_peers: std::collections::HashSet<Uuid>,
    /// The moderator mute of each participant of the nearby channel, set
    /// by the app from the voice session; None off that channel.
    pub nearby_moderated: Option<std::collections::HashMap<Uuid, bool>>,
    /// Ours, as last seen on the nearby channel
    /// (LLNearbyVoiceModeration::mChannelMuteMap).
    pub own_moderated: Option<bool>,
}

/// The file of the voice volumes, in the account's folder
/// (LLSpeakerVolumeStorage keeps volume_settings.xml there).
pub const VOLUMES_FILE: &str = "voice_volumes.xml";

impl Calls {
    /// The account's voice volumes (LLSD map: resident → volume).
    pub fn load_volumes(&mut self, path: std::path::PathBuf) {
        let saved = std::fs::read(&path).ok().and_then(|b| aurora_llsd::from_xml(&b).ok());
        self.volumes = saved
            .as_ref()
            .and_then(|v| v.as_map())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((Uuid::parse_str(k).ok()?, v.as_f64() as f32)))
                    .filter(|(_, v)| v.is_finite())
                    .map(|(k, v)| (k, v.clamp(0.0, 1.0)))
                    .collect()
            })
            .unwrap_or_default();
        self.volumes_path = Some(path);
    }

    /// A resident's volume, 0.5 (heard as sent) when none is kept.
    pub fn volume(&self, id: &Uuid) -> f32 {
        self.volumes.get(id).copied().unwrap_or(0.5)
    }

    /// The resident is in our voice channel (LLVoiceClient::getVoiceEnabled
    /// = isParticipant): one of the voice session's participants, or a
    /// member of the connected call.
    pub fn hears(&self, id: &Uuid) -> bool {
        self.voice_peers.contains(id)
            || self
                .active
                .as_ref()
                .is_some_and(|c| c.state == CallState::Connected && c.members.iter().any(|m| m.id == *id))
    }

    /// A resident's volume; back at 0.5 it is forgotten.
    pub fn set_volume(&mut self, id: Uuid, volume: f32) {
        if (volume - 0.5).abs() < 0.005 || !volume.is_finite() {
            self.volumes.remove(&id);
        } else {
            self.volumes.insert(id, volume.clamp(0.0, 1.0));
        }
    }

    pub fn save_volumes(&self) {
        let Some(path) = &self.volumes_path else {
            return;
        };
        let mut map = aurora_llsd::Map::new();
        for (id, v) in &self.volumes {
            map.insert(id.to_string(), Llsd::from(*v as f64));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&Llsd::Map(map))) {
            log::warn!("voice volumes not saved: {e}");
        }
    }

    /// The invitation waiting in a conversation.
    pub fn invite_for(&self, conversation: &Uuid) -> Option<&PendingInvite> {
        self.invites.iter().find(|p| p.conversation() == *conversation)
    }
}

/// A Second Life staff member's name ("… Linden", LLMuteList::isLinden):
/// never filtered by "friends only".
fn is_linden(name: &str) -> bool {
    name.trim()
        .rsplit(' ')
        .next()
        .is_some_and(|last| last.eq_ignore_ascii_case("linden"))
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
        self.conversation_notice(conversation, text, super::notice_topics::Topic::Call);
    }

    /// "Only friends and groups can call or IM me" (VoiceCallsFriendsOnly):
    /// what a resident who is not a friend sends is dropped, Lindens apart
    /// (llimprocessing.cpp, LLIMMgr::addMessage).
    pub(super) fn friends_only_drops(&self, from: &Uuid, name: &str) -> bool {
        self.status.cfg.friends_only && *from != self.agent_id && !self.is_friend(from) && !is_linden(name)
    }

    /// The call of a conversation, if it is the one in progress.
    pub fn call_of(&self, conversation: &Uuid) -> Option<&Call> {
        self.calls.active.as_ref().filter(|c| c.conversation() == *conversation)
    }

    /// A call started by someone in a group or conference conversation,
    /// which we are not in and no longer asked to join: who started it
    /// and when (forgotten after `LAUNCHED_KEPT`).
    pub fn call_launched(&self, conversation: &Uuid) -> Option<(Uuid, Instant)> {
        if self.call_of(conversation).is_some() || self.calls.invite_for(conversation).is_some() {
            return None;
        }
        self.calls
            .launched
            .get(conversation)
            .copied()
            .filter(|(_, since)| since.elapsed() < LAUNCHED_KEPT)
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
        if !self.rlva_start_ims(&[target]) {
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

    /// A voice conference with several residents (LLAvatarActions::
    /// startAdhocCall): a conference session under a temporary id
    /// ("start conference"), whose call starts once the server answered
    /// with the session's real id (autoStartCallOnStartup).
    pub fn start_conference(&mut self, residents: Vec<Uuid>) {
        let agents = self.conference_agents(residents);
        if !self.rlva_start_ims(&agents) {
            return;
        }
        match agents.len() {
            0 => return,
            1 => return self.start_call(agents[0]),
            _ => {}
        }
        self.end_call();
        let session = self.open_conference(agents, true);
        let name = aurora_i18n::tr!("world-groups-conference").to_owned();
        self.calls.active = Some(Call {
            session,
            kind: CallKind::AdHoc,
            name,
            incoming: false,
            state: CallState::Requesting,
            channel: None,
            wait_session: true,
            retries: RETRIES,
            others_joined: false,
            connected_at: None,
            members: Vec::new(),
        });
    }

    /// A text conference with several residents (LLAvatarActions::
    /// startConference, the "IM" of a multiple selection); one resident is
    /// an ordinary conversation (startIM).
    pub fn start_text_conference(&mut self, residents: Vec<Uuid>) {
        let agents = self.conference_agents(residents);
        if !self.rlva_start_ims(&agents) {
            return;
        }
        match agents.len() {
            0 => {}
            1 => {
                self.social.session_mut(agents[0]);
                self.social.focus_im = Some(agents[0]);
                self.ui_sounds.push(crate::ui_sound::UiSound::StartIm);
            }
            _ => {
                self.open_conference(agents, false);
            }
        }
    }

    /// The residents of a new conference: ourselves, nil ids and repeats
    /// left out, in their order.
    fn conference_agents(&self, residents: Vec<Uuid>) -> Vec<Uuid> {
        let me = self.agent_id;
        let mut agents: Vec<Uuid> = Vec::new();
        for id in residents {
            if !id.is_nil() && id != me && !agents.contains(&id) {
                agents.push(id);
            }
        }
        agents
    }

    /// LLIMMgr::addSession(IM_SESSION_CONFERENCE_START) → sendStartSession:
    /// the conversation under a temporary id, "start conference" asked; the
    /// deprecated start by message follows a refusal (`on_conference_reply`).
    fn open_conference(&mut self, agents: Vec<Uuid>, voice: bool) -> Uuid {
        let session = Uuid::new_v4();
        let name = aurora_i18n::tr!("world-groups-conference").to_owned();
        log::info!(
            "conference: starting with {} residents{}",
            agents.len(),
            if voice { ", with a call" } else { "" }
        );
        self.info_mut(session, &name).started_by_us = true;
        self.session_lines(session);
        self.social.focus_im = Some(session);
        self.calls.conferences.insert(session, agents.clone());
        self.call_request(CallMethod::StartConference { agents }, session);
        self.ui_sounds.push(crate::ui_sound::UiSound::StartIm);
        session
    }

    /// startConferenceCoro: a 400 falls back on the old way, the
    /// conference started by message (start_deprecated_conference_chat:
    /// IM_SESSION_CONFERENCE_START to the first resident, every invited id
    /// in the binary bucket); another failure ends there.
    fn on_conference_reply(&mut self, session: Uuid, result: &Result<Llsd, u16>) {
        let Some(agents) = self.calls.conferences.get(&session).cloned() else {
            return;
        };
        match result {
            Ok(_) => {}
            Err(400) => {
                log::info!("conference: start refused (400), starting it by message");
                let bucket: Vec<u8> = agents.iter().flat_map(|a| *a.as_bytes()).collect();
                self.groups.out.push(NetCommand::SendImDialog {
                    to: agents[0],
                    dialog: IM_SESSION_CONFERENCE_START,
                    id: session,
                    message: String::new(),
                    bucket,
                });
            }
            Err(status) => {
                log::warn!("conference: start failed ({status})");
                self.calls.conferences.remove(&session);
                if self.calls.active.as_ref().is_some_and(|c| c.session == session) {
                    let name = aurora_i18n::tr!("world-groups-conference");
                    self.call_failed(aurora_i18n::tr!("world-calls-generic-error", name = name));
                }
            }
        }
    }

    /// Add residents to a conversation (FSFloaterIM::addSessionParticipants):
    /// a conference invites them ("invite"); a conversation with one
    /// resident asks first, then becomes a new conference
    /// (`convert_to_conference`); a group's cannot (isInviteAllowed).
    pub fn invite_to_conversation(&mut self, conversation: Uuid, residents: Vec<Uuid>) -> AddOutcome {
        let agents = self.conference_agents(residents);
        if agents.is_empty() || self.groups.is_member(&conversation) {
            return AddOutcome::NotAllowed;
        }
        if self.is_group_session(&conversation) {
            let present = self
                .groups
                .sessions
                .get(&conversation)
                .map(|s| s.participants.clone())
                .unwrap_or_default();
            let agents: Vec<Uuid> = agents.into_iter().filter(|a| !present.contains(a)).collect();
            if !agents.is_empty() {
                log::info!("conference: inviting {} residents", agents.len());
                // remembered, to tell the others when they join
                self.calls.invited.entry(conversation).or_default().extend(agents.iter().copied());
                self.call_request(CallMethod::Invite { agents }, conversation);
            }
            return AddOutcome::Invited;
        }
        // a P2P conversation: not the other participant again
        // (FSFloaterIM::canAddSelectedToChat)
        if agents.contains(&conversation) {
            return AddOutcome::NotAllowed;
        }
        AddOutcome::Confirm(agents)
    }

    /// The confirmed addition to a conversation with one resident
    /// (FSFloaterIM::addP2PSessionParticipants): that conversation closes
    /// (its call with it) and a conference starts with the resident and the
    /// new ones, a voice conference if a call was in progress.
    pub fn convert_to_conference(&mut self, other: Uuid, added: Vec<Uuid>) {
        // A rejected conversion must leave the existing conversation and
        // its call intact (LLAvatarActions::startConference/startAdhocCall).
        if !self.rlva_start_ims(&added) || !self.rlva_start_ims(&[other]) {
            return;
        }
        let voice = self.call_of(&other).is_some();
        self.conversation_closed(&other);
        let mut residents = vec![other];
        residents.extend(added.iter().copied());
        if voice {
            self.start_conference(residents);
        } else {
            self.start_text_conference(residents);
        }
        // the new ones are the invited of the new conference (mInitialInvitedIDs)
        if let Some(session) = self.social.focus_im.filter(|s| self.calls.conferences.contains_key(s)) {
            self.calls.invited.insert(session, added);
        }
    }

    /// Residents joined a conference (ChatterBoxSessionAgentListUpdates,
    /// "ENTER" transitions): those we invited are announced to everyone in
    /// it by a message of ours, "<names> was / were invited to the
    /// conversation." (FSFloaterIM::processAgentListUpdates,
    /// sendParticipantsAddedNotification), and forgotten.
    pub(super) fn conference_joined(&mut self, session: Uuid, agents: &[aurora_net::SessionAgent]) {
        let Some(invited) = self.calls.invited.get_mut(&session) else {
            return;
        };
        let joined: Vec<Uuid> = agents.iter().filter(|a| a.entered).map(|a| a.agent).collect();
        let arrived: Vec<Uuid> = invited.iter().copied().filter(|id| joined.contains(id)).collect();
        invited.retain(|id| !joined.contains(id));
        if invited.is_empty() {
            self.calls.invited.remove(&session);
        }
        if arrived.is_empty() {
            return;
        }
        // LLAvatarActions::buildResidentsString: the complete names known,
        // in the order of their usernames
        let names = &self.social.avatar_names;
        let mut known: Vec<(String, String)> = arrived
            .iter()
            .filter_map(|id| {
                let n = names.get(id)?;
                Some((n.username.clone(), names.complete(id)?))
            })
            .collect();
        if known.is_empty() {
            return;
        }
        known.sort();
        let list = known.into_iter().map(|(_, name)| name).collect::<Vec<_>>().join(", ");
        let text = if arrived.len() > 1 {
            aurora_i18n::tr!("world-calls-participants-added", names = list.as_str())
        } else {
            aurora_i18n::tr!("world-calls-participant-added", name = list.as_str())
        };
        self.send_session_im(session, &text);
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
        self.calls.launched.remove(conversation);
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

    /// The session we waited for is started: ask the credentials. A
    /// conference's temporary id becomes the server's
    /// (processSessionInitializedReply).
    pub(super) fn call_session_started(&mut self, temp: Uuid, session: Uuid, success: bool) {
        // a conference we started: its conversation follows it to its id
        if self.calls.conferences.remove(&temp).is_some() && success && !session.is_nil() && session != temp {
            self.social.focus_im = Some(session);
            if let Some(invited) = self.calls.invited.remove(&temp) {
                self.calls.invited.insert(session, invited);
            }
        }
        let Some(c) = self
            .calls
            .active
            .as_mut()
            .filter(|c| (c.session == session || (!temp.is_nil() && c.session == temp)) && c.wait_session)
        else {
            return;
        };
        c.wait_session = false;
        let moved = success && !session.is_nil() && c.session != session;
        if moved {
            c.session = session;
        }
        let (session, name) = (c.session, c.name.clone());
        if moved {
            // the conversation shown follows the conference to its new id
            self.social.focus_im = Some(session);
        }
        if success {
            self.call_request(CallMethod::Call, session);
        } else {
            self.call_failed(aurora_i18n::tr!("world-calls-generic-error", name = name.as_str()));
        }
    }

    /// The server closed the session: the call goes with it.
    pub(super) fn call_session_closed(&mut self, session: Uuid) {
        if self.calls.active.as_ref().is_some_and(|c| c.session == session) {
            self.end_call();
        }
        self.invite_withdrawn(session);
    }

    fn on_call_reply(&mut self, method: CallMethod, session: Uuid, result: Result<Llsd, u16>) {
        // a conference's start, with or without a call: its own fallback
        if matches!(method, CallMethod::StartConference { .. }) {
            self.on_conference_reply(session, &result);
            return;
        }
        let Some(c) = self.calls.active.as_mut().filter(|c| c.session == session) else {
            return;
        };
        let name = c.name.clone();
        match (method, result) {
            (CallMethod::StartConference { .. } | CallMethod::Invite { .. }, _) => {}
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
        // muted residents are declined without a word
        if self.is_avatar_blocked(&inv.from) {
            log::info!("call: invitation from {} declined (blocked)", inv.from);
            self.decline_voice_invite(&inv);
            return;
        }
        // VoiceCallsRejectGroup / AdHoc / P2P, VoiceCallsFriendsOnly and do
        // not disturb decline silently; do not disturb alone tells it in the
        // conversation and answers the caller (send_do_not_disturb_message)
        let c = &self.calls;
        let rejected = (group && c.reject_group)
            || (!group && !inv.p2p && c.reject_adhoc)
            || (inv.p2p && c.reject_p2p)
            || (self.status.cfg.friends_only && !self.is_friend(&inv.from) && !is_linden(&inv.from_name));
        if rejected || self.status.dnd {
            log::info!(
                "call: invitation from {} declined ({})",
                inv.from,
                if rejected { "settings" } else { "do not disturb" }
            );
            if !rejected {
                let conversation = if inv.p2p { inv.from } else { inv.session };
                if inv.p2p {
                    self.social.session_mut(inv.from);
                }
                self.call_note(conversation, aurora_i18n::tr!("world-calls-auto-rejected").to_owned());
                let text = self.status.cfg.dnd_text.clone();
                self.auto_im(inv.from, inv.session, &text);
            }
            self.decline_voice_invite(&inv);
            return;
        }
        // one card per session
        if self.calls.invites.iter().any(|p| p.invite.session == inv.session) {
            return;
        }
        let text = invite_text(&inv, group);
        let title = if inv.from_name.is_empty() {
            self.social.name_of(&inv.from)
        } else {
            inv.from_name.clone()
        };
        // the conversation shows the call too (created as Firestorm's
        // invitation does, brought up and flashing like an unread message)
        let conversation = if inv.p2p {
            self.social.session_mut(inv.from);
            inv.from
        } else {
            let name = if inv.session_name.is_empty() {
                aurora_i18n::tr!("world-groups-conference").to_owned()
            } else {
                inv.session_name.clone()
            };
            self.info_mut(inv.session, &name);
            self.session_lines(inv.session);
            inv.session
        };
        self.conversation_push(
            conversation,
            ChatLine {
                time: SystemTime::now(),
                from: String::new(),
                text: text.clone(),
                kind: ChatKind::System,
                source: Uuid::nil(),
                origin: Origin::Live,
            },
            true,
        );
        let notification = self.notifications.push(
            notifications::Kind::Call,
            title.clone(),
            text,
            notifications::Data::Call {
                invite: Box::new(inv.clone()),
            },
        );
        self.notifications
            .link_name(notification, title, notifications::NameTarget::Avatar(inv.from));
        if group {
            self.notifications.link_name(
                notification,
                inv.session_name.clone(),
                notifications::NameTarget::Group(inv.session),
            );
        }
        if !inv.p2p {
            self.calls.launched.insert(inv.session, (inv.from, Instant::now()));
        }
        self.calls.invites.push(PendingInvite {
            invite: inv,
            notification,
            since: Instant::now(),
        });
        // LLIncomingCallDialog::onOpen
        self.ui_sounds.push(crate::ui_sound::UiSound::IncomingVoiceCall);
    }

    /// Answer the invitation of a session from its conversation's strip:
    /// its card leaves the notifications with the same answer.
    pub fn answer_call_invite(&mut self, session: Uuid, answer: InviteAnswer) {
        let Some(i) = self.calls.invites.iter().position(|p| p.invite.session == session) else {
            return;
        };
        let p = self.calls.invites.remove(i);
        let response = match answer {
            InviteAnswer::Accept => notifications::Response::Accept,
            InviteAnswer::Decline => notifications::Response::Decline,
            InviteAnswer::Text => notifications::Response::Button(0),
        };
        self.notifications.respond(p.notification, response, |_| Uuid::nil(), Uuid::nil());
        self.apply_invite_answer(p.invite, answer);
    }

    /// The card of an invitation was answered: its strip goes too.
    pub(super) fn invite_card_answered(&mut self, invite: VoiceInvite, answer: InviteAnswer) {
        self.calls.invites.retain(|p| p.invite.session != invite.session);
        self.apply_invite_answer(invite, answer);
    }

    fn apply_invite_answer(&mut self, invite: VoiceInvite, answer: InviteAnswer) {
        match answer {
            InviteAnswer::Accept => self.accept_voice_invite(invite),
            InviteAnswer::Text if invite.p2p => self.answer_voice_invite_by_im(&invite),
            _ => self.decline_voice_invite(&invite),
        }
    }

    /// Invitations left unanswered for INVITE_LIFETIME are declined, their
    /// card and strip closed (LLIncomingCallDialog::onLifetimeExpired).
    pub fn expire_call_invites(&mut self) {
        let expired: Vec<Uuid> = self
            .calls
            .invites
            .iter()
            .filter(|p| p.since.elapsed() >= INVITE_LIFETIME)
            .map(|p| p.invite.session)
            .collect();
        for session in expired {
            log::info!("call: invitation for {session} expired");
            self.answer_call_invite(session, InviteAnswer::Decline);
        }
    }

    /// The server closed the session of an invitation: the caller gave up.
    fn invite_withdrawn(&mut self, session: Uuid) {
        let (gone, kept) = std::mem::take(&mut self.calls.invites)
            .into_iter()
            .partition(|p| p.invite.session == session);
        self.calls.invites = kept;
        self.calls.launched.remove(&session);
        for p in gone {
            self.notifications.list.retain(|n| n.id != p.notification);
        }
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
            muted: false,
        };
        let them = CallMember {
            id: other,
            level: 0.3,
            speaking: true,
            muted: false,
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

    fn p2p_invite(from: u128) -> VoiceInvite {
        VoiceInvite {
            session: Uuid::from_u128(1 ^ from),
            session_name: String::new(),
            from: Uuid::from_u128(from),
            from_name: "Tess Touch".into(),
            p2p: true,
            voice: llsd_map! { "channel_uri" => "chan", "channel_credentials" => "c" },
        }
    }

    #[test]
    fn the_strip_and_the_card_answer_the_same_invitation() {
        let mut w = world();
        let inv = p2p_invite(7);
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        // the conversation exists, unread, with the strip's invitation
        let s = w.social.ims.iter().find(|s| s.other == inv.from).expect("conversation");
        assert_eq!(s.unread, 1);
        assert!(w.calls.invite_for(&inv.from).is_some());
        assert_eq!(w.notifications.list.len(), 1);
        // answered from the strip: the card goes
        w.answer_call_invite(inv.session, InviteAnswer::Decline);
        assert!(w.notifications.list.is_empty());
        assert!(w.calls.invites.is_empty());
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::DeclineP2p, inv.session)]);
        // answered from the card: the strip goes
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        let id = w.notifications.list[0].id;
        w.respond_notification(id, notifications::Response::Accept);
        assert!(w.calls.invites.is_empty());
        assert!(w.calls.active.as_ref().is_some_and(|c| c.incoming));
    }

    #[test]
    fn an_unanswered_or_withdrawn_invitation_closes_card_and_strip() {
        let mut w = world();
        let inv = p2p_invite(7);
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        w.calls.invites[0].since = Instant::now().checked_sub(INVITE_LIFETIME).expect("past");
        w.expire_call_invites();
        assert!(w.calls.invites.is_empty() && w.notifications.list.is_empty());
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::DeclineP2p, inv.session)]);
        let group = VoiceInvite {
            p2p: false,
            session: Uuid::from_u128(40),
            ..p2p_invite(8)
        };
        w.on_call_event(NetEvent::VoiceInvite(group.clone()));
        assert_eq!(w.calls.invites.len(), 1);
        w.call_session_closed(group.session);
        assert!(w.calls.invites.is_empty() && w.notifications.list.is_empty());
    }

    #[test]
    fn reject_settings_and_friends_only_decline_silently() {
        let inv = p2p_invite(7);
        for setup in [(|w: &mut World| w.calls.reject_p2p = true) as fn(&mut World), |w: &mut World| {
            w.status.cfg.friends_only = true
        }] {
            let mut w = world();
            setup(&mut w);
            w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
            assert!(w.notifications.list.is_empty());
            assert!(!w.social.ims.iter().any(|s| s.other == inv.from), "no notice");
            assert_eq!(calls_sent(&mut w), vec![(CallMethod::DeclineP2p, inv.session)]);
        }
        // the group setting leaves a resident's call alone
        let mut w = world();
        w.calls.reject_group = true;
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        assert_eq!(w.calls.invites.len(), 1);
        // do not disturb answers the caller with its text
        let mut w = world();
        w.status.dnd = true;
        w.on_call_event(NetEvent::VoiceInvite(inv.clone()));
        assert!(w.groups.out.iter().any(|c| matches!(c,
            NetCommand::SendImDialog { to, dialog, .. } if *to == inv.from && *dialog == super::super::status::AUTO_RESPONSE)));
        // friends only: a Linden still gets through, a friend too
        let mut w = world();
        w.status.cfg.friends_only = true;
        assert!(w.friends_only_drops(&inv.from, "Tess Touch"));
        assert!(!w.friends_only_drops(&inv.from, "Oz Linden"));
    }

    #[test]
    fn voice_volumes_are_kept_per_account_file() {
        let dir = std::env::temp_dir().join(format!("aurora-volumes-{}", Uuid::new_v4()));
        let path = dir.join(VOLUMES_FILE);
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(3));
        let mut calls = Calls::default();
        calls.load_volumes(path.clone());
        assert!(calls.volumes.is_empty());
        calls.set_volume(a, 0.8);
        calls.set_volume(b, 0.3);
        calls.set_volume(b, 0.5);
        calls.save_volumes();
        let mut again = Calls::default();
        again.load_volumes(path);
        assert_eq!(again.volumes.len(), 1);
        assert!((again.volumes[&a] - 0.8).abs() < 1e-6);
        assert!((again.volume(&a) - 0.8).abs() < 1e-6);
        assert_eq!(again.volume(&b), 0.5, "a forgotten resident is heard as sent");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn residents_are_heard_in_nearby_voice_or_once_the_call_connects() {
        let mut w = world();
        let (other, nearby) = (Uuid::from_u128(2), Uuid::from_u128(7));
        assert!(!w.calls.hears(&other) && !w.calls.hears(&nearby));
        w.calls.voice_peers.insert(nearby);
        assert!(w.calls.hears(&nearby));
        w.start_call(other);
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::Call,
            session: Uuid::from_u128(1 ^ 2),
            result: Ok(credentials()),
        });
        let member = |id| CallMember {
            id,
            level: 0.0,
            speaking: false,
            muted: false,
        };
        let me = member(w.agent_id);
        // still ringing: they are not in the channel yet
        w.call_voice_update(ChannelStatus::Connected, vec![me]);
        assert!(!w.calls.hears(&other));
        w.call_voice_update(ChannelStatus::Connected, vec![me, member(other)]);
        assert!(w.calls.hears(&other));
    }

    #[test]
    fn a_refused_conference_start_falls_back_on_the_message() {
        let mut w = world();
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(3));
        w.start_text_conference(vec![a, b]);
        assert!(w.calls.active.is_none(), "a text conference has no call");
        let temp = *w.calls.conferences.keys().next().expect("conference");
        w.groups.out.clear();
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::StartConference { agents: vec![a, b] },
            session: temp,
            result: Err(400),
        });
        let sent = std::mem::take(&mut w.groups.out);
        let Some(NetCommand::SendImDialog {
            to, dialog, id, bucket, ..
        }) = sent.first()
        else {
            panic!("the conference started by message: {sent:?}");
        };
        assert_eq!((*to, *dialog, *id), (a, IM_SESSION_CONFERENCE_START, temp));
        let mut expected = a.as_bytes().to_vec();
        expected.extend_from_slice(b.as_bytes());
        assert_eq!(*bucket, expected);
        // another failure gives up
        let mut w = world();
        w.start_conference(vec![a, b]);
        let temp = w.calls.active.as_ref().expect("call").session;
        w.on_call_event(NetEvent::CallReply {
            method: CallMethod::StartConference { agents: vec![a, b] },
            session: temp,
            result: Err(500),
        });
        assert!(w.calls.active.is_none());
    }

    #[test]
    fn adding_people_invites_into_a_conference_and_converts_a_p2p_call() {
        let (a, b, c) = (Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        // a conference: "invite" for who is not in it yet
        let mut w = world();
        let session = Uuid::from_u128(50);
        w.info_mut(session, "Conference").participants.insert(b);
        assert_eq!(w.invite_to_conversation(session, vec![b, c]), AddOutcome::Invited);
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::Invite { agents: vec![c] }, session)]);
        // a group's conversation cannot
        w.groups.groups.push(aurora_net::GroupMembership {
            id: Uuid::from_u128(60),
            name: "G".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        assert_eq!(w.invite_to_conversation(Uuid::from_u128(60), vec![c]), AddOutcome::NotAllowed);
        // a call with one resident: confirmed, it becomes a voice conference
        let mut w = world();
        w.start_call(a);
        assert_eq!(w.invite_to_conversation(a, vec![a]), AddOutcome::NotAllowed);
        assert_eq!(w.invite_to_conversation(a, vec![b]), AddOutcome::Confirm(vec![b]));
        calls_sent(&mut w);
        w.convert_to_conference(a, vec![b]);
        let call = w.calls.active.as_ref().expect("conference call");
        assert!(matches!(call.kind, CallKind::AdHoc));
        let session = call.session;
        assert_eq!(
            calls_sent(&mut w),
            vec![(CallMethod::StartConference { agents: vec![a, b] }, session)]
        );
    }

    #[test]
    fn the_invited_who_join_are_announced_to_the_conference() {
        let mut w = world();
        let (a, b, c) = (Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        w.social.avatar_names.on_legacy(a, "Tess", "Touch");
        w.social.avatar_names.on_legacy(b, "Nova", "Example");
        let session = Uuid::from_u128(50);
        w.info_mut(session, "Conference").participants.insert(c);
        w.invite_to_conversation(session, vec![a, b]);
        w.groups.out.clear();
        let agent = |id: Uuid, entered: bool| aurora_net::SessionAgent {
            agent: id,
            present: Some(true),
            entered,
            ..Default::default()
        };
        let said = |w: &mut World| -> Vec<String> {
            std::mem::take(&mut w.groups.out)
                .into_iter()
                .filter_map(|c| match c {
                    NetCommand::SendImDialog { dialog: 17, message, .. } => Some(message),
                    _ => None,
                })
                .collect()
        };
        // a full list is no arrival; someone not invited is no news
        w.apply(NetEvent::SessionAgents {
            session,
            agents: vec![agent(a, false), agent(c, true)],
        });
        assert!(said(&mut w).is_empty());
        // both arrive: one message, names in the order of their usernames
        w.apply(NetEvent::SessionAgents {
            session,
            agents: vec![agent(a, true), agent(b, true)],
        });
        let names = format!(
            "{}, {}",
            w.social.avatar_names.complete(&b).expect("name"),
            w.social.avatar_names.complete(&a).expect("name")
        );
        assert_eq!(
            said(&mut w),
            vec![aurora_i18n::tr!("world-calls-participants-added", names = names.as_str())]
        );
        // told once
        w.apply(NetEvent::SessionAgents {
            session,
            agents: vec![agent(a, true)],
        });
        assert!(said(&mut w).is_empty());
        assert!(!w.calls.invited.contains_key(&session));
    }

    #[test]
    fn a_conference_starts_its_call_under_the_servers_session_id() {
        let mut w = world();
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(3));
        w.start_conference(vec![a, b, a]);
        let temp = w.calls.active.as_ref().expect("call").session;
        let sent = calls_sent(&mut w);
        assert_eq!(sent, vec![(CallMethod::StartConference { agents: vec![a, b] }, temp)]);
        let real = Uuid::from_u128(99);
        w.apply(NetEvent::SessionStarted {
            temp_session: temp,
            session: real,
            success: true,
            error: String::new(),
            agents: Vec::new(),
        });
        assert_eq!(w.calls.active.as_ref().map(|c| c.session), Some(real));
        assert_eq!(calls_sent(&mut w), vec![(CallMethod::Call, real)]);
        assert!(w.groups.sessions.contains_key(&real));
        // one resident only: a plain call
        let mut w = world();
        w.start_conference(vec![a]);
        assert!(w.calls.active.as_ref().is_some_and(|c| c.p2p()));
    }
}
