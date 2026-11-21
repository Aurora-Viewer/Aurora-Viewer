//! Cloth simulation of a worn rigged mesh (Aurora's own; no Legacy Viewer
//! counterpart): position-based dynamics on the welded surface of the mesh,
//! in world space, so the avatar's walk, turns and jumps swing it.
//!
//! Each node is pulled toward its skinned place by its pin weight (1 -
//! freedom, weights.rs) and may stray from it at most `freedom × MAX_STRAY`;
//! the edges keep their bind lengths; the skeleton's collision volumes push
//! it out. What reaches the GPU is, per vertex, the offset from its skinned
//! place taken back through its skin matrix: the vertex shader adds it before
//! skinning (`cloth_delta`, object.wgsl), in every pass.

use super::coarse::Lattice;
use super::weights::Weights;
use glam::{Mat3, Mat4, Vec3};
use rayon::prelude::*;

/// Furthest a fully free vertex may stray from its skinned place.
pub const MAX_STRAY: f32 = 0.6;
/// Constraint passes per step.
const ITERATIONS: usize = 4;
/// Velocity kept per 1/60 s: the air drag that makes the hem trail while
/// walking (lag ≈ drag × speed / spring: about 4 cm at a walk, 7 cm running).
const DAMPING: f32 = 0.95;
/// The simulation's fixed step: frames, however uneven, add up time and the
/// cloth steps by this; what is drawn is interpolated between its last two
/// steps (a varying step jerked the cloth at high frame rates). (The backstop,
/// not a shorter step, keeps a quick wiggle out of the legs: it tests each
/// vertex against its own surface, which nothing can jump over.)
const MAX_STEP: f32 = 1.0 / 60.0;
/// A skeleton that jumped further than this in one frame (a teleport, a
/// region crossing, a sit) restarts its cloth at the skinned pose.
const RESET_JUMP: f32 = 2.0;
/// Share of the avatar's own motion (its root's move and turn each step)
/// the cloth is carried along by; the rest it feels as inertia. In pure
/// world space every wobble of the root (simulator corrections, motion
/// prediction, the pelvis bobbing in the walk) twitches it; real cloth
/// shrugs most of that off (Unity's world velocity / acceleration scale).
const CARRY: f32 = 0.2;
/// Time constant of the smoothed root: its fast wobbles (above ~2 Hz) the
/// cloth follows rigidly, only the smooth motion swings it (tuned with
/// `small_root_wobbles_do_not_twitch_the_hem`: 1 cm wobbles twitched the hem
/// 2.4 cm a frame in pure world space, 3.5 mm now).
const ROOT_SMOOTHING: f32 = 0.06;
/// The creator modelled the drape at rest, so full gravity would sag it
/// further: only enough to make the cloth fall back heavily after a swing.
const GRAVITY: Vec3 = Vec3::new(0.0, 0.0, -9.81 * 0.3);
const DOWN: Vec3 = Vec3::new(0.0, 0.0, -1.0);
/// The gravity the drape does not already hold: real down less the body's
/// rest down as the body (its pelvis) now turns it. Nothing standing;
/// upside down, twice gravity toward the ground (the skirt falls over the
/// wearer). The body's, not each vertex's through its skinning: an
/// animation curling a tail up is no reason to drag it down.
const TILT_GRAVITY: f32 = 9.81;
/// How much a fully upturned vertex (tilt 1) loosens its shape spring and
/// how much further than `MAX_STRAY` / `BACKSTOP` it may go: an upturned
/// skirt cannot keep its modelled shape, it must fall.
const TILT_LOOSEN: f32 = 0.9;
const TILT_STRAY: f32 = 1.4;
const TILT_BACKSTOP: f32 = 0.6;
/// Share of the way back to its skinned place a fully free vertex goes per
/// 1/60 s step: the spring that keeps the creator's shape at rest, so the
/// simulation only adds the motion (inertia against it).
const SHAPE_PULL: f32 = 0.045;
/// A node never gets further from the pinned node it hangs from than this
/// times its distance along the surface (or than the skinning puts it): a
/// long strand, held by its edges alone over a few passes, stretched to
/// thrice its length when the avatar flew (long-range attachments, as in
/// Kim et al. 2012).
const TETHER_SLACK: f32 = 1.05;
/// Acceleration of a fully free vertex per m/s of wind (the weather's): a
/// steady 5 m/s breeze holds a hem about 10 cm downwind against its shape
/// spring; the gusts and a flutter per vertex make it move.
const WIND_PUSH: f32 = 3.0;
/// Air drag growing with the square of the speed through the air (per m/s,
/// on top of `DAMPING`'s linear drag): at a walk it is nothing, flying at
/// 10 m/s it streams hair behind (30 m/s²) rather than letting it hang down
/// under the tilt's gravity.
const AIR_DRAG: f32 = 0.3;
/// Fastest air taken into account (a teleport's jump is no wind).
const MAX_AIR: f32 = 30.0;
/// Under water (below the region's water line, `ClothState::water`): the
/// drape no longer hangs, most of the gravity it was modelled under is
/// lifted (`WATER_LIFT` of g, upward; the rest lets it settle down a
/// little between the current's drifts) and the shape spring loosened
/// (`WATER_LOOSEN`): hair and skirts float up and spread. Water is heavy:
/// most of the velocity goes each step (`WATER_DAMPING` kept per 1/60 s),
/// no air drag nor wind; a slow current sways it (`CURRENT`, m/s²).
/// Vertices within `WET_BAND` of the surface are partly in it.
const WATER_LIFT: f32 = 0.5;
const WATER_LOOSEN: f32 = 0.9;
const WATER_DAMPING: f32 = 0.85;
const CURRENT: f32 = 1.5;
/// Under water, a push away from the body's axis (m/s²): hair fans out
/// round the head rather than settling back against it.
const WATER_FAN: f32 = 1.5;
const WET_BAND: f32 = 0.05;
/// Share of the freedom a vertex lacks that water gives it (the pinned
/// stay pinned), fully from `WATER_FREE_FROM` free on land, less nearer
/// its pins.
const WATER_FREE: f32 = 0.6;
const WATER_FREE_FROM: f32 = 0.25;
/// How much further than `MAX_STRAY` a vertex may stray under water.
const WATER_STRAY: f32 = 1.0;
/// How hard, under water, a strand resists folding back on itself (its
/// next-but-one nodes kept apart; 1: fully).
const BEND_WATER: f32 = 1.0;
/// How much wider than modelled a sheet may spread under water.
const WATER_SPREAD: f32 = 0.5;
/// The flutter: how much each vertex's push wavers, and how fast.
const FLUTTER: f32 = 0.5;
const FLUTTER_RATE: f32 = 7.0;
/// Fastest a vertex may move against its skinned place, per step (3 m/s):
/// at a low frame rate the body jumps between frames, the constraints throw
/// the cloth after it and that throw, kept as velocity, flung hair all
/// about. Swinging hair moves well under it.
const MAX_SLIP: f32 = 0.05;
/// How hard an edge resists shortening, against 1 for stretching.
const COMPRESSION: f32 = 0.5;
/// A vertex keeps this far outside a collision volume: the volumes are
/// rough ellipsoids, smaller than the drawn body (and a triangle between
/// two vertices outside a thin volume can still cut through it).
pub const SKIN_GAP: f32 = 0.025;
/// How far a vertex may go behind its skinned surface, against its normal
/// (Unity's "backstop"): the creator modelled the cloth outside the body,
/// so however fast the avatar moves, it swings out, back and sideways but
/// never through the legs.
pub const BACKSTOP: f32 = 0.01;

