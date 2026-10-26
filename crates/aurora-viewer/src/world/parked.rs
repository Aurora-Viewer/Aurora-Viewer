//! Regions out of reach.
//!
//! After a teleport, the simulator of the region left only closes its
//! circuit tens of seconds later (DisableSimulator, 50 s measured on Agni).
//! Until then Firestorm keeps the region and its objects
//! (process_disable_simulator -> LLWorld::removeRegion, llworld.cpp) and
//! simply does not walk a region that is not in view. Here every object in
//! the store costs the scene sync, the draw lists, the texture streaming and
//! the particles each frame wherever it is: a skybox ran at 320 frames a
//! second instead of 840 for those 50 s.
//!
//! The objects must be kept all the same: a simulator that has not closed
//! its circuit does not send again what it already sent. Back in the region
//! within those seconds, only the avatars and what they wear came anew.
//!
//! So the objects of a region farther than [`OUT_OF_REACH`] from the agent's
//! are parked: taken out of the store, which frees everything the scene
//! held for them, and kept here with the object events their region still
//! sends. They return as they were when the region is entered again or back
//! in reach, and go for good with the region (`RegionRemoved`). Its coarse
//! locations wait with them: the avatars there are not around either.

use super::World;
use super::objects::{ObjKey, Object};
use aurora_net::{NetEvent, RegionHandle, handle_to_origin};
use glam::Vec3;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

/// Farther than this from the agent's region, nothing of a region can be
/// drawn: the draw distance stops at 512 m, and what a region holds stays
/// within a few tens of metres of its borders.
pub const OUT_OF_REACH: f64 = 1024.0;

/// Metres between two regions, from their handles and sizes (0 when they
/// touch or overlap).
pub fn region_gap(a: (RegionHandle, (u32, u32)), b: (RegionHandle, (u32, u32))) -> f64 {
    let axis = |a0: u32, a_len: u32, b0: u32, b_len: u32| {
        let (a0, b0) = (f64::from(a0), f64::from(b0));
        (b0 - (a0 + f64::from(a_len))).max(a0 - (b0 + f64::from(b_len))).max(0.0)
    };
    let ((ax, ay), (bx, by)) = (handle_to_origin(a.0), handle_to_origin(b.0));
    axis(ax, a.1.0, bx, b.1.0).hypot(axis(ay, a.1.1, by, b.1.1))
}

/// What a parked region holds.
#[derive(Default)]
pub struct Parked {
    /// Its objects, in the order `ObjectStore::restore` takes them back.
    objects: Vec<Object>,
    /// Their GLTF material overrides.
    overrides: Vec<(ObjKey, Arc<[(u8, aurora_assets::material::PbrOverride)]>)>,
    /// Object events of the region received since, in order.
    events: Vec<NetEvent>,
    /// Its last coarse locations: the simulator left keeps sending them
    /// while the agent is its child, until it closes the region. Shown,
    /// they would keep the avatars there "around" (issue #1000).
    coarse: Option<Vec<(Uuid, Vec3)>>,
    /// How many lists it sent while parked.
    coarse_lists: usize,
}

impl World {
    /// An object event of a parked region waits for its objects: None when
    /// the event was put aside.
    pub(super) fn hold_for_parked(&mut self, ev: NetEvent) -> Option<NetEvent> {
        if self.parked.is_empty() {
            return Some(ev);
        }
        let handle = match &ev {
            NetEvent::ObjectUpdates { handle, .. }
            | NetEvent::TerseUpdates { handle, .. }
            | NetEvent::ObjectsKilled { handle, .. }
            | NetEvent::GltfOverrides { handle, .. }
            | NetEvent::CoarseLocations { handle, .. } => *handle,
            _ => return Some(ev),
        };
        let Some(p) = self.parked.get_mut(&handle) else {
            return Some(ev);
        };
        match ev {
            NetEvent::CoarseLocations { positions, .. } => {
                p.coarse = Some(positions);
                p.coarse_lists += 1;
                if p.coarse_lists == 1 {
                    log::info!(
                        "region {} still sends its coarse locations: held while out of reach",
                        self.regions.get(&handle).map_or("", |r| r.name.as_str())
                    );
                }
            }
            ev => p.events.push(ev),
        }
        None
    }

    /// The agent has arrived in its region: the regions now out of reach
    /// are parked, those back in reach get their objects again.
    pub(super) fn park_far_regions(&mut self) {
        let Some(main) = self.main_region else {
            return;
        };
        let rect = |w: &World, h: RegionHandle| w.regions.get(&h).map(|r| (h, (r.heightmap.size_x, r.heightmap.size_y)));
        let Some(main_rect) = rect(self, main) else {
            return;
        };
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for &h in self.regions.keys() {
            let gap = rect(self, h).map_or(0.0, |r| region_gap(main_rect, r));
            if h != main && gap > OUT_OF_REACH {
                far.push((h, gap));
            } else {
                near.push(h);
            }
        }
        for h in near {
            self.unpark_region(h);
        }
        for (h, gap) in far {
            self.park_region(h, gap);
        }
    }

