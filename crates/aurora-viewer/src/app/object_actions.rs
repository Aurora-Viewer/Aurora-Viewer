//! Click actions and held touches, following LLToolPie / LLToolGrabBase
//! (Firestorm indra/newview, originally LGPL 2.1).
use super::*;
use crate::interaction::{self, Action, Contents, Held, Target};

impl App {
    pub(super) fn activate_object_action(&mut self, target: Target, idx: usize, point: Vec3, ray: Option<(Vec3, Vec3)>) {
        self.activate_object_action_at(target, idx, point, ray, None);
    }

    /// `shown`: the touched surface when it was read from the faces the GPU
    /// drew (a rigged mesh, which the CPU cannot trace: scene/drawn.rs).
    pub(super) fn activate_object_action_at(
        &mut self,
        target: Target,
        idx: usize,
        point: Vec3,
        ray: Option<(Vec3, Vec3)>,
        shown: Option<aurora_net::TouchSurface>,
    ) {
        let now = Instant::now();
        // FSBlockClickSit (LLToolPie::handleLeftClickPick): a sit click goes
        // on as a touch; the menu's Sit is not a click action and still sits
        let target = if target.action == Action::Sit && self.settings.block_click_sit {
            Target {
                action: Action::Touch,
                ..target
            }
        } else {
            target
        };
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
                let face = shown
                    .or_else(|| ray.and_then(|r| self.scene.touch_surface(&self.world, idx, r)))
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
                let offset = rot.inverse() * (pos - clicked) / self.world.hud_scale(idx);
                let surface = shown
                    .or_else(|| ray.and_then(|r| self.scene.touch_surface(&self.world, idx, r)))
                    .unwrap_or_default();
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
                    shown: shown.is_some(),
                    drawn: None,
                    asked: now,
                    latest: (surface, self.cursor_pos),
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
                .world
                .objects
                .index_of(&h.target.key)
                .filter(|_| !h.shown)
                .and_then(|idx| {
                    let ray = self
                        .gfx
                        .as_ref()
                        .and_then(|g| g.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1));
                    self.scene.object_cursor_ray(idx, self.cursor_pos, ray)
                })
                .map(|ray| {
                    self.world
                        .objects
                        .index_of(&h.target.key)
                        .and_then(|idx| self.scene.touch_surface(&self.world, idx, ray))
                        .unwrap_or_default()
                })
                .unwrap_or(if h.shown { h.latest.0 } else { h.surface });
            self.send(NetCommand::ObjectRelease {
                handle: h.target.key.region,
                local_id: h.target.key.local_id,
                surface,
            });
        }
    }

    pub(super) fn update_object_hold(&mut self, renderer: &mut Renderer) {
        if self.build.open || !self.left_down {
            if let Some((ticket, _)) = self.interactions.held.as_mut().and_then(|h| h.drawn.take()) {
                renderer.forget_drawn(ticket);
            }
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
            if let Some((ticket, _)) = h.drawn {
                renderer.forget_drawn(ticket);
            }
            self.interactions.held = None;
            return;
        };
        if h.shown {
            self.take_shown_answer(renderer, idx);
            self.send_object_hold(renderer, idx);
            self.ask_shown_surface(renderer);
        } else {
            self.send_object_hold(renderer, idx);
        }
    }

    /// One ObjectGrabUpdate, at most every 100 ms and when something
    /// changed (LLToolGrabBase::handleHoverNonPhysical).
    fn send_object_hold(&mut self, renderer: &Renderer, idx: usize) {
        let Some(h) = self.interactions.held.as_ref() else { return };
        if h.last.elapsed() < Duration::from_millis(100) {
            return;
        }
        let surface = if h.shown {
            // an answer for where the cursor is now is on its way: wait for
            // it rather than send an older surface with the new position
            if h.drawn.is_some() && h.latest.1 != self.cursor_pos {
                return;
            }
            h.latest.0
        } else {
            self.scene
                .object_cursor_ray(idx, self.cursor_pos, renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
                .and_then(|ray| self.scene.touch_surface(&self.world, idx, ray))
                .unwrap_or_default()
        };
        if self.cursor_pos == h.cursor && surface == h.surface {
            return;
        }
        let Some(h) = self.interactions.held.as_mut() else { return };
        if !self.scene.gpu.get(idx).is_some_and(|g| g.hud) {
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
            position: h.position
                - if self.scene.gpu.get(idx).is_some_and(|g| g.hud) {
                    Vec3::ZERO
                } else {
                    region
                },
            elapsed_ms: h.last.elapsed().as_millis().min(u32::MAX as u128) as u32,
            surface,
        };
        h.cursor = self.cursor_pos;
        h.surface = surface;
        h.last = Instant::now();
        self.send(cmd);
    }

    /// LLToolGrabBase::handleHoverNonPhysical: the touch follows the cursor
    /// on the held object at each frame (LLPickInfo::getSurfaceInfo
    /// intersects that object alone), so a script reading
    /// llDetectedTouchST / UV / Pos sees the touch move, and a still cursor sees the
    /// object move under it. On a surface the GPU alone poses, that is the
    /// faces drawn under the cursor, one request at a time: asked again
    /// when the cursor moved since the last one, and every 100 ms.
    fn ask_shown_surface(&mut self, renderer: &mut Renderer) {
        let cursor = self.cursor_pos;
        let Some(h) = self.interactions.held.as_mut() else { return };
        if h.drawn.is_none() && (h.latest.1 != cursor || h.asked.elapsed() >= Duration::from_millis(100)) {
            h.drawn = Some((
                renderer.request_drawn(cursor.0, cursor.1, aurora_render::DrawnMode::Interaction),
                cursor,
            ));
            h.asked = Instant::now();
        }
    }

    /// The answer to [`Self::ask_shown_surface`]: the held object drawn
    /// there gives its surface. Something else drawn in front of the held
    /// object's bounds hides it (a hand, hair, clothing going by): the
    /// Legacy Viewer still finds the object behind, so the touch keeps its
    /// last surface. Anything else is off the object (face -1).
    fn take_shown_answer(&mut self, renderer: &mut Renderer, idx: usize) {
        let Some(h) = self.interactions.held.as_mut() else { return };
        let Some((ticket, at)) = h.drawn else { return };
        let Some(drawn) = renderer.take_drawn(ticket) else { return };
        h.drawn = None;
        let on_held = [(drawn.blend, true), (drawn.opaque, false)].into_iter().find_map(|(hit, blend)| {
            let hit = hit?;
            let (i, face) = self.scene.record_object(hit.record)?;
            (i == idx).then_some(crate::scene::drawn::DrawnObject { idx, face, blend, hit })
        });
        let surface = match on_held {
            Some(d) => self.scene.drawn_surface(&self.world, &d).unwrap_or_default(),
            None => {
                let nearest = drawn.blend.or(drawn.opaque);
                let hidden = nearest
                    .zip(renderer.cursor_ray(at.0, at.1))
                    .zip(self.scene.gpu.get(idx))
                    .is_some_and(|((hit, (o, d)), g)| hidden_behind(o, d, hit.point, g.center, g.radius));
                if hidden { h.latest.0 } else { Default::default() }
            }
        };
        h.latest = (surface, at);
    }
}

/// Whether a surface drawn at `point` on the ray (`origin`, unit `dir`)
/// lies in front of a sphere the ray goes through: what is drawn there
/// hides the sphere's object rather than missing it.
fn hidden_behind(origin: Vec3, dir: Vec3, point: Vec3, center: Vec3, radius: f32) -> bool {
    let along = (center - origin).dot(dir);
    let off_ray = (center - origin - dir * along).length();
    radius > 0.0 && off_ray <= radius && (point - origin).dot(dir) < along - radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_in_front_of_the_held_object_hides_it() {
        let (o, d) = (Vec3::ZERO, Vec3::X);
        let (center, radius) = (Vec3::new(3.0, 0.0, 0.0), 0.2);
        // a hand going by in front
        assert!(hidden_behind(o, d, Vec3::new(2.0, 0.0, 0.0), center, radius));
        // the body behind the attachment: the cursor slid off it
        assert!(!hidden_behind(o, d, Vec3::new(3.3, 0.0, 0.0), center, radius));
        // in front, but the ray passes beside the object
        assert!(!hidden_behind(o, d, Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 0.5, 0.0), radius));
    }
}
