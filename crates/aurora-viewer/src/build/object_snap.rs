//! "Snap to objects": while a move arrow or plane handle is dragged, the
//! selection snaps flush against nearby prims (their box, or what a mesh
//! shows, below), with guides on what it snapped to, like the smart guides
//! of design tools.
//!
//! An Aurora feature, with no Legacy Viewer equivalent; the move still goes
//! out as any other (`TransformUpdate` on release), so it needs only the
//! move right on the selection, none on the prims it snaps to.
//!
//! Every face direction of a target is a one-dimensional snap: the
//! selection's extent along that direction (its support, so a rotated
//! selection rests on its corner) meets the target's face from outside
//! (touching), lies in the same plane (flush) or shares its centre. An
//! arrow only takes the snaps it can reach along its axis; a plane handle
//! combines two of them, which puts an edge on an edge.
//!
//! The selection never passes through a target either: each step of the
//! drag is swept against their boxes and stops at the first face met (`stop`,
//! the separating axis test along the move); a plane drag then slides along
//! that face. A target the selection already overlaps is left out of it, so
//! a box rezzed inside another can still be moved out.
//!
//! Only plain box prims are their box. A mesh or a shaped prim (cut, hollow,
//! a cylinder…) is its visible surface (`Surface`): the triangles picking
//! uses, so the floor of a mesh house holds what is put on it, where its
//! box would only offer the outside of the whole house. On a surface, a
//! snap is a contact: a sweep within the threshold that meets a triangle.

use super::Bounds;
use crate::scene::Scene;
use crate::world::World;
use aurora_prim::VolumeParams;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// A snap applies when it moves the selection by this many pixels or fewer.
pub const SNAP_PX: f32 = 8.0;
/// Prims whose box is this close to the selection's box are targets (m).
pub const RADIUS: f32 = 10.0;
/// The nearest targets kept.
pub const MAX_TARGETS: usize = 256;
/// Triangles of the nearest surfaces kept; a surface past them is left out.
pub const MAX_TRIANGLES: usize = 200_000;
/// A face whose normal makes less than this cosine with the arrow (or the
/// handle's plane) would need a long move for a small snap: left out.
const MIN_ALONG: f32 = 0.3;
/// Two snaps of a plane drag must be this far from parallel (sine).
const MIN_CROSS: f32 = 0.5;
/// Score penalty in pixels: a real contact wins over a nearby plane
/// alignment, which wins over a centre.
const PRIORITY_PX: f32 = 2.0;
/// Overlap tolerance of a contact (m).
const CONTACT_EPS: f32 = 0.001;
/// Faces count this much smaller in the sweep (m): a selection resting on a
/// face slides along it, and stops exactly on it.
const SKIN: f32 = 0.0005;
/// The pad drawn on a surface under the selection, times its footprint.
const PAD_SCALE: f32 = 1.25;
/// A selection face this parallel to a target face (cosine) can be flush
/// with it or line up with its centre; otherwise only a contact snaps (a
/// corner resting on a slope), not the endless plane of a tilted face.
const PARALLEL: f32 = 0.9995;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Opposite faces meet: the selection rests on the target.
    Touch,
    /// The same face plane on both.
    Flush,
    /// The centres line up along the face normal.
    Center,
}

/// How the drag may move the selection.
#[derive(Debug, Clone, Copy)]
pub enum Constraint {
    /// Along this unit axis (a move arrow).
    Axis(Vec3),
    /// In the plane of this unit normal (a plane handle).
    Plane(Vec3),
}

/// One applied snap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Index in the targets.
    pub target: usize,
    /// Target axis whose faces are snapped to.
    pub axis: usize,
    pub kind: Kind,
    /// Target face (-1 / +1), 0 for a centre.
    pub side: f32,
    /// The selection's side along the face normal (-1 / +1), 0 for a centre.
    pub sel_side: f32,
    /// Both boxes overlap across the face (a real contact).
    pub contact: bool,
    /// The triangle met on a surface (`target` is then the surface's index).
    pub tri: Option<[Vec3; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    /// Added to the drag's move.
    pub offset: Vec3,
    pub hits: Vec<Hit>,
}

/// The visible surface of a target, in world space.
#[derive(Debug, Clone, Default)]
pub struct Surface {
    pub tris: Vec<[Vec3; 3]>,
    min: Vec3,
    max: Vec3,
}

impl Surface {
    pub fn new(tris: Vec<[Vec3; 3]>) -> Self {
        let min = tris.iter().flatten().fold(Vec3::splat(f32::MAX), |m, v| m.min(*v));
        let max = tris.iter().flatten().fold(Vec3::splat(f32::MIN), |m, v| m.max(*v));
        Self { tris, min, max }
    }
}

/// Targets collected for one drag, with where they were collected.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    pub boxes: Vec<Bounds>,
    pub surfaces: Vec<Surface>,
    pub around: Vec3,
    /// What each prim was, at its box then: collected again, a prim that
    /// has not moved keeps it (its triangles are not read again).
    seen: HashMap<usize, (Bounds, Seen)>,
}

#[derive(Debug, Clone, Copy)]
enum Seen {
    Box,
    /// Its index in `surfaces`.
    Surface(usize),
    /// Nothing drawn.
    Hidden,
}

/// Whether two boxes are the same, to a hundredth of a millimetre.
pub(super) fn same_box(a: &Bounds, b: &Bounds) -> bool {
    a.center.abs_diff_eq(b.center, 1e-5) && a.half.abs_diff_eq(b.half, 1e-5) && a.rotation.abs_diff_eq(b.rotation, 1e-6)
}