    fn park_region(&mut self, handle: RegionHandle, gap: f64) {
        if self.parked.contains_key(&handle) {
            return;
        }
        // LLViewerObjectList::killObject never destroys gAgentAvatarp: our
        // avatar moves to the agent's region with what it wears
        let kept = match (self.objects.index_of_uuid(&self.agent_id), self.main_region) {
            (Some(own), Some(main)) if main != handle => {
                self.objects.carry(own, handle, main);
                self.objects.subtree(own)
            }
            _ => Vec::new(),
        };
        let started = std::time::Instant::now();
        let objects = self.objects.park_region(handle, &kept);
        self.move_rekeyed();
        // what is parked cannot be seen: a look-at of an avatar left there
        // would follow us (on our head when it looked at us)
        self.forget_gone_look_ats();
        let keys: Vec<ObjKey> = self.gltf_overrides.keys().filter(|k| k.region == handle).copied().collect();
        let overrides = keys
            .into_iter()
            .filter_map(|k| self.gltf_overrides.remove(&k).map(|sides| (k, sides)))
            .collect();
        log::info!(
            "region {} is {gap:.0} m away: {} objects parked in {:.2} ms until it is closed or in reach again",
            self.regions.get(&handle).map_or("", |r| r.name.as_str()),
            objects.len(),
            started.elapsed().as_secs_f64() * 1e3
        );
        let coarse = self.coarse.remove(&handle);
        self.parked.insert(
            handle,
            Parked {
                objects,
                overrides,
                events: Vec::new(),
                coarse,
                coarse_lists: 0,
            },
        );
    }

    /// The objects of a parked region come back as they were, then take
    /// what their region sent in the meantime.
    pub(super) fn unpark_region(&mut self, handle: RegionHandle) {
        let Some(p) = self.parked.remove(&handle) else {
            return;
        };
        let started = std::time::Instant::now();
        let total = p.objects.len();
        // an object that came from elsewhere in the meantime is the newer
        // one: its parked self is dropped with what hung from it
        let mut dropped: HashSet<ObjKey> = HashSet::new();
        for obj in p.objects {
            let key = obj.key;
            if obj.parent_key().is_some_and(|k| dropped.contains(&k)) || self.objects.restore(obj).is_none() {
                dropped.insert(key);
            }
        }
        for (key, sides) in p.overrides {
            if self.objects.index_of(&key).is_some() {
                self.gltf_overrides.entry(key).or_insert(sides);
            }
        }
        let restored = started.elapsed();
        let events = p.events.len();
        for ev in p.events {
            self.apply(ev);
        }
        if let Some(positions) = p.coarse {
            self.coarse.insert(handle, positions);
        }
        log::info!(
            "region {} is in reach again: {} objects back in {:.2} ms, {events} events caught up in {:.2} ms",
            self.regions.get(&handle).map_or("", |r| r.name.as_str()),
            total - dropped.len(),
            restored.as_secs_f64() * 1e3,
            (started.elapsed() - restored).as_secs_f64() * 1e3
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (u32, u32) = (256, 256);

    #[test]
    fn regions_out_of_reach_are_those_far_from_the_agent_region() {
        let at = |x: u32, y: u32| (aurora_net::origin_to_handle(x * 256, y * 256), SIZE);
        let main = at(1000, 1000);
        // itself, an edge neighbour and a corner neighbour touch it
        assert_eq!(region_gap(main, main), 0.0);
        assert_eq!(region_gap(main, at(1001, 1000)), 0.0);
        assert_eq!(region_gap(main, at(999, 1001)), 0.0);
        // two regions away on one axis, then on both; either way round
        assert_eq!(region_gap(main, at(1002, 1000)), 256.0);
        assert_eq!(region_gap(at(1002, 1000), main), 256.0);
        assert_eq!(region_gap(main, at(997, 996)), 512.0f64.hypot(768.0));
        // what a draw distance of 512 m can show is in reach, a teleport
        // across the map is not
        assert!(region_gap(main, at(1003, 1003)) < OUT_OF_REACH);
        assert!(region_gap(main, at(1006, 1000)) > OUT_OF_REACH);
        assert!(region_gap(main, at(677, 1082)) > OUT_OF_REACH);
        // a large region reaches its far neighbours
        let wide = (aurora_net::origin_to_handle(1000 * 256, 1000 * 256), (1024, 1024));
        assert_eq!(region_gap(wide, at(1004, 1002)), 0.0);
        assert_eq!(region_gap(at(1005, 1002), wide), 256.0);
    }
}
