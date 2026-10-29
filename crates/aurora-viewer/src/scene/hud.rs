//! HUD attachments of the local avatar, independent of its world transform.
//! Port of LLVOAvatarSelf::updateCharacter / render_hud_attachments and
//! LLViewerWindow::cursorIntersect (Firestorm, originally LGPL 2.1).
use super::*;
use aurora_render::HudView;

/// LLVOVolume::updateTextureVirtualSize gives every HUD face the whole
/// screen and BOOST_HUD, which LLViewerFetchedTexture::setBoostLevel and
/// addTextureStats (llviewertexture.cpp) turn into 2048 * 2048 and discard
/// 0: HUD textures load at full resolution, whatever the size of their prim.
/// A size from the prim's radius left the small buttons of a HUD, which
/// often show one corner of a shared atlas, three levels too coarse.
pub(super) const TEXTURE_PIXELS: f32 = 2048.0;

impl Scene {
    /// Screen bounds of the visible linkset, using its zoomed geometry.
    pub fn hud_rect(&self, world: &World, idx: usize, view: HudView) -> Option<[f32; 4]> {
        let root = world.hud_root(idx)?;
        let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for idx in crate::build::family(world, root) {
            if !self
                .gpu
                .get(idx)
                .is_some_and(|g| g.hud && g.geom.is_some() && g.faces.iter().any(|f| f.pass != Pass::Hidden))
            {
                continue;
            }
            let (center, size) = self.click_bounds(world, idx)?;
            let (_, rot, _) = Self::object_transform(world, idx, Instant::now(), 0)?;
            for corner in 0..8 {
                let sign = Vec3::new(
                    if corner & 1 == 0 { -1.0 } else { 1.0 },
                    if corner & 2 == 0 { -1.0 } else { 1.0 },
                    if corner & 4 == 0 { -1.0 } else { 1.0 },
                );
                let p = center + rot * (size * sign * 0.5);
                let x = view.width * 0.5 - p.y * view.height * view.zoom;
                let y = view.height * 0.5 - p.z * view.height * view.zoom;
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            }
        }
        bounds.iter().all(|v| v.is_finite()).then_some(bounds)
    }

    /// Every prim of the linkset has its final geometry (or gave up on it).
    /// At login the first prims to arrive can be panels the HUD parks off
    /// screen: their bounds alone would push the whole HUD aside (#626).
    pub fn hud_loaded(&self, world: &World, idx: usize) -> bool {
        world.hud_root(idx).is_some_and(|root| {
            crate::build::family(world, root).into_iter().all(|idx| {
                self.gpu
                    .get(idx)
                    .and_then(|g| g.geom)
                    .is_some_and(|key| self.geom_ready(&key).is_some() || self.geom_failed(&key))
            })
        })
    }

