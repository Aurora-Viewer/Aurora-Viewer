//! What a click designates from the faces the GPU drew under its pixel.
//!
//! The searches of picking.rs work on triangles the scene keeps on the CPU.
//! A rigged mesh is not where those are: the GPU poses it. So a click also
//! asks the renderer which draw records it shows under the pixel
//! (`Renderer::request_drawn`, aurora-render/src/drawn_pick.rs), and the
//! objects the CPU cannot find where they are shown are designated from
//! that answer: animated objects (animesh), rigged meshes worn by an
//! avatar, and worn meshes in general. A left click also takes a worn prim
//! with a touch script or a click action from it: the CPU places a worn
//! prim at the pose of the frame the answer comes in, a few frames after
//! the click, and a small clicker on an animated avatar is off the ray by
//! then.
//!
//! **Wanted difference from Firestorm.** There, a rigged mesh is only
//! reached when `pick_rigged` is set: the right click
//! (LLToolPie::handleRightMouseDown) and the build tool that moves
//! (LLToolCompTranslate::handleMouseDown), plus our own rigged attachments
//! while the build floater is open (LLVOVolume::lineSegmentIntersect:
//! `pick_rigged || (getAvatar()->isSelf() && LLFloater::isVisible(gFloaterTools))`).
//! The left click and the hover cursor pass `pick_rigged = false`
//! (LLToolPie::handleMouseDown / handleHover; the setting
//! AnimatedObjectsAllowLeftClick is commented out): an animesh or a rigged
//! attachment cannot be touched, sat on or bought by a click, the click
//! goes to the avatar's collision volumes or to what is behind. Firestorm
//! skins the mesh again on the CPU for each pick, which is why it is left
//! out. Aurora reads what is drawn, so the menu and the build tools reach a
//! rigged mesh where it is shown. Once designated it follows the rules of
//! any object of its kind: click action, IGNORE, transparency, rights,
//! menus.
//!
//! **The left click passes through a rigged attachment, as in the Legacy
//! Viewer**, to what the click finds under it: content made for that viewer
//! counts on it, like the scripted prims worn inside a scripted mesh body,
//! which the body would otherwise keep from their touch. Three preferences
//! say which rigged meshes with a touch script or a click action the left
//! click and the hover cursor act on ([`RiggedClicks`]; Preferences › Mouse
//! and keyboard, "Left click on rigged objects"): animated objects, rezzed
//! or worn (on), the meshes skinned to our avatar (off) and those skinned
//! to another avatar (off). **Wanted difference from the Legacy Viewer**:
//! a scripted animesh takes the left click by default, at the human's
//! request; there no rigged mesh ever does. Only the plain left click
//! follows the preferences: with Alt, Ctrl or Shift held, a click reaches
//! what it reached before them ([`RiggedClicks::MODIFIER_CLICK`], at the
//! human's request). Whatever they say, a rigged mesh with nothing to do
//! with the click lets it through (`touch_passes_through`, `touch_depth`):
//! a press on our mesh body steers our avatar.
//!
//! **Wanted difference from the Legacy Viewer, inventory drops.** There,
//! LLToolDragAndDrop::pick treats any attachment outside the HUD as its
//! avatar, where a texture does nothing (dad3dNULL). Here a texture or a
//! material dragged over the world lands on the face shown of what we wear
//! and may edit, with the permission rules of the Texture tab
//! (`Gesture::Drop`, app/texture_library.rs). Ctrl+Shift covers its whole
//! linkset, where dropTextureAllFaces stops at the prim hit.
//!
//! Camera clicks (Alt + click, Focus tool) use the visible-surface mode of
//! that pass directly: no CPU rest-pose or invisible touch-surface search.
//!
//! [`reaches`] is that rule, alone and tested. Worn Blend faces intercept a
//! press regardless of texel opacity; world Blend left picks skip alpha <= 32/255.

use super::*;
use aurora_net::TouchSurface;
use aurora_render::{Drawn, DrawnFace};

/// What the pointer is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Left click (LLToolPie).
    Touch,
    /// Right click: the menu of what is shown.
    Menu,
    /// Build tools: selection.
    Build,
    /// A texture or a material dragged over the world, on what is worn.
    Drop,
}

/// The skeleton a rigged mesh is skinned to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rigged {
    /// Its own: an animated object (animesh), rezzed or worn.
    Animesh,
    /// Our avatar's: our mesh body, rigged clothing or hair.
    Own,
    /// Another avatar's.
    Others,
}

/// The rigged meshes with a touch script or a click action that a left
/// click and the hover cursor act on (Preferences › Mouse and keyboard,
/// "Left click on rigged objects"). A kind that is off lets the click
/// through, as LLToolPie::handleMouseDown and handleHover do for every
/// rigged mesh with `pick_rigged = false` (indra/newview/lltoolpie.cpp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiggedClicks {
    pub animesh: bool,
    pub own: bool,
    pub others: bool,
}

impl Default for RiggedClicks {
    /// Animesh alone, the human's choice: the Legacy Viewer has none of the
    /// three.
    fn default() -> Self {
        Self {
            animesh: true,
            own: false,
            others: false,
        }
    }
}

impl RiggedClicks {
    /// Every kind (`clickable` of AURORA_DEMO_CLICKS).
    pub const ALL: Self = Self {
        animesh: true,
        own: true,
        others: true,
    };

    /// A click with Alt, Ctrl or Shift held, which the preferences do not
    /// speak for: what it reached before them, every scripted rigged mesh
    /// but the ones skinned to our avatar.
    pub const MODIFIER_CLICK: Self = Self {
        animesh: true,
        own: false,
        others: true,
    };

    fn allows(self, kind: Rigged) -> bool {
        match kind {
            Rigged::Animesh => self.animesh,
            Rigged::Own => self.own,
            Rigged::Others => self.others,
        }
    }
}

/// Whether a left click on the drawn face of an object goes on to what is
/// behind it ([`Scene::touch_passes_through`]).
pub fn passes_through(s: Status, clicks: RiggedClicks) -> bool {
    let kept_out = s.rigged.is_some_and(|kind| !clicks.allows(kind));
    s.posed_by_gpu && !s.avatar && (!s.actionable || kept_out)
}

/// A surface the CPU finds this far (m) in front of a drawn face keeps the
/// click: under it, the two are the same surface, a frame apart.
const SAME: f32 = 0.05;

/// What a gesture needs to know of the object of a drawn face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Skinned to a skeleton, and whose (HUDs and avatars aside).
    pub rigged: Option<Rigged>,
    /// The CPU searches cannot find it where it is shown: rigged (animesh,
    /// rigged attachment) or a mesh worn by an avatar.
    pub posed_by_gpu: bool,
    /// Worn by an avatar.
    pub worn: bool,
    /// A touch script or a click action (`interaction::target`).
    pub actionable: bool,
    pub avatar: bool,
    /// CLICK_ACTION_IGNORE.
    pub ignored: bool,
    /// The build tools may select it.
    pub selectable: bool,
}

