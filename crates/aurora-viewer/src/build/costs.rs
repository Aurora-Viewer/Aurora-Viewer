//! Land impact and weights of the selection, and the parcel capacity, as
//! the build floater shows them (LLFloaterTools::refresh / updateLandImpacts,
//! LLFloaterObjectWeights, LLViewerObjectList::fetchObjectCosts /
//! fetchPhysicsFlags, LLAccountingCostManager; originally LGPL 2.1).
//!
//! - GetObjectCost {object_ids} → per object {linked_set_resource_cost...}
//! - ResourceCostSelected {selected_roots} → {selected: {physics, streaming, simulation}}
//! - GetObjectPhysicsData {object_ids} → per object physics shape and material
//! - ParcelPropertiesRequest at the first object → SimWideMaxPrims / TotalPrims

use super::BuildTool;
use super::edits::CapPurpose;
use crate::world::World;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::build::{BUILD_PARCEL_SEQ, BuildCmd, PhysicsParams};
use aurora_net::{ParcelInfo, RegionHandle};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Costs are asked for again this long after the last answer while the
/// selection stays (edits change them; LL marks objects stale on updates).
const REFRESH: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, Default)]
pub struct ObjectCost {
    /// linked_set_resource_cost: the land impact of the object's linkset.
    pub linked_set: f32,
}

/// ResourceCostSelected answer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Weights {
    pub download: f32,
    pub physics: f32,
    pub server: f32,
}

#[derive(Debug, Default)]
pub struct Costs {
    pub objects: HashMap<Uuid, ObjectCost>,
    /// Selection weights; `None` while unknown, Err when the request failed.
    pub weights: Option<Result<Weights, String>>,
    /// Parcel under the first selected object (None: selection on several
    /// parcels, or not known yet).
    pub parcel: Option<(RegionHandle, Arc<ParcelInfo>)>,
    /// Roots the last requests were made for, and when.
    asked: Vec<Uuid>,
    asked_at: Option<Instant>,
    parcel_seq: i32,
}

impl Costs {
    /// Total and remaining capacity (updateLandImpacts): min(parcel's
    /// sim-wide max, region capacity) and that minus the sim-wide count.
    pub fn capacity(&self, world: &World) -> Option<(i32, i32)> {
        let (handle, p) = self.parcel.as_ref()?;
        // SimStats ObjectCapacity (LLViewerRegion::getMaxTasks), 0 until known
        let max_tasks = world.regions.get(handle).map(|r| r.max_tasks).unwrap_or(0);
        Some(capacity_of(p, max_tasks))
    }
}

impl BuildTool {
    /// Land impact of the selection: the linkset cost of each distinct root
    /// (LLSelectMgr::getSelectedLinksetCost, attachments excluded), or
    /// None while one is unknown.
    pub fn land_impact(&self, world: &World) -> Option<f32> {
        let mut total = 0.0;
        for r in self.roots(world) {
            let o = world.objects.get(r)?;
            total += self.costs.objects.get(&o.full_id)?.linked_set;
        }
        Some(total)
    }

    /// Ask for the costs, weights, physics data and parcel when the
    /// selection changed or the last answers are old.
    pub fn refresh_costs(&mut self, world: &World) {
        let roots: Vec<(Uuid, RegionHandle, glam::Vec3)> = self
            .roots(world)
            .into_iter()
            .filter_map(|r| {
                let o = world.objects.get(r)?;
                let pos = crate::scene::Scene::object_transform(world, r, Instant::now(), 0)?.0;
                Some((o.full_id, o.key.region, pos))
            })
            .collect();
        let ids: Vec<Uuid> = roots.iter().map(|r| r.0).collect();
        let changed = ids != self.costs.asked;
        let old = self.costs.asked_at.is_none_or(|t| t.elapsed() > REFRESH);
        if ids.is_empty() {
            if changed {
                self.costs.asked.clear();
                self.costs.weights = None;
                self.costs.parcel = None;
            }
            return;
        }
        if !changed && !old {
            return;
        }
        if changed {
            self.costs.weights = None;
            self.costs.parcel = None;
        }
        self.costs.asked = ids.clone();
        self.costs.asked_at = Some(Instant::now());
        // GetObjectCost per region
        let mut by_region: HashMap<RegionHandle, Vec<Uuid>> = HashMap::new();
        for (id, h, _) in &roots {
            by_region.entry(*h).or_default().push(*id);
        }
        for (h, list) in &by_region {
            let body = llsd_map! { "object_ids" => Llsd::Array(list.iter().map(|u| Llsd::from(*u)).collect()) };
            self.cap(*h, "GetObjectCost", false, body, CapPurpose::ObjectCost(list.clone()));
        }
        // the weights come from the region of the first root
        let (_, first_region, first_pos) = roots[0];
        let body = llsd_map! { "selected_roots" => Llsd::Array(ids.iter().map(|u| Llsd::from(*u)).collect()) };
        self.cap(first_region, "ResourceCostSelected", false, body, CapPurpose::SelectionCost);
        // physics values of every selected prim (Features tab)
        let prims: Vec<(Uuid, RegionHandle)> = self
            .sel_prims(world)
            .into_iter()
            .filter_map(|i| world.objects.get(i).map(|o| (o.full_id, o.key.region)))
            .collect();
        let mut by_region: HashMap<RegionHandle, Vec<Uuid>> = HashMap::new();
        for (id, h) in prims {
            by_region.entry(h).or_default().push(id);
        }
        for (h, list) in by_region {
            let body = llsd_map! { "object_ids" => Llsd::Array(list.iter().map(|u| Llsd::from(*u)).collect()) };
            self.cap(h, "GetObjectPhysicsData", false, body, CapPurpose::PhysicsData(list));
        }
        // the parcel under the first object, unless the selection spans
        // several regions (LLCrossParcelFunctor, coarse version)
        if by_region_count(&roots) == 1
            && let Some(off) = world.region_offset(first_region)
        {
            let l = first_pos - off;
            self.costs.parcel_seq = BUILD_PARCEL_SEQ + (self.costs.parcel_seq + 1 - BUILD_PARCEL_SEQ).rem_euclid(1000);
            // LLViewerParcelMgr::selectParcelAt: one 4 m cell, snapped to the parcel
            let (x, y) = ((l.x / 4.0).floor() * 4.0, (l.y / 4.0).floor() * 4.0);
            self.send(BuildCmd::ParcelRequest {
                handle: first_region,
                sequence: self.costs.parcel_seq,
                snap: true,
                west: x + 1.0,
                south: y + 1.0,
                east: x + 3.0,
                north: y + 3.0,
            });
        }
    }

