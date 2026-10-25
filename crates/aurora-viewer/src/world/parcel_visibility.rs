//! Viewer-local parcel isolation. Parcel membership follows
//! LLViewerParcelMgr::inAgentParcel (indra/newview/llviewerparcelmgr.cpp).
//! The configurable isolation and connected same-owner parcels are Aurora options.

use super::World;
use aurora_net::{NetCommand, ParcelInfo, RegionHandle, build::BuildCmd};
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const FIRST_SEQUENCE: i32 = 1_000_000;
const MAX_REQUESTS: usize = 4;
const MAX_PARCELS: usize = 256;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Off,
    Always,
    Listed,
    ExceptListed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub region: Uuid,
    pub local_id: i32,
    pub name: String,
    pub region_name: String,
}

impl Entry {
    pub fn same_parcel(&self, other: &Self) -> bool {
        self.region == other.region && self.local_id == other.local_id
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub same_owner: bool,
    pub parcels: Vec<Entry>,
}

impl Settings {
    fn applies(&self, entry: &Entry) -> bool {
        let listed = self.parcels.iter().any(|p| p.same_parcel(entry));
        match self.mode {
            Mode::Off => false,
            Mode::Always => true,
            Mode::Listed => listed,
            Mode::ExceptListed => !listed,
        }
    }
}

#[derive(Debug, Clone)]
struct Mask {
    handle: RegionHandle,
    size: (u32, u32),
    parcel: Arc<ParcelInfo>,
}

impl Mask {
    fn contains(&self, p: DVec3) -> bool {
        let (ox, oy) = aurora_net::handle_to_origin(self.handle);
        let (x, y) = (p.x - f64::from(ox), p.y - f64::from(oy));
        if !p.is_finite() || !(0.0..f64::from(self.size.0)).contains(&x) || !(0.0..f64::from(self.size.1)).contains(&y) {
            return false;
        }
        let i = y as usize / 4 * (self.size.0 as usize / 4) + x as usize / 4;
        self.parcel.bitmap.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
    }

    fn valid(&self) -> bool {
        let (x, y) = self.size;
        x > 0
            && y > 0
            && x <= 4096
            && y <= 4096
            && x.is_multiple_of(4)
            && y.is_multiple_of(4)
            && self.parcel.bitmap.len() == (x as usize / 4 * (y as usize / 4)).div_ceil(8)
            && self.parcel.bitmap.iter().any(|b| *b != 0)
    }
}

type Cell = (RegionHandle, u32, u32);

#[derive(Debug)]
struct Request {
    cell: Cell,
    sent: Instant,
}

#[derive(Debug, Default)]
pub struct Visibility {
    settings: Settings,
    current: Option<Mask>,
    allowed: Vec<Mask>,
    known: Vec<Mask>,
    frontier: VecDeque<Cell>,
    seen: HashSet<Cell>,
    pending: HashMap<i32, Request>,
    attempts: HashMap<Cell, u8>,
    next_sequence: i32,
    next_request: Option<Instant>,
    region_count: usize,
    /// Changes only when the visible parcel mask changes.
    pub version: u64,
}

impl Visibility {
    pub fn invalidate(&mut self) {
        if self.active() {
            self.reset();
        }
    }
    pub fn active(&self) -> bool {
        !self.allowed.is_empty()
    }

    pub fn contains(&self, position: DVec3) -> bool {
        !self.active() || self.allowed.iter().any(|m| m.contains(position))
    }

    fn reset(&mut self) {
        self.current = None;
        self.allowed.clear();
        self.known.clear();
        self.frontier.clear();
        self.seen.clear();
        self.pending.clear();
        self.attempts.clear();
        self.next_request = None;
        self.version = self.version.wrapping_add(1);
    }

