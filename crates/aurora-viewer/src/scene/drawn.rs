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
            (along(hit.point) <= limit && reaches(gesture, self.drawn_status(world, idx), build_open)).then_some(DrawnObject {
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
                // surface stops the press, even a non-actionable Blend face.
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
}
