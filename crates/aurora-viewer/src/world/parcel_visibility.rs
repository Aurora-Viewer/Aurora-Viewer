//! Viewer-local parcel isolation. Parcel membership follows
//! LLViewerParcelMgr::inAgentParcel (indra/newview/llviewerparcelmgr.cpp).
//! The configurable isolation and connected same-owner parcels are Aurora options.

use super::World;
use aurora_net::{NetCommand, ParcelInfo, RegionHandle, build::BuildCmd};
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
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
    pub same_group: bool,
    pub parcels: Vec<Entry>,
    /// Rendering exceptions, independent of the parcels where isolation applies.
    pub always_visible: Vec<Entry>,
}

impl Settings {
    fn automatic_neighbours(&self) -> bool {
        self.same_owner || self.same_group
    }

    fn includes_related(&self, current: &ParcelInfo, other: &ParcelInfo) -> bool {
        let owner = self.same_owner
            && !current.owner_id.is_nil()
            && current.owner_id == other.owner_id
            && current.is_group_owned == other.is_group_owned;
        let group = |parcel: &ParcelInfo| {
            let id = if parcel.is_group_owned && !parcel.owner_id.is_nil() {
                parcel.owner_id
            } else {
                parcel.group_id
            };
            (!id.is_nil()).then_some(id)
        };
        owner || self.same_group && group(current).zip(group(other)).is_some_and(|(a, b)| a == b)
    }

    fn applies(&self, region: Uuid, local_id: i32) -> bool {
        let listed = self.parcels.iter().any(|p| p.region == region && p.local_id == local_id);
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
    region: Uuid,
    size: (u32, u32),
    parcel: Arc<ParcelInfo>,
}

impl Mask {
    fn connected(&self, world: &World) -> bool {
        world.regions.get(&self.handle).is_some_and(|region| {
            region.info.as_ref().is_some_and(|info| info.region_id == self.region)
                && self.size == (region.heightmap.size_x, region.heightmap.size_y)
        })
    }

    fn same_bounds(&self, other: &Self) -> bool {
        self.handle == other.handle
            && self.region == other.region
            && self.size == other.size
            && self.parcel.local_id == other.parcel.local_id
            && (Arc::ptr_eq(&self.parcel, &other.parcel) || self.parcel.bitmap == other.parcel.bitmap)
    }

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
    /// None discovers a neighbouring cell; Some reads an explicit rendering exception.
    parcel: Option<(Uuid, i32)>,
}

#[derive(Debug, Default)]
pub struct Visibility {
    /// Rendering reads only the completed decision; discovery builds `allowed` separately.
    displayed: Vec<Mask>,
    complete: bool,
    /// Session-local explicit exceptions survive changes of the agent parcel.
    cache: HashMap<(Uuid, i32), Mask>,
    stale: HashSet<(Uuid, i32)>,
    settings: Settings,
    current: Option<Mask>,
    allowed: Vec<Mask>,
    known: Vec<Mask>,
    frontier: VecDeque<Cell>,
    seen: HashSet<Cell>,
    pending: HashMap<i32, Request>,
    attempts: HashMap<Cell, u8>,
    visible_requested: HashSet<(Uuid, i32)>,
    visible_attempts: HashMap<(Uuid, i32), u8>,
    next_sequence: i32,
    next_request: Option<Instant>,
    connected_regions: u64,
    /// Changes only when the visible parcel mask changes.
    pub version: u64,
}

impl Visibility {
    pub fn invalidate(&mut self, handle: RegionHandle) {
        // ParcelOverlay describes one region (LLViewerParcelMgr::processParcelOverlay).
        // Recheck its exceptions without hiding the last complete result meanwhile.
        self.stale
            .extend(self.cache.iter().filter(|(_, m)| m.handle == handle).map(|(&key, _)| key));
        self.reset();
    }
    pub fn active(&self) -> bool {
        !self.displayed.is_empty()
    }

    pub fn contains(&self, position: DVec3) -> bool {
        !self.active() || self.displayed.iter().any(|m| m.contains(position))
    }

    fn reset(&mut self) {
        self.current = None;
        self.allowed.clear();
        self.known.clear();
        self.frontier.clear();
        self.seen.clear();
        self.pending.clear();
        self.attempts.clear();
        self.visible_requested.clear();
        self.visible_attempts.clear();
        self.next_request = None;
        self.complete = false;
    }

    fn publish(&mut self, world: &World) {
        if self.allowed.iter().any(|mask| !mask.connected(world)) {
            return;
        }
        self.allowed.sort_unstable_by_key(|mask| (mask.handle, mask.parcel.local_id));
        if self.displayed.len() != self.allowed.len() || !self.displayed.iter().zip(&self.allowed).all(|(a, b)| a.same_bounds(b)) {
            self.displayed.clone_from(&self.allowed);
            self.version = self.version.wrapping_add(1);
        }
        self.complete = true;
    }

