//! « Aligner »: align or pack the selected linksets against a side of
//! their common bounding box.
//!
//! Port of QToolAlign (indra/newview/qtoolalign.cpp, Firestorm, originally
//! LGPL 2.1): an axis-aligned box around the selection, a cone on each of
//! its six sides, pointing inwards; a click on a cone moves every object
//! along that axis, the way it points, until it touches the opposite side
//! of the box (« force », the default) or, with Shift, packs them there
//! without overlapping.

use super::draw::{Painter3d, rgba};
use super::geom::Cam;
use super::{BuildTool, family};
use crate::scene::Scene;
use crate::world::World;
use aurora_net::build::{BuildCmd, TransformUpdate};
use glam::Vec3;
use std::collections::HashMap;
use std::time::Instant;

/// MANIPULATOR_SIZE (pixels) and MANIPULATOR_SELECT_SIZE.
const MANIPULATOR_SIZE: f32 = 5.0;
const MANIPULATOR_SELECT_SIZE: f32 = 20.0;
/// F_ALMOST_ZERO.
const ALMOST_ZERO: f32 = 0.0001;
/// FLAGS_OBJECT_MOVE.
const FLAGS_OBJECT_MOVE: u32 = 1 << 8;

/// An axis-aligned box (agent / world space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    fn empty() -> Aabb {
        Aabb {
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        }
    }
    fn add(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    fn extent(&self) -> Vec3 {
        self.max - self.min
    }
    fn at(center: Vec3, extent: Vec3) -> Aabb {
        Aabb {
            min: center - extent * 0.5,
            max: center + extent * 0.5,
        }
    }
}

/// bbox_overlap: strictly overlapping on all three axes.
fn overlap(a: &Aabb, b: &Aabb) -> bool {
    let d = a.center() - b.center();
    let h = (a.extent() + b.extent()) * 0.5;
    (0..3).all(|i| d[i].abs() < h[i] - ALMOST_ZERO)
}

/// The near corner of a box along `axis` for `direction` (-1 / 1).
fn corner(b: &Aabb, axis: usize, direction: f32) -> f32 {
    (b.center() - b.extent() * 0.5 * direction)[axis]
}

/// QToolAlign::align: new box of each object (same order as `boxes`).
/// `target`: the selection's box.
pub fn align_boxes(boxes: &[Aabb], target: &Aabb, axis: usize, direction: f32, force: bool) -> Vec<Aabb> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| (direction * corner(&boxes[a], axis, direction)).total_cmp(&(direction * corner(&boxes[b], axis, direction))));
    let mut new = boxes.to_vec();
    for i in 0..order.len() {
        let this = boxes[order[i]];
        let this_corner = corner(&this, axis, direction);
        let mut target_corner = corner(target, axis, direction);
        let mut smallest = direction * 9_999_999.0;
        for j in 0..=i {
            let mut center = this.center();
            center[axis] += target_corner - this_corner;
            let candidate = Aabb::at(center, this.extent());
            let blocked = !force && (0..i).any(|k| overlap(&new[order[k]], &candidate));
            if !blocked {
                let v = corner(&candidate, axis, direction);
                if direction * v < direction * smallest {
                    smallest = v;
                    new[order[i]] = candidate;
                }
            }
            // next try: against the far side of an object already placed
            let next = new[order[j]];
            target_corner = (next.center() + next.extent() * 0.5 * direction)[axis];
        }
    }
    new
}

#[derive(Debug, Default)]
pub struct Align {
    /// Highlighted cone: (axis, direction).
    pub hover: Option<(usize, f32)>,
    /// Without Shift: « force » mode (double cones).
    pub force: bool,
}

/// Axis-aligned box of a prim (getBoundingBoxAgent).
fn prim_box(world: &World, idx: usize, now: Instant) -> Option<Aabb> {
    let o = world.objects.get(idx)?;
    let (p, r, _) = Scene::object_transform(world, idx, now, 0)?;
    let mut b = Aabb::empty();
    let h = o.scale * 0.5;
    for c in 0..8 {
        let s = Vec3::new(
            if c & 1 != 0 { 1.0 } else { -1.0 },
            if c & 2 != 0 { 1.0 } else { -1.0 },
            if c & 4 != 0 { 1.0 } else { -1.0 },
        );
        b.add(p + r * (h * s));
    }
    Some(b)
}

fn linkset_box(world: &World, root: usize, now: Instant) -> Option<Aabb> {
    let mut b = Aabb::empty();
    for i in family(world, root) {
        if let Some(pb) = prim_box(world, i, now) {
            b.add(pb.min);
            b.add(pb.max);
        }
    }
    (b.min.x <= b.max.x).then_some(b)
}

impl BuildTool {
    /// QToolAlign::canAffectSelection: every object movable (and modifiable
    /// with « Modification liée »).
    fn align_allowed(&self, world: &World) -> bool {
        let prims = self.sel_prims(world);
        !prims.is_empty()
            && prims.iter().all(|&i| {
                world
                    .objects
                    .get(i)
                    .is_some_and(|o| o.update_flags & FLAGS_OBJECT_MOVE != 0 && (!self.selection_linked || self.can_modify(world, i)))
            })
    }

    /// Box around the selection (get_selection_axis_aligned_bbox).
    fn align_box(&self, world: &World, now: Instant) -> Option<Aabb> {
        let mut b = Aabb::empty();
        for r in self.roots(world) {
            let lb = linkset_box(world, r, now)?;
            b.add(lb.min);
            b.add(lb.max);
        }
        (b.min.x <= b.max.x).then_some(b)
    }

