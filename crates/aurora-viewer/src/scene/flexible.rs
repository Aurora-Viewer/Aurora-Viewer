//! Per-object flexies: world-space physics and swept geometry in parallel;
//! vertex allocations and face records stay in place between frames.
//! Source: LLVolumeImplFlexible (indra/newview/llflexibleobject.cpp, LGPL 2.1).

use super::*;
use crate::build::silhouette::{self, Topology};
use aurora_prim::flexible::Flexible;

pub(super) struct State {
    id: Uuid,
    pub(super) sim: Flexible,
    last: Instant,
    outline: Option<Topology>,
}

impl Scene {
    /// LLVolumeImplFlexible::getPivotPosition / getAnchorPosition: the
    /// negative-Z end, independent of the bent geometry's bounds.
    pub fn object_edit_pivot(world: &World, idx: usize, now: Instant) -> Option<Vec3> {
        let o = world.objects.get(idx)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        Some(if o.is_flexible() {
            pos - rot * Vec3::Z * (world.visual_scale(idx).z * 0.5)
        } else {
            pos
        })
    }

    pub(crate) fn flexible_outline(&self, idx: usize) -> Option<&Topology> {
        self.flexibles.get(&idx)?.outline.as_ref()
    }

    pub(super) fn sync_flexibles(&mut self, renderer: &mut Renderer, world: &World, synced: &[usize], now: Instant, view: &CullView) {
        self.flexibles
            .retain(|&idx, s| world.objects.get(idx).is_some_and(|o| o.full_id == s.id && o.is_flexible()));
        for &idx in synced {
            let Some(o) = world.objects.get(idx).filter(|o| o.is_flexible()) else {
                continue;
            };
            let Some(p) = o.extra.flexible else {
                continue;
            };
            let Some((pos, rot, _)) = self.xf_cache.get(idx).or_else(|| Self::object_transform(world, idx, now, 0)) else {
                continue;
            };
            self.flexibles.entry(idx).or_insert_with(|| State {
                id: o.full_id,
                sim: Flexible::new(p.softness, world.visual_scale(idx).z, pos, rot),
                last: now,
                outline: None,
            });
        }
        let mut states: Vec<(usize, &mut State)> = self.flexibles.iter_mut().map(|(&idx, state)| (idx, state)).collect();
        let updates = frame_par::filter_map_mut(&mut states, FLEXIBLE_GRAIN, |(idx, state)| {
            let idx = *idx;
            let o = world.objects.get(idx)?;
            let p = o.extra.flexible?;
            let g = self.gpu.get(idx)?;
            let key = g.geom.filter(|k| matches!(k, GeomKey::Flexible { .. }))?;
            let Some(GeomState::Ready(_)) = self.geoms.get(&key).map(|e| &e.state) else {
                return None;
            };
            if g.owner_avatar.is_some_and(|a| self.too_complex.contains(&a)) {
                return None;
            }
            let (pos, rot, hud) = self.xf_cache.get(idx).or_else(|| Self::object_transform(world, idx, now, 0))?;
            let scale = world.visual_scale(idx).max(Vec3::splat(0.001));
            if !hud && pos.distance(view.eye) > self.draw_distance + scale.length() {
                return None;
            }
            if !state.sim.matches(p.softness, scale.z) {
                state.sim.reconfigure(p.softness, scale.z, pos, rot);
            }
            let dt = now.saturating_duration_since(state.last).as_secs_f64();
            state.last = now;
            let region = world.regions.get(&o.key.region);
            let offset = world.region_offset(o.key.region).unwrap_or(Vec3::ZERO);
            state.sim.update(p, pos, rot, dt, |sample| {
                if hud {
                    Vec3::ZERO
                } else {
                    region.map_or(Vec3::ZERO, |r| r.wind.velocity(sample - offset, r.heightmap.size_x as f32))
                }
            });
            let mesh = aurora_prim::volume::generate_flexible_volume(
                &o.volume,
                aurora_prim::volume::DETAIL_SCALES[g.lod as usize % 4],
                &state.sim.render_path(pos, rot),
                scale,
            );
            // Only selected flexies need silhouette topology. Build it
            // from the exact rendered mesh on the same thread.
            let outline = self.selection_silhouettes.contains(&idx).then(|| silhouette::from_mesh(&mesh));
            let (faces, min, max) = jobs::volume_to_faces(&mesh);
            let prepared = jobs::PreparedGeom::new(faces, min, max, true, None);
            Some((idx, key, prepared, outline, pos, rot, scale))
        });
        for (idx, key, prepared, outline, pos, rot, scale) in updates {
            let Some(GeomEntry {
                state: GeomState::Ready(geom),
                ..
            }) = self.geoms.get_mut(&key)
            else {
                continue;
            };
            let Some(geom) = Arc::get_mut(geom) else {
                continue;
            };
            // A shape or LOD change binds another key; the topology of this
            // one stays fixed. Validate before any write, including empty faces.
            if geom.faces.len() != prepared.faces.len()
                || geom.faces.iter().zip(&prepared.faces).any(|(alloc, face)| match (alloc, face) {
                    (Some(a), Some(jobs::FaceUpload::Memory(f))) => a.vertex_count as usize != f.vertices.len(),
                    (None, None) => false,
                    _ => true,
                })
            {
                continue;
            }
            for (alloc, face) in geom.faces.iter().zip(prepared.faces) {
                if let (Some(alloc), Some(jobs::FaceUpload::Memory(face))) = (alloc, face) {
                    renderer.update_mesh_vertices(*alloc, &face.vertices);
                }
            }
            geom.min = prepared.min;
            geom.max = prepared.max;
            geom.pick_faces = prepared.pick_faces;
            if let Some(state) = self.flexibles.get_mut(&idx) {
                state.outline = outline;
            }
            let model = Mat4::from_scale_rotation_translation(scale, rot, pos);
            let g = &mut self.gpu[idx];
            g.center = model.transform_point3((geom.min + geom.max) * 0.5);
            g.radius = (((geom.max - geom.min) * scale * 0.5).length()).max(0.05);
            self.pick.changed(idx);
        }
    }
}
