//! Region-local, voluntary portal crossings. Invitations are immutable for
//! their lifetime: replaying one never moves it or resets its proximity gate.
use super::World;
use aurora_net::{
    NetCommand,
    bridge::{
        Endpoint,
        portal::{Access, Announcement, CLOCK_SKEW, Details, LIFETIME, Message, Portal},
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
const SEND_INTERVAL: Duration = Duration::from_millis(250);

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
    pub portal: Announcement,
    pub details: Option<Details>,
    gate: Gate,
    expires_at: Instant,
}

impl Entry {
    pub fn allowed(&self, world: &World) -> bool {
        self.details
            .as_ref()
            .is_some_and(|details| allowed(world, self.owner, details.access))
    }

    fn resolved(&self) -> Option<Portal> {
        let details = self.details.as_ref()?;
        Some(Portal {
            id: self.portal.id,
            region: self.portal.region,
            center: self.portal.center,
            yaw: self.portal.yaw,
            expires: self.portal.expires,
            destination: details.destination.region.clone(),
            position: details.destination.position,
            access: details.access,
        })
    }

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

    fn sample(&mut self, portal: &Announcement, position: Vec3, voluntary: bool, now: Instant) -> bool {
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

struct Fetch {
    nonce: Uuid,
    attempts: u8,
    queued: Instant,
}

struct Inquiry {
    id: Uuid,
    nonce: Uuid,
    received: Instant,
    replied: bool,
}

struct Delivery {
    sent: Instant,
    action: Option<Message>,
    attempt: u8,
}

struct Retry {
    due: Instant,
    action: Message,
    attempt: u8,
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
    last_exchange: Option<Instant>,
    discoveries: HashMap<Uuid, Instant>,
    pending: HashMap<Uuid, Delivery>,
    retries: VecDeque<Retry>,
    fetches: HashMap<(Uuid, Uuid), Fetch>,
    inquiries: HashMap<Uuid, Inquiry>,
    group_refresh: Option<Uuid>,
    pub failed: bool,
}

impl State {
    pub fn sync(&mut self, world: &World, endpoint: Option<Uuid>, now: u64) {
        if self.agent != world.agent_id || self.region != world.main_region || world.teleporting {
            *self = Self {
                agent: world.agent_id,
                region: world.main_region,
                endpoint,
                ..Default::default()
            };
        }
        // Renewing HTTP-in within the same region must not erase invitations
        // or reset an observer's clearance gate. Session/region changes do.
        if self.endpoint != endpoint {
            self.endpoint = endpoint;
            self.discovered = false;
        }
        self.entries
            .retain(|entry| entry.portal.live(now) && Instant::now() < entry.expires_at && !world.mutes.is_muted_id(&entry.owner));
        self.ended.retain(|_, until| *until > now);
        self.discoveries.retain(|_, at| at.elapsed() < Duration::from_secs(30));
        self.fetches.retain(|(owner, id), _| {
            self.entries
                .iter()
                .any(|e| e.owner == *owner && e.portal.id == *id && e.details.is_none())
        });
        self.inquiries
            .retain(|_, inquiry| inquiry.received.elapsed() < Duration::from_secs(30));
        self.out.retain(|(target, message)| match message {
            Message::Open(portal) => portal.live(now),
            Message::Details { id, .. } => self.entries.iter().any(|entry| entry.owner == self.agent && entry.portal.id == *id),
            Message::Request { id, .. } => self
                .entries
                .iter()
                .any(|entry| entry.owner == *target && entry.portal.id == *id && entry.details.is_none()),
            _ => true,
        });
        let own = self.own().map(|entry| entry.portal.id);
        self.retries.retain(|retry| match &retry.action {
            Message::Open(portal) => Some(portal.id) == own && portal.live(now),
            Message::Close { id, .. } => self.ended.contains_key(&(self.agent, *id)),
            _ => false,
        });
    }

    pub fn own(&self) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.owner == self.agent)
    }

    /// One fresh server-backed member list per group portal, shared by all
    /// requesters. Never authorize using a claim made by the requesting viewer.
    pub fn take_group_refresh(&mut self) -> Option<Uuid> {
        self.group_refresh.take()
    }

    pub fn create(&mut self, portal: Portal, position: Vec3, now: Instant, unix: u64) -> bool {
        let private_fits = portal.access == Access::Everyone
            || Message::Details {
                region: portal.region,
                id: portal.id,
                nonce: portal.id,
                details: portal.details(),
            }
            .encode()
            .is_some();
        if Some(portal.region) != self.region
            || !portal.live(unix)
            || !private_fits
            || Message::Open(portal.announcement()).encode().is_none()
        {
            return false;
        }
        if self.out.len() > LIMIT * 2 - 2 || self.entries.len() >= LIMIT && self.own().is_none() {
            return false;
        }
        self.close(unix);
        if self.entries.len() >= LIMIT {
            return false;
        }
        self.group_refresh = if let Access::Group(id) = portal.access { Some(id) } else { None };
        let announcement = portal.announcement();
        self.queue(Uuid::nil(), Message::Open(announcement.clone()));
        self.entries.push(Entry {
            owner: self.agent,
            gate: Gate::new(Vec3::from(portal.center), position, now),
            expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
            details: Some(portal.details()),
            portal: announcement,
        });
        true
    }

    pub fn close(&mut self, unix: u64) {
        if let Some(entry) = self.own() {
            let id = entry.portal.id;
            let region = entry.portal.region;
            self.out.retain(|(_, message)| {
                !matches!(message, Message::Open(portal) if portal.id == id)
                    && !matches!(message, Message::Details { id: old, .. } if *old == id)
            });
            self.retries
                .retain(|retry| !matches!(&retry.action, Message::Open(portal) if portal.id == id));
            self.inquiries.clear();
            self.group_refresh = None;
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
        if target.is_nil() && matches!(message, Message::Request { .. } | Message::Details { .. }) {
            return false;
        }
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
                    details: portal.destination.clone().map(|destination| Details {
                        destination,
                        access: Access::Everyone,
                    }),
                    portal,
                });
            }
            Message::Close { id, .. } => {
                self.remember(owner, id, unix);
                self.entries.retain(|entry| entry.owner != owner || entry.portal.id != id);
                self.fetches.remove(&(owner, id));
                self.out
                    .retain(|(target, message)| !(*target == owner && matches!(message, Message::Request { id: old, .. } if *old == id)));
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
            Message::Request { id, nonce, .. } => {
                if !self
                    .own()
                    .is_some_and(|entry| entry.portal.id == id && entry.portal.live(unix) && entry.portal.destination.is_none())
                {
                    return;
                }
                if self
                    .inquiries
                    .get(&owner)
                    .is_some_and(|inquiry| now.saturating_duration_since(inquiry.received) < Duration::from_secs(1))
                {
                    return;
                }
                if self.inquiries.len() >= LIMIT * 4
                    && let Some(oldest) = self
                        .inquiries
                        .iter()
                        .min_by_key(|(_, inquiry)| inquiry.received)
                        .map(|(peer, _)| *peer)
                {
                    self.inquiries.remove(&oldest);
                }
                self.inquiries.insert(
                    owner,
                    Inquiry {
                        id,
                        nonce,
                        received: now,
                        replied: false,
                    },
                );
            }
            Message::Details { id, nonce, details, .. } => {
                if !self
                    .fetches
                    .get(&(owner, id))
                    .is_some_and(|fetch| fetch.nonce == nonce && fetch.attempts > 0)
                {
                    return;
                }
                if let Some(entry) = self
                    .entries
                    .iter_mut()
                    .find(|entry| entry.owner == owner && entry.portal.id == id && entry.portal.live(unix))
                    && entry.details.is_none()
                {
                    entry.details = Some(details);
                    self.fetches.remove(&(owner, id));
                    self.out.retain(|(target, message)| {
                        !(*target == owner && matches!(message, Message::Request { id: old, .. } if *old == id))
                    });
                }
            }
        }
    }

    fn exchange_details(&mut self, world: &World, now: Instant) {
        if let Some(own) = self.own()
            && let Some(details) = &own.details
            && details.access != Access::Everyone
        {
            let id = own.portal.id;
            let region = own.portal.region;
            let access = details.access;
            let group_ready = self.group_refresh.is_none();
            let authorized = |peer| {
                !world.mutes.is_muted_id(&peer)
                    && match access {
                        Access::Friends => world.social.friends.iter().any(|friend| friend.id == peer),
                        Access::Group(group) => {
                            group_ready
                                && world.groups.groups.iter().any(|membership| membership.id == group)
                                && world
                                    .group_data
                                    .get(&group)
                                    .is_some_and(|data| data.members_complete && data.members.contains_key(&peer))
                        }
                        Access::Everyone => false,
                    }
            };
            // Permissions can change while the bounded send queue drains.
            // Remove unsent grants before selecting this frame's command.
            self.out
                .retain(|(peer, message)| !matches!(message, Message::Details { .. }) || authorized(*peer));
            for (peer, inquiry) in &mut self.inquiries {
                if !authorized(*peer) {
                    inquiry.replied = false;
                }
            }
            let replies: Vec<_> = self
                .inquiries
                .iter()
                .filter(|(peer, inquiry)| inquiry.id == id && !inquiry.replied && authorized(**peer))
                .map(|(peer, inquiry)| (*peer, inquiry.nonce))
                .collect();
            for (peer, nonce) in replies {
                let Some(details) = self.own().and_then(|entry| entry.details.clone()) else {
                    continue;
                };
                if self.queue(
                    peer,
                    Message::Details {
                        region,
                        id,
                        nonce,
                        details,
                    },
                ) && let Some(inquiry) = self.inquiries.get_mut(&peer)
                {
                    inquiry.replied = true;
                }
            }
        }
        // A logical request has at most one retry. Its nonce remains valid
        // until portal expiry, including a delayed group-members response.
        let requests: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.owner != self.agent && entry.details.is_none())
            .map(|entry| (entry.owner, entry.portal.id, entry.portal.region))
            .collect();
        for (owner, id, region) in requests {
            let fetch = self.fetches.entry((owner, id)).or_insert_with(|| Fetch {
                nonce: Uuid::new_v4(),
                attempts: 0,
                queued: now,
            });
            if fetch.attempts < 2 && (fetch.attempts == 0 || now.saturating_duration_since(fetch.queued) >= Duration::from_secs(2)) {
                let nonce = fetch.nonce;
                if self.queue(owner, Message::Request { region, id, nonce })
                    && let Some(fetch) = self.fetches.get_mut(&(owner, id))
                {
                    fetch.attempts += 1;
                    fetch.queued = now;
                }
            }
        }
    }

    pub fn update(&mut self, world: &World, intent: Vec3, now: Instant) -> Option<Portal> {
        // Authorization is refreshed immediately before a command can leave,
        // at the transport cadence rather than every rendered frame.
        let due = self
            .last_exchange
            .is_none_or(|at| now.saturating_duration_since(at) >= SEND_INTERVAL);
        let sending_grant = self.pending.len() < 32
            && self.last_send.is_none_or(|at| now.saturating_duration_since(at) >= SEND_INTERVAL)
            && self.out.iter().any(|(_, message)| matches!(message, Message::Details { .. }));
        if due || sending_grant {
            self.exchange_details(world, now);
            self.last_exchange = Some(now);
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, delivery)| now.saturating_duration_since(delivery.sent) >= Duration::from_secs(8))
            .map(|(request, _)| *request)
            .collect();
        for request in expired {
            self.reply(request, &Err(String::new()), now);
        }
        let mut crossed = None;
        for entry in &mut self.entries {
            let travel = world.agent.position - entry.gate.previous;
            let can_enter = intent.is_finite() && travel.dot(intent) > 1e-5 && entry.allowed(world);
            if entry.gate.sample(&entry.portal, world.agent.position, can_enter, now) && crossed.is_none() {
                crossed = entry.resolved();
            }
        }
        crossed
    }

    /// Drain after `update`, which rechecks queued grants against current rights.
    pub fn command(&mut self, endpoint: Option<&Endpoint>, now: Instant) -> Option<NetCommand> {
        let endpoint = endpoint?;
        if !self.discovered
            && let Some(region) = self.region
        {
            self.queue(Uuid::nil(), Message::Discover { region });
            self.discovered = true;
        }
        if self.pending.len() >= 32 || self.last_send.is_some_and(|at| now.saturating_duration_since(at) < SEND_INTERVAL) {
            return None;
        }
        let retry = self
            .retries
            .iter()
            .position(|retry| retry.due <= now)
            .and_then(|at| self.retries.remove(at));
        let (target, message, attempt) = if let Some(retry) = retry {
            (Uuid::nil(), retry.action, retry.attempt)
        } else {
            let (target, message) = self.out.pop_front()?;
            (target, message, 1)
        };
        self.last_send = Some(now);
        let request = Uuid::new_v4();
        let action = if target.is_nil() {
            match &message {
                Message::Open(_) | Message::Close { .. } => Some(message.clone()),
                _ => None,
            }
        } else {
            None
        };
        self.pending.insert(
            request,
            Delivery {
                sent: now,
                action,
                attempt,
            },
        );
        Some(NetCommand::BridgeQuery {
            request,
            endpoint: endpoint.clone(),
            query: Query::Portal { target, message },
        })
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.pending.contains_key(&request)
    }

    pub fn reply(&mut self, request: Uuid, result: &Result<Reply, String>, now: Instant) {
        let Some(delivery) = self.pending.remove(&request) else { return };
        if !matches!(result, Ok(Reply::Portal))
            && let Some(action) = delivery.action
        {
            // A timed-out POST may already have been relayed. Replay the same
            // immutable announcement/close, never a new portal or private grant.
            let current = match &action {
                Message::Open(portal) => self
                    .own()
                    .is_some_and(|entry| entry.portal.id == portal.id && entry.expires_at > now),
                Message::Close { id, .. } => self.ended.contains_key(&(self.agent, *id)),
                _ => false,
            };
            if !current {
                return;
            }
            if delivery.attempt < 3 && self.retries.len() < LIMIT * 2 {
                self.retries.push_back(Retry {
                    due: now + Duration::from_secs(u64::from(delivery.attempt)),
                    action,
                    attempt: delivery.attempt + 1,
                });
                return;
            }
            if let Message::Open(portal) = action {
                self.entries
                    .retain(|entry| entry.owner != self.agent || entry.portal.id != portal.id);
            }
            self.failed = true;
        }
    }
}

#[cfg(test)]
mod privacy_tests;
#[cfg(test)]
mod tests;
