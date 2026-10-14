//! Windows snap to each other and to the work area while moved with Alt or
//! resized, and stay together once attached side by side.
//!
//! Firestorm snaps a dragged floater to the edges of its siblings and of the
//! floater view (LLFloater's drag handle, LLView::findSnapRect and
//! findSnapEdge in indra/llui/llview.cpp; settings SnapEnabled, on, and
//! SnapMargin, 10 px), and moves the dependents of a floater with it
//! (LLFloater::addDependentFloater, LLFloater::translate). The rules kept:
//! the margin, an edge meets the facing edge of a neighbor or lines up with
//! its same-side edge when the two are already neighbors on the other axis,
//! and the edges of the parent count too. What Aurora does its own way: a
//! window only attracts another when they would share a stretch of edge (a
//! floater far above attracts in Firestorm), the snap of a moved window is
//! shown by a guide and applied when the mouse is released rather than under
//! the pointer, and any two windows snapped side by side move as one block
//! (Firestorm keeps that for declared dependents). At the user's request,
//! moving and dropping without Alt never snaps or creates a group; Alt
//! detaches the dragged window and enables attachment to a new target.
//!
//! This module is the geometry: which snap a set of moving edges is offered
//! (`find`), what to draw for it (`guide`), which windows are side by side
//! (`touching`) and move together (`group`). `layout` feeds it the windows
//! of the frame and applies the result.

pub(crate) mod overlay;

use super::layout::GAP;
use egui::{Pos2, Rect, Vec2};
use std::collections::BTreeSet;

/// How near an edge must come to be offered a snap (SnapMargin).
pub(crate) const MARGIN: f32 = 10.0;

/// How much further the snap already shown holds, and how much nearer
/// another must be to replace it: the guide does not flicker at the limit
/// of the margin nor between two neighbors at the same distance.
const KEEP: f32 = 4.0;

/// Two windows are side by side when their facing edges stand at the gap,
/// give or take this (positions are rounded to pixels), and share more than
/// this of their length.
const SLACK: f32 = 1.0;

/// The edges of a window that follow the pointer: all four when it is
/// moved, one or two when it is resized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    pub(crate) const ALL: Edges = Edges {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };

    /// (the near edge moves, the far edge moves) along `axis`.
    fn along(self, axis: Axis) -> (bool, bool) {
        match axis {
            Axis::X => (self.left, self.right),
            Axis::Y => (self.top, self.bottom),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// A rect seen along one axis: where it starts and ends on it (left and
/// right for X), and across it (top and bottom for X).
#[derive(Clone, Copy)]
struct Span {
    near: f32,
    far: f32,
    cross_near: f32,
    cross_far: f32,
}

fn span(rect: Rect, axis: Axis) -> Span {
    match axis {
        Axis::X => Span {
            near: rect.left(),
            far: rect.right(),
            cross_near: rect.top(),
            cross_far: rect.bottom(),
        },
        Axis::Y => Span {
            near: rect.top(),
            far: rect.bottom(),
            cross_near: rect.left(),
            cross_far: rect.right(),
        },
    }
}

fn bounds(rects: &[Rect]) -> Rect {
    rects.iter().fold(Rect::NOTHING, |all, r| all.union(*r))
}

/// What an edge snaps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The facing edge of another window, at the gap: the two end up side
    /// by side.
    Adjacent,
    /// The edge of the work area, at the gap.
    Area,
    /// The same-side edge of a window it is next to on the other axis: the
    /// two line up.
    Aligned,
}

/// The snap of one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AxisSnap {
    /// How far the moving edges go from where the pointer has them.
    pub shift: f32,
    pub kind: Kind,
    /// Which moving rect meets something (the first for the work area,
    /// which the bounds of all of them meet).
    pub mover: usize,
    /// Which still rect it meets; none for the work area.
    pub other: Option<usize>,
    /// Its far edge (right, bottom) rather than its near one (left, top).
    pub far: bool,
}

