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
pub mod contents;
pub mod costs;
pub mod demo;
pub mod demo_sim;
pub mod draw;
pub mod edits;
pub mod geom;
pub mod grab;
pub mod hud_drag;
pub mod land;
pub mod manip;
pub mod materials;
pub mod shapes;
pub mod silhouette;
pub mod ui;

use crate::world::World;
use crate::world::objects::ObjKey;
use aurora_net::build::{BuildCmd, ObjectProps, TransformUpdate, derez};
use aurora_net::{NetCommand, RegionHandle};
use geom::Cam;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid::Uuid;

/// LL_PCODE_* / FLAGS_CREATE_SELECTED of object updates (llprimitive.h).
pub const FLAGS_CREATE_SELECTED: u32 = 1 << 1;
/// REGION_FLAGS_BLOCK_TERRAFORM (llregionflags.h).
pub const REGION_FLAGS_BLOCK_TERRAFORM: u32 = 1 << 6;
/// MaxSelectDistance (LimitSelectDistance on).
pub const MAX_SELECT_DISTANCE: f32 = 128.0;
/// SL prim size limits (llmath/xform.h: DEFAULT_MIN_PRIM_SCALE / MAX).
pub const MIN_PRIM_SCALE: f32 = 0.01;
pub const MAX_PRIM_SCALE: f32 = 64.0;

