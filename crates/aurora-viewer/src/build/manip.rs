//! The edit manipulators: move arrows and plane handles (LLManipTranslate),
//! rotation rings (LLManipRotate) and stretch handles (LLManipScale), with
//! their grid snapping, snap rulers and the position readout.
//!
//! Sizes follow LL: handles keep a constant size on screen (50 px arrows,
//! 100 px rotation sphere, 10 px stretch boxes). Objects follow the mouse
//! locally; MultipleObjectUpdate goes out on release (stretching also every
//! 0.1 s like LLManipScale::sendUpdates).

use super::draw::{Painter3d, fade, rgba};
use super::geom::{Cam, nearest_on_line, ray_plane, round_to, seg_distance, subdivision_level};
use super::{Bounds, BuildSettings, EditMode, Grid, MAX_PRIM_SCALE, MIN_PRIM_SCALE, Mods, bounds_of, highlighted_of};
use crate::scene::Scene;
use crate::world::World;
use crate::world::objects::ObjKey;
use aurora_net::RegionHandle;
use aurora_net::build::{BuildCmd, TransformUpdate};
use egui::{Color32, Pos2};
use glam::{EulerRot, Quat, Vec3};
use std::collections::HashMap;
use std::time::Instant;

/// LLManipTranslate constants.
const ARROW_PX: f32 = 50.0;
const SELECTED_ARROW_SCALE: f32 = 1.3;
const HOTSPOT_START: f32 = 0.2;
const HOTSPOT_END: f32 = 1.2;
const PLANE_OFFSET: f32 = 1.8;
const PLANE_TICK_SIZE: f32 = 0.4;
const MIN_PLANE_MANIP_DOT: f32 = 0.25;
const SNAP_GUIDE_SCREEN_SIZE: f32 = 0.7;
const SNAP_ARROW_SCALE: f32 = 0.7;
const SCALE_HALF_LIFE: f32 = 0.07;
const DRAG_SLOP_PX: f32 = 2.0;
/// MaxDragDistance (LimitDragDistance on).
const MAX_DRAG_DISTANCE: f32 = 48.0;
/// LLManipRotate constants.
const RING_PX: f32 = 100.0;
const SNAP_ANGLE: f32 = 5.625;
/// LLManipScale: pick radius, box size, streaming interval.
const SCALE_PICK_PX: f32 = 11.0;
const SCALE_BOX_FACTOR: f32 = 0.2;
const SCALE_UPDATE_DELAY: f32 = 0.1;
const SNAP_GUIDE_SCREEN_OFFSET: f32 = 0.05;
/// Tick label spacing in pixels (LLManip::sTickLabelSpacing).
const LABEL_SPACING: (f32, f32) = (60.0, 25.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Move along a grid axis.
    Arrow(usize),
    /// Move in the plane whose normal is this grid axis (YZ, XZ, XY).
    Plane(usize),
    /// Rotate about a grid axis.
    Ring(usize),
    /// Rotate about the view direction.
    Roll,
    /// Free rotation (inside the sphere).
    Free,
    /// Stretch corner (bit 0 = +x, 1 = +y, 2 = +z).
    Corner(u8),
    /// Stretch face (axis, positive side).
    Face(usize, bool),
}

fn unit(i: usize) -> Vec3 {
    let mut v = Vec3::ZERO;
    v[i] = 1.0;
    v
}

fn axis_rgb(i: usize, v: f32, a: f32) -> Color32 {
    let mut c = [0.0f32; 3];
    c[i] = v;
    rgba(c[0], c[1], c[2], a)
}

/// One object moved by a drag, as it was when the drag began.
#[derive(Debug, Clone)]
struct Mover {
    idx: usize,
    key: ObjKey,
    root: bool,
    pos: Vec3,
    rot: Quat,
    scale: Vec3,
    region_off: Vec3,
    parent_idx: Option<usize>,
    parent: Option<(Vec3, Quat)>,
    /// Children that follow along: kept in place when their root moves alone
    /// ("Edit linked"), or scaled with the linkset.
    children: Vec<Child>,
}

#[derive(Debug, Clone, Copy)]
struct Child {
    idx: usize,
    world_pos: Vec3,
    world_rot: Quat,
    local_pos: Vec3,
    scale: Vec3,
}

#[derive(Debug, Clone)]
pub struct Drag {
    pub part: Part,
    mode: EditMode,
    movers: Vec<Mover>,
    bounds: Bounds,
    pub pivot: Vec3,
    grid: Grid,
    pub start_cursor: (f32, f32),
    started: bool,
    linked: bool,
    // translate
    axis: Vec3,
    normal: Vec3,
    start_hit: Vec3,
    snap_axis: Vec3,
    snap_offset: f32,
    pub in_snap: bool,
    pub delta: Vec3,
    // rotate
    radius: f32,
    down_vec: Vec3,
    edge_on: bool,
    pub rotation: Quat,
    obj_axis: Vec3,
    // scale
    factor: Vec3,
    anchor: Vec3,
    last_send: Option<Instant>,
    changed: bool,
    /// Ruler info for the scale guide (line start, direction, unit).
    scale_line: Option<(Vec3, Vec3, f32, f32)>,
}

#[derive(Debug, Default)]
pub struct Manip {
    /// Bounds of the reference objects (grid mode « Référence »), set by
    /// the build tool every frame.
    pub grid_ref: Option<Bounds>,
    pub hover: Option<Part>,
    pub drag: Option<Drag>,
    arrow_scale: [f32; 3],
    plane_scale: [f32; 3],
    ring_scale: [f32; 5],
    last: Option<Instant>,
}

/// What the gizmo is placed on this frame.
struct Frame {
    bounds: Bounds,
    pivot: Vec3,
    grid: Grid,
    prims: usize,
}

fn frame(world: &World, s: &BuildSettings, sel: &[ObjKey], linked: bool, now: Instant, grid_ref: Option<&Bounds>) -> Option<Frame> {
    let bounds = bounds_of(world, sel, linked, now)?;
    let grid = super::grid_of(s, Some(&bounds), grid_ref);
    // FSBuildPrefs_ActualRoot (LLManip::getPivotPoint): pivot on the
    // primary object's root prim rather than the selection's center
    let pivot = if s.actual_root {
        sel.first()
            .and_then(|k| world.objects.index_of(k))
            .map(|i| super::root_of(world, i))
            .and_then(|r| Scene::object_transform(world, r, now, 0))
            .map(|(p, _, _)| p)
            .unwrap_or(bounds.center)
    } else {
        bounds.center
    };
    Some(Frame {
        pivot,
        bounds,
        grid,
        prims: highlighted_of(world, sel, linked).len(),
    })
}

