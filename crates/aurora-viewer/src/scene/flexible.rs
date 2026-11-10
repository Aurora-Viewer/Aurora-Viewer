//! Per-object flexies: world-space physics and swept geometry in parallel;
//! vertex allocations and face records stay in place between frames.
//! Source: LLVolumeImplFlexible (indra/newview/llflexibleobject.cpp, LGPL 2.1).
//!
//! The physics of every flexie within draw distance runs each frame, but
//! its mesh is swept again (and its vertices sent) only while a view can
//! show it: the camera, the water's reflection, the mirror drawn, or the
//! shadow it casts toward the sun or the moon. Measured on Agni on
//! 2026-11-04 (two busy regions at rest, ~640 flexies each): the sweeps
//! were 1.3 to 2.3 MB of vertices per frame, the largest share of the
//! render thread's `r_resources`. A held mesh keeps the last shape it was
//! seen with and takes the current one the frame it can be seen again,
//! from a simulation that never stopped. The Legacy Viewer skips the
//! flexies that are not visible too (LLVolumeImplFlexible::doIdleUpdate,
//! `isVisible`), simulation included, and spaces the updates of the small
//! ones out over up to 32 frames, which Aurora does not.
//! `AURORA_FLEXI_CULL=0` sweeps every one each frame, as before.

use super::*;
use crate::build::silhouette::{self, Topology};
use aurora_prim::flexible::Flexible;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// How far a held flexie's shadow is followed toward the light (m): past
/// it, a flexie outside every view is taken as casting nothing seen.
const SHADOW_REACH: f32 = 64.0;

/// AURORA_FLEXI_CULL: `0` sweeps every flexie's mesh each frame, as before;
/// `alt` does so one second in three.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CullMode {
    On,
    Off,
    Alternate,
}

fn cull_mode() -> CullMode {
    static MODE: std::sync::OnceLock<CullMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_FLEXI_CULL").as_deref() {
        Ok("0") => CullMode::Off,
        Ok("alt") => CullMode::Alternate,
        _ => CullMode::On,
    })
}

/// The old way this second, with `AURORA_FLEXI_CULL=alt`.
static OLD_NOW: AtomicBool = AtomicBool::new(false);

/// `AURORA_FLEXI_CULL=alt`: the way of second `n` (true: the old way).
/// None without `alt`.
pub fn alternate(n: u64) -> Option<bool> {
    (cull_mode() == CullMode::Alternate).then(|| {
        let old = n.is_multiple_of(3);
        OLD_NOW.store(old, Ordering::Relaxed);
        old
    })
}

fn flexi_cull() -> bool {
    match cull_mode() {
        CullMode::On => true,
        CullMode::Off => false,
        CullMode::Alternate => !OLD_NOW.load(Ordering::Relaxed),
    }
}

/// The views a flexie's mesh can be seen in this frame.
pub(super) struct FlexiViews<'a> {
    pub(super) view: &'a CullView,
    /// Directions a shadow goes from its caster: away from the sun and the
    /// moon (unit vectors).
    pub(super) shadows: [Vec3; 2],
    /// The plane (point, normal) of the mirror drawn.
    pub(super) mirror: Option<(Vec3, Vec3)>,
}

impl FlexiViews<'_> {
    /// A flexie within the sphere (`c`, `r`), over water at height `water`,
    /// can be seen: in the camera, mirrored by the water or the mirror, or
    /// by its shadow within [`SHADOW_REACH`].
    pub(super) fn show(&self, c: Vec3, r: f32, water: Option<f32>) -> bool {
        let v = self.view;
        if v.sphere_visible(c, r) {
            return true;
        }
        if water.is_some_and(|h| v.sphere_visible(Vec3::new(c.x, c.y, 2.0 * h - c.z), r)) {
            return true;
        }
        if self.mirror.is_some_and(|(p, n)| v.sphere_visible(c - 2.0 * (c - p).dot(n) * n, r)) {
            return true;
        }
        self.shadows.iter().any(|&d| capsule_visible(v, c, c + d * SHADOW_REACH, r))
    }
}

/// The capsule from `a` to `b` of radius `r` is not wholly outside any
/// plane of the view.
fn capsule_visible(v: &CullView, a: Vec3, b: Vec3, r: f32) -> bool {
    v.planes.iter().all(|p| {
        let n = p.truncate();
        (n.dot(a) + p.w).max(n.dot(b) + p.w) >= -r
    })
}

