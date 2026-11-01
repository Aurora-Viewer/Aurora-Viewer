//! Build tools: selecting objects, moving / rotating / stretching them with
//! the manipulators, creating prims and terraforming, as in Firestorm
//! (LLToolMgr, LLSelectMgr, LLManipTranslate / Rotate / Scale, LLToolPlacer,
//! LLToolBrushLand; originally LGPL 2.1 sources).
//!
//! The selection is local to the viewer and mirrored to the simulators with
//! ObjectSelect / ObjectDeselect. Drags change the objects locally every
//! frame and send MultipleObjectUpdate when the mouse is released (stretching
//! also every 0.1 s, like LLManipScale).

pub mod align;
pub mod anim_pause;
pub mod contents;
pub mod contents_drop;
pub mod costs;
mod cycle;
pub mod demo;
pub mod demo_sim;
pub mod draw;
mod edge_pan;
pub mod edits;
pub mod geom;
pub mod grab;
pub mod hud_drag;
pub mod land;
pub mod layout;
pub mod manip;
mod material_preview;
pub mod materials;
pub mod particle_editor;
mod property_query;
pub mod replace;
pub mod script_queue;
#[cfg(test)]
mod selection_tests;
pub mod shapes;
pub mod silhouette;
#[cfg(test)]
mod take_tests;
pub mod task_inventory;
pub mod ui;

use crate::scene::Scene;
use crate::world::World;
use crate::world::objects::{LocalAppearance, ObjKey};
use aurora_net::build::{BuildCmd, ObjectProps, TransformUpdate, derez};
use aurora_net::{NetCommand, RegionHandle};
use geom::{Cam, RectCover, verts_in_rect};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LL_PCODE_* / FLAGS_CREATE_SELECTED of object updates (llprimitive.h).
pub const FLAGS_CREATE_SELECTED: u32 = 1 << 1;
/// REGION_FLAGS_BLOCK_TERRAFORM (llregionflags.h).
pub const REGION_FLAGS_BLOCK_TERRAFORM: u32 = 1 << 6;
/// MaxSelectDistance and MaxDragDistance defaults (settings.xml).
pub const MAX_SELECT_DISTANCE: f32 = 128.0;
pub const MAX_DRAG_DISTANCE: f32 = 48.0;
/// LLToolPlacer::raycastForNewObjPos: the simulator ray's length through
/// an object hit while the select distance is not limited.
const UNLIMITED_PLACEMENT_RAY: f32 = 129.0;
/// SL prim size limits (llmath/xform.h: DEFAULT_MIN_PRIM_SCALE / MAX).
pub const MIN_PRIM_SCALE: f32 = 0.01;
pub const MAX_PRIM_SCALE: f32 = 64.0;
/// How far from its point of the body an attachment may be typed
/// (llviewerjointattachment.h).
pub const MAX_ATTACHMENT_DIST: f32 = 3.5;

/// Build preferences (Firestorm settings.xml names in comments).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BuildSettings {
    /// Legacy Viewer tool strip and tabs instead of the Aurora workspace.
    pub legacy_window: bool,
    /// Personal compact inspector arrangement, independent of edited objects.
    pub layout: layout::Layout,
    pub transform_expanded: bool,
    /// Firestorm's ApplyColorImmediately and 32 saved color palette slots.
    pub color_picker: crate::ui::color_picker::Preferences,
    /// GridResolution (m).
    pub grid_resolution: f32,
    /// GridDrawSize: extent of the plane grid (m).
    pub grid_draw_size: f32,
    /// GridSubUnit: snap to 1/32 of a unit when close.
    pub grid_sub_unit: bool,
    /// GridOpacity.
    pub grid_opacity: f32,
    /// GridMode: 0 world, 1 local.
    pub grid_mode: u8,
    /// SnapEnabled.
    pub snap: bool,
    /// RotationStep (degrees) outside the snap ring.
    pub rotation_step: f32,
    /// EditLinkedParts.
    pub edit_linked: bool,
    /// ScaleUniform: stretch both sides.
    pub scale_uniform: bool,
    /// ScaleStretchTextures.
    pub scale_stretch_textures: bool,
    /// Keep the camera attached to the focused object while editing it.
    pub follow_edit_object: bool,
    /// RenderHighlightSelections.
    pub show_highlight: bool,
    /// Create tool: shape (index in `shapes::SHAPES`).
    pub create_shape: usize,
    /// CreateToolKeepSelected.
    pub keep_tool: bool,
    /// Firestorm RezUnderLandGroup, enabled by default.
    pub rez_under_land_group: bool,
    /// New prim size (FSBuildPrefs_X/Y/Zsize).
    pub new_prim_size: [f32; 3],
    /// New prim material (FSBuildPrefs_Material, LL_MCODE_WOOD).
    pub new_prim_material: u8,
    /// Land tool: 0..5 brush actions (RadioLandBrushAction), 6 select land.
    pub land_action: u8,
    /// LandBrushSize (m).
    pub land_brush_size: f32,
    /// LandBrushForce (0.1 .. 100, logarithmic slider).
    pub land_brush_force: f32,
    /// Actual-root pivot: primary linkset root, or primary prim in linked editing.
    pub actual_root: bool,
    /// SelectReflectionProbes: off at each start (settings.xml, Persist=0).
    #[serde(skip)]
    pub select_probes: bool,
    /// SelectOwnedOnly: clicks, rectangles and Area Search select only the
    /// objects we own (`passes_select_filters`).
    pub select_owned_only: bool,
    /// SelectMovableOnly: the same for the objects we may move.
    pub select_movable_only: bool,
    /// RectangleSelectInclusive, the Legacy Viewer's "Select By Surrounding"
    /// (on by default): the selection rectangle takes only the prims it
    /// wholly surrounds; off, any prim it touches (`rect_select`). The
    /// setting's own comment there describes the opposite of what it does.
    pub select_surrounding: bool,
    /// FSBuildToolDecimalPrecision: decimals of the Object tab numbers.
    pub decimal_precision: u8,
    /// CreateToolCopySelection / CopyCenters / CopyRotates.
    pub copy_selection: bool,
    pub copy_centers: bool,
    pub copy_rotates: bool,
    /// LastSelectedTree / LastSelectedGrass: species name, empty = random.
    pub last_tree: String,
    pub last_grass: String,
    /// FSToolboxExpanded: the tabs are shown under the tools.
    pub expanded: bool,
    /// ShowParcelOwners: parcels colored by owner on the ground.
    pub show_parcel_owners: bool,
    /// SyncMaterialSettings: normal / specular maps follow the diffuse UVs.
    pub sync_materials: bool,
    /// ShowPhysicsShapeInEdit (Features tab eye toggle).
    pub show_physics_shape: bool,
    /// FSCopyObjKeySeparator for "Copy UUID".
    pub copy_key_separator: String,
    /// LimitSelectDistance, on by default like the Legacy Viewer: a
    /// rectangle drawn from a skybox stops before the ground's objects.
    pub limit_select_distance: bool,
    /// MaxSelectDistance (m from the avatar).
    pub max_select_distance: f32,
    /// LimitDragDistance.
    pub limit_drag_distance: bool,
    /// MaxDragDistance (m from the start point of one move).
    pub max_drag_distance: f32,
    /// Aurora: size of the edit manipulators (arrows, rings, stretch
    /// handles) and of their pick areas, as a factor of the Legacy Viewer's
    /// fixed screen sizes, which it has no setting for (#1328).
    pub gizmo_scale: f32,
}

impl BuildSettings {
    /// `gizmo_scale` within the slider's range; an edited settings file
    /// falls back to the Legacy Viewer's size.
    pub fn gizmo_scale(&self) -> f32 {
        if self.gizmo_scale.is_finite() {
            self.gizmo_scale.clamp(*GIZMO_SCALE_RANGE.start(), *GIZMO_SCALE_RANGE.end())
        } else {
            1.0
        }
    }

    /// MaxSelectDistance while LimitSelectDistance is on.
    pub fn select_limit(&self) -> Option<f32> {
        self.limit_select_distance
            .then(|| sane_distance(self.max_select_distance, MAX_SELECT_DISTANCE))
    }

    /// MaxDragDistance while LimitDragDistance is on (LLManipTranslate::handleHover).
    pub fn drag_limit(&self) -> Option<f32> {
        self.limit_drag_distance
            .then(|| sane_distance(self.max_drag_distance, MAX_DRAG_DISTANCE))
    }
}

/// The spinners' range of the Legacy Viewer's preferences (0.001 .. 1024 m);
/// an edited settings file falls back to the default.
pub const DISTANCE_LIMIT_RANGE: std::ops::RangeInclusive<f32> = 0.001..=1024.0;
/// The manipulator size slider (`gizmo_scale`).
pub const GIZMO_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

fn sane_distance(value: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(*DISTANCE_LIMIT_RANGE.start(), *DISTANCE_LIMIT_RANGE.end())
    } else {
        default
    }
}

#[cfg(test)]
mod worn_tests;

/// LLManipTranslate / Rotate / Scale::canAffectSelection (Firestorm): the
/// simulator's effective flags apply before any local transform. Moving a
/// whole no-modify object is allowed; linked edits and stretching need Modify.
/// LLViewerObject::permMove / permModify inherit the root's effective flags.
pub fn can_transform(world: &World, idx: usize, linked: bool, mode: EditMode) -> bool {
    if !world.rlva_can_edit(idx) {
        return false;
    }
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let Some(root) = world.objects.get(root_of(world, idx)) else {
        return false;
    };
    root.update_flags & OBJECT_MOVE != 0 && (!(linked || mode == EditMode::Stretch) || root.update_flags & OBJECT_MODIFY != 0)
}

/// SelectOwnedOnly / SelectMovableOnly in LLSelectMgr::canSelectObject
/// (indra/newview/llselectmgr.cpp, originally LGPL 2.1). permYouOwner and
/// permMove read the root's flags; a permanent (pathfinding) object only
/// counts as movable in the agent's region (isPermanentEnforced). The
/// right-click menu's temporary selection skips this check, as
/// LLToolSelect::handleObjectSelection does with temp_select.
pub fn passes_select_filters(world: &World, s: &BuildSettings, idx: usize) -> bool {
    use crate::ui::context::flags::{AFFECTS_NAVMESH, OBJECT_MOVE, OBJECT_YOU_OWNER};
    if !s.select_owned_only && !s.select_movable_only {
        return true;
    }
    let Some(root) = world.objects.get(root_of(world, idx)) else {
        return false;
    };
    let flags = root.update_flags;
    let permanent_enforced = flags & AFFECTS_NAVMESH != 0 && world.main_region != Some(root.key.region);
    !(s.select_owned_only && flags & OBJECT_YOU_OWNER == 0) && !(s.select_movable_only && (flags & OBJECT_MOVE == 0 || permanent_enforced))
}