impl Manip {
    pub fn dragging(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.started)
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.hover = None;
    }

    fn animate(&mut self, active: Option<Part>) {
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.016);
        self.last = Some(now);
        let k = 1.0 - 0.5f32.powf(dt / SCALE_HALF_LIFE);
        for i in 0..3 {
            let a = if active == Some(Part::Arrow(i)) {
                SELECTED_ARROW_SCALE
            } else {
                1.0
            };
            let p = if active == Some(Part::Plane(i)) {
                SELECTED_ARROW_SCALE
            } else {
                1.0
            };
            self.arrow_scale[i] += (a - self.arrow_scale[i]) * k;
            self.plane_scale[i] += (p - self.plane_scale[i]) * k;
            if self.arrow_scale[i] == 0.0 {
                self.arrow_scale[i] = 1.0;
            }
            if self.plane_scale[i] == 0.0 {
                self.plane_scale[i] = 1.0;
            }
        }
        for i in 0..5 {
            let part = match i {
                0..=2 => Part::Ring(i),
                3 => Part::Roll,
                _ => Part::Free,
            };
            let t = if active == Some(part) { 1.05 } else { 1.0 };
            if self.ring_scale[i] == 0.0 {
                self.ring_scale[i] = 1.0;
            }
            self.ring_scale[i] += (t - self.ring_scale[i]) * k;
        }
    }

    // ---- hit testing

    fn pick(&self, cam: &Cam, f: &Frame, mode: EditMode, cursor: (f32, f32)) -> Option<Part> {
        let m = Pos2::new(cursor.0 / cam.ppp, cursor.1 / cam.ppp);
        match mode {
            EditMode::Move => pick_translate(cam, f, m),
            EditMode::Rotate => pick_rotate(cam, f, m),
            EditMode::Stretch => pick_scale(cam, f, m),
            EditMode::Face | EditMode::Align => None,
        }
    }

    // ---- per frame

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &mut World,
        s: &BuildSettings,
        cam: &Cam,
        sel: &[ObjKey],
        linked: bool,
        mode: EditMode,
        cursor: (f32, f32),
        over_ui: bool,
        mods: Mods,
        now: Instant,
        out: &mut Vec<BuildCmd>,
    ) {
        if let Some(mut d) = self.drag.take() {
            drag_update(&mut d, world, s, cam, cursor, mods, now, out);
            self.animate(Some(d.part));
            self.drag = Some(d);
            return;
        }
        self.hover = if over_ui || sel.is_empty() {
            None
        } else {
            frame(world, s, sel, linked, now, self.grid_ref.as_ref()).and_then(|f| self.pick(cam, &f, mode, cursor))
        };
        let h = self.hover;
        self.animate(h);
    }

    /// Mouse down: start dragging the handle under the cursor.
    #[allow(clippy::too_many_arguments)]
    pub fn try_grab(
        &mut self,
        world: &mut World,
        s: &BuildSettings,
        cam: &Cam,
        sel: &[ObjKey],
        linked: bool,
        mode: EditMode,
        cursor: (f32, f32),
        now: Instant,
    ) -> bool {
        let Some(f) = frame(world, s, sel, linked, now, self.grid_ref.as_ref()) else {
            return false;
        };
        let Some(part) = self.pick(cam, &f, mode, cursor) else {
            return false;
        };
        let movers = save_movers(world, sel, linked, now);
        if movers.is_empty() {
            return false;
        }
        let (o, dir) = cam.ray(cursor.0, cursor.1);
        let mut d = Drag {
            part,
            mode,
            movers,
            bounds: f.bounds,
            pivot: f.pivot,
            grid: f.grid,
            start_cursor: cursor,
            started: false,
            linked,
            axis: Vec3::X,
            normal: Vec3::Z,
            start_hit: f.pivot,
            snap_axis: Vec3::Y,
            snap_offset: 0.0,
            in_snap: false,
            delta: Vec3::ZERO,
            radius: 1.0,
            down_vec: Vec3::X,
            edge_on: false,
            rotation: Quat::IDENTITY,
            obj_axis: Vec3::X,
            factor: Vec3::ONE,
            anchor: f.pivot,
            last_send: None,
            changed: false,
            scale_line: None,
        };
        let g = f.grid.rotation;
        match part {
            Part::Arrow(i) => {
                d.axis = g * unit(i);
                // the plane containing the axis that faces the camera (getManipNormal)
                d.normal = d.axis.cross(cam.at).cross(d.axis).try_normalize().unwrap_or(cam.at);
                let rel = (g.inverse() * (f.pivot - cam.eye).normalize_or_zero()).abs();
                d.snap_axis = g * unit(snap_offset_axis(i, rel));
                d.snap_offset = cam.meters_for_pixels(d.movers[0].pos, ARROW_PX) * 1.5;
            }
            Part::Plane(i) => {
                d.normal = g * unit(i);
            }
            Part::Ring(i) => {
                d.axis = g * unit(i);
                d.radius = ring_radius(cam, f.pivot);
                let cam_dir = (f.pivot - cam.eye).normalize_or_zero();
                d.edge_on = d.axis.dot(cam_dir).abs() < 85f32.to_radians().cos();
                if let Some(q) = ray_plane(o, dir, f.pivot, d.axis) {
                    d.down_vec = (q - f.pivot).try_normalize().unwrap_or(Vec3::X);
                }
                // object axis nearest the grab point (for absolute snapping)
                let r = d.movers[0].rot;
                let mut best = (f32::MIN, Vec3::X);
                for k in 0..3 {
                    for sgn in [1.0f32, -1.0] {
                        let a = r * unit(k) * sgn;
                        let dd = a.dot(d.down_vec);
                        if dd > best.0 {
                            best = (dd, a);
                        }
                    }
                }
                d.obj_axis = best.1;
            }
            Part::Free => {
                d.radius = ring_radius(cam, f.pivot);
                d.down_vec = sphere_point(o, dir, f.pivot, d.radius, cam);
            }
            Part::Roll => {}
            Part::Corner(_) | Part::Face(_, _) => {}
        }
        if matches!(part, Part::Arrow(_) | Part::Plane(_)) {
            match ray_plane(o, dir, f.pivot, d.normal) {
                Some(h) => d.start_hit = h,
                None => return false,
            }
        }
        if let Part::Corner(c) = part {
            let b = &f.bounds;
            let corner = b.center + b.rotation * (b.half * corner_signs(c));
            let opposite = b.center + b.rotation * (b.half * -corner_signs(c));
            d.anchor = if s.scale_uniform { b.center } else { opposite };
            let _ = corner;
        }
        if let Part::Face(i, pos) = part {
            let b = &f.bounds;
            let sgn = if pos { 1.0 } else { -1.0 };
            d.anchor = if s.scale_uniform {
                b.center
            } else {
                b.center - b.rotation * unit(i) * b.half[i] * sgn
            };
        }
        self.drag = Some(d);
        true
    }

    /// Mouse up: send the final transforms (LLManip*::handleMouseUp).
    pub fn release(&mut self, world: &mut World, s: &BuildSettings, linked: bool, out: &mut Vec<BuildCmd>) {
        let Some(d) = self.drag.take() else {
            return;
        };
        if !d.started || !d.changed {
            return;
        }
        let _ = s;
        send_updates(world, &d, linked, out);
    }

    // ---- drawing

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, p: &mut Painter3d, world: &World, s: &BuildSettings, sel: &[ObjKey], linked: bool, mode: EditMode, now: Instant) {
        if !mode.is_manip() {
            return;
        }
        let Some(mut f) = frame(world, s, sel, linked, now, self.grid_ref.as_ref()) else {
            return;
        };
        let cam = p.cam;
        if f.pivot.distance(cam.eye) > super::MAX_SELECT_DISTANCE + 64.0 {
            return;
        }
        let drag = self.drag.as_ref().filter(|d| d.started);
        let mode = drag.map(|d| d.mode).unwrap_or(mode);
        if let Some(d) = drag {
            // the grid stays where it was when the drag began
            f.grid = d.grid;
        }
        match mode {
            EditMode::Move => self.draw_translate(p, s, &f, drag),
            EditMode::Rotate => self.draw_rotate(p, s, &f, drag, world, sel, now),
            EditMode::Stretch | EditMode::Face | EditMode::Align => self.draw_scale(p, s, &f, drag),
        }
        // position / rotation / size readout (LLManip::renderXYZ)
        let values = match mode {
            EditMode::Move => {
                let off = sel.first().and_then(|k| world.region_offset(k.region)).unwrap_or(Vec3::ZERO);
                (f.pivot - off).to_array()
            }
            EditMode::Rotate => {
                let r = sel
                    .first()
                    .and_then(|k| world.objects.index_of(k))
                    .and_then(|i| Scene::object_transform(world, i, now, 0))
                    .map(|(_, r, _)| r);
                let (z, y, x) = r.unwrap_or(Quat::IDENTITY).to_euler(EulerRot::ZYX);
                let deg = |v: f32| (v.to_degrees() / 0.05).round() * 0.05;
                [deg(x), deg(y), deg(z)]
            }
            EditMode::Stretch | EditMode::Face | EditMode::Align => (f.bounds.half * 2.0).to_array(),
        };
        if drag.is_some() {
            readout(p, values);
        }
    }

    fn draw_translate(&self, p: &mut Painter3d, s: &BuildSettings, f: &Frame, drag: Option<&Drag>) {
        let cam = p.cam;
        let piv = f.pivot;
        let g = f.grid.rotation;
        let len = cam.meters_for_pixels(piv, ARROW_PX);
        let cone = len / 4.0;
        // guidelines through the pivot (LLManip::renderGuidelines)
        for i in 0..3 {
            let a = g * unit(i);
            p.line(piv - a * 256.0, piv + a * 256.0, axis_rgb(i, 1.0, 0.33), 1.5);
        }
        let active = drag.map(|d| d.part);
        // snap ruler / plane grid while dragging
        if let Some(d) = drag {
            match d.part {
                Part::Arrow(i) if s.snap => self.draw_ruler(p, s, f, d, i),
                Part::Plane(i) => draw_plane_grid(p, s, f, d, i),
                _ => {}
            }
        }
        // plane handles
        let at_g = g.inverse() * cam.at;
        let signs = Vec3::new(at_g.x.signum(), at_g.y.signum(), at_g.z.signum());
        let rel = (g.inverse() * (piv - cam.eye).normalize_or_zero()).abs();
        let off = PLANE_OFFSET * len;
        for i in 0..3 {
            if !(active.is_none() || active == Some(Part::Plane(i))) || rel[i] <= MIN_PLANE_MANIP_DOT {
                continue;
            }
            let hl = self.hover == Some(Part::Plane(i)) || active == Some(Part::Plane(i));
            let (j, k) = ((i + 1) % 3, (i + 2) % 3);
            // LL draws YZ with (y, z), XZ with (x, z), XY with (x, y)
            let (u, v) = if i == 1 { (k, j) } else { (j, k) };
            let sc = self.plane_scale[i].max(1.0);
            let center = {
                let mut c = Vec3::ZERO;
                c[u] = off * signs[u];
                c[v] = off * signs[v];
                c
            };
            let at = |a: f32, b: f32| {
                let mut q = Vec3::ZERO;
                q[u] = a * off * PLANE_TICK_SIZE * sc * signs[u];
                q[v] = b * off * PLANE_TICK_SIZE * sc * signs[v];
                piv + g * (center + q)
            };
            let alpha = if hl { 1.0 } else { 0.6 };
            let (c1, c2) = match i {
                0 => (rgba(0.0, 1.0, 0.0, alpha), rgba(0.0, 0.0, 1.0, alpha)),
                1 => (rgba(0.0, 0.0, 1.0, alpha), rgba(1.0, 0.0, 0.0, alpha)),
                _ if hl => (rgba(1.0, 0.0, 0.0, 1.0), rgba(0.0, 1.0, 0.0, 1.0)),
                _ => (rgba(0.8, 0.0, 0.0, 0.6), rgba(0.0, 0.8, 0.0, 0.6)),
            };
            p.tri(at(-0.25, -0.25), at(0.25, -0.75), at(0.25, 0.25), g * unit(i), c1);
            p.tri(at(0.25, 0.25), at(-0.75, 0.25), at(-0.25, -0.25), g * unit(i), c2);
            p.flush();
            let dark = rgba(0.0, 0.0, 0.0, 0.3);
            p.line(at(-0.25, -0.25), at(0.25, -0.25), dark, 3.0);
            p.line(at(0.25, -0.25), at(0.1, -0.1), dark, 3.0);
            p.line(at(0.25, -0.25), at(0.1, -0.4), dark, 3.0);
            p.line(at(-0.25, -0.25), at(-0.25, 0.25), dark, 3.0);
            p.line(at(-0.25, 0.25), at(-0.1, 0.1), dark, 3.0);
            p.line(at(-0.25, 0.25), at(-0.4, 0.1), dark, 3.0);
        }
        // arrows, farthest first; cones all point to +axis as in LL
        let mut arrows: Vec<(f32, usize, f32)> = Vec::new();
        for i in 0..3 {
            for sgn in [1.0f32, -1.0] {
                let tip = piv + g * unit(i) * len * sgn;
                arrows.push((cam.depth(tip), i, sgn));
            }
        }
        arrows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, i, sgn) in arrows {
            let selected = active == Some(Part::Arrow(i)) || self.hover == Some(Part::Arrow(i));
            if active.is_some() && !selected {
                continue;
            }
            let color = if selected { axis_rgb(i, 1.0, 1.0) } else { axis_rgb(i, 0.8, 0.6) };
            let a = g * unit(i);
            p.line(piv + a * cone * sgn, piv + a * len * sgn, color, 2.0);
            let sc = self.arrow_scale[i].max(1.0);
            let h = cone * sc * 1.5;
            let tip_center = piv + a * len * sgn;
            p.cone(tip_center - a * h * 0.5, a, cone * sc * 0.5, h, color);
            p.flush();
        }
    }

    /// Snap ruler along the dragged arrow (LLManipTranslate::renderSnapGuides).
    fn draw_ruler(&self, p: &mut Painter3d, s: &BuildSettings, f: &Frame, d: &Drag, i: usize) {
        let cam = p.cam;
        let axis = d.axis;
        let center = f.pivot;
        let opacity = s.grid_opacity;
        let max_sub = s.max_subdivision();
        let min_scale = d.grid.scale[i];
        let smallest = min_scale / max_sub;
        let range = (center - cam.eye).length();
        let guide = SNAP_GUIDE_SCREEN_SIZE * cam.height * range / cam.pixel_meter_ratio();
        let dist_axis = (center - d.grid.origin).dot(axis);
        let off_unit = dist_axis.rem_euclid(smallest);
        let n = ((0.5 * guide / smallest).floor() as i32).clamp(1, 400);
        let (so, sa) = (d.snap_offset, d.snap_axis);
        let white = Color32::WHITE;
        for side in [1.0f32, -1.0] {
            let base = center + sa * so * side;
            let ext = guide * 0.5 + off_unit;
            let (a, b) = (base + axis * ext, base - axis * ext);
            p.line_gradient(base, a, fade(white, opacity), fade(white, opacity * 0.2), 1.0);
            p.line_gradient(base, b, fade(white, opacity), fade(white, opacity * 0.2), 1.0);
        }
        let sub_div_offset = ((dist_axis - off_unit).rem_euclid(min_scale * 32.0) / smallest).round() as i32;
        let screen_axis = egui::vec2(axis.dot(cam.left).abs(), axis.dot(cam.up).abs()).normalized();
        let label_px = (screen_axis.x * LABEL_SPACING.0 + screen_axis.y * LABEL_SPACING.1)
            .round()
            .max(10.0);
        let up_side = if sa.dot(cam.up) > 0.0 { 1.0 } else { -1.0 };
        for k in -n..=n {
            let along = smallest * k as f32 - off_unit;
            let tick_pos = center + axis * along;
            let mut tick_scale = 1.0;
            let mut level = max_sub;
            while level >= 1.0 / 32.0 {
                if ((k + sub_div_offset) as f32).rem_euclid(level) == 0.0 {
                    break;
                }
                tick_scale *= 0.7;
                level /= 2.0;
            }
            for side in [1.0f32, -1.0] {
                let t0 = tick_pos + sa * so * side;
                p.guide_line(t0, t0 + sa * so * tick_scale * side, white, opacity);
            }
            // labels where they do not crowd (getSubdivisionLevel with the label spacing)
            let lvl = subdivision_level(cam, tick_pos, axis, min_scale, label_px, max_sub);
            if ((k + sub_div_offset) as f32).rem_euclid(max_sub / lvl) == 0.0 {
                let alpha = opacity * (1.0 - 0.5 * (k.abs() as f32 / n as f32));
                let snapped = d.in_snap && k - (off_unit / smallest).round() as i32 == 0;
                let hl = if snapped { 1.0 } else { 0.8 };
                let text_at = tick_pos + sa * so * up_side * (1.0 + tick_scale);
                let local = d.grid.rotation.inverse() * (tick_pos - d.grid.origin);
                let mut value = 0.5 * local[i] / min_scale;
                if d.grid.world {
                    value *= 2.0 * d.grid.scale[i];
                    p.tick_value(text_at, value, "m", rgba(hl, hl, hl, alpha));
                } else {
                    p.tick_value(text_at, value, "x", rgba(hl, hl, hl, alpha));
                }
            }
        }
        if d.in_snap {
            let (a, b) = (center - sa * so, center + sa * so);
            p.guide_line(a, b, white, opacity);
            let cone = cam.meters_for_pixels(center, ARROW_PX) / 4.0 * SNAP_ARROW_SCALE;
            p.tri(a - sa * cone, a + axis * cone, a - axis * cone, cam.at, fade(white, opacity));
            p.tri(b + sa * cone, b + axis * cone, b - axis * cone, cam.at, fade(white, opacity));
            p.flush();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_rotate(
        &self,
        p: &mut Painter3d,
        s: &BuildSettings,
        f: &Frame,
        drag: Option<&Drag>,
        world: &World,
        sel: &[ObjKey],
        now: Instant,
    ) {
        let cam = p.cam;
        let c = drag.map(|d| d.pivot).unwrap_or(f.pivot);
        if cam.depth(c) <= 0.05 {
            return;
        }
        let r = ring_radius(cam, c);
        let w = 8.0 * r / RING_PX;
        let g = f.grid.rotation;
        let active = drag.map(|d| d.part);
        let to_cam = (cam.eye - c).normalize_or_zero();
        let _ = (world, sel, now);
        if active.is_none() {
            // free-rotate sphere and roll ring (screen circles)
            if let (Some(sc), Some(edge)) = (cam.project(c), cam.project(c + cam.up * r)) {
                let rp = sc.distance(edge);
                p.painter
                    .circle_filled(sc, rp, rgba(0.7, 0.7, 0.7, if self.hover == Some(Part::Free) { 0.4 } else { 0.3 }));
                let wpx = (w / r * rp).max(2.0);
                let (col, k) = if self.hover == Some(Part::Roll) {
                    (rgba(0.8, 0.8, 0.8, 0.8), self.ring_scale[3])
                } else {
                    (rgba(0.7, 0.7, 0.7, 0.6), 1.0)
                };
                p.painter.circle_stroke(sc, (rp + wpx * 0.5) * k, egui::Stroke::new(wpx, col));
            }
        }
        for i in 0..3 {
            let part = Part::Ring(i);
            if active.is_some() && active != Some(part) {
                continue;
            }
            let hl = self.hover == Some(part) || active == Some(part);
            let a = g * unit(i);
            let (u, v) = (a.any_orthonormal_vector(), a.cross(a.any_orthonormal_vector()));
            let rr = r * if hl { self.ring_scale[i] } else { 1.0 };
            const N: usize = 100;
            let pt = |k: usize| {
                let t = k as f32 / N as f32 * std::f32::consts::TAU;
                c + (u * t.cos() + v * t.sin()) * rr
            };
            let (front, back) = if hl {
                (axis_rgb(i, 1.0, 1.0), axis_rgb(i, 1.0, if active.is_some() { 0.3 } else { 0.5 }))
            } else {
                (axis_rgb(i, 0.8, 0.8), axis_rgb(i, 0.8, 0.4))
            };
            for k in 0..N {
                let (q0, q1) = (pt(k), pt(k + 1));
                let facing = ((q0 + q1) * 0.5 - c).dot(to_cam) >= -0.05 * rr;
                let wpx = cam
                    .project(q0)
                    .zip(cam.project(q0 + cam.up * w))
                    .map(|(x, y)| x.distance(y))
                    .unwrap_or(2.0)
                    .clamp(1.5, 10.0);
                p.line(q0, q1, if facing { front } else { back }, wpx);
            }
        }
        // snap guides of a constrained drag (LLManipRotate::renderSnapGuides)
        if let Some(d) = drag
            && let (Part::Ring(_), true) = (d.part, s.snap)
        {
            let a = d.axis;
            let (ax1, ax2) = snap_axes(a, d.grid.rotation);
            let opacity = s.grid_opacity;
            let white = Color32::WHITE;
            const N: usize = 96;
            for k in 0..N {
                let t0 = k as f32 / N as f32 * std::f32::consts::TAU;
                let t1 = (k + 1) as f32 / N as f32 * std::f32::consts::TAU;
                let q0 = c + (ax2 * t0.cos() + ax1 * t0.sin()) * 2.0 * r;
                let q1 = c + (ax2 * t1.cos() + ax1 * t1.sin()) * 2.0 * r;
                p.guide_line(q0, q1, white, opacity);
            }
            for k in 0..64 {
                let t = (k as f32 * SNAP_ANGLE).to_radians();
                let dir = ax2 * t.cos() + ax1 * t.sin();
                let len = if k % 16 == 0 {
                    2.8
                } else if k % 8 == 0 {
                    2.4
                } else if k % 4 == 0 {
                    2.2
                } else if k % 2 == 0 {
                    2.1
                } else {
                    2.05
                };
                p.guide_line(c + dir * 2.0 * r, c + dir * len * r, white, opacity);
                if k % 16 == 0 {
                    p.text(c + dir * (len + 0.1) * r, direction_name(dir), 12.0, fade(white, opacity));
                }
            }
            if d.in_snap {
                let o = (d.rotation * d.obj_axis - a * (d.rotation * d.obj_axis).dot(a)).normalize_or_zero();
                let tip = c + o * 2.0 * r;
                p.guide_line(c, tip, white, 1.0);
                let side = a.cross(o) * 0.1 * r;
                p.tri(tip + o * 0.1 * r, tip + side, tip - side, a, white);
                p.flush();
            }
        }
    }

    fn draw_scale(&self, p: &mut Painter3d, s: &BuildSettings, f: &Frame, drag: Option<&Drag>) {
        let cam = p.cam;
        let b = &f.bounds;
        let active = drag.map(|d| d.part);
        if active.is_none() {
            // translucent box faces
            let corner = |sg: Vec3| b.center + b.rotation * (b.half * sg);
            for i in 0..3 {
                for sgn in [-1.0f32, 1.0] {
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    let mk = |a: f32, bb: f32| {
                        let mut v = Vec3::ZERO;
                        v[i] = sgn;
                        v[j] = a;
                        v[k] = bb;
                        corner(v)
                    };
                    p.quad(
                        [mk(-1.0, -1.0), mk(1.0, -1.0), mk(1.0, 1.0), mk(-1.0, 1.0)],
                        rgba(0.7, 0.7, 0.7, 0.15),
                    );
                }
            }
            p.flush();
        }
        if let Some(d) = drag {
            // guideline from the fixed point through the handle
            let handle = handle_pos(&d.bounds, d.part);
            let dir = (handle - d.anchor).normalize_or_zero();
            p.line(d.anchor - dir * 256.0, d.anchor + dir * 256.0, rgba(1.0, 1.0, 1.0, 0.5), 1.0);
            if s.snap
                && let Some((start, line_dir, unit_m, value_scale)) = d.scale_line
            {
                draw_scale_ruler(p, s, start, line_dir, unit_m, value_scale, d.in_snap);
            }
        }
        let mut handles: Vec<(f32, Part, Vec3)> = Vec::new();
        for c in 0..8u8 {
            handles.push((0.0, Part::Corner(c), handle_pos(b, Part::Corner(c))));
        }
        if f.prims == 1 {
            for i in 0..3 {
                for pos in [true, false] {
                    handles.push((0.0, Part::Face(i, pos), handle_pos(b, Part::Face(i, pos))));
                }
            }
        }
        for h in handles.iter_mut() {
            h.0 = cam.depth(h.2);
        }
        handles.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, part, pos) in handles {
            if active.is_some() && active != Some(part) {
                continue;
            }
            let hl = self.hover == Some(part) || active == Some(part);
            let color = match part {
                Part::Corner(_) if hl => Color32::WHITE,
                Part::Corner(_) => rgba(0.7, 0.7, 0.7, 0.6),
                Part::Face(i, _) if hl => {
                    let mut c = [0.2f32; 3];
                    c[i] = 1.0;
                    rgba(c[0], c[1], c[2], 1.0)
                }
                Part::Face(i, _) => axis_rgb(i, 0.6, 0.4),
                _ => continue,
            };
            let size = cam.meters_for_pixels(pos, ARROW_PX) * SCALE_BOX_FACTOR * if hl { 1.2 } else { 1.0 };
            p.cube(pos, b.rotation, Vec3::splat(size * 0.5), color);
            p.flush();
        }
    }
}

