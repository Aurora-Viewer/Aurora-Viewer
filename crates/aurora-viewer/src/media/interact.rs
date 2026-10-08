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
    /// cached mesh asset), kept while the object's shape is unchanged.
    fn cpu_faces(&mut self, world: &World, scene: &Scene, idx: usize) -> Option<Arc<Vec<Option<CpuFace>>>> {
        let o = world.objects.get(idx)?;
        let lod = scene.gpu.get(idx).map(|g| g.lod).unwrap_or(3).min(3);
        let key = o.volume.cache_key() ^ ((lod as u64) << 60);
        if let Some((k, faces)) = self.pick_cache.get(&o.full_id)
            && *k == key
        {
            return Some(faces.clone());
        }
        let faces: Vec<Option<CpuFace>> = if o.volume.is_mesh() {
            let id = o.volume.sculpt?.texture;
            let data = std::fs::read(scene.meshes.cache_path(&id)).ok()?;
            let header = aurora_assets::mesh::parse_mesh_header(&data).ok()?;
            let l = header.actual_lod(3)?;
            let section = aurora_assets::mesh::section_bytes(&data, header.lods[l]?)?;
            aurora_assets::mesh::decode_mesh_lod(section)
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
        } else if o.volume.is_sculpt() {
            return None;
        } else {
            let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize];
            aurora_prim::generate_volume(&o.volume, detail)
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

    /// The nearest media face along a ray (objects with media data only),
    /// unless something drawn is in front (`depth_t`: distance of the
    /// picked depth).
    pub fn hit_test(&mut self, world: &World, scene: &Scene, origin: Vec3, dir: Vec3, depth_t: Option<f32>) -> Option<MediaHit> {
        let now = Instant::now();
        let candidates: Vec<(Uuid, usize, Vec<bool>)> = self
            .objects
            .iter()
            .filter(|(_, om)| !om.hud)
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
            let Some((pos, rot, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let model = Mat4::from_scale_rotation_translation(o.scale, rot, pos);
            let Some(faces) = self.cpu_faces(world, scene, idx) else {
                continue;
            };
            let tris = faces.iter().enumerate().filter_map(|(fi, f)| {
                let f = f.as_ref()?;
                with_media.get(fi).copied().unwrap_or(false).then_some(FaceTris {
                    face: fi,
                    positions: &f.positions,
                    uvs: &f.uvs,
                    indices: &f.indices,
                })
            });
            let Some(hit) = pick::ray_faces(tris, model, origin, dir) else {
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

    /// May the agent interact with this face (perms_interact)?
    fn may_interact(&self, world: &World, key: &MediaKey) -> bool {
        let MediaKey::Prim { object, .. } = key else {
            return false;
        };
        let Some(om) = self.objects.get(object) else {
            return false;
        };
        let Some(entry) = self.find(key).and_then(|m| m.entry.as_ref()) else {
            return false;
        };
        let bit = if om.owner == world.agent_id {
            entry::PERM_OWNER | entry::PERM_ANYONE
        } else {
            entry::PERM_ANYONE
        };
        entry.perms_interact & bit != 0
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
    ) -> bool {
        if !settings.enabled || !settings.prim_media {
            return false;
        }
        let Some(hit) = self.hit_test(world, scene, ray.0, ray.1, depth_t) else {
            self.unfocus();
            return false;
        };
        if !self.may_interact(world, &hit.key) {
            return false;
        }
        let already = self.focus == Some(hit.key);
        if !already {
            self.unfocus();
            self.focus = Some(hit.key);
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
    pub fn on_hover(&mut self, world: &World, scene: &Scene, ray: (Vec3, Vec3), depth_t: Option<f32>, mods: Modifiers) -> Option<String> {
        let focus = self.focus?;
        let hit = self.hit_test(world, scene, ray.0, ray.1, depth_t).filter(|h| h.key == focus)?;
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
    ) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        let Some(hit) = self.hit_test(world, scene, ray.0, ray.1, depth_t).filter(|h| h.key == focus) else {
            return false;
        };
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
        code: winit::keyboard::KeyCode,
        scancode: u32,
        pressed: bool,
        repeat: bool,
        text: Option<&str>,
        mods: Modifiers,
    ) -> bool {
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
        let Some(vk) = keys::virtual_key(code) else {
            return pressed;
        };
        let lparam = keys::key_lparam(scancode, !pressed, repeat);
        let (ev, msg) = match (pressed, repeat) {
            (false, _) => (aurora_media::KeyEvent::Up, keys::WM_KEYUP),
            (true, true) => (aurora_media::KeyEvent::Repeat, keys::WM_KEYDOWN),
            (true, false) => (aurora_media::KeyEvent::Down, keys::WM_KEYDOWN),
        };
        p.key_event(ev, vk as i32, mods, MediaPlugin::native_key_data(msg, vk, lparam));
        if pressed && let Some(t) = text.filter(|t| !t.is_empty()) {
            for unit in keys::char_units(t) {
                p.text_input(t, mods, MediaPlugin::native_key_data(keys::WM_CHAR, unit, lparam));
            }
        }
        true
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
    }
}