/// LLSelectMgr::highlightObjectAndFamily for the objects of ForceObjectSelect:
/// each one's linkset, its root first.
fn land_highlighted(world: &World) -> Vec<(usize, bool)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (region, local_id) in &world.land.highlight {
        let key = crate::world::objects::ObjKey {
            region: *region,
            local_id: *local_id,
        };
        let Some(idx) = world.objects.index_of(&key) else {
            continue;
        };
        let root_key = match world.objects.get(idx) {
            Some(o) if o.parent_id != 0 => crate::world::objects::ObjKey {
                region: *region,
                local_id: o.parent_id,
            },
            _ => key,
        };
        let root = world.objects.index_of(&root_key).unwrap_or(idx);
        if seen.insert(root) {
            out.push((root, true));
        }
        for child in world.objects.children_of(&root_key) {
            if seen.insert(*child) {
                out.push((*child, false));
            }
        }
    }
    out
}

/// The folder the selected roots were rezzed from, as their full
/// ObjectProperties give it (nil when unknown), when Take may send them back
/// there (handle_take): every root that knows its folder names the same
/// one, and that folder is still in the agent's inventory, outside the Trash
/// and the Library. None sends them to the Objects folder. The Inbox is
/// accepted, as the Legacy Viewer does.
pub(crate) fn take_folder(inv: &crate::world::inventory::Inventory, folders: impl IntoIterator<Item = Uuid>, trash: Uuid) -> Option<Uuid> {
    let mut chosen = None;
    for folder in folders.into_iter().filter(|f| !f.is_nil()) {
        match chosen {
            None => chosen = Some(folder),
            // two possible destinations: the default one
            Some(c) if c != folder => return None,
            Some(_) => {}
        }
    }
    let folder = chosen?;
    let known = inv.folders.get(&folder).is_some_and(|f| !f.library);
    let trashed = !trash.is_nil() && crate::world::inventory::actions::under(inv, folder, trash);
    (known && !trashed).then_some(folder)
}

#[cfg(test)]
mod hud_tests {
    use super::*;

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        world
    }

    #[test]
    fn typed_hud_moves_allow_no_modify_root_but_reject_no_modify_linked_part() {
        use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
        let mut world = world();
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let child = family(&world, root)[1];
        for idx in [root, child] {
            world.objects.get_mut(idx).unwrap().update_flags = OBJECT_MOVE;
        }
        let mut tool = BuildTool::default();
        let mut settings = BuildSettings::default();
        tool.click_select(&mut world, &settings, Some(root), false);
        tool.out.clear();
        let position = Vec3::new(0.0, 0.22, 0.16);
        tool.set_transform(&mut world, &settings, Some(position), None, None, true);
        assert_eq!(world.objects.get(root).unwrap().position, position);
        assert_eq!(tool.out.len(), 1);
        settings.edit_linked = true;
        tool.click_select(&mut world, &settings, Some(child), false);
        tool.out.clear();
        let before = world.objects.get(child).unwrap().position;
        tool.set_transform(&mut world, &settings, Some(position), None, None, true);
        assert_eq!(world.objects.get(child).unwrap().position, before);
        assert!(tool.out.is_empty());
        world.objects.get_mut(root).unwrap().update_flags |= OBJECT_MODIFY;
        tool.set_transform(&mut world, &settings, Some(position), None, None, true);
        assert!(world.objects.get(child).unwrap().position.abs_diff_eq(position, 1e-5));
        assert_eq!(tool.out.len(), 1);
    }

    #[test]
    fn hud_selection_uses_linkset_root_and_survives_avatar_distance() {
        let mut world = world();
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let child = family(&world, root)[1];
        let other = world.objects.iter().find(|(_, o)| o.key.local_id == 8241).map(|(i, _)| i).unwrap();
        assert_eq!(BuildTool::unit_of(&world, child, false), Some(world.objects.get(root).unwrap().key));
        assert!(BuildTool::unit_of(&world, other, false).is_none());
        let s = BuildSettings::default();
        let mut tool = BuildTool::default();
        assert!(tool.select_for_menu(&mut world, &s, child));
        tool.open_build(Tool::Edit);
        world.agent.position = Vec3::splat(1000.0);
        tool.prune(
            &world,
            &BuildSettings {
                limit_select_distance: true,
                ..Default::default()
            },
            Instant::now(),
        );
        assert!(tool.hud_selected(&world));
        tool.close(&mut world, &mut BuildSettings::default());
        assert!(tool.selection.is_empty());
    }

    #[test]
    fn typed_hud_position_is_attachment_local_for_roots_and_children() {
        let mut world = world();
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(32, false).full_id).unwrap();
        let child_root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let child = family(&world, child_root)[1];
        world.objects.get_mut(child_root).unwrap().rotation = Quat::from_rotation_x(0.7);
        for idx in [root, child] {
            let s = BuildSettings {
                edit_linked: idx == child,
                ..Default::default()
            };
            let mut tool = BuildTool::default();
            tool.click_select(&mut world, &s, Some(idx), false);
            tool.out.clear();
            let pos = Vec3::new(0.02, -0.24, 0.12);
            let rotation = Quat::from_rotation_x(0.3);
            let state = world.objects.get(idx).unwrap().state;
            tool.set_transform(&mut world, &s, Some(pos), None, Some(rotation), true);
            let o = world.objects.get(idx).unwrap();
            assert!(o.position.abs_diff_eq(pos, 1e-5));
            assert!(o.rotation.abs_diff_eq(rotation, 1e-5));
            assert_eq!(o.state, state);
            assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::Transform { updates, .. })]
                if updates[0].linked == (idx == root) && updates[0].position.unwrap().abs_diff_eq(pos, 1e-5)));
        }
    }

    #[test]
    fn shift_selection_does_not_mix_hud_and_world_coordinates() {
        let mut world = world();
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let ground = world
            .objects
            .iter()
            .find(|(idx, _)| {
                selectable(&world, *idx)
                    && crate::scene::Scene::object_transform(&world, *idx, Instant::now(), 0).is_some_and(|(_, _, hud)| !hud)
            })
            .map(|(i, _)| i)
            .unwrap();
        let mut tool = BuildTool::default();
        tool.click_select(&mut world, &BuildSettings::default(), Some(root), false);
        tool.click_select(&mut world, &BuildSettings::default(), Some(ground), true);
        assert_eq!(tool.selection, vec![world.objects.get(ground).unwrap().key]);
    }

    #[test]
    fn shift_face_selection_does_not_mix_hud_and_world_coordinates() {
        let mut world = world();
        let hud = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let ground = world
            .objects
            .iter()
            .find(|(idx, _)| {
                selectable(&world, *idx)
                    && crate::scene::Scene::object_transform(&world, *idx, Instant::now(), 0).is_some_and(|(_, _, hud)| !hud)
            })
            .unwrap()
            .0;
        let mut tool = BuildTool::default();
        tool.open_build(Tool::Edit);
        tool.edit_mode = EditMode::Face;
        tool.face_click(&mut world, Some(hud), Some(0), false);
        tool.face_click(&mut world, Some(ground), Some(0), true);
        assert_eq!(tool.selection, vec![world.objects.get(ground).unwrap().key]);
        assert_eq!(tool.sel_te(&world), [(ground, 0)]);
        tool.face_click(&mut world, Some(hud), Some(0), true);
        assert_eq!(tool.selection, vec![world.objects.get(hud).unwrap().key]);
        assert_eq!(tool.sel_te(&world), [(hud, 0)]);
    }
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings {
            legacy_window: false,
            layout: layout::Layout::default(),
            transform_expanded: true,
            color_picker: crate::ui::color_picker::Preferences::default(),
            grid_resolution: 0.5,
            grid_draw_size: 12.0,
            grid_sub_unit: false,
            grid_opacity: 0.7,
            grid_mode: 0,
            snap: true,
            rotation_step: 1.0,
            edit_linked: false,
            scale_uniform: false,
            scale_stretch_textures: true,
            follow_edit_object: false,
            show_highlight: true,
            create_shape: 0,
            keep_tool: false,
            rez_under_land_group: true,
            new_prim_size: [0.5; 3],
            new_prim_material: 3,
            land_action: 6,
            land_brush_size: 2.0,
            land_brush_force: 1.0,
            actual_root: false,
            select_probes: false,
            select_owned_only: false,
            select_movable_only: false,
            select_surrounding: true,
            decimal_precision: 5,
            copy_selection: false,
            copy_centers: true,
            copy_rotates: false,
            last_tree: String::new(),
            last_grass: String::new(),
            expanded: true,
            show_parcel_owners: false,
            sync_materials: false,
            show_physics_shape: false,
            copy_key_separator: ",".into(),
            limit_select_distance: true,
            max_select_distance: MAX_SELECT_DISTANCE,
            limit_drag_distance: true,
            max_drag_distance: MAX_DRAG_DISTANCE,
            gizmo_scale: 1.0,
        }
    }
}

impl BuildSettings {
    /// LLManip::updateGridSettings: finest subdivision allowed.
    pub fn max_subdivision(&self) -> f32 {
        if self.grid_sub_unit { 32.0 } else { 1.0 }
    }
}

/// The five tools of the floater (LLToolCamera, LLToolGrab,
/// LLToolCompTranslate, LLToolCompCreate, LLToolSelectLand).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Focus,
    Grab,
    Edit,
    Create,
    Land,
}

/// The Edit tool's radio: the three manipulators, LLToolFace and QToolAlign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Move,
    Rotate,
    Stretch,
    Face,
    Align,
}

impl EditMode {
    /// Uses the move / rotate / stretch manipulators.
    pub fn is_manip(self) -> bool {
        matches!(self, EditMode::Move | EditMode::Rotate | EditMode::Stretch)
    }
}

/// Focus tool radio (gCameraBtnZoom / Orbit / Pan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMode {
    Zoom,
    Orbit,
    Pan,
}

/// Grab tool radio (gGrabBtnVertical / gGrabBtnSpin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabMode {
    Move,
    Lift,
    Spin,
}

#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// The grid the manipulators snap to (LLSelectMgr::getGrid).
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub origin: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub world: bool,
}

/// Oriented bounds of the selection (LLSelectMgr::getBBoxOfSelection).
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub center: Vec3,
    pub rotation: Quat,
    pub half: Vec3,
}

/// The prims of an unlinked whole-object selection. The Legacy Viewer keeps
/// one LLSelectNode per prim, so after LLSelectMgr::sendDelink every former
/// child is still selected as its own object; Aurora selects roots only, so
/// each prim joins the selection once the simulator makes it a root.
#[derive(Debug)]
struct PendingUnlink {
    /// Every prim of the unlink, the former roots included.
    prims: Vec<ObjKey>,
    /// Prims still waiting for the update that makes them roots.
    waiting: Vec<ObjKey>,
    started: Instant,
}

