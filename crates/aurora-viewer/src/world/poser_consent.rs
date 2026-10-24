//! Explicit, region-scoped photo-posing consent. No viewer detection or probes.
use super::World;
use aurora_net::{
    NetCommand,
    bridge::{
        Endpoint,
        query::{PoseKind, Query},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(60);
const COOLDOWN: Duration = Duration::from_secs(600);
const LIMIT: usize = 64;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Policy {
    Everyone,
    #[default]
    Friends,
    Nobody,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub policy: Policy,
    pub blocked: HashSet<Uuid>,
}
#[derive(Clone, Copy)]
pub struct Request {
    pub id: Uuid,
    pub peer: Uuid,
    pub at: Instant,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ask,
    Pending,
    Accepted,
    Refused,
    NoAnswer,
}
#[derive(Default)]
pub struct State {
    agent: Uuid,
    region: Option<u64>,
    pub preferences: Preferences,
    pub incoming: VecDeque<Request>,
    outgoing: HashMap<Uuid, Request>,
    pub granted_to: HashMap<Uuid, Uuid>,
    accepted: HashMap<Uuid, Uuid>,
    refused: HashMap<Uuid, (Instant, bool)>,
    inbound_cooldown: HashMap<Uuid, Instant>,
    ended: HashMap<(Uuid, Uuid), Instant>,
    out: VecDeque<(Uuid, PoseKind, Uuid)>,
    pub notice: Option<&'static str>,
    pub dirty: bool,
}
#[cfg(test)]
mod tests;
pub fn present(world: &World, peer: Uuid) -> bool {
    let Some(own_index) = world.objects.index_of_uuid(&world.agent_id) else {
        return false;
    };
    let Some(other_index) = world.objects.index_of_uuid(&peer) else {
        return false;
    };
    let Some(own) = world.objects.get(own_index) else {
        return false;
    };
    let Some(other) = world.objects.get(other_index) else {
        return false;
    };
    peer != world.agent_id
        && other.is_avatar()
        && other.key.region == own.key.region
        && Some(other.key.region) == world.main_region
        && !world.mutes.is_muted_id(&peer)
}
pub fn distance(world: &World, peer: Uuid) -> Option<f32> {
    let own = world.objects.index_of_uuid(&world.agent_id)?;
    let other = world.objects.index_of_uuid(&peer)?;
    let now = Instant::now();
    let a = crate::scene::Scene::object_transform(world, own, now, 0)?;
    let b = crate::scene::Scene::object_transform(world, other, now, 0)?;
    Some(a.0.distance(b.0))
}
pub fn nearby(world: &World, peer: Uuid) -> bool {
    present(world, peer) && distance(world, peer).is_some_and(|d| d <= 50.0)
}
pub fn name(world: &World, peer: Uuid) -> String {
    world
        .social
        .avatar_names
        .complete(&peer)
        .or_else(|| world.person_name(&peer))
        .unwrap_or_else(|| aurora_i18n::tr!("poser-resident").to_string())
}
impl State {
    pub fn sync(&mut self, world: &World, now: Instant) {
        if self.agent != world.agent_id || self.region != world.main_region || world.teleporting {
            let prefs = self.preferences.clone();
            let dirty = self.dirty;
            *self = Self {
                agent: world.agent_id,
                region: world.main_region,
                preferences: prefs,
                dirty,
                ..Default::default()
            };
        }
        let departed: Vec<_> = self.accepted.keys().copied().filter(|p| !present(world, *p)).collect();
        for p in departed {
            self.cancel(p);
            self.notice = Some("poser-consent-ended");
        }
        let ended: Vec<_> = self.granted_to.keys().copied().filter(|p| !present(world, *p)).collect();
        for peer in ended {
            self.withdraw(peer);
        }
        let expired: Vec<_> = self
            .outgoing
            .values()
            .filter(|r| now.duration_since(r.at) >= TIMEOUT || !nearby(world, r.peer))
            .copied()
            .collect();
        for r in expired {
            self.outgoing.remove(&r.peer);
            self.out.retain(|(id, kind, _)| *id != r.id || *kind != PoseKind::Request);
            self.refused.insert(r.peer, (now, true));
            self.send(r.id, PoseKind::Cancel, r.peer);
        }
        let expired: Vec<_> = self
            .incoming
            .iter()
            .filter(|r| now.duration_since(r.at) >= TIMEOUT || !nearby(world, r.peer))
            .copied()
            .collect();
        for r in expired {
            self.answer(r, false, false, now);
        }
        self.refused.retain(|_, (at, _)| now.duration_since(*at) < COOLDOWN);
        self.inbound_cooldown.retain(|_, at| now.duration_since(*at) < COOLDOWN);
        self.ended.retain(|_, at| now.duration_since(*at) < COOLDOWN);
        let permitted: HashSet<_> = self.incoming.iter().filter(|r| self.allowed(world, r.peer)).map(|r| r.id).collect();
        self.incoming.retain(|r| permitted.contains(&r.id));
    }
    fn allowed(&self, world: &World, peer: Uuid) -> bool {
        nearby(world, peer)
            && !self.preferences.blocked.contains(&peer)
            && match self.preferences.policy {
                Policy::Everyone => true,
                Policy::Friends => world.social.friends.iter().any(|f| f.id == peer),
                Policy::Nobody => false,
            }
    }
    fn send(&mut self, id: Uuid, kind: PoseKind, peer: Uuid) {
        if self.out.len() < LIMIT * 2 {
            self.out.push_back((id, kind, peer));
        }
    }
    pub fn request(&mut self, world: &World, peer: Uuid, now: Instant) -> bool {
        if !nearby(world, peer)
            || self.outgoing.len() >= LIMIT
            || self.status(peer) != Status::Ask && self.status(peer) != Status::NoAnswer
            || self.refused.contains_key(&peer)
        {
            return false;
        }
        let request = Request {
            id: Uuid::new_v4(),
            peer,
            at: now,
        };
        self.outgoing.insert(peer, request);
        self.send(request.id, PoseKind::Request, peer);
        true
    }
    pub fn status(&self, peer: Uuid) -> Status {
        if self.accepted.contains_key(&peer) {
            Status::Accepted
        } else if self.outgoing.contains_key(&peer) {
            Status::Pending
        } else if let Some((_, timeout)) = self.refused.get(&peer) {
            if *timeout { Status::NoAnswer } else { Status::Refused }
        } else {
            Status::Ask
        }
    }
    pub fn can_pose(&self, world: &World, peer: Uuid) -> bool {
        self.accepted.contains_key(&peer) && nearby(world, peer)
    }
    pub fn cancel(&mut self, peer: Uuid) {
        if let Some(r) = self.outgoing.remove(&peer) {
            self.out.retain(|(id, _, _)| *id != r.id);
            self.send(r.id, PoseKind::Cancel, peer);
        }
        if let Some(id) = self.accepted.remove(&peer) {
            self.send(id, PoseKind::Cancel, peer);
        }
    }
    pub fn withdraw(&mut self, peer: Uuid) {
        if let Some(id) = self.granted_to.remove(&peer) {
            if self.ended.len() < LIMIT * 2 {
                self.ended.insert((peer, id), Instant::now());
            }
            self.out.retain(|(request, _, _)| *request != id);
            self.send(id, PoseKind::Withdraw, peer);
        }
    }
    pub fn answer(&mut self, request: Request, accept: bool, block: bool, now: Instant) {
        if !self.incoming.iter().any(|r| r.id == request.id && r.peer == request.peer) {
            return;
        }
        self.incoming.retain(|r| r.id != request.id);
        if block && self.preferences.blocked.len() < 256 {
            self.preferences.blocked.insert(request.peer);
            self.dirty = true;
        }
        if accept && now.duration_since(request.at) < TIMEOUT {
            self.granted_to.insert(request.peer, request.id);
            self.send(request.id, PoseKind::Accept, request.peer);
        } else {
            self.inbound_cooldown.insert(request.peer, now);
            self.send(request.id, PoseKind::Refuse, request.peer);
        }
    }
    /// Called only after the worn bridge root and its owner/region were checked.
    pub fn receive(&mut self, world: &World, text: &str, now: Instant) {
        if text.len() > 160 {
            return;
        }
        let Some(body) = text.strip_prefix("<auroraPose>").and_then(|s| s.strip_suffix("</auroraPose>")) else {
            return;
        };
        let fields: Vec<_> = body.split('|').collect();
        if fields.len() != 3 {
            return;
        }
        let (Ok(id), Some(kind), Ok(peer)) = (Uuid::parse_str(fields[0]), PoseKind::parse(fields[1]), Uuid::parse_str(fields[2])) else {
            return;
        };
        if id.is_nil() || !present(world, peer) {
            return;
        }
        match kind {
            PoseKind::Request => {
                if !self.allowed(world, peer)
                    || self.ended.contains_key(&(peer, id))
                    || self.inbound_cooldown.contains_key(&peer)
                    || self.incoming.len() >= LIMIT
                    || self.incoming.iter().any(|r| r.peer == peer)
                    || self.granted_to.contains_key(&peer)
                {
                    return;
                }
                self.incoming.push_back(Request { id, peer, at: now });
            }
            PoseKind::Accept | PoseKind::Refuse => {
                let Some(r) = self
                    .outgoing
                    .get(&peer)
                    .filter(|r| r.id == id && now.duration_since(r.at) < TIMEOUT)
                    .copied()
                else {
                    return;
                };
                self.outgoing.remove(&peer);
                if kind == PoseKind::Accept {
                    self.accepted.insert(peer, r.id);
                } else {
                    self.refused.insert(peer, (now, false));
                }
            }
            PoseKind::Cancel => {
                if self.ended.len() < LIMIT * 2 {
                    self.ended.insert((peer, id), now);
                }
                self.incoming.retain(|r| r.peer != peer || r.id != id);
                self.out.retain(|(request, _, _)| *request != id);
                if self.granted_to.get(&peer) == Some(&id) {
                    self.granted_to.remove(&peer);
                }
            }
            PoseKind::Withdraw => {
                if self.outgoing.get(&peer).is_some_and(|r| r.id == id) {
                    self.outgoing.remove(&peer);
                    self.out.retain(|(request, _, _)| *request != id);
                    self.notice = Some("poser-consent-ended");
                }
                if self.accepted.get(&peer) == Some(&id) {
                    self.accepted.remove(&peer);
                    self.notice = Some("poser-consent-ended");
                }
            }
        }
    }
    pub fn commands(&mut self, endpoint: Option<Endpoint>) -> Vec<NetCommand> {
        let Some(endpoint) = endpoint else { return Vec::new() };
        self.out
            .drain(..)
            .map(|(request, kind, target)| NetCommand::BridgeQuery {
                request: Uuid::new_v4(),
                endpoint: endpoint.clone(),
                query: Query::Pose { request, kind, target },
            })
            .collect()
    }
    /// Offline fixture only; callers gate this on the application's demo mode.
    pub fn demo(&mut self, world: &World, peer: Uuid, mode: &str, frame: u64) {
        let now = Instant::now();
        if frame == 450 {
            if mode == "consent-request" || mode == "consent-reminder" {
                let id = Uuid::new_v4();
                self.receive(world, &format!("<auroraPose>{id}|request|{peer}</auroraPose>"), now);
            } else {
                self.request(world, peer, now);
            }
        }
        if frame == 480 {
            if mode == "consent-timeout" {
                if let Some(r) = self.outgoing.get_mut(&peer) {
                    r.at = now - TIMEOUT - Duration::from_secs(1);
                }
                self.sync(world, now);
            } else if mode == "consent-reminder" {
                if let Some(r) = self.incoming.front().copied() {
                    self.answer(r, true, false, now);
                }
            } else if mode != "consent-request"
                && let Some(r) = self.outgoing.get(&peer)
            {
                let id = r.id;
                let kind = if mode == "consent-refuse" { "refuse" } else { "accept" };
                self.receive(world, &format!("<auroraPose>{id}|{kind}|{peer}</auroraPose>"), now);
            }
        }
        if frame == 560
            && mode == "consent-withdraw"
            && let Some(id) = self.accepted.get(&peer).copied()
        {
            self.receive(world, &format!("<auroraPose>{id}|withdraw|{peer}</auroraPose>"), now);
        }
    }
}
