//! One explicit authoring snapshot, isolated from every runtime session.
//! A selected unused object may answer without activating or caching live IK.
use super::*;

pub const TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub struct SeatData {
    pub runtime: GraphRuntime,
    pub enabled: bool,
}

pub enum ReloadStatus<'a> {
    Waiting { receiving: bool },
    Ready(&'a SeatData),
    TimedOut,
    Cancelled,
}

pub(super) struct Request {
    id: Uuid,
    pub(super) at: Instant,
    result: Option<SeatData>,
    bytes: usize,
}
impl Request {
    pub(super) fn expired_result(&self, now: Instant) -> bool {
        self.result.is_some() && now.saturating_duration_since(self.at) >= TIMEOUT
    }
    pub(super) fn prune(&mut self, now: Instant) {
        if self.expired_result(now) {
            self.result = None;
            self.bytes = 0;
        }
    }
    pub(super) fn retained_bytes(&self, current: Option<&std::sync::Arc<crate::world::rig_graph::Definition>>) -> usize {
        self.result.as_ref().map_or(0, |r| {
            if current.is_some_and(|d| std::sync::Arc::ptr_eq(d, &r.runtime.definition)) {
                0
            } else {
                self.bytes
            }
        })
    }
}

impl Session {
    pub(super) fn reloading(&self, now: Instant) -> bool {
        self.reload
            .as_ref()
            .is_some_and(|r| r.result.is_none() && now.saturating_duration_since(r.at) < TIMEOUT)
    }

    pub(super) fn complete_reload(&mut self, now: Instant) {
        if !self.reloading(now) {
            return;
        }
        let (Some(source), Some(definition), Some((anchor, phase))) = (self.seat, &self.graph.definition, self.graph.clock) else {
            return;
        };
        let bindings: Vec<_> = definition
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.enabled)
            .map(|(index, _)| index)
            .collect();
        let bones = bindings.iter().map(|&index| definition.bindings[index].bone.clone()).collect();
        if let Some(request) = &mut self.reload {
            request.bytes = self.graph.definition_bytes();
            request.result = Some(SeatData {
                runtime: GraphRuntime {
                    source,
                    definition: definition.clone(),
                    anchor,
                    phase,
                    bones,
                    bindings,
                    smoothing: self.graph.smoothing(),
                    activation: self.graph.activation(),
                },
                enabled: self.graph.enabled,
            });
        }
    }
}

impl World {
    /// Queue one selected-object snapshot through the shared regional budget.
    pub fn request_object_ik(&mut self, object: Uuid, now: Instant) -> Option<(Seat, Uuid)> {
        let seat = object_source(self, object)?;
        let contacts = &mut self.hand_contacts;
        if contacts.observed_region != Some(seat.region) {
            let required = contacts.relay_required;
            let relay = contacts.relay.take().filter(|r| r.region == seat.region);
            *contacts = Contacts::default();
            contacts.relay_required = required;
            contacts.relay = relay;
            contacts.observed_region = Some(seat.region);
        }
        contacts.subject = Some(self.agent_id);
        let session = &mut contacts.editor;
        *session = Session::default();
        session.track(Some(seat), now);
        session.subject = Some(self.agent_id);
        session.push = true;
        let id = Uuid::new_v4();
        session.reload = Some(Request {
            id,
            at: now,
            result: None,
            bytes: 0,
        });
        // Retire replies already in flight, including a partially received rig.
        session.nonce = Uuid::nil();
        session.sent_at = None;
        session.probes = 0;
        session.graph.cancel_transfer();
        Some((seat, id))
    }

    pub fn cancel_object_ik(&mut self) {
        self.hand_contacts.editor = Session::default();
    }

    pub fn object_ik_reload(&self, seat: Seat, id: Uuid, now: Instant) -> ReloadStatus<'_> {
        let session = &self.hand_contacts.editor;
        if object_source(self, seat.root) != Some(seat) || session.seat != Some(seat) {
            return ReloadStatus::Cancelled;
        }
        let Some(request) = session.reload.as_ref().filter(|r| r.id == id) else {
            return ReloadStatus::Cancelled;
        };
        if now.saturating_duration_since(request.at) >= TIMEOUT {
            ReloadStatus::TimedOut
        } else if let Some(result) = &request.result {
            ReloadStatus::Ready(result)
        } else {
            ReloadStatus::Waiting {
                receiving: session.graph.transferring(now),
            }
        }
    }
}

/// Normalize child clicks to a world or worn-object root using actual parentage.
pub fn object_source(world: &World, id: Uuid) -> Option<Seat> {
    if world.teleporting {
        return None;
    }
    let mut index = world.objects.index_of_uuid(&id)?;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..64 {
        let object = world.objects.get(index)?;
        if object.is_avatar() || Some(object.key.region) != world.main_region || !seen.insert(index) {
            return None;
        }
        if object.parent_id == 0 || world.objects.get(world.objects.parent_of(object)?)?.is_avatar() {
            return Some(Seat {
                root: object.full_id,
                region: object.key.region,
            });
        }
        index = world.objects.parent_of(object)?;
    }
    None
}