    fn refresh(&mut self, world: &World, settings: &Settings, now: Instant) -> Vec<NetCommand> {
        self.cache.retain(|&(region, local_id), mask| {
            mask.connected(world)
                && settings
                    .always_visible
                    .iter()
                    .any(|entry| entry.region == region && entry.local_id == local_id)
        });
        self.stale.retain(|key| self.cache.contains_key(key));
        let previous = self.displayed.len();
        self.displayed.retain(|mask| mask.connected(world));
        if self.displayed.len() != previous {
            self.version = self.version.wrapping_add(1);
        }
        let candidate = (settings.mode != Mode::Off)
            .then(|| world.current_parcel_entry())
            .flatten()
            .and_then(|_| {
                let handle = world.main_region?;
                let region = world.regions.get(&handle)?;
                let mask = Mask {
                    handle,
                    region: region.info.as_ref()?.region_id,
                    size: (region.heightmap.size_x, region.heightmap.size_y),
                    parcel: world.parcel.clone()?,
                };
                // AgentParcel can still describe the previous parcel during a crossing.
                let validated = self.current.as_ref().is_some_and(|current| {
                    current.handle == mask.handle
                        && current.region == mask.region
                        && current.size == mask.size
                        && Arc::ptr_eq(&current.parcel, &mask.parcel)
                });
                ((validated || mask.valid()) && mask.contains(world.agent_global()?)).then_some(mask)
            });
        if settings.mode != Mode::Off && candidate.is_none() {
            // The simulator may still describe the parcel we just left. Keep the
            // previous display until the arrival's rules can actually be evaluated.
            return Vec::new();
        }
        let candidate = candidate.filter(|mask| settings.applies(mask.region, mask.parcel.local_id));
        let connected_regions = if candidate.is_some() && (settings.automatic_neighbours() || !settings.always_visible.is_empty()) {
            let mut hash = std::hash::DefaultHasher::new();
            for (&handle, region) in &world.regions {
                (
                    handle,
                    region.info.as_ref().map(|info| info.region_id),
                    region.heightmap.size_x,
                    region.heightmap.size_y,
                )
                    .hash(&mut hash);
            }
            hash.finish()
        } else {
            0
        };
        let unchanged = self.settings == *settings
            && self.connected_regions == connected_regions
            && match (&self.current, &candidate) {
                (None, None) => true,
                (Some(a), Some(b)) => a.handle == b.handle && a.region == b.region && a.size == b.size && Arc::ptr_eq(&a.parcel, &b.parcel),
                _ => false,
            };
        if !unchanged {
            self.reset();
            self.settings.clone_from(settings);
            self.connected_regions = connected_regions;
            self.current = candidate.clone();
            if let Some(mask) = candidate {
                self.add_mask(world, mask, true);
                for mask in self.cache.values().cloned().collect::<Vec<_>>() {
                    if !self
                        .allowed
                        .iter()
                        .any(|current| current.handle == mask.handle && current.parcel.local_id == mask.parcel.local_id)
                    {
                        self.add_mask(world, mask, false);
                    }
                }
            }
        }
        if self.complete {
            return Vec::new();
        }
        if self.current.is_none() {
            self.publish(world);
            return Vec::new();
        }
        // Bounded background discovery; never select land or change the agent parcel.
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, r)| now.saturating_duration_since(r.sent) >= Duration::from_secs(10))
            .map(|(&sequence, r)| (sequence, r.cell, r.parcel))
            .collect();
        for (sequence, cell, parcel) in expired {
            self.pending.remove(&sequence);
            if let Some(key) = parcel {
                if self.visible_attempts.get(&key).copied().unwrap_or(0) < 2 {
                    self.visible_requested.remove(&key);
                }
            } else if self.attempts.get(&cell).copied().unwrap_or(0) < 2 {
                self.seen.remove(&cell);
                self.frontier.push_front(cell);
            }
        }
        if self.pending.len() >= MAX_REQUESTS || self.next_request.is_some_and(|t| now < t) {
            return Vec::new();
        }
        // Explicit visibility has priority over automatic owner/group discovery.
        // Resolve by region UUID and parcel ID, never by a saved name or old bounds.
        for entry in &settings.always_visible {
            let key = (entry.region, entry.local_id);
            if entry.region.is_nil() || entry.local_id <= 0 || self.visible_requested.contains(&key) {
                continue;
            }
            let Some((&handle, _)) = world
                .regions
                .iter()
                .find(|(_, region)| region.info.as_ref().is_some_and(|info| info.region_id == entry.region))
            else {
                continue;
            };
            if self
                .allowed
                .iter()
                .any(|m| m.handle == handle && m.parcel.local_id == entry.local_id)
                && !self.stale.contains(&key)
            {
                continue;
            }
            self.next_sequence = FIRST_SEQUENCE + (self.next_sequence + 1 - FIRST_SEQUENCE).rem_euclid(1_000_000);
            let sequence = self.next_sequence;
            self.visible_requested.insert(key);
            *self.visible_attempts.entry(key).or_default() += 1;
            self.pending.insert(
                sequence,
                Request {
                    cell: (handle, 0, 0),
                    sent: now,
                    parcel: Some(key),
                },
            );
            self.next_request = Some(now + Duration::from_millis(150));
            return vec![NetCommand::Build(BuildCmd::ParcelRequestById {
                handle,
                sequence,
                local_id: entry.local_id,
            })];
        }
        if !settings.automatic_neighbours() || self.known.len() >= MAX_PARCELS {
            if self.pending.is_empty() {
                self.publish(world);
            }
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
            self.pending.insert(
                sequence,
                Request {
                    cell,
                    sent: now,
                    parcel: None,
                },
            );
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
        if self.pending.is_empty() {
            self.publish(world);
        }
        Vec::new()
    }

