//! World map data (LLWorldMap / LLWorldMapMessage): regions known by grid
//! position, map items (hubs, people, land for sale, events), the tracked
//! location (LLTracker), the friend the map follows (LLAvatarTracker) and
//! the parcel overlays used for the mini-map's property lines
//! (LLViewerParcelOverlay).
//!
//! Ported from Firestorm's llworldmap.cpp / llworldmapmessage.cpp /
//! llcallingcard.cpp (originally LGPL 2.1, Linden Research, Inc.).

use aurora_net::{MapBlock, MapItem, NetCommand, RegionHandle, map_item};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LLWorldMap: regions are asked by blocks of 16 x 16 (Firestorm MAP_BLOCK_SIZE).
const MAP_BLOCK_SIZE: u32 = 16;
/// MAP_MAX_SIZE / MAP_BLOCK_SIZE.
const MAP_BLOCK_RES: u32 = 1024;
/// A block is asked again after this long (BLOCK_UPDATE_TIMER).
const BLOCK_UPDATE: Duration = Duration::from_secs(60);
/// Agent counts of a region are refreshed this often (AGENTS_UPDATE_TIMER).
const AGENTS_UPDATE: Duration = Duration::from_secs(60);
/// Grid-wide items (hubs, land for sale, events) are reloaded after this long
/// (REQUEST_ITEMS_TIMER).
const ITEMS_UPDATE: Duration = Duration::from_secs(600);
/// A followed friend is asked again this often (FIND_FREQUENCY: each
/// FindAgent is a database query of the grid).
pub const FIND_AGENT_EVERY: Duration = Duration::from_millis(29_700);

pub const SIM_ACCESS_PG: u8 = 13;
pub const SIM_ACCESS_MATURE: u8 = 21;
pub const SIM_ACCESS_ADULT: u8 = 42;
pub const SIM_ACCESS_DOWN: u8 = 254;
/// Non-existent slot (MapBlockReply with MAP_SIM_RETURN_NULL_SIMS).
pub const SIM_ACCESS_NONE: u8 = 255;

/// REGION_FLAGS_SANDBOX / REGION_FLAGS_ALLOW_DAMAGE (LLSimInfo tooltip).
const REGION_FLAGS_SANDBOX: u32 = 1 << 8;
const REGION_FLAGS_ALLOW_DAMAGE: u32 = 1 << 0;

#[derive(Debug, Clone)]
pub struct SimInfo {
    pub name: String,
    pub access: u8,
    pub flags: u32,
    /// Land-for-sale overlay (MapImageID), not drawn yet.
    #[allow(dead_code)]
    pub image: Uuid,
    /// Size in meters (variable regions).
    pub size_x: u32,
    pub size_y: u32,
}

impl SimInfo {
    pub fn is_down(&self) -> bool {
        self.access == SIM_ACCESS_DOWN
    }

    /// LLSimInfo::getAccessString, Firestorm wording in French.
    pub fn access_label(&self) -> &'static str {
        access_label(self.access)
    }

    /// "Sandbox", "Unsecured" (LLWorldMapView tooltip flags).
    pub fn flag_labels(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.flags & REGION_FLAGS_SANDBOX != 0 {
            v.push(aurora_i18n::tr!("world-worldmap-sandbox"));
        }
        if self.flags & REGION_FLAGS_ALLOW_DAMAGE != 0 {
            v.push(aurora_i18n::tr!("world-worldmap-unsecured"));
        }
        v
    }
}

pub fn access_label(access: u8) -> &'static str {
    match access {
        0..=SIM_ACCESS_PG => aurora_i18n::tr!("common-general"),
        SIM_ACCESS_MATURE => aurora_i18n::tr!("common-moderate"),
        SIM_ACCESS_ADULT => aurora_i18n::tr!("common-adult"),
        SIM_ACCESS_DOWN => aurora_i18n::tr!("common-offline"),
        _ => aurora_i18n::tr!("common-unknown"),
    }
}

/// The tracked location (LLTracker TRACKING_LOCATION).
#[derive(Debug, Clone)]
pub struct Track {
    /// Global position (meters).
    pub x: f64,
    pub y: f64,
    pub z: f32,
    pub label: String,
    pub tooltip: String,
    /// The region at that spot is not known yet (asked with
    /// MAP_SIM_RETURN_NULL_SIMS); teleport on arrival when `teleport`.
    pub pending: bool,
    pub teleport: bool,
    /// No region there, or it is down.
    pub invalid: bool,
}

/// The parcel of the tracked spot (LLFloaterWorldMap::requestParcelInfo):
/// RemoteParcelRequest for the point, then ParcelInfoRequest for its id,
/// once per spot as Firestorm (mRequestedGlobalPos).
#[derive(Debug, Clone, PartialEq)]
pub struct TrackParcel {
    pub x: f64,
    pub y: f64,
    handle: RegionHandle,
    position: glam::Vec3,
    /// The parcel's id once RemoteParcelRequest answered.
    pub id: Option<Uuid>,
}

/// The friend the map follows (LLAvatarTracker's LLTrackingData,
/// TRACKING_AVATAR): asked with FindAgent, the tracked location follows
/// their answers.
#[derive(Debug, Clone)]
pub struct AvatarTrack {
    pub id: Uuid,
    pub name: String,
    /// Answered at least once: `WorldMap::track` points at them.
    pub found: bool,
    /// FindAgent supplies X/Y only; never present a ground fallback as the
    /// friend's measured altitude.
    pub altitude_known: bool,
    /// An explicit edit of Z on the map, scoped to this friend.
    altitude_override: Option<f32>,
    asked: Instant,
    /// One explicit remote teleport, consumed by the first matching reply.
    teleport: Option<Instant>,
}

impl super::World {
    /// Loaded or coarse avatar position in render space. Remote friends
    /// require FindAgent instead; map rights alone cannot enable a camera zoom.
    pub fn avatar_map_position(&self, id: Uuid) -> Option<glam::Vec3> {
        if let Some(o) = self.objects.index_of_uuid(&id).and_then(|i| self.objects.get(i))
            && let Some(off) = self.region_offset(o.key.region)
        {
            return Some(off + o.position);
        }
        self.coarse
            .iter()
            .find_map(|(h, list)| Some(self.region_offset(*h)? + *list.iter().find(|(i, _)| *i == id).map(|(_, p)| p)?))
    }

    pub fn can_locate_avatar(&self, id: Uuid) -> bool {
        self.avatar_map_position(id).is_some() || self.social.can_map_friend(id)
    }

    /// LLAvatarActions::showOnMap uses LLFloaterWorldMap::trackAvatar for
    /// remote friends. Extend the nearby radar's Teleport To through that
    /// same lookup when the friend explicitly grants us location rights.
    pub fn locate_avatar(&mut self, id: Uuid, teleport: bool) -> bool {
        let can_map = self.social.can_map_friend(id);
        if can_map && !teleport {
            let name = self.social.name_of(&id);
            self.map.track_avatar(id, &name);
            self.map.focus_track = true;
        } else if let Some(p) = self.avatar_map_position(id) {
            let (ox, oy) = self.main_origin().unwrap_or((0, 0));
            self.map
                .track_location(ox as f64 + p.x as f64, oy as f64 + p.y as f64, p.z, teleport);
            if teleport && can_map {
                self.map.friend_teleport = Some(id);
            }
        } else if can_map {
            let name = self.social.name_of(&id);
            self.map.teleport_to_avatar(id, &name);
        } else {
            return false;
        }
        true
    }

    /// Recheck rights/presence before sending a delayed lookup or teleport:
    /// a menu may have been opened before the friend revoked permission.
    pub fn take_map_commands(&mut self) -> Vec<NetCommand> {
        let allowed = self.map.avatar.as_ref().is_none_or(|a| self.social.can_map_friend(a.id));
        if !allowed {
            self.map.clear_track();
        }
        let mut out = self.map.take_commands();
        if !allowed {
            out.retain(|c| {
                !matches!(
                    c,
                    NetCommand::FindAgent(_) | NetCommand::TrackAgent(_) | NetCommand::TeleportTo { .. }
                ) || matches!(c, NetCommand::TrackAgent(id) if id.is_nil())
            });
        }
        out
    }
}