/// Whether `gesture` designates the object of a face drawn under it rather
/// than what the CPU search finds. Material cutouts are handled by the pick
/// pass itself; Blend intercepts presses even without an action.
pub fn reaches(gesture: Gesture, s: Status, build_open: bool) -> bool {
    // LLVOVolume::lineSegmentIntersect: IGNORE is not picked outside the
    // build tools, rigged or not
    if s.ignored && !build_open {
        return false;
    }
    match gesture {
        // a worn prim with something to do with the click as well: the CPU
        // places it a frame or more after the one clicked, and a small
        // clicker on an animated avatar moves out of the click's reach
        Gesture::Touch => !s.avatar && (s.posed_by_gpu || (s.worn && s.actionable)),
        // the menu of what is shown (pick_transparent, pick_rigged): the
        // attachment in front of an avatar rather than the avatar
        Gesture::Menu => s.posed_by_gpu || s.worn || s.avatar,
        Gesture::Build => s.posed_by_gpu && s.selectable,
        // `selectable` carries RLVa's edit restrictions
        Gesture::Drop => s.worn && s.selectable,
    }
}

/// A drawn face and the object it belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnObject {
    pub idx: usize,
    /// Face of the object (texture entry index), when its geometry tells.
    pub face: Option<usize>,
    pub blend: bool,
    pub hit: DrawnFace,
}

impl Scene {
    /// Cursor feedback follows the same surface and designation as a left
    /// press, including the bounded search under a wearer's body. The
    /// caller caches this asynchronous answer; HUD picking stays separate.
    pub fn drawn_hover(
        &self,
        world: &World,
        drawn: &Drawn,
        ray: (Vec3, Vec3),
        depth: Option<Vec3>,
        far: f32,
    ) -> (Option<Vec3>, Option<usize>) {
        let adjusted = self.touch_depth(world, drawn, Some(ray), depth, far);
        let point = self.action_point(world, Some(ray), adjusted, false, far);
        if let Some(shown) = self.designated(world, Gesture::Touch, drawn, Some(ray), point, depth, false) {
            return (Some(shown.hit.point), Some(shown.idx));
        }
        (point, point.and_then(|p| self.interaction_at_ray(world, p, ray, true)))
    }

    /// [`Self::drawn_hover`] between two answers of the GPU, for a pointer
    /// that moved since the last one was asked. An answer speaks for its
    /// pixel alone, and the next one comes [`super::hover::REFRESH`] later:
    /// until then the cursor showed what the CPU finds, nothing under a
    /// rigged mesh, and flickered over a clicker worn under a mesh body
    /// whenever the pointer moved. Here `under`, the object whose opaque
    /// face the last answer saw nearest, is taken to be the one under the
    /// pointer still, at the `depth` read now, and the searches of the CPU
    /// run from there as they do for an answer. No query of the GPU; a
    /// wrong guess (the pointer left the object) lasts until the next
    /// answer.
    pub fn drawn_hover_between(
        &self,
        world: &World,
        under: usize,
        ray: (Vec3, Vec3),
        depth: Vec3,
        far: f32,
    ) -> (Option<Vec3>, Option<usize>) {
        let at = (depth - ray.0).dot(ray.1);
        // (each frame the pointer moves: only the objects the ray can meet)
        let mut near = Vec::new();
        self.pick.ray_candidates(ray, &mut near);
        let adjusted = self.touch_depth_under(world, under, at, ray, Some(depth), far, Some(&near));
        let point = self.action_point(world, Some(ray), adjusted, false, far);
        // as `designated` for an opaque face of `under` at the depth
        let before = point.filter(|p| *p != depth).is_some_and(|p| (p - ray.0).dot(ray.1) + SAME < at);
        if !before && self.takes(world, Gesture::Touch, under, false) {
            return (Some(depth), Some(under));
        }
        (point, point.and_then(|p| self.interaction_at_ray(world, p, ray, true)))
    }

    /// A press on our displayed body, including meshes skinned to it.
    /// LLVOAvatar::lineSegmentIntersect tests posed collision volumes rather
    /// than accepting its bounding box. The GPU already gives us the posed
    /// surface: empty space around the head or between the legs stays empty.
    pub fn own_avatar_surface(&self, world: &World, drawn: &Drawn, ray: (Vec3, Vec3), found: Option<Vec3>) -> Option<Vec3> {
        for hit in [drawn.blend, drawn.opaque].into_iter().flatten() {
            let (idx, _) = self.record_object(hit.record)?;
            let status = self.drawn_status(world, idx);
            if status.ignored {
                continue;
            }
            // A transparent touch surface in front keeps its click, just as
            // it does before any other surface designated from the GPU answer.
            if found.is_some_and(|p| (p - hit.point).dot(ray.1) < -0.05) {
                return None;
            }
            let own_body = world.objects.get(idx).is_some_and(|o| o.is_avatar() && o.full_id == world.agent_id);
            // LLToolPie::handleLeftClickPick walks a passive attachment's
            // parents back to its wearer; a scripted prim keeps its touch.
            let own_passive = status.worn
                && !status.actionable
                && self.gpu[idx]
                    .owner_avatar
                    .and_then(|i| world.objects.get(i))
                    .is_some_and(|o| o.full_id == world.agent_id);
            // a mesh skinned to our skeleton is our body, unless the left
            // click acts on it ("Rigged attachments you wear")
            let own_rig = status.rigged == Some(Rigged::Own) && passes_through(status, self.rigged_clicks);
            return (own_body || own_rig || own_passive).then_some(hit.point);
        }
        None
    }

    /// The object and the face a draw record belongs to. A scan of the
    /// objects' faces: at a click, never per frame.
    pub fn record_object(&self, record: u32) -> Option<(usize, Option<usize>)> {
        self.gpu.iter().enumerate().find_map(|(idx, g)| {
            let draw = g.faces.iter().find(|f| f.record == record)?;
            let face = g.geom.and_then(|key| self.geom_ready(&key)).and_then(|geom| {
                geom.faces
                    .iter()
                    .position(|a| a.as_ref().is_some_and(|a| a.index_offset == draw.cmd.first_index))
            });
            Some((idx, face))
        })
    }

    pub fn drawn_status(&self, world: &World, idx: usize) -> Status {
        let (Some(g), Some(o)) = (self.gpu.get(idx), world.objects.get(idx)) else {
            return Status::default();
        };
        let worn = g.owner_avatar.is_some() && !g.is_avatar;
        // a worn animesh has a skeleton of its own, like a rezzed one
        let own_skeleton = !worn || matches!(Self::skeleton_owner(world, idx), Some((_, a)) if Some(a) != g.owner_avatar);
        let ours = g
            .owner_avatar
            .and_then(|a| world.objects.get(a))
            .is_some_and(|a| a.full_id == world.agent_id);
        let kind = match (own_skeleton, ours) {
            (true, _) => Rigged::Animesh,
            (false, true) => Rigged::Own,
            (false, false) => Rigged::Others,
        };
        Status {
            rigged: (g.rigged && !g.hud && !g.is_avatar).then_some(kind),
            posed_by_gpu: !g.hud && !g.is_avatar && (g.rigged || (worn && o.visual_volume().is_mesh())),
            worn: worn && !g.hud,
            actionable: crate::interaction::target(world, idx, &HashMap::new()).is_some(),
            avatar: g.is_avatar,
            ignored: o.click_action == crate::interaction::code::IGNORE,
            selectable: crate::build::selectable(world, idx),
        }
    }