pub struct BuildTool {
    wheel_selection: cycle::Wheel,
    /// Last inspected selection/mode; object display choices live in the world.
    material_preview_selection: HashSet<Uuid>,
    material_preview_mode: Option<bool>,
    /// The build floater is open (build mode).
    pub open: bool,
    pub tool: Tool,
    pub edit_mode: EditMode,
    pub focus_mode: FocusMode,
    pub grab_mode: GrabMode,
    /// Selected units (linksets, or prims with "Edit linked"), latest first.
    pub selection: Vec<ObjKey>,
    /// Whether the last frame edited HUDs; leaving that selection resets view zoom.
    hud_editing: bool,
    /// Last world pick, retained in the object's frame while it moves.
    point_pick: Option<(ObjKey, glam::DVec3)>,
    /// "Select face" mode: the faces chosen on each selected prim (missing
    /// = all its faces; LLSelectNode::isTESelected).
    pub faces: HashMap<ObjKey, Vec<u8>>,
    /// Last face picked (LLSelectMgr::getLastOperatedTE, ◄ ► part buttons).
    pub last_face: Option<u8>,
    /// Number of faces of the selected prims (filled by the app from the
    /// meshes / volumes: the texture entry always carries all of them).
    pub face_counts: HashMap<uuid::Uuid, usize>,
    /// Objects used as the reference grid (LLSelectMgr::mGridObjects).
    pub grid_objects: Vec<ObjKey>,
    /// Physics shape and material values of selected prims
    /// (ObjectPhysicsProperties / GetObjectPhysicsData).
    pub physics: HashMap<u32, aurora_net::build::PhysicsParams>,
    /// Materials actually received from the server, rather than defaults
    /// accompanying a GetObjectPhysicsData reply containing only shape type.
    pub physics_materials_known: HashSet<u32>,
    /// Land impact, weights and parcel capacity.
    pub costs: costs::Costs,
    /// Object inventories (Contents tab).
    pub contents: contents::Contents,
    pub replacement: replace::Replacement,
    pub grab: grab::Grab,
    pub align: align::Align,
    /// Everything the floater keeps between frames (tabs, clipboards,
    /// fields being typed, secondary floaters).
    pub ui: ui::FloaterState,
    /// Capability requests in flight (tag -> what for).
    pub caps: HashMap<u64, edits::CapPurpose>,
    next_tag: u64,
    /// The selection only lives while a right-click menu is open (the pie
    /// menu selection of LLViewerMenuHolderGL, dropped by deselectUnused
    /// when the menu closes outside build mode). Outlined like in build mode.
    pub menu_selection: bool,
    /// Selection mode of `selection`.
    pub selection_linked: bool,
    /// Ordered link instruction awaiting confirmed simulator parents.
    pending_link: Option<(Vec<Uuid>, Instant)>,
    /// Unlink awaiting the simulator's new roots.
    pending_unlink: Option<PendingUnlink>,
    /// ObjectProperties of selected objects.
    pub props: HashMap<Uuid, ObjectProps>,
    pub cam: Cam,
    /// Commands to send (drained by the app).
    pub out: Vec<NetCommand>,
    /// Client-only picker bindings, drained into the local preview registry.
    pub local_appearance_changes: Vec<(Uuid, Option<u8>, Option<LocalAppearance>)>,
    /// Latest color updates kept local until the pointer is released.
    color_preview: Vec<NetCommand>,
    pub manip: manip::Manip,
    pub land: land::LandTool,
    pub silhouettes: silhouette::Cache,
    /// Objects created / duplicated with FLAGS_CREATE_SELECTED: select them
    /// when they arrive (LLViewerObjectList). (Sent at, objects already flagged.)
    pending_create: Option<(Instant, HashSet<Uuid>)>,
    /// Selection rectangle being dragged (start, current in pixels, extend).
    pub rect: Option<((f32, f32), (f32, f32), bool)>,
    /// Compact section (Rights, Transform, Features, Materials, Contents, Shape).
    pub tab: usize,
    /// Last message for the floater's status line.
    pub status: String,
    last_frame: Option<Instant>,
}

impl Default for BuildTool {
    fn default() -> Self {
        BuildTool {
            wheel_selection: cycle::Wheel::default(),
            material_preview_selection: HashSet::new(),
            material_preview_mode: None,
            open: false,
            tool: Tool::Edit,
            edit_mode: EditMode::Move,
            focus_mode: FocusMode::Zoom,
            grab_mode: GrabMode::Move,
            selection: Vec::new(),
            hud_editing: false,
            point_pick: None,
            faces: HashMap::new(),
            last_face: None,
            face_counts: HashMap::new(),
            grid_objects: Vec::new(),
            physics: HashMap::new(),
            physics_materials_known: HashSet::new(),
            costs: costs::Costs::default(),
            contents: contents::Contents::default(),
            replacement: replace::Replacement::default(),
            grab: grab::Grab::default(),
            align: align::Align::default(),
            ui: ui::FloaterState::default(),
            caps: HashMap::new(),
            next_tag: 1,
            menu_selection: false,
            selection_linked: false,
            pending_link: None,
            pending_unlink: None,
            props: HashMap::new(),
            cam: Cam::default(),
            out: Vec::new(),
            local_appearance_changes: Vec::new(),
            color_preview: Vec::new(),
            manip: manip::Manip::default(),
            land: land::LandTool::default(),
            silhouettes: silhouette::Cache::default(),
            pending_create: None,
            rect: None,
            tab: 6,
            status: String::new(),
            last_frame: None,
        }
    }
}

pub fn root_of(world: &World, mut idx: usize) -> usize {
    for _ in 0..64 {
        let Some(o) = world.objects.get(idx) else {
            break;
        };
        if o.parent_id == 0 {
            break;
        }
        match world.objects.parent_of(o) {
            Some(p) if !world.objects.get(p).is_some_and(|po| po.is_avatar()) => idx = p,
            _ => break,
        }
    }
    idx
}

/// The three spaces a selection lives in (ESelectType of llselectmgr.h):
/// objects of one are never selected with those of another
/// (LLSelectMgr::canSelectObject).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectType {
    World,
    /// Worn on our avatar's body: edited relative to its attachment point.
    Attachment,
    Hud,
}

/// LLSelectMgr::getSelectTypeForObject: what `idx` hangs from.
pub fn select_type(world: &World, mut idx: usize) -> SelectType {
    for _ in 0..64 {
        let Some(o) = world.objects.get(idx) else { break };
        let Some(parent) = world.objects.parent_of(o) else { break };
        if world.objects.get(parent).is_some_and(|p| p.is_avatar()) {
            return if (31..=38).contains(&o.attachment_point()) {
                SelectType::Hud
            } else {
                SelectType::Attachment
            };
        }
        idx = parent;
    }
    SelectType::World
}

/// World objects and what our avatar wears, HUDs included; the attachments
/// of other avatars cannot be edited.
pub fn selectable(world: &World, idx: usize) -> bool {
    if !world.rlva_can_edit(idx) {
        return false;
    }
    let mut i = idx;
    for _ in 0..64 {
        let Some(o) = world.objects.get(i) else {
            return false;
        };
        if (o.is_client_preview() && !o.is_editable_preview())
            || o.is_avatar()
            || o.pcode != aurora_prim::params::LL_PCODE_VOLUME && !o.is_tree()
        {
            return false;
        }
        if o.parent_id == 0 {
            return true;
        }
        match world.objects.parent_of(o) {
            Some(p) => {
                if let Some(parent) = world.objects.get(p)
                    && parent.is_avatar()
                {
                    return parent.full_id == world.agent_id;
                }
                i = p;
            }
            None => return true,
        }
    }
    false
}

/// Editing uses the attachment point, never the avatar's world transform
/// (LLViewerJointAttachment / LLSelectMgr::sendMultipleUpdate).
pub fn edit_parent(world: &World, idx: usize, now: Instant) -> Option<(Vec3, Quat)> {
    let o = world.objects.get(idx)?;
    let parent_idx = world.objects.parent_of(o)?;
    let (p, r, _) = crate::scene::Scene::object_transform(world, idx, now, 0)?;
    if world.objects.get(parent_idx)?.is_avatar() {
        // the attachment point as posed now, on the body or in the HUD
        let rotation = (r * o.rotation.inverse()).normalize();
        Some((p - rotation * o.position, rotation))
    } else {
        crate::scene::Scene::object_transform(world, parent_idx, now, 0).map(|(p, r, _)| (p, r))
    }
}

/// Editable linkset, excluding seated avatars and everything they wear.
/// LLViewerObject::addThisAndNonJointChildren (indra/newview/llviewerobject.cpp).
pub fn family(world: &World, root: usize) -> Vec<usize> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() && out.len() < 4096 {
        if let Some(o) = world.objects.get(out[i])
            && !o.is_avatar()
        {
            out.extend(
                world
                    .objects
                    .children_of(&o.key)
                    .iter()
                    .copied()
                    .filter(|&child| world.objects.get(child).is_some_and(|o| !o.is_avatar())),
            );
        }
        i += 1;
    }
    out
}

/// Object indices of a selection with their highlight (true = root color):
/// whole linksets, or the selected prims with "Edit linked".
pub fn highlighted_of(world: &World, selection: &[ObjKey], linked: bool) -> Vec<(usize, bool)> {
    let mut out = Vec::new();
    for key in selection {
        let Some(idx) = world.objects.index_of(key) else {
            continue;
        };
        if linked {
            let root = root_of(world, idx) == idx;
            out.push((idx, root));
        } else {
            let root = root_of(world, idx);
            for p in family(world, root) {
                if !out.iter().any(|(i, _)| *i == p) {
                    out.push((p, p == root));
                }
            }
        }
    }
    out
}

