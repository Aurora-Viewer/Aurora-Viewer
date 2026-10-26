//! Region-local, voluntary portal crossings. Invitations are immutable for
//! their lifetime: replaying one never moves it or resets its proximity gate.
use super::World;
use aurora_net::{
    NetCommand,
    bridge::{
        Endpoint,
        portal::{Access, CLOCK_SKEW, LIFETIME, Message, Portal},
        query::{Query, Reply},
    },
};
use glam::Vec3;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const WIDTH: f32 = 1.0;
pub const HEIGHT: f32 = 1.5;
const LIMIT: usize = 64;
const CLEARANCE: f32 = 3.5;

pub fn fits_region(center: Vec3, size: [u32; 2]) -> bool {
    center.is_finite() && (WIDTH..=size[0] as f32 - WIDTH).contains(&center.x) && (WIDTH..=size[1] as f32 - WIDTH).contains(&center.y)
}

pub fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub fn allowed(world: &World, owner: Uuid, access: Access) -> bool {
    !world.mutes.is_muted_id(&owner)
        && (owner == world.agent_id
            || match access {
                Access::Everyone => true,
                Access::Friends => world.social.friends.iter().any(|friend| friend.id == owner),
                Access::Group(id) => world.groups.groups.iter().any(|group| group.id == id),
            })
}

pub struct Entry {
    pub owner: Uuid,
    pub portal: Portal,
    gate: Gate,
    expires_at: Instant,
}

impl Entry {
    pub fn waiting_for_clearance(&self) -> bool {
        !self.gate.armed
    }
}

struct Gate {
    previous: Vec3,
    armed: bool,
    appeared: Instant,
    last_sample: Instant,
}

impl Gate {
    fn new(center: Vec3, position: Vec3, now: Instant) -> Self {
        Self {
            previous: position,
            armed: position.distance(center) > 3.0,
            appeared: now,
            last_sample: now,
        }
    }

    fn sample(&mut self, portal: &Portal, position: Vec3, voluntary: bool, now: Instant) -> bool {
        let previous = self.previous;
        self.previous = position;
        let gap = now.saturating_duration_since(self.last_sample);
        self.last_sample = now;
        let center = Vec3::from(portal.center);
        // Arrival, simulator corrections and stale frame gaps cannot be entry.
        if !position.is_finite() || !previous.is_finite() || previous.distance(position) > 3.0 || gap > Duration::from_secs(1) {
            self.armed = false;
            return false;
        }
        if !self.armed {
            if position.distance(center) >= CLEARANCE {
                self.armed = true;
            }
            return false;
        }
        if now.saturating_duration_since(self.appeared) < Duration::from_millis(600) {
            return false;
        }
        let normal = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
        let a = (previous - center).dot(normal);
        let b = (position - center).dot(normal);
        if a * b > 0.0 || (a - b).abs() < 1e-5 {
            return false;
        }
        let at = previous.lerp(position, (a / (a - b)).clamp(0.0, 1.0)) - center;
        let side = Vec3::new(-normal.y, normal.x, 0.0);
        let inside = (at.dot(side) / WIDTH).powi(2) + (at.z / HEIGHT).powi(2) <= 1.0;
        if inside {
            // Consume even a refused or involuntary crossing; no delayed TP.
            self.armed = false;
        }
        inside && voluntary
    }
}

#[derive(Default)]
pub struct State {
    agent: Uuid,
    region: Option<u64>,
    endpoint: Option<Uuid>,
    pub entries: Vec<Entry>,
    ended: HashMap<(Uuid, Uuid), u64>,
    out: VecDeque<(Uuid, Message)>,
    discovered: bool,
    last_send: Option<Instant>,
    discoveries: HashMap<Uuid, Instant>,
    pending: HashMap<Uuid, (Instant, Option<Uuid>)>,
    pub failed: bool,
}

impl State {
    pub fn sync(&mut self, world: &World, endpoint: Option<Uuid>, now: u64) {
        if self.agent != world.agent_id || self.region != world.main_region || self.endpoint != endpoint || world.teleporting {
            *self = Self {
                agent: world.agent_id,
                region: world.main_region,
                endpoint,
                ..Default::default()
            };
        }
        self.entries
            .retain(|entry| entry.portal.live(now) && Instant::now() < entry.expires_at && !world.mutes.is_muted_id(&entry.owner));
        self.ended.retain(|_, until| *until > now);
        self.discoveries.retain(|_, at| at.elapsed() < Duration::from_secs(30));
    }