/// LLManipTranslate::renderSnapGuides: the perpendicular grid axis the snap
/// rulers stand on (the one the camera looks along the least).
fn snap_offset_axis(arrow: usize, at_abs: Vec3) -> usize {
    let (x, y, z) = (at_abs.x, at_abs.y, at_abs.z);
    if x > y && x > z {
        match arrow {
            1 => 2,
            2 => 1,
            _ if y > z => 2,
            _ => 1,
        }
    } else if y > z {
        match arrow {
            0 => 2,
            2 => 0,
            _ if x > z => 2,
            _ => 0,
        }
    } else {
        match arrow {
            0 => 1,
            1 => 0,
            _ if x > y => 1,
            _ => 0,
        }
    }
}

fn ring_radius(cam: &Cam, c: Vec3) -> f32 {
    cam.depth(c).max(0.01) * (RING_PX / cam.height * cam.fov_y).tan()
}

/// Point of the sphere under the mouse (or of its silhouette), as a unit
/// vector from the center.
fn sphere_point(o: Vec3, d: Vec3, c: Vec3, r: f32, cam: &Cam) -> Vec3 {
    let oc = o - c;
    let b = oc.dot(d);
    let disc = b * b - (oc.length_squared() - r * r);
    if disc >= 0.0 {
        let t = -b - disc.sqrt();
        return (o + d * t - c).normalize_or_zero();
    }
    // outside: the closest point on the camera-facing plane, on the silhouette
    let n = (cam.eye - c).normalize_or_zero();
    match ray_plane(o, d, c, n) {
        Some(q) => (q - c).normalize_or_zero(),
        None => n,
    }
}

