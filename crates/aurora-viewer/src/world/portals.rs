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

pub const WIDTH: f32 = aurora_render::PORTAL_HALF_WIDTH;
pub const HEIGHT: f32 = aurora_render::PORTAL_HALF_HEIGHT;
const LIMIT: usize = 64;
const CLEARANCE: f32 = 1.0;
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
    relays: Vec<Uuid>,
    shared: bool,
    created: bool,
    return_pass: bool,
}

impl Entry {
    pub fn label_owner(&self) -> Uuid {
        // A relay's creator field is a routing hint, not authenticated identity.
        // Name the original only when seen directly or retained by our journey.
        if self.return_pass || self.created || self.relays.contains(&self.owner) {
            self.owner
        } else {
            self.relays.first().copied().unwrap_or(self.owner)
        }
    }

    pub fn allowed(&self, world: &World) -> bool {
        self.details.as_ref().is_some_and(|details| {
            !world.mutes.is_muted_id(&self.owner) && (self.return_pass || allowed(world, self.owner, details.access))
        })
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
    last_sample: Instant,
    clearance: f32,
    ready_at: Instant,
}

impl Gate {
    fn new(center: Vec3, position: Vec3, now: Instant) -> Self {
        Self {
            previous: position,
            armed: position.distance(center) >= CLEARANCE,
            last_sample: now,
            clearance: CLEARANCE,
            ready_at: now,
        }
    }

    fn observer(center: Vec3, position: Vec3, now: Instant) -> Self {
        Self {
            armed: position.distance(center) > 3.0,
            clearance: 3.5,
            ready_at: now + Duration::from_millis(600),
            ..Self::new(center, position, now)
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
            // Reject this segment, but do not make an already distant observer
            // walk farther away after a frame stall or a server correction.
            self.armed = position.is_finite() && position.distance(center) >= self.clearance;
            return false;
        }
        if !self.armed {
            if position.distance(center) >= self.clearance {
                self.armed = true;
            }
            return false;
        }
        if now < self.ready_at {
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
    inquiries: HashMap<(Uuid, Uuid), Inquiry>,
    group_refresh: VecDeque<Uuid>,
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
                .any(|e| e.portal.id == *id && e.details.is_none() && e.relays.contains(owner))
        });
        self.inquiries
            .retain(|_, inquiry| inquiry.received.elapsed() < Duration::from_secs(30));
        self.out.retain(|(target, message)| match message {
            Message::Open(portal) => portal.live(now),
            Message::Details { id, .. } => self.entries.iter().any(|entry| entry.shared && entry.portal.id == *id),
            Message::Request { id, .. } => self
                .entries
                .iter()
                .any(|entry| entry.portal.id == *id && entry.details.is_none() && entry.relays.contains(target)),
            _ => true,
        });
        self.retries.retain(|retry| match &retry.action {
            Message::Open(portal) => self.entries.iter().any(|e| e.shared && e.portal.id == portal.id) && portal.live(now),
            Message::Close { id, .. } => self.ended.contains_key(&(self.agent, *id)),
            _ => false,
        });
    }

