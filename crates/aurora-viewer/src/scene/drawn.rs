//! What a click designates from the faces the GPU drew under its pixel.
//!
//! The searches of picking.rs work on triangles the scene keeps on the CPU.
//! A rigged mesh is not where those are: the GPU poses it. So a click also
//! asks the renderer which draw records it shows under the pixel
//! (`Renderer::request_drawn`, aurora-render/src/drawn_pick.rs), and the
//! objects the CPU cannot find where they are shown are designated from
//! that answer: animated objects (animesh), rigged meshes worn by an
//! avatar, and worn meshes in general.
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
//! out. Aurora reads what is drawn, so every gesture reaches a rigged mesh,
//! where it is shown. Once designated it follows the rules of any object of
//! its kind: click action, touch, IGNORE, transparency, rights, menus.
//! Except that a left click on one with nothing to do with it (no touch
//! script, no click action) goes on to what is behind, as in the Legacy
//! Viewer: the scripted prims worn under a mesh body or rigged clothing
//! stay touchable (`touch_passes_through`, `touch_depth`).
//!
//! Camera clicks (Alt + click, Focus tool) use the visible-surface mode of
//! that pass directly: no CPU rest-pose or invisible touch-surface search.
//!
//! [`reaches`] is that rule, alone and tested. Blend faces intercept a
//! press regardless of their texel opacity or the presence of an action.

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
}