    fn refresh(&mut self, world: &World, settings: &Settings, now: Instant) -> Vec<NetCommand> {
        let candidate = (settings.mode != Mode::Off)
            .then(|| world.current_parcel_entry())
            .flatten()
            .filter(|p| settings.applies(p))
            .and_then(|_| {
                let handle = world.main_region?;
                let region = world.regions.get(&handle)?;
                let mask = Mask {
                    handle,
                    size: (region.heightmap.size_x, region.heightmap.size_y),
                    parcel: world.parcel.clone()?,
                };
                // AgentParcel can still describe the previous parcel during a crossing.
                let validated = self.current.as_ref().is_some_and(|current| {
                    current.handle == mask.handle && current.size == mask.size && Arc::ptr_eq(&current.parcel, &mask.parcel)
                });
                ((validated || mask.valid()) && mask.contains(world.agent_global()?)).then_some(mask)
            });
        let unchanged = self.settings == *settings
            && (!settings.same_owner || candidate.is_none() || self.region_count == world.regions.len())
            && match (&self.current, &candidate) {
                (None, None) => true,
                (Some(a), Some(b)) => a.handle == b.handle && a.size == b.size && Arc::ptr_eq(&a.parcel, &b.parcel),
                _ => false,
            };
        if !unchanged {
            self.reset();
            self.settings.clone_from(settings);
            self.region_count = world.regions.len();
            self.current = candidate.clone();
            if let Some(mask) = candidate {
                self.add_mask(world, mask);
            }
        }
        if !self.active() || !settings.same_owner {
            return Vec::new();
        }
        // Bounded background discovery; never select land or change the agent parcel.
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, r)| now.saturating_duration_since(r.sent) >= Duration::from_secs(10))
            .map(|(&sequence, r)| (sequence, r.cell))
            .collect();
        for (sequence, cell) in expired {
            self.pending.remove(&sequence);
            if self.attempts.get(&cell).copied().unwrap_or(0) < 2 {
                self.seen.remove(&cell);
                self.frontier.push_front(cell);
            }
        }
        if self.pending.len() >= MAX_REQUESTS || self.next_request.is_some_and(|t| now < t) || self.known.len() >= MAX_PARCELS {
            return Vec::new();
        }
        while let Some(cell @ (handle, x, y)) = self.frontier.pop_front() {
            let (ox, oy) = aurora_net::handle_to_origin(handle);
            let point = DVec3::new(f64::from(ox) + f64::from(x) + 2.0, f64::from(oy) + f64::from(y) + 2.0, 0.0);
            if self.known.iter().any(|m| m.contains(point)) {
                continue;
            }
            if !self.seen.insert(cell) {
                continue;
            }
            *self.attempts.entry(cell).or_default() += 1;
            self.next_sequence = FIRST_SEQUENCE + (self.next_sequence + 1 - FIRST_SEQUENCE).rem_euclid(1_000_000);
            let sequence = self.next_sequence;
            self.pending.insert(sequence, Request { cell, sent: now });
            self.next_request = Some(now + Duration::from_millis(150));
            return vec![NetCommand::Build(BuildCmd::ParcelRequest {
                handle,
                sequence,
                snap: true,
                west: x as f32,
                south: y as f32,
                east: (x + 4) as f32,
                north: (y + 4) as f32,
            })];
        }
        Vec::new()
    }

    fn add_mask(&mut self, world: &World, mask: Mask) {
        let cells_x = mask.size.0 / 4;
        let (ox, oy) = aurora_net::handle_to_origin(mask.handle);
        for (byte, bits) in mask.parcel.bitmap.iter().enumerate() {
            for bit in 0..8 {
                if bits & (1 << bit) == 0 {
                    continue;
                }
                let i = byte as u32 * 8 + bit;
                let (x, y) = (i % cells_x * 4, i / cells_x * 4);
                if self.settings.same_owner {
                    for (dx, dy) in [(-4, 0), (4, 0), (0, -4), (0, 4)] {
                        let p = DVec3::new(
                            f64::from(ox) + f64::from(x) + f64::from(dx) + 2.0,
                            f64::from(oy) + f64::from(y) + f64::from(dy) + 2.0,
                            0.0,
                        );
                        if mask.contains(p) {
                            continue;
                        }
                        if let Some(cell) = cell_at(world, p)
                            && !self.seen.contains(&cell)
                            && self.frontier.len() < 8192
                        {
                            self.frontier.push_back(cell);
                        }
                    }
                }
            }
        }
        self.known.push(mask.clone());
        self.allowed.push(mask);
        self.version = self.version.wrapping_add(1);
    }

    fn reply(&mut self, world: &World, handle: RegionHandle, sequence: i32, parcel: Arc<ParcelInfo>) -> bool {
        if sequence < FIRST_SEQUENCE {
            return false;
        }
        let Some(request) = self.pending.remove(&sequence) else {
            return true;
        };
        if request.cell.0 != handle {
            return true;
        }
        let Some(region) = world.regions.get(&handle) else {
            return true;
        };
        let mask = Mask {
            handle,
            size: (region.heightmap.size_x, region.heightmap.size_y),
            parcel,
        };
        let (ox, oy) = aurora_net::handle_to_origin(handle);
        let point = DVec3::new(
            f64::from(ox) + f64::from(request.cell.1) + 2.0,
            f64::from(oy) + f64::from(request.cell.2) + 2.0,
            0.0,
        );
        if !mask.valid() || !mask.contains(point) {
            return true;
        }
        // Retain a bitmap instead of a hash entry for every cell of a large region.
        let same_owner = self.current.as_ref().is_some_and(|current| {
            !current.parcel.owner_id.is_nil()
                && current.parcel.owner_id == mask.parcel.owner_id
                && current.parcel.is_group_owned == mask.parcel.is_group_owned
        });
        if same_owner
            && self.allowed.len() < MAX_PARCELS
            && !self
                .allowed
                .iter()
                .any(|m| m.handle == handle && m.parcel.local_id == mask.parcel.local_id)
        {
            self.add_mask(world, mask);
        } else if self.known.len() < MAX_PARCELS {
            self.known.push(mask);
        }
        true
    }
}

