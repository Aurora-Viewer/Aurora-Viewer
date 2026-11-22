//! Explorer uses a material's base-color map and linear color factor as a small swatch.
//! The full material editor keeps its lit PBR model. Browsing never allocates a scene or
//! loads normal, metallic/roughness or emissive maps for every entry in a large folder.
use crate::scene::meshes::{MaterialStatus, MaterialStreamer};
use aurora_assets::{AlphaMode, PbrMaterial};
use egui::Color32;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const MAX_VISIBLE: usize = 2048;
const MAX_FETCHING: usize = 4;

#[derive(Copy, Clone, PartialEq, Eq)]
pub(super) struct Swatch {
    pub map: Option<Uuid>,
    pub tint: Color32,
}

impl From<&PbrMaterial> for Swatch {
    fn from(material: &PbrMaterial) -> Self {
        let [r, g, b, alpha] = material.base_color_factor;
        let a = if material.alpha_mode == AlphaMode::Opaque { 1.0 } else { alpha };
        Self {
            map: material.base_color_texture.filter(|id| !id.is_nil()),
            tint: egui::Rgba::from_rgba_unmultiplied(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a.clamp(0.0, 1.0)).into(),
        }
    }
}

/// Shared across the main window and detached Explorer panes. Only visible icon tiles
/// submit requests. Texture pixels use the existing eight-stream, size-limited grid.
#[derive(Default)]
pub(crate) struct MaterialTiles {
    visible: Vec<Uuid>,
    unique: HashSet<Uuid>,
    ready: HashMap<Uuid, Swatch>,
    unavailable: HashSet<Uuid>,
    pending: HashSet<Uuid>,
    generation: u64,
    account: Uuid,
}

impl MaterialTiles {
    pub(super) fn tile(&mut self, id: Uuid) -> Option<Swatch> {
        if !self.unique.contains(&id) && self.visible.len() < MAX_VISIBLE {
            self.unique.insert(id);
            self.visible.push(id);
        }
        self.ready.get(&id).copied()
    }

    pub(crate) fn update(&mut self, account: Uuid, materials: &mut MaterialStreamer) {
        if self.generation != materials.generation {
            // Already-loaded assets do not consume fetch slots. Recheck them, and
            // local materials that may have arrived/replaced the same UUID, without
            // launching a fetch when an unrelated scene material changes.
            self.ready.retain(|id, swatch| {
                if let Some(material) = materials.peek(id) {
                    *swatch = Swatch::from(material.as_ref());
                    true
                } else {
                    false
                }
            });
            self.unavailable.retain(|id| {
                if let Some(material) = materials.peek(id) {
                    self.ready.insert(*id, Swatch::from(material.as_ref()));
                    false
                } else {
                    true
                }
            });
            self.generation = materials.generation;
        }
        self.refresh(account, |id| materials.resolve(&id));
    }

    fn refresh(&mut self, account: Uuid, mut resolve: impl FnMut(Uuid) -> MaterialStatus) {
        if self.account != account {
            self.ready.clear();
            self.unavailable.clear();
            self.pending.clear();
            self.account = account;
        }
        self.ready.retain(|id, _| self.unique.contains(id));
        self.unavailable.retain(|id| self.unique.contains(id));
        // The scene streamer owns submitted requests; complete them even after a fast
        // scroll, so scrolling through 1000 materials cannot launch 1000 downloads.
        self.pending.retain(|id| match resolve(*id) {
            MaterialStatus::Pending => true,
            MaterialStatus::Loaded(material) => {
                if self.unique.contains(id) {
                    self.ready.insert(*id, Swatch::from(material.as_ref()));
                }
                false
            }
            MaterialStatus::Missing => {
                if self.unique.contains(id) {
                    self.unavailable.insert(*id);
                }
                false
            }
        });
        let mut new = 0;
        for id in self.visible.drain(..) {
            if new >= MAX_FETCHING || self.pending.len() >= MAX_FETCHING {
                break;
            }
            if self.ready.contains_key(&id) || self.unavailable.contains(&id) || self.pending.contains(&id) {
                continue;
            }
            new += 1;
            match resolve(id) {
                MaterialStatus::Loaded(material) => {
                    self.ready.insert(id, Swatch::from(material.as_ref()));
                }
                MaterialStatus::Pending => {
                    self.pending.insert(id);
                }
                MaterialStatus::Missing => {
                    self.unavailable.insert(id);
                }
            }
        }
        self.unique.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn organized_material_tiles_bound_pending_reads_across_scroll_and_windows() {
        let account = Uuid::from_u128(10000);
        let ids: Vec<_> = (1..=40).map(Uuid::from_u128).collect();
        let mut tiles = MaterialTiles::default();
        for id in &ids {
            tiles.tile(*id);
            tiles.tile(*id);
        }
        let mut seen = Vec::new();
        tiles.refresh(account, |id| {
            seen.push(id);
            MaterialStatus::Pending
        });
        assert_eq!(seen, ids[..4]);
        for id in &ids[20..] {
            tiles.tile(*id);
        }
        tiles.refresh(account, |id| {
            assert!(ids[..4].contains(&id));
            MaterialStatus::Pending
        });
        for id in &ids[20..] {
            tiles.tile(*id);
        }
        let value = Arc::new(PbrMaterial::default());
        let mut fresh = Vec::new();
        tiles.refresh(account, |id| {
            if ids[..4].contains(&id) {
                MaterialStatus::Missing
            } else {
                fresh.push(id);
                MaterialStatus::Loaded(value.clone())
            }
        });
        assert_eq!(fresh, ids[20..24]);
        assert_eq!(tiles.ready.len(), 4);
        assert!(tiles.pending.is_empty());
        tiles.refresh(account, |_| panic!("no offscreen material fetches"));
        assert!(tiles.ready.is_empty());
    }

    #[test]
    fn organized_material_swatch_uses_base_map_tint_alpha_and_color_space() {
        let map = Uuid::from_u128(17);
        let mut material = PbrMaterial {
            base_color_texture: Some(map),
            normal_texture: Some(Uuid::from_u128(18)),
            base_color_factor: [0.5, 0.0, 1.0, 0.25],
            ..Default::default()
        };
        let swatch = Swatch::from(&material);
        assert_eq!(swatch.map, Some(map));
        assert_eq!(swatch.tint.to_array(), [188, 0, 255, 255]);
        material.alpha_mode = AlphaMode::Blend;
        assert_eq!(Swatch::from(&material).tint.a(), 64);
        material.base_color_texture = None;
        assert!(Swatch::from(&material).map.is_none());
    }
}
