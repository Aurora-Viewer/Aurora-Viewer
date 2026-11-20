//! Click actions and held touches, following Legacy Viewer LLToolPie / LLToolGrabBase
//! (indra/newview, originally LGPL 2.1).
use super::*;
use crate::interaction::{self, Action, Contents, GrabMotion, Held, Target};

/// A held touch on a surface the GPU alone poses asks for the drawn faces
/// once per `ASK_EVERY`, `ASK_LEAD` before the next update falls due:
/// about two frames at 60 Hz, so the answer usually lands as the update
/// falls due, and the surface sent is at most that old.
const ASK_LEAD: Duration = Duration::from_millis(34);
const ASK_EVERY: Duration = Duration::from_millis(100);

/// handle_object_sit uses the clicked prim's LLPickInfo::mObjectOffset,
/// computed by LLAgentCamera::calcFocusOffset (indra/newview). Keep the
/// clicked child origin and world axes; neither a root-relative offset nor
/// an inverse object rotation belongs in AgentRequestSit.
pub(super) fn object_sit_offset(world: &World, camera: &Camera, idx: usize, point: Vec3, ray: Option<(Vec3, Vec3)>) -> Option<Vec3> {
    let object = world.objects.get(idx)?;
    let geometry = crate::camera::focus_object_geom(world, &object.full_id, Instant::now())?;
    Some(camera.object_pick_offset(&geometry, point, ray))
}

/// What RLVa leaves of the one-click action on the prim `idx` when it is
/// clicked: LLToolPie::handleLeftClickPick (indra/newview/lltoolpie.cpp).
/// `point` is the point under the cursor, `sit_offset` the offset a sit
/// there would send.
/// - no action at all on an object out of reach (RlvActions::canInteract,
///   asked of every click action);
/// - Sit only where RlvActions::canSit allows it;
/// - under @buy, Buy and Pay become the touch (or the grab) the object
///   would take without its click action, or nothing;
/// - the touch and the grab, and they alone, ask RlvActions::canTouch: a
///   click action other than Touch (Sit, Open, Play, Zoom…) is made
///   whatever @touchworld, @touchall or @touchthis hold, although the hover
///   shows the plain cursor on it (`App::hover_cursor_action`).
///
/// A menu's entry is not a click action: its own facts grey it out.
pub(super) fn rlva_click_target(
    world: &World,
    target: Target,
    idx: usize,
    point: Option<Vec3>,
    sit_offset: Option<Vec3>,
) -> Option<Target> {
    if !world.rlva.enabled() || target.menu {
        return Some(target);
    }
    if !world.rlva_can_interact_at(idx, point) {
        return None;
    }
    let target = match target.action {
        Action::Sit => {
            return world
                .rlva_allows_command(
                    &NetCommand::RequestSit {
                        handle: target.key.region,
                        target: target.object,
                        offset: sit_offset?,
                    },
                    uuid::Uuid::nil(),
                )
                .then_some(target);
        }
        Action::Buy | Action::Pay if world.rlva.has("buy") => {
            let clicked = world.objects.get(idx)?;
            let root = interaction::root(world, idx).and_then(|(r, _)| world.objects.get(r))?;
            // as `interaction::target` without a click action: physics
            // first, then a touch script on the prim or its root
            let flags = clicked.update_flags | root.update_flags;
            let action = if flags & 1 != 0 {
                Action::Grab
            } else if flags & crate::ui::context::flags::HANDLE_TOUCH != 0 {
                Action::Touch
            } else {
                return None;
            };
            Target {
                action,
                key: target.clicked,
                object: target.clicked_id,
                ..target
            }
        }
        _ => target,
    };
    // (what our avatar wears and @showself or @showselfhead hide is not
    // offered to the click)
    if matches!(target.action, Action::Touch | Action::Grab)
        && (!world.rlva_object_visible(idx) || !world.rlva_can_touch_at(target.clicked.region, target.clicked.local_id, point))
    {
        return None;
    }
    Some(target)
}