/// A prim that fills its box: the default box shape, unsculpted.
pub fn is_box(v: &VolumeParams) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    let (pr, pa) = (&v.profile, &v.path);
    v.sculpt.is_none()
        && pr.curve_type & 0x0f == LL_PCODE_PROFILE_SQUARE
        && near(pr.begin, 0.0)
        && near(pr.end, 1.0)
        && near(pr.hollow, 0.0)
        && pa.curve_type == LL_PCODE_PATH_LINE
        && near(pa.begin, 0.0)
        && near(pa.end, 1.0)
        && pa.scale.iter().all(|x| near(*x, 1.0))
        && pa.shear.iter().chain(&pa.taper).all(|x| near(*x, 0.0))
        && near(pa.twist_begin, 0.0)
        && near(pa.twist_end, 0.0)
}

fn unit(i: usize) -> Vec3 {
    let mut v = Vec3::ZERO;
    v[i] = 1.0;
    v
}

fn axis_of(b: &Bounds, i: usize) -> Vec3 {
    b.rotation * unit(i)
}

/// Half extent of a box along a unit direction.
pub(super) fn support(b: &Bounds, u: Vec3) -> f32 {
    (0..3).map(|i| (b.half[i] * axis_of(b, i).dot(u)).abs()).sum()
}

fn interval(b: &Bounds, u: Vec3) -> (f32, f32) {
    let (c, r) = (b.center.dot(u), support(b, u));
    (c - r, c + r)
}

pub fn moved(b: &Bounds, by: Vec3) -> Bounds {
    Bounds {
        center: b.center + by,
        ..*b
    }
}

/// The smallest move allowed by `c` that shifts the selection by `delta`
/// along the unit direction `u`.
fn reach(c: Constraint, u: Vec3, delta: f32) -> Option<Vec3> {
    match c {
        Constraint::Axis(a) => {
            let along = a.dot(u);
            (along.abs() >= MIN_ALONG).then(|| a * (delta / along))
        }
        Constraint::Plane(n) => {
            let w = u - n * n.dot(u);
            let len2 = w.length_squared();
            (len2 >= MIN_ALONG * MIN_ALONG).then(|| w * (delta / len2))
        }
    }
}

/// Does `sel` overlap `t` on the two target axes other than `k`?
fn contact(sel: &Bounds, t: &Bounds, k: usize) -> bool {
    (0..3).filter(|i| *i != k).all(|i| {
        let u = axis_of(t, i);
        let (s0, s1) = interval(sel, u);
        let c = t.center.dot(u);
        s1 > c - t.half[i] + CONTACT_EPS && s0 < c + t.half[i] - CONTACT_EPS
    })
}

struct Candidate {
    hit: Hit,
    /// The selection's shift along the face normal `u`.
    delta: f32,
    u: Vec3,
    offset: Vec3,
    score: f32,
}

/// Every snap within `SNAP_PX` of the selection `sel` (already moved by the
/// drag), best first. `px(from, offset)` measures a move on screen.
fn candidates(sel: &Bounds, targets: &[Bounds], c: Constraint, px: &dyn Fn(Vec3, Vec3) -> f32) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (ti, t) in targets.iter().enumerate() {
        for k in 0..3 {
            let u = axis_of(t, k);
            let parallel = (0..3).any(|i| axis_of(sel, i).dot(u).abs() >= PARALLEL);
            let (s0, s1) = interval(sel, u);
            let tc = t.center.dot(u);
            let (c0, c1) = (tc - t.half[k], tc + t.half[k]);
            for (kind, side, sel_side, delta) in [
                (Kind::Touch, 1.0, -1.0, c1 - s0),
                (Kind::Touch, -1.0, 1.0, c0 - s1),
                (Kind::Flush, 1.0, 1.0, c1 - s1),
                (Kind::Flush, -1.0, -1.0, c0 - s0),
                (Kind::Center, 0.0, 0.0, tc - sel.center.dot(u)),
            ] {
                let Some(offset) = reach(c, u, delta) else { continue };
                let pixels = px(sel.center, offset);
                // (a NaN is out too)
                if pixels.is_nan() || pixels > SNAP_PX {
                    continue;
                }
                let touching = kind == Kind::Touch && contact(&moved(sel, offset), t, k);
                if !touching && !parallel {
                    continue;
                }
                let priority = match kind {
                    Kind::Touch if touching => 0.0,
                    Kind::Touch | Kind::Flush => 1.0,
                    Kind::Center => 2.0,
                };
                out.push(Candidate {
                    hit: Hit {
                        target: ti,
                        axis: k,
                        kind,
                        side,
                        sel_side,
                        contact: touching,
                        tri: None,
                    },
                    delta,
                    u,
                    offset,
                    score: pixels + priority * PRIORITY_PX,
                });
            }
        }
    }
    out.sort_by(|a, b| a.score.total_cmp(&b.score));
    out
}

/// Contacts with the surfaces within `SNAP_PX`: a short sweep each way the
/// drag may move (along the arrow, or the plane's two axes).
fn surface_candidates(sel: &Bounds, surfaces: &[Surface], c: Constraint, px: &dyn Fn(Vec3, Vec3) -> f32) -> Vec<Candidate> {
    let dirs = match c {
        Constraint::Axis(a) => vec![a, -a],
        Constraint::Plane(n) => {
            // the world axis furthest from the normal, in the plane
            let w = [Vec3::X, Vec3::Y, Vec3::Z]
                .into_iter()
                .min_by(|a, b| a.dot(n).abs().total_cmp(&b.dot(n).abs()))
                .unwrap_or(Vec3::X);
            let e1 = (w - n * n.dot(w)).normalize_or_zero();
            let e2 = n.cross(e1);
            vec![e1, -e1, e2, -e2]
        }
    };
    let mut out = Vec::new();
    for dir in dirs {
        let per_m = px(sel.center, dir);
        if !(per_m.is_finite() && per_m > 0.0) {
            continue;
        }
        let (offset, hit) = stop(sel, dir * (SNAP_PX / per_m), &[], surfaces, Constraint::Axis(dir));
        if let Some(hit) = hit {
            out.push(Candidate {
                hit,
                delta: offset.dot(dir),
                u: dir,
                offset,
                score: px(sel.center, offset),
            });
        }
    }
    out
}

