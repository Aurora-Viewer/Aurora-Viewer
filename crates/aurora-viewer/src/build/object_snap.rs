//! "Snap to objects": while a move arrow or plane handle is dragged, the
//! selection's box snaps flush against the boxes of nearby prims, with
//! guides on what it snapped to, like the smart guides of design tools.
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

use super::Bounds;
use crate::scene::Scene;
use crate::world::World;
use glam::Vec3;
use std::collections::HashSet;
use std::time::Instant;

/// A snap applies when it moves the selection by this many pixels or fewer.
pub const SNAP_PX: f32 = 8.0;
/// Prims whose box is this close to the selection's box are targets (m).
pub const RADIUS: f32 = 10.0;
/// The nearest targets kept.
pub const MAX_TARGETS: usize = 256;
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    /// Added to the drag's move.
    pub offset: Vec3,
    pub hits: Vec<Hit>,
}

/// Targets collected for one drag, with where they were collected.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    pub boxes: Vec<Bounds>,
    pub around: Vec3,
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
fn support(b: &Bounds, u: Vec3) -> f32 {
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
                if !(pixels <= SNAP_PX) {
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

/// The snap of the selection `sel` (where the drag puts it) against
/// `targets`, if one is within `SNAP_PX`.
pub fn find(sel: &Bounds, targets: &[Bounds], c: Constraint, px: &dyn Fn(Vec3, Vec3) -> f32) -> Option<Snap> {
    let found = candidates(sel, targets, c, px);
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

/// The prims a drag can snap to: shown in the world (not worn, not an
/// avatar, not a reflection probe's invisible volume), not moved by the
/// drag (`moving`), nearest first, within `RADIUS` of `around`.
pub fn collect(world: &World, moving: &HashSet<usize>, around: &Bounds, now: Instant) -> Targets {
    let reach = around.half.length() + RADIUS;
    let mut near: Vec<(f32, Bounds)> = Vec::new();
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
            Bounds {
                center: p,
                rotation: r,
                half,
            },
        ));
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(MAX_TARGETS);
    Targets {
        boxes: near.into_iter().map(|(_, b)| b).collect(),
        around: around.center,
    }
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

/// The guides of a snap, for the selection where the snap put it.
pub fn guides(sel: &Bounds, targets: &[Bounds], snap: &Snap) -> Vec<Guide> {
    snap.hits
        .iter()
        .filter_map(|h| {
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
            let toward = u * h.sel_side;
            let parallel = (0..3).find(|i| axis_of(sel, *i).dot(toward).abs() > 0.999);
            let sel_face = match parallel {
                Some(i) => face(sel, i, axis_of(sel, i).dot(toward).signum()).to_vec(),
                None => {
                    // the corner of the selection furthest along `toward`
                    let corner = (0..3).fold(sel.center, |p, i| {
                        let a = axis_of(sel, i);
                        p + a * sel.half[i] * a.dot(toward).signum()
                    });
                    vec![corner]
                }
            };
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
            let snap = find(&sel, &[t], Constraint::Axis(Vec3::X), &px);
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
            if let Some(snap) = find(&sel, &targets, Constraint::Axis(a), &px) {
                assert!(snap.offset.cross(a).length() < 1e-6, "{a}: {snap:?}");
                assert_eq!(snap.hits.len(), 1);
                assert!(px(sel.center, snap.offset) <= SNAP_PX + 1e-3);
            }
        }
        // a face too oblique to the arrow is out of its reach
        let side = cube(Vec3::new(1.02, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        let flat = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let snap = find(&flat, &[side], Constraint::Axis(Vec3::Y), &px).expect("its Y faces are flush");
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
        let snap = find(&sel, &[plank], Constraint::Axis(Vec3::Z), &px).expect("in reach");
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
        let snap = find(&sel, &[t], Constraint::Plane(Vec3::Z), &px).expect("in reach");
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
        assert!(find(&sel, &[plank], Constraint::Axis(Vec3::Z), &px).is_none());
        // over it, the same corner rests on it
        let over = moved(&sel, Vec3::new(0.0, -1.5, 0.0));
        assert!(find(&over, &[plank], Constraint::Axis(Vec3::Z), &px).is_some_and(|s| s.hits[0].contact));
    }

    #[test]
    fn a_plane_drag_puts_an_edge_on_an_edge() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let t = cube(Vec3::new(1.04, 0.03, 0.2), Vec3::splat(0.5), Quat::IDENTITY);
        let snap = find(&sel, &[t], Constraint::Plane(Vec3::Z), &px).expect("in reach");
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
        let snap = find(&sel, &[plane, touch], Constraint::Axis(Vec3::X), &px).unwrap();
        assert_eq!(snap.hits[0].target, 1);
        assert!(snap.hits[0].contact);
        // without the contact, the nearer alignment
        let snap = find(&sel, &[plane], Constraint::Axis(Vec3::X), &px).unwrap();
        assert!(!snap.hits[0].contact);
        assert!(snap.offset.abs_diff_eq(Vec3::new(0.03, 0.0, 0.0), 1e-5));
    }

    #[test]
    fn nothing_snaps_off_screen_or_without_targets() {
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), Quat::IDENTITY);
        let t = cube(Vec3::new(1.01, 0.0, 0.0), Vec3::splat(0.5), Quat::IDENTITY);
        assert!(find(&sel, &[t], Constraint::Axis(Vec3::X), &|_, _| f32::INFINITY).is_none());
        assert!(find(&sel, &[t], Constraint::Axis(Vec3::X), &|_, _| f32::NAN).is_none());
        assert!(find(&sel, &[], Constraint::Plane(Vec3::Z), &px).is_none());
    }
}