/// One node's skinning: joint indices (into the matrices of the meshes
/// simulated together, each mesh's after the one before) and normalized
/// weights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NodeSkin {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// What a mesh's cloth needs, built once per mesh (shared by its wearers).
#[derive(Debug, Default)]
pub struct ClothMesh {
    /// Per node, its place in the mesh's bind space (what the skinning
    /// takes); the edges' rest lengths are the rest pose's.
    pub nodes: Vec<Vec3>,
    pub free: Vec<f32>,
    /// The freedom while its animated parts are still (a Bento tail no
    /// animation moves: free to swing, `ClothState::held` false).
    pub free_still: Vec<f32>,
    pub skin: Vec<NodeSkin>,
    pub edges: Vec<(u32, u32, f32)>,
    /// See `Weights::bends`.
    pub bends: Vec<(u32, u32, f32)>,
    /// Water lifts and spreads it (hair, clothes); a tail is a limb and
    /// does not (a mermaid's fins drifted apart).
    pub floats: bool,
    /// Per face, per GPU vertex: its node.
    pub node_of: Vec<Vec<u32>>,
    /// Indices of the nodes that move at all.
    pub moving: Vec<u32>,
    /// Per node, its surface normal at bind pose, turned away from the body
    /// (zero: no backstop).
    pub normals: Vec<Vec3>,
    /// Per node, the pinned node it hangs from and its distance along the
    /// surface at rest: the tether (`TETHER_SLACK`).
    pub root: Vec<u32>,
    pub reach: Vec<f32>,
    /// The vertices skinned otherwise than their node (welded across two
    /// meshes simulated together, each skinned in its own bind space): face,
    /// vertex, skin, in face and vertex order. Their offset is taken back
    /// through their own skin (`ClothState::deltas`).
    pub exceptions: Vec<(u32, u32, NodeSkin)>,
    /// Simulated on a lattice (`ClothMesh::coarse`): the nodes above are its
    /// points, `node_of` and `exceptions` the mesh's own welded nodes, which
    /// follow them. None: each node simulated.
    pub drape: Option<Drape>,
    /// The edges (indices into `edges`) in batches no two of whose edges
    /// share a node, and the bends likewise: each batch is solved on every
    /// core at once (`batches`).
    pub edge_batches: Vec<Vec<u32>>,
    pub bend_batches: Vec<Vec<u32>>,
}

/// How the mesh's own nodes follow the lattice's points (`coarse::Lattice`).
#[derive(Debug, Default)]
pub struct Drape {
    /// Per node of the mesh: its skin, its freedom, its cell, whether its
    /// skin is its cell's (its offset taken back through the cell's matrix).
    pub skin: Vec<NodeSkin>,
    pub free: Vec<f32>,
    pub cell: Vec<u32>,
    pub same_skin: Vec<bool>,
    pub lattice: Lattice,
}

/// Per face, per GPU vertex, what the skinning needs: its joints and
/// weights, its place and its normal turned away from the body (zero: no
/// backstop), in the mesh's bind space.
#[derive(Debug, Default)]
pub struct BindData {
    pub skin: Vec<Vec<NodeSkin>>,
    pub positions: Vec<Vec<Vec3>>,
    pub normals: Vec<Vec<Vec3>>,
}

