//! Automatic cloth freedom of a rigged mesh (Aurora's own; no Legacy Viewer
//! counterpart). Per vertex, how far the simulation may take it from its
//! skinned place: 0 pinned to the skeleton, 1 fully simulated. Nothing comes
//! from the creator: only the geometry at bind pose, the skeleton's collision
//! volumes and the skin weights.
//!
//! 1. Vertices at the same place are welded (meshes are split along UV and
//!    normal seams, and the tangents split some more).
//! 2. Anchors: vertices within `ANCHOR_DIST` of the body (the collision
//!    volumes); when nothing touches it, the part nearest to it.
//! 3. Per island, only the highest anchor cluster(s) are kept: the waistband
//!    or the scalp, not the hem brushing the thighs nor hair lying on the
//!    shoulders (those are contacts for the collisions, not pins). An island
//!    touching the body only well below its own top rests on it: no anchor.
//! 4. Distance along the surface from the anchors (Dijkstra over the welded
//!    edges); islands with no anchor (hair cards, fur tufts) hang off their
//!    nearest solved neighbour.
//!    A loose piece far from the rest (a sign, a logo) stays rigid.
//! 5. A mesh that hugs the body (mesh bodies, heads, fitted clothes, shoes;
//!    not hair, which may lie down the back) stays rigid, and so does every vertex skinned to a joint that must not
//!    swing (fingers, face, eyes, toes).
//!
//! Prototyped and checked in Blender on the legacy avatar skirt and hair and
//! on a fur tail before this port.

use glam::{Mat4, Vec3};
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Tunables, in metres (Second Life is at real scale).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Vertices closer than this are one node of the surface graph.
    pub weld: f32,
    /// A vertex this near the body touches it.
    pub anchor_dist: f32,
    /// Anchor clusters whose top is within this of the island's highest one
    /// are kept too (both shoulders of a cape).
    pub keep_band: f32,
    /// With no anchor at all: the top of the topmost island is pinned.
    pub top_band: f32,
    /// Fully pinned this far from an anchor...
    pub pin_margin: f32,
    /// ...then free over this distance (shortened on small accessories).
    pub falloff: f32,
    /// The shortest falloff a small accessory gets.
    pub min_falloff: f32,
    /// A mesh with at least `hug_fraction` of its vertices within `hug_dist`
    /// of the body is worn skin-tight: rigid.
    pub hug_dist: f32,
    pub hug_fraction: f32,
    /// Of an attachment contact, only this top band pins (a waistband, the
    /// scalp): below it the cloth rests on the body (hair down the back),
    /// held off it by the collisions and the backstop, free to lift.
    pub attach_band: f32,
    /// Hair (`RestMesh::hair`): lying on the body is no reason to keep it
    /// rigid (long hair drapes down the back), only clothes and bodies hug.
    pub drapes: bool,
    /// Only body contacts reaching this high pin (hair: the scalp). Lower
    /// ones are contacts (a lock of hair lying on the back, its own piece,
    /// hangs from the scalp's hair, not from where it touches the back).
    pub pin_above: Option<f32>,
    /// A mesh hugging the body that still has this share of its vertices
    /// further than `loose_dist` from it is a fitted top over a loose skirt,
    /// not a body: held where it hugs, free where it stands off.
    pub loose_dist: f32,
    pub loose_share: f32,
    /// A mesh worn close to the body (a fitted top, a body piece) only sways
    /// where an open edge of it stands off the body (a flared hem, a
    /// sleeve's cuff): from `sway_from` off it, fully `sway_ramp` further,
    /// at most `sway_cap` free, fading out over `sway_reach` from the edge.
    /// `sway`: weighed that way whether it hugs or not (a body-like mesh).
    pub sway: bool,
    pub sway_from: f32,
    pub sway_ramp: f32,
    pub sway_cap: f32,
    pub sway_reach: f32,
    /// Clothes worn close (with the body worn known): free by their slack,
    /// from 0 within `sway_from` of the body to fully free this much further
    /// out.
    pub slack: f32,
    /// A loose piece further than this from the rest of the mesh (a demo
    /// sign, a floating logo) is no card of it: it stays rigid.
    pub float_gap: f32,
    /// A piece of the mesh (its own vertices, not welded to the rest) no
    /// bigger across than this is solid (`Weights::pieces`): a buckle, a
    /// button, a gem.
    pub solid_piece: f32,
    /// Separate pieces this near each other (not sharing vertices: a
    /// mermaid's fins on its tail, the layers of a skirt) are stitched where
    /// they are nearest, held at their distance: they swing as one. 0: none
    /// (hair, whose cards part).
    pub stitch: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            weld: 0.0005,
            // the collision volumes are a rough body: a few centimetres
            anchor_dist: 0.04,
            keep_band: 0.05,
            top_band: 0.03,
            pin_margin: 0.02,
            falloff: 0.25,
            min_falloff: 0.06,
            hug_dist: 0.06,
            hug_fraction: 0.7,
            attach_band: 0.12,
            drapes: false,
            pin_above: None,
            loose_dist: 0.10,
            loose_share: 0.08,
            sway: false,
            sway_from: 0.02,
            sway_ramp: 0.04,
            sway_cap: 0.2,
            sway_reach: 0.12,
            slack: 0.05,
            float_gap: 0.08,
            solid_piece: 0.05,
            stitch: 0.0,
        }
    }
}

/// The body the cloth hangs from: the skeleton's collision volumes, each a
/// unit sphere under its joint's world matrix (its own scale included), and
/// the surface of the body worn when known (the fit's points).
#[derive(Clone, Debug, Default)]
pub struct Body {
    volumes: Vec<(Mat4, Mat4)>,
    surface: Option<std::sync::Arc<Surface>>,
    /// The body meshes being weighed: not measured against themselves.
    exclude: Vec<u32>,
}

