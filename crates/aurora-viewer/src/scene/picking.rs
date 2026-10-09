//! LLVOVolume::lineSegmentIntersect: CLICK_ACTION_IGNORE is excluded outside
//! build mode (Firestorm llvovolume.cpp / llspatialpartition.cpp, LGPL 2.1).
//! CPU triangles share the lifetime of the uploaded geometry, including sculpts.
use super::*;
use crate::media::pick::{self, FaceTris};
use aurora_net::TouchSurface;

pub struct PickFace {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
}

/// Whether a ray can meet an object, from its world bounding sphere kept by
/// the sync: a few multiplications instead of the transform, matrix inverse
/// and box test of `face_hit`, for every object, every hovered frame. Rigged
/// meshes (sphere of the wearer, not of the bind pose) and objects without
/// bounds yet always pass.
pub(super) fn ray_may_hit(g: &super::ObjGpu, ray: (Vec3, Vec3)) -> bool {
    if g.rigged || g.radius <= 0.0 {
        return true;
    }
    let r = g.radius * 1.05 + 0.05;
    let to = g.center - ray.0;
    let along = to.dot(ray.1);
    if along < -r {
        return false;
    }
    to.length_squared() - along * along <= r * r
}

pub fn ray_box(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut near = 0.0f32;
    let mut far = f32::INFINITY;
    for i in 0..3 {
        if dir[i].abs() < 1e-8 {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
        } else {
            let a = (min[i] - origin[i]) / dir[i];
            let b = (max[i] - origin[i]) / dir[i];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if far < near {
                return None;
            }
        }
    }
    Some(near)
}

impl Scene {
    pub fn click_bounds(&self, world: &World, idx: usize) -> Option<(Vec3, Vec3)> {
        let o = world.objects.get(idx)?;
        let (pos, rot, _) = Self::object_transform(world, idx, Instant::now(), 0)?;
        let geom = self.gpu.get(idx).and_then(|g| g.geom).and_then(|key| self.geom_ready(&key));
        let (min, max) = geom.map_or((-Vec3::splat(0.5), Vec3::splat(0.5)), |g| (g.min, g.max));
        Some((pos + rot * ((min + max) * 0.5 * o.scale), (max - min) * o.scale))
    }

    fn face_hit(&self, world: &World, idx: usize, ray: (Vec3, Vec3), now: Instant, include_hidden: bool) -> Option<pick::FaceHit> {
        let o = world.objects.get(idx)?;
        let g = self.gpu.get(idx)?;
        let geom = self.geom_ready(&g.geom?)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        let model = Mat4::from_scale_rotation_translation(o.scale, rot, pos);
        let inv = model.inverse();
        ray_box(inv.transform_point3(ray.0), inv.transform_vector3(ray.1), geom.min, geom.max)?;
        let faces = geom.pick_faces.iter().enumerate().filter_map(|(face, f)| {
            if !include_hidden && o.te.as_ref().is_some_and(|te| te.face(face).color[3] < 0.004) {
                return None;
            }
            let f = f.as_ref()?;
            Some(FaceTris {
                face,
                positions: &f.positions,
                uvs: &f.uvs,
                indices: &f.indices,
            })
        });
        pick::ray_faces(faces, model, ray.0, ray.1)
    }

    pub fn touch_surface(&self, world: &World, idx: usize, ray: (Vec3, Vec3)) -> Option<TouchSurface> {
        let o = world.objects.get(idx)?;
        let hit = self.face_hit(world, idx, ray, Instant::now(), true)?;
        let tf = o.te.as_ref().map(|t| *t.face(hit.face)).unwrap_or_default();
        let uv = pick::te_transform(hit.uv, [tf.scale_s, tf.scale_t], [tf.offset_s, tf.offset_t], tf.rotation);
        Some(TouchSurface {
            uv: uv.extend(0.0),
            st: hit.uv.extend(0.0),
            face: hit.face as i32,
            position: ray.0 + ray.1 * hit.t - world.region_offset(o.key.region)?,
            normal: hit.normal,
            binormal: hit.binormal,
        })
    }