/// Parcel the agent may not enter, near it (LLViewerParcelMgr collision
/// parcel): drawn as ban lines in the world and on the mini-map.
#[derive(Debug, Clone)]
pub struct Collision {
    pub handle: RegionHandle,
    /// 1 banned, 2 not in the group, 3 not on the access list (BA_*).
    pub kind: u8,
    /// The parcel sells passes (NoEntryPassLines).
    pub use_pass: bool,
    /// One bit per 4 m cell (x + y * cells per edge).
    pub bitmap: Vec<u8>,
    /// Last collision message (ShowBanLines "on proximity" shows 1 s).
    pub received: Instant,
}

impl Collision {
    pub fn cell(&self, x: u32, y: u32, per_edge: u32) -> bool {
        let i = (x + y * per_edge) as usize;
        self.bitmap.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
    }
}

/// ShowBanLines (0 hidden, 1 on collision, 2 on proximity): how long the
/// lines stay after their trigger (PARCEL_BAN_LINES_DRAW_SECS_ON_COLLISION,
/// PARCEL_COLLISION_DRAW_SECS_ON_PROXIMITY).
pub const BAN_LINES_ON_COLLISION_SECS: f32 = 10.0;
pub const BAN_LINES_ON_PROXIMITY_SECS: f32 = 1.0;

#[derive(Default)]
pub struct WorldMap {
    /// By grid position of the region's south-west corner (region units).
    pub sims: HashMap<(u32, u32), SimInfo>,
    /// Slots known to hold no region.
    pub null_sims: HashSet<(u32, u32)>,
    blocks: HashMap<(u32, u32), Instant>,
    /// Grid-wide items by type (telehubs/infohubs, land for sale, events).
    pub items: HashMap<u32, Vec<MapItem>>,
    items_loaded: Option<Instant>,
    /// MAP_ITEM_AGENT_LOCATIONS by region: (global x, y, count).
    pub agents: HashMap<(u32, u32), Vec<(u32, u32, i32)>>,
    agents_asked: HashMap<(u32, u32), Instant>,
    pub track: Option<Track>,
    /// The friend `track` follows, if any.
    pub avatar: Option<AvatarTrack>,
    /// Friend targeted by a direct position-based Teleport To.
    pub(super) friend_teleport: Option<Uuid>,
    pub(super) friend_join: Option<super::friend_join::FriendJoin>,
    pub(super) friend_join_flight: Option<super::friend_join::FriendJoinFlight>,
    /// The parcel asked for the tracked spot.
    pub track_parcel: Option<TrackParcel>,
    /// Centre the open map on `track` once its region is known
    /// (llMapDestination's BEACON_FOCUS_MAP, LLFloaterWorldMap autofocus).
    pub focus_track: bool,
    /// Region name search in progress (lowercase) and when it was sent.
    pub search: Option<(String, Instant)>,
    /// Place link waiting for its region by name (lowercase name, position in
    /// the region): tracked when the MapNameRequest answer arrives.
    wanted_place: Option<(String, glam::Vec3)>,
    /// Home location from the login response (global meters).
    pub home: Option<(f64, f64, f32)>,
    /// Parcel overlay of each connected region: (cells per side x, y, bytes).
    pub overlays: HashMap<RegionHandle, (u32, u32, Vec<u8>)>,
    /// Bumped when an overlay changes (mini-map property lines redraw).
    pub overlay_generation: u64,
    /// Latest collision parcel and the last "Cannot enter parcel" alert.
    pub collision: Option<Collision>,
    pub blocked_alert: Option<Instant>,
    /// The rating of each region known by name (lowercase), for the badges
    /// of the place links.
    pub ratings: RegionRatings,
    out: Vec<NetCommand>,
}

/// The rating (SIM_ACCESS_*) of regions by name, for the badges of place
/// links: what the map, the connected regions and the searches taught,
/// and the names to ask the server.
///
/// Deliberately more than Firestorm, which never looks a region up to show
/// a link (only when it is clicked, LLURLDispatcher → sendNamedRegionRequest):
/// a region not known yet is asked once per session with MapNameRequest,
/// at most `RATING_REQUESTS_PER_SECOND` names a second, and only for the
/// links shown on screen.
#[derive(Default)]
pub struct RegionRatings {
    by_name: HashMap<String, u8>,
    /// Names already asked (or queued), never asked again.
    asked: HashSet<String>,
    queue: std::collections::VecDeque<String>,
    last_sent: Option<Instant>,
}

/// Region name lookups sent per second at most (a burst of links in the
/// chat waits in the queue).
pub const RATING_REQUESTS_PER_SECOND: u32 = 4;
/// Names waiting at most; older ones are dropped (they are asked again
/// the next time their link is shown).
const RATING_QUEUE_MAX: usize = 64;

impl RegionRatings {
    pub fn learn(&mut self, name: &str, access: u8) {
        if !name.trim().is_empty() {
            self.by_name.insert(name.trim().to_lowercase(), access);
        }
    }

    /// The rating of a region by name, if known.
    pub fn get(&self, name: &str) -> Option<u8> {
        self.by_name.get(&name.trim().to_lowercase()).copied()
    }

    /// A link to this region is on screen: its rating, or None while it is
    /// unknown (then asked once).
    pub fn want(&mut self, name: &str) -> Option<u8> {
        let key = name.trim().to_lowercase();
        if let Some(access) = self.by_name.get(&key) {
            return Some(*access);
        }
        if !key.is_empty() && self.asked.insert(key.clone()) {
            if self.queue.len() >= RATING_QUEUE_MAX
                && let Some(old) = self.queue.pop_front()
            {
                self.asked.remove(&old);
            }
            self.queue.push_back(key);
        }
        None
    }

    /// The next lookup to send, when the rate allows it.
    pub fn next_request(&mut self, now: Instant) -> Option<NetCommand> {
        let interval = std::time::Duration::from_secs(1) / RATING_REQUESTS_PER_SECOND;
        if self.last_sent.is_some_and(|t| now.duration_since(t) < interval) {
            return None;
        }
        let name = self.queue.pop_front()?;
        self.last_sent = Some(now);
        // LLWorldMapMessage::sendNamedRegionRequest, as the map's search
        Some(NetCommand::MapNameRequest { name })
    }
}

impl WorldMap {
    pub fn apply_collision(&mut self, handle: RegionHandle, kind: u8, use_pass: bool, bitmap: Vec<u8>) {
        self.collision = Some(Collision {
            handle,
            kind,
            use_pass,
            bitmap,
            received: Instant::now(),
        });
        self.overlay_generation += 1;
    }

    /// process_alert_message: being pushed out of a parcel shows the ban
    /// lines in the "on collision" mode.
    pub fn on_alert(&mut self, message: &str) {
        if message.contains("Cannot enter parcel") {
            self.blocked_alert = Some(Instant::now());
        }
    }

    /// LLViewerParcelMgr::renderParcelCollision: are the ban lines drawn now?
    pub fn ban_lines_visible(&self, mode: u8) -> bool {
        let Some(c) = &self.collision else {
            return false;
        };
        match mode {
            1 => self
                .blocked_alert
                .is_some_and(|t| t.elapsed().as_secs_f32() < BAN_LINES_ON_COLLISION_SECS),
            2 => c.received.elapsed().as_secs_f32() < BAN_LINES_ON_PROXIMITY_SECS,
            _ => false,
        }
    }

    /// Queue a command for the network (sent by the app each frame).
    pub fn push(&mut self, c: NetCommand) {
        self.out.push(c);
    }

    /// LLTracker: stop tracking once within 3 m of the spot (a friend keeps
    /// being followed).
    pub fn check_arrival(&mut self, x: f64, y: f64, z: f32) {
        if let Some(t) = &self.track
            && self.avatar.is_none()
            && !t.pending
            && !t.invalid
            && (t.x - x).hypot(t.y - y) < 3.0
            && (t.z - z).abs() * 0.5 < 3.0
        {
            self.clear_track();
        }
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        let now = Instant::now();
        while let Some(c) = self.ratings.next_request(now) {
            self.out.push(c);
        }
        // LLTrackingData::haveTrackingInfo: the followed friend asked again
        if let Some(a) = self.avatar.as_mut()
            && now.duration_since(a.asked) >= FIND_AGENT_EVERY
        {
            a.asked = now;
            self.out.push(NetCommand::FindAgent(a.id));
        }
        std::mem::take(&mut self.out)
    }