/// Points on the surface of the body worn, for nearest queries.
#[derive(Debug)]
pub struct Surface {
    points: Vec<Vec3>,
    /// Per point, the body mesh it is from (`Body::excluding`).
    owners: Vec<u32>,
    grid: Grid,
}

/// Grid cell of the surface points, and how many rings a query looks
/// through (beyond, the volumes alone answer).
const SURFACE_CELL: f32 = 0.05;
const SURFACE_RINGS: i32 = 3;

impl Surface {
    /// `owners`: per point, its body mesh (missing: 0).
    pub fn new(points: &[Vec3], owners: &[u32]) -> Self {
        let mut grid = Grid::new(SURFACE_CELL);
        for (i, &p) in points.iter().enumerate() {
            grid.insert(p, i as u32);
        }
        Self {
            points: points.to_vec(),
            owners: (0..points.len()).map(|i| owners.get(i).copied().unwrap_or(0)).collect(),
            grid,
        }
    }

    /// Distance to the nearest point not of the body meshes `exclude`.
    fn distance(&self, p: Vec3, exclude: &[u32]) -> Option<f32> {
        let mut best: Option<f32> = None;
        for ring in 0..=SURFACE_RINGS {
            self.grid.around(p, ring, true, |i| {
                if exclude.contains(&self.owners[i as usize]) {
                    return;
                }
                let d = (self.points[i as usize] - p).length();
                if best.is_none_or(|b| d < b) {
                    best = Some(d);
                }
            });
            if best.is_some_and(|b| b <= ring as f32 * self.grid.cell) {
                break;
            }
        }
        best
    }
}

impl Body {
    pub fn new(volumes: impl IntoIterator<Item = Mat4>) -> Self {
        Self {
            volumes: volumes
                .into_iter()
                .filter(|m| m.determinant().abs() > 1e-12)
                .map(|m| (m, m.inverse()))
                .collect(),
            surface: None,
            exclude: Vec::new(),
        }
    }

    /// For the body's own meshes: measured against the others.
    pub fn excluding(mut self, meshes: Vec<u32>) -> Self {
        self.exclude = meshes;
        self
    }

    /// Measured against the body worn too: its volumes are rough and, fitted,
    /// held within `FIT_RANGE` (a Reborn's hips outgrow theirs: a skirt lying
    /// on them seemed to stand well off the body).
    pub fn with_surface(mut self, surface: Option<std::sync::Arc<Surface>>) -> Self {
        self.surface = surface.filter(|s| !s.points.is_empty());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.volumes.is_empty()
    }

    /// The body worn's surface is known (the volumes alone overstate how far
    /// an edge stands off: a waistband, a segment's edge near a pole).
    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    /// Which way is away from the body at `p`: from the centre of the
    /// nearest volume (zero without volumes).
    pub fn outward(&self, p: Vec3) -> Vec3 {
        self.volumes
            .iter()
            .map(|(m, inv)| (Self::signed(m, inv, p), m.w_axis.truncate()))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map_or(Vec3::ZERO, |(_, c)| (p - c).normalize_or_zero())
    }