    fn cone_size(cam: &Cam, center: Vec3) -> f32 {
        let d = cam.eye.distance(center);
        if d < ALMOST_ZERO {
            return MANIPULATOR_SIZE;
        }
        MANIPULATOR_SIZE * d * (MANIPULATOR_SIZE / cam.height * cam.fov_y).tan()
    }

    /// handleHover / findSelectedManipulator.
    pub fn align_hover(&mut self, world: &World, cursor: (f32, f32), shift: bool) {
        self.align.force = !shift;
        self.align.hover = None;
        if !self.align_allowed(world) {
            return;
        }
        let Some(b) = self.align_box(world, Instant::now()) else { return };
        let c = b.center();
        let e = b.extent();
        for axis in 0..3 {
            for direction in [-1.0f32, 1.0] {
                let mut p = c;
                p[axis] += direction * e[axis] * 0.5;
                if let Some((x, y)) = self.cam.project_px(p) {
                    let (dx, dy) = ((x - cursor.0) / self.cam.ppp, (y - cursor.1) / self.cam.ppp);
                    if dx * dx + dy * dy < MANIPULATOR_SELECT_SIZE * MANIPULATOR_SELECT_SIZE {
                        self.align.hover = Some((axis, direction));
                        return;
                    }
                }
            }
        }
    }

    /// Click on a cone: QToolAlign::align, then MultipleObjectUpdate (position).
    pub fn align_apply(&mut self, world: &mut World) -> bool {
        let Some((axis, direction)) = self.align.hover else {
            return false;
        };
        let now = Instant::now();
        let Some(target) = self.align_box(world, now) else {
            return false;
        };
        let roots = self.roots(world);
        let boxes: Vec<Aabb> = roots.iter().filter_map(|&r| linkset_box(world, r, now)).collect();
        if boxes.len() != roots.len() {
            return false;
        }
        let new = align_boxes(&boxes, &target, axis, direction, self.align.force);
        let mut by_region: HashMap<aurora_net::RegionHandle, Vec<TransformUpdate>> = HashMap::new();
        for (i, &r) in roots.iter().enumerate() {
            let delta = new[i].center() - boxes[i].center();
            if delta.length_squared() < 1e-10 {
                continue;
            }
            let Some(o) = world.objects.get_mut(r) else { continue };
            o.position += delta;
            o.velocity = Vec3::ZERO;
            o.render.needs_records = true;
            by_region.entry(o.key.region).or_default().push(TransformUpdate {
                local_id: o.key.local_id,
                position: Some(o.position),
                rotation: None,
                scale: None,
                linked: true,
                uniform: false,
            });
        }
        for (handle, updates) in by_region {
            self.send(BuildCmd::Transform { handle, updates });
        }
        true
    }

    /// QToolAlign::render: the grey box and the six cones.
    pub fn align_draw(&self, p: &mut Painter3d, world: &World) {
        if !self.align_allowed(world) {
            return;
        }
        let Some(b) = self.align_box(world, Instant::now()) else { return };
        let (c, e) = (b.center(), b.extent());
        p.cube(c, glam::Quat::IDENTITY, e * 0.5, rgba(0.7, 0.7, 0.7, 0.1));
        let base = Self::cone_size(p.cam, c);
        for axis in 0..3 {
            for direction in [-1.0f32, 1.0] {
                let lit = self.align.hover == Some((axis, direction));
                let size = if lit { base * 1.25 } else { base };
                let k = if lit { 1.5 } else { 1.0 };
                let mut col = [0.0, 0.0, 0.0];
                col[axis] = 0.7 * k;
                let color = rgba(col[0], col[1], col[2], 0.5 * k);
                let arrows = if self.align.force { 2 } else { 1 };
                for i in 0..arrows {
                    let mut v = Vec3::ZERO;
                    v[axis] = direction * (e[axis] * 0.5 + i as f32 * size / 3.0);
                    let center = c + v;
                    // the cone points at the box (shortestArc(z, -axis_vector))
                    let dir = -v.normalize_or_zero();
                    p.cone(center - dir * size * 0.375, dir, size * 0.5, size * 0.75, color);
                }
            }
        }
        p.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f32, w: f32) -> Aabb {
        Aabb::at(Vec3::new(x, 0.0, 0.0), Vec3::new(w, 1.0, 1.0))
    }

    #[test]
    fn force_aligns_on_the_side() {
        let boxes = [bx(0.0, 1.0), bx(5.0, 1.0)];
        let target = Aabb {
            min: Vec3::new(-0.5, -0.5, -0.5),
            max: Vec3::new(5.5, 0.5, 0.5),
        };
        // the cone on the -X side points to +X: both go against x = 5.5
        let new = align_boxes(&boxes, &target, 0, -1.0, true);
        assert!((new[0].max.x - 5.5).abs() < 1e-5);
        assert!((new[1].max.x - 5.5).abs() < 1e-5);
        // and the +X cone against x = -0.5
        let new = align_boxes(&boxes, &target, 0, 1.0, true);
        assert!((new[1].min.x + 0.5).abs() < 1e-5);
    }

    #[test]
    fn shift_packs_without_overlap() {
        let boxes = [bx(0.0, 1.0), bx(5.0, 1.0)];
        let target = Aabb {
            min: Vec3::new(-0.5, -0.5, -0.5),
            max: Vec3::new(5.5, 0.5, 0.5),
        };
        let new = align_boxes(&boxes, &target, 0, -1.0, false);
        assert!((new[1].max.x - 5.5).abs() < 1e-5);
        // stacked against the first one placed
        assert!((new[0].max.x - 4.5).abs() < 1e-5);
        assert!(!overlap(&new[0], &new[1]));
    }
}