impl ClothMesh {
    /// The weights were worked out on the mesh as it stands on a default
    /// avatar (its rest pose); the skinning takes it from its bind space,
    /// described per face and vertex in `bind` (missing parts: the rest
    /// places stand for the bind ones, no backstop, no tilt).
    pub fn new(w: Weights, bind: &BindData) -> Self {
        let n = w.nodes.len();
        let mut node_skin = vec![NodeSkin::default(); n];
        let mut node_normal = vec![Vec3::ZERO; n];
        let mut node_bind = w.nodes.clone();
        let mut seen = vec![false; n];
        let at = |list: &[Vec<Vec3>], f: usize, v: usize| list.get(f).and_then(|face| face.get(v)).copied();
        for (f, map) in w.node_of.iter().enumerate() {
            for (v, &node) in map.iter().enumerate() {
                let node = node as usize;
                if node >= n {
                    continue;
                }
                if !seen[node]
                    && let Some(s) = bind.skin.get(f).and_then(|face| face.get(v))
                {
                    seen[node] = true;
                    node_skin[node] = *s;
                    if let Some(b) = at(&bind.positions, f, v) {
                        node_bind[node] = b;
                    }
                }
                node_normal[node] += at(&bind.normals, f, v).unwrap_or(Vec3::ZERO);
            }
        }
        let mut exceptions = Vec::new();
        for (f, map) in w.node_of.iter().enumerate() {
            for (v, &node) in map.iter().enumerate() {
                if let Some(s) = bind.skin.get(f).and_then(|face| face.get(v))
                    && node_skin.get(node as usize).is_some_and(|k| s != k)
                {
                    exceptions.push((f as u32, v as u32, *s));
                }
            }
        }
        let unit = |v: Vec<Vec3>| v.into_iter().map(|n| n.normalize_or_zero()).collect();
        let moving = (0..n as u32).filter(|&i| w.node_free[i as usize] > 0.0).collect();
        let (edge_batches, bend_batches) = (batches(&w.edges, n), batches(&w.bends, n));
        Self {
            nodes: node_bind,
            free_still: w.node_free.clone(),
            free: w.node_free,
            skin: node_skin,
            edges: w.edges,
            bends: w.bends,
            floats: true,
            node_of: w.node_of,
            moving,
            normals: unit(node_normal),
            root: w.node_root,
            reach: w.node_reach,
            exceptions,
            drape: None,
            edge_batches,
            bend_batches,
        }
    }

    /// [`ClothMesh::new`], simulated on a lattice of cells about `spacing`
    /// apart (`coarse`): only its points are skinned and stepped; the mesh's
    /// nodes follow them when drawn. `free_still`: per node, see
    /// `ClothMesh::free_still`.
    pub fn coarse(w: Weights, bind: &BindData, free_still: Vec<f32>, spacing: f32) -> Self {
        let rest = w.nodes.clone();
        let pieces = w.pieces.clone();
        let mut fine = Self::new(w, bind);
        if free_still.len() == fine.free.len() {
            fine.free_still = free_still;
        }
        let lattice = super::coarse::build(&rest, &fine.free, &fine.free_still, &fine.edges, &pieces, spacing);
        let reps: Vec<usize> = lattice.rep.iter().map(|&r| r as usize).collect();
        let pick = |v: &[Vec3]| -> Vec<Vec3> { reps.iter().map(|&r| v[r]).collect() };
        let rest_reps = pick(&rest);
        // nearly in line through a third, as the mesh's (`Weights::bends`)
        let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); reps.len()];
        for &(a, b, d) in &lattice.edges {
            adj[a as usize].push((b, d));
            adj[b as usize].push((a, d));
        }
        let bends: Vec<(u32, u32, f32)> = super::weights::bend_pairs(&rest_reps, &adj)
            .into_iter()
            .filter(|&(a, c, _)| lattice.free[a as usize] > 0.0 || lattice.free[c as usize] > 0.0)
            .collect();
        let bend_batches = batches(&bends, reps.len());
        let skin: Vec<NodeSkin> = reps.iter().map(|&r| fine.skin[r]).collect();
        let root = reps
            .iter()
            .map(|&r| {
                lattice
                    .cell_of
                    .get(fine.root.get(r).copied().unwrap_or(r as u32) as usize)
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        let moving = (0..reps.len() as u32).filter(|&c| lattice.free[c as usize] > 0.0).collect();
        let same_skin = (0..fine.skin.len())
            .map(|f| fine.skin[f] == skin[lattice.cell_of[f] as usize])
            .collect();
        Self {
            nodes: pick(&fine.nodes),
            free: lattice.free.clone(),
            free_still: lattice.free_still.clone(),
            skin,
            edges: lattice.edges.clone(),
            bends,
            floats: fine.floats,
            node_of: fine.node_of,
            moving,
            normals: pick(&fine.normals),
            root,
            reach: reps.iter().map(|&r| fine.reach.get(r).copied().unwrap_or(0.0)).collect(),
            exceptions: fine.exceptions,
            edge_batches: batches(&lattice.edges, reps.len()),
            bend_batches,
            drape: Some(Drape {
                cell: lattice.cell_of.clone(),
                skin: fine.skin,
                free: fine.free,
                same_skin,
                lattice,
            }),
        }
    }

    /// The freedom of the mesh's node `f` (`node_of`'s), simulated or not.
    #[cfg(test)]
    pub fn freedom_of(&self, f: usize) -> f32 {
        match &self.drape {
            Some(d) => d.free.get(f).copied().unwrap_or(0.0),
            None => self.free.get(f).copied().unwrap_or(0.0),
        }
    }

    #[cfg(test)]
    pub fn vertex_count(&self) -> usize {
        self.node_of.iter().map(Vec::len).sum()
    }

    /// The skin matrix of a node (linear blend, as `skin_matrix` in
    /// object.wgsl). `mats[j]`: world matrix of mesh joint j times its
    /// inverse bind.
    fn skin_matrix(&self, n: usize, mats: &[Mat4]) -> Mat4 {
        skin_matrix(&self.skin[n], mats)
    }
}