    pub fn build_huds(&mut self, world: &World, size: [u32; 2], visible: bool) {
        self.lists.hud_view = None;
        self.lists.hud_opaque.clear();
        self.lists.hud_blend.clear();
        self.lists.hud_debug_red.clear();
        self.lists.hud_debug_blue.clear();
        self.hud_own.clear();
        self.lists.hud_select_faces.clear();
        if !visible {
            return;
        }
        let Some(own) = world.objects.index_of_uuid(&world.agent_id) else {
            return;
        };
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut blend = Vec::new();
        // (the HUD attachments, from the lists' index: not a walk of every
        // object at each frame)
        let slots: Box<dyn Iterator<Item = usize> + '_> = if list_index::mode() == list_index::Mode::Walk {
            Box::new(0..self.gpu.len())
        } else {
            Box::new(self.list_index.with(list_index::HUD).map(|(idx, _)| idx))
        };
        for idx in slots {
            let Some(g) = self.gpu.get(idx) else {
                continue;
            };
            if !g.hud || g.owner_avatar != Some(own) || g.geom.is_none() {
                continue;
            }
            self.hud_own.push(idx);
            min_x = min_x.min(g.center.x - g.radius - 0.1);
            max_x = max_x.max(g.center.x + g.radius + 0.1);
            // HUD textures must stream even when the world camera is far
            // away; never world distance, nor the size of the prim.
            for id in &g.tex_ids {
                self.textures.note_usage_fixed(id, TEXTURE_PIXELS);
            }
            for face in &g.faces {
                // LLDrawPoolAlpha::renderDebugAlpha also runs in the HUD pass.
                if self.debug_alpha.is_some() {
                    match face.pass {
                        Pass::Blend | Pass::Hidden => self.lists.hud_debug_red.push(face.cmd),
                        Pass::Mask | Pass::MaskTwoSided if face.pbr_alpha.is_some() || face.legacy_alpha.is_some() => {
                            self.lists.hud_debug_blue.push(face.cmd);
                        }
                        Pass::Mask | Pass::MaskTwoSided => self.lists.hud_debug_red.push(face.cmd),
                        _ => {}
                    }
                }
                if self
                    .selection_faces
                    .get(&idx)
                    .is_some_and(|mask| mask & 1u64.checked_shl(face.te_index.into()).unwrap_or(0) != 0)
                {
                    self.lists.hud_select_faces.push(face.cmd);
                }
                match face.pass {
                    Pass::Hidden => {}
                    Pass::Blend => blend.push((g.center.x, face.cmd)),
                    _ => self.lists.hud_opaque.push(face.cmd),
                }
            }
        }
        if min_x.is_finite() && max_x.is_finite() {
            self.lists.hud_view = Some(HudView::new(size, world.hud_zoom, min_x, max_x));
        }
        blend.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.lists.hud_blend.extend(blend.into_iter().map(|(_, cmd)| cmd));
    }

    /// Left clicks and hover intercept HUD geometry regardless of opacity.
    pub fn hud_pick(&self, world: &World, cursor: (f32, f32), include_hidden: bool) -> Option<(usize, Vec3, (Vec3, Vec3))> {
        self.hud_pick_impl(
            world,
            cursor,
            include_hidden || self.debug_alpha.is_some(),
            false,
            picking::FacePick::Surface,
        )
    }

    /// The HUD move gesture reaches IGNORE prims, but respects texture holes.
    pub fn hud_move_pick(&self, world: &World, cursor: (f32, f32)) -> Option<usize> {
        self.hud_pick_impl(world, cursor, self.debug_alpha.is_some(), true, picking::FacePick::HudMove)
            .map(|(idx, _, _)| idx)
    }

    /// Editing and right-click selection also reach CLICK_ACTION_IGNORE
    /// prims, as LLViewerWindow::cursorIntersect does in build mode.
    pub fn hud_edit_pick(&self, world: &World, cursor: (f32, f32)) -> Option<(usize, Vec3, (Vec3, Vec3))> {
        self.hud_pick_impl(world, cursor, true, true, picking::FacePick::Surface)
    }

    fn hud_pick_impl(
        &self,
        world: &World,
        cursor: (f32, f32),
        include_hidden: bool,
        include_ignored: bool,
        mode: picking::FacePick,
    ) -> Option<(usize, Vec3, (Vec3, Vec3))> {
        let view = self.lists.hud_view?;
        let ray = view.ray(cursor.0, cursor.1);
        let own = world.objects.index_of_uuid(&world.agent_id)?;
        let now = Instant::now();
        let mut nearest = view.max_x - view.min_x;
        let mut found = None;
        // The HUD objects `build_huds` listed for this frame's view, in
        // index order: not a pass over every object of the region at each
        // frame the cursor is over the world (AURORA_HOVER_LEGACY: that
        // pass, to compare).
        let legacy = self.hover.legacy();
        let every = if legacy { 0..self.gpu.len() } else { 0..0 };
        for idx in self.hud_own.iter().copied().filter(|_| !legacy).chain(every) {
            let (Some(g), Some(o)) = (self.gpu.get(idx), world.objects.get(idx)) else {
                continue;
            };
            if !g.hud
                || g.owner_avatar != Some(own)
                || (!include_hidden && mode != picking::FacePick::Surface && g.faces.iter().all(|f| f.pass == Pass::Hidden))
                || (!include_ignored && o.click_action == crate::interaction::code::IGNORE)
                || !picking::ray_may_hit(g, ray)
            {
                continue;
            }
            if let Some(hit) = self.face_hit_mode(world, idx, ray, now, include_hidden, mode)
                && hit.t < nearest
            {
                nearest = hit.t;
                found = Some((idx, ray.0 + ray.1 * hit.t, ray));
            }
        }
        found
    }

    /// The held touch stays in its original space when the cursor leaves
    /// the HUD; it must never fall back to a world ray.
    pub fn object_cursor_ray(&self, idx: usize, cursor: (f32, f32), world_ray: Option<(Vec3, Vec3)>) -> Option<(Vec3, Vec3)> {
        if self.gpu.get(idx).is_some_and(|g| g.hud) {
            self.lists.hud_view.map(|view| view.ray(cursor.0, cursor.1))
        } else {
            world_ray
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_zoom_scales_linked_offsets_without_changing_simulator_transforms() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let child = crate::build::family(&world, root)[1];
        let now = Instant::now();
        let pivot = Scene::object_transform(&world, root, now, 0).unwrap().0;
        let before = Scene::object_transform(&world, child, now, 0).unwrap().0;
        let id = world.objects.get(root).unwrap().full_id;
        let original = [root, child].map(|idx| {
            let o = world.objects.get(idx).unwrap();
            (o.position, o.rotation, o.scale)
        });
        world.objects.get_mut(root).unwrap().update_flags = 0;
        world.hud_scales.set(id, 2.0);
        assert_eq!(Scene::object_transform(&world, root, now, 0).unwrap().0, pivot);
        assert!(
            Scene::object_transform(&world, child, now, 0)
                .unwrap()
                .0
                .abs_diff_eq(pivot + (before - pivot) * 2.0, 1e-5)
        );
        for (idx, transform) in [root, child].into_iter().zip(original) {
            let o = world.objects.get(idx).unwrap();
            assert_eq!((o.position, o.rotation, o.scale), transform);
            assert_eq!(world.visual_scale(idx), o.scale * 2.0);
        }
        assert_eq!(world.hud_scale(world.objects.index_of_uuid(&world.agent_id).unwrap()), 1.0);
        let foreign = world.objects.iter().find(|(_, o)| o.key.local_id == 8241).unwrap().0;
        assert_eq!(world.hud_scale(foreign), 1.0);
    }

    #[test]
    fn a_hud_is_measured_for_the_accessibility_offset_only_once_every_prim_has_geometry() {
        let (_, mut scene, ..) = crate::scene::picking::tests::scene_with_two_objects();
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let family = crate::build::family(&world, root);
        let child = family[1];
        let (ready, pending, failed) = (
            GeomKey::Prim { hash: 0, lod: 0 },
            GeomKey::Prim { hash: 1, lod: 0 },
            GeomKey::Prim { hash: 2, lod: 0 },
        );
        for (key, state) in [(pending, GeomState::Pending), (failed, GeomState::Failed)] {
            scene.geoms.insert(
                key,
                GeomEntry {
                    refs: 1,
                    zero_since: 0,
                    state,
                },
            );
        }
        scene.gpu.resize_with(scene.gpu.len().max(root.max(child) + 1), Default::default);
        scene.gpu[root].geom = Some(ready);
        scene.gpu[child].geom = None;
        // The child is not synced yet, then waits for its geometry: the
        // root alone is not the HUD.
        assert!(!scene.hud_loaded(&world, root));
        scene.gpu[child].geom = Some(pending);
        assert!(!scene.hud_loaded(&world, child));
        scene.gpu[child].geom = Some(failed);
        assert!(
            scene.hud_loaded(&world, root),
            "a prim that will never load does not hold the HUD back"
        );
        scene.gpu[child].geom = Some(ready);
        assert!(scene.hud_loaded(&world, child));
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert!(!scene.hud_loaded(&world, avatar));
    }

    #[test]
    fn hud_anchors_and_linksets_ignore_avatar_pose_and_region_offset() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib);
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        world.hud_aspect = 2.0;
        let now = Instant::now();
        let roots: Vec<_> = (31..=38)
            .map(|p| world.objects.index_of_uuid(&crate::demo::hud::object(p, false).full_id).unwrap())
            .collect();
        let before: Vec<_> = roots
            .iter()
            .map(|&idx| Scene::object_transform(&world, idx, now, 0).unwrap())
            .collect();
        for (point, (pos, _, hud)) in (31..=38).zip(&before) {
            let o = crate::demo::hud::object(point, false);
            let anchor = world.avatar_lib.attach_points[&point].position;
            assert!(*hud);
            assert!((*pos - o.position - anchor * Vec3::new(1.0, 2.0, 1.0)).length() < 1e-5);
        }
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let o = world.objects.get_mut(avatar).unwrap();
        o.position += Vec3::splat(500.0);
        o.rotation = Quat::from_rotation_z(1.7);
        world.agent.position += Vec3::splat(500.0);
        world.agent.yaw += 1.7;
        for (&idx, old) in roots.iter().zip(before) {
            assert_eq!(Scene::object_transform(&world, idx, now, 0).unwrap(), old);
        }
        let child = world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == crate::demo::hud::CHILD)
            .map(|(idx, _)| idx)
            .unwrap();
        let root = roots[0];
        let (rp, rr, _) = Scene::object_transform(&world, root, now, 0).unwrap();
        let child_o = world.objects.get(child).unwrap();
        let (cp, cr, hud) = Scene::object_transform(&world, child, now, 0).unwrap();
        assert!(hud);
        assert!((cp - rp - rr * child_o.position).length() < 1e-5);
        assert!(cr.abs_diff_eq(rr * child_o.rotation, 1e-5));
    }
}