    pub fn own(&self) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.owner == self.agent)
    }

    pub fn create(&mut self, portal: Portal, position: Vec3, now: Instant, unix: u64) -> bool {
        if Some(portal.region) != self.region || !portal.live(unix) || Message::Open(portal.clone()).encode().is_none() {
            return false;
        }
        if self.out.len() > LIMIT * 2 - 2 || self.entries.len() >= LIMIT && self.own().is_none() {
            return false;
        }
        self.close(unix);
        if self.entries.len() >= LIMIT {
            return false;
        }
        self.queue(Uuid::nil(), Message::Open(portal.clone()));
        self.entries.push(Entry {
            owner: self.agent,
            gate: Gate::new(Vec3::from(portal.center), position, now),
            expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
            portal,
        });
        true
    }

    pub fn close(&mut self, unix: u64) {
        if let Some(entry) = self.own() {
            let id = entry.portal.id;
            let region = entry.portal.region;
            self.out
                .retain(|(_, message)| !matches!(message, Message::Open(portal) if portal.id == id));
            self.remember(self.agent, id, unix);
            self.entries.retain(|entry| entry.owner != self.agent);
            if !self.queue(Uuid::nil(), Message::Close { region, id }) {
                self.failed = true;
            }
        }
    }

    fn remember(&mut self, owner: Uuid, id: Uuid, unix: u64) {
        if self.ended.len() < LIMIT * 4 {
            self.ended.insert((owner, id), unix + LIFETIME + CLOCK_SKEW);
        }
    }

    fn queue(&mut self, target: Uuid, message: Message) -> bool {
        if self.out.len() < LIMIT * 2 {
            if target.is_nil() {
                // Our open/close actions precede responses to late arrivals,
                // while retaining the close-before-replacement ordering.
                let at = self.out.iter().position(|(target, _)| !target.is_nil()).unwrap_or(self.out.len());
                self.out.insert(at, (target, message));
            } else {
                self.out.push_back((target, message));
            }
            true
        } else {
            false
        }
    }

    /// Called only after the worn bridge's source, owner and region checks.
    pub fn receive(&mut self, world: &World, text: &str, now: Instant, unix: u64) {
        let Some(body) = text.strip_prefix("<auroraPortal>").and_then(|s| s.strip_suffix("</auroraPortal>")) else {
            return;
        };
        let Some((owner, wire)) = body.split_once('|') else { return };
        let Ok(owner) = Uuid::parse_str(owner) else { return };
        if owner.is_nil() || owner == world.agent_id || world.mutes.is_muted_id(&owner) {
            return;
        }
        let Some(message) = Message::decode(wire) else { return };
        if Some(message.region()) != world.main_region {
            return;
        }
        match message {
            Message::Open(portal) => {
                if !portal.live(unix)
                    || world
                        .main()
                        .is_some_and(|region| !fits_region(Vec3::from(portal.center), [region.heightmap.size_x, region.heightmap.size_y]))
                    || self.ended.contains_key(&(owner, portal.id))
                    || self.entries.iter().any(|entry| entry.owner == owner)
                    || self.entries.len() >= LIMIT
                {
                    return;
                }
                self.entries.push(Entry {
                    owner,
                    gate: Gate::new(Vec3::from(portal.center), world.agent.position, now),
                    expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
                    portal,
                });
            }
            Message::Close { id, .. } => {
                self.remember(owner, id, unix);
                self.entries.retain(|entry| entry.owner != owner || entry.portal.id != id);
            }
            Message::Discover { .. } => {
                if self
                    .discoveries
                    .get(&owner)
                    .is_some_and(|last| now.saturating_duration_since(*last) < Duration::from_secs(5))
                {
                    return;
                }
                if self.discoveries.len() >= LIMIT {
                    return;
                }
                self.discoveries.insert(owner, now);
                if let Some(own) = self.own() {
                    self.queue(owner, Message::Open(own.portal.clone()));
                }
            }
        }
    }

    pub fn update(&mut self, world: &World, intent: Vec3, now: Instant) -> Option<Portal> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, (at, _))| now.saturating_duration_since(*at) >= Duration::from_secs(8))
            .map(|(request, _)| *request)
            .collect();
        for request in expired {
            self.reply(request, &Err(String::new()));
        }
        let mut crossed = None;
        for entry in &mut self.entries {
            let travel = world.agent.position - entry.gate.previous;
            let can_enter = intent.is_finite() && travel.dot(intent) > 1e-5 && allowed(world, entry.owner, entry.portal.access);
            if entry.gate.sample(&entry.portal, world.agent.position, can_enter, now) && crossed.is_none() {
                crossed = Some(entry.portal.clone());
            }
        }
        crossed
    }

    pub fn command(&mut self, endpoint: Option<&Endpoint>, now: Instant) -> Option<NetCommand> {
        let endpoint = endpoint?;
        if !self.discovered
            && let Some(region) = self.region
        {
            self.queue(Uuid::nil(), Message::Discover { region });
            self.discovered = true;
        }
        if self.pending.len() >= 32
            || self
                .last_send
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(250))
        {
            return None;
        }
        let (target, message) = self.out.pop_front()?;
        self.last_send = Some(now);
        let request = Uuid::new_v4();
        let public_portal = if target.is_nil() {
            match &message {
                Message::Open(portal) => Some(portal.id),
                Message::Close { id, .. } => Some(*id),
                Message::Discover { .. } => None,
            }
        } else {
            None
        };
        self.pending.insert(request, (now, public_portal));
        Some(NetCommand::BridgeQuery {
            request,
            endpoint: endpoint.clone(),
            query: Query::Portal { target, message },
        })
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.pending.contains_key(&request)
    }

    pub fn reply(&mut self, request: Uuid, result: &Result<Reply, String>) {
        let Some((_, portal)) = self.pending.remove(&request) else { return };
        if !matches!(result, Ok(Reply::Portal))
            && let Some(id) = portal
        {
            self.entries.retain(|entry| entry.owner != self.agent || entry.portal.id != id);
            self.failed = true;
        }
    }
}

#[cfg(test)]
mod tests;