/// Pairs of nodes (`ClothMesh::edges`, `bends`) sorted into batches no two
/// of whose pairs share a node (a greedy colouring): within a batch, each
/// pair moves only its own two nodes and all are solved at once; the
/// batches go one after the other (Gauss-Seidel across them, as before).
pub fn batches(pairs: &[(u32, u32, f32)], n: usize) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = Vec::new();
    // (a pair naming a node out of range is no constraint)
    let mut left: Vec<u32> = (0..pairs.len() as u32)
        .filter(|&e| {
            let (a, b, _) = pairs[e as usize];
            (a as usize) < n && (b as usize) < n
        })
        .collect();
    // 64 batches a round (a node's batches in a bit mask); what a round
    // cannot place goes to the next (each round places its first at least)
    while !left.is_empty() {
        let base = out.len();
        let mut used = vec![0u64; n];
        let mut next = Vec::new();
        for e in left {
            let (a, b, _) = pairs[e as usize];
            let (a, b) = (a as usize, b as usize);
            let taken = used[a] | used[b];
            if taken == u64::MAX {
                next.push(e);
                continue;
            }
            let c = (!taken).trailing_zeros();
            used[a] |= 1 << c;
            used[b] |= 1 << c;
            let c = base + c as usize;
            if out.len() <= c {
                out.resize(c + 1, Vec::new());
            }
            out[c].push(e);
        }
        left = next;
    }
    out
}

/// Below this many items, a loop of the step stays on one core (handing it
/// out costs more than it saves).
const PARALLEL_FROM: usize = 1024;

/// `f` for each of `items`, on every core when there are many.
fn each(items: &[u32], f: impl Fn(u32) + Sync + Send) {
    if items.len() < PARALLEL_FROM {
        items.iter().for_each(|&i| f(i));
    } else {
        items.par_iter().with_min_len(256).for_each(|&i| f(i));
    }
}

/// A slice written from several threads at once, each at indices no other
/// touches meanwhile (a batch's nodes, `moving`'s).
struct Shared<T>(*mut T, usize);
// SAFETY: only `get` and `set` reach the slice, at indices the callers keep
// apart between threads (see their SAFETY comments); `T` is plain data.
unsafe impl<T: Send> Send for Shared<T> {}
unsafe impl<T: Send> Sync for Shared<T> {}

impl<T: Copy> Shared<T> {
    fn new(v: &mut [T]) -> Self {
        Self(v.as_mut_ptr(), v.len())
    }
    /// # Safety
    /// No other thread writes `i` meanwhile.
    unsafe fn get(&self, i: usize) -> T {
        assert!(i < self.1, "index {i} out of {}", self.1);
        // SAFETY: in bounds (checked), and the caller keeps writers of `i`
        // on other threads away
        unsafe { *self.0.add(i) }
    }
    /// # Safety
    /// No other thread reads or writes `i` meanwhile.
    unsafe fn set(&self, i: usize, v: T) {
        assert!(i < self.1, "index {i} out of {}", self.1);
        // SAFETY: as `get`
        unsafe { *self.0.add(i) = v }
    }
}

/// A skin's matrix (linear blend, as `skin_matrix` in object.wgsl).
fn skin_matrix(s: &NodeSkin, mats: &[Mat4]) -> Mat4 {
    let mut m = Mat4::ZERO;
    let mut sum = 0.0;
    for k in 0..4 {
        let w = s.weights[k];
        if w > 0.0 {
            m += *mats.get(s.joints[k] as usize).unwrap_or(&Mat4::IDENTITY) * w;
            sum += w;
        }
    }
    if sum <= 1e-4 { Mat4::IDENTITY } else { m * (1.0 / sum) }
}

/// A collision volume posed this frame: a unit sphere under `m`.
#[derive(Clone, Copy, Debug)]
pub struct Volume {
    m: Mat4,
    inv: Mat4,
    center: Vec3,
    radius: f32,
}

impl Volume {
    pub fn new(m: Mat4) -> Option<Self> {
        (m.determinant().abs() > 1e-12).then(|| Self {
            m,
            inv: m.inverse(),
            center: m.w_axis.truncate(),
            radius: m
                .x_axis
                .truncate()
                .length()
                .max(m.y_axis.truncate().length())
                .max(m.z_axis.truncate().length()),
        })
    }

    /// `p` pushed out of the volume (with a small gap), or None outside it.
    /// `p` pushed out of the volume and its skin gap, but never further
    /// out than `target`, the vertex's own skinned place, lies (the cloth was
    /// modelled there: hair tight to the back, a waistband inside a fitted
    /// hip are not pushed off the body), or None when it is out already.
    fn push_out(&self, p: Vec3, target: Vec3) -> Option<Vec3> {
        if (p - self.center).length_squared() > (self.radius + SKIN_GAP).powi(2) {
            return None;
        }
        let local = self.inv.transform_point3(p);
        let r = local.length();
        let dir = if r < 1e-6 { Vec3::Z } else { local / r };
        // the skin gap in the volume's units along that direction
        let reach = (self.m.transform_point3(dir) - self.center).length().max(1e-4);
        let limit = (1.0 + SKIN_GAP / reach).min(self.inv.transform_point3(target).length());
        (r < limit).then(|| self.m.transform_point3(dir * limit))
    }
}