/// Bounds of a selection, oriented like its primary (latest) object.
pub fn bounds_of(world: &World, selection: &[ObjKey], linked: bool, now: Instant) -> Option<Bounds> {
    let prims = highlighted_of(world, selection, linked);
    if prims.is_empty() {
        return None;
    }
    let primary = world.objects.index_of(selection.first()?)?;

    let (_, rot, _) = crate::scene::Scene::object_transform(world, primary, now, 0)?;
    let inv = rot.inverse();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (idx, _) in prims {
        let Some((p, r, _)) = crate::scene::Scene::object_transform(world, idx, now, 0) else {
            continue;
        };
        let h = world.visual_scale(idx) * 0.5;
        for c in 0..8 {
            let s = Vec3::new(
                if c & 1 != 0 { 1.0 } else { -1.0 },
                if c & 2 != 0 { 1.0 } else { -1.0 },
                if c & 4 != 0 { 1.0 } else { -1.0 },
            );
            let q = inv * (p + r * (h * s));
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    if lo.x > hi.x {
        return None;
    }
    Some(Bounds {
        center: rot * ((lo + hi) * 0.5),
        rotation: rot,
        half: (hi - lo) * 0.5,
    })
}

/// LLSelectMgr::getGrid: world grid, the selection's own frame ("local"),
/// or the frame of the objects chosen as reference ("Use Selection for
/// Grid").
pub fn grid_of(s: &BuildSettings, bounds: Option<&Bounds>, reference: Option<&Bounds>) -> Grid {
    let bounds = if s.grid_mode == 2 { reference } else { bounds };
    match (s.grid_mode, bounds) {
        (1 | 2, Some(b)) => Grid {
            origin: b.center,
            rotation: b.rotation,
            scale: b.half.max(Vec3::splat(0.001)),
            world: false,
        },
        _ => Grid {
            origin: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(s.grid_resolution.max(0.001)),
            world: true,
        },
    }
}

impl BuildTool {
    pub fn send(&mut self, c: BuildCmd) {
        self.out.push(NetCommand::Build(c));
    }

    /// The edit manipulator in use, with the modifier keys of the Edit tool
    /// (lltoolcomp: Ctrl = rotate, Ctrl+Shift = stretch).
    pub fn effective_mode(&self, m: Mods) -> EditMode {
        match (self.edit_mode, m.ctrl, m.shift) {
            (EditMode::Move, true, false) => EditMode::Rotate,
            (EditMode::Move, true, true) => EditMode::Stretch,
            (EditMode::Stretch, true, false) => EditMode::Rotate,
            (EditMode::Rotate, true, true) => EditMode::Stretch,
            (m, _, _) => m,
        }
    }

    /// Give object handles precedence over overlapping local pose handles.
    /// Floaters still own their input; this only arbitrates the world overlays.
    pub fn manipulator_at(&self, world: &World, settings: &BuildSettings, cursor: (f32, f32), mods: Mods) -> bool {
        let mode = self.effective_mode(mods);
        if !self.open
            || self.tool != Tool::Edit
            || !mode.is_manip()
            || !manip::can_transform_selection(world, &self.selection, self.selection_linked, mode)
        {
            return false;
        }
        manip::frame(
            world,
            settings,
            &self.selection,
            self.selection_linked,
            Instant::now(),
            self.manip.grid_ref.as_ref(),
        )
        .is_some_and(|frame| self.manip.pick(&self.cam, &frame, mode, cursor).is_some())
    }

    pub fn open_build(&mut self, tool: Tool) {
        self.open = true;
        // a right-click selection becomes the build selection
        self.menu_selection = false;
        self.set_tool(tool);
    }

    /// LLFloaterTools::onClose: everything is deselected, the tools reset
    /// (resetToolState: zoom, no spin / lift).
    pub fn close(&mut self, world: &mut World, s: &mut BuildSettings) {
        if self.replacement_busy() {
            self.status = aurora_i18n::tr!("replace-waiting-server").into();
            return;
        }
        self.cancel_replacement(world);
        self.cancel_color_picker(world);
        self.commit_color_edit();
        self.open = false;
        self.manip.cancel();
        self.land.cancel();
        self.grab_up();
        self.deselect_all(world);
        self.faces.clear();
        self.sync_hud_zoom(world);
        s.edit_linked = false;
        self.focus_mode = FocusMode::Zoom;
        self.grab_mode = GrabMode::Move;
        self.update_material_preview(world, s);
    }

    // ---- selection

    /// Object indices of the selection with their highlight (true = root
    /// color): whole linksets, or the selected prims with "Edit linked".
    pub fn highlighted(&self, world: &World) -> Vec<(usize, bool)> {
        let mut out = highlighted_of(world, &self.selection, self.selection_linked);
        let source = self
            .replacement
            .draft
            .as_ref()
            .and_then(|d| d.source)
            .or_else(|| self.replacement.job.as_ref().filter(|j| !j.stage.terminal()).map(|j| j.source));
        if let Some(idx) = source.and_then(|id| world.objects.index_of_uuid(&id))
            && !out.iter().any(|(i, _)| *i == idx)
        {
            out.push((idx, world.objects.get(idx).is_some_and(|o| o.parent_id == 0)));
        }
        out
    }

    /// Whether editing this selection can change the camera's focus object.
    pub fn edits_object(&self, world: &World, id: &Uuid) -> bool {
        if !self.open || self.tool != Tool::Edit {
            return false;
        }
        let Some(idx) = world.objects.index_of_uuid(id) else { return false };
        let Some(o) = world.objects.get(idx) else { return false };
        self.selection.contains(&o.key)
            || (!self.selection_linked
                && self.selection.iter().any(|key| {
                    world
                        .objects
                        .index_of(key)
                        .is_some_and(|selected| root_of(world, selected) == root_of(world, idx))
                }))
    }

    /// Local ids to send ObjectSelect / ObjectDeselect for, per region.
    fn wire_ids(&self, world: &World, keys: &[ObjKey], linked: bool) -> HashMap<RegionHandle, Vec<u32>> {
        let mut by_region: HashMap<RegionHandle, Vec<u32>> = HashMap::new();
        for key in keys {
            let Some(idx) = world.objects.index_of(key) else {
                continue;
            };
            let prims = if linked { vec![idx] } else { family(world, idx) };
            for p in prims {
                if let Some(o) = world.objects.get(p) {
                    by_region.entry(o.key.region).or_default().push(o.key.local_id);
                }
            }
        }
        by_region
    }

    pub fn is_selected(&self, key: &ObjKey) -> bool {
        self.selection.contains(key)
    }

    /// The unit a click on object `idx` selects.
    pub fn unit_of(world: &World, idx: usize, linked: bool) -> Option<ObjKey> {
        if !selectable(world, idx) {
            return None;
        }
        let i = if linked { idx } else { root_of(world, idx) };
        world.objects.get(i).map(|o| o.key)
    }

    /// Click on an object (lltoolselect): replace the selection, or with
    /// Shift / Ctrl add it or take it out.
    pub fn click_select(&mut self, world: &mut World, s: &BuildSettings, idx: Option<usize>, extend: bool) {
        if self.replacement_active() {
            return;
        }
        if self.selection_linked != s.edit_linked && !self.selection.is_empty() {
            self.deselect_all(world);
        }
        self.selection_linked = s.edit_linked;
        let unit = idx.and_then(|i| Self::unit_of(world, i, s.edit_linked));
        let Some(unit) = unit else {
            if !extend {
                self.deselect_all(world);
            }
            return;
        };
        // LLSelectMgr keeps world, worn and HUD selections in separate spaces.
        if let Some(i) = idx
            && !self.selection.is_empty()
            && select_type(world, i) != self.select_type(world)
        {
            self.deselect_all(world);
        }
        if extend {
            if self.is_selected(&unit) {
                self.deselect(world, &[unit]);
            } else {
                self.add(world, unit);
            }
        } else if !self.is_selected(&unit) || (s.edit_linked && self.selection.len() > 1) {
            // LLToolSelect::handleObjectSelection clears the other selected
            // parts when ignore_group is true, even if this part is selected.
            self.deselect_all(world);
            self.add(world, unit);
        } else {
            // clicking a selected object makes it the primary one
            self.selection.retain(|k| *k != unit);
            self.selection.insert(0, unit);
        }
    }

    /// Several objects selected at once, replacing the selection: the
    /// Area Search menu's selectObjectAndFamily on each row, the first one
    /// primary. Objects of another kind than the first (worn, HUD) are left
    /// out, as LLSelectMgr keeps them in separate selections.
    pub fn select_many(&mut self, world: &mut World, s: &BuildSettings, idxs: &[usize]) {
        if self.replacement_active() {
            return;
        }
        self.deselect_all(world);
        self.selection_linked = s.edit_linked;
        let kind = idxs.first().map(|&i| select_type(world, i));
        let mut seen = std::collections::HashSet::new();
        let units: Vec<ObjKey> = idxs
            .iter()
            .filter(|&&i| Some(select_type(world, i)) == kind && passes_select_filters(world, s, i))
            .filter_map(|&i| Self::unit_of(world, i, s.edit_linked))
            .filter(|u| seen.insert(*u))
            .collect();
        // one ObjectSelect per region and 254 prims (sendListToRegions)
        for (handle, ids) in self.wire_ids(world, &units, self.selection_linked) {
            self.send(BuildCmd::Select { handle, local_ids: ids });
        }
        self.selection = units;
    }

    /// Right click on an object: select it like a click while the menu is
    /// open (LLToolPie::handleRightClickPick -> LLToolSelect::handleObjectSelection
    /// with temp_select). Returns whether something got selected.
    pub fn select_for_menu(&mut self, world: &mut World, s: &BuildSettings, idx: usize) -> bool {
        if Self::unit_of(world, idx, s.edit_linked).is_none() {
            return false;
        }
        self.click_select(world, s, Some(idx), false);
        if !self.open {
            self.menu_selection = true;
        }
        !self.selection.is_empty()
    }

    /// The right-click menu closed: drop its selection unless the build
    /// tools took it over (LLSelectMgr::deselectUnused).
    pub fn release_menu_selection(&mut self, world: &World) {
        if std::mem::take(&mut self.menu_selection) && !self.open {
            self.deselect_all(world);
        }
    }

    pub fn add(&mut self, world: &World, unit: ObjKey) {
        if self.is_selected(&unit) {
            return;
        }
        self.selection.insert(0, unit);
        for (handle, ids) in self.wire_ids(world, &[unit], self.selection_linked) {
            self.send(BuildCmd::Select { handle, local_ids: ids });
        }
    }

    pub fn deselect(&mut self, world: &World, keys: &[ObjKey]) {
        for (handle, ids) in self.wire_ids(world, keys, self.selection_linked) {
            self.send(BuildCmd::Deselect { handle, local_ids: ids });
        }
        self.selection.retain(|k| !keys.contains(k));
    }

    pub fn deselect_all(&mut self, world: &World) {
        let keys = std::mem::take(&mut self.selection);
        for (handle, ids) in self.wire_ids(world, &keys, self.selection_linked) {
            self.send(BuildCmd::Deselect { handle, local_ids: ids });
        }
        self.manip.cancel();
    }

    /// "Edit linked" toggled: select the same objects the other way
    /// (demoteSelectionToIndividuals / promoteSelectionToRoot).
    pub fn set_edit_linked(&mut self, world: &mut World, s: &mut BuildSettings, on: bool) {
        if s.edit_linked == on {
            return;
        }
        let old: Vec<usize> = self.highlighted(world).into_iter().map(|(i, _)| i).collect();
        self.deselect_all(world);
        s.edit_linked = on;
        self.selection_linked = on;
        let mut seen = HashSet::new();
        for idx in old {
            if let Some(unit) = Self::unit_of(world, idx, on)
                && seen.insert(unit)
            {
                self.add(world, unit);
            }
        }
    }

    /// Selected roots (whole linksets), latest first.
    pub fn roots(&self, world: &World) -> Vec<usize> {
        let mut out = Vec::new();
        for key in &self.selection {
            if let Some(idx) = world.objects.index_of(key) {
                let r = root_of(world, idx);
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        out
    }

    /// Bounds of the selection, oriented like its primary object.
    pub fn bounds(&self, world: &World, now: Instant) -> Option<Bounds> {
        bounds_of(world, &self.selection, self.selection_linked, now)
    }

    pub fn hud_selected(&self, world: &World) -> bool {
        self.selection
            .first()
            .and_then(|key| world.objects.index_of(key))
            .and_then(|idx| crate::scene::Scene::object_transform(world, idx, Instant::now(), 0))
            .is_some_and(|(_, _, hud)| hud)
    }

    /// LLSelectMgr::updateSelectionCenter / resetAgentHUDZoom
    /// (indra/newview/llselectmgr.cpp): leaving HUD editing restores 100%.
    /// Track the transition so World > HUDs zoom still works outside editing.
    pub fn sync_hud_zoom(&mut self, world: &mut World) {
        let editing = self.open && self.hud_selected(world);
        if self.hud_editing && !editing {
            world.hud_zoom = 1.0;
        }
        self.hud_editing = editing;
    }

    /// The space of the selection (LLObjectSelection::getSelectType).
    pub fn select_type(&self, world: &World) -> SelectType {
        self.selection
            .first()
            .and_then(|key| world.objects.index_of(key))
            .map_or(SelectType::World, |idx| select_type(world, idx))
    }

    /// Something we wear is selected, on the body or in the HUD
    /// (LLObjectSelection::isAttachment): it cannot be deleted, taken,
    /// copied, linked or sold, and has no land impact.
    pub fn attachment_selected(&self, world: &World) -> bool {
        self.select_type(world) != SelectType::World
    }

    fn advance_link(&mut self, world: &mut World, now: Instant) {
        if let Some((members, started)) = &self.pending_link {
            if let Some(root) = members.first()
                && world.objects.remember_link_order(*root, &members[1..])
            {
                self.status = aurora_i18n::tr!("build-mod-objects-linked", value0 = format!("{}", members.len()));
                self.pending_link = None;
                self.costs_stale();
            } else if now.saturating_duration_since(*started) > Duration::from_secs(30) {
                self.pending_link = None;
                self.status = aurora_i18n::tr!("build-link-unconfirmed").into();
            }
        }
    }

    /// Prims of an unlink join the selection as the simulator makes them
    /// roots, after the selected former root (PendingUnlink).
    fn advance_unlink(&mut self, world: &World, now: Instant) {
        let Some(mut pending) = self.pending_unlink.take() else {
            return;
        };
        // a selection changed in the meantime is the user's: leave it alone
        if self.selection_linked || !pending.prims.iter().any(|k| self.is_selected(k)) {
            return;
        }
        let mut joined = Vec::new();
        pending.waiting.retain(|key| match world.objects.index_of(key) {
            None => false,
            Some(i) if root_of(world, i) != i => true,
            Some(_) => {
                joined.push(*key);
                false
            }
        });
        for key in &joined {
            if !self.is_selected(key) {
                self.selection.push(*key);
            }
        }
        // ObjectSelect brings the new roots' properties to the edit floater
        for (handle, ids) in self.wire_ids(world, &joined, false) {
            self.send(BuildCmd::Select { handle, local_ids: ids });
        }
        if !pending.waiting.is_empty() && now.saturating_duration_since(pending.started) < Duration::from_secs(30) {
            self.pending_unlink = Some(pending);
        }
    }

    /// Forget objects that are gone; drop a selection left far behind
    /// (LLSelectMgr::deselectAllIfTooFar).
    fn prune(&mut self, world: &World, s: &BuildSettings, now: Instant) {
        self.advance_unlink(world, now);
        // Parent changes are authoritative only after simulator object updates.
        // Former roots become one whole-object selection after a successful link.
        if !self.selection_linked
            && self
                .selection
                .iter()
                .any(|key| world.objects.index_of(key).is_some_and(|i| root_of(world, i) != i))
        {
            let roots = self.roots(world);
            self.selection = roots.into_iter().filter_map(|i| world.objects.get(i).map(|o| o.key)).collect();
        }
        self.selection.retain(|k| world.objects.index_of(k).is_some());
        if !self.selection.is_empty()
            && !self.attachment_selected(world)
            && !self.manip.dragging()
            && let Some(max) = s.select_limit()
            && let Some(b) = self.bounds(world, now)
            && b.center.distance(world.agent.position) > max
        {
            self.deselect_all(world);
            self.status = aurora_i18n::tr!("build-mod-selection-too-far-away-deselected").into();
        }
    }

    // ---- actions on the selection

    fn group_by_region(&self, world: &World, idxs: &[usize]) -> HashMap<RegionHandle, Vec<u32>> {
        let mut m: HashMap<RegionHandle, Vec<u32>> = HashMap::new();
        for &i in idxs {
            if let Some(o) = world.objects.get(i) {
                m.entry(o.key.region).or_default().push(o.key.local_id);
            }
        }
        m
    }

    /// Take's folder: where the roots were rezzed from (`take_folder`), else
    /// the Objects folder; a copy always goes to Objects (handle_take_copy).
    fn take_destination(&self, world: &World, roots: &[usize], copy: bool) -> Uuid {
        let rezzed_from = (!copy)
            .then(|| {
                let folders = roots
                    .iter()
                    .filter_map(|&r| world.objects.get(r))
                    .map(|o| self.props.get(&o.full_id).map_or(Uuid::nil(), |p| p.folder_id));
                take_folder(&world.inventory, folders, Self::folder_of_type(world, 14))
            })
            .flatten();
        rezzed_from.unwrap_or_else(|| Self::folder_of_type(world, 6))
    }

    fn folder_of_type(world: &World, t: i32) -> Uuid {
        world
            .inventory
            .folders
            .values()
            .find(|f| !f.library && f.info.type_default == t)
            .map(|f| f.info.id)
            .unwrap_or_default()
    }

    /// Delete (to the trash): roots only (LLSelectMgr::selectDelete).
    /// The active group goes in DeRezObject, like LLSelectMgr::packDeRezHeader
    /// (indra/newview/llselectmgr.cpp), regardless of the parcel group.
    pub fn delete(&mut self, world: &mut World) {
        if self.attachment_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if roots.is_empty() {
            return;
        }
        let trash = Self::folder_of_type(world, 14);
        let own = world.agent_id;
        let not_owned = roots
            .iter()
            .filter(|&&i| {
                world.objects.get(i).is_some_and(|o| {
                    let owner = self.props.get(&o.full_id).map_or(o.owner_id, |p| p.owner_id);
                    o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER == 0 && !owner.is_nil() && owner != own
                })
            })
            .count();
        for (handle, ids) in self.group_by_region(world, &roots) {
            self.send(BuildCmd::DeRez {
                handle,
                local_ids: ids,
                destination: derez::TRASH,
                destination_id: trash,
                group_id: world.groups.active,
            });
        }
        // LLSelectMgr::confirmDelete
        world.ui_sounds.push(crate::ui_sound::UiSound::ObjectDelete);
        self.status = if not_owned > 0 {
            aurora_i18n::tr!(
                "build-mod-deletion-requested-object-s-owned-by-others-likely-to",
                not_owned = format!("{}", not_owned)
            )
        } else {
            aurora_i18n::tr!("build-mod-object-s-sent-to-trash", value0 = format!("{}", roots.len()))
        };
        self.deselect_all(world);
    }

    /// Take into the folder the objects were rezzed from (handle_take), take
    /// a copy into the Objects folder (handle_take_copy; derez_objects).
    /// DeRezObject uses the active group (indra/newview/llviewermenu.cpp).
    pub fn take(&mut self, world: &mut World, copy: bool) {
        if self.attachment_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if roots.is_empty() {
            return;
        }
        let objects = self.take_destination(world, &roots, copy);
        let dest = if copy {
            derez::ACQUIRE_TO_AGENT_INVENTORY
        } else {
            derez::TAKE_INTO_AGENT_INVENTORY
        };
        // all in one region (AcquireErrorObjectSpan otherwise): the first one's
        let by_region = self.group_by_region(world, &roots);
        if by_region.len() > 1 {
            self.status = aurora_i18n::tr!("build-mod-cannot-take-objects-from-multiple-regions-at-once").into();
            return;
        }
        for (handle, ids) in by_region {
            self.send(BuildCmd::DeRez {
                handle,
                local_ids: ids,
                destination: dest,
                destination_id: objects,
                group_id: world.groups.active,
            });
        }
        // derez_objects
        world.ui_sounds.push(crate::ui_sound::UiSound::ObjectRezOut);
        if !copy {
            self.deselect_all(world);
        }
    }

    /// "Take as separate items" / "Take copies as separate items"
    /// (derez_objects_separate): one DeRezObject per root, each its own
    /// transaction and inventory item, with the active group
    /// (derez_objects, indra/newview/llviewermenu.cpp).
    pub fn take_separate(&mut self, world: &mut World, copy: bool) {
        if self.attachment_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if self.group_by_region(world, &roots).len() > 1 {
            self.status = aurora_i18n::tr!("build-mod-cannot-take-objects-from-multiple-regions-at-once").into();
            return;
        }
        let objects = self.take_destination(world, &roots, copy);
        let destination = if copy {
            derez::ACQUIRE_TO_AGENT_INVENTORY
        } else {
            derez::TAKE_INTO_AGENT_INVENTORY
        };
        for r in &roots {
            if let Some(o) = world.objects.get(*r) {
                let (handle, local_id) = (o.key.region, o.key.local_id);
                self.send(BuildCmd::DeRez {
                    handle,
                    local_ids: vec![local_id],
                    destination,
                    destination_id: objects,
                    group_id: world.groups.active,
                });
                world.ui_sounds.push(crate::ui_sound::UiSound::ObjectRezOut);
            }
        }
        if !copy {
            self.deselect_all(world);
        }
    }

    /// Ctrl+D: copies 0.5 m away, selected (LLSelectMgr::duplicate).
    /// Group selection follows packDuplicateHeader (indra/newview/llselectmgr.cpp).
    pub fn duplicate(&mut self, world: &mut World, s: &BuildSettings, offset: Vec3, select_copy: bool) {
        if self.attachment_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if roots.is_empty() {
            return;
        }
        if select_copy {
            self.expect_created(world);
            self.deselect_all(world);
        }
        let flags = if select_copy { FLAGS_CREATE_SELECTED } else { 0 };
        for (handle, ids) in self.group_by_region(world, &roots) {
            self.send(BuildCmd::Duplicate {
                handle,
                local_ids: ids,
                offset,
                flags,
                group_id: world.group_for_rez(s.rez_under_land_group),
            });
        }
    }

    /// Ctrl+L: link the selected linksets; the last selected becomes the root.
    pub fn link(&mut self, world: &mut World) {
        if self.attachment_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if roots.len() < 2 {
            self.status = aurora_i18n::tr!("build-mod-select-at-least-two-objects-to-link").into();
            return;
        }
        let first = world.objects.get(roots[0]).map(|o| (o.key.region, o.owner_id));
        let same = roots
            .iter()
            .all(|&r| world.objects.get(r).map(|o| (o.key.region, o.owner_id)) == first);
        if !same {
            self.status = aurora_i18n::tr!("build-mod-objects-to-link-must-have-the-same-owner-and").into();
            return;
        }
        let total: usize = roots.iter().map(|&r| family(world, r).len()).sum();
        if total > 256 {
            self.status = aurora_i18n::tr!("build-mod-too-many-prims-in-a-single-object-256", total = format!("{}", total));
            return;
        }
        let Some((handle, _)) = first else { return };
        let ids: Vec<u32> = roots.iter().filter_map(|&r| world.objects.get(r).map(|o| o.key.local_id)).collect();
        // New children precede the root's existing children (ObjectLink).
        // Retain this instruction only after all simulator parents agree,
        // so reordered UDP packets cannot redefine the newly created links.
        let mut expected = vec![roots[0]];
        expected.extend(roots.iter().skip(1).flat_map(|&r| family(world, r)));
        expected.extend(family(world, roots[0]).into_iter().skip(1));
        let members = expected
            .into_iter()
            .filter_map(|i| world.objects.get(i).map(|o| o.full_id))
            .collect();
        self.pending_link = Some((members, Instant::now()));
        self.send(BuildCmd::Link { handle, local_ids: ids });
        self.status = aurora_i18n::tr!("build-link-pending").into();
        self.costs_stale();
    }

    /// Ctrl+Shift+L: every selected prim (SEND_INDIVIDUALS).
    pub fn unlink(&mut self, world: &mut World) {
        if self.attachment_selected(world) {
            return;
        }
        let prims: Vec<usize> = self.highlighted(world).into_iter().map(|(i, _)| i).collect();
        if prims.is_empty() {
            return;
        }
        for (handle, ids) in self.group_by_region(world, &prims) {
            self.send(BuildCmd::Delink { handle, local_ids: ids });
        }
        // with "Edit linked" the selection already holds each prim
        if !self.selection_linked {
            let keys: Vec<ObjKey> = highlighted_of(world, &self.selection, false)
                .into_iter()
                .filter_map(|(i, _)| world.objects.get(i).map(|o| o.key))
                .collect();
            self.pending_unlink = Some(PendingUnlink {
                waiting: keys.iter().copied().filter(|k| !self.is_selected(k)).collect(),
                prims: keys,
                started: Instant::now(),
            });
        }
        self.status = aurora_i18n::tr!("build-mod-object-unlinked").into();
        self.costs_stale();
    }

    /// Ctrl+Z / Ctrl+Y: the simulator's undo history of the selected roots.
    pub fn undo(&mut self, world: &World, redo: bool) {
        let units: Vec<usize> = if self.selection_linked {
            self.highlighted(world).into_iter().map(|(i, _)| i).collect()
        } else {
            self.roots(world)
        };
        let mut by_region: HashMap<RegionHandle, Vec<Uuid>> = HashMap::new();
        for i in units {
            if let Some(o) = world.objects.get(i) {
                by_region.entry(o.key.region).or_default().push(o.full_id);
            }
        }
        for (handle, ids) in by_region {
            let group_id = world.groups.active;
            self.send(if redo {
                BuildCmd::Redo { handle, ids, group_id }
            } else {
                BuildCmd::Undo { handle, ids, group_id }
            });
        }
    }

    /// Remember the objects already flagged "create selected" so that only
    /// new ones get selected when they arrive.
    pub fn expect_created(&mut self, world: &World) {
        let flagged = world
            .objects
            .iter()
            .filter(|(_, o)| o.update_flags & FLAGS_CREATE_SELECTED != 0)
            .map(|(_, o)| o.full_id)
            .collect();
        self.pending_create = Some((Instant::now(), flagged));
    }

    fn pick_up_created(&mut self, world: &World, s: &BuildSettings) {
        let Some((at, known)) = &self.pending_create else {
            return;
        };
        if at.elapsed().as_secs_f32() > 15.0 {
            self.pending_create = None;
            return;
        }
        // FLAGS_CREATE_SELECTED is addressed to the creating client, as in
        // LLViewerObjectList::processObjectUpdate. OwnerID is absent from
        // ordinary updates until sound / particles or ObjectProperties arrive.
        let new: Vec<usize> = world
            .objects
            .iter()
            .filter(|(_, o)| o.parent_id == 0 && o.update_flags & FLAGS_CREATE_SELECTED != 0 && !known.contains(&o.full_id))
            .map(|(i, _)| i)
            .collect();
        if new.is_empty() {
            return;
        }
        if let Some((_, known)) = self.pending_create.as_mut() {
            for &i in &new {
                if let Some(o) = world.objects.get(i) {
                    known.insert(o.full_id);
                }
            }
        }
        self.selection_linked = s.edit_linked;
        for i in new {
            if let Some(unit) = Self::unit_of(world, i, false) {
                self.add(world, unit);
            }
        }
    }

    /// LLToolSelectRect::handleRectangleSelection (indra/newview/llglsandbox.cpp,
    /// originally LGPL 2.1): each prim, children included, whose drawn shape
    /// the dragged rectangle takes (in front of the camera, within the
    /// selection distance). A prim's box decides when it is wholly inside or
    /// outside; across an edge its vertices do, all of them with "Select by
    /// surrounding", one otherwise (LLViewerCamera::areVertsVisible). Outside
    /// linked editing a prim brings its linkset, as
    /// LLSelectMgr::updateSelectionSilhouette roots the rect selection.
    /// Without `scene` (tests) or geometry yet, a prim across the edge is
    /// taken only without "Select by surrounding".
    fn rect_select(&mut self, world: &mut World, s: &BuildSettings, scene: Option<&Scene>, a: (f32, f32), b: (f32, f32), extend: bool) {
        if (a.0 - b.0).abs() < 3.0 && (a.1 - b.1).abs() < 3.0 {
            return; // a click, already handled
        }
        let rect = egui::Rect::from_two_pos(egui::pos2(a.0, a.1), egui::pos2(b.0, b.1));
        let now = Instant::now();
        self.selection_linked = s.edit_linked;
        let mut units = Vec::new();
        let mut seen = HashSet::new();
        for (idx, o) in world.objects.iter() {
            if o.is_avatar() || (!s.select_probes && o.extra.reflection_probe.is_some()) {
                continue;
            }
            let Some((p, rot, hud)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if hud != self.cam.hud.is_some() || (!hud && s.select_limit().is_some_and(|max| p.distance(world.agent.position) > max)) {
                continue;
            }
            // what avatars wear is left out of a rectangle drawn in the
            // world (LLToolSelectRect::handleRectangleSelection skips isAttachment)
            if !hud && select_type(world, idx) != SelectType::World {
                continue;
            }
            let drawn = scene
                .and_then(|scene| scene.drawn_geometry(world, idx, now))
                .filter(|(_, g)| g.pick_faces.iter().flatten().any(|f| !f.positions.is_empty()));
            let (model, min, max) = match &drawn {
                Some((model, g)) => (*model, g.min, g.max),
                None => {
                    let scale = world.visual_scale_at(idx, now);
                    (
                        glam::Mat4::from_scale_rotation_translation(scale, rot, p),
                        Vec3::splat(-0.5),
                        Vec3::splat(0.5),
                    )
                }
            };
            let taken = match self.cam.box_in_rect(model, min, max, rect) {
                RectCover::Inside => true,
                RectCover::Outside => false,
                RectCover::Across => drawn.as_ref().map_or(!s.select_surrounding, |(model, g)| {
                    let vertices = g.pick_faces.iter().flatten().flat_map(|f| &f.positions);
                    verts_in_rect(
                        vertices.map(|v| self.cam.in_rect(model.transform_point3(Vec3::from(*v)), rect)),
                        s.select_surrounding,
                    )
                }),
            };
            if taken
                && passes_select_filters(world, s, idx)
                && let Some(u) = Self::unit_of(world, idx, s.edit_linked)
                && seen.insert(u)
            {
                units.push(u);
            }
        }
        if !extend && units.is_empty() {
            return;
        }
        for u in units.into_iter().take(1024) {
            self.add(world, u);
        }
    }

    pub fn on_properties(&mut self, props: Vec<ObjectProps>) {
        for p in props {
            // the family reply has no creator: keep a full one we already have
            if !p.full && self.props.get(&p.object_id).is_some_and(|o| o.full) {
                continue;
            }
            self.props.insert(p.object_id, p);
        }
        if self.props.len() > 4096 {
            self.props.clear();
        }
    }

    // ---- per frame

    /// Mouse, drags and land brush, once per frame (before the UI).
    pub fn update(&mut self, world: &mut World, s: &BuildSettings, cursor: (f32, f32), over_ui: bool, mods: Mods) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.016)
            .clamp(0.001, 0.25);
        self.last_frame = Some(now);
        self.update_replacement(world, now);
        self.advance_bulk_permissions(world, now);
        self.advance_link(world, now);
        self.update_material_preview(world, s);
        if !self.open || self.replacement_active() {
            self.sync_hud_zoom(world);
            return;
        }
        self.prune(world, s, now);
        self.pick_up_created(world, s);
        self.sync_hud_zoom(world);
        self.refresh_costs(world);
        // the reference grid follows its objects
        self.grid_objects.retain(|k| world.objects.index_of(k).is_some());
        self.manip.grid_ref = if self.grid_objects.is_empty() {
            None
        } else {
            bounds_of(world, &self.grid_objects, false, now)
        };
        match self.tool {
            Tool::Edit => {
                let mode = self.effective_mode(mods);
                if mode == EditMode::Align {
                    self.align_hover(world, cursor, mods.shift);
                }
                if mode.is_manip() {
                    let mut cmds = Vec::new();
                    self.manip.update(
                        world,
                        s,
                        &self.cam,
                        &self.selection,
                        self.selection_linked,
                        mode,
                        cursor,
                        over_ui,
                        mods,
                        now,
                        &mut cmds,
                    );
                    for c in cmds {
                        self.send(c);
                    }
                }
                if let Some(r) = self.rect.as_mut() {
                    r.1 = cursor;
                }
            }
            Tool::Grab => self.grab_hover(world, cursor, mods),
            Tool::Land => {
                let mut cmds = Vec::new();
                self.land.update(world, s, &self.cam, cursor, over_ui, dt, &mut cmds);
                for c in cmds {
                    self.send(c);
                }
            }
            Tool::Create | Tool::Focus => {
                if let Some(r) = self.rect.as_mut() {
                    r.1 = cursor;
                }
            }
        }
    }

    /// Left press in the world while building. `hit` is the picked surface
    /// point, `target` the object there and `surface` its face under the
    /// cursor (scene picking). Returns whether it was used. The Focus tool
    /// is the app's (camera).
    #[allow(clippy::too_many_arguments)]
    pub fn mouse_down(
        &mut self,
        world: &mut World,
        s: &mut BuildSettings,
        hit: Option<Vec3>,
        target: Option<usize>,
        surface: Option<aurora_net::TouchSurface>,
        cursor: (f32, f32),
        mods: Mods,
    ) -> bool {
        if !self.open {
            return false;
        }
        if self.replacement.draft.as_ref().is_some_and(|d| d.picking) {
            self.pick_replacement(world, target);
            return true;
        }
        if self.replacement_active() {
            return true;
        }
        let now = Instant::now();
        self.point_pick = target.zip(hit).and_then(|(idx, point)| {
            let o = world.objects.get(idx)?;
            let (at, rot, _) = Scene::object_transform(world, idx, now, 0)?;
            Some((o.key, crate::world::lookat::local_offset(point, at, rot)))
        });
        // what a click may select; Create still places on any surface
        let pick = target.filter(|&idx| passes_select_filters(world, s, idx));
        match self.tool {
            Tool::Focus => false,
            Tool::Grab => {
                if let (Some(idx), Some(point)) = (target, hit) {
                    self.grab_down(world, idx, point, surface.unwrap_or_default(), cursor, mods);
                } else if !mods.shift {
                    self.deselect_all(world);
                }
                true
            }
            Tool::Edit => {
                let mode = self.effective_mode(mods);
                match mode {
                    EditMode::Face => {
                        let face = surface.and_then(|s| u8::try_from(s.face).ok());
                        self.face_click(world, pick, face, mods.shift);
                        true
                    }
                    EditMode::Align => {
                        if !self.align_apply(world) {
                            // QToolAlign::pickCallback: whole linksets
                            s.edit_linked = false;
                            self.click_select(world, s, pick, mods.shift);
                        }
                        true
                    }
                    _ => {
                        if self
                            .manip
                            .try_grab(world, s, &self.cam, &self.selection, self.selection_linked, mode, cursor, now)
                        {
                            return true;
                        }
                        // not on a handle: select, or drag a selection rectangle
                        // (LLToolCompTranslate -> LLToolSelectRect)
                        let extend = mods.shift || mods.ctrl;
                        self.click_select(world, s, pick, extend);
                        self.rect = Some((cursor, cursor, extend));
                        true
                    }
                }
            }
            Tool::Create => {
                if mods.shift || mods.ctrl {
                    // Shift / Ctrl click selects instead (LLToolCompCreate)
                    self.click_select(world, s, pick, true);
                    return true;
                }
                if s.copy_selection {
                    self.place_copy(world, s, hit, target, cursor);
                } else {
                    self.place(world, s, hit, target, cursor);
                }
                true
            }
            Tool::Land => {
                let mut cmds = Vec::new();
                let used = self.land.mouse_down(world, s, &self.cam, hit, target, cursor, &mut cmds);
                for c in cmds {
                    self.send(c);
                }
                if let Some(m) = self.land.take_message() {
                    world.system_message(m);
                }
                used
            }
        }
    }

    /// LLSelectMgr::updatePointAt / LLAgentCamera::setPointAt: selected
    /// world objects only; attachments and avatars never raise the arm.
    pub fn pointing_target(&self, world: &World, now: Instant) -> Option<crate::world::lookat::Target> {
        if !(self.open || self.menu_selection) || self.select_type(world) != SelectType::World {
            return None;
        }
        let picked = self.point_pick.and_then(|(key, offset)| {
            let idx = world.objects.index_of(&key)?;
            let selected = Self::unit_of(world, idx, self.selection_linked).is_some_and(|unit| self.is_selected(&unit));
            (selected || self.tool == Tool::Grab && self.grab.state.is_some()).then_some((idx, offset))
        });
        let (idx, offset) = picked.or_else(|| Some((world.objects.index_of(self.selection.first()?)?, glam::DVec3::ZERO)))?;
        let o = world.objects.get(idx)?;
        if o.is_avatar() || select_type(world, idx) != SelectType::World || Scene::object_transform(world, idx, now, 0)?.2 {
            return None;
        }
        Some(crate::world::lookat::Target {
            kind: if self.tool == Tool::Grab && self.grab.state.is_some() {
                crate::world::pointat::GRAB
            } else {
                crate::world::pointat::SELECT
            },
            object: Some(o.full_id),
            offset,
        })
    }

    /// LLToolFace::pickCallback: a click selects that face of that prim
    /// alone; Shift adds it or takes it out.
    fn face_click(&mut self, world: &mut World, target: Option<usize>, face: Option<u8>, shift: bool) {
        let unit = target.and_then(|i| Self::unit_of(world, i, true));
        let (Some(unit), Some(face)) = (unit, face) else {
            if !shift {
                self.deselect_all(world);
                self.faces.clear();
            }
            return;
        };
        let Some(idx) = world.objects.index_of(&unit) else { return };
        if face as usize >= self.num_faces(world, idx) {
            return;
        }
        // LLSelectMgr keeps world, worn and HUD selections in separate spaces.
        if shift && !self.selection.is_empty() && self.select_type(world) != select_type(world, idx) {
            self.deselect_all(world);
            self.faces.clear();
        }
        if !shift {
            if !self.selection_linked || self.selection != vec![unit] {
                self.deselect_all(world);
                self.selection_linked = true;
                self.add(world, unit);
            }
            self.faces.clear();
            self.faces.insert(unit, vec![face]);
        } else {
            if !self.selection_linked {
                // Entering face selection from a whole linkset keeps all its
                // selected prims and faces (LLToolFace::pickCallback).
                let prims: Vec<_> = self
                    .sel_prims(world)
                    .into_iter()
                    .filter_map(|i| Self::unit_of(world, i, true))
                    .collect();
                self.deselect_all(world);
                self.faces.clear();
                self.selection_linked = true;
                for prim in prims {
                    self.add(world, prim);
                }
            }
            if !self.is_selected(&unit) {
                self.add(world, unit);
                self.faces.insert(unit, vec![face]);
            } else {
                let n = world.objects.index_of(&unit).map(|i| self.num_faces(world, i)).unwrap_or(1);
                let list = self.faces.entry(unit).or_insert_with(|| (0..n as u8).collect());
                if let Some(pos) = list.iter().position(|f| *f == face) {
                    list.remove(pos);
                    if list.is_empty() {
                        self.faces.remove(&unit);
                        self.deselect(world, &[unit]);
                    }
                } else {
                    list.push(face);
                }
            }
        }
        self.last_face = Some(face);
    }

    /// `scene` gives the selection rectangle the drawn vertices of the prims.
    pub fn mouse_up(&mut self, world: &mut World, s: &BuildSettings, scene: Option<&Scene>) {
        if let Some((a, b, extend)) = self.rect.take() {
            self.rect_select(world, s, scene, a, b, extend);
        }
        let mut cmds = Vec::new();
        self.manip.release(world, s, self.selection_linked, &mut cmds);
        self.land.mouse_up();
        for c in cmds {
            self.send(c);
        }
        self.grab_up();
    }

    /// The region under a point and its offset.
    pub(crate) fn region_at(world: &World, point: Vec3) -> Option<(RegionHandle, Vec3)> {
        world.regions.keys().find_map(|&h| {
            let off = world.region_offset(h)?;
            let r = world.regions.get(&h)?;
            let (lx, ly) = (point.x - off.x, point.y - off.y);
            (lx >= 0.0 && ly >= 0.0 && lx < r.heightmap.size_x as f32 && ly < r.heightmap.size_y as f32).then_some((h, off))
        })
    }

    /// The ray of LLToolPlacer::raycastForNewObjPos (region-local): from the
    /// camera to the land point, or through the object hit.
    #[allow(clippy::too_many_arguments)]
    fn placement_ray(
        &self,
        world: &World,
        s: &BuildSettings,
        point: Vec3,
        target: Option<usize>,
        cursor: (f32, f32),
        off: Vec3,
    ) -> (Vec3, Vec3, Uuid, bool) {
        // flora and avatars are not surfaces to build on
        let target = target.filter(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar() && !o.is_tree()));
        let (_, dir) = self.cam.ray(cursor.0, cursor.1);
        let ray_start = self.cam.eye - off + self.cam.at * (0.1 + 0.01);
        match target.and_then(|i| world.objects.get(i)) {
            Some(o) => {
                // Without the limit, the Legacy Viewer's 129 m ray stops short
                // of a surface seen from farther away: reach past the hit.
                let length = s
                    .select_limit()
                    .unwrap_or_else(|| UNLIMITED_PLACEMENT_RAY.max((point - off).distance(ray_start) + 1.0));
                (ray_start, ray_start + dir * length, o.full_id, false)
            }
            None => (ray_start, point - off, Uuid::nil(), true),
        }
    }

    /// Create tool click: ObjectAdd (LLToolPlacer::addObject).
    fn place(&mut self, world: &mut World, s: &BuildSettings, hit: Option<Vec3>, target: Option<usize>, cursor: (f32, f32)) {
        let Some(point) = hit else {
            return;
        };
        if s.select_limit().is_some_and(|max| point.distance(world.agent.position) > max) {
            self.status = aurora_i18n::tr!("build-mod-too-far-away-to-create-an-object-here").into();
            return;
        }
        let Some((handle, off)) = Self::region_at(world, point) else {
            return;
        };
        let (ray_start, ray_end, target_id, bypass) = self.placement_ray(world, s, point, target, cursor, off);
        let random = Uuid::new_v4().as_u128() as u32;
        let n = shapes::shapes().len();
        let plant = s.create_shape == n || s.create_shape == n + 1;
        let (pcode, shape, rotation, state, scale, flags) = if plant {
            // trees and grass: not created selected (LLToolPlacer)
            let tree = s.create_shape == n;
            let state = if tree {
                shapes::plant_species(&shapes::tree_species(), &s.last_tree, random)
            } else {
                shapes::plant_species(&shapes::grass_species(), &s.last_grass, random)
            };
            let scale = if tree {
                Vec3::from_array(s.new_prim_size)
            } else {
                // random grass patch: 10..30 × 10..30 × 1..3
                let r = |k: u32| ((random >> k) & 0xff) as f32 / 255.0;
                Vec3::new(10.0 + r(0) * 20.0, 10.0 + r(8) * 20.0, 1.0 + r(16) * 2.0)
            };
            let pcode = if tree {
                aurora_prim::params::LL_PCODE_LEGACY_TREE
            } else {
                aurora_prim::params::LL_PCODE_LEGACY_GRASS
            };
            (pcode, aurora_prim::RawShape::default(), glam::Quat::IDENTITY, state, scale, 0)
        } else {
            let sh = &shapes::shapes()[s.create_shape.min(n - 1)];
            (
                aurora_prim::params::LL_PCODE_VOLUME,
                sh.raw(),
                sh.rotation(),
                0,
                Vec3::from_array(s.new_prim_size).clamp(Vec3::splat(MIN_PRIM_SCALE), Vec3::splat(MAX_PRIM_SCALE)),
                FLAGS_CREATE_SELECTED,
            )
        };
        let p = aurora_net::build::NewPrim {
            handle,
            group_id: world.group_for_rez(s.rez_under_land_group),
            pcode,
            material: s.new_prim_material,
            add_flags: flags,
            shape,
            ray_start,
            ray_end,
            ray_target: target_id,
            ray_end_is_intersection: false,
            bypass_raycast: bypass,
            scale,
            rotation,
            state,
        };
        self.deselect_all(world);
        if !plant {
            self.expect_created(world);
        }
        self.send(BuildCmd::Add(Box::new(p)));
        // LLToolPlacer::addObject
        world.ui_sounds.push(crate::ui_sound::UiSound::ObjectCreate);
        if !s.keep_tool {
            self.set_tool(Tool::Edit);
            self.edit_mode = EditMode::Move;
        }
    }

    /// "Copy selection": ObjectDuplicateOnRay of the selected roots
    /// where the ray hits (LLToolPlacer::addDuplicate).
    fn place_copy(&mut self, world: &mut World, s: &BuildSettings, hit: Option<Vec3>, target: Option<usize>, cursor: (f32, f32)) {
        let Some(point) = hit else { return };
        let roots = self.roots(world);
        if roots.is_empty() {
            self.status = aurora_i18n::tr!("build-mod-select-the-objects-to-copy-first").into();
            return;
        }
        // raycastForNewObjPos refuses the duplicate's surface too
        if s.select_limit().is_some_and(|max| point.distance(world.agent.position) > max) {
            self.status = aurora_i18n::tr!("build-mod-too-far-away-to-create-an-object-here").into();
            return;
        }
        let Some((_, off)) = Self::region_at(world, point) else { return };
        let (ray_start, ray_end, ray_target, bypass) = self.placement_ray(world, s, point, target, cursor, off);
        let mut by_region: HashMap<RegionHandle, Vec<u32>> = HashMap::new();
        for r in roots {
            if let Some(o) = world.objects.get(r) {
                by_region.entry(o.key.region).or_default().push(o.key.local_id);
            }
        }
        for (handle, local_ids) in by_region {
            self.send(BuildCmd::DuplicateOnRay {
                handle,
                local_ids,
                group_id: world.group_for_rez(s.rez_under_land_group),
                ray_start,
                ray_end,
                ray_target,
                bypass_raycast: bypass,
                copy_centers: s.copy_centers,
                copy_rotates: s.copy_rotates,
            });
        }
        world.ui_sounds.push(crate::ui_sound::UiSound::ObjectCreate);
        if !s.keep_tool {
            self.set_tool(Tool::Edit);
            self.edit_mode = EditMode::Move;
        }
    }

    /// The outlined prims: the selection while the tools show it, and the
    /// objects About Land's "Show" highlights (LLSelectMgr's highlighted
    /// objects, drawn whether the tools are open or not).
    fn outlined(&self, world: &World, s: &BuildSettings) -> Vec<(usize, bool)> {
        let mut prims = if self.face_mode() || !(self.open || self.menu_selection) || !s.show_highlight {
            Vec::new()
        } else {
            self.color_highlighted(world)
        };
        for p in land_highlighted(world) {
            if !prims.iter().any(|(i, _)| *i == p.0) {
                prims.push(p);
            }
        }
        prims
    }

    /// Selected mesh / sculpted prims the renderer outlines as wireframes
    /// (object index -> root color); the others get their edges drawn here.
    pub fn wire_selection(&self, world: &World, s: &BuildSettings) -> HashMap<usize, bool> {
        self.outlined(world, s)
            .into_iter()
            .filter(|(i, _)| world.objects.get(*i).is_some_and(|o| o.volume.sculpt.is_some()))
            .collect()
    }

    pub fn silhouette_selection(&self, world: &World, s: &BuildSettings) -> HashSet<usize> {
        self.outlined(world, s)
            .into_iter()
            .filter_map(|(i, _)| world.objects.get(i).is_some_and(|o| o.is_flexible()).then_some(i))
            .collect()
    }

    /// LLToolCompCreate uses Shift / Ctrl to select instead of placing.
    /// The local loading hologram is an Aurora placement aid (#854).
    pub fn creation_preview_enabled(&self, s: &BuildSettings, over_ui: bool, mods: Mods) -> bool {
        self.open
            && self.tool == Tool::Create
            && !self.replacement_active()
            && !self.ui.color_picker.pipette
            && !s.copy_selection
            && s.create_shape < shapes::shapes().len()
            && !over_ui
            && !(mods.shift || mods.ctrl || mods.alt)
    }

    fn face_mode(&self) -> bool {
        self.open && self.tool == Tool::Edit && self.edit_mode == EditMode::Face
    }

    /// LLSelectMgr's TE mode: the selected surfaces, not whole-object outlines.
    pub fn face_selection(&self, world: &World, s: &BuildSettings) -> HashMap<usize, u64> {
        if !self.face_mode() || !s.show_highlight {
            return HashMap::new();
        }
        self.sel_prims(world)
            .into_iter()
            .map(|idx| {
                let mask = self
                    .sel_faces(world, idx)
                    .into_iter()
                    .fold(0, |mask, face| mask | 1u64.checked_shl(face.into()).unwrap_or(0));
                (idx, mask)
            })
            .collect()
    }

    /// Selection outlines, manipulators, land brush and selection rectangle
    /// over the scene.
    pub fn draw_overlay(
        &mut self,
        ctx: &egui::Context,
        pal: &crate::theme::Palette,
        world: &World,
        scene: &crate::scene::Scene,
        s: &BuildSettings,
        mods: Mods,
    ) {
        if !self.open && !self.menu_selection && world.land.highlight.is_empty() {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("build_overlay")));
        let cam = self.cam;
        let mut p = draw::Painter3d::new(painter, &cam, *pal);
        let now = Instant::now();
        let prims = self.outlined(world, s);
        if !prims.is_empty() {
            self.silhouettes.draw(&p, world, scene, &prims);
        }
        if !self.open && !self.menu_selection {
            return;
        }
        if !self.open || self.replacement_active() || self.ui.color_picker.pipette {
            return;
        }
        match self.tool {
            Tool::Edit => {
                let mode = self.effective_mode(mods);
                if mode == EditMode::Align {
                    self.align_draw(&mut p, world);
                } else if mode != EditMode::Face {
                    self.manip.draw(&mut p, world, s, &self.selection, self.selection_linked, mode, now);
                }
            }
            Tool::Land => {
                self.land.draw(&mut p, world, s);
            }
            Tool::Create | Tool::Grab | Tool::Focus => {}
        }
        if let Some((a, b, _)) = self.rect
            && (a.0 - b.0).abs() + (a.1 - b.1).abs() > 3.0
        {
            let k = 1.0 / cam.ppp;
            let r = egui::Rect::from_two_pos(egui::pos2(a.0 * k, a.1 * k), egui::pos2(b.0 * k, b.1 * k));
            p.painter.rect_filled(r, 0.0, draw::rgba(1.0, 1.0, 1.0, 0.06));
            p.painter.rect_stroke(
                r,
                0.0,
                egui::Stroke::new(1.0, draw::rgba(1.0, 1.0, 1.0, 0.8)),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// Apply typed values from the Object tab (position / size / rotation
    /// in region coordinates; LLPanelObject::sendPosition / sendScale / sendRotation).
    /// Changed locally at once; sent when `commit` (typed value, end of drag).
    pub fn set_transform(
        &mut self,
        world: &mut World,
        s: &BuildSettings,
        pos: Option<Vec3>,
        size: Option<Vec3>,
        rot: Option<Quat>,
        commit: bool,
    ) {
        let Some(key) = self.selection.first().copied() else { return };
        let Some(idx) = world.objects.index_of(&key) else { return };
        let mode = if size.is_some() { EditMode::Stretch } else { EditMode::Move };
        if !can_transform(world, idx, s.edit_linked, mode) {
            return;
        }
        let now = Instant::now();
        let Some((wp, wr, _)) = crate::scene::Scene::object_transform(world, idx, now, 0) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else { return };
        // what we wear is typed relative to its attachment point, as the
        // simulator stores it (LLPanelObject::sendPosition, isAttachment)
        let space = select_type(world, idx);
        let local = space != SelectType::World;
        // no further than MAX_ATTACHMENT_DIST from a point of the body
        let pos = pos.map(|p| {
            if space == SelectType::Attachment && root_of(world, idx) == idx {
                p.clamp_length_max(MAX_ATTACHMENT_DIST)
            } else {
                p
            }
        });
        let Some(off) = world.region_offset(o.key.region) else { return };
        let parent = edit_parent(world, idx, now);
        let position_scale = world.hud_position_scale(idx);
        let new_world_pos = pos
            .map(|p| {
                if local {
                    parent.map_or(p, |(pp, pr)| pp + pr * (p * position_scale))
                } else {
                    p + off
                }
            })
            .unwrap_or(wp);
        let new_world_rot = rot
            .map(|r| if local { parent.map_or(r, |(_, pr)| pr * r) } else { r })
            .unwrap_or(wr);
        let (local_pos, local_rot) = match parent {
            Some((pp, pr)) => (
                pr.inverse() * (new_world_pos - pp) / position_scale,
                (pr.inverse() * new_world_rot).normalize(),
            ),
            None => (new_world_pos - off, new_world_rot.normalize()),
        };
        let scale = size.map(|v| v.clamp(Vec3::splat(MIN_PRIM_SCALE), Vec3::splat(MAX_PRIM_SCALE)));
        if let Some(o) = world.objects.get_mut(idx) {
            o.position = local_pos;
            o.rotation = local_rot;
            if let Some(sc) = scale {
                o.scale = sc;
            }
            o.velocity = Vec3::ZERO;
            o.angular_velocity = Vec3::ZERO;
            o.render.needs_records = true;
        }
        if !commit {
            return;
        }
        // read back what is now in the object: a drag may have changed
        // another field before this commit
        let (local_pos, local_rot, scale) = match world.objects.get(idx) {
            Some(o) => (o.position, o.rotation, size.map(|_| o.scale)),
            None => (local_pos, local_rot, scale),
        };
        let u = TransformUpdate {
            local_id: key.local_id,
            position: Some(local_pos),
            rotation: rot.map(|_| local_rot),
            scale,
            linked: !s.edit_linked && root_of(world, idx) == idx,
            uniform: false,
        };
        self.send(BuildCmd::Transform {
            handle: key.region,
            updates: vec![u],
        });
    }
}