    fn add_mask(&mut self, world: &World, mask: Mask, discover: bool) {
        if discover {
            let cells_x = mask.size.0 / 4;
            let (ox, oy) = aurora_net::handle_to_origin(mask.handle);
            for (byte, bits) in mask.parcel.bitmap.iter().enumerate() {
                for bit in 0..8 {
                    if bits & (1 << bit) == 0 {
                        continue;
                    }
                    let i = byte as u32 * 8 + bit;
                    let (x, y) = (i % cells_x * 4, i / cells_x * 4);
                    if self.settings.automatic_neighbours() {
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
        }
        if let Some(previous) = self
            .allowed
            .iter_mut()
            .find(|m| m.handle == mask.handle && m.parcel.local_id == mask.parcel.local_id)
        {
            *previous = mask;
        } else {
            self.allowed.push(mask);
        }
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
            region: region.info.as_ref().map_or(Uuid::nil(), |info| info.region_id),
            size: (region.heightmap.size_x, region.heightmap.size_y),
            parcel,
        };
        if let Some((region_id, local_id)) = request.parcel {
            if region.info.as_ref().is_some_and(|info| info.region_id == region_id)
                && mask.parcel.local_id == local_id
                && mask.valid()
                && self
                    .settings
                    .always_visible
                    .iter()
                    .any(|entry| entry.region == region_id && entry.local_id == local_id)
            {
                self.cache.insert((region_id, local_id), mask.clone());
                self.stale.remove(&(region_id, local_id));
                self.add_mask(world, mask, false);
            }
            return true;
        }
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
        let related = self
            .current
            .as_ref()
            .is_some_and(|current| self.settings.includes_related(&current.parcel, &mask.parcel));
        if related
            && self.known.len() < MAX_PARCELS
            && !self
                .known
                .iter()
                .any(|m| m.handle == handle && m.parcel.local_id == mask.parcel.local_id)
        {
            self.add_mask(world, mask, true);
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
        self.parcel_entry(self.main_region?, self.parcel.as_deref()?)
    }

    pub fn selected_parcel_entry(&self) -> Option<Entry> {
        let selection = self.land.sel.as_ref()?;
        if selection.result != aurora_net::land::PARCEL_RESULT_SUCCESS {
            return None;
        }
        self.parcel_entry(selection.handle, selection.parcel.as_deref()?)
    }

    pub(crate) fn parcel_entry(&self, handle: RegionHandle, parcel: &ParcelInfo) -> Option<Entry> {
        let region = self.regions.get(&handle)?;
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
            assert_eq!(settings.applies(renamed.region, renamed.local_id), listed);
            assert_eq!(settings.applies(other.region, other.local_id), unlisted);
        }
        settings.same_owner = true;
        settings.same_group = true;
        settings.always_visible.push(other);
        let encoded = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&encoded).unwrap(), settings);
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
        let old: Settings = serde_json::from_str(r#"{"mode":"Always","same_owner":true,"parcels":[]}"#).unwrap();
        assert!(
            !old.same_group && old.always_visible.is_empty(),
            "existing preferences keep the new options off"
        );
    }

    fn response(command: NetCommand) -> (RegionHandle, i32, Arc<ParcelInfo>) {
        match command {
            NetCommand::Build(BuildCmd::ParcelRequest {
                handle,
                sequence,
                west,
                south,
                ..
            }) => (handle, sequence, crate::demo::parcel_visibility::parcel_at(west, south)),
            NetCommand::Build(BuildCmd::ParcelRequestById {
                handle,
                sequence,
                local_id,
            }) => (
                handle,
                sequence,
                crate::demo::parcel_visibility::parcel_by_id(local_id).expect("demo parcel"),
            ),
            _ => panic!("parcel visibility request"),
        }
    }

    #[test]
    fn parcel_visibility_same_group_matches_assigned_and_deeded_groups_on_both_sides() {
        let group = Uuid::from_u128(946);
        let config = Settings {
            same_group: true,
            ..Default::default()
        };
        for current_owned in [false, true] {
            for neighbour_owned in [false, true] {
                let parcel = |owned, resident| ParcelInfo {
                    owner_id: if owned { group } else { Uuid::from_u128(resident) },
                    group_id: if owned { Uuid::nil() } else { group },
                    is_group_owned: owned,
                    ..Default::default()
                };
                let current = parcel(current_owned, 1);
                let neighbour = parcel(neighbour_owned, 2);
                assert!(
                    config.includes_related(&current, &neighbour),
                    "current owned={current_owned}, neighbour owned={neighbour_owned}"
                );
                let mut different = neighbour.clone();
                different.owner_id = Uuid::from_u128(3);
                different.group_id = Uuid::from_u128(4);
                assert!(!config.includes_related(&current, &different));
                assert!(!Settings::default().includes_related(&current, &neighbour));
            }
        }
        assert!(
            !config.includes_related(&ParcelInfo::default(), &ParcelInfo::default()),
            "no group is not a common group"
        );
    }

    #[test]
    fn parcel_visibility_owner_and_group_options_are_independent_and_combined_by_union() {
        let current = ParcelInfo {
            owner_id: Uuid::from_u128(1),
            group_id: Uuid::from_u128(10),
            ..Default::default()
        };
        let same_owner = ParcelInfo {
            group_id: Uuid::from_u128(20),
            ..current.clone()
        };
        let same_group = ParcelInfo {
            owner_id: Uuid::from_u128(2),
            ..current.clone()
        };
        for (owner, group) in [(false, false), (true, false), (false, true), (true, true)] {
            let config = Settings {
                same_owner: owner,
                same_group: group,
                ..Default::default()
            };
            assert_eq!(config.includes_related(&current, &same_owner), owner);
            assert_eq!(config.includes_related(&current, &same_group), group);
        }
    }

    #[test]
    fn parcel_visibility_same_group_discovers_a_deeded_neighbour_of_an_assigned_current_parcel() {
        let mut world = world();
        let group = Uuid::from_u128(946);
        Arc::make_mut(world.parcel.as_mut().unwrap()).group_id = group;
        let config = Settings {
            mode: Mode::Always,
            same_group: true,
            ..Default::default()
        };
        let start = Instant::now();
        for step in 0..200 {
            for command in world.update_parcel_visibility(&config, start + Duration::from_millis(step * 150)) {
                let (handle, sequence, mut parcel) = response(command);
                if parcel.local_id == 2 {
                    let parcel = Arc::make_mut(&mut parcel);
                    parcel.owner_id = group;
                    parcel.is_group_owned = true;
                    parcel.group_id = Uuid::nil();
                } else if parcel.local_id == 3 {
                    Arc::make_mut(&mut parcel).owner_id = crate::demo::DEMO_LOUP;
                }
                world.parcel_visibility_reply(handle, sequence, parcel);
            }
        }
        assert_eq!(world.parcel_visibility.allowed.len(), 2);
        assert!(world.parcel_visibility.contains(DVec3::new(256_120.0, 256_140.0, 25.0)));
        assert!(
            !world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)),
            "a matching owner alone does not enable group inclusion"
        );
    }

    #[test]
    fn parcel_visibility_saved_parcels_stay_visible_after_we_leave_under_each_active_mode() {
        for mode in [Mode::Always, Mode::Listed, Mode::ExceptListed] {
            let mut world = world();
            let saved = world.current_parcel_entry().unwrap();
            world.parcel = Some(crate::demo::parcel_visibility::parcel_at(120.0, 130.0));
            world.agent.get_off(Vec3::new(120.0, 130.0, 25.0));
            let mut config = Settings {
                mode,
                always_visible: vec![saved],
                ..Default::default()
            };
            if mode == Mode::Listed {
                config.parcels.push(world.current_parcel_entry().unwrap());
            }
            let now = Instant::now();
            let command = world.update_parcel_visibility(&config, now).pop().unwrap();
            assert!(matches!(
                command,
                NetCommand::Build(BuildCmd::ParcelRequestById { local_id: 1, .. })
            ));
            let (handle, sequence, parcel) = response(command);
            world.parcel_visibility_reply(handle, sequence, parcel);
            world.update_parcel_visibility(&config, now + Duration::from_millis(150));
            assert!(world.parcel_visibility.contains(DVec3::new(256_140.0, 256_140.0, 25.0)));
            assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
            config.always_visible.clear();
            world.update_parcel_visibility(&config, now);
            assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_140.0, 25.0)));
        }
    }

    #[test]
    fn parcel_visibility_explicit_foreign_parcels_do_not_expand_the_automatic_neighbour_scope() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let entry = world
            .parcel_entry(handle, &crate::demo::parcel_visibility::parcel_at(120.0, 120.0))
            .unwrap();
        let config = Settings {
            mode: Mode::Always,
            same_owner: true,
            always_visible: vec![entry],
            ..Default::default()
        };
        let command = world.update_parcel_visibility(&config, Instant::now()).pop().unwrap();
        assert!(
            matches!(command, NetCommand::Build(BuildCmd::ParcelRequestById { local_id: 4, .. })),
            "explicit parcels are requested first"
        );
        let frontier = world.parcel_visibility.frontier.len();
        let (handle, sequence, parcel) = response(command);
        world.parcel_visibility_reply(handle, sequence, parcel);
        assert_eq!(
            world.parcel_visibility.frontier.len(),
            frontier,
            "an exception does not start an unrelated estate crawl"
        );
        for step in 1..200 {
            for command in world.update_parcel_visibility(&config, Instant::now() + Duration::from_millis(step * 150)) {
                let (handle, sequence, parcel) = response(command);
                world.parcel_visibility_reply(handle, sequence, parcel);
            }
        }
        assert!(world.parcel_visibility.contains(DVec3::new(256_120.0, 256_120.0, 25.0)));
        assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
    }

    #[test]
    fn parcel_visibility_publishes_multiple_exceptions_together_after_all_replies() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let base = Settings {
            mode: Mode::Always,
            ..Default::default()
        };
        let start = Instant::now();
        world.update_parcel_visibility(&base, start);
        let version = world.parcel_visibility.version;
        let config = Settings {
            always_visible: [2, 3]
                .into_iter()
                .map(|id| {
                    world
                        .parcel_entry(handle, &crate::demo::parcel_visibility::parcel_by_id(id).unwrap())
                        .unwrap()
                })
                .collect(),
            ..base
        };
        let b = DVec3::new(256_120.0, 256_140.0, 25.0);
        let c = DVec3::new(256_140.0, 256_120.0, 25.0);
        for (step, id) in [(0, 2), (1, 3)] {
            let command = world
                .update_parcel_visibility(&config, start + Duration::from_millis(step * 150))
                .pop()
                .unwrap();
            let (handle, sequence, parcel) = response(command);
            assert_eq!(parcel.local_id, id);
            world.parcel_visibility_reply(handle, sequence, parcel);
            assert_eq!(
                world.parcel_visibility.version, version,
                "a partial reply must not rebuild the scene"
            );
            assert!(!world.parcel_visibility.contains(b) && !world.parcel_visibility.contains(c));
        }
        world.update_parcel_visibility(&config, start + Duration::from_millis(300));
        assert_eq!(world.parcel_visibility.version, version + 1);
        assert!(world.parcel_visibility.contains(b) && world.parcel_visibility.contains(c));
        assert!(!world.parcel_visibility.contains(DVec3::new(256_120.0, 256_120.0, 25.0)));
    }

    #[test]
    fn parcel_visibility_a_late_reply_cannot_publish_the_previous_parcels_decision() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let base = Settings {
            mode: Mode::Always,
            ..Default::default()
        };
        let start = Instant::now();
        world.update_parcel_visibility(&base, start);
        let version = world.parcel_visibility.version;
        let neighbour = crate::demo::parcel_visibility::parcel_by_id(2).unwrap();
        let config = Settings {
            always_visible: vec![world.parcel_entry(handle, &neighbour).unwrap()],
            ..base
        };
        let (old_handle, old_sequence, old_parcel) = response(world.update_parcel_visibility(&config, start).pop().unwrap());
        world.agent.get_off(Vec3::new(140.0, 120.0, 25.0));
        world.parcel = Some(crate::demo::parcel_visibility::parcel_by_id(3).unwrap());
        let (handle, sequence, parcel) = response(
            world
                .update_parcel_visibility(&config, start + Duration::from_secs(1))
                .pop()
                .unwrap(),
        );
        world.parcel_visibility_reply(old_handle, old_sequence, old_parcel);
        assert_eq!(world.parcel_visibility.version, version);
        assert!(world.parcel_visibility.cache.is_empty());
        assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
        world.parcel_visibility_reply(handle, sequence, parcel);
        world.update_parcel_visibility(&config, start + Duration::from_secs(2));
        assert_eq!(world.parcel_visibility.version, version + 1);
        assert!(world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
        assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_140.0, 25.0)));
    }

    #[test]
    fn parcel_visibility_round_trips_keep_whitelisted_content_visible_without_new_requests() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let home = world.parcel.clone().unwrap();
        let neighbour = crate::demo::parcel_visibility::parcel_by_id(2).unwrap();
        let config = Settings {
            mode: Mode::Listed,
            parcels: vec![world.current_parcel_entry().unwrap()],
            always_visible: vec![world.parcel_entry(handle, &neighbour).unwrap()],
            ..Default::default()
        };
        let start = Instant::now();
        let (handle, sequence, parcel) = response(world.update_parcel_visibility(&config, start).pop().unwrap());
        world.parcel_visibility_reply(handle, sequence, parcel);
        world.update_parcel_visibility(&config, start + Duration::from_millis(150));
        let target = DVec3::new(256_120.0, 256_140.0, 25.0);
        let cache = world.parcel_visibility.cache.clone();
        for step in 1..=8 {
            let now = start + Duration::from_secs(step);
            for (position, parcel, active) in [
                (
                    Vec3::new(140.0, 120.0, 25.0),
                    crate::demo::parcel_visibility::parcel_by_id(3).unwrap(),
                    false,
                ),
                (Vec3::new(140.0, 140.0, 25.0), home.clone(), true),
            ] {
                let version = world.parcel_visibility.version;
                world.agent.get_off(position);
                assert!(world.update_parcel_visibility(&config, now).is_empty());
                assert!(
                    world.parcel_visibility.contains(target),
                    "delayed AgentParcel must not hide the exception"
                );
                assert_eq!(world.parcel_visibility.version, version);
                world.parcel = Some(parcel);
                assert!(
                    world.update_parcel_visibility(&config, now).is_empty(),
                    "known exceptions need no round trip"
                );
                assert_eq!(world.parcel_visibility.active(), active);
                assert!(
                    world.parcel_visibility.contains(target),
                    "the exception remains visible in the committed result"
                );
                assert_eq!(world.parcel_visibility.version, version + 1);
            }
        }
        assert!(Arc::ptr_eq(
            &world.parcel_visibility.cache.values().next().unwrap().parcel,
            &cache.values().next().unwrap().parcel
        ));
    }

    #[test]
    fn parcel_visibility_owner_and_group_discovery_publish_one_complete_decision() {
        for group in [false, true] {
            let mut world = world();
            if group {
                Arc::make_mut(world.parcel.as_mut().unwrap()).group_id = Uuid::from_u128(946);
            }
            let base = Settings {
                mode: Mode::Always,
                ..Default::default()
            };
            let start = Instant::now();
            world.update_parcel_visibility(&base, start);
            let version = world.parcel_visibility.version;
            let config = Settings {
                same_owner: !group,
                same_group: group,
                ..base
            };
            let target = DVec3::new(256_120.0, 256_140.0, 25.0);
            let mut partial = false;
            for step in 0..200 {
                let commands = world.update_parcel_visibility(&config, start + Duration::from_millis(step * 150));
                if world.parcel_visibility.complete {
                    break;
                }
                assert_eq!(world.parcel_visibility.version, version);
                assert!(!world.parcel_visibility.contains(target));
                for command in commands {
                    let (handle, sequence, mut parcel) = response(command);
                    if group && parcel.local_id == 2 {
                        let parcel = Arc::make_mut(&mut parcel);
                        parcel.owner_id = Uuid::from_u128(946);
                        parcel.is_group_owned = true;
                        parcel.group_id = Uuid::nil();
                    }
                    world.parcel_visibility_reply(handle, sequence, parcel);
                    partial |= world.parcel_visibility.allowed.len() > 1;
                    assert_eq!(world.parcel_visibility.version, version);
                    assert!(
                        !world.parcel_visibility.contains(target),
                        "discovery cannot publish a partial scope"
                    );
                }
            }
            assert!(partial && world.parcel_visibility.complete);
            assert_eq!(world.parcel_visibility.version, version + 1);
            assert!(world.parcel_visibility.contains(target));
            assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
        }
    }

    #[test]
    fn parcel_visibility_overlay_revalidates_cached_bounds_without_hiding_them_while_waiting() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let neighbour = crate::demo::parcel_visibility::parcel_by_id(2).unwrap();
        let config = Settings {
            mode: Mode::Always,
            always_visible: vec![world.parcel_entry(handle, &neighbour).unwrap()],
            ..Default::default()
        };
        let start = Instant::now();
        let (handle, sequence, parcel) = response(world.update_parcel_visibility(&config, start).pop().unwrap());
        world.parcel_visibility_reply(handle, sequence, parcel);
        world.update_parcel_visibility(&config, start + Duration::from_millis(150));
        let version = world.parcel_visibility.version;
        let old = DVec3::new(256_120.0, 256_140.0, 25.0);
        let new = DVec3::new(256_120.0, 256_120.0, 25.0);
        world.parcel_visibility.invalidate(handle);
        assert!(world.parcel_visibility.contains(old));
        let (handle, sequence, _) = response(
            world
                .update_parcel_visibility(&config, start + Duration::from_secs(1))
                .pop()
                .unwrap(),
        );
        assert!(world.parcel_visibility.contains(old) && !world.parcel_visibility.contains(new));
        let mut updated = crate::demo::parcel_visibility::parcel_by_id(4).unwrap();
        Arc::make_mut(&mut updated).local_id = 2;
        world.parcel_visibility_reply(handle, sequence, updated);
        assert_eq!(world.parcel_visibility.version, version);
        assert!(world.parcel_visibility.contains(old));
        world.update_parcel_visibility(&config, start + Duration::from_secs(2));
        assert_eq!(world.parcel_visibility.version, version + 1);
        assert!(!world.parcel_visibility.contains(old) && world.parcel_visibility.contains(new));
    }

    #[test]
    fn parcel_visibility_cached_bounds_cannot_replace_newer_agent_parcel_data() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let neighbour = crate::demo::parcel_visibility::parcel_by_id(2).unwrap();
        let config = Settings {
            mode: Mode::Always,
            always_visible: vec![world.parcel_entry(handle, &neighbour).unwrap()],
            ..Default::default()
        };
        let start = Instant::now();
        let (handle, sequence, parcel) = response(world.update_parcel_visibility(&config, start).pop().unwrap());
        world.parcel_visibility_reply(handle, sequence, parcel);
        world.update_parcel_visibility(&config, start + Duration::from_millis(150));
        let mut arrival = neighbour;
        let north_east = crate::demo::parcel_visibility::parcel_by_id(1).unwrap();
        for (byte, other) in Arc::make_mut(&mut arrival).bitmap.iter_mut().zip(&north_east.bitmap) {
            *byte |= other;
        }
        world.parcel = Some(arrival);
        world.agent.get_off(Vec3::new(120.0, 140.0, 25.0));
        assert!(world.update_parcel_visibility(&config, start + Duration::from_secs(1)).is_empty());
        assert!(world.parcel_visibility.contains(DVec3::new(256_140.0, 256_140.0, 25.0)));
        assert!(!world.parcel_visibility.contains(DVec3::new(256_140.0, 256_120.0, 25.0)));
    }

    #[test]
    fn parcel_visibility_cached_exceptions_survive_revalidation_timeouts_but_not_removal_or_logout() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let neighbour = crate::demo::parcel_visibility::parcel_by_id(2).unwrap();
        let mut config = Settings {
            mode: Mode::Always,
            always_visible: vec![world.parcel_entry(handle, &neighbour).unwrap()],
            ..Default::default()
        };
        let start = Instant::now();
        let (handle, sequence, parcel) = response(world.update_parcel_visibility(&config, start).pop().unwrap());
        world.parcel_visibility_reply(handle, sequence, parcel);
        world.update_parcel_visibility(&config, start + Duration::from_millis(150));
        let version = world.parcel_visibility.version;
        let target = DVec3::new(256_120.0, 256_140.0, 25.0);
        world.parcel_visibility.invalidate(handle);
        assert_eq!(world.update_parcel_visibility(&config, start + Duration::from_secs(1)).len(), 1);
        assert_eq!(world.update_parcel_visibility(&config, start + Duration::from_secs(11)).len(), 1);
        assert!(world.update_parcel_visibility(&config, start + Duration::from_secs(21)).is_empty());
        assert!(world.parcel_visibility.contains(target));
        assert_eq!(
            world.parcel_visibility.version, version,
            "a lost refresh cannot blink a previously resolved exception"
        );
        let saved = config.always_visible.pop().unwrap();
        world.update_parcel_visibility(&config, start + Duration::from_secs(22));
        assert!(world.parcel_visibility.cache.is_empty());
        assert!(!world.parcel_visibility.contains(target));
        config.always_visible.push(saved);
        let (handle, sequence, parcel) = response(
            world
                .update_parcel_visibility(&config, start + Duration::from_secs(23))
                .pop()
                .unwrap(),
        );
        world.parcel_visibility_reply(handle, sequence, parcel);
        assert_eq!(world.parcel_visibility.cache.len(), 1);
        world.reset();
        assert!(world.parcel_visibility.cache.is_empty() && !world.parcel_visibility.active());
    }

    #[test]
    fn parcel_visibility_automatic_discovery_continues_through_an_explicit_same_owner_parcel() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let entry = world
            .parcel_entry(handle, &crate::demo::parcel_visibility::parcel_at(120.0, 130.0))
            .unwrap();
        let config = Settings {
            mode: Mode::Always,
            same_owner: true,
            always_visible: vec![entry],
            ..Default::default()
        };
        let start = Instant::now();
        for step in 0..200 {
            for command in world.update_parcel_visibility(&config, start + Duration::from_millis(step * 150)) {
                let (handle, sequence, mut parcel) = response(command);
                if parcel.local_id == 4 {
                    Arc::make_mut(&mut parcel).owner_id = crate::demo::DEMO_LOUP;
                }
                world.parcel_visibility_reply(handle, sequence, parcel);
            }
        }
        assert_eq!(world.parcel_visibility.allowed.len(), 3);
        assert!(world.parcel_visibility.contains(DVec3::new(256_120.0, 256_120.0, 25.0)));
    }

    #[test]
    fn parcel_visibility_explicit_requests_reject_wrong_ids_and_stale_replies() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let entry = world
            .parcel_entry(handle, &crate::demo::parcel_visibility::parcel_at(120.0, 120.0))
            .unwrap();
        let config = Settings {
            mode: Mode::Always,
            always_visible: vec![entry],
            ..Default::default()
        };
        let now = Instant::now();
        let (handle, sequence, _) = response(world.update_parcel_visibility(&config, now).pop().unwrap());
        world.parcel_visibility_reply(handle, sequence, crate::demo::parcel_visibility::parcel_at(120.0, 130.0));
        assert_eq!(world.parcel_visibility.allowed.len(), 1);
        world.update_parcel_visibility(&Settings::default(), now);
        let (handle, sequence, parcel) = response(world.update_parcel_visibility(&config, now).pop().unwrap());
        world.update_parcel_visibility(
            &Settings {
                mode: Mode::Always,
                ..Default::default()
            },
            now,
        );
        world.parcel_visibility_reply(handle, sequence, parcel);
        assert_eq!(
            world.parcel_visibility.allowed.len(),
            1,
            "a removed exception cannot reappear through a late reply"
        );
    }

    #[test]
    fn parcel_visibility_explicit_requests_retry_once_without_polling_forever() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let entry = world
            .parcel_entry(handle, &crate::demo::parcel_visibility::parcel_at(120.0, 120.0))
            .unwrap();
        let config = Settings {
            mode: Mode::Always,
            always_visible: vec![entry],
            ..Default::default()
        };
        let start = Instant::now();
        assert_eq!(world.update_parcel_visibility(&config, start).len(), 1);
        assert!(world.update_parcel_visibility(&config, start + Duration::from_secs(1)).is_empty());
        assert_eq!(world.update_parcel_visibility(&config, start + Duration::from_secs(10)).len(), 1);
        assert!(world.update_parcel_visibility(&config, start + Duration::from_secs(20)).is_empty());
        assert!(world.parcel_visibility.pending.is_empty());
    }

    #[test]
    fn parcel_visibility_saved_regions_resolve_by_uuid_on_arrival_and_retire_on_replacement() {
        let mut world = world();
        let main = world.main_region.unwrap();
        let (ox, oy) = aurora_net::handle_to_origin(main);
        let neighbour = aurora_net::origin_to_handle(ox + 256, oy);
        let region_id = Uuid::from_u128(12345);
        let entry = Entry {
            region: region_id,
            local_id: 4,
            name: "Visible".into(),
            region_name: "Neighbour".into(),
        };
        let config = Settings {
            mode: Mode::Always,
            always_visible: vec![entry],
            ..Default::default()
        };
        let now = Instant::now();
        assert!(
            world.update_parcel_visibility(&config, now).is_empty(),
            "disconnected regions are not requested"
        );
        let mut info = world.main().unwrap().info.as_ref().unwrap().as_ref().clone();
        info.handle = neighbour;
        info.region_id = region_id;
        world.ensure_region(neighbour, (256, 256)).info = Some(Arc::new(info));
        let command = world.update_parcel_visibility(&config, now).pop().unwrap();
        assert!(matches!(command, NetCommand::Build(BuildCmd::ParcelRequestById { handle, local_id: 4, .. }) if handle == neighbour));
        let (handle, sequence, _) = response(command);
        world.parcel_visibility_reply(
            handle,
            sequence,
            Arc::new(ParcelInfo {
                local_id: 4,
                bitmap: vec![255; 512],
                ..Default::default()
            }),
        );
        world.update_parcel_visibility(&config, now + Duration::from_millis(150));
        let position = DVec3::new(f64::from(ox + 260), f64::from(oy + 140), 25.0);
        assert!(world.parcel_visibility.contains(position));
        let region = world.regions.get_mut(&neighbour).unwrap();
        Arc::make_mut(region.info.as_mut().unwrap()).region_id = Uuid::from_u128(54321);
        world.update_parcel_visibility(&config, now);
        assert!(
            !world.parcel_visibility.contains(position),
            "a different region at the old position is not the saved parcel"
        );
    }

    #[test]
    fn parcel_visibility_selected_parcels_can_be_saved_without_replacing_the_current_parcel() {
        let mut world = world();
        let current = world.parcel.clone().unwrap();
        let handle = world.main_region.unwrap();
        world.land.select_at(handle, Vec3::new(120.0, 120.0, 25.0), (256.0, 256.0));
        world.apply(aurora_net::NetEvent::Land(aurora_net::land::LandEvent::Selected {
            handle,
            info: crate::demo::parcel_visibility::parcel_at(120.0, 120.0),
            result: aurora_net::land::PARCEL_RESULT_SUCCESS,
        }));
        assert_eq!(world.selected_parcel_entry().unwrap().local_id, 4);
        assert!(Arc::ptr_eq(world.parcel.as_ref().unwrap(), &current));
        world.land.sel.as_mut().unwrap().result = aurora_net::land::PARCEL_RESULT_MULTIPLE;
        assert!(
            world.selected_parcel_entry().is_none(),
            "a multi-parcel selection cannot become one exception"
        );
    }

    #[test]
    fn parcel_visibility_bitmap_uses_exact_cells_region_and_unlimited_altitude() {
        let handle = aurora_net::origin_to_handle(256_000, 256_000);
        let mask = Mask {
            handle,
            region: Uuid::from_u128(946),
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
        assert!(
            world.parcel_visibility.active(),
            "keep the previous decision until arrival data is known"
        );
        assert_eq!(
            world.parcel_visibility.version, version,
            "a delayed AgentParcel cannot change the display"
        );
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
            if step == 20 {
                let mut info = world.main().unwrap().info.as_ref().unwrap().as_ref().clone();
                info.handle = neighbour;
                info.region_id = Uuid::from_u128(947);
                world.regions.get_mut(&neighbour).unwrap().info = Some(Arc::new(info));
            }
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
            if step < 20 {
                assert_eq!(
                    world.parcel_visibility.version, 0,
                    "an unidentified included region cannot be published"
                );
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