/// One wearer's state of one mesh's cloth.
#[derive(Debug, Default)]
pub struct ClothState {
    pub x: Vec<Vec3>,
    prev: Vec<Vec3>,
    target: Vec<Vec3>,
    /// This frame's skinned normals (the backstop's).
    normal: Vec<Vec3>,
    /// How far the body is turned over (0 upright .. 1 upside down).
    pub tilt: f32,
    /// The offsets from the skinned places before the last step and after
    /// it, as that step left them (carried with the body until the next
    /// one: measured against where the body is now, they drifted with it
    /// between steps and snapped back at each), and the ones drawn.
    shown: Vec<Vec3>,
    latest: Vec<Vec3>,
    view: Vec<Vec3>,
    skin_inv: Vec<Mat3>,
    /// Per `ClothMesh::exceptions`, its own matrix taking an offset back.
    exception_inv: Vec<Mat3>,
    /// On a lattice: per mesh node skinned otherwise than its cell, its own
    /// matrix taking an offset back (others: unused); per mesh node, its
    /// offset drawn this frame.
    fine_inv: Vec<Mat3>,
    fine_view: Vec<Vec3>,
    last_root: Option<Mat4>,
    /// The root's place, smoothed (`ROOT_SMOOTHING`).
    smooth_root: Option<Vec3>,
    /// Time not simulated yet (under one step after a frame).
    pending: f32,
    /// The skinned places of the last frame, and those a step works to: the
    /// body as posed at the step's own time, between the last frame's pose
    /// and this one's (read from the frame each step fell in, the body
    /// seemed to jump unevenly between steps: a jitter at high frame rates).
    last_target: Vec<Vec3>,
    step_target: Vec<Vec3>,
    /// Time since the cloth last stepped.
    since: f32,
    /// Per edge, its length this step (the skinned one).
    lengths: Vec<f32>,
    /// Per bend pair, its distance this step (under water only).
    bend_lengths: Vec<f32>,
    /// The wind blowing on it (world, m/s), set before each step.
    pub wind: Vec3,
    /// The height of the water's surface where it is worn (world), if any.
    pub water: Option<f32>,
    /// Its animated parts are animated now: held to their capped freedom
    /// (`ClothMesh::free`), else free (`free_still`).
    pub held: bool,

    /// Simulated time (the flutter's clock).
    clock: f32,
    /// The skinned places of the step before.
    before: Vec<Vec3>,
}