impl AxisSnap {
    fn same(&self, other: &AxisSnap) -> bool {
        (self.kind, self.mover, self.other, self.far) == (other.kind, other.mover, other.other, other.far)
    }
}

/// The snap offered on each axis; they are found independently.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Snap {
    pub x: Option<AxisSnap>,
    pub y: Option<AxisSnap>,
}

impl Snap {
    pub(crate) fn is_none(&self) -> bool {
        self.x.is_none() && self.y.is_none()
    }

    pub(crate) fn shift(&self) -> Vec2 {
        Vec2::new(self.x.map_or(0.0, |s| s.shift), self.y.map_or(0.0, |s| s.shift))
    }

    /// (moving rect, still rect) of each snap that puts two windows side by
    /// side.
    pub(crate) fn adjacent(&self) -> impl Iterator<Item = (usize, usize)> {
        [self.x, self.y]
            .into_iter()
            .flatten()
            .filter(|s| s.kind == Kind::Adjacent)
            .filter_map(|s| Some((s.mover, s.other?)))
    }
}

/// What moves and what it can snap to.
pub(crate) struct Scene<'a> {
    /// The rects that move together, where the pointer has them: one
    /// window, or the windows of a group. A resize has one.
    pub movers: &'a [Rect],
    pub edges: Edges,
    /// The other open windows.
    pub others: &'a [Rect],
    /// The work area: the screen without the bars.
    pub area: Rect,
    /// Limits of the size of a resized window: a snap that would pass them
    /// is not offered.
    pub min_size: Vec2,
    pub max_size: Vec2,
}

/// The snap offered to the moving edges of `scene`: on each axis, the
/// nearest within the margin among the facing edges of the other windows,
/// the edges of the work area and the same-side edges of the neighbors of
/// the other axis. `previous` is the snap of the last frame of the same
/// gesture: it holds a little past the margin.
pub(crate) fn find(scene: &Scene, previous: Snap) -> Snap {
    Snap {
        x: find_along(scene, Axis::X, previous.x),
        y: find_along(scene, Axis::Y, previous.y),
    }
}

fn find_along(scene: &Scene, axis: Axis, previous: Option<AxisSnap>) -> Option<AxisSnap> {
    let (near_moves, far_moves) = scene.edges.along(axis);
    if !near_moves && !far_moves {
        return None;
    }
    let resized = !(near_moves && far_moves);
    let (min, max) = match axis {
        Axis::X => (scene.min_size.x, scene.max_size.x),
        Axis::Y => (scene.min_size.y, scene.max_size.y),
    };
    let mut found: Vec<AxisSnap> = Vec::new();
    let mut offer = |shift: f32, kind: Kind, mover: usize, other: Option<usize>, far: bool| {
        if !shift.is_finite() || shift.abs() > MARGIN + KEEP {
            return;
        }
        if resized {
            let now = span(scene.movers[mover], axis);
            let length = now.far - now.near + if far { shift } else { -shift };
            if length < min - 0.01 || length > max + 0.01 {
                return;
            }
        }
        found.push(AxisSnap {
            shift,
            kind,
            mover,
            other,
            far,
        });
    };
    for (m, mover) in scene.movers.iter().enumerate() {
        let a = span(*mover, axis);
        for (o, other) in scene.others.iter().enumerate() {
            let b = span(*other, axis);
            // side by side they would share a stretch of edge, or nearly
            let facing = a.cross_near < b.cross_far + MARGIN && b.cross_near < a.cross_far + MARGIN;
            if facing {
                if near_moves {
                    offer(b.far + GAP - a.near, Kind::Adjacent, m, Some(o), false);
                }
                if far_moves {
                    offer(b.near - GAP - a.far, Kind::Adjacent, m, Some(o), true);
                }
            }
            // neighbors on the other axis, or about to be: their same-side
            // edges line up ("if snapped with sibling along other axis,
            // check for shared edge", LLView::findSnapEdge)
            let stacked = (a.cross_near - (b.cross_far + GAP)).abs() <= MARGIN || (a.cross_far - (b.cross_near - GAP)).abs() <= MARGIN;
            if stacked {
                if near_moves {
                    offer(b.near - a.near, Kind::Aligned, m, Some(o), false);
                }
                if far_moves {
                    offer(b.far - a.far, Kind::Aligned, m, Some(o), true);
                }
            }
        }
    }
    // the work area holds the block as a whole
    let all = span(bounds(scene.movers), axis);
    let area = span(scene.area, axis);
    if near_moves {
        offer(area.near + GAP - all.near, Kind::Area, 0, None, false);
    }
    if far_moves {
        offer(area.far - GAP - all.far, Kind::Area, 0, None, true);
    }

    // the nearest wins; at the same distance, side by side before the work
    // area before lined up (the order of `Kind`)
    let rank = |s: &AxisSnap| (s.shift.abs(), s.kind as u8);
    let best = found
        .iter()
        .filter(|s| s.shift.abs() <= MARGIN)
        .min_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal))
        .copied();
    let kept = previous.and_then(|p| found.iter().find(|s| s.same(&p)).copied());
    match (kept, best) {
        (Some(kept), Some(best)) if best.shift.abs() + KEEP < kept.shift.abs() => Some(best),
        (Some(kept), _) => Some(kept),
        (None, best) => best,
    }
}

