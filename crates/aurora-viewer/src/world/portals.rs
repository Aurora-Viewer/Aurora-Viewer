//! Region-local, voluntary portal crossings. Invitations are immutable for
//! their lifetime: replaying one never moves it or resets its proximity gate.
use super::World;
use super::land::BuildRight;
use aurora_net::{
    NetCommand,
    bridge::{
        Endpoint,
        portal::{Access, Announcement, CLOCK_SKEW, Details, LIFETIME, Message, Portal},
        query::{Query, Reply},
    },
};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
pub mod arrival;
mod returns;

pub const WIDTH: f32 = aurora_render::PORTAL_HALF_WIDTH;
pub const HEIGHT: f32 = aurora_render::PORTAL_HALF_HEIGHT;
const LIMIT: usize = 64;
/// Entries one announcing avatar can hold: its own portal, plus a few returns
/// it shares after travelling. One relay cannot fill the table.
const PER_RELAY: usize = 4;
/// Entries naming one creator, whoever announces them.
const PER_CREATOR: usize = 4;
const CLEARANCE: f32 = 1.0;
const INITIAL_CLEARANCE: f32 = 0.25;
const SEND_INTERVAL: Duration = Duration::from_millis(250);

/// Why our portal was not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Outside the region, expired, too long to share, or too many portals.
    Invalid,
    /// We may not build where it would stand.
    NoBuild,
    /// The parcel there is not known yet; it is asked for.
    LandUnknown,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::Invalid => aurora_i18n::tr!("portal-invalid"),
            Self::NoBuild => aurora_i18n::tr!("portal-no-build"),
            Self::LandUnknown => aurora_i18n::tr!("portal-land-checking"),
        }
    }
}

pub fn fits_region(center: Vec3, size: [u32; 2]) -> bool {
    center.is_finite() && (WIDTH..=size[0] as f32 - WIDTH).contains(&center.x) && (WIDTH..=size[1] as f32 - WIDTH).contains(&center.y)
}

pub fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Preferences › Aurora › Behavior: whose portals are shown. Our own portals
/// and the returns of our own journeys are always shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sources {
    Nobody,
    #[default]
    Friends,
    /// Friends, and residents the grid lists in one of our groups.
    FriendsAndGroups,
    Everyone,
}

fn friend(world: &World, id: Uuid) -> bool {
    world.social.friends.iter().any(|friend| friend.id == id)
}

/// A server-backed shared group: a loaded member list of one of our groups,
/// or the groups of the resident's profile. Never a claim from a portal.
fn shares_group(world: &World, id: Uuid) -> bool {
    world.groups.groups.iter().any(|group| {
        world.group_data.get(&group.id).is_some_and(|data| data.members.contains_key(&id))
            || world
                .profiles
                .get(&id)
                .is_some_and(|profile| profile.groups.iter().any(|g| g.id == group.id))
    })
}

/// Whether the setting shows portals this resident announces or created.
pub fn source_shown(world: &World, id: Uuid) -> bool {
    !world.mutes.is_muted_id(&id)
        && (id == world.agent_id
            || match world.portal_sources {
                Sources::Nobody => false,
                Sources::Friends => friend(world, id),
                Sources::FriendsAndGroups => friend(world, id) || shares_group(world, id),
                Sources::Everyone => true,
            })
}

