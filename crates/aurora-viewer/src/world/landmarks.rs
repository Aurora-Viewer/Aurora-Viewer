//! Landmark assets and where they point: the asset text (region id and
//! position in the region, or an old global position) fetched once per
//! asset, and the handles of the regions they name, asked with
//! RegionHandleRequest. Port of LLLandmark (indra/llinventory/lllandmark.cpp:
//! constructFromString, getGlobalPos, requestRegionHandle,
//! processRegionIDAndHandle) and LLLandmarkList::getAsset
//! (indra/newview/lllandmarklist.cpp), originally LGPL 2.1.

use crate::world::inventory::Inventory;
use aurora_net::{NetCommand, RegionHandle};
use glam::{DVec3, Vec3};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LLItemBridge::performAction / doShowOnMap (indra/newview/llinventorybridge.cpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CopySlurl,
    ShowOnMap,
}

#[derive(Debug, PartialEq)]
pub enum ActionResult {
    Slurl(String),
    Map(DVec3),
    Unavailable,
}

#[derive(Debug)]
struct PendingAction {
    asset: Uuid,
    action: Action,
    since: Instant,
    asked_region: bool,
}

/// Fetch key family of landmark assets (top four bits, as scene/*).
pub const FETCH_KIND_LANDMARK: u64 = 7 << 60;

/// What a landmark asset holds (LLLandmark versions 1 and 2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LandmarkAsset {
    /// "Landmark version 1": a global position.
    Global(DVec3),
    /// "Landmark version 2": a region and a position in it.
    Region { region_id: Uuid, pos: Vec3 },
}

