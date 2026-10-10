//! HUD attachments of the local avatar, independent of its world transform.
//! Port of LLVOAvatarSelf::updateCharacter / render_hud_attachments and
//! LLViewerWindow::cursorIntersect (Firestorm, originally LGPL 2.1).
use super::*;
use aurora_render::HudView;

impl Scene {
    pub fn build_huds(&mut self, world: &World, size: [u32; 2], visible: bool) {
        self.lists.hud_view = None;
        self.lists.hud_opaque.clear();
        self.lists.hud_blend.clear();
        if !visible {
            return;
        }
        let Some(own) = world.objects.index_of_uuid(&world.agent_id) else {
            return;
        };
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut blend = Vec::new();
        for g in &self.gpu {
            if !g.hud || g.owner_avatar != Some(own) || g.geom.is_none() {
                continue;
            }
            min_x = min_x.min(g.center.x - g.radius - 0.1);
            max_x = max_x.max(g.center.x + g.radius + 0.1);
            // HUD textures must stream even when the world camera is far
            // away; their interest is screen area, never world distance.
            let px = (g.radius * 2.0 * size[1] as f32 * world.hud_zoom).max(32.0);
            for id in &g.tex_ids {
                self.textures.note_usage(id, px);
            }
            for face in &g.faces {
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

    pub fn hud_pick(&self, world: &World, cursor: (f32, f32), include_hidden: bool) -> Option<(usize, Vec3, (Vec3, Vec3))> {
        self.hud_pick_impl(world, cursor, include_hidden, false)
    }

    /// Alt movement picks visible geometry even when its click action is IGNORE.
    pub fn hud_move_pick(&self, world: &World, cursor: (f32, f32)) -> Option<usize> {
        self.hud_pick_impl(world, cursor, false, true).map(|(idx, _, _)| idx)
    }

    fn hud_pick_impl(
        &self,
        world: &World,
        cursor: (f32, f32),
        include_hidden: bool,
        include_ignored: bool,
    ) -> Option<(usize, Vec3, (Vec3, Vec3))> {
        let view = self.lists.hud_view?;
        let ray = view.ray(cursor.0, cursor.1);
        let own = world.objects.index_of_uuid(&world.agent_id)?;
        let now = Instant::now();
        let mut nearest = view.max_x - view.min_x;
        let mut found = None;
        for (idx, g) in self.gpu.iter().enumerate() {
            let Some(o) = world.objects.get(idx) else { continue };
            if !g.hud
                || g.owner_avatar != Some(own)
                || (!include_hidden && g.faces.iter().all(|f| f.pass == Pass::Hidden))
                || (!include_ignored && o.click_action == crate::interaction::code::IGNORE)
                || !picking::ray_may_hit(g, ray)
            {
                continue;
            }
            if let Some(hit) = self.face_hit(world, idx, ray, now, include_hidden)
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