/// `rect` with its moving edges shifted.
pub(crate) fn apply(rect: Rect, edges: Edges, shift: Vec2) -> Rect {
    let mut out = rect;
    if edges.left {
        out.min.x += shift.x;
    }
    if edges.right {
        out.max.x += shift.x;
    }
    if edges.top {
        out.min.y += shift.y;
    }
    if edges.bottom {
        out.max.y += shift.y;
    }
    out
}

/// What shows a snap before it is applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Guide {
    /// Along the edges that will meet, over the length they share; one line
    /// through both windows for edges that line up.
    pub lines: Vec<[Pos2; 2]>,
    /// Where the moving windows land.
    pub landing: Vec<Rect>,
}

pub(crate) fn guide(scene: &Scene, snap: Snap) -> Guide {
    let landing: Vec<Rect> = scene.movers.iter().map(|r| apply(*r, scene.edges, snap.shift())).collect();
    let mut lines = Vec::new();
    let line = |axis: Axis, at: f32, from: f32, to: f32| match axis {
        Axis::X => [Pos2::new(at, from), Pos2::new(at, to)],
        Axis::Y => [Pos2::new(from, at), Pos2::new(to, at)],
    };
    for (axis, snap) in [(Axis::X, snap.x), (Axis::Y, snap.y)] {
        let Some(snap) = snap else { continue };
        let Some(landed) = landing.get(snap.mover) else { continue };
        let m = span(*landed, axis);
        let edge = if snap.far { m.far } else { m.near };
        match (snap.kind, snap.other.and_then(|o| scene.others.get(o))) {
            (Kind::Adjacent, Some(other)) => {
                let o = span(*other, axis);
                let (mut from, mut to) = (m.cross_near.max(o.cross_near), m.cross_far.min(o.cross_far));
                if to <= from {
                    // corner to corner: the whole edge of the moving window
                    (from, to) = (m.cross_near, m.cross_far);
                }
                lines.push(line(axis, edge, from, to));
                lines.push(line(axis, if snap.far { o.near } else { o.far }, from, to));
            }
            (Kind::Aligned, Some(other)) => {
                let o = span(*other, axis);
                lines.push(line(axis, edge, m.cross_near.min(o.cross_near), m.cross_far.max(o.cross_far)));
            }
            _ => {
                let all = span(bounds(&landing), axis);
                lines.push(line(axis, if snap.far { all.far } else { all.near }, all.cross_near, all.cross_far));
            }
        }
    }
    Guide { lines, landing }
}