    /// The nearest volume to `p`: its index, the signed distance and its
    /// centre.
    pub fn nearest(&self, p: Vec3) -> Option<(usize, f32, Vec3)> {
        self.volumes
            .iter()
            .enumerate()
            .map(|(i, (m, inv))| (i, Self::signed(m, inv, p), m.w_axis.truncate()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Signed distance from `p` to one volume's surface (radial projection).
    fn signed(m: &Mat4, inv: &Mat4, p: Vec3) -> f32 {
        let local = inv.transform_point3(p);
        let r = local.length();
        if r < 1e-6 {
            return -m.transform_vector3(Vec3::X).length().min(m.transform_vector3(Vec3::Y).length());
        }
        (p - m.transform_point3(local / r)).length() * if r < 1.0 { -1.0 } else { 1.0 }
    }

    /// Approximate signed distance to the nearest volume's surface (negative
    /// inside): the radial projection of `p` on each ellipsoid.
    pub fn distance(&self, p: Vec3) -> f32 {
        let volumes = self
            .volumes
            .iter()
            .map(|(m, inv)| Self::signed(m, inv, p))
            .fold(f32::INFINITY, f32::min);
        match self.surface.as_ref().and_then(|s| s.distance(p, &self.exclude)) {
            Some(d) => volumes.min(d),
            None => volumes,
        }
    }
}

/// One face of the mesh, in the order its vertices reach the GPU.
pub struct FaceInput<'a> {
    pub positions: &'a [Vec3],
    pub indices: &'a [u32],
    /// Skinned to a joint that never swings (fingers, face...).
    pub rigid: &'a [bool],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub nodes: usize,
    pub islands: usize,
    pub anchors: usize,
    pub dropped_clusters: usize,
    pub floating_islands: usize,
    /// Of those, too far from the rest to hang from it (rigid).
    pub detached: usize,
    pub body_gap: Option<f32>,
    pub max_distance: f32,
    pub hugging: bool,
    /// Share of the vertices further than `loose_dist` from the body.
    pub loose: f32,
    /// Stitches between separate pieces (`Params::stitch`).
    pub stitches: usize,
    /// Worn close, the nodes swaying at its open edges (`Params::sway`).
    pub swaying: usize,
    /// Mean resultant of the directions from the body to the vertices near
    /// it: 0 wrapped all round, 1 all on one side.
    pub wrap: f32,
    /// Per mesh of the group, how it was judged (diagnostics).
    pub members: Vec<MemberStats>,
}

/// How one mesh of a group was judged (`compute_members`; diagnostics).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemberStats {
    pub nodes: usize,
    /// Its lowest and highest node (rest pose, m).
    pub low: f32,
    pub high: f32,
    /// Nodes touching the body (within `Params::sway_from`), near it
    /// (`hug_dist`), standing off it (`loose_dist`).
    pub touching: usize,
    pub near: usize,
    pub loose: usize,
    /// It hugs the body; worn close (`hugs` and few standing off).
    pub hugs: bool,
    pub tight: bool,
    /// Its nodes pinned as anchors, skinned to joints that never swing,
    /// hanging from no anchor.
    pub anchors: usize,
    pub rigid: usize,
    pub unreached: usize,
    /// Its mean and largest freedom, and its hem's (its lowest 5 cm).
    pub mean_free: f32,
    pub max_free: f32,
    pub hem_free: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Weights {
    /// Per face, per vertex: 0 pinned .. 1 free.
    pub faces: Vec<Vec<f32>>,
    pub stats: Stats,
    /// The welded surface the simulation runs on: each face vertex's node,
    /// the nodes' bind positions and freedom, and the edges with their rest
    /// lengths.
    pub node_of: Vec<Vec<u32>>,
    pub nodes: Vec<Vec3>,
    pub node_free: Vec<f32>,
    pub edges: Vec<(u32, u32, f32)>,
    /// Pairs of nodes on either side of one, nearly in line (a strand's
    /// next but one), with their rest distance: what keeps it from folding
    /// back on itself (`sim::BEND_WATER`).
    pub bends: Vec<(u32, u32, f32)>,
    /// Per node, the anchor it hangs from and how far along the surface
    /// (a node with none: itself, 0).
    pub node_root: Vec<u32>,
    pub node_reach: Vec<f32>,
    /// The small pieces that move as one, never bending
    /// (`Params::solid_piece`): their nodes.
    pub pieces: Vec<Vec<u32>>,
}

/// `Weights::node_of` of a vertex whose node was pruned: it never moves.
pub const NO_NODE: u32 = u32::MAX;

impl Weights {
    /// Nothing of the mesh moves: no simulation needed.
    pub fn is_rigid(&self) -> bool {
        self.faces.iter().flatten().all(|&f| f <= 0.0)
    }

    /// Keeps only the nodes the simulation needs: those that move, the
    /// pinned ones an edge, a bend or a tether ties them to, and drops the
    /// rest (their vertices `NO_NODE`). A pinned node only follows its
    /// skinning; stepped and skinned every frame, the bodice and sleeves of
    /// a dress in many pieces took the whole budget and its skirt was left
    /// rigid. Returns, per node kept, its index before.
    pub fn prune(&mut self) -> Vec<u32> {
        let n = self.nodes.len();
        let moving = |u: u32| self.node_free.get(u as usize).is_some_and(|&f| f > 0.0);
        let mut keep = vec![false; n];
        for u in 0..n as u32 {
            if moving(u) {
                keep[u as usize] = true;
                if let Some(&r) = self.node_root.get(u as usize) {
                    keep[r as usize] = true;
                }
            }
        }
        for &(a, b, _) in self.edges.iter().chain(&self.bends) {
            if moving(a) || moving(b) {
                keep[a as usize] = true;
                keep[b as usize] = true;
            }
        }
        let kept: Vec<u32> = (0..n as u32).filter(|&u| keep[u as usize]).collect();
        let mut new_of = vec![NO_NODE; n];
        for (i, &u) in kept.iter().enumerate() {
            new_of[u as usize] = i as u32;
        }
        let constraint = |list: &[(u32, u32, f32)]| -> Vec<(u32, u32, f32)> {
            list.iter()
                .filter(|&&(a, b, _)| moving(a) || moving(b))
                .map(|&(a, b, w)| (new_of[a as usize], new_of[b as usize], w))
                .collect()
        };
        let edges = constraint(&self.edges);
        let bends = constraint(&self.bends);
        let pick = |v: &[f32]| -> Vec<f32> { kept.iter().map(|&u| v.get(u as usize).copied().unwrap_or(0.0)).collect() };
        self.node_free = pick(&self.node_free);
        self.node_reach = pick(&self.node_reach);
        self.node_root = kept
            .iter()
            .enumerate()
            .map(|(i, &u)| {
                let r = self.node_root.get(u as usize).map_or(NO_NODE, |&r| new_of[r as usize]);
                if r == NO_NODE { i as u32 } else { r }
            })
            .collect();
        self.nodes = kept.iter().map(|&u| self.nodes[u as usize]).collect();
        self.edges = edges;
        self.bends = bends;
        self.pieces = std::mem::take(&mut self.pieces)
            .into_iter()
            .map(|m| {
                m.into_iter()
                    .map(|u| new_of.get(u as usize).copied().unwrap_or(NO_NODE))
                    .filter(|&u| u != NO_NODE)
                    .collect::<Vec<u32>>()
            })
            .filter(|m| m.len() > 1)
            .collect();
        for map in &mut self.node_of {
            for u in map.iter_mut() {
                *u = new_of.get(*u as usize).copied().unwrap_or(NO_NODE);
            }
        }
        kept
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Spatial hash of points for weld and nearest queries.
#[derive(Debug)]
struct Grid {
    cell: f32,
    cells: HashMap<(i32, i32, i32), Vec<u32>>,
}

impl Grid {
    fn new(cell: f32) -> Self {
        Self {
            cell,
            cells: HashMap::new(),
        }
    }
    fn key(&self, p: Vec3) -> (i32, i32, i32) {
        let c = (p / self.cell).floor();
        (c.x as i32, c.y as i32, c.z as i32)
    }
    fn insert(&mut self, p: Vec3, i: u32) {
        self.cells.entry(self.key(p)).or_default().push(i);
    }
    /// Points of the cells within `ring` cells of `p`'s; `shell`: only those
    /// exactly `ring` cells away (the inner ones were already searched).
    fn around(&self, p: Vec3, ring: i32, shell: bool, mut f: impl FnMut(u32)) {
        let (x, y, z) = self.key(p);
        for dx in -ring..=ring {
            for dy in -ring..=ring {
                for dz in -ring..=ring {
                    if shell && ring > 0 && dx.abs() != ring && dy.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    if let Some(v) = self.cells.get(&(x + dx, y + dy, z + dz)) {
                        v.iter().copied().for_each(&mut f);
                    }
                }
            }
        }
    }
    /// Nearest point to `p` (by `pos`), searching ring by ring.
    fn nearest(&self, p: Vec3, pos: &[Vec3], max_ring: i32) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        for ring in 0..=max_ring {
            self.around(p, ring, true, |i| {
                let d = (pos[i as usize] - p).length();
                if best.is_none_or(|b| d < b.1) {
                    best = Some((i, d));
                }
            });
            // a point found in ring r may still be beaten in ring r + 1
            if let Some(b) = best
                && b.1 <= ring as f32 * self.cell
            {
                break;
            }
        }
        best
    }
}

#[derive(PartialEq)]
struct Item(f32, u32);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    // min-heap on the distance
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

fn components(n: usize, adj: &[Vec<(u32, f32)>], allowed: Option<&[bool]>) -> Vec<Vec<u32>> {
    let mut seen = vec![false; n];
    let mut out = Vec::new();
    for s in 0..n {
        if seen[s] || allowed.is_some_and(|a| !a[s]) {
            continue;
        }
        seen[s] = true;
        let mut stack = vec![s as u32];
        let mut members = Vec::new();
        while let Some(u) = stack.pop() {
            members.push(u);
            for &(v, _) in &adj[u as usize] {
                if !seen[v as usize] && allowed.is_none_or(|a| a[v as usize]) {
                    seen[v as usize] = true;
                    stack.push(v);
                }
            }
        }
        out.push(members);
    }
    out
}

/// `root`: per node, the anchor its distance is measured from (seeds: the
/// node, its distance, its anchor).
fn dijkstra(adj: &[Vec<(u32, f32)>], dist: &mut [f32], root: &mut [u32], seeds: impl IntoIterator<Item = (u32, f32, u32)>) {
    let mut heap = BinaryHeap::new();
    for (u, d, r) in seeds {
        if d < dist[u as usize] {
            dist[u as usize] = d;
            root[u as usize] = r;
            heap.push(Item(d, u));
        }
    }
    while let Some(Item(d, u)) = heap.pop() {
        if d > dist[u as usize] {
            continue;
        }
        for &(v, w) in &adj[u as usize] {
            let nd = d + w;
            if nd < dist[v as usize] {
                dist[v as usize] = nd;
                root[v as usize] = root[u as usize];
                heap.push(Item(nd, v));
            }
        }
    }
}

/// How one-sided the points near the body are: per nearest volume, the
/// length of the mean of their directions from its centre, averaged over the
/// volumes by their share of the points (0 wrapped all round, 1 one-sided).
fn wrap_of(body: &Body, points: impl Iterator<Item = Vec3>) -> f32 {
    let mut sums: HashMap<usize, (Vec3, u32)> = HashMap::new();
    for q in points {
        if let Some((i, _, c)) = body.nearest(q) {
            let e = sums.entry(i).or_insert((Vec3::ZERO, 0));
            e.0 += (q - c).normalize_or_zero();
            e.1 += 1;
        }
    }
    let total: u32 = sums.values().map(|e| e.1).sum();
    if total == 0 {
        return 1.0;
    }
    sums.values().map(|(v, _)| v.length()).sum::<f32>() / total as f32
}

/// Whether points (a mesh at rest) hug the body as a body part, a segment
/// of one or fitted clothes do: most of them near it. Also its wrap.
pub fn hugs(body: &Body, points: &[Vec3], p: &Params) -> (bool, f32) {
    if points.is_empty() || body.is_empty() {
        return (false, 1.0);
    }
    let near: Vec<Vec3> = points.iter().copied().filter(|&q| body.distance(q) <= p.hug_dist).collect();
    let wrap = wrap_of(body, near.iter().copied());
    let loose = points.iter().filter(|&&q| body.distance(q) > p.loose_dist).count();
    (
        near.len() as f32 >= p.hug_fraction * points.len() as f32 && (loose as f32) < p.loose_share * points.len() as f32,
        wrap,
    )
}

/// How much each collision volume must grow or shrink to fit the body
/// actually worn (`points`: its surface at rest, from the meshes that hug
/// the default body: a mesh body or its segments, head, hands, feet). Each
/// point counts for its nearest volume, if near it; a volume's scale is the
/// median of their distances in its own units (the body all round it),
/// kept within `FIT_RANGE`: a Reborn's hips grow theirs, a slim body's
/// shrink. A volume with too few points keeps its size. One uniform scale:
/// per axis, the points beyond a limb volume's ends (its neighbour's) blew
/// the length up.
pub fn fit_scales(volumes: &[Mat4], points: &[Vec3]) -> Vec<Vec3> {
    let body = Body::new(volumes.iter().copied());
    let inverses: Vec<Mat4> = volumes.iter().map(|m| m.inverse()).collect();
    let mut per: Vec<Vec<f32>> = vec![Vec::new(); volumes.len()];
    for &q in points {
        let Some((i, d, _)) = body.nearest(q) else { continue };
        if d.abs() > 0.1 {
            continue;
        }
        per[i].push(inverses[i].transform_point3(q).length());
    }
    per.into_iter()
        .map(|mut r| {
            if r.len() < 24 {
                return Vec3::ONE;
            }
            r.sort_by(f32::total_cmp);
            Vec3::splat(r[r.len() / 2].clamp(FIT_RANGE.0, FIT_RANGE.1))
        })
        .collect()
}

/// How far a fit may scale a volume: the body worn differs from the
/// default, not by a whole size.
pub const FIT_RANGE: (f32, f32) = (0.8, 1.4);

/// The freedom of every vertex of a mesh's faces. `body`: the collision
/// volumes at bind pose, in the same space as the positions.
#[cfg(test)]
pub fn compute(faces: &[FaceInput], body: Option<&Body>, p: &Params) -> Weights {
    compute_members(faces, &[], body, p)
}

/// [`compute`] for the meshes of a group weighed together (`members`: per
/// face, its mesh; missing: 0). Welded where they meet, but each mesh judged
/// on its own: whether it hugs the body (one hugging is held, the others
/// swing: a fitted bodice and the skirt pieces of a dress), how near the
/// body its pins are (a second hair's base, a little further from the head
/// than the first's, was left unpinned and hung off the first's ends), its
/// top pinned when nothing of it touches.
pub fn compute_members(faces: &[FaceInput], members: &[u32], body: Option<&Body>, p: &Params) -> Weights {
    let body = body.filter(|b| !b.is_empty());
    let member_of_face = |f: usize| members.get(f).copied().unwrap_or(0) as usize;
    let count = (0..faces.len()).map(|f| member_of_face(f) + 1).max().unwrap_or(1);
    // --- weld ----------------------------------------------------------------
    let mut grid = Grid::new(p.weld.max(1e-5) * 2.0);
    let mut nodes: Vec<Vec3> = Vec::new();
    let mut rigid_node: Vec<bool> = Vec::new();
    let mut node_of: Vec<Vec<u32>> = Vec::with_capacity(faces.len());
    // per node, the mesh it was first found in
    let mut node_member: Vec<usize> = Vec::new();
    for (fi, f) in faces.iter().enumerate() {
        let mut map = Vec::with_capacity(f.positions.len());
        for (vi, &pos) in f.positions.iter().enumerate() {
            let mut found = None;
            grid.around(pos, 1, false, |i| {
                if found.is_none() && (nodes[i as usize] - pos).length() <= p.weld {
                    found = Some(i);
                }
            });
            let id = found.unwrap_or_else(|| {
                let id = nodes.len() as u32;
                nodes.push(pos);
                rigid_node.push(false);
                node_member.push(member_of_face(fi));
                grid.insert(pos, id);
                id
            });
            if f.rigid.get(vi).copied().unwrap_or(false) {
                rigid_node[id as usize] = true;
            }
            map.push(id);
        }
        node_of.push(map);
    }
    let n = nodes.len();
    let mut stats = Stats {
        nodes: n,
        ..Default::default()
    };
    let zeros = |stats: Stats| Weights {
        faces: faces.iter().map(|f| vec![0.0; f.positions.len()]).collect(),
        stats,
        ..Default::default()
    };
    if n == 0 {
        return zeros(stats);
    }
    // --- graph ---------------------------------------------------------------
    let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
    // triangles per edge: an edge of one only is an open edge (a hem)
    let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
    let mut link = |a: u32, b: u32| {
        if a != b && !adj[a as usize].iter().any(|e| e.0 == b) {
            let w = (nodes[a as usize] - nodes[b as usize]).length();
            adj[a as usize].push((b, w));
            adj[b as usize].push((a, w));
        }
    };
    for (f, map) in faces.iter().zip(&node_of) {
        for t in f.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| map.get(i as usize).copied());
            if let (Some(a), Some(b), Some(c)) = (a, b, c) {
                for (x, y) in [(a, b), (b, c), (c, a)] {
                    if x != y {
                        *uses.entry((x.min(y), x.max(y))).or_default() += 1;
                    }
                }
                link(a, b);
                link(b, c);
                link(c, a);
            }
        }
    }
    // small pieces of their own: solid (buckles, buttons, gems)
    let solid: Vec<Vec<u32>> = components(n, &adj, None)
        .into_iter()
        .filter(|m| {
            let (lo, hi) = m.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &u| {
                (lo.min(nodes[u as usize]), hi.max(nodes[u as usize]))
            });
            m.len() > 1 && (hi - lo).length() <= p.solid_piece
        })
        .collect();
    // stitches: each node to the nearest node of another piece within reach
    // (an edge held at its rest length, as the welded ones)
    if p.stitch > 0.0 {
        let pieces = components(n, &adj, None);
        let mut piece_of = vec![0u32; n];
        for (i, m) in pieces.iter().enumerate() {
            for &u in m {
                piece_of[u as usize] = i as u32;
            }
        }
        if pieces.len() > 1 {
            let mut grid = Grid::new(p.stitch);
            for (u, &q) in nodes.iter().enumerate() {
                grid.insert(q, u as u32);
            }
            let mut stitches: Vec<(u32, u32)> = Vec::new();
            for u in 0..n {
                let mut best: Option<(u32, f32)> = None;
                grid.around(nodes[u], 1, false, |v| {
                    if piece_of[v as usize] != piece_of[u] {
                        let d = (nodes[v as usize] - nodes[u]).length();
                        if d <= p.stitch && best.is_none_or(|b| d < b.1) {
                            best = Some((v, d));
                        }
                    }
                });
                if let Some((v, _)) = best {
                    stitches.push((u as u32, v));
                }
            }
            stats.stitches = stitches.len();
            for (a, b) in stitches {
                if !adj[a as usize].iter().any(|e| e.0 == b) {
                    let w = (nodes[a as usize] - nodes[b as usize]).length();
                    adj[a as usize].push((b, w));
                    adj[b as usize].push((a, w));
                }
            }
        }
    }
    let islands = components(n, &adj, None);
    stats.islands = islands.len();
    let mut island_of = vec![0u32; n];
    for (i, m) in islands.iter().enumerate() {
        for &u in m {
            island_of[u as usize] = i as u32;
        }
    }
    // --- anchors -------------------------------------------------------------
    let mut anchor = rigid_node.clone();
    let joint_rigid = rigid_node.clone();
    // per mesh: whether it hugs, whether worn close (diagnostics)
    let mut judged: Vec<(bool, bool)> = vec![(false, false); count];
    let mut bdist_all: Vec<f32> = Vec::new();
    // per node of a mesh worn close, its freedom by its slack
    let mut slack_free: Vec<Option<f32>> = vec![None; n];
    if let Some(body) = body {
        let bdist: Vec<f32> = nodes.iter().map(|&q| body.distance(q)).collect();
        bdist_all = bdist.clone();
        let dmin = bdist.iter().copied().fold(f32::INFINITY, f32::min);
        stats.body_gap = Some(dmin);
        let near_body = bdist.iter().filter(|&&d| d <= p.hug_dist).count();
        let wrap = wrap_of(body, nodes.iter().zip(&bdist).filter(|(_, d)| **d <= p.hug_dist).map(|(q, _)| *q));
        stats.wrap = wrap;
        // (not the wrap too: a mesh body split into segments, each covering
        // one side, is as one-sided as hair lying down the back: a segment
        // taken for cloth swings out of the body, far worse than hair kept
        // still; the wrap is logged)
        stats.loose = bdist.iter().filter(|&&d| d > p.loose_dist).count() as f32 / n as f32;
        // per mesh: its nodes, those near the body, those standing off it,
        // its nearest
        let mut per = vec![(0usize, 0usize, 0usize, f32::INFINITY); count];
        for u in 0..n {
            let m = &mut per[node_member[u]];
            m.0 += 1;
            m.1 += (bdist[u] <= p.hug_dist) as usize;
            m.2 += (bdist[u] > p.loose_dist) as usize;
            m.3 = m.3.min(bdist[u]);
        }
        let hugs_m: Vec<bool> = per
            .iter()
            .map(|&(total, near, _, _)| total > 0 && !p.drapes && near as f32 >= p.hug_fraction * total as f32)
            .collect();
        let tight_m: Vec<bool> = per
            .iter()
            .zip(&hugs_m)
            .map(|(&(total, _, loose, _), &h)| h && (loose as f32) < p.loose_share * total as f32)
            .collect();
        let hugs = !p.drapes && near_body as f32 >= p.hug_fraction * n as f32;
        judged = hugs_m.iter().copied().zip(tight_m.iter().copied()).collect();
        let mut open = vec![false; n];
        for (&(a, b), &c) in &uses {
            if c == 1 {
                open[a as usize] = true;
                open[b as usize] = true;
            }
        }
        // (a body-like mesh sways at its open edges only; clothes worn close
        // are weighed below)
        if count == 1 && (p.sway || (hugs && stats.loose < p.loose_share && !body.has_surface())) {
            stats.hugging = true;
            if !body.has_surface() {
                return zeros(stats);
            }
            return sway(&nodes, &adj, &open, &bdist, &rigid_node, node_of, faces, stats, p);
        }
        stats.hugging = tight_m.iter().any(|&t| t);
        let touching = body.has_surface();
        for u in 0..n {
            let m = node_member[u];
            // skinned to joints that never swing (a hand) but standing off
            // the body worn: a cuff over the hand, not a glove
            if touching && !p.drapes && joint_rigid[u] && bdist[u] > p.sway_from {
                rigid_node[u] = false;
                anchor[u] = false;
            }
            // clothes worn close: as free as they are slack on the body worn
            // (`Params::slack`), pinned where snug. Pinned wherever they
            // touched, a sweater's sleeves along the arms never moved; hung
            // from the shoulders, its body would balloon. (Its hems are
            // modelled with a thickness, no open edge: swaying at its open
            // edges only, it never moved at all.) Against the rough volumes
            // alone, held whole
            if tight_m[m] {
                if !touching {
                    rigid_node[u] = true;
                    anchor[u] = true;
                } else {
                    slack_free[u] = Some(smoothstep((bdist[u] - p.sway_from) / p.slack));
                    if bdist[u] <= p.sway_from {
                        anchor[u] = true;
                    }
                }
            // a fitted top over a loose skirt: held wherever it hugs
            } else if hugs_m[m] && bdist[u] <= p.hug_dist {
                anchor[u] = true;
            }
        }
        let thresh: Vec<f32> = per
            .iter()
            .zip(&hugs_m)
            .map(|(&(_, _, _, dmin), &h)| {
                if h {
                    -1.0
                } else if dmin <= p.anchor_dist {
                    p.anchor_dist
                } else {
                    dmin + p.anchor_dist
                }
            })
            .collect();
        let near: Vec<bool> = (0..n).map(|u| bdist[u] <= thresh[node_member[u]]).collect();
        let clusters = components(n, &adj, Some(&near));
        let mut by_island: HashMap<u32, Vec<(f32, &Vec<u32>)>> = HashMap::new();
        for c in &clusters {
            let top = c.iter().map(|&u| nodes[u as usize].z).fold(f32::MIN, f32::max);
            by_island.entry(island_of[c[0] as usize]).or_default().push((top, c));
        }
        for (island, cs) in &by_island {
            let low = cs.iter().filter(|c| p.pin_above.is_some_and(|z| c.0 < z)).count();
            stats.dropped_clusters += low;
            let cs: Vec<(f32, &Vec<u32>)> = cs.iter().filter(|c| p.pin_above.is_none_or(|z| c.0 >= z)).copied().collect();
            if cs.is_empty() {
                continue;
            }
            let cs = &cs;
            let best = cs.iter().map(|c| c.0).fold(f32::MIN, f32::max);
            // touching the body only well below its own top, the island rests
            // on it (a card on the back): it hangs off its neighbours instead
            let island_top = islands[*island as usize]
                .iter()
                .map(|&u| nodes[u as usize].z)
                .fold(f32::MIN, f32::max);
            if best < island_top - p.keep_band {
                stats.dropped_clusters += cs.len();
                continue;
            }
            for (top, c) in cs {
                if *top >= best - p.keep_band {
                    for &u in c.iter() {
                        if nodes[u as usize].z >= *top - p.attach_band {
                            anchor[u as usize] = true;
                        }
                    }
                } else {
                    stats.dropped_clusters += 1;
                }
            }
        }
    }
    // a mesh whose pieces hang from nothing pinned: the top of its topmost
    // islands (the highest mesh first: a chain of pieces hangs from its top)
    let mut order: Vec<usize> = (0..count).collect();
    let top_of = |m: usize| (0..n).filter(|&u| node_member[u] == m).map(|u| nodes[u].z).fold(f32::MIN, f32::max);
    order.sort_by(|&a, &b| top_of(b).total_cmp(&top_of(a)));
    for m in order {
        // (pinned somewhere itself: its other pieces hang off the nearest,
        // `floating_islands` below)
        if (0..n).any(|u| node_member[u] == m && anchor[u]) {
            continue;
        }
        let held: Vec<bool> = islands.iter().map(|isl| isl.iter().any(|&u| anchor[u as usize])).collect();
        let tops: Vec<f32> = islands
            .iter()
            .zip(&held)
            .map(|(isl, &held)| {
                if held {
                    return f32::MIN;
                }
                isl.iter()
                    .filter(|&&u| node_member[u as usize] == m)
                    .map(|&u| nodes[u as usize].z)
                    .fold(f32::MIN, f32::max)
            })
            .collect();
        let best = tops.iter().copied().fold(f32::MIN, f32::max);
        if best == f32::MIN {
            continue;
        }
        for (isl, &top) in islands.iter().zip(&tops) {
            if top > f32::MIN && top >= best - p.keep_band {
                for &u in isl {
                    if node_member[u as usize] == m && nodes[u as usize].z >= top - p.top_band {
                        anchor[u as usize] = true;
                    }
                }
            }
        }
    }
    stats.anchors = anchor.iter().filter(|&&a| a).count();
    // --- distance along the surface -------------------------------------------
    let mut dist = vec![f32::INFINITY; n];
    let mut root: Vec<u32> = (0..n as u32).collect();
    dijkstra(
        &adj,
        &mut dist,
        &mut root,
        (0..n as u32).filter(|&u| anchor[u as usize]).map(|u| (u, 0.0, u)),
    );
    // islands with no anchor hang off their nearest solved neighbour, nearest
    // island first so chains of cards resolve in order
    let mut unsolved: Vec<usize> = (0..islands.len()).filter(|&i| dist[islands[i][0] as usize].is_infinite()).collect();
    stats.floating_islands = unsolved.len();
    if !unsolved.is_empty() {
        let extent = nodes.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &q| (lo.min(q), hi.max(q)));
        let cell = ((extent.1 - extent.0).max_element() / 32.0).max(0.005);
        let max_ring = 40;
        let mut solved = Grid::new(cell);
        for u in 0..n {
            if dist[u].is_finite() {
                solved.insert(nodes[u], u as u32);
            }
        }
        while !unsolved.is_empty() {
            let mut best: Option<(usize, f32, Vec<(u32, f32, u32)>)> = None;
            for (k, &i) in unsolved.iter().enumerate() {
                let mut gap = f32::INFINITY;
                let mut seeds = Vec::with_capacity(islands[i].len());
                for &u in &islands[i] {
                    let q = nodes[u as usize];
                    let (s, g) = solved.nearest(q, &nodes, max_ring).unwrap_or_else(|| {
                        // beyond the rings: the slow way
                        (0..n as u32)
                            .filter(|&s| dist[s as usize].is_finite())
                            .map(|s| (s, (nodes[s as usize] - q).length()))
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .unwrap_or((u, 0.0))
                    });
                    seeds.push((u, dist[s as usize] + g, root[s as usize]));
                    gap = gap.min(g);
                }
                if best.as_ref().is_none_or(|b| gap < b.1) {
                    best = Some((k, gap, seeds));
                }
            }
            // the nearest is far from the mesh already: none of the rest
            // hangs from it (signs, logos)
            let Some((k, _, seeds)) = best.filter(|b| b.1 <= p.float_gap) else {
                stats.detached = unsolved.len();
                break;
            };
            let i = unsolved.swap_remove(k);
            dijkstra(&adj, &mut dist, &mut root, seeds);
            for &u in &islands[i] {
                if dist[u as usize].is_finite() {
                    solved.insert(nodes[u as usize], u);
                }
            }
        }
    }
    // --- freedom ---------------------------------------------------------------
    let max_d = dist.iter().copied().filter(|d| d.is_finite()).fold(0.0, f32::max);
    stats.max_distance = max_d;
    // a small accessory (a tail, a short ponytail, a top's short flare)
    // ramps over its own length: each mesh's own (a top worn with a long
    // scarf barely moved, ramped over the scarf's length)
    let mut longest = vec![0.0f32; count];
    for u in 0..n {
        if dist[u].is_finite() {
            longest[node_member[u]] = longest[node_member[u]].max(dist[u]);
        }
    }
    let falloff: Vec<f32> = longest
        .iter()
        .map(|&d| p.falloff.min((d - p.pin_margin) * 0.8).max(p.min_falloff))
        .collect();
    let free: Vec<f32> = (0..n)
        .map(|u| {
            if rigid_node[u] {
                0.0
            } else if let Some(slack) = slack_free[u] {
                slack
            } else if !dist[u].is_finite() {
                0.0
            } else {
                smoothstep((dist[u] - p.pin_margin) / falloff[node_member[u]])
            }
        })
        .collect();
    stats.members = (0..count)
        .map(|m| {
            let mine: Vec<usize> = (0..n).filter(|&u| node_member[u] == m).collect();
            let low = mine.iter().map(|&u| nodes[u].z).fold(f32::INFINITY, f32::min);
            let high = mine.iter().map(|&u| nodes[u].z).fold(f32::NEG_INFINITY, f32::max);
            let within = |d: f32| mine.iter().filter(|&&u| bdist_all.get(u).is_some_and(|&b| b <= d)).count();
            let hem: Vec<f32> = mine.iter().filter(|&&u| nodes[u].z <= low + 0.05).map(|&u| free[u]).collect();
            MemberStats {
                nodes: mine.len(),
                low,
                high,
                touching: within(p.sway_from),
                near: within(p.hug_dist),
                loose: mine
                    .iter()
                    .filter(|&&u| bdist_all.get(u).is_some_and(|&b| b > p.loose_dist))
                    .count(),
                hugs: judged[m].0,
                tight: judged[m].1,
                anchors: mine.iter().filter(|&&u| anchor[u]).count(),
                rigid: mine.iter().filter(|&&u| joint_rigid[u]).count(),
                unreached: mine.iter().filter(|&&u| !dist[u].is_finite()).count(),
                mean_free: mine.iter().map(|&u| free[u]).sum::<f32>() / mine.len().max(1) as f32,
                max_free: mine.iter().map(|&u| free[u]).fold(0.0, f32::max),
                hem_free: hem.iter().sum::<f32>() / hem.len().max(1) as f32,
            }
        })
        .collect();
    let bends = bend_pairs(&nodes, &adj);
    let edges = adj
        .iter()
        .enumerate()
        .flat_map(|(a, e)| e.iter().filter(move |&&(b, _)| (a as u32) < b).map(move |&(b, w)| (a as u32, b, w)))
        .collect();
    Weights {
        faces: node_of.iter().map(|map| map.iter().map(|&u| free[u as usize]).collect()).collect(),
        stats,
        node_of,
        nodes,
        node_free: free,
        bends,
        edges,
        node_reach: (0..n).map(|u| if dist[u].is_finite() { dist[u] } else { 0.0 }).collect(),
        pieces: solid,
        node_root: (0..n).map(|u| if dist[u].is_finite() { root[u] } else { u as u32 }).collect(),
    }
}

