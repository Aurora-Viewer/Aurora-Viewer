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

impl Protection {
    /// The area from which geometry `key` counts; None for what has no
    /// grid of its own.
    fn threshold(&self, key: GeomKey) -> Option<f32> {
        match key {
            GeomKey::Prim { .. } => Some(self.volume_threshold),
            GeomKey::Sculpt { .. } | GeomKey::SculptPlaceholder { .. } => Some(self.sculpt_threshold),
            _ => None,
        }
    }
}

/// The objects hidden among `counted` (object, area): the areas are added
/// from the smallest, and every object whose addition passes `frame_max` is
/// hidden, as the Legacy budget hides the object that passes it.
pub fn hidden(counted: &HashMap<usize, f32>, frame_max: f32) -> HashSet<usize> {
    let mut order: Vec<(usize, f32)> = counted.iter().map(|(i, a)| (*i, *a)).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut total = 0.0f32;
    let mut out = HashSet::new();
    for (idx, area) in order {
        total += area;
        if total > frame_max {
            out.insert(idx);
        }
    }
    out
}

impl Scene {
    /// Notes object `idx`, which wants geometry `wanted`: counted when its
    /// area passes the threshold of its kind. True when it is hidden.
    pub(super) fn count_surface(&mut self, idx: usize, wanted: Option<GeomKey>) -> bool {
        let p = self.surface_protection;
        let area = wanted.and_then(|k| Some((p.threshold(k)?, *self.grid_area.get(&k)?)));
        match area {
            Some((threshold, area)) if p.on && area > threshold => {
                self.surface_counted.insert(idx, area);
            }
            _ => {
                self.surface_counted.remove(&idx);
            }
        }
        self.surface_hidden.contains(&idx)
    }

    /// Each frame: the objects hidden by the protection, and those whose
    /// state changed synced again.
    pub(super) fn update_surface_protection(&mut self, world: &mut World) {
        // new settings: every object is counted again, by its next sync
        if self.surface_settings_seen != Some(self.surface_protection) {
            self.surface_settings_seen = Some(self.surface_protection);
            self.surface_counted.clear();
            for (idx, object) in world.objects.slots.iter_mut().enumerate() {
                if let Some(object) = object {
                    object.render.needs_records = true;
                    self.sets.dirty.insert(idx);
                }
            }
        }
        // gone objects leave the count
        self.surface_counted.retain(|i, _| world.objects.get(*i).is_some());
        let now = if self.surface_protection.on {
            hidden(&self.surface_counted, self.surface_protection.frame_max)
        } else {
            HashSet::new()
        };
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

    #[test]
    fn the_largest_are_hidden_past_the_total() {
        let counted = HashMap::from([(1, 100.0), (2, 3000.0), (3, 200.0), (4, 2500.0)]);
        // 100 + 200 + 2500 = 2800, + 3000 = 5800 > 5000: only the largest
        assert_eq!(hidden(&counted, 5000.0), HashSet::from([2]));
        // 100 + 200 + 2500 > 2000: the two largest
        assert_eq!(hidden(&counted, 2000.0), HashSet::from([2, 4]));
        assert!(hidden(&counted, 10_000.0).is_empty());
        assert_eq!(hidden(&counted, 0.0).len(), 4);
        assert!(hidden(&HashMap::new(), 0.0).is_empty());
    }

    #[test]
    fn equal_areas_hide_the_same_objects_every_time() {
        let counted: HashMap<usize, f32> = (0..10).map(|i| (i, 600.0)).collect();
        let first = hidden(&counted, 5000.0);
        assert_eq!(first, (8..10).collect::<HashSet<usize>>());
        for _ in 0..5 {
            assert_eq!(hidden(&counted, 5000.0), first);
        }
    }

    #[test]
    fn each_kind_has_its_threshold_and_meshes_none() {
        let p = Protection::default();
        let prim = GeomKey::Prim { hash: 1, lod: 3 };
        assert_eq!(p.threshold(prim), Some(75.0));
        assert_eq!(p.threshold(GeomKey::Sculpt { hash: 1, lod: 3 }), Some(150.0));
        assert_eq!(p.threshold(GeomKey::SculptPlaceholder { hash: 1, lod: 3 }), Some(150.0));
        assert_eq!(
            p.threshold(GeomKey::Mesh {
                id: uuid::Uuid::nil(),
                lod: 3
            }),
            None
        );
    }
}