/// The snap of the selection `sel` (where the drag puts it) against the
/// `boxes` and `surfaces`, if one is within `SNAP_PX`.
pub fn find(sel: &Bounds, boxes: &[Bounds], surfaces: &[Surface], c: Constraint, px: &dyn Fn(Vec3, Vec3) -> f32) -> Option<Snap> {
    let mut found = candidates(sel, boxes, c, px);
    if !surfaces.is_empty() {
        found.extend(surface_candidates(sel, surfaces, c, px));
        found.sort_by(|a, b| a.score.total_cmp(&b.score));
    }
    let first = found.first()?;
    let mut snap = Snap {
        offset: first.offset,
        hits: vec![first.hit],
    };
    // a plane drag also takes the best snap across the first one
    if let Constraint::Plane(n) = c {
        let in_plane = |u: Vec3| (u - n * n.dot(u)).normalize_or_zero();
        let w1 = in_plane(first.u);
        if let Some(second) = found.iter().find(|s| w1.cross(in_plane(s.u)).length() >= MIN_CROSS) {
            // x in the plane with x·u1 = d1 and x·u2 = d2 (x·u = x·w there)
            let (a, b) = (first.u - n * n.dot(first.u), second.u - n * n.dot(second.u));
            let (aa, ab, bb) = (a.dot(a), a.dot(b), b.dot(b));
            let det = aa * bb - ab * ab;
            if det.abs() > 1e-6 {
                let x = (first.delta * bb - second.delta * ab) / det;
                let y = (second.delta * aa - first.delta * ab) / det;
                snap.offset = a * x + b * y;
                snap.hits.push(second.hit);
            }
        }
    }
    Some(snap)
}

/// What a sweep meets: a target's box, or a triangle of a surface.
#[derive(Clone, Copy)]
enum Shape<'a> {
    Box(&'a Bounds),
    Tri(&'a [Vec3; 3]),
}

impl Shape<'_> {
    fn interval(&self, l: Vec3) -> (f32, f32) {
        match self {
            Shape::Box(b) => interval(b, l),
            Shape::Tri(t) => {
                let d = t.map(|v| v.dot(l));
                (d[0].min(d[1]).min(d[2]), d[0].max(d[1]).max(d[2]))
            }
        }
    }
    /// Its face normals and edge directions, for the separating axes.
    fn axes(&self) -> ([Vec3; 3], [Vec3; 3]) {
        match self {
            Shape::Box(b) => {
                let a = [0, 1, 2].map(|i| axis_of(b, i));
                (a, a)
            }
            Shape::Tri(t) => {
                let e = [t[1] - t[0], t[2] - t[1], t[0] - t[2]];
                let n = e[0].cross(e[1]);
                ([n, n, n], e)
            }
        }
    }
}

/// When the selection `sel`, moved by `by`, first enters `shape` (fraction
/// of `by`), and the axis it meets, pointing back along the move; None if it
/// does not, or if it overlaps `shape` already. Shapes count `SKIN` smaller
/// on every side, so resting on a face is not entering it.
fn entry(sel: &Bounds, by: Vec3, shape: Shape) -> Option<(f32, Vec3)> {
    let a = [0, 1, 2].map(|i| axis_of(sel, i));
    let (normals, edges) = shape.axes();
    let crosses = a.iter().flat_map(|x| edges.iter().map(move |y| x.cross(*y)));
    let (mut enter, mut exit, mut normal) = (f32::NEG_INFINITY, f32::INFINITY, Vec3::ZERO);
    for axis in a.into_iter().chain(normals).chain(crosses) {
        let Some(l) = axis.try_normalize() else { continue };
        let (s0, s1) = interval(sel, l);
        let (c0, c1) = shape.interval(l);
        let (c0, c1) = (c0 + SKIN, c1 - SKIN);
        let v = by.dot(l);
        if v.abs() < 1e-7 {
            if s1 <= c0 || s0 >= c1 {
                return None;
            }
            continue;
        }
        let (ta, tb) = ((c0 - s1) / v, (c1 - s0) / v);
        let (t0, t1) = (ta.min(tb), ta.max(tb));
        if t0 > enter {
            enter = t0;
            normal = l * -v.signum();
        }
        exit = exit.min(t1);
        if enter >= exit {
            return None;
        }
    }
    (0.0..=1.0).contains(&enter).then_some((enter, normal))
}

/// The first box or triangle the selection meets moving by `by`: fraction,
/// axis, and its hit (without its side for a box).
fn first_met(sel: &Bounds, by: Vec3, boxes: &[Bounds], surfaces: &[Surface]) -> Option<(f32, Vec3, Hit)> {
    let hit = |target, tri| Hit {
        target,
        axis: 0,
        kind: Kind::Touch,
        side: 0.0,
        sel_side: 0.0,
        contact: true,
        tri,
    };
    let mut best: Option<(f32, Vec3, Hit)> = None;
    let mut keep = |found: Option<(f32, Vec3)>, h: Hit| {
        if let Some((at, n)) = found
            && best.as_ref().is_none_or(|b| at < b.0)
        {
            best = Some((at, n, h));
        }
    };
    for (i, b) in boxes.iter().enumerate() {
        keep(entry(sel, by, Shape::Box(b)), hit(i, None));
    }
    if !surfaces.is_empty() {
        // only the triangles near the swept box
        let reach = Vec3::splat(support(sel, Vec3::X).max(support(sel, Vec3::Y)).max(support(sel, Vec3::Z)) + SKIN);
        let (lo, hi) = (sel.center.min(sel.center + by) - reach, sel.center.max(sel.center + by) + reach);
        for (i, s) in surfaces.iter().enumerate() {
            if s.max.cmplt(lo).any() || s.min.cmpgt(hi).any() {
                continue;
            }
            for t in &s.tris {
                let (tmin, tmax) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
                if tmax.cmplt(lo).any() || tmin.cmpgt(hi).any() {
                    continue;
                }
                keep(entry(sel, by, Shape::Tri(t)), hit(i, Some(*t)));
            }
        }
    }
    best
}

