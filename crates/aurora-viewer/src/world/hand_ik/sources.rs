//! Independent leases for an avatar's seat and owned worn roots. A seated
//! creation wins a bone conflict; worn roots then use stable UUID order.
use super::*;
use std::collections::{HashMap, HashSet};

const ANNOUNCE: &str = "auroraIKAvailable";
const PROBE_SPACING: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct Contacts {
    pub(crate) relay: Option<super::relay::Relay>,
    /// Live sessions require the worn relay; offline protocol fixtures can
    /// exercise direct simulator delivery without provisioning an attachment.
    pub(crate) relay_required: bool,
    pub(super) declarations: super::relay::declarations::Declarations,
    pub(super) subject: Option<Uuid>,
    pub(super) observers: HashMap<Uuid, Contacts>,
    pub(super) observed_region: Option<RegionHandle>,
    pub(super) scan_at: Option<Instant>,
    pub(super) network_at: Option<Instant>,
    pub(super) observer_cursor: usize,
    pub(super) primary: Session,
    pub(super) editor: Session,
    pub(super) worn: HashMap<Uuid, Session>,
    next_probe: Option<Instant>,
    cursor: usize,
    pub(super) hints: Vec<Uuid>,
    graphs: Vec<GraphRuntime>,
    pub(super) ingress: limits::Ingress,
    pub(super) notice_ingress: limits::Ingress,
    pub(super) relay_diagnostics: super::diagnostics::RelayCounters,
}

pub fn attachments(world: &World) -> Vec<Seat> {
    attachments_for(world, world.agent_id)
}

pub(super) fn attachments_for(world: &World, subject: Uuid) -> Vec<Seat> {
    if world.teleporting {
        return Vec::new();
    }
    let Some(avatar) = world
        .objects
        .index_of_uuid(&subject)
        .and_then(|i| world.objects.get(i))
        .filter(|o| o.is_avatar())
    else {
        return Vec::new();
    };
    if world.main_region.is_some_and(|region| region != avatar.key.region) {
        return Vec::new();
    }
    let mut roots: Vec<_> = world
        .objects
        .children_of(&avatar.key)
        .iter()
        .filter_map(|i| {
            let object = world.objects.get(*i)?;
            // A full update may omit ownership; direct avatar parentage is
            // sufficient to discover a candidate. Its chat owner is checked
            // against the wearer before accepting any definition.
            (!object.is_avatar() && (object.owner_id.is_nil() || object.owner_id == subject) && object.key.region == avatar.key.region)
                .then_some(Seat {
                    root: object.full_id,
                    region: object.key.region,
                })
        })
        .take(128)
        .collect();
    roots.sort_by_key(|s| s.root);
    roots
}

pub fn eligible_for(world: &World, subject: Uuid, source: Seat) -> bool {
    if world.teleporting {
        return false;
    }
    if avatar_seat(world, subject) == Some(source) {
        return true;
    }
    let Some(object) = world.objects.index_of_uuid(&source.root).and_then(|i| world.objects.get(i)) else {
        return false;
    };
    (object.owner_id.is_nil() || object.owner_id == subject)
        && object.key.region == source.region
        && world.main_region.is_none_or(|region| region == source.region)
        && !object.is_avatar()
        && world
            .objects
            .parent_of(object)
            .and_then(|i| world.objects.get(i))
            .is_some_and(|a| a.is_avatar() && a.full_id == subject && a.key.region == source.region)
}

impl Contacts {
    pub(crate) fn for_each_definition(&self, f: &mut impl FnMut(&super::super::rig_graph::Definition, Uuid, Uuid)) {
        for session in std::iter::once(&self.primary)
            .chain(self.worn.values())
            .chain(std::iter::once(&self.editor))
        {
            if let (Some(source), Some(definition)) = (session.seat, &session.graph.definition) {
                f(definition, source.root, session.graph.bundle().unwrap_or_default());
            }
        }
        for contacts in self.observers.values() {
            contacts.for_each_definition(f);
        }
    }
    pub(super) fn session_mut(&mut self, source: Seat) -> Option<&mut Session> {
        if self.primary.seat == Some(source) {
            Some(&mut self.primary)
        } else {
            self.worn.get_mut(&source.root)
        }
    }
    pub(super) fn worn_sessions(&self) -> impl Iterator<Item = &Session> {
        self.worn.values()
    }
    pub(super) fn sources(&self) -> Vec<Seat> {
        self.primary
            .seat
            .into_iter()
            .chain(self.worn.values().filter_map(|s| s.seat))
            .collect()
    }

    pub(super) fn refresh_due(&self, now: Instant) -> Option<Instant> {
        if self.next_probe.is_some_and(|at| now < at) {
            return None;
        }
        std::iter::once(&self.primary)
            .chain(self.worn.values())
            .filter(|s| s.received_at.is_some() && s.due(now))
            .filter_map(|s| s.sent_at)
            .min()
    }

