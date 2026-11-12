//! Media focus and input (LLViewerMediaFocus, LLToolPie::handleMediaClick /
//! handleMediaHover): a click on a media face focuses it (and loads it);
//! the mouse, wheel and keyboard then go to the page.

use super::pick::{self, FaceTris};
use super::*;
use std::sync::Arc;

/// CPU copy of a face for picking.
pub(super) struct CpuFace {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u16>,
}

/// A media face under the cursor.
#[derive(Debug, Clone, Copy)]
pub struct MediaHit {
    pub key: MediaKey,
    /// LL texture coordinates on the face (after its texture transform).
    pub st: Vec2,
    /// Distance along the ray.
    pub t: f32,
}

impl MediaManager {
    /// Faces of an object for picking (the prim volume at its LOD, or the
    /// cached mesh asset), kept while the object's shape is unchanged. A
    /// local mesh preview counts as the shape, as it replaces the volume in
    /// VJLocalMeshObject (setVolume).
    fn cpu_faces(&mut self, world: &World, scene: &Scene, idx: usize) -> Option<Arc<Vec<Option<CpuFace>>>> {
        let o = world.objects.get(idx)?;
        let volume = o.visual_volume();
        let lod = scene.gpu.get(idx).map(|g| g.lod).unwrap_or(3).min(3);
        let key = volume.cache_key() ^ ((lod as u64) << 60);
        if let Some((k, faces)) = self.pick_cache.get(&o.full_id)
            && *k == key
        {
            return Some(faces.clone());
        }
        let faces: Vec<Option<CpuFace>> = if volume.is_mesh() {
            let id = volume.sculpt?.texture;
            // the finest level held (no longer read back from the disk cache)
            let mesh = scene.meshes.view(&id)?;
            aurora_assets::mesh::decode_mesh_lod(mesh.lod_section(3)?)
                .ok()?
                .into_iter()
                .map(|f| {
                    (!f.is_empty()).then_some(CpuFace {
                        positions: f.positions,
                        uvs: f.uvs,
                        indices: f.indices,
                    })
                })
                .collect()
        } else if volume.is_sculpt() {
            return None;
        } else {
            let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize];
            aurora_prim::generate_volume(&volume, detail)
                .faces
                .into_iter()
                .map(|f| {
                    Some(CpuFace {
                        positions: f.positions,
                        uvs: f.uvs,
                        indices: f.indices,
                    })
                })
                .collect()
        };
        let faces = Arc::new(faces);
        if self.pick_cache.len() > 64 {
            self.pick_cache.clear();
        }
        self.pick_cache.insert(o.full_id, (key, faces.clone()));
        Some(faces)
    }

    /// Number of faces (texture entries) of an object: the prim volume's,
    /// or the mesh asset's submeshes once it is cached (the local mesh's
    /// during a preview).
    pub fn face_count(&mut self, world: &World, scene: &Scene, idx: usize) -> Option<usize> {
        let volume = world.objects.get(idx)?.visual_volume();
        if volume.is_mesh() {
            return self.cpu_faces(world, scene, idx).map(|f| f.len().max(1));
        }
        if volume.is_sculpt() {
            return Some(1);
        }
        Some(aurora_prim::volume::num_faces(&volume).max(1))
    }

    /// The nearest media face along a ray (objects with media data only),
    /// unless something drawn is in front (`depth_t`: distance of the
    /// picked depth).
    /// `picked`: the object the click's own pick found (a HUD's): only its
    /// faces count, as LLToolPie::handleMediaClick looks at mPick's face.
    pub fn hit_test(
        &mut self,
        world: &World,
        scene: &Scene,
        origin: Vec3,
        dir: Vec3,
        depth_t: Option<f32>,
        hud: bool,
        picked: Option<usize>,
    ) -> Option<MediaHit> {
        let now = Instant::now();
        let candidates: Vec<(Uuid, usize, Vec<bool>)> = self
            .objects
            .iter()
            .filter(|(_, om)| om.hud == hud && picked.is_none_or(|p| p == om.idx))
            .filter(|(_, om)| {
                !hud || scene
                    .gpu
                    .get(om.idx)
                    .is_some_and(|g| g.owner_avatar == world.objects.index_of_uuid(&world.agent_id))
            })
            .filter_map(|(id, om)| {
                let d = om.data.as_ref()?;
                Some((
                    *id,
                    om.idx,
                    d.faces.iter().map(|f| f.as_ref().is_some_and(|e| !e.url().is_empty())).collect(),
                ))
            })
            .collect();
        let mut best: Option<MediaHit> = None;
        for (id, idx, with_media) in candidates {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            if !self.build_mode && o.click_action == crate::interaction::code::IGNORE {
                continue;
            }
            let Some((pos, rot, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let model = Mat4::from_scale_rotation_translation(o.scale, rot, pos);
            let Some(faces) = self.cpu_faces(world, scene, idx) else {
                continue;
            };
            // LLVOVolume::lineSegmentIntersect: a face at alpha 0 is not
            // picked unless its own prim shows the special cursor
            // (specialHoverCursor: physics, a touch script, a click action),
            // so a hidden media panel in front of a HUD button lets the
            // button take the click
            let special_cursor = crate::interaction::special_hover_cursor(o);
            let shown = |fi: usize| special_cursor || self.build_mode || o.te.as_ref().is_none_or(|te| te.face(fi).color[3] > 0.0);
            let tris = faces.iter().enumerate().filter_map(|(fi, f)| {
                let f = f.as_ref()?;
                (with_media.get(fi).copied().unwrap_or(false) && shown(fi)).then_some(FaceTris {
                    face: fi,
                    positions: &f.positions,
                    uvs: &f.uvs,
                    indices: &f.indices,
                })
            });
            let Some(hit) = pick::ray_faces_with_sidedness(tris, model, origin, dir, |face| scene.face_double_sided(world, o, face)) else {
                continue;
            };
            if best.is_some_and(|b| hit.t >= b.t) {
                continue;
            }
            let tf = o.te.as_ref().map(|te| *te.face(hit.face)).unwrap_or_default();
            let st = pick::te_transform(hit.uv, [tf.scale_s, tf.scale_t], [tf.offset_s, tf.offset_t], tf.rotation);
            best = Some(MediaHit {
                key: MediaKey::Prim {
                    object: id,
                    face: hit.face as u8,
                },
                st,
                t: hit.t,
            });
        }
        // hidden behind something closer
        match (best, depth_t) {
            (Some(b), Some(d)) if d < b.t - 0.15 => None,
            (b, _) => b,
        }
    }

    /// Pixel of the media under texture coordinates.
    fn pixel(&self, key: &MediaKey, st: Vec2) -> Option<(i32, i32)> {
        let m = self.find(key)?;
        let p = m.plugin.as_ref()?;
        let tex = p.texture_size();
        let media = p.media_size();
        if media.0 <= 0 || media.1 <= 0 {
            return None;
        }
        Some(pick::media_pixel(st, tex, media))
    }

    /// May the agent touch this face (RLVa)? perms_interact is not checked
    /// here: LLToolPie::handleMediaClick gives the click to the page anyway
    /// and LLVOVolume::mediaNavigated only refuses the navigation it leads
    /// to (`MediaImpl::may_navigate`). Refusing the click sent it to the
    /// object's script instead: a HUD page with interaction set to nobody
    /// closed at the first click.
    fn may_touch(&self, world: &World, key: &MediaKey, position: Option<Vec3>) -> bool {
        let MediaKey::Prim { object, .. } = key else {
            return false;
        };
        let Some(om) = self.objects.get(object) else {
            return false;
        };
        !world.rlva.enabled()
            || world
                .objects
                .get(om.idx)
                .is_some_and(|o| world.rlva_can_touch_at(o.key.region, o.key.local_id, position))
    }

    /// Left press in the world. Returns true when a media face took it.
    pub fn on_click(
        &mut self,
        world: &World,
        scene: &Scene,
        ray: (Vec3, Vec3),
        depth_t: Option<f32>,
        settings: &MediaSettings,
        mods: Modifiers,
        hud: bool,
        picked: Option<usize>,
    ) -> bool {
        if !settings.enabled || !settings.prim_media {
            return false;
        }
        let Some(hit) = self.hit_test(world, scene, ray.0, ray.1, depth_t, hud, picked) else {
            self.unfocus();
            return false;
        };
        let position = Some(ray.0 + ray.1 * hit.t);
        if !self.may_touch(world, &hit.key, position) {
            return false;
        }
        let already = self.focus == Some(hit.key);
        if !already {
            self.unfocus();
            self.focus = Some(hit.key);
            self.last_surface = position;
            let Some(m) = self.find_mut(&hit.key) else {
                return false;
            };
            // LLViewerMediaFocus::setFocusFace: load it if needed
            if !m.wanted {
                m.wanted = true;
                if let Some(u) = m.entry.as_ref().map(|e| e.url().to_string())
                    && !u.is_empty()
                    && u != m.url
                {
                    m.navigate(&u);
                }
            }
            m.failed = None;
            if let Some(p) = m.plugin.as_mut() {
                p.focus(true);
            }
            let first_click = settings.first_click_interact && m.entry.as_ref().is_some_and(|e| e.first_click_interact);
            if !first_click || !m.is_loaded() {
                return true;
            }
        }
        self.last_surface = position;
        if let Some((x, y)) = self.pixel(&hit.key, hit.st) {
            self.last_pixel = Some((x, y));
            if let Some(m) = self.find_mut(&hit.key)
                && let Some(p) = m.plugin.as_mut()
            {
                p.focus(true);
                p.mouse_event(MouseEvent::Down, 0, x, y, mods);
                m.mouse_down = true;
                m.user_nav = true;
            }
        }
        true
    }

    /// Left release: mouse up on the focused media.
    pub fn on_release(&mut self, mods: Modifiers) {
        let pos = self.last_pixel;
        if let Some(m) = self.focused_mut()
            && m.mouse_down
        {
            m.mouse_down = false;
            if let (Some(p), Some((x, y))) = (m.plugin.as_mut(), pos) {
                p.mouse_event(MouseEvent::Up, 0, x, y, mods);
            }
        }
    }

    /// Cursor moved over the world: mouse moves for the focused media.
    /// Returns the plugin's cursor name when the cursor is over it.
    pub fn on_hover(
        &mut self,
        world: &World,
        scene: &Scene,
        ray: (Vec3, Vec3),
        depth_t: Option<f32>,
        mods: Modifiers,
        hud: bool,
    ) -> Option<String> {
        let focus = self.focus?;
        let hit = self
            .hit_test(world, scene, ray.0, ray.1, depth_t, hud, None)
            .filter(|h| h.key == focus)?;
        let position = Some(ray.0 + ray.1 * hit.t);
        if !self.may_touch(world, &focus, position) {
            self.unfocus();
            return None;
        }
        self.last_surface = position;
        let (x, y) = self.pixel(&focus, hit.st)?;
        if self.last_pixel != Some((x, y)) {
            self.last_pixel = Some((x, y));
            if let Some(p) = self.focused_mut().and_then(|m| m.plugin.as_mut()) {
                p.mouse_event(MouseEvent::Move, 0, x, y, mods);
            }
        }
        self.focused().map(|m| m.cursor.clone())
    }

    /// Wheel over the focused media (true: used).
    pub fn on_scroll(
        &mut self,
        world: &World,
        scene: &Scene,
        ray: (Vec3, Vec3),
        depth_t: Option<f32>,
        clicks: f32,
        mods: Modifiers,
        hud: bool,
    ) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        let Some(hit) = self
            .hit_test(world, scene, ray.0, ray.1, depth_t, hud, None)
            .filter(|h| h.key == focus)
        else {
            return false;
        };
        let position = Some(ray.0 + ray.1 * hit.t);
        if !self.may_touch(world, &focus, position) {
            self.unfocus();
            return false;
        }
        self.last_surface = position;
        let Some((x, y)) = self.pixel(&focus, hit.st) else {
            return false;
        };
        if let Some(p) = self.focused_mut().and_then(|m| m.plugin.as_mut()) {
            // LL passes wheel clicks (down = positive)
            p.scroll_event(x, y, 0, (-clicks).round() as i32, mods);
        }
        true
    }

    /// A key while a media has focus (true: used). Escape releases it.
    pub fn on_key(
        &mut self,
        world: &World,
        letter: Option<char>,
        code: winit::keyboard::KeyCode,
        scancode: u32,
        pressed: bool,
        repeat: bool,
        text: Option<&str>,
        mods: Modifiers,
        clipboard: Option<&str>,
    ) -> bool {
        self.enforce_focus(world);
        if self.focus.is_none() {
            return false;
        }
        if pressed && code == winit::keyboard::KeyCode::Escape {
            self.unfocus();
            return true;
        }
        let Some(p) = self.focused_mut().and_then(|m| m.plugin.as_mut()) else {
            return pressed;
        };
        keys::edit(p, letter, code, pressed, repeat, mods, clipboard)
            || keys::forward(p, code, scancode, pressed, repeat, text, mods)
            || pressed
    }

    /// Release the focus (LLViewerMediaFocus::clearFocus).
    pub fn enforce_focus(&mut self, world: &World) {
        if world.rlva.enabled()
            && self
                .focus
                .is_some_and(|key| matches!(key, MediaKey::Prim { .. }) && !self.may_touch(world, &key, self.last_surface))
        {
            self.unfocus();
        }
    }

    /// Release the focus (LLViewerMediaFocus::clearFocus).
    pub fn unfocus(&mut self) {
        if let Some(m) = self.focused_mut() {
            m.mouse_down = false;
            if let Some(p) = m.plugin.as_mut() {
                p.focus(false);
            }
        }
        self.focus = None;
        self.last_pixel = None;
        self.last_surface = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_press_reaches_media_behind_a_plain_invisible_panel() {
        let (mut world, mut scene, front, back) = crate::scene::picking::tests::scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let parent = world.objects.get(own).unwrap().key.local_id;
        let mut media = MediaManager::default();
        for (idx, x) in [(front, 3.0), (back, 7.0)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.parent_id = parent;
            o.state = 31u8.rotate_left(4);
            o.position = Vec3::new(x, 0.0, 0.0);
            o.click_action = crate::interaction::code::NONE;
            o.update_flags = 0;
            let id = o.full_id;
            let cache_key = o.visual_volume().cache_key() ^ (3u64 << 60);
            let key = MediaKey::Prim { object: id, face: 0 };
            media.objects.insert(
                id,
                ObjMedia {
                    idx,
                    data: Some(ObjectMediaData {
                        faces: vec![Some(MediaEntry {
                            current_url: "https://example.invalid/media".into(),
                            ..Default::default()
                        })],
                        ..Default::default()
                    }),
                    hud: true,
                    ..Default::default()
                },
            );
            media
                .impls
                .push(MediaImpl::new(key, "https://example.invalid/media".into(), "text/html".into()));
            media.pick_cache.insert(
                id,
                (
                    cache_key,
                    Arc::new(vec![Some(CpuFace {
                        positions: vec![[-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [-0.5, -0.5, 0.5]],
                        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                        indices: vec![0, 2, 1, 0, 3, 2],
                    })]),
                ),
            );
            let g = &mut scene.gpu[idx];
            g.hud = true;
            g.lod = 3;
            g.owner_avatar = Some(own);
            g.center = o.position;
            g.radius = 1.0;
        }
        for idx in [front, back] {
            // the server marks a face with prim media (TEM_MEDIA_MASK)
            Arc::make_mut(world.objects.get_mut(idx).unwrap().te.as_mut().unwrap()).faces[0].media_flags |= 1;
        }
        Arc::make_mut(world.objects.get_mut(front).unwrap().te.as_mut().unwrap()).faces[0].color[3] = 0.0;
        crate::scene::picking::tests::index_test_objects(&mut scene);
        scene.build_huds(&world, [1280, 720], true, |_| None);
        let cursor = (640.0, 360.0);
        let press = |media: &mut MediaManager, world: &World, scene: &Scene| {
            let (idx, point, ray) = scene.hud_click_pick(world, cursor).unwrap();
            let distance = (point - ray.0).dot(ray.1);
            assert!(media.on_click(
                world,
                scene,
                ray,
                Some(distance),
                &MediaSettings::default(),
                Modifiers::default(),
                true,
                Some(idx),
            ));
            idx
        };
        assert_eq!(press(&mut media, &world, &scene), back);
        assert_eq!(
            media.focus,
            Some(MediaKey::Prim {
                object: world.objects.get(back).unwrap().full_id,
                face: 0
            })
        );

        // A touch script makes the transparent front face a deliberate clicker.
        world.objects.get_mut(front).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        assert_eq!(press(&mut media, &world, &scene), front);
    }

    #[test]
    fn a_media_face_at_alpha_zero_lets_the_click_through_unless_its_prim_is_touchable() {
        let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(library.clone());
        for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let object = world.objects.get_mut(idx).unwrap();
        object.position = Vec3::X * 3.0;
        // facing the ray
        object.rotation = glam::Quat::from_rotation_z(std::f32::consts::PI);
        object.scale = Vec3::ONE;
        object.click_action = 0;
        object.update_flags = 0;
        let id = object.full_id;
        let cache_key = object.visual_volume().cache_key() ^ (3u64 << 60);
        let scene = Scene::new(std::path::PathBuf::new(), library);
        let mut media = MediaManager::default();
        media.objects.insert(
            id,
            ObjMedia {
                idx,
                data: Some(ObjectMediaData {
                    faces: vec![Some(MediaEntry {
                        current_url: "https://example.invalid/media".into(),
                        ..Default::default()
                    })],
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        media.pick_cache.insert(
            id,
            (
                cache_key,
                Arc::new(vec![Some(CpuFace {
                    positions: vec![[0.0, -0.5, -0.5], [0.0, 0.5, -0.5], [0.0, 0.5, 0.5], [0.0, -0.5, 0.5]],
                    uvs: vec![[0.0; 2]; 4],
                    indices: vec![0, 1, 2, 0, 2, 3],
                })]),
            ),
        );
        let set_alpha = |world: &mut World, alpha: f32| {
            let o = world.objects.get_mut(idx).unwrap();
            let mut te = o.te.as_deref().cloned().unwrap_or_default();
            for f in &mut te.faces {
                f.color[3] = alpha;
            }
            o.te = Some(Arc::new(te));
        };
        set_alpha(&mut world, 1.0);
        assert!(media.hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, None).is_some());
        // a click whose own pick found another object in front (a HUD button
        // with a partly transparent texture) is not the media's
        assert!(
            media
                .hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, Some(idx))
                .is_some()
        );
        assert!(
            media
                .hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, Some(idx + 1))
                .is_none()
        );
        // a web HUD's collapsed panel: hidden in front of its root button
        set_alpha(&mut world, 0.0);
        assert!(media.hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, None).is_none());
        // a prim with its own touch handler still takes it (specialHoverCursor)
        world.objects.get_mut(idx).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        assert!(media.hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, None).is_some());
    }

    #[test]
    fn invisible_back_of_a_media_face_cannot_steal_world_clicks() {
        let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(library.clone());
        for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let object = world.objects.get_mut(idx).unwrap();
        object.position = Vec3::X * 3.0;
        object.rotation = glam::Quat::IDENTITY;
        object.scale = Vec3::ONE;
        let id = object.full_id;
        let cache_key = object.visual_volume().cache_key() ^ (3u64 << 60);
        let mut scene = Scene::new(std::path::PathBuf::new(), library);
        let mut media = MediaManager::default();
        media.objects.insert(
            id,
            ObjMedia {
                idx,
                data: Some(ObjectMediaData {
                    faces: vec![Some(MediaEntry {
                        current_url: "https://example.invalid/media".into(),
                        ..Default::default()
                    })],
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        media.pick_cache.insert(
            id,
            (
                cache_key,
                Arc::new(vec![Some(CpuFace {
                    positions: vec![[0.0, -0.5, -0.5], [0.0, 0.5, -0.5], [0.0, 0.5, 0.5], [0.0, -0.5, 0.5]],
                    uvs: vec![[0.0; 2]; 4],
                    indices: vec![0, 1, 2, 0, 2, 3],
                })]),
            ),
        );
        assert!(media.hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, None).is_none());
        world.objects.get_mut(idx).unwrap().rotation = glam::Quat::from_rotation_z(std::f32::consts::PI);
        assert!(media.hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, None).is_some());
        world.objects.get_mut(idx).unwrap().rotation = glam::Quat::IDENTITY;
        let material = Uuid::from_u128(8896);
        world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(0, material)];
        scene.materials.insert(
            material,
            aurora_assets::PbrMaterial {
                double_sided: true,
                ..Default::default()
            },
        );
        assert!(media.hit_test(&world, &scene, Vec3::ZERO, Vec3::X, None, false, None).is_some());
    }
}