/// LLManipRotate::dragConstrained: the reference axes of the snap circle
/// (the grid axis after the ring axis' dominant component, projected).
fn snap_axes(a: Vec3, grid: Quat) -> (Vec3, Vec3) {
    let local = (grid.inverse() * a).abs();
    let next = if local.x >= local.y && local.x >= local.z {
        1
    } else if local.y >= local.z {
        2
    } else {
        0
    };
    let mut ax1 = grid * unit(next);
    ax1 = (ax1 - a * ax1.dot(a)).try_normalize().unwrap_or(a.any_orthonormal_vector());
    let ax2 = a.cross(ax1);
    (ax1, ax2)
}

fn direction_name(d: Vec3) -> &'static str {
    let a = d.abs();
    if a.z >= a.x && a.z >= a.y {
        if d.z > 0.0 { "Haut" } else { "Bas" }
    } else if a.x >= a.y {
        if d.x > 0.0 { "Est" } else { "Ouest" }
    } else if d.y > 0.0 {
        "Nord"
    } else {
        "Sud"
    }
}

fn corner_signs(c: u8) -> Vec3 {
    Vec3::new(
        if c & 1 != 0 { 1.0 } else { -1.0 },
        if c & 2 != 0 { 1.0 } else { -1.0 },
        if c & 4 != 0 { 1.0 } else { -1.0 },
    )
}