/// How far the selection `sel` may move of `by` within `c` before it meets
/// a box or a surface, sliding along what it meets in a plane drag; with
/// the contact met last, for the guides.
pub fn stop(sel: &Bounds, by: Vec3, boxes: &[Bounds], surfaces: &[Surface], c: Constraint) -> (Vec3, Option<Hit>) {
    let (mut moved_by, mut rest, mut hit) = (Vec3::ZERO, by, None);
    for _ in 0..3 {
        let Some((at, n, mut h)) = first_met(&moved(sel, moved_by), rest, boxes, surfaces) else {
            moved_by += rest;
            break;
        };
        // back off the skin: right on the face
        let at = (at - SKIN / rest.dot(n).abs().max(1e-6)).max(0.0);
        moved_by += rest * at;
        if h.tri.is_none() {
            let t = &boxes[h.target];
            let k = (0..3)
                .max_by(|x, y| axis_of(t, *x).dot(n).abs().total_cmp(&axis_of(t, *y).dot(n).abs()))
                .unwrap_or(0);
            h.axis = k;
            h.side = axis_of(t, k).dot(n).signum();
            h.sel_side = -h.side;
        }
        hit = Some(h);
        let Constraint::Plane(pn) = c else { break };
        // what is left of the move, without its part into the face
        let w = n - pn * pn.dot(n);
        if w.length_squared() < 1e-6 {
            break;
        }
        let left = rest * (1.0 - at);
        rest = left - w * (left.dot(w) / w.length_squared()).min(0.0);
        if rest.length_squared() < 1e-12 {
            break;
        }
    }
    (moved_by, hit)
}

/// The box of what the selection shows, turned like `like` (its edit box):
/// the visible triangles of the `moving` prims, so an invisible root prim
/// or an empty part of a mesh's box does not hold it off a floor. A prim not
/// drawn yet counts as its box; None when nothing is drawn.
pub fn shown_bounds(world: &World, scene: &Scene, moving: &HashSet<usize>, like: &Bounds, now: Instant) -> Option<Bounds> {
    let inv = like.rotation.inverse();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut add = |v: Vec3| {
        let q = inv * v;
        lo = lo.min(q);
        hi = hi.max(q);
    };
    for &idx in moving {
        match scene.visible_triangles(world, idx, now) {
            Some(tris) => tris.iter().flatten().for_each(|v| add(*v)),
            None => {
                let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                    continue;
                };
                let h = world.visual_scale_at(idx, now) * 0.5;
                for c in 0..8 {
                    let s = Vec3::new(
                        if c & 1 != 0 { 1.0 } else { -1.0 },
                        if c & 2 != 0 { 1.0 } else { -1.0 },
                        if c & 4 != 0 { 1.0 } else { -1.0 },
                    );
                    add(p + r * (h * s));
                }
            }
        }
    }
    (lo.x <= hi.x && (hi - lo).is_finite()).then(|| Bounds {
        center: like.rotation * ((lo + hi) * 0.5),
        rotation: like.rotation,
        half: ((hi - lo) * 0.5).max(Vec3::splat(0.0005)),
    })
}

/// The prims a drag can snap to: shown in the world (not worn, not an
/// avatar, not a reflection probe's invisible volume), not moved by the
/// drag (`moving`), nearest first, within `RADIUS` of `around`. With the
/// scene, a prim that is not a plain box is its visible surface, a prim
/// with nothing visible (an invisible root, a clicker) is left out, and a
/// prim not drawn yet waits; without it (tests), every prim is its box.
/// The prims around `around` to snap to; `previous`, the targets collected
/// before in the same drag, gives back those that have not moved.
pub fn collect(
    world: &World,
    scene: Option<&Scene>,
    moving: &HashSet<usize>,
    around: &Bounds,
    now: Instant,
    previous: Option<Targets>,
) -> Targets {
    let (mut kept, seen) = previous.map_or_else(Default::default, |p| (p.surfaces.into_iter().map(Some).collect::<Vec<_>>(), p.seen));
    let reach = around.half.length() + RADIUS;
    let mut near: Vec<(f32, usize, Bounds)> = Vec::new();
    for (idx, o) in world.objects.iter() {
        if o.is_avatar() || o.extra.reflection_probe.is_some() || moving.contains(&idx) {
            continue;
        }
        if super::select_type(world, idx) != super::SelectType::World {
            continue;
        }
        let Some((p, r, hud)) = Scene::object_transform(world, idx, now, 0) else {
            continue;
        };
        if hud {
            continue;
        }
        let half = world.visual_scale_at(idx, now) * 0.5;
        let gap = p.distance(around.center) - half.length();
        if gap > reach || !half.is_finite() || half.min_element() <= 0.0 {
            continue;
        }
        near.push((
            gap,
            idx,
            Bounds {
                center: p,
                rotation: r,
                half,
            },
        ));
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(MAX_TARGETS);
    let mut targets = Targets {
        around: around.center,
        ..Default::default()
    };
    let mut triangles = 0;
    for (_, idx, b) in near {
        let Some(scene) = scene else {
            targets.boxes.push(b);
            continue;
        };
        let before = seen.get(&idx).filter(|(at, _)| same_box(at, &b)).map(|(_, s)| *s);
        let surface = match before {
            Some(Seen::Hidden) => None,
            Some(Seen::Box) => {
                targets.boxes.push(b);
                targets.seen.insert(idx, (b, Seen::Box));
                continue;
            }
            Some(Seen::Surface(i)) => kept.get_mut(i).and_then(Option::take),
            // a box prim with something drawn counts as its box
            None => match scene.visible_triangles(world, idx, now).filter(|t| !t.is_empty()) {
                Some(_) if world.objects.get(idx).is_some_and(|o| is_box(&o.volume)) => {
                    targets.boxes.push(b);
                    targets.seen.insert(idx, (b, Seen::Box));
                    continue;
                }
                tris => tris.map(Surface::new),
            },
        };
        let Some(surface) = surface else {
            targets.seen.insert(idx, (b, Seen::Hidden));
            continue;
        };
        triangles += surface.tris.len();
        if triangles > MAX_TRIANGLES {
            break;
        }
        targets.seen.insert(idx, (b, Seen::Surface(targets.surfaces.len())));
        targets.surfaces.push(surface);
    }
    targets
}

/// What the guides show for one hit: the target's face (or centre) and
/// the selection's matching face, or the corner that meets it.
#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    pub kind: Kind,
    pub contact: bool,
    /// Outline of the target's face; empty for a centre.
    pub target_face: Vec<Vec3>,
    /// Outline of the selection's face when it is parallel, else its point
    /// on the target's plane.
    pub sel_face: Vec<Vec3>,
    /// From the selection's mark to the target's (centres for a centre).
    pub link: (Vec3, Vec3),
}

