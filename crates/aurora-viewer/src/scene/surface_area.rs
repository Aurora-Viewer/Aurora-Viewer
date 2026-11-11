//! Large surface protection: prims and sculpts whose surface is very large
//! stop being drawn past a total each frame, against the content made to
//! overload video cards (Preferences › Protections).
//!
//! Port of the NaCl graphics crasher protection of the Legacy Viewer:
//! LLVOVolume::setVolume measuring every volume (`calcSurfaceArea`),
//! LLVolumeGeometryManager::rebuildGeom skipping the objects past the
//! budget, and LLPipeline::postSort resetting it (indra/newview/llvovolume.cpp,
//! pipeline.cpp, indra/llmath/llvolume.cpp), originally LGPL 2.1. Settings
//! RenderVolumeSAProtection (off), RenderVolumeSAThreshold (75),
//! RenderSculptSAThreshold (150) and RenderVolumeSAFrameMax (5000).
//!
//! The area is the swept grid's at unit scale, as the Legacy Viewer measures
//! it (object scale not applied): an ordinary prim measures a few square
//! metres, a sculpt folded on itself hundreds. A mesh has no grid and is
//! never hidden. As there, our own objects and attachments are not exempt.
//!
//! Deliberate difference: the Legacy Viewer hides the objects past the
//! budget in the order its octree happens to rebuild them, so which ones
//! vanish varies. Aurora adds the areas from the smallest, so that the
//! largest surfaces are the ones hidden, and the same ones every frame.

use super::{GeomKey, Scene};
use crate::world::World;
use std::collections::{HashMap, HashSet};

/// The settings of the protection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Protection {
    pub on: bool,
    /// RenderVolumeSAThreshold: from this area a prim counts.
    pub volume_threshold: f32,
    /// RenderSculptSAThreshold: the same for sculpts.
    pub sculpt_threshold: f32,
    /// RenderVolumeSAFrameMax: the total drawn; past it, what remains is
    /// hidden.
    pub frame_max: f32,
}

impl Default for Protection {
    fn default() -> Self {
        Protection {
            on: false,
            volume_threshold: 75.0,
            sculpt_threshold: 150.0,
            frame_max: 5000.0,
        }
    }
}

/// What an object's surface is measured as: its grid's area and whether
/// it is a sculpt (the sculpts' threshold) or a prim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    pub area: f32,
    pub sculpt: bool,
}

impl Measure {
    /// The measure of geometry `key` of `area`; None for what has no grid
    /// of its own (meshes, plants, avatars, flexible prims).
    fn of(key: GeomKey, area: f32) -> Option<Measure> {
        match key {
            GeomKey::Prim { .. } => Some(Measure { area, sculpt: false }),
            GeomKey::Sculpt { .. } | GeomKey::SculptPlaceholder { .. } => Some(Measure { area, sculpt: true }),
            _ => None,
        }
    }
}

impl Protection {
    /// Whether a surface of `m` counts (past the threshold of its kind).
    fn counts(&self, m: Measure) -> bool {
        m.area > if m.sculpt { self.sculpt_threshold } else { self.volume_threshold }
    }
}

/// The objects hidden: those of `measured` past their threshold, added from
/// the smallest; every object whose addition passes the total is hidden, as
/// the Legacy budget hides the object that passes it.
pub fn hidden(measured: &HashMap<usize, Measure>, p: Protection) -> HashSet<usize> {
    let mut out = HashSet::new();
    if !p.on {
        return out;
    }
    let mut order: Vec<(usize, f32)> = measured.iter().filter(|(_, m)| p.counts(**m)).map(|(i, m)| (*i, m.area)).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut total = 0.0f32;
    for (idx, area) in order {
        total += area;
        if total > p.frame_max {
            out.insert(idx);
        }
    }
    out
}

impl Scene {
    /// The measure of geometry `key` once it was built.
    fn measure_of(&self, key: GeomKey) -> Option<Measure> {
        Measure::of(key, *self.grid_area.get(&key)?)
    }

    /// Notes the measure of object `idx`, which wants geometry `wanted`
    /// (each full sync, whatever the settings). True when it is hidden.
    pub(super) fn count_surface(&mut self, idx: usize, wanted: Option<GeomKey>) -> bool {
        match wanted.and_then(|k| self.measure_of(k)) {
            Some(m) => {
                self.surface_measured.insert(idx, m);
            }
            None => {
                self.surface_measured.remove(&idx);
            }
        }
        self.surface_hidden.contains(&idx)
    }