/// The two windows stand side by side: facing edges at the gap, sharing a
/// stretch of their length. What a snap of kind `Adjacent` produces, and
/// what glued windows must keep.
pub(crate) fn touching(a: Rect, b: Rect) -> bool {
    [Axis::X, Axis::Y].into_iter().any(|axis| {
        let (a, b) = (span(a, axis), span(b, axis));
        let at_gap = (a.near - b.far - GAP).abs() <= SLACK || (b.near - a.far - GAP).abs() <= SLACK;
        let shared = a.cross_far.min(b.cross_far) - a.cross_near.max(b.cross_near);
        at_gap && shared > SLACK
    })
}

/// Two windows glued together, by `Id::value`, smaller first.
pub(crate) type Link = (u64, u64);

pub(crate) fn link(a: u64, b: u64) -> Link {
    (a.min(b), a.max(b))
}

/// `window` first, then every window glued to it, directly or through
/// others: what moves with it.
pub(crate) fn group(links: &BTreeSet<Link>, window: u64) -> Vec<u64> {
    let mut group = vec![window];
    let mut next = 0;
    while let Some(&current) = group.get(next) {
        next += 1;
        for &(a, b) in links {
            let neighbor = if a == current {
                b
            } else if b == current {
                a
            } else {
                continue;
            };
            if !group.contains(&neighbor) {
                group.push(neighbor);
            }
        }
    }
    group
}