fn face(b: &Bounds, k: usize, side: f32) -> [Vec3; 4] {
    let (i, j) = ((k + 1) % 3, (k + 2) % 3);
    let c = b.center + axis_of(b, k) * b.half[k] * side;
    let (a, e) = (axis_of(b, i) * b.half[i], axis_of(b, j) * b.half[j]);
    [c - a - e, c + a - e, c + a + e, c - a + e]
}

/// The selection's face (or corner) that faces `toward`.
fn sel_mark(sel: &Bounds, toward: Vec3) -> Vec<Vec3> {
    match (0..3).find(|i| axis_of(sel, *i).dot(toward).abs() > 0.999) {
        Some(i) => face(sel, i, axis_of(sel, i).dot(toward).signum()).to_vec(),
        None => {
            // the corner of the selection furthest along `toward`
            let corner = (0..3).fold(sel.center, |p, i| {
                let a = axis_of(sel, i);
                p + a * sel.half[i] * a.dot(toward).signum()
            });
            vec![corner]
        }
    }
}

/// A contact with a surface: the selection's face (or corner) and, around
/// it on the surface's plane, a pad a little larger than its footprint. The
/// triangle itself is not drawn: meshes cut their floors into long thin
/// triangles that have nothing to do with where the selection stands.
fn surface_guide(sel: &Bounds, tri: &[Vec3; 3]) -> Guide {
    let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
    let mid = (tri[0] + tri[1] + tri[2]) / 3.0;
    let up = if (sel.center - mid).dot(n) >= 0.0 { n } else { -n };
    let sel_face = sel_mark(sel, -up);
    let mark = sel_face.iter().copied().sum::<Vec3>() / sel_face.len() as f32;
    let flat = |v: Vec3| v - up * (v - mid).dot(up);
    let at = flat(mark);
    let pad = if sel_face.len() > 1 {
        sel_face.iter().map(|v| at + (flat(*v) - at) * PAD_SCALE).collect()
    } else {
        // a corner on a slope: a small square around where it rests
        let e1 = up.any_orthonormal_vector();
        let (e1, e2) = (e1 * sel.half.min_element() * 0.6, up.cross(e1) * sel.half.min_element() * 0.6);
        vec![at - e1 - e2, at + e1 - e2, at + e1 + e2, at - e1 + e2]
    };
    Guide {
        kind: Kind::Touch,
        contact: true,
        target_face: pad,
        sel_face,
        link: (mark, at),
    }
}