/// What a gesture needs to know of the object of a drawn face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The CPU searches cannot find it where it is shown: rigged (animesh,
    /// rigged attachment) or a mesh worn by an avatar.
    pub posed_by_gpu: bool,
    /// Worn by an avatar.
    pub worn: bool,
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
        Gesture::Touch => s.posed_by_gpu && !s.avatar,
        // the menu of what is shown (pick_transparent, pick_rigged): the
        // attachment in front of an avatar rather than the avatar
        Gesture::Menu => s.posed_by_gpu || s.worn || s.avatar,
        Gesture::Build => s.posed_by_gpu && s.selectable,
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
        Status {
            posed_by_gpu: !g.hud && !g.is_avatar && (g.rigged || (worn && o.visual_volume().is_mesh())),
            worn: worn && !g.hud,
            avatar: g.is_avatar,
            ignored: o.click_action == crate::interaction::code::IGNORE,
            selectable: crate::build::selectable(world, idx),
        }
    }

    /// Touch: a rigged mesh (or a mesh worn by an avatar) that has nothing
    /// to do with a click, no touch script and no click action, lets it
    /// through, as the Legacy Viewer's left click does with every rigged
    /// mesh: a mesh body or rigged clothing does not hide the scripted prims
    /// worn under it (#488).
    pub fn touch_passes_through(&self, world: &World, idx: usize) -> bool {
        let s = self.drawn_status(world, idx);
        s.posed_by_gpu && !s.avatar && crate::interaction::target(world, idx, &HashMap::new()).is_none()
    }

    /// The depth a left click searches from: the GPU's, or, when the
    /// nearest opaque face drawn under it is a mesh the click passes through
    /// ([`Self::touch_passes_through`]), the nearest surface the CPU finds
    /// behind that mesh. On an avatar, only just under its surface (a prim
    /// under the clothing, not through the body to what is worn on the other
    /// side); nothing there: the GPU's depth, the click is the avatar's.
    pub fn touch_depth(&self, world: &World, drawn: &Drawn, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, far: f32) -> Option<Vec3> {
        /// How far under a worn mesh's surface the click goes on.
        const UNDER_WORN: f32 = 0.25;
        let (Some(face), Some(ray)) = (drawn.opaque, ray) else {
            return depth;
        };
        let Some((idx, _)) = self.record_object(face.record) else {
            return depth;
        };
        if !self.touch_passes_through(world, idx) {
            return depth;
        }
        let wearer = self.gpu.get(idx).and_then(|g| g.owner_avatar);
        let at = (face.point - ray.0).dot(ray.1);
        let limit = if wearer.is_some() { at + UNDER_WORN } else { far };
        // (the drawn face is a frame younger than the CPU's poses)
        self.point_behind_rigged(world, ray, at - 0.05, limit, wearer).or(depth)
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
        const SAME: f32 = 0.05;
        let along = |p: Vec3| ray.map_or(0.0, |(o, d)| (p - o).dot(d));
        let limit = found.filter(|p| Some(*p) != depth).map_or(f32::INFINITY, |p| along(p) + SAME);
        [(drawn.blend, true), (drawn.opaque, false)].into_iter().find_map(|(hit, blend)| {
            let hit = hit?;
            let (idx, face) = self.record_object(hit.record)?;
            if world
                .objects
                .get(idx)
                .is_some_and(|o| o.is_client_preview() && !o.is_editable_preview())
            {
                return None;
            }
            let through = gesture == Gesture::Touch && !build_open && self.touch_passes_through(world, idx);
            (along(hit.point) <= limit && reaches(gesture, self.drawn_status(world, idx), build_open) && !through).then_some(DrawnObject {
                idx,
                face,
                blend,
                hit,
            })
        })
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
        posed_by_gpu: true,
        worn: false,
        avatar: false,
        ignored: false,
        selectable: true,
    };

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
        // a worn prim and an avatar: only the menu takes them from what is
        // drawn (the attachment in front of its wearer)
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
    /// the clothing; a scripted rigged mesh keeps the touch.
    #[test]
    fn a_touch_goes_through_a_rigged_mesh_without_an_action() {
        let (mut world, mut scene, clothing, prim) = super::super::picking::tests::scene_with_two_objects();
        let wearer = world.objects.avatars().next().map(|(idx, _)| idx);
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

        // scripted clothing keeps the touch
        world.objects.get_mut(clothing).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        assert!(!scene.touch_passes_through(&world, clothing));
        assert_eq!(designated(&scene, &world, Gesture::Touch), Some(clothing));
        assert_eq!(
            scene.touch_depth(&world, &drawn, Some(ray), Some(on_clothing), 100.0),
            Some(on_clothing)
        );
    }

    /// #1241: an invisible clicker (every face at alpha 0, no draw record)
    /// worn partly inside a mesh body without a script. The Legacy Viewer's
    /// transparent pick never sees the rigged body: the click reaches the
    /// clicker 15 cm under the body's surface, every time, not only when an
    /// animation brings it within the click tolerance. Without an action it
    /// stops nothing.
    #[test]
    fn a_touch_reaches_an_invisible_clicker_inside_a_mesh_body() {
        let (mut world, mut scene, body, clicker) = super::super::picking::tests::scene_with_two_objects();
        let wearer = world.objects.avatars().next().map(|(idx, _)| idx);
        // the body's face at x = 2.5, the clicker's 15 cm behind it
        let o = world.objects.get_mut(clicker).unwrap();
        o.position = Vec3::new(3.15, 0.0, 50.0);
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        for f in &mut Arc::make_mut(o.te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        world.objects.get_mut(body).unwrap().update_flags = 0;
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
        assert!(depth.is_some_and(|p| (p.x - 2.65).abs() < 1e-3), "the clicker under the body: {depth:?}");
        let hit = scene.action_point(&world, Some(ray), depth, false, 100.0).expect("a point");
        assert_eq!(scene.interaction_at_ray(&world, hit, ray, true), Some(clicker));
        let designated = scene.designated(&world, Gesture::Touch, &drawn, Some(ray), Some(hit), Some(on_body), false);
        assert_eq!(designated, None, "the body without a script lets the touch through");

        // an invisible attachment with nothing to do stops nothing
        let o = world.objects.get_mut(clicker).unwrap();
        o.click_action = crate::interaction::code::NONE;
        o.update_flags = 0;
        assert_eq!(scene.touch_depth(&world, &drawn, Some(ray), Some(on_body), 100.0), Some(on_body));
    }
}