/// The sphere a flexie's mesh stays in, whatever its bend: from its centre
/// `pos`, the anchor end is half its length away and the free end at most
/// its length further, with its section around it.
fn flexi_bounds(scale: Vec3) -> f32 {
    1.5 * scale.z.abs() + scale.truncate().length()
}

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
            let p = o.extra.flexible.unwrap_or(crate::build::edits::FLEX_DEFAULT);
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
        let views = FlexiViews {
            view,
            shadows: [-world.sky_directions.0, -world.sky_directions.1],
            mirror: self.mirror_plane,
        };
        let cull = flexi_cull();
        let held = AtomicUsize::new(0);
        let mut states: Vec<(usize, &mut State)> = self.flexibles.iter_mut().map(|(&idx, state)| (idx, state)).collect();
        let updates = frame_par::filter_map_mut(&mut states, FLEXIBLE_GRAIN, |(idx, state)| {
            let idx = *idx;
            let o = world.objects.get(idx)?;
            let p = o.extra.flexible.unwrap_or(crate::build::edits::FLEX_DEFAULT);
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
            // (a selected one keeps its outline in step with its mesh)
            if cull && !hud && !self.selection_silhouettes.contains(&idx) {
                let water = region.and_then(|r| r.info.as_ref()).map(|i| i.water_height);
                if !views.show(pos, flexi_bounds(scale), water) {
                    held.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            }
            let path = state.sim.render_path(pos, rot);
            let mesh = aurora_prim::volume::generate_flexible_volume(
                &o.volume,
                aurora_prim::volume::DETAIL_SCALES[g.lod as usize % 4],
                &path,
                scale,
            );
            // Only selected flexies need silhouette topology. Build it
            // from the exact rendered mesh on the same thread.
            let outline = self.selection_silhouettes.contains(&idx).then(|| silhouette::from_mesh(&mesh));
            let (faces, min, max) = jobs::volume_to_faces(&mesh);
            let prepared = jobs::PreparedGeom::new(faces, min, max, true, None);
            Some((idx, key, prepared, outline, pos, rot, scale))
        });
        self.stats.flexies = updates.len();
        self.stats.flexies_held = held.into_inner();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn view(eye: Vec3, at: Vec3) -> CullView {
        let fov = 60f32.to_radians();
        let v = glam::camera::rh::view::look_at_mat4(eye, at, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 16.0 / 9.0, 0.1);
        CullView::new(proj * v, eye, 900.0, fov)
    }

    /// Shadows straight up: they reach no view of these tests.
    const NO_SHADOW: [Vec3; 2] = [Vec3::Z, Vec3::Z];

    #[test]
    fn a_flexie_in_front_shows_and_one_behind_is_held() {
        let v = view(Vec3::ZERO, Vec3::X);
        let views = FlexiViews {
            view: &v,
            shadows: NO_SHADOW,
            mirror: None,
        };
        assert!(views.show(Vec3::new(10.0, 0.0, 0.0), 0.5, None));
        assert!(!views.show(Vec3::new(-10.0, 0.0, 0.0), 0.5, None));
        // its bounds reach into the view
        assert!(views.show(Vec3::new(-0.5, 0.0, 0.0), 1.0, None));
    }

    #[test]
    fn a_flexie_behind_shows_when_its_shadow_falls_in_view() {
        // looking ahead and down from 10 m: the ground ahead is in view
        let v = view(Vec3::new(0.0, 0.0, 10.0), Vec3::new(20.0, 0.0, 0.0));
        let flexie = Vec3::new(-10.0, 0.0, 12.0);
        // sun ahead and high: the shadow goes further behind
        let ahead = FlexiViews {
            view: &v,
            shadows: [-Vec3::new(1.0, 0.0, 1.0).normalize(), Vec3::Z],
            mirror: None,
        };
        assert!(!ahead.show(flexie, 0.5, None));
        // sun behind the camera: the shadow is cast forward, into the view
        let behind = FlexiViews {
            view: &v,
            shadows: [-Vec3::new(-1.0, 0.0, 1.0).normalize(), Vec3::Z],
            mirror: None,
        };
        assert!(behind.show(flexie, 0.5, None));
        // past the reach it is held
        assert!(!behind.show(Vec3::new(-SHADOW_REACH - 10.0, 0.0, 12.0), 0.5, None));
    }

    #[test]
    fn a_flexie_shows_in_the_mirror_and_on_the_water() {
        // a mirror ahead, facing the camera, shows what is behind it
        let v = view(Vec3::ZERO, Vec3::X);
        let behind = Vec3::new(-5.0, 0.0, 0.0);
        let mirror = FlexiViews {
            view: &v,
            shadows: NO_SHADOW,
            mirror: Some((Vec3::new(10.0, 0.0, 0.0), -Vec3::X)),
        };
        assert!(mirror.show(behind, 0.5, None));
        // looking down at water 20 m high: what is above the view shows in it
        let v = view(Vec3::new(0.0, 0.0, 22.0), Vec3::new(10.0, 0.0, 10.0));
        let views = FlexiViews {
            view: &v,
            shadows: NO_SHADOW,
            mirror: None,
        };
        let above = Vec3::new(10.0, 0.0, 60.0);
        assert!(!views.show(above, 0.5, None));
        assert!(views.show(above, 0.5, Some(20.0)));
    }

    #[test]
    fn the_bounds_hold_a_flexie_bent_any_way() {
        // anchor end at -z/2; the free end at most the length from it
        let scale = Vec3::new(0.2, 0.3, 4.0);
        let far = (scale.z * 0.5 + scale.z) + scale.truncate().length() * 0.5;
        assert!(flexi_bounds(scale) >= far);
    }
}