/// LLLandmark::constructFromString.
pub fn parse(data: &[u8]) -> Option<LandmarkAsset> {
    let text = String::from_utf8_lossy(data);
    let mut lines = text.lines().map(str::trim);
    let version: u32 = lines.next()?.strip_prefix("Landmark version")?.trim().parse().ok()?;
    let floats = |s: &str, n: usize| -> Option<Vec<f64>> {
        let v: Vec<f64> = s.split_whitespace().take(n).map(|x| x.parse().ok()).collect::<Option<_>>()?;
        (v.len() == n).then_some(v)
    };
    match version {
        1 => {
            let p = floats(lines.next()?.strip_prefix("position")?, 3)?;
            Some(LandmarkAsset::Global(DVec3::new(p[0], p[1], p[2])))
        }
        2 => {
            let region_id = Uuid::parse_str(lines.next()?.strip_prefix("region_id")?.trim()).ok()?;
            if region_id.is_nil() {
                return None;
            }
            let p = floats(lines.next()?.strip_prefix("local_pos")?, 3)?;
            Some(LandmarkAsset::Region {
                region_id,
                pos: Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Wanted,
    Fetching,
    Ready(LandmarkAsset),
    Failed,
}

/// Where a landmark points, as far as it is known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolved {
    /// The asset or its region's handle is on its way.
    Pending,
    /// The asset could not be loaded or read.
    Failed,
    /// Region id (nil for a version 1 landmark), position in the region and
    /// global position.
    Ready { region_id: Uuid, region_pos: Vec3, global: DVec3 },
}

#[derive(Debug, Default)]
pub struct Landmarks {
    assets: HashMap<Uuid, State>,
    /// Region handles by region id (sRegions, mLocalRegion).
    handles: HashMap<Uuid, RegionHandle>,
    asked: HashSet<Uuid>,
    /// Fetches in flight: key → asset.
    keys: HashMap<u64, Uuid>,
    next_key: u64,
    out: Vec<NetCommand>,
    /// At most one pending request per action: a newer click supersedes it.
    actions: [Option<PendingAction>; 2],
    /// Bumped whenever an asset or a region handle arrives (cached lookups
    /// such as "a landmark already points here" rebuild).
    pub version: u64,
    /// [`named`] for the inventory generation it was built from.
    named: Option<(u64, std::sync::Arc<[Named]>)>,
}

/// LLViewerParcelMgr::inAgentParcel: is the global position `pos` on the
/// agent's parcel? `bitmap` holds the parcel's cells (ParcelProperties
/// Bitmap: one bit per 4 m cell, x + y * cells, low bit first, as
/// writeAgentParcelFromBitmap reads it) and `origin` is the agent region's
/// corner; another region is never the agent's parcel.
pub fn in_parcel(bitmap: &[u8], origin: (u32, u32), pos: DVec3) -> bool {
    let cells = (bitmap.len() * 8).isqrt();
    let width = (cells * 4) as f64;
    let (x, y) = (pos.x - f64::from(origin.0), pos.y - f64::from(origin.1));
    if cells == 0 || !(0.0..width).contains(&x) || !(0.0..width).contains(&y) {
        return false;
    }
    let i = (y / 4.0) as usize * cells + (x / 4.0) as usize;
    bitmap.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
}

impl Landmarks {
    /// [`named`], walked again only when the inventory changed.
    pub fn named(&mut self, inv: &Inventory) -> std::sync::Arc<[Named]> {
        match &self.named {
            Some((generation, list)) if *generation == inv.generation => list.clone(),
            _ => {
                let list: std::sync::Arc<[Named]> = named(inv).into();
                self.named = Some((inv.generation, list.clone()));
                list
            }
        }
    }

    /// Resolve inventory destination actions asynchronously, including assets
    /// that have never been opened. Never use the inventory label as a region.
    pub fn request_action(&mut self, asset: Uuid, action: Action, now: Instant) {
        let slot = match action {
            Action::CopySlurl => 0,
            Action::ShowOnMap => 1,
        };
        self.actions[slot] = Some(PendingAction {
            asset,
            action,
            since: now,
            asked_region: false,
        });
        self.resolve(asset);
    }

    pub fn take_actions(&mut self, map: &mut super::worldmap::WorldMap, now: Instant) -> Vec<ActionResult> {
        let mut results = Vec::new();
        for (slot, pending) in std::mem::take(&mut self.actions).into_iter().enumerate() {
            let Some(mut pending) = pending else { continue };
            if pending.asset.is_nil() || now.saturating_duration_since(pending.since) >= Duration::from_secs(30) {
                results.push(ActionResult::Unavailable);
                continue;
            }
            match self.resolve(pending.asset) {
                Resolved::Failed => results.push(ActionResult::Unavailable),
                Resolved::Ready { global, region_pos, .. } => {
                    if !global.is_finite() || global == DVec3::ZERO || global.x < 0.0 || global.y < 0.0 {
                        results.push(ActionResult::Unavailable);
                    } else if pending.action == Action::ShowOnMap {
                        results.push(ActionResult::Map(global));
                    } else if let Some(((x, y), sim)) = map.sim_at_global(global.x, global.y).filter(|(_, sim)| !sim.name.is_empty()) {
                        let local = Vec3::new(
                            (global.x - f64::from(x) * 256.0) as f32,
                            (global.y - f64::from(y) * 256.0) as f32,
                            global.z as f32,
                        );
                        results.push(ActionResult::Slurl(crate::slurl::make(&sim.name, local)));
                    } else {
                        let (x, y) = (
                            ((global.x - f64::from(region_pos.x)) / 256.0).round() as u16,
                            ((global.y - f64::from(region_pos.y)) / 256.0).round() as u16,
                        );
                        if pending.asked_region && map.null_sims.contains(&(u32::from(x), u32::from(y))) {
                            results.push(ActionResult::Unavailable);
                        } else {
                            if !pending.asked_region {
                                // LLLandmarkActions::getSLURLfromPosGlobal /
                                // LLWorldMapMessage::sendHandleRegionRequest:
                                // one cell, including a reply for a removed region.
                                map.null_sims.remove(&(u32::from(x), u32::from(y)));
                                map.push(NetCommand::MapBlockRequest {
                                    min_x: x,
                                    min_y: y,
                                    max_x: x,
                                    max_y: y,
                                    null_sims: true,
                                });
                                pending.asked_region = true;
                            }
                            self.actions[slot] = Some(pending);
                        }
                    }
                }
                Resolved::Pending => self.actions[slot] = Some(pending),
            }
        }
        results
    }

    /// LLLandmarkList::getAsset: load the asset if it is not known yet.
    pub fn want(&mut self, asset: Uuid) {
        if !asset.is_nil() {
            self.assets.entry(asset).or_insert(State::Wanted);
        }
    }

    /// Assets to fetch now, with their fetch keys.
    pub fn take_wanted(&mut self) -> Vec<(u64, Uuid)> {
        let mut v = Vec::new();
        for (id, s) in self.assets.iter_mut() {
            if *s == State::Wanted {
                *s = State::Fetching;
                self.next_key = (self.next_key + 1) & ((1 << 60) - 1);
                let key = FETCH_KIND_LANDMARK | self.next_key;
                self.keys.insert(key, *id);
                v.push((key, *id));
            }
        }
        v
    }

    /// A fetch finished (LLLandmarkList::processGetAssetReply); None = failed.
    pub fn on_fetch(&mut self, key: u64, data: Option<&[u8]>) {
        if let Some(id) = self.keys.remove(&key) {
            self.on_data(id, data);
        }
    }

    /// The asset's bytes (or None when it could not be loaded).
    pub fn on_data(&mut self, asset: Uuid, data: Option<&[u8]>) {
        let state = match data.and_then(parse) {
            Some(a) => State::Ready(a),
            None => {
                log::warn!("landmark asset {asset} could not be loaded");
                State::Failed
            }
        };
        self.assets.insert(asset, state);
        self.version += 1;
    }

    /// LLLandmark::setRegionHandle: the agent's region is always known.
    pub fn set_local_region(&mut self, region_id: Uuid, handle: RegionHandle) {
        if !region_id.is_nil() && self.handles.insert(region_id, handle) != Some(handle) {
            self.version += 1;
        }
    }

    /// RegionIDAndHandleReply.
    pub fn on_region_handle(&mut self, region_id: Uuid, handle: RegionHandle) {
        self.asked.remove(&region_id);
        self.handles.insert(region_id, handle);
        self.version += 1;
    }

    /// What is known of a landmark, without asking for anything (the
    /// interface reads it; `resolve` is called by the app).
    pub fn peek(&self, asset: Uuid) -> Resolved {
        match self.assets.get(&asset) {
            Some(State::Failed) => Resolved::Failed,
            Some(State::Ready(LandmarkAsset::Global(g))) => Resolved::Ready {
                region_id: Uuid::nil(),
                region_pos: Vec3::new(g.x.rem_euclid(256.0) as f32, g.y.rem_euclid(256.0) as f32, g.z as f32),
                global: *g,
            },
            Some(State::Ready(LandmarkAsset::Region { region_id, pos })) => match self.handles.get(region_id) {
                Some(&h) => {
                    let (x, y) = aurora_net::handle_to_origin(h);
                    Resolved::Ready {
                        region_id: *region_id,
                        region_pos: *pos,
                        global: DVec3::new(x as f64 + pos.x as f64, y as f64 + pos.y as f64, pos.z as f64),
                    }
                }
                None => Resolved::Pending,
            },
            _ => Resolved::Pending,
        }
    }

    /// LLLandmark::getGlobalPos, asking the asset or the region's handle when
    /// unknown.
    pub fn resolve(&mut self, asset: Uuid) -> Resolved {
        if let r @ (Resolved::Ready { .. } | Resolved::Failed) = self.peek(asset) {
            return r;
        }
        match self.assets.get(&asset).copied() {
            None => self.want(asset),
            // the asset is read, its region's handle is not known yet
            Some(State::Ready(LandmarkAsset::Region { region_id, .. })) if self.asked.insert(region_id) => {
                self.out.push(NetCommand::RegionHandleRequest(region_id));
            }
            _ => {}
        }
        Resolved::Pending
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }

    /// fetch_landmarks with LLFirstAgentParcelLandmark / LLFetchFirstLandmarkByPos:
    /// the first landmark of the Favorites or Landmarks folders, subfolders
    /// included, whose global position `matches`. As in Firestorm, the
    /// assets not loaded yet are asked for and count as pointing elsewhere
    /// meanwhile.
    pub fn find(&mut self, inv: &Inventory, matches: impl Fn(DVec3) -> bool) -> Option<Uuid> {
        let mut stack: Vec<Uuid> = [FOLDER_FAVORITES, FOLDER_LANDMARKS]
            .into_iter()
            .filter_map(|kind| crate::world::appearance::system_folder(inv, kind))
            .collect();
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            let Some(folder) = inv.folders.get(&id).filter(|_| seen.insert(id)) else {
                continue;
            };
            stack.extend(folder.children.iter().copied());
            for it in folder.items.iter().filter_map(|i| inv.items.get(i)) {
                if it.asset_type != ASSET_LANDMARK {
                    continue;
                }
                self.want(it.asset_id);
                if let Resolved::Ready { global, .. } = self.peek(it.asset_id)
                    && matches(global)
                {
                    return Some(it.id);
                }
            }
        }
        None
    }
}

/// The folders of the Favorites and Landmarks trees never fetched yet: the
/// landmark lookups only see fetched folders, where Firestorm has the whole
/// inventory loaded in the background after login.
pub fn unfetched_folders(inv: &Inventory) -> Vec<Uuid> {
    let mut stack: Vec<Uuid> = [FOLDER_FAVORITES, FOLDER_LANDMARKS]
        .into_iter()
        .filter_map(|kind| crate::world::appearance::system_folder(inv, kind))
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if let Some(folder) = inv.folders.get(&id).filter(|_| seen.insert(id)) {
            stack.extend(folder.children.iter().copied());
            if folder.state == crate::world::inventory::FetchState::Unknown {
                out.push(id);
            }
        }
    }
    out
}

/// A landmark the navigation bar can suggest by its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Named {
    pub name: String,
    /// `name` in lower case, the form it is matched in.
    pub lower: String,
    pub asset: Uuid,
}

