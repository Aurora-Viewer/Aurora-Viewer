//! Click actions and held touches, following LLToolPie / LLToolGrabBase
//! (Firestorm indra/newview, originally LGPL 2.1).
use super::*;
use crate::interaction::{self, Action, Contents, Held, Target};

impl App {
    pub(super) fn activate_object_action(&mut self, target: Target, idx: usize, point: Vec3, ray: Option<(Vec3, Vec3)>) {
        let now = Instant::now();
        match target.action {
            Action::Disabled => {}
            Action::Sit => {
                if let Some((pos, rot, _)) = Scene::object_transform(&self.world, idx, now, 0) {
                    self.send(NetCommand::RequestSit {
                        handle: target.key.region,
                        target: target.object,
                        offset: rot.inverse() * (point - pos),
                    });
                }
            }
            Action::Buy | Action::Pay => {
                for cmd in self.interactions.open(target) {
                    self.send(cmd);
                }
            }
            Action::Open => {
                if interaction::allow_open(&self.world, idx) {
                    self.interactions.contents = Some(Contents {
                        target,
                        result: None,
                        opened: now,
                    });
                    self.send(NetCommand::RequestObjectProperties {
                        handle: target.key.region,
                        object: target.root,
                    });
                    self.send(NetCommand::RequestTaskInventory {
                        handle: target.key.region,
                        local_id: target.key.local_id,
                        object: target.object,
                    });
                }
            }
            Action::Play => self.media.toggle_parcel(&self.world, &self.settings.media),
            Action::OpenMedia => {
                let face = ray
                    .and_then(|r| self.scene.touch_surface(&self.world, idx, r))
                    .map_or(-1, |s| s.face);
                self.interactions.media_url = self.media.click_open_media(&self.world, idx, face, &self.settings.media);
            }
            Action::Zoom => {
                if let Some((center, extent)) = self.scene.click_bounds(&self.world, idx) {
                    let object = crate::camera::pick_focus_object(&self.world, idx, now);
                    let aspect = self.gfx.as_ref().map_or(1.0, |g| {
                        let s = g.window.inner_size();
                        s.width as f32 / s.height.max(1) as f32
                    });
                    self.camera
                        .zoom_object(center, extent, object, aspect, &mut self.world.agent, &self.settings.camera);
                }
            }
            Action::Touch | Action::Grab => {
                let Some(root) = self.world.objects.index_of_uuid(&target.root) else {
                    return;
                };
                let Some((pos, rot, _)) = Scene::object_transform(&self.world, root, now, 0) else {
                    return;
                };
                let Some((clicked, _, _)) = Scene::object_transform(&self.world, idx, now, 0) else {
                    return;
                };
                // Firestorm starts from the root center, not the surface hit,
                // so dragging a child does not make a linked object jump.
                let offset = rot.inverse() * (pos - clicked);
                let surface = ray.and_then(|r| self.scene.touch_surface(&self.world, idx, r)).unwrap_or_default();
                let physical = self
                    .world
                    .objects
                    .get(root)
                    .is_some_and(|o| o.parent_id == 0 && o.update_flags & 1 != 0 && o.update_flags & (1 << 8) != 0);
                self.interactions.held = Some(Held {
                    target,
                    offset,
                    position: pos,
                    surface,
                    cursor: self.cursor_pos,
                    last: now,
                    physical,
                });
                self.send(NetCommand::ObjectGrab {
                    handle: target.key.region,
                    local_id: target.key.local_id,
                    offset,
                    surface,
                });
            }
        }
        if self.demo {
            log::info!("demo click action: {:?}, object={}", target.action, target.key.local_id);
        }
    }

    pub(super) fn release_object_hold(&mut self) {
        if let Some(h) = self.interactions.held.take()
            && self
                .world
                .objects
                .index_of(&h.target.key)
                .and_then(|i| self.world.objects.get(i))
                .is_some_and(|o| o.full_id == h.target.object)
        {
            let surface = self
                .gfx
                .as_ref()
                .and_then(|g| g.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
                .map(|ray| {
                    self.world
                        .objects
                        .index_of(&h.target.key)
                        .and_then(|idx| self.scene.touch_surface(&self.world, idx, ray))
                        .unwrap_or_default()
                })
                .unwrap_or(h.surface);
            self.send(NetCommand::ObjectRelease {
                handle: h.target.key.region,
                local_id: h.target.key.local_id,
                surface,
            });
        }
    }

    pub(super) fn update_object_hold(&mut self, renderer: &Renderer) {
        if self.build.open || !self.left_down {
            self.release_object_hold();
            return;
        }
        let Some(h) = self.interactions.held.as_ref() else { return };
        let Some(idx) = self
            .world
            .objects
            .index_of(&h.target.key)
            .filter(|i| self.world.objects.get(*i).is_some_and(|o| o.full_id == h.target.object))
        else {
            self.interactions.held = None;
            return;
        };
        if h.last.elapsed() < Duration::from_millis(100) {
            return;
        }
        let surface = renderer
            .cursor_ray(self.cursor_pos.0, self.cursor_pos.1)
            .and_then(|ray| self.scene.touch_surface(&self.world, idx, ray))
            .unwrap_or_default();
        if self.cursor_pos == h.cursor && surface == h.surface {
            return;
        }
        let Some(h) = self.interactions.held.as_mut() else { return };
        {
            let forward = self.camera.forward();
            let left = Vec3::Z.cross(forward).normalize_or(Vec3::Y);
            let vertical = forward.cross(left).normalize_or(Vec3::Z);
            let y_axis = if self.ctrl {
                vertical
            } else {
                left.cross(Vec3::Z).normalize_or(Vec3::X)
            };
            // LLToolGrab's 0.0075 m per logical pixel, with screen Y down.
            let ppp = self.egui_ctx.pixels_per_point().max(0.1);
            let dx = (self.cursor_pos.0 - h.cursor.0) / ppp;
            let dy = (h.cursor.1 - self.cursor_pos.1) / ppp;
            h.position += (left * (-dx) + y_axis * dy) * 0.0075;
        }
        let Some(region) = self.world.region_offset(h.target.key.region) else {
            return;
        };
        let cmd = NetCommand::ObjectGrabUpdate {
            handle: h.target.key.region,
            object: h.target.object,
            offset: h.offset,
            position: h.position - region,
            elapsed_ms: h.last.elapsed().as_millis().min(u32::MAX as u128) as u32,
            surface,
        };
        h.cursor = self.cursor_pos;
        h.surface = surface;
        h.last = Instant::now();
        self.send(cmd);
    }
}