    /// Home position from the login "home" string:
    /// `{'region_handle':[r256000, r256000], 'position':[r128, r128, r30], ...}`.
    pub fn set_home_from_login(&mut self, home: &str) {
        let nums = |key: &str| -> Vec<f64> {
            let Some(i) = home.find(key) else {
                return Vec::new();
            };
            let rest = &home[i + key.len()..];
            let Some(a) = rest.find('[') else {
                return Vec::new();
            };
            let Some(b) = rest[a..].find(']') else {
                return Vec::new();
            };
            rest[a + 1..a + b]
                .split(',')
                .filter_map(|s| s.trim().trim_start_matches('r').parse::<f64>().ok())
                .collect()
        };
        let h = nums("region_handle");
        let p = nums("position");
        if h.len() == 2 && p.len() == 3 {
            self.home = Some((h[0] + p[0], h[1] + p[1], p[2] as f32));
        }
    }

    /// Region holding a grid slot (variable regions span several).
    pub fn sim_at(&self, gx: u32, gy: u32) -> Option<((u32, u32), &SimInfo)> {
        if let Some(s) = self.sims.get(&(gx, gy)) {
            return Some(((gx, gy), s));
        }
        self.sims
            .iter()
            .find(|((x, y), s)| gx >= *x && gy >= *y && gx < x + s.size_x / 256 && gy < y + s.size_y / 256)
            .map(|(k, s)| (*k, s))
    }

