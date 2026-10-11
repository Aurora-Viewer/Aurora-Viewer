//! Per-object flexies: world-space physics and swept geometry on rayon;
//! vertex allocations and face records stay in place between frames.
//! Source: LLVolumeImplFlexible (indra/newview/llflexibleobject.cpp, LGPL 2.1).

use super::*;
use aurora_prim::flexible::Flexible;
use aurora_prim::params::{LL_PCODE_PATH_FLEXIBLE, LL_PCODE_VOLUME};

pub(super) struct State {
    id: Uuid,
    pub(super) sim: Flexible,
    last: Instant,
}

pub(super) fn enabled(o: &Object) -> bool {
    o.pcode == LL_PCODE_VOLUME
        && o.volume.path.curve_type == LL_PCODE_PATH_FLEXIBLE
        && o.extra.flexible.is_some()
        && !o.volume.is_sculpt()
        && !o.volume.is_mesh()
}

impl Scene {
    pub(super) fn sync_flexibles(&mut self, renderer: &mut Renderer, world: &World, synced: &[usize], now: Instant, view: &CullView) {
        self.flexibles
            .retain(|&idx, s| world.objects.get(idx).is_some_and(|o| o.full_id == s.id && enabled(o)));
        for &idx in synced {
            let Some(o) = world.objects.get(idx).filter(|o| enabled(o)) else {
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
            });
        }
        let updates: Vec<_> = self
            .flexibles
            .par_iter_mut()
            .filter_map(|(&idx, state)| {
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
                    state.sim = Flexible::new(p.softness, scale.z, pos, rot);
                }
                let dt = now.saturating_duration_since(state.last).as_secs_f32();
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
                    &state.sim.path(pos, rot),
                    scale,
                );
                let (faces, min, max) = jobs::volume_to_faces(&mesh);
                let prepared = jobs::PreparedGeom::new(faces, min, max, true, None);
                Some((idx, key, prepared, pos, rot, scale))
            })
            .collect();
        for (idx, key, prepared, pos, rot, scale) in updates {
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
            let model = Mat4::from_scale_rotation_translation(scale, rot, pos);
            let g = &mut self.gpu[idx];
            g.center = model.transform_point3((geom.min + geom.max) * 0.5);
            g.radius = (((geom.max - geom.min) * scale * 0.5).length()).max(0.05);
            self.pick.changed(idx);
        }
    }
}