fn handle_pos(b: &Bounds, part: Part) -> Vec3 {
    match part {
        Part::Corner(c) => b.center + b.rotation * (b.half * corner_signs(c)),
        Part::Face(i, pos) => b.center + b.rotation * (unit(i) * b.half[i] * if pos { 1.0 } else { -1.0 }),
        _ => b.center,
    }
}

// ---- hit tests

fn pick_translate(cam: &Cam, f: &Frame, m: Pos2) -> Option<Part> {
    let piv = f.pivot;
    let g = f.grid.rotation;
    let len = cam.meters_for_pixels(piv, ARROW_PX);
    let off = PLANE_OFFSET * len;
    let at_g = g.inverse() * cam.at;
    let signs = Vec3::new(at_g.x.signum(), at_g.y.signum(), at_g.z.signum());
    let rel = (g.inverse() * (piv - cam.eye).normalize_or_zero()).abs();
    // (depth of the end, part, start, end, radius)
    let mut segs: Vec<(f32, Part, Vec3, Vec3, f32)> = Vec::new();
    for i in 0..3 {
        let a = g * unit(i);
        for sgn in [1.0f32, -1.0] {
            let (s0, s1) = (piv + a * len * HOTSPOT_START * sgn, piv + a * len * HOTSPOT_END * sgn);
            segs.push((cam.depth(s1), Part::Arrow(i), s0, s1, 10.0));
        }
        if rel[i] > MIN_PLANE_MANIP_DOT {
            let mut v = Vec3::ZERO;
            for j in 0..3 {
                if j != i {
                    v[j] = signs[j];
                }
            }
            let (s0, s1) = (piv + g * (v * off * 0.8), piv + g * (v * off * 1.2));
            segs.push((cam.depth(s1), Part::Plane(i), s0, s1, 20.0));
        }
    }
    segs.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, part, s0, s1, r) in segs {
        let (Some(a), Some(b)) = (cam.project(s0), cam.project(s1)) else {
            continue;
        };
        let (dist, t) = seg_distance(m, a, b);
        if dist < r && t > 0.0 && t < 1.0 {
            return Some(part);
        }
    }
    None
}

fn pick_rotate(cam: &Cam, f: &Frame, m: Pos2) -> Option<Part> {
    let c = f.pivot;
    if cam.depth(c) <= 0.05 {
        return None;
    }
    let r = ring_radius(cam, c);
    let w = 8.0 * r / RING_PX;
    let g = f.grid.rotation;
    let to_cam = (cam.eye - c).normalize_or_zero();
    let mut best: Option<(f32, Part)> = None;
    for i in 0..3 {
        let a = g * unit(i);
        let (u, v) = (a.any_orthonormal_vector(), a.cross(a.any_orthonormal_vector()));
        const N: usize = 64;
        for k in 0..N {
            let t0 = k as f32 / N as f32 * std::f32::consts::TAU;
            let t1 = (k + 1) as f32 / N as f32 * std::f32::consts::TAU;
            let (q0, q1) = (c + (u * t0.cos() + v * t0.sin()) * r, c + (u * t1.cos() + v * t1.sin()) * r);
            if ((q0 + q1) * 0.5 - c).dot(to_cam) < -0.05 * r {
                continue; // back half of the ring
            }
            let (Some(a2), Some(b2)) = (cam.project(q0), cam.project(q1)) else {
                continue;
            };
            let (dist, _) = seg_distance(m, a2, b2);
            if dist < 8.0 && best.is_none_or(|(d, _)| dist < d) {
                best = Some((dist, Part::Ring(i)));
            }
        }
    }
    if let Some((_, part)) = best {
        return Some(part);
    }
    let (sc, edge) = (cam.project(c)?, cam.project(c + cam.up * r)?);
    let rp = sc.distance(edge);
    let wpx = (w / r * rp).max(2.0);
    let d = m.distance(sc);
    if (d - (rp + wpx)).abs() < 2.0 * wpx.max(5.0) {
        Some(Part::Roll)
    } else if d < rp {
        Some(Part::Free)
    } else {
        None
    }
}