    /// Region under a global position.
    pub fn sim_at_global(&self, x: f64, y: f64) -> Option<((u32, u32), &SimInfo)> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        self.sim_at((x / 256.0) as u32, (y / 256.0) as u32)
    }

    pub fn apply_blocks(&mut self, blocks: Vec<MapBlock>) {
        for b in blocks {
            let key = (b.x as u32, b.y as u32);
            // a non-existent slot is never a region (insertRegion), whatever
            // its name: a name search ends on a block that repeats the
            // requested name with this access, away from the real region
            if b.access == SIM_ACCESS_NONE {
                if b.name.is_empty() {
                    self.null_sims.insert(key);
                    self.sims.remove(&key);
                }
                continue;
            }
            if b.name.is_empty() {
                continue;
            }
            self.null_sims.remove(&key);
            self.ratings.learn(&b.name, b.access);
            self.sims.insert(
                key,
                SimInfo {
                    name: b.name,
                    access: b.access,
                    flags: b.region_flags,
                    image: b.map_image_id,
                    size_x: b.size_x as u32,
                    size_y: b.size_y as u32,
                },
            );
        }
        // a tracked spot waiting for its region (LLWorldMap::insertRegion)
        let mut teleport = None;
        if let Some(t) = self.track.as_mut().filter(|t| t.pending) {
            let (gx, gy) = ((t.x / 256.0) as u32, (t.y / 256.0) as u32);
            let found = self
                .sims
                .iter()
                .find(|((x, y), s)| gx >= *x && gy >= *y && gx < x + s.size_x / 256 && gy < y + s.size_y / 256)
                .map(|(k, s)| (*k, s.clone()));
            if let Some(((sx, sy), s)) = found {
                t.pending = false;
                t.invalid = s.is_down();
                if t.label.is_empty() {
                    t.label = location_label(&s.name, t.x - sx as f64 * 256.0, t.y - sy as f64 * 256.0, t.z);
                }
                if t.teleport && !t.invalid {
                    teleport = Some((t.x, t.y, t.z));
                }
                t.teleport = false;
            } else if self.null_sims.contains(&(gx, gy)) {
                t.pending = false;
                t.invalid = true;
                t.teleport = false;
            }
        }
        if let Some((x, y, z)) = teleport {
            self.out.push(teleport_command(x, y, z));
        }
        // a place link waiting for its region by name
        if let Some((name, pos)) = self.wanted_place.take()
            && !self.track_known_region(&name, pos)
        {
            self.wanted_place = Some((name, pos));
        }
    }

    pub fn apply_items(&mut self, item_type: u32, items: Vec<MapItem>) {
        if item_type == map_item::AGENT_LOCATIONS {
            // replaces the counts of the region(s) the reply covers
            let mut by_sim: HashMap<(u32, u32), Vec<(u32, u32, i32)>> = HashMap::new();
            for it in items {
                let key = self
                    .sim_at(it.x / 256, it.y / 256)
                    .map(|(k, _)| k)
                    .unwrap_or((it.x / 256, it.y / 256));
                let list = by_sim.entry(key).or_default();
                if it.extra > 0 {
                    list.push((it.x, it.y, it.extra));
                }
            }
            self.agents.extend(by_sim);
            return;
        }
        let list = self.items.entry(item_type).or_default();
        for it in items {
            if !list.iter().any(|o| o.id == it.id && o.x == it.x && o.y == it.y) {
                list.push(it);
            }
        }
    }

    /// LLWorldMap::reloadItems: grid-wide items, at most every 10 min
    /// unless forced.
    pub fn reload_items(&mut self, force: bool) {
        let now = Instant::now();
        if !force && self.items_loaded.is_some_and(|t| now.duration_since(t) < ITEMS_UPDATE) {
            return;
        }
        self.items_loaded = Some(now);
        self.items.clear();
        for t in [
            map_item::TELEHUB,
            map_item::PG_EVENT,
            map_item::MATURE_EVENT,
            map_item::ADULT_EVENT,
            map_item::LAND_FOR_SALE,
            map_item::LAND_FOR_SALE_ADULT,
        ] {
            self.out.push(NetCommand::MapItemRequest { item_type: t, handle: 0 });
        }
    }

    /// LLWorldMap::updateRegions: ask the 16 x 16 blocks covering a range of
    /// grid positions (region units), each at most once a minute.
    pub fn want_regions(&mut self, x0: i64, y0: i64, x1: i64, y1: i64) {
        let clamp = |v: i64| (v.max(0) as u32 / MAP_BLOCK_SIZE).min(MAP_BLOCK_RES - 1);
        let now = Instant::now();
        let (bx0, by0, bx1, by1) = (clamp(x0), clamp(y0), clamp(x1), clamp(y1));
        // a world map zoomed far out stops asking (level > 3 never gets here)
        if (bx1 - bx0 + 1) * (by1 - by0 + 1) > 64 {
            return;
        }
        for by in by0..=by1 {
            for bx in bx0..=bx1 {
                let fresh = self.blocks.get(&(bx, by)).is_some_and(|t| now.duration_since(*t) < BLOCK_UPDATE);
                if fresh {
                    continue;
                }
                self.blocks.insert((bx, by), now);
                self.out.push(NetCommand::MapBlockRequest {
                    min_x: (bx * MAP_BLOCK_SIZE) as u16,
                    min_y: (by * MAP_BLOCK_SIZE) as u16,
                    max_x: (bx * MAP_BLOCK_SIZE + MAP_BLOCK_SIZE - 1) as u16,
                    max_y: (by * MAP_BLOCK_SIZE + MAP_BLOCK_SIZE - 1) as u16,
                    null_sims: false,
                });
            }
        }
    }

    /// LLSimInfo::updateAgentCount: people dots of a visible region.
    pub fn want_agents(&mut self, key: (u32, u32)) {
        let now = Instant::now();
        if self.agents_asked.get(&key).is_some_and(|t| now.duration_since(*t) < AGENTS_UPDATE) {
            return;
        }
        self.agents_asked.insert(key, now);
        self.out.push(NetCommand::MapItemRequest {
            item_type: map_item::AGENT_LOCATIONS,
            handle: aurora_net::origin_to_handle(key.0 * 256, key.1 * 256),
        });
    }

    /// LLFloaterWorldMap::trackAvatar: follow a friend who lets us see them
    /// on the map; the track points at them once the grid answers, and the
    /// open map centres on it.
    pub fn track_avatar(&mut self, id: Uuid, name: &str) {
        self.friend_teleport = None;
        self.friend_join = None;
        self.friend_join_flight = None;
        if let Some(a) = self.avatar.as_mut().filter(|a| a.id == id) {
            a.teleport = None;
            if let Some(t) = self.track.as_mut() {
                t.teleport = false;
            }
            return;
        }
        self.avatar = Some(AvatarTrack {
            id,
            name: name.to_owned(),
            found: false,
            altitude_known: false,
            altitude_override: None,
            asked: Instant::now(),
            teleport: None,
        });
        self.track = None;
        self.focus_track = true;
        // LLAvatarTracker::track: TrackAgent, then findAgent
        self.out.push(NetCommand::TrackAgent(id));
        self.out.push(NetCommand::FindAgent(id));
    }

    /// A remote Teleport To waits for a fresh FindAgent reply, at most one
    /// normal lookup interval. Subsequent periodic replies never teleport.
    pub fn teleport_to_avatar(&mut self, id: Uuid, name: &str) {
        let tracked = self.avatar.as_ref().is_some_and(|a| a.id == id);
        self.track_avatar(id, name);
        if let Some(a) = self.avatar.as_mut() {
            a.teleport = Some(Instant::now());
            a.found = false;
            a.altitude_override = None;
        }
        self.track = None;
        if tracked {
            self.out.push(NetCommand::FindAgent(id));
            if let Some(a) = self.avatar.as_mut() {
                a.asked = Instant::now();
            }
        }
    }

    /// LLAvatarTracker::untrack.
    fn stop_avatar(&mut self) {
        self.friend_teleport = None;
        self.friend_join = None;
        self.friend_join_flight = None;
        if self.avatar.take().is_some() {
            self.out.push(NetCommand::TrackAgent(Uuid::nil()));
        }
    }

    /// LLAvatarTracker::agentFound / setTrackedCoarseLocation: FindAgent
    /// has no Z. Use the target's observed height when available, otherwise
    /// ground level, never our own height or a previous destination's Z.
    pub fn agent_found(&mut self, prey: Uuid, x: f64, y: f64, altitude: Option<f32>) {
        let Some(a) = self.avatar.as_mut().filter(|a| a.id == prey) else {
            return;
        };
        a.found = true;
        a.altitude_known = altitude.is_some_and(f32::is_finite);
        let name = a.name.clone();
        let teleport = a.teleport.take().is_some_and(|started| started.elapsed() < FIND_AGENT_EVERY);
        let z = a.altitude_override.or(altitude.filter(|z| z.is_finite())).unwrap_or(0.0);
        // A duplicate reply must not cancel an outstanding region lookup.
        if self.track.as_ref().is_some_and(|t| t.teleport) {
            return;
        }
        self.place_track(x, y, z, teleport);
        if let Some(t) = self.track.as_mut() {
            t.label = name;
        }
    }

    /// Editing only Z keeps following the friend, but the teleport height
    /// remains the resident's choice until another friend/action is selected.
    pub fn set_track_altitude(&mut self, z: f32) {
        if !z.is_finite() {
            return;
        }
        self.friend_teleport = None;
        self.friend_join = None;
        self.friend_join_flight = None;
        if let Some(t) = self.track.as_mut() {
            t.z = z;
            if let Some(a) = self.avatar.as_mut() {
                a.altitude_override = Some(z);
            }
        }
    }

    pub(super) fn unknown_height_teleport_avatar(&self, command: &NetCommand) -> Option<Uuid> {
        self.avatar.as_ref().filter(|a| !a.altitude_known)?;
        self.friend_teleport_avatar(command)
    }

    pub(super) fn friend_teleport_avatar(&self, command: &NetCommand) -> Option<Uuid> {
        let NetCommand::TeleportTo { handle, position, .. } = command else {
            return None;
        };
        let avatar = self.friend_teleport.or_else(|| {
            self.avatar
                .as_ref()
                .filter(|a| a.found && a.altitude_override.is_none())
                .map(|a| a.id)
        })?;
        let t = self.track.as_ref().filter(|t| !t.pending && !t.invalid)?;
        let NetCommand::TeleportTo {
            handle: expected,
            position: at,
            ..
        } = teleport_command(t.x, t.y, t.z)
        else {
            return None;
        };
        (*handle == expected && *position == at).then_some(avatar)
    }

    pub(super) fn cancel_avatar_teleport(&mut self) {
        self.friend_teleport = None;
        if let Some(a) = &mut self.avatar {
            a.teleport = None;
        }
        if let Some(t) = &mut self.track {
            t.teleport = false;
        }
        self.out.retain(|c| !matches!(c, NetCommand::TeleportTo { .. }));
    }

    /// LLFloaterWorldMap::trackLocation → requestParcelInfo: ask the parcel
    /// of the tracked spot, once per spot (the open map calls it each frame
    /// for a spot, not for a landmark nor a friend).
    pub fn want_track_parcel(&mut self) {
        let Some(t) = self.track.as_ref().filter(|t| !t.pending && !t.invalid) else {
            return;
        };
        if self.track_parcel.as_ref().is_some_and(|p| p.x == t.x && p.y == t.y) {
            return;
        }
        let slot = |g: f64| ((g.max(0.0) / 256.0).floor() as u32) * 256;
        let handle = aurora_net::origin_to_handle(slot(t.x), slot(t.y));
        let position = glam::Vec3::new(t.x.rem_euclid(256.0) as f32, t.y.rem_euclid(256.0) as f32, t.z);
        self.track_parcel = Some(TrackParcel {
            x: t.x,
            y: t.y,
            handle,
            position,
            id: None,
        });
        self.out.push(NetCommand::RemoteParcelRequest {
            handle,
            position,
            region_id: Uuid::nil(),
        });
    }

    /// RemoteParcelRequest answered: the tracked spot's parcel is asked
    /// (LLWorldMapParcelInfoObserver::setParcelID → sendParcelInfoRequest).
    pub fn apply_remote_parcel(&mut self, handle: RegionHandle, position: glam::Vec3, id: Option<Uuid>) {
        if let Some(p) = self
            .track_parcel
            .as_mut()
            .filter(|p| p.handle == handle && p.position == position && p.id.is_none())
            && let Some(id) = id
        {
            p.id = Some(id);
            self.out.push(NetCommand::ParcelInfoRequest(id));
        }
    }

    /// The parcel id of the tracked spot, once known.
    pub fn track_parcel_id(&self) -> Option<Uuid> {
        let t = self.track.as_ref()?;
        self.track_parcel.as_ref().filter(|p| p.x == t.x && p.y == t.y)?.id
    }

    /// Forget the destination and its followed friend or pending place
    /// lookup, so a later reply cannot recreate the dismissed beacon.
    pub fn clear_track(&mut self) {
        self.stop_avatar();
        self.track = None;
        self.wanted_place = None;
        self.focus_track = false;
    }

    /// Track a spot set by hand (a region of the search): no friend is
    /// followed any more.
    pub fn set_track(&mut self, t: Track) {
        self.stop_avatar();
        self.track = Some(t);
    }

    /// LLFloaterWorldMap::trackLocation: point at a global position; an
    /// unknown region is asked first (MAP_SIM_RETURN_NULL_SIMS).
    pub fn track_location(&mut self, x: f64, y: f64, z: f32, teleport: bool) {
        self.stop_avatar();
        self.place_track(x, y, z, teleport);
    }

    fn place_track(&mut self, x: f64, y: f64, z: f32, teleport: bool) {
        let (gx, gy) = ((x.max(0.0) / 256.0) as u32, (y.max(0.0) / 256.0) as u32);
        let mut t = Track {
            x,
            y,
            z,
            label: String::new(),
            tooltip: String::new(),
            pending: false,
            teleport: false,
            invalid: false,
        };
        match self.sim_at(gx, gy).map(|(k, s)| (k, s.clone())) {
            Some(((sx, sy), s)) => {
                t.invalid = s.is_down();
                t.label = location_label(&s.name, x - sx as f64 * 256.0, y - sy as f64 * 256.0, z);
                if teleport && !t.invalid {
                    self.out.push(teleport_command(x, y, z));
                }
            }
            None if self.null_sims.contains(&(gx, gy)) => t.invalid = true,
            None => {
                t.pending = true;
                t.teleport = teleport;
                self.out.push(NetCommand::MapBlockRequest {
                    min_x: gx as u16,
                    min_y: gy as u16,
                    max_x: gx as u16,
                    max_y: gy as u16,
                    null_sims: true,
                });
            }
        }
        self.track = Some(t);
    }

    /// LLFloaterWorldMap::trackURL: track a position of a region given by
    /// name, asking the region with MapNameRequest when it is not known yet
    /// (LLWorldMapMessage::sendNamedRegionRequest).
    pub fn track_region(&mut self, name: &str, pos: glam::Vec3) {
        let lower = name.trim().to_lowercase();
        if lower.is_empty() {
            return;
        }
        if !self.track_known_region(&lower, pos) {
            self.out.push(NetCommand::MapNameRequest { name: lower.clone() });
            self.wanted_place = Some((lower, pos));
        }
    }

    /// llMapDestination (process_script_teleport_request, then trackURL):
    /// whole metres, the height kept within Second Life's 0..4096 m. Returns
    /// whether the map opens (BEACON_SHOW_MAP); it centres on the place with
    /// BEACON_FOCUS_MAP. Nothing teleports until the resident asks to.
    pub fn map_destination(&mut self, region: &str, pos: glam::Vec3, flags: u32) -> bool {
        use aurora_net::beacon;
        let show = flags & beacon::SHOW_MAP != 0;
        self.track_region(
            region,
            glam::Vec3::new(pos.x.trunc(), pos.y.trunc(), pos.z.trunc().clamp(0.0, 4096.0)),
        );
        self.focus_track = show && flags & beacon::FOCUS_MAP != 0;
        show
    }

    fn track_known_region(&mut self, lower: &str, pos: glam::Vec3) -> bool {
        let Some((sx, sy)) = self.region_by_name(lower) else {
            return false;
        };
        self.track_location(sx as f64 * 256.0 + pos.x as f64, sy as f64 * 256.0 + pos.y as f64, pos.z, false);
        true
    }

    /// Grid position (region units) of a known region, by name, ignoring case.
    pub fn region_by_name(&self, name: &str) -> Option<(u32, u32)> {
        let lower = name.trim().to_lowercase();
        self.sims.iter().find(|(_, s)| s.name.to_lowercase() == lower).map(|(k, _)| *k)
    }

    /// LLFloaterWorldMap::onLocationCommit: MapNameRequest (a "#" is
    /// appended under 3 characters).
    pub fn search_region(&mut self, text: &str) {
        let s = text.trim().to_lowercase();
        if s.is_empty() {
            return;
        }
        let name = if s.chars().count() < 3 { format!("{s}#") } else { s.clone() };
        self.out.push(NetCommand::MapNameRequest { name });
        self.search = Some((s, Instant::now()));
    }

    /// LLFloaterWorldMap::updateSims: known regions whose name contains the
    /// search, sorted by name.
    pub fn search_results(&self) -> Vec<((u32, u32), &SimInfo)> {
        let Some((s, _)) = &self.search else {
            return Vec::new();
        };
        let mut v: Vec<_> = self
            .sims
            .iter()
            .filter(|(_, i)| i.name.to_lowercase().contains(s.as_str()))
            .map(|(k, i)| (*k, i))
            .collect();
        v.sort_by_key(|a| a.1.name.to_lowercase());
        v
    }

    /// LLViewerParcelOverlay::uncompressLandOverlay: store one quarter.
    pub fn apply_overlay(&mut self, handle: RegionHandle, size: (u32, u32), sequence: i32, data: &[u8]) {
        let (cx, cy) = (size.0 / 4, size.1 / 4);
        let total = (cx * cy) as usize;
        let e = self.overlays.entry(handle).or_insert_with(|| (cx, cy, vec![0; total]));
        if e.2.len() != total {
            *e = (cx, cy, vec![0; total]);
        }
        let part = total / 4;
        let start = (sequence.clamp(0, 3) as usize) * part;
        let n = data.len().min(part);
        e.2[start..start + n].copy_from_slice(&data[..n]);
        self.overlay_generation += 1;
    }
}