/// The landmarks of the Favorites tree, then of the Landmarks tree
/// (subfolders included, as fetch_landmarks / collectDescendentsIf), one
/// per lower-case name, the first one found winning (LLFetchLandmarksByName's
/// check_duplicate, lllandmarkactions.cpp). Unlike the Legacy Viewer, a
/// landmark whose asset is not loaded yet is listed too: it skips them only
/// because it reads the asset to list them, and TeleportLandmarkRequest
/// needs nothing but the asset id.
pub fn named(inv: &Inventory) -> Vec<Named> {
    let mut out = Vec::new();
    let mut names = HashSet::new();
    let mut seen = HashSet::new();
    for kind in [FOLDER_FAVORITES, FOLDER_LANDMARKS] {
        let Some(root) = crate::world::appearance::system_folder(inv, kind) else {
            continue;
        };
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(folder) = inv.folders.get(&id).filter(|_| seen.insert(id)) else {
                continue;
            };
            // reversed: the walk goes down the subfolders in their order
            stack.extend(folder.children.iter().rev().copied());
            // a link stands for its original: its name, type and asset
            // (LLViewerInventoryItem::getName / getType / getAssetUUID)
            let originals = folder
                .items
                .iter()
                .filter_map(|i| crate::world::inventory::actions::original(inv, *i))
                .filter_map(|i| inv.items.get(&i));
            for it in originals {
                if it.asset_type != ASSET_LANDMARK || it.asset_id.is_nil() {
                    continue;
                }
                let lower = it.name.to_lowercase();
                if names.insert(lower.clone()) {
                    out.push(Named {
                        name: it.name.clone(),
                        lower,
                        asset: it.asset_id,
                    });
                }
            }
        }
    }
    out
}

