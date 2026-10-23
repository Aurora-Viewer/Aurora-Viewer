//! Contacts supplied by a seated creation or an owned worn attachment. The
//! script sends definitions, never a stream of joint poses or executable code.

use super::World;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand, RegionHandle};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

mod diagnostics;
mod graph_wire;
mod limits;
mod observers;
mod profiles;
pub(crate) mod relay;
mod reload;
mod sources;
pub use graph_wire::{ActivationStamp, Runtime as GraphRuntime};
pub use reload::{ReloadStatus, object_source};
pub use sources::eligible_for as source_eligible_for;
pub use sources::{Contacts, attachments as attachment_sources};

pub const CHANNEL: i32 = -190018107;
pub use graph_wire::PREFIX;
const RETRY: Duration = Duration::from_secs(3);
const REFRESH: Duration = Duration::from_secs(25);
const LEASE: Duration = Duration::from_secs(70);

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Hand {
    #[serde(rename = "mWristLeft")]
    Left,
    #[serde(rename = "mWristRight")]
    Right,
}

#[cfg(test)]
impl Hand {
    pub fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }

    pub fn joints(self) -> [&'static str; 3] {
        match self {
            Self::Left => ["mShoulderLeft", "mElbowLeft", "mWristLeft"],
            Self::Right => ["mShoulderRight", "mElbowRight", "mWristRight"],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintMode {
    #[default]
    Ik,
    Direct,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    #[serde(default, skip_serializing_if = "super::rig_graph::Activation::is_default")]
    pub activation: super::rig_graph::Activation,
    /// Zero releases the correction; one fully follows the target.
    #[serde(default = "full_weight")]
    pub weight: f32,
    pub bone: String,
    pub target: Uuid,
    pub position: [f32; 3],
    pub rotation: Option<[f32; 4]>,
    /// Optional pole point in the target prim's metric local axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pole: Option<[f32; 3]>,
    #[serde(default)]
    pub contact: [f32; 3],
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub mode: ConstraintMode,
}

fn full_weight() -> f32 {
    1.0
}

impl Constraint {
    pub(crate) fn valid(&self) -> bool {
        let name = |s: &str| !s.is_empty() && s.len() <= 64 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
        let position = Vec3::from_array(self.position);
        let contact = Vec3::from_array(self.contact);
        name(&self.bone)
            && self.activation.valid()
            && self.weight.is_finite()
            && (0.0..=1.0).contains(&self.weight)
            && self.root.as_deref().is_none_or(name)
            && !self.target.is_nil()
            && position.is_finite()
            && position.length_squared() <= 64.0 * 64.0
            && contact.is_finite()
            && contact.length_squared() <= 0.3 * 0.3
            && self.pole.is_none_or(|p| {
                let p = Vec3::from_array(p);
                self.mode == ConstraintMode::Ik && p.is_finite() && p.length_squared() <= 64.0 * 64.0
            })
            && self.rotation.is_none_or(|r| {
                let q = Quat::from_array(r);
                q.is_finite() && (0.9..=1.1).contains(&q.length_squared())
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub root: Uuid,
    pub region: RegionHandle,
}

#[derive(Default)]
struct Session {
    push: bool,
    advertised_bundle: Option<Uuid>,
    subject: Option<Uuid>,
    seat: Option<Seat>,
    nonce: Uuid,
    probes: u8,
    sent_at: Option<Instant>,
    received_at: Option<Instant>,
    graph: graph_wire::State,
    reload: Option<reload::Request>,
    ingress: limits::Ingress,
    announced_at: Option<Instant>,
}

impl Session {
    fn track(&mut self, seat: Option<Seat>, now: Instant) {
        if self.seat != seat {
            *self = Self { seat, ..Self::default() };
        }
        self.graph.prune(now);
        if let Some(request) = &mut self.reload {
            request.prune(now);
        }
        if !self.push && self.received_at.is_some_and(|at| now.saturating_duration_since(at) >= LEASE) {
            self.graph = graph_wire::State::default();
            self.nonce = Uuid::nil();
            self.received_at = None;
            self.reload = None;
            self.probes = 0;
        }
    }

    fn due(&self, now: Instant) -> bool {
        self.seat.is_some()
            && !(self.push && self.received_at.is_some() && !self.reloading(now))
            && !(self.push && self.probes >= 5)
            && !self.graph.transferring(now)
            && !(self.received_at.is_none()
                && self.probes >= 5
                && self
                    .sent_at
                    .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(60)))
            && self.sent_at.is_none_or(|at| {
                now.saturating_duration_since(at)
                    >= if self.received_at.is_some() && !self.reloading(now) {
                        REFRESH
                    } else {
                        RETRY
                    }
            })
    }

    fn update(&mut self, seat: Option<Seat>, agent: Uuid, now: Instant) -> Option<NetCommand> {
        self.track(seat, now);
        let seat = self.seat?;
        if !self.due(now) {
            return None;
        }
        // Same transport as bridge::discover / really_send_chat_from_viewer:
        // ScriptDialogReply emits avatar chat on negative channels. The
        // script filters the avatar and the root UUID before answering.
        self.nonce = Uuid::new_v4();
        self.sent_at = Some(now);
        self.probes = self.probes.saturating_add(1);
        Some(NetCommand::ScriptDialogReply {
            sender: None,
            object_id: seat.root,
            channel: CHANNEL,
            index: 0,
            label: format!(
                "{}|{}|{}|{}|{}",
                if self.reload.is_some() { "auroraIKEdit" } else { "auroraIKObserve" },
                seat.root,
                self.subject.unwrap_or(agent),
                self.nonce,
                if self.reloading(now) {
                    Uuid::nil()
                } else {
                    self.graph.bundle().unwrap_or(Uuid::nil())
                }
            ),
        })
    }

    fn receive(&mut self, seat: Seat, chat: &ChatMessage, now: Instant, allowance: usize) -> bool {
        if self.seat != Some(seat) || chat.source_id != seat.root || !chat.message.starts_with(PREFIX) {
            return false;
        }
        let serial = self.graph.snapshot_serial();
        if let Some(subject) = self.subject
            && self.graph.receive_bounded(&chat.message, subject, self.nonce, now, allowance)
            // A release may safely stop a still unknown rig, but it cannot
            // acknowledge its definition or stop the finite initial retries.
            && self.graph.definition.is_some()
            && self.advertised_bundle.is_none_or(|bundle| self.graph.bundle() == Some(bundle))
        {
            self.received_at = Some(now);
            if self.graph.snapshot_serial() != serial {
                self.complete_reload(now);
            }
        }
        true
    }
}

/// The avatar's seat may be a child prim. Resolve through known parentage,
/// never through the UUIDs or names supplied in a chat message.
pub fn own_seat(world: &World) -> Option<Seat> {
    avatar_seat(world, world.agent_id)
}

pub fn avatar_seat(world: &World, subject: Uuid) -> Option<Seat> {
    if world.teleporting {
        return None;
    }
    let avatar = world.objects.get(world.objects.index_of_uuid(&subject)?)?;
    if !avatar.is_avatar() || world.main_region.is_some_and(|region| region != avatar.key.region) {
        return None;
    }
    let mut index = world.objects.parent_of(avatar)?;
    for _ in 0..17 {
        let o = world.objects.get(index)?;
        if o.is_avatar() {
            return None;
        }
        if o.parent_id == 0 {
            return (o.key.region == avatar.key.region).then_some(Seat {
                root: o.full_id,
                region: o.key.region,
            });
        }
        index = world.objects.parent_of(o)?;
    }
    None
}

/// A target is a rigid prim in that seat's linkset. Avatars, worn objects,
/// unrelated objects and cyclic/missing parent chains are excluded.
pub fn target_index(world: &World, seat: Seat, id: Uuid) -> Option<usize> {
    let index = world.objects.index_of_uuid(&id)?;
    let mut current = index;
    for _ in 0..17 {
        let o = world.objects.get(current)?;
        if o.is_avatar() || o.key.region != seat.region {
            return None;
        }
        if o.full_id == seat.root {
            return Some(index);
        }
        current = world.objects.parent_of(o)?;
    }
    None
}

impl World {
    pub fn update_hand_contacts(&mut self, now: Instant) -> Option<NetCommand> {
        let mut contacts = std::mem::take(&mut self.hand_contacts);
        let command = contacts.update_world(self, now);
        let command = command.map(|command| match &contacts.relay {
            Some(relay) => relay.route(command),
            None => command,
        });
        self.hand_contacts = contacts;
        command
    }

    pub(super) fn receive_hand_contacts(&mut self, chat: &ChatMessage, now: Instant) -> bool {
        if chat.source_type != ChatSourceType::Object {
            return false;
        }
        if !chat.message.starts_with(PREFIX)
            && chat.message != "auroraIKAvailable"
            && !chat.message.starts_with(relay::AVAILABLE)
            && !chat.message.starts_with(relay::STATE)
        {
            return false;
        }
        if !matches!(chat.chat_type, ChatType::Normal | ChatType::OwnerSay | ChatType::Other(9)) {
            return true;
        }
        let source = self
            .objects
            .index_of_uuid(&chat.source_id)
            .and_then(|i| self.objects.get(i))
            // LLViewerObject::processUpdateMessage: full ObjectUpdate's
            // OwnerID is only valid with sound/particles. Simulator chat
            // authenticates its source/owner even when that field is absent.
            .filter(|o| !o.is_avatar() && !chat.owner_id.is_nil() && (o.owner_id.is_nil() || o.owner_id == chat.owner_id))
            .map(|o| Seat {
                root: o.full_id,
                region: o.key.region,
            });
        // Protocol messages are never retained as ordinary chat, even when
        // their source is unused, unknown, oversized or unauthenticated.
        let Some(source) = source else {
            self.hand_contacts.relay_diagnostics.failure = "source_unknown_or_owner_mismatch";
            return true;
        };
        if self.receive_ik_notice(source, chat, now) || self.receive_ik_state(source, chat, now) {
            return true;
        }
        if !self.hand_contacts.admits(source, now) {
            self.hand_contacts.relay_diagnostics.failure = "source_unknown_or_ingress_limit";
            return true;
        }
        if chat.message.starts_with(PREFIX) {
            if self.hand_contacts.editor.seat == Some(source) && self.hand_contacts.editor.reloading(now) {
                let own = self.hand_contacts.editor.retained_bytes();
                let allowance = limits::REGION_BYTES
                    .saturating_sub(self.hand_contacts.total_retained_bytes().saturating_sub(own))
                    .min(limits::AVATAR_BYTES.saturating_sub(self.hand_contacts.retained_bytes().saturating_sub(own)))
                    .min(super::rig_graph::MAX_BYTES);
                self.hand_contacts.editor.receive(source, chat, now, allowance);
            }
            if let Some(subject) = graph_wire::observer_subject(&chat.message) {
                let allowance = self.hand_contacts.graph_allowance(subject, source);
                if sources::eligible_for(self, subject, source) && (avatar_seat(self, subject) == Some(source) || chat.owner_id == subject)
                {
                    if subject == self.agent_id {
                        self.hand_contacts.receive(source, chat, now, allowance);
                    } else {
                        self.hand_contacts.receive_observer(subject, source, chat, now, allowance);
                    }
                } else {
                    self.hand_contacts.relay_diagnostics.failure = "subject_not_eligible";
                }
            } else {
                self.hand_contacts.relay_diagnostics.failure = "invalid_definition_envelope";
            }
            return true;
        }
        let subjects: Vec<_> = std::iter::once((self.agent_id, &self.hand_contacts))
            .chain(self.hand_contacts.observers.iter().map(|(id, c)| (*id, c)))
            .filter(|(id, c)| {
                c.sessions().any(|s| s.seat == Some(source))
                    && sources::eligible_for(self, *id, source)
                    && (avatar_seat(self, *id) == Some(source) || chat.owner_id == *id)
            })
            .map(|(id, _)| id)
            .collect();
        for subject in subjects {
            if subject == self.agent_id {
                self.hand_contacts.receive(source, chat, now, 0);
            } else {
                self.hand_contacts.receive_observer(subject, source, chat, now, 0);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