/// The sway of nodes worn close: free (at most `Params::sway_cap`) within
/// `sway_reach` of an open edge standing off the body, by how far each
/// stands off it; 0 elsewhere and where `rigid`.
fn sway_free(adj: &[Vec<(u32, f32)>], open: &[bool], bdist: &[f32], rigid: &[bool], p: &Params) -> Vec<f32> {
    let n = adj.len();
    let mut dist = vec![f32::INFINITY; n];
    let mut root: Vec<u32> = (0..n as u32).collect();
    dijkstra(
        adj,
        &mut dist,
        &mut root,
        (0..n as u32)
            .filter(|&u| open[u as usize] && bdist[u as usize] > p.sway_from)
            .map(|u| (u, 0.0, u)),
    );
    (0..n)
        .map(|u| {
            if rigid[u] || !dist[u].is_finite() {
                0.0
            } else {
                p.sway_cap * smoothstep((bdist[u] - p.sway_from) / p.sway_ramp) * (1.0 - smoothstep(dist[u] / p.sway_reach))
            }
        })
        .collect()
}

/// Weights of a mesh worn close to the body: rigid but where an open edge
/// stands off it (`Params::sway`).
#[allow(clippy::too_many_arguments)]
fn sway(
    nodes: &[Vec3],
    adj: &[Vec<(u32, f32)>],
    open: &[bool],
    bdist: &[f32],
    rigid: &[bool],
    node_of: Vec<Vec<u32>>,
    faces: &[FaceInput],
    mut stats: Stats,
    p: &Params,
) -> Weights {
    let n = nodes.len();
    let free = sway_free(adj, open, bdist, rigid, p);
    stats.swaying = free.iter().filter(|&&f| f > 0.0).count();
    if stats.swaying == 0 {
        return Weights {
            faces: faces.iter().map(|f| vec![0.0; f.positions.len()]).collect(),
            stats,
            ..Default::default()
        };
    }
    stats.max_distance = 0.0;
    let edges = adj
        .iter()
        .enumerate()
        .flat_map(|(a, e)| e.iter().filter(move |&&(b, _)| (a as u32) < b).map(move |&(b, w)| (a as u32, b, w)))
        .collect();
    Weights {
        faces: node_of.iter().map(|map| map.iter().map(|&u| free[u as usize]).collect()).collect(),
        stats,
        node_of,
        nodes: nodes.to_vec(),
        node_free: free,
        edges,
        bends: Vec::new(),
        // (no tether: it swings a few centimetres off its place)
        node_root: (0..n as u32).collect(),
        node_reach: vec![0.0; n],
        pieces: Vec::new(),
    }
}

/// Nodes nearly in line through a third (`Weights::bends`).
pub(super) fn bend_pairs(nodes: &[Vec3], adj: &[Vec<(u32, f32)>]) -> Vec<(u32, u32, f32)> {
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let mut out = Vec::new();
    for (b, around) in adj.iter().enumerate() {
        for (i, &(a, _)) in around.iter().enumerate() {
            for &(c, _) in &around[i + 1..] {
                let da = (nodes[a as usize] - nodes[b]).normalize_or_zero();
                let dc = (nodes[c as usize] - nodes[b]).normalize_or_zero();
                if da.dot(dc) < -0.85 {
                    let k = (a.min(c), a.max(c));
                    if seen.insert(k) {
                        out.push((k.0, k.1, (nodes[a as usize] - nodes[c as usize]).length()));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "weights_tests.rs"]
mod tests;
