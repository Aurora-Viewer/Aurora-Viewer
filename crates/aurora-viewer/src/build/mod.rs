//! Build tools: selecting objects, moving / rotating / stretching them with
//! the manipulators, creating prims and terraforming, as in Firestorm
//! (LLToolMgr, LLSelectMgr, LLManipTranslate / Rotate / Scale, LLToolPlacer,
//! LLToolBrushLand; originally LGPL 2.1 sources).
//!
//! The selection is local to the viewer and mirrored to the simulators with
//! ObjectSelect / ObjectDeselect. Drags change the objects locally every
//! frame and send MultipleObjectUpdate when the mouse is released (stretching
//! also every 0.1 s, like LLManipScale).

pub mod demo;
pub mod draw;
pub mod geom;
pub mod land;
pub mod manip;
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
        }
    }
}

impl BuildSettings {
    /// LLManip::updateGridSettings: finest subdivision allowed.
    pub fn max_subdivision(&self) -> f32 {
        if self.grid_sub_unit { 32.0 } else { 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Edit,
    Create,
    Land,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Move,
    Rotate,
    Stretch,
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
    /// Selected units (linksets, or prims with "Edit linked"), latest first.
    pub selection: Vec<ObjKey>,
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
    /// Floater tab (General, Object).
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
            selection: Vec::new(),
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

fn root_of(world: &World, mut idx: usize) -> usize {
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

/// Attachments and avatars cannot be edited here (only in-world objects).
fn selectable(world: &World, idx: usize) -> bool {
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
            Some(p) => i = p,
            None => return true,
        }
    }
    false
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
            let root = world.objects.get(idx).is_some_and(|o| o.parent_id == 0);
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

/// LLSelectMgr::getGrid: world grid, or the selection's own frame ("local").
pub fn grid_of(s: &BuildSettings, bounds: Option<&Bounds>) -> Grid {
    match (s.grid_mode, bounds) {
        (1, Some(b)) => Grid {
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
        self.tool = tool;
    }

    /// LLFloaterTools::onClose: everything is deselected.
    pub fn close(&mut self, world: &mut World, s: &mut BuildSettings) {
        self.open = false;
        self.manip.cancel();
        self.land.cancel();
        self.deselect_all(world);
        s.edit_linked = false;
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

    /// Forget objects that are gone; drop a selection left far behind
    /// (LLSelectMgr::deselectAllIfTooFar).
    fn prune(&mut self, world: &World, now: Instant) {
        self.selection.retain(|k| world.objects.index_of(k).is_some());
        if !self.selection.is_empty()
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
        self.status = if not_owned > 0 {
            format!("Suppression demandée ({not_owned} objet(s) d'autres propriétaires : refus probable)")
        } else {
            format!("{} objet(s) envoyé(s) à la corbeille", roots.len())
        };
        self.deselect_all(world);
    }

    /// Take / take a copy into the Objects folder (derez_objects).
    pub fn take(&mut self, world: &mut World, copy: bool) {
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
        if !copy {
            self.deselect_all(world);
        }
    }

    /// Ctrl+D: copies 0.5 m away, selected (LLSelectMgr::duplicate).
    pub fn duplicate(&mut self, world: &mut World, offset: Vec3, select_copy: bool) {
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
    }

    /// Ctrl+Shift+L: every selected prim (SEND_INDIVIDUALS).
    pub fn unlink(&mut self, world: &mut World) {
        let prims: Vec<usize> = self.highlighted(world).into_iter().map(|(i, _)| i).collect();
        if prims.is_empty() {
            return;
        }
        for (handle, ids) in self.group_by_region(world, &prims) {
            self.send(BuildCmd::Delink { handle, local_ids: ids });
        }
        self.status = "Objet délié".into();
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
            if o.is_avatar() || (!s.edit_linked && o.parent_id != 0) {
                continue;
            }
            let Some((p, _, hud)) = crate::scene::Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if hud || p.distance(world.agent.position) > MAX_SELECT_DISTANCE {
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
        match self.tool {
            Tool::Edit => {
                let mode = self.effective_mode(mods);
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
                if let Some(r) = self.rect.as_mut() {
                    r.1 = cursor;
                }
                for c in cmds {
                    self.send(c);
                }
            }
            Tool::Land => {
                let mut cmds = Vec::new();
                self.land.update(world, s, &self.cam, cursor, over_ui, dt, &mut cmds);
                for c in cmds {
                    self.send(c);
                }
            }
            Tool::Create => {}
        }
    }

    /// Left press in the world while building. `hit` is the picked surface
    /// point and `target` the object there. Returns whether it was used.
    pub fn mouse_down(
        &mut self,
        world: &mut World,
        s: &mut BuildSettings,
        hit: Option<Vec3>,
        target: Option<usize>,
        cursor: (f32, f32),
        mods: Mods,
    ) -> bool {
        if !self.open {
            return false;
        }
        let now = Instant::now();
        match self.tool {
            Tool::Edit => {
                let mode = self.effective_mode(mods);
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
                let _ = hit;
                true
            }
            Tool::Create => {
                if mods.shift || mods.ctrl {
                    // Shift / Ctrl click selects instead (LLToolCompCreate)
                    self.click_select(world, s, target, true);
                    return true;
                }
                self.place(world, s, hit, target, cursor);
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
        // flora and avatars are not surfaces to build on
        let target = target.filter(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar() && !o.is_tree()));
        let Some((handle, off)) = world.regions.keys().find_map(|&h| {
            let off = world.region_offset(h)?;
            let r = world.regions.get(&h)?;
            let (lx, ly) = (point.x - off.x, point.y - off.y);
            (lx >= 0.0 && ly >= 0.0 && lx < r.heightmap.size_x as f32 && ly < r.heightmap.size_y as f32).then_some((h, off))
        }) else {
            return;
        };
        let (_, dir) = self.cam.ray(cursor.0, cursor.1);
        let ray_start = self.cam.eye - off + self.cam.at * (0.1 + 0.01);
        let (ray_end, target_id, bypass) = match target.and_then(|i| world.objects.get(i)) {
            Some(o) => (ray_start + dir * MAX_SELECT_DISTANCE, o.full_id, false),
            None => (point - off, Uuid::nil(), true),
        };
        let shape = &shapes::SHAPES[s.create_shape.min(shapes::SHAPES.len() - 1)];
        let p = aurora_net::build::NewPrim {
            handle,
            group_id: Uuid::nil(),
            pcode: aurora_prim::params::LL_PCODE_VOLUME,
            material: s.new_prim_material,
            add_flags: FLAGS_CREATE_SELECTED,
            shape: shape.raw(),
            ray_start,
            ray_end,
            ray_target: target_id,
            ray_end_is_intersection: false,
            bypass_raycast: bypass,
            scale: Vec3::from_array(s.new_prim_size).clamp(Vec3::splat(MIN_PRIM_SCALE), Vec3::splat(MAX_PRIM_SCALE)),
            rotation: shape.rotation(),
            state: 0,
        };
        self.deselect_all(world);
        self.expect_created(world);
        self.send(BuildCmd::Add(Box::new(p)));
        if !s.keep_tool {
            self.tool = Tool::Edit;
            self.edit_mode = EditMode::Move;
        }
    }

    /// Selected mesh / sculpted prims the renderer outlines as wireframes
    /// (object index -> root color); the others get their edges drawn here.
    pub fn wire_selection(&self, world: &World, s: &BuildSettings) -> HashMap<usize, bool> {
        if !self.open || !s.show_highlight || self.manip.dragging() {
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
        if !self.open {
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
        match self.tool {
            Tool::Edit => {
                let mode = self.effective_mode(mods);
                self.manip.draw(&mut p, world, s, &self.selection, self.selection_linked, mode, now);
            }
            Tool::Land => self.land.draw(&mut p, world, s),
            Tool::Create => {}
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
        let Some((wp, wr, _)) = crate::scene::Scene::object_transform(world, idx, now, 0) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else { return };
        let Some(off) = world.region_offset(o.key.region) else { return };
        let parent = if o.parent_id != 0 {
            world
                .objects
                .parent_of(o)
                .and_then(|p| crate::scene::Scene::object_transform(world, p, now, 0))
                .map(|(p, r, _)| (p, r))
        } else {
            None
        };
        let new_world_pos = pos.map(|p| p + off).unwrap_or(wp);
        let new_world_rot = rot.unwrap_or(wr);
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
            linked: !s.edit_linked && parent.is_none(),
            uniform: false,
        };
        self.send(BuildCmd::Transform {
            handle: key.region,
            updates: vec![u],
        });
    }
}