impl ClothState {
    /// Advance by `dt` seconds. `mats`: see [`ClothMesh::skin_matrix`];
    /// `root`: the skeleton's placement (its motion carries the cloth in
    /// part, its jumps reset it); `body_down`: the body's rest down as its
    /// pelvis now turns it (zero: upright).
    pub fn step(&mut self, mesh: &ClothMesh, mats: &[Mat4], volumes: &[Volume], root: Mat4, body_down: Vec3, dt: f32) {
        let n = mesh.nodes.len();
        let free: &[f32] = if self.held || mesh.free_still.len() != mesh.free.len() {
            &mesh.free
        } else {
            &mesh.free_still
        };
        // under water, what is not pinned is freer (`WATER_FREE`): with its
        // dry freedom, a strand's upper part, short-leashed and stiff,
        // bunched up round the head and shoulders instead of floating
        // (per node, how far in the water: empty on land)
        let wet_of: Vec<f32> = match self.water.filter(|_| mesh.floats) {
            Some(z) if self.x.len() == free.len() => self.x.iter().map(|p| ((z - p.z) / WET_BAND).clamp(0.0, 1.0)).collect(),
            _ => Vec::new(),
        };
        let wet_free: Vec<f32>;
        let free: &[f32] = if wet_of.is_empty() {
            free
        } else {
            wet_free = free
                .iter()
                .zip(&wet_of)
                // (in proportion near the pins: 2% free on land and 60% in
                // the water, beside a pinned node, it was a hinge the strand
                // folded at)
                .map(|(&f, &wet)| {
                    if f <= 0.0 {
                        0.0
                    } else {
                        f + (1.0 - f) * WATER_FREE * wet * (f / WATER_FREE_FROM).min(1.0)
                    }
                })
                .collect();
            &wet_free
        };
        // this frame's skinned places, and the matrices taking an offset back
        let skinned: Vec<((Vec3, Vec3), Mat3)> = (0..n)
            .into_par_iter()
            .map(|i| {
                let m = mesh.skin_matrix(i, mats);
                let lin = Mat3::from_mat4(m);
                let inv = if lin.determinant().abs() > 1e-12 {
                    lin.inverse()
                } else {
                    Mat3::IDENTITY
                };
                let normal = mesh.normals.get(i).map_or(Vec3::ZERO, |n| (lin * *n).normalize_or_zero());
                ((m.transform_point3(mesh.nodes[i]), normal), inv)
            })
            .collect();
        self.target.clear();
        self.normal.clear();
        self.exception_inv.clear();
        self.exception_inv.extend(mesh.exceptions.iter().map(|(_, _, s)| {
            let lin = Mat3::from_mat4(skin_matrix(s, mats));
            if lin.determinant().abs() > 1e-12 {
                lin.inverse()
            } else {
                Mat3::IDENTITY
            }
        }));
        self.skin_inv.clear();
        for ((t, nrm), inv) in skinned {
            self.target.push(t);
            self.normal.push(nrm);
            self.skin_inv.push(inv);
        }
        if let Some(d) = &mesh.drape {
            let inverse = |f: usize| {
                if d.same_skin[f] || d.lattice.links_of(f).is_empty() {
                    return Mat3::IDENTITY;
                }
                let lin = Mat3::from_mat4(skin_matrix(&d.skin[f], mats));
                if lin.determinant().abs() > 1e-12 {
                    lin.inverse()
                } else {
                    Mat3::IDENTITY
                }
            };
            self.fine_inv.clear();
            self.fine_inv.par_extend((0..d.skin.len()).into_par_iter().map(inverse));
        }
        let body_down = body_down.normalize_or_zero();
        let (tilt, felt) = if body_down == Vec3::ZERO {
            (0.0, Vec3::ZERO)
        } else {
            (
                ((1.0 - body_down.dot(DOWN)) * 0.5).clamp(0.0, 1.0),
                (DOWN - body_down) * TILT_GRAVITY,
            )
        };
        self.tilt = tilt;
        // (the root where the cloth last stepped)
        let previous = self.last_root;
        let jumped = previous.is_some_and(|r| (r.w_axis - root.w_axis).truncate().length() > RESET_JUMP);
        let at = root.w_axis.truncate();
        if self.x.len() != n || jumped || !dt.is_finite() {
            self.x = self.target.clone();
            self.prev = self.target.clone();
            self.pending = 0.0;
            self.last_target = self.target.clone();
            self.step_target = self.target.clone();
            self.before = self.target.clone();
            self.since = 0.0;
            self.last_root = Some(root);
            self.shown = vec![Vec3::ZERO; n];
            self.latest = vec![Vec3::ZERO; n];
            self.view = vec![Vec3::ZERO; n];
            self.smooth_root = Some(at);
            return;
        }
        // a frame of no time (two updates at once): nothing moves. Stepped,
        // the velocity was carried a whole step in no time and snapped back
        // the next: a stutter at high frame rates, gone with a frame cap
        if dt <= 1e-5 {
            return;
        }
        // (time beyond 8 steps a frame, under 7.5 fps, is dropped)
        self.pending = (self.pending + dt).min(8.0 * MAX_STEP);
        self.since += dt;
        let h = MAX_STEP;
        // carried along with the avatar for a share of its motion since the
        // cloth last stepped, and wholly for the root's wobble (its move less
        // the smoothed one's): the cloth feels as inertia only (1 - CARRY) of
        // the smooth motion. Only in a frame it steps: carried every frame,
        // it went with the body between steps and was pulled back at each
        // (a stutter at high frame rates)
        if self.pending >= MAX_STEP
            && let Some(previous) = previous
            && previous.determinant().abs() > 1e-12
        {
            let smooth_before = self.smooth_root.unwrap_or(at);
            let smooth = smooth_before + (at - smooth_before) * (1.0 - (-self.since / ROOT_SMOOTHING).exp());
            self.smooth_root = Some(smooth);
            self.since = 0.0;
            self.last_root = Some(root);
            let moved = root * previous.inverse();
            let wobble = ((at - previous.w_axis.truncate()) - (smooth - smooth_before)) * (1.0 - CARRY);
            for &i in &mesh.moving {
                let i = i as usize;
                self.x[i] = self.x[i].lerp(moved.transform_point3(self.x[i]), CARRY) + wobble;
                self.prev[i] = self.prev[i].lerp(moved.transform_point3(self.prev[i]), CARRY) + wobble;
            }
        }
        let damping = DAMPING.powf(h * 60.0);
        // step until under a step of the frame's time is left; the frame's
        // motion spread evenly over the steps when time was dropped (a frame
        // over 4 steps long: the first step took most of the body's jump)
        let span = dt.min(self.pending).max(1e-6);
        while self.pending >= h {
            self.pending -= h;
            // the body at this step's time: `pending` before the frame's
            let u = (1.0 - self.pending / span).clamp(0.0, 1.0);
            self.before.copy_from_slice(&self.step_target);
            for i in 0..n {
                self.step_target[i] = self.last_target[i].lerp(self.target[i], u);
                // pinned nodes simply follow (the body at this step's time,
                // as their free neighbours see it)
                if free[i] <= 0.0 {
                    self.x[i] = self.step_target[i];
                    self.prev[i] = self.step_target[i];
                }
            }
            self.shown.copy_from_slice(&self.latest);
            // the edges' lengths as the body now skins them: measured on a
            // default avatar, they fought a wearer's shape and the joint
            // offsets a tail or a body brings (a tail's cards squashed
            // sideways to the default skeleton's lengths)
            self.lengths.clear();
            self.lengths.extend(
                mesh.edges
                    .iter()
                    .map(|&(a, b, _)| (self.step_target[b as usize] - self.step_target[a as usize]).length()),
            );
            self.bend_lengths.clear();
            if !wet_of.is_empty() {
                self.bend_lengths.extend(
                    mesh.bends
                        .iter()
                        .map(|&(a, c, _)| (self.step_target[c as usize] - self.step_target[a as usize]).length()),
                );
            }
            // the edges (and bends) in batches of their own: stored, or for
            // a mesh put together by hand, worked out now
            let worked: Vec<Vec<u32>>;
            let edge_batches = if mesh.edge_batches.iter().map(Vec::len).sum::<usize>() == mesh.edges.len() {
                &mesh.edge_batches
            } else {
                worked = batches(&mesh.edges, n);
                &worked
            };
            let worked_bends: Vec<Vec<u32>>;
            let bend_batches = if mesh.bend_batches.iter().map(Vec::len).sum::<usize>() == mesh.bends.len() {
                &mesh.bend_batches
            } else {
                worked_bends = batches(&mesh.bends, n);
                &worked_bends
            };
            let (target, normal) = (&self.step_target, &self.normal);
            // every core works on the nodes at once: each moving node once
            // (`moving` holds no index twice), each batch's pairs on nodes of
            // their own; a pinned node is only read
            let x = Shared::new(&mut self.x);
            let prev = Shared::new(&mut self.prev);
            let before = &self.before;
            let lengths = &self.lengths;
            let bend_lengths = &self.bend_lengths;
            let carried = damping;
            // integrate
            let spring = h * 60.0;
            self.clock += h;
            let (wind, clock, water) = (self.wind, self.clock, self.water.filter(|_| mesh.floats));
            let water_damping = WATER_DAMPING.powf(h * 60.0);
            let wet_of = &wet_of;
            each(&mesh.moving, |i| {
                let i = i as usize;
                // SAFETY: node i is this call's alone (above)
                let (mut xi, pi) = unsafe { (x.get(i), prev.get(i)) };
                let f = free[i];
                // how far in the water (0 dry .. 1 under)
                let wet = water.map_or(0.0, |z| ((z - xi.z) / WET_BAND).clamp(0.0, 1.0));
                let mut v = (xi - pi) * carried * (1.0 + (water_damping - 1.0) * wet);
                // (the carry moves the cloth with the body outside this step:
                // its share of the body's motion is not in `v`)
                let moved = target[i] - before[i];
                // through the air: its own motion and the carried one
                let air = {
                    let rel = wind - (v + moved * CARRY) / h;
                    let speed = rel.length().min(MAX_AIR);
                    rel.normalize_or_zero() * (AIR_DRAG * speed * speed)
                };
                // no faster than MAX_SLIP against the body's own motion
                let body = moved * (1.0 - CARRY);
                let slip = v - body;
                if slip.length_squared() > MAX_SLIP * MAX_SLIP {
                    v = body + slip.normalize_or_zero() * MAX_SLIP;
                }
                // SAFETY: as above
                unsafe { prev.set(i, xi) };
                let flutter = if wind == Vec3::ZERO {
                    0.0
                } else {
                    1.0 + FLUTTER * (clock * FLUTTER_RATE + i as f32 * 0.37).sin()
                };
                let in_air = GRAVITY + felt + wind * (WIND_PUSH * flutter) + air;
                let in_water = if wet > 0.0 {
                    // the current: smooth across space, so neighbours drift
                    // together (a phase of their own each tangled the sheet)
                    let p = xi;
                    let flow = Vec3::new(
                        (p.y * 2.7 + clock * 0.6).sin(),
                        (p.x * 2.9 + clock * 0.5 + 1.7).sin(),
                        0.4 * ((p.x + p.y) * 2.1 + clock * 0.4).sin(),
                    ) * CURRENT;
                    let fan = Vec3::new(p.x - at.x, p.y - at.y, 0.0).normalize_or_zero() * WATER_FAN;
                    Vec3::new(0.0, 0.0, WATER_LIFT * 9.81) + flow + fan
                } else {
                    Vec3::ZERO
                };
                xi += v + in_air.lerp(in_water, wet) * f * h * h;
                // the spring back to the skinned shape, stiffer where less free
                let pull = ((SHAPE_PULL + (1.0 - f) * (1.0 - f) * 0.5) * spring * (1.0 - TILT_LOOSEN * tilt) * (1.0 - WATER_LOOSEN * wet))
                    .min(1.0);
                xi += (target[i] - xi) * pull;
                // SAFETY: as above
                unsafe { x.set(i, xi) };
            });
            for pass in 0..ITERATIONS {
                let last = pass + 1 == ITERATIONS;
                // edges do not stretch and shorten only reluctantly: fabric
                // gathers and folds (a cone of inextensible triangles pinned
                // at its waist could not swing at all); a pinned end does not
                // move
                for batch in edge_batches {
                    each(batch, |e| {
                        let ((a, b, _), rest) = (mesh.edges[e as usize], lengths[e as usize]);
                        let (a, b) = (a as usize, b as usize);
                        let (wa, wb) = (free[a], free[b]);
                        let sum = wa + wb;
                        if sum <= 0.0 {
                            return;
                        }
                        // SAFETY: nodes a and b are this pair's alone in its
                        // batch (`batches`)
                        let (xa, xb) = unsafe { (x.get(a), x.get(b)) };
                        let d = xb - xa;
                        let len = d.length();
                        if len < 1e-7 {
                            return;
                        }
                        // shortening only meets a softer push back: fabric
                        // gathers, but a long thin tail or strand keeps its
                        // length. Under water a sheet may widen
                        // (`WATER_SPREAD`): hair spreads as its strands part
                        // (the tether still keeps each strand's length); held
                        // to its width, it bunched
                        let longest = rest
                            * wet_of
                                .get(a)
                                .zip(wet_of.get(b))
                                .map_or(1.0, |(wa, wb)| 1.0 + WATER_SPREAD * wa.max(*wb));
                        let (goal, stiff) = if len > longest {
                            (longest, 1.0)
                        } else if len < rest {
                            // under water it does not bunch into itself
                            let wet = wet_of.get(a).zip(wet_of.get(b)).map_or(0.0, |(wa, wb)| wa.max(*wb));
                            (rest, COMPRESSION + (1.0 - COMPRESSION) * wet)
                        } else {
                            return;
                        };
                        let corr = d * ((len - goal) * stiff / (len * sum));
                        // SAFETY: as above
                        unsafe {
                            x.set(a, xa + corr * wa);
                            x.set(b, xb - corr * wb);
                        }
                    });
                }
                // under water, no folding back (`BEND_WATER`): the lift
                // pushed the strands' ends toward the scalp and, nothing
                // keeping them straight, they crumpled into zigzags
                if !bend_lengths.is_empty() {
                    for batch in bend_batches {
                        each(batch, |e| {
                            let ((a, c, _), rest) = (mesh.bends[e as usize], bend_lengths[e as usize]);
                            let (a, c) = (a as usize, c as usize);
                            let wet = wet_of[a].max(wet_of[c]);
                            let (wa, wc) = (free[a], free[c]);
                            let sum = wa + wc;
                            if wet <= 0.0 || sum <= 0.0 {
                                return;
                            }
                            // SAFETY: nodes a and c are this pair's alone in
                            // its batch (`batches`)
                            let (xa, xc) = unsafe { (x.get(a), x.get(c)) };
                            let d = xc - xa;
                            let len = d.length();
                            if len < 1e-7 || len >= rest {
                                return;
                            }
                            let corr = d * ((len - rest) * BEND_WATER * wet / (len * sum));
                            // SAFETY: as above
                            unsafe {
                                x.set(a, xa + corr * wa);
                                x.set(c, xc - corr * wc);
                            }
                        });
                    }
                }
                // tethered to the node it hangs from (a pinned one: only read)
                each(&mesh.moving, |i| {
                    let i = i as usize;
                    let r = mesh.root.get(i).map_or(i, |&r| r as usize);
                    if r == i || free[r] > 0.0 {
                        return;
                    }
                    let reach = mesh.reach[i].max((target[i] - target[r]).length()) * TETHER_SLACK;
                    // SAFETY: node i is this call's alone; r is pinned, which
                    // no call writes
                    let (xi, xr) = unsafe { (x.get(i), x.get(r)) };
                    let d = xi - xr;
                    if d.length_squared() > reach * reach {
                        // SAFETY: as above
                        unsafe { x.set(i, xr + d.normalize_or_zero() * reach) };
                    }
                });
                // how far it may stray from its skinned place and never behind
                // its surface (the backstop), in every pass so the edges cannot
                // pull it back inside; then, once, the body's volumes (the
                // backstop already keeps it out of the wearer's own shape: they
                // catch the rest, an arm through the hair, a seat)
                each(&mesh.moving, |i| {
                    let i = i as usize;
                    // SAFETY: node i is this call's alone
                    let mut xi = unsafe { x.get(i) };
                    let off = xi - target[i];
                    // (under water further: hair floating up over the scalp
                    // goes twice its depth below it from where it hangs, and
                    // held short it kinked; the tether keeps it on)
                    let wet = wet_of.get(i).copied().unwrap_or(0.0);
                    let max = free[i] * (MAX_STRAY + TILT_STRAY * tilt + WATER_STRAY * wet);
                    if off.length_squared() > max * max {
                        xi = target[i] + off.normalize_or_zero() * max;
                    }
                    let behind = (xi - target[i]).dot(normal[i]);
                    let backstop = BACKSTOP + TILT_BACKSTOP * tilt;
                    if behind < -backstop {
                        xi += normal[i] * (-backstop - behind);
                    }
                    if last {
                        for v in volumes {
                            if let Some(p) = v.push_out(xi, target[i]) {
                                xi = p;
                            }
                        }
                    }
                    // SAFETY: as above
                    unsafe { x.set(i, xi) };
                });
            }
            for i in 0..n {
                self.latest[i] = self.x[i] - self.step_target[i];
            }
        }
        self.last_target.copy_from_slice(&self.target);
        // drawn: between the last two steps, by the time since the last
        // (the offsets from the body, which itself is drawn as posed now)
        let alpha = (self.pending / h).clamp(0.0, 1.0);
        for i in 0..n {
            self.view[i] = self.shown[i].lerp(self.latest[i], alpha);
        }
    }