fn cell_at(world: &World, p: DVec3) -> Option<Cell> {
    world.regions.iter().find_map(|(&handle, region)| {
        let (ox, oy) = aurora_net::handle_to_origin(handle);
        let (x, y) = (p.x - f64::from(ox), p.y - f64::from(oy));
        ((0.0..f64::from(region.heightmap.size_x)).contains(&x) && (0.0..f64::from(region.heightmap.size_y)).contains(&y))
            .then(|| (handle, x as u32 / 4 * 4, y as u32 / 4 * 4))
    })
}

impl World {
    pub fn current_parcel_entry(&self) -> Option<Entry> {
        let parcel = self.parcel.as_ref()?;
        let region = self.main()?;
        let id = region.info.as_ref()?.region_id;
        if id.is_nil() || parcel.local_id <= 0 {
            return None;
        }
        Some(Entry {
            region: id,
            local_id: parcel.local_id,
            name: parcel.name.clone(),
            region_name: region.name.clone(),
        })
    }

    pub fn update_parcel_visibility(&mut self, settings: &Settings, now: Instant) -> Vec<NetCommand> {
        let mut visibility = std::mem::take(&mut self.parcel_visibility);
        let commands = visibility.refresh(self, settings, now);
        self.parcel_visibility = visibility;
        commands
    }

    pub fn parcel_visibility_reply(&mut self, handle: RegionHandle, sequence: i32, parcel: Arc<ParcelInfo>) -> bool {
        let mut visibility = std::mem::take(&mut self.parcel_visibility);
        let consumed = visibility.reply(self, handle, sequence, parcel);
        self.parcel_visibility = visibility;
        consumed
    }

