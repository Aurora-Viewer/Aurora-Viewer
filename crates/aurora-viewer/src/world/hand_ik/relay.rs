//! A worn bridge forwards original script identity and bounded regional events.
use super::*;
use std::collections::HashSet;
pub(super) mod declarations;
use declarations::Notice;

pub const AVAILABLE: &str = "auroraIKAvailable|";
pub const STATE: &str = "auroraIKState/1 ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relay {
    pub root: Uuid,
    pub region: RegionHandle,
    pub session: Uuid,
}
impl Relay {
    pub fn command(&self, command: &str) -> NetCommand {
        NetCommand::ScriptDialogReply {
            sender: None,
            object_id: self.root,
            channel: super::super::bridge::ik::CONTROL,
            index: 0,
            label: format!("auroraIKRelay|{}|{command}", self.session),
        }
    }
    pub fn route(&self, command: NetCommand) -> NetCommand {
        if let NetCommand::ScriptDialogReply {
            channel: CHANNEL, label, ..
        } = &command
            && let Some(fields) = label.strip_prefix("auroraIKObserve|")
        {
            self.command(&format!("observe|{fields}"))
        } else if let NetCommand::ScriptDialogReply {
            channel: CHANNEL, label, ..
        } = &command
            && let Some(fields) = label.strip_prefix("auroraIKEdit|")
        {
            self.command(&format!("edit|{fields}"))
        } else {
            command
        }
    }
}

impl Contacts {
    pub(crate) fn set_relay(&mut self, relay: Relay) {
        if self.observed_region != Some(relay.region) {
            let required = self.relay_required;
            *self = Self::default();
            self.relay_required = required;
        }
        self.observed_region = Some(relay.region);
        let retire_request = |contacts: &mut Contacts| {
            for session in std::iter::once(&mut contacts.primary).chain(contacts.worn.values_mut()) {
                session.received_at = None;
                session.sent_at = None;
                session.probes = 0;
                session.nonce = Uuid::nil();
                session.advertised_bundle = None;
                session.graph.cancel_transfer();
            }
        };
        retire_request(self);
        self.observers.values_mut().for_each(retire_request);
        self.relay = Some(relay);
    }
}

impl World {
    pub(crate) fn receive_ik_relay(&mut self, chat: &ChatMessage, now: Instant) {
        let diagnostics = &mut self.hand_contacts.relay_diagnostics;
        diagnostics.envelopes = diagnostics.envelopes.saturating_add(1);
        diagnostics.max_bytes = diagnostics.max_bytes.max(chat.message.len());
        diagnostics.last = "unauthorized_relay";
        let Some(relay) = self.hand_contacts.relay.as_ref() else { return };
        if chat.source_id != relay.root
            || chat.owner_id != self.agent_id
            || self.teleporting
            || chat.message.len() > 1024
            || Some(relay.region) != self.main_region
            || !sources::eligible_for(
                self,
                self.agent_id,
                Seat {
                    root: relay.root,
                    region: relay.region,
                },
            )
        {
            self.hand_contacts.relay_diagnostics.failure = self.hand_contacts.relay_diagnostics.last;
            return;
        }
        self.hand_contacts.relay_diagnostics.last = "malformed_or_truncated_envelope";
        let Some(body) = chat
            .message
            .strip_prefix("<auroraIKRelay>")
            .and_then(|s| s.strip_suffix("</auroraIKRelay>"))
        else {
            self.hand_contacts.relay_diagnostics.failure = self.hand_contacts.relay_diagnostics.last;
            return;
        };
        let mut fields = body.splitn(4, '|');
        self.hand_contacts.relay_diagnostics.last = "relay_session_or_identity";
        if fields.next().and_then(|v| v.parse::<Uuid>().ok()) != Some(relay.session) {
            self.hand_contacts.relay_diagnostics.failure = self.hand_contacts.relay_diagnostics.last;
            return;
        }
        let Some(source_id) = fields.next().and_then(|v| v.parse::<Uuid>().ok()) else {
            self.hand_contacts.relay_diagnostics.failure = self.hand_contacts.relay_diagnostics.last;
            return;
        };
        let Some(owner_id) = fields.next().and_then(|v| v.parse::<Uuid>().ok()) else {
            self.hand_contacts.relay_diagnostics.failure = self.hand_contacts.relay_diagnostics.last;
            return;
        };
        let Some(message) = fields.next() else { return };
        let diagnostics = &mut self.hand_contacts.relay_diagnostics;
        diagnostics.forwarded = diagnostics.forwarded.saturating_add(1);
        diagnostics.last = if message.starts_with(AVAILABLE) {
            "availability"
        } else if message.starts_with(STATE) {
            "state"
        } else {
            "definition"
        };
        let source = Seat {
            root: source_id,
            region: relay.region,
        };
        let original = ChatMessage {
            source_id,
            owner_id,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::Other(9),
            message: message.into(),
            ..chat.clone()
        };
        if !self.receive_ik_notice(source, &original, now) {
            self.receive_hand_contacts(&original, now);
        }
    }

