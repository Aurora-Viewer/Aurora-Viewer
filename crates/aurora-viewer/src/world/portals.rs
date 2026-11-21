//! Walk-through portals of this viewer alone: our own portal and the returns
//! of our own journeys, drawn and crossed for us only. Nothing is sent to the
//! region or read from it. Portals were once announced to other Aurora
//! viewers over region chat; that was taken out under Second Life's shared
//! experience policy (AGENTS.md, "No data of our own in shared places").
//! A portal is immutable for its lifetime: nothing moves it or resets its
//! proximity gate.
use super::World;
use super::land::BuildRight;
use glam::Vec3;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const WIDTH: f32 = aurora_render::PORTAL_HALF_WIDTH;
pub const HEIGHT: f32 = aurora_render::PORTAL_HALF_HEIGHT;
/// Seconds a portal stays open.
pub const LIFETIME: u64 = 120;
/// Our portal and the returns of a few journeys.
const LIMIT: usize = 8;
const CLEARANCE: f32 = 1.0;

/// Why our portal was not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Outside the region, expired, a destination that is not a place, or
    /// too many portals.
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

/// A portal: where it stands in the current region, and where it leads.
#[derive(Clone, Debug, PartialEq)]
pub struct Portal {
    pub id: Uuid,
    pub region: u64,
    pub center: [f32; 3],
    pub yaw: f32,
    /// The destination region's name and the arrival in it.
    pub destination: String,
    pub position: [f32; 3],
    pub expires: u64,
}

impl Portal {
    fn valid(&self) -> bool {
        let inside = |v: [f32; 3]| {
            v.iter().all(|v| v.is_finite())
                && (0.0..=8192.0).contains(&v[0])
                && (0.0..=8192.0).contains(&v[1])
                && (0.0..=4096.0).contains(&v[2])
        };
        !self.id.is_nil()
            && self.region != 0
            && inside(self.center)
            && inside(self.position)
            && self.yaw.is_finite()
            && self.yaw.abs() <= std::f32::consts::TAU
            && !self.destination.trim().is_empty()
            && self.destination.len() <= 128
            && !self.destination.chars().any(|c| c.is_control() || "/\\:|".contains(c))
    }

    pub fn live(&self, now: u64) -> bool {
        self.valid() && self.expires > now && self.expires - now <= LIFETIME
    }
}

pub struct Entry {
    pub portal: Portal,
    gate: Gate,
    expires_at: Instant,
    /// Opened here; the others are returns of our journeys.
    created: bool,
}

impl Entry {
    fn new(portal: Portal, created: bool, position: Vec3, now: Instant, unix: u64) -> Self {
        Self {
            gate: Gate::new(Vec3::from(portal.center), position, now),
            expires_at: now + Duration::from_secs(portal.expires.saturating_sub(unix).min(LIFETIME)),
            portal,
            created,
        }
    }

    pub fn waiting_for_clearance(&self) -> bool {
        !self.gate.armed
    }
}

/// Entry needs a voluntary crossing of the plane, after standing at least
/// [`CLEARANCE`] away: a portal opened on us, standing inside it, being
/// pushed or corrected, and the arrival of a teleport never cross it.
struct Gate {
    previous: Vec3,
    armed: bool,
    last_sample: Instant,
}

impl Gate {
    fn new(center: Vec3, position: Vec3, now: Instant) -> Self {
        Self {
            previous: position,
            armed: position.distance(center) >= CLEARANCE,
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
            // Reject this segment, but do not make an already distant avatar
            // walk farther away after a frame stall or a server correction.
            self.armed = position.is_finite() && position.distance(center) >= CLEARANCE;
            return false;
        }
        if !self.armed {
            self.armed = position.distance(center) >= CLEARANCE;
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
    pub entries: Vec<Entry>,
}

impl State {
    /// A new session or region starts empty; expired portals go.
    pub fn sync(&mut self, world: &World, unix: u64) {
        if self.agent != world.agent_id || self.region != world.main_region {
            *self = Self {
                agent: world.agent_id,
                region: world.main_region,
                ..Default::default()
            };
        }
        let now = Instant::now();
        self.entries.retain(|entry| entry.portal.live(unix) && now < entry.expires_at);
    }

    /// The portal opened here, returns aside.
    pub fn own(&self) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.created)
    }

    /// Photo isolation leaves world portal surfaces and their labels out.
    /// The portals, their crossing guards and the posed body cuts remain.
    pub fn visible<'a>(&'a self, world: &'a World) -> impl Iterator<Item = &'a Entry> {
        let shown = !world.photo_isolation.applied();
        self.entries.iter().filter(move |_| shown)
    }

    /// Open our portal, in place of the one already open. Placing one needs
    /// the right to build (rez) where it stands, the land test of
    /// [`crate::world::land::agent_can_build`]: an unknown parcel refuses
    /// too. A refusal keeps the portal already open.
    pub fn create(&mut self, portal: Portal, build: BuildRight, position: Vec3, now: Instant, unix: u64) -> Result<(), Refusal> {
        match build {
            BuildRight::Allowed => {}
            BuildRight::Denied => return Err(Refusal::NoBuild),
            BuildRight::Unknown => return Err(Refusal::LandUnknown),
        }
        if Some(portal.region) != self.region || !portal.live(unix) {
            return Err(Refusal::Invalid);
        }
        self.close();
        if self.entries.len() >= LIMIT {
            return Err(Refusal::Invalid);
        }
        self.entries.push(Entry::new(portal, true, position, now, unix));
        Ok(())
    }

    pub fn close(&mut self) {
        self.entries.retain(|entry| !entry.created);
    }

    /// The return of our journey, crossable at once from the 1 m clearance.
    /// One already here (the portal we came through, in the same region) is
    /// kept as it is: arrivals never move it or rearm its gate.
    pub fn add_return(&mut self, portal: Portal, position: Vec3, now: Instant, unix: u64) -> bool {
        if Some(portal.region) != self.region || !portal.live(unix) {
            return false;
        }
        if self.entries.iter().any(|entry| entry.portal.id == portal.id) {
            return true;
        }
        if self.entries.len() >= LIMIT {
            return false;
        }
        self.entries.push(Entry::new(portal, false, position, now, unix));
        true
    }

    /// The portal our voluntary movement (`intent`) just crossed.
    pub fn update(&mut self, world: &World, intent: Vec3, now: Instant) -> Option<Portal> {
        if world.teleporting {
            // No movement counts during transit or on the first arrival sample.
            for entry in &mut self.entries {
                entry.gate.previous = world.agent.position;
                entry.gate.armed = false;
                entry.gate.last_sample = now;
            }
            return None;
        }
        let mut crossed = None;
        for entry in &mut self.entries {
            let travel = world.agent.position - entry.gate.previous;
            let can_enter = intent.is_finite() && travel.dot(intent) > 1e-5;
            if entry.gate.sample(&entry.portal, world.agent.position, can_enter, now) && crossed.is_none() {
                crossed = Some(entry.portal.clone());
            }
        }
        crossed
    }
}

#[cfg(test)]
mod tests;