    /// Usually keep the GPU depth pick. Only trace past it when an ignored
    /// object's triangles actually intercept the ray; it stays visible.
    pub fn interaction_point(&self, world: &World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, build: bool, far: f32) -> Option<Vec3> {
        if build {
            return depth;
        }
        let ray = ray?;
        let now = Instant::now();
        let limit = depth.map_or(far, |p| (p - ray.0).dot(ray.1) + 0.1);
        let ignored = self.gpu.iter().enumerate().any(|(idx, g)| {
            !g.faces.is_empty()
                && !g.hud
                && world
                    .objects
                    .get(idx)
                    .is_some_and(|o| o.click_action == crate::interaction::code::IGNORE)
                && ray_may_hit(g, ray)
                && self.face_hit(world, idx, ray, now, true).is_some_and(|h| h.t <= limit)
        });
        if !ignored {
            return self.transparent_action_point(world, ray, depth, false, far);
        }
        let mut nearest = far;
        let mut found = false;
        for (idx, g) in self.gpu.iter().enumerate() {
            let Some(o) = world.objects.get(idx) else { continue };
            if g.faces.is_empty() || g.hud || o.click_action == crate::interaction::code::IGNORE {
                continue;
            }
            if !g.is_avatar && !ray_may_hit(g, ray) {
                continue;
            }
            let t = if g.is_avatar {
                ray_box(
                    ray.0,
                    ray.1,
                    g.center - Vec3::new(0.35, 0.35, 1.0),
                    g.center + Vec3::new(0.35, 0.35, 1.0),
                )
            } else {
                self.face_hit(world, idx, ray, now, false).map(|h| h.t)
            };
            if let Some(t) = t
                && t > 0.0
                && t < nearest
            {
                nearest = t;
                found = true;
            }
        }
        // Terrain can also be the target behind an ignored screen.
        let mut previous = 0.0;
        let mut t = 1.0;
        while t < nearest {
            let p = ray.0 + ray.1 * t;
            if world.ground_height(p).is_some_and(|z| p.z <= z) {
                let mut lo = previous;
                let mut hi = t;
                for _ in 0..10 {
                    let mid = (lo + hi) * 0.5;
                    let p = ray.0 + ray.1 * mid;
                    if world.ground_height(p).is_some_and(|z| p.z <= z) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                nearest = hi;
                found = true;
                break;
            }
            previous = t;
            t += 1.0;
        }
        found.then_some(ray.0 + ray.1 * nearest)
    }

    /// Firestorm also picks actionable transparent faces: they do not write
    /// the GPU depth buffer. Hidden touch panels take clicks but no hover.
    fn transparent_action_point(
        &self,
        world: &World,
        ray: (Vec3, Vec3),
        depth: Option<Vec3>,
        include_hidden: bool,
        far: f32,
    ) -> Option<Vec3> {
        let mut nearest = depth.map_or(far, |p| (p - ray.0).dot(ray.1));
        let mut found = false;
        let props = HashMap::new();
        for (idx, g) in self.gpu.iter().enumerate() {
            let Some(o) = world.objects.get(idx) else { continue };
            if g.hud || g.is_avatar || o.click_action == crate::interaction::code::IGNORE || !ray_may_hit(g, ray) {
                continue;
            }
            if !g.faces.iter().any(|f| {
                let alpha = self
                    .textures
                    .alpha_by_slot
                    .get(f.base_slot as usize)
                    .copied()
                    .unwrap_or(AlphaKind::Opaque);
                match classify(f, alpha) {
                    Pass::Blend => true,
                    Pass::Hidden => include_hidden && o.click_action != crate::interaction::code::DISABLED,
                    _ => false,
                }
            }) || crate::interaction::target(world, idx, &props).is_none()
            {
                continue;
            }
            if let Some(hit) = self.face_hit(world, idx, ray, Instant::now(), include_hidden)
                && hit.t < nearest
            {
                nearest = hit.t;
                found = true;
            }
        }
        if found { Some(ray.0 + ray.1 * nearest) } else { depth }
    }

    pub fn action_point(&self, world: &World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, build: bool, far: f32) -> Option<Vec3> {
        let point = self.interaction_point(world, ray, depth, build, far);
        if build {
            return point;
        }
        self.transparent_action_point(world, ray?, point, true, far)
    }

    pub fn interaction_at(&self, world: &World, point: Vec3, now: Instant, build: bool) -> Option<usize> {
        self.pick_at_filtered(world, point, now, !build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sphere_test_keeps_what_the_ray_can_reach() {
        let g = |center: Vec3, radius: f32| super::super::ObjGpu {
            center,
            radius,
            ..Default::default()
        };
        let ray = (Vec3::ZERO, Vec3::X);
        assert!(ray_may_hit(&g(Vec3::new(10.0, 0.5, 0.0), 1.0), ray));
        assert!(!ray_may_hit(&g(Vec3::new(10.0, 3.0, 0.0), 1.0), ray));
        // behind the eye, but the eye is inside it
        assert!(ray_may_hit(&g(Vec3::new(-0.5, 0.0, 0.0), 1.0), ray));
        assert!(!ray_may_hit(&g(Vec3::new(-5.0, 0.0, 0.0), 1.0), ray));
        // no bounds yet, rigged: never rejected
        assert!(ray_may_hit(&g(Vec3::new(0.0, 50.0, 0.0), 0.0), ray));
        let mut rigged = g(Vec3::new(0.0, 50.0, 0.0), 1.0);
        rigged.rigged = true;
        assert!(ray_may_hit(&rigged, ray));
    }

    #[test]
    fn box_rejects_misses_and_parallel_rays() {
        assert_eq!(ray_box(Vec3::new(-2.0, 0.0, 0.0), Vec3::X, -Vec3::ONE, Vec3::ONE), Some(1.0));
        assert!(ray_box(Vec3::new(-2.0, 3.0, 0.0), Vec3::X, -Vec3::ONE, Vec3::ONE).is_none());
        assert!(ray_box(Vec3::new(-2.0, 0.0, 0.0), -Vec3::X, -Vec3::ONE, Vec3::ONE).is_none());
    }
    #[test]
    fn ignore_picks_through_disabled_occludes_and_build_includes_both() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            world.apply(ev);
        }
        let front = world.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let back = world.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        for (idx, x) in [(front, 3.0), (back, 7.0)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.position = Vec3::new(x, 0.0, 50.0);
            o.rotation = Quat::IDENTITY;
            o.scale = Vec3::ONE;
        }
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        scene.gpu.resize_with(front.max(back) + 1, Default::default);
        let key = GeomKey::Prim { hash: 0, lod: 0 };
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 2,
                unused_frames: 0,
                state: GeomState::Ready(Arc::new(GpuGeom {
                    faces: vec![None],
                    min: -Vec3::splat(0.5),
                    max: Vec3::splat(0.5),
                    joint_bounds: Vec::new(),
                    pick_faces: vec![Some(PickFace {
                        positions: vec![[-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [-0.5, -0.5, 0.5]],
                        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                        indices: vec![0, 1, 2, 0, 2, 3],
                    })],
                })),
            },
        );
        let face = FaceDraw {
            record: 0,
            cmd: DrawCmd {
                index_count: 6,
                first_index: 0,
                base_vertex: 0,
                record: 0,
                bounds: [0.0; 4],
            },
            base_slot: 0,
            tex_id: Uuid::nil(),
            aux_tex: [Uuid::nil(); 3],
            aux_repeats: [1.0; 3],
            te_alpha: 1.0,
            pbr_alpha: None,
            legacy_alpha: None,
            two_sided: false,
            repeats: 1.0,
            glow: false,
        };
        for idx in [front, back] {
            scene.gpu[idx].geom = Some(key);
            scene.gpu[idx].faces.push(face);
        }
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let depth = Vec3::new(2.5, 0.0, 50.0);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        let behind = scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0).unwrap();
        assert_eq!(behind, Vec3::new(6.5, 0.0, 50.0));
        assert_eq!(scene.interaction_at(&world, behind, Instant::now(), false), Some(back));
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(depth), true, 100.0), Some(depth));
        assert_eq!(scene.interaction_at(&world, depth, Instant::now(), true), Some(front));
        let surface = scene.touch_surface(&world, back, ray).unwrap();
        assert_eq!(surface.face, 0);
        assert_eq!(surface.st, Vec3::new(0.5, 0.5, 0.0));
        assert_eq!(surface.position, behind);
        assert!((surface.normal.length() - 1.0).abs() < 1e-5);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::DISABLED;
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0), Some(depth));
        assert_eq!(scene.interaction_at(&world, depth, Instant::now(), false), Some(front));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::IGNORE;
        assert!(scene.interaction_point(&world, Some(ray), Some(depth), false, 100.0).is_none());
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::BUY;
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        world.objects.get_mut(front).unwrap().update_flags = 1 << 7;
        scene.gpu[front].faces[0].te_alpha = 0.5;
        let through = Vec3::new(6.5, 0.0, 50.0);
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(through), false, 100.0), Some(depth));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::DISABLED;
        assert_eq!(scene.interaction_point(&world, Some(ray), Some(through), false, 100.0), Some(depth));
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::NONE;
        scene.gpu[front].faces[0].te_alpha = 0.0;
        let te = world.objects.get_mut(front).unwrap().te.as_mut().unwrap();
        for face in Arc::make_mut(te).faces.iter_mut() {
            face.color[3] = 0.0;
        }
        assert_eq!(
            scene.interaction_point(&world, Some(ray), Some(through), false, 100.0),
            Some(through)
        );
        assert_eq!(scene.action_point(&world, Some(ray), Some(through), false, 100.0), Some(depth));
    }
}