    pub(super) fn receive_ik_notice(&mut self, source: Seat, chat: &ChatMessage, now: Instant) -> bool {
        let Some(body) = chat.message.strip_prefix(AVAILABLE) else {
            return false;
        };
        if chat.message.len() > 880
            || chat.owner_id.is_nil()
            || self.hand_contacts.relay.is_none()
            || !self.hand_contacts.notice_ingress.allow(now, 64.0, 128.0)
        {
            return true;
        }
        let Some((bundle, users)) = body.split_once('|') else { return true };
        let Ok(bundle) = bundle.parse::<Uuid>() else { return true };
        let Ok(users) = serde_json::from_str::<Vec<Uuid>>(users) else {
            return true;
        };
        if bundle.is_nil()
            || users.len() > 16
            || users.iter().any(Uuid::is_nil)
            || users.iter().collect::<HashSet<_>>().len() != users.len()
        {
            return true;
        }
        let notice = Notice {
            source,
            owner: chat.owner_id,
            bundle,
            users,
        };
        self.hand_contacts.declarations.remember(notice.clone());
        self.apply_ik_notice(&notice, now);
        true
    }

    fn apply_ik_notice(&mut self, notice: &Notice, now: Instant) {
        let source = notice.source;
        let Some(root) = self.objects.index_of_uuid(&source.root).and_then(|i| self.objects.get(i)) else {
            return;
        };
        if root.is_avatar()
            || root.key.region != source.region
            || Some(source.region) != self.main_region
            || (!root.owner_id.is_nil() && root.owner_id != notice.owner)
        {
            return;
        }
        if notice.users.is_empty() {
            let drop_source = |contacts: &mut Contacts| {
                if contacts.primary.seat == Some(source) {
                    contacts.primary.track(None, now);
                }
                contacts.worn.remove(&source.root);
                contacts.merge();
            };
            drop_source(&mut self.hand_contacts);
            self.hand_contacts.observers.values_mut().for_each(drop_source);
            return;
        }
        // Identity comes from the trusted relay's actual listen sender, not a
        // claimed JSON root. Unused objects cannot create cached definitions.
        for subject in notice.users.iter().copied() {
            if !sources::eligible_for(self, subject, source) || (avatar_seat(self, subject) != Some(source) && notice.owner != subject) {
                continue;
            }
            let seated = avatar_seat(self, subject) == Some(source);
            let known_sources = self.hand_contacts.sources().len()
                + self
                    .hand_contacts
                    .observers
                    .values()
                    .map(|contacts| contacts.sources().len())
                    .sum::<usize>();
            let known = self
                .hand_contacts
                .for_avatar(self.agent_id, subject)
                .is_some_and(|contacts| contacts.sessions().any(|session| session.seat == Some(source)));
            if known_sources >= 256 && !known {
                continue;
            }
            let contacts = if subject == self.agent_id {
                &mut self.hand_contacts
            } else {
                if self.hand_contacts.observers.len() >= 128 && !self.hand_contacts.observers.contains_key(&subject) {
                    continue;
                }
                self.hand_contacts.observers.entry(subject).or_default()
            };
            contacts.subject = Some(subject);
            if seated {
                contacts.primary.track(Some(source), now);
                contacts.primary.subject = Some(subject);
            } else {
                let session = contacts.worn.entry(source.root).or_default();
                session.track(Some(source), now);
                session.subject = Some(subject);
            }
            if let Some(session) = contacts.session_mut(source) {
                session.push = true;
                if session.advertised_bundle != Some(notice.bundle)
                    && (session.graph.bundle() != Some(notice.bundle) || session.received_at.is_none())
                {
                    session.advertised_bundle = Some(notice.bundle);
                    session.received_at = None;
                    session.sent_at = None;
                    session.probes = 0;
                    session.graph.cancel_transfer();
                    if !contacts.hints.contains(&source.root) {
                        contacts.hints.push(source.root);
                    }
                }
            }
        }
    }

    pub(super) fn receive_ik_state(&mut self, source: Seat, chat: &ChatMessage, now: Instant) -> bool {
        let Some(body) = chat.message.strip_prefix(STATE) else {
            return false;
        };
        if body.len() > 880 || self.hand_contacts.relay.is_none() || !self.hand_contacts.admits(source, now) {
            return true;
        }
        let Ok(mut packet) = serde_json::from_str::<serde_json::Value>(body) else {
            return true;
        };
        if !packet.is_object()
            || ["data", "part", "count", "avatar", "nonce"]
                .iter()
                .any(|key| packet.get(key).is_some())
        {
            return true;
        }
        let subjects: Vec<_> = std::iter::once((self.agent_id, &self.hand_contacts))
            .chain(self.hand_contacts.observers.iter().map(|(id, contacts)| (*id, contacts)))
            .filter_map(|(subject, contacts)| contacts.sessions().find(|s| s.seat == Some(source)).map(|s| (subject, s.nonce)))
            .collect();
        for (subject, nonce) in subjects {
            packet["avatar"] = serde_json::json!(subject);
            packet["nonce"] = serde_json::json!(nonce);
            // Reuse the exact atomic profile/revision parser and live-source
            // authorization used by requested data, without inventing a lease.
            self.receive_hand_contacts(
                &ChatMessage {
                    message: format!("{PREFIX}{packet}"),
                    ..chat.clone()
                },
                now,
            );
        }
        true
    }
}

#[cfg(test)]
mod tests;