    pub fn own(&self) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.created)
            .or_else(|| self.entries.iter().find(|entry| entry.shared && entry.owner == self.agent))
    }

    /// Photo isolation leaves world portal surfaces and their labels out.
    /// The invitation, crossing guards and posed body cuts remain live.
    pub fn visible<'a>(&'a self, world: &'a World) -> impl Iterator<Item = &'a Entry> {
        let shown = !world.photo_isolation.applied();
        self.entries.iter().filter(move |_| shown)
    }

    /// One fresh server-backed member list per group portal, shared by all
    /// requesters. Never authorize using a claim made by the requesting viewer.
    pub fn take_group_refresh(&mut self) -> Option<Uuid> {
        self.group_refresh.pop_front()
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
        if let Access::Group(id) = portal.access
            && !self.group_refresh.contains(&id)
        {
            self.group_refresh.push_back(id);
        }
        let announcement = portal.announcement();
        self.queue(Uuid::nil(), Message::Open(announcement.clone()));
        self.entries.push(Entry {
            owner: self.agent,
            gate: Gate::new(Vec3::from(portal.center), position, now),
            expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
            details: Some(portal.details()),
            portal: announcement,
            relays: Vec::new(),
            shared: true,
            created: true,
            return_pass: false,
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
            self.inquiries.retain(|(_, portal), _| *portal != id);
            self.remember(self.agent, id, unix);
            self.entries.retain(|entry| entry.owner != self.agent || entry.portal.id != id);
            if !self.queue(Uuid::nil(), Message::Close { region, id }) {
                self.failed = true;
            }
        }
    }

    /// A crossing grants this traveler a return, without making their friends
    /// friends of the original creator. Only the creator can grant Friends.
    pub fn share_return(&mut self, portal: Portal, creator: Uuid, position: Vec3, now: Instant, unix: u64) -> bool {
        let mut announcement = portal.announcement();
        announcement.creator = Some(creator);
        if creator.is_nil()
            || Some(portal.region) != self.region
            || !portal.live(unix)
            || Message::Open(announcement.clone()).encode().is_none()
            || (portal.access != Access::Everyone
                && Message::Details {
                    region: portal.region,
                    id: portal.id,
                    nonce: portal.id,
                    details: portal.details(),
                }
                .encode()
                .is_none())
        {
            return false;
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.owner == creator && e.portal.id == portal.id) {
            // Keep the first aperture and gate; arrivals never move or rearm it.
            entry.details = Some(portal.details());
            entry.return_pass = true;
            entry.gate.clearance = CLEARANCE;
            entry.gate.ready_at = now;
            entry.shared = true;
            entry.portal.creator = Some(creator);
            announcement = entry.portal.clone();
        } else {
            if self.entries.len() >= LIMIT {
                return false;
            }
            self.entries.push(Entry {
                owner: creator,
                portal: announcement.clone(),
                details: Some(portal.details()),
                gate: Gate::new(Vec3::from(portal.center), position, now),
                expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
                relays: Vec::new(),
                shared: true,
                created: false,
                return_pass: true,
            });
        }
        if let Access::Group(group) = portal.access
            && !self.group_refresh.contains(&group)
        {
            self.group_refresh.push_back(group);
        }
        self.queue(Uuid::nil(), Message::Open(announcement))
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
                let creator = portal.creator.unwrap_or(owner);
                if !portal.live(unix)
                    || world.mutes.is_muted_id(&creator)
                    || world
                        .main()
                        .is_some_and(|region| !fits_region(Vec3::from(portal.center), [region.heightmap.size_x, region.heightmap.size_y]))
                    || self.ended.contains_key(&(owner, portal.id))
                {
                    return;
                }
                if let Some(entry) = self.entries.iter_mut().find(|e| e.owner == creator && e.portal.id == portal.id) {
                    // Multiple travelers may relay one immutable return. Its
                    // geometry, permission gate and lifetime belong to the link.
                    if entry.portal.center == portal.center
                        && entry.portal.yaw == portal.yaw
                        && entry.portal.expires == portal.expires
                        && entry.portal.destination == portal.destination
                        && !entry.relays.contains(&owner)
                        && entry.relays.len() < LIMIT
                    {
                        entry.relays.push(owner);
                    }
                    return;
                }
                if self.entries.len() >= LIMIT
                    || (portal.creator.is_none() && self.entries.iter().any(|e| e.owner == owner && e.portal.creator.is_none()))
                {
                    return;
                }
                self.entries.push(Entry {
                    owner: creator,
                    gate: Gate::observer(Vec3::from(portal.center), world.agent.position, now),
                    expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
                    details: portal.destination.clone().map(|destination| Details {
                        destination,
                        access: Access::Everyone,
                    }),
                    portal,
                    relays: vec![owner],
                    shared: false,
                    created: false,
                    return_pass: false,
                });
            }
            Message::Close { id, .. } => {
                self.remember(owner, id, unix);
                for entry in &mut self.entries {
                    if entry.portal.id == id {
                        entry.relays.retain(|relay| *relay != owner);
                    }
                }
                self.entries
                    .retain(|entry| entry.portal.id != id || entry.shared || !entry.relays.is_empty());
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
                let shared: Vec<_> = self.entries.iter().filter(|e| e.shared).map(|e| e.portal.clone()).collect();
                for portal in shared {
                    self.queue(owner, Message::Open(portal));
                }
            }
            Message::Request { id, nonce, .. } => {
                if !self
                    .entries
                    .iter()
                    .find(|entry| entry.shared && entry.portal.id == id)
                    .is_some_and(|entry| entry.portal.id == id && entry.portal.live(unix) && entry.portal.destination.is_none())
                {
                    return;
                }
                if self
                    .inquiries
                    .get(&(owner, id))
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
                    (owner, id),
                    Inquiry {
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
                    .find(|entry| entry.portal.id == id && entry.portal.live(unix) && entry.relays.contains(&owner))
                    && entry.details.is_none()
                {
                    if details.access == Access::Friends && owner != entry.owner && world.agent_id != entry.owner {
                        // Only the original creator can prove friendship. A
                        // return relay cannot grant access on their behalf.
                        return;
                    }
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
        let authorized = |entry: &Entry, peer: Uuid| {
            entry.shared
                && !world.mutes.is_muted_id(&peer)
                && entry.details.as_ref().is_some_and(|details| {
                    peer == entry.owner
                        || match details.access {
                            // A return relay must never substitute its own friends.
                            Access::Friends => entry.owner == self.agent && world.social.friends.iter().any(|f| f.id == peer),
                            Access::Group(group) => {
                                !self.group_refresh.contains(&group)
                                    && world.groups.groups.iter().any(|m| m.id == group)
                                    && world
                                        .group_data
                                        .get(&group)
                                        .is_some_and(|data| data.members_complete && data.members.contains_key(&peer))
                            }
                            Access::Everyone => false,
                        }
                })
        };
        self.out.retain(|(peer, message)| match message {
            Message::Details { id, .. } => self.entries.iter().any(|e| e.portal.id == *id && authorized(e, *peer)),
            _ => true,
        });
        for ((peer, id), inquiry) in &mut self.inquiries {
            if !self.entries.iter().any(|e| e.portal.id == *id && authorized(e, *peer)) {
                inquiry.replied = false;
            }
        }
        let replies: Vec<_> = self
            .inquiries
            .iter()
            .filter(|(_, inquiry)| !inquiry.replied)
            .filter_map(|((peer, id), inquiry)| {
                let entry = self.entries.iter().find(|e| e.portal.id == *id && authorized(e, *peer))?;
                let details = entry.details.clone()?;
                Some((
                    *peer,
                    Message::Details {
                        region: entry.portal.region,
                        id: *id,
                        nonce: inquiry.nonce,
                        details,
                    },
                ))
            })
            .collect();
        for (peer, message) in replies {
            let Message::Details { id, .. } = &message else { continue };
            let id = *id;
            if self.queue(peer, message)
                && let Some(inquiry) = self.inquiries.get_mut(&(peer, id))
            {
                inquiry.replied = true;
            }
        }
        // A logical request has at most one retry. Its nonce remains valid
        // until portal expiry, including a delayed group-members response.
        let requests: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.details.is_none())
            .flat_map(|entry| entry.relays.iter().map(|relay| (*relay, entry.portal.id, entry.portal.region)))
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

    pub fn update(&mut self, world: &World, intent: Vec3, now: Instant) -> Option<(Uuid, Portal)> {
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
                crossed = entry.resolved().map(|portal| (entry.owner, portal));
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
                    .entries
                    .iter()
                    .any(|entry| entry.shared && entry.portal.id == portal.id && entry.expires_at > now),
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
                    .retain(|entry| !entry.shared || entry.portal.id != portal.id || entry.return_pass);
            }
            self.failed = true;
        }
    }
}

#[cfg(test)]
mod privacy_tests;
#[cfg(test)]
mod tests;