/// Whether an announcement is kept at all. With groups, a resident's portal
/// is kept hidden until their profile says whether they share a group.
fn source_kept(world: &World, id: Uuid) -> bool {
    source_shown(world, id) || (world.portal_sources == Sources::FriendsAndGroups && !world.mutes.is_muted_id(&id))
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

impl World {
    pub(super) fn portal_relocation(&self, region: u64, index: usize, position: Vec3) -> bool {
        let Some(object) = self.objects.get(index) else { return false };
        if !object.is_avatar() || object.full_id == self.agent_id || object.parent_id != 0 || object.key.region != region {
            return false;
        }
        let now = self.received.unwrap_or_else(Instant::now);
        let clock = self.objects.report_clock(region, self.received);
        self.portals
            .observed_relocation(self, region, object.reported_pose(clock).0, position, now)
    }
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
    /// A return kept ours only because its land was not known yet: it is
    /// announced once that land is known and open to us.
    awaiting_land: bool,
}

impl Entry {
    fn shared_announcement(&self) -> Announcement {
        let mut announcement = self.portal.clone();
        if !self.created {
            announcement.creator = Some(self.owner);
        }
        announcement
    }

    pub fn label_owner(&self) -> Uuid {
        // A relayed creator is a routing hint, not proof of identity.
        if self.return_pass || self.created || self.relays.contains(&self.owner) {
            self.owner
        } else {
            self.relays.first().copied().unwrap_or(self.owner)
        }
    }

    /// Ours: created here, or a return of our own journey.
    fn own(&self) -> bool {
        self.created || self.return_pass
    }

    /// Shown by the setting: the announcing relays are checked as well as
    /// the creator they name, a creator field being only a claim.
    pub fn shown(&self, world: &World) -> bool {
        if self.own() {
            return !world.mutes.is_muted_id(&self.owner);
        }
        source_shown(world, self.owner) && self.relays.iter().any(|relay| source_shown(world, *relay))
    }

    fn kept(&self, world: &World) -> bool {
        self.own() || (source_kept(world, self.owner) && self.relays.iter().any(|relay| source_kept(world, *relay)))
    }

    /// Someone who is not our friend opened or relays it: the first crossing
    /// asks first. Ours, or a friend's relayed by a friend, does not.
    pub fn opened_by_non_friend(&self, world: &World) -> bool {
        !self.own()
            && !((self.owner == world.agent_id || friend(world, self.owner))
                && self
                    .relays
                    .iter()
                    .any(|relay| friend(world, *relay) && !world.mutes.is_muted_id(relay)))
    }

    pub fn allowed(&self, world: &World) -> bool {
        self.shown(world)
            && self.details.as_ref().is_some_and(|details| {
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

    pub fn ready_to_cross(&self, now: Instant) -> bool {
        self.gate.armed && now >= self.gate.ready_at
    }

    pub fn reveals_arrival(&self) -> bool {
        self.return_pass && self.gate.arriving
    }
}

struct Gate {
    previous: Vec3,
    armed: bool,
    last_sample: Instant,
    clearance: f32,
    ready_at: Instant,
    arriving: bool,
}

impl Gate {
    fn new(center: Vec3, position: Vec3, now: Instant) -> Self {
        Self {
            previous: position,
            armed: position.distance(center) >= INITIAL_CLEARANCE,
            last_sample: now,
            clearance: INITIAL_CLEARANCE,
            ready_at: now,
            arriving: false,
        }
    }

    fn observer(center: Vec3, position: Vec3, now: Instant) -> Self {
        Self {
            ready_at: now + Duration::from_millis(600),
            ..Self::new(center, position, now)
        }
    }

    fn after_teleport(center: Vec3, position: Vec3, now: Instant, emerge: bool) -> Self {
        let cleared = position.distance(center) >= CLEARANCE;
        Self {
            armed: cleared,
            clearance: CLEARANCE,
            arriving: emerge && !cleared,
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
            if !position.is_finite() || !previous.is_finite() || previous.distance(position) > 3.0 {
                self.clearance = CLEARANCE;
            }
            // Reject this segment, but do not make an already distant observer
            // walk farther away after a frame stall or a server correction.
            self.armed = position.is_finite() && position.distance(center) >= self.clearance;
            self.arriving = false;
            if self.armed {
                self.clearance = CLEARANCE;
            }
            return false;
        }
        if !self.armed {
            if position.distance(center) >= self.clearance {
                self.armed = true;
                self.clearance = CLEARANCE;
                self.arriving = false;
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
            self.clearance = CLEARANCE;
            self.arriving = false;
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
    /// Only the source of an accepted portal journey may keep us dissolved
    /// during transit; an unrelated teleport status is not that acceptance.
    pub departure: Option<Uuid>,
    pub arrivals: HashMap<Uuid, arrival::Effect>,
    /// Short-lived events awaiting server-backed group information. The
    /// renderer only receives them after both source filters pass.
    waiting_arrivals: HashMap<Uuid, aurora_net::bridge::portal::Arrival>,
    pub reduced_motion: bool,
    pub awaiting_floor: bool,
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
    /// Profiles to ask for, to learn whether a resident shares a group.
    profile_wants: VecDeque<Uuid>,
    /// Portals of non-friends whose crossing was confirmed (owner, id).
    confirmed: HashSet<(Uuid, Uuid)>,
    sources: Sources,
    pub failed: bool,
}

impl State {
    /// A simulator placement at a known doorway is discontinuous, even within
    /// LLDrawable::updateXform's ordinary 10 m damping range. This only affects
    /// presentation of an authoritative report, never an avatar's position.
    fn observed_relocation(&self, world: &World, region: u64, from: Vec3, to: Vec3, now: Instant) -> bool {
        if !from.is_finite() || !to.is_finite() || from.distance_squared(to) < 0.5 * 0.5 {
            return false;
        }
        self.shown(world).any(|entry| {
            let portal = &entry.portal;
            if portal.region != region || now >= entry.expires_at {
                return false;
            }
            let center = Vec3::from(portal.center);
            let normal = Vec3::new(portal.yaw.cos(), portal.yaw.sin(), 0.0);
            let tangent = Vec3::new(-normal.y, normal.x, 0.0);
            let delta = from - center;
            // Compare against extrapolated simulator motion, without its
            // visual damping: ordinary walking/running has no placement gap.
            // A source is sufficient, including for observers who cannot see
            // a private destination or have not received its arrival effect.
            delta.dot(normal).abs() <= 1.0 && (delta.dot(tangent) / WIDTH).powi(2) + (delta.z / HEIGHT).powi(2) <= 1.0
        })
    }

    pub fn sync(&mut self, world: &World, endpoint: Option<Uuid>, now: u64) {
        if self.agent != world.agent_id || self.region != world.main_region {
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
        if self.sources != world.portal_sources {
            // Another choice asks the region's bridges to announce again:
            // portals left out before can now be shown.
            if world.portal_sources != Sources::Nobody {
                self.discovered = false;
            }
            self.sources = world.portal_sources;
        }
        // A muted relay's announcements go; an entry nobody else relays too.
        for entry in &mut self.entries {
            entry.relays.retain(|relay| !world.mutes.is_muted_id(relay));
        }
        self.entries.retain(|entry| {
            entry.portal.live(now)
                && Instant::now() < entry.expires_at
                && !world.mutes.is_muted_id(&entry.owner)
                && (entry.own() || !entry.relays.is_empty())
                && entry.kept(world)
        });
        self.confirmed
            .retain(|(owner, id)| self.entries.iter().any(|e| e.owner == *owner && e.portal.id == *id));
        self.profile_wants.retain(|id| !source_shown(world, *id));
        self.sync_arrivals(world, Instant::now());
        self.ended.retain(|_, until| *until > now);
        self.discoveries.retain(|_, at| at.elapsed() < Duration::from_secs(30));
        // A queued private request can outlive a preference, friendship or
        // aperture change. Retire its nonce before the bridge can send it.
        let mut fetches = std::mem::take(&mut self.fetches);
        fetches.retain(|(relay, id), _| self.awaiting_details_from(world, *relay, *id).is_some());
        self.fetches = fetches;
        self.inquiries
            .retain(|_, inquiry| inquiry.received.elapsed() < Duration::from_secs(30));
        let own_arrival = self.arrivals.get(&self.agent).map(|effect| effect.event.id);
        self.out.retain(|(target, message)| match message {
            Message::Arrival(event) => own_arrival == Some(event.id) && event.live(arrival::unix_millis()),
            Message::Open(portal) => portal.live(now),
            Message::Details { id, .. } => self.entries.iter().any(|entry| entry.shared && entry.portal.id == *id),
            Message::Request { id, nonce, .. } => self.fetches.get(&(*target, *id)).is_some_and(|fetch| fetch.nonce == *nonce),
            _ => true,
        });
        self.retries.retain(|retry| match &retry.action {
            Message::Arrival(event) => own_arrival == Some(event.id) && event.live(arrival::unix_millis()),
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
        self.shown(world).filter(move |_| shown)
    }

    /// The entries the setting shows, photo isolation aside (body cuts).
    pub fn shown<'a>(&'a self, world: &'a World) -> impl Iterator<Item = &'a Entry> {
        self.entries.iter().filter(move |entry| self.shown_entry(world, entry))
    }

    /// A profile to request, to learn whether a resident shares a group.
    pub fn take_profile_want(&mut self) -> Option<Uuid> {
        self.profile_wants.pop_front()
    }

    /// A crossing of this portal waits for the user's answer first: opened
    /// or relayed by a non-friend, and not confirmed yet.
    pub fn ask_before_crossing(&self, world: &World, owner: Uuid, id: Uuid) -> bool {
        !self.confirmed.contains(&(owner, id))
            && self
                .entries
                .iter()
                .find(|entry| entry.owner == owner && entry.portal.id == id)
                .is_none_or(|entry| entry.opened_by_non_friend(world))
    }

    /// The portal a question is about, while it can still be crossed.
    pub fn crossable(&self, world: &World, owner: Uuid, id: Uuid) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.owner == owner && entry.portal.id == id && self.shown_entry(world, entry) && entry.allowed(world))
    }

    /// The user said Go: later crossings of this portal ask no more.
    pub fn confirm(&mut self, owner: Uuid, id: Uuid) {
        if self.confirmed.len() < LIMIT * 2 {
            self.confirmed.insert((owner, id));
        }
    }

    /// Entries this relay announces, ours aside.
    fn relayed_by(&self, relay: Uuid) -> usize {
        self.entries
            .iter()
            .filter(|entry| !entry.own() && entry.relays.contains(&relay))
            .count()
    }

    fn want_profile(&mut self, world: &World, id: Uuid) {
        if world.portal_sources == Sources::FriendsAndGroups
            && !source_shown(world, id)
            && world.profiles.get(&id).is_none()
            && !self.profile_wants.contains(&id)
            && self.profile_wants.len() < LIMIT
        {
            self.profile_wants.push_back(id);
        }
    }

    /// One fresh server-backed member list per group portal, shared by all
    /// requesters. Never authorize using a claim made by the requesting viewer.
    pub fn take_group_refresh(&mut self) -> Option<Uuid> {
        self.group_refresh.pop_front()
    }

    /// Open our portal. Placing one needs the right to build (rez) where
    /// it stands, the land test of [`crate::world::land::agent_can_build`]:
    /// an unknown parcel refuses too. A refusal keeps the portal already open.
    pub fn create(&mut self, portal: Portal, build: BuildRight, position: Vec3, now: Instant, unix: u64) -> Result<(), Refusal> {
        match build {
            BuildRight::Allowed => {}
            BuildRight::Denied => return Err(Refusal::NoBuild),
            BuildRight::Unknown => return Err(Refusal::LandUnknown),
        }
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
            return Err(Refusal::Invalid);
        }
        if self.out.len() > LIMIT * 2 - 2 || self.entries.len() >= LIMIT && self.own().is_none() {
            return Err(Refusal::Invalid);
        }
        self.close(unix);
        if self.entries.len() >= LIMIT {
            return Err(Refusal::Invalid);
        }
        let announcement = portal.announcement();
        self.announce(announcement.clone(), portal.access);
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
            awaiting_land: false,
        });
        Ok(())
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

    fn refresh_access_group(&mut self, access: Access) {
        if let Access::Group(group) = access
            && !self.group_refresh.contains(&group)
        {
            self.group_refresh.push_back(group);
        }
    }

    fn announce(&mut self, announcement: Announcement, access: Access) -> bool {
        self.refresh_access_group(access);
        self.queue(Uuid::nil(), Message::Open(announcement))
    }

    /// Our returns waiting for their land to be known: (creator, id, center).
    pub fn awaiting_land(&self) -> Vec<(Uuid, Uuid, Vec3)> {
        self.entries
            .iter()
            .filter(|e| e.awaiting_land)
            .map(|e| (e.owner, e.portal.id, Vec3::from(e.portal.center)))
            .collect()
    }

    /// The land under a waiting return became known: open to us, it is
    /// announced like any return; closed, it stays ours alone for good. A
    /// return gone or expired meanwhile is not brought back.
    pub fn land_known(&mut self, creator: Uuid, id: Uuid, build: BuildRight, unix: u64) {
        if build == BuildRight::Unknown {
            return;
        }
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.awaiting_land && e.owner == creator && e.portal.id == id)
        else {
            return;
        };
        entry.awaiting_land = false;
        if build != BuildRight::Allowed || !entry.portal.live(unix) {
            return;
        }
        entry.shared = true;
        let (announcement, access) = (
            entry.shared_announcement(),
            entry.details.as_ref().map_or(Access::Everyone, |d| d.access),
        );
        if !self.announce(announcement, access) {
            self.failed = true;
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
            Message::Arrival(event) => {
                if world
                    .objects
                    .index_of_uuid(&owner)
                    .and_then(|i| world.objects.get(i))
                    .is_none_or(|o| o.is_avatar())
                {
                    self.receive_arrival(world, owner, event, now);
                }
            }
            Message::Open(portal) => {
                let creator = portal.creator.unwrap_or(owner);
                if !portal.live(unix)
                    || !source_kept(world, owner)
                    || !source_kept(world, creator)
                    || world
                        .main()
                        .is_some_and(|region| !fits_region(Vec3::from(portal.center), [region.heightmap.size_x, region.heightmap.size_y]))
                    || self.ended.contains_key(&(owner, portal.id))
                {
                    return;
                }
                let relayed = self.relayed_by(owner);
                if let Some(entry) = self.entries.iter_mut().find(|e| e.owner == creator && e.portal.id == portal.id) {
                    // Multiple travelers may relay one immutable return. Its
                    // geometry, permission gate and lifetime belong to the link.
                    if entry.portal.center == portal.center
                        && entry.portal.yaw == portal.yaw
                        && entry.portal.expires == portal.expires
                        && entry.portal.destination == portal.destination
                        && !entry.relays.contains(&owner)
                        && entry.relays.len() < LIMIT
                        && (entry.own() || relayed < PER_RELAY)
                    {
                        entry.relays.push(owner);
                    }
                    return;
                }
                // A creator other than the relay exists for travelers' returns
                // (`keep_return`). Such a claim never names us unless it joins
                // a portal of ours (above); it never names the label
                // (`label_owner`), and the relay passes the setting with it.
                if creator == world.agent_id
                    || self.entries.len() >= LIMIT
                    || relayed >= PER_RELAY
                    || self.entries.iter().filter(|e| !e.own() && e.owner == creator).count() >= PER_CREATOR
                    || (portal.creator.is_none() && self.entries.iter().any(|e| e.owner == owner && e.portal.creator.is_none()))
                    || (portal.creator.is_some()
                        && source_shown(world, owner)
                        && source_shown(world, creator)
                        && self.shown(world).any(|e| returns::overlaps(&e.portal, &portal)))
                {
                    return;
                }
                self.want_profile(world, owner);
                self.want_profile(world, creator);
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
                    awaiting_land: false,
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
                    .retain(|entry| entry.portal.id != id || entry.shared || entry.return_pass || !entry.relays.is_empty());
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
                let shared: Vec<_> = self.entries.iter().filter(|e| e.shared).map(Entry::shared_announcement).collect();
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
                if let Some(index) = self.awaiting_details_from(world, owner, id)
                    && self.entries[index].portal.live(unix)
                {
                    let entry = &mut self.entries[index];
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

    fn awaiting_details_from(&self, world: &World, relay: Uuid, id: Uuid) -> Option<usize> {
        if !source_shown(world, relay) {
            return None;
        }
        self.entries.iter().position(|entry| {
            entry.portal.id == id && entry.details.is_none() && entry.relays.contains(&relay) && self.shown_entry(world, entry)
        })
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
        // A hidden portal asks nobody: no request reaches a resident the
        // setting leaves out.
        let requests: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.details.is_none() && self.shown_entry(world, entry))
            .flat_map(|entry| {
                entry
                    .relays
                    .iter()
                    .filter(|relay| source_shown(world, **relay))
                    .map(|relay| (*relay, entry.portal.id, entry.portal.region))
            })
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
        if world.teleporting {
            // The source bridge can still deliver private grants until the
            // actual region change. Keep those exchanges, but consume no
            // movement during transit or on the first arrival sample.
            for entry in &mut self.entries {
                entry.gate.previous = world.agent.position;
                entry.gate.armed = false;
                entry.gate.arriving = false;
                entry.gate.last_sample = now;
            }
            return None;
        }
        let mut crossed = None;
        for index in 0..self.entries.len() {
            if !self.shown_entry(world, &self.entries[index]) {
                continue;
            }
            let entry = &mut self.entries[index];
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
                Message::Open(_) | Message::Close { .. } | Message::Arrival(_) => Some(message.clone()),
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
                Message::Arrival(event) => event.live(arrival::unix_millis()),
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
mod source_tests;
#[cfg(test)]
mod tests;