    /// The offsets drawn this frame, per node.
    #[cfg(test)]
    pub fn view(&self) -> &[Vec3] {
        &self.view
    }

    /// How far the furthest node is from its skinned place (diagnostics).
    pub fn largest_offset(&self) -> f32 {
        self.view.iter().map(|v| v.length()).fold(0.0, f32::max)
    }

    /// Per face, per GPU vertex, its offset in the mesh's bind space
    /// (xyz, w unused), appended to `out` face after face.
    pub fn deltas(&mut self, mesh: &ClothMesh, out: &mut Vec<[f32; 4]>) {
        if let Some(d) = &mesh.drape {
            // the mesh's nodes, each between the points it follows
            let view = &self.view;
            self.fine_view.clear();
            self.fine_view.par_extend((0..d.skin.len()).into_par_iter().map(|f| {
                d.lattice
                    .links_of(f)
                    .iter()
                    .map(|&(c, w)| view.get(c as usize).copied().unwrap_or(Vec3::ZERO) * w)
                    .sum::<Vec3>()
            }));
        }
        // every vertex on every core (a dense dress: 200 000 a frame), in
        // face and vertex order
        let this = &*self;
        let vertices = mesh
            .node_of
            .par_iter()
            .enumerate()
            .flat_map(|(f, map)| map.par_iter().enumerate().map(move |(v, &node)| (f, v, node)));
        out.par_extend(vertices.map(|(f, v, node)| {
            let n = node as usize;
            // (a vertex skinned otherwise than its node: its own matrix;
            // `exceptions` is in face and vertex order)
            let exception = mesh
                .exceptions
                .binary_search_by(|x| (x.0, x.1).cmp(&(f as u32, v as u32)))
                .ok()
                .and_then(|k| this.exception_inv.get(k));
            let d = match &mesh.drape {
                Some(drape) => {
                    let inv = exception.or_else(|| {
                        if drape.same_skin.get(n).copied().unwrap_or(false) {
                            this.skin_inv.get(*drape.cell.get(n)? as usize)
                        } else {
                            this.fine_inv.get(n)
                        }
                    });
                    match (this.fine_view.get(n), inv) {
                        (Some(&off), Some(inv)) if drape.free.get(n).is_some_and(|&f| f > 0.0) => *inv * off,
                        _ => Vec3::ZERO,
                    }
                }
                None => match (this.view.get(n), exception.or_else(|| this.skin_inv.get(n))) {
                    (Some(&off), Some(inv)) if mesh.free.get(n).is_some_and(|&f| f > 0.0) => *inv * off,
                    _ => Vec3::ZERO,
                },
            };
            [d.x, d.y, d.z, 0.0]
        }));
    }

    /// The offset drawn this frame of the mesh's node `f` (`node_of`'s), in
    /// world space (after `deltas`, on a lattice).
    #[cfg(test)]
    pub fn offset_of(&self, mesh: &ClothMesh, f: usize) -> Vec3 {
        match &mesh.drape {
            Some(_) => self.fine_view.get(f).copied().unwrap_or(Vec3::ZERO),
            None => self.view.get(f).copied().unwrap_or(Vec3::ZERO),
        }
    }
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