    /// Whole linksets follow their root; attachments follow their wearer. Our
    /// avatar and HUDs stay available, including while seated across a boundary.
    pub fn parcel_object_visible(&self, index: usize) -> bool {
        self.parcel_object_visibility(index).unwrap_or(false)
    }

    /// None waits for parent data without preparing visual assets.
    pub(crate) fn parcel_object_visibility(&self, index: usize) -> Option<bool> {
        if !self.parcel_visibility.active() {
            return Some(true);
        }
        let mut index = index;
        for _ in 0..64 {
            let object = self.objects.get(index)?;
            if object.full_id == self.agent_id {
                return Some(true);
            }
            if object.is_avatar() || object.parent_id == 0 {
                let (pos, _, _) = crate::scene::Scene::object_transform_unshifted(self, index, Instant::now())?;
                let Some((ox, oy)) = self.main_origin() else {
                    return Some(true);
                };
                return Some(self.parcel_visibility.contains(DVec3::new(
                    f64::from(ox) + f64::from(pos.x),
                    f64::from(oy) + f64::from(pos.y),
                    f64::from(pos.z),
                )));
            }
            let parent = self.objects.parent_of(object)?;
            index = parent;
        }
        Some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn world() -> World {
        static LIB: std::sync::OnceLock<Arc<crate::scene::avatar::AvatarLibrary>> = std::sync::OnceLock::new();
        let lib = LIB.get_or_init(|| Arc::new(crate::scene::avatar::AvatarLibrary::load())).clone();
        let mut world = World::new(lib);
        for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            world.apply(event);
        }
        crate::demo::parcel_visibility::install(&mut world, &mut crate::settings::Settings::default(), "off");
        world.agent.position = Vec3::new(130.0, 130.0, 25.0);
        world
    }