    /// A capability answer (NetEvent::CapReply).
    pub fn on_cap_reply(&mut self, world: &mut World, tag: u64, result: Result<Llsd, String>) {
        let Some(purpose) = self.caps.remove(&tag) else { return };
        match purpose {
            CapPurpose::ObjectCost(ids) => {
                let Ok(v) = result else { return };
                for id in ids {
                    let c = &v[id.to_string().as_str()];
                    if c.is_undef() {
                        continue;
                    }
                    self.costs.objects.insert(
                        id,
                        ObjectCost {
                            linked_set: c["linked_set_resource_cost"].as_f32(),
                        },
                    );
                }
                if self.costs.objects.len() > 4096 {
                    self.costs.objects.clear();
                }
            }
            CapPurpose::SelectionCost => {
                self.costs.weights = Some(result.map(|v| {
                    let s = &v["selected"];
                    Weights {
                        download: s["streaming"].as_f32(),
                        physics: s["physics"].as_f32(),
                        server: s["simulation"].as_f32(),
                    }
                }));
            }
            CapPurpose::PhysicsData(ids) => {
                let Ok(v) = result else { return };
                for id in ids {
                    let d = &v[id.to_string().as_str()];
                    if d.is_undef() {
                        continue;
                    }
                    if let Some(o) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i)) {
                        self.physics.insert(
                            o.key.local_id,
                            PhysicsParams {
                                shape_type: d["PhysicsShapeType"].as_i32() as u8,
                                density: d["Density"].as_f32(),
                                friction: d["Friction"].as_f32(),
                                restitution: d["Restitution"].as_f32(),
                                gravity_multiplier: d["GravityMultiplier"].as_f32(),
                            },
                        );
                    }
                }
            }
            other => self.on_media_reply(world, other, result),
        }
    }

    /// ParcelProperties answering our request (NetEvent::SelectedParcel).
    pub fn on_selected_parcel(&mut self, handle: RegionHandle, sequence: i32, parcel: Arc<ParcelInfo>) {
        if sequence == self.costs.parcel_seq {
            self.costs.parcel = Some((handle, parcel));
        } else {
            self.land.on_selected_parcel(handle, sequence, parcel);
        }
    }

    /// Forget the costs of objects that changed (link, unlink, shape).
    pub fn costs_stale(&mut self) {
        self.costs.asked_at = None;
    }
}

/// min(sim-wide max, region capacity) and that minus the sim-wide count.
fn capacity_of(p: &ParcelInfo, max_tasks: u32) -> (i32, i32) {
    let mut total = p.sim_max_prims;
    if max_tasks > 0 {
        total = total.min(max_tasks as i32);
    }
    (total, total - p.sim_total_prims)
}

fn by_region_count(roots: &[(Uuid, RegionHandle, glam::Vec3)]) -> usize {
    let mut v: Vec<RegionHandle> = roots.iter().map(|r| r.1).collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_min_of_parcel_and_region() {
        let p = ParcelInfo {
            sim_max_prims: 15000,
            sim_total_prims: 49,
            ..Default::default()
        };
        assert_eq!(capacity_of(&p, 0), (15000, 14951));
        assert_eq!(capacity_of(&p, 7031), (7031, 6982));
    }
}