    pub(super) fn needs_sync(&self, world: &World, subject: Uuid, now: Instant) -> bool {
        std::iter::once(&self.primary).chain(self.worn.values()).any(|session| {
            session.seat.is_some_and(|s| !eligible_for(world, subject, s))
                || (!session.push && session.received_at.is_some_and(|at| now.saturating_duration_since(at) >= LEASE))
                || session.graph.expired_transfer(now)
                || session.reload.as_ref().is_some_and(|r| r.expired_result(now))
        })
    }

    pub fn graphs(&self) -> &[GraphRuntime] {
        &self.graphs
    }

    pub(super) fn merge(&mut self) {
        self.graphs.clear();
        let mut sessions: Vec<_> = self.worn.values().collect();
        sessions.sort_by_key(|s| s.seat.map(|s| s.root));
        sessions.insert(0, &self.primary);
        let mut bones = HashSet::new();
        for session in sessions {
            let Some(source) = session.seat else { continue };
            if session.graph.enabled
                && let (Some(definition), Some((anchor, phase))) = (&session.graph.definition, session.graph.clock)
            {
                let bindings: Vec<_> = definition
                    .bindings
                    .iter()
                    .enumerate()
                    .filter(|(index, b)| {
                        b.enabled
                            && session.graph.profile_allows(*index, &b.bone)
                            && session.graph.binding_enabled(*index)
                            && !bones.contains(&b.bone)
                    })
                    .map(|(index, _)| index)
                    .collect();
                let mut claimed = Vec::new();
                for &index in &bindings {
                    let name = &definition.bindings[index].bone;
                    if !claimed.contains(name) {
                        claimed.push(name.clone());
                    }
                }
                bones.extend(claimed.iter().cloned());
                if !claimed.is_empty() {
                    self.graphs.push(GraphRuntime {
                        source,
                        definition: definition.clone(),
                        anchor,
                        phase,
                        bones: claimed,
                        bindings,
                        smoothing: session.graph.smoothing(),
                        activation: session.graph.activation(),
                    });
                }
            }
        }
    }

    pub(super) fn sync_sources(&mut self, seat: Option<Seat>, worn: &[Seat], now: Instant) {
        self.primary.track(seat, now);
        self.primary.subject = self.subject;
        self.worn.retain(|id, _| worn.iter().any(|s| s.root == *id));
        for source in worn {
            let session = self.worn.entry(source.root).or_default();
            session.track(Some(*source), now);
            session.subject = self.subject;
        }
        self.hints
            .retain(|id| self.worn.contains_key(id) || self.primary.seat.is_some_and(|s| s.root == *id));
        self.merge();
    }

    pub(super) fn probe(&mut self, seat: Option<Seat>, agent: Uuid, now: Instant) -> Option<NetCommand> {
        if self.next_probe.is_some_and(|at| now < at) {
            return None;
        }
        let established = std::iter::once(&self.primary)
            .chain(self.worn.values())
            .filter(|s| s.received_at.is_some() && s.due(now))
            .min_by_key(|s| s.sent_at)
            .and_then(|s| s.seat);
        if let Some(source) = established {
            let session = if self.primary.seat == Some(source) {
                &mut self.primary
            } else {
                self.worn.get_mut(&source.root)?
            };
            let result = session.update(Some(source), agent, now);
            if result.is_some() {
                self.next_probe = Some(now + PROBE_SPACING);
            }
            return result;
        }
        // Finish one initial multipart transfer before starting another source.
        // Known leases above retain priority over discovery traffic.
        if std::iter::once(&self.primary)
            .chain(self.worn.values())
            .any(|s| s.graph.transferring(now))
        {
            return None;
        }
        let hinted = self.hints.first().copied();
        let result = if let Some(root) = hinted {
            self.hints.remove(0);
            let session = if self.primary.seat.is_some_and(|s| s.root == root) {
                &mut self.primary
            } else {
                self.worn.get_mut(&root)?
            };
            session.sent_at = None;
            session.probes = 0;
            session.update(session.seat, agent, now)
        } else if self.primary.due(now) {
            self.primary.update(seat, agent, now)
        } else {
            let mut roots: Vec<_> = self.worn.keys().copied().collect();
            roots.sort();
            let count = roots.len();
            let mut command = None;
            for delta in 0..count {
                let index = (self.cursor + delta) % count;
                let Some(session) = self.worn.get_mut(&roots[index]).filter(|s| s.due(now)) else {
                    continue;
                };
                command = session.update(session.seat, agent, now);
                self.cursor = (index + 1) % count;
                break;
            }
            command
        };
        if result.is_some() {
            self.next_probe = Some(now + PROBE_SPACING);
        }
        result
    }

    pub fn receive(&mut self, source: Seat, chat: &ChatMessage, now: Instant, allowance: usize) -> bool {
        let Some(session) = self.session_mut(source) else { return false };
        if chat.message == ANNOUNCE {
            if session
                .announced_at
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(5))
            {
                session.announced_at = Some(now);
                if !self.hints.contains(&source.root) {
                    self.hints.push(source.root);
                }
            }
            return true;
        }
        let accepted = session.receive(source, chat, now, allowance);
        if accepted {
            self.merge();
        }
        accepted
    }
}
