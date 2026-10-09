//! World map data (LLWorldMap / LLWorldMapMessage): regions known by grid
//! position, map items (hubs, people, land for sale, events), the tracked
//! location (LLTracker) and the parcel overlays used for the mini-map's
//! property lines (LLViewerParcelOverlay).
//!
//! Ported from Firestorm's llworldmap.cpp / llworldmapmessage.cpp (originally LGPL 2.1,
//! Linden Research, Inc.).

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

    /// "Bac à sable", "Non sécurisé" (LLWorldMapView tooltip flags).
    pub fn flag_labels(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.flags & REGION_FLAGS_SANDBOX != 0 {
            v.push("Bac à sable");
        }
        if self.flags & REGION_FLAGS_ALLOW_DAMAGE != 0 {
            v.push("Non sécurisé");
        }
        v
    }
}

pub fn access_label(access: u8) -> &'static str {
    match access {
        0..=SIM_ACCESS_PG => "Général",
        SIM_ACCESS_MATURE => "Modéré",
        SIM_ACCESS_ADULT => "Adulte",
        SIM_ACCESS_DOWN => "Hors ligne",
        _ => "Inconnu",
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
    out: Vec<NetCommand>,
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

    /// LLTracker: stop tracking once within 3 m of the spot.
    pub fn check_arrival(&mut self, x: f64, y: f64, z: f32) {
        if let Some(t) = &self.track
            && !t.pending
            && !t.invalid
            && (t.x - x).hypot(t.y - y) < 3.0
            && (t.z - z).abs() * 0.5 < 3.0
        {
            self.track = None;
        }
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
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
            if b.name.is_empty() {
                // only "non-existent" answers are kept (insertRegion)
                if b.access == SIM_ACCESS_NONE {
                    self.null_sims.insert(key);
                    self.sims.remove(&key);
                }
                continue;
            }
            self.null_sims.remove(&key);
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

    /// LLFloaterWorldMap::trackLocation: point at a global position; an
    /// unknown region is asked first (MAP_SIM_RETURN_NULL_SIMS).
    pub fn track_location(&mut self, x: f64, y: f64, z: f32, teleport: bool) {
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

    fn track_known_region(&mut self, lower: &str, pos: glam::Vec3) -> bool {
        let Some(&(sx, sy)) = self.sims.iter().find(|(_, s)| s.name.to_lowercase() == lower).map(|(k, _)| k) else {
            return false;
        };
        self.track_location(sx as f64 * 256.0 + pos.x as f64, sy as f64 * 256.0 + pos.y as f64, pos.z, false);
        true
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

/// "Région (x, y, z)" (LLFloaterWorldMap::trackLocation).
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

    #[test]
    fn home_from_login() {
        let mut m = WorldMap::default();
        m.set_home_from_login("{'region_handle':[r256000, r256512], 'position':[r33.5, r40, r22.75], 'look_at':[r1, r0, r0]}");
        assert_eq!(m.home, Some((256033.5, 256552.0, 22.75)));
    }

    #[test]
    fn place_link_waits_for_its_region() {
        let mut m = WorldMap::default();
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
        // known now: tracked at once
        m.track = None;
        m.track_region("ahern", glam::Vec3::new(1.0, 2.0, 3.0));
        assert!(m.take_commands().iter().all(|c| !matches!(c, NetCommand::MapNameRequest { .. })));
        assert!(m.track.is_some());
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