/// The cursor's half of `App::hover_cursor_action`, RLVa on.
fn rlva_hover_action(world: &World, target: Target, idx: usize, point: Option<Vec3>, sit_offset: Option<Vec3>) -> Option<Action> {
    if !world.rlva_object_visible(idx) || !world.rlva_can_touch_at(target.clicked.region, target.clicked.local_id, point) {
        return None;
    }
    let left = rlva_click_target(world, target, idx, point, sit_offset)?;
    if matches!(target.action, Action::Buy | Action::Pay) && left.action == Action::Grab {
        return None;
    }
    Some(left.action)
}

impl App {
    /// The one-click action the cursor shows on the prim `idx` hovered at
    /// `point`: `interaction::cursor_action`, less what RLVa refuses, so
    /// that the cursor never promises a click the viewer will not make.
    ///
    /// LLToolPie::handleHover: the plain cursor on what RlvActions::canTouch
    /// refuses, whatever its click action (the click itself may still act,
    /// `rlva_click_target`); cursorFromObject under @buy shows the hand on a
    /// prim with a touch script and the plain cursor otherwise.
    pub(super) fn hover_cursor_action(&self, idx: usize, point: Option<Vec3>, ray: Option<(Vec3, Vec3)>) -> Option<Action> {
        let action = interaction::cursor_action(&self.world, idx, &self.interactions.props)?;
        if !self.world.rlva.enabled() {
            return Some(action);
        }
        let target = interaction::target(&self.world, idx, &self.interactions.props)?;
        let sit = (action == Action::Sit)
            .then(|| point.and_then(|p| object_sit_offset(&self.world, &self.camera, idx, p, ray)))
            .flatten();
        rlva_hover_action(&self.world, Target { action, ..target }, idx, point, sit)
    }

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
        let sit_offset = (target.action == Action::Sit)
            .then(|| object_sit_offset(&self.world, &self.camera, idx, point, ray))
            .flatten();
        // a click RLVa refuses is taken and does nothing, without a notice
        // (LLToolPie::handleLeftClickPick): the cursor did not offer it
        let Some(target) = rlva_click_target(&self.world, target, idx, Some(point), sit_offset) else {
            return;
        };
        match target.action {
            Action::Disabled => {}
            Action::Sit => {
                if let Some(offset) = sit_offset {
                    self.send(NetCommand::RequestSit {
                        handle: target.key.region,
                        target: target.object,
                        offset,
                    });
                }
            }
            Action::Buy | Action::Pay => {
                // enable_buy_object and enable_pay_object (indra/newview/
                // llviewermenu.cpp, RlvActions::canBuyObject / canPayObject):
                // the menu's entry is greyed out under @buy, and whatever
                // still asks for it opens no window
                if self.world.rlva.has("buy") {
                    return;
                }
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
                    .or_else(|| ray.and_then(|r| self.scene.left_touch_surface(&self.world, idx, r)))
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
                    .or_else(|| ray.and_then(|r| self.scene.left_touch_surface(&self.world, idx, r)))
                    .unwrap_or_default();
                let physical = self
                    .world
                    .objects
                    .get(root)
                    .is_some_and(|o| o.parent_id == 0 && o.update_flags & 1 != 0 && o.update_flags & (1 << 8) != 0);
                // A HUD's motion starts from its root in the HUD frame, where
                // the scene poses it: LLViewerObject::getPositionGlobal
                // (indra/newview/llviewerobject.cpp) adds the agent's region
                // origin to a HUD's render position, and the region frame of
                // the grab update takes that same origin away again
                // (LLViewerRegion::getPosRegionFromGlobal), so the wearer's
                // position is never part of a HUD's GrabPosition.
                let start = pos;
                let grab = NetCommand::ObjectGrab {
                    handle: target.key.region,
                    local_id: target.key.local_id,
                    offset,
                    surface,
                };
                // the send boundary's own answer, asked before the hold
                // exists: a refused touch leaves no held object to follow
                // nor to release, and prints nothing (LLToolPie::
                // handleLeftClickPick does not start the grab)
                if !self.world.rlva_allows_command(&grab, uuid::Uuid::nil()) {
                    return;
                }
                self.interactions.held = Some(Held {
                    target,
                    offset,
                    position: start,
                    grab_motion: (!physical).then(|| GrabMotion::new(start, self.camera.position, self.cursor_pos)),
                    surface,
                    shown: shown.is_some(),
                    drawn: None,
                    // one lead before the press, so the first request
                    // leaves in time for the first update
                    asked: now.checked_sub(ASK_LEAD).unwrap_or(now),
                    latest: (surface, self.cursor_pos),
                    cursor: self.cursor_pos,
                    last: now,
                    physical,
                });
                self.send(grab);
            }
        }
        if self.demo {
            log::info!("demo click action: {:?}, object={}", target.action, target.key.local_id);
        }
    }

    pub(super) fn release_object_hold(&mut self) {
        // outside the frame (mouse up, menus) the renderer is ours: forget
        // a drawn-face request still on its way; in the frame,
        // update_object_hold forgets it before calling this
        if let Some((ticket, _)) = self.interactions.held.as_mut().and_then(|h| h.drawn.take())
            && let Some(gfx) = self.gfx.as_mut()
        {
            gfx.renderer.forget_drawn(ticket);
        }
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
                        .and_then(|idx| self.scene.left_touch_surface(&self.world, idx, ray))
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
                .and_then(|ray| self.scene.left_touch_surface(&self.world, idx, ray))
                .unwrap_or_default()
        };
        let Some(h) = self.interactions.held.as_mut() else { return };
        let hud = self.scene.gpu.get(idx).is_some_and(|g| g.hud);
        let previous_position = h.position;
        // Vertical dragging is Ctrl alone: `mask == MASK_VERTICAL`
        // (MASK_CONTROL, indra/newview/lltoolmgr.h) in
        // LLToolGrabBase::handleHoverNonPhysical and handleHoverActive, so
        // Ctrl+Shift (MASK_SPIN) or Ctrl+Alt does not drag vertically.
        let vertical = self.ctrl && !self.shift && !self.alt;
        if let Some(motion) = h.grab_motion.as_mut() {
            h.position = motion.update(
                self.cursor_pos,
                self.camera.position,
                self.camera.forward(),
                vertical,
                self.egui_ctx.pixels_per_point(),
            );
        } else if !hud {
            let forward = self.camera.forward();
            let left = Vec3::Z.cross(forward).normalize_or(Vec3::Y);
            let up = forward.cross(left).normalize_or(Vec3::Z);
            let y_axis = if vertical { up } else { left.cross(Vec3::Z).normalize_or(Vec3::X) };
            // LLToolGrab's 0.0075 m per logical pixel, with screen Y down.
            let ppp = self.egui_ctx.pixels_per_point().max(0.1);
            let dx = (self.cursor_pos.0 - h.cursor.0) / ppp;
            let dy = (h.cursor.1 - self.cursor_pos.1) / ppp;
            h.position += (left * (-dx) + y_axis * dy) * 0.0075;
        }
        if self.cursor_pos == h.cursor && surface == h.surface && h.position == previous_position {
            return;
        }
        let Some(region) = self.world.region_offset(h.target.key.region) else {
            return;
        };
        let cmd = grab_update_command(h, region, hud, surface, h.last.elapsed().as_millis().min(u32::MAX as u128) as u32);
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
    /// faces drawn under the cursor, asked at the pace of the updates: once
    /// per 100 ms, when the next update is nearly due, so the answer (one
    /// to three frames later) is there when it can be sent. Each request is
    /// one more pass over the visible geometry.
    fn ask_shown_surface(&mut self, renderer: &mut Renderer) {
        let cursor = self.cursor_pos;
        let Some(h) = self.interactions.held.as_mut() else { return };
        if h.drawn.is_none() && h.asked.elapsed() >= ASK_EVERY && h.last.elapsed() + ASK_LEAD >= ASK_EVERY {
            // the press's own pick rule (DrawnMode::Left, which skips fully
            // transparent texels of world Blend faces): the hover reads the
            // surface with the press's pick settings (LLToolGrabBase::
            // handleHoverNonPhysical, `LLPickInfo pick = mGrabPick`)
            h.drawn = Some((renderer.request_drawn(cursor.0, cursor.1, aurora_render::DrawnMode::Left), cursor));
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

fn grab_update_command(held: &Held, region: Vec3, hud: bool, surface: aurora_net::TouchSurface, elapsed_ms: u32) -> NetCommand {
    NetCommand::ObjectGrabUpdate {
        handle: held.target.key.region,
        object: held.target.object,
        offset: held.offset,
        // A HUD's position is in the HUD frame, which the region frame
        // leaves as it is (see the start of the grab); a world object's is
        // sent in its region's frame.
        position: if hud { held.position } else { held.position - region },
        elapsed_ms,
        surface,
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
    fn virtual_grab_moves_hud_and_world_points_in_camera_frame() {
        let camera = Vec3::new(100.0, 200.0, 30.0);
        let start = Vec3::new(104.0, 198.0, 31.0);
        let mut grab = GrabMotion::new(start, camera, (10.0, 10.0));
        let forward = Vec3::X;
        let moved = grab.update((20.0, 10.0), camera, forward, false, 1.0);
        assert!(moved.distance(start) > 0.07);
        assert!((moved - start).z.abs() < 1e-6);

        // A stationary cursor still follows the camera, as the Legacy
        // Viewer computes camera position + the hidden camera-relative offset.
        let translated = grab.update((20.0, 10.0), camera + Vec3::Y, forward, false, 1.0);
        assert!(translated.distance(moved) > 0.99);
    }

    #[test]
    fn virtual_grab_vertical_axis_and_ui_scale_are_respected() {
        let mut horizontal = GrabMotion::new(Vec3::ZERO, Vec3::ZERO, (0.0, 0.0));
        let mut vertical = horizontal;
        let forward = Vec3::X;
        let h = horizontal.update((0.0, -10.0), Vec3::ZERO, forward, false, 2.0);
        let v = vertical.update((0.0, -10.0), Vec3::ZERO, forward, true, 2.0);
        assert!(h.distance(v) > 0.05);
        assert!(h.is_finite() && v.is_finite());
    }

    fn held_with(position: Vec3, grab_motion: Option<GrabMotion>) -> Held {
        let key = crate::world::objects::ObjKey { region: 7, local_id: 2 };
        let id = uuid::Uuid::nil();
        let target = Target {
            action: Action::Touch,
            clicked: key,
            clicked_id: id,
            key,
            object: id,
            root: id,
            menu: false,
        };
        Held {
            target,
            offset: Vec3::ZERO,
            position,
            grab_motion,
            surface: aurora_net::TouchSurface::default(),
            shown: false,
            drawn: None,
            asked: std::time::Instant::now(),
            latest: (aurora_net::TouchSurface::default(), (12.0, 0.0)),
            cursor: (12.0, 0.0),
            last: std::time::Instant::now(),
            physical: false,
        }
    }

    fn sent_position(held: &Held, region: Vec3, hud: bool) -> Vec3 {
        let NetCommand::ObjectGrabUpdate { position, .. } = grab_update_command(held, region, hud, held.surface, 100) else {
            panic!("expected grab update")
        };
        position
    }

    #[test]
    fn hud_grab_motion_is_sent_in_the_hud_frame() {
        // A HUD's root sits in the HUD frame (a fraction of a metre from the
        // screen joint), while the camera and the region are far away in the
        // world: the drag moves the point, and nothing of the wearer's or the
        // region's position gets into GrabPosition.
        let camera = Vec3::new(128.0, 128.0, 27.0);
        let start = Vec3::new(0.0, 0.25, -0.1);
        let mut motion = GrabMotion::new(start, camera, (0.0, 0.0));
        let moved = motion.update((12.0, 0.0), camera, Vec3::X, false, 1.0);
        let position = sent_position(&held_with(moved, Some(motion)), Vec3::new(256.0, 512.0, 0.0), true);
        assert_eq!(position, moved);
        assert_ne!(position, start);
        assert!(position.length() < 1.0, "a HUD grab point stays near the HUD: {position}");
    }

    #[test]
    fn world_grab_motion_is_sent_in_the_region_frame() {
        let camera = Vec3::new(100.0, 200.0, 30.0);
        let start = Vec3::new(104.0, 198.0, 31.0);
        let mut motion = GrabMotion::new(start, camera, (0.0, 0.0));
        let moved = motion.update((12.0, 0.0), camera, Vec3::X, false, 1.0);
        let region = Vec3::new(100.0, 190.0, 20.0);
        let position = sent_position(&held_with(moved, Some(motion)), region, false);
        assert_eq!(position, moved - region);
        assert_ne!(position, start - region);
    }

    #[test]
    fn sit_offsets_keep_world_axes_and_the_clicked_child_origin() {
        use glam::Quat;
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let handle = world.main_region.expect("demo region");
        let origin = Vec3::new(100.0, 120.0, 30.0);
        for linked in [false, true] {
            for (turn, degrees) in [0.0_f32, 90.0, 180.0, 237.0].into_iter().enumerate() {
                let rotation = Quat::from_rotation_z(degrees.to_radians());
                // Distinct objects avoid interpolating one case's rotation
                // into the next case while testing a stationary seat.
                let local_id = 31000 + (u32::from(linked) * 4 + turn as u32) * 2;
                let mut root = crate::demo::action_avatar(false);
                root.local_id = local_id;
                root.full_id = crate::demo::action_id(local_id);
                root.pcode = aurora_prim::params::LL_PCODE_VOLUME;
                root.parent_id = 0;
                root.position = origin;
                root.rotation = rotation;
                root.scale = Vec3::new(4.0, 1.0, 0.5);
                root.volume.sculpt = Some(aurora_prim::params::SculptParams {
                    texture: uuid::Uuid::from_u128(31000),
                    sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
                });
                let mut seat = root.clone();
                seat.local_id = local_id + 1;
                seat.full_id = crate::demo::action_id(local_id + 1);
                if linked {
                    root.rotation = rotation * Quat::from_rotation_z(-0.4);
                    root.position = origin - root.rotation * Vec3::new(3.0, -2.0, 1.0);
                    seat.parent_id = root.local_id;
                    seat.position = Vec3::new(3.0, -2.0, 1.0);
                    seat.rotation = Quat::from_rotation_z(0.4);
                }
                world.apply(aurora_net::NetEvent::ObjectUpdates {
                    handle,
                    objects: vec![root, seat],
                });
                let idx = world.objects.index_of_uuid(&crate::demo::action_id(local_id + 1)).expect("seat");
                let mut camera = Camera::default();
                camera.position = origin + rotation * Vec3::new(0.0, -7.0, 4.0);
                camera.target = origin;
                for side in [-1.0, 1.0] {
                    let point = origin + rotation * Vec3::new(side, -0.25, 0.25);
                    let ray = (camera.position, (point - camera.position).normalize());
                    let offset = object_sit_offset(&world, &camera, idx, point, Some(ray)).expect("offset");
                    // calcFocusOffset intersects the cursor ray with the
                    // seat's central XY plane at t=4/3.75; at this distance
                    // its near-surface bias is zero. No scale division.
                    let expected = rotation * Vec3::new(side * 16.0 / 15.0, 0.2, 0.0);
                    assert!(
                        offset.abs_diff_eq(expected, 1e-4),
                        "linked={linked}, turn={degrees}, side={side}: {offset:?}"
                    );
                    let fallback = object_sit_offset(&world, &camera, idx, point, None).expect("offset without ray");
                    assert!(fallback.abs_diff_eq(point - origin, 1e-4));
                }
            }
        }
    }

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

    const HANDLE_TOUCH: u32 = crate::ui::context::flags::HANDLE_TOUCH;
    const TAKES_MONEY: u32 = 1 << 9;
    const USE_PHYSICS: u32 = 1;

    /// A prim of the world 1 m from the agent, with RLVa on.
    fn rlva_prim(click_action: u8, flags: u32) -> (World, usize) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let handle = world.main_region.expect("demo region");
        let mut prim = crate::demo::action_avatar(false);
        prim.local_id = 33000;
        prim.full_id = crate::demo::action_id(33000);
        prim.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        prim.parent_id = 0;
        prim.position = Vec3::new(100.0, 120.0, 30.0);
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle,
            objects: vec![prim],
        });
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(33000)).expect("prim");
        let o = world.objects.get_mut(idx).expect("prim");
        o.click_action = click_action;
        o.update_flags = flags;
        world.agent.position = Vec3::new(100.0, 121.0, 30.0);
        world.rlva.configure(true);
        (world, idx)
    }

    fn restrict(world: &mut World, text: &str) {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Test object".to_owned(),
            source_id: uuid::Uuid::from_u128(1),
            owner_id: uuid::Uuid::from_u128(2),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: Vec3::ZERO,
            message: text.to_owned(),
        }));
    }

    /// The action left on the prim hovered or clicked at `point`.
    fn left(world: &World, idx: usize, point: Vec3) -> Option<Action> {
        let target = interaction::target(world, idx, &std::collections::HashMap::new()).expect("a click action");
        rlva_click_target(world, target, idx, Some(point), Some(Vec3::ZERO)).map(|t| t.action)
    }

    /// The action the cursor shows on the prim hovered at `point`.
    fn shown(world: &World, idx: usize, point: Vec3) -> Option<Action> {
        let target = interaction::target(world, idx, &std::collections::HashMap::new()).expect("a click action");
        rlva_hover_action(world, target, idx, Some(point), Some(Vec3::ZERO))
    }

    /// LLToolPie::handleLeftClickPick: RlvActions::canInteract (@interact,
    /// @fartouch) blankets every click action; RlvActions::canTouch is asked
    /// of the touch and the grab alone, so a Sit, Open or Zoom click action
    /// still acts under @touchall, @touchworld or @touchthis. The hover
    /// (LLToolPie::handleHover) shows the plain cursor on all of them.
    #[test]
    fn rlva_touch_restrictions_leave_the_other_click_actions() {
        let near = Vec3::new(100.0, 120.5, 30.0);
        let others = [
            (interaction::code::SIT, Action::Sit),
            (interaction::code::OPEN, Action::Open),
            (interaction::code::ZOOM, Action::Zoom),
        ];
        let this = format!("@touchthis:{}=n", crate::demo::action_id(33000));
        for restriction in ["@touchall=n", "@touchworld=n", this.as_str()] {
            let (mut world, idx) = rlva_prim(interaction::code::TOUCH, HANDLE_TOUCH);
            assert_eq!(left(&world, idx, near), Some(Action::Touch));
            restrict(&mut world, restriction);
            assert_eq!(left(&world, idx, near), None, "{restriction}");
            assert_eq!(shown(&world, idx, near), None, "{restriction}");
            for (code, action) in others {
                world.objects.get_mut(idx).expect("prim").click_action = code;
                assert_eq!(left(&world, idx, near), Some(action), "{restriction}, click action {code}");
                assert_eq!(shown(&world, idx, near), None, "{restriction}, cursor on click action {code}");
            }
        }
        for restriction in ["@interact=n", "@fartouch:0.2=n"] {
            let (mut world, idx) = rlva_prim(interaction::code::TOUCH, HANDLE_TOUCH);
            restrict(&mut world, restriction);
            assert_eq!(left(&world, idx, near), None, "{restriction}");
            for (code, _) in others {
                world.objects.get_mut(idx).expect("prim").click_action = code;
                assert_eq!(left(&world, idx, near), None, "{restriction}, click action {code}");
                assert_eq!(shown(&world, idx, near), None, "{restriction}, cursor on click action {code}");
            }
        }
    }

    #[test]
    fn rlva_fartouch_follows_the_point_under_the_cursor() {
        let (mut world, idx) = rlva_prim(interaction::code::TOUCH, HANDLE_TOUCH);
        restrict(&mut world, "@fartouch:0.8=n");
        // the prim's origin is 1 m away, this face of it 0.5 m
        assert_eq!(left(&world, idx, Vec3::new(100.0, 120.5, 30.0)), Some(Action::Touch));
        assert_eq!(left(&world, idx, Vec3::new(100.0, 119.5, 30.0)), None);
    }

    #[test]
    fn rlva_sit_click_actions_follow_the_sit_rule() {
        let (mut world, idx) = rlva_prim(interaction::code::SIT, 0);
        let near = Vec3::new(100.0, 120.5, 30.0);
        assert_eq!(left(&world, idx, near), Some(Action::Sit));
        restrict(&mut world, "@sit=n");
        assert_eq!(left(&world, idx, near), None);
        // no offset to measure: nothing is offered
        let (world, idx) = rlva_prim(interaction::code::SIT, 0);
        let target = interaction::target(&world, idx, &std::collections::HashMap::new()).expect("sit");
        assert!(rlva_click_target(&world, target, idx, Some(near), None).is_none());
    }

    #[test]
    fn rlva_buy_turns_buy_and_pay_click_actions_into_the_objects_touch() {
        let near = Vec3::new(100.0, 120.5, 30.0);
        for (code, action, flags) in [
            (interaction::code::BUY, Action::Buy, 0),
            (interaction::code::PAY, Action::Pay, TAKES_MONEY),
        ] {
            // without a touch script: nothing
            let (mut world, idx) = rlva_prim(code, flags);
            assert_eq!(left(&world, idx, near), Some(action));
            restrict(&mut world, "@buy=n");
            assert_eq!(left(&world, idx, near), None, "{action:?}");
            // with one: the touch, on the clicked prim
            world.objects.get_mut(idx).expect("prim").update_flags = flags | HANDLE_TOUCH;
            let target = interaction::target(&world, idx, &std::collections::HashMap::new()).expect("a click action");
            let touch = rlva_click_target(&world, target, idx, Some(near), None).expect("the touch is left");
            assert_eq!(touch.action, Action::Touch, "{action:?}");
            assert_eq!((touch.key.local_id, touch.object), (33000, crate::demo::action_id(33000)));
            // a physical object: the grab
            world.objects.get_mut(idx).expect("prim").update_flags = flags | USE_PHYSICS;
            assert_eq!(left(&world, idx, near), Some(Action::Grab), "{action:?}");
            // cursorFromObject: the hand needs a touch script, a physical
            // object shows the plain cursor
            assert_eq!(shown(&world, idx, near), None, "{action:?}");
            world.objects.get_mut(idx).expect("prim").update_flags = flags | HANDLE_TOUCH;
            assert_eq!(shown(&world, idx, near), Some(Action::Touch), "{action:?}");
            world.objects.get_mut(idx).expect("prim").update_flags = flags | USE_PHYSICS;
            // a menu's Pay or Buy is not a click action: its facts grey it out
            let menu = interaction::menu_target(&world, idx, action).expect("menu target");
            assert_eq!(rlva_click_target(&world, menu, idx, None, None).map(|t| t.action), Some(action));
        }
    }

    #[test]
    fn rlva_off_leaves_every_click_action() {
        let (mut world, idx) = rlva_prim(interaction::code::BUY, 0);
        restrict(&mut world, "@buy=n,touchall=n,interact=n");
        world.rlva.configure(false);
        assert_eq!(left(&world, idx, Vec3::splat(4000.0)), Some(Action::Buy));
    }
}