    /// Touch: a rigged mesh lets the click through, as the Legacy Viewer's
    /// left click does with every rigged mesh (LLToolPie::handleMouseDown,
    /// `pick_rigged = false`): a mesh body or rigged clothing, scripted or
    /// not, does not hide the scripted prims worn under it (#488). The
    /// preferences of [`RiggedClicks`] give the click to a scripted rigged
    /// mesh of the kinds that are on instead. A rigged mesh, or a mesh worn
    /// by an avatar, with nothing to do with a click (no touch script and
    /// no click action) lets it through whatever they say: a press on our
    /// mesh body steers our avatar (LLToolCamera::mMouseSteering).
    pub fn touch_passes_through(&self, world: &World, idx: usize) -> bool {
        passes_through(self.drawn_status(world, idx), self.rigged_clicks)
    }

    /// The depth a left click searches from: the GPU's, or, when the
    /// nearest opaque face drawn under it is a mesh the click passes through
    /// ([`Self::touch_passes_through`]), the nearest surface the CPU finds
    /// behind that mesh. On an avatar, only just under its surface (a prim
    /// under the clothing, not through the body to what is worn on the other
    /// side); nothing there: the GPU's depth, the click is the avatar's.
    pub fn touch_depth(&self, world: &World, drawn: &Drawn, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, far: f32) -> Option<Vec3> {
        let (Some(face), Some(ray)) = (drawn.opaque, ray) else {
            return depth;
        };
        let Some((idx, _)) = self.record_object(face.record) else {
            return depth;
        };
        self.touch_depth_under(world, idx, (face.point - ray.0).dot(ray.1), ray, depth, far, None)
    }

    /// [`Self::touch_depth`] once the nearest opaque face drawn under the
    /// click is known: `idx`'s, `at` along `ray`. `among`: the objects the
    /// searches behind it may meet (None: all).
    fn touch_depth_under(
        &self,
        world: &World,
        idx: usize,
        at: f32,
        ray: (Vec3, Vec3),
        depth: Option<Vec3>,
        far: f32,
        among: Option<&[usize]>,
    ) -> Option<Vec3> {
        /// How far under a worn mesh's surface the click goes on.
        const UNDER_WORN: f32 = 0.25;
        if self.gpu.get(idx).is_some_and(|g| g.is_avatar && !g.hud) {
            // LLPipeline::lineSegmentIntersectInWorld (indra/newview/pipeline.cpp,
            // LGPL 2.1): ATTACHMENT_OVERRIDE_DIST lets an attachment up to
            // 10 cm inside the avatar win. Use the displayed body surface,
            // so standing and seated poses share the same bounded policy.
            return self
                .point_behind_rigged(world, ray, at, (at + 0.1).next_up(), Some(idx), among)
                .filter(|point| {
                    self.interaction_at_ray(world, *point, ray, true)
                        .and_then(|i| self.gpu.get(i))
                        .is_some_and(|g| g.owner_avatar.is_some() && !g.is_avatar && !g.hud)
                })
                .or(depth);
        }
        if !self.touch_passes_through(world, idx) {
            return depth;
        }
        let wearer = self.gpu.get(idx).and_then(|g| g.owner_avatar);
        let limit = if wearer.is_some() { at + UNDER_WORN } else { far };
        // (the drawn face is a frame younger than the CPU's poses)
        self.point_behind_rigged(world, ray, at - 0.05, limit, wearer, among).or(depth)
    }

    /// Whether a face of `idx` drawn under the pixel gives it `gesture`
    /// ([`reaches`]): not a mesh the left click passes through.
    fn takes(&self, world: &World, gesture: Gesture, idx: usize, build_open: bool) -> bool {
        let through = gesture == Gesture::Touch && !build_open && self.touch_passes_through(world, idx);
        !through && reaches(gesture, self.drawn_status(world, idx), build_open)
    }

    /// The object `gesture` designates among the faces drawn under its
    /// pixel, if it is one the drawn faces speak for ([`reaches`]). `ray`:
    /// the cursor ray; `found`: the point the CPU search settled on, which
    /// keeps the click when it is in front (a transparent prim before the
    /// rigged mesh). `depth` is the older opaque GPU sample: it is not an
    /// extra occluder when the CPU search left it unchanged.
    pub fn designated(
        &self,
        world: &World,
        gesture: Gesture,
        drawn: &Drawn,
        ray: Option<(Vec3, Vec3)>,
        found: Option<Vec3>,
        depth: Option<Vec3>,
        build_open: bool,
    ) -> Option<DrawnObject> {
        // Only a surface actually found by the CPU can bound the newer
        // GPU pick. An animated attachment can move more than 5 cm between
        // the two frames (#416); its own older depth must not reject it.
        // Opaque occlusion is already resolved in the drawn-face pass.
        let along = |p: Vec3| ray.map_or(0.0, |(o, d)| (p - o).dot(d));
        let limit = found.filter(|p| Some(*p) != depth).map_or(f32::INFINITY, |p| along(p) + SAME);
        for (hit, blend) in [(drawn.blend, true), (drawn.opaque, false)] {
            let Some(hit) = hit else { continue };
            let Some((idx, face)) = self.record_object(hit.record) else {
                continue;
            };
            if world
                .objects
                .get(idx)
                .is_some_and(|o| o.is_client_preview() && !o.is_editable_preview())
            {
                continue;
            }
            if along(hit.point) <= limit && self.takes(world, gesture, idx, build_open) {
                return Some(DrawnObject { idx, face, blend, hit });
            }
            let status = self.drawn_status(world, idx);
            // a worn Blend prim with nothing to do still stops the press
            // (picking.rs, `blend_hit`): the search on the CPU speaks for it
            if blend && gesture == Gesture::Touch && status.worn && !status.posed_by_gpu && !status.ignored {
                return None;
            }
        }
        None
    }

