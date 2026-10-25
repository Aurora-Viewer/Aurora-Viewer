//! Parcel ownership and property lines from LLViewerParcelOverlay
//! (indra/newview/llviewerparceloverlay.cpp, originally LGPL 2.1).
//! A tiny nearest-cell texture reuses the terrain's existing GPU geometry.
//! No terrain tessellation, per-cell draw calls or frame-loop GPU readback.

use super::{CullView, Scene, te_invisible};
use crate::frame_par::{self, Grain};
use crate::theme::Palette;
use crate::world::World;
use crate::world::beacons::{Beacons, Kind, Overlays};
use aurora_render::{DrawRecord, MipLevel, Renderer};
use glam::Mat4;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

struct ParcelTexture {
    data: Vec<u8>,
    size: (u32, u32),
    colors: [[u8; 4]; 6],
    slot: u32,
    records: [u32; 2],
}

#[derive(Default)]
pub(super) struct LandOverlays {
    textures: HashMap<u64, ParcelTexture>,
    pub enabled: [bool; 2],
}

fn colors(p: &Palette) -> [[u8; 4]; 6] {
    [p.muted, p.danger, p.teal, p.success, p.amber, p.violet_light].map(|c| c.to_srgba_unmultiplied())
}

/// RGB is the ownership legend; alpha retains the original ParcelOverlay
/// byte (west/south border bits included), without a lossy colour encoding.
fn texels(data: &[u8], colors: &[[u8; 4]; 6]) -> Vec<u8> {
    data.iter()
        .flat_map(|&byte| {
            let c = colors[usize::from(byte & 7).min(5)];
            [c[0], c[1], c[2], byte]
        })
        .collect()
}

impl LandOverlays {
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for (_, t) in self.textures.drain() {
            renderer.free_texture(t.slot);
            for record in t.records {
                renderer.records.free(record);
            }
        }
        self.enabled = [false; 2];
    }

    pub fn records(&self, region: u64) -> impl Iterator<Item = u32> + '_ {
        self.textures
            .get(&region)
            .into_iter()
            .flat_map(|t| t.records.into_iter().enumerate())
            .filter(|(i, _)| self.enabled[*i])
            .map(|(_, r)| r)
    }
}

impl Scene {
    /// Collect only while a source filter is on. The object walk shares the
    /// frame's worker pool, and only changed invisible faces are rebuilt.
    pub fn update_beacons(&mut self, world: &mut World, s: &Beacons, view: &CullView) {
        let now = Instant::now();
        let mut highlights = HashSet::new();
        self.beacon_markers.clear();
        if s.objects_enabled() {
            let groups = frame_par::fold(
                &world.objects.slots,
                Grain { chunk: 128, per_helper: 8 },
                Vec::new,
                |mut out, slot| {
                    if let Some(o) = slot {
                        let kinds: Vec<_> = s.kinds(o, s.particles && self.particles.beacon_source(&o.full_id)).collect();
                        if !kinds.is_empty()
                            && let Some(idx) = world.objects.index_of(&o.key)
                            && world.parcel_object_visible(idx)
                            && let Some((pos, _, false)) = Scene::object_transform(world, idx, now, 0)
                        {
                            let radius = o.scale.length() * 0.5;
                            let shown = pos.distance(view.eye) - radius <= self.draw_distance && view.sphere_visible(pos, radius);
                            out.extend(kinds.into_iter().map(|kind| (idx, pos, kind, shown)));
                        }
                    }
                    out
                },
            );
            for (idx, pos, kind, shown) in groups.into_iter().flatten() {
                if s.highlights {
                    highlights.insert(idx);
                }
                if s.lines && shown {
                    self.beacon_markers.push((pos, kind));
                }
            }
            if s.sound {
                for (idx, pos) in self.sounds.beacon_sources(world, now) {
                    if idx.is_some_and(|idx| !world.parcel_object_visible(idx)) {
                        continue;
                    }
                    if s.highlights
                        && let Some(idx) = idx
                    {
                        highlights.insert(idx);
                    }
                    if s.lines {
                        self.beacon_markers.push((pos, Kind::Sound));
                    }
                }
            }
        }
        if self.beacon_highlights != highlights {
            for &idx in self.beacon_highlights.symmetric_difference(&highlights) {
                if let Some(o) = world.objects.get_mut(idx)
                    && (0..aurora_prim::te::MAX_TES).any(|face| te_invisible(o, face))
                {
                    o.material_dirty = true;
                    o.render.needs_records = true;
                    self.sets.dirty.insert(idx);
                }
            }
            self.beacon_highlights = highlights;
        }
    }

