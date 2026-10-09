//! Landmark assets and where they point: the asset text (region id and
//! position in the region, or an old global position) fetched once per
//! asset, and the handles of the regions they name, asked with
//! RegionHandleRequest. Port of LLLandmark (indra/llinventory/lllandmark.cpp:
//! constructFromString, getGlobalPos, requestRegionHandle,
//! processRegionIDAndHandle) and LLLandmarkList::getAsset
//! (indra/newview/lllandmarklist.cpp), originally LGPL 2.1.

use aurora_net::{NetCommand, RegionHandle};
use glam::{DVec3, Vec3};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

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
}

impl Landmarks {
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
    }

    /// LLLandmark::setRegionHandle: the agent's region is always known.
    pub fn set_local_region(&mut self, region_id: Uuid, handle: RegionHandle) {
        if !region_id.is_nil() {
            self.handles.insert(region_id, handle);
        }
    }

    /// RegionIDAndHandleReply.
    pub fn on_region_handle(&mut self, region_id: Uuid, handle: RegionHandle) {
        self.asked.remove(&region_id);
        self.handles.insert(region_id, handle);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION: Uuid = Uuid::from_u128(0x5E61_0000_0000_0000_0000_0000_0000_0001);

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