fn pick_scale(cam: &Cam, f: &Frame, m: Pos2) -> Option<Part> {
    let b = &f.bounds;
    let mut best: Option<(u8, f32, Part)> = None;
    let mut consider = |kind: u8, part: Part| {
        let pos = handle_pos(b, part);
        let Some(s) = cam.project(pos) else { return };
        if s.distance(m) < SCALE_PICK_PX {
            let depth = cam.depth(pos);
            if best.is_none_or(|(k, d, _)| (kind, depth) < (k, d)) {
                best = Some((kind, depth, part));
            }
        }
    };
    for c in 0..8u8 {
        consider(0, Part::Corner(c));
    }
    if f.prims == 1 {
        for i in 0..3 {
            consider(1, Part::Face(i, true));
            consider(1, Part::Face(i, false));
        }
    }
    best.map(|(_, _, p)| p)
}

// ---- dragging

fn save_movers(world: &World, sel: &[ObjKey], linked: bool, now: Instant) -> Vec<Mover> {
    let prims: Vec<usize> = sel.iter().filter_map(|k| world.objects.index_of(k)).collect();
    let selected: std::collections::HashSet<usize> = prims.iter().copied().collect();
    let mut out = Vec::new();
    for idx in prims {
        let (Some(o), Some((pos, rot, _))) = (world.objects.get(idx), Scene::object_transform(world, idx, now, 0)) else {
            continue;
        };
        let Some(region_off) = world.region_offset(o.key.region) else {
            continue;
        };
        let parent_idx = (o.parent_id != 0).then(|| world.objects.parent_of(o)).flatten();
        let parent = parent_idx
            .and_then(|p| Scene::object_transform(world, p, now, 0))
            .map(|(p, r, _)| (p, r));
        let mut children = Vec::new();
        if o.parent_id == 0 {
            for c in super::family(world, idx).into_iter().skip(1) {
                if linked && selected.contains(&c) {
                    continue;
                }
                if let (Some(co), Some((cp, cr, _))) = (world.objects.get(c), Scene::object_transform(world, c, now, 0)) {
                    children.push(Child {
                        idx: c,
                        world_pos: cp,
                        world_rot: cr,
                        local_pos: co.position,
                        scale: co.scale,
                    });
                }
            }
        }
        out.push(Mover {
            idx,
            key: o.key,
            root: o.parent_id == 0,
            pos,
            rot,
            scale: o.scale,
            region_off,
            parent_idx,
            parent,
            children,
        });
    }
    out
}