/// Build preferences (Firestorm settings.xml names in comments).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BuildSettings {
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
    /// RenderHighlightSelections.
    pub show_highlight: bool,
    /// Create tool: shape (index in `shapes::SHAPES`).
    pub create_shape: usize,
    /// CreateToolKeepSelected.
    pub keep_tool: bool,
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
    /// FSBuildPrefs_ActualRoot: the manipulators pivot on the root prim.
    pub actual_root: bool,
    /// SelectReflectionProbes: invisible reflection probes can be picked.
    pub select_probes: bool,
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
    /// FSCopyObjKeySeparator for « Copier l'UUID ».
    pub copy_key_separator: String,
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
        world.objects.get_mut(child).unwrap().update_flags |= OBJECT_MODIFY;
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
        tool.prune(&world, Instant::now());
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
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings {
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
            show_highlight: true,
            create_shape: 0,
            keep_tool: false,
            new_prim_size: [0.5; 3],
            new_prim_material: 3,
            land_action: 6,
            land_brush_size: 2.0,
            land_brush_force: 1.0,
            actual_root: false,
            select_probes: false,
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

pub struct BuildTool {
    /// The build floater is open (build mode).
    pub open: bool,
    pub tool: Tool,
    pub edit_mode: EditMode,
    pub focus_mode: FocusMode,
    pub grab_mode: GrabMode,
    /// Selected units (linksets, or prims with "Edit linked"), latest first.
    pub selection: Vec<ObjKey>,
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
    /// Land impact, weights and parcel capacity.
    pub costs: costs::Costs,
    /// Object inventories (Contents tab).
    pub contents: contents::Contents,
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
    /// ObjectProperties of selected objects.
    pub props: HashMap<Uuid, ObjectProps>,
    pub cam: Cam,
    /// Commands to send (drained by the app).
    pub out: Vec<NetCommand>,
    pub manip: manip::Manip,
    pub land: land::LandTool,
    pub silhouettes: silhouette::Cache,
    /// Objects created / duplicated with FLAGS_CREATE_SELECTED: select them
    /// when they arrive (LLViewerObjectList). (Sent at, objects already flagged.)
    pending_create: Option<(Instant, HashSet<Uuid>)>,
    /// Selection rectangle being dragged (start, current in pixels, extend).
    pub rect: Option<((f32, f32), (f32, f32), bool)>,
    /// Floater tab (General, Object, Features, Texture, Contents).
    pub tab: usize,
    /// Last message for the floater's status line.
    pub status: String,
    last_frame: Option<Instant>,
}

impl Default for BuildTool {
    fn default() -> Self {
        BuildTool {
            open: false,
            tool: Tool::Edit,
            edit_mode: EditMode::Move,
            focus_mode: FocusMode::Zoom,
            grab_mode: GrabMode::Move,
            selection: Vec::new(),
            faces: HashMap::new(),
            last_face: None,
            face_counts: HashMap::new(),
            grid_objects: Vec::new(),
            physics: HashMap::new(),
            costs: costs::Costs::default(),
            contents: contents::Contents::default(),
            grab: grab::Grab::default(),
            align: align::Align::default(),
            ui: ui::FloaterState::default(),
            caps: HashMap::new(),
            next_tag: 1,
            menu_selection: false,
            selection_linked: false,
            props: HashMap::new(),
            cam: Cam::default(),
            out: Vec::new(),
            manip: manip::Manip::default(),
            land: land::LandTool::default(),
            silhouettes: silhouette::Cache::default(),
            pending_create: None,
            rect: None,
            tab: 1,
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

/// World objects and our own HUDs; other attachments remain unsupported.
pub fn selectable(world: &World, idx: usize) -> bool {
    let mut i = idx;
    for _ in 0..64 {
        let Some(o) = world.objects.get(i) else {
            return false;
        };
        if o.is_avatar() || o.pcode != aurora_prim::params::LL_PCODE_VOLUME && !o.is_tree() {
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
                    return parent.full_id == world.agent_id && (31..=38).contains(&o.attachment_point());
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
    let (p, r, hud) = crate::scene::Scene::object_transform(world, idx, now, 0)?;
    if hud && world.objects.get(parent_idx)?.is_avatar() {
        let rotation = (r * o.rotation.inverse()).normalize();
        Some((p - rotation * o.position, rotation))
    } else {
        crate::scene::Scene::object_transform(world, parent_idx, now, 0).map(|(p, r, _)| (p, r))
    }
}

/// Root and all its children (recursively).
pub fn family(world: &World, root: usize) -> Vec<usize> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() && out.len() < 4096 {
        if let Some(o) = world.objects.get(out[i]) {
            out.extend(world.objects.children_of(&o.key).iter().copied());
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
            for (i, p) in family(world, idx).into_iter().enumerate() {
                out.push((p, i == 0));
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
        let (Some(o), Some((p, r, _))) = (world.objects.get(idx), crate::scene::Scene::object_transform(world, idx, now, 0)) else {
            continue;
        };
        let h = o.scale * 0.5;
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
/// or the frame of the objects chosen as reference (« Utiliser la sélection
/// pour la grille »).
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

    pub fn open_build(&mut self, tool: Tool) {
        self.open = true;
        // a right-click selection becomes the build selection
        self.menu_selection = false;
        self.set_tool(tool);
    }

    /// LLFloaterTools::onClose: everything is deselected, the tools reset
    /// (resetToolState: zoom, no spin / lift).
    pub fn close(&mut self, world: &mut World, s: &mut BuildSettings) {
        self.open = false;
        self.manip.cancel();
        self.land.cancel();
        self.grab_up();
        self.deselect_all(world);
        self.faces.clear();
        s.edit_linked = false;
        self.focus_mode = FocusMode::Zoom;
        self.grab_mode = GrabMode::Move;
    }

    // ---- selection

    /// Object indices of the selection with their highlight (true = root
    /// color): whole linksets, or the selected prims with "Edit linked".
    pub fn highlighted(&self, world: &World) -> Vec<(usize, bool)> {
        highlighted_of(world, &self.selection, self.selection_linked)
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
        // LLSelectMgr keeps HUD and world selections in separate spaces.
        if let Some(i) = idx
            && let Some((_, _, hud)) = crate::scene::Scene::object_transform(world, i, Instant::now(), 0)
            && !self.selection.is_empty()
            && hud != self.hud_selected(world)
        {
            self.deselect_all(world);
        }
        if extend {
            if self.is_selected(&unit) {
                self.deselect(world, &[unit]);
            } else {
                self.add(world, unit);
            }
        } else if !self.is_selected(&unit) {
            self.deselect_all(world);
            self.add(world, unit);
        } else {
            // clicking a selected object makes it the primary one
            self.selection.retain(|k| *k != unit);
            self.selection.insert(0, unit);
        }
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

    /// Forget objects that are gone; drop a selection left far behind
    /// (LLSelectMgr::deselectAllIfTooFar).
    fn prune(&mut self, world: &World, now: Instant) {
        self.selection.retain(|k| world.objects.index_of(k).is_some());
        if !self.selection.is_empty()
            && !self.hud_selected(world)
            && !self.manip.dragging()
            && let Some(b) = self.bounds(world, now)
            && b.center.distance(world.agent.position) > MAX_SELECT_DISTANCE
        {
            self.deselect_all(world);
            self.status = "Sélection trop loin : désélectionnée".into();
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
    pub fn delete(&mut self, world: &mut World) {
        if self.hud_selected(world) {
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
            .filter(|&&i| world.objects.get(i).is_some_and(|o| o.owner_id != own))
            .count();
        for (handle, ids) in self.group_by_region(world, &roots) {
            self.send(BuildCmd::DeRez {
                handle,
                local_ids: ids,
                destination: derez::TRASH,
                destination_id: trash,
                group_id: Uuid::nil(),
            });
        }
        // LLSelectMgr::confirmDelete
        world.ui_sounds.push(crate::ui_sound::UiSound::ObjectDelete);
        self.status = if not_owned > 0 {
            format!("Suppression demandée ({not_owned} objet(s) d'autres propriétaires : refus probable)")
        } else {
            format!("{} objet(s) envoyé(s) à la corbeille", roots.len())
        };
        self.deselect_all(world);
    }

    /// Take / take a copy into the Objects folder (derez_objects).
    pub fn take(&mut self, world: &mut World, copy: bool) {
        if self.hud_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if roots.is_empty() {
            return;
        }
        let objects = Self::folder_of_type(world, 6);
        let dest = if copy {
            derez::ACQUIRE_TO_AGENT_INVENTORY
        } else {
            derez::TAKE_INTO_AGENT_INVENTORY
        };
        // all in one region (AcquireErrorObjectSpan otherwise): the first one's
        let by_region = self.group_by_region(world, &roots);
        if by_region.len() > 1 {
            self.status = "Impossible de prendre des objets de plusieurs régions à la fois".into();
            return;
        }
        for (handle, ids) in by_region {
            self.send(BuildCmd::DeRez {
                handle,
                local_ids: ids,
                destination: dest,
                destination_id: objects,
                group_id: Uuid::nil(),
            });
        }
        // derez_objects
        world.ui_sounds.push(crate::ui_sound::UiSound::ObjectRezOut);
        if !copy {
            self.deselect_all(world);
        }
    }

    /// Ctrl+D: copies 0.5 m away, selected (LLSelectMgr::duplicate).
    pub fn duplicate(&mut self, world: &mut World, offset: Vec3, select_copy: bool) {
        if self.hud_selected(world) {
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
                group_id: Uuid::nil(),
            });
        }
    }

    /// Ctrl+L: link the selected linksets; the last selected becomes the root.
    pub fn link(&mut self, world: &mut World) {
        if self.hud_selected(world) {
            return;
        }
        let roots = self.roots(world);
        if roots.len() < 2 {
            self.status = "Sélectionnez au moins deux objets à lier".into();
            return;
        }
        let first = world.objects.get(roots[0]).map(|o| (o.key.region, o.owner_id));
        let same = roots
            .iter()
            .all(|&r| world.objects.get(r).map(|o| (o.key.region, o.owner_id)) == first);
        if !same {
            self.status = "Les objets à lier doivent avoir le même propriétaire et être dans la même région".into();
            return;
        }
        let total: usize = roots.iter().map(|&r| family(world, r).len()).sum();
        if total > 256 {
            self.status = format!("Trop de prims pour un seul objet ({total} > 256)");
            return;
        }
        let Some((handle, _)) = first else { return };
        let ids: Vec<u32> = roots.iter().filter_map(|&r| world.objects.get(r).map(|o| o.key.local_id)).collect();
        self.send(BuildCmd::Link { handle, local_ids: ids });
        self.status = format!("{} objets liés", roots.len());
        self.costs_stale();
    }

    /// Ctrl+Shift+L: every selected prim (SEND_INDIVIDUALS).
    pub fn unlink(&mut self, world: &mut World) {
        if self.hud_selected(world) {
            return;
        }
        let prims: Vec<usize> = self.highlighted(world).into_iter().map(|(i, _)| i).collect();
        if prims.is_empty() {
            return;
        }
        for (handle, ids) in self.group_by_region(world, &prims) {
            self.send(BuildCmd::Delink { handle, local_ids: ids });
        }
        self.status = "Objet délié".into();
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
            let group_id = Uuid::nil();
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
        let own = world.agent_id;
        let flagged = world
            .objects
            .iter()
            .filter(|(_, o)| o.owner_id == own && o.update_flags & FLAGS_CREATE_SELECTED != 0)
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
        let own = world.agent_id;
        let new: Vec<usize> = world
            .objects
            .iter()
            .filter(|(_, o)| {
                o.owner_id == own && o.parent_id == 0 && o.update_flags & FLAGS_CREATE_SELECTED != 0 && !known.contains(&o.full_id)
            })
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

    /// LLToolSelectRect: objects whose center is inside the dragged
    /// rectangle (in front of the camera, within the selection distance).
    fn rect_select(&mut self, world: &mut World, s: &BuildSettings, a: (f32, f32), b: (f32, f32), extend: bool) {
        if (a.0 - b.0).abs() < 3.0 && (a.1 - b.1).abs() < 3.0 {
            return; // a click, already handled
        }
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        let now = Instant::now();
        self.selection_linked = s.edit_linked;
        let mut units = Vec::new();
        for (idx, o) in world.objects.iter() {
            if o.is_avatar() || (!s.edit_linked && root_of(world, idx) != idx) {
                continue;
            }
            let Some((p, _, hud)) = crate::scene::Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if hud != self.cam.hud.is_some() || (!hud && p.distance(world.agent.position) > MAX_SELECT_DISTANCE) {
                continue;
            }
            let Some((x, y)) = self.cam.project_px(p) else { continue };
            if x >= x0
                && x <= x1
                && y >= y0
                && y <= y1
                && let Some(u) = Self::unit_of(world, idx, s.edit_linked)
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
        if !self.open {
            return;
        }
        self.prune(world, now);
        self.pick_up_created(world, s);
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
        let now = Instant::now();
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
                        self.face_click(world, target, face, mods.shift);
                        true
                    }
                    EditMode::Align => {
                        if !self.align_apply(world) {
                            // QToolAlign::pickCallback: whole linksets
                            s.edit_linked = false;
                            self.click_select(world, s, target, mods.shift);
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
                        self.click_select(world, s, target, extend);
                        self.rect = Some((cursor, cursor, extend));
                        true
                    }
                }
            }
            Tool::Create => {
                if mods.shift || mods.ctrl {
                    // Shift / Ctrl click selects instead (LLToolCompCreate)
                    self.click_select(world, s, target, true);
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
                self.deselect_all(world);
                self.faces.clear();
                self.selection_linked = true;
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

    pub fn mouse_up(&mut self, world: &mut World, s: &BuildSettings) {
        if let Some((a, b, extend)) = self.rect.take() {
            self.rect_select(world, s, a, b, extend);
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
    fn region_at(world: &World, point: Vec3) -> Option<(RegionHandle, Vec3)> {
        world.regions.keys().find_map(|&h| {
            let off = world.region_offset(h)?;
            let r = world.regions.get(&h)?;
            let (lx, ly) = (point.x - off.x, point.y - off.y);
            (lx >= 0.0 && ly >= 0.0 && lx < r.heightmap.size_x as f32 && ly < r.heightmap.size_y as f32).then_some((h, off))
        })
    }

    /// The ray of LLToolPlacer::raycastForNewObjPos (region-local): from the
    /// camera to the land point, or through the object hit.
    fn placement_ray(&self, world: &World, point: Vec3, target: Option<usize>, cursor: (f32, f32), off: Vec3) -> (Vec3, Vec3, Uuid, bool) {
        // flora and avatars are not surfaces to build on
        let target = target.filter(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar() && !o.is_tree()));
        let (_, dir) = self.cam.ray(cursor.0, cursor.1);
        let ray_start = self.cam.eye - off + self.cam.at * (0.1 + 0.01);
        match target.and_then(|i| world.objects.get(i)) {
            Some(o) => (ray_start, ray_start + dir * MAX_SELECT_DISTANCE, o.full_id, false),
            None => (ray_start, point - off, Uuid::nil(), true),
        }
    }

    /// Create tool click: ObjectAdd (LLToolPlacer::addObject).
    fn place(&mut self, world: &mut World, s: &BuildSettings, hit: Option<Vec3>, target: Option<usize>, cursor: (f32, f32)) {
        let Some(point) = hit else {
            return;
        };
        if point.distance(world.agent.position) > MAX_SELECT_DISTANCE {
            self.status = "Trop loin pour créer un objet ici".into();
            return;
        }
        let Some((handle, off)) = Self::region_at(world, point) else {
            return;
        };
        let (ray_start, ray_end, target_id, bypass) = self.placement_ray(world, point, target, cursor, off);
        let random = Uuid::new_v4().as_u128() as u32;
        let n = shapes::SHAPES.len();
        let plant = s.create_shape == n || s.create_shape == n + 1;
        let (pcode, shape, rotation, state, scale, flags) = if plant {
            // trees and grass: not created selected (LLToolPlacer)
            let tree = s.create_shape == n;
            let state = if tree {
                shapes::plant_species(&shapes::TREE_SPECIES, &s.last_tree, random)
            } else {
                shapes::plant_species(&shapes::GRASS_SPECIES, &s.last_grass, random)
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
            let sh = &shapes::SHAPES[s.create_shape.min(n - 1)];
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
            group_id: Uuid::nil(),
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

    /// « Copier la sélection »: ObjectDuplicateOnRay of the selected roots
    /// where the ray hits (LLToolPlacer::addDuplicate).
    fn place_copy(&mut self, world: &mut World, s: &BuildSettings, hit: Option<Vec3>, target: Option<usize>, cursor: (f32, f32)) {
        let Some(point) = hit else { return };
        let roots = self.roots(world);
        if roots.is_empty() {
            self.status = "Sélectionnez d'abord les objets à copier.".into();
            return;
        }
        let Some((_, off)) = Self::region_at(world, point) else { return };
        let (ray_start, ray_end, ray_target, bypass) = self.placement_ray(world, point, target, cursor, off);
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
                group_id: Uuid::nil(),
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

    /// Selected mesh / sculpted prims the renderer outlines as wireframes
    /// (object index -> root color); the others get their edges drawn here.
    pub fn wire_selection(&self, world: &World, s: &BuildSettings) -> HashMap<usize, bool> {
        if !(self.open || self.menu_selection) || !s.show_highlight || self.manip.dragging() {
            return HashMap::new();
        }
        self.highlighted(world)
            .into_iter()
            .filter(|(i, _)| world.objects.get(*i).is_some_and(|o| o.volume.sculpt.is_some()))
            .collect()
    }

    /// Selection outlines, manipulators, land brush and selection rectangle
    /// over the scene.
    pub fn draw_overlay(&mut self, ctx: &egui::Context, world: &World, s: &BuildSettings, mods: Mods) {
        if !self.open && !self.menu_selection {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("build_overlay")));
        let cam = self.cam;
        let mut p = draw::Painter3d::new(painter, &cam);
        let now = Instant::now();
        if s.show_highlight && !self.manip.dragging() {
            let prims = self.highlighted(world);
            self.silhouettes.draw(&p, world, &prims);
        }
        if !self.open {
            return;
        }
        match self.tool {
            Tool::Edit => {
                let mode = self.effective_mode(mods);
                if mode == EditMode::Align {
                    self.align_draw(&mut p, world);
                } else {
                    self.manip.draw(&mut p, world, s, &self.selection, self.selection_linked, mode, now);
                }
            }
            Tool::Land => {
                self.land.draw(&mut p, world, s);
                if s.show_parcel_owners {
                    land::draw_owners(&mut p, world);
                }
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
        let now = Instant::now();
        let Some((wp, wr, hud)) = crate::scene::Scene::object_transform(world, idx, now, 0) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else { return };
        if hud {
            let flags = world.objects.get(root_of(world, idx)).map_or(0, |o| o.update_flags);
            if flags & crate::ui::context::flags::OBJECT_MOVE == 0
                || ((s.edit_linked || size.is_some()) && o.update_flags & crate::ui::context::flags::OBJECT_MODIFY == 0)
            {
                return;
            }
        }
        let Some(off) = world.region_offset(o.key.region) else { return };
        let parent = edit_parent(world, idx, now);
        let new_world_pos = pos
            .map(|p| if hud { parent.map_or(p, |(pp, pr)| pp + pr * p) } else { p + off })
            .unwrap_or(wp);
        let new_world_rot = rot.map(|r| if hud { parent.map_or(r, |(_, pr)| pr * r) } else { r }).unwrap_or(wr);
        let (local_pos, local_rot) = match parent {
            Some((pp, pr)) => (pr.inverse() * (new_world_pos - pp), (pr.inverse() * new_world_rot).normalize()),
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