    pub fn update_land_overlays(&mut self, renderer: &mut Renderer, world: &World, s: Overlays, p: &Palette) {
        let cache = &mut self.land_overlays;
        cache.enabled = [s.land_owners, s.property_lines];
        if !cache.enabled.contains(&true) {
            if !cache.textures.is_empty() {
                cache.clear(renderer);
            }
            return;
        }
        let stale: Vec<_> = cache
            .textures
            .keys()
            .filter(|h| !world.map.overlays.contains_key(h) || !world.regions.contains_key(h))
            .copied()
            .collect();
        for h in stale {
            if let Some(t) = cache.textures.remove(&h) {
                renderer.free_texture(t.slot);
                for r in t.records {
                    renderer.records.free(r);
                }
            }
        }
        let colors = colors(p);
        for (&h, &(nx, ny, ref data)) in &world.map.overlays {
            let (Some(region), Some(offset)) = (world.regions.get(&h), world.region_offset(h)) else {
                continue;
            };
            if nx == 0 || ny == 0 || data.len() != nx as usize * ny as usize {
                continue;
            }
            let changed = cache
                .textures
                .get(&h)
                .is_none_or(|t| t.data != *data || t.size != (nx, ny) || t.colors != colors);
            if changed {
                let rgba = texels(data, &colors);
                let Some(slot) = renderer.create_texture(&[MipLevel {
                    width: nx,
                    height: ny,
                    data: &rgba,
                }]) else {
                    continue;
                };
                let records = if let Some(old) = cache.textures.remove(&h) {
                    renderer.free_texture(old.slot);
                    old.records
                } else {
                    [
                        renderer.records.alloc(DrawRecord::default()),
                        renderer.records.alloc(DrawRecord::default()),
                    ]
                };
                cache.textures.insert(
                    h,
                    ParcelTexture {
                        data: data.clone(),
                        size: (nx, ny),
                        colors,
                        slot,
                        records,
                    },
                );
            }
            if let Some(t) = cache.textures.get(&h) {
                for (mode, &record) in t.records.iter().enumerate() {
                    renderer.records.set(
                        record,
                        DrawRecord {
                            model: Mat4::from_translation(offset).to_cols_array_2d(),
                            uv_st: [nx as f32, ny as f32, 0.0, 0.0],
                            tex: [t.slot, 0, 0, 0],
                            params: [mode as f32, region.info.as_ref().map_or(0.0, |i| i.water_height), 0.0, 0.0],
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;

    #[test]
    fn beacon_highlights_rebuild_only_changed_invisible_faces_and_keep_selection() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for e in crate::demo::events() {
            world.apply(e);
        }
        world.apply(crate::demo::beacons::events());
        let invisible = world.objects.iter().find(|(_, o)| o.key.local_id == 68206).unwrap().0;
        let plain = world.objects.iter().find(|(_, o)| o.key.local_id == 68200).unwrap().0;
        let mut scene = Scene::new(Default::default(), lib);
        let (eye, target) = crate::demo::beacons::view("1");
        let vp = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.1, 1.0, 0.1)
            * glam::camera::rh::view::look_at_mat4(eye, target, glam::Vec3::Z);
        let view = CullView::new(vp, eye, 1000.0, 1.1);
        let selection = HashMap::from([(invisible, 1)]);
        scene.set_face_selection(&mut world, selection.clone());
        let clean = |scene: &mut Scene, world: &mut World| {
            scene.sets.dirty.take();
            for o in world.objects.slots.iter_mut().flatten() {
                o.material_dirty = false;
                o.render.needs_records = false;
            }
        };
        let s = Beacons {
            scripted: true,
            lines: true,
            ..Default::default()
        };
        clean(&mut scene, &mut world);
        scene.update_beacons(&mut world, &s, &view);
        assert!(scene.beacon_highlights.contains(&invisible));
        assert!(world.objects.get(invisible).unwrap().material_dirty);
        assert!(!world.objects.get(plain).unwrap().material_dirty);
        assert_eq!(scene.selection_faces, selection);
        clean(&mut scene, &mut world);
        scene.update_beacons(&mut world, &s, &view);
        assert!(!world.objects.get(invisible).unwrap().material_dirty);
        let before = scene.beacon_markers.clone();
        world.objects.get_mut(plain).unwrap().position.x += 5.0;
        scene.update_beacons(&mut world, &s, &view);
        assert_ne!(scene.beacon_markers, before);
        assert!(!world.objects.get(invisible).unwrap().material_dirty);
        let off = Beacons { highlights: false, ..s };
        scene.update_beacons(&mut world, &off, &view);
        assert!(scene.beacon_highlights.is_empty());
        assert!(!scene.beacon_markers.is_empty(), "lines remain independent");
        assert!(world.objects.get(invisible).unwrap().material_dirty);
        assert_eq!(scene.selection_faces, selection);
        scene.update_beacons(&mut world, &Beacons::default(), &view);
        assert!(scene.beacon_markers.is_empty());
    }

    #[test]
    fn parcel_texels_keep_all_boundary_and_ownership_flags() {
        let p = crate::theme::Theme::default().palette();
        let colors = colors(&p);
        let input: Vec<_> = (0..=255).collect();
        let out = texels(&input, &colors);
        for (byte, rgba) in input.iter().zip(out.as_chunks::<4>().0) {
            assert_eq!(rgba[3], *byte);
            assert_eq!(rgba[..3], colors[usize::from(byte & 7).min(5)][..3]);
        }
    }
}