/// Write new world transforms to the objects (local echo). `targets` maps
/// mover index -> (world pos, world rot, scale, uniform factor for children).
fn apply(world: &mut World, d: &Drag, targets: &[(Vec3, Quat, Vec3)]) {
    let new_world: HashMap<usize, (Vec3, Quat)> = d.movers.iter().zip(targets).map(|(m, t)| (m.idx, (t.0, t.1))).collect();
    for (m, &(wp, wr, sc)) in d.movers.iter().zip(targets) {
        let parent = m.parent_idx.and_then(|p| new_world.get(&p).copied()).or(m.parent);
        let (lp, lr) = match (m.root, parent) {
            (false, Some((pp, pr))) => (pr.inverse() * (wp - pp), (pr.inverse() * wr).normalize()),
            _ => (wp - m.region_off, wr.normalize()),
        };
        if let Some(o) = world.objects.get_mut(m.idx) {
            o.position = lp;
            o.rotation = lr;
            o.scale = sc;
            o.velocity = Vec3::ZERO;
            o.acceleration = Vec3::ZERO;
            o.angular_velocity = Vec3::ZERO;
            o.render.needs_records = true;
        }
        if !m.root {
            continue;
        }
        if d.linked {
            // root moved alone: its other prims stay where they are
            for c in &m.children {
                if let Some(o) = world.objects.get_mut(c.idx) {
                    o.position = wr.inverse() * (c.world_pos - wp);
                    o.rotation = (wr.inverse() * c.world_rot).normalize();
                    o.render.needs_records = true;
                }
            }
        } else if (sc - m.scale).length_squared() > 1e-10 {
            // whole linkset stretched (uniformly, corners only)
            let f = sc / m.scale.max(Vec3::splat(1e-6));
            for c in &m.children {
                if let Some(o) = world.objects.get_mut(c.idx) {
                    o.position = c.local_pos * f;
                    o.scale = (c.scale * f).clamp(Vec3::splat(MIN_PRIM_SCALE), Vec3::splat(MAX_PRIM_SCALE));
                    o.render.needs_records = true;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn drag_update(
    d: &mut Drag,
    world: &mut World,
    s: &BuildSettings,
    cam: &Cam,
    cursor: (f32, f32),
    mods: Mods,
    now: Instant,
    out: &mut Vec<BuildCmd>,
) {
    if !d.started {
        let moved = (cursor.0 - d.start_cursor.0).abs() + (cursor.1 - d.start_cursor.1).abs();
        if moved < DRAG_SLOP_PX {
            return;
        }
        d.started = true;
        // Shift-drag leaves a copy behind (LLManipTranslate::handleHover, selectDuplicate)
        if d.mode == EditMode::Move && mods.shift && !mods.ctrl {
            if d.linked {
                // Firestorm refuses to copy individual parts
            } else {
                let mut by_region: HashMap<RegionHandle, Vec<u32>> = HashMap::new();
                for m in &d.movers {
                    by_region.entry(m.key.region).or_default().push(m.key.local_id);
                }
                for (handle, ids) in by_region {
                    out.push(BuildCmd::Duplicate {
                        handle,
                        local_ids: ids,
                        offset: Vec3::ZERO,
                        flags: 0,
                        group_id: uuid::Uuid::nil(),
                    });
                }
            }
        }
    }
    let (o, dir) = cam.ray(cursor.0, cursor.1);
    match d.part {
        Part::Arrow(i) => {
            let Some(hit) = ray_plane(o, dir, d.pivot, d.normal) else { return };
            let rel = hit - d.start_hit;
            if rel.length() > MAX_DRAG_DISTANCE {
                return;
            }
            let mut mag = rel.dot(d.axis);
            d.in_snap = false;
            if s.snap {
                let plane_n = d.snap_axis.cross(d.axis);
                if let Some(p) = ray_plane(o, dir, d.pivot, plane_n) {
                    let off = (p - d.pivot).dot(d.snap_axis).abs();
                    if off > d.snap_offset {
                        d.in_snap = true;
                        let sub = subdivision_level(cam, d.pivot, d.axis, d.grid.scale[i], 3.0, s.max_subdivision());
                        let unit = d.grid.scale[i] / sub;
                        let dd = round_to((p - d.grid.origin).dot(d.axis), unit);
                        mag = dd - (d.pivot - d.grid.origin).dot(d.axis);
                    }
                }
            }
            d.delta = d.axis * mag;
        }
        Part::Plane(i) => {
            let Some(hit) = ray_plane(o, dir, d.pivot, d.normal) else { return };
            let mut rel = hit - d.start_hit;
            if rel.length() > MAX_DRAG_DISTANCE {
                return;
            }
            d.in_snap = false;
            if s.snap {
                let gi = d.grid.rotation.inverse();
                let mut local = gi * (d.pivot + rel - d.grid.origin);
                for k in 0..3 {
                    if k == i {
                        continue;
                    }
                    let a = d.grid.rotation * unit(k);
                    let sub = subdivision_level(cam, d.pivot, a, d.grid.scale[k], 3.0, s.max_subdivision());
                    local[k] = round_to(local[k], d.grid.scale[k] / sub);
                }
                rel = d.grid.rotation * local + d.grid.origin - d.pivot;
                d.in_snap = true;
            }
            d.delta = rel;
        }
        Part::Ring(_) => {
            let a = d.axis;
            let r = d.radius;
            let angle = if d.edge_on {
                // edge-on ring: drag across it (LLManipRotate, mCamEdgeOn)
                let up = a.cross(cam.at).normalize_or_zero();
                let (Some(p0), Some(p1)) = (cam.project(d.pivot), cam.project(d.pivot + up * r)) else {
                    return;
                };
                let screen_up = (p1 - p0).normalized();
                let px = (cursor.0 - d.start_cursor.0, cursor.1 - d.start_cursor.1);
                let along = (px.0 * screen_up.x + px.1 * screen_up.y) / cam.ppp;
                let r_px = p0.distance(p1).max(1.0);
                d.in_snap = false;
                let mut ang = along / (2.0 * r_px) * -std::f32::consts::FRAC_PI_2 * 2.0;
                let step = s.rotation_step.max(0.01).to_radians();
                ang -= ang % step;
                ang
            } else {
                let Some(q) = ray_plane(o, dir, d.pivot, a).or_else(|| ray_plane(o, -dir, d.pivot, a)) else {
                    return;
                };
                let v = q - d.pivot;
                if s.snap && v.length() > 2.0 * r {
                    d.in_snap = true;
                    let (ax1, ax2) = snap_axes(a, d.grid.rotation);
                    let mouse_ang = v.dot(ax1).atan2(v.dot(ax2)).to_degrees();
                    let snapped = (mouse_ang / SNAP_ANGLE).round() * SNAP_ANGLE;
                    let oa = (d.obj_axis - a * d.obj_axis.dot(a)).normalize_or_zero();
                    let obj_ang = oa.dot(ax1).atan2(oa.dot(ax2)).to_degrees();
                    (obj_ang - snapped).to_radians()
                } else {
                    d.in_snap = false;
                    let v = (v - a * v.dot(a)).normalize_or_zero();
                    let d0 = d.down_vec;
                    let mut ang = d0.cross(v).dot(a).atan2(d0.dot(v));
                    let step = s.rotation_step.max(0.01).to_radians();
                    ang -= ang % step;
                    ang
                }
            };
            d.rotation = Quat::from_axis_angle(a, angle);
        }
        Part::Free => {
            let cur = sphere_point(o, dir, d.pivot, d.radius, cam);
            d.rotation = Quat::from_rotation_arc(d.down_vec, cur);
        }
        Part::Roll => {
            let Some(c) = cam.project(d.pivot) else { return };
            let c = (c.x * cam.ppp, c.y * cam.ppp);
            let v0 = (d.start_cursor.0 - c.0, d.start_cursor.1 - c.1);
            let v1 = (cursor.0 - c.0, cursor.1 - c.1);
            let ang = (v0.0 * v1.1 - v0.1 * v1.0).atan2(v0.0 * v1.0 + v0.1 * v1.1);
            let mut ang = ang;
            let step = s.rotation_step.max(0.01).to_radians();
            ang -= ang % step;
            d.rotation = Quat::from_axis_angle(cam.at, ang);
        }
        Part::Corner(c) => scale_corner(d, s, cam, o, dir, cursor, c),
        Part::Face(i, pos) => scale_face(d, s, cam, o, dir, cursor, i, pos),
    }
    // new transforms
    let targets: Vec<(Vec3, Quat, Vec3)> = d
        .movers
        .iter()
        .map(|m| match d.mode {
            EditMode::Move | EditMode::Face | EditMode::Align => (m.pos + d.delta, m.rot, m.scale),
            EditMode::Rotate => (d.pivot + d.rotation * (m.pos - d.pivot), (d.rotation * m.rot).normalize(), m.scale),
            EditMode::Stretch => scale_target(d, m),
        })
        .collect();
    apply(world, d, &targets);
    d.changed = true;
    // stretching streams updates (LLManipScale::sendUpdates, 10 per second)
    if d.mode == EditMode::Stretch
        && !d.linked
        && d.last_send
            .is_none_or(|t| now.duration_since(t).as_secs_f32() >= SCALE_UPDATE_DELAY)
    {
        d.last_send = Some(now);
        send_updates(world, d, d.linked, out);
    }
}

/// Fixed-point stretch of one mover.
fn scale_target(d: &Drag, m: &Mover) -> (Vec3, Quat, Vec3) {
    match d.part {
        Part::Corner(_) => {
            let f = d.factor.x;
            (
                d.anchor + (m.pos - d.anchor) * f,
                m.rot,
                (m.scale * f).clamp(Vec3::splat(MIN_PRIM_SCALE), Vec3::splat(MAX_PRIM_SCALE)),
            )
        }
        Part::Face(i, _) => {
            // one prim: its axis nearest to the box axis
            let box_axis = d.bounds.rotation * unit(i);
            let local = m.rot.inverse() * box_axis;
            let k = if local.x.abs() >= local.y.abs() && local.x.abs() >= local.z.abs() {
                0
            } else if local.y.abs() >= local.z.abs() {
                1
            } else {
                2
            };
            let mut sc = m.scale;
            sc[k] = (m.scale[k] * d.factor[i]).clamp(MIN_PRIM_SCALE, MAX_PRIM_SCALE);
            let grown = sc[k] - m.scale[k];
            let pos = if (d.anchor - d.bounds.center).length_squared() < 1e-10 {
                m.pos
            } else {
                // the anchored face stays: the center moves half the growth
                let toward = (d.bounds.center - d.anchor).normalize_or_zero();
                m.pos + toward * grown * 0.5
            };
            (pos, m.rot, sc)
        }
        _ => (m.pos, m.rot, m.scale),
    }
}

/// Is the mouse far enough from the stretch line to snap (5% of the screen width)?
fn scale_snap_regime(cam: &Cam, cursor: (f32, f32), a: Vec3, b: Vec3) -> bool {
    let (Some(pa), Some(pb)) = (cam.project(a), cam.project(b)) else {
        return false;
    };
    let m = Pos2::new(cursor.0 / cam.ppp, cursor.1 / cam.ppp);
    let dir = (pb - pa).normalized();
    let rel = m - pa;
    let perp = (rel - dir * rel.dot(dir)).length();
    perp > SNAP_GUIDE_SCREEN_OFFSET * cam.width / cam.ppp
}

#[allow(clippy::too_many_arguments)]
fn scale_corner(d: &mut Drag, s: &BuildSettings, cam: &Cam, o: Vec3, dir: Vec3, cursor: (f32, f32), c: u8) {
    let b = d.bounds;
    let corner = b.center + b.rotation * (b.half * corner_signs(c));
    let Some(t) = nearest_on_line(o, dir, b.center, corner) else {
        return;
    };
    let mut f = if s.scale_uniform { t } else { 0.5 + 0.5 * t };
    let diag = (corner - d.anchor).length().max(1e-6);
    d.in_snap = false;
    let unit = s.grid_resolution.max(0.001) / s.max_subdivision();
    if s.snap && scale_snap_regime(cam, cursor, d.anchor, corner) {
        d.in_snap = true;
        // the largest box side lands on the grid
        let k = if b.half.x >= b.half.y && b.half.x >= b.half.z {
            0
        } else if b.half.y >= b.half.z {
            1
        } else {
            2
        };
        let size = 2.0 * b.half[k] * f;
        let snapped = round_to(size, unit).max(unit);
        f = snapped / (2.0 * b.half[k]).max(1e-6);
    }
    // every prim stays within the size limits
    let (mut lo, mut hi) = (0.0f32, f32::MAX);
    for m in &d.movers {
        let mn = m.scale.min_element().max(1e-6);
        let mx = m.scale.max_element().max(1e-6);
        lo = lo.max(MIN_PRIM_SCALE / mn);
        hi = hi.min(MAX_PRIM_SCALE / mx);
    }
    f = f.clamp(lo, hi.max(lo));
    d.factor = Vec3::splat(f);
    let k = if b.half.x >= b.half.y && b.half.x >= b.half.z {
        0
    } else if b.half.y >= b.half.z {
        1
    } else {
        2
    };
    d.scale_line = Some((d.anchor, (corner - d.anchor) / diag, unit, 2.0 * b.half[k] / diag));
}

#[allow(clippy::too_many_arguments)]
fn scale_face(d: &mut Drag, s: &BuildSettings, cam: &Cam, o: Vec3, dir: Vec3, cursor: (f32, f32), i: usize, pos: bool) {
    let b = d.bounds;
    let sgn = if pos { 1.0 } else { -1.0 };
    let face = b.center + b.rotation * unit(i) * b.half[i] * sgn;
    let Some(t) = nearest_on_line(o, dir, b.center, face) else { return };
    let size = 2.0 * b.half[i];
    // distance the face moved
    let mut delta = (t - 1.0) * b.half[i];
    if s.scale_uniform {
        delta *= 2.0;
    }
    let mut new_size = size + delta;
    d.in_snap = false;
    let unit = s.grid_resolution.max(0.001) / s.max_subdivision();
    if s.snap && scale_snap_regime(cam, cursor, d.anchor, face) {
        d.in_snap = true;
        new_size = round_to(new_size, unit).max(unit);
    }
    new_size = new_size.clamp(MIN_PRIM_SCALE, MAX_PRIM_SCALE);
    let mut f = Vec3::ONE;
    f[i] = new_size / size.max(1e-6);
    d.factor = f;
    let axis = (face - d.anchor).normalize_or_zero();
    let value_scale = if s.scale_uniform { 2.0 } else { 1.0 };
    d.scale_line = Some((d.anchor, axis, unit / value_scale, value_scale));
}

/// MultipleObjectUpdate for the movers (LLSelectMgr::sendMultipleUpdate):
/// whole linksets (UPD_LINKED_SETS) unless "Edit linked", roots first.
fn send_updates(world: &World, d: &Drag, linked: bool, out: &mut Vec<BuildCmd>) {
    let (pos, rot, scale, uniform) = match d.mode {
        EditMode::Move | EditMode::Face | EditMode::Align => (true, false, false, false),
        EditMode::Rotate => (true, true, false, false),
        EditMode::Stretch => (true, false, true, matches!(d.part, Part::Corner(_))),
    };
    let mut by_region: HashMap<RegionHandle, Vec<TransformUpdate>> = HashMap::new();
    let mut ordered: Vec<&Mover> = d.movers.iter().collect();
    ordered.sort_by_key(|m| !m.root);
    for m in ordered {
        let Some(o) = world.objects.get(m.idx) else { continue };
        by_region.entry(m.key.region).or_default().push(TransformUpdate {
            local_id: m.key.local_id,
            position: pos.then_some(o.position),
            rotation: rot.then_some(o.rotation),
            scale: scale.then_some(o.scale),
            linked: !linked && m.root,
            uniform,
        });
    }
    for (handle, updates) in by_region {
        out.push(BuildCmd::Transform { handle, updates });
    }
}

// ---- guides

/// LLManip::renderXYZ: the values at the top of the screen.
fn readout(p: &Painter3d, v: [f32; 3]) {
    let cam = p.cam;
    let cx = cam.width / cam.ppp * 0.5;
    let rect = egui::Rect::from_min_size(Pos2::new(cx - 117.5, 56.0), egui::vec2(235.0, 30.0));
    p.painter.rect_filled(rect, 6.0, rgba(0.0, 0.0, 0.0, 0.7));
    let colors = [rgba(1.0, 0.5, 0.5, 1.0), rgba(0.5, 1.0, 0.5, 1.0), rgba(0.5, 0.5, 1.0, 1.0)];
    for (k, x) in [-102.0f32, -27.0, 48.0].into_iter().enumerate() {
        let pos = Pos2::new(cx + x, rect.center().y);
        let text = format!("{:.3}", v[k]);
        let font = egui::FontId::proportional(13.0);
        p.painter.text(
            pos + egui::vec2(1.0, 1.0),
            egui::Align2::LEFT_CENTER,
            &text,
            font.clone(),
            Color32::BLACK,
        );
        p.painter.text(pos, egui::Align2::LEFT_CENTER, &text, font, colors[k]);
    }
}

/// Grid in the plane being dragged (LLManipTranslate::renderGrid): lines
/// on the grid units around the pivot, fading out radially.
fn draw_plane_grid(p: &mut Painter3d, s: &BuildSettings, f: &Frame, d: &Drag, i: usize) {
    let cam = p.cam;
    let size = s.grid_draw_size.max(1.0);
    let g = d.grid.rotation;
    let (u, v) = ((i + 1) % 3, (i + 2) % 3);
    let au = g * unit(u);
    let av = g * unit(v);
    let piv = f.pivot;
    let local = g.inverse() * (piv - d.grid.origin);
    let opacity = s.grid_opacity;
    for (a, b, ka, kb) in [(au, av, u, v), (av, au, v, u)] {
        // spacing: grid units, coarser when they would be closer than 6 px
        let sub = subdivision_level(cam, piv, b, d.grid.scale[kb], 6.0, 1.0);
        let step = d.grid.scale[kb] / sub;
        let along_step = (size / 12.0).max(step);
        let first = ((local[kb] - size * 1.5) / step).ceil() as i32;
        let last = ((local[kb] + size * 1.5) / step).floor() as i32;
        if last - first > 400 {
            continue;
        }
        for n in first..=last {
            let off_b = n as f32 * step - local[kb];
            let mut t = -size * 1.5;
            while t < size * 1.5 {
                let t1 = t + along_step;
                let mid = ((t + t1) * 0.5).hypot(off_b);
                let alpha = (1.0 - mid / size).max(0.0).sqrt() * opacity;
                if alpha > 0.01 {
                    let p0 = piv + b * off_b + a * t;
                    let p1 = piv + b * off_b + a * t1;
                    p.line(p0, p1, rgba(1.0, 1.0, 1.0, alpha * 0.6), 1.0);
                }
                t = t1;
            }
        }
        let _ = ka;
    }
}

/// Ruler along a stretch (LLManipScale::renderSnapGuides): ticks at grid
/// sizes from the fixed point, labeled with the size in meters.
fn draw_scale_ruler(p: &mut Painter3d, s: &BuildSettings, start: Vec3, dir: Vec3, unit_m: f32, value_scale: f32, in_snap: bool) {
    let cam = p.cam;
    let opacity = s.grid_opacity;
    let white = Color32::WHITE;
    let side = dir.cross(cam.at).try_normalize().unwrap_or(cam.up);
    let side = if side.dot(cam.up) < 0.0 { -side } else { side };
    let off = cam.meters_for_pixels(start, SNAP_GUIDE_SCREEN_OFFSET * cam.width);
    let range = (start - cam.eye).length();
    let len = SNAP_GUIDE_SCREEN_SIZE * cam.width * range / cam.pixel_meter_ratio();
    let base = start + side * off;
    p.line_gradient(base, base + dir * len, fade(white, opacity), fade(white, opacity * 0.1), 1.0);
    let step = unit_m / value_scale.max(1e-6);
    if step <= 0.0 {
        return;
    }
    let sub = subdivision_level(cam, start, dir, step, 8.0, 1.0);
    let step = step / sub;
    let n = ((len / step) as i32).min(300);
    let label_sub = subdivision_level(cam, start, dir, step, 60.0, 1.0);
    let every = (1.0 / label_sub).round().max(1.0) as i32;
    for k in 1..=n {
        let at = base + dir * step * k as f32;
        let alpha = opacity * (1.0 - k as f32 / n.max(1) as f32);
        p.guide_line(at, at - side * off * 0.5, white, alpha);
        if k % every == 0 {
            let value = step * k as f32 * value_scale;
            p.tick_value(
                at + side * off * 0.4,
                value,
                "m",
                fade(white, alpha.max(0.2) * if in_snap { 1.0 } else { 0.8 }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_axis_choice_avoids_view_direction() {
        // looking mostly along X while dragging the Y arrow: rulers stand on Z
        assert_eq!(snap_offset_axis(1, Vec3::new(0.9, 0.3, 0.2)), 2);
        // looking down (Z dominant) dragging X: rulers on Y
        assert_eq!(snap_offset_axis(0, Vec3::new(0.1, 0.2, 0.95)), 1);
    }

    #[test]
    fn snap_axes_are_orthonormal() {
        let (a1, a2) = snap_axes(Vec3::Z, Quat::IDENTITY);
        assert!((a1 - Vec3::X).length() < 1e-5);
        assert!((a2 - Vec3::Y).length() < 1e-5);
    }
}