/// Where `window` can go for the whole block it belongs to (`block`, the
/// bounds of its group) to stay inside `screen`.
pub(crate) fn block_limits(screen: Rect, window: Rect, block: Rect) -> Rect {
    Rect::from_min_max(screen.min + (window.min - block.min), screen.max - (block.max - window.max))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::from_min_max(Pos2::new(0.0, 24.0), Pos2::new(1600.0, 870.0));

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    /// A still 400 x 300 window in the middle of the area.
    fn still() -> Rect {
        rect(600.0, 300.0, 400.0, 300.0)
    }

    fn moved(movers: &[Rect], others: &[Rect], previous: Snap) -> Snap {
        let scene = Scene {
            movers,
            edges: Edges::ALL,
            others,
            area: AREA,
            min_size: Vec2::ZERO,
            max_size: Vec2::INFINITY,
        };
        find(&scene, previous)
    }

    fn move_one(mover: Rect, others: &[Rect]) -> Snap {
        moved(&[mover], others, Snap::default())
    }

    #[test]
    fn a_moved_window_snaps_next_to_each_side_of_another() {
        let o = still();
        // (the moved window, the shift that puts it side by side, on x)
        let cases = [
            (rect(o.right() + GAP + 6.0, 350.0, 200.0, 100.0), Vec2::new(-6.0, 0.0), true),
            (rect(o.left() - GAP - 200.0 - 7.0, 350.0, 200.0, 100.0), Vec2::new(7.0, 0.0), true),
            (rect(700.0, o.bottom() + GAP - 4.0, 200.0, 100.0), Vec2::new(0.0, 4.0), false),
            (rect(700.0, o.top() - GAP - 100.0 + 9.0, 200.0, 100.0), Vec2::new(0.0, -9.0), false),
        ];
        for (mover, shift, on_x) in cases {
            let snap = move_one(mover, &[o]);
            assert_eq!(snap.shift(), shift, "{mover:?}");
            let side = if on_x { snap.x } else { snap.y }.expect("a snap on the axis of the gap");
            assert_eq!((side.kind, side.other), (Kind::Adjacent, Some(0)));
            assert!((if on_x { snap.y } else { snap.x }).is_none(), "{mover:?}");
            let landed = apply(mover, Edges::ALL, snap.shift());
            assert!(touching(landed, o), "{landed:?}");
            assert_eq!(snap.adjacent().collect::<Vec<_>>(), [(0, 0)]);
        }
    }

    #[test]
    fn a_window_far_along_the_edge_does_not_attract() {
        let o = still();
        // at the snapping distance of its right edge, but far above or below
        for y in [o.top() - 100.0 - MARGIN - 1.0, o.bottom() + MARGIN + 1.0] {
            let snap = move_one(rect(o.right() + GAP + 3.0, y, 200.0, 100.0), &[o]);
            assert!(snap.x.is_none(), "{y}: {snap:?}");
        }
        // nearly sharing an edge counts
        let snap = move_one(rect(o.right() + GAP + 3.0, o.bottom() + MARGIN - 1.0, 200.0, 100.0), &[o]);
        assert_eq!(snap.x.map(|s| s.kind), Some(Kind::Adjacent));
    }

    #[test]
    fn the_margin_is_the_limit() {
        let o = still();
        let at = |distance: f32| move_one(rect(o.right() + GAP + distance, 350.0, 200.0, 100.0), &[o]);
        assert_eq!(at(MARGIN).shift(), Vec2::new(-MARGIN, 0.0));
        assert_eq!(at(-MARGIN).shift(), Vec2::new(MARGIN, 0.0));
        assert!(at(MARGIN + 0.5).is_none());
        assert!(at(-MARGIN - 0.5).is_none());
        // already there: still a snap, the two are side by side
        assert_eq!(at(0.0).x.map(|s| (s.shift, s.kind)), Some((0.0, Kind::Adjacent)));
    }

    /// Side by side on one axis, the same-side edges of the other line up.
    #[test]
    fn neighbors_line_up() {
        let o = still();
        // on the right of it, top edges 5 px apart
        let mover = rect(o.right() + GAP + 2.0, o.top() + 5.0, 200.0, 100.0);
        let snap = move_one(mover, &[o]);
        assert_eq!(snap.shift(), Vec2::new(-2.0, -5.0));
        assert_eq!(snap.y.map(|s| (s.kind, s.far)), Some((Kind::Aligned, false)));
        // bottom edges
        let mover = rect(o.right() + GAP + 2.0, o.bottom() - 100.0 - 6.0, 200.0, 100.0);
        let snap = move_one(mover, &[o]);
        assert_eq!(snap.shift(), Vec2::new(-2.0, 6.0));
        assert_eq!(snap.y.map(|s| (s.kind, s.far)), Some((Kind::Aligned, true)));
        // under it, left then right edges
        let mover = rect(o.left() - 4.0, o.bottom() + GAP + 1.0, 200.0, 100.0);
        assert_eq!(move_one(mover, &[o]).shift(), Vec2::new(4.0, -1.0));
        let mover = rect(o.right() - 200.0 + 3.0, o.bottom() + GAP + 1.0, 200.0, 100.0);
        assert_eq!(move_one(mover, &[o]).shift(), Vec2::new(-3.0, -1.0));
        // edges of a window that is not a neighbor do not line up: only
        // lined up, the two would be over each other
        let over = rect(o.left() + 5.0, o.top() + 100.0, 200.0, 100.0);
        assert!(move_one(over, &[o]).is_none());
        // lining up does not glue
        let mover = rect(o.left() - 4.0, o.bottom() + GAP + 30.0, 200.0, 100.0);
        assert!(move_one(mover, &[o]).is_none(), "30 px under: not a neighbor yet");
    }

    #[test]
    fn the_work_area_holds_windows_at_the_gap() {
        let snap = move_one(rect(GAP - 5.0, AREA.top() + GAP + 7.0, 200.0, 100.0), &[]);
        assert_eq!(snap.shift(), Vec2::new(5.0, -7.0));
        assert_eq!(snap.x.map(|s| (s.kind, s.other)), Some((Kind::Area, None)));
        // the bottom bar and the right edge
        let snap = move_one(
            rect(AREA.right() - GAP - 200.0 + 6.0, AREA.bottom() - GAP - 100.0 - 9.0, 200.0, 100.0),
            &[],
        );
        assert_eq!(snap.shift(), Vec2::new(-6.0, 9.0));
        // a window over the bar comes back above it
        let snap = move_one(rect(700.0, AREA.bottom() - GAP - 100.0 + 9.0, 200.0, 100.0), &[]);
        assert_eq!(snap.shift(), Vec2::new(0.0, -9.0));
        // nothing glues a window to the work area
        assert_eq!(snap.adjacent().count(), 0);
        assert!(move_one(rect(700.0, 400.0, 200.0, 100.0), &[]).is_none());
    }

    #[test]
    fn the_nearest_of_two_snaps_wins() {
        let o = still();
        // 3 px from the right of `o`, 6 px from the left of another
        let other = rect(o.right() + GAP + 3.0 + 200.0 + GAP + 6.0, 300.0, 300.0, 300.0);
        let mover = rect(o.right() + GAP + 3.0, 350.0, 200.0, 100.0);
        let snap = move_one(mover, &[o, other]);
        assert_eq!(snap.x.map(|s| (s.shift, s.other)), Some((-3.0, Some(0))));
        let snap = move_one(mover.translate(Vec2::new(2.0, 0.0)), &[o, other]);
        assert_eq!(snap.x.map(|s| (s.shift, s.other)), Some((4.0, Some(1))));
        // at the same distance, side by side wins over the work area
        let edge = rect(AREA.right() - GAP - 150.0, 100.0, 150.0, 400.0);
        let mover = rect(edge.left() - GAP - 200.0 - 5.0, 200.0, 200.0, 100.0);
        assert_eq!(move_one(mover, &[edge]).x.map(|s| s.kind), Some(Kind::Adjacent));
    }

    /// The snap shown holds a little past the margin, and a neighbor must
    /// be clearly nearer to replace it.
    #[test]
    fn the_shown_snap_does_not_flicker() {
        let o = still();
        let at = |distance: f32, previous: Snap| moved(&[rect(o.right() + GAP + distance, 350.0, 200.0, 100.0)], &[o], previous);
        let shown = at(MARGIN - 1.0, Snap::default());
        assert!(shown.x.is_some());
        // past the margin: kept if it was shown, not offered otherwise
        assert_eq!(at(MARGIN + 3.0, shown).x.map(|s| s.shift), Some(-MARGIN - 3.0));
        assert!(at(MARGIN + 3.0, Snap::default()).is_none());
        assert!(at(MARGIN + KEEP + 1.0, shown).is_none());
        // two neighbors at about the same distance
        let other = rect(o.right() + GAP + 200.0 + GAP + 10.0, 300.0, 300.0, 300.0);
        let between = |x: f32, previous: Snap| moved(&[rect(o.right() + GAP + x, 350.0, 200.0, 100.0)], &[o, other], previous);
        let left = between(4.0, Snap::default());
        assert_eq!(left.x.and_then(|s| s.other), Some(0));
        // one pixel nearer the other: the first stays
        assert_eq!(between(5.5, left).x.and_then(|s| s.other), Some(0));
        assert_eq!(between(5.5, Snap::default()).x.and_then(|s| s.other), Some(1));
        // clearly nearer: the other takes over
        assert_eq!(between(8.0, left).x.and_then(|s| s.other), Some(1));
    }

    fn resized(window: Rect, edges: Edges, others: &[Rect], min: Vec2) -> Snap {
        let scene = Scene {
            movers: &[window],
            edges,
            others,
            area: AREA,
            min_size: min,
            max_size: Vec2::new(900.0, 900.0),
        };
        find(&scene, Snap::default())
    }

    const NONE: Edges = Edges {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };

    #[test]
    fn a_resized_edge_snaps_alone() {
        let o = still();
        let window = rect(200.0, 350.0, o.left() - GAP - 200.0 - 6.0, 200.0);
        // its right edge comes next to the left edge of `o`
        let right = Edges { right: true, ..NONE };
        let snap = resized(window, right, &[o], Vec2::splat(100.0));
        assert_eq!(snap.shift(), Vec2::new(6.0, 0.0));
        let after = apply(window, right, snap.shift());
        assert_eq!((after.left(), after.right()), (200.0, o.left() - GAP));
        assert_eq!((after.top(), after.bottom()), (window.top(), window.bottom()));
        assert!(touching(after, o));
        // the edges that do not move snap to nothing: its top is 7 px
        // under the top of `o`... which only a moving top edge would join
        let window = rect(200.0, o.top() + 7.0, o.left() - GAP - 200.0, 200.0);
        assert!(resized(window, Edges { bottom: true, ..NONE }, &[o], Vec2::splat(100.0)).is_none());
        let snap = resized(window, Edges { top: true, ..NONE }, &[o], Vec2::splat(100.0));
        assert_eq!(snap.y.map(|s| (s.shift, s.kind)), Some((-7.0, Kind::Aligned)));
        // the bottom edge lines up with the bottom of its neighbor
        let window = rect(200.0, 350.0, o.left() - GAP - 200.0, o.bottom() - 350.0 + 8.0);
        let bottom = Edges { bottom: true, ..NONE };
        let snap = resized(window, bottom, &[o], Vec2::splat(100.0));
        assert_eq!(apply(window, bottom, snap.shift()).bottom(), o.bottom());
        // a corner: both of its edges
        let corner = Edges {
            right: true,
            bottom: true,
            ..NONE
        };
        let window = rect(200.0, 350.0, o.left() - GAP - 200.0 - 5.0, o.bottom() - 350.0 - 3.0);
        assert_eq!(resized(window, corner, &[o], Vec2::splat(100.0)).shift(), Vec2::new(5.0, 3.0));
        // and the work area
        let window = rect(200.0, 350.0, 300.0, AREA.bottom() - GAP - 350.0 + 4.0);
        assert_eq!(resized(window, bottom, &[], Vec2::splat(100.0)).shift(), Vec2::new(0.0, -4.0));
    }

    /// A snap that would make the window smaller than its minimum, or
    /// larger than its maximum, is not offered.
    #[test]
    fn a_resize_snap_respects_the_size_limits() {
        let o = still();
        let left = Edges { left: true, ..NONE };
        // 104 px wide, its left edge 6 px before the snap: 98 px after it
        let window = Rect::from_min_max(Pos2::new(o.right() + GAP - 6.0, 350.0), Pos2::new(o.right() + GAP + 98.0, 500.0));
        assert!(resized(window, left, &[o], Vec2::splat(100.0)).is_none());
        assert_eq!(resized(window, left, &[o], Vec2::splat(98.0)).shift(), Vec2::new(6.0, 0.0));
        // 896 px wide, the snap asks for 904
        let window = rect(o.left() - GAP - 896.0 - 8.0 + 16.0, 350.0, 896.0, 100.0);
        let right = Edges { right: true, ..NONE };
        let near = rect(window.right() + 8.0 + GAP, 350.0, 100.0, 100.0);
        assert!(resized(window, right, &[near], Vec2::splat(100.0)).is_none());
    }

    #[test]
    fn the_guide_runs_along_the_edges_that_meet() {
        let o = still();
        let mover = rect(o.right() + GAP + 6.0, o.top() + 4.0, 200.0, 100.0);
        let scene = Scene {
            movers: &[mover],
            edges: Edges::ALL,
            others: &[o],
            area: AREA,
            min_size: Vec2::ZERO,
            max_size: Vec2::INFINITY,
        };
        let snap = find(&scene, Snap::default());
        let shown = guide(&scene, snap);
        let landed = rect(o.right() + GAP, o.top(), 200.0, 100.0);
        assert_eq!(shown.landing, [landed]);
        // the two facing edges over the 100 px they share, then the line
        // of the tops through both windows
        assert_eq!(
            shown.lines,
            [
                [landed.left_top(), landed.left_bottom()],
                [o.right_top(), Pos2::new(o.right(), landed.bottom())],
                [o.left_top(), landed.right_top()],
            ]
        );
        // the work area: the edge of the window that lands on it
        let mover = rect(4.0, 400.0, 200.0, 100.0);
        let scene = Scene {
            movers: &[mover],
            others: &[],
            ..scene
        };
        let shown = guide(&scene, find(&scene, Snap::default()));
        assert_eq!(shown.lines, [[Pos2::new(GAP, 400.0), Pos2::new(GAP, 500.0)]]);
    }

    /// A group moves as one block: each of its windows can meet a
    /// neighbor, its bounds meet the work area.
    #[test]
    fn a_group_snaps_as_a_block() {
        let o = still();
        // two windows side by side; the second comes next to `o`
        let first = rect(o.left() - GAP - 150.0 - GAP - 150.0 - 5.0, 350.0, 150.0, 100.0);
        let second = first.translate(Vec2::new(150.0 + GAP, 0.0));
        let snap = moved(&[first, second], &[o], Snap::default());
        assert_eq!(snap.x.map(|s| (s.shift, s.mover, s.other)), Some((5.0, 1, Some(0))));
        assert_eq!(snap.adjacent().collect::<Vec<_>>(), [(1, 0)]);
        // the outer edge of the block against the work area, not the edge
        // between its windows
        let first = rect(GAP + 4.0, 350.0, 150.0, 100.0);
        let second = first.translate(Vec2::new(150.0 + GAP, 0.0));
        let snap = moved(&[second, first], &[], Snap::default());
        assert_eq!(snap.x.map(|s| (s.shift, s.kind)), Some((-4.0, Kind::Area)));
    }

    #[test]
    fn touching_windows_stand_at_the_gap_and_share_an_edge() {
        let o = still();
        assert!(touching(rect(o.right() + GAP, 350.0, 200.0, 100.0), o));
        assert!(touching(o, rect(o.right() + GAP, 350.0, 200.0, 100.0)));
        assert!(touching(rect(700.0, o.top() - GAP - 100.0, 200.0, 100.0), o));
        // rounded positions
        assert!(touching(rect(o.right() + GAP + 0.6, 350.0, 200.0, 100.0), o));
        // too far, too near (over each other), corner to corner
        assert!(!touching(rect(o.right() + GAP + 2.0, 350.0, 200.0, 100.0), o));
        assert!(!touching(rect(o.right() + GAP - 2.0, 350.0, 200.0, 100.0), o));
        assert!(!touching(rect(o.right() + GAP, o.bottom() + GAP, 200.0, 100.0), o));
        assert!(!touching(rect(o.left(), o.top(), 200.0, 100.0), o));
    }

    #[test]
    fn a_group_is_everything_linked_to_a_window() {
        let links: BTreeSet<Link> = [link(2, 1), link(2, 3), link(7, 8), link(3, 4)].into_iter().collect();
        assert_eq!(link(2, 1), (1, 2));
        assert_eq!(group(&links, 1), [1, 2, 3, 4]);
        let mut from_three = group(&links, 3);
        assert_eq!(from_three[0], 3);
        from_three.sort_unstable();
        assert_eq!(from_three, [1, 2, 3, 4]);
        assert_eq!(group(&links, 8), [8, 7]);
        assert_eq!(group(&links, 5), [5]);
        // a ring ends
        let ring: BTreeSet<Link> = [link(1, 2), link(2, 3), link(3, 1)].into_iter().collect();
        assert_eq!(group(&ring, 1).len(), 3);
    }

    /// The window dragged stops where its group reaches the edge of the
    /// screen.
    #[test]
    fn a_block_stays_inside_the_screen() {
        let screen = rect(0.0, 0.0, 1600.0, 900.0);
        let window = rect(500.0, 300.0, 200.0, 100.0);
        // a neighbor on its right and one above it
        let block = bounds(&[window, rect(708.0, 300.0, 300.0, 100.0), rect(500.0, 192.0, 200.0, 100.0)]);
        let limits = block_limits(screen, window, block);
        assert_eq!(limits, Rect::from_min_max(Pos2::new(0.0, 108.0), Pos2::new(1292.0, 900.0)));
        // against each limit, the block is against the screen
        let at_right = window.translate(Vec2::new(limits.right() - window.right(), 0.0));
        assert_eq!(at_right.right() + (block.right() - window.right()), screen.right());
        let at_top = window.translate(Vec2::new(0.0, limits.top() - window.top()));
        assert_eq!(at_top.top() - (window.top() - block.top()), screen.top());
        // alone, the screen itself
        assert_eq!(block_limits(screen, window, window), screen);
    }
}