/// "Region (x, y, z)" (LLFloaterWorldMap::trackLocation).
pub fn location_label(region: &str, x: f64, y: f64, z: f32) -> String {
    format!("{region} ({}, {}, {})", x.round() as i64, y.round() as i64, z.round() as i64)
}

/// LLAgent::teleportViaLocation: handle of the 256 m slot, position inside it
/// (the simulator of a variable region accepts its own slots).
pub fn teleport_command(x: f64, y: f64, z: f32) -> NetCommand {
    let (gx, gy) = ((x / 256.0).floor().max(0.0) as u32, (y / 256.0).floor().max(0.0) as u32);
    let lx = (x - gx as f64 * 256.0) as f32;
    let ly = (y - gy as f64 * 256.0) as f32;
    NetCommand::TeleportTo {
        handle: aurora_net::origin_to_handle(gx * 256, gy * 256),
        position: glam::Vec3::new(lx, ly, z),
        look_at: glam::Vec3::new(lx + 1.0, ly, z),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_friend() -> (super::super::World, Uuid) {
        let mut w = super::super::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(1);
        let id = Uuid::from_u128(7);
        w.social.friends.push(super::super::social::Friend {
            id,
            online: true,
            rights_given: 0,
            rights_has: super::super::social::rights::MAP_LOCATION,
        });
        w.social.names.insert(id, "Remote friend".into());
        (w, id)
    }

    #[test]
    fn remote_friend_location_uses_received_rights_and_current_presence() {
        let (mut w, id) = remote_friend();
        for online in [false, true] {
            for given in 0..8 {
                for received in 0..8 {
                    let f = &mut w.social.friends[0];
                    (f.online, f.rights_given, f.rights_has) = (online, given, received);
                    let allowed = online && received & 2 != 0;
                    assert_eq!(w.can_locate_avatar(id), allowed);
                    assert_eq!(w.locate_avatar(id, false), allowed);
                    let out = w.take_map_commands();
                    assert_eq!(out.iter().any(|c| matches!(c, NetCommand::FindAgent(i) if *i == id)), allowed);
                    assert!(w.avatar_map_position(id).is_none(), "map rights do not enable nearby actions");
                    w.map.clear_track();
                    w.map.take_commands();
                }
            }
        }
        w.social.friends.clear();
        assert!(!w.locate_avatar(id, true));
        assert!(w.take_map_commands().is_empty());
    }

    #[test]
    fn remote_friend_map_action_tracks_without_teleporting() {
        let (mut w, id) = remote_friend();
        assert!(w.locate_avatar(id, false));
        assert!(matches!(&w.take_map_commands()[..], [NetCommand::TrackAgent(a), NetCommand::FindAgent(b)] if *a == id && *b == id));
        w.apply(aurora_net::NetEvent::AgentFound {
            prey: id,
            x: 256_010.0,
            y: 256_266.0,
        });
        w.map.apply_blocks(vec![yiff()]);
        let t = w.map.track.as_ref().expect("remote friend located");
        assert_eq!((t.x, t.y, t.label.as_str()), (256_010.0, 256_266.0, "Remote friend"));
        assert!(w.take_map_commands().iter().all(|c| !matches!(c, NetCommand::TeleportTo { .. })));
        w.map.focus_track = false;
        assert!(w.locate_avatar(id, false));
        assert!(w.map.focus_track, "an explicit map action recenters an already tracked friend");
        assert!(w.take_map_commands().is_empty(), "reuse the active lookup without another request");
    }

    #[test]
    fn remote_friend_altitude_never_inherits_our_skybox_or_previous_destination() {
        for teleport in [false, true] {
            for known_region in [false, true] {
                let (mut w, id) = remote_friend();
                w.agent.position.z = 3000.0;
                if known_region {
                    w.map.apply_blocks(vec![yiff()]);
                }
                w.map.track_location(256_010.0, 256_266.0, 3500.0, false);
                w.take_map_commands();
                assert!(w.locate_avatar(id, teleport));
                w.take_map_commands();
                w.apply(aurora_net::NetEvent::AgentFound {
                    prey: id,
                    x: 256_010.0,
                    y: 256_266.0,
                });
                let mut out = w.take_map_commands();
                w.map.apply_blocks(vec![yiff()]);
                out.extend(w.take_map_commands());
                assert_eq!(w.map.track.as_ref().map(|t| t.z), Some(0.0));
                assert!(!w.map.avatar.as_ref().unwrap().altitude_known);
                let heights: Vec<_> = out
                    .iter()
                    .filter_map(|c| match c {
                        NetCommand::TeleportTo { position, .. } => Some(position.z),
                        _ => None,
                    })
                    .collect();
                assert_eq!(heights, if teleport { vec![0.0] } else { vec![] });
            }
        }
    }

    #[test]
    fn mapped_friend_uses_target_altitude_and_updates_from_coarse_locations() {
        let (mut w, id) = remote_friend();
        let handle = aurora_net::origin_to_handle(256_000, 256_256);
        w.main_region = Some(handle);
        w.agent.position.z = 3000.0;
        w.coarse.insert(handle, vec![(id, glam::Vec3::new(10.0, 10.0, 20.0))]);
        w.locate_avatar(id, false);
        w.apply(aurora_net::NetEvent::AgentFound {
            prey: id,
            x: 256_010.0,
            y: 256_266.0,
        });
        assert_eq!(w.map.track.as_ref().map(|t| t.z), Some(20.0));
        assert!(w.map.avatar.as_ref().unwrap().altitude_known);
        w.apply(aurora_net::NetEvent::CoarseLocations {
            handle,
            positions: vec![(id, glam::Vec3::new(12.0, 10.0, 24.0))],
        });
        assert_eq!(w.map.track.as_ref().map(|t| (t.x, t.z)), Some((256_012.0, 24.0)));
        let mut avatar = crate::demo::action_avatar(false);
        avatar.full_id = id;
        avatar.position = glam::Vec3::new(12.0, 10.0, 2048.0);
        w.objects.upsert_at(handle, avatar, Some(Instant::now()));
        w.apply(aurora_net::NetEvent::CoarseLocations {
            handle,
            positions: vec![(id, glam::Vec3::new(12.0, 10.0, 1020.0))],
        });
        assert_eq!(
            w.map.track.as_ref().map(|t| t.z),
            Some(2048.0),
            "a coarse update cannot lower a known exact height"
        );
        // A distant reply cannot borrow Z from an old local position.
        w.apply(aurora_net::NetEvent::AgentFound {
            prey: id,
            x: 300_010.0,
            y: 300_266.0,
        });
        assert_eq!(w.map.track.as_ref().map(|t| t.z), Some(0.0));
        assert!(!w.map.avatar.as_ref().unwrap().altitude_known);
    }

    #[test]
    fn friend_altitude_override_is_scoped_to_the_friend_and_explicit_action() {
        let mut m = WorldMap::default();
        let id = Uuid::from_u128(7);
        m.track_avatar(id, "Remote friend");
        m.agent_found(id, 256_010.0, 256_266.0, None);
        m.set_track_altitude(2000.0);
        m.agent_found(id, 256_010.0, 256_266.0, None);
        assert_eq!(m.track.as_ref().map(|t| t.z), Some(2000.0));
        m.teleport_to_avatar(id, "Remote friend");
        m.agent_found(id, 256_010.0, 256_266.0, None);
        assert_eq!(m.track.as_ref().map(|t| t.z), Some(0.0));
        m.set_track_altitude(3000.0);
        let other = Uuid::from_u128(8);
        m.track_avatar(other, "Other friend");
        m.agent_found(other, 256_010.0, 256_266.0, None);
        assert_eq!(m.track.as_ref().map(|t| t.z), Some(0.0));
    }

    #[test]
    fn contact_teleport_with_known_position_retains_the_friend_for_arrival_flight() {
        let target = aurora_net::origin_to_handle(256_000, 256_256);
        for main in [target, aurora_net::origin_to_handle(256_000, 256_000)] {
            let (mut w, id) = remote_friend();
            w.main_region = Some(main);
            w.map.apply_blocks(vec![yiff()]);
            w.coarse.insert(target, vec![(id, glam::Vec3::new(10.0, 10.0, 25.0))]);
            assert!(w.locate_avatar(id, true));
            assert!(w.map.avatar.is_none(), "the direct action tracks a position");
            let out = w.take_map_commands();
            let command = out.iter().find(|c| matches!(c, NetCommand::TeleportTo { .. })).unwrap();
            assert_eq!(w.friend_teleport_avatar(command), Some(id));
            assert!(w.prepare_friend_join(command).is_none(), "a known height needs no second hop");
            assert!(out.iter().all(|c| !matches!(c, NetCommand::FindAgent(_))));
            w.social.friends[0].rights_has = 0;
            assert_eq!(w.friend_teleport_avatar(command), None, "recheck received rights");
            w.social.friends[0].rights_has = super::super::social::rights::MAP_LOCATION;
            w.map.set_track_altitude(30.0);
            assert_eq!(w.friend_teleport_avatar(command), None, "an explicit height ends the friend intent");
            w.locate_avatar(id, true);
            w.map.clear_track();
            assert_eq!(
                w.friend_teleport_avatar(command),
                None,
                "a cleared destination cannot retain the friend"
            );
        }
    }

    #[test]
    fn mapped_friend_with_known_height_keeps_flight_intent_until_an_explicit_destination() {
        let (mut w, id) = remote_friend();
        w.main_region = Some(aurora_net::origin_to_handle(256_000, 256_000));
        w.map.apply_blocks(vec![yiff()]);
        w.map.track_avatar(id, "Remote friend");
        w.map.agent_found(id, 256_010.0, 256_266.0, Some(25.0));
        let command = teleport_command(256_010.0, 256_266.0, 25.0);
        assert_eq!(w.friend_teleport_avatar(&command), Some(id));
        assert!(w.prepare_friend_join(&command).is_none());
        assert_eq!(w.friend_teleport_avatar(&teleport_command(256_020.0, 256_266.0, 25.0)), None);
        assert_eq!(
            w.friend_teleport_avatar(&NetCommand::AcceptLure {
                lure_id: id,
                godlike: false
            }),
            None
        );
        w.map.set_track_altitude(25.0);
        assert_eq!(w.friend_teleport_avatar(&command), None);
    }

    #[test]
    fn remote_friend_teleport_waits_for_the_matching_reply_and_region_only_once() {
        for known in [false, true] {
            let (mut w, id) = remote_friend();
            if known {
                w.map.apply_blocks(vec![yiff()]);
            }
            assert!(w.locate_avatar(id, true));
            assert!(matches!(&w.take_map_commands()[..], [NetCommand::TrackAgent(a), NetCommand::FindAgent(b)] if *a == id && *b == id));
            w.map.agent_found(Uuid::from_u128(8), 256_010.0, 256_266.0, Some(25.0));
            assert!(w.map.track.is_none());
            w.map.agent_found(id, 256_010.0, 256_266.0, Some(25.0));
            let mut out = w.take_map_commands();
            // Duplicate replies cannot cancel or repeat a pending teleport.
            w.map.agent_found(id, 256_010.0, 256_266.0, Some(25.0));
            w.map.apply_blocks(vec![yiff()]);
            out.extend(w.take_map_commands());
            let teleports: Vec<_> = out.iter().filter(|c| matches!(c, NetCommand::TeleportTo { .. })).collect();
            assert_eq!(teleports.len(), 1, "known region: {known}: {out:?}");
            assert!(matches!(teleports[0], NetCommand::TeleportTo { position, .. } if *position == glam::Vec3::new(10.0, 10.0, 25.0)));
            w.map.agent_found(id, 256_020.0, 256_270.0, Some(25.0));
            assert!(w.take_map_commands().iter().all(|c| !matches!(c, NetCommand::TeleportTo { .. })));
        }
    }

    #[test]
    fn remote_friend_revocation_cancels_pending_and_queued_teleports() {
        for phase in 0..3 {
            for offline in [false, true] {
                let (mut w, id) = remote_friend();
                w.map.apply_blocks(vec![yiff()]);
                w.locate_avatar(id, true);
                if phase > 0 {
                    w.take_map_commands();
                }
                if phase == 2 {
                    w.map.agent_found(id, 256_010.0, 256_266.0, Some(25.0));
                }
                if offline {
                    w.social.set_online(&[id], false);
                } else {
                    w.apply(aurora_net::NetEvent::UserRights {
                        agent: id,
                        rights: vec![(w.agent_id, 0)],
                    });
                }
                let out = w.take_map_commands();
                assert!(
                    out.iter()
                        .all(|c| !matches!(c, NetCommand::TeleportTo { .. } | NetCommand::FindAgent(_)))
                );
                assert!(w.map.avatar.is_none() && w.map.track.is_none());
                w.map.agent_found(id, 256_010.0, 256_266.0, Some(25.0));
                assert!(w.map.track.is_none(), "late reply must not recreate a revoked track");
            }
        }
    }

    #[test]
    fn remote_friend_teleport_expires_or_is_cancelled_by_map_navigation() {
        let id = Uuid::from_u128(7);
        for cancel in 0..4 {
            let mut m = WorldMap::default();
            m.apply_blocks(vec![yiff()]);
            m.teleport_to_avatar(id, "Remote friend");
            m.take_commands();
            match cancel {
                0 => *m.avatar.as_mut().unwrap().teleport.as_mut().unwrap() -= FIND_AGENT_EVERY,
                1 => m.track_avatar(id, "Remote friend"),
                2 => m.clear_track(),
                _ => m.track_location(256_010.0, 256_266.0, 25.0, false),
            }
            m.agent_found(id, 256_010.0, 256_266.0, Some(25.0));
            assert!(m.take_commands().iter().all(|c| !matches!(c, NetCommand::TeleportTo { .. })));
        }
    }

    #[test]
    fn nearby_avatar_location_still_works_without_friendship() {
        let (mut w, id) = remote_friend();
        let handle = aurora_net::origin_to_handle(256_000, 256_256);
        let p = glam::Vec3::new(20.0, 40.0, 25.0);
        w.main_region = Some(handle);
        w.coarse.insert(handle, vec![(id, p)]);
        w.social.friends.clear();
        assert_eq!(w.avatar_map_position(id), Some(p));
        let (ox, oy) = w.main_origin().unwrap();
        assert!(w.can_locate_avatar(id));
        for teleport in [false, true] {
            assert!(w.locate_avatar(id, teleport));
            let t = w.map.track.as_ref().unwrap();
            assert_eq!((t.x, t.y, t.z), (ox as f64 + p.x as f64, oy as f64 + p.y as f64, p.z));
            assert!(w.map.avatar.is_none());
            assert!(w.take_map_commands().iter().all(|c| !matches!(c, NetCommand::FindAgent(_))));
        }
    }

    #[test]
    fn clearing_a_pending_place_does_not_recreate_it_when_the_reply_arrives() {
        let mut m = WorldMap::default();
        m.map_destination("YIFF", glam::Vec3::new(10.0, 20.0, 30.0), aurora_net::beacon::FOCUS_MAP);
        m.clear_track();
        m.apply_blocks(vec![yiff()]);
        assert!(m.track.is_none());
        assert!(!m.focus_track);
    }

    #[test]
    fn arrival_keeps_the_existing_horizontal_and_vertical_thresholds() {
        let mut m = WorldMap::default();
        m.apply_blocks(vec![yiff()]);
        m.track_region("YIFF", glam::Vec3::new(10.0, 20.0, 30.0));
        let t = m.track.as_ref().expect("tracked").clone();
        m.check_arrival(t.x + 3.0, t.y, t.z);
        assert!(m.track.is_some());
        m.check_arrival(t.x, t.y, t.z + 6.0);
        assert!(m.track.is_some());
        m.check_arrival(t.x + 2.9, t.y, t.z + 5.9);
        assert!(m.track.is_none());
    }

    #[test]
    fn home_from_login() {
        let mut m = WorldMap::default();
        m.set_home_from_login("{'region_handle':[r256000, r256512], 'position':[r33.5, r40, r22.75], 'look_at':[r1, r0, r0]}");
        assert_eq!(m.home, Some((256033.5, 256552.0, 22.75)));
    }

    #[test]
    fn place_link_waits_for_its_region() {
        let mut m = WorldMap::default();
        assert_eq!(m.region_by_name("Ahern"), None);
        m.track_region("Ahern", glam::Vec3::new(10.0, 20.0, 30.0));
        assert!(matches!(&m.take_commands()[..], [NetCommand::MapNameRequest { name }] if name == "ahern"));
        assert!(m.track.is_none());
        m.apply_blocks(vec![MapBlock {
            x: 1000,
            y: 1001,
            name: "Ahern".into(),
            access: 13,
            region_flags: 0,
            water_height: 20,
            agents: 0,
            map_image_id: uuid::Uuid::nil(),
            size_x: 256,
            size_y: 256,
        }]);
        let t = m.track.as_ref().expect("tracked");
        assert_eq!((t.x, t.y, t.z), (256010.0, 256276.0, 30.0));
        assert_eq!(m.region_by_name(" AHERN "), Some((1000, 1001)));
        // known now: tracked at once
        m.track = None;
        m.track_region("ahern", glam::Vec3::new(1.0, 2.0, 3.0));
        assert!(m.take_commands().iter().all(|c| !matches!(c, NetCommand::MapNameRequest { .. })));
        assert!(m.track.is_some());
    }

    #[test]
    fn the_name_search_terminator_is_not_a_region() {
        let block = |x, y, name: &str, access| MapBlock {
            x,
            y,
            name: name.into(),
            access,
            region_flags: 0,
            water_height: 20,
            agents: 0,
            map_image_id: uuid::Uuid::nil(),
            size_x: 256,
            size_y: 256,
        };
        let mut m = WorldMap::default();
        m.track_region("alphatribe island", glam::Vec3::new(168.0, 34.0, 21.0));
        m.take_commands();
        // the reply to MapNameRequest: the match, then the closing block
        // that repeats the requested name with access 255
        m.apply_blocks(vec![
            block(1000, 1001, "Alphatribe Island", SIM_ACCESS_PG),
            block(0, 0, "alphatribe island", SIM_ACCESS_NONE),
        ]);
        assert_eq!(m.sims.len(), 1);
        assert!(!m.sims.contains_key(&(0, 0)));
        assert!(!m.null_sims.contains(&(0, 0)), "a named block says nothing of its slot");
        assert_eq!(m.region_by_name("alphatribe island"), Some((1000, 1001)));
        let t = m.track.as_ref().expect("tracked");
        assert_eq!((t.x, t.y), (256_168.0, 256_290.0));
        // nor does it replace a region already known at its slot
        m.apply_blocks(vec![block(1000, 1001, "alphatribe island", SIM_ACCESS_NONE)]);
        assert_eq!(m.sims[&(1000, 1001)].access, SIM_ACCESS_PG);
    }

    #[test]
    fn a_followed_friend_is_asked_again_and_dropped_by_a_new_track() {
        let friend = Uuid::from_u128(7);
        let mut m = WorldMap::default();
        m.apply_blocks(vec![yiff()]);
        m.track_avatar(friend, "Nova");
        assert!(matches!(
            &m.take_commands()[..],
            [NetCommand::TrackAgent(a), NetCommand::FindAgent(b)] if *a == friend && *b == friend
        ));
        // asked once: the same friend again sends nothing
        m.track_avatar(friend, "Nova");
        assert!(m.take_commands().is_empty());
        assert!(m.track.is_none() && m.focus_track);
        // somebody else's answer is not ours
        m.agent_found(Uuid::from_u128(8), 256_010.0, 256_266.0, Some(25.0));
        assert!(m.track.is_none());
        m.agent_found(friend, 256_010.0, 256_266.0, Some(25.0));
        let t = m.track.as_ref().expect("following");
        assert_eq!((t.x, t.y, t.z, t.label.as_str()), (256_010.0, 256_266.0, 25.0, "Nova"));
        assert!(m.avatar.as_ref().is_some_and(|a| a.found));
        // a friend is not dropped on arrival, the height of the card stays
        m.check_arrival(256_010.0, 256_266.0, 25.0);
        assert!(m.track.is_some());
        m.set_track_altitude(40.0);
        m.agent_found(friend, 256_020.0, 256_270.0, Some(25.0));
        assert_eq!(m.track.as_ref().map(|t| (t.x, t.z)), Some((256_020.0, 40.0)));
        // FIND_FREQUENCY: asked again later
        m.avatar.as_mut().expect("following").asked -= FIND_AGENT_EVERY;
        assert!(matches!(&m.take_commands()[..], [NetCommand::FindAgent(a)] if *a == friend));
        assert!(m.take_commands().is_empty());
        // a spot clicked on the map ends the following
        m.track_location(256_100.0, 256_300.0, 20.0, false);
        assert!(m.avatar.is_none());
        assert!(matches!(&m.take_commands()[..], [NetCommand::TrackAgent(a)] if a.is_nil()));
        m.track_avatar(friend, "Nova");
        m.take_commands();
        m.clear_track();
        assert!(m.track.is_none() && m.avatar.is_none());
        assert!(matches!(&m.take_commands()[..], [NetCommand::TrackAgent(a)] if a.is_nil()));
    }

    #[test]
    fn the_parcel_of_a_spot_is_asked_once_like_firestorm() {
        let mut m = WorldMap::default();
        m.apply_blocks(vec![yiff()]);
        m.track_location(256_010.0, 256_266.0, 25.0, false);
        m.take_commands();
        m.want_track_parcel();
        m.want_track_parcel();
        let asked = m.take_commands();
        let [NetCommand::RemoteParcelRequest { handle, position, .. }] = &asked[..] else {
            panic!("one RemoteParcelRequest: {asked:?}");
        };
        assert_eq!(*handle, aurora_net::origin_to_handle(256_000, 256_256));
        assert_eq!(*position, glam::Vec3::new(10.0, 10.0, 25.0));
        let parcel = Uuid::from_u128(9);
        // another point's answer is not ours
        m.apply_remote_parcel(*handle, glam::Vec3::ZERO, Some(parcel));
        assert_eq!(m.track_parcel_id(), None);
        m.apply_remote_parcel(*handle, *position, Some(parcel));
        assert!(matches!(&m.take_commands()[..], [NetCommand::ParcelInfoRequest(p)] if *p == parcel));
        assert_eq!(m.track_parcel_id(), Some(parcel));
        // another spot: asked again, the former parcel is not its
        m.track_location(256_100.0, 256_300.0, 25.0, false);
        assert_eq!(m.track_parcel_id(), None);
        m.want_track_parcel();
        assert!(matches!(&m.take_commands()[..], [NetCommand::RemoteParcelRequest { .. }]));
    }

    fn yiff() -> MapBlock {
        MapBlock {
            x: 1000,
            y: 1001,
            name: "YIFF".into(),
            access: 21,
            region_flags: 0,
            water_height: 20,
            agents: 0,
            map_image_id: uuid::Uuid::nil(),
            size_x: 256,
            size_y: 256,
        }
    }

    #[test]
    fn map_destination_tracks_whole_metres_and_only_shows_the_map() {
        use aurora_net::beacon::{FOCUS_MAP, SHOW_MAP};
        let mut m = WorldMap::default();
        m.apply_blocks(vec![yiff()]);
        // trackURL: (S32) of each coordinate, height within 0..4096 m
        assert!(m.map_destination("yiff", glam::Vec3::new(168.9, 149.6, 5000.7), SHOW_MAP | FOCUS_MAP));
        let t = m.track.as_ref().expect("tracked");
        assert_eq!((t.x, t.y, t.z, t.teleport), (256168.0, 256405.0, 4096.0, false));
        assert!(m.focus_track);
        assert!(m.take_commands().iter().all(|c| !matches!(c, NetCommand::TeleportTo { .. })));
        // without BEACON_SHOW_MAP the place is tracked, the map stays closed
        assert!(!m.map_destination("YIFF", glam::Vec3::new(10.0, 20.0, 30.0), FOCUS_MAP));
        assert!(!m.focus_track);
        assert_eq!(m.track.as_ref().map(|t| (t.x, t.y)), Some((256010.0, 256276.0)));
        assert!(m.map_destination("YIFF", glam::Vec3::new(10.0, 20.0, -3.0), SHOW_MAP));
        assert!(!m.focus_track);
        assert_eq!(m.track.as_ref().map(|t| t.z), Some(0.0));
    }

    #[test]
    fn map_destination_in_an_unknown_region_waits_for_it_before_centring() {
        let mut m = WorldMap::default();
        assert!(m.map_destination(
            "YIFF",
            glam::Vec3::new(168.0, 149.0, 118.0),
            aurora_net::beacon::SHOW_MAP | aurora_net::beacon::FOCUS_MAP
        ));
        assert!(matches!(&m.take_commands()[..], [NetCommand::MapNameRequest { name }] if name == "yiff"));
        assert!(m.track.is_none() && m.focus_track);
        m.apply_blocks(vec![yiff()]);
        let t = m.track.as_ref().expect("tracked once the region is known");
        assert_eq!((t.x, t.y, t.z), (256168.0, 256405.0, 118.0));
        assert!(m.focus_track, "the open map centres on it next frame");
    }

    fn lookups(m: &mut WorldMap) -> Vec<String> {
        m.take_commands()
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::MapNameRequest { name } => Some(name),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_region_rating_is_asked_once_and_learnt_from_the_answer() {
        let mut m = WorldMap::default();
        assert_eq!(m.ratings.want("YIFF"), None);
        assert_eq!(m.ratings.want(" yiff "), None);
        assert_eq!(lookups(&mut m), vec!["yiff".to_owned()]);
        // shown again before and after the answer: never asked again
        assert_eq!(m.ratings.want("Yiff"), None);
        assert!(lookups(&mut m).is_empty());
        m.apply_blocks(vec![yiff()]);
        assert_eq!(m.ratings.want("yiff"), Some(21));
        assert_eq!(m.ratings.get("YIFF"), Some(21));
        assert!(lookups(&mut m).is_empty());
    }

    #[test]
    fn known_regions_are_never_asked() {
        let mut m = WorldMap::default();
        m.ratings.learn("Aurora Démo", 13);
        m.apply_blocks(vec![yiff()]);
        assert_eq!(m.ratings.want("aurora démo"), Some(13));
        assert_eq!(m.ratings.want("Yiff"), Some(21));
        assert_eq!(m.ratings.want(""), None);
        assert!(lookups(&mut m).is_empty());
    }

    #[test]
    fn a_burst_of_links_is_asked_a_few_a_second() {
        let mut r = RegionRatings::default();
        for i in 0..10 {
            r.want(&format!("Region {i}"));
        }
        let t0 = Instant::now();
        let step = Duration::from_secs(1) / RATING_REQUESTS_PER_SECOND;
        assert!(r.next_request(t0).is_some());
        assert!(r.next_request(t0).is_none());
        assert!(r.next_request(t0 + step / 2).is_none());
        // over one second, RATING_REQUESTS_PER_SECOND names at most
        let sent = (1..=RATING_REQUESTS_PER_SECOND)
            .filter(|k| r.next_request(t0 + step * *k).is_some())
            .count() as u32;
        assert_eq!(sent, RATING_REQUESTS_PER_SECOND);
        let Some(NetCommand::MapNameRequest { name }) = r.next_request(t0 + step * 10) else {
            panic!("the queue keeps its order");
        };
        assert_eq!(name, format!("region {}", RATING_REQUESTS_PER_SECOND + 1));
    }

    #[test]
    fn a_full_queue_drops_its_oldest_names_which_may_be_asked_again() {
        let mut r = RegionRatings::default();
        for i in 0..RATING_QUEUE_MAX + 2 {
            r.want(&format!("r{i}"));
        }
        assert_eq!(r.queue.len(), RATING_QUEUE_MAX);
        assert_eq!(r.queue.front().map(String::as_str), Some("r2"));
        // dropped before being asked: its link asks it again
        r.want("r0");
        assert_eq!(r.queue.back().map(String::as_str), Some("r0"));
        assert_eq!(r.queue.len(), RATING_QUEUE_MAX);
    }

    #[test]
    fn overlay_quarters() {
        let mut m = WorldMap::default();
        m.apply_overlay(1, (256, 256), 2, &[7; 1024]);
        let o = &m.overlays[&1];
        assert_eq!((o.0, o.1, o.2.len()), (64, 64, 4096));
        assert_eq!(o.2[2047], 0);
        assert_eq!(o.2[2048], 7);
        assert_eq!(o.2[3071], 7);
        assert_eq!(o.2[3072], 0);
    }
}