/// FT_FAVORITE and FT_LANDMARK, the two folders landmarks are looked for in.
const FOLDER_FAVORITES: i32 = 23;
const FOLDER_LANDMARKS: i32 = 3;
const ASSET_LANDMARK: i32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    const REGION: Uuid = Uuid::from_u128(0x5E61_0000_0000_0000_0000_0000_0000_0001);

    #[test]
    fn inventory_actions_wait_for_asset_handle_and_actual_region_name() {
        let now = Instant::now();
        let asset = Uuid::from_u128(42);
        let mut l = Landmarks::default();
        let mut map = super::super::worldmap::WorldMap::default();
        l.request_action(asset, Action::CopySlurl, now);
        l.request_action(asset, Action::ShowOnMap, now);
        assert!(l.take_actions(&mut map, now).is_empty());
        let wanted = l.take_wanted();
        assert_eq!(wanted.len(), 1);
        let data = format!("Landmark version 2\nregion_id {REGION}\nlocal_pos 300.4 20.6 30.5\n");
        l.on_fetch(wanted[0].0, Some(data.as_bytes()));
        assert!(l.take_actions(&mut map, now).is_empty());
        assert!(matches!(&l.take_commands()[..], [NetCommand::RegionHandleRequest(id)] if *id == REGION));
        assert!(l.take_actions(&mut map, now).is_empty());
        assert!(l.take_commands().is_empty(), "one shared handle request");
        l.on_region_handle(REGION, aurora_net::origin_to_handle(256_000, 256_000));
        let Resolved::Ready { global, .. } = l.peek(asset) else {
            panic!("resolved landmark")
        };
        assert_eq!(l.take_actions(&mut map, now), vec![ActionResult::Map(global)]);
        assert!(matches!(
            &map.take_commands()[..],
            [NetCommand::MapBlockRequest {
                min_x: 1000,
                min_y: 1000,
                max_x: 1000,
                max_y: 1000,
                null_sims: true,
            }]
        ));
        assert!(l.take_actions(&mut map, now).is_empty());
        assert!(map.take_commands().is_empty(), "map requests are deduplicated");
        map.sims.insert(
            (1000, 1000),
            super::super::worldmap::SimInfo {
                name: "Demo & Test".into(),
                access: 13,
                flags: 0,
                image: Uuid::nil(),
                size_x: 512,
                size_y: 512,
            },
        );
        assert_eq!(
            l.take_actions(&mut map, now),
            vec![ActionResult::Slurl(
                "http://maps.secondlife.com/secondlife/Demo%20%26%20Test/300/21/31".into()
            )]
        );
        assert!(l.take_actions(&mut map, now).is_empty(), "actions complete once");
    }

    #[test]
    fn inventory_actions_supersede_old_clicks_and_finish_failed_or_stalled_requests() {
        let now = Instant::now();
        let mut l = Landmarks::default();
        let mut map = super::super::worldmap::WorldMap::default();
        let old = Uuid::from_u128(42);
        let new = Uuid::from_u128(43);
        l.request_action(old, Action::ShowOnMap, now);
        l.request_action(new, Action::ShowOnMap, now);
        l.on_data(old, Some(b"Landmark version 1\nposition 256010.1 256020.1 30\n"));
        assert!(l.take_actions(&mut map, now).is_empty(), "old click cannot move the map later");
        l.on_data(new, None);
        assert_eq!(l.take_actions(&mut map, now), vec![ActionResult::Unavailable]);
        let stalled = Uuid::from_u128(44);
        l.request_action(stalled, Action::CopySlurl, now);
        assert!(l.take_actions(&mut map, now).is_empty());
        assert_eq!(
            l.take_actions(&mut map, now + Duration::from_secs(31)),
            vec![ActionResult::Unavailable]
        );
        for data in [
            b"Landmark version 1\nposition NaN 10 30\n".as_slice(),
            b"Landmark version 1\nposition 0 0 0\n".as_slice(),
        ] {
            let asset = Uuid::from_u128(44);
            l.request_action(asset, Action::CopySlurl, now);
            l.on_data(asset, Some(data));
            assert_eq!(l.take_actions(&mut map, now), vec![ActionResult::Unavailable]);
            assert!(l.take_actions(&mut map, now).is_empty());
        }
        l.request_action(old, Action::ShowOnMap, now);
        assert_eq!(
            l.take_actions(&mut map, now),
            vec![ActionResult::Map(DVec3::new(256010.1, 256020.1, 30.0))]
        );
        l.request_action(Uuid::nil(), Action::CopySlurl, now);
        assert_eq!(l.take_actions(&mut map, now), vec![ActionResult::Unavailable]);
        map.null_sims.insert((1000, 1000));
        l.request_action(old, Action::CopySlurl, now);
        assert!(l.take_actions(&mut map, now).is_empty(), "refresh a cached missing region");
        map.null_sims.insert((1000, 1000));
        assert_eq!(l.take_actions(&mut map, now), vec![ActionResult::Unavailable]);
    }

    #[test]
    fn parses_both_versions() {
        let v2 = format!("Landmark version 2\nregion_id {REGION}\nlocal_pos 148.5 195 24\n");
        assert_eq!(
            parse(v2.as_bytes()),
            Some(LandmarkAsset::Region {
                region_id: REGION,
                pos: Vec3::new(148.5, 195.0, 24.0)
            })
        );
        let v1 = b"Landmark version 1\nposition 256148 256195 24\n";
        assert_eq!(parse(v1), Some(LandmarkAsset::Global(DVec3::new(256148.0, 256195.0, 24.0))));
        assert_eq!(parse(b"Landmark version 3\n"), None);
        assert_eq!(
            parse(b"Landmark version 2\nregion_id 00000000-0000-0000-0000-000000000000\nlocal_pos 1 2 3\n"),
            None
        );
        assert_eq!(parse(b"Landmark version 2\nregion_id oops\nlocal_pos 1 2 3\n"), None);
        assert_eq!(parse(b"garbage"), None);
    }

    #[test]
    fn fetch_then_region_handle() {
        let asset = Uuid::from_u128(42);
        let mut l = Landmarks::default();
        assert_eq!(l.resolve(asset), Resolved::Pending);
        let wanted = l.take_wanted();
        assert_eq!(wanted.len(), 1);
        assert!(l.take_wanted().is_empty(), "fetched once");
        let data = format!("Landmark version 2\nregion_id {REGION}\nlocal_pos 10 20 30\n");
        l.on_fetch(wanted[0].0, Some(data.as_bytes()));
        assert_eq!(l.resolve(asset), Resolved::Pending);
        assert!(matches!(&l.take_commands()[..], [NetCommand::RegionHandleRequest(r)] if *r == REGION));
        assert_eq!(l.resolve(asset), Resolved::Pending);
        assert!(l.take_commands().is_empty(), "asked once");
        l.on_region_handle(REGION, aurora_net::origin_to_handle(256_000, 256_256));
        assert_eq!(
            l.resolve(asset),
            Resolved::Ready {
                region_id: REGION,
                region_pos: Vec3::new(10.0, 20.0, 30.0),
                global: DVec3::new(256_010.0, 256_276.0, 30.0)
            }
        );
        let other = Uuid::from_u128(43);
        l.want(other);
        let k = l.take_wanted()[0].0;
        l.on_fetch(k, None);
        assert_eq!(l.resolve(other), Resolved::Failed);
    }

    #[test]
    fn the_parcel_bitmap_holds_one_bit_per_4_m_cell() {
        // a 256 m region: 64 x 64 cells; the parcel is cells x 2..=3, y 10
        let mut bitmap = vec![0u8; 512];
        for x in 2..=3 {
            let i = 10 * 64 + x;
            bitmap[i / 8] |= 1 << (i % 8);
        }
        let origin = (256_000, 512_000);
        let at = |x: f64, y: f64| DVec3::new(256_000.0 + x, 512_000.0 + y, 20.0);
        assert!(in_parcel(&bitmap, origin, at(8.0, 40.0)));
        assert!(in_parcel(&bitmap, origin, at(15.9, 43.9)));
        assert!(!in_parcel(&bitmap, origin, at(16.0, 40.0)), "next cell east");
        assert!(!in_parcel(&bitmap, origin, at(8.0, 44.0)), "next cell north");
        assert!(!in_parcel(&bitmap, origin, at(8.0 + 256.0, 40.0)), "same cell of the neighbour");
        assert!(!in_parcel(&bitmap, origin, at(-248.0, 40.0)), "west neighbour");
        assert!(!in_parcel(&[], origin, at(8.0, 40.0)), "parcel not known yet");
        // a 512 m variable region: 128 x 128 cells
        let mut big = vec![0u8; 2048];
        let i = 100 * 128 + 120;
        big[i / 8] |= 1 << (i % 8);
        assert!(in_parcel(&big, origin, at(481.0, 401.0)));
        assert!(!in_parcel(&big, origin, at(481.0, 401.0 - 256.0)));
    }

    #[test]
    fn finds_a_landmark_of_the_same_spot_in_landmark_folders_only() {
        use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
        let folder = |id: u128, parent: u128, t: i32| InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: format!("f{id}"),
            type_default: t,
            version: 1,
            favorite: false,
            favorite_known: false,
            thumbnail: Uuid::nil(),
        };
        let landmark = |id: u128, parent: u128| InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            asset_id: Uuid::from_u128(id + 1000),
            asset_type: 3,
            inv_type: 3,
            ..Default::default()
        };
        let contents = |f: InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>| FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        };
        let mut inv = Inventory::default();
        let (root, lms, trips, notes) = (folder(1, 0, 8), folder(10, 1, 3), folder(11, 10, -1), folder(20, 1, 7));
        inv.apply(vec![contents(root, vec![lms.clone(), notes.clone()], Vec::new())]);
        inv.apply(vec![contents(lms, vec![trips.clone()], Vec::new())]);
        inv.apply(vec![contents(trips, Vec::new(), vec![landmark(100, 11)])]);
        inv.apply(vec![contents(notes, Vec::new(), vec![landmark(200, 20)])]);
        let mut l = Landmarks::default();
        l.set_local_region(REGION, aurora_net::origin_to_handle(256_000, 256_000));
        let here = DVec3::new(256_010.4, 256_020.0, 30.0);
        let same_metre = |g: DVec3| g.round() == here.round();
        assert_eq!(l.find(&inv, same_metre), None, "assets not loaded yet");
        assert_eq!(l.take_wanted().len(), 1, "only the landmark under Landmarks is asked for");
        for (asset, x) in [(1100u128, 10.0), (1200, 10.0)] {
            let text = format!("Landmark version 2\nregion_id {REGION}\nlocal_pos {x} 20.2 29.6\n");
            l.on_data(Uuid::from_u128(asset), Some(text.as_bytes()));
        }
        assert_eq!(l.find(&inv, same_metre), Some(Uuid::from_u128(100)));
        assert_eq!(l.find(&inv, |g| g.round() == (here + DVec3::X).round()), None, "one metre away");
    }

    #[test]
    fn named_lists_favorites_then_landmarks_once_per_lower_case_name() {
        use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
        let folder = |id: u128, parent: u128, t: i32| InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: format!("f{id}"),
            type_default: t,
            version: 1,
            ..Default::default()
        };
        let landmark = |id: u128, parent: u128, name: &str| InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            asset_id: Uuid::from_u128(id + 1000),
            asset_type: 3,
            inv_type: 3,
            ..Default::default()
        };
        let contents = |f: InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>| FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        };
        let mut inv = Inventory::default();
        let (root, favs, lms, trips, notes) = (
            folder(1, 0, 8),
            folder(10, 1, 23),
            folder(20, 1, 3),
            folder(21, 20, -1),
            folder(30, 1, 7),
        );
        inv.apply(vec![contents(root, vec![lms.clone(), favs.clone(), notes.clone()], Vec::new())]);
        inv.apply(vec![contents(favs, Vec::new(), vec![landmark(100, 10, "Chonk Landing")])]);
        inv.apply(vec![contents(
            lms,
            vec![trips.clone()],
            vec![landmark(200, 20, "chonk landing"), landmark(201, 20, "Beach")],
        )]);
        let mut no_asset = landmark(210, 21, "Broken");
        no_asset.asset_id = Uuid::nil();
        // a link to a landmark kept elsewhere stands for it
        let link = InvItem {
            id: Uuid::from_u128(212),
            parent: Uuid::from_u128(21),
            name: "link name".into(),
            asset_id: Uuid::from_u128(300),
            asset_type: 24,
            inv_type: 3,
            ..Default::default()
        };
        inv.apply(vec![contents(
            trips,
            Vec::new(),
            vec![landmark(211, 21, "KB Store"), no_asset, link],
        )]);
        inv.apply(vec![contents(notes, Vec::new(), vec![landmark(300, 30, "Elsewhere")])]);
        let names: Vec<(String, Uuid)> = named(&inv).into_iter().map(|n| (n.name, n.asset)).collect();
        assert_eq!(
            names,
            vec![
                ("Chonk Landing".into(), Uuid::from_u128(1100)),
                ("Beach".into(), Uuid::from_u128(1201)),
                ("KB Store".into(), Uuid::from_u128(1211)),
                ("Elsewhere".into(), Uuid::from_u128(1300)),
            ],
            "the Favorites copy wins; subfolders and links count; other folders and assetless items do not"
        );
    }

    #[test]
    fn local_region_needs_no_request() {
        let asset = Uuid::from_u128(44);
        let mut l = Landmarks::default();
        l.set_local_region(REGION, aurora_net::origin_to_handle(256_000, 256_000));
        l.on_data(
            asset,
            Some(format!("Landmark version 2\nregion_id {REGION}\nlocal_pos 1 2 3\n").as_bytes()),
        );
        assert!(matches!(l.resolve(asset), Resolved::Ready { .. }));
        assert!(l.take_commands().is_empty());
    }
}