    #[test]
    fn parcel_visibility_modes_use_identity_and_survive_settings_roundtrip() {
        let entry = Entry {
            region: Uuid::from_u128(1),
            local_id: 12,
            name: "Home".into(),
            region_name: "Region".into(),
        };
        let mut settings = Settings {
            parcels: vec![entry.clone()],
            ..Default::default()
        };
        let mut renamed = entry.clone();
        renamed.name = "New name".into();
        let mut other = entry.clone();
        other.region = Uuid::from_u128(2);
        for (mode, listed, unlisted) in [
            (Mode::Off, false, false),
            (Mode::Always, true, true),
            (Mode::Listed, true, false),
            (Mode::ExceptListed, false, true),
        ] {
            settings.mode = mode;
            assert_eq!(settings.applies(&renamed), listed);
            assert_eq!(settings.applies(&other), unlisted);
        }
        settings.same_owner = true;
        let encoded = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&encoded).unwrap(), settings);
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
    }

    #[test]
    fn parcel_visibility_bitmap_uses_exact_cells_region_and_unlimited_altitude() {
        let handle = aurora_net::origin_to_handle(256_000, 256_000);
        let mask = Mask {
            handle,
            size: (256, 256),
            parcel: crate::demo::parcel_visibility::parcel_at(130.0, 130.0),
        };
        assert!(mask.valid());
        let p = |x, y, z| DVec3::new(256_000.0 + x, 256_000.0 + y, z);
        assert!(mask.contains(p(128.0, 128.0, 4096.0)));
        assert!(!mask.contains(p(127.99, 128.0, 4096.0)));
        assert!(!mask.contains(p(256.0, 128.0, 0.0)));
        assert!(!mask.contains(p(128.0, 128.0, f64::NAN)));
        let mut rectangular = mask.clone();
        rectangular.size = (256, 128);
        Arc::make_mut(&mut rectangular.parcel).bitmap = vec![255; 256];
        assert!(rectangular.valid());
        assert!(rectangular.contains(p(255.0, 127.0, 20.0)));
        assert!(!rectangular.contains(p(255.0, 128.0, 20.0)));
        Arc::make_mut(&mut rectangular.parcel).bitmap.pop();
        assert!(!rectangular.valid());
    }

    #[test]
    fn parcel_visibility_follows_linkset_roots_and_preserves_our_attachments() {
        let mut world = world();
        let now = Instant::now();
        let settings = Settings {
            mode: Mode::Always,
            ..Default::default()
        };
        world.update_parcel_visibility(&settings, now);
        let root = world.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let child = world.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let parent_id = world.objects.get(root).unwrap().key.local_id;
        world.objects.get_mut(root).unwrap().position = Vec3::new(140.0, 140.0, 25.0);
        let o = world.objects.get_mut(child).unwrap();
        o.parent_id = parent_id;
        o.position = Vec3::new(-50.0, 0.0, 0.0);
        assert!(world.parcel_object_visible(child), "whole linkset follows its root");
        world.objects.get_mut(root).unwrap().position.x = 120.0;
        assert!(!world.parcel_object_visible(root));
        assert!(!world.parcel_object_visible(child));
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let own_id = world.objects.get(own).unwrap().key.local_id;
        world.objects.get_mut(root).unwrap().parent_id = own_id;
        assert!(world.parcel_object_visible(root), "our worn objects stay available");
        assert!(world.parcel_object_visible(own));
        world.update_parcel_visibility(&Settings::default(), now);
        assert!(world.parcel_object_visible(child), "switching off restores content immediately");
    }

    #[test]
    fn parcel_visibility_waits_for_the_parcel_under_the_avatar_and_changes_only_on_events() {
        let mut world = world();
        let inactive_version = world.parcel_visibility.version;
        world.update_parcel_visibility(&Settings::default(), Instant::now());
        assert_eq!(
            world.parcel_visibility.version, inactive_version,
            "disabled filtering does not invalidate the scene when regions arrive"
        );
        let settings = Settings {
            mode: Mode::Always,
            ..Default::default()
        };
        let now = Instant::now();
        world.update_parcel_visibility(&settings, now);
        assert!(world.parcel_visibility.active());
        let version = world.parcel_visibility.version;
        world.update_parcel_visibility(&settings, now);
        assert_eq!(world.parcel_visibility.version, version, "stationary frames do not rebuild objects");
        world.agent.position.x = 120.0;
        world.update_parcel_visibility(&settings, now);
        assert!(!world.parcel_visibility.active(), "old parcel data cannot hide the arrival parcel");
        world.parcel = Some(crate::demo::parcel_visibility::parcel_at(120.0, 130.0));
        world.update_parcel_visibility(&settings, now);
        assert!(world.parcel_visibility.active());
    }

    #[test]
    fn parcel_visibility_discovers_only_connected_same_owner_parcels_with_bounded_requests() {
        let mut world = world();
        let settings = Settings {
            mode: Mode::Always,
            same_owner: true,
            ..Default::default()
        };
        let start = Instant::now();
        let mut requests = 0;
        for step in 0..200 {
            let now = start + Duration::from_millis(step * 150);
            let commands = world.update_parcel_visibility(&settings, now);
            for command in commands {
                let NetCommand::Build(BuildCmd::ParcelRequest {
                    handle,
                    sequence,
                    west,
                    south,
                    ..
                }) = command
                else {
                    panic!("parcel request")
                };
                requests += 1;
                assert!(world.parcel_visibility_reply(handle, sequence, crate::demo::parcel_visibility::parcel_at(west, south)));
            }
            assert!(world.parcel_visibility.pending.len() <= MAX_REQUESTS);
        }
        assert_eq!(
            requests, 3,
            "each neighbouring parcel requested once, regardless of boundary length"
        );
        assert_eq!(world.parcel_visibility.allowed.len(), 2);
        assert!(world.parcel_visibility.contains(DVec3::new(256_120.0, 256_140.0, 1000.0)));
        assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
        world.update_parcel_visibility(&Settings::default(), start);
        assert!(world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
    }

    #[test]
    fn parcel_visibility_rejects_wrong_region_late_and_foreign_owner_replies() {
        let mut world = world();
        let now = Instant::now();
        let settings = Settings {
            mode: Mode::Always,
            same_owner: true,
            ..Default::default()
        };
        let command = world.update_parcel_visibility(&settings, now).pop().unwrap();
        let NetCommand::Build(BuildCmd::ParcelRequest {
            handle,
            sequence,
            west,
            south,
            ..
        }) = command
        else {
            panic!("parcel request")
        };
        assert!(world.parcel_visibility_reply(handle + 256, sequence, crate::demo::parcel_visibility::parcel_at(west, south)));
        assert_eq!(world.parcel_visibility.allowed.len(), 1);
        world.update_parcel_visibility(&Settings::default(), now);
        assert!(world.parcel_visibility_reply(handle, sequence, crate::demo::parcel_visibility::parcel_at(west, south)));
        assert!(!world.parcel_visibility.active());
        assert!(
            !world.parcel_visibility_reply(handle, 1001, crate::demo::parcel_visibility::parcel_at(west, south)),
            "build replies keep their existing route"
        );
    }

    #[test]
    fn parcel_visibility_includes_group_owned_neighbours_across_connected_region_borders() {
        let mut world = world();
        let main = world.main_region.unwrap();
        let (ox, oy) = aurora_net::handle_to_origin(main);
        let neighbour = aurora_net::origin_to_handle(ox + 256, oy);
        world.ensure_region(neighbour, (256, 256));
        let owner = Uuid::from_u128(946);
        let current = Arc::make_mut(world.parcel.as_mut().unwrap());
        current.owner_id = owner;
        current.is_group_owned = true;
        let settings = Settings {
            mode: Mode::Always,
            same_owner: true,
            ..Default::default()
        };
        let start = Instant::now();
        for step in 0..200 {
            for command in world.update_parcel_visibility(&settings, start + Duration::from_millis(step * 150)) {
                let NetCommand::Build(BuildCmd::ParcelRequest {
                    handle,
                    sequence,
                    west,
                    south,
                    ..
                }) = command
                else {
                    panic!("parcel request")
                };
                let parcel = if handle == neighbour {
                    Arc::new(ParcelInfo {
                        local_id: 2,
                        owner_id: owner,
                        is_group_owned: true,
                        bitmap: vec![255; 512],
                        ..Default::default()
                    })
                } else {
                    crate::demo::parcel_visibility::parcel_at(west, south)
                };
                world.parcel_visibility_reply(handle, sequence, parcel);
            }
        }
        assert!(
            world
                .parcel_visibility
                .contains(DVec3::new(f64::from(ox + 260), f64::from(oy + 140), 25.0))
        );
        assert!(
            !world
                .parcel_visibility
                .contains(DVec3::new(f64::from(ox + 120), f64::from(oy + 140), 25.0)),
            "resident-owned neighbour does not match a group owner"
        );
    }

    #[test]
    fn parcel_visibility_retries_a_lost_reply_once_and_bounds_in_flight_requests() {
        let mut world = world();
        let settings = Settings {
            mode: Mode::Always,
            same_owner: true,
            ..Default::default()
        };
        let start = Instant::now();
        let mut requests = 0;
        for step in 0..500 {
            requests += world
                .update_parcel_visibility(&settings, start + Duration::from_millis(step * 150))
                .len();
            assert!(world.parcel_visibility.pending.len() <= MAX_REQUESTS);
            assert!(world.parcel_visibility.attempts.values().all(|n| *n <= 2));
        }
        assert!(requests > MAX_REQUESTS, "timed-out requests release their slots");
        assert!(
            world.parcel_visibility.attempts.values().any(|n| *n == 2),
            "a lost response is retried once"
        );
    }
}