    /// Each frame: the objects hidden by the protection, and those whose
    /// state changed synced again.
    pub(super) fn update_surface_protection(&mut self, world: &mut World) {
        // gone objects leave the measures; an object drawn before any full
        // sync measured it (geometry built after) is measured from what it
        // wants or draws
        self.surface_measured.retain(|i, _| world.objects.get(*i).is_some());
        // (only when geometry was measured since the last look: meshes,
        // avatars and plants never are, and are not looked at every frame)
        if self.grid_area.len() != self.surface_scanned {
            self.surface_scanned = self.grid_area.len();
            let unmeasured: Vec<(usize, Measure)> = self
                .gpu
                .iter()
                .enumerate()
                .filter(|(i, _)| !self.surface_measured.contains_key(i))
                .filter_map(|(i, g)| Some((i, self.measure_of(g.wanted_geom.or(g.geom)?)?)))
                .collect();
            self.surface_measured.extend(unmeasured);
        }
        let now = hidden(&self.surface_measured, self.surface_protection);
        if now == self.surface_hidden {
            return;
        }
        for &idx in now.symmetric_difference(&self.surface_hidden) {
            if let Some(object) = world.objects.get_mut_untracked(idx) {
                object.render.needs_records = true;
                self.sets.dirty.insert(idx);
            }
        }
        if now.len() != self.surface_hidden.len() {
            log::info!("large surface protection: {} object(s) hidden", now.len());
        }
        self.surface_hidden = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(frame_max: f32) -> Protection {
        Protection {
            on: true,
            frame_max,
            ..Protection::default()
        }
    }

    fn measured(areas: &[(usize, f32)]) -> HashMap<usize, Measure> {
        areas.iter().map(|(i, a)| (*i, Measure { area: *a, sculpt: true })).collect()
    }

    #[test]
    fn the_largest_are_hidden_past_the_total() {
        let m = measured(&[(1, 300.0), (2, 3000.0), (3, 200.0), (4, 2500.0)]);
        // 200 + 300 + 2500 = 3000, + 3000 = 6000 > 5000: only the largest
        assert_eq!(hidden(&m, on(5000.0)), HashSet::from([2]));
        assert_eq!(hidden(&m, on(2000.0)), HashSet::from([2, 4]));
        assert!(hidden(&m, on(10_000.0)).is_empty());
        assert_eq!(hidden(&m, on(0.0)).len(), 4);
        // off: nothing, whatever the measures
        assert!(
            hidden(
                &m,
                Protection {
                    frame_max: 0.0,
                    ..Protection::default()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn only_what_passes_the_threshold_of_its_kind_counts() {
        let mut m = measured(&[(1, 149.0), (2, 151.0)]);
        m.insert(3, Measure { area: 76.0, sculpt: false });
        m.insert(4, Measure { area: 74.0, sculpt: false });
        // 151 (sculpt) and 76 (prim) count; 76 + 151 > 100 hides the larger
        assert_eq!(hidden(&m, on(100.0)), HashSet::from([2]));
        assert_eq!(hidden(&m, on(0.0)), HashSet::from([2, 3]));
    }

    #[test]
    fn equal_areas_hide_the_same_objects_every_time() {
        let m: HashMap<usize, Measure> = (0..10).map(|i| (i, Measure { area: 600.0, sculpt: true })).collect();
        let first = hidden(&m, on(5000.0));
        assert_eq!(first, (8..10).collect::<HashSet<usize>>());
        for _ in 0..5 {
            assert_eq!(hidden(&m, on(5000.0)), first);
        }
    }

    #[test]
    fn prims_and_sculpts_are_measured_and_meshes_not() {
        let prim = Measure::of(GeomKey::Prim { hash: 1, lod: 3 }, 5.0);
        assert_eq!(prim, Some(Measure { area: 5.0, sculpt: false }));
        assert!(Measure::of(GeomKey::Sculpt { hash: 1, lod: 3 }, 5.0).is_some_and(|m| m.sculpt));
        assert!(Measure::of(GeomKey::SculptPlaceholder { hash: 1, lod: 3 }, 5.0).is_some_and(|m| m.sculpt));
        let mesh = GeomKey::Mesh {
            id: uuid::Uuid::nil(),
            lod: 3,
        };
        assert_eq!(Measure::of(mesh, 5.0), None);
    }
}