/// The guides of a snap, for the selection where the snap put it.
pub fn guides(sel: &Bounds, targets: &[Bounds], snap: &Snap) -> Vec<Guide> {
    snap.hits
        .iter()
        .filter_map(|h| {
            if let Some(tri) = h.tri {
                return Some(surface_guide(sel, &tri));
            }
            let t = targets.get(h.target)?;
            let u = axis_of(t, h.axis);
            if h.kind == Kind::Center {
                return Some(Guide {
                    kind: h.kind,
                    contact: false,
                    target_face: Vec::new(),
                    sel_face: Vec::new(),
                    link: (sel.center, t.center),
                });
            }
            let target_face = face(t, h.axis, h.side).to_vec();
            let sel_face = sel_mark(sel, u * h.sel_side);
            let mark = sel_face.iter().copied().sum::<Vec3>() / sel_face.len() as f32;
            let tc = target_face.iter().copied().sum::<Vec3>() / 4.0;
            // the target's mark: the selection's mark brought onto its face plane
            let on_plane = mark + u * (tc - mark).dot(u);
            Some(Guide {
                kind: h.kind,
                contact: h.contact,
                target_face,
                sel_face,
                link: (mark, if h.contact { on_plane } else { tc }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    /// 100 pixels per metre wherever the move is.
    fn px(_: Vec3, by: Vec3) -> f32 {
        by.length() * 100.0
    }

    fn cube(center: Vec3, half: Vec3, rotation: Quat) -> Bounds {
        Bounds { center, rotation, half }
    }

    #[test]
    fn an_arrow_touches_the_neighbour_within_the_threshold_only() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        for (gap, snaps) in [(0.02, true), (0.079, true), (0.081, false), (0.3, false)] {
            // a neighbour offset in y and z: only its -X face is in reach
            let t = cube(Vec3::new(1.0 + gap, 0.2, 0.3), Vec3::splat(0.5), Quat::IDENTITY);
            let snap = find(&sel, &[t], &[], Constraint::Axis(Vec3::X), &px);
            assert_eq!(snap.is_some(), snaps, "gap {gap}");
            if let Some(snap) = snap {
                assert!(snap.offset.abs_diff_eq(Vec3::new(gap, 0.0, 0.0), 1e-5), "{snap:?}");
                assert_eq!(snap.hits[0].kind, Kind::Touch);
                assert!(snap.hits[0].contact);
            }
        }
    }

    #[test]
    fn an_arrow_snap_never_leaves_its_axis() {
        let targets = [
            cube(Vec3::new(1.03, 0.04, -0.02), Vec3::splat(0.5), Quat::IDENTITY),
            cube(Vec3::new(0.2, 1.06, 0.01), Vec3::new(0.3, 0.5, 0.7), Quat::from_rotation_z(0.4)),
            cube(Vec3::new(-0.1, 0.03, -1.04), Vec3::splat(0.5), Quat::from_rotation_x(0.3)),
        ];
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::from_rotation_z(0.1));
        for a in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 1.0, 0.0).normalize(),
            Vec3::new(0.3, -0.5, 0.8).normalize(),
        ] {
            if let Some(snap) = find(&sel, &targets, &[], Constraint::Axis(a), &px) {
                assert!(snap.offset.cross(a).length() < 1e-6, "{a}: {snap:?}");
                assert_eq!(snap.hits.len(), 1);
                assert!(px(sel.center, snap.offset) <= SNAP_PX + 1e-3);
            }
        }
        // a face too oblique to the arrow is out of its reach
        let side = cube(Vec3::new(1.02, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let flat = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let snap = find(&flat, &[side], &[], Constraint::Axis(Vec3::Y), &px).expect("its Y faces are flush");
        assert_eq!(snap.offset, Vec3::ZERO);
        assert!(snap.hits.iter().all(|h| h.axis != 0));
    }

    #[test]
    fn a_rotated_selection_rests_its_corner_on_a_tilted_face() {
        let plank = cube(Vec3::ZERO, Vec3::new(0.8, 0.25, 0.05), Quat::from_rotation_y(20f32.to_radians()));
        let sel = cube(
            Vec3::new(0.1, 0.0, 0.9),
            Vec3::splat(0.3),
            Quat::from_rotation_z(30f32.to_radians()),
        );
        let n = plank.rotation * Vec3::Z;
        let top = plank.center.dot(n) + plank.half.z;
        let (low, _) = interval(&sel, n);
        // 3 cm above the slope along Z, within the threshold
        let sel = moved(&sel, Vec3::Z * ((top - low) / n.z + 0.03));
        let snap = find(&sel, &[plank], &[], Constraint::Axis(Vec3::Z), &px).expect("in reach");
        let rest = moved(&sel, snap.offset);
        assert!((interval(&rest, n).0 - top).abs() < 1e-5);
        assert!(snap.offset.cross(Vec3::Z).length() < 1e-6);
        assert_eq!(snap.hits[0].kind, Kind::Touch);
        assert!(snap.hits[0].contact);
        let guide = &guides(&rest, &[plank], &snap)[0];
        assert_eq!(guide.target_face.len(), 4);
        // no face of the turned cube is parallel to the slope: its corner
        assert_eq!(guide.sel_face.len(), 1);
        assert!((guide.sel_face[0].dot(n) - top).abs() < 1e-4);
    }

    #[test]
    fn a_rotated_target_is_met_along_its_own_face_normal() {
        let t = cube(
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(0.5, 0.3, 0.4),
            Quat::from_rotation_z(30f32.to_radians()),
        );
        let u = t.rotation * Vec3::X;
        let sel_at = |gap: f32| {
            let mut s = cube(Vec3::ZERO, Vec3::splat(0.4), Quat::IDENTITY);
            s.center = t.center + u * (t.half.x + support(&s, u) + gap);
            s
        };
        let sel = sel_at(0.05);
        let snap = find(&sel, &[t], &[], Constraint::Plane(Vec3::Z), &px).expect("in reach");
        let rest = moved(&sel, snap.offset);
        // no face of the cube is parallel to the turned target: only contacts
        assert!(snap.hits.iter().all(|h| h.kind == Kind::Touch && h.contact), "{snap:?}");
        assert_eq!((snap.hits[0].axis, snap.hits[0].side), (0, 1.0));
        assert!((interval(&rest, u).0 - (t.center.dot(u) + t.half.x)).abs() < 1e-5);
        assert_eq!(rest.center.z, sel.center.z);
    }

    #[test]
    fn a_tilted_face_snaps_only_on_contact() {
        let plank = cube(Vec3::ZERO, Vec3::new(0.8, 0.25, 0.05), Quat::from_rotation_y(20f32.to_radians()));
        let n = plank.rotation * Vec3::Z;
        let top = plank.center.dot(n) + plank.half.z;
        // beside the plank, its corner 3 cm (along Z) from the slope's plane
        let sel = cube(Vec3::new(0.0, 1.5, 0.9), Vec3::splat(0.3), Quat::IDENTITY);
        let sel = moved(&sel, Vec3::Z * ((top - interval(&sel, n).0) / n.z + 0.03));
        assert!(find(&sel, &[plank], &[], Constraint::Axis(Vec3::Z), &px).is_none());
        // over it, the same corner rests on it
        let over = moved(&sel, Vec3::new(0.0, -1.5, 0.0));
        assert!(find(&over, &[plank], &[], Constraint::Axis(Vec3::Z), &px).is_some_and(|s| s.hits[0].contact));
    }

    #[test]
    fn a_plane_drag_puts_an_edge_on_an_edge() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let t = cube(Vec3::new(1.04, 0.03, 0.2), Vec3::splat(0.5), Quat::IDENTITY);
        let snap = find(&sel, &[t], &[], Constraint::Plane(Vec3::Z), &px).expect("in reach");
        assert!(snap.offset.abs_diff_eq(Vec3::new(0.04, 0.03, 0.0), 1e-5), "{snap:?}");
        assert_eq!(snap.hits.len(), 2);
        let kinds: Vec<_> = snap.hits.iter().map(|h| (h.axis, h.kind)).collect();
        assert!(kinds.contains(&(0, Kind::Touch)) && kinds.contains(&(1, Kind::Flush)), "{kinds:?}");
        assert_eq!(guides(&moved(&sel, snap.offset), &[t], &snap).len(), 2);
    }

    #[test]
    fn a_contact_wins_over_a_slightly_nearer_alignment() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        // touching 4 px away, or a far box's -X face plane 3 px away
        let touch = cube(Vec3::new(1.04, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let plane = cube(Vec3::new(1.03, 3.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let snap = find(&sel, &[plane, touch], &[], Constraint::Axis(Vec3::X), &px).unwrap();
        assert_eq!(snap.hits[0].target, 1);
        assert!(snap.hits[0].contact);
        // without the contact, the nearer alignment
        let snap = find(&sel, &[plane], &[], Constraint::Axis(Vec3::X), &px).unwrap();
        assert!(!snap.hits[0].contact);
        assert!(snap.offset.abs_diff_eq(Vec3::new(0.03, 0.0, 0.0), 1e-5));
    }

    #[test]
    fn nothing_snaps_off_screen_or_without_targets() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let t = cube(Vec3::new(1.01, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        assert!(find(&sel, &[t], &[], Constraint::Axis(Vec3::X), &|_, _| f32::INFINITY).is_none());
        assert!(find(&sel, &[t], &[], Constraint::Axis(Vec3::X), &|_, _| f32::NAN).is_none());
        assert!(find(&sel, &[], &[], Constraint::Plane(Vec3::Z), &px).is_none());
    }

    #[test]
    fn a_move_stops_at_the_first_face_even_far_past_it() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let near = cube(Vec3::new(2.0, 0.2, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let far = cube(Vec3::new(4.0, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        for by in [1.5, 3.0, 10.0] {
            let (moved_by, hit) = stop(&sel, Vec3::X * by, &[far, near], &[], Constraint::Axis(Vec3::X));
            assert!(moved_by.abs_diff_eq(Vec3::X, 1e-5), "{by}: {moved_by}");
            let hit = hit.expect("a contact");
            assert_eq!((hit.target, hit.axis, hit.side, hit.contact), (1, 0, -1.0, true));
        }
        // short of it, or away from it: the whole move
        for by in [Vec3::X * 0.9, Vec3::X * -3.0, Vec3::Y * 2.0] {
            assert_eq!(stop(&sel, by, &[near], &[], Constraint::Axis(by.normalize())), (by, None));
        }
    }

    #[test]
    fn a_resting_box_stays_on_the_face_and_slides_along_it() {
        // resting on the -X face of a target
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let t = cube(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let (pushed, _) = stop(&sel, Vec3::X * 0.3, &[t], &[], Constraint::Axis(Vec3::X));
        assert!(pushed.length() < 1e-5, "{pushed}");
        let (slid, hit) = stop(&sel, Vec3::new(0.4, 0.7, 0.0), &[t], &[], Constraint::Plane(Vec3::Z));
        assert!(slid.abs_diff_eq(Vec3::new(0.0, 0.7, 0.0), 1e-5), "{slid}");
        assert!(hit.is_some());
        // along the face, no contact in the way
        let (along, hit) = stop(&sel, Vec3::Y * 0.7, &[t], &[], Constraint::Plane(Vec3::Z));
        assert_eq!((along, hit), (Vec3::Y * 0.7, None));
    }

    #[test]
    fn a_turned_box_is_met_on_its_slanted_side() {
        let t = cube(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::splat(0.5),
            Quat::from_rotation_z(45f32.to_radians()),
        );
        let sel = cube(Vec3::new(0.0, 0.3, 0.0), Vec3::splat(0.4), Quat::IDENTITY);
        let (moved_by, hit) = stop(&sel, Vec3::X * 3.0, &[t], &[], Constraint::Axis(Vec3::X));
        let rest = moved(&sel, moved_by);
        assert!(hit.is_some() && moved_by.x > 0.5 && moved_by.x < 1.5, "{moved_by}");
        // touching: no further step in, any step back
        assert!(stop(&rest, Vec3::X * 0.01, &[t], &[], Constraint::Axis(Vec3::X)).0.length() < 1e-4);
        assert!(entry(&rest, Vec3::X * -0.5, Shape::Box(&t)).is_none());
    }

    #[test]
    fn a_box_already_inside_moves_out_freely() {
        let room = cube(Vec3::ZERO, Vec3::splat(5.0), Quat::IDENTITY);
        let sel = cube(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let by = Vec3::X * 8.0;
        assert_eq!(stop(&sel, by, &[room], &[], Constraint::Axis(Vec3::X)), (by, None));
    }

    /// A horizontal square of side 2 `half` at height `z`, as two triangles.
    fn floor(half: f32, z: f32) -> [[Vec3; 3]; 2] {
        let c = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| Vec3::new(x * half, y * half, z));
        [[c[0], c[1], c[2]], [c[0], c[2], c[3]]]
    }

    /// A closed room: the inside of a 6 m cube standing on z = 0, its box
    /// all around anything put in it.
    fn room() -> Surface {
        let mut tris = Vec::new();
        for (axis, at) in [(2, 0.0), (2, 6.0), (0, -3.0), (0, 3.0), (1, -3.0), (1, 3.0)] {
            for t in floor(3.0, 0.0) {
                tris.push(t.map(|v| {
                    let mut p = match axis {
                        0 => Vec3::new(0.0, v.x, v.y + 3.0),
                        1 => Vec3::new(v.x, 0.0, v.y + 3.0),
                        _ => v,
                    };
                    p[axis] = at;
                    p
                }));
            }
        }
        Surface::new(tris)
    }

    #[test]
    fn plain_boxes_are_their_box_every_other_shape_its_surface() {
        use aurora_prim::SculptParams;
        use aurora_prim::params::{LL_PCODE_PROFILE_CIRCLE, LL_SCULPT_TYPE_MESH};
        let plain = VolumeParams::default();
        assert!(is_box(&plain));
        let mut hollow = plain;
        hollow.profile.hollow = 0.85;
        let mut cut = plain;
        cut.path.end = 0.5;
        let mut cylinder = plain;
        cylinder.profile.curve_type = LL_PCODE_PROFILE_CIRCLE;
        let mut tapered = plain;
        tapered.path.taper[0] = 0.3;
        let mut mesh = plain;
        mesh.sculpt = Some(SculptParams {
            texture: uuid::Uuid::from_u128(1),
            sculpt_type: LL_SCULPT_TYPE_MESH,
        });
        for (name, v) in [
            ("hollow", hollow),
            ("cut", cut),
            ("cylinder", cylinder),
            ("tapered", tapered),
            ("mesh", mesh),
        ] {
            assert!(!is_box(&v), "{name}");
        }
    }

    #[test]
    fn a_floor_inside_a_house_holds_what_is_put_on_it() {
        let house = room();
        let sel = cube(Vec3::new(0.5, 0.0, 2.0), Vec3::splat(0.5), Quat::IDENTITY);
        // down onto the floor however far the cursor goes
        for down in [2.0, 10.0] {
            let (moved_by, hit) = stop(&sel, -Vec3::Z * down, &[], std::slice::from_ref(&house), Constraint::Axis(Vec3::Z));
            assert!((moved_by.z + 1.5).abs() < 1e-4, "{down}: {moved_by}");
            assert!(hit.is_some_and(|h| h.contact && h.tri.is_some()));
        }
        // against an inner wall, then sliding along it in a plane drag
        let (moved_by, _) = stop(&sel, Vec3::X * 5.0, &[], std::slice::from_ref(&house), Constraint::Axis(Vec3::X));
        assert!((moved_by.x - 2.0).abs() < 1e-4, "{moved_by}");
        let (slid, _) = stop(
            &sel,
            Vec3::new(5.0, 1.0, 0.0),
            &[],
            std::slice::from_ref(&house),
            Constraint::Plane(Vec3::Z),
        );
        assert!(slid.abs_diff_eq(Vec3::new(2.0, 1.0, 0.0), 1e-4), "{slid}");
    }

    #[test]
    fn a_surface_snaps_within_the_threshold_with_its_triangle_as_guide() {
        let ground = Surface::new(floor(5.0, 0.0).to_vec());
        for (above, snaps) in [(0.03, true), (0.079, true), (0.1, false)] {
            let sel = cube(Vec3::new(0.2, 0.3, 0.5 + above), Vec3::splat(0.5), Quat::IDENTITY);
            let snap = find(&sel, &[], std::slice::from_ref(&ground), Constraint::Axis(Vec3::Z), &px);
            assert_eq!(snap.is_some(), snaps, "{above} m above");
            if let Some(snap) = snap {
                assert!(snap.offset.abs_diff_eq(-Vec3::Z * above, 1e-4), "{snap:?}");
                let guide = &guides(&moved(&sel, snap.offset), &[], &snap)[0];
                // a pad on the floor around the cube's footprint, not the triangle
                assert_eq!(guide.target_face.len(), 4);
                assert!(guide.target_face.iter().all(|v| v.z.abs() < 1e-4));
                let pad = guide.target_face.iter().fold(0f32, |m, v| m.max((v.x - 0.2).abs()));
                assert!((pad - 0.5 * PAD_SCALE).abs() < 1e-4, "{pad}");
                assert_eq!(guide.sel_face.len(), 4, "the cube's bottom face");
                assert!(guide.sel_face.iter().all(|v| v.z.abs() < 1e-4));
            }
        }
        // a plane drag reaches it along the plane's axes only
        let sel = cube(Vec3::new(0.0, 0.0, 0.53), Vec3::splat(0.5), Quat::IDENTITY);
        assert!(find(&sel, &[], std::slice::from_ref(&ground), Constraint::Plane(Vec3::Z), &px).is_none());
        let snap = find(&sel, &[], std::slice::from_ref(&ground), Constraint::Plane(Vec3::Y), &px).expect("down in XZ");
        assert!(snap.offset.abs_diff_eq(-Vec3::Z * 0.03, 1e-4), "{snap:?}");
    }

    #[test]
    fn a_triangle_already_through_the_selection_does_not_hold_it() {
        // a rug's edge crossing the box when the drag begins
        let rug = Surface::new(floor(1.0, 0.0).to_vec());
        let sel = cube(Vec3::new(0.0, 0.0, 0.2), Vec3::splat(0.5), Quat::IDENTITY);
        let by = Vec3::X * 3.0;
        assert_eq!(
            stop(&sel, by, &[], std::slice::from_ref(&rug), Constraint::Axis(Vec3::X)),
            (by, None)
        );
    }
}