    /// The surface information of a touch on a drawn face (ObjectGrab's
    /// SurfaceInfo, LLPickInfo::getSurfaceInfo): the face and the position
    /// are those shown; ST is interpolated by the GPU at the pixel and UV
    /// follows from the texture entry; the normal and the binormal are
    /// those of the posed surface around the pixel (Firestorm sends the
    /// unskinned ones of the rigged volume), zero at the edge of a face.
    pub fn drawn_surface(&self, world: &World, d: &DrawnObject) -> Option<TouchSurface> {
        let o = world.objects.get(d.idx)?;
        let face = d.face?;
        let tf = o.te.as_ref().map(|t| *t.face(face)).unwrap_or_default();
        let uv = crate::media::pick::te_transform(d.hit.st, [tf.scale_s, tf.scale_t], [tf.offset_s, tf.offset_t], tf.rotation);
        Some(TouchSurface {
            uv: uv.extend(0.0),
            st: d.hit.st.extend(0.0),
            face: face as i32,
            position: d.hit.point - world.region_offset(o.key.region)?,
            normal: d.hit.normal,
            binormal: d.hit.binormal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIGGED: Status = Status {
        rigged: Some(Rigged::Animesh),
        posed_by_gpu: true,
        worn: false,
        actionable: false,
        avatar: false,
        ignored: false,
        selectable: true,
    };

    #[test]
    fn own_avatar_steering_requires_a_drawn_body_surface() {
        let (world, mut scene, front, back) = super::super::picking::tests::indexed_scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        scene.gpu.resize_with(scene.gpu.len().max(own + 1), Default::default);
        scene.gpu[own].is_avatar = true;
        let body_draw = FaceDraw {
            record: 2,
            ..scene.gpu[front].faces[0]
        };
        scene.gpu[own].faces.push(body_draw);
        scene.gpu[back].faces[0].record = 1;
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let face = |record, point| DrawnFace {
            record,
            point,
            st: glam::Vec2::splat(0.5),
            normal: -Vec3::X,
            binormal: Vec3::Z,
        };
        let body_point = Vec3::new(2.5, 0.0, 50.0);
        let body = face(2, body_point);
        let drawn = Drawn {
            opaque: Some(body),
            blend: None,
        };
        let pick = |scene: &Scene, drawn: &Drawn, point| scene.own_avatar_surface(&world, drawn, ray, point);
        assert_eq!(pick(&scene, &drawn, Some(body_point)), Some(body_point));

        // A nearby world surface, a distant one behind the avatar's axis,
        // and sky all leave the avatar's empty silhouette unclaimed.
        for point in [body_point + Vec3::X * 0.1, body_point + Vec3::X * 4.0] {
            let gap = Drawn {
                opaque: Some(face(1, point)),
                blend: None,
            };
            assert_eq!(pick(&scene, &gap, Some(point)), None);
        }
        assert_eq!(pick(&scene, &Drawn::default(), None), None);
        assert_eq!(pick(&scene, &drawn, Some(body_point - Vec3::X)), None);

        // Blended objects and actionable worn prims in front keep their
        // own gestures, even when the opaque answer is our body.
        let covered = Drawn {
            blend: Some(face(0, body_point - Vec3::X)),
            ..drawn
        };
        assert_eq!(pick(&scene, &covered, Some(body_point)), None);
        scene.gpu[front].owner_avatar = Some(own);
        assert_eq!(pick(&scene, &covered, Some(body_point)), None);

        // A mesh skinned to our skeleton remains our body, including
        // Blend clothing and parts outside the simulator's size box.
        scene.gpu[front].rigged = true;
        let mesh = Drawn {
            opaque: Some(face(0, body_point)),
            blend: None,
        };
        assert_eq!(pick(&scene, &mesh, Some(body_point)), Some(body_point));
        assert_eq!(pick(&scene, &covered, Some(body_point)), Some(body_point - Vec3::X));
        let other = world.objects.avatars().find(|(_, o)| o.full_id != world.agent_id).unwrap().0;
        scene.gpu[front].owner_avatar = Some(other);
        assert_eq!(pick(&scene, &mesh, Some(body_point)), None);
    }

    #[test]
    fn passive_own_attachments_steer_but_scripted_ones_keep_their_touch() {
        let (mut world, mut scene, front, _) = super::super::picking::tests::indexed_scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        scene.gpu[front].owner_avatar = Some(own);
        let point = Vec3::new(2.5, 0.0, 50.0);
        let ray = (point - Vec3::X, Vec3::X);
        let drawn = Drawn {
            opaque: Some(DrawnFace {
                record: 0,
                point,
                st: glam::Vec2::ZERO,
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            blend: None,
        };
        let o = world.objects.get_mut(front).unwrap();
        o.click_action = crate::interaction::code::NONE;
        o.update_flags = 0;
        assert_eq!(scene.own_avatar_surface(&world, &drawn, ray, Some(point)), Some(point));
        world.objects.get_mut(front).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        assert_eq!(scene.own_avatar_surface(&world, &drawn, ray, Some(point)), None);
        world.objects.get_mut(front).unwrap().click_action = crate::interaction::code::IGNORE;
        assert_eq!(scene.own_avatar_surface(&world, &drawn, ray, Some(point)), None);
        scene.gpu.resize_with(scene.gpu.len().max(own + 1), Default::default);
        let body = FaceDraw {
            record: 2,
            ..scene.gpu[front].faces[0]
        };
        scene.gpu[own].faces.push(body);
        let face = drawn.opaque.unwrap();
        let through = Drawn {
            blend: Some(face),
            opaque: Some(DrawnFace {
                record: 2,
                point: point + Vec3::X,
                ..face
            }),
        };
        assert_eq!(
            scene.own_avatar_surface(&world, &through, ray, Some(point + Vec3::X)),
            Some(point + Vec3::X)
        );
    }

    /// Gesture x target: what Aurora designates from the drawn faces.
    #[test]
    fn every_gesture_reaches_a_rigged_mesh_without_requiring_an_action() {
        let animesh = RIGGED;
        let worn_self = Status { worn: true, ..RIGGED };
        let worn_other = Status {
            worn: true,
            selectable: false,
            ..RIGGED
        };
        for build in [false, true] {
            for target in [animesh, worn_self, worn_other] {
                // Material cutouts are resolved by the GPU. Every remaining
                // surface may be designated, even a Blend face; a touch
                // without an action then goes through (`touch_passes_through`).
                assert!(reaches(Gesture::Touch, target, build));
                assert!(reaches(Gesture::Menu, target, build));
                assert_eq!(reaches(Gesture::Build, target, build), target.selectable);
            }
        }
    }

    #[test]
    fn a_drop_reaches_what_is_worn_and_editable_and_nothing_else() {
        let worn = Status { worn: true, ..RIGGED };
        assert!(reaches(Gesture::Drop, worn, false), "rigged or not");
        let worn_prim = Status {
            posed_by_gpu: false,
            ..worn
        };
        assert!(reaches(Gesture::Drop, worn_prim, false), "a worn prim too");
        assert!(!reaches(Gesture::Drop, RIGGED, false), "an animesh is a world object");
        let locked = Status { selectable: false, ..worn };
        assert!(!reaches(Gesture::Drop, locked, false), "not editable");
        let ignored = Status { ignored: true, ..worn };
        assert!(!reaches(Gesture::Drop, ignored, false));
        let avatar = Status {
            avatar: true,
            worn: false,
            posed_by_gpu: false,
            ..RIGGED
        };
        assert!(!reaches(Gesture::Drop, avatar, false));
    }

    #[test]
    fn ignore_is_seen_through_outside_the_build_tools() {
        let ignored = Status { ignored: true, ..RIGGED };
        for gesture in [Gesture::Touch, Gesture::Menu, Gesture::Build] {
            assert!(!reaches(gesture, ignored, false), "{gesture:?}");
        }
        assert!(reaches(Gesture::Menu, ignored, true));
        assert!(reaches(Gesture::Build, ignored, true));
    }

    #[test]
    fn what_the_cpu_finds_itself_keeps_its_search() {
        // a prim on the ground: picking.rs, as before
        let prim = Status {
            selectable: true,
            ..Status::default()
        };
        for gesture in [Gesture::Touch, Gesture::Menu, Gesture::Build] {
            assert!(!reaches(gesture, prim, false), "{gesture:?}");
        }
        // a worn prim without an action and an avatar: only the menu takes
        // them from what is drawn (the attachment in front of its wearer)
        let worn_prim = Status {
            worn: true,
            ..Status::default()
        };
        let avatar = Status {
            avatar: true,
            ..Status::default()
        };
        for s in [worn_prim, avatar] {
            assert!(reaches(Gesture::Menu, s, false));
            assert!(!reaches(Gesture::Touch, s, false));
            assert!(!reaches(Gesture::Build, s, false));
        }
    }

    /// #488: rigged clothing without a script over a scripted prim worn just
    /// under it. The touch goes on to the prim; the menu still designates
    /// the clothing; a scripted rigged mesh keeps the touch only when its
    /// preference says so.
    #[test]
    fn a_touch_goes_through_a_rigged_mesh_without_an_action() {
        let (mut world, mut scene, clothing, prim) = super::super::picking::tests::indexed_scene_with_two_objects();
        // another resident wears it: a rigged mesh we wear never takes the touch
        let wearer = world
            .objects
            .avatars()
            .find(|(_, a)| a.full_id != world.agent_id)
            .map(|(idx, _)| idx);
        // the clothing's face at x = 2.5, the prim's 10 cm behind it
        world.objects.get_mut(prim).unwrap().position = Vec3::new(3.1, 0.0, 50.0);
        for (idx, flags) in [(clothing, 0), (prim, crate::ui::context::flags::HANDLE_TOUCH)] {
            let o = world.objects.get_mut(idx).unwrap();
            o.click_action = crate::interaction::code::TOUCH;
            o.update_flags = flags;
        }
        scene.gpu[clothing].rigged = true;
        scene.gpu[clothing].owner_avatar = wearer;
        scene.gpu[prim].faces[0].record = 1;
        // the ray grazes the box of another avatar (ours, seen from behind)
        // before the clothing: no surface there, the GPU drew none
        let grazed = world
            .objects
            .avatars()
            .map(|(idx, _)| idx)
            .find(|&idx| Some(idx) != wearer)
            .unwrap();
        scene.gpu.resize_with(scene.gpu.len().max(grazed + 1), Default::default);
        scene.gpu[grazed].is_avatar = true;
        scene.gpu[grazed].center = Vec3::new(1.0, 0.0, 50.0);
        let face = FaceDraw {
            record: 2,
            ..scene.gpu[prim].faces[0]
        };
        scene.gpu[grazed].faces.push(face);
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let on_clothing = Vec3::new(2.5, 0.0, 50.0);
        let drawn = Drawn {
            opaque: Some(DrawnFace {
                record: 0,
                point: on_clothing,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            ..Default::default()
        };
        let designated = |scene: &Scene, world: &World, gesture| {
            scene
                .designated(world, gesture, &drawn, Some(ray), Some(on_clothing), Some(on_clothing), false)
                .map(|d| d.idx)
        };
        assert!(scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), None);
        assert_eq!(designated(&scene, &world, Gesture::Menu), Some(clothing));
        let depth = scene
            .touch_depth(&world, &drawn, Some(ray), Some(on_clothing), 100.0)
            .expect("a depth");
        assert!((depth.x - 2.6).abs() < 1e-3, "the prim under the clothing: {depth:?}");
        assert_eq!(scene.interaction_at_ray(&world, depth, ray, true), Some(prim));

        // what is worn on the other side of the body stays out of reach:
        // the click is the avatar's
        world.objects.get_mut(prim).unwrap().position = Vec3::new(7.0, 0.0, 50.0);
        assert_eq!(
            scene.touch_depth(&world, &drawn, Some(ray), Some(on_clothing), 100.0),
            Some(on_clothing)
        );

        // scripted clothing lets the touch through as well, as in the
        // Legacy Viewer; the other two preferences do not speak for it
        world.objects.get_mut(clothing).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        assert_eq!(scene.drawn_status(&world, clothing).rigged, Some(Rigged::Others));
        for (animesh, own) in [(false, false), (true, false), (false, true), (true, true)] {
            scene.rigged_clicks = RiggedClicks {
                animesh,
                own,
                others: false,
            };
            assert!(scene.touch_passes_through(&world, clothing));
            assert_eq!(designated(&scene, &world, Gesture::Touch), None);
            assert_eq!(designated(&scene, &world, Gesture::Menu), Some(clothing));
        }

        // "Rigged attachments of other avatars": it keeps the touch
        scene.rigged_clicks = RiggedClicks {
            animesh: false,
            own: false,
            others: true,
        };
        assert!(!scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), Some(clothing));
        assert_eq!(
            scene.touch_depth(&world, &drawn, Some(ray), Some(on_clothing), 100.0),
            Some(on_clothing)
        );

        // ...unless we wear it: "Rigged attachments you wear" speaks for it
        // then. Off, the press goes on to our avatar (to steer it), or to
        // the scripted prim worn just under it
        world.agent_id = world.objects.get(wearer.unwrap()).unwrap().full_id;
        assert_eq!(scene.drawn_status(&world, clothing).rigged, Some(Rigged::Own));
        assert!(scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), None);
        assert_eq!(designated(&scene, &world, Gesture::Menu), Some(clothing));
        assert_eq!(scene.own_avatar_surface(&world, &drawn, ray, Some(on_clothing)), Some(on_clothing));
        world.objects.get_mut(prim).unwrap().position = Vec3::new(3.1, 0.0, 50.0);
        let depth = scene
            .touch_depth(&world, &drawn, Some(ray), Some(on_clothing), 100.0)
            .expect("a depth");
        assert_eq!(scene.interaction_at_ray(&world, depth, ray, true), Some(prim));
        // the build tools still reach it, as with the build floater open in
        // the Legacy Viewer (LLVOVolume::lineSegmentIntersect)
        let built = scene.designated(
            &world,
            Gesture::Touch,
            &drawn,
            Some(ray),
            Some(on_clothing),
            Some(on_clothing),
            true,
        );
        assert_eq!(built.map(|d| d.idx), Some(clothing));
        // on, our scripted clothing keeps the touch and no longer steers
        scene.rigged_clicks = RiggedClicks {
            animesh: false,
            own: true,
            others: false,
        };
        assert!(!scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), Some(clothing));
        assert_eq!(
            scene.touch_depth(&world, &drawn, Some(ray), Some(on_clothing), 100.0),
            Some(on_clothing)
        );
        assert_eq!(scene.own_avatar_surface(&world, &drawn, ray, Some(on_clothing)), None);

        // a scripted animesh we wear has a skeleton of its own: it follows
        // "Animesh", like a rezzed one (a pet worn on the head)
        let wearer_local = world.objects.get(wearer.unwrap()).unwrap().key.local_id;
        let pet = world.objects.get_mut(clothing).unwrap();
        pet.parent_id = wearer_local;
        pet.extra.extended_mesh_flags = Some(aurora_prim::extra::EXTENDED_MESH_ANIMATED);
        assert_eq!(Scene::skeleton_owner(&world, clothing).map(|(_, a)| a), Some(clothing));
        assert_eq!(scene.drawn_status(&world, clothing).rigged, Some(Rigged::Animesh));
        assert!(scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), None);
        scene.rigged_clicks = RiggedClicks::default();
        assert!(!scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), Some(clothing));
    }

    /// The left click's rule alone: a rigged mesh with an action keeps the
    /// click only when the preference of its kind is on; one with nothing
    /// to do never does; what is not rigged is not asked.
    #[test]
    fn a_rigged_mesh_keeps_a_click_only_when_its_preference_is_on() {
        const KINDS: [Rigged; 3] = [Rigged::Animesh, Rigged::Own, Rigged::Others];
        let scripted = |rigged| Status {
            rigged: Some(rigged),
            actionable: true,
            worn: rigged != Rigged::Animesh,
            ..RIGGED
        };
        for bits in 0..8 {
            let clicks = RiggedClicks {
                animesh: bits & 1 != 0,
                own: bits & 2 != 0,
                others: bits & 4 != 0,
            };
            assert_eq!(passes_through(scripted(Rigged::Animesh), clicks), !clicks.animesh);
            assert_eq!(passes_through(scripted(Rigged::Own), clicks), !clicks.own);
            assert_eq!(passes_through(scripted(Rigged::Others), clicks), !clicks.others);
            for kind in KINDS {
                let passive = Status {
                    actionable: false,
                    ..scripted(kind)
                };
                assert!(passes_through(passive, clicks));
            }
            // a scripted mesh worn without a skin, a scripted prim and an
            // avatar: found where they are, as before
            let worn_mesh = Status {
                rigged: None,
                ..scripted(Rigged::Others)
            };
            assert!(!passes_through(worn_mesh, clicks));
            let prim = Status {
                actionable: true,
                ..Status::default()
            };
            assert!(!passes_through(prim, clicks));
            let avatar = Status {
                avatar: true,
                ..scripted(Rigged::Animesh)
            };
            assert!(!passes_through(avatar, clicks));
        }
        // animesh alone by default
        let default = RiggedClicks::default();
        assert!(!passes_through(scripted(Rigged::Animesh), default));
        assert!(passes_through(scripted(Rigged::Own), default));
        assert!(passes_through(scripted(Rigged::Others), default));
        // a click with Alt, Ctrl or Shift: as before the preferences, every
        // scripted rigged mesh but ours
        let modified = RiggedClicks::MODIFIER_CLICK;
        assert!(!passes_through(scripted(Rigged::Animesh), modified));
        assert!(passes_through(scripted(Rigged::Own), modified));
        assert!(!passes_through(scripted(Rigged::Others), modified));
        assert!(KINDS.iter().all(|kind| !passes_through(scripted(*kind), RiggedClicks::ALL)));
    }

    /// The reported case: a scripted mesh body worn by another resident,
    /// with a scripted prim worn inside it. As in the Legacy Viewer, the
    /// left click and the hover cursor reach the prim; "Rigged attachments
    /// of other avatars" gives them to the body, as before.
    #[test]
    fn a_scripted_mesh_body_lets_the_touch_reach_the_prim_worn_inside_it() {
        let (mut world, mut scene, body, prim) = super::super::picking::tests::indexed_scene_with_two_objects();
        let wearer = world
            .objects
            .avatars()
            .find(|(_, a)| a.full_id != world.agent_id)
            .map(|(idx, _)| idx);
        // the body's face at x = 2.5, the prim's 10 cm behind it
        world.objects.get_mut(prim).unwrap().position = Vec3::new(3.1, 0.0, 50.0);
        for idx in [body, prim] {
            let o = world.objects.get_mut(idx).unwrap();
            o.click_action = crate::interaction::code::TOUCH;
            o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        }
        scene.gpu[body].rigged = true;
        scene.gpu[body].owner_avatar = wearer;
        scene.gpu[prim].owner_avatar = wearer;
        scene.gpu[prim].faces[0].record = 1;
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let on_body = Vec3::new(2.5, 0.0, 50.0);
        let drawn = Drawn {
            opaque: Some(DrawnFace {
                record: 0,
                point: on_body,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            ..Default::default()
        };
        assert_eq!(scene.drawn_status(&world, body).rigged, Some(Rigged::Others));
        let touched = |scene: &Scene| {
            let depth = scene.touch_depth(&world, &drawn, Some(ray), Some(on_body), 100.0);
            let hit = scene.action_point(&world, Some(ray), depth, false, 100.0)?;
            match scene.designated(&world, Gesture::Touch, &drawn, Some(ray), Some(hit), Some(on_body), false) {
                Some(shown) => Some(shown.idx),
                None => scene.left_interaction_at_ray(&world, hit, ray, true),
            }
        };
        assert_eq!(touched(&scene), Some(prim));
        assert_eq!(scene.drawn_hover(&world, &drawn, ray, Some(on_body), 100.0).1, Some(prim));
        scene.rigged_clicks = RiggedClicks {
            others: false,
            ..RiggedClicks::ALL
        };
        assert_eq!(touched(&scene), Some(prim), "neither an animesh nor ours");

        scene.rigged_clicks.others = true;
        assert_eq!(touched(&scene), Some(body));
        assert_eq!(scene.drawn_hover(&world, &drawn, ray, Some(on_body), 100.0).1, Some(body));
    }

    /// The pointer moves over a mesh body between two answers of the GPU:
    /// the cursor keeps the clicker worn under the body, or the body when
    /// its preference gives it the click, without a new answer; beside the
    /// clicker it is the avatar's again.
    #[test]
    fn the_hover_follows_the_pointer_between_two_answers() {
        let (mut world, mut scene, body, prim) = super::super::picking::tests::indexed_scene_with_two_objects();
        let wearer = world
            .objects
            .avatars()
            .find(|(_, a)| a.full_id != world.agent_id)
            .map(|(idx, _)| idx);
        // the body's face at x = 2.5, the prim's 10 cm behind it
        world.objects.get_mut(prim).unwrap().position = Vec3::new(3.1, 0.0, 50.0);
        for idx in [body, prim] {
            let o = world.objects.get_mut(idx).unwrap();
            o.click_action = crate::interaction::code::TOUCH;
            o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        }
        scene.gpu[body].rigged = true;
        scene.gpu[body].owner_avatar = wearer;
        scene.gpu[prim].owner_avatar = wearer;
        scene.pick.changed(prim);
        // the answer was asked along y = 0; the pointer is 2 cm to the side
        let moved = (Vec3::new(0.0, 0.02, 50.0), Vec3::X);
        let on_body = Vec3::new(2.5, 0.02, 50.0);
        let between = |scene: &Scene, ray: (Vec3, Vec3), depth: Vec3| scene.drawn_hover_between(&world, body, ray, depth, 100.0);
        let (point, object) = between(&scene, moved, on_body);
        assert_eq!(object, Some(prim));
        assert!((point.unwrap().x - 2.6).abs() < 1e-3, "on the prim: {point:?}");
        // beside the prim: nothing under the body there
        let beside = (Vec3::new(0.0, 2.0, 50.0), Vec3::X);
        assert_eq!(between(&scene, beside, Vec3::new(2.5, 2.0, 50.0)).1, None);

        // "Rigged attachments of other avatars": the body, where the
        // pointer is now
        scene.rigged_clicks.others = true;
        assert_eq!(between(&scene, moved, on_body), (Some(on_body), Some(body)));
        assert_eq!(between(&scene, beside, Vec3::new(2.5, 2.0, 50.0)).1, Some(body));
    }

    /// A scripted animesh rezzed in front of a scripted prim: with
    /// "Animesh" off, the left click goes on to the prim, however far
    /// behind; on, as it is by default, the animesh keeps it. The menu
    /// designates the animesh either way.
    #[test]
    fn a_touch_passes_through_a_scripted_animesh_unless_it_is_clickable() {
        let (mut world, mut scene, animesh, prim) = super::super::picking::tests::indexed_scene_with_two_objects();
        world.objects.get_mut(prim).unwrap().position = Vec3::new(9.0, 0.0, 50.0);
        for idx in [animesh, prim] {
            let o = world.objects.get_mut(idx).unwrap();
            o.click_action = crate::interaction::code::TOUCH;
            o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        }
        scene.gpu[animesh].rigged = true;
        scene.gpu[prim].faces[0].record = 1;
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let on_animesh = Vec3::new(2.5, 0.0, 50.0);
        let drawn = Drawn {
            opaque: Some(DrawnFace {
                record: 0,
                point: on_animesh,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            ..Default::default()
        };
        assert_eq!(scene.drawn_status(&world, animesh).rigged, Some(Rigged::Animesh));
        scene.rigged_clicks.animesh = false;
        let designated = |scene: &Scene, gesture, found| {
            scene
                .designated(&world, gesture, &drawn, Some(ray), found, Some(on_animesh), false)
                .map(|d| d.idx)
        };
        let depth = scene
            .touch_depth(&world, &drawn, Some(ray), Some(on_animesh), 100.0)
            .expect("the prim behind");
        assert!(depth.x > 8.0, "behind the animesh: {depth:?}");
        assert_eq!(scene.interaction_at_ray(&world, depth, ray, true), Some(prim));
        assert_eq!(designated(&scene, Gesture::Touch, Some(depth)), None);
        assert_eq!(designated(&scene, Gesture::Menu, Some(on_animesh)), Some(animesh));
        assert_eq!(scene.drawn_hover(&world, &drawn, ray, Some(on_animesh), 100.0).1, Some(prim));
        scene.rigged_clicks = RiggedClicks {
            animesh: false,
            ..RiggedClicks::ALL
        };
        assert_eq!(designated(&scene, Gesture::Touch, Some(depth)), None, "not an attachment");

        scene.rigged_clicks = RiggedClicks::default();
        assert_eq!(
            scene.touch_depth(&world, &drawn, Some(ray), Some(on_animesh), 100.0),
            Some(on_animesh)
        );
        assert_eq!(designated(&scene, Gesture::Touch, Some(on_animesh)), Some(animesh));
        assert_eq!(scene.drawn_hover(&world, &drawn, ray, Some(on_animesh), 100.0).1, Some(animesh));
    }

    /// #1241: an invisible clicker (every face at alpha 0, no draw record)
    /// worn partly inside a mesh body without a script. The Legacy Viewer's
    /// transparent pick never sees the rigged body: the click reaches the
    /// clicker 15 cm under the body's surface, every time, not only when an
    /// animation brings it within the click tolerance. Without an action it
    /// stops nothing.
    #[test]
    fn a_touch_reaches_an_invisible_clicker_inside_a_mesh_body() {
        let (mut world, mut scene, body, clicker) = super::super::picking::tests::indexed_scene_with_two_objects();
        let wearer = world.objects.avatars().next().map(|(idx, _)| idx);
        // the body's face at x = 2.5, the clicker's 15 cm behind it
        let o = world.objects.get_mut(clicker).unwrap();
        o.position = Vec3::new(3.15, 0.0, 50.0);
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        for f in &mut Arc::make_mut(o.te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        // a mesh body without a script
        let o = world.objects.get_mut(body).unwrap();
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = 0;
        scene.gpu[body].rigged = true;
        scene.gpu[body].owner_avatar = wearer;
        scene.gpu[clicker].owner_avatar = wearer;
        scene.gpu[clicker].built_faces = 1;
        scene.gpu[clicker].faces.clear();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let on_body = Vec3::new(2.5, 0.0, 50.0);
        let drawn = Drawn {
            opaque: Some(DrawnFace {
                record: 0,
                point: on_body,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            ..Default::default()
        };
        let depth = scene.touch_depth(&world, &drawn, Some(ray), Some(on_body), 100.0);
        assert!(
            depth.is_some_and(|p| (p.x - 2.65).abs() < 1e-3),
            "the clicker under the body: {depth:?}"
        );
        let hit = scene.action_point(&world, Some(ray), depth, false, 100.0).expect("a point");
        assert_eq!(scene.interaction_at_ray(&world, hit, ray, true), Some(clicker));
        assert_eq!(scene.drawn_hover(&world, &drawn, ray, Some(on_body), 100.0).1, Some(clicker));
        let designated = scene.designated(&world, Gesture::Touch, &drawn, Some(ray), Some(hit), Some(on_body), false);
        assert_eq!(designated, None, "the body without a script lets the touch through");

        // an invisible attachment with nothing to do stops nothing
        let o = world.objects.get_mut(clicker).unwrap();
        o.click_action = crate::interaction::code::NONE;
        o.update_flags = 0;
        assert_eq!(scene.touch_depth(&world, &drawn, Some(ray), Some(on_body), 100.0), Some(on_body));
    }

    /// A scripted prim worn by an animated avatar: the CPU places it a few
    /// frames after the one clicked, out of the click's ray, while the GPU
    /// drew it under the cursor. The touch is designated from what was
    /// drawn; a worn prim with nothing to do is not, and a worn Blend prim
    /// without an action in front leaves the press to the CPU's search.
    #[test]
    fn a_touch_reaches_a_scripted_worn_prim_where_it_was_drawn() {
        let (mut world, mut scene, front, prim) = super::super::picking::tests::scene_with_two_objects();
        let wearer = world.objects.avatars().next().map(|(idx, _)| idx);
        // drawn at x = 2.5 when clicked; the CPU now has it aside, off the
        // ray (and the other object out of the way)
        world.objects.get_mut(front).unwrap().position = Vec3::new(3.0, -3.0, 50.0);
        let o = world.objects.get_mut(prim).unwrap();
        o.position = Vec3::new(3.0, 1.5, 50.0);
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        scene.gpu[prim].owner_avatar = wearer;
        scene.gpu[prim].faces[0].record = 1;
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let on_prim = Vec3::new(2.5, 0.0, 50.0);
        let face = |record| DrawnFace {
            record,
            point: on_prim,
            st: glam::Vec2::splat(0.5),
            normal: -Vec3::X,
            binormal: Vec3::Z,
        };
        let drawn = Drawn {
            opaque: Some(face(1)),
            ..Default::default()
        };
        assert_eq!(scene.interaction_at_ray(&world, on_prim, ray, true), None, "the CPU misses it");
        let designated = |scene: &Scene, world: &World, drawn: &Drawn| {
            scene
                .designated(world, Gesture::Touch, drawn, Some(ray), Some(on_prim), Some(on_prim), false)
                .map(|d| d.idx)
        };
        assert_eq!(designated(&scene, &world, &drawn), Some(prim));
        assert_eq!(
            scene.drawn_hover(&world, &drawn, ray, Some(on_prim), 100.0),
            (Some(on_prim), Some(prim))
        );

        // a worn Blend prim without an action in front stops the press
        let o = world.objects.get_mut(front).unwrap();
        o.click_action = crate::interaction::code::NONE;
        o.update_flags = 0;
        scene.gpu[front].owner_avatar = wearer;
        let covered = Drawn {
            blend: Some(face(0)),
            ..drawn
        };
        assert_eq!(designated(&scene, &world, &covered), None);

        // nothing to do: the CPU's search speaks for it, as before
        let o = world.objects.get_mut(prim).unwrap();
        o.click_action = crate::interaction::code::NONE;
        o.update_flags = 0;
        assert_eq!(designated(&scene, &world, &drawn), None);
    }

    #[test]
    fn a_touch_reaches_a_worn_prim_only_with_an_action() {
        let worn_prim = Status {
            worn: true,
            ..Status::default()
        };
        assert!(!reaches(Gesture::Touch, worn_prim, false));
        let scripted = Status {
            actionable: true,
            ..worn_prim
        };
        assert!(reaches(Gesture::Touch, scripted, false));
        assert!(!reaches(Gesture::Build, scripted, false));
        // an avatar is never touched from what is drawn
        assert!(!reaches(Gesture::Touch, Status { avatar: true, ..scripted }, false));
    }
    #[test]
    fn attachment_click_targets_stop_at_the_wearer_when_standing_or_seated() {
        for own in [false, true] {
            for seated in [false, true] {
                let (mut world, mut scene, child, root) = super::super::picking::tests::indexed_scene_with_two_objects();
                let (wearer, avatar) = world
                    .objects
                    .avatars()
                    .find(|(_, a)| (a.full_id == world.agent_id) == own)
                    .map(|(i, a)| (i, a.key.local_id))
                    .expect("wearer");
                let root_id = world.objects.get(root).unwrap().key.local_id;
                world.objects.get_mut(root).unwrap().parent_id = avatar;
                world.objects.get_mut(root).unwrap().state = 0x10;
                world.objects.get_mut(child).unwrap().parent_id = root_id;
                world.objects.get_mut(child).unwrap().update_flags = 0;
                if seated {
                    world.apply(crate::demo::action_overlay(
                        989,
                        Vec3::new(130.0, 128.0, 49.0),
                        Quat::from_rotation_z(0.6),
                    ));
                    let a = world.objects.get_mut(wearer).unwrap();
                    a.parent_id = 989;
                    a.position = Vec3::new(0.15, 0.0, 0.65);
                }
                let owner = world.objects.get(wearer).unwrap().full_id;
                assert_eq!(Scene::wearer_avatar(&world, child), Some((owner, wearer)));
                assert_eq!(Scene::skeleton_owner(&world, child), Some((owner, wearer)));
                scene.gpu[child].owner_avatar = Some(wearer);
                scene.gpu[child].faces[0].record = 0;
                let hit = DrawnFace {
                    record: 0,
                    point: Vec3::new(3.0, 0.0, 50.0),
                    st: glam::Vec2::splat(0.5),
                    normal: Vec3::X,
                    binormal: Vec3::Y,
                };
                for scripted in [false, true] {
                    world.objects.get_mut(root).unwrap().update_flags = if scripted { crate::ui::context::flags::HANDLE_TOUCH } else { 0 };
                    let target = crate::interaction::target(&world, child, &HashMap::new());
                    assert_eq!(target.is_some(), scripted, "own={own}, seated={seated}");
                    if let Some(target) = target {
                        assert_eq!(target.action, crate::interaction::Action::Touch);
                        assert_eq!(target.clicked_id, world.objects.get(child).unwrap().full_id);
                        assert_eq!(target.root, world.objects.get(root).unwrap().full_id);
                    }
                    for (rigged, clickable) in [(false, false), (true, false), (true, true)] {
                        scene.gpu[child].rigged = rigged;
                        // its wearer's preference alone speaks for it: the
                        // other two stay on
                        scene.rigged_clicks = RiggedClicks {
                            animesh: true,
                            own: clickable || !own,
                            others: clickable || own,
                        };
                        let kind = if own { Rigged::Own } else { Rigged::Others };
                        assert_eq!(scene.drawn_status(&world, child).rigged, rigged.then_some(kind));
                        // a rigged mesh keeps the touch only when scripted
                        // and made clickable
                        let through = rigged && (!scripted || !clickable);
                        assert_eq!(scene.touch_passes_through(&world, child), through);
                        for blend in [false, true] {
                            let drawn = if blend {
                                Drawn {
                                    blend: Some(hit),
                                    ..Default::default()
                                }
                            } else {
                                Drawn {
                                    opaque: Some(hit),
                                    ..Default::default()
                                }
                            };
                            let picked = scene.designated(&world, Gesture::Touch, &drawn, None, None, None, false);
                            assert_eq!(
                                picked.map(|p| p.idx),
                                (scripted && !through).then_some(child),
                                "own={own}, seated={seated}, rigged={rigged}, clickable={clickable}, blend={blend}, scripted={scripted}"
                            );
                            assert_eq!(
                                scene
                                    .designated(&world, Gesture::Menu, &drawn, None, None, None, false)
                                    .map(|p| p.idx),
                                Some(child)
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn classic_avatar_keeps_the_legacy_ten_centimetre_attachment_priority() {
        let (mut world, mut scene, body, clicker) = super::super::picking::tests::indexed_scene_with_two_objects();
        let o = world.objects.get_mut(clicker).unwrap();
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        for face in &mut Arc::make_mut(o.te.as_mut().unwrap()).faces {
            face.color[3] = 0.0;
        }
        scene.gpu[body].is_avatar = true;
        scene.gpu[body].faces[0].record = 0;
        scene.gpu[clicker].built_faces = 1;
        scene.gpu[clicker].faces.clear();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let on_body = Vec3::new(2.5, 0.0, 50.0);
        let drawn = Drawn {
            opaque: Some(DrawnFace {
                record: 0,
                point: on_body,
                st: glam::Vec2::splat(0.5),
                normal: -Vec3::X,
                binormal: Vec3::Z,
            }),
            ..Default::default()
        };
        for (gap, worn, hud, accepted) in [
            (0.05, true, false, true),
            (0.099, true, false, true),
            (0.101, true, false, false),
            (0.05, false, false, false),
            (0.05, true, true, false),
        ] {
            world.objects.get_mut(clicker).unwrap().position.x = 3.0 + gap;
            scene.gpu[clicker].owner_avatar = worn.then_some(body);
            scene.gpu[clicker].hud = hud;
            let depth = scene.touch_depth(&world, &drawn, Some(ray), Some(on_body), 100.0).unwrap();
            let expected = if accepted { on_body + Vec3::X * gap } else { on_body };
            assert!(depth.abs_diff_eq(expected, 1e-5), "gap={gap}, worn={worn}, hud={hud}: {depth:?}");
            if accepted {
                assert_eq!(scene.interaction_at_ray(&world, depth, ray, true), Some(clicker));
            }
            if worn || hud {
                let hover = scene.drawn_hover(&world, &drawn, ray, Some(on_body), 100.0);
                assert_eq!(hover.1, accepted.then_some(clicker), "hover: gap={gap}, worn={worn}, hud={hud}");
            }
        }
    }
}
